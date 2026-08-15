use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use job_queue::recommendation::child::{
    TargetChildError, TargetChildPaths, TargetChildRequest, TargetProvenance, run_target_child,
};
use job_queue::types::ErrorClass;
use serde_json::json;
use tempfile::TempDir;
use uuid::Uuid;

const MEMBERS: [&str; 11] = [
    "069500.KRX",
    "102110.KRX",
    "229200.KRX",
    "143850.KRX",
    "133690.KRX",
    "195930.KRX",
    "192090.KRX",
    "148070.KRX",
    "114260.KRX",
    "153130.KRX",
    "132030.KRX",
];

fn request(strategy_id: &str) -> TargetChildRequest {
    let mut factors = BTreeMap::new();
    for member in MEMBERS {
        factors.insert(member.to_owned(), BTreeMap::new());
    }
    TargetChildRequest {
        strategy_id: strategy_id.to_owned(),
        strategy_version: "1.0.0".to_owned(),
        parameters: json!({
            "benchmark_instrument": "069500.KRX",
            "target_weight": 1.0,
            "rebalance_cadence": "none"
        }),
        as_of: "2020-12-30".to_owned(),
        universe: MEMBERS.into_iter().map(str::to_owned).collect(),
        factors,
        provenance: TargetProvenance {
            dataset_version_id: Uuid::parse_str("123e4567-e89b-42d3-a456-426614174000").unwrap(),
            dataset_id: "krx_eod_bars".to_owned(),
            dataset_version: "phase0-v2".to_owned(),
            curated_version: 2,
            dataset_manifest_sha256: "c".repeat(64),
            universe_snapshot_id: format!("sha256:{}", "a".repeat(64)),
            factor_snapshot_hash: format!("sha256:{}", "b".repeat(64)),
        },
    }
}

fn uv_bin() -> Option<PathBuf> {
    let output = if cfg!(windows) {
        Command::new("where.exe").arg("uv.exe").output().ok()?
    } else {
        Command::new("which").arg("uv").output().ok()?
    };
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()?
        .lines()
        .next()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

#[tokio::test]
async fn real_uv_child_generates_a_strict_result_and_cleans_job_files() {
    let Some(uv) = uv_bin() else {
        eprintln!("skipping: uv unavailable");
        return;
    };
    let scratch = tempfile::tempdir().expect("temp root");
    let paths = TargetChildPaths {
        uv_bin: uv,
        repo_root: repo_root(),
        temp_root: scratch.path().to_path_buf(),
    };
    let job_id = Uuid::new_v4();
    let result = run_target_child(
        &paths,
        job_id,
        &request("buy_and_hold"),
        Duration::from_secs(30),
    )
    .await
    .expect("real child succeeds");
    assert_eq!(result.strategy_version, "buy_and_hold@1.0.0");
    assert_eq!(result.as_of, "2020-12-30");
    assert_eq!(
        result.dataset_version_id,
        Uuid::parse_str("123e4567-e89b-42d3-a456-426614174000").unwrap()
    );
    assert_eq!(result.dataset_version, "phase0-v2");
    assert_eq!(result.curated_version, 2);
    assert_eq!(result.dataset_manifest_sha256, "c".repeat(64));
    assert_eq!(result.targets.len(), 1);
    assert!(scratch.path().read_dir().unwrap().next().is_none());
}

fn fake_project(script_body: &str) -> (TempDir, PathBuf) {
    let root = tempfile::tempdir().expect("fake repo");
    let package = root.path().join("nt/strategies");
    fs::create_dir_all(&package).expect("package directory");
    fs::write(
        root.path().join("nt/pyproject.toml"),
        "[project]\nname='fake-child'\nversion='0.0.0'\nrequires-python='>=3.12'\n",
    )
    .expect("pyproject");
    fs::write(package.join("__init__.py"), "").expect("init");
    fs::write(package.join("recommendation_cli.py"), script_body).expect("fake child");
    let uv = uv_bin().expect("uv is required for child integration tests");
    (root, uv)
}

fn paths_for(root: &TempDir, uv: PathBuf, scratch: &TempDir) -> TargetChildPaths {
    TargetChildPaths {
        uv_bin: uv,
        repo_root: root.path().to_path_buf(),
        temp_root: scratch.path().to_path_buf(),
    }
}

#[tokio::test]
async fn timeout_kills_and_reaps_the_child_then_cleans_files() {
    let (root, uv) = fake_project("import time\ntime.sleep(5)\n");
    let scratch = tempfile::tempdir().unwrap();
    let error = run_target_child(
        &paths_for(&root, uv, &scratch),
        Uuid::new_v4(),
        &request("buy_and_hold"),
        Duration::from_millis(300),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), "TARGET_CHILD_TIMEOUT");
    assert_eq!(error.class(), ErrorClass::Transient);
    assert!(scratch.path().read_dir().unwrap().next().is_none());
}

