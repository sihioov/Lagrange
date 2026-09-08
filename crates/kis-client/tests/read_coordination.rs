#[path = "read_coordination/support.rs"]
mod support;

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use fs2::FileExt;
use kis_client::auth::{AccessToken, TokenIssuer};
use kis_client::clock::Clock;
use kis_client::error::KisError;
use kis_client::read_coordination::{
    LOCK_FILE_NAME, LockAcquisition, ReadCallbackResult, ReadChannel, ReadCoordinationError,
    ReadCoordinator, ReadFailureKind, STATE_FILE_NAME,
};
use kis_client::secret::Secret;

use support::{
    CountingIssuer, FailingIssuer, PrivateTemp, SettableClock, coordinator_at, prime_token,
};

const PRICE_PATH: &str = "/uapi/domestic-stock/v1/quotations/inquire-price";
const PRICE_TR: &str = "FHKST01010100";
const DAILY_PATH: &str = "/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice";
const DAILY_TR: &str = "FHKST03010100";

fn bounded() -> LockAcquisition {
    LockAcquisition::Bounded(Duration::from_secs(1))
}

async fn successful_read(
    coordinator: &ReadCoordinator,
    issuer: &dyn TokenIssuer,
    path: &str,
    tr_id: &str,
) -> Result<&'static str, ReadCoordinationError> {
    coordinator
        .execute(
            path,
            tr_id,
            issuer,
            bounded(),
            Duration::from_secs(3),
            |_| async { ReadCallbackResult::Success("ok") },
        )
        .await
}

