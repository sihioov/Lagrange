#!/usr/bin/env python3
"""Generate the simulation (and optional Daily) operation inputs for a new release.

From the previous release's protected configuration this writes, into one empty
root-owned directory, the new commit's simulation config plus byte-copies of the
build receipt and COMPLETE files, in the exact form the installer's own
`stage_config` reads.  Validation is the installer's: nothing is re-implemented.
The tool prints closed codes and counts only, never a configured value or path.
"""
from __future__ import annotations

import argparse
import contextlib
import copy
import hashlib
import importlib.util
import json
import os
import re
import stat
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
LIB = os.path.join(HERE, "lib")
COMMIT_RE = re.compile(r"[0-9a-f]{40}\Z")
ROOT_UID = 0
READER_GID = 10001

SIM_CONFIG_FILE = "simulation-config.json"
SIM_RECEIPT_FILE = "simulation-artifact.json"
SIM_COMPLETE_FILE = "simulation-COMPLETE"
DAILY_CONFIG_FILE = "daily-config.json"

SIM_CARRIED = ("owner", "entitlement_reference", "entitlement_sha256", "historical_root", "limits")


def _load(name, filename):
    spec = importlib.util.spec_from_file_location(name, os.path.join(LIB, filename))
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


daily = _load("_lagrange_inputs_daily_release", "owner-beta-daily-release.py")
release = _load("_lagrange_inputs_simulation_release", "owner-beta-simulation-release.py")


class InputsError(Exception):
    def __init__(self, code, cause=None):
        super().__init__(code)
        self.code = code
        self.cause = cause


def fail(code, cause=None):
    raise InputsError(code, cause)


class ClosedParser(argparse.ArgumentParser):
    def error(self, message):  # never echo argument values
        fail("SIMULATION_INPUTS_ARGUMENTS")


def parse_args(argv):
    parser = ClosedParser(add_help=False, allow_abbrev=False)
    group = parser.add_mutually_exclusive_group(required=True)
    for name in ("plan", "preflight", "apply"):
        group.add_argument("--" + name, dest="mode", action="store_const", const=name)
    for name in ("commit", "manifest", "receipt", "complete", "previous-release", "output-dir"):
        parser.add_argument("--" + name, required=True)
    parser.add_argument("--daily", action="store_true")
    return parser.parse_args(argv)


def canonical_path(value):
    if (type(value) is not str or not value.startswith("/") or value == "/"
            or value.endswith("/") or "\x00" in value
            or any(part in ("", ".", "..") for part in value[1:].split("/"))):
        fail("SIMULATION_INPUTS_ARGUMENTS")
    return value


# ---- seams for the root-only filesystem rules (patched by local tests) ------------

def read_protected(path, gid, mode, maximum):
    """The installer's own protected-file reader (root-owned chain, exact mode, one link)."""
    try:
        return daily.read_root_file(path, gid=gid, mode=mode, maximum=maximum)
    except daily.DailyInstallError as error:
        raise OSError(str(error.args[0] if error.args else "")) from None


def root_directory(path):
    return daily.root_directory(path)


@contextlib.contextmanager
def open_output_dir(path):
    """Yield a descriptor of an empty root-owned 0700 directory under a root-owned chain."""
    try:
        with root_directory(path) as fd:
            check_output_dir(fd)
            yield fd
    except (daily.DailyInstallError, OSError):
        fail("SIMULATION_INPUTS_OUTPUT")


def check_output_dir(fd):
    info = os.fstat(fd)
    if (not stat.S_ISDIR(info.st_mode) or info.st_uid != ROOT_UID
            or stat.S_IMODE(info.st_mode) != 0o700 or os.listdir(fd)):
        fail("SIMULATION_INPUTS_OUTPUT")


def chown_root(fd):
    os.fchown(fd, ROOT_UID, 0)


# ---- inputs -------------------------------------------------------------------------

def read_plain(path, maximum, code):
    flags = os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC
    try:
        fd = os.open(path, flags)
    except OSError:
        fail(code)
    try:
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or info.st_size > maximum:
            fail(code)
        data = b""
        while len(data) <= maximum:
            chunk = os.read(fd, maximum + 1 - len(data))
            if not chunk:
                break
            data += chunk
    except OSError:
        fail(code)
    finally:
        os.close(fd)
    if len(data) != info.st_size:
        fail(code)
    return data


def serialize(value):
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"),
                       allow_nan=False) + "\n").encode("utf-8")


def installer(call, code, *args):
    try:
        return call(*args)
    except release.SimulationInstallError as error:
        fail(code, error.code)
    except daily.DailyInstallError as error:
        fail(code, str(error.args[0]) if error.args else "DAILY_INSTALL_INVALID")


