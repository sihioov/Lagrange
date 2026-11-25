#!/usr/bin/env python3
"""Pre-upgrade inactive-simulation backup sequencer for a root with simulation operations.

It never re-implements the backup.  The normal journaled order (--step all) is:

  0 pause-observer       stop lagrange-service-status.timer if it was active, wait for the
                         oneshot collector to finish (never disable, never touch the service)
  1 stop-beta            drain the exact owner-beta-runner generation with the installed
                         application release's own owner-beta-daily-runtime.drain
                         (exit 0, restart=no); no Daily control write, no --daily-disable
  2 backup               pinned maintenance entrypoint, inactive-simulation-backup
  3 verify               pinned maintenance entrypoint, inactive-simulation-verify
  4 restore-beta-policy  docker update --restart=<recorded> on the exact still-stopped
                         generation, without starting it (must precede 5)
  5 resume               release wrapper --simulation-release-start, then --simulation-start
                         only if the backup captured a simulation worker (else a read-only
                         no-runner check), then a read-only check that the Beta is healthy
  6 restore-observer     start the timer again only if step 0 stopped it

Every step is journaled to a root 0600 progress file in the explicit --state-dir before
it mutates anything.  stop-beta, backup and resume-simulation are final once started;
the other steps may be re-run, and their observed-state guards run before they are
journaled again.  ``--step recover`` returns the host to normal after a failure, driven
by the observed simulation handoff: with a ``switched``/``api-started`` handoff it runs
the remaining restore/resume steps; without a handoff it restores the recorded Beta
generation (``recover-beta``).  A recovery prints RECOVERED, never PASS; PASS means every
normal step completed.  Mutating steps run only while step 0 is done (the timer paused).

Nightly apply requires a new state directory and verifies the isolated restore before
any service resumes. Its resume scope is the fenced API, the exact original Beta, and
the simulation worker only when it was running. A journal from an interrupted nightly
invocation can only be continued through explicit attended recovery.

It never runs --daily-enable/--daily-disable or a reconcile, never deletes a
pending/handoff file, never calls docker rm/stop or SIGKILL on a container, never
enables a KIS timer, and never synthesizes a rollout confirmation.  Output is value-free.
"""
from __future__ import annotations

import contextlib
import hashlib
import importlib.util
import os
import re
import stat
import sys
import time
from pathlib import Path
from types import SimpleNamespace

sys.dont_write_bytecode = True


def _load_helpers():
    path = Path(__file__).resolve().with_name("lib") / "reader-generation.py"
    spec = importlib.util.spec_from_file_location("reader_generation", path)
    module = importlib.util.module_from_spec(spec)
    sys.modules["reader_generation"] = module
    spec.loader.exec_module(module)
    return module


rg = _load_helpers()
require, ToolError = rg.require, rg.ToolError

SCHEMA = "lagrange-preupgrade-simulation-backup-progress-v1"
PROGRESS_NAME = "preupgrade-simulation-backup"
BETA = "owner-beta-runner"
TIMER = "lagrange-service-status.timer"
OBSERVER_SERVICE = "lagrange-service-status.service"
SYSTEMCTL = "/usr/bin/systemctl"
START_PENDING = ".owner-beta-simulation-start-pending"
HANDOFF_PENDING = ".owner-beta-simulation-release-handoff"
SIMULATION_RUNNER = "owner-beta-simulation-runner"
API = "api-server"
V2 = "owner-equity-v2-runner"
DATABASE = "postgres"
NIGHTLY_SERVICES = (API, BETA, SIMULATION_RUNNER, V2, DATABASE)
DAILY_SERVICES = ("owner-beta-daily-producer", "owner-beta-daily-admitter")
DAILY_RESUME_STEP = "resume-daily-writers"
NIGHTLY_ERROR = "PREUPGRADE_NIGHTLY_COHORT"
NIGHTLY_FENCING_UNCERTAIN = "PREUPGRADE_NIGHTLY_FENCING_UNCERTAIN"
NIGHTLY_API_ONLY = "--simulation-resume-api-only"
# Journal key written together with the resume-release "started" mark, from the capture's
# handoff record, before the wrapper can delete that record (no worker: finish-release removes it).
WORKER_DECISION = "simulation_worker_captured"
WORKER_KEYS = ("worker_generation", "worker_policy", "worker_running")
API_ATTEMPT = "nightly_api_resume_attempt"
RESUMED_API = "nightly_simulation_resume_api_generation"
OBSERVED_WORKER = "nightly_observed_simulation_resume_generation"
PENDING_LIMIT = 16384
CONFIRMATIONS = ("OWNER_EQUITY_V2_ROLLOUT_CONFIRM", "OWNER_MARKET_STREAM_ROLLOUT_CONFIRM")
ENTRYPOINT = "scripts/ops/lib/owner-beta-daily-backup.py"
WRAPPER = "scripts/ops/compose-release.sh"

# Normal order. Recovery-only steps are outside it.
STEPS = ("pause-observer", "stop-beta", "backup", "verify", "restore-beta-policy",
         "resume-release", "resume-simulation", "verify-beta", "restore-observer")
JOURNALED = STEPS + ("recover-beta", "recover-handoff", "observe-simulation-resume")
GROUPS = {"pause-observer": ("pause-observer",), "stop-beta": ("stop-beta",), "backup": ("backup",),
          "verify": ("verify",), "restore-beta-policy": ("restore-beta-policy",),
          "resume": ("resume-release", "resume-simulation", "verify-beta"),
          "recover-beta": ("recover-beta",), "restore-observer": ("restore-observer",),
          "recover": None, "all": STEPS}
# resume-release may be retried because its guards (handoff still "switched", the exact
# Beta still stopped with its recorded policy) run before it is journaled again; once the
# wrapper has moved the handoff past "switched" those guards refuse.  stop-beta, backup
# and resume-simulation are final.
RETRYABLE = frozenset(("pause-observer", "verify", "restore-beta-policy", "resume-release",
                        "verify-beta", "recover-beta", "recover-handoff", "observe-simulation-resume",
                        "restore-observer"))
# Steps that change Docker, the database or simulation state.  Each requires the
# observer timer to be paused (pause-observer done) when it starts.
MUTATING = frozenset(("stop-beta", "backup", "verify", "restore-beta-policy", "resume-release",
                       "resume-simulation", "recover-beta", "recover-handoff"))
# 312 compose-release.sh:565-584 requires these exact process-local values only when
# the release env selects the corresponding mode (see required_confirmations).
EQUITY_CONFIRMATION = ("OWNER_EQUITY_V2_ROLLOUT_CONFIRM", "I_UNDERSTAND_OWNER_EQUITY_V2_READ_ONLY_KIS_CALLS")
STREAM_CONFIRMATION = ("OWNER_MARKET_STREAM_ROLLOUT_CONFIRM",
                       "I_UNDERSTAND_OWNER_MARKET_STREAM_READ_ONLY_WS_CALLS")
# Reads only the three mode selectors with the release's own non-evaluating dotenv
# parser and KIS read configuration, as compose-release.sh does; prints nothing else.
CONFIRMATION_PROBE = r'''root=$1
source "$root/scripts/ops/lib/dotenv.sh" || exit 3
source "$root/scripts/ops/lib/kis-read-compose.sh" || exit 3
dotenv_load "$root/deploy/compose/.env" >/dev/null 2>&1 || exit 3
kis_read_compose_configure "$root" >/dev/null 2>&1 || exit 3
equity=$(dotenv_effective_get OWNER_EQUITY_V2_RUNTIME_MODE) || exit 3
printf 'equity=%s\nquotes=%s\ntransport=%s\n' "$equity" \
  "$KIS_READ_COMPOSE_OWNER_INTRADAY_QUOTES_MODE" "$KIS_READ_COMPOSE_QUOTE_TRANSPORT"
'''

PLAN_LINE = "INACTIVE_SIMULATION_PLAN: "
NIGHTLY_PLAN_SUFFIX = "; paper_history=legacy_empty_required; inactive_daily_preflight=pinned_dates"
PREFLIGHT_LINE = "DAILY_BACKUP: PREFLIGHT"
PAPER_PREFLIGHT_LINE = "DAILY_BACKUP: NIGHTLY_PAPER_CONFIGURED_OFF"
PAPER_BACKUP_LINE = "DAILY_BACKUP: NIGHTLY_PAPER_CONFIGURED_OFF_VERIFIED_SOURCE_UNCHANGED"
PAPER_VERIFY_LINE = "DAILY_BACKUP: NIGHTLY_PAPER_CONFIGURED_OFF_VERIFIED_ISOLATED_DISABLED"
PAPER_ARCHIVE_PREFIX = "DAILY_BACKUP: NIGHTLY_PAPER_ARCHIVE "
PAPER_ORIGIN_PREFIX = "DAILY_BACKUP: NIGHTLY_PAPER_ORIGIN "
PAPER_READBACK_LINE = "DAILY_BACKUP: NIGHTLY_PAPER_ORIGIN_UNCHANGED"
PAPER_ARCHIVE_DIGEST = re.compile(r"sha256:[0-9a-f]{64}")
BACKUP_LINE = "DAILY_BACKUP: INACTIVE_SIMULATION_VERIFIED_DISABLED"
VERIFY_LINE = "DAILY_BACKUP: INACTIVE_SIMULATION_VERIFIED_ISOLATED_DISABLED"
# Exact final line of the installed 312 compose-release.sh (CR:1063).
RELEASE_LINE = ("COMPOSE_RELEASE: PASS (immutable manifest image IDs bound; run post-backfill-health.sh "
                "--scope release --check to assert data readiness)")
SIMULATION_LINE = "SIMULATION_OPERATION: STARTED"
NIGHTLY_API_LINE = "COMPOSE_SIMULATION_API_RESUME: PASS"
SIMULATION_DRAINED_LINE = "SIMULATION_OPERATION: DRAINED_OFF"
SKIP_LINE = "PREUPGRADE_BACKUP: resume-simulation: SKIPPED_NO_CAPTURED_WORKER"
PASS_SKIPPED_FIELD = " simulation_worker=not_running_before_not_started"

