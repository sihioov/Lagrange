#!/usr/bin/env python3
"""Install, verify or retire the opt-in nightly simulation backup systemd drop-in.

Run as root from the exact installed application release. Stage the protected
scheduler config and its separate inactive backup config before installing.
Stop the canonical daily backup timer and disable its boot activation before
applying. The installer reloads systemd and leaves activation to the operator.
For rotation, retire the exact old override before replacing the old scheduler
and maintenance configs. Retirement keeps its bytes and all scheduler history.
"""
from __future__ import annotations

import argparse
import contextlib
import fcntl
import hashlib
import importlib.util
import os
from pathlib import Path
import re
import subprocess
import sys
import uuid

sys.dont_write_bytecode = True
LIB = Path(__file__).absolute().parent / "lib"


def load(name, directory=LIB):
    spec = importlib.util.spec_from_file_location(name, directory / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


files = load("owner-beta-daily-release")
policy = load("service-status-policy")
package = load("production-backup-tool")
nightly = load("nightly-simulation-backup", LIB.parent)
SERVICE = "lagrange-production-backup.service"
TIMER = "lagrange-production-backup.timer"
DROP_IN = "50-nightly-simulation.conf"
TEMPLATE = "deploy/systemd/lagrange-production-backup.nightly-simulation.conf.in"
ENTRYPOINT = "scripts/ops/nightly-simulation-backup.py"
CONFIG = "/etc/lagrange/nightly-simulation-backup.json"
LEGACY = "/etc/lagrange/production-backup.conf"
PINS = Path("/etc/lagrange/release-manifests")
STATE = Path("/var/lib/lagrange/backup-state/nightly-simulation")
INSTALL = Path("/opt/lagrange")
SYSTEMD = Path("/etc/systemd/system")
CREDENTIAL = "nightly-simulation-rollout.json"
PYTHON = ("/usr/bin/python3", "-I", "-S", "-B")
ENV = {"PATH": "/usr/sbin:/usr/bin:/sbin:/bin", "HOME": "/root", "LANG": "C.UTF-8",
       "SYSTEMD_PAGER": "cat", "SYSTEMD_COLORS": "0"}
MAX_CONFIG = 32768
LIMIT = 4 * 1024 * 1024


class Refused(Exception):
    pass


def require(condition, code):
    if not condition:
        raise Refused(code)


def read(path, mode=None, maximum=LIMIT):
    return files.read_root_file(str(path), gid=0, mode=mode, maximum=maximum)


def config_values(raw):
    result = {}
    for line in raw.decode("ascii").splitlines():
        if line and not line.startswith("#"):
            key, separator, value = line.partition("=")
            require(separator and key not in result and value, "CONFIG")
            result[key] = value
    return result


def unit_path(value):
    require(re.fullmatch(r"/[A-Za-z0-9_./-]+", value), "CONFIG_PATH")
    files.path_parts(value)
    return Path(value)


def check_directory(path, mode=None):
    with files.root_directory(str(path)) as fd:
        files.directory_info(fd, gid=0, mode=mode)


def check_separate_path(path, other, code):
    require(path != other and path not in other.parents and other not in path.parents, code)


def render_override(template, app, credential, writable):
    replacements = {
        "@APPLICATION_COMMIT@": app,
        "@CREDENTIAL@": "" if credential is None else f"LoadCredential={CREDENTIAL}:{credential}",
        "@WRITABLE_BACKUP_PATHS@": writable,
    }
    for marker, replacement in replacements.items():
        require(marker in template, "TEMPLATE")
        template = template.replace(marker, replacement)
    require("@" not in template, "TEMPLATE")
    return template.encode("ascii")


def deployment(*, retiring=False):
    config_data = read(CONFIG, 0o600, MAX_CONFIG)
    config = files.json_bytes(config_data, MAX_CONFIG)
    require(type(config) is dict and set(config) == {
        "schema", "application_commit", "tool_commit", "config_file", "config_sha256",
        "expected_db_first_session", "expected_config_first_session",
    } and config["schema"] == "lagrange-nightly-simulation-backup-v1", "CONFIG")
    app, tool = config["application_commit"], config["tool_commit"]
    require(files.is_commit(app) and files.is_commit(tool) and files.is_hash(config["config_sha256"])
            and files.trading_date(config["expected_db_first_session"]) <
                files.trading_date(config["expected_config_first_session"]), "CONFIG")
    pin = unit_path(config["config_file"])
    require(PINS in pin.parents and pin.suffix == ".conf", "CONFIG_PATH")
    raw = read(pin, 0o600, MAX_CONFIG)
    require("sha256:" + hashlib.sha256(raw).hexdigest() == config["config_sha256"], "CONFIG_HASH")
    maintenance = config_values(raw)
    backup = unit_path(maintenance["BACKUP_ROOT"])
    lock = unit_path(maintenance["LOCK_FILE"])
    metrics = unit_path(maintenance["METRICS_FILE"])
    require(Path("/srv/backups") in backup.parents and backup in metrics.parents
            and maintenance["DATA_ROOT"] == "/var/lib/lagrange/data"
            and maintenance["DAILY_BACKUP_TOOL_COMMIT"] == tool
            and files.is_hash(maintenance["DAILY_BACKUP_TOOL_MANIFEST"])
            and maintenance["LAGRANGE_CODE_COMMIT"] == app, "CONFIG_BINDING")
    require(backup == lock.parent or backup in lock.parents
            or lock.parent == STATE.parent or STATE.parent in lock.parents, "LOCK_ROOT")
    require(STATE not in lock.parents, "LOCK_ROOT")
    release = INSTALL / "releases" / app
    source = Path(__file__).resolve()
    installer = Path("scripts/ops/install-nightly-simulation-backup.py")
    if retiring:
        original = release / installer
        if source != original:
            # The newly current installed installer may retire an older override.
            current = source.parents[2]
            require(source == INSTALL / "releases" / current.name / installer
                    and files.is_commit(current.name), "INSTALLER_RELEASE")
            policy.validate_current_release(str(INSTALL), str(current), current.name)
        require(read(release / ".lagrange-release", 0o600, 41) == (app + "\n").encode(),
                "INSTALLER_RELEASE")
        policy.validate_manifest(str(release / ".lagrange-release-manifest"), app)
        template = read(release / TEMPLATE, 0o644).decode("ascii")
        credential = PINS / "nightly-simulation" / app / CREDENTIAL
        writable = " ".join(str(path) for path in sorted({backup, lock.parent}))
        return release, tuple(render_override(template, app, item, writable) for item in (None, credential))

    require(source == release / installer, "INSTALLER_RELEASE")
    legacy = config_values(read(LEGACY, 0o600, MAX_CONFIG))
    check_separate_path(backup, unit_path(legacy["BACKUP_ROOT"]), "BACKUP_ROOT_OVERLAP")
    for path in {backup, metrics.parent, lock.parent}:
        check_directory(path, 0o700)
    policy.validate_current_release(str(INSTALL), str(release), app)
    package.read_manifest(INSTALL / "maintenance" / tool, app, install_root=INSTALL,
                          expected_hash=maintenance["DAILY_BACKUP_TOOL_MANIFEST"])
    for path in (ENTRYPOINT, "scripts/ops/preupgrade-simulation-backup.py", "scripts/ops/compose-release.sh"):
        require(bool(read(release / path)), "APPLICATION_COMPONENT")
    for unit in (SERVICE, TIMER):
        require(read(SYSTEMD / unit, 0o644) == read(release / "deploy/systemd" / unit, 0o644), "LEGACY_UNIT")

    credential = PINS / "nightly-simulation" / app / CREDENTIAL
    try:
        credential_data = read(credential, 0o400, MAX_CONFIG)
        require(bool(credential_data), "CREDENTIAL")
    except FileNotFoundError:
        credential, credential_data = None, b""
    writable = " ".join(str(path) for path in sorted({backup, lock.parent}))
    template = read(release / TEMPLATE, 0o644).decode("ascii")
    configuration = (hashlib.sha256(config_data).digest(), hashlib.sha256(credential_data).digest())
    return release, credential, render_override(template, app, credential, writable), configuration


def run(argv, *, env=ENV, timeout=30):
    result = subprocess.run(argv, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                            stderr=subprocess.DEVNULL, check=False, timeout=timeout)
    require(result.returncode == 0 and len(result.stdout) <= MAX_CONFIG, "COMMAND")
    return result.stdout.decode("ascii")


def show(unit, properties):
    output = run(("/usr/bin/systemctl", "show", "--all", "--no-pager",
                  "--property=" + ",".join(properties), unit))
    result = {}
    for line in output.splitlines():
        key, separator, value = line.partition("=")
        require(separator and key not in result, "SYSTEMD")
        result[key] = value
    require(result.keys() == set(properties), "SYSTEMD")
    return result


def check_unit(unit, drop_in, *, installing, retiring=False):
    status = show(unit, ("LoadState", "FragmentPath", "DropInPaths", "NeedDaemonReload",
                         "ActiveState", "UnitFileState"))
    permitted_drop_ins = {drop_in, ""} if retiring else {drop_in}
    reload_states = {"yes", "no"} if retiring else {"no"}
    require(status["LoadState"] == "loaded" and status["FragmentPath"] == str(SYSTEMD / unit)
            and status["DropInPaths"] in permitted_drop_ins
            and status["NeedDaemonReload"] in reload_states, "SYSTEMD_UNIT")
    if installing:
        require(status["ActiveState"] in {"inactive", "failed"}
                and status["UnitFileState"] == "disabled", "SYSTEMD_ACTIVE_OR_ENABLED")
        if unit == TIMER:
            require(status["ActiveState"] == "inactive" and status["UnitFileState"] == "disabled",
                    "TIMER_MUST_BE_INACTIVE_AND_DISABLED")


def make_directory(path, mode):
    with files.root_directory(str(path.parent)) as parent:
        try:
            os.mkdir(path.name, mode, dir_fd=parent)
        except FileExistsError:
            pass
        else:
            child = os.open(path.name, files.DIR_FLAGS, dir_fd=parent)
            try:
                os.fchmod(child, mode)
                os.fsync(child)
                os.fsync(parent)
            finally:
                os.close(child)
    check_directory(path)


def check_destination(target, expected):
    require(read(target, 0o644) == expected, "DROP_IN_CHANGED")
    check_unit(SERVICE, str(target), installing=False)
    check_unit(TIMER, "", installing=False)


@contextlib.contextmanager
def release_lock():
    with files.root_directory(str(INSTALL)) as parent:
        descriptor = os.open(".lagrange-release.lock", os.O_RDWR | os.O_NOFOLLOW | os.O_CLOEXEC,
                             dir_fd=parent)
        try:
            before = files.file_info(descriptor, 0, 0, 0o600)
            require(before.st_size == 0, "RELEASE_LOCK")
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
            require(files.file_identity(before) == files.file_identity(
                os.stat(".lagrange-release.lock", dir_fd=parent, follow_symlinks=False)),
                "RELEASE_LOCK")
            yield
        finally:
            os.close(descriptor)


@contextlib.contextmanager
def scheduler_lock():
    # The scheduler holds this exact directory flock, including while its
    # direct sequencer is alive. Take it before the runtime release lock.
    with files.root_directory(str(STATE)) as descriptor:
        files.directory_info(descriptor, gid=0, mode=0o700)
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise Refused("SCHEDULER_BUSY") from None
        yield descriptor


def check_no_unit_jobs():
    jobs = run(("/usr/bin/systemctl", "list-jobs", "--no-legend", "--no-pager", "--plain"))
    require(not any({SERVICE, TIMER}.intersection(line.split()) for line in jobs.splitlines()),
            "SYSTEMD_ACTIVE_OR_ENABLED")


def retire():
    target = SYSTEMD / (SERVICE + ".d") / DROP_IN
    with scheduler_lock() as state_fd, release_lock():
        release, expected = deployment(retiring=True)
        retired = target.with_name(target.name + ".retired." + release.name)
        check_unit(SERVICE, str(target), installing=True, retiring=True)
        check_unit(TIMER, "", installing=True, retiring=True)
        check_no_unit_jobs()
        # A previous run can only be recovered with its original release pins.
        try:
            nightly.check_history(state_fd)
        except nightly.rg.ToolError as error:
            raise Refused(error.code) from None
        with files.root_directory(str(target.parent)) as directory:
            if os.path.lexists(target):
                require(read(target, 0o644) in expected, "DROP_IN_CHANGED")
                require(not os.path.lexists(retired), "RETIRE_DESTINATION_EXISTS")
                try:
                    files.rename_no_replace(directory, target.name, directory, retired.name)
                except OSError:
                    if not os.path.lexists(target) and os.path.lexists(retired):
                        raise Refused("RETIRED_CHECK_REQUIRED") from None
                    raise
            else:
                # A previous attempt may have finished the durable rename and
                # then lost systemd's reload result. Only these exact bytes can
                # authorize completing that same retirement.
                require(os.path.lexists(retired) and read(retired, 0o644) in expected, "DROP_IN_CHANGED")
        try:
            require(not os.path.lexists(target) and read(retired, 0o644) in expected,
                    "DROP_IN_CHANGED")
            run(("/usr/bin/systemctl", "daemon-reload"))
            check_unit(SERVICE, "", installing=True)
            check_unit(TIMER, "", installing=True)
            check_no_unit_jobs()
        except Exception:
            raise Refused("RETIRED_CHECK_REQUIRED") from None


def install():
    target = SYSTEMD / (SERVICE + ".d") / DROP_IN
    with release_lock():
        selected = deployment()
        release, credential, rendered, _configuration = selected
        require(not os.path.lexists(target), "DROP_IN_EXISTS")
        check_unit(SERVICE, "", installing=True)
        check_unit(TIMER, "", installing=True)
        make_directory(STATE, 0o700)
        check_directory(STATE, 0o700)

    # The pinned preflight takes the existing release lock independently.
    env = ENV if credential is None else dict(ENV, CREDENTIALS_DIRECTORY=str(credential.parent))
    run((*PYTHON, str(release / ENTRYPOINT), "--config-file", CONFIG, "--preflight"),
        env=env, timeout=26 * 60)

    with release_lock():
        require(deployment() == selected, "DEPLOYMENT_CHANGED")
        check_unit(SERVICE, "", installing=True)
        check_unit(TIMER, "", installing=True)
        make_directory(target.parent, 0o755)
        with files.root_directory(str(target.parent)) as directory:
            leaf = ".nightly-" + uuid.uuid4().hex
            files.create_file(directory, leaf, rendered, 0, 0, 0o644)
            try:
                files.rename_no_replace(directory, leaf, directory, target.name)
            finally:
                try:
                    os.unlink(leaf, dir_fd=directory)
                except FileNotFoundError:
                    pass
        try:
            run(("/usr/bin/systemctl", "daemon-reload"))
            check_destination(target, rendered)
        except Exception:
            # Keep exact published bytes for the operator. Neither eligible unit
            # is enabled, and the installer never runs the scheduled command.
            raise Refused("PUBLISHED_CHECK_REQUIRED") from None


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    modes = parser.add_mutually_exclusive_group(required=True)
    modes.add_argument("--apply", action="store_true")
    modes.add_argument("--check", action="store_true")
    modes.add_argument("--retire", action="store_true")
    args = parser.parse_args(argv)
    try:
        files.require_root()
        os.umask(0o077)
        if args.retire:
            retire()
        elif args.apply:
            install()
        else:
            _release, _credential, rendered, _configuration = deployment()
            check_directory(STATE, 0o700)
            check_destination(SYSTEMD / (SERVICE + ".d") / DROP_IN, rendered)
        print("NIGHTLY_BACKUP_INSTALL: RETIRED" if args.retire else "NIGHTLY_BACKUP_INSTALL: PASS")
        return 0
    except Exception as error:
        reason = str(error) if type(error) is Refused else "VALIDATION"
        print("NIGHTLY_BACKUP_INSTALL: FAILED " + reason, file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