#[tokio::test]
async fn descendant_holding_inherited_stderr_cannot_outlive_operation_deadline() {
    let script = r#"
import pathlib, subprocess, sys
sentinel = pathlib.Path(__file__).resolve().parents[2] / 'descendant-late-sentinel'
code = "import pathlib, sys, time; time.sleep(2); pathlib.Path(sys.argv[1]).write_text('escaped')"
subprocess.Popen([sys.executable, '-c', code, str(sentinel)], stderr=sys.stderr)
"#;
    let (root, uv) = fake_project(script);
    let sentinel = root.path().join("descendant-late-sentinel");
    let scratch = tempfile::tempdir().unwrap();
    let started = Instant::now();
    let error = run_target_child(
        &paths_for(&root, uv, &scratch),
        Uuid::new_v4(),
        &request("buy_and_hold"),
        Duration::from_millis(750),
    )
    .await
    .unwrap_err();
    #[cfg(windows)]
    assert_eq!(error.code(), "TARGET_CHILD_TIMEOUT");
    // Unix deliberately terminates the retained PGID before reaping its
    // exited leader, so EOF arrives early and the missing result is reported.
    #[cfg(unix)]
    assert_eq!(error.code(), "TARGET_CHILD_NO_RESULT");
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "stderr descendant held the call for {:?}",
        started.elapsed()
    );
    assert!(scratch.path().read_dir().unwrap().next().is_none());
    tokio::time::sleep(Duration::from_millis(1_750)).await;
    assert!(
        !sentinel.exists(),
        "descendant survived its exited parent and wrote a late sentinel"
    );
}

#[tokio::test]
async fn descendant_with_closed_stdio_is_terminated_before_result_reading() {
    let script = r#"
import pathlib, subprocess, sys
sentinel = pathlib.Path(__file__).resolve().parents[2] / 'closed-stdio-late-sentinel'
code = "import pathlib, sys, time; time.sleep(2); pathlib.Path(sys.argv[1]).write_text('escaped')"
subprocess.Popen(
    [sys.executable, '-c', code, str(sentinel)],
    stdin=subprocess.DEVNULL,
    stdout=subprocess.DEVNULL,
    stderr=subprocess.DEVNULL,
    close_fds=True,
)
"#;
    let (root, uv) = fake_project(script);
    let sentinel = root.path().join("closed-stdio-late-sentinel");
    let scratch = tempfile::tempdir().unwrap();
    let error = run_target_child(
        &paths_for(&root, uv, &scratch),
        Uuid::new_v4(),
        &request("buy_and_hold"),
        Duration::from_secs(10),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), "TARGET_CHILD_NO_RESULT");
    tokio::time::sleep(Duration::from_millis(2_250)).await;
    assert!(
        !sentinel.exists(),
        "closed-stdio descendant survived successful direct-child reap"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn child_status_symlink_is_rejected_without_following_it() {
    let script = r#"
import argparse, os
p=argparse.ArgumentParser();p.add_argument('--request');p.add_argument('--result');p.add_argument('--status');a=p.parse_args()
os.symlink('/dev/zero', a.status)
raise SystemExit(1)
"#;
    let (root, uv) = fake_project(script);
    let scratch = tempfile::tempdir().unwrap();
    let error = run_target_child(
        &paths_for(&root, uv, &scratch),
        Uuid::new_v4(),
        &request("buy_and_hold"),
        Duration::from_secs(10),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), "TARGET_CHILD_INVALID_STATUS");
    assert_eq!(error.class(), ErrorClass::Integrity);
}

