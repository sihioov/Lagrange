#!/usr/bin/env python3
"""Schedule the installed inactive-Daily backup sequencer once per Seoul date."""
from __future__ import annotations

import contextlib
import datetime as dt
import fcntl
import hashlib
import importlib.util
import itertools
import os
import re
import signal
import subprocess
import sys
import uuid
from dataclasses import dataclass
from pathlib import Path
from zoneinfo import ZoneInfo

sys.dont_write_bytecode = True


def load_sibling(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name("lib") / filename)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


rg = load_sibling("nightly_reader_generation", "reader-generation.py")
install = load_sibling("nightly_daily_install", "owner-beta-daily-release.py")
require = rg.require

CONFIG_SCHEMA = "lagrange-nightly-simulation-backup-v1"
CLAIM_SCHEMA = "lagrange-nightly-simulation-backup-claim-v1"
RESULT_SCHEMA = "lagrange-nightly-simulation-backup-result-v1"
ACK_SCHEMA = "lagrange-nightly-simulation-backup-disposition-v1"
ROLLOUT_SCHEMA = "lagrange-nightly-simulation-rollout-v1"
PROGRESS_SCHEMA = "lagrange-preupgrade-simulation-backup-progress-v1"
SELF = "scripts/ops/nightly-simulation-backup.py"
SEQUENCER = "scripts/ops/preupgrade-simulation-backup.py"
PROGRESS = "preupgrade-simulation-backup.json"
CHILD_LOCK = "preupgrade-simulation-backup.lock"
CLAIM, RESULT, ACK = "claim.json", "result.json", "disposition.json"
CREDENTIAL = "nightly-simulation-rollout.json"
CONFIRMATIONS = ("OWNER_EQUITY_V2_ROLLOUT_CONFIRM", "OWNER_MARKET_STREAM_ROLLOUT_CONFIRM")
CONFIRMATION_VALUES = dict(zip(CONFIRMATIONS, (
    "I_UNDERSTAND_OWNER_EQUITY_V2_READ_ONLY_KIS_CALLS",
    "I_UNDERSTAND_OWNER_MARKET_STREAM_READ_ONLY_WS_CALLS",
)))
SERVICES = ("api-server", "owner-beta-runner", "owner-beta-simulation-runner", "owner-equity-v2-runner", "postgres")
UNTOUCHED = ("owner-equity-v2-runner", "postgres")
STEPS = ("pause-observer", "stop-beta", "backup", "verify", "restore-beta-policy",
         "resume-release", "resume-simulation", "verify-beta", "restore-observer")
CONFIG_KEYS = {"schema", "application_commit", "tool_commit", "config_file", "config_sha256",
               "expected_db_first_session", "expected_config_first_session"}
CLAIM_KEYS = {"schema", "date", "config", "sequencer_sha256", "started_at_utc"}
RESULT_KEYS = {"schema", "date", "claim_sha256", "status", "stage", "exit_code", "diagnostic",
               "preflight", "progress_sha256", "finished_at_utc"}
ACK_KEYS = {"schema", "date", "claim_sha256", "result_sha256", "progress_sha256", "config_sha256",
            "observation", "acknowledged_at_utc"}
HASH = re.compile(r"sha256:[0-9a-f]{64}")
PREFLIGHT_OBSERVER = re.compile(r"PREUPGRADE_BACKUP: OBSERVER timer=(active|inactive)")
PREFLIGHT_CONFIRMATIONS = re.compile(r"PREUPGRADE_BACKUP: CONFIRMATIONS owner_equity_v2=(required|not_required) "
                                     r"owner_market_stream=(required|not_required)")
PREFLIGHT_COHORT = re.compile(r"PREUPGRADE_BACKUP: NIGHTLY_COHORT simulation=(running|absent) "
                              r"owner_v2=(running|stopped|absent)")
PREFLIGHT_UNTOUCHED = re.compile(r"PREUPGRADE_BACKUP: NIGHTLY_UNTOUCHED_SHA256 (sha256:[0-9a-f]{64})")
PASS_LINES = {True: b"PREUPGRADE_BACKUP: PASS\n",
              False: (b"PREUPGRADE_BACKUP: resume-simulation: SKIPPED_NO_CAPTURED_WORKER\n"
                      b"PREUPGRADE_BACKUP: PASS simulation_worker=not_running_before_not_started\n")}
