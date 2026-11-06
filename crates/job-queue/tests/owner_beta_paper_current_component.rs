// PRIVATE TEMPLATE ONLY. Future install path (not created in this preparation):
// crates/job-queue/tests/owner_beta_paper_current_component.rs
// No production authority is supplied by the run UUID or stage context.

#[cfg(target_os = "linux")]
mod component {
    use job_queue::owner_beta::paper::source_evidence::{self as wire, ProtectedCurrentBinding};
    use serde_json::{Value, json};
    use std::io::Write;
    use std::os::unix::fs::MetadataExt;
    use std::time::{Duration, Instant};

    const TAG: &[u8] = b"@@R5PC1@@";
    const CURRENT: &str = "/data/owner-beta-paper-evidence/current/current-binding.json";
    const CONTEXT: &str = "/opt/lagrange-paper-component/context.json";

    struct Channel {
        nonce: String,
        outgoing: u64,
        incoming: u64,
        deadline: Instant,
    }
    impl Channel {
        fn send(&mut self, event: &str, data: Value) {
            let frame = json!({"actor":"reader","nonce":self.nonce,"seq":self.outgoing,"event":event,"data":data});
            let raw = wire::canonical(&frame).expect("PROTOCOL_ENCODE");
            assert!(raw.len() < 1024, "PROTOCOL_SIZE");
            let mut out = std::io::stdout().lock();
            out.write_all(TAG).expect("PROTOCOL_WRITE");
            out.write_all(&raw).expect("PROTOCOL_WRITE");
            out.write_all(b"\n").expect("PROTOCOL_WRITE");
            out.flush().expect("PROTOCOL_FLUSH");
            self.outgoing += 1;
        }
        fn receive(&mut self, expected: &str) {
            let mut raw = Vec::new();
            loop {
                let remaining = self
                    .deadline
                    .checked_duration_since(Instant::now())
                    .expect("PROTOCOL_DEADLINE");
                let mut poll = libc::pollfd {
                    fd: 0,
                    events: libc::POLLIN,
                    revents: 0,
                };
                let millis = remaining.as_millis().clamp(1, 1000) as i32;
                let result = unsafe { libc::poll(&mut poll, 1, millis) };
                assert!(result >= 0, "PROTOCOL_POLL");
                if result == 0 {
                    continue;
                }
                assert!(poll.revents & libc::POLLIN != 0, "PROTOCOL_EOF");
                let mut byte = [0];
                // Do not mix fd readiness with Rust's buffered stdin: it can
                // retain bytes while the descriptor is no longer readable.
                let read = unsafe { libc::read(0, byte.as_mut_ptr().cast(), 1) };
                assert!(read == 1, "PROTOCOL_READ");
                if byte[0] == b'\n' {
                    break;
                }
                raw.push(byte[0]);
                assert!(raw.len() <= 1024, "PROTOCOL_SIZE");
            }
            assert!(raw.starts_with(TAG), "PROTOCOL_TAG");
            let body = &raw[TAG.len()..];
            let value: Value = serde_json::from_slice(body).expect("PROTOCOL_JSON");
            assert!(
                wire::canonical(&value).expect("PROTOCOL_CANONICAL") == body,
                "PROTOCOL_CANONICAL"
            );
            let fields = value.as_object().expect("PROTOCOL_OBJECT");
            assert!(
                fields.len() == 5
                    && ["actor", "nonce", "seq", "event", "data"]
                        .iter()
                        .all(|key| fields.contains_key(*key)),
                "PROTOCOL_KEYS"
            );
            assert!(
                value["actor"] == "control"
                    && value["nonce"] == self.nonce
                    && value["event"] == expected,
                "PROTOCOL_IDENTITY"
            );
            assert!(
                value["seq"].as_u64() == Some(self.incoming) && value["data"] == json!({}),
                "PROTOCOL_SEQUENCE"
            );
            self.incoming += 1;
            assert!(self.incoming <= 32, "PROTOCOL_LIMIT");
        }
    }