PINNED_STDERR = re.compile(r"(?:DAILY_BACKUP: FAILED|PREACTIVATION_BACKUP: FAILED [A-Z][A-Z0-9_]{0,120}"
                           r"|PAPER_BACKUP: FAILED [A-Z][A-Z0-9_]{0,120})")
WRAPPER_STDERR = re.compile(r"(?:SIMULATION_OPERATION_[A-Z_]{1,60}|BLOCKED_EXTERNAL: [a-z0-9_]{1,100}"
                            r"|compose-release: [a-z0-9 _.,:;'()-]{1,160})")

TIMEOUTS = {"plan": 60, "preflight": 600, "backup": 4 * 3600, "verify": 4 * 3600,
            "release": 3600, "simulation": 1800, "systemctl": 30}
OBSERVER_WAIT_SECONDS = 180
BETA_HEALTH_SECONDS = 300
POLL_SECONDS = 2.0


def parse(argv):
    parser = rg.Parser(prog="preupgrade-simulation-backup.py", code="PREUPGRADE_ARGUMENTS",
                       description="Sequence the pinned inactive-simulation backup around the manual Beta.")
    modes = parser.add_mutually_exclusive_group(required=True)
    for mode in ("plan", "preflight", "apply"):
        modes.add_argument("--" + mode, dest="mode", action="store_const", const=mode)
    for option in ("--install-root", "--application-commit", "--tool-commit", "--config-file",
                   "--expected-db-first-session", "--expected-config-first-session",
                   "--stopped-worker-from-commit", "--state-dir"):
        parser.add_argument(option, required=True)
    parser.add_argument("--step", choices=tuple(GROUPS))
    parser.add_argument("--backup-profile", choices=("daily-simulation",))
    parser.add_argument("--expected-nightly-paper-profile", choices=("LEGACY_EMPTY", "CONFIGURED_OFF"))
    parser.add_argument("--expected-nightly-paper-origin")
    parser.add_argument("--nightly", action="store_true",
                        help="Use a fresh journal and resume only the captured running cohort.")
    rg.reject_repeated(argv, ("--plan", "--preflight", "--apply", "--install-root", "--application-commit",
                              "--tool-commit", "--config-file", "--expected-db-first-session",
                              "--expected-config-first-session", "--stopped-worker-from-commit",
                              "--state-dir", "--step", "--nightly", "--backup-profile",
                              "--expected-nightly-paper-profile", "--expected-nightly-paper-origin"),
                       "PREUPGRADE_ARGUMENTS")
    args = parser.parse_args(argv)
    require(rg.absolute(args.install_root) and rg.absolute(args.config_file) and rg.absolute(args.state_dir)
            and rg.is_commit(args.application_commit) and rg.is_commit(args.tool_commit)
            and rg.is_commit(args.stopped_worker_from_commit)
            and rg.is_date(args.expected_db_first_session) and rg.is_date(args.expected_config_first_session)
            and (args.expected_db_first_session == args.expected_config_first_session if args.backup_profile
                 else args.expected_db_first_session < args.expected_config_first_session)
            and (not args.backup_profile or args.nightly or (args.mode == "apply" and args.step == "recover"))
            and (args.expected_nightly_paper_profile is None or args.backup_profile
                 and args.nightly and args.mode == "apply")
            and (args.expected_nightly_paper_origin is None or args.expected_nightly_paper_profile == "CONFIGURED_OFF"
                 and PAPER_ARCHIVE_DIGEST.fullmatch(args.expected_nightly_paper_origin))
            and (args.step is None or args.mode == "apply")
            and (not args.nightly or (args.mode in ("apply", "preflight") and args.step in (None, "all"))),
            "PREUPGRADE_ARGUMENTS")
    args.step = args.step or "all"
    return args


def arguments_record(args):
    arguments = {"install_root": args.install_root, "application_commit": args.application_commit,
            "tool_commit": args.tool_commit, "config_file": args.config_file,
            "expected_db_first_session": args.expected_db_first_session,
            "expected_config_first_session": args.expected_config_first_session,
            "stopped_worker_from_commit": args.stopped_worker_from_commit}
    if getattr(args, "backup_profile", None):
        arguments["backup_profile"] = args.backup_profile
    return arguments