CHILD_ERROR = re.compile(rb"(?m)^PREUPGRADE_BACKUP: FAILED ([A-Z][A-Z0-9_]{0,100})$")
MAX_HISTORY = 20_000
CONFIG_BYTES = 4096
MAINTENANCE_BYTES = 64 * 1024
SEQUENCER_BYTES = 256 * 1024
PREFLIGHT_TIMEOUT = 15 * 60
APPLY_TIMEOUT = 12 * 3600


@dataclass(frozen=True)
class Layout:
    install_root: str = "/opt/lagrange"
    state_root: str = "/var/lib/lagrange/backup-state/nightly-simulation"
    config_file: str = "/etc/lagrange/nightly-simulation-backup.json"
    config_root: str = "/etc/lagrange/release-manifests"
    credentials_dir: str = "/run/credentials/lagrange-production-backup.service"


def digest(data):
    return "sha256:" + hashlib.sha256(data).hexdigest()


def optional_digest(data):
    return None if data is None else digest(data)


def parse(argv, layout):
    parser = rg.Parser(prog="nightly-simulation-backup.py", code="NIGHTLY_ARGUMENTS")
    parser.add_argument("--config-file", required=True)
    modes = parser.add_mutually_exclusive_group(required=True)
    modes.add_argument("--preflight", action="store_true")
    modes.add_argument("--apply", action="store_true")
    modes.add_argument("--acknowledge", metavar="YYYY-MM-DD")
    rg.reject_repeated(argv, ("--config-file", "--preflight", "--apply", "--acknowledge"), "NIGHTLY_ARGUMENTS")
    args = parser.parse_args(argv)
    require(args.config_file == layout.config_file and (args.acknowledge is None or rg.is_date(args.acknowledge)),
            "NIGHTLY_ARGUMENTS")
    return args


def read_config(layout):
    raw = install.read_root_file(layout.config_file, gid=0, mode=0o600, maximum=CONFIG_BYTES)
    value = rg.strict_json(raw, "NIGHTLY_CONFIG")
    require(type(value) is dict and set(value) == CONFIG_KEYS and value["schema"] == CONFIG_SCHEMA,
            "NIGHTLY_CONFIG")
    claim_date, config_date = value["expected_db_first_session"], value["expected_config_first_session"]
    path, checksum = value["config_file"], value["config_sha256"]
    require(rg.is_commit(value["application_commit"]) and rg.is_commit(value["tool_commit"])
            and rg.is_date(claim_date) and rg.is_date(config_date) and claim_date < config_date
            and rg.absolute(path) and path.startswith(layout.config_root + "/") and path.endswith(".conf")
            and isinstance(checksum, str) and HASH.fullmatch(checksum), "NIGHTLY_CONFIG")
    read_maintenance(value)
    return value


def read_maintenance(config):
    raw = install.read_root_file(config["config_file"], gid=0, mode=0o600, maximum=MAINTENANCE_BYTES)
    require(digest(raw) == config["config_sha256"], "NIGHTLY_CONFIG")


def read_confirmations(layout, config, environ):
    directory = environ.get("CREDENTIALS_DIRECTORY")
    if directory is None:
        return {}
    commit = config["application_commit"]
    owner_dir = layout.config_root + "/nightly-simulation/" + commit
    require(directory in (layout.credentials_dir, owner_dir), "NIGHTLY_CONFIRMATIONS")
    try:
        raw = install.read_root_file(directory + "/" + CREDENTIAL, gid=0, mode=0o400, maximum=CONFIG_BYTES)
    except FileNotFoundError:
        return {}
    item = rg.strict_json(raw, "NIGHTLY_CONFIRMATIONS")
    require(type(item) is dict and set(item) == {"schema", "application_commit", "operation", "confirmations"}
            and item["schema"] == ROLLOUT_SCHEMA and item["application_commit"] == commit
            and item["operation"] == "nightly-simulation-backup" and type(item["confirmations"]) is dict,
            "NIGHTLY_CONFIRMATIONS")
    values = item["confirmations"]
    require(all(key in CONFIRMATION_VALUES and values[key] == CONFIRMATION_VALUES[key] for key in values),
            "NIGHTLY_CONFIRMATIONS")
    return values