def previous_simulation(data, previous_commit):
    return installer(release.validate_config, "SIMULATION_INPUTS_PREVIOUS", data, previous_commit)


def build_simulation(previous, commit, manifest_data, receipt_data, complete_data):
    """Return the new config bytes; every rule is delegated to the installer validators."""
    manifest = installer(release.validate_manifest, "SIMULATION_INPUTS_MANIFEST",
                         manifest_data, commit)
    if manifest["format"] != release.MANIFEST_V3_FORMAT:
        fail("SIMULATION_INPUTS_MANIFEST")
    image_id = manifest["image_ids"].get(release.BIN)
    if image_id is None:
        fail("SIMULATION_INPUTS_MANIFEST")
    try:
        receipt = release._json_object(receipt_data, release.RECEIPT_LIMIT)
    except release.SimulationInstallError:
        fail("SIMULATION_INPUTS_RECEIPT")
    digest = hashlib.sha256(receipt_data).hexdigest()
    if complete_data != digest.encode("ascii") + b"\n" or receipt.get("source_commit") != commit:
        fail("SIMULATION_INPUTS_RECEIPT")
    bound = {}
    for config_key, receipt_key in (("binary_sha256", "binary_sha256"),
                                    ("source_input_sha256", "input_sha256"),
                                    ("recipe_sha256", "recipe_sha256")):
        value = receipt.get(receipt_key)
        if type(value) is not str or re.fullmatch(r"[0-9a-f]{64}", value) is None:
            fail("SIMULATION_INPUTS_RECEIPT")
        bound[config_key] = "sha256:" + value
    config = copy.deepcopy(previous)  # fixed constants, validated above
    config["code_commit"] = commit
    config["manifest_sha256"] = manifest["sha256"]
    config["image_id"] = image_id
    config["receipt_sha256"] = "sha256:" + digest
    config.update(bound)
    data = serialize(config)
    installer(release.validate_binding, "SIMULATION_INPUTS_BINDING",
              data, manifest_data, receipt_data, complete_data, commit)
    for key in SIM_CARRIED:  # the five free values come from the previous release unchanged
        if config[key] != previous[key]:
            fail("SIMULATION_INPUTS_BINDING")
    return data


def assert_daily_only_commit_differs(old, new):
    if old.get("first_auto_session") != new.get("first_auto_session"):
        fail("SIMULATION_INPUTS_DAILY_SESSION")
    changed = {key for key in set(old) | set(new) if old.get(key) != new.get(key)}
    if changed != {"code_commit"}:
        fail("SIMULATION_INPUTS_DAILY")


def build_daily(previous_data, previous_commit, commit):
    old = installer(daily.validate_config, "SIMULATION_INPUTS_DAILY", previous_data, previous_commit)
    new = copy.deepcopy(old)
    new["code_commit"] = commit
    assert_daily_only_commit_differs(old, new)
    data = serialize(new)
    installer(daily.validate_config, "SIMULATION_INPUTS_DAILY", data, commit)
    return data


def is_current_release(release_dir):
    """The previous release must be the target of <install-root>/current."""
    root = os.path.dirname(os.path.dirname(release_dir))
    try:
        return os.path.realpath(os.path.join(root, "current"), strict=True) == \
            os.path.realpath(release_dir, strict=True)
    except OSError:
        return False


def build_all(commit, previous_release, manifest, receipt, complete, with_daily):
    """Read inputs and build every output as {file name: (bytes, mode)}; writes nothing."""
    previous_commit = os.path.basename(previous_release)
    if COMMIT_RE.fullmatch(previous_commit) is None or previous_commit == commit:
        fail("SIMULATION_INPUTS_ARGUMENTS")
    if not is_current_release(previous_release):
        fail("SIMULATION_INPUTS_PREVIOUS_NOT_CURRENT")
    manifest_data = read_plain(manifest, release.MANIFEST_LIMIT, "SIMULATION_INPUTS_MANIFEST")
    receipt_data = read_plain(receipt, release.RECEIPT_LIMIT, "SIMULATION_INPUTS_RECEIPT")
    complete_data = read_plain(complete, release.COMPLETE_BYTES, "SIMULATION_INPUTS_RECEIPT")
    try:
        previous_data = read_protected(
            f"{previous_release}/{release.CONFIG_DIRECTORY}/{release.CONFIG_NAME}",
            READER_GID, 0o440, release.CONFIG_LIMIT)
    except OSError:
        fail("SIMULATION_INPUTS_PREVIOUS")
    previous = previous_simulation(previous_data, previous_commit)
    outputs = {
        SIM_CONFIG_FILE: (build_simulation(previous, commit, manifest_data, receipt_data,
                                           complete_data), 0o600),
        SIM_RECEIPT_FILE: (receipt_data, 0o444),
        SIM_COMPLETE_FILE: (complete_data, 0o444),
    }
    if with_daily:
        try:
            daily_previous = read_protected(
                f"{previous_release}/{daily.CONFIG_DIRECTORY}/config.json",
                READER_GID, 0o440, daily.CONFIG_LIMIT)
        except OSError:
            fail("SIMULATION_INPUTS_DAILY")
        outputs[DAILY_CONFIG_FILE] = (build_daily(daily_previous, previous_commit, commit), 0o600)
    return outputs, (manifest_data, receipt_data, complete_data)