    fn context() -> String {
        assert!(
            std::env::var("R5_PC_OPT_IN").ok().as_deref() == Some("ISOLATED_SYNTHETIC_CURRENT_V1"),
            "EXPLICIT_OPT_IN_REQUIRED"
        );
        let nonce = std::env::var("R5_PC_RUN_UUID").expect("RUN_UUID_REQUIRED");
        let parsed = uuid::Uuid::parse_str(&nonce).expect("RUN_UUID_INVALID");
        assert!(
            !parsed.is_nil() && parsed.to_string() == nonce,
            "RUN_UUID_INVALID"
        );
        assert!(
            unsafe {
                libc::getuid() == 10001
                    && libc::geteuid() == 10001
                    && libc::getgid() == 10001
                    && libc::getegid() == 10001
            },
            "READER_UID_GID"
        );
        let status = std::fs::read_to_string("/proc/self/status").expect("PROCESS_STATUS");
        let field = |key: &str| {
            status
                .lines()
                .find_map(|line| line.strip_prefix(key))
                .map(str::trim)
        };
        let caps =
            u64::from_str_radix(field("CapEff:").expect("CAPS_REQUIRED"), 16).expect("CAPS_FORMAT");
        assert!(
            caps == 0 && field("NoNewPrivs:") == Some("1"),
            "READER_PRIVILEGES"
        );
        let metadata = std::fs::symlink_metadata(CONTEXT).expect("CONTEXT_METADATA");
        assert!(
            metadata.file_type().is_file()
                && metadata.nlink() == 1
                && metadata.mode() & 0o777 == 0o444
                && metadata.len() <= 32768,
            "CONTEXT_FILE"
        );
        let raw = std::fs::read(CONTEXT).expect("CONTEXT_READ");
        let ctx: Value = serde_json::from_slice(&raw).expect("CONTEXT_JSON");
        assert!(
            wire::canonical(&ctx).expect("CONTEXT_CANONICAL") == raw,
            "CONTEXT_CANONICAL"
        );
        let keys = ctx.as_object().expect("CONTEXT_OBJECT");
        assert!(
            keys.len() == 8
                && [
                    "schema",
                    "run_uuid",
                    "synthetic",
                    "production_eligible",
                    "image",
                    "files",
                    "binary_glibc",
                    "binary_loader"
                ]
                .iter()
                .all(|key| keys.contains_key(*key)),
            "CONTEXT_KEYS"
        );
        assert!(
            ctx["schema"] == "r5-protected-current-component-v1"
                && ctx["run_uuid"] == nonce
                && ctx["synthetic"] == true
                && ctx["production_eligible"] == false,
            "CONTEXT_IDENTITY"
        );
        assert!(
            ctx["image"]
                == "sha256:0033324c5e8ba6413a1a85f2719a79a9ca00c75d72edb69e537949063e963269",
            "CONTEXT_IMAGE"
        );
        // This context is isolation/test sanity only. No alternate current path,
        // supplied clocks or unchecked authority constructor is accepted.
        nonce
    }
    fn observed(current: &mut ProtectedCurrentBinding) -> (String, u64, Value) {
        let binding = current.binding().expect("BINDING_REVALIDATION");
        assert!(
            binding["evidence_root"] == "/data/owner-beta-paper-evidence",
            "FIXED_EVIDENCE_ROOT"
        );
        let snapshot = current.snapshot().expect("SNAPSHOT_REVALIDATION");
        let value = snapshot.value();
        assert!(
            snapshot.status().expect("STATUS") == "CURRENT",
            "CURRENT_REQUIRED"
        );
        let generation = value["generation"].as_str().expect("GENERATION").to_owned();
        let sequence = value["sequence"].as_u64().expect("SEQUENCE");
        let metadata = std::fs::symlink_metadata(CURRENT).expect("CURRENT_METADATA");
        assert!(
            metadata.file_type().is_file()
                && metadata.uid() == 0
                && metadata.gid() == 10001
                && metadata.mode() & 0o777 == 0o440
                && metadata.nlink() == 1,
            "CURRENT_ROOT_440"
        );
        let parent = std::fs::symlink_metadata("/data/owner-beta-paper-evidence/current")
            .expect("CURRENT_PARENT");
        assert!(
            parent.file_type().is_dir()
                && parent.uid() == 0
                && parent.gid() == 10001
                && parent.mode() & 0o777 == 0o750,
            "CURRENT_ROOT_750"
        );
        (
            generation.clone(),
            sequence,
            json!({"generation":generation,"publication_sequence":sequence}),
        )
    }
    fn denied(current: &mut ProtectedCurrentBinding) {
        assert!(current.revalidate().is_err(), "REVALIDATION_MUST_DENY");
        assert!(
            current.binding().is_err() && current.snapshot().is_err(),
            "FAILED_INSTANCE_REVOKED"
        );
    }
    fn new_evaluation(channel: &mut Channel) -> ProtectedCurrentBinding {
        channel.receive("NEW_EVAL");
        let mut next = ProtectedCurrentBinding::open().expect("NEW_EVALUATION_OPEN");
        let (_, _, data) = observed(&mut next);
        channel.send("OPEN", data);
        next
    }
    pub fn run() {
        let nonce = context();
        let mut channel = Channel {
            nonce,
            outgoing: 0,
            incoming: 0,
            deadline: Instant::now() + Duration::from_secs(120),
        };
        channel.send("READY", json!({}));
        channel.receive("OPEN");
        let mut current = ProtectedCurrentBinding::open().expect("PROTECTED_OPEN");
        let (generation, sequence, data) = observed(&mut current);
        channel.send("OPEN", data);
        channel.receive("REFRESH");
        current.revalidate().expect("SAME_GENERATION_REFRESH");
        let (new_generation, new_sequence, data) = observed(&mut current);
        assert!(
            generation == new_generation && new_sequence > sequence,
            "MONOTONE_REFRESH"
        );
        channel.send("REFRESH", data);
        channel.receive("INVALID");
        denied(&mut current);
        channel.send("DENIED", json!({}));
        channel.receive("REVOKED");
        denied(&mut current);
        channel.send("STILL_REVOKED", json!({}));
        current = new_evaluation(&mut channel);
        let (previous_generation, _, _) = observed(&mut current);
        channel.receive("GENERATION");
        denied(&mut current);
        channel.send("DENIED", json!({}));
        current = new_evaluation(&mut channel);
        let (next_generation, _, _) = observed(&mut current);
        assert!(
            previous_generation != next_generation,
            "NEW_GENERATION_REQUIRED"
        );
        channel.receive("CLOSED");
        denied(&mut current);
        channel.send("DENIED", json!({}));
        current = new_evaluation(&mut channel);
        channel.receive("COPY");
        denied(&mut current);
        channel.send("DENIED", json!({}));
        current = new_evaluation(&mut channel);
        channel.receive("CRASH");
        denied(&mut current);
        channel.send("DENIED", json!({}));
        channel.receive("FINISH");
        drop(current);
        channel.send("DONE", json!({}));
    }