def child_arguments(layout, config):
    return {"install_root": layout.install_root, "application_commit": config["application_commit"],
            "tool_commit": config["tool_commit"], "config_file": config["config_file"],
            "expected_db_first_session": config["expected_db_first_session"],
            "expected_config_first_session": config["expected_config_first_session"],
            "stopped_worker_from_commit": config["application_commit"]}


def check_installed(layout, config):
    script = layout.install_root + "/releases/" + config["application_commit"] + "/" + SELF
    require(Path(__file__).resolve() == Path(script), "NIGHTLY_CURRENT")
    install.read_root_file(script, maximum=SEQUENCER_BYTES)


def check_current(layout, install_fd, config):
    commit = config["application_commit"]
    try:
        require(os.readlink("current", dir_fd=install_fd) == "releases/" + commit, "NIGHTLY_CURRENT")
    except OSError:
        raise rg.ToolError("NIGHTLY_CURRENT") from None
    entry = layout.install_root + "/releases/" + commit + "/" + SEQUENCER
    source = install.read_root_file(entry, maximum=SEQUENCER_BYTES)
    return entry, digest(source)


def run_child(argv, env, timeout, lock_fd):
    # Preserve the same flock in the direct sequencer if the scheduler dies.
    # Descendants use the existing sequencer's bounded command execution.
    try:
        process = subprocess.Popen(argv, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, pass_fds=(lock_fd,), start_new_session=True)
    except OSError:
        return None, b"", b""
    try:
        out, err = process.communicate(timeout=timeout)
        return process.returncode, out, err
    except subprocess.TimeoutExpired:
        rg._signal_group(process, signal.SIGTERM)
    try:
        out, err = process.communicate(timeout=600)
    except subprocess.TimeoutExpired:
        rg._signal_group(process, signal.SIGKILL)
        try:
            out, err = process.communicate(timeout=30)
        except subprocess.TimeoutExpired:
            return None, b"", b""
    return None, out, err


def preflight_result(code, data):
    lines = rg.stdout_lines(data)
    require(code == 0 and lines is not None and len(lines) == 5
            and lines[-1] == "PREUPGRADE_BACKUP: PREFLIGHT", "NIGHTLY_CHILD_PREFLIGHT")
    formats = (PREFLIGHT_OBSERVER, PREFLIGHT_CONFIRMATIONS, PREFLIGHT_COHORT, PREFLIGHT_UNTOUCHED)
    values = []
    for pattern, line in zip(formats, lines):
        match = pattern.fullmatch(line)
        require(match is not None, "NIGHTLY_CHILD_PREFLIGHT")
        values.append(match.groups())
    (timer,), confirmations, (simulation, v2), (untouched,) = values
    return {"observer_timer_active": timer == "active", "simulation": simulation, "owner_v2": v2,
            "untouched_sha256": untouched,
            "confirmations": [name for name, needed in zip(CONFIRMATIONS, confirmations) if needed == "required"]}


def read_optional(fd, name, maximum=rg.PROGRESS_LIMIT):
    try:
        return install.read_file(fd, name, gid=0, mode=0o600, maximum=maximum)
    except FileNotFoundError:
        return None


def write_record(fd, name, value):
    temporary = "." + name + "." + uuid.uuid4().hex + ".pending"
    install.create_file(fd, temporary, rg.canonical(value), 0, 0, 0o600)
    install.rename_no_replace(fd, temporary, fd, name)


def load_record(data, code):
    if data is None:
        return None
    value = rg.strict_json(data, code)
    require(type(value) is dict and rg.canonical(value) == data, code)
    return value


