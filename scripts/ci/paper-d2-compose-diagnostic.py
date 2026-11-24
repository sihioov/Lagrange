"""Disposable hosted D2 consumer probe, without a Rust build or acceptance receipt."""
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import sys
import time
import types


SERVICE = "recommendation-runner"
EXPECTED_GUARD = "release-build-layout: verified artifact bundle paths are invalid"
ERROR_WITNESSES = (
    ("DISK_FULL", ("no space left on device", "disk quota exceeded")),
    ("CONTEXT_MISSING", ("unable to prepare context", "failed to walk", "failed to calculate checksum")),
    ("DAEMON_UNAVAILABLE", ("cannot connect to the docker daemon", "is the docker daemon running")),
    ("BUILDER_UNAVAILABLE", ("no builder", "no nodes available", "failed to find driver")),
    ("PERMISSION_DENIED", ("permission denied", "operation not permitted")),
    ("CONFIGURATION", ("invalid interpolation format", "invalid compose project", "unknown flag",
                       "failed to load bake definition", "invalid build context")),
    ("NETWORK", ("tls handshake timeout", "no such host", "i/o timeout", "connection refused",
                 "context deadline exceeded", "connection reset by peer", "toomanyrequests")),
)


def fixture_module(source):
    path = source / "scripts/ops/build-paper-fixture-images.sh"
    _, _, embedded = path.read_text(encoding="utf-8").partition("<<'PY'\n")
    body, terminator, tail = embedded.rpartition("\nPY\n")
    if not body or not terminator or tail:
        raise ValueError("FIXTURE_EMBEDDING")
    fixture = types.ModuleType("d2_probe_fixture")
    sys.modules[fixture.__name__] = fixture
    exec(compile(body, str(path), "exec"), fixture.__dict__)
    return fixture


def disk_free_mib(path):
    status = os.statvfs(path)
    return (status.f_frsize * status.f_bavail) // 1024**2


def error_label(log):
    if EXPECTED_GUARD in log:
        return "EXPECTED_GUARD_STOP"
    for label, patterns in ERROR_WITNESSES:
        if any(pattern in log.lower() for pattern in patterns):
            return label
    return "UNCLASSIFIED"


