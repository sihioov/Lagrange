// Isolated diagnostic regression variant: complete authority reads before OPEN ACK.
// The original ignored test and accepted product inputs remain preserved.
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
    fn denied(current: &mut ProtectedCurrentBinding, channel: &mut Channel) {
        if current.revalidate().is_ok() {
            channel.send("FAIL_REVALIDATION", json!({}));
            panic!("REVALIDATION_MUST_DENY");
        }
        if !(current.binding().is_err() && current.snapshot().is_err()) {
            channel.send("FAIL_STICKY_REVOKE", json!({}));
            panic!("FAILED_INSTANCE_REVOKED");
        }
    }
    fn new_evaluation(
        channel: &mut Channel,
        previous_generation: Option<&str>,
    ) -> (ProtectedCurrentBinding, String) {
        channel.receive("NEW_EVAL");
        let mut next = ProtectedCurrentBinding::open().expect("NEW_EVALUATION_OPEN");
        let (generation, _, data) = observed(&mut next);
        if previous_generation.is_some_and(|previous| previous == generation.as_str()) {
            channel.send("FAIL_GENERATION", json!({}));
            panic!("NEW_GENERATION_REQUIRED");
        }
        // OPEN permits the host to mutate authority immediately. All reads and
        // generation assertions for this phase must finish before the ACK.
        channel.send("OPEN", data);
        (next, generation)
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
        denied(&mut current, &mut channel);
        channel.send("DENIED", json!({}));
        channel.receive("REVOKED");
        denied(&mut current, &mut channel);
        channel.send("STILL_REVOKED", json!({}));
        let (replacement, previous_generation) = new_evaluation(&mut channel, None);
        current = replacement;
        channel.receive("GENERATION");
        denied(&mut current, &mut channel);
        channel.send("DENIED", json!({}));
        current = new_evaluation(&mut channel, Some(&previous_generation)).0;
        channel.receive("CLOSED");
        denied(&mut current, &mut channel);
        channel.send("DENIED", json!({}));
        current = new_evaluation(&mut channel, None).0;
        channel.receive("COPY");
        denied(&mut current, &mut channel);
        channel.send("DENIED", json!({}));
        current = new_evaluation(&mut channel, None).0;
        channel.receive("CRASH");
        denied(&mut current, &mut channel);
        channel.send("DENIED", json!({}));
        channel.receive("FINISH");
        drop(current);
        channel.send("DONE", json!({}));
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