def read_history(fd, date):
    files = tuple(read_optional(fd, name) for name in (CLAIM, RESULT, ACK, PROGRESS))
    claim_bytes, result_bytes, ack_bytes, progress_bytes = files
    if claim_bytes is not None:
        claim = load_record(claim_bytes, "NIGHTLY_HISTORY")
        require(set(claim) == CLAIM_KEYS and claim["schema"] == CLAIM_SCHEMA
                and claim["date"] == date, "NIGHTLY_HISTORY")
    if result_bytes is not None:
        result = load_record(result_bytes, "NIGHTLY_HISTORY")
        require(claim_bytes is not None and set(result) == RESULT_KEYS
                and result["schema"] == RESULT_SCHEMA and result["date"] == date
                and result.get("claim_sha256") == digest(claim_bytes)
                and result.get("status") in ("PASS", "FAILED", "UNKNOWN"), "NIGHTLY_HISTORY")
        if result["status"] == "PASS":
            require(ack_bytes is None and progress_bytes is not None
                    and result.get("progress_sha256") == digest(progress_bytes), "NIGHTLY_HISTORY")
            return True, files
    if ack_bytes is not None:
        ack = load_record(ack_bytes, "NIGHTLY_HISTORY")
        require(set(ack) == ACK_KEYS and ack["schema"] == ACK_SCHEMA and ack["date"] == date
                and ack.get("claim_sha256") == optional_digest(claim_bytes)
                and ack.get("result_sha256") == optional_digest(result_bytes)
                and ack.get("progress_sha256") == optional_digest(progress_bytes), "NIGHTLY_HISTORY")
        return True, files
    return False, files


@contextlib.contextmanager
def date_directory(state_fd, date):
    fd = os.open(date, install.DIR_FLAGS, dir_fd=state_fd)
    try:
        identity = install.directory_info(fd, gid=0, mode=0o700)
        yield fd
        named = os.stat(date, dir_fd=state_fd, follow_symlinks=False)
        require((named.st_dev, named.st_ino) == (identity.st_dev, identity.st_ino), "NIGHTLY_HISTORY")
    finally:
        os.close(fd)


def lock(fd):
    try:
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        raise rg.ToolError("NIGHTLY_BUSY") from None


@contextlib.contextmanager
def child_quiesced(date_fd):
    try:
        fd = os.open(CHILD_LOCK, install.FILE_FLAGS, dir_fd=date_fd)
    except FileNotFoundError:
        def absent():
            try:
                os.stat(CHILD_LOCK, dir_fd=date_fd, follow_symlinks=False)
            except FileNotFoundError:
                return False
            raise rg.ToolError("NIGHTLY_PROGRESS")

        yield absent
        return
    try:
        identity = install.file_identity(install.file_info(fd, 0, gid=0, mode=0o600))
        lock(fd)

        def held():
            named = os.stat(CHILD_LOCK, dir_fd=date_fd, follow_symlinks=False)
            require(install.file_identity(named) == identity, "NIGHTLY_PROGRESS")
            return True

        yield held
    finally:
        os.close(fd)


def check_history(state_fd, today):
    with os.scandir(state_fd) as entries:
        dates = [entry.name for entry in itertools.islice(entries, MAX_HISTORY + 1)]
    require(len(dates) <= MAX_HISTORY and all(rg.is_date(day) and day <= today for day in dates),
            "NIGHTLY_HISTORY")
    for day in sorted(dates):
        with date_directory(state_fd, day) as fd:
            settled, _ = read_history(fd, day)
            require(settled, "NIGHTLY_UNRESOLVED")
    return today in dates


def progress_record(data, layout, config):
    record = load_record(data, "NIGHTLY_PROGRESS")
    require(record is not None and record.get("schema") == PROGRESS_SCHEMA
            and record.get("arguments") == child_arguments(layout, config)
            and type(record.get("steps")) is dict and type(record.get("nightly")) is dict, "NIGHTLY_PROGRESS")
    return record


def complete(record):
    return record.get("nightly_completed") is True and all(
        record["steps"].get(name) == {"state": "done", "code": None} for name in STEPS)