def write_outputs(dir_fd, outputs):
    created = []
    try:
        for name, (data, mode) in outputs.items():
            try:
                fd = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW
                             | os.O_CLOEXEC, 0o600, dir_fd=dir_fd)
            except OSError:
                fail("SIMULATION_INPUTS_OUTPUT")
            created.append(name)
            try:
                offset = 0
                while offset < len(data):
                    written = os.write(fd, data[offset:])
                    if written <= 0:
                        fail("SIMULATION_INPUTS_OUTPUT")
                    offset += written
                chown_root(fd)
                os.fchmod(fd, mode)
                os.fsync(fd)
            except OSError:
                fail("SIMULATION_INPUTS_OUTPUT")
            finally:
                os.close(fd)
        os.fsync(dir_fd)
    except BaseException:
        for name in created:
            with contextlib.suppress(OSError):
                os.unlink(name, dir_fd=dir_fd)
        raise


def verify_outputs(output_dir, outputs, commit, inputs):
    """Re-read through the installer's reader and repeat the binding check."""
    manifest_data = inputs[0]
    seen = {}
    for name, (data, mode) in outputs.items():
        try:
            seen[name] = read_protected(f"{output_dir}/{name}", 0, mode,
                                        release.RECEIPT_LIMIT)
        except OSError:
            fail("SIMULATION_INPUTS_CHANGED")
        if seen[name] != data:
            fail("SIMULATION_INPUTS_CHANGED")
    installer(release.validate_binding, "SIMULATION_INPUTS_BINDING",
              seen[SIM_CONFIG_FILE], manifest_data, seen[SIM_RECEIPT_FILE],
              seen[SIM_COMPLETE_FILE], commit)
    if DAILY_CONFIG_FILE in seen:
        installer(daily.validate_config, "SIMULATION_INPUTS_DAILY",
                  seen[DAILY_CONFIG_FILE], commit)


def run(argv):
    args = parse_args(argv)
    commit = args.commit
    if COMMIT_RE.fullmatch(commit) is None or commit == "0" * 40:
        fail("SIMULATION_INPUTS_ARGUMENTS")
    paths = [canonical_path(getattr(args, name)) for name in
             ("manifest", "receipt", "complete", "previous_release", "output_dir")]
    manifest, receipt, complete, previous_release, output_dir = paths
    names = [SIM_CONFIG_FILE, SIM_RECEIPT_FILE, SIM_COMPLETE_FILE]
    if args.daily:
        names.append(DAILY_CONFIG_FILE)
    listing = ",".join(names)
    if args.mode == "plan":
        print(f"SIMULATION_RELEASE_INPUTS: PLAN commit={commit} files={listing}")
        return 0
    outputs, inputs = build_all(commit, previous_release, manifest, receipt, complete, args.daily)
    if args.mode == "preflight":
        with open_output_dir(output_dir):
            pass
        print(f"SIMULATION_RELEASE_INPUTS: PREFLIGHT commit={commit} files={listing}")
        return 0
    with open_output_dir(output_dir) as dir_fd:
        write_outputs(dir_fd, outputs)
        try:
            verify_outputs(output_dir, outputs, commit, inputs)
        except InputsError:
            for name in outputs:
                with contextlib.suppress(OSError):
                    os.unlink(name, dir_fd=dir_fd)
            raise
    print(f"SIMULATION_RELEASE_INPUTS: PASS commit={commit} files={listing}")
    return 0


def main(argv=None):
    try:
        return run(sys.argv[1:] if argv is None else argv)
    except InputsError as error:
        suffix = f" cause={error.cause}" if error.cause else ""
        print(f"SIMULATION_RELEASE_INPUTS: FAIL code={error.code}{suffix}", file=sys.stderr)
        return 1
    except Exception:
        print("SIMULATION_RELEASE_INPUTS: FAIL code=SIMULATION_INPUTS_INTERNAL", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