#[tokio::test]
async fn scratch_cleanup_failure_overrides_the_lifecycle_error() {
    let script = r#"
import argparse, pathlib
p=argparse.ArgumentParser();p.add_argument('--request');p.add_argument('--result');p.add_argument('--status');a=p.parse_args()
owned = pathlib.Path(a.request).parent
owned.rename(owned.with_name(owned.name + '-moved'))
"#;
    let (root, uv) = fake_project(script);
    let scratch = tempfile::tempdir().unwrap();
    let error = run_target_child(
        &paths_for(&root, uv, &scratch),
        Uuid::new_v4(),
        &request("buy_and_hold"),
        Duration::from_secs(10),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), "TARGET_CHILD_CLEANUP_FAILED");
    assert_eq!(error.class(), ErrorClass::Transient);
}

#[tokio::test]
async fn oversized_result_status_and_stderr_are_bounded_and_classified() {
    let cases = [
        (
            "import argparse\np=argparse.ArgumentParser();p.add_argument('--request');p.add_argument('--result');p.add_argument('--status');a=p.parse_args();open(a.result,'wb').write(b'x'*(1024*1024+1))\n",
            "TARGET_CHILD_RESULT_TOO_LARGE",
            ErrorClass::Integrity,
        ),
        (
            "import argparse\np=argparse.ArgumentParser();p.add_argument('--request');p.add_argument('--result');p.add_argument('--status');a=p.parse_args();open(a.status,'wb').write(b'x'*(16*1024+1));raise SystemExit(1)\n",
            "TARGET_CHILD_STATUS_TOO_LARGE",
            ErrorClass::Integrity,
        ),
        (
            "import sys\nsys.stderr.write('secret-token-'+'x'*100000);raise SystemExit(2)\n",
            "TARGET_CHILD_EXITED",
            ErrorClass::Transient,
        ),
    ];
    for (script, code, class) in cases {
        let (root, uv) = fake_project(script);
        let scratch = tempfile::tempdir().unwrap();
        let error = run_target_child(
            &paths_for(&root, uv, &scratch),
            Uuid::new_v4(),
            &request("buy_and_hold"),
            Duration::from_secs(20),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code(), code);
        assert_eq!(error.class(), class);
        assert!(error.safe_summary().len() <= 512);
        assert!(!error.safe_summary().contains("secret-token"));
    }
}

#[tokio::test]
async fn nonzero_status_is_stable_and_malformed_or_missing_results_fail_closed() {
    let cases = [
        (
            "import argparse,json\np=argparse.ArgumentParser();p.add_argument('--request');p.add_argument('--result');p.add_argument('--status');a=p.parse_args();json.dump({'code':'TARGET_GENERATION_FAILED','summary':'safe'},open(a.status,'w'));raise SystemExit(1)\n",
            "TARGET_GENERATION_FAILED",
            ErrorClass::Input,
        ),
        (
            "import argparse,json\np=argparse.ArgumentParser();p.add_argument('--request');p.add_argument('--result');p.add_argument('--status');a=p.parse_args();json.dump({'code':'TARGET_GENERATOR_INTERNAL','summary':'safe'},open(a.status,'w'));raise SystemExit(1)\n",
            "TARGET_GENERATOR_INTERNAL",
            ErrorClass::Integrity,
        ),
        (
            "import argparse\np=argparse.ArgumentParser();p.add_argument('--request');p.add_argument('--result');p.add_argument('--status');a=p.parse_args();open(a.result,'w').write('{bad')\n",
            "TARGET_CHILD_INVALID_RESULT",
            ErrorClass::Integrity,
        ),
        ("pass\n", "TARGET_CHILD_NO_RESULT", ErrorClass::Transient),
    ];
    for (script, code, class) in cases {
        let (root, uv) = fake_project(script);
        let scratch = tempfile::tempdir().unwrap();
        let error = run_target_child(
            &paths_for(&root, uv, &scratch),
            Uuid::new_v4(),
            &request("buy_and_hold"),
            Duration::from_secs(20),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code(), code);
        assert_eq!(error.class(), class);
    }
}