    mod raw_context_case {
        use super::*;
        use domain::{BatchId, ContentHash, TradingDate, UtcTimestamp};
        use job_queue::owner_beta::paper::source_evidence::ExecutionGateFailureKind;
        use market_data::checked_close::{Attempt, Report};
        use market_data::{
            contract::{FetchMode, RawEnvelope, RequestMetadata, ResponseKind},
            raw_custody::RawInvocation,
            storage::{
                BatchSpec, CheckedCommittedEvidenceBoundary, CheckedCommittedEvidenceObservation,
                CheckedCommittedEvidenceObservationFailure, CommittedManifestEvidenceRecord,
                CommittedManifestReadLimits, ManifestEntry, RawStore, StoreError,
            },
        };
        use std::str::FromStr;

        const RAW_ROOT: &str = "/data";
        const RAW_UID: u32 = 10001;
        const PROVIDER: &str = "r5-component-fixture";
        const MARKET: &str = "fixture";
        const PAYLOAD: &[u8] = b"{\"synthetic_component_vector\":1}\n";
        const BATCHES: [&str; 2] = [
            "00000000-0000-4000-8000-000000000001",
            "00000000-0000-4000-8000-000000000002",
        ];

        struct Seed {
            store: RawStore,
            entries: Vec<ManifestEntry>,
            manifest: Vec<u8>,
        }

        #[derive(Clone, Copy, Default)]
        struct Counts {
            attempted: usize,
            closed: usize,
            unknown: usize,
            live: usize,
        }

