use std::process::{Command, Output};

const SLOT: &str = "62c6713e-19b7-4bd3-8be6-6a8c5fa3dd3c";
const GENERATION_ENV: &str = "KIS_READ_CREDENTIAL_GENERATION";

fn command(args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_kis-market-stream-state"));
    command.args(args);
    command
}

fn output(args: &[&str], generation: Option<&str>) -> Output {
    let mut command = command(args);
    match generation {
        Some(value) => {
            command.env(GENERATION_ENV, value);
        }
        None => {
            command.env_remove(GENERATION_ENV);
        }
    }
    command.output().unwrap()
}

fn assert_error(result: Output, status: u8, code: &str) {
    assert_eq!(result.status.code(), Some(i32::from(status)));
    assert!(result.stdout.is_empty());
    assert_eq!(
        String::from_utf8(result.stderr).unwrap(),
        format!("{code}\n")
    );
}

#[test]
fn cli_rejects_unknown_duplicate_missing_override_and_noncanonical_inputs() {
    for args in [
        vec![
            "initialize-new",
            "--credential-slot-id",
            SLOT,
            "--credential-generation",
            "17",
            "--path",
            "/tmp",
        ],
        vec![
            "initialize-new",
            "--credential-slot-id",
            SLOT,
            "--credential-generation",
            "17",
            "--state-dir",
            "/tmp",
        ],
        vec![
            "initialize-new",
            "--credential-slot-id",
            SLOT,
            "--credential-generation",
            "17",
            "--owner",
            "10001:10001",
        ],
        vec![
            "initialize-new",
            "--credential-slot-id",
            SLOT,
            "--credential-generation",
            "17",
            "--uid",
            "10001",
        ],
        vec![
            "validate-existing",
            "--credential-slot-id",
            SLOT,
            "--credential-slot-id",
            SLOT,
            "--credential-generation",
            "17",
        ],
        vec!["initialize-new", "--credential-slot-id", SLOT],
        vec![
            "validate-existing",
            "--credential-slot-id",
            SLOT,
            "--credential-generation",
            "017",
        ],
        vec![
            "initialize-new",
            "--credential-slot-id",
            "00000000-0000-0000-0000-000000000000",
            "--credential-generation",
            "17",
        ],
        vec![
            "initialize-new",
            "--credential-slot-id",
            SLOT,
            "--credential-generation",
            "17",
            "--reset",
        ],
        vec![
            "initialize-new",
            "--credential-slot-id",
            SLOT,
            "--credential-generation",
            "17",
            "--url",
            "ws://localhost",
        ],
        vec![
            "initialize-new",
            "--credential-slot-id",
            SLOT,
            "--credential-generation",
            "17",
            "--secret",
            "hidden",
        ],
        vec![
            "initialize-new",
            "--credential-slot-id",
            SLOT,
            "--credential-generation",
            "17",
            "--key",
            "placeholder",
        ],
        vec![
            "initialize-new",
            "--credential-slot-id",
            SLOT,
            "--credential-generation",
            "17",
            "--repair",
        ],
        vec![
            "initialize-new",
            "--credential-slot-id",
            SLOT,
            "--credential-generation",
            "17",
            "--force",
        ],
        vec!["initialize-new", SLOT, "17"],
    ] {
        assert_error(output(&args, Some("17")), 2, "ERR_USAGE");
    }
}

#[test]
fn cli_requires_canonical_environment_generation_and_never_sources_dotenv() {
    let args = [
        "initialize-new",
        "--credential-slot-id",
        SLOT,
        "--credential-generation",
        "17",
    ];
    assert_error(output(&args, None), 4, "ERR_CONFIG");
    assert_error(output(&args, Some("017")), 4, "ERR_CONFIG");
    assert_error(output(&args, Some("18")), 4, "ERR_CONFIG");

    let working = tempfile::tempdir().unwrap();
    std::fs::write(
        working.path().join(".env"),
        "KIS_READ_CREDENTIAL_GENERATION=17\n",
    )
    .unwrap();
    let mut process = command(&args);
    process
        .current_dir(working.path())
        .env_remove(GENERATION_ENV);
    assert_error(process.output().unwrap(), 4, "ERR_CONFIG");
}

#[test]
fn denied_actor_is_reported_before_any_production_path_access() {
    let uid = unsafe { getuid() };
    let euid = unsafe { geteuid() };
    let gid = unsafe { getgid() };
    let egid = unsafe { getegid() };
    if uid == 0 && euid == 0 {
        // Root is permitted; the fixture-backed library tests cover positive
        // creation without touching fixed production paths.
        assert_error(
            output(
                &[
                    "initialize-new",
                    "--credential-slot-id",
                    "00000000-0000-0000-0000-000000000000",
                    "--credential-generation",
                    "17",
                ],
                Some("17"),
            ),
            2,
            "ERR_USAGE",
        );
        return;
    }
    if uid == 10_001 && euid == 10_001 && gid == 10_001 && egid == 10_001 {
        // The runtime identity may validate existing state. Do not probe the
        // fixed host path from this unit test.
        assert_error(
            output(
                &[
                    "validate-existing",
                    "--credential-slot-id",
                    "00000000-0000-0000-0000-000000000000",
                    "--credential-generation",
                    "17",
                ],
                Some("17"),
            ),
            2,
            "ERR_USAGE",
        );
        return;
    }
    assert_error(
        output(
            &[
                "initialize-new",
                "--credential-slot-id",
                SLOT,
                "--credential-generation",
                "17",
            ],
            Some("17"),
        ),
        3,
        "ERR_ACTOR",
    );
    assert_error(
        output(
            &[
                "validate-existing",
                "--credential-slot-id",
                SLOT,
                "--credential-generation",
                "17",
            ],
            Some("17"),
        ),
        3,
        "ERR_ACTOR",
    );
}

unsafe extern "C" {
    fn getuid() -> u32;
    fn geteuid() -> u32;
    fn getgid() -> u32;
    fn getegid() -> u32;
}