#[tokio::test]
async fn relative_or_symlinked_execution_paths_are_rejected_before_launch() {
    let scratch = tempfile::tempdir().unwrap();
    let relative = TargetChildPaths {
        uv_bin: PathBuf::from("uv"),
        repo_root: repo_root(),
        temp_root: scratch.path().to_path_buf(),
    };
    let error = run_target_child(
        &relative,
        Uuid::new_v4(),
        &request("buy_and_hold"),
        Duration::from_secs(1),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), "TARGET_CHILD_UNSAFE_PATH");
    assert_eq!(error.class(), ErrorClass::Integrity);
}

#[tokio::test]
async fn different_jobs_can_run_concurrently() {
    let Some(uv) = uv_bin() else { return };
    let scratch = tempfile::tempdir().unwrap();
    let paths = TargetChildPaths {
        uv_bin: uv,
        repo_root: repo_root(),
        temp_root: scratch.path().to_path_buf(),
    };
    let first_request = request("buy_and_hold");
    let second_request = request("buy_and_hold");
    let first = run_target_child(
        &paths,
        Uuid::new_v4(),
        &first_request,
        Duration::from_secs(30),
    );
    let second = run_target_child(
        &paths,
        Uuid::new_v4(),
        &second_request,
        Duration::from_secs(30),
    );
    let (first, second) = tokio::join!(first, second);
    assert!(first.is_ok(), "{first:?}");
    assert!(second.is_ok(), "{second:?}");
}

#[tokio::test]
async fn concurrent_invocations_of_the_same_job_are_isolated() {
    let Some(uv) = uv_bin() else { return };
    let scratch = tempfile::tempdir().unwrap();
    let paths = TargetChildPaths {
        uv_bin: uv,
        repo_root: repo_root(),
        temp_root: scratch.path().to_path_buf(),
    };
    let job_id = Uuid::new_v4();
    let first_request = request("buy_and_hold");
    let second_request = request("buy_and_hold");
    let first = run_target_child(&paths, job_id, &first_request, Duration::from_secs(30));
    let second = run_target_child(&paths, job_id, &second_request, Duration::from_secs(30));
    let (first, second) = tokio::join!(first, second);
    assert!(first.is_ok(), "{first:?}");
    assert!(second.is_ok(), "{second:?}");
    assert!(scratch.path().read_dir().unwrap().next().is_none());
}

#[tokio::test]
async fn stale_job_directory_does_not_block_retry_and_is_not_deleted() {
    let Some(uv) = uv_bin() else { return };
    let scratch = tempfile::tempdir().unwrap();
    let job_id = Uuid::new_v4();
    let legacy_stale = scratch
        .path()
        .join(format!("recommendation-{}", job_id.simple()));
    fs::create_dir(&legacy_stale).unwrap();
    fs::write(legacy_stale.join("request.json"), b"legacy stale secret").unwrap();
    let stale_invocation = scratch.path().join(format!(
        "recommendation-{}-invocation-{}",
        job_id.simple(),
        Uuid::new_v4().simple()
    ));
    fs::create_dir(&stale_invocation).unwrap();
    fs::write(stale_invocation.join("request.json"), b"stale secret").unwrap();
    let paths = TargetChildPaths {
        uv_bin: uv,
        repo_root: repo_root(),
        temp_root: scratch.path().to_path_buf(),
    };
    let result = run_target_child(
        &paths,
        job_id,
        &request("buy_and_hold"),
        Duration::from_secs(30),
    )
    .await
    .expect("retry uses a fresh invocation directory");
    assert_eq!(result.strategy_version, "buy_and_hold@1.0.0");
    assert_eq!(
        fs::read(legacy_stale.join("request.json")).unwrap(),
        b"legacy stale secret"
    );
    assert_eq!(
        fs::read(stale_invocation.join("request.json")).unwrap(),
        b"stale secret"
    );
    assert_eq!(scratch.path().read_dir().unwrap().count(), 2);
}