def verify_recovery(progress, observation):
    if not progress["steps"]:
        return
    require(complete(progress) or progress.get("nightly_recovered") is True, "NIGHTLY_RECOVERY_REQUIRED")
    original = progress["nightly"]
    services = original.get("services")
    require(type(services) is dict and set(services) == set(SERVICES)
            and type(original.get("observer_timer_active")) is bool, "NIGHTLY_PROGRESS")
    v2 = services["owner-equity-v2-runner"]
    require(v2 is None or (type(v2) is dict and type(v2.get("running")) is bool), "NIGHTLY_PROGRESS")
    worker = services["owner-beta-simulation-runner"]
    require(worker is None or (type(worker) is dict and worker.get("running") is True), "NIGHTLY_PROGRESS")
    require(observation["observer_timer_active"] == original["observer_timer_active"]
            and observation["simulation"] == ("absent" if worker is None else "running")
            and observation["owner_v2"] == ("absent" if v2 is None else ("running" if v2["running"] else "stopped"))
            and observation["untouched_sha256"] == digest(rg.canonical({key: services[key] for key in UNTOUCHED})),
            "NIGHTLY_RECOVERY_COHORT")


class Nightly:
    def __init__(self, layout, state_fd, install_fd, config, environ, runner):
        self.layout, self.state_fd, self.install_fd = layout, state_fd, install_fd
        self.config, self.environ, self.runner = config, environ, runner
        self.entry, self.entry_sha256 = check_current(layout, install_fd, config)

    def verify_pins(self):
        require(check_current(self.layout, self.install_fd, self.config) == (self.entry, self.entry_sha256),
                "NIGHTLY_CURRENT")
        read_maintenance(self.config)

    def child(self, mode, state_dir, confirmations, timeout):
        self.verify_pins()
        argv = ["/usr/bin/python3", "-I", "-S", "-B", self.entry, "--nightly", mode, "--state-dir", state_dir]
        for name, value in child_arguments(self.layout, self.config).items():
            argv.extend(("--" + name.replace("_", "-"), value))
        result = self.runner(argv, rg.child_environment(confirmations), timeout, self.state_fd)
        self.verify_pins()
        return result

    def preflight(self):
        confirmations = read_confirmations(self.layout, self.config, self.environ)
        code, data, _ = self.child("--preflight", self.layout.state_root, confirmations, PREFLIGHT_TIMEOUT)
        value = preflight_result(code, data)
        require(set(value["confirmations"]) <= confirmations.keys(), "NIGHTLY_CONFIRMATIONS")
        return value, {name: confirmations[name] for name in value["confirmations"]}

    def apply(self, date, started_at):
        os.mkdir(date, 0o700, dir_fd=self.state_fd)
        os.fsync(self.state_fd)
        with date_directory(self.state_fd, date) as fd:
            claim = {"schema": CLAIM_SCHEMA, "date": date, "config": self.config,
                     "sequencer_sha256": self.entry_sha256, "started_at_utc": rg.utc_stamp(started_at)}
            write_record(fd, CLAIM, claim)
            status, stage, exit_code, diagnostic = "UNKNOWN", "PREFLIGHT", None, None
            observation, successful_progress = None, None
            try:
                observation, required = self.preflight()
                stage = "APPLY"
                code, data, err = self.child("--apply", self.layout.state_root + "/" + date, required, APPLY_TIMEOUT)
                exit_code = code
                failure = CHILD_ERROR.search(err[:4096])
                diagnostic = failure.group(1).decode("ascii") if failure else None
                require(code == 0, "NIGHTLY_CHILD_APPLY")
                with child_quiesced(fd) as check_lock:
                    require(check_lock(), "NIGHTLY_PROGRESS")
                    successful_progress = read_optional(fd, PROGRESS)
                    progress = progress_record(successful_progress, self.layout, self.config)
                    captured = progress.get("simulation_worker_captured")
                    require(complete(progress) and type(captured) is bool, "NIGHTLY_PROGRESS")
                    require(data == PASS_LINES[captured], "NIGHTLY_CHILD_APPLY")
                    verify_recovery(progress, observation)
                    require(check_lock() and read_optional(fd, PROGRESS) == successful_progress, "NIGHTLY_PROGRESS")
                    status = "PASS"
            except rg.ToolError as error:
                status = "FAILED" if stage == "PREFLIGHT" or exit_code not in (None, 0) else "UNKNOWN"
                diagnostic = diagnostic or error.code
                raise
            finally:
                progress = successful_progress if status == "PASS" else read_optional(fd, PROGRESS)
                write_record(fd, RESULT, {"schema": RESULT_SCHEMA, "date": date,
                    "claim_sha256": digest(rg.canonical(claim)), "status": status, "stage": stage,
                    "exit_code": exit_code, "diagnostic": diagnostic, "preflight": observation,
                    "finished_at_utc": rg.utc_stamp(),
                    "progress_sha256": optional_digest(progress)})

    def acknowledge(self, date):
        with date_directory(self.state_fd, date) as fd, child_quiesced(fd) as check_lock:
            settled, (claim, result, _, progress) = read_history(fd, date)
            require(not settled, "NIGHTLY_ALREADY_SETTLED")
            if claim is None:
                require(result is None and progress is None, "NIGHTLY_HISTORY")
            elif progress is not None:
                original = load_record(claim, "NIGHTLY_HISTORY")
                require(original.get("config") == self.config
                        and original.get("sequencer_sha256") == self.entry_sha256, "NIGHTLY_RECOVERY_PINS")
            prior = load_record(result, "NIGHTLY_HISTORY")
            prior_progress = None if prior is None else prior["progress_sha256"]
            quiesced = check_lock()
            record = None
            if progress is None:
                # A lost progress record is not evidence that the sequencer
                # never reached its mutation boundary.
                require(not quiesced and prior_progress is None, "NIGHTLY_RECOVERY_REQUIRED")
            else:
                require(quiesced, "NIGHTLY_PROGRESS")
                record = progress_record(progress, self.layout, self.config)
                if not record["steps"] and prior_progress is not None:
                    require(digest(progress) == prior_progress, "NIGHTLY_RECOVERY_REQUIRED")
            observation, _ = self.preflight()
            if record is not None:
                verify_recovery(record, observation)
            require(check_lock() == quiesced and read_optional(fd, PROGRESS) == progress, "NIGHTLY_PROGRESS")
            write_record(fd, ACK, {"schema": ACK_SCHEMA, "date": date, "claim_sha256": optional_digest(claim),
                "result_sha256": optional_digest(result), "progress_sha256": optional_digest(progress),
                "config_sha256": digest(rg.canonical(self.config)), "observation": observation,
                "acknowledged_at_utc": rg.utc_stamp()})