        fn counts(report: &Report) -> Counts {
            Counts {
                attempted: report.attempted(),
                closed: report.closed(),
                unknown: report.unknown(),
                live: report.live(),
            }
        }

        fn resources_closed(value: Counts) -> bool {
            value.attempted > 0
                && value.attempted == value.closed
                && value.unknown == 0
                && value.live == 0
        }

        fn report_clean(report: &Report) -> bool {
            report.first().is_none() && report.secondary().is_empty() && report.omitted() == 0
        }

        fn seed() -> Result<Seed, &'static str> {
            let store = RawStore::new(std::path::Path::new(RAW_ROOT));
            let date = TradingDate::parse("2020-01-02").map_err(|_| "fixture_date")?;
            let retrieved = UtcTimestamp::parse_rfc3339("2020-01-02T06:00:00Z")
                .map_err(|_| "fixture_timestamp")?;
            let mut entries = Vec::new();
            for text in BATCHES {
                let batch_id = BatchId::from_str(text).map_err(|_| "fixture_batch_id")?;
                let envelope = RawEnvelope::new(
                    batch_id,
                    ResponseKind::Bars,
                    "synthetic.json",
                    PAYLOAD.to_vec(),
                    retrieved,
                    RequestMetadata {
                        endpoint: "fixture.local.raw-manifest-component".to_owned(),
                        query: Vec::new(),
                        headers: Vec::new(),
                        mode: FetchMode::Synthetic,
                    },
                );
                let entry = store
                    .store_batch(
                        &BatchSpec {
                            provider: PROVIDER,
                            market: MARKET,
                            date: &date,
                            batch_id,
                            entitlement_reference: None,
                            mode: FetchMode::Synthetic,
                        },
                        &[envelope],
                    )
                    .map_err(|_| "fixture_write")?;
                entries.push(entry);
            }
            let manifest = std::fs::read(store.manifest_path(PROVIDER, MARKET))
                .map_err(|_| "fixture_manifest")?;
            for entry in &entries {
                let files = store
                    .read_batch_bytes(PROVIDER, MARKET, entry)
                    .map_err(|_| "fixture_payload")?;
                if files.len() != 1
                    || files[0].file_name != "synthetic.json"
                    || files[0].bytes != PAYLOAD
                {
                    return Err("fixture_payload_mismatch");
                }
            }
            Ok(Seed {
                store,
                entries,
                manifest,
            })
        }

        fn limits() -> CommittedManifestReadLimits {
            CommittedManifestReadLimits {
                max_manifest_bytes: 32 * 1024,
                max_records: 4,
            }
        }

        fn rows_match(seed: &Seed, rows: &[CommittedManifestEvidenceRecord]) -> bool {
            if !seed.manifest.ends_with(b"\n") || rows.len() != seed.entries.len() {
                return false;
            }
            let lines: Vec<&[u8]> = seed.manifest[..seed.manifest.len() - 1]
                .split(|byte| *byte == b'\n')
                .collect();
            if lines.len() != rows.len() {
                return false;
            }
            rows.iter().zip(&seed.entries).zip(lines).enumerate().all(
                |(index, ((row, entry), line))| {
                    row.entry() == entry
                        && row.line_number() == index + 1
                        && row.line_bytes() == line
                        && row.line_sha256() == &ContentHash::from_bytes(line)
                        && entry.files.len() == 1
                        && entry.files[0].content_hash == ContentHash::from_bytes(PAYLOAD)
                },
            )
        }

        fn observed_rows_match(
            seed: &Seed,
            result: &Result<
                CheckedCommittedEvidenceObservation,
                CheckedCommittedEvidenceObservationFailure<StoreError>,
            >,
        ) -> bool {
            result
                .as_ref()
                .is_ok_and(|observation| rows_match(seed, observation.records()))
        }

        fn current_binding_observation(
            context: &wire::ProtectedExecutionContext<'_, '_>,
        ) -> std::result::Result<bool, wire::ExecutionGateFailure> {
            context.with_binding(|binding| {
                binding["data_root"] == RAW_ROOT
                    && binding["evidence_root"]
                        == CURRENT.trim_end_matches("/current/current-binding.json")
                    && binding["raw_owner_uid"] == RAW_UID
            })
        }