#[tokio::test]
async fn aborting_caller_does_not_cancel_termination_or_owned_scratch_cleanup() {
    tokio::time::timeout(Duration::from_secs(15), async {
        let script = r#"
import pathlib, time
root = pathlib.Path(__file__).resolve().parents[2]
(root / 'child-started').write_text('started')
time.sleep(5)
(root / 'late-sentinel').write_text('escaped')
"#;
        let (root, uv) = fake_project(script);
        fs::create_dir(root.path().join("prewarm-home")).unwrap();
        fs::create_dir(root.path().join("prewarm-cache")).unwrap();
        let prewarm = Command::new(&uv)
            .arg("run")
            .arg("--project")
            .arg(root.path().join("nt"))
            .arg("--no-sync")
            .arg("python")
            .arg("-c")
            .arg("pass")
            .env("HOME", root.path().join("prewarm-home"))
            .env("UV_CACHE_DIR", root.path().join("prewarm-cache"))
            .status()
            .expect("prewarm fake project");
        assert!(prewarm.success(), "fake project prewarm succeeds");
        let scratch = tempfile::tempdir().unwrap();
        let stale = scratch.path().join("unrelated-stale");
        fs::create_dir(&stale).unwrap();
        fs::write(stale.join("keep"), b"unrelated").unwrap();
        let paths = paths_for(&root, uv, &scratch);
        let child_request = request("buy_and_hold");
        let caller = tokio::spawn(async move {
            run_target_child(
                &paths,
                Uuid::new_v4(),
                &child_request,
                Duration::from_secs(3),
            )
            .await
        });

        // Wait for an explicit child-side start signal. A cold `uv` startup can
        // legitimately take several seconds on Windows CI.
        wait_until(Duration::from_secs(4), || {
            root.path().join("child-started").exists()
        })
        .await;
        let invocation = scratch
            .path()
            .read_dir()
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| path != &stale)
            .expect("owned invocation directory exists");

        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());
        wait_until(Duration::from_secs(5), || !invocation.exists()).await;
        // If cancellation had killed the owner task, the child would survive
        // long enough to write this five-second sentinel.
        tokio::time::sleep(Duration::from_millis(5_250)).await;

        assert!(!root.path().join("late-sentinel").exists());
        assert_eq!(fs::read(stale.join("keep")).unwrap(), b"unrelated");
        assert_eq!(scratch.path().read_dir().unwrap().count(), 1);
    })
    .await
    .expect("cancellation cleanup exceeded strict test timeout");
}

async fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(timeout, async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("condition was not reached before timeout");
}

#[tokio::test]
async fn request_size_is_bounded_before_launch() {
    let Some(uv) = uv_bin() else { return };
    let scratch = tempfile::tempdir().unwrap();
    let paths = TargetChildPaths {
        uv_bin: uv,
        repo_root: repo_root(),
        temp_root: scratch.path().to_path_buf(),
    };
    let mut oversized = request("buy_and_hold");
    oversized.parameters = json!({"padding": "x".repeat(1024 * 1024)});
    let error = run_target_child(&paths, Uuid::new_v4(), &oversized, Duration::from_secs(1))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "TARGET_CHILD_REQUEST_TOO_LARGE");
    assert_eq!(error.class(), ErrorClass::Input);
}