#[tokio::test]
async fn token_reuse_alias_dedup_and_get_accounting_are_separate() {
    let temp = PrivateTemp::new("token-reuse");
    let clock = SettableClock::at(1_000_000);
    let issuer = CountingIssuer::new(clock.clone());
    let first = coordinator_at(
        temp.root().to_path_buf(),
        temp.uid(),
        Arc::new(clock.clone()),
        2,
        " app-key\n",
        " app-secret\n",
    );
    let alias = coordinator_at(
        temp.root().to_path_buf(),
        temp.uid(),
        Arc::new(clock.clone()),
        2,
        "app-key",
        "app-secret",
    );

    let one = prime_token(&first, &issuer).await.expect("first token");
    let two = prime_token(&alias, &issuer).await.expect("alias token");
    assert_eq!(issuer.calls(), 1, "same resolved values share one token");
    assert!(
        one.value.expose() == two.value.expose(),
        "shared token values differed"
    );

    let reads = AtomicUsize::new(0);
    let result = alias
        .execute(
            PRICE_PATH,
            PRICE_TR,
            &issuer,
            bounded(),
            Duration::from_secs(3),
            |_| async {
                reads.fetch_add(1, Ordering::SeqCst);
                ReadCallbackResult::Success(())
            },
        )
        .await;
    assert_eq!(result, Ok(()));
    assert_eq!(reads.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn invalid_channel_is_rejected_before_token_or_read_callback() {
    let temp = PrivateTemp::new("deny-channel");
    let clock = SettableClock::at(1_000_000);
    let issuer = CountingIssuer::new(clock.clone());
    let coordinator = temp.coordinator(Arc::new(clock), 1);
    let reads = AtomicUsize::new(0);
    let error = coordinator
        .execute(
            "/uapi/domestic-stock/v1/trading/inquire-balance",
            "TTTC8434R",
            &issuer,
            bounded(),
            Duration::from_secs(3),
            |_| async {
                reads.fetch_add(1, Ordering::SeqCst);
                ReadCallbackResult::Success(())
            },
        )
        .await
        .expect_err("account channel must be denied");
    assert!(matches!(
        error,
        ReadCoordinationError::UnsupportedChannel { .. }
    ));
    assert_eq!(issuer.calls(), 0);
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    assert!(!temp.root().join(LOCK_FILE_NAME).exists());
}

#[tokio::test]
async fn lock_wait_and_request_deadline_inputs_are_bounded_before_callbacks() {
    let temp = PrivateTemp::new("bounded-inputs");
    let clock = SettableClock::at(1_500_000);
    let issuer = CountingIssuer::new(clock.clone());
    let coordinator = temp.coordinator(Arc::new(clock), 1);
    let reads = AtomicUsize::new(0);
    let error = coordinator
        .execute(
            PRICE_PATH,
            PRICE_TR,
            &issuer,
            bounded(),
            Duration::ZERO,
            |_| async {
                reads.fetch_add(1, Ordering::SeqCst);
                ReadCallbackResult::Success(())
            },
        )
        .await
        .expect_err("zero request deadline");
    assert_eq!(error, ReadCoordinationError::InvalidRequestTimeout);
    assert_eq!(issuer.calls(), 0);
    assert_eq!(reads.load(Ordering::SeqCst), 0);

    let error = coordinator
        .token(&issuer, LockAcquisition::Bounded(Duration::ZERO))
        .await
        .expect_err("zero lock wait");
    assert_eq!(error, ReadCoordinationError::InvalidLockWait);
    assert_eq!(issuer.calls(), 0);
    assert!(!temp.root().join(LOCK_FILE_NAME).exists());
}

#[test]
fn isolated_channel_enum_has_exact_parity_with_existing_private_allowlist() {
    let source = include_str!("../src/market_data.rs");
    let start = source
        .find("const READ_ONLY_CHANNELS")
        .expect("allowlist start");
    let tail = &source[start..];
    let end = tail.find("];\n").expect("allowlist end");
    let block = &tail[..end];
    let literals = quoted_literals(block);
    let existing: Vec<(&str, &str)> = literals
        .chunks_exact(2)
        .map(|pair| (pair[0], pair[1]))
        .collect();
    let isolated: Vec<_> = ReadChannel::ALL
        .into_iter()
        .map(ReadChannel::pair)
        .collect();
    assert_eq!(existing, isolated);
}

fn quoted_literals(input: &str) -> Vec<&str> {
    let mut output = Vec::new();
    let mut rest = input;
    while let Some(start) = rest.find('"') {
        rest = &rest[start + 1..];
        let end = rest.find('"').expect("closed literal");
        output.push(&rest[..end]);
        rest = &rest[end + 1..];
    }
    output
}

#[tokio::test]
async fn global_channel_and_intraday_spacing_use_final_post_issue_time() {
    let temp = PrivateTemp::new("spacing");
    let clock = SettableClock::at(2_000_000);
    let issuer = CountingIssuer::advancing(clock.clone(), 2_000);
    let coordinator = temp.coordinator(Arc::new(clock.clone()), 1);

    successful_read(&coordinator, &issuer, PRICE_PATH, PRICE_TR)
        .await
        .expect("first read");
    assert_eq!(issuer.calls(), 1);
    // The reservation was made after the issuer advanced time to 2_002_000.
    clock.set(2_006_000);
    let error = successful_read(&coordinator, &issuer, PRICE_PATH, PRICE_TR)
        .await
        .expect_err("only four seconds from actual read start");
    assert_eq!(
        error,
        ReadCoordinationError::IntradaySpacing {
            retry_after_ms: 1_000
        }
    );
    clock.set(2_007_000);
    successful_read(&coordinator, &issuer, PRICE_PATH, PRICE_TR)
        .await
        .expect("five seconds from actual start");

    let error = successful_read(&coordinator, &issuer, DAILY_PATH, DAILY_TR)
        .await
        .expect_err("global one-second spacing applies across channels");
    assert!(matches!(error, ReadCoordinationError::GlobalSpacing { .. }));
    clock.advance(1_000);
    successful_read(&coordinator, &issuer, DAILY_PATH, DAILY_TR)
        .await
        .expect("different channel after global interval");
}

#[tokio::test]
async fn normal_completion_clears_only_its_fence_but_retains_spacing_debt() {
    let temp = PrivateTemp::new("normal-fence");
    let clock = SettableClock::at(3_000_000);
    let issuer = CountingIssuer::new(clock.clone());
    let coordinator = temp.coordinator(Arc::new(clock.clone()), 1);
    successful_read(&coordinator, &issuer, PRICE_PATH, PRICE_TR)
        .await
        .expect("read");

    let error = successful_read(&coordinator, &issuer, PRICE_PATH, PRICE_TR)
        .await
        .expect_err("spacing remains");
    assert!(matches!(error, ReadCoordinationError::GlobalSpacing { .. }));
    assert!(!matches!(
        error,
        ReadCoordinationError::ReservationActive { .. }
    ));
}

#[tokio::test]
async fn failed_issue_is_durable_and_cooldown_survives_the_callback() {
    let temp = PrivateTemp::new("issue-failure");
    let clock = SettableClock::at(4_000_000);
    let coordinator = temp.coordinator(Arc::new(clock.clone()), 1);
    let issuer = FailingIssuer {
        calls: AtomicUsize::new(0),
    };
    assert_eq!(
        prime_token(&coordinator, &issuer)
            .await
            .expect_err("issuer failure"),
        ReadCoordinationError::TokenIssueFailed
    );
    assert!(matches!(
        prime_token(&coordinator, &issuer).await,
        Err(ReadCoordinationError::TokenIssueCooldown {
            retry_after_ms: 60_000
        })
    ));
    assert_eq!(issuer.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn unauthorized_invalidates_shared_token_and_keeps_issue_gate() {
    let temp = PrivateTemp::new("unauthorized");
    let clock = SettableClock::at(5_000_000);
    let issuer = CountingIssuer::new(clock.clone());
    let coordinator = temp.coordinator(Arc::new(clock.clone()), 1);
    let error = coordinator
        .execute(
            PRICE_PATH,
            PRICE_TR,
            &issuer,
            bounded(),
            Duration::from_secs(3),
            |_| async { ReadCallbackResult::<()>::Unauthorized },
        )
        .await
        .expect_err("401");
    assert_eq!(error, ReadCoordinationError::Unauthorized);
    clock.advance(5_000);
    assert!(matches!(
        successful_read(&coordinator, &issuer, PRICE_PATH, PRICE_TR).await,
        Err(ReadCoordinationError::TokenIssueCooldown { .. })
    ));
    assert_eq!(issuer.calls(), 1);
    clock.advance(55_000);
    successful_read(&coordinator, &issuer, PRICE_PATH, PRICE_TR)
        .await
        .expect("reissue after minute");
    assert_eq!(issuer.calls(), 2);
}

#[tokio::test]
async fn retry_after_is_persisted_before_unlock() {
    let temp = PrivateTemp::new("retry-after");
    let clock = SettableClock::at(6_000_000);
    let issuer = CountingIssuer::new(clock.clone());
    let first = temp.coordinator(Arc::new(clock.clone()), 1);
    let error = first
        .execute(
            PRICE_PATH,
            PRICE_TR,
            &issuer,
            bounded(),
            Duration::from_secs(3),
            |_| async {
                ReadCallbackResult::<()>::RateLimited {
                    retry_after: Duration::from_secs(10),
                }
            },
        )
        .await
        .expect_err("429");
    assert!(matches!(
        error,
        ReadCoordinationError::CallbackRateLimited { .. }
    ));

    clock.advance(5_000);
    let restarted = temp.coordinator(Arc::new(clock.clone()), 1);
    let reads = AtomicUsize::new(0);
    let error = restarted
        .execute(
            PRICE_PATH,
            PRICE_TR,
            &issuer,
            bounded(),
            Duration::from_secs(3),
            |_| async {
                reads.fetch_add(1, Ordering::SeqCst);
                ReadCallbackResult::Success(())
            },
        )
        .await
        .expect_err("shared cooldown");
    assert_eq!(
        error,
        ReadCoordinationError::BrokerCooldown {
            retry_after_ms: 5_000
        }
    );
    assert_eq!(reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn generation_rotation_rejects_same_or_older_mismatch_and_retains_cooldowns() {
    let temp = PrivateTemp::new("rotation");
    let clock = SettableClock::at(7_000_000);
    let issuer = CountingIssuer::new(clock.clone());
    let generation_two = coordinator_at(
        temp.root().to_path_buf(),
        temp.uid(),
        Arc::new(clock.clone()),
        2,
        "key-two",
        "secret-two",
    );
    prime_token(&generation_two, &issuer)
        .await
        .expect("gen2 token");

    let same_generation_mismatch = coordinator_at(
        temp.root().to_path_buf(),
        temp.uid(),
        Arc::new(clock.clone()),
        2,
        "other-key",
        "other-secret",
    );
    assert_eq!(
        prime_token(&same_generation_mismatch, &issuer)
            .await
            .expect_err("same-generation mismatch"),
        ReadCoordinationError::CredentialScopeMismatch
    );
    let older = coordinator_at(
        temp.root().to_path_buf(),
        temp.uid(),
        Arc::new(clock.clone()),
        1,
        "key-two",
        "secret-two",
    );
    assert_eq!(
        prime_token(&older, &issuer)
            .await
            .expect_err("older generation"),
        ReadCoordinationError::StaleCredentialGeneration
    );

    let generation_three = coordinator_at(
        temp.root().to_path_buf(),
        temp.uid(),
        Arc::new(clock.clone()),
        3,
        "key-three",
        "secret-three",
    );
    assert!(matches!(
        prime_token(&generation_three, &issuer).await,
        Err(ReadCoordinationError::TokenIssueCooldown { .. })
    ));
    assert_eq!(issuer.calls(), 1);
    clock.advance(60_000);
    prime_token(&generation_three, &issuer)
        .await
        .expect("rotated token after retained cooldown");
    assert_eq!(issuer.calls(), 2);
    assert_eq!(
        prime_token(&generation_two, &issuer)
            .await
            .expect_err("old reader after rotation"),
        ReadCoordinationError::StaleCredentialGeneration
    );
}

#[tokio::test]
async fn clock_rollback_and_future_high_water_fail_closed() {
    let temp = PrivateTemp::new("clock-rollback");
    let clock = SettableClock::at(8_000_000);
    let issuer = CountingIssuer::new(clock.clone());
    let coordinator = temp.coordinator(Arc::new(clock.clone()), 1);
    prime_token(&coordinator, &issuer).await.expect("prime");
    clock.set(7_999_999);
    let error = prime_token(&coordinator, &issuer)
        .await
        .expect_err("rollback");
    assert_eq!(error, ReadCoordinationError::ClockRollback);
    assert_eq!(issuer.calls(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_bounded_wait_does_not_block_reactor_and_quote_skips_busy_lock() {
    let temp = PrivateTemp::new("lock-timeout");
    let clock = SettableClock::at(9_000_000);
    let issuer = CountingIssuer::new(clock.clone());
    let coordinator = temp.coordinator(Arc::new(clock.clone()), 1);
    prime_token(&coordinator, &issuer)
        .await
        .expect("create lock");

    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(temp.root().join(LOCK_FILE_NAME))
        .expect("open stable lock");
    FileExt::lock_exclusive(&lock).expect("hold lock");

    let start = Instant::now();
    let error = coordinator
        .token(&issuer, LockAcquisition::NonBlocking)
        .await
        .expect_err("quote skips busy gate");
    assert_eq!(error, ReadCoordinationError::LockBusy);
    assert!(start.elapsed() < Duration::from_millis(100));

    let ticks = Arc::new(AtomicUsize::new(0));
    let ticker_count = Arc::clone(&ticks);
    let ticker = tokio::spawn(async move {
        for _ in 0..5 {
            tokio::time::sleep(Duration::from_millis(10)).await;
            ticker_count.fetch_add(1, Ordering::SeqCst);
        }
    });
    let error = coordinator
        .token(&issuer, LockAcquisition::Bounded(Duration::from_millis(75)))
        .await
        .expect_err("bounded lock timeout");
    ticker.await.expect("ticker");
    assert_eq!(error, ReadCoordinationError::LockTimeout);
    assert_eq!(ticks.load(Ordering::SeqCst), 5, "reactor kept scheduling");
    FileExt::unlock(&lock).expect("unlock");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropping_a_waiting_acquisition_cannot_leak_a_future_lock_holder() {
    let temp = PrivateTemp::new("cancel-lock-wait");
    let clock = SettableClock::at(9_500_000);
    let issuer = Arc::new(CountingIssuer::new(clock.clone()));
    let coordinator = temp.coordinator(Arc::new(clock), 1);
    prime_token(&coordinator, issuer.as_ref())
        .await
        .expect("create lock");
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(temp.root().join(LOCK_FILE_NAME))
        .expect("open lock");
    FileExt::lock_exclusive(&lock).expect("hold lock");

    let waiting_coordinator = coordinator.clone();
    let waiting_issuer = Arc::clone(&issuer);
    let waiting = tokio::spawn(async move {
        waiting_coordinator
            .token(
                waiting_issuer.as_ref(),
                LockAcquisition::Bounded(Duration::from_millis(150)),
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    waiting.abort();
    assert!(waiting.await.expect_err("aborted waiter").is_cancelled());
    FileExt::unlock(&lock).expect("release original holder");

    // The detached blocking poll may finish after cancellation, but its local
    // descriptor must immediately drop rather than retaining the lock.
    tokio::time::sleep(Duration::from_millis(30)).await;
    prime_token(&coordinator, issuer.as_ref())
        .await
        .expect("lock remains usable after dropped waiter");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_callback_preserves_reservation_debt_and_releases_kernel_lock() {
    let temp = PrivateTemp::new("cancel-read");
    let clock = SettableClock::at(10_000_000);
    let issuer = Arc::new(CountingIssuer::new(clock.clone()));
    let coordinator = temp.coordinator(Arc::new(clock.clone()), 1);
    let task_coordinator = coordinator.clone();
    let task_issuer = Arc::clone(&issuer);
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        task_coordinator
            .execute(
                PRICE_PATH,
                PRICE_TR,
                task_issuer.as_ref(),
                bounded(),
                Duration::from_secs(3),
                |_| async move {
                    let _ = started_tx.send(());
                    std::future::pending::<ReadCallbackResult<()>>().await
                },
            )
            .await
    });
    started_rx
        .await
        .expect("reservation durable before callback");
    task.abort();
    assert!(task.await.expect_err("cancelled").is_cancelled());

    let error = successful_read(&coordinator, issuer.as_ref(), PRICE_PATH, PRICE_TR)
        .await
        .expect_err("abandoned fence remains");
    assert_eq!(
        error,
        ReadCoordinationError::ReservationActive {
            retry_after_ms: 3_000
        }
    );
}

#[tokio::test]
async fn local_callback_timeout_drops_future_and_preserves_durable_reservation() {
    struct DropCounter(Arc<AtomicUsize>);

    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    let temp = PrivateTemp::new("callback-timeout");
    let clock = SettableClock::at(10_500_000);
    let issuer = CountingIssuer::new(clock.clone());
    let coordinator = temp.coordinator(Arc::new(clock.clone()), 1);
    let drops = Arc::new(AtomicUsize::new(0));
    let callback_drops = Arc::clone(&drops);
    let started = Instant::now();
    let error = coordinator
        .execute(
            PRICE_PATH,
            PRICE_TR,
            &issuer,
            bounded(),
            Duration::from_millis(30),
            move |_| async move {
                let _drop_counter = DropCounter(callback_drops);
                std::future::pending::<ReadCallbackResult<()>>().await
            },
        )
        .await
        .expect_err("cooperative pending callback reaches its local deadline");
    assert_eq!(error, ReadCoordinationError::CallbackTimedOut);
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(drops.load(Ordering::SeqCst), 1, "callback future dropped");

    let restarted = temp.coordinator(Arc::new(clock.clone()), 1);
    let reads = AtomicUsize::new(0);
    let error = restarted
        .execute(
            PRICE_PATH,
            PRICE_TR,
            &issuer,
            LockAcquisition::NonBlocking,
            Duration::from_secs(3),
            |_| async {
                reads.fetch_add(1, Ordering::SeqCst);
                ReadCallbackResult::Success(())
            },
        )
        .await
        .expect_err("persisted fence denies a restarted caller before expiry");
    assert_eq!(
        error,
        ReadCoordinationError::ReservationActive { retry_after_ms: 30 }
    );
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    assert_eq!(issuer.calls(), 1);

    clock.advance(5_000);
    successful_read(&restarted, &issuer, PRICE_PATH, PRICE_TR)
        .await
        .expect("expired fence permits a separately accounted attempt");
}

#[tokio::test]
async fn explicit_ambiguous_result_preserves_debt_but_completed_failure_clears_it() {
    let temp = PrivateTemp::new("ambiguity");
    let clock = SettableClock::at(11_000_000);
    let issuer = CountingIssuer::new(clock.clone());
    let coordinator = temp.coordinator(Arc::new(clock.clone()), 1);
    let error = coordinator
        .execute(
            DAILY_PATH,
            DAILY_TR,
            &issuer,
            bounded(),
            Duration::from_secs(3),
            |_| async { ReadCallbackResult::<()>::Ambiguous },
        )
        .await
        .expect_err("ambiguous");
    assert_eq!(error, ReadCoordinationError::CallbackAmbiguous);
    assert!(matches!(
        successful_read(&coordinator, &issuer, DAILY_PATH, DAILY_TR).await,
        Err(ReadCoordinationError::ReservationActive { .. })
    ));

    clock.advance(3_000);
    let error = coordinator
        .execute(
            DAILY_PATH,
            DAILY_TR,
            &issuer,
            bounded(),
            Duration::from_secs(3),
            |_| async {
                ReadCallbackResult::<()>::CompletedFailure(ReadFailureKind::ResponseInvalid)
            },
        )
        .await
        .expect_err("known failure");
    assert!(matches!(
        error,
        ReadCoordinationError::CallbackFailed { .. }
    ));
    let error = successful_read(&coordinator, &issuer, DAILY_PATH, DAILY_TR)
        .await
        .expect_err("known completion cleared fence, spacing remains");
    assert!(matches!(error, ReadCoordinationError::GlobalSpacing { .. }));
}

#[tokio::test]
async fn state_root_and_files_reject_wrong_owner_modes_symlinks_hardlinks_and_corruption() {
    // Wrong expected owner requires no chown and touches only a private temp dir.
    let temp = PrivateTemp::new("trust-owner");
    let clock = SettableClock::at(12_000_000);
    let issuer = CountingIssuer::new(clock.clone());
    let wrong_owner = coordinator_at(
        temp.root().to_path_buf(),
        temp.uid().saturating_add(1),
        Arc::new(clock.clone()),
        1,
        "key",
        "secret",
    );
    assert_eq!(
        prime_token(&wrong_owner, &issuer)
            .await
            .expect_err("wrong expected owner"),
        ReadCoordinationError::UnsafeRoot
    );

    fs::set_permissions(temp.root(), fs::Permissions::from_mode(0o750)).expect("broaden root");
    assert_eq!(
        prime_token(&temp.coordinator(Arc::new(clock.clone()), 1), &issuer)
            .await
            .expect_err("broad root mode"),
        ReadCoordinationError::UnsafeRoot
    );
    fs::set_permissions(temp.root(), fs::Permissions::from_mode(0o700)).expect("restore root");

    let coordinator = temp.coordinator(Arc::new(clock.clone()), 1);
    prime_token(&coordinator, &issuer)
        .await
        .expect("initialize state");
    let state = temp.root().join(STATE_FILE_NAME);
    fs::set_permissions(&state, fs::Permissions::from_mode(0o640)).expect("broaden state");
    assert_eq!(
        prime_token(&coordinator, &issuer)
            .await
            .expect_err("broad state mode"),
        ReadCoordinationError::UnsafeFile
    );
    fs::set_permissions(&state, fs::Permissions::from_mode(0o600)).expect("restore state");

    let hardlink = temp.base().join("state-hardlink");
    fs::hard_link(&state, &hardlink).expect("hard link state");
    assert_eq!(
        prime_token(&coordinator, &issuer)
            .await
            .expect_err("hardlinked state"),
        ReadCoordinationError::UnsafeFile
    );
    fs::remove_file(&hardlink).expect("remove test hardlink");

    let original = fs::read(&state).expect("read canonical state");
    let mut noncanonical = Vec::with_capacity(original.len() + 1);
    noncanonical.push(b' ');
    noncanonical.extend_from_slice(&original);
    fs::write(&state, &noncanonical).expect("write noncanonical state");
    assert_eq!(
        prime_token(&coordinator, &issuer)
            .await
            .expect_err("noncanonical state"),
        ReadCoordinationError::NonCanonicalState
    );
    fs::write(&state, b"{\"schema_version\":2}\n").expect("write unknown schema");
    assert_eq!(
        prime_token(&coordinator, &issuer)
            .await
            .expect_err("unknown schema"),
        ReadCoordinationError::CorruptState
    );
    fs::write(&state, vec![b'x'; 65 * 1024]).expect("oversized state");
    assert_eq!(
        prime_token(&coordinator, &issuer)
            .await
            .expect_err("oversized state"),
        ReadCoordinationError::StateTooLarge
    );
}

#[tokio::test]
async fn denied_state_invokes_neither_issuer_nor_read_callback() {
    let temp = PrivateTemp::new("denied-state-callbacks");
    let clock = SettableClock::at(12_500_000);
    let issuer = CountingIssuer::new(clock.clone());
    let coordinator = temp.coordinator(Arc::new(clock), 1);
    prime_token(&coordinator, &issuer)
        .await
        .expect("initialize");
    let issue_calls = issuer.calls();
    fs::write(temp.root().join(STATE_FILE_NAME), b"not-json\n").expect("corrupt state");
    let reads = AtomicUsize::new(0);
    let error = coordinator
        .execute(
            PRICE_PATH,
            PRICE_TR,
            &issuer,
            bounded(),
            Duration::from_secs(3),
            |_| async {
                reads.fetch_add(1, Ordering::SeqCst);
                ReadCallbackResult::Success(())
            },
        )
        .await
        .expect_err("corrupt state denies before callbacks");
    assert_eq!(error, ReadCoordinationError::CorruptState);
    assert_eq!(issuer.calls(), issue_calls);
    assert_eq!(reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn stable_lock_inode_is_never_replaced_by_atomic_state_commits() {
    let temp = PrivateTemp::new("stable-lock-inode");
    let clock = SettableClock::at(12_750_000);
    let issuer = CountingIssuer::new(clock.clone());
    let coordinator = temp.coordinator(Arc::new(clock.clone()), 1);
    prime_token(&coordinator, &issuer)
        .await
        .expect("initialize");
    let before = fs::metadata(temp.root().join(LOCK_FILE_NAME)).expect("lock metadata");
    clock.advance(1_000);
    successful_read(&coordinator, &issuer, DAILY_PATH, DAILY_TR)
        .await
        .expect("state commit");
    let after = fs::metadata(temp.root().join(LOCK_FILE_NAME)).expect("lock metadata");
    assert_eq!((before.dev(), before.ino()), (after.dev(), after.ino()));
}

#[tokio::test]
async fn higher_generation_retains_broker_cooldown_and_makes_zero_callbacks() {
    let temp = PrivateTemp::new("rotation-cooldown");
    let clock = SettableClock::at(12_875_000);
    let issuer = CountingIssuer::new(clock.clone());
    let first = coordinator_at(
        temp.root().to_path_buf(),
        temp.uid(),
        Arc::new(clock.clone()),
        1,
        "old-key",
        "old-secret",
    );
    let _ = first
        .execute(
            PRICE_PATH,
            PRICE_TR,
            &issuer,
            bounded(),
            Duration::from_secs(3),
            |_| async {
                ReadCallbackResult::<()>::RateLimited {
                    retry_after: Duration::from_secs(30),
                }
            },
        )
        .await;
    let rotated = coordinator_at(
        temp.root().to_path_buf(),
        temp.uid(),
        Arc::new(clock.clone()),
        2,
        "new-key",
        "new-secret",
    );
    let reads = AtomicUsize::new(0);
    let error = rotated
        .execute(
            PRICE_PATH,
            PRICE_TR,
            &issuer,
            bounded(),
            Duration::from_secs(3),
            |_| async {
                reads.fetch_add(1, Ordering::SeqCst);
                ReadCallbackResult::Success(())
            },
        )
        .await
        .expect_err("rotation does not erase cooldown");
    assert_eq!(
        error,
        ReadCoordinationError::BrokerCooldown {
            retry_after_ms: 30_000
        }
    );
    assert_eq!(issuer.calls(), 1);
    assert_eq!(reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn root_state_and_lock_symlinks_and_lock_hardlinks_are_rejected() {
    let temp = PrivateTemp::new("trust-links");
    let clock = SettableClock::at(13_000_000);
    let issuer = CountingIssuer::new(clock.clone());
    let root_link = temp.base().join("root-link");
    symlink(temp.root(), &root_link).expect("root symlink");
    let linked_root = coordinator_at(
        root_link,
        temp.uid(),
        Arc::new(clock.clone()),
        1,
        "key",
        "secret",
    );
    assert!(prime_token(&linked_root, &issuer).await.is_err());

    let lock_target = temp.base().join("lock-target");
    fs::write(&lock_target, b"").expect("lock target");
    fs::set_permissions(&lock_target, fs::Permissions::from_mode(0o600)).expect("target mode");
    symlink(&lock_target, temp.root().join(LOCK_FILE_NAME)).expect("lock symlink");
    let coordinator = temp.coordinator(Arc::new(clock.clone()), 1);
    assert!(prime_token(&coordinator, &issuer).await.is_err());
    fs::remove_file(temp.root().join(LOCK_FILE_NAME)).expect("remove lock symlink");

    prime_token(&coordinator, &issuer)
        .await
        .expect("create regular lock/state");
    let state = temp.root().join(STATE_FILE_NAME);
    let state_backup = temp.base().join("state-backup");
    fs::rename(&state, &state_backup).expect("move state");
    symlink(&state_backup, &state).expect("state symlink");
    assert!(prime_token(&coordinator, &issuer).await.is_err());
    fs::remove_file(&state).expect("remove state symlink");
    fs::rename(&state_backup, &state).expect("restore state");

    let lock_path = temp.root().join(LOCK_FILE_NAME);
    fs::set_permissions(&lock_path, fs::Permissions::from_mode(0o640)).expect("broaden lock");
    assert_eq!(
        prime_token(&coordinator, &issuer)
            .await
            .expect_err("broad lock mode"),
        ReadCoordinationError::UnsafeFile
    );
    fs::set_permissions(&lock_path, fs::Permissions::from_mode(0o600)).expect("restore lock");

    let lock_hardlink = temp.base().join("lock-hardlink");
    fs::hard_link(&lock_path, &lock_hardlink).expect("hardlink lock");
    assert_eq!(
        prime_token(&coordinator, &issuer)
            .await
            .expect_err("hardlinked lock"),
        ReadCoordinationError::UnsafeFile
    );
}

#[tokio::test]
async fn initialized_state_never_recreates_a_missing_stable_lock_inode() {
    let temp = PrivateTemp::new("missing-stable-lock");
    let clock = SettableClock::at(13_500_000);
    let issuer = CountingIssuer::new(clock.clone());
    let coordinator = temp.coordinator(Arc::new(clock), 1);
    prime_token(&coordinator, &issuer)
        .await
        .expect("initialize");
    let calls = issuer.calls();
    fs::remove_file(temp.root().join(LOCK_FILE_NAME)).expect("remove test lock");
    assert_eq!(
        prime_token(&coordinator, &issuer)
            .await
            .expect_err("missing initialized lock fails closed"),
        ReadCoordinationError::UnsafeFile
    );
    assert_eq!(issuer.calls(), calls);
    assert!(!temp.root().join(LOCK_FILE_NAME).exists());
}

#[tokio::test]
async fn initialized_witness_rejects_deleted_state_before_all_callbacks() {
    let temp = PrivateTemp::new("missing-committed-state");
    let clock = SettableClock::at(13_750_000);
    let initial_issuer = CountingIssuer::new(clock.clone());
    let coordinator = temp.coordinator(Arc::new(clock.clone()), 1);
    prime_token(&coordinator, &initial_issuer)
        .await
        .expect("fresh coordinator bootstraps once");
    fs::remove_file(temp.root().join(STATE_FILE_NAME)).expect("delete committed state fixture");

    let restarted = temp.coordinator(Arc::new(clock), 1);
    let denied_issuer = CountingIssuer::new(SettableClock::at(13_750_000));
    let reads = AtomicUsize::new(0);
    assert_eq!(
        prime_token(&restarted, &denied_issuer)
            .await
            .expect_err("initialized witness forbids token-state reset"),
        ReadCoordinationError::MissingCommittedState
    );
    let error = restarted
        .execute(
            PRICE_PATH,
            PRICE_TR,
            &denied_issuer,
            bounded(),
            Duration::from_secs(3),
            |_| async {
                reads.fetch_add(1, Ordering::SeqCst);
                ReadCallbackResult::Success(())
            },
        )
        .await
        .expect_err("missing committed state denies read");
    assert_eq!(error, ReadCoordinationError::MissingCommittedState);
    assert_eq!(denied_issuer.calls(), 0);
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    assert!(!temp.root().join(STATE_FILE_NAME).exists());
}

#[tokio::test]
async fn empty_initialization_witness_bootstraps_once_and_malformed_witness_fails_closed() {
    let temp = PrivateTemp::new("ambiguous-initialization");
    let lock_path = temp.root().join(LOCK_FILE_NAME);
    fs::write(&lock_path, b"").expect("create preexisting empty lock fixture");
    fs::set_permissions(&lock_path, fs::Permissions::from_mode(0o600)).expect("protect lock");
    let clock = SettableClock::at(13_875_000);
    let issuer = CountingIssuer::new(clock.clone());
    let coordinator = temp.coordinator(Arc::new(clock), 1);
    prime_token(&coordinator, &issuer)
        .await
        .expect("empty witness has no committed-state history and bootstraps safely");

    fs::write(&lock_path, b"invalid-witness\n").expect("malform witness fixture");
    assert_eq!(
        prime_token(&coordinator, &issuer)
            .await
            .expect_err("malformed witness is invalid"),
        ReadCoordinationError::InvalidInitializationWitness
    );
    assert_eq!(issuer.calls(), 1, "malformed witness invokes no new issuer");
    assert!(temp.root().join(STATE_FILE_NAME).exists());
}

#[tokio::test]
async fn secret_values_never_render_in_debug_or_errors() {
    let temp = PrivateTemp::new("redaction");
    let key = "WP2A-SENTINEL-APP-KEY";
    let secret = "WP2A-SENTINEL-APP-SECRET";
    let token = "WP2A-SENTINEL-ACCESS-TOKEN";
    let clock = SettableClock::at(14_000_000);
    struct SentinelIssuer {
        clock: SettableClock,
        token: &'static str,
    }
    #[async_trait::async_trait]
    impl TokenIssuer for SentinelIssuer {
        async fn issue(&self) -> Result<AccessToken, KisError> {
            Ok(AccessToken {
                value: Secret::new(self.token.to_owned()),
                expires_at_ms: self.clock.now_ms() + 3_600_000,
            })
        }
    }
    let coordinator = coordinator_at(
        temp.root().to_path_buf(),
        temp.uid(),
        Arc::new(clock.clone()),
        1,
        key,
        secret,
    );
    let issuer = SentinelIssuer { clock, token };
    prime_token(&coordinator, &issuer)
        .await
        .expect("persist sentinel token");
    let rendered = format!("{coordinator:?}");
    assert!(!rendered.contains(key));
    assert!(!rendered.contains(secret));
    assert!(!rendered.contains(token));

    let mismatch = coordinator_at(
        temp.root().to_path_buf(),
        temp.uid(),
        Arc::new(SettableClock::at(14_000_000)),
        1,
        "different-key",
        "different-secret",
    );
    let error = prime_token(&mismatch, &issuer)
        .await
        .expect_err("scope mismatch");
    let rendered = format!("{error:?} {error}");
    for value in [key, secret, token] {
        assert!(!rendered.contains(value));
    }
}

#[tokio::test]
async fn two_real_processes_share_one_token_and_one_read_attempt() {
    let temp = PrivateTemp::new("process-race");
    let issue_count = temp.base().join("issue-count");
    let read_count = temp.base().join("read-count");
    let now = 20_000_000_i64;
    let mut first = spawn_helper(
        "execute",
        temp.root(),
        temp.uid(),
        now,
        &issue_count,
        &read_count,
    );
    let mut second = spawn_helper(
        "execute",
        temp.root(),
        temp.uid(),
        now,
        &issue_count,
        &read_count,
    );
    release_child(&mut first);
    release_child(&mut second);
    let first_output = first.wait_with_output().expect("first child");
    let second_output = second.wait_with_output().expect("second child");
    assert!(first_output.status.success());
    assert!(second_output.status.success());
    let outcomes = [
        String::from_utf8(first_output.stdout).expect("first output"),
        String::from_utf8(second_output.stdout).expect("second output"),
    ];
    assert_eq!(
        outcomes
            .iter()
            .filter(|value| value.contains("WP2A_SUCCESS"))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|value| value.contains("WP2A_DENIED"))
            .count(),
        1
    );
    assert_eq!(line_count(&issue_count), 1);
    assert_eq!(line_count(&read_count), 1);
}

#[tokio::test]
async fn child_restart_after_state_deletion_invokes_no_issuer_or_read_callback() {
    let temp = PrivateTemp::new("process-missing-state");
    let issue_count = temp.base().join("issue-count");
    let read_count = temp.base().join("read-count");
    let now = 20_500_000_i64;
    let mut initializer = spawn_helper(
        "execute",
        temp.root(),
        temp.uid(),
        now,
        &issue_count,
        &read_count,
    );
    release_child(&mut initializer);
    let initial_output = initializer.wait_with_output().expect("initializer child");
    assert!(initial_output.status.success());
    assert_eq!(line_count(&issue_count), 1);
    assert_eq!(line_count(&read_count), 1);

    fs::remove_file(temp.root().join(STATE_FILE_NAME)).expect("delete initialized state fixture");
    let mut restarted = spawn_helper(
        "missing-state",
        temp.root(),
        temp.uid(),
        now,
        &issue_count,
        &read_count,
    );
    release_child(&mut restarted);
    let restarted_output = restarted.wait_with_output().expect("restarted child");
    assert!(restarted_output.status.success());
    assert!(
        String::from_utf8(restarted_output.stdout)
            .expect("child output")
            .contains("WP2A_MISSING_STATE")
    );
    assert_eq!(
        line_count(&issue_count),
        1,
        "issuer callback count unchanged"
    );
    assert_eq!(line_count(&read_count), 1, "read callback count unchanged");
}

#[tokio::test]
async fn process_crash_after_reservation_blocks_until_conservative_debt_expires() {
    let temp = PrivateTemp::new("process-crash-read");
    let issue_count = temp.base().join("issue-count");
    let read_count = temp.base().join("read-count");
    let now = 21_000_000_i64;
    let mut child = spawn_helper(
        "crash-read",
        temp.root(),
        temp.uid(),
        now,
        &issue_count,
        &read_count,
    );
    release_child(&mut child);
    let status = child.wait().expect("crash child");
    assert!(!status.success());
    wait_for_lines(&read_count, 1);

    let clock = SettableClock::at(now);
    let issuer = CountingIssuer::new(clock.clone());
    let coordinator = coordinator_at(
        temp.root().to_path_buf(),
        temp.uid(),
        Arc::new(clock.clone()),
        1,
        "child-app-key",
        "child-app-secret",
    );
    assert_eq!(
        successful_read(&coordinator, &issuer, PRICE_PATH, PRICE_TR).await,
        Err(ReadCoordinationError::ReservationActive {
            retry_after_ms: 3_000
        })
    );
    clock.advance(5_000);
    successful_read(&coordinator, &issuer, PRICE_PATH, PRICE_TR)
        .await
        .expect("conservative fence and spacing elapsed");
    assert_eq!(issuer.calls(), 0, "child token was reusable");
}

#[tokio::test]
async fn process_death_inside_issuer_preserves_issue_cooldown_without_get_debt() {
    let temp = PrivateTemp::new("process-crash-issue");
    let issue_count = temp.base().join("issue-count");
    let read_count = temp.base().join("read-count");
    let now = 22_000_000_i64;
    let mut child = spawn_helper(
        "hang-issue",
        temp.root(),
        temp.uid(),
        now,
        &issue_count,
        &read_count,
    );
    release_child(&mut child);
    wait_for_lines(&issue_count, 1);
    child.kill().expect("kill issuer child");
    let _ = child.wait();

    let clock = SettableClock::at(now);
    let issuer = CountingIssuer::new(clock.clone());
    let coordinator = coordinator_at(
        temp.root().to_path_buf(),
        temp.uid(),
        Arc::new(clock.clone()),
        1,
        "child-app-key",
        "child-app-secret",
    );
    assert_eq!(
        successful_read(&coordinator, &issuer, PRICE_PATH, PRICE_TR).await,
        Err(ReadCoordinationError::TokenIssueCooldown {
            retry_after_ms: 60_000
        })
    );
    assert_eq!(line_count(&read_count), 0, "no GET was reserved or called");
    assert_eq!(issuer.calls(), 0);
}

#[tokio::test]
async fn process_crash_after_token_commit_but_before_read_reservation_reuses_token_without_get_debt()
 {
    let temp = PrivateTemp::new("process-crash-after-token");
    let issue_count = temp.base().join("issue-count");
    let read_count = temp.base().join("read-count");
    let now = 22_500_000_i64;
    let mut child = spawn_helper(
        "crash-after-token",
        temp.root(),
        temp.uid(),
        now,
        &issue_count,
        &read_count,
    );
    release_child(&mut child);
    let status = child.wait().expect("token crash child");
    assert!(!status.success());
    assert_eq!(line_count(&issue_count), 1);
    assert_eq!(line_count(&read_count), 0);

    let clock = SettableClock::at(now);
    let issuer = CountingIssuer::new(clock.clone());
    let coordinator = coordinator_at(
        temp.root().to_path_buf(),
        temp.uid(),
        Arc::new(clock),
        1,
        "child-app-key",
        "child-app-secret",
    );
    successful_read(&coordinator, &issuer, PRICE_PATH, PRICE_TR)
        .await
        .expect("persisted child token is reusable immediately");
    assert_eq!(issuer.calls(), 0);
}

fn spawn_helper(
    action: &str,
    root: &Path,
    uid: u32,
    now_ms: i64,
    issue_count: &Path,
    read_count: &Path,
) -> Child {
    Command::new(std::env::current_exe().expect("test executable"))
        .arg("--exact")
        .arg("read_coordination_process_helper")
        .arg("--nocapture")
        .env("WP2A_CHILD_ACTION", action)
        .env("WP2A_CHILD_ROOT", root)
        .env("WP2A_CHILD_UID", uid.to_string())
        .env("WP2A_CHILD_NOW_MS", now_ms.to_string())
        .env("WP2A_CHILD_ISSUE_COUNT", issue_count)
        .env("WP2A_CHILD_READ_COUNT", read_count)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn helper")
}

fn release_child(child: &mut Child) {
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(b"go\n")
        .expect("release child");
}

fn line_count(path: &Path) -> usize {
    match fs::File::open(path) {
        Ok(file) => BufReader::new(file).lines().count(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(_) => panic!("counter file unavailable"),
    }
}

fn wait_for_lines(path: &Path, wanted: usize) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if line_count(path) >= wanted {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("child did not reach bounded synchronization point");
}

struct FileIssuer {
    clock: SettableClock,
    count_path: PathBuf,
    hang: bool,
}

#[async_trait::async_trait]
impl TokenIssuer for FileIssuer {
    async fn issue(&self) -> Result<AccessToken, KisError> {
        append_counter(&self.count_path);
        if self.hang {
            std::future::pending::<()>().await;
        }
        Ok(AccessToken {
            value: Secret::new("child-fake-token".to_owned()),
            expires_at_ms: self.clock.now_ms() + 3_600_000,
        })
    }
}

fn append_counter(path: &Path) {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("open counter");
    file.write_all(b"1\n").expect("append counter");
    file.sync_all().expect("sync counter");
}

#[test]
fn read_coordination_process_helper() {
    let Ok(action) = std::env::var("WP2A_CHILD_ACTION") else {
        return;
    };
    let root = PathBuf::from(std::env::var_os("WP2A_CHILD_ROOT").expect("root"));
    let uid = std::env::var("WP2A_CHILD_UID")
        .expect("uid")
        .parse::<u32>()
        .expect("numeric uid");
    let now_ms = std::env::var("WP2A_CHILD_NOW_MS")
        .expect("now")
        .parse::<i64>()
        .expect("numeric now");
    let issue_count =
        PathBuf::from(std::env::var_os("WP2A_CHILD_ISSUE_COUNT").expect("issue count"));
    let read_count = PathBuf::from(std::env::var_os("WP2A_CHILD_READ_COUNT").expect("read count"));
    let mut signal = String::new();
    std::io::stdin()
        .read_to_string(&mut signal)
        .expect("read barrier");
    assert_eq!(signal, "go\n");

    let clock = SettableClock::at(now_ms);
    let coordinator = coordinator_at(
        root,
        uid,
        Arc::new(clock.clone()),
        1,
        "child-app-key",
        "child-app-secret",
    );
    let issuer = FileIssuer {
        clock,
        count_path: issue_count,
        hang: action == "hang-issue",
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("runtime");
    if action == "crash-after-token" {
        runtime
            .block_on(coordinator.token(&issuer, bounded()))
            .expect("commit token before crash");
        std::process::exit(86);
    }
    let result = runtime.block_on(coordinator.execute(
        PRICE_PATH,
        PRICE_TR,
        &issuer,
        bounded(),
        Duration::from_secs(3),
        |_| async {
            append_counter(&read_count);
            if action == "crash-read" {
                std::process::exit(86);
            }
            ReadCallbackResult::Success(())
        },
    ));
    if action == "missing-state" {
        match result {
            Err(ReadCoordinationError::MissingCommittedState) => {
                println!("WP2A_MISSING_STATE");
                return;
            }
            _ => panic!("missing committed state was not rejected"),
        }
    }
    match result {
        Ok(()) => println!("WP2A_SUCCESS"),
        Err(
            ReadCoordinationError::GlobalSpacing { .. }
            | ReadCoordinationError::ChannelSpacing { .. }
            | ReadCoordinationError::IntradaySpacing { .. }
            | ReadCoordinationError::ReservationActive { .. },
        ) => println!("WP2A_DENIED"),
        Err(_) => panic!("unexpected typed child result"),
    }
}

#[test]
fn generated_files_are_private_and_state_is_bounded() {
    let temp = PrivateTemp::new("permissions");
    let clock = SettableClock::at(23_000_000);
    let issuer = CountingIssuer::new(clock.clone());
    let coordinator = temp.coordinator(Arc::new(clock), 1);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("runtime");
    runtime
        .block_on(prime_token(&coordinator, &issuer))
        .expect("state");
    for name in [LOCK_FILE_NAME, STATE_FILE_NAME] {
        let metadata = fs::metadata(temp.root().join(name)).expect("metadata");
        assert_eq!(metadata.permissions().mode() & 0o7777, 0o600);
        assert_eq!(metadata.uid(), temp.uid());
        assert_eq!(metadata.nlink(), 1);
        assert!(metadata.len() <= 64 * 1024);
    }
}
