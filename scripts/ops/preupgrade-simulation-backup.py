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
  5 resume               installed release wrapper --simulation-release-start, then
                         --simulation-start (operator's rollout confirmations passed through),
                         then a read-only check that the Beta is healthy with its policy
  6 restore-observer     start the timer again only if step 0 stopped it

Every step is journaled to a root 0600 progress file in the explicit --state-dir before
it mutates anything.  stop-beta, backup and resume-simulation are final once started;
the other steps may be re-run, and their observed-state guards run before they are
journaled again.  ``--step recover`` returns the host to normal after a failure, driven
by the observed simulation handoff: with a ``switched``/``api-started`` handoff it runs
the remaining restore/resume steps; without a handoff it restores the recorded Beta
generation (``recover-beta``).  A recovery prints RECOVERED, never PASS; PASS means every
normal step completed.  Mutating steps run only while step 0 is done (the timer paused).

It never runs --daily-enable/--daily-disable or a reconcile, never deletes a
pending/handoff file, never calls docker rm/stop or SIGKILL on a container, never
enables a KIS timer, and never synthesizes a rollout confirmation.  Output is value-free.
"""
from __future__ import annotations

import importlib.util
import os
import re
import stat
import sys
import time
from pathlib import Path

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
PENDING_LIMIT = 16384
CONFIRMATIONS = ("OWNER_EQUITY_V2_ROLLOUT_CONFIRM", "OWNER_MARKET_STREAM_ROLLOUT_CONFIRM")
ENTRYPOINT = "scripts/ops/lib/owner-beta-daily-backup.py"
WRAPPER = "scripts/ops/compose-release.sh"

# Normal order.  recover-beta is a recovery-only step outside it.
STEPS = ("pause-observer", "stop-beta", "backup", "verify", "restore-beta-policy",
         "resume-release", "resume-simulation", "verify-beta", "restore-observer")
JOURNALED = STEPS + ("recover-beta",)
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
                       "verify-beta", "recover-beta", "restore-observer"))
# Steps that change Docker, the database or simulation state.  Each requires the
# observer timer to be paused (pause-observer done) when it starts.
MUTATING = frozenset(("stop-beta", "backup", "verify", "restore-beta-policy", "resume-release",
                      "resume-simulation", "recover-beta"))
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
PREFLIGHT_LINE = "DAILY_BACKUP: PREFLIGHT"
BACKUP_LINE = "DAILY_BACKUP: INACTIVE_SIMULATION_VERIFIED_DISABLED"
VERIFY_LINE = "DAILY_BACKUP: INACTIVE_SIMULATION_VERIFIED_ISOLATED_DISABLED"
# Exact final line of the installed 312 compose-release.sh (CR:1063).
RELEASE_LINE = ("COMPOSE_RELEASE: PASS (immutable manifest image IDs bound; run post-backfill-health.sh "
                "--scope release --check to assert data readiness)")
SIMULATION_LINE = "SIMULATION_OPERATION: STARTED"

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
    rg.reject_repeated(argv, ("--plan", "--preflight", "--apply", "--install-root", "--application-commit",
                              "--tool-commit", "--config-file", "--expected-db-first-session",
                              "--expected-config-first-session", "--stopped-worker-from-commit",
                              "--state-dir", "--step"), "PREUPGRADE_ARGUMENTS")
    args = parser.parse_args(argv)
    require(rg.absolute(args.install_root) and rg.absolute(args.config_file) and rg.absolute(args.state_dir)
            and rg.is_commit(args.application_commit) and rg.is_commit(args.tool_commit)
            and rg.is_commit(args.stopped_worker_from_commit)
            and rg.is_date(args.expected_db_first_session) and rg.is_date(args.expected_config_first_session)
            and args.expected_db_first_session < args.expected_config_first_session
            and (args.step is None or args.mode == "apply"), "PREUPGRADE_ARGUMENTS")
    args.step = args.step or "all"
    return args


def arguments_record(args):
    return {"install_root": args.install_root, "application_commit": args.application_commit,
            "tool_commit": args.tool_commit, "config_file": args.config_file,
            "expected_db_first_session": args.expected_db_first_session,
            "expected_config_first_session": args.expected_config_first_session,
            "stopped_worker_from_commit": args.stopped_worker_from_commit}


class Sequencer:
    def __init__(self, args, host, *, uid, runner, environ, err, clock, sleep):
        self.args, self.host, self.uid = args, host, uid
        self.runner, self.environ, self.err = runner, environ, err
        self.clock, self.sleep = clock, sleep
        self.docker = host.docker if host is not None else None
        self.progress = None
        self._project = None

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

    def pinned(self, operation, mode, timeout):
        a = self.args
        argv = ["/usr/bin/python3", "-I", "-S", "-B", self.entrypoint(),
                "--operation", operation, "--mode", mode, "--install-root", a.install_root,
                "--application-release", a.install_root + "/releases/" + a.application_commit,
                "--config-file", a.config_file,
                "--expected-db-first-session", a.expected_db_first_session,
                "--expected-config-first-session", a.expected_config_first_session,
                "--stopped-worker-from-commit", a.stopped_worker_from_commit]
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

    def wrapper(self, flag, timeout):
        a = self.args
        path = rg.trusted_chain(a.install_root, "releases/" + a.application_commit + "/" + WRAPPER,
                                self.uid, "PREUPGRADE_RESUME_STEP")
        # Passed through unchanged when the operator set them; never synthesized.
        env = rg.child_environment({"LAGRANGE_RELEASE_ROOT": a.install_root, **self.confirmations()})
        code, out, err = self.runner(["/bin/bash", "--noprofile", "--norc", path,
                                      "--scope", "release", "--apply", flag], env, timeout)
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
        own, snapshot, beta = self.running_beta()

        def body(current):
            self.wrap("PREUPGRADE_BETA_STOP", self.host.drain, own, snapshot)
            # DBK requires the whole dict; drain itself checks only the name.
            self.stopped_beta(current["beta"], [rg.STOPPED_POLICY], "PREUPGRADE_BETA_STOP")
        return self.execute(record, "stop-beta", "PREUPGRADE_BETA_STOP", body, beta=beta)

    def step_backup(self, record):
        beta = record["beta"]
        self.no_pending("PREUPGRADE_PENDING")
        self.stopped_beta(beta, [rg.STOPPED_POLICY], "PREUPGRADE_BETA_CHANGED")
        self.tool()

        def body(_):
            code, lines = self.pinned("inactive-simulation-backup", "apply", TIMEOUTS["backup"])
            require(code == 0 and lines == [BACKUP_LINE], "PREUPGRADE_BACKUP_STEP")
            self.handoff(("switched",), "PREUPGRADE_BACKUP_STEP")
            self.stopped_beta(beta, [rg.STOPPED_POLICY], "PREUPGRADE_BETA_CHANGED")
        return self.execute(record, "backup", "PREUPGRADE_BACKUP_STEP", body)

    def step_verify(self, record):
        beta = record["beta"]
        before = self.handoff(("switched",), "PREUPGRADE_PENDING")
        self.stopped_beta(beta, [rg.STOPPED_POLICY], "PREUPGRADE_BETA_CHANGED")
        self.tool()

        def body(_):
            code, lines = self.pinned("inactive-simulation-verify", "apply", TIMEOUTS["verify"])
            require(code == 0 and lines == [VERIFY_LINE], "PREUPGRADE_VERIFY_STEP")
            require(self.handoff(("switched",), "PREUPGRADE_VERIFY_STEP") == before, "PREUPGRADE_VERIFY_STEP")
            self.stopped_beta(beta, [rg.STOPPED_POLICY], "PREUPGRADE_BETA_CHANGED")
        return self.execute(record, "verify", "PREUPGRADE_VERIFY_STEP", body)

    def step_restore_beta_policy(self, record):
        beta = record["beta"]
        self.handoff(("switched",), "PREUPGRADE_PENDING")
        self.stopped_beta(beta, [rg.STOPPED_POLICY, beta["restart_policy"]], "PREUPGRADE_BETA_CHANGED")
        argument = rg.restart_argument(beta["restart_policy"], "PREUPGRADE_BETA_RESTORE")

        def body(_):
            self.wrap("PREUPGRADE_BETA_RESTORE", self.docker.call, ["update", argument, beta["identity"]], 15)
            self.stopped_beta(beta, [beta["restart_policy"]], "PREUPGRADE_BETA_RESTORE")
        return self.execute(record, "restore-beta-policy", "PREUPGRADE_BETA_RESTORE", body)

    def step_resume_release(self, record):
        beta = record["beta"]
        self.current()
        self.handoff(("switched",), "PREUPGRADE_PENDING")
        self.stopped_beta(beta, [beta["restart_policy"]], "PREUPGRADE_BETA_CHANGED")
        self.check_confirmations()

        def body(_):
            code, lines = self.wrapper("--simulation-release-start", TIMEOUTS["release"])
            require(code == 0 and lines and lines[-1] == RELEASE_LINE, "PREUPGRADE_RESUME_STEP")
            # Done once the API start is recorded; the Beta health check is verify-beta.
            self.handoff(("api-started",), "PREUPGRADE_RESUME_STEP", absent_ok=True)
        return self.execute(record, "resume-release", "PREUPGRADE_RESUME_STEP", body)

    def step_resume_simulation(self, record):
        self.current()
        self.handoff(("api-started",), "PREUPGRADE_PENDING", absent_ok=True)

        def body(_):
            code, lines = self.wrapper("--simulation-start", TIMEOUTS["simulation"])
            require(code == 0 and lines and lines[-1] == SIMULATION_LINE, "PREUPGRADE_RESUME_STEP")
            self.no_pending("PREUPGRADE_RESUME_STEP")
        return self.execute(record, "resume-simulation", "PREUPGRADE_RESUME_STEP", body)

    def step_verify_beta(self, record):
        beta = record["beta"]
        self.no_pending("PREUPGRADE_PENDING")

        def body(_):
            return {"beta_generation_reused": self.resumed_beta(beta)}
        return self.execute(record, "verify-beta", "PREUPGRADE_RESUME_STEP", body)

    def step_recover_beta(self, record):
        """Return the recorded Beta generation to service when no handoff exists.

        Reached after a partial stop-beta (restart=no and stopped or still stopping) or
        a backup that failed before it wrote a handoff.  A running original start whose
        policy was already set to "no" may have received TERM; it is never restarted.
        """
        beta = record["beta"]
        require(beta is not None, "PREUPGRADE_PROGRESS")
        self.no_pending("PREUPGRADE_PENDING")
        code = "PREUPGRADE_BETA_CHANGED"
        own = self.worker_ownership(beta["image"])
        self.wrap(code, self.host.global_cohort, own)
        recorded = [beta["identity"], beta["created"], beta["started_at"], beta["restart_count"]]

        def observe():
            require(rg.compose_ids(self.docker, own.project, BETA, code) == [beta["identity"]], code)
            value = self.wrap(code, self.docker.object, "container", beta["identity"], 15)
            return value, list(self.wrap(code, self.host.owned, value, own, BETA, beta["identity"]))

        observe()

        def body(_):
            outcome = rg.restore_generation(
                self.docker, observe, recorded, beta["restart_policy"],
                signalled=lambda v: rg.restart_policy(v, "PREUPGRADE_BETA_RESTORE") != beta["restart_policy"],
                clock=self.clock, sleep=self.sleep, health_seconds=BETA_HEALTH_SECONDS,
                poll_seconds=POLL_SECONDS,
                codes={"changed": code, "state": "PREUPGRADE_BETA_RESTORE", "policy": "PREUPGRADE_BETA_RESTORE",
                       "start": "PREUPGRADE_BETA_RESTORE", "health": "PREUPGRADE_BETA_RESTORE"})
            return {"beta_recovery": outcome}
        return self.execute(record, "recover-beta", "PREUPGRADE_BETA_RESTORE", body)

    def step_restore_observer(self, record):
        def body(current):
            if current["observer"]["timer_was_active"]:
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
        code, lines = self.pinned("inactive-simulation-backup", "plan", TIMEOUTS["plan"])
        require(code == 0 and lines is not None and len(lines) == 1 and lines[0].startswith(PLAN_LINE),
                "PREUPGRADE_TOOL")

    def preflight(self):
        # The installed wrapper's release start is run without --daily-from-commit, so
        # its Daily guard binds the Beta to the application release: only a Beta from
        # that same release is supported here.
        require(self.args.stopped_worker_from_commit == self.args.application_commit,
                "PREUPGRADE_WORKER_SOURCE")
        self.current()
        self.no_pending("PREUPGRADE_PENDING")
        self.tool()
        self.running_beta()
        timer = self.timer_active()
        # A collector already in "failed" would block step 0 with the timer stopped.
        require(self.unit_state(OBSERVER_SERVICE)[1] in ("inactive", "active", "activating", "deactivating"),
                "PREUPGRADE_OBSERVER")
        self.pinned_plan()
        code, lines = self.pinned("inactive-simulation-backup", "preflight", TIMEOUTS["preflight"])
        require(code == 0 and lines == [PREFLIGHT_LINE], "PREUPGRADE_TOOL")
        required = self.check_confirmations()
        return timer, required

    def states(self, record):
        return {name: (record["steps"].get(name) or {}).get("state") for name in JOURNALED}

    def allowed(self, name, record):
        """Journal precondition; each step's own guards then check the observed state."""
        st = self.states(record)
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
            "resume-simulation": st["resume-release"] in ("done", "failed"),
            "verify-beta": st["resume-simulation"] == "done",
            "recover-beta": st["stop-beta"] in ("started", "failed", "done") and st["resume-release"] is None,
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
                middle = ["resume-simulation", "verify-beta"]
            else:
                middle = ["recover-beta"] if st["stop-beta"] is not None else []
        else:
            commit = self.args.application_commit
            require(handoff.get("source_commit") == commit and handoff.get("target_commit") == commit,
                    "PREUPGRADE_PENDING")
            phase = handoff.get("phase")
            if phase == "switched":
                middle = ["restore-beta-policy", "resume-release", "resume-simulation", "verify-beta"]
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

    def apply(self):
        record = self.progress.read()
        if record is None:
            require(self.args.step in ("all", "pause-observer"), "PREUPGRADE_PROGRESS")
            self.preflight()
            record = self.progress.create({"schema": SCHEMA, "arguments": arguments_record(self.args),
                                           "steps": {}, "observer": None, "beta": None})
        else:
            require(record.get("schema") == SCHEMA and record.get("arguments") == arguments_record(self.args)
                    and isinstance(record.get("steps"), dict), "PREUPGRADE_PROGRESS")
            self.current()
        for name in self.selection(record):
            require(self.allowed(name, record), "PREUPGRADE_PROGRESS")
            record = getattr(self, "step_" + name.replace("-", "_"))(record)
        if all(self.states(record)[n] == "done" for n in STEPS):
            return "PASS"
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
    "  5 resume: installed wrapper --simulation-release-start, then --simulation-start; Beta healthy check",
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
                              clock=clock, sleep=sleep)
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
            print("PREUPGRADE_BACKUP: PREFLIGHT", file=out)
            return 0
        sequencer.progress.lock()
        print("PREUPGRADE_BACKUP: " + sequencer.apply(), file=out)
        return 0
    except ToolError as exc:
        print("PREUPGRADE_BACKUP: FAILED " + exc.code, file=err)
        if exc.cause:
            print("PREUPGRADE_BACKUP: CAUSE " + exc.cause, file=err)
        return 1
    except Exception:
        print("PREUPGRADE_BACKUP: FAILED PREUPGRADE_INTERNAL", file=err)
        return 1
    finally:
        if sequencer is not None and sequencer.progress is not None:
            sequencer.progress.unlock()


if __name__ == "__main__":
    raise SystemExit(main())