class Sequencer:
    def __init__(self, args, host, *, uid, runner, environ, err, clock, sleep, out=None):
        self.args, self.host, self.uid = args, host, uid
        self.runner, self.environ, self.err = runner, environ, err
        self.out = sys.stdout if out is None else out
        self.clock, self.sleep = clock, sleep
        self.docker = host.docker if host is not None else None
        self.progress = None
        self._project = None
        self.nightly_capture = None
        self.daily_snapshot = None
        self.daily_drain_started = False

    @property
    def combined(self):
        return getattr(self.args, "backup_profile", None) == "daily-simulation"

    def backup_operation(self, action):
        return ("nightly-combined-" if self.combined else "inactive-simulation-") + action

    def combined_operations(self):
        host, a = self.host, self.args
        ops = host.ops
        images = host.manifest_images(a.application_commit)
        values, overlays = ops.read_context(host.release, a.application_commit, "enable")
        ops.reject_ambient(values)
        project, mode = ops.project_binding(a.application_commit, a.application_commit)
        require(project == self.project() and mode in {"daily", "daemon"} and
                values["OWNER_BETA_DAILY_MODE"] == "owner_only" and values["OWNER_BETA_ACCESS_MODE"] == "owner_only" and
                values["OWNER_BETA_PRICE_INPUT_MODE"] == "sealed_v1" and
                values["OWNER_BETA_PAPER_MODE"] in ("", "disabled"), NIGHTLY_ERROR)
        config, digest = ops.install.verify_config(host.release, a.application_commit)
        require(config["first_auto_session"] == a.expected_config_first_session, NIGHTLY_ERROR)
        binding = host.runtime.Binding(project, a.application_commit, images[BETA], digest,
            host.release, values["LAGRANGE_DATA_DIR"], values["LAGRANGE_ARTIFACTS_DIR"],
            values["LAGRANGE_RUNTIME_SECRET_DIR"], ops.database_name(values), values["MAX_JOBS_PER_OWNER"],
            values["PRICE_BETA_LEASE_SECS"], values["PRICE_BETA_BACKOFF_SECS"])
        context = ops.Context(host.release, a.application_commit, images, values, overlays)
        database = ops.Database(self.docker, project, a.application_commit, images["db-migrate"])
        return ops.Operations(self.docker, database, context, context, binding, a.install_root, mode), binding

    def daily_state(self, operation):
        target = operation.target
        return operation.database.nightly_state(target.release, target.commit, target.values,
                                                self.args.expected_db_first_session)

    def original_daily_state(self, record, operation):
        original = record["nightly"]
        expected = (original["daily_enabled"], original["daily_epoch"])
        require(self.daily_state(operation) == expected, NIGHTLY_ERROR)
        return expected

    def daily_api_mode(self, operation, binding, services):
        api = services[API]["generation"]
        value = self.docker.object("container", api[0], 15)
        require(list(self.host.runtime.owned(value,
            self.host.ownership(binding.project, binding.commit, operation.target.images[API]), API, api[0]))
            == api and rg.running_healthy(value), NIGHTLY_ERROR)
        mode = self.host.runtime.environment(value).get("OWNER_BETA_DAILY_MODE")
        require(mode in {"disabled", "owner_only"}, NIGHTLY_ERROR)
        if mode == "owner_only":
            observed = self.host.ops.verify_api(self.docker, binding, operation.target.images[API])
            require(list(observed) == api, NIGHTLY_ERROR)
        return mode == "owner_only"

    def verify_active_daily(self, operation, binding, services):
        ops, runtime = self.host.ops, self.host.runtime
        ops.global_cohort(self.docker, binding)
        cohort = runtime.verify_running(self.docker, binding)
        require(all(services[name] and services[name]["generation"] == list(cohort[name]["generation"])
                    for name in runtime.SERVICES), NIGHTLY_ERROR)
        api = ops.verify_api(self.docker, binding, operation.target.images[API])
        require(list(api) == services[API]["generation"], NIGHTLY_ERROR)
        return cohort

    # -- child processes ---------------------------------------------------
    def passthrough(self, data, pattern):
        lines = rg.stdout_lines(data or b"") or []
        for line in lines[-20:]:
            if pattern.fullmatch(line):
                print(line, file=self.err)

    def entrypoint(self):
        return rg.trusted_chain(self.args.install_root,
                                "maintenance/" + self.args.tool_commit + "/" + ENTRYPOINT,
                                self.uid, "PREUPGRADE_TOOL")

    def pinned(self, operation, mode, timeout, *, record=None, paper_origin=None):
        a = self.args
        argv = ["/usr/bin/python3", "-I", "-S", "-B", self.entrypoint(),
                "--operation", operation, "--mode", mode, "--install-root", a.install_root,
                "--application-release", a.install_root + "/releases/" + a.application_commit,
                "--config-file", a.config_file,
                "--expected-db-first-session", a.expected_db_first_session,
                "--expected-config-first-session", a.expected_config_first_session,
                "--stopped-worker-from-commit", a.stopped_worker_from_commit]
        if (self.combined and a.mode == "apply" and
                (mode == "preflight"
                 or (mode == "apply" and record["nightly"].get("paper_configured_off") is True))):
            argv.extend(("--nightly-state-date", Path(a.state_dir).name))
        if self.combined and mode in {"apply", "readback"}:
            original = record["nightly"]
            state = "ENABLED" if original["daily_enabled"] else "DISABLED"
            argv.extend(("--expected-nightly-daily-control", f"{state}:{original['daily_epoch']}"))
            if original.get("paper_configured_off") is True:
                argv.append("--expected-nightly-paper-off")
                if operation == "nightly-combined-verify":
                    archive = record.get("nightly_paper_archive")
                    require(type(archive) is str and PAPER_ARCHIVE_DIGEST.fullmatch(archive), NIGHTLY_ERROR)
                    argv.extend(("--expected-nightly-paper-archive", archive))
                elif operation == "nightly-combined-paper-readback":
                    require(type(paper_origin) is str and PAPER_ARCHIVE_DIGEST.fullmatch(paper_origin), NIGHTLY_ERROR)
                    argv.extend(("--expected-nightly-paper-origin", paper_origin))
        code, out, err = self.runner(argv, rg.child_environment(), timeout)
        self.passthrough(err, PINNED_STDERR)
        return code, rg.stdout_lines(out or b"")

    def confirmations(self):
        return {k: self.environ[k] for k in CONFIRMATIONS if self.environ.get(k)}

    def required_confirmations(self):
        """Which confirmations the installed wrapper's release start will demand."""
        a, code = self.args, "PREUPGRADE_CONFIRMATION"
        release = "releases/" + a.application_commit
        for name in ("dotenv.sh", "kis-read-compose.sh"):
            rg.trusted_chain(a.install_root, release + "/scripts/ops/lib/" + name, self.uid, code)
        result, out, _ = self.runner(["/bin/bash", "--noprofile", "--norc", "-c", CONFIRMATION_PROBE,
                                      "confirmation-probe", a.install_root + "/" + release],
                                     rg.child_environment(), 60)
        lines = rg.stdout_lines(out or b"")
        require(result == 0 and lines is not None and len(lines) == 3, code)
        fields = {}
        for line, key in zip(lines, ("equity", "quotes", "transport")):
            name, sep, value = line.partition("=")
            require(sep and name == key and re.fullmatch(r"[a-z_]{0,32}", value), code)
            fields[key] = value
        required = []
        if (fields["equity"] or "disabled") == "owner_only":
            required.append(EQUITY_CONFIRMATION)
        if fields["quotes"] == "owner_only" and fields["transport"] == "market_ws":
            required.append(STREAM_CONFIRMATION)
        return required

    def check_confirmations(self):
        """Fail closed before any mutation when a demanded confirmation is missing."""
        required = self.required_confirmations()
        for name, value in required:
            require(self.environ.get(name) == value, "PREUPGRADE_CONFIRMATION")
        return {name for name, _ in required}

    def wrapper(self, flag, timeout, *, api_only=False, daily_api=False, daily_control=None, paper_origin=None,
                mode="apply"):
        a = self.args
        path = rg.trusted_chain(a.install_root, "releases/" + a.application_commit + "/" + WRAPPER,
                                self.uid, "PREUPGRADE_RESUME_STEP")
        # Passed through unchanged when the operator set them; never synthesized.
        env = rg.child_environment({"LAGRANGE_RELEASE_ROOT": a.install_root, **self.confirmations()})
        argv = ["/bin/bash", "--noprofile", "--norc", path, "--scope", "release", "--" + mode, flag]
        if api_only:
            argv.append(NIGHTLY_API_ONLY)
        if daily_api:
            require(api_only and self.combined, NIGHTLY_ERROR)
            argv.append("--simulation-resume-daily-api")
        if daily_control is not None:
            enabled, epoch = daily_control
            require(api_only and mode == "apply" and (daily_api if enabled else paper_origin is not None),
                    NIGHTLY_ERROR)
            argv.extend(("--expected-nightly-daily-control", f"{'ENABLED' if enabled else 'DISABLED'}:{epoch}"))
        if paper_origin is not None:
            require(daily_control is not None and PAPER_ARCHIVE_DIGEST.fullmatch(paper_origin), NIGHTLY_ERROR)
            argv.extend(("--expected-nightly-paper-origin", paper_origin))
        code, out, err = self.runner(argv, env, timeout)
        self.passthrough(err, WRAPPER_STDERR)
        return code, rg.stdout_lines(out or b"")

    def unit_state(self, unit):
        code, out, _ = self.runner((SYSTEMCTL, "show", "--property=LoadState", "--property=ActiveState", unit),
                                   rg.child_environment(), TIMEOUTS["systemctl"])
        lines = rg.stdout_lines(out or b"")
        require(code == 0 and lines is not None, "PREUPGRADE_OBSERVER")
        fields = {}
        for line in lines:
            key, sep, value = line.partition("=")
            require(sep and key not in fields, "PREUPGRADE_OBSERVER")
            fields[key] = value
        require(set(fields) == {"LoadState", "ActiveState"}, "PREUPGRADE_OBSERVER")
        return fields["LoadState"], fields["ActiveState"]

    def systemctl(self, verb, unit):
        code, _, _ = self.runner((SYSTEMCTL, verb, unit), rg.child_environment(), TIMEOUTS["systemctl"])
        require(code == 0, "PREUPGRADE_OBSERVER")

    def timer_active(self):
        load, active = self.unit_state(TIMER)
        if load == "not-found":
            require(active == "inactive", "PREUPGRADE_OBSERVER")
            return False
        require(load == "loaded" and active in ("active", "inactive"), "PREUPGRADE_OBSERVER")
        return active == "active"

    # -- host observations -------------------------------------------------
    def wrap(self, code, action, *values):
        try:
            return action(*values)
        except ToolError:
            raise
        except Exception as exc:
            raise ToolError(code, rg.cause_of(exc)) from None

    def project(self):
        if self._project is None:
            self._project = self.wrap("PREUPGRADE_WORKER_SOURCE", self.host.project)
        return self._project

    def current(self):
        self.wrap("PREUPGRADE_CURRENT", self.host.validate_current)

    def tool(self):
        self.entrypoint()
        self.wrap("PREUPGRADE_TOOL", self.host.verify_tool, self.args.tool_commit)

    def nightly_tool(self):
        """Run the pinned capture's own app-package compatibility check before downtime."""
        a = self.args
        relative = "maintenance/" + a.tool_commit

        def check():
            controller = rg.load_module("preupgrade_nightly_pinned", a.install_root,
                                        relative + "/" + ENTRYPOINT, self.uid, "PREUPGRADE_TOOL")
            application = a.install_root + "/releases/" + a.application_commit
            controller.inactive_simulation_tool(SimpleNamespace(release=application),
                                                a.install_root + "/" + relative)

        self.wrap("PREUPGRADE_TOOL", check)

    def pending(self, name):
        path = self.args.install_root + "/" + name
        try:
            fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
        except FileNotFoundError:
            return None
        except OSError:
            raise ToolError("PREUPGRADE_PENDING") from None
        try:
            info = os.fstat(fd)
            require(stat.S_ISREG(info.st_mode) and info.st_uid == self.uid and info.st_nlink == 1
                    and stat.S_IMODE(info.st_mode) == 0o600 and info.st_size <= PENDING_LIMIT,
                    "PREUPGRADE_PENDING")
            data = os.read(fd, PENDING_LIMIT + 1)
        finally:
            os.close(fd)
        value = rg.strict_json(data, "PREUPGRADE_PENDING")
        require(isinstance(value, dict), "PREUPGRADE_PENDING")
        return value

    def no_pending(self, code):
        require(self.pending(START_PENDING) is None and self.pending(HANDOFF_PENDING) is None, code)

    def handoff(self, phases, code, *, absent_ok=False):
        require(self.pending(START_PENDING) is None, code)
        record = self.pending(HANDOFF_PENDING)
        if record is None:
            require(absent_ok, code)
            return None
        commit = self.args.application_commit
        require(record.get("phase") in phases and record.get("source_commit") == commit
                and record.get("target_commit") == commit, code)
        return record

    def worker_decision(self, handoff):
        """True when the capture recorded a simulation worker, False when it recorded none.

        Mirrors the handoff's own worker fields (SO:_handoff_read) and the rule finish-release
        uses to delete the handoff (worker_generation None).  Anything else is ambiguous.
        """
        code = "PREUPGRADE_WORKER_DECISION"
        require(all(key in handoff for key in WORKER_KEYS), code)
        generation, policy, running = (handoff[key] for key in WORKER_KEYS)
        if generation is None:
            require(policy is None and running is False, code)
            return False
        require(type(generation) is list and len(generation) == 4 and type(generation[0]) is str
                and rg.CONTAINER.fullmatch(generation[0])
                and all(type(generation[i]) is str and generation[i] for i in (1, 2))
                and type(generation[3]) is int and generation[3] >= 0
                and policy in ("no", "unless-stopped") and type(running) is bool, code)
        return True

    def recorded_decision(self, record):
        value = record.get(WORKER_DECISION)
        require(type(value) is bool, "PREUPGRADE_WORKER_DECISION")
        return value

    def no_simulation_runner(self):
        """No simulation runner container in any state (same global service filter as the wrapper)."""
        code = "PREUPGRADE_SIMULATION_RUNNER"
        lines = self.wrap(code, lambda: self.docker.call(
            ["ps", "--all", "--no-trunc", "--filter", "label=com.docker.compose.service=" + SIMULATION_RUNNER,
             "--format", "{{.ID}}"], 15).splitlines())
        require(all(rg.CONTAINER.fullmatch(line) for line in lines) and lines == [], code)

    def nightly_services(self):
        """Read the original service cohort by Docker identity, never by a mutable tag."""
        code = NIGHTLY_ERROR
        project = self.project()
        images = self.wrap(code, self.host.manifest_images, self.args.application_commit)
        observed = {}
        names = (*NIGHTLY_SERVICES, *DAILY_SERVICES) if self.combined else NIGHTLY_SERVICES
        for service in names:
            if service in (API, SIMULATION_RUNNER) or self.combined and service in (BETA, *DAILY_SERVICES):
                ids = self.wrap(code, self.docker.call, [
                    "ps", "--all", "--no-trunc", "--filter", "label=com.docker.compose.service=" + service,
                    "--format", "{{.ID}}"], 15).splitlines()
            else:
                ids = rg.compose_ids(self.docker, project, service, code)
            require(len(ids) <= 1 and all(rg.CONTAINER.fullmatch(cid) for cid in ids), code)
            if not ids:
                observed[service] = None
                continue
            identity = ids[0]
            value = self.wrap(code, self.docker.object, "container", identity, 15)
            labels = value["Config"]["Labels"]
            state, image = value["State"], value["Image"]
            image_service = BETA if service in DAILY_SERVICES else service
            require(value["Id"] == identity and labels.get("com.docker.compose.project") == project
                    and labels.get("com.docker.compose.service") == service
                    and labels.get("com.docker.compose.oneoff") == "False"
                    and re.fullmatch(r"sha256:[0-9a-f]{64}", image)
                    and (service in (V2, DATABASE) or (
                        image == images.get(image_service)
                        and labels.get("org.opencontainers.image.revision") == self.args.application_commit))
                    and type(state.get("Running")) is bool
                    and state.get("Status") == ("running" if state["Running"] else "exited")
                    and not any(state.get(key) for key in ("Restarting", "Paused", "OOMKilled", "Dead", "Error"))
                    and all(isinstance(stamp, str) and stamp for stamp in (value["Created"], state.get("StartedAt")))
                    and type(value.get("RestartCount")) is int and value["RestartCount"] >= 0, code)
            if state["Running"] and service in (API, BETA, DATABASE, *DAILY_SERVICES):
                require(rg.running_healthy(value), code)
            observed[service] = {
                "generation": [identity, value["Created"], state["StartedAt"], value["RestartCount"]],
                "image": image, "running": state["Running"], "restart_policy": rg.restart_policy(value, code),
            }
        return observed

    def nightly_targets(self, *, daily_api=False):
        kwargs = {"daily_api": True} if daily_api else {}
        code, lines = self.wrapper("--simulation-release-start", TIMEOUTS["plan"], api_only=True, mode="plan", **kwargs)
        prefix = "SIMULATION_DAILY_API_RESUME_TARGETS: " if daily_api else "SIMULATION_API_RESUME_TARGETS: "
        require(code == 0 and lines == [prefix + API], "PREUPGRADE_NIGHTLY_TARGETS")

    def capture_nightly(self, timer):
        self.nightly_targets()
        services = self.nightly_services()
        require(all(services[name] and services[name]["running"] for name in (API, BETA, DATABASE)), NIGHTLY_ERROR)
        worker = services[SIMULATION_RUNNER]
        require(services[API]["restart_policy"]["Name"] == "unless-stopped"
                and (worker is None or (worker["running"] and worker["restart_policy"] == rg.STOPPED_POLICY)),
                NIGHTLY_ERROR)
        capture = {"services": services, "observer_timer_active": timer}
        if self.combined:
            operation, binding = self.combined_operations()
            active, epoch = self.daily_state(operation)
            daily_api = self.daily_api_mode(operation, binding, services)
            if daily_api:
                self.nightly_targets(daily_api=True)
            if active:
                require(daily_api, NIGHTLY_ERROR)
                self.verify_active_daily(operation, binding, services)
            else:
                require(all(services[name] is None for name in DAILY_SERVICES), NIGHTLY_ERROR)
                _, snapshot, _ = self.running_beta()
                env = self.host.runtime.environment(snapshot[BETA]["value"])
                require(env.get("PRICE_BETA_DAILY_MODE") == "disabled", NIGHTLY_ERROR)
            capture.update(daily_enabled=active, daily_epoch=epoch, daily_api=daily_api)
        return capture

    def check_nightly(self, record, *, stopped=(), resumed=()):
        original = record["nightly"]["services"]
        live = self.nightly_services()
        require(set(original) == set(live), NIGHTLY_ERROR)
        if self.combined and record["nightly"]["daily_enabled"]:
            if BETA in stopped:
                stopped = (*stopped, *DAILY_SERVICES)
            if BETA in resumed:
                resumed = (*resumed, *DAILY_SERVICES)
        for service in live:
            before, after = original[service], live[service]
            if service in stopped:
                require(before and before["running"] and after and not after["running"]
                        and before["generation"] == after["generation"] and before["image"] == after["image"]
                        and after["restart_policy"] in (rg.STOPPED_POLICY, before["restart_policy"]), NIGHTLY_ERROR)
            elif service in resumed:
                require(before and before["running"] and after and after["running"]
                        and before["image"] == after["image"]
                        and before["restart_policy"] == after["restart_policy"], NIGHTLY_ERROR)
                if service in (BETA, *DAILY_SERVICES):
                    require(rg.same_generation(after["generation"], before["generation"]), NIGHTLY_ERROR)
            else:
                require(before == after, NIGHTLY_ERROR)
        return live

    def check_nightly_handoff(self, record, handoff, *, api_started=False):
        original = record["nightly"]["services"]
        worker = original[SIMULATION_RUNNER]
        expected_worker = worker["generation"] if worker else None
        require(self.worker_decision(handoff) == bool(worker)
                and (worker is None or handoff["worker_running"] is True)
                and handoff["worker_generation"] == expected_worker
                and (worker is None or handoff["worker_policy"] == worker["restart_policy"]["Name"])
                and handoff.get("api_generation") == original[API]["generation"], NIGHTLY_ERROR)
        stopped = [BETA, *([SIMULATION_RUNNER] if worker else [])]
        if handoff.get("phase") == "fencing":
            # Retry only before either fenced service or its policy has changed.
            try:
                self.check_nightly(record, stopped=(BETA,))
            except ToolError:
                raise ToolError(NIGHTLY_FENCING_UNCERTAIN) from None
        elif api_started:
            live = self.check_nightly(record, stopped=stopped, resumed=(API,))
            require(handoff.get("target_api_generation") == live[API]["generation"], NIGHTLY_ERROR)
        else:
            self.check_nightly(record, stopped=[*stopped, API])

    @contextlib.contextmanager
    def nightly_restore_lock(self, record):
        if "nightly" not in record:
            yield
            return
        with self.host.ops.release_lock(self.args.install_root):
            self.current()
            self.no_pending(NIGHTLY_ERROR)
            yield

    def nightly_untouched_digest(self, capture):
        services = capture["services"]
        return "sha256:" + hashlib.sha256(rg.canonical({name: services[name] for name in (V2, DATABASE)})).hexdigest()

    def worker_ownership(self, image=None):
        def build():
            nonlocal image
            commit = self.args.stopped_worker_from_commit
            if image is None:
                images = self.host.manifest_images(commit)
                require(isinstance(images, dict) and isinstance(images.get(BETA), str),
                        "PREUPGRADE_WORKER_SOURCE")
                image = images[BETA]
            return self.host.ownership(self.project(), commit, image)
        return self.wrap("PREUPGRADE_WORKER_SOURCE", build)

    def running_beta(self):
        """The one captured Beta, owned by --stopped-worker-from-commit (DRT:156-170 rule)."""
        own = self.worker_ownership()
        self.wrap("PREUPGRADE_WORKER_SOURCE", self.host.global_cohort, own)
        snapshot = self.wrap("PREUPGRADE_WORKER_SOURCE", self.host.capture, own)
        require(isinstance(snapshot, dict) and set(snapshot) == {BETA}, "PREUPGRADE_WORKER_SOURCE")
        entry = snapshot[BETA]
        require(rg.running_healthy(entry["value"]), "PREUPGRADE_BETA_STOP")
        policy = rg.restart_policy(entry["value"], "PREUPGRADE_BETA_STOP")
        generation = list(entry["generation"])
        beta = {"identity": generation[0], "created": generation[1], "started_at": generation[2],
                "restart_count": generation[3], "image": own.image_id, "restart_policy": policy}
        return own, snapshot, beta

    def stopped_beta(self, beta, policies, code):
        """Exact recorded generation, still exited 0, with one of the allowed policies."""
        own = self.worker_ownership(beta["image"])
        require(rg.compose_ids(self.docker, own.project, BETA, code) == [beta["identity"]], code)
        self.wrap(code, self.host.global_cohort, own)
        value = self.wrap(code, self.docker.object, "container", beta["identity"], 15)
        generation = list(self.wrap(code, self.host.owned, value, own, BETA, beta["identity"]))
        require(generation == [beta["identity"], beta["created"], beta["started_at"], beta["restart_count"]]
                and rg.exited_zero(value) and rg.restart_policy(value, code) in policies, code)

    def resumed_beta(self, beta):
        """Started by the release wrapper (reused or recreated): current manifest, recorded policy."""
        code = "PREUPGRADE_RESUME_STEP"
        commit = self.args.application_commit
        images = self.wrap(code, self.host.manifest_images, commit)
        require(isinstance(images, dict) and isinstance(images.get(BETA), str), code)
        own = self.wrap(code, self.host.ownership, self.project(), commit, images[BETA])
        deadline = self.clock() + BETA_HEALTH_SECONDS
        while True:
            ids = rg.compose_ids(self.docker, own.project, BETA, code)
            require(len(ids) == 1, code)
            value = self.wrap(code, self.docker.object, "container", ids[0], 15)
            self.wrap(code, self.host.owned, value, own, BETA, ids[0])
            state = value.get("State") or {}
            require(rg.restart_policy(value, code) == beta["restart_policy"]
                    and state.get("Running") is True and not state.get("Restarting")
                    and not state.get("Paused") and not state.get("OOMKilled")
                    and (state.get("Health") or {}).get("Status") in ("starting", "healthy"), code)
            if rg.running_healthy(value):
                return ids[0] == beta["identity"]
            require(self.clock() < deadline, code)
            self.sleep(POLL_SECONDS)

    # -- journal -----------------------------------------------------------
    def mark(self, record, name, state, code=None, **extra):
        steps = dict(record["steps"])
        steps[name] = {"state": state, "code": code}
        return self.progress.replace(record, dict(record, steps=steps, **extra))

    def execute(self, record, name, code, body, *, final_state="done", **extra):
        record = self.mark(record, name, "started", **extra)
        try:
            updates = body(record) or {}
        except ToolError as exc:
            self.mark(record, name, "failed", exc.code)
            raise
        except Exception as exc:
            self.mark(record, name, "failed", code)
            raise ToolError(code, rg.cause_of(exc)) from None
        return self.mark(record, name, final_state, **updates)

    # -- steps ---------------------------------------------------------------
    def step_pause_observer(self, record):
        observer = record["observer"]
        if observer is None:
            # Recorded once, before the first stop; a retry never re-observes it.
            observer = {"timer_was_active": self.timer_active()}
            if "nightly" in record:
                require(observer["timer_was_active"] == record["nightly"]["observer_timer_active"], NIGHTLY_ERROR)

        def body(current):
            if current["observer"]["timer_was_active"]:
                self.systemctl("stop", TIMER)
                require(self.unit_state(TIMER)[1] == "inactive", "PREUPGRADE_OBSERVER")
            deadline = self.clock() + OBSERVER_WAIT_SECONDS
            while True:
                active = self.unit_state(OBSERVER_SERVICE)[1]
                if active == "inactive":
                    return None
                require(active in ("active", "activating", "deactivating")
                        and self.clock() < deadline, "PREUPGRADE_OBSERVER")
                self.sleep(POLL_SECONDS)
        return self.execute(record, "pause-observer", "PREUPGRADE_OBSERVER", body, observer=observer)

    def step_stop_beta(self, record):
        self.no_pending("PREUPGRADE_PENDING")
        if "nightly" in record:
            self.check_nightly(record)
        if self.combined and record["nightly"]["daily_enabled"]:
            return self.step_stop_daily(record)
        own, snapshot, beta = self.running_beta()
        if "nightly" in record:
            require(beta["identity"] == record["nightly"]["services"][BETA]["generation"][0]
                    and [beta["created"], beta["started_at"], beta["restart_count"]]
                    == record["nightly"]["services"][BETA]["generation"][1:], NIGHTLY_ERROR)

        def body(current):
            self.wrap("PREUPGRADE_BETA_STOP", self.host.drain, own, snapshot)
            # DBK requires the whole dict; drain itself checks only the name.
            self.stopped_beta(current["beta"], [rg.STOPPED_POLICY], "PREUPGRADE_BETA_STOP")
        return self.execute(record, "stop-beta", "PREUPGRADE_BETA_STOP", body, beta=beta)

    def step_stop_daily(self, record):
        ops = self.host.ops
        with ops.release_lock(self.args.install_root):
            self.current()
            live = self.check_nightly(record)
            operation, binding = self.combined_operations()
            require(self.original_daily_state(record, operation)[0], NIGHTLY_ERROR)
            self.daily_snapshot = self.verify_active_daily(operation, binding, live)
            generation = live[BETA]["generation"]
            beta = {"identity": generation[0], "created": generation[1], "started_at": generation[2],
                    "restart_count": generation[3], "image": live[BETA]["image"],
                    "restart_policy": live[BETA]["restart_policy"]}

            def body(_):
                self.daily_drain_started = True
                self.host.runtime.drain(self.docker, binding, self.daily_snapshot)
                ops.global_cohort(self.docker, binding)
                ops.stopped(self.docker, binding, self.daily_snapshot)
                self.original_daily_state(record, operation)
                self.check_nightly(record, stopped=(BETA,))

            return self.execute(record, "stop-beta", NIGHTLY_ERROR, body, beta=beta)

    def step_backup(self, record):
        beta = record["beta"]
        self.no_pending("PREUPGRADE_PENDING")
        self.stopped_beta(beta, [rg.STOPPED_POLICY], "PREUPGRADE_BETA_CHANGED")
        self.tool()
        if "nightly" in record:
            self.check_nightly(record, stopped=(BETA,))

        def body(_):
            code, lines = self.pinned(self.backup_operation("backup"), "apply", TIMEOUTS["backup"], record=record)
            expected = "DAILY_BACKUP: NIGHTLY_COMBINED_VERIFIED_SOURCE_UNCHANGED" if self.combined else BACKUP_LINE
            paper_configured_off = record.get("nightly", {}).get("paper_configured_off") is True
            archive = origin = None
            if paper_configured_off:
                require(code == 0 and lines is not None and len(lines) == 3 and lines[0] == PAPER_BACKUP_LINE
                        and lines[1].startswith(PAPER_ARCHIVE_PREFIX) and lines[2].startswith(PAPER_ORIGIN_PREFIX),
                        "PREUPGRADE_BACKUP_STEP")
                archive = lines[1][len(PAPER_ARCHIVE_PREFIX):]
                origin = lines[2][len(PAPER_ORIGIN_PREFIX):]
                require(PAPER_ARCHIVE_DIGEST.fullmatch(archive) and origin == record["nightly"]["paper_preflight_origin"],
                        "PREUPGRADE_BACKUP_STEP")
            else:
                require(code == 0 and lines == [expected], "PREUPGRADE_BACKUP_STEP")
            handoff = self.handoff(("switched",), "PREUPGRADE_BACKUP_STEP")
            if "nightly" in record:
                self.check_nightly_handoff(record, handoff)
            self.stopped_beta(beta, [rg.STOPPED_POLICY], "PREUPGRADE_BETA_CHANGED")
            if archive is not None:
                return {"nightly_paper_archive": archive, "nightly_paper_origin": origin}
        return self.execute(record, "backup", "PREUPGRADE_BACKUP_STEP", body)

    def step_verify(self, record):
        beta = record["beta"]
        before = self.handoff(("switched",), "PREUPGRADE_PENDING")
        self.stopped_beta(beta, [rg.STOPPED_POLICY], "PREUPGRADE_BETA_CHANGED")
        self.tool()
        if "nightly" in record:
            self.check_nightly_handoff(record, before)

        def body(_):
            code, lines = self.pinned(self.backup_operation("verify"), "apply", TIMEOUTS["verify"], record=record)
            expected = "DAILY_BACKUP: NIGHTLY_COMBINED_VERIFIED_ISOLATED_DISABLED" if self.combined else VERIFY_LINE
            if record.get("nightly", {}).get("paper_configured_off") is True:
                require(code == 0 and lines == [PAPER_VERIFY_LINE,
                        PAPER_ARCHIVE_PREFIX + record["nightly_paper_archive"],
                        PAPER_ORIGIN_PREFIX + record["nightly_paper_origin"]], "PREUPGRADE_VERIFY_STEP")
            else:
                require(code == 0 and lines == [expected], "PREUPGRADE_VERIFY_STEP")
            require(self.handoff(("switched",), "PREUPGRADE_VERIFY_STEP") == before, "PREUPGRADE_VERIFY_STEP")
            self.stopped_beta(beta, [rg.STOPPED_POLICY], "PREUPGRADE_BETA_CHANGED")
        return self.execute(record, "verify", "PREUPGRADE_VERIFY_STEP", body)

    def step_restore_beta_policy(self, record):
        beta = record["beta"]
        handoff = self.handoff(("switched",), "PREUPGRADE_PENDING")
        if "nightly" in record:
            self.check_nightly_handoff(record, handoff)
        self.stopped_beta(beta, [rg.STOPPED_POLICY, beta["restart_policy"]], "PREUPGRADE_BETA_CHANGED")
        if self.combined and record["nightly"]["daily_enabled"]:
            self.stopped_beta(beta, [rg.STOPPED_POLICY], "PREUPGRADE_BETA_CHANGED")
            return self.execute(record, "restore-beta-policy", NIGHTLY_ERROR, lambda _: None)
        argument = rg.restart_argument(beta["restart_policy"], "PREUPGRADE_BETA_RESTORE")

        def body(_):
            self.wrap("PREUPGRADE_BETA_RESTORE", self.docker.call, ["update", argument, beta["identity"]], 15)
            self.stopped_beta(beta, [beta["restart_policy"]], "PREUPGRADE_BETA_RESTORE")
        return self.execute(record, "restore-beta-policy", "PREUPGRADE_BETA_RESTORE", body)

    def step_resume_release(self, record):
        beta = record["beta"]
        self.current()
        handoff = self.handoff(("switched",), "PREUPGRADE_PENDING")
        captured = self.worker_decision(handoff)
        nightly = "nightly" in record
        intent = {WORKER_DECISION: captured}
        daily_api = self.combined and record["nightly"]["daily_api"]
        if nightly:
            self.nightly_targets(daily_api=daily_api)
            self.check_nightly_handoff(record, handoff)
            attempt = handoff.get("api_attempt")
            require(isinstance(attempt, str) and re.fullmatch(r"[0-9a-f]{32}", attempt)
                    and record.get(API_ATTEMPT, attempt) == attempt, NIGHTLY_ERROR)
            intent[API_ATTEMPT] = attempt
        # A retry sees the same handoff; a different answer means the evidence changed.
        previous = record.get(WORKER_DECISION)
        require(previous is None or previous is captured, "PREUPGRADE_WORKER_DECISION")
        expect_stopped = self.combined and record["nightly"]["daily_enabled"]
        policy = rg.STOPPED_POLICY if expect_stopped else beta["restart_policy"]
        self.stopped_beta(beta, [policy], "PREUPGRADE_BETA_CHANGED")
        self.check_confirmations()

        def body(_):
            kwargs = {"daily_api": True} if daily_api else {}
            configured_off = self.combined and record["nightly"].get("paper_configured_off") is True
            if expect_stopped or configured_off:
                kwargs["daily_control"] = (expect_stopped, record["nightly"]["daily_epoch"])
            if configured_off:
                origin = record.get("nightly_paper_origin")
                require(self.states(record)["verify"] == "done" and type(origin) is str
                        and origin == record["nightly"].get("paper_preflight_origin"), NIGHTLY_ERROR)
                kwargs["paper_origin"] = origin
            code, lines = self.wrapper("--simulation-release-start", TIMEOUTS["release"], api_only=nightly, **kwargs)
            require(code == 0 and lines and lines[-1] == (NIGHTLY_API_LINE if nightly else RELEASE_LINE),
                    "PREUPGRADE_RESUME_STEP")
            # Done once the API start is recorded; the Beta health check is verify-beta.
            after = self.handoff(("api-started",), "PREUPGRADE_RESUME_STEP", absent_ok=True)
            if nightly:
                if captured:
                    require(after is not None, NIGHTLY_ERROR)
                    self.check_nightly_handoff(record, after, api_started=True)
                else:
                    require(after is None, NIGHTLY_ERROR)
                    self.check_nightly(record, stopped=(BETA,), resumed=(API,))
        return self.execute(record, "resume-release", "PREUPGRADE_RESUME_STEP", body,
                            **intent)

    def check_api_attempt(self, record, live):
        expected = record.get(API_ATTEMPT)
        require(isinstance(expected, str) and re.fullmatch(r"[0-9a-f]{32}", expected), NIGHTLY_ERROR)
        generation = live[API]["generation"]
        value = self.wrap(NIGHTLY_ERROR, self.docker.object, "container", generation[0], 15)
        require([value["Id"], value["Created"], value["State"]["StartedAt"], value["RestartCount"]] == generation
                and value["Config"]["Labels"].get("org.lagrange.api-release-attempt") == expected
                and rg.running_healthy(value), NIGHTLY_ERROR)

    def step_resume_simulation(self, record):
        self.current()
        if not self.recorded_decision(record):
            if "nightly" in record:
                live = self.check_nightly(record, stopped=(BETA,), resumed=(API,))
                self.check_api_attempt(record, live)
            return self.skip_resume_simulation(record)
        handoff = self.handoff(("api-started",), "PREUPGRADE_PENDING", absent_ok=True)
        intent = {}
        if "nightly" in record:
            require(handoff is not None and handoff.get("api_attempt") == record.get(API_ATTEMPT), NIGHTLY_ERROR)
            self.check_nightly_handoff(record, handoff, api_started=True)
            intent[RESUMED_API] = handoff["target_api_generation"]

        def body(_):
            code, lines = self.wrapper("--simulation-start", TIMEOUTS["simulation"])
            require(code == 0 and lines and lines[-1] == SIMULATION_LINE, "PREUPGRADE_RESUME_STEP")
            self.no_pending("PREUPGRADE_RESUME_STEP")
            if "nightly" in record:
                self.check_nightly(record, stopped=(BETA,), resumed=(API, SIMULATION_RUNNER))
        return self.execute(record, "resume-simulation", "PREUPGRADE_RESUME_STEP", body, **intent)

    def verify_nightly_worker(self, generation):
        a, code = self.args, NIGHTLY_ERROR
        release = "releases/" + a.application_commit
        controller = rg.load_module("preupgrade_nightly_simulation", a.install_root,
                                    release + "/scripts/ops/lib/owner-beta-simulation-operations.py", self.uid, code)
        sim_docker = controller.runtime.Docker()
        context, _ = controller.inspect_transition(a.install_root, a.install_root + "/" + release,
                                                   a.application_commit, self.docker, sim_docker)
        require(context.source_release == context.target_release and context.binding is not None, code)
        worker = controller.verify_runtime_policy(self.docker, sim_docker, context.binding,
                                                 context.values["LAGRANGE_RUNTIME_SECRET_DIR"],
                                                 controller.daily.database_name(context.values))
        require(list(worker.generation) == generation, code)

    def step_observe_simulation_resume(self, record):
        def body(_):
            with self.nightly_restore_lock(record):
                resumed = [API, SIMULATION_RUNNER]
                beta = self.nightly_services()[BETA]
                if beta and beta["running"]:
                    resumed.append(BETA)
                stopped = () if BETA in resumed else (BETA,)
                live = self.check_nightly(record, stopped=stopped, resumed=resumed)
                require(live[API]["generation"] == record.get(RESUMED_API), NIGHTLY_ERROR)
                self.check_api_attempt(record, live)
                self.wrap(NIGHTLY_ERROR, self.verify_nightly_worker, live[SIMULATION_RUNNER]["generation"])
                require(self.check_nightly(record, stopped=stopped, resumed=resumed) == live, NIGHTLY_ERROR)
                self.no_pending(NIGHTLY_ERROR)
                return {OBSERVED_WORKER: live[SIMULATION_RUNNER]["generation"]}

        return self.execute(record, "observe-simulation-resume", NIGHTLY_ERROR, body)

    def check_observed_resume(self, record, live):
        if OBSERVED_WORKER in record:
            require(live[API]["generation"] == record.get(RESUMED_API)
                    and live[SIMULATION_RUNNER]["generation"] == record[OBSERVED_WORKER], NIGHTLY_ERROR)

    def skip_resume_simulation(self, record):
        """No worker was captured, so none was running: start nothing, prove nothing exists.

        Read-only; the guards run before the step is journaled, so a refusal stays retryable.
        """
        self.no_pending("PREUPGRADE_PENDING")
        self.no_simulation_runner()
        record = self.execute(record, "resume-simulation", "PREUPGRADE_RESUME_STEP", lambda _: None)
        print(SKIP_LINE, file=self.out)
        return record

    def step_verify_beta(self, record):
        beta = record["beta"]
        self.no_pending("PREUPGRADE_PENDING")
        if self.combined and record["nightly"]["daily_enabled"]:
            self.stopped_beta(beta, [rg.STOPPED_POLICY], "PREUPGRADE_BETA_CHANGED")
            return self.execute(record, "verify-beta", NIGHTLY_ERROR, lambda _: None)

        def body(_):
            if "nightly" in record:
                with self.nightly_restore_lock(record):
                    resumed = [API, *([SIMULATION_RUNNER] if self.recorded_decision(record) else [])]
                    live_beta = self.nightly_services()[BETA]
                    if not self.args.nightly and live_beta and live_beta["running"]:
                        live = self.check_nightly(record, resumed=[*resumed, BETA])
                    else:
                        live = self.check_nightly(record, stopped=(BETA,), resumed=resumed)
                        self.check_observed_resume(record, live)
                        self.restore_recorded_beta(beta)
                        live = self.check_nightly(record, resumed=[*resumed, BETA])
                    self.check_observed_resume(record, live)
                return {"beta_generation_reused": True}
            return {"beta_generation_reused": self.resumed_beta(beta)}
        return self.execute(record, "verify-beta", "PREUPGRADE_RESUME_STEP", body)

    def nightly_restored_services(self, record):
        services = record["nightly"]["services"]
        return (API, BETA, SIMULATION_RUNNER) if services[SIMULATION_RUNNER] else (API, BETA)

    def resume_nightly_daily(self, record):
        """Resume the prior healthy Daily writers last, at their existing epoch."""
        with self.nightly_restore_lock(record):
            operation, binding = self.combined_operations()
            active, _ = self.original_daily_state(record, operation)
            restored = self.nightly_restored_services(record)
            resumed = tuple(name for name in restored if not active or name != BETA)
            stopped = (BETA,) if active else ()
            live = self.check_nightly(record, stopped=stopped, resumed=resumed)
            self.check_observed_resume(record, live)
            require(self.daily_api_mode(operation, binding, live) == record["nightly"]["daily_api"] and
                    self.timer_active() == record["nightly"]["observer_timer_active"], NIGHTLY_ERROR)

            def body(_):
                if active:
                    require(self.daily_snapshot is not None, NIGHTLY_ERROR)
                    self.host.ops.global_cohort(self.docker, binding)
                    self.host.ops.stopped(self.docker, binding, self.daily_snapshot)
                    self.original_daily_state(record, operation)
                    self.host.runtime.resume_after_backup(self.docker, binding, self.daily_snapshot)
                    live = self.check_nightly(record, resumed=restored)
                    self.verify_active_daily(operation, binding, live)
                self.original_daily_state(record, operation)

            return self.execute(record, DAILY_RESUME_STEP, NIGHTLY_ERROR, body)

    def fail_combined_daily_off(self):
        if not self.daily_drain_started:
            return
        with self.host.ops.release_lock(self.args.install_root):
            self.current()
            operation, binding = self.combined_operations()
            target = operation.target
            # Even if a replacement appeared, disable the authenticated current
            # control. Only original Docker identities may be drained.
            try:
                operation.database.control(target.release, target.commit, target.values, "disable")
            finally:
                self.host.ops.global_cohort(self.docker, binding)
                current = self.host.runtime.capture(self.docker, binding)
                require(self.daily_snapshot is not None and set(current) <= set(self.daily_snapshot) and all(
                    rg.same_generation(entry["generation"], self.daily_snapshot[name]["generation"])
                    for name, entry in current.items()), NIGHTLY_ERROR)
                self.host.runtime.drain(self.docker, binding, current)

    def restore_recorded_beta(self, beta):
        code = "PREUPGRADE_BETA_CHANGED"
        own = self.worker_ownership(beta["image"])
        self.wrap(code, self.host.global_cohort, own)
        recorded = [beta["identity"], beta["created"], beta["started_at"], beta["restart_count"]]

        def observe():
            require(rg.compose_ids(self.docker, own.project, BETA, code) == [beta["identity"]], code)
            value = self.wrap(code, self.docker.object, "container", beta["identity"], 15)
            return value, list(self.wrap(code, self.host.owned, value, own, BETA, beta["identity"]))

        return rg.restore_generation(
            self.docker, observe, recorded, beta["restart_policy"],
            signalled=lambda v: rg.restart_policy(v, "PREUPGRADE_BETA_RESTORE") != beta["restart_policy"],
            clock=self.clock, sleep=self.sleep, health_seconds=BETA_HEALTH_SECONDS,
            poll_seconds=POLL_SECONDS,
            codes={"changed": code, "state": "PREUPGRADE_BETA_RESTORE", "policy": "PREUPGRADE_BETA_RESTORE",
                   "start": "PREUPGRADE_BETA_RESTORE", "health": "PREUPGRADE_BETA_RESTORE"})

    def step_recover_beta(self, record):
        """Return the recorded Beta generation to service when no handoff exists."""
        beta = record["beta"]
        require(beta is not None, "PREUPGRADE_PROGRESS")
        self.no_pending("PREUPGRADE_PENDING")

        def body(_):
            with self.nightly_restore_lock(record):
                return {"beta_recovery": self.restore_recorded_beta(beta)}

        return self.execute(record, "recover-beta", "PREUPGRADE_BETA_RESTORE", body)

    def step_recover_handoff(self, record):
        code = "PREUPGRADE_RECOVER_HANDOFF"
        handoff = self.handoff(("fencing", "prepared"), code)
        self.check_nightly_handoff(record, handoff)
        self.stopped_beta(record["beta"], [rg.STOPPED_POLICY], code)

        def body(_):
            status, lines = self.wrapper("--simulation-drain", TIMEOUTS["simulation"])
            require(status == 0 and lines and lines[-1] == SIMULATION_DRAINED_LINE, code)
            switched = self.handoff(("switched",), code)
            require(switched == dict(handoff, phase="switched"), code)
            self.check_nightly_handoff(record, switched)

        return self.execute(record, "recover-handoff", code, body)

    def step_restore_observer(self, record):
        def body(current):
            if current["observer"]["timer_was_active"]:
                if "nightly" not in current or not self.timer_active():
                    self.systemctl("start", TIMER)
                require(self.unit_state(TIMER)[1] == "active", "PREUPGRADE_OBSERVER")
        complete = all((record["steps"].get(n) or {}).get("state") == "done" for n in STEPS[:-1])
        record = self.execute(record, "restore-observer", "PREUPGRADE_OBSERVER", body,
                              final_state="done" if complete else "recovered")
        if not complete:
            # The timer runs again before the sequence is complete: un-journal the pause
            # so every later mutating step first re-runs pause-observer (MUTATING rule).
            steps = dict(record["steps"])
            steps.pop("pause-observer", None)
            record = self.progress.replace(record, dict(record, steps=steps))
        return record

    # -- modes ---------------------------------------------------------------
    def pinned_plan(self):
        code, lines = self.pinned(self.backup_operation("backup"), "plan", TIMEOUTS["plan"])
        prefix = "NIGHTLY_COMBINED_PLAN: " if self.combined else PLAN_LINE
        require(code == 0 and lines is not None and len(lines) == 1 and lines[0].startswith(prefix),
                "PREUPGRADE_TOOL")
        if self.args.nightly and not self.combined:
            require(lines[0].endswith(NIGHTLY_PLAN_SUFFIX), "PREUPGRADE_TOOL")

    def preflight(self):
        # The installed wrapper's release start is run without --daily-from-commit, so
        # its Daily guard binds the Beta to the application release: only a Beta from
        # that same release is supported here.
        require(self.args.stopped_worker_from_commit == self.args.application_commit,
                "PREUPGRADE_WORKER_SOURCE")
        self.current()
        self.no_pending("PREUPGRADE_PENDING")
        self.tool()
        if self.args.nightly:
            self.nightly_tool()
        if not self.combined:
            self.running_beta()
        timer = self.timer_active()
        # A collector already in "failed" would block step 0 with the timer stopped.
        require(self.unit_state(OBSERVER_SERVICE)[1] in ("inactive", "active", "activating", "deactivating"),
                "PREUPGRADE_OBSERVER")
        self.pinned_plan()
        code, lines = self.pinned(self.backup_operation("backup"), "preflight", TIMEOUTS["preflight"])
        paper_origin = None
        if (self.combined and lines is not None and len(lines) == 3
                and lines[:2] == [PREFLIGHT_LINE, PAPER_PREFLIGHT_LINE] and lines[2].startswith(PAPER_ORIGIN_PREFIX)):
            paper_origin = lines[2][len(PAPER_ORIGIN_PREFIX):]
            require(PAPER_ARCHIVE_DIGEST.fullmatch(paper_origin), "PREUPGRADE_TOOL")
        paper_configured_off = paper_origin is not None
        require(code == 0 and (lines == [PREFLIGHT_LINE] or paper_configured_off), "PREUPGRADE_TOOL")
        expected_paper = self.args.expected_nightly_paper_profile
        require(expected_paper is None or (expected_paper == "CONFIGURED_OFF") == paper_configured_off,
                NIGHTLY_ERROR)
        expected_origin = self.args.expected_nightly_paper_origin
        require(expected_origin is None or expected_origin == paper_origin, NIGHTLY_ERROR)
        required = self.check_confirmations()
        if self.args.nightly:
            self.nightly_capture = self.capture_nightly(timer)
            if paper_configured_off:
                self.nightly_capture.update(paper_configured_off=True, paper_preflight_origin=paper_origin)
        return timer, required

    def states(self, record):
        return {name: (record["steps"].get(name) or {}).get("state") for name in JOURNALED}

    def allowed(self, name, record):
        """Journal precondition; each step's own guards then check the observed state."""
        st = self.states(record)
        if self.args.nightly and name in ("restore-beta-policy", "resume-release", "resume-simulation",
                                          "verify-beta", "restore-observer") and st["verify"] != "done":
            return False
        if name in MUTATING and st["pause-observer"] != "done":
            return False
        return {
            "pause-observer": True,
            "stop-beta": True,
            "backup": st["stop-beta"] == "done",
            "verify": st["backup"] == "done",
            # Gated on observed state, not on backup/verify: after a failed backup or
            # verify that left a "switched" handoff, the API can still be resumed.
            "restore-beta-policy": st["stop-beta"] == "done",
            "resume-release": st["restore-beta-policy"] == "done",
            "resume-simulation": (st["resume-release"] in ("done", "failed") or
                                  ("nightly" in record and self.args.step == "recover"
                                   and st["resume-release"] == "started")),
            "verify-beta": st["resume-simulation"] == "done" or st["observe-simulation-resume"] == "done",
            "observe-simulation-resume": ("nightly" in record and self.args.step == "recover"
                                          and record.get(WORKER_DECISION) is True
                                          and st["resume-simulation"] in ("started", "failed")),
            "recover-beta": st["stop-beta"] in ("started", "failed", "done") and st["resume-release"] is None,
            "recover-handoff": ("nightly" in record and self.args.step == "recover"
                                and st["backup"] in ("started", "failed", "done")
                                and st["resume-release"] is None),
            "restore-observer": record["observer"] is not None
                                and st["pause-observer"] in ("started", "done", "failed"),
        }[name]

    def api_started(self):
        if self.pending(START_PENDING) is not None:
            return False
        handoff = self.pending(HANDOFF_PENDING)
        commit = self.args.application_commit
        return handoff is None or (handoff.get("phase") == "api-started"
                                   and handoff.get("source_commit") == commit
                                   and handoff.get("target_commit") == commit)

    def recovery_plan(self, record):
        st = self.states(record)
        require(self.pending(START_PENDING) is None, "PREUPGRADE_PENDING")
        handoff = self.pending(HANDOFF_PENDING)
        if handoff is None:
            if st["resume-release"] is not None:
                worker_step = "resume-simulation"
                if ("nightly" in record and record.get(WORKER_DECISION) is True
                        and st["resume-simulation"] in ("started", "failed")):
                    worker_step = "observe-simulation-resume"
                middle = [worker_step, "verify-beta"]
            else:
                middle = ["recover-beta"] if st["stop-beta"] is not None else []
        else:
            commit = self.args.application_commit
            require(handoff.get("source_commit") == commit and handoff.get("target_commit") == commit,
                    "PREUPGRADE_PENDING")
            phase = handoff.get("phase")
            if phase in ("switched", "fencing", "prepared"):
                middle = ["restore-beta-policy", "resume-release", "resume-simulation", "verify-beta"]
                if phase != "switched":
                    require("nightly" in record, "PREUPGRADE_PENDING")
                    middle.insert(0, "recover-handoff")
            else:
                # Any other phase belongs to the wrapper's own reconcile commands.
                require(phase == "api-started", "PREUPGRADE_PENDING")
                middle = ["resume-simulation", "verify-beta"]
        middle = [n for n in middle if st[n] != "done"]
        if not middle and st["restore-observer"] in ("done", "recovered"):
            return []
        return ["pause-observer", *middle, "restore-observer"]

    def selection(self, record):
        st = self.states(record)
        selected = self.args.step
        wanted = self.recovery_plan(record) if selected == "recover" else GROUPS[selected]
        result = []
        for name in wanted:
            if st[name] == "done":
                continue
            if name == "resume-release" and st[name] == "failed" and self.api_started():
                # The wrapper already moved past "switched": the API start happened and
                # resume-simulation follows.  The failure stays recorded (no PASS).
                continue
            # A started-but-unfinished or failed final step is never re-run.
            require(st[name] in (None, "recovered") or name in RETRYABLE, "PREUPGRADE_PROGRESS")
            result.append(name)
        require(result or selected in ("all", "recover"), "PREUPGRADE_PROGRESS")
        return result

    def readback_nightly_paper(self, record, *, before_backup=False):
        preflight_origin = record["nightly"].get("paper_preflight_origin")
        origin = preflight_origin if before_backup else record.get("nightly_paper_origin")
        require(type(origin) is str and PAPER_ARCHIVE_DIGEST.fullmatch(origin) and origin == preflight_origin,
                NIGHTLY_ERROR)
        code, lines = self.pinned("nightly-combined-paper-readback", "readback", TIMEOUTS["preflight"],
                                  record=record, paper_origin=origin)
        require(code == 0 and lines == [PAPER_READBACK_LINE], NIGHTLY_ERROR)

    def verify_manual_combined_recovery(self, record):
        # This observes the operator's recovered originals and starts nothing.
        require(self.args.step == "recover" and "nightly" in record, NIGHTLY_ERROR)
        steps = record["steps"]
        if not steps:
            require(set(record) == {"schema", "arguments", "steps", "observer", "beta", "nightly"}
                    and record["observer"] is None and record["beta"] is None, NIGHTLY_ERROR)
        with self.nightly_restore_lock(record):
            if record["nightly"]:
                resumed = self.nightly_restored_services(record) if steps else ()
                live = self.check_nightly(record, resumed=resumed)
                operation, binding = self.combined_operations()
                active = record["nightly"]["daily_enabled"]
                enabled, epoch = self.daily_state(operation)
                require(enabled is active and epoch >= record["nightly"]["daily_epoch"] and
                        self.daily_api_mode(operation, binding, live) == record["nightly"]["daily_api"] and
                        self.timer_active() == record["nightly"]["observer_timer_active"], NIGHTLY_ERROR)
                if active:
                    self.verify_active_daily(operation, binding, live)
            else:
                require(not record["steps"], NIGHTLY_ERROR)
        recovered = dict(record, nightly_recovered=True)
        if record["nightly"].get("paper_configured_off") is True:
            backup = self.states(record)["backup"]
            if backup is not None:
                # An interrupted backup may already have published an archive.
                # The absence of an exact receipt cannot prove it never ran.
                archive = record.get("nightly_paper_archive")
                require(backup == "done" and type(archive) is str and PAPER_ARCHIVE_DIGEST.fullmatch(archive),
                        NIGHTLY_ERROR)
            else:
                require(set(record["steps"]) <= {"pause-observer", "stop-beta"}
                        and "nightly_paper_archive" not in record and "nightly_paper_origin" not in record,
                        NIGHTLY_ERROR)
            # The pinned readback acquires its own release lock and validates
            # the captured source without mutating or restarting any service.
            self.readback_nightly_paper(record, before_backup=backup is None)
            recovered["nightly_paper_origin_unchanged"] = True
        # The scheduler acknowledges an early refusal against the original bytes.
        if steps:
            self.progress.replace(record, recovered)
        return "RECOVERED"

    def apply(self):
        record = self.progress.read()
        if record is None:
            require(self.args.step in ("all", "pause-observer"), "PREUPGRADE_PROGRESS")
            record = {"schema": SCHEMA, "arguments": arguments_record(self.args),
                      "steps": {}, "observer": None, "beta": None}
            if self.args.nightly:
                # A failure in the repeated preflight leaves proof that no step began.
                record = self.progress.create(dict(record, nightly={}))
                self.preflight()
                record = self.progress.replace(record, dict(record, nightly=self.nightly_capture))
            else:
                self.preflight()
                record = self.progress.create(record)
        else:
            require(record.get("schema") == SCHEMA and record.get("arguments") == arguments_record(self.args)
                    and isinstance(record.get("steps"), dict) and not self.args.nightly
                    and ("nightly" not in record or self.args.step == "recover"), "PREUPGRADE_PROGRESS")
            self.current()
            if self.combined:
                return self.verify_manual_combined_recovery(record)
            if "nightly" in record and not record["steps"]:
                self.no_pending(NIGHTLY_ERROR)
                if record["nightly"]:
                    self.check_nightly(record)
                    require(self.timer_active() == record["nightly"]["observer_timer_active"], NIGHTLY_ERROR)
                return "RECOVERED"
        for name in self.selection(record):
            try:
                require(self.allowed(name, record), "PREUPGRADE_PROGRESS")
                record = getattr(self, "step_" + name.replace("-", "_"))(record)
            except ToolError as exc:
                if "nightly" in record:
                    latest = self.progress.read()
                    if self.states(latest)[name] != "failed":
                        self.mark(latest, name, "failed", exc.code)
                raise
        if "nightly" in record:
            restored = self.nightly_restored_services(record)
            try:
                self.no_pending(NIGHTLY_ERROR)
                if self.combined and record["nightly"]["daily_enabled"]:
                    others = tuple(service for service in restored if service != BETA)
                    live = self.check_nightly(record, stopped=(BETA,), resumed=others)
                else:
                    live = self.check_nightly(record, resumed=restored)
                self.check_observed_resume(record, live)
                require(self.timer_active() == record["nightly"]["observer_timer_active"], NIGHTLY_ERROR)
            except ToolError as exc:
                self.progress.replace(record, dict(record, nightly_final_failure=exc.code))
                raise
            if not self.args.nightly:
                self.progress.replace(record, dict(record, nightly_recovered=True))
                return "RECOVERED"
            require(all(self.states(record)[name] == "done" for name in STEPS), "PREUPGRADE_PROGRESS")
            if self.combined:
                record = self.resume_nightly_daily(record)
                if record["nightly"].get("paper_configured_off") is True:
                    try:
                        self.readback_nightly_paper(record)
                    except ToolError:
                        self.progress.replace(record, dict(record, nightly_final_failure=NIGHTLY_ERROR))
                        raise
                    record = self.progress.replace(record, dict(record, nightly_paper_origin_unchanged=True))
            record = self.progress.replace(record, dict(record, nightly_completed=True))
        if all(self.states(record)[n] == "done" for n in STEPS):
            captured = record.get(WORKER_DECISION)
            require(captured is None or type(captured) is bool, "PREUPGRADE_PROGRESS")
            return "PASS" + (PASS_SKIPPED_FIELD if captured is False else "")
        if self.args.step in ("recover", "all"):
            # Returned to normal without every normal step completing.
            return "RECOVERED"
        return "STEP " + self.args.step


