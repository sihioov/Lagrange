use std::fs;

#[test]
fn generic_pending_and_valuation_scans_exclude_configured_beta_account() {
    let pending = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/repos/pending_targets.rs"
    ))
    .expect("pending target source");
    let runner = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/paper_runner.rs"))
        .expect("runner source");

    assert!(pending.contains("NOT public.paper_0067_is_beta_account(pending_targets.account_id)"));
    assert!(runner.contains("NOT public.paper_0067_is_beta_account(a.id)"));
}

#[test]
fn paper_runner_reads_only_existing_strict_beta_mode_settings() {
    let source = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/bin/paper-runner.rs"
    ))
    .expect("paper runner binary source");
    let runtime = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/runtime.rs"))
        .expect("runtime source");

    assert!(source.contains("load_owner_beta_process_modes()"));
    assert!(runtime.contains("OWNER_BETA_ACCESS_MODE_FILE"));
    assert!(runtime.contains("OWNER_BETA_PAPER_MODE_FILE"));
    assert!(runtime.contains("owner_beta_process_modes_from"));
}

#[test]
fn beta_cycle_settles_due_targets_before_queueing_or_claiming_preview() {
    let source = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/paper_session/owner_beta.rs"
    ))
    .expect("owner-beta worker source");
    let settle = source
        .find("self.run_targets_once(context).await")
        .expect("due-target pass");
    let retry_auto = source
        .find("self.retry_ready_auto_preview_once(context).await?")
        .expect("saved AUTO retry pass");
    let queue = source
        .find("QueueSignalConsumer {")
        .expect("checked current T AUTO queue");
    let queued = source
        .find("finish_checked_action(initial)?;")
        .expect("checked AUTO queue completes before preview claim");
    let consumer = source
        .split_once("for QueueSignalConsumer<'c, 's, 'attempt>")
        .expect("current T queue consumer")
        .1;
    assert!(consumer.contains("SELECT queue_owner_beta_paper_target_v1($1,$2,$3)"));
    let claim = source
        .find("claim_next_paper_for_owner(&self.worker_id, owner)")
        .expect("owner-scoped preview claim");

    assert!(settle < retry_auto && retry_auto < queue && queue < queued && queued < claim);
    assert!(source.contains("has_due_unvalued_target(&scan)"));
}