def main():
    source, commit, state = Path(sys.argv[1]), sys.argv[2], Path(sys.argv[3])
    fixture = fixture_module(source)
    final_deadline = time.monotonic_ns() + 250_000_000_000
    manifest = state / "never-a-release-manifest"
    fixture.validate_path_arguments(str(source), commit, str(state), str(manifest), final_deadline)
    fixture.assert_clean_source(source, commit)
    env_file, _ = fixture.create_private_state(source, state, manifest, commit)
    monitor = fixture.ResourceMonitor(source, final_deadline)
    builder = "paper-d2-probe-" + secrets.token_hex(16)
    env = fixture.process_environment(state, commit, builder, env_file)
    env.update({
        "RESEARCH_APP_ENV": "prebuild-disabled",
        "RESEARCH_ENTITLEMENT_REFERENCE": "prebuild-disabled",
        "BACKTEST_MIN_FREE_BYTES": "0",
        "BACKTEST_MAX_QUEUED_BACKTESTS": "0",
        "BACKTEST_RECONCILE_GRACE_SECS": "0",
        "BACKTEST_RECONCILE_INTERVAL_SECS": "0",
        "RANGE_RAW_BATCH_ID": "compose-config-disabled",
        "LIVE_NODE_MODE": "disabled",
        "LIVE_NODE_DRY_RUN": "1",
        "BUILDKIT_PROGRESS": "plain",
    })
    print(f"D2_DIAGNOSTIC_DISK_BEGIN docker_mib={disk_free_mib('/var/lib/docker')} state_mib={disk_free_mib(state)}", flush=True)

    def run(argv, step, *, output=False, expected_guard=False):
        start = time.monotonic()
        try:
            result = fixture.run_owned(argv, step, env, state, monitor.build_deadline_ns, output=output)
        except fixture.BuildFailure as error:
            path = state / "logs" / (step + ".log")
            category = error_label(path.read_text(encoding="utf-8", errors="replace")) if path.is_file() else "LOG_UNAVAILABLE"
            code = re.fullmatch(f"command-failed:{step}:(-?[0-9]+)", str(error))
            outcome = code[1] if code else "TIMEOUT" if isinstance(error, fixture.DeadlineExceeded) else "OTHER"
            print(f"D2_DIAGNOSTIC_STEP {step} exit={outcome} class={category} seconds={time.monotonic() - start:.2f}", flush=True)
            if expected_guard and code and category == "EXPECTED_GUARD_STOP":
                return None
            raise ValueError("STEP_" + step.upper().replace("-", "_")) from None
        print(f"D2_DIAGNOSTIC_STEP {step} exit=0 seconds={time.monotonic() - start:.2f}", flush=True)
        if expected_guard:
            raise ValueError("INCOMPLETE_DIAGNOSTIC_CONTEXT_WAS_ACCEPTED")
        return result

    def version(argv, step):
        raw = run(argv, step, output=True)
        match = re.search(r"(?<![0-9])v?([0-9]+\.[0-9]+\.[0-9]+)(?![0-9])", raw)
        print(f"D2_DIAGNOSTIC_VERSION {step}={match[1] if match else 'UNRECOGNIZED'}", flush=True)

    monitor.check("before-d2-diagnostic")
    version(["docker", "compose", "version"], "compose-version")
    version(["docker", "buildx", "version"], "buildx-version")
    version(["docker", "version", "--format", "{{.Server.Version}}"], "daemon-version")
    driver = run(["docker", "info", "--format", "{{.Driver}}"], "daemon-storage", output=True)
    print("D2_DIAGNOSTIC_STORAGE=" + (driver if driver in {"overlay2", "overlayfs", "btrfs", "devicemapper", "vfs", "zfs"} else "OTHER"), flush=True)
    before = run(["docker", "buildx", "ls", "--format", "{{.Name}}"], "before-builder", output=True)
    if builder in {row.strip().removesuffix("*") for row in before.splitlines()}:
        raise ValueError("BUILDER_COLLISION")
    try:
        run(["docker", "buildx", "create", "--name", builder, "--driver", "docker-container"], "create-builder")
        run(["docker", "buildx", "inspect", "--bootstrap", builder], "bootstrap-builder")
        monitor.check("after-d2-builder")
        # Include only the real helper and its correct digests. The absent request is a
        # deliberate negative witness inside the original consumer's named context.
        bundle = state / "diagnostic-context"
        release = bundle / ".release-build"
        release.mkdir(parents=True)
        helper = source / "scripts/ops/lib/release-build-layout.sh"
        shutil.copyfile(helper, release / helper.name)
        helper_sha = hashlib.sha256(helper.read_bytes()).hexdigest()
        bundle_sha = run(
            ["/bin/bash", "-c", 'source "$1"; rbl_tree_hash "$2"', "paper-d2-probe", str(helper), str(bundle)],
            "diagnostic-context-digest", output=True,
        )
        if not re.fullmatch("[0-9a-f]{64}", bundle_sha):
            raise ValueError("DIAGNOSTIC_CONTEXT_DIGEST")
        args = {
            "LAGRANGE_CODE_COMMIT": commit,
            "RUST_ARTIFACT_SOURCE": "verified-artifacts",
            "RUST_ARTIFACT_HELPER_SHA256": helper_sha,
            "RUST_ARTIFACT_BUNDLE_SHA256": bundle_sha,
        }
        override = state / "diagnostic-override.json"
        override.write_text(json.dumps({"services": {SERVICE: {"build": {
            "args": args, "additional_contexts": {"release_artifacts": str(bundle)}
        }}}}), encoding="ascii")
        compose = ["docker", "compose", "--env-file", str(env_file),
                   "--file", str(source / "deploy/compose/compose.yml"), "--file", str(override)]
        model = json.loads(run(compose + ["config", "--format", "json"], "compose-config", output=True))
        service = model["services"][SERVICE]
        build = service["build"]
        if (build["context"] != str(source) or build["dockerfile"] != "crates/job-queue/Dockerfile"
                or not args.items() <= build["args"].items()
                or build["additional_contexts"]["release_artifacts"] != str(bundle)
                or service["image"] != "lagrange-station-recommendation-runner:" + commit):
            raise ValueError("RESOLVED_D2_CONFIG")
        print("D2_DIAGNOSTIC_MODEL=EXACT_DOCKERFILE_AND_VERIFIED_ARTIFACTS_NAMED_CONTEXT", flush=True)
        run(compose + ["build", "--pull=false", SERVICE], "actual-d2-consumer", expected_guard=True)
        monitor.check("after-d2-consumer")
        print(f"D2_DIAGNOSTIC_DISK_END docker_mib={disk_free_mib('/var/lib/docker')} state_mib={disk_free_mib(state)}", flush=True)
    finally:
        fixture.cleanup_builder(builder, env, state, final_deadline)
        print("D2_DIAGNOSTIC_CLEANUP=BUILDER_ABSENCE_VERIFIED", flush=True)
    print("D2_DIAGNOSTIC=GUARD_REACHED_WITH_SYNTHETIC_INCOMPLETE_CONTEXT", flush=True)


if __name__ == "__main__":
    try:
        main()
    except (KeyError, OSError, RuntimeError, ValueError) as error:
        token = str(error)
        reason = token if re.fullmatch("[A-Z0-9_]+", token) else type(error).__name__
        raise SystemExit("D2_DIAGNOSTIC_FAILURE=" + reason) from None
