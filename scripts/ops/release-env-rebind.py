#!/usr/bin/env python3
"""Create the pending compose .env for a new release commit.

The previous release's .env is copied byte-for-byte except for exactly two
lines: LAGRANGE_CODE_COMMIT and RESEARCH_R2_ACTION_SUPPLY_CONFIG_HEX.  In the
supplier line the hex-encoded JSON keeps every field and only release_commit
changes.  The tool prints counts and closed codes only, never a value.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import secrets
import stat
import subprocess
import sys

COMMIT_KEY = b"LAGRANGE_CODE_COMMIT="
SUPPLY_KEY = b"RESEARCH_R2_ACTION_SUPPLY_CONFIG_HEX="
SUPPLY_SCHEMA = "r2-action-supply-v1"
# Mirrors `Config` in data-pipelines/collectors/src/action_supply.rs.
SUPPLY_KEYS = frozenset({
    "schema", "enabled", "release_commit", "owner", "policy_sha256", "activation_date",
    "base_candidate_sha256", "initial_civil_frontier", "entitlement_reference_sha256",
    "raw_owner_uid", "timeout_seconds",
})
MAX_HEX = 8192
MAX_ENV_BYTES = 1024 * 1024
COMMIT_RE = re.compile(r"[0-9a-f]{40}\Z")
HEX_RE = re.compile(r"(?:[0-9a-f]{2})+\Z")
HERE = os.path.dirname(os.path.abspath(__file__))
DOTENV_LIB = os.path.join(HERE, "lib", "dotenv.sh")
MANIFEST_LIB = os.path.join(HERE, "lib", "release-image-manifest.sh")


class RebindError(Exception):
    def __init__(self, code):
        super().__init__(code)
        self.code = code


def fail(code):
    raise RebindError(code)


class ClosedParser(argparse.ArgumentParser):
    def error(self, message):  # never echo argument values
        fail("ENV_REBIND_ARGUMENTS")


def parse_args(argv):
    parser = ClosedParser(add_help=False, allow_abbrev=False)
    group = parser.add_mutually_exclusive_group(required=True)
    for name in ("plan", "preflight", "apply"):
        group.add_argument("--" + name, dest="mode", action="store_const", const=name)
    parser.add_argument("--previous-env", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--output", required=True)
    return parser.parse_args(argv)


def canonical_path(value):
    if (type(value) is not str or not value.startswith("/") or value == "/"
            or value.endswith("/") or "\x00" in value
            or any(part in ("", ".", "..") for part in value[1:].split("/"))):
        fail("ENV_REBIND_ARGUMENTS")
    return value


def validate_arguments(args):
    commit = args.commit
    if COMMIT_RE.fullmatch(commit) is None or commit == "0" * 40:
        fail("ENV_REBIND_ARGUMENTS")
    previous = canonical_path(args.previous_env)
    output = canonical_path(args.output)
    parts = previous[1:].split("/")
    if (len(parts) < 5 or parts[-3:] != ["deploy", "compose", ".env"]
            or COMMIT_RE.fullmatch(parts[-4]) is None or parts[-4] == commit):
        fail("ENV_REBIND_ARGUMENTS")
    if output == previous:
        fail("ENV_REBIND_ARGUMENTS")
    return parts[-4], commit, previous, output


def is_current_release(release_dir):
    """The previous release must be the target of <install-root>/current."""
    root = os.path.dirname(os.path.dirname(release_dir))
    try:
        return os.path.realpath(os.path.join(root, "current"), strict=True) == \
            os.path.realpath(release_dir, strict=True)
    except OSError:
        return False


def _bash(script, *arguments):
    try:
        result = subprocess.run(
            ["bash", "-c", script, "_", *arguments], stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False, timeout=60,
            env={"PATH": "/usr/sbin:/usr/bin:/sbin:/bin", "LC_ALL": "C"})
    except (OSError, subprocess.SubprocessError):
        return False
    return result.returncode == 0


def trusted_file(path):
    """Real root:root 0600 and root-owned parent-chain rule used for --env-source."""
    return _bash('source "$1"; release_image_manifest_trusted_file "$2" env', MANIFEST_LIB, path)


def dotenv_ok(path):
    """Real strict dotenv parser used by the release gates."""
    return _bash('source "$1"; dotenv_load "$2"', DOTENV_LIB, path)


def read_previous(path):
    flags = os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC
    try:
        fd = os.open(path, flags)
    except OSError:
        fail("ENV_REBIND_SOURCE_TRUST")
    try:
        info = os.fstat(fd)
        if (not stat.S_ISREG(info.st_mode) or info.st_nlink != 1
                or info.st_size > MAX_ENV_BYTES):
            fail("ENV_REBIND_SOURCE_TRUST")
        chunks = []
        remaining = info.st_size + 1
        while remaining > 0:
            chunk = os.read(fd, min(65536, remaining))
            if not chunk:
                break
            chunks.append(chunk)
            remaining -= len(chunk)
        data = b"".join(chunks)
    finally:
        os.close(fd)
    if len(data) != info.st_size:
        fail("ENV_REBIND_SOURCE_TRUST")
    return data


def split_keepends(data):
    # Bash `read` splits on LF only; mirror that, not bytes.splitlines().
    return re.findall(rb"[^\n]*\n|[^\n]+", data)


def split_line(raw):
    """Separate one line from its ending exactly as dotenv_load strips it (LF, then one CR)."""
    body = raw[:-1] if raw.endswith(b"\n") else raw
    if body.endswith(b"\r"):
        body = body[:-1]
    return body, raw[len(body):]


def unique_pairs(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError
        result[key] = value
    return result


def _bad_constant(_):
    raise ValueError


def rebind_supply(value, commit):
    """Return the new lowercase-hex value for the supplier line."""
    if (len(value) > MAX_HEX or HEX_RE.fullmatch(value.decode("ascii", "replace")) is None):
        fail("ENV_REBIND_SUPPLY_SHAPE")
    try:
        config = json.loads(bytes.fromhex(value.decode("ascii")).decode("utf-8", "strict"),
                            object_pairs_hook=unique_pairs, parse_constant=_bad_constant)
    except (ValueError, RecursionError):
        fail("ENV_REBIND_SUPPLY_SHAPE")
    if (type(config) is not dict or config.get("schema") != SUPPLY_SCHEMA
            or set(config) != SUPPLY_KEYS
            or type(config["release_commit"]) is not str
            or COMMIT_RE.fullmatch(config["release_commit"]) is None):
        fail("ENV_REBIND_SUPPLY_SHAPE")
    config["release_commit"] = commit
    encoded = json.dumps(config, separators=(",", ":")).encode("utf-8").hex()
    if len(encoded) > MAX_HEX:
        fail("ENV_REBIND_SUPPLY_SHAPE")
    return encoded.encode("ascii")


def transform(previous_bytes, previous_commit, commit):
    """Pure byte transform: replace exactly the commit and supplier lines."""
    lines = split_keepends(previous_bytes)
    commit_at = [i for i, raw in enumerate(lines) if raw.startswith(COMMIT_KEY)]
    supply_at = [i for i, raw in enumerate(lines) if raw.startswith(SUPPLY_KEY)]
    if len(commit_at) != 1:
        fail("ENV_REBIND_COMMIT_LINE")
    body, ending = split_line(lines[commit_at[0]])
    if body[len(COMMIT_KEY):] != previous_commit.encode("ascii"):
        fail("ENV_REBIND_COMMIT_LINE")
    if not supply_at:
        fail("ENV_REBIND_SUPPLY_ABSENT")
    if len(supply_at) != 1:
        fail("ENV_REBIND_SUPPLY_SHAPE")
    output = list(lines)
    output[commit_at[0]] = COMMIT_KEY + commit.encode("ascii") + ending
    supply_body, supply_ending = split_line(lines[supply_at[0]])
    new_value = rebind_supply(supply_body[len(SUPPLY_KEY):], commit)
    output[supply_at[0]] = SUPPLY_KEY + new_value + supply_ending
    return b"".join(output)


def parse_env(data):
    values = {}
    for raw in split_keepends(data):
        line, _ = split_line(raw)
        if not line or line.startswith(b"#") or b"=" not in line:
            continue
        key, value = line.split(b"=", 1)
        values[key] = value
    return values


def self_check(previous_bytes, new_bytes, commit):
    """The compose-release.sh preserved-config rule: only these two keys differ."""
    old_lines = split_keepends(previous_bytes)
    new_lines = split_keepends(new_bytes)
    if len(old_lines) != len(new_lines):
        fail("ENV_REBIND_DIFF")
    old, new = parse_env(previous_bytes), parse_env(new_bytes)
    if set(old) != set(new):
        fail("ENV_REBIND_DIFF")
    changed = {key for key in old if old[key] != new[key]}
    if changed != {COMMIT_KEY[:-1], SUPPLY_KEY[:-1]}:
        fail("ENV_REBIND_DIFF")
    differing = [i for i, (a, b) in enumerate(zip(old_lines, new_lines)) if a != b]
    if len(differing) != 2:
        fail("ENV_REBIND_DIFF")
    if new[COMMIT_KEY[:-1]] != commit.encode("ascii"):
        fail("ENV_REBIND_DIFF")
    try:
        before = json.loads(bytes.fromhex(old[SUPPLY_KEY[:-1]].decode("ascii")))
        after = json.loads(bytes.fromhex(new[SUPPLY_KEY[:-1]].decode("ascii")))
    except (ValueError, UnicodeError):
        fail("ENV_REBIND_DIFF")
    if (after.get("release_commit") != commit
            or {k: v for k, v in after.items() if k != "release_commit"}
            != {k: v for k, v in before.items() if k != "release_commit"}):
        fail("ENV_REBIND_DIFF")


def output_absent(path):
    try:
        os.lstat(path)
    except FileNotFoundError:
        return
    except OSError:
        fail("ENV_REBIND_OUTPUT_TRUST")
    fail("ENV_REBIND_OUTPUT_EXISTS")


def write_no_clobber(path, data):
    directory, name = os.path.split(path)
    temporary = os.path.join(directory, f"{name}.tmp.{secrets.token_hex(8)}")
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC
    try:
        fd = os.open(temporary, flags, 0o600)
    except OSError:
        fail("ENV_REBIND_OUTPUT_TRUST")
    try:
        try:
            offset = 0
            while offset < len(data):
                written = os.write(fd, data[offset:])
                if written <= 0:
                    fail("ENV_REBIND_OUTPUT_TRUST")
                offset += written
            os.fchmod(fd, 0o600)
            os.fsync(fd)
        finally:
            os.close(fd)
        try:
            os.link(temporary, path)
        except FileExistsError:
            fail("ENV_REBIND_OUTPUT_EXISTS")
        except OSError:
            fail("ENV_REBIND_OUTPUT_TRUST")
    finally:
        try:
            os.unlink(temporary)
        except OSError:
            pass
    try:
        dir_fd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
        try:
            os.fsync(dir_fd)
        finally:
            os.close(dir_fd)
    except OSError:
        fail("ENV_REBIND_OUTPUT_TRUST")


def prepare(args):
    previous_commit, commit, previous, output = validate_arguments(args)
    if args.mode == "plan":
        return None
    if not is_current_release(previous.rsplit("/", 3)[0]):
        fail("ENV_REBIND_PREVIOUS_NOT_CURRENT")
    if not trusted_file(previous):
        fail("ENV_REBIND_SOURCE_TRUST")
    previous_bytes = read_previous(previous)
    if not dotenv_ok(previous):
        fail("ENV_REBIND_SOURCE_SYNTAX")
    new_bytes = transform(previous_bytes, previous_commit, commit)
    self_check(previous_bytes, new_bytes, commit)
    output_absent(output)
    return commit, output, previous_bytes, new_bytes


def run(argv):
    args = parse_args(argv)
    prepared = prepare(args)
    if args.mode == "plan":
        print(f"RELEASE_ENV_REBIND: PLAN commit={args.commit} changed_keys=2")
        return 0
    commit, output, previous_bytes, new_bytes = prepared
    if args.mode == "preflight":
        print(f"RELEASE_ENV_REBIND: PREFLIGHT commit={commit} changed_keys=2")
        return 0
    write_no_clobber(output, new_bytes)
    try:
        if read_previous_output(output) != new_bytes:
            fail("ENV_REBIND_OUTPUT_TRUST")
        if not trusted_file(output) or not dotenv_ok(output):
            fail("ENV_REBIND_OUTPUT_TRUST")
        self_check(previous_bytes, new_bytes, commit)
    except RebindError:
        try:
            os.unlink(output)
        except OSError:
            pass
        raise
    print(f"RELEASE_ENV_REBIND: PASS commit={commit} changed_keys=2")
    return 0


def read_previous_output(path):
    try:
        return read_previous(path)
    except RebindError:
        fail("ENV_REBIND_OUTPUT_TRUST")


def main(argv=None):
    try:
        return run(sys.argv[1:] if argv is None else argv)
    except RebindError as error:
        print(f"RELEASE_ENV_REBIND: FAIL code={error.code}", file=sys.stderr)
        return 1
    except Exception:
        print("RELEASE_ENV_REBIND: FAIL code=ENV_REBIND_INTERNAL", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