def main(argv=None, *, layout=None, runner=None, environ=None, now=None, out=None, err=None):
    production = layout is None
    layout = Layout() if production else layout
    runner = run_child if runner is None else runner
    environ = os.environ if environ is None else environ
    now = (lambda: dt.datetime.now(dt.timezone.utc)) if now is None else now
    out, err = (sys.stdout if out is None else out), (sys.stderr if err is None else err)
    try:
        require(not production or (sys.platform == "linux" and os.geteuid() == 0), "NIGHTLY_ROOT")
        args = parse(list(sys.argv[1:] if argv is None else argv), layout)
        with install.root_directory(layout.state_root) as fd, install.root_directory(layout.install_root) as install_fd:
            install.directory_info(fd, gid=0, mode=0o700)
            lock(fd)
            started_at = now()
            today = started_at.astimezone(ZoneInfo("Asia/Seoul")).date().isoformat()
            config = read_config(layout)
            if production:
                check_installed(layout, config)
            night = Nightly(layout, fd, install_fd, config, environ, runner)
            if args.acknowledge is not None:
                require(args.acknowledge <= today, "NIGHTLY_ARGUMENTS")
                night.acknowledge(args.acknowledge)
                output = "ACKNOWLEDGED date=" + args.acknowledge
            else:
                already_ran = check_history(fd, today)
                if args.preflight:
                    night.preflight()
                    output = "PREFLIGHT"
                else:
                    require(not already_ran, "NIGHTLY_ALREADY_RAN")
                    night.apply(today, started_at)
                    output = "PASS date=" + today
        print("NIGHTLY_BACKUP: " + output, file=out)
        return 0
    except rg.ToolError as error:
        print("NIGHTLY_BACKUP: FAILED " + error.code, file=err)
    except (OSError, install.DailyInstallError, RecursionError):
        print("NIGHTLY_BACKUP: FAILED NIGHTLY_PROTECTED_IO", file=err)
    except Exception:
        print("NIGHTLY_BACKUP: FAILED NIGHTLY_INTERNAL", file=err)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