        fn snapshots_match(
            completion: &wire::CheckedCurrentCompletion,
            generation: &str,
            sequence: u64,
        ) -> bool {
            let diagnostics = completion.diagnostics();
            diagnostics.retained_values() > 0
                && (0..diagnostics.retained_values()).all(|index| {
                    diagnostics
                        .retained_snapshot(index)
                        .is_some_and(|snapshot| {
                            snapshot.status().ok() == Some("CURRENT")
                                && snapshot.value()["generation"].as_str() == Some(generation)
                                && snapshot.value()["sequence"].as_u64() == Some(sequence)
                        })
                })
        }

        fn receive_profile(channel: &mut Channel, expected: &str) -> Value {
            let mut raw = Vec::new();
            loop {
                let remaining = channel
                    .deadline
                    .checked_duration_since(Instant::now())
                    .expect("PROFILE_DEADLINE");
                let mut poll = libc::pollfd {
                    fd: 0,
                    events: libc::POLLIN,
                    revents: 0,
                };
                let millis = remaining.as_millis().clamp(1, 1000) as i32;
                let result = unsafe { libc::poll(&mut poll, 1, millis) };
                assert!(result >= 0, "PROFILE_POLL");
                if result == 0 {
                    continue;
                }
                assert!(poll.revents & libc::POLLIN != 0, "PROFILE_EOF");
                let mut byte = [0];
                let read = unsafe { libc::read(0, byte.as_mut_ptr().cast(), 1) };
                assert!(read == 1, "PROFILE_READ");
                if byte[0] == b'\n' {
                    break;
                }
                raw.push(byte[0]);
                assert!(raw.len() <= 1024, "PROFILE_SIZE");
            }
            assert!(raw.starts_with(TAG), "PROFILE_TAG");
            let body = &raw[TAG.len()..];
            let value: Value = serde_json::from_slice(body).expect("PROFILE_JSON");
            assert!(
                wire::canonical(&value).expect("PROFILE_CANONICAL") == body,
                "PROFILE_CANONICAL"
            );
            let fields = value.as_object().expect("PROFILE_OBJECT");
            assert!(
                fields.len() == 5
                    && ["actor", "nonce", "seq", "event", "data"]
                        .iter()
                        .all(|key| fields.contains_key(*key)),
                "PROFILE_KEYS"
            );
            assert!(
                value["actor"] == "control"
                    && value["nonce"] == channel.nonce
                    && value["event"] == expected
                    && value["seq"].as_u64() == Some(channel.incoming),
                "PROFILE_IDENTITY"
            );
            let data = value["data"].clone();
            if matches!(expected, "BOUND_SUCCESS" | "FENCE_CASE") {
                let object = data.as_object().expect("PROFILE_DATA");
                assert!(
                    object.len() == 2
                        && object.contains_key("generation")
                        && object.contains_key("publication_sequence"),
                    "PROFILE_DATA"
                );
                let generation = data["generation"].as_str().expect("PROFILE_GENERATION");
                let parsed = uuid::Uuid::parse_str(generation).expect("PROFILE_GENERATION");
                assert!(
                    !parsed.is_nil() && parsed.to_string() == generation,
                    "PROFILE_GENERATION"
                );
                assert!(
                    data["publication_sequence"]
                        .as_u64()
                        .is_some_and(|n| n < i64::MAX as u64),
                    "PROFILE_SEQUENCE"
                );
            } else {
                assert!(data == json!({}), "PROFILE_DATA");
            }
            channel.incoming += 1;
            assert!(channel.incoming <= 32, "PROFILE_LIMIT");
            data
        }

        fn publication(data: &Value) -> (String, u64) {
            (
                data["generation"]
                    .as_str()
                    .expect("PUBLICATION_GENERATION")
                    .to_owned(),
                data["publication_sequence"]
                    .as_u64()
                    .expect("PUBLICATION_SEQUENCE"),
            )
        }

        fn checked_failure_matches(
            failure: &CheckedCommittedEvidenceObservationFailure<StoreError>,
        ) -> bool {
            let source = failure.source();
            let gate = source.execution_failure();
            source.boundary() == CheckedCommittedEvidenceBoundary::Preflight
                && source.source_report().is_none()
                && source.phase().is_none()
                && gate.is_some_and(|gate| {
                    gate.kind() == ExecutionGateFailureKind::Current
                        && gate.current_failure().is_some_and(|current| {
                            current.diagnostics().source_kind()
                                == Some(wire::CurrentSourceKind::Policy)
                                && current.diagnostics().source_report().is_none()
                        })
                })
        }

