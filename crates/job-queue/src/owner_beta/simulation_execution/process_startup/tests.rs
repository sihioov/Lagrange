use super::*;

#[test]
fn independent_receipt_pin_must_match_checked_config() {
    assert!(check_receipt_pin("sha256:abc", "abc").is_ok());
    assert!(check_receipt_pin("sha256:abc", "def").is_err());
    assert!(check_receipt_pin("sha256:abc", "sha256:abc").is_err());
}

#[test]
fn observed_identity_mismatch_is_rejected_at_production_gate() {
    let good = || {
        identities_match(
            "checked",
            1,
            2,
            3,
            4,
            ["checked"; 6],
            [1; 5],
            [2; 5],
            [3; 4],
            [4; 3],
        )
    };
    assert!(good());
    assert!(!identities_match(
        "checked",
        1,
        2,
        3,
        4,
        [
            "other", "checked", "checked", "checked", "checked", "checked"
        ],
        [1; 5],
        [2; 5],
        [3; 4],
        [4; 3]
    ));
    assert!(!identities_match(
        "checked",
        1,
        2,
        3,
        4,
        ["checked"; 6],
        [1, 1, 1, 1, 9],
        [2; 5],
        [3; 4],
        [4; 3]
    ));
    assert!(!identities_match(
        "checked",
        1,
        2,
        3,
        4,
        ["checked"; 6],
        [1; 5],
        [2, 2, 2, 9, 2],
        [3; 4],
        [4; 3]
    ));
    assert!(!identities_match(
        "checked",
        1,
        2,
        3,
        4,
        ["checked"; 6],
        [1; 5],
        [2; 5],
        [3, 3, 9, 3],
        [4; 3]
    ));
    assert!(!identities_match(
        "checked",
        1,
        2,
        3,
        4,
        ["checked"; 6],
        [1; 5],
        [2; 5],
        [3; 4],
        [4, 9, 4]
    ));
}

#[test]
fn each_worker_checkout_rejects_identity_role_and_timeout_contamination() {
    let valid = Some((2, 1, true, true, 5_000, 5_000, true));
    assert!(session_matches(valid, 1, 2, 30_000));
    for row in [
        None,
        Some((3, 1, true, true, 5_000, 5_000, true)),
        Some((2, 9, true, true, 5_000, 5_000, true)),
        Some((2, 1, false, true, 5_000, 5_000, true)),
        Some((2, 1, true, false, 5_000, 5_000, true)),
        Some((2, 1, true, true, 0, 5_000, true)),
        Some((2, 1, true, true, 5_000, 30_001, true)),
        Some((2, 1, true, true, 5_000, 5_000, false)),
    ] {
        assert!(!session_matches(row, 1, 2, 30_000));
    }
}

#[tokio::test]
async fn canceled_or_failed_owned_startup_step_cannot_return_evidence() {
    let (tx, mut rx) = watch::channel(false);
    let canceled = AtomicBool::new(false);
    let result = owned_step(
        async { Err::<(), ()>(()) },
        Instant::now() + Duration::from_secs(1),
        &mut rx,
        &canceled,
    )
    .await;
    assert!(result.is_err());
    tx.send_replace(true);
    let called = AtomicBool::new(false);
    let result = owned_step(
        async {
            called.store(true, Ordering::SeqCst);
            Ok::<_, ()>(())
        },
        Instant::now() + Duration::from_secs(1),
        &mut rx,
        &canceled,
    )
    .await;
    assert!(result.is_err());
    assert!(canceled.load(Ordering::SeqCst));
    assert!(
        !called.load(Ordering::SeqCst),
        "pre-canceled work must not start"
    );
}

#[tokio::test]
async fn startup_preserves_error_and_cancellation_from_same_await() {
    let (tx, mut rx) = watch::channel(false);
    let canceled = AtomicBool::new(false);
    let result = owned_step(
        async {
            tx.send_replace(true);
            Err::<(), ()>(())
        },
        Instant::now() + Duration::from_secs(1),
        &mut rx,
        &canceled,
    )
    .await;
    assert_eq!(
        result,
        Err(StepFailure {
            operation: Some(()),
            canceled: true,
            timed_out: false
        })
    );
}
