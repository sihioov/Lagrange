"""Run the original D2 producer and first consumer without a release manifest."""
import json
from pathlib import Path
import re
import runpy
import sys
import time


SERVICE = "recommendation-runner"
BINS = frozenset({"recommendation-runner", "candidate-runner"})
COMMON = runpy.run_path(str(Path(__file__).with_name("paper-d2-compose-diagnostic.py")))
COMPOSE_ERROR_LINE = 'chmod 0600 "$log" 2>/dev/null || true; reply_error compose-build-failed'
COMPOSE_WITNESS_LINE = 'rc=$?; printf "\\nPAPER_DIAGNOSTIC_COMPOSE_EXIT %d\\n" "$rc" >>"$log" || true; ' + COMPOSE_ERROR_LINE


class DiagnosticComplete(Exception):
    def __init__(self, image, originals):
        self.image = image
        self.originals = originals


def main():
    source, commit, state = Path(sys.argv[1]), sys.argv[2], Path(sys.argv[3])
    fixture = COMMON["fixture_module"](source)
    if fixture.BASH_SERVER.count(COMPOSE_ERROR_LINE) != 2:
        raise ValueError("UNEXPECTED_FIXTURE_COMPOSE_CALLS")
    # Add only the return status to the existing private log after Compose exits.
    fixture.BASH_SERVER = fixture.BASH_SERVER.replace(COMPOSE_ERROR_LINE, COMPOSE_WITNESS_LINE)
    manifest = state / "never-a-release-manifest"
    deadline = time.monotonic_ns() + 38 * 60_000_000_000

    def resources(stage, backend):
        check = backend.monitor.checks[-1]
        free_mib = COMMON["disk_free_mib"]
        print("D2_GENUINE_RESOURCES " + json.dumps({
            "stage": stage,
            "disk_docker_mib": free_mib("/var/lib/docker"),
            "disk_state_mib": free_mib(state),
            "memory_available_kib": check["MemAvailable"],
            "swap_free_kib": check["SwapFree"],
            "oom_kill_delta": fixture.oom_kill_count() - backend.monitor.oom_baseline,
        }, sort_keys=True), flush=True)

    def witness(backend):
        resources("after-compose-error", backend)
        log = state / "logs" / ("compose-" + SERVICE + ".log")
        code = None
        category = "UNAVAILABLE"
        if log.is_file():
            raw = log.read_bytes()[-65536:].decode("utf-8", errors="replace").rstrip()
            sample, marker, status = raw.rpartition("\nPAPER_DIAGNOSTIC_COMPOSE_EXIT ")
            if marker and re.fullmatch("[1-9][0-9]{0,2}", status) and int(status) <= 255:
                code = int(status)
                category = COMMON["error_label"](sample)
        print(f"D2_GENUINE_COMPOSE exit={code} class={category}", flush=True)

    def only_d2(plan, backend, guard, actual_commit, actual_state, candidate, progress=None, build_only=None):
        entries = [item for item in plan if item.service == SERVICE]
        if len(entries) != 1 or entries[0].recipe != "D2" or frozenset(entries[0].bins) != BINS or build_only is not None:
            raise ValueError("UNEXPECTED_D2_PLAN")
        compose_build = backend.compose_build

        def observe_compose(service, override):
            resources("before-compose", backend)
            started = time.monotonic()
            try:
                result = compose_build(service, override)
            except fixture.BuildFailure:
                witness(backend)
                raise
            print(f"D2_GENUINE_COMPOSE exit=0 seconds={time.monotonic() - started:.2f}", flush=True)
            resources("after-compose-success", backend)
            return result

        backend.compose_build = observe_compose
        fixture.SUBSET_SERVICES = (SERVICE,)
        images = fixture.execute_plan(plan, backend, guard, actual_commit, actual_state,
                                      progress=progress, build_only=fixture.SUBSET_SERVICES)
        fixture.SUBSET_RECEIPT_BINS = {"D2": BINS}
        guard("before:d2-original-receipts")
        originals = fixture.producer_receipt_evidence(state, tuple(entries), commit,
                                                     fixture.SUBSET_RECEIPT_BINS)
        guard("after:d2-original-receipts")
        # Stop before official manifest creation. Main still closes its owned helper/builder.
        raise DiagnosticComplete(images[SERVICE], originals)

    fixture.execute_build = only_d2
    try:
        fixture.main(["--source-root", str(source), "--commit", commit, "--state-root", str(state),
                      "--manifest-file", str(manifest), "--deadline-monotonic-ns", str(deadline)])
    except DiagnosticComplete as result:
        if manifest.exists() or getattr(result, "_cleanup_failure", None):
            raise ValueError("COMPLETION_OR_CLEANUP_INVALID") from None
        print("D2_GENUINE_RECEIPTS " + json.dumps(result.originals, sort_keys=True), flush=True)
        print("D2_GENUINE_IMAGE " + json.dumps(result.image, sort_keys=True), flush=True)
        print("D2_GENUINE=ORIGINAL_TWO_BINARIES_CONSUMED_IMAGE_ID_VERIFIED_NO_MANIFEST", flush=True)
    except fixture.BuildFailure as error:
        fixture.report_failure(error)
        raise SystemExit(1) from None
    else:
        raise ValueError("UNEXPECTED_MANIFEST_FLOW")


if __name__ == "__main__":
    try:
        main()
    except (KeyError, OSError, RuntimeError, ValueError) as error:
        token = str(error)
        reason = token if re.fullmatch("[A-Z0-9_]+", token) else type(error).__name__
        raise SystemExit("D2_GENUINE_FAILURE=" + reason) from None