        #[expect(
            clippy::drop_non_drop,
            reason = "Consumes invocation loans before same-context and terminal settlement; this is a lifetime boundary, not native close evidence."
        )]
        fn run_bound_success(seed: &Seed, channel: &mut Channel, data: &Value) -> Value {
            let (generation, sequence) = publication(data);
            let attempt = Attempt::default();
            let captured = attempt.clone();
            let bootstrap = RawInvocation::new(&attempt);
            // Return only owned data/errors after consuming every borrowed Current holder.
            let acquisition = {
                let opened =
                    wire::ProtectedCurrentBinding::open_checked_in(&bootstrap, &attempt, &captured);
                opened.map(|binding| {
                    let context = binding.into_execution_context();
                    let binding_observation = current_binding_observation(&context);
                    let admitted = binding_observation.as_ref().is_ok_and(|matches| *matches);
                    let invocation = context.invocation();
                    let checked = if admitted {
                        Some(seed.store.read_committed_manifest_evidence_checked(
                            &invocation,
                            &attempt,
                            PROVIDER,
                            MARKET,
                            RAW_UID,
                            &limits(),
                        ))
                    } else {
                        None
                    };
                    let observed = if checked.as_ref().is_some_and(|value| value.is_ok()) {
                        Some(
                            seed.store
                                .read_committed_manifest_evidence_observed_checked(
                                    &invocation,
                                    &attempt,
                                    PROVIDER,
                                    MARKET,
                                    RAW_UID,
                                    &limits(),
                                ),
                        )
                    } else {
                        None
                    };
                    drop(invocation);
                    let completion = context.finish_local();
                    let snapshot_match = snapshots_match(&completion, &generation, sequence);
                    (
                        checked,
                        observed,
                        completion,
                        binding_observation,
                        snapshot_match,
                    )
                })
            };
            let report = attempt.report();
            let resource_counts = counts(&report);
            drop(bootstrap);
            // Whole acquisition/read/local results stay outside unit T across terminal settlement.
            let terminal = attempt.finish(Ok::<(), std::convert::Infallible>(()));
            let outcome = match &acquisition {
                Ok((checked, observed, completion, binding_observation, snapshot_match)) => json!({
                    "case":"bound_success","binding_matches":binding_observation.as_ref().is_ok_and(|matches| *matches),
                    "checked_rows_match":checked.as_ref().is_some_and(|value| value.as_ref().is_ok_and(|rows| rows_match(seed, rows))),
                    "observed_rows_match":observed.as_ref().is_some_and(|value| observed_rows_match(seed, value)),
                    "current_matches":snapshot_match,
                    "completion_clean":completion.is_clean() && report_clean(&report),
                    "resources_closed":resources_closed(resource_counts),
                    "attempted":resource_counts.attempted,"closed":resource_counts.closed,
                    "unknown":resource_counts.unknown,"live":resource_counts.live,
                    "terminal_ok":terminal.is_ok()
                }),
                Err(failure) => json!({
                    "case":"bound_success","binding_matches":false,
                    "checked_rows_match":false,"observed_rows_match":false,
                    "current_matches":false,"completion_clean":false,
                    "resources_closed":false,
                    "first_report_present":failure.diagnostics().source_report().is_some(),
                    "attempted":resource_counts.attempted,"closed":resource_counts.closed,
                    "unknown":resource_counts.unknown,"live":resource_counts.live,
                    "terminal_failure":terminal.is_err()
                }),
            };
            drop((acquisition, terminal));
            channel.send("RAW_OK", outcome.clone());
            outcome
        }

        #[expect(
            clippy::drop_non_drop,
            reason = "Consumes invocation loans before same-context and terminal settlement; this is a lifetime boundary, not native close evidence."
        )]
        fn run_writer_fence(seed: &Seed, channel: &mut Channel, data: &Value) -> Value {
            let (generation, sequence) = publication(data);
            let attempt = Attempt::default();
            let captured = attempt.clone();
            let bootstrap = RawInvocation::new(&attempt);
            let acquisition = {
                let opened =
                    wire::ProtectedCurrentBinding::open_checked_in(&bootstrap, &attempt, &captured);
                match opened {
                    Ok(binding) => {
                        let context = binding.into_execution_context();
                        let binding_observation = current_binding_observation(&context);
                        let admitted = binding_observation.as_ref().is_ok_and(|matches| *matches);
                        let invocation = context.invocation();
                        let first = if admitted {
                            Some(
                                seed.store
                                    .read_committed_manifest_evidence_observed_checked(
                                        &invocation,
                                        &attempt,
                                        PROVIDER,
                                        MARKET,
                                        RAW_UID,
                                        &limits(),
                                    ),
                            )
                        } else {
                            None
                        };
                        let first_match = first
                            .as_ref()
                            .is_some_and(|value| observed_rows_match(seed, value));
                        drop(invocation);
                        channel.send(
                            "BOUND",
                            json!({
                                "case":"writer_fence_closed","binding_matches":binding_observation.as_ref().is_ok_and(|matches| *matches),
                                "first_read_match":first_match,"generation":generation,
                                "publication_sequence":sequence
                            }),
                        );
                        let _closed = receive_profile(channel, "CLOSED");
                        // Deliberately no with_binding/revalidate between CLOSED and this read.
                        let invocation = context.invocation();
                        let second = if first_match {
                            Some(
                                seed.store
                                    .read_committed_manifest_evidence_observed_checked(
                                        &invocation,
                                        &attempt,
                                        PROVIDER,
                                        MARKET,
                                        RAW_UID,
                                        &limits(),
                                    ),
                            )
                        } else {
                            None
                        };
                        drop(invocation);
                        let refusal = match second.as_ref() {
                            Some(Err(failure)) => (
                                checked_failure_matches(failure),
                                failure.source().source_report().is_some(),
                                failure.source().phase().is_some(),
                            ),
                            _ => (false, false, false),
                        };
                        let completion = context.finish_local();
                        let current_match = snapshots_match(&completion, &generation, sequence);
                        Ok((
                            first,
                            second,
                            completion,
                            binding_observation,
                            first_match,
                            current_match,
                            refusal,
                        ))
                    }
                    Err(failure) => {
                        channel.send(
                            "BOUND",
                            json!({
                                "case":"writer_fence_closed","binding_matches":false,
                                "first_read_match":false,"generation":generation,
                                "publication_sequence":sequence
                            }),
                        );
                        let _closed = receive_profile(channel, "CLOSED");
                        Err(failure)
                    }
                }
            };
            let report = attempt.report();
            let resource_counts = counts(&report);
            drop(bootstrap);
            let terminal = attempt.finish(Ok::<(), std::convert::Infallible>(()));
            let outcome = match &acquisition {
                Ok((
                    _first,
                    _second,
                    completion,
                    binding_observation,
                    first_match,
                    current_match,
                    refusal,
                )) => json!({
                    "case":"writer_fence_closed","binding_matches":binding_observation.as_ref().is_ok_and(|matches| *matches),
                    "first_read_match":first_match,"preflight_current_policy":refusal.0,
                    "source_report_present":refusal.1,"phase_present":refusal.2,
                    "completion_clean":completion.is_clean(),"current_matches":current_match,
                    "resources_closed":resources_closed(resource_counts),
                    "attempted":resource_counts.attempted,"closed":resource_counts.closed,
                    "unknown":resource_counts.unknown,"live":resource_counts.live,
                    "terminal_ok":terminal.is_ok()
                }),
                Err(failure) => json!({
                    "case":"writer_fence_closed","binding_matches":false,
                    "first_read_match":false,"preflight_current_policy":false,
                    "source_report_present":failure.diagnostics().source_report().is_some(),
                    "phase_present":false,"completion_clean":false,"current_matches":false,
                    "resources_closed":false,"attempted":resource_counts.attempted,
                    "closed":resource_counts.closed,"unknown":resource_counts.unknown,
                    "live":resource_counts.live,"terminal_failure":terminal.is_err()
                }),
            };
            drop((acquisition, terminal));
            channel.send("RAW_REFUSED", outcome.clone());
            outcome
        }

        #[expect(
            clippy::drop_non_drop,
            reason = "Consumes invocation loans before same-context and terminal settlement; this is a lifetime boundary, not native close evidence."
        )]
        fn run_unfenced_no_mint(channel: &mut Channel) -> Value {
            let attempt = Attempt::default();
            let captured = attempt.clone();
            let bootstrap = RawInvocation::new(&attempt);
            let acquisition = {
                let opened =
                    wire::ProtectedCurrentBinding::open_checked_in(&bootstrap, &attempt, &captured);
                opened.map(|binding| binding.into_execution_context().finish_local())
            };
            let report = attempt.report();
            let resource_counts = counts(&report);
            drop(bootstrap);
            let terminal = attempt.finish(Ok::<(), std::convert::Infallible>(()));
            let outcome = match &acquisition {
                Err(failure) => json!({
                    "case":"unfenced_no_mint","refused":true,
                    "first_report_present":failure.diagnostics().source_report().is_some(),
                    "raw_calls":0,"terminal_failure":terminal.is_err(),
                    "attempted":resource_counts.attempted,"closed":resource_counts.closed,
                    "unknown":resource_counts.unknown,"live":resource_counts.live
                }),
                Ok(completion) => json!({
                    "case":"unfenced_no_mint","refused":false,
                    "first_report_present":false,"raw_calls":0,
                    "terminal_failure":terminal.is_err(),"completion_clean":completion.is_clean(),
                    "attempted":resource_counts.attempted,"closed":resource_counts.closed,
                    "unknown":resource_counts.unknown,"live":resource_counts.live
                }),
            };
            drop((acquisition, terminal));
            channel.send("NO_CONTEXT", outcome.clone());
            outcome
        }

        pub fn run() {
            let nonce = context();
            let seed = seed().expect("RAW_FIXTURE_SETUP");
            let mut channel = Channel {
                nonce,
                outgoing: 0,
                incoming: 0,
                deadline: Instant::now() + Duration::from_secs(120),
            };
            channel.send("READY", json!({}));
            let first = receive_profile(&mut channel, "BOUND_SUCCESS");
            let bound = run_bound_success(&seed, &mut channel, &first);
            let fence_data = receive_profile(&mut channel, "FENCE_CASE");
            let refusal = run_writer_fence(&seed, &mut channel, &fence_data);
            let _empty = receive_profile(&mut channel, "NO_CONTEXT_CASE");
            let no_context = run_unfenced_no_mint(&mut channel);
            let _finish = receive_profile(&mut channel, "FINISH");
            channel.send("DONE", json!({}));
            assert!(
                bound["binding_matches"] == true
                    && bound["checked_rows_match"] == true
                    && bound["observed_rows_match"] == true
                    && bound["current_matches"] == true
                    && bound["completion_clean"] == true
                    && bound["resources_closed"] == true
                    && bound["terminal_ok"] == true,
                "BOUND_SUCCESS_CASE"
            );
            assert!(
                refusal["binding_matches"] == true
                    && refusal["first_read_match"] == true
                    && refusal["preflight_current_policy"] == true
                    && refusal["source_report_present"] == false
                    && refusal["phase_present"] == false
                    && refusal["current_matches"] == true
                    && refusal["resources_closed"] == true,
                "WRITER_FENCE_CASE"
            );
            assert!(
                no_context["refused"] == true
                    && no_context["raw_calls"] == 0
                    && no_context["terminal_failure"] == true,
                "NO_CONTEXT_CASE"
            );
        }
    }

    pub fn run_raw_context() {
        raw_context_case::run();
    }
}

#[test]
#[ignore = "requires separately authorized isolated root publisher and explicit component context"]
fn protected_current_component() {
    #[cfg(target_os = "linux")]
    component::run();
    #[cfg(not(target_os = "linux"))]
    panic!("LINUX_COMPONENT_REQUIRED");
}

#[test]
#[ignore = "requires separately authorized isolated Current publisher and synthetic Raw context"]
fn protected_current_raw_context_component() {
    #[cfg(target_os = "linux")]
    component::run_raw_context();
    #[cfg(not(target_os = "linux"))]
    panic!("LINUX_COMPONENT_REQUIRED");
}