PLAN_LINES = (
    "PREUPGRADE_BACKUP: PLAN",
    "  0 pause-observer: stop the status timer only if active; wait for the collector to finish",
    "  1 stop-beta: drain the exact owner-beta-runner generation (installed release drain; exit 0; restart=no)",
    "  2 backup: pinned inactive-simulation-backup apply",
    "  3 verify: pinned inactive-simulation-verify apply",
    "  4 restore-beta-policy: recorded restart policy on the exact stopped generation; not started",
    "  5 resume: installed wrapper --simulation-release-start, then --simulation-start only if the backup",
    "    captured a simulation worker (otherwise a read-only no-runner check); Beta healthy check",
    "  6 restore-observer: start the status timer only if step 0 stopped it",
    "  recover: after a failure, return to normal from the observed handoff (prints RECOVERED, never PASS)",
    "PLAN_ONLY: pinned plan only; no Docker, systemd or state write",
)


def main(argv=None, *, host_factory=None, uid=None, runner=None, environ=None, out=None, err=None,
         clock=time.monotonic, sleep=time.sleep):
    out = sys.stdout if out is None else out
    err = sys.stderr if err is None else err
    argv = list(sys.argv[1:] if argv is None else argv)
    environ = os.environ if environ is None else environ
    runner = rg.run_child if runner is None else runner
    sequencer = None
    try:
        args = parse(argv)
        production = host_factory is None
        uid = 0 if production else (os.geteuid() if uid is None else uid)
        if args.mode == "plan":
            sequencer = Sequencer(args, None, uid=uid, runner=runner, environ=environ, err=err,
                                  clock=clock, sleep=sleep)
            sequencer.pinned_plan()
            for line in PLAN_LINES:
                print(line, file=out)
            return 0
        try:
            host = (rg.ReleaseHost(args.install_root, args.application_commit, "PREUPGRADE_ROOT")
                    if production else host_factory(args))
        except ToolError:
            raise
        except Exception as exc:
            raise ToolError("PREUPGRADE_ROOT", rg.cause_of(exc)) from None
        sequencer = Sequencer(args, host, uid=uid, runner=runner, environ=environ, err=err,
                              clock=clock, sleep=sleep, out=out)
        rg.state_directory(args.state_dir, uid, "PREUPGRADE_PROGRESS")
        sequencer.progress = rg.Progress(args.state_dir, PROGRESS_NAME, uid, "PREUPGRADE_PROGRESS")
        if args.mode == "preflight":
            require(sequencer.progress.read() is None, "PREUPGRADE_PROGRESS")
            timer, required = sequencer.preflight()
            print("PREUPGRADE_BACKUP: OBSERVER timer=" + ("active" if timer else "inactive"), file=out)
            print("PREUPGRADE_BACKUP: CONFIRMATIONS owner_equity_v2="
                  + ("required" if CONFIRMATIONS[0] in required else "not_required")
                  + " owner_market_stream=" + ("required" if CONFIRMATIONS[1] in required else "not_required"),
                  file=out)
            if args.nightly:
                capture = sequencer.nightly_capture
                services = capture["services"]
                worker, v2 = services[SIMULATION_RUNNER], services[V2]
                owner_state = "absent" if v2 is None else ("running" if v2["running"] else "stopped")
                print("PREUPGRADE_BACKUP: NIGHTLY_COHORT simulation=" + ("running" if worker else "absent")
                      + " owner_v2=" + owner_state, file=out)
                print("PREUPGRADE_BACKUP: NIGHTLY_UNTOUCHED_SHA256 " + sequencer.nightly_untouched_digest(capture),
                      file=out)
                if sequencer.combined:
                    print("PREUPGRADE_BACKUP: NIGHTLY_DAILY_COHORT "
                          + ("enabled" if capture["daily_enabled"] else "disabled")
                          + " api=" + ("owner_only" if capture["daily_api"] else "disabled")
                          + f" epoch={capture['daily_epoch']}", file=out)
                    if capture.get("paper_configured_off") is True:
                        print("PREUPGRADE_BACKUP: NIGHTLY_PAPER CONFIGURED_OFF " + capture["paper_preflight_origin"],
                              file=out)
            print("PREUPGRADE_BACKUP: PREFLIGHT", file=out)
            return 0
        sequencer.progress.lock()
        try:
            outcome = sequencer.apply()
        except BaseException:
            try:
                sequencer.fail_combined_daily_off()
            except Exception:
                try:
                    record = sequencer.progress.read()
                    if record is not None:
                        sequencer.progress.replace(record, dict(record, nightly_daily_cleanup="unknown"))
                except Exception:
                    pass  # The original attempted step remains unresolved.
            raise
        print("PREUPGRADE_BACKUP: " + outcome, file=out)
        return 0
    except ToolError as exc:
        print("PREUPGRADE_BACKUP: FAILED " + exc.code, file=err)
        if exc.cause:
            print("PREUPGRADE_BACKUP: CAUSE " + exc.cause, file=err)
        return 1
    except KeyboardInterrupt:
        print("PREUPGRADE_BACKUP: FAILED PREUPGRADE_INTERRUPTED", file=err)
        return 1
    except Exception:
        print("PREUPGRADE_BACKUP: FAILED PREUPGRADE_INTERNAL", file=err)
        return 1
    finally:
        if sequencer is not None and sequencer.progress is not None:
            sequencer.progress.unlock()


if __name__ == "__main__":
    raise SystemExit(main())