#[tokio::test]
async fn nonfinite_rust_factor_is_rejected_before_launch() {
    let Some(uv) = uv_bin() else { return };
    let scratch = tempfile::tempdir().unwrap();
    let paths = TargetChildPaths {
        uv_bin: uv,
        repo_root: repo_root(),
        temp_root: scratch.path().to_path_buf(),
    };
    let mut invalid = request("buy_and_hold");
    invalid
        .factors
        .get_mut("069500.KRX")
        .unwrap()
        .insert("unexpected".to_owned(), Some(f64::NAN));
    let error = run_target_child(&paths, Uuid::new_v4(), &invalid, Duration::from_secs(10))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "TARGET_CHILD_INVALID_REQUEST");
    assert_eq!(error.class(), ErrorClass::Input);
}

#[tokio::test]
async fn child_environment_does_not_inherit_sensitive_parent_variables() {
    let script = r#"
import argparse, json, os
p=argparse.ArgumentParser();p.add_argument('--request');p.add_argument('--result');p.add_argument('--status');a=p.parse_args()
for forbidden in ('DATABASE_URL','AUTH0_CLIENT_SECRET','AWS_SECRET_ACCESS_KEY','USERPROFILE'):
    if forbidden in os.environ:
        json.dump({'code':'CHILD_INTERNAL_ERROR','summary':'unsafe environment'},open(a.status,'w'))
        raise SystemExit(1)
for writable in ('HOME','UV_CACHE_DIR'):
    path = os.environ.get(writable)
    if not path or not os.path.isdir(path):
        json.dump({'code':'CHILD_INTERNAL_ERROR','summary':'missing private runtime directory'},open(a.status,'w'))
        raise SystemExit(1)
    open(os.path.join(path, 'child-write'), 'w').write('ok')
result={
 'as_of':'2020-12-30','strategy_version':'buy_and_hold@1.0.0',
 'universe_snapshot_id':'sha256:'+'a'*64,'factor_snapshot_hash':'sha256:'+'b'*64,
 'dataset_version_id':'123e4567-e89b-42d3-a456-426614174000',
 'dataset_id':'krx_eod_bars','dataset_version':'phase0-v2','curated_version':2,
 'dataset_manifest_sha256':'c'*64,'targets':[],'exclusions':[],
 'cash_weight':1.0,
 'constraints':{'top_n':0,'max_weight':1.0,'cash_floor':0.0,'weight_scale':4,'tolerance':1e-9},
 'portfolio_reasons':[],'portfolio_snapshot_id':'sha256:'+'c'*64}
json.dump(result,open(a.result,'w'))
"#;
    let (root, uv) = fake_project(script);
    let scratch = tempfile::tempdir().unwrap();
    let result = run_target_child(
        &paths_for(&root, uv, &scratch),
        Uuid::new_v4(),
        &request("buy_and_hold"),
        Duration::from_secs(20),
    )
    .await
    .expect("sanitized child succeeds");
    assert_eq!(result.cash_weight, 1.0);
}

#[test]
fn error_type_is_send_sync_and_has_stable_classification() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<TargetChildError>();
    assert_eq!(
        TargetChildError::ScratchCollision.class(),
        ErrorClass::Transient
    );
    assert_eq!(TargetChildError::Termination.class(), ErrorClass::Transient);
    assert_eq!(TargetChildError::OwnerTask.class(), ErrorClass::Transient);
    assert_eq!(
        TargetChildError::ChildStatus {
            code: "RESULT_TOO_LARGE".to_owned(),
        }
        .class(),
        ErrorClass::Integrity
    );
    assert_eq!(
        TargetChildError::ChildStatus {
            code: "CHILD_INTERNAL_ERROR".to_owned(),
        }
        .class(),
        ErrorClass::Transient
    );
}
