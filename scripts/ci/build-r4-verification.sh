#!/usr/bin/env bash
set -euo pipefail

readonly BUILDER_IMAGE='rust:1.97.1-alpine@sha256:3c38f3f82c2f3d73da3b38e18d279393a04cb43ddded0e35088a8c3324d40900'
readonly BUILDER_SCRIPT_NAME='build-in-container.sh'

: "${RUNNER_TEMP:?RUNNER_TEMP is required}"
: "${GITHUB_WORKSPACE:?GITHUB_WORKSPACE is required}"
: "${GITHUB_SHA:?GITHUB_SHA is required}"
: "${GITHUB_RUN_ID:?GITHUB_RUN_ID is required}"
: "${GITHUB_RUN_ATTEMPT:?GITHUB_RUN_ATTEMPT is required}"

if [[ ! "$GITHUB_SHA" =~ ^[0-9a-f]{40}$ ]]; then
  echo 'R4 verification build requires a full lowercase GitHub SHA.' >&2
  exit 2
fi
if [[ ! "$GITHUB_RUN_ID" =~ ^[0-9]+$ ]]; then
  echo 'R4 verification build requires a numeric workflow run ID.' >&2
  exit 2
fi
if [[ ! "$GITHUB_RUN_ATTEMPT" =~ ^[0-9]+$ ]]; then
  echo 'R4 verification build requires a numeric workflow run attempt.' >&2
  exit 2
fi
if [[ ! -d "$GITHUB_WORKSPACE" ]]; then
  echo 'R4 verification build checkout is unavailable.' >&2
  exit 2
fi

checked_out_sha="$(git -C "$GITHUB_WORKSPACE" rev-parse HEAD)"
if [[ "$checked_out_sha" != "$GITHUB_SHA" ]]; then
  echo 'R4 verification build checkout does not match GITHUB_SHA.' >&2
  exit 2
fi

build_root="${RUNNER_TEMP}/r4-verification-build"
delivery_dir="${build_root}/delivery"
proof_delivery_dir="${build_root}/proof-delivery"
if [[ -e "$build_root" ]]; then
  echo 'R4 verification build output directory already exists.' >&2
  exit 2
fi
mkdir -m 700 "$build_root"
mkdir -m 700 "$delivery_dir"
mkdir -m 700 "$proof_delivery_dir"

# R4_SOURCE_TEST_LOG_PARSER_BEGIN
cat >"${build_root}/verify-rust-test-log.py" <<'PYTHON'
import re
import sys
from collections import Counter
from pathlib import Path


TEST_ROW = re.compile(
    r"^test ([A-Za-z_][A-Za-z0-9_:]*) \.\.\. (ok|FAILED|ignored)$"
)
SUMMARY = re.compile(
    r"^test result: (ok|FAILED)\. ([0-9]+) passed; ([0-9]+) failed; "
    r"([0-9]+) ignored;.*$"
)
TEST_NAME = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)*$")


def fail(lane: str, reason: str) -> None:
    print(f"R4_SOURCE_TEST_LOG:FAIL:{lane}:{reason}", file=sys.stderr)
    raise SystemExit(1)


def main() -> None:
    if len(sys.argv) < 4:
        raise SystemExit("R4_SOURCE_TEST_LOG:FAIL:invalid parser arguments")
    log_path = Path(sys.argv[1])
    lane = sys.argv[2]
    expected = sys.argv[3:]
    if len(set(expected)) != len(expected) or any(
        not TEST_NAME.fullmatch(name) for name in expected
    ):
        fail(lane, "expected test inventory is malformed or duplicated")
    try:
        contents = log_path.read_text(encoding="utf-8", errors="strict")
    except (OSError, UnicodeError):
        fail(lane, "test log is unavailable or not UTF-8")
    if "SKIP:" in contents:
        fail(lane, "test log contains SKIP")

    summaries = [
        match
        for line in contents.splitlines()
        if (match := SUMMARY.fullmatch(line)) is not None
    ]
    if len(summaries) != 1:
        fail(lane, "test log must contain exactly one libtest summary")
    summary = summaries[0]
    summary_status, passed, failed, ignored = summary.groups()
    passed_count, failed_count, ignored_count = map(
        int, (passed, failed, ignored)
    )
    if summary_status != "ok" or failed_count or ignored_count:
        fail(lane, "test summary reports failure or ignored tests")

    rows = []
    for line in contents.splitlines():
        match = TEST_ROW.fullmatch(line)
        if match is not None:
            rows.append(match.groups())
    if len(rows) != len(expected) or passed_count != len(expected):
        fail(lane, "executed test row count does not match the frozen inventory")

    observed = Counter()
    for name, result in rows:
        matches = [
            candidate
            for candidate in expected
            if name == candidate or name.endswith(f"::{candidate}")
        ]
        if len(matches) != 1:
            fail(lane, "test row is unexpected or ambiguously qualified")
        observed[matches[0]] += 1
        if result != "ok":
            fail(lane, "named test did not pass")
    if any(observed[name] != 1 for name in expected):
        fail(lane, "a frozen test is missing or duplicated")
    print(f"R4_SOURCE_TEST_LOG:PASS:{lane}:{len(expected)}")


if __name__ == "__main__":
    main()
PYTHON
# R4_SOURCE_TEST_LOG_PARSER_END
chmod 600 "${build_root}/verify-rust-test-log.py"

checkout_status_before="$(git -C "$GITHUB_WORKSPACE" status --porcelain --untracked-files=all)"
if [[ -n "$checkout_status_before" ]]; then
  echo 'R4 verification build requires a clean source checkout.' >&2
  exit 2
fi

# R4_CONNECTION_PROOF_SOURCE_INVENTORY_BEGIN
proof_source_inventory_script="${build_root}/proof-source-inventory.py"
cat >"$proof_source_inventory_script" <<'PYTHON'
import ast
import os
import re
import stat
import subprocess
import sys
from pathlib import Path


def fail(reason: str) -> None:
    print(f"R4_CONNECTION_PROOF_SOURCE_INVENTORY:FAIL:{reason}", file=sys.stderr)
    raise SystemExit(1)


def safe_relative_path(value: str) -> bool:
    return (
        bool(value)
        and not value.startswith("/")
        and not any(
            char in value
            for char in (
                "\\", ":", "\n", "\r", "\t", "\v", "\f", "\x1c", "\x1d",
                "\x1e", "\x85", "\u2028", "\u2029", "\0",
            )
        )
        and all(part not in ("", ".", "..") for part in value.split("/"))
    )


def ensure_regular_source(root: Path, value: str) -> None:
    if not safe_relative_path(value):
        fail("source inventory contains an unsafe path")
    current = root
    parts = value.split("/")
    for index, part in enumerate(parts):
        current = current / part
        try:
            metadata = current.lstat()
        except OSError:
            fail("declared source input is unavailable")
        if stat.S_ISLNK(metadata.st_mode):
            fail("declared source input contains a symlink")
        if index < len(parts) - 1:
            if not stat.S_ISDIR(metadata.st_mode):
                fail("declared source path has a non-directory parent")
        elif not stat.S_ISREG(metadata.st_mode):
            fail("declared source input is not a regular file")
    try:
        current.resolve(strict=True).relative_to(root)
    except (OSError, ValueError):
        fail("declared source input escapes the checkout")


def normalize_relative(base: str, value: str) -> str:
    if not value or value.startswith("/") or "\\" in value or "\0" in value:
        fail("compile-time include path is unsafe")
    parts = []
    for part in (Path(base) / value).as_posix().split("/"):
        if part in ("", "."):
            continue
        if part == "..":
            if not parts:
                fail("compile-time include path escapes the checkout")
            parts.pop()
        else:
            parts.append(part)
    resolved = "/".join(parts)
    if not safe_relative_path(resolved):
        fail("compile-time include path is unsafe")
    return resolved


def include_invocations(source: str):
    pattern = re.compile(r"include_(?:bytes|str)!\s*\(")
    for match in pattern.finditer(source):
        start = match.end()
        depth = 1
        in_string = False
        escaped = False
        index = start
        while index < len(source) and depth:
            char = source[index]
            if in_string:
                if escaped:
                    escaped = False
                elif char == "\\":
                    escaped = True
                elif char == '"':
                    in_string = False
            else:
                if char == '"':
                    in_string = True
                elif char == "(":
                    depth += 1
                elif char == ")":
                    depth -= 1
            index += 1
        if depth:
            fail("compile-time include invocation is unterminated")
        yield source[start : index - 1].strip()


def rust_string_literals(expression: str) -> list[str]:
    tokens = re.findall(r'"(?:\\.|[^"\\])*"', expression)
    try:
        return [ast.literal_eval(token) for token in tokens]
    except (SyntaxError, ValueError):
        fail("compile-time include string is malformed")


def main() -> None:
    if len(sys.argv) != 3:
        fail("invalid inventory arguments")
    root = Path(sys.argv[1]).resolve(strict=True)
    output = Path(sys.argv[2])
    listing = subprocess.run(
        ["git", "-C", str(root), "ls-files", "-z", "--"],
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )
    if listing.returncode:
        fail("tracked source inventory is unavailable")
    try:
        tracked = [entry.decode("utf-8", "strict") for entry in listing.stdout.split(b"\0") if entry]
    except UnicodeDecodeError:
        fail("tracked source path is not UTF-8")
    if len(tracked) != len(set(tracked)):
        fail("tracked source inventory contains duplicates")

    selected = set()
    rust_sources = []
    for value in tracked:
        basename = value.rsplit("/", 1)[-1]
        if (
            value.endswith((".rs", ".sql"))
            or basename in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml")
        ):
            ensure_regular_source(root, value)
            selected.add(value)
        if value.endswith(".rs"):
            rust_sources.append(value)

    tracked_set = set(tracked)
    include_paths = set()
    string_pattern = re.compile(r'"(?:\\.|[^"\\])*"')
    for source_path in rust_sources:
        source_file = root / source_path
        try:
            source_text = source_file.read_text(encoding="utf-8", errors="strict")
        except (OSError, UnicodeError):
            fail("tracked Rust source is unavailable or not UTF-8")
        for expression in include_invocations(source_text):
            if 'env!("OUT_DIR")' in expression:
                continue
            if 'env!("CARGO_MANIFEST_DIR")' in expression:
                marker = 'env!("CARGO_MANIFEST_DIR")'
                suffix = expression[expression.index(marker) + len(marker) :]
                pieces = string_pattern.findall(suffix)
                try:
                    suffix_value = "".join(ast.literal_eval(piece) for piece in pieces)
                except (SyntaxError, ValueError):
                    fail("compile-time include path is malformed")
                package_dir = Path(source_path).parent
                while package_dir != Path(".") and (package_dir.as_posix() + "/Cargo.toml") not in tracked_set:
                    package_dir = package_dir.parent
                if package_dir == Path(".") and "Cargo.toml" not in tracked_set:
                    fail("compile-time include package manifest is unavailable")
                include_path = normalize_relative(package_dir.as_posix(), suffix_value.lstrip("/"))
            else:
                literals = rust_string_literals(expression)
                if len(literals) != 1 or expression != string_pattern.search(expression).group(0):
                    fail("compile-time include form is unsupported")
                include_path = normalize_relative(Path(source_path).parent.as_posix(), literals[0])
            ensure_regular_source(root, include_path)
            include_paths.add(include_path)

    paths = sorted(selected | include_paths)
    if not paths or len(paths) > 50000:
        fail("tracked source inventory is empty or exceeds its bound")
    encoded = ("\n".join(paths) + "\n").encode("utf-8")
    if len(encoded) > 8 * 1024 * 1024:
        fail("tracked source inventory exceeds its byte bound")
    try:
        fd = os.open(output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "wb") as stream:
            stream.write(encoded)
    except OSError:
        fail("private source inventory could not be created")
    print(f"R4_CONNECTION_PROOF_SOURCE_INVENTORY:PASS:{len(paths)}")


if __name__ == "__main__":
    main()
PYTHON
chmod 600 "$proof_source_inventory_script"
python3 "$proof_source_inventory_script" "$GITHUB_WORKSPACE" "${build_root}/proof-source-paths.txt"
# R4_CONNECTION_PROOF_SOURCE_INVENTORY_END

cat >"${build_root}/${BUILDER_SCRIPT_NAME}" <<'BUILDER'
#!/bin/sh
set -eu
umask 077

# The builder runs as root for apk. Hand private bind-mounted receipts and
# test logs back to the host runner without widening their 0600/0700 modes.
case "$R4_VERIFICATION_HOST_UID:$R4_VERIFICATION_HOST_GID" in
  *[!0-9:]*|:*|*:) exit 2 ;;
esac
handoff_private_build_outputs() {
  local path
  for path in \
    /out/simulation-build-metadata.txt \
    /out/simulation-source-paths-before.txt \
    /out/simulation-source-paths-after.txt \
    /out/simulation-source-hashes-before.tsv \
    /out/simulation-source-hashes-after.tsv \
    /out/test-logs \
    /out/proof-build-metadata.txt \
    /out/proof-source-paths-before.txt \
    /out/proof-source-paths-after.txt \
    /out/proof-source-hashes-before.tsv \
    /out/proof-source-hashes-after.tsv \
    /out/proof-test-logs \
    /out/proof-delivery; do
    [ -e "$path" ] || continue
    chown -R "$R4_VERIFICATION_HOST_UID:$R4_VERIFICATION_HOST_GID" "$path" || return 1
  done
}
finish_builder() {
  local status=$?
  trap - EXIT
  if ! handoff_private_build_outputs; then
    echo 'R4 private build output ownership handoff failed.' >&2
    [ "$status" -ne 0 ] || status=1
  fi
  exit "$status"
}
trap finish_builder EXIT

cd /source
apk add --no-cache build-base musl-dev pkgconf openssl-dev postgresql-dev python3

if ! rustup component list --installed | grep -q '^rustfmt-'; then
  rustup component add rustfmt
fi
if ! rustup component list --installed | grep -q '^clippy-'; then
  rustup component add clippy
fi

rustc_version="$(rustc --version)"
rustc_vv="$(rustc -vV)"
rustc_version_release="$(printf '%s\n' "$rustc_version" | awk '{ print $2; exit }')"
rustc_version_commit="$(printf '%s\n' "$rustc_version" | awk '{ gsub(/[()]/, "", $3); print $3; exit }')"
rustc_release="$(printf '%s\n' "$rustc_vv" | awk '$1 == "release:" { print $2; exit }')"
rustc_commit="$(printf '%s\n' "$rustc_vv" | awk '$1 == "commit-hash:" { print $2; exit }')"
rustc_target="$(printf '%s\n' "$rustc_vv" | awk '$1 == "host:" { print $2; exit }')"

if [ -z "$rustc_version_release" ] || [ -z "$rustc_version_commit" ]; then
  echo 'R4 verification build could not read rustc --version.' >&2
  exit 2
fi
if [ -z "$rustc_release" ] || [ -z "$rustc_commit" ] || [ -z "$rustc_target" ]; then
  echo 'R4 verification build could not read the actual Rust toolchain identity.' >&2
  exit 2
fi
if [ "$rustc_version_release" != "$rustc_release" ]; then
  echo 'R4 verification build Rust release output does not match rustc -vV.' >&2
  exit 2
fi
rustc_short_commit="$(printf '%.9s' "$rustc_commit")"
if [ "${#rustc_commit}" -ne 40 ] || [ "${#rustc_version_commit}" -ne 9 ] || [ "$rustc_version_commit" != "$rustc_short_commit" ]; then
  echo 'R4 verification build Rust commit output does not match rustc -vV.' >&2
  exit 2
fi
if [ "$rustc_release" != '1.97.1' ]; then
  echo 'R4 verification build Rust release differs from the pinned release.' >&2
  exit 2
fi
toolchain="rustc-${rustc_version_release}-${rustc_version_commit}"
case "$toolchain" in
  *[!A-Za-z0-9._-]*|'')
    echo 'R4 verification build produced an invalid toolchain token.' >&2
    exit 2
    ;;
esac
if [ "$rustc_target" != 'x86_64-unknown-linux-musl' ]; then
  echo 'R4 verification build requires the builder host target x86_64-unknown-linux-musl.' >&2
  exit 2
fi
case "${R4_VERIFICATION_GIT_COMMIT:-}" in
  *[!0-9a-f]*|'')
    echo 'R4 verification build requires a full lowercase source SHA.' >&2
    exit 2
    ;;
esac
if [ "${#R4_VERIFICATION_GIT_COMMIT}" -ne 40 ]; then
  echo 'R4 verification build requires a full lowercase source SHA.' >&2
  exit 2
fi
case "${R4_VERIFICATION_WORKFLOW_RUN_ID:-}" in
  *[!0-9]*|'')
    echo 'R4 verification build requires a numeric workflow run ID.' >&2
    exit 2
    ;;
esac
case "${R4_VERIFICATION_WORKFLOW_RUN_ATTEMPT:-}" in
  *[!0-9]*|'')
    echo 'R4 verification build requires a numeric workflow run attempt.' >&2
    exit 2
    ;;
esac
case "$R4_VERIFICATION_WORKFLOW_RUN_ID" in
  *[1-9]*) ;;
  *)
    echo 'R4 verification build requires a positive workflow run ID.' >&2
    exit 2
    ;;
esac
case "$R4_VERIFICATION_WORKFLOW_RUN_ATTEMPT" in
  *[1-9]*) ;;
  *)
    echo 'R4 verification build requires a positive workflow run attempt.' >&2
    exit 2
    ;;
esac

export CARGO_BUILD_JOBS=2
export CARGO_TARGET_DIR=/out/cargo-target
export CARGO_INCREMENTAL=0
export CARGO_TERM_COLOR=never
export R4_VERIFICATION_TOOLCHAIN="$toolchain"
export R4_VERIFICATION_TARGET="$rustc_target"
export R4_VERIFICATION_PROFILE=release

printf '%s\n' "$rustc_version" >/out/rustc-version.txt
printf '%s\n' "$rustc_vv" >/out/rustc-vV.txt
printf '%s\n' "$R4_VERIFICATION_TOOLCHAIN" >/out/toolchain.txt
printf '%s\n' "$R4_VERIFICATION_TARGET" >/out/target.txt
printf '%s\n' "$R4_VERIFICATION_PROFILE" >/out/profile.txt

example_path=crates/job-queue/examples/verify_approved_simulation.rs
lock_path=Cargo.lock
manifest_path=crates/job-queue/Cargo.toml
example_hash_before="$(sha256sum "/source/$example_path")"
example_hash_before="${example_hash_before%% *}"
lock_hash_before="$(sha256sum "/source/$lock_path")"
lock_hash_before="${lock_hash_before%% *}"
manifest_hash_before="$(sha256sum "/source/$manifest_path")"
manifest_hash_before="${manifest_hash_before%% *}"

simulation_source_paths() {
  {
    printf '%s\n' \
      Cargo.toml \
      Cargo.lock \
      crates/job-queue/Cargo.toml \
      crates/factor-engine/Cargo.toml \
      crates/selector/Cargo.toml \
      crates/portfolio-model/Cargo.toml \
      crates/market-data/Cargo.toml \
      crates/domain/Cargo.toml \
      data-pipelines/collectors/Cargo.toml \
      crates/kis-client/Cargo.toml \
      crates/job-queue/src/lib.rs \
      crates/job-queue/src/queue.rs \
      crates/job-queue/src/owner_beta/mod.rs \
      crates/job-queue/src/bin/owner-beta-simulation-runner.rs \
      crates/job-queue/examples/verify_approved_simulation.rs \
      crates/job-queue/tests/owner_beta_simulation_process.rs
    find crates/job-queue/src/owner_beta/simulation -type f -name '*.rs' -print
    find crates/job-queue/src/owner_beta/simulation_execution -type f -name '*.rs' -print
  } | LC_ALL=C sort -u
}

write_simulation_source_hashes() {
  local path digest
  while IFS= read -r path; do
    [ -n "$path" ] || continue
    if [ -L "/source/$path" ] || [ ! -f "/source/$path" ]; then
      echo 'R4 simulation build source input is missing or not a regular file.' >&2
      exit 1
    fi
    digest="$(sha256sum <"/source/$path")"
    digest="${digest%% *}"
    printf '%s\t%s\n' "$path" "$digest"
  done <"$1"
}

write_proof_source_hashes() {
# R4_CONNECTION_PROOF_SOURCE_HASH_BEGIN
  python3 - "$1" "$2" "$3" <<'PYTHON'
import hashlib
import os
import stat
import sys
from pathlib import Path


def fail(reason: str) -> None:
    print(f"R4_CONNECTION_PROOF_SOURCE_HASH:FAIL:{reason}", file=sys.stderr)
    raise SystemExit(1)


def safe_path(value: str) -> bool:
    return (
        bool(value)
        and not value.startswith("/")
        and not any(
            char in value
            for char in (
                "\\", ":", "\n", "\r", "\t", "\v", "\f", "\x1c", "\x1d",
                "\x1e", "\x85", "\u2028", "\u2029", "\0",
            )
        )
        and all(part not in ("", ".", "..") for part in value.split("/"))
    )


def main() -> None:
    if len(sys.argv) != 4:
        fail("invalid hash arguments")
    inventory = Path(sys.argv[1])
    output = Path(sys.argv[2])
    source_root = Path(sys.argv[3])
    if inventory.is_symlink() or output.exists() or output.is_symlink():
        fail("source inventory or hash output is unsafe")
    try:
        raw = inventory.read_bytes()
        text = raw.decode("utf-8", "strict")
    except (OSError, UnicodeError):
        fail("source inventory is unavailable or not UTF-8")
    if not raw.endswith(b"\n") or b"\0" in raw or b"\r" in raw:
        fail("source inventory has invalid separators")
    paths = text[:-1].split("\n")
    if not paths or paths != sorted(set(paths)) or any(not safe_path(path) for path in paths):
        fail("source inventory is empty, duplicated, unsorted, or unsafe")

    rows = []
    for value in paths:
        current = source_root
        parts = value.split("/")
        for index, part in enumerate(parts):
            current = current / part
            try:
                info = current.lstat()
            except OSError:
                fail("declared source input is unavailable")
            if stat.S_ISLNK(info.st_mode):
                fail("declared source input contains a symlink")
            if index < len(parts) - 1:
                if not stat.S_ISDIR(info.st_mode):
                    fail("declared source path has a non-directory parent")
            elif not stat.S_ISREG(info.st_mode):
                fail("declared source input is not a regular file")
        digest = hashlib.sha256()
        try:
            with current.open("rb") as stream:
                for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                    digest.update(chunk)
        except OSError:
            fail("declared source input could not be hashed")
        rows.append(f"{value}\t{digest.hexdigest()}\n")
    encoded = "".join(rows).encode("utf-8")
    try:
        descriptor = os.open(output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(encoded)
    except OSError:
        fail("private source hash output could not be created")


if __name__ == "__main__":
    main()
PYTHON
# R4_CONNECTION_PROOF_SOURCE_HASH_END
}

if [ -e /out/proof-source-paths-before.txt ] || [ -e /out/proof-source-hashes-before.tsv ]; then
  echo 'R4 connection proof source inventory output already exists.' >&2
  exit 1
fi
cp /out/proof-source-paths.txt /out/proof-source-paths-before.txt
chmod 600 /out/proof-source-paths-before.txt
write_proof_source_hashes /out/proof-source-paths-before.txt \
  /out/proof-source-hashes-before.tsv /source

simulation_source_paths >/out/simulation-source-paths-before.txt
write_simulation_source_hashes /out/simulation-source-paths-before.txt \
  >/out/simulation-source-hashes-before.tsv
chmod 600 /out/simulation-source-paths-before.txt /out/simulation-source-hashes-before.tsv

# Hosted verification never inherits the distinct official-release origin.
unset R4_SIMULATION_BUILD_ORIGIN R4_SIMULATION_BUILD_SOURCE_INPUT_SHA256 \
  R4_SIMULATION_BUILD_RECIPE_SHA256
cargo fmt --all --check
cargo clippy --locked --release -p job-queue --example verify_approved_simulation -- -D warnings
mkdir -m 700 /out/test-logs
umask 077
cargo test --locked --release -p job-queue --example verify_approved_simulation -- \
  --nocapture --test-threads=1 >/out/test-logs/verify-approved-simulation.log 2>&1 || {
    echo 'R4 verification example tests failed; see the private build log.' >&2
    exit 1
  }
python3 /out/verify-rust-test-log.py /out/test-logs/verify-approved-simulation.log verifier \
  accepts_the_two_required_options_in_either_order \
  rejects_unknown_repeated_missing_and_relative_arguments \
  rejects_malformed_expected_binary_hashes \
  rejects_missing_or_invalid_compile_time_provenance

export R4_SIMULATION_BUILD_GIT_COMMIT="$R4_VERIFICATION_GIT_COMMIT"
export R4_SIMULATION_BUILD_TOOLCHAIN="$R4_VERIFICATION_TOOLCHAIN"
export R4_SIMULATION_BUILD_TARGET="$R4_VERIFICATION_TARGET"
export R4_SIMULATION_BUILD_PROFILE="$R4_VERIFICATION_PROFILE"
export R4_SIMULATION_BUILD_WORKFLOW_RUN_ID="$R4_VERIFICATION_WORKFLOW_RUN_ID"
export R4_SIMULATION_BUILD_WORKFLOW_RUN_ATTEMPT="$R4_VERIFICATION_WORKFLOW_RUN_ATTEMPT"

cargo clippy --locked --release -p job-queue --bin owner-beta-simulation-runner -- -D warnings
cargo test --locked --release -p job-queue --lib owner_beta::simulation_execution::child::tests -- \
  --nocapture --test-threads=1 >/out/test-logs/simulation-child.log 2>&1 || {
    echo 'R4 simulation child tests failed; see the private build log.' >&2
    exit 1
  }
python3 /out/verify-rust-test-log.py /out/test-logs/simulation-child.log simulation-child \
  child_request_and_pair_wire_are_bounded_and_strict \
  child_dispatch_uses_checked_version_and_keeps_the_v1_transport_envelope \
  fixture_process_uses_clean_environment_and_returns_handshake \
  fixture_process_escalates_reaps_leader_and_leaves_no_live_group_member \
  fixture_process_cleans_descendant_after_leader_exits \
  fixture_process_cancellation_reaps_the_owned_group \
  fixture_process_enforces_stdout_limit_while_reading \
  quickly_exiting_oversized_child_remains_output_too_large_after_join \
  broken_child_stdin_cannot_be_accepted_as_successful_input_delivery
cargo test --locked --release -p job-queue --lib owner_beta::simulation_execution::provenance::tests -- \
  --nocapture --test-threads=1 >/out/test-logs/simulation-provenance.log 2>&1 || {
    echo 'R4 simulation provenance tests failed; see the private build log.' >&2
    exit 1
  }
python3 /out/verify-rust-test-log.py /out/test-logs/simulation-provenance.log simulation-provenance \
  provenance_receipt_is_closed_duplicate_free_and_trailing_free \
  provenance_rejects_expected_receipt_compile_and_executable_mismatches \
  provenance_accepts_matching_receipt_compile_and_executable \
  provenance_requires_complete_compile_variables_and_safe_source_paths \
  checked_process_config_fixture_rejects_a_raw_byte_mutation_under_its_original_pin \
  provenance_origin_selection_is_compile_time_closed_and_disjoint \
  provenance_official_origin_rejects_hosted_receipts_and_hosted_origin_rejects_official \
  provenance_official_origin_requires_pinned_receipt_and_original_executable \
  official_checked_config_uses_shared_receipt_and_complete_binding_flow \
  official_checked_config_rejects_independent_pin_and_compile_mismatches_before_reads \
  official_checked_config_rejects_binary_binding_and_current_executable_mismatch \
  official_checked_config_rejects_raw_receipt_mutation_and_invalid_receipt_schema \
  official_checked_config_requires_exact_bounded_complete_marker \
  retained_official_binding_revalidates_the_same_config_and_original_pins
cargo test --locked --release -p job-queue --lib owner_beta::simulation_execution::runner::tests -- \
  --nocapture --test-threads=1 >/out/test-logs/simulation-runner.log 2>&1 || {
    echo 'R4 simulation runner tests failed; see the private build log.' >&2
    exit 1
  }
python3 /out/verify-rust-test-log.py /out/test-logs/simulation-runner.log simulation-runner \
  claim_and_lease_success_are_withheld_when_commit_fails \
  heartbeat_guard_waits_for_owned_task_before_returning \
  legacy_guard_aborts_pending_heartbeat_and_awaits_drop \
  official_shutdown_keeps_later_lease_uncertainty_while_legacy_keeps_priority \
  runner_classifies_only_static_child_exit_codes \
  runner_uses_one_process_slot_and_fixed_reconciliation_bound \
  runner_runtime_boundary_is_linux_only
# R4_PROCESS_INTEGRATION_TESTS_BEGIN
cargo test --locked --release -p job-queue --lib owner_beta::simulation_execution::process_ -- \
  --nocapture --test-threads=1 >/out/test-logs/simulation-process-integration.log 2>&1 || {
    echo 'R4 process integration tests failed; see the private build log.' >&2
    exit 1
  }
python3 /out/verify-rust-test-log.py /out/test-logs/simulation-process-integration.log simulation-process-integration \
  owner_beta::simulation_execution::process_acl::tests::command_result_discards_row_payload_and_rejects_presence \
  owner_beta::simulation_execution::process_acl::tests::positive_column_only_acl_observation_commits_closes_and_binds_checked_config \
  owner_beta::simulation_execution::process_acl::tests::timeout_milliseconds_are_checked_and_static_sql_is_bounded_and_catalog_only \
  owner_beta::simulation_execution::process_acl::tests::invalid_millisecond_bound_fails_before_acquiring_a_connection \
  owner_beta::simulation_execution::process_acl::tests::acquisition_and_begin_failures_close_only_when_a_connection_exists \
  owner_beta::simulation_execution::process_acl::tests::setup_and_each_catalog_failure_roll_back_then_close_without_later_queries \
  owner_beta::simulation_execution::process_acl::tests::precommit_rollback_or_close_failure_is_unconfirmed_and_both_are_attempted \
  owner_beta::simulation_execution::process_acl::tests::commit_and_close_uncertainty_never_roll_back_or_return_evidence \
  owner_beta::simulation_execution::process_acl::tests::context_identity_role_flags_and_session_settings_fail_closed \
  owner_beta::simulation_execution::process_acl::tests::context_requires_positive_bounded_oids_and_valid_names_and_distinct_owners \
  owner_beta::simulation_execution::process_acl::tests::any_outgoing_worker_membership_is_rejected_including_noinherit_set_role_edges \
  owner_beta::simulation_execution::process_acl::tests::relation_identity_owner_partition_and_table_privileges_are_exact \
  owner_beta::simulation_execution::process_acl::tests::exact_thirty_nine_column_shape_rejects_missing_extra_dropped_and_malformed_rows \
  owner_beta::simulation_execution::process_acl::tests::column_effective_write_grants_match_only_the_migration_matrix \
  owner_beta::simulation_execution::process_acl::tests::raw_acl_rejects_public_grant_options_unknown_and_unexpected_writes \
  owner_beta::simulation_execution::process_acl::tests::effective_grant_options_fail_shared_flow_when_raw_acls_are_allowed \
  owner_beta::simulation_execution::process_acl::tests::direct_raw_worker_grants_are_required_even_when_effective_checks_are_true \
  owner_beta::simulation_execution::process_acl::tests::malformed_duplicate_unknown_and_overlimit_raw_acl_rows_fail_closed \
  owner_beta::simulation_execution::process_acl::tests::every_catalog_collection_rejects_cap_plus_one_and_missing_context_rows \
  owner_beta::simulation_execution::process_acl::tests::cancellation_before_acquisition_and_after_acquisition_never_issues_sql \
  owner_beta::simulation_execution::process_acl::tests::cancellation_during_setup_each_query_and_before_commit_rolls_back_and_closes \
  owner_beta::simulation_execution::process_acl::tests::cancellation_during_commit_or_close_still_closes_and_withholds_evidence \
  owner_beta::simulation_execution::process_acl::tests::dropping_a_pending_query_future_keeps_no_evidence_and_never_reuses_connection \
  owner_beta::simulation_execution::process_acl::tests::successful_query_sequence_has_no_application_or_role_switch_statements \
  owner_beta::simulation_execution::process_configured_input_rights::tests::input_requires_v3_envelope_and_all_four_summary_pins \
  owner_beta::simulation_execution::process_configured_input_rights::tests::input_rejects_count_range_instrument_and_pair_mismatches \
  owner_beta::simulation_execution::process_configured_input_rights::tests::input_rejects_more_than_2452_dates_even_with_fixed_bar_count \
  owner_beta::simulation_execution::process_configured_input_rights::tests::actual_date_binding_uses_sorted_bar_dates_and_changes_with_the_input \
  owner_beta::simulation_execution::process_configured_input_rights::tests::config_hash_prefix_is_exact_and_all_zero_is_syntactically_allowed \
  owner_beta::simulation_execution::process_configured_input_rights::tests::config_rejects_bad_reference_and_checked_timeout_arithmetic_caps_per_statement_value \
  owner_beta::simulation_execution::process_configured_input_rights::tests::config_reference_preserves_unicode_and_enforces_exact_trimmed_value \
  owner_beta::simulation_execution::process_configured_input_rights::tests::setup_projection_requires_nonempty_values_and_context_checks_timeout_values \
  owner_beta::simulation_execution::process_configured_input_rights::tests::owner_requires_the_exact_configured_user_and_owner_role \
  owner_beta::simulation_execution::process_configured_input_rights::tests::context_requires_worker_identity_exact_settings_and_stable_snapshot \
  owner_beta::simulation_execution::process_configured_input_rights::tests::entitlement_row_limits_are_zero_to_sixty_four_with_a_sixty_five_row_sentinel \
  owner_beta::simulation_execution::process_configured_input_rights::tests::entitlement_ids_must_be_strictly_ordered_and_unique \
  owner_beta::simulation_execution::process_configured_input_rights::tests::known_inactive_statuses_are_decoded_but_unknown_status_is_rejected \
  owner_beta::simulation_execution::process_configured_input_rights::tests::entitlement_identity_is_exact_and_no_different_license_row_can_cover \
  owner_beta::simulation_execution::process_configured_input_rights::tests::exact_license_windows_allow_partitions_and_overlaps_but_reject_gaps \
  owner_beta::simulation_execution::process_configured_input_rights::tests::covers_any_is_recomputed_for_active_and_inactive_rows \
  owner_beta::simulation_execution::process_configured_input_rights::tests::identifier_json_is_bounded_distinct_control_free_and_exactly_typed \
  owner_beta::simulation_execution::process_configured_input_rights::tests::entitlement_dates_must_be_present_ordered_and_raw_utf8_must_be_complete \
  owner_beta::simulation_execution::process_configured_input_rights::tests::raw_decoder_rejects_null_contradictions_lying_lengths_caps_and_wrong_types \
  owner_beta::simulation_execution::process_configured_input_rights::tests::raw_payload_is_charged_from_materialized_bytes_before_row_semantics \
  owner_beta::simulation_execution::process_configured_input_rights::tests::raw_fake_shape_category_and_row_sentinel_checks_share_the_production_decoder \
  owner_beta::simulation_execution::process_configured_input_rights::tests::fetched_and_date_logical_payload_maxima_remain_inside_frozen_allowances \
  owner_beta::simulation_execution::process_configured_input_rights::tests::all_statement_plans_pin_sql_category_projection_bounds_and_typed_binds \
  owner_beta::simulation_execution::process_configured_input_rights::tests::projection_guard_rejects_mutations_in_each_real_sql_projection \
  owner_beta::simulation_execution::process_configured_input_rights::tests::command_result_classifier_projects_only_optional_row_presence \
  owner_beta::simulation_execution::process_configured_input_rights::tests::actual_date_vector_controls_interval_coverage_and_a_changed_vector_changes_identity \
  owner_beta::simulation_execution::process_configured_input_rights::tests::successful_fake_observation_uses_guarded_readonly_order_and_hides_values \
  owner_beta::simulation_execution::process_configured_input_rights::tests::acquisition_error_and_same_await_cancel_preserve_acquisition_failure \
  owner_beta::simulation_execution::process_configured_input_rights::tests::same_await_acquire_success_cancellation_closes_without_starting_sql \
  owner_beta::simulation_execution::process_configured_input_rights::tests::same_await_begin_success_cancel_and_begin_error_cancel_keep_uncertainty \
  owner_beta::simulation_execution::process_configured_input_rights::tests::successful_statement_same_await_cancellation_matrix_rolls_back_and_closes \
  owner_beta::simulation_execution::process_configured_input_rights::tests::failed_statement_same_await_cancellation_matrix_preserves_primary_failure \
  owner_beta::simulation_execution::process_configured_input_rights::tests::begin_nonempty_and_rollback_nonempty_are_uncertain_without_observation \
  owner_beta::simulation_execution::process_configured_input_rights::tests::commit_state_and_fetch_bound_same_await_cancel_keep_completion_uncertainty \
  owner_beta::simulation_execution::process_configured_input_rights::tests::commit_database_and_nonempty_commit_also_roll_back_then_close \
  owner_beta::simulation_execution::process_configured_input_rights::tests::acknowledged_commit_cancel_and_close_failure_never_create_observation \
  owner_beta::simulation_execution::process_configured_input_rights::tests::rollback_error_and_cancel_are_sticky_and_close_is_independent \
  owner_beta::simulation_execution::process_configured_input_rights::tests::dropped_pending_acquire_commit_rollback_and_close_do_not_claim_acknowledgement \
  owner_beta::simulation_execution::process_configured_input_rights::tests::private_errors_and_success_debug_never_include_sentinels_or_provider_text \
  owner_beta::simulation_execution::process_health::tests::maximum_sequence_rejects_valid_transition_without_mutation \
  owner_beta::simulation_execution::process_health::tests::maximum_minus_one_can_advance_once_to_maximum \
  owner_beta::simulation_execution::process_migrations::tests::accepted_baseline_is_finite_raw_and_fingerprint_pinned \
  owner_beta::simulation_execution::process_migrations::tests::baseline_rejects_down_paths_duplicates_and_version_gaps \
  owner_beta::simulation_execution::process_migrations::tests::baseline_rejects_byte_edits_and_newline_normalization \
  owner_beta::simulation_execution::process_migrations::tests::timeout_bound_rejects_zero_and_i32_millisecond_overflow \
  owner_beta::simulation_execution::process_migrations::tests::invalid_bound_and_pre_acquire_cancel_do_no_transport_io \
  owner_beta::simulation_execution::process_migrations::tests::acquisition_failure_does_not_arm_or_close_a_connection \
  owner_beta::simulation_execution::process_migrations::tests::canceled_immediately_after_acquire_arms_then_closes_without_begin \
  owner_beta::simulation_execution::process_migrations::tests::begin_failure_closes_without_claiming_rollback \
  owner_beta::simulation_execution::process_migrations::tests::odd_begin_result_rolls_back_once_then_closes \
  owner_beta::simulation_execution::process_migrations::tests::successful_observation_has_exact_order_and_retains_checked_identity \
  owner_beta::simulation_execution::process_migrations::tests::precommit_statement_failures_preserve_typed_failure_and_cleanup \
  owner_beta::simulation_execution::process_migrations::tests::precommit_cleanup_uncertainty_takes_precedence_over_original_error \
  owner_beta::simulation_execution::process_migrations::tests::cancellation_is_checked_at_each_observer_await \
  owner_beta::simulation_execution::process_migrations::tests::dropped_observation_future_has_armed_close_on_drop_and_no_async_cleanup_claim \
  owner_beta::simulation_execution::process_migrations::tests::context_identity_security_and_grant_option_checks_fail_closed \
  owner_beta::simulation_execution::process_migrations::tests::any_outgoing_worker_membership_is_rejected_after_shared_decoding \
  owner_beta::simulation_execution::process_migrations::tests::relation_shape_inheritance_and_effective_privileges_fail_closed \
  owner_beta::simulation_execution::process_migrations::tests::relation_oid_checked_bounds_and_upper_boundary_flow_through_observer \
  owner_beta::simulation_execution::process_migrations::tests::fixed_column_sql_projection_matches_shape_and_common_decoder \
  owner_beta::simulation_execution::process_migrations::tests::each_column_grant_option_is_denied_by_common_observer \
  owner_beta::simulation_execution::process_migrations::tests::exact_column_types_nullability_and_worker_privileges_are_checked \
  owner_beta::simulation_execution::process_migrations::tests::primary_key_metadata_requires_one_valid_public_version_key \
  owner_beta::simulation_execution::process_migrations::tests::raw_acl_requires_exact_direct_worker_select_and_rejects_public_or_extra_grants \
  owner_beta::simulation_execution::process_migrations::tests::logical_payload_budget_charges_cell_kinds_cumulatively_and_checks_overflow \
  owner_beta::simulation_execution::process_migrations::tests::raw_decoder_null_wrong_type_and_payload_bounds_share_production_path \
  owner_beta::simulation_execution::process_migrations::tests::ledger_requires_exact_order_success_tableoid_and_raw_checksum \
  owner_beta::simulation_execution::process_migrations::tests::ledger_unknown_65_is_distinguished_from_duplicate_65 \
  owner_beta::simulation_execution::process_migrations::tests::query_fetch_accept_limits_are_enforced_before_row_processing \
  owner_beta::simulation_execution::process_migrations::tests::commit_error_or_odd_result_never_rolls_back_or_retries \
  owner_beta::simulation_execution::process_migrations::tests::confirmed_commit_still_requires_close_and_postcommit_cancel_withholds_evidence \
  owner_beta::simulation_execution::process_migrations::tests::cancellation_does_not_erase_postcommit_cleanup_uncertainty \
  owner_beta::simulation_execution::process_migrations::tests::raw_wrong_type_is_decoded_before_policy_rejection \
  owner_beta::simulation_execution::process_migrations::tests::raw_acl_rejects_invalid_oids_attnums_and_missing_direct_grant \
  owner_beta::simulation_execution::process_migrations::tests::ledger_rejects_nonpositive_version_wrong_checksum_type_and_wrong_width \
  owner_beta::simulation_execution::process_migrations::tests::command_result_discards_row_payload_and_rejects_presence \
  owner_beta::simulation_execution::process_observation::tests::stat_parser_uses_field_22_and_accepts_opaque_comm_bytes \
  owner_beta::simulation_execution::process_observation::tests::stat_parser_rejects_wrong_pid_nul_short_state_and_unknown_states \
  owner_beta::simulation_execution::process_observation::tests::stat_parser_rejects_noncanonical_or_overflowed_start_ticks \
  owner_beta::simulation_execution::process_observation::tests::stat_parser_validates_optional_suffix_fields_without_parsing_the_identity_ticks \
  owner_beta::simulation_execution::process_observation::tests::stat_parser_enforces_the_exact_read_cap \
  owner_beta::simulation_execution::process_observation::tests::boot_parser_requires_one_canonical_non_nil_uuid_line \
  owner_beta::simulation_execution::process_observation::tests::clock_conversion_checks_signs_ranges_and_arithmetic_overflow \
  owner_beta::simulation_execution::process_observation::tests::capture_and_revalidate_share_ordered_flow_and_retain_one_generation \
  owner_beta::simulation_execution::process_observation::tests::current_pid_helper_uses_the_supplied_current_id_without_assuming_pid_one \
  owner_beta::simulation_execution::process_observation::tests::capture_rejects_boot_or_start_tick_changes_between_bounded_reads \
  owner_beta::simulation_execution::process_observation::tests::failed_revalidation_preserves_snapshot_then_invalidates_without_more_io \
  owner_beta::simulation_execution::process_observation::tests::failed_revalidation_on_boot_change_preserves_snapshot_and_poison_state \
  owner_beta::simulation_execution::process_observation::tests::revalidation_accepts_equal_time_and_suspend_sized_forward_jumps \
  owner_beta::simulation_execution::process_observation::tests::capture_and_revalidation_reject_backward_boot_clock_values \
  owner_beta::simulation_execution::process_observation::tests::capture_failure_boundaries_preserve_classified_flow_errors \
  owner_beta::simulation_execution::process_observation::tests::pidfd_poll_failures_at_capture_and_revalidation_boundaries_poison_handles \
  owner_beta::simulation_execution::process_observation::tests::unsupported_pidfd_factory_fails_without_replacement_fallback \
  owner_beta::simulation_execution::process_observation::tests::linux_errno_classification_is_fail_closed_and_value_free \
  owner_beta::simulation_execution::process_observation::tests::diagnostics_expose_only_static_type_and_error_names \
  owner_beta::simulation_execution::process_observation::tests::partial_descriptor_acquisition_drops_each_actual_handle_once \
  owner_beta::simulation_execution::process_observation::tests::capture_failure_after_descriptor_acquisition_drops_every_actual_handle \
  owner_beta::simulation_execution::process_observation::tests::production_policy_requests_fixed_nofollow_flags_and_retains_descriptors \
  owner_beta::simulation_execution::process_observation::tests::filesystem_type_and_held_or_named_types_fail_closed \
  owner_beta::simulation_execution::process_observation::tests::leaf_type_symlink_and_identity_replacements_use_shared_metadata_checks \
  owner_beta::simulation_execution::process_observation::tests::directory_and_leaf_replacements_are_checked_before_and_after_reads \
  owner_beta::simulation_execution::process_observation::tests::ordinary_missing_and_permission_errors_are_unavailable_on_the_shared_path \
  owner_beta::simulation_execution::process_observation::tests::bounded_leaf_reader_handles_short_reads_eof_overflow_and_eintr_without_retry \
  owner_beta::simulation_execution::process_observation::tests::pidfd_poll_statuses_events_and_syscall_errors_are_fail_closed \
  owner_beta::simulation_execution::process_observation::tests::successful_revalidation_uses_the_same_retained_pidfd_and_directories \
  owner_beta::simulation_execution::process_observation::tests::missing_named_entry_during_revalidation_poisoned_without_more_io \
  owner_beta::simulation_execution::process_observation::tests::non_linux_factories_return_unsupported_before_observation \
  owner_beta::simulation_execution::process_parent::tests::only_acknowledged_no_candidate_idle_counts_as_cycle \
  owner_beta::simulation_execution::process_parent::tests::failed_startup_never_invokes_claim_path \
  owner_beta::simulation_execution::process_parent::tests::canceled_and_failed_startup_never_admit_the_claim_path \
  owner_beta::simulation_execution::process_parent::tests::production_ack_controls_attempt_and_rejects_stale_epoch \
  owner_beta::simulation_execution::process_parent::tests::sender_full_closed_and_lost_ack_are_failures \
  owner_beta::simulation_execution::process_parent::tests::shutdown_rejects_old_event_and_first_fatal_and_poison_are_sticky \
  owner_beta::simulation_execution::process_parent::tests::lost_ack_after_apply_is_a_sticky_fatal \
  owner_beta::simulation_execution::process_parent::tests::same_await_signal_and_cleanup_error_keeps_error_and_closes_admission \
  owner_beta::simulation_execution::process_parent::tests::unconfirmed_commit_or_cleanup_prevents_next_claim \
  owner_beta::simulation_execution::process_parent::tests::confirmed_cycle_then_shutdown_is_a_clean_drain \
  owner_beta::simulation_execution::process_parent::tests::pending_claim_ack_after_signal_runs_owned_cleanup_without_stale_apply \
  owner_beta::simulation_execution::process_parent::tests::pending_lease_ack_after_signal_stops_heartbeat_without_false_uncertainty \
  owner_beta::simulation_execution::process_parent::tests::progress_failure_still_cleans_confirmed_claim_and_preserves_primary \
  owner_beta::simulation_execution::process_parent::tests::primary_shutdown_heartbeat_join_and_cleanup_facts_remain_independent \
  owner_beta::simulation_execution::process_parent::tests::fatal_completion_starts_bounded_cleanup_without_resetting_deadline \
  owner_beta::simulation_execution::process_parent::tests::real_lease_error_notifies_root_and_is_not_drain_suppression \
  owner_beta::simulation_execution::process_parent::tests::validation_error_during_shutdown_is_not_erased_by_settlement_outcome \
  owner_beta::simulation_execution::process_read::tests::config_reader_uses_read_only_no_follow_handles_and_exact_c3a_bytes \
  owner_beta::simulation_execution::process_read::tests::invalid_pins_are_rejected_before_io_and_c3a_rejects_bad_raw_config \
  owner_beta::simulation_execution::process_read::tests::config_metadata_acl_type_and_hardlink_policies_fail_closed \
  owner_beta::simulation_execution::process_read::tests::config_bounded_reads_cover_empty_size_limit_short_eof_and_eintr \
  owner_beta::simulation_execution::process_read::tests::exact_config_and_health_caps_are_accepted_with_bounded_reads \
  owner_beta::simulation_execution::process_read::tests::config_read_time_cap_plus_one_is_bounded_and_rejected \
  owner_beta::simulation_execution::process_read::tests::health_read_time_cap_plus_one_is_bounded_and_precedes_observation \
  owner_beta::simulation_execution::process_read::tests::config_leaf_mutation_fails_but_directory_timestamp_change_is_allowed \
  owner_beta::simulation_execution::process_read::tests::current_name_or_metadata_change_after_read_is_rejected_before_observation \
  owner_beta::simulation_execution::process_read::tests::config_root_and_parent_metadata_or_symlink_fail_closed \
  owner_beta::simulation_execution::process_read::tests::health_ready_and_busy_records_follow_c3b_and_observer_order \
  owner_beta::simulation_execution::process_read::tests::malformed_duplicate_and_unbound_health_never_reach_process_observer \
  owner_beta::simulation_execution::process_read::tests::generation_binding_and_final_observer_clock_control_readiness \
  owner_beta::simulation_execution::process_read::tests::observer_unavailability_is_typed_and_closes_the_read_set \
  owner_beta::simulation_execution::process_read::tests::c3b_phase_staleness_future_clock_and_sticky_failure_remain_authoritative \
  owner_beta::simulation_execution::process_read::tests::health_storage_metadata_acl_tmpfs_and_device_relationships_fail_closed \
  owner_beta::simulation_execution::process_read::tests::health_read_size_eof_and_interruption_are_bounded_and_close_descriptors \
  owner_beta::simulation_execution::process_read::tests::retained_file_and_lock_replacements_during_observation_are_rejected \
  owner_beta::simulation_execution::process_read::tests::partial_acquisition_closes_every_descriptor_and_static_errors_do_not_echo_paths \
  owner_beta::simulation_execution::process_read::tests::root_and_child_open_errors_share_typed_classification_without_retry \
  owner_beta::simulation_execution::process_read::tests::non_linux_public_factories_fail_before_io_or_config_inspection \
  owner_beta::simulation_execution::process_rls_metadata::tests::every_catalog_projection_has_exact_top_level_order_alias_and_cast_type \
  owner_beta::simulation_execution::process_rls_metadata::tests::fixed_sql_is_bounded_catalog_only_and_never_reads_expression_or_body_text \
  owner_beta::simulation_execution::process_rls_metadata::tests::embedded_baseline_identity_relation_roles_and_ten_policy_contracts_are_exact \
  owner_beta::simulation_execution::process_rls_metadata::tests::good_control_uses_the_shared_decoder_and_returns_only_opaque_metadata \
  owner_beta::simulation_execution::process_rls_metadata::tests::strict_worker_context_checks_identity_owner_flags_and_grants \
  owner_beta::simulation_execution::process_rls_metadata::tests::role_resolution_requires_six_exact_distinct_positive_named_roles \
  owner_beta::simulation_execution::process_rls_metadata::tests::worker_membership_is_zero_even_when_inheritance_is_disabled \
  owner_beta::simulation_execution::process_rls_metadata::tests::relation_metadata_checks_every_identity_and_rls_flag \
  owner_beta::simulation_execution::process_rls_metadata::tests::either_direction_of_relation_inheritance_is_rejected \
  owner_beta::simulation_execution::process_rls_metadata::tests::all_ten_policy_identities_flags_and_clause_null_shapes_are_exact \
  owner_beta::simulation_execution::process_rls_metadata::tests::malformed_policy_role_array_shapes_stop_before_scalar_expansion \
  owner_beta::simulation_execution::process_rls_metadata::tests::policy_role_rows_link_the_exact_header_oid_relation_ordinal_and_role \
  owner_beta::simulation_execution::process_rls_metadata::tests::oid_conversion_rejects_public_zero_negative_and_overflow_but_accepts_maximum \
  owner_beta::simulation_execution::process_rls_metadata::tests::primitive_decoder_rejects_bad_type_category_count_null_pair_and_text_contract \
  owner_beta::simulation_execution::process_rls_metadata::tests::text_byte_cap_accepts_exact_boundary_and_rejects_overflow_sentinel \
  owner_beta::simulation_execution::process_rls_metadata::tests::final_context_identity_and_checked_policy_are_reread_before_commit \
  owner_beta::simulation_execution::process_rls_metadata::tests::each_query_cap_accepts_only_its_fixed_rows_and_rejects_the_sentinel \
  owner_beta::simulation_execution::process_rls_metadata::tests::logical_payload_budget_uses_checked_addition_and_charges_final_context \
  owner_beta::simulation_execution::process_rls_metadata::tests::timeout_is_checked_before_acquire_and_fixed_bind_helpers_are_exact \
  owner_beta::simulation_execution::process_rls_metadata::tests::setup_acquire_query_commit_and_cleanup_failures_are_classified_without_raw_text \
  owner_beta::simulation_execution::process_rls_metadata::tests::same_await_database_failures_keep_primary_and_record_cancellation_for_every_query \
  owner_beta::simulation_execution::process_rls_metadata::tests::same_await_state_and_fetch_bound_failures_keep_primary_with_cancellation \
  owner_beta::simulation_execution::process_rls_metadata::tests::same_await_primary_cancellation_and_failed_cleanup_states_stay_combined \
  owner_beta::simulation_execution::process_rls_metadata::tests::primary_rollback_close_and_cancellation_uncertainty_are_preserved_together \
  owner_beta::simulation_execution::process_rls_metadata::tests::cancellation_at_every_awaited_boundary_withholds_observation_and_closes \
  owner_beta::simulation_execution::process_rls_metadata::tests::pending_future_drop_is_only_claimed_after_close_on_drop_was_armed \
  owner_beta::simulation_execution::process_rls_metadata::tests::command_result_discards_row_payload_and_rejects_presence \
  owner_beta::simulation_execution::process_routine_metadata::tests::good_control_uses_the_shared_raw_decoder_and_returns_only_metadata \
  owner_beta::simulation_execution::process_routine_metadata::tests::statement_and_sqlx_bind_values_match_independent_expected_values \
  owner_beta::simulation_execution::process_routine_metadata::tests::routine_sql_rank_matches_independent_literal_fixture_order \
  owner_beta::simulation_execution::process_routine_metadata::tests::timeout_uses_only_checked_startup_seconds \
  owner_beta::simulation_execution::process_routine_metadata::tests::positive_oid_rejects_zero_negative_and_values_above_u32 \
  owner_beta::simulation_execution::process_routine_metadata::tests::worker_context_policy_and_final_identity_are_checked \
  owner_beta::simulation_execution::process_routine_metadata::tests::exactly_six_distinct_resolved_role_names_and_oids_are_required \
  owner_beta::simulation_execution::process_routine_metadata::tests::public_routine_overload_and_header_metadata_mutations_fail_before_arguments \
  owner_beta::simulation_execution::process_routine_metadata::tests::ordered_named_argument_types_namespace_and_counts_are_checked \
  owner_beta::simulation_execution::process_routine_metadata::tests::every_fixed_row_cap_fetches_one_overflow_sentinel_and_rejects_it \
  owner_beta::simulation_execution::process_routine_metadata::tests::raw_acl_rejects_public_unknown_grantor_unknown_grantee_extra_privileges_and_grant_options \
  owner_beta::simulation_execution::process_routine_metadata::tests::effective_execute_and_grant_option_are_independently_enforced_for_all_eighteen_pairs \
  owner_beta::simulation_execution::process_routine_metadata::tests::required_binding_execute_bits_must_remain_enabled \
  owner_beta::simulation_execution::process_routine_metadata::tests::row_text_and_cumulative_payload_caps_include_sentinels_and_checked_addition \
  owner_beta::simulation_execution::process_routine_metadata::tests::malformed_raw_types_and_field_counts_fail_closed \
  owner_beta::simulation_execution::process_routine_metadata::tests::sql_projection_aliases_types_order_caps_and_body_exclusion_are_fixed \
  owner_beta::simulation_execution::process_routine_metadata::tests::sql_projection_regression_rejects_omitted_reordered_and_extra_outer_columns \
  owner_beta::simulation_execution::process_routine_metadata::tests::lifecycle_failures_preserve_primary_rollback_commit_and_close_uncertainty \
  owner_beta::simulation_execution::process_routine_metadata::tests::combined_cancellation_commit_close_and_primary_cleanup_failures_are_preserved \
  owner_beta::simulation_execution::process_routine_metadata::tests::cancellation_at_each_observer_boundary_never_returns_evidence \
  owner_beta::simulation_execution::process_routine_metadata::tests::dropping_a_pending_observation_has_armed_the_disposable_connection_and_yields_no_token \
  owner_beta::simulation_execution::process_routine_metadata::tests::command_result_discards_row_payload_and_rejects_presence \
  owner_beta::simulation_execution::process_startup::tests::independent_receipt_pin_must_match_checked_config \
  owner_beta::simulation_execution::process_startup::tests::observed_identity_mismatch_is_rejected_at_production_gate \
  owner_beta::simulation_execution::process_startup::tests::each_worker_checkout_rejects_identity_role_and_timeout_contamination \
  owner_beta::simulation_execution::process_startup::tests::canceled_or_failed_owned_startup_step_cannot_return_evidence \
  owner_beta::simulation_execution::process_startup::tests::startup_preserves_error_and_cancellation_from_same_await \
  owner_beta::simulation_execution::process_trigger_metadata::tests::adopted_candidate_names_settings_and_counts_are_independent_literals \
  owner_beta::simulation_execution::process_trigger_metadata::tests::real_statement_sql_projections_match_the_shared_raw_decoder_shapes \
  owner_beta::simulation_execution::process_trigger_metadata::tests::bounded_projection_parser_rejects_omitted_reordered_extra_type_and_alias_drift \
  owner_beta::simulation_execution::process_trigger_metadata::tests::sqlx_projection_alias_order_and_primitive_types_are_fixed \
  owner_beta::simulation_execution::process_trigger_metadata::tests::typed_statement_plans_keep_timeout_and_oid_array_binds_exact \
  owner_beta::simulation_execution::process_trigger_metadata::tests::a_complete_fake_catalog_yields_only_a_private_observation_after_close \
  owner_beta::simulation_execution::process_trigger_metadata::tests::every_admitted_category_rejects_its_fetch_plus_one_row \
  owner_beta::simulation_execution::process_trigger_metadata::tests::context_roles_relations_and_context_drift_fail_closed \
  owner_beta::simulation_execution::process_trigger_metadata::tests::root_overloads_oids_and_proconfig_shapes_fail_closed \
  owner_beta::simulation_execution::process_trigger_metadata::tests::root_setting_keys_order_values_and_lengths_fail_closed \
  owner_beta::simulation_execution::process_trigger_metadata::tests::root_search_path_allows_only_optional_ascii_spaces_after_commas \
  owner_beta::simulation_execution::process_trigger_metadata::tests::shared_raw_decoder_rejects_wrong_types_null_pairs_and_length_lies \
  owner_beta::simulation_execution::process_trigger_metadata::tests::positive_oid_conversion_has_an_explicit_i64_and_u32_boundary \
  owner_beta::simulation_execution::process_trigger_metadata::tests::logical_payload_budget_uses_the_fixed_cell_costs_and_checked_limit \
  owner_beta::simulation_execution::process_trigger_metadata::tests::shared_raw_text_decoder_accepts_multibyte_utf8_and_rejects_invalid_bytes \
  owner_beta::simulation_execution::process_trigger_metadata::tests::trigger_numeric_candidates_and_update_vector_bounds_fail_closed \
  owner_beta::simulation_execution::process_trigger_metadata::tests::constraint_crosslinks_reject_non_slice_properties_and_flag_drift \
  owner_beta::simulation_execution::process_trigger_metadata::tests::missing_extra_unknown_and_redirected_trigger_rows_fail_closed \
  owner_beta::simulation_execution::process_trigger_metadata::tests::trigger_header_flags_event_masks_and_array_shapes_are_checked \
  owner_beta::simulation_execution::process_trigger_metadata::tests::trigger_defer_flags_match_ordinary_and_constraint_censuses \
  owner_beta::simulation_execution::process_trigger_metadata::tests::update_of_status_requires_the_resolved_live_attribute_crosslink \
  owner_beta::simulation_execution::process_trigger_metadata::tests::trigger_args_distinguish_empty_null_wrong_type_and_nonempty_bytes \
  owner_beta::simulation_execution::process_trigger_metadata::tests::all_thirteen_user_constraint_crosslinks_and_flags_are_checked \
  owner_beta::simulation_execution::process_trigger_metadata::tests::same_await_database_failures_keep_the_primary_and_record_cancellation \
  owner_beta::simulation_execution::process_trigger_metadata::tests::same_await_state_and_fetch_bound_failures_keep_their_classification \
  owner_beta::simulation_execution::process_trigger_metadata::tests::primary_cancellation_rollback_and_close_uncertainty_are_independent \
  owner_beta::simulation_execution::process_trigger_metadata::tests::unconfirmed_commit_rolls_back_then_closes_without_claiming_completion \
  owner_beta::simulation_execution::process_trigger_metadata::tests::commit_state_and_fetch_bound_failures_preserve_uncertainty_and_cleanup \
  owner_beta::simulation_execution::process_trigger_metadata::tests::postcommit_and_close_cancellation_never_publish_an_observation \
  owner_beta::simulation_execution::process_trigger_metadata::tests::prebegin_acquisition_and_begin_failures_preserve_lifecycle_states \
  owner_beta::simulation_execution::process_trigger_metadata::tests::dropping_pending_futures_records_only_guarded_disposal \
  owner_beta::simulation_execution::process_trigger_metadata::tests::command_result_discards_row_payload_and_rejects_presence \
  owner_beta::simulation_execution::process_trigger_metadata::tests::rollback_row_keeps_cleanup_unconfirmed_after_primary_failure \
  owner_beta::simulation_execution::process_write::tests::checked_startup_and_event_lifecycle_publish_c3b_records \
  owner_beta::simulation_execution::process_write::tests::initial_existing_current_pending_or_unknown_entry_is_dirty_without_mutation \
  owner_beta::simulation_execution::process_write::tests::nonblocking_lock_contender_never_observes_or_mutates_records \
  owner_beta::simulation_execution::process_write::tests::observation_failures_are_typed_and_do_not_cross_visibility \
  owner_beta::simulation_execution::process_write::tests::changed_generation_before_and_after_rename_has_boundary_specific_outcome \
  owner_beta::simulation_execution::process_write::tests::short_writes_and_reads_complete_with_exact_same_descriptor_readback \
  owner_beta::simulation_execution::process_write::tests::every_apply_failure_poison_prevents_all_later_io_and_keeps_lock_until_drop \
  owner_beta::simulation_execution::process_write::tests::syscall_failures_are_bounded_and_visibility_depends_on_rename_boundary \
  owner_beta::simulation_execution::process_write::tests::post_rename_observation_failure_is_unconfirmed_and_never_restored \
  owner_beta::simulation_execution::process_write::tests::post_rename_apply_failure_keeps_new_record_and_poisoned_lock \
  owner_beta::simulation_execution::process_write::tests::stable_in_range_size_disagreement_fails_before_rename_and_preserves_prior_current \
  owner_beta::simulation_execution::process_write::tests::stable_in_range_size_disagreement_after_rename_is_unconfirmed_and_poisoned \
  owner_beta::simulation_execution::process_write::tests::read_time_cap_plus_one_with_in_range_metadata_is_bounded_and_not_published \
  owner_beta::simulation_execution::process_write::tests::size_or_time_change_during_readback_fails_before_visibility \
  owner_beta::simulation_execution::process_write::tests::create_collision_preserves_pending_and_prior_current_without_retry \
  owner_beta::simulation_execution::process_write::tests::pre_rename_ancestor_or_current_replacement_keeps_old_publication \
  owner_beta::simulation_execution::process_write::tests::post_rename_ancestor_or_current_replacement_is_unconfirmed_and_not_restored \
  owner_beta::simulation_execution::process_write::tests::altered_readback_and_backward_clock_fail_closed_without_retry \
  owner_beta::simulation_execution::process_write::tests::zero_write_and_zero_length_read_are_terminal_failures \
  owner_beta::simulation_execution::process_write::tests::temporary_acl_and_creation_mode_faults_leave_the_pending_evidence \
  owner_beta::simulation_execution::process_write::tests::temporary_type_owner_group_link_device_and_size_faults_fail_before_write \
  owner_beta::simulation_execution::process_write::tests::effective_identity_and_acl_metadata_fail_before_observation \
  owner_beta::simulation_execution::process_write::tests::apply_rechecks_uid_and_gid_before_filesystem_or_observation_work \
  owner_beta::simulation_execution::process_write::tests::fixed_tree_metadata_modes_types_devices_and_tmpfs_are_checked \
  owner_beta::simulation_execution::process_write::tests::access_and_default_acls_are_rejected_on_every_retained_tree_object \
  owner_beta::simulation_execution::process_write::tests::acl_absence_errno_is_accepted_but_other_xattr_errors_fail_once \
  owner_beta::simulation_execution::process_write::tests::current_record_type_owner_group_mode_link_acl_device_and_size_faults_fail_closed \
  owner_beta::simulation_execution::process_write::tests::descriptor_errno_classification_never_retries_interrupts \
  owner_beta::simulation_execution::process_write::tests::raw_dirent_parser_rejects_truncation_duplicates_missing_names_and_slashes \
  owner_beta::simulation_execution::process_write::tests::malformed_inventory_stops_before_observation_and_temp_creation \
  owner_beta::simulation_execution::process_write::tests::interrupted_inventory_read_is_not_retried \
  owner_beta::simulation_execution::process_write::tests::missing_dot_entry_and_over_limit_inventory_are_rejected \
  owner_beta::simulation_execution::process_write::tests::replacing_retained_ancestor_lock_or_published_inode_before_apply_is_detected \
  owner_beta::simulation_execution::process_write::tests::health_sequence_overflow_maps_to_a_value_free_rejection \
  owner_beta::simulation_execution::process_write::tests::writer_and_errors_have_redacted_debug_output \
  owner_beta::simulation_execution::process_write::tests::non_linux_factory_fails_before_config_inspection_or_observation
# R4_PROCESS_INTEGRATION_TESTS_END
cargo test --locked --release -p job-queue --bin owner-beta-simulation-runner -- \
  --nocapture --test-threads=1 >/out/test-logs/simulation-runner-cli.log 2>&1 || {
    echo 'R4 simulation runner CLI test failed; see the private build log.' >&2
    exit 1
  }
python3 /out/verify-rust-test-log.py /out/test-logs/simulation-runner-cli.log simulation-runner-cli \
  official_cli_requires_exact_independent_pins \
  health_parser_accepts_only_the_three_explicit_pins_in_any_order \
  health_parser_rejects_missing_duplicate_unknown_mixed_and_malformed_pins \
  health_parser_rejects_non_utf8_pin_values \
  health_dispatch_reads_config_then_checks_receipt_then_probes_without_other_effects \
  health_dispatch_stops_before_probe_on_config_error_or_receipt_mismatch \
  health_dispatch_maps_probe_failure_and_non_linux_without_adapter_io \
  existing_parent_and_child_argument_shapes_remain_accepted \
  ctrl_c_and_sigterm_request_the_same_cooperative_shutdown
cargo test --locked --release -p job-queue --test owner_beta_simulation_process -- \
  --nocapture --test-threads=1 >/out/test-logs/simulation-process.log 2>&1 || {
    echo 'R4 simulation process tests failed; see the private build log.' >&2
    exit 1
  }
python3 /out/verify-rust-test-log.py /out/test-logs/simulation-process.log simulation-process \
  owner_beta_simulation_runner_requires_exactly_one_mode_and_closed_arguments \
  owner_beta_simulation_health_rejects_malformed_arguments_before_any_health_read \
  owner_beta_simulation_runner_rejects_provenance_before_database_connect \
  owner_beta_simulation_child_rejects_untrusted_startup_before_request_processing

unset R4_SIMULATION_BUILD_GIT_COMMIT R4_SIMULATION_BUILD_TOOLCHAIN \
  R4_SIMULATION_BUILD_TARGET R4_SIMULATION_BUILD_PROFILE \
  R4_SIMULATION_BUILD_WORKFLOW_RUN_ID R4_SIMULATION_BUILD_WORKFLOW_RUN_ATTEMPT \
  R4_SIMULATION_BUILD_ORIGIN R4_SIMULATION_BUILD_SOURCE_INPUT_SHA256 \
  R4_SIMULATION_BUILD_RECIPE_SHA256
cargo build --locked --release -p job-queue --example verify_approved_simulation
export R4_SIMULATION_BUILD_GIT_COMMIT="$R4_VERIFICATION_GIT_COMMIT"
export R4_SIMULATION_BUILD_TOOLCHAIN="$R4_VERIFICATION_TOOLCHAIN"
export R4_SIMULATION_BUILD_TARGET="$R4_VERIFICATION_TARGET"
export R4_SIMULATION_BUILD_PROFILE="$R4_VERIFICATION_PROFILE"
export R4_SIMULATION_BUILD_WORKFLOW_RUN_ID="$R4_VERIFICATION_WORKFLOW_RUN_ID"
export R4_SIMULATION_BUILD_WORKFLOW_RUN_ATTEMPT="$R4_VERIFICATION_WORKFLOW_RUN_ATTEMPT"
cargo build --locked --release -p job-queue --bin owner-beta-simulation-runner

binary_path="$CARGO_TARGET_DIR/release/examples/verify_approved_simulation"
if [ ! -f "$binary_path" ] || [ ! -x "$binary_path" ]; then
  echo 'R4 verification build did not produce the expected executable.' >&2
  exit 1
fi
elf_header="$(readelf -h "$binary_path")"
case "$elf_header" in
  *'ELF Header:'*) ;;
  *)
    echo 'R4 verification build output is not an ELF executable.' >&2
    exit 1
    ;;
esac
program_headers="$(readelf -l "$binary_path")"
case "$program_headers" in
  *INTERP*)
    echo 'R4 verification executable has PT_INTERP and is not static musl.' >&2
    exit 1
    ;;
esac
cp "$binary_path" /out/delivery/verify_approved_simulation
chmod 755 /out/delivery/verify_approved_simulation

simulation_binary_path="$CARGO_TARGET_DIR/release/owner-beta-simulation-runner"
if [ ! -f "$simulation_binary_path" ] || [ ! -x "$simulation_binary_path" ]; then
  echo 'R4 simulation build did not produce the expected executable.' >&2
  exit 1
fi
simulation_elf_header="$(readelf -h "$simulation_binary_path")"
case "$simulation_elf_header" in
  *'ELF Header:'*) ;;
  *)
    echo 'R4 simulation build output is not an ELF executable.' >&2
    exit 1
    ;;
esac
simulation_program_headers="$(readelf -l "$simulation_binary_path")"
case "$simulation_program_headers" in
  *INTERP*)
    echo 'R4 simulation executable has PT_INTERP and is not static musl.' >&2
    exit 1
    ;;
esac
cp "$simulation_binary_path" /out/delivery/owner-beta-simulation-runner
chmod 755 /out/delivery/owner-beta-simulation-runner

export R4_CONNECTION_PROOF_BUILD_GIT_COMMIT="$R4_VERIFICATION_GIT_COMMIT"
export R4_CONNECTION_PROOF_BUILD_TOOLCHAIN="$R4_VERIFICATION_TOOLCHAIN"
export R4_CONNECTION_PROOF_BUILD_TARGET="$R4_VERIFICATION_TARGET"
export R4_CONNECTION_PROOF_BUILD_PROFILE="$R4_VERIFICATION_PROFILE"
export R4_CONNECTION_PROOF_BUILD_WORKFLOW_RUN_ID="$R4_VERIFICATION_WORKFLOW_RUN_ID"
export R4_CONNECTION_PROOF_BUILD_WORKFLOW_RUN_ATTEMPT="$R4_VERIFICATION_WORKFLOW_RUN_ATTEMPT"

cargo clippy --locked --release -p api-server --example verify_approved_simulation_connection -- -D warnings
mkdir -m 700 /out/proof-test-logs
cargo test --locked --release -p api-server --example verify_approved_simulation_connection -- --nocapture --test-threads=1 \
  >/out/proof-test-logs/connection-proof.log 2>&1 || {
    echo 'R4 connection proof source tests failed; see the private build log.' >&2
    exit 1
  }
python3 /out/verify-rust-test-log.py /out/proof-test-logs/connection-proof.log connection-proof \
  proof_cli_rejects_missing_duplicate_unknown_and_nonisolated_options \
  proof_receipts_bind_origin_compile_and_binaries \
  proof_scratch_guard_requires_socket_nonce_and_empty_cluster \
  proof_control_files_are_private_closed_and_no_overwrite \
  proof_pair_rejects_mixed_binding_and_truncated_content \
  proof_role_url_preserves_unix_socket_query
printf 'R4_CONNECTION_PROOF_TEST_LOG:PASS:6\n'
cargo build --locked --release -p api-server --example verify_approved_simulation_connection

proof_binary_path="$CARGO_TARGET_DIR/release/examples/verify_approved_simulation_connection"
if [ ! -f "$proof_binary_path" ] || [ ! -x "$proof_binary_path" ]; then
  echo 'R4 connection proof build did not produce the expected executable.' >&2
  exit 1
fi
proof_elf_header="$(readelf -h "$proof_binary_path")"
case "$proof_elf_header" in
  *'ELF Header:'*) ;;
  *)
    echo 'R4 connection proof output is not an ELF executable.' >&2
    exit 1
    ;;
esac
proof_program_headers="$(readelf -l "$proof_binary_path")"
case "$proof_program_headers" in
  *INTERP*)
    echo 'R4 connection proof executable has PT_INTERP and is not static musl.' >&2
    exit 1
    ;;
esac
cp "$proof_binary_path" /out/proof-delivery/verify_approved_simulation_connection
chmod 755 /out/proof-delivery/verify_approved_simulation_connection
proof_binary_hash_line="$(sha256sum /out/proof-delivery/verify_approved_simulation_connection)"
proof_binary_hash="${proof_binary_hash_line%% *}"
proof_binary_size="$(stat -c '%s' /out/proof-delivery/verify_approved_simulation_connection)"
cat >/out/proof-build-metadata.txt <<METADATA
git_commit=$R4_CONNECTION_PROOF_BUILD_GIT_COMMIT
toolchain=$R4_CONNECTION_PROOF_BUILD_TOOLCHAIN
target=$R4_CONNECTION_PROOF_BUILD_TARGET
profile=$R4_CONNECTION_PROOF_BUILD_PROFILE
workflow_run_id=$R4_CONNECTION_PROOF_BUILD_WORKFLOW_RUN_ID
workflow_run_attempt=$R4_CONNECTION_PROOF_BUILD_WORKFLOW_RUN_ATTEMPT
binary_sha256=$proof_binary_hash
binary_size_bytes=$proof_binary_size
METADATA
chmod 600 /out/proof-build-metadata.txt

example_hash_after="$(sha256sum "/source/$example_path")"
example_hash_after="${example_hash_after%% *}"
lock_hash_after="$(sha256sum "/source/$lock_path")"
lock_hash_after="${lock_hash_after%% *}"
manifest_hash_after="$(sha256sum "/source/$manifest_path")"
manifest_hash_after="${manifest_hash_after%% *}"
if [ "$example_hash_before" != "$example_hash_after" ]; then
  echo 'R4 verification build source example changed during the build.' >&2
  exit 1
fi
if [ "$lock_hash_before" != "$lock_hash_after" ]; then
  echo 'R4 verification build Cargo.lock changed during the build.' >&2
  exit 1
fi
if [ "$manifest_hash_before" != "$manifest_hash_after" ]; then
  echo 'R4 verification build job-queue manifest changed during the build.' >&2
  exit 1
fi

simulation_source_paths >/out/simulation-source-paths-after.txt
if ! cmp -s /out/simulation-source-paths-before.txt /out/simulation-source-paths-after.txt; then
  echo 'R4 simulation build source input set changed during the build.' >&2
  exit 1
fi
write_simulation_source_hashes /out/simulation-source-paths-after.txt \
  >/out/simulation-source-hashes-after.tsv
if ! cmp -s /out/simulation-source-hashes-before.tsv /out/simulation-source-hashes-after.tsv; then
  echo 'R4 simulation build source input changed during the build.' >&2
  exit 1
fi

if [ -e /out/proof-source-paths-after.txt ] || [ -e /out/proof-source-hashes-after.tsv ]; then
  echo 'R4 connection proof after-build source inventory already exists.' >&2
  exit 1
fi
cp /out/proof-source-paths.txt /out/proof-source-paths-after.txt
chmod 600 /out/proof-source-paths-after.txt
if ! cmp -s /out/proof-source-paths-before.txt /out/proof-source-paths-after.txt; then
  echo 'R4 connection proof source input set changed during the build.' >&2
  exit 1
fi
write_proof_source_hashes /out/proof-source-paths-after.txt \
  /out/proof-source-hashes-after.tsv /source
if ! cmp -s /out/proof-source-hashes-before.tsv /out/proof-source-hashes-after.tsv; then
  echo 'R4 connection proof source input changed during the build.' >&2
  exit 1
fi

binary_hash_line="$(sha256sum /out/delivery/verify_approved_simulation)"
binary_hash="${binary_hash_line%% *}"
binary_size="$(stat -c '%s' /out/delivery/verify_approved_simulation)"
cat >/out/build-metadata.txt <<METADATA
git_commit=$R4_VERIFICATION_GIT_COMMIT
workflow_run_id=$R4_VERIFICATION_WORKFLOW_RUN_ID
workflow_run_attempt=$R4_VERIFICATION_WORKFLOW_RUN_ATTEMPT
builder_image=$R4_VERIFICATION_BUILDER_IMAGE
rustc_version=$rustc_version
toolchain=$R4_VERIFICATION_TOOLCHAIN
target=$R4_VERIFICATION_TARGET
profile=$R4_VERIFICATION_PROFILE
source_example_sha256=$example_hash_before
source_cargo_lock_sha256=$lock_hash_before
source_job_queue_manifest_sha256=$manifest_hash_before
binary_sha256=$binary_hash
binary_size_bytes=$binary_size
METADATA
chmod 644 /out/build-metadata.txt /out/rustc-version.txt /out/rustc-vV.txt

simulation_binary_hash_line="$(sha256sum /out/delivery/owner-beta-simulation-runner)"
simulation_binary_hash="${simulation_binary_hash_line%% *}"
simulation_binary_size="$(stat -c '%s' /out/delivery/owner-beta-simulation-runner)"
cat >/out/simulation-build-metadata.txt <<METADATA
git_commit=$R4_SIMULATION_BUILD_GIT_COMMIT
toolchain=$R4_SIMULATION_BUILD_TOOLCHAIN
target=$R4_SIMULATION_BUILD_TARGET
profile=$R4_SIMULATION_BUILD_PROFILE
workflow_run_id=$R4_SIMULATION_BUILD_WORKFLOW_RUN_ID
workflow_run_attempt=$R4_SIMULATION_BUILD_WORKFLOW_RUN_ATTEMPT
binary_sha256=$simulation_binary_hash
binary_size_bytes=$simulation_binary_size
METADATA
chmod 600 /out/simulation-build-metadata.txt
BUILDER
chmod 600 "${build_root}/${BUILDER_SCRIPT_NAME}"

docker_args=(
  run
  --rm
  --mount "type=bind,source=${GITHUB_WORKSPACE},target=/source,readonly"
  --mount "type=bind,source=${build_root},target=/out"
  --workdir /source
  --env "R4_VERIFICATION_GIT_COMMIT=${GITHUB_SHA}"
  --env "R4_VERIFICATION_WORKFLOW_RUN_ID=${GITHUB_RUN_ID}"
  --env "R4_VERIFICATION_WORKFLOW_RUN_ATTEMPT=${GITHUB_RUN_ATTEMPT}"
  --env "R4_VERIFICATION_BUILDER_IMAGE=${BUILDER_IMAGE}"
  --env "R4_VERIFICATION_HOST_UID=$(id -u)"
  --env "R4_VERIFICATION_HOST_GID=$(id -g)"
  "$BUILDER_IMAGE"
  /bin/sh
  "/out/${BUILDER_SCRIPT_NAME}"
)
docker "${docker_args[@]}"

checkout_status_after="$(git -C "$GITHUB_WORKSPACE" status --porcelain --untracked-files=all)"
if [[ "$checkout_status_after" != "$checkout_status_before" ]]; then
  echo 'R4 verification build changed the source checkout.' >&2
  exit 1
fi

python3 "$proof_source_inventory_script" "$GITHUB_WORKSPACE" \
  "${build_root}/proof-source-paths-after-host.txt"
if ! cmp -s "${build_root}/proof-source-paths.txt" \
  "${build_root}/proof-source-paths-after-host.txt"; then
  echo 'R4 connection proof host source inventory changed during the build.' >&2
  exit 1
fi

python3 - "$build_root" "$delivery_dir" "$GITHUB_SHA" "$GITHUB_RUN_ID" "$GITHUB_RUN_ATTEMPT" "$BUILDER_IMAGE" <<'PYTHON'
import hashlib
import json
import os
import re
import sys
from pathlib import Path

build_root = Path(sys.argv[1])
delivery_dir = Path(sys.argv[2])
expected_commit, expected_run_id, expected_attempt, expected_builder = sys.argv[3:]

metadata_path = build_root / "build-metadata.txt"
metadata = {}
for line in metadata_path.read_text(encoding="ascii").splitlines():
    key, separator, value = line.partition("=")
    if not separator or key in metadata:
        raise SystemExit("R4 verification build metadata is malformed.")
    metadata[key] = value

expected_keys = {
    "git_commit", "workflow_run_id", "workflow_run_attempt", "builder_image",
    "rustc_version", "toolchain", "target", "profile",
    "source_example_sha256", "source_cargo_lock_sha256",
    "source_job_queue_manifest_sha256", "binary_sha256", "binary_size_bytes",
}
if set(metadata) != expected_keys:
    raise SystemExit("R4 verification build metadata fields are incomplete or unexpected.")
if metadata["git_commit"] != expected_commit:
    raise SystemExit("R4 verification build source SHA does not match the workflow SHA.")
if metadata["workflow_run_id"] != expected_run_id:
    raise SystemExit("R4 verification build run ID does not match the workflow.")
if metadata["workflow_run_attempt"] != expected_attempt:
    raise SystemExit("R4 verification build attempt does not match the workflow.")
if metadata["builder_image"] != expected_builder:
    raise SystemExit("R4 verification build image pin does not match the workflow.")
if metadata["target"] != "x86_64-unknown-linux-musl":
    raise SystemExit("R4 verification build target is not the required musl target.")
if metadata["profile"] != "release":
    raise SystemExit("R4 verification build profile is not release.")
if metadata["toolchain"] != "rustc-1.97.1-8bab26f4f":
    raise SystemExit("R4 verification build toolchain does not match the pinned builder.")

binary_path = delivery_dir / "verify_approved_simulation"
provenance_path = delivery_dir / "provenance.json"
entries = sorted(path.name for path in delivery_dir.iterdir())
if entries != ["owner-beta-simulation-runner", "verify_approved_simulation"]:
    raise SystemExit("R4 verification build delivery directory has unexpected contents.")
if binary_path.is_symlink() or not binary_path.is_file() or not os.access(binary_path, os.X_OK):
    raise SystemExit("R4 verification build executable is missing or invalid.")

def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()

binary_hash = sha256(binary_path)
if binary_hash != metadata["binary_sha256"]:
    raise SystemExit("R4 verification build executable hash changed after compilation.")
if binary_path.stat().st_size != int(metadata["binary_size_bytes"]):
    raise SystemExit("R4 verification build executable size changed after compilation.")
with binary_path.open("rb") as stream:
    if stream.read(4) != b"\x7fELF":
        raise SystemExit("R4 verification build executable is not ELF.")

simulation_binary_path = delivery_dir / "owner-beta-simulation-runner"
simulation_provenance_path = delivery_dir / "simulation-runner-provenance.json"
if (
    simulation_binary_path.is_symlink()
    or not simulation_binary_path.is_file()
    or not os.access(simulation_binary_path, os.X_OK)
):
    raise SystemExit("R4 simulation runner executable is missing or invalid.")
with simulation_binary_path.open("rb") as stream:
    if stream.read(4) != b"\x7fELF":
        raise SystemExit("R4 simulation runner executable is not ELF.")

simulation_metadata = {}
for line in (build_root / "simulation-build-metadata.txt").read_text(
    encoding="ascii"
).splitlines():
    key, separator, value = line.partition("=")
    if not separator or key in simulation_metadata:
        raise SystemExit("R4 simulation build metadata is malformed.")
    simulation_metadata[key] = value
expected_simulation_metadata_keys = {
    "git_commit", "toolchain", "target", "profile", "workflow_run_id",
    "workflow_run_attempt", "binary_sha256", "binary_size_bytes",
}
if set(simulation_metadata) != expected_simulation_metadata_keys:
    raise SystemExit("R4 simulation build metadata fields are incomplete or unexpected.")
if simulation_metadata["git_commit"] != expected_commit:
    raise SystemExit("R4 simulation source SHA does not match the workflow SHA.")
if simulation_metadata["workflow_run_id"] != expected_run_id:
    raise SystemExit("R4 simulation run ID does not match the workflow.")
if simulation_metadata["workflow_run_attempt"] != expected_attempt:
    raise SystemExit("R4 simulation attempt does not match the workflow.")
if not re.fullmatch(r"[0-9a-f]{40}", simulation_metadata["git_commit"]):
    raise SystemExit("R4 simulation source SHA is malformed.")
for field in ("workflow_run_id", "workflow_run_attempt"):
    value = simulation_metadata[field]
    if not value.isascii() or not value.isdigit() or not any(char != "0" for char in value):
        raise SystemExit("R4 simulation run identity is not a positive ASCII decimal.")
if simulation_metadata["toolchain"] != metadata["toolchain"]:
    raise SystemExit("R4 simulation toolchain does not match actual rustc identity.")
if simulation_metadata["target"] != "x86_64-unknown-linux-musl":
    raise SystemExit("R4 simulation target is not the required musl target.")
if simulation_metadata["profile"] != "release":
    raise SystemExit("R4 simulation profile is not release.")

simulation_binary_hash = sha256(simulation_binary_path)
if simulation_binary_hash != simulation_metadata["binary_sha256"]:
    raise SystemExit("R4 simulation runner hash changed after compilation.")
simulation_binary_size = simulation_binary_path.stat().st_size
if not simulation_metadata["binary_size_bytes"].isascii() or not simulation_metadata[
    "binary_size_bytes"
].isdigit() or simulation_binary_size != int(simulation_metadata["binary_size_bytes"]):
    raise SystemExit("R4 simulation runner size changed after compilation.")

simulation_source_hashes = {}
source_hash_lines = (build_root / "simulation-source-hashes-before.tsv").read_text(
    encoding="ascii"
).splitlines()
for line in source_hash_lines:
    source_path, separator, source_digest = line.partition("\t")
    if not separator or source_path in simulation_source_hashes:
        raise SystemExit("R4 simulation source hash inventory is malformed or duplicated.")
    pure_path = Path(source_path)
    if (
        not source_path
        or "\\" in source_path
        or ":" in source_path
        or pure_path.is_absolute()
        or any(part in ("", ".", "..") for part in source_path.split("/"))
        or not re.fullmatch(r"[0-9a-f]{64}", source_digest)
    ):
        raise SystemExit("R4 simulation source hash inventory contains an unsafe path or digest.")
    simulation_source_hashes[source_path] = source_digest
required_simulation_sources = {
    "Cargo.toml",
    "Cargo.lock",
    "crates/job-queue/Cargo.toml",
    "crates/job-queue/src/bin/owner-beta-simulation-runner.rs",
    "crates/job-queue/src/owner_beta/simulation/mod.rs",
    "crates/job-queue/src/owner_beta/simulation_execution/child.rs",
    "crates/job-queue/src/owner_beta/simulation_execution/mod.rs",
    "crates/job-queue/src/owner_beta/simulation_execution/provenance.rs",
    "crates/job-queue/src/owner_beta/simulation_execution/runner.rs",
}
if not required_simulation_sources.issubset(simulation_source_hashes):
    raise SystemExit("R4 simulation source hash inventory omits a required build input.")

rustc_vv = (build_root / "rustc-vV.txt").read_text(encoding="ascii").rstrip("\n")
rustc_version = (build_root / "rustc-version.txt").read_text(encoding="ascii").rstrip("\n")
if not rustc_vv or not rustc_version.startswith("rustc 1.97.1 ("):
    raise SystemExit("R4 verification build did not record the actual Rust toolchain.")
if not re.search(r"(?m)^host: x86_64-unknown-linux-musl$", rustc_vv):
    raise SystemExit("R4 verification build rustc -vV host does not match the required target.")

source_hashes = {
    "crates/job-queue/examples/verify_approved_simulation.rs": metadata["source_example_sha256"],
    "Cargo.lock": metadata["source_cargo_lock_sha256"],
    "crates/job-queue/Cargo.toml": metadata["source_job_queue_manifest_sha256"],
}
if any(not re.fullmatch(r"[0-9a-f]{64}", value) for value in source_hashes.values()):
    raise SystemExit("R4 verification build source hashes are malformed.")

provenance = {
    "schema_version": 1,
    "source_sha": metadata["git_commit"],
    "binary_sha256": f"sha256:{binary_hash}",
    "binary_size_bytes": binary_path.stat().st_size,
    "rustc_version": rustc_version,
    "rustc_vv": rustc_vv,
    "toolchain": metadata["toolchain"],
    "target": metadata["target"],
    "profile": metadata["profile"],
    "builder_image": metadata["builder_image"],
    "workflow_run_id": int(metadata["workflow_run_id"]),
    "workflow_run_attempt": int(metadata["workflow_run_attempt"]),
    "source_hashes": {
        path: f"sha256:{digest}" for path, digest in source_hashes.items()
    },
}
provenance_path.write_text(
    json.dumps(provenance, sort_keys=True, separators=(",", ":")) + "\n",
    encoding="utf-8",
)
provenance_path.chmod(0o600)

simulation_provenance = {
    "schema": "owner-beta-simulation-build-v1",
    "git_commit": simulation_metadata["git_commit"],
    "toolchain": simulation_metadata["toolchain"],
    "target": simulation_metadata["target"],
    "profile": simulation_metadata["profile"],
    "workflow_run_id": simulation_metadata["workflow_run_id"],
    "workflow_run_attempt": simulation_metadata["workflow_run_attempt"],
    "binary_sha256": simulation_binary_hash,
    "source_hashes": simulation_source_hashes,
}
if set(simulation_provenance) != {
    "schema", "git_commit", "toolchain", "target", "profile",
    "workflow_run_id", "workflow_run_attempt", "binary_sha256", "source_hashes",
}:
    raise SystemExit("R4 simulation provenance schema is incomplete or unexpected.")
simulation_provenance_path.write_text(
    json.dumps(simulation_provenance, sort_keys=True, separators=(",", ":")) + "\n",
    encoding="utf-8",
)
simulation_provenance_path.chmod(0o600)
PYTHON

# R4_CONNECTION_PROOF_RECEIPT_BEGIN
python3 - "$build_root" "$proof_delivery_dir" \
  "$delivery_dir/simulation-runner-provenance.json" "$GITHUB_WORKSPACE" \
  "$GITHUB_SHA" "$GITHUB_RUN_ID" "$GITHUB_RUN_ATTEMPT" <<'PYTHON'
import hashlib
import json
import os
import re
import stat
import sys
from pathlib import Path


def fail(reason: str) -> None:
    print(f"R4_CONNECTION_PROOF_RECEIPT:FAIL:{reason}", file=sys.stderr)
    raise SystemExit(1)


def safe_relative_path(value: str) -> bool:
    return (
        bool(value)
        and not value.startswith("/")
        and not any(
            char in value
            for char in (
                "\\", ":", "\n", "\r", "\t", "\v", "\f", "\x1c", "\x1d",
                "\x1e", "\x85", "\u2028", "\u2029", "\0",
            )
        )
        and all(part not in ("", ".", "..") for part in value.split("/"))
    )


def reject_duplicate_keys(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON key")
        result[key] = value
    return result


def read_regular(path: Path, max_bytes: int, expected_mode: int | None = None) -> bytes:
    try:
        info = path.lstat()
    except OSError:
        fail("required private input is unavailable")
    if not stat.S_ISREG(info.st_mode) or stat.S_ISLNK(info.st_mode):
        fail("private input is not a regular file")
    if expected_mode is not None and stat.S_IMODE(info.st_mode) != expected_mode:
        fail("private input mode is invalid")
    if info.st_size > max_bytes:
        fail("private input exceeds its bound")
    try:
        data = path.read_bytes()
    except OSError:
        fail("private input could not be read")
    if len(data) > max_bytes or len(data) != info.st_size:
        fail("private input changed while being read")
    return data


def digest_regular(path: Path, max_bytes: int, expected_mode: int) -> tuple[str, int, bytes]:
    try:
        info = path.lstat()
    except OSError:
        fail("required executable is unavailable")
    if (
        not stat.S_ISREG(info.st_mode)
        or stat.S_ISLNK(info.st_mode)
        or stat.S_IMODE(info.st_mode) != expected_mode
        or info.st_size > max_bytes
    ):
        fail("executable file type, mode, or size is invalid")
    digest = hashlib.sha256()
    size = 0
    first_bytes = b""
    try:
        with path.open("rb") as stream:
            while chunk := stream.read(1024 * 1024):
                if not first_bytes:
                    first_bytes = chunk[:4]
                size += len(chunk)
                if size > max_bytes:
                    fail("executable exceeds its bound")
                digest.update(chunk)
            opened = stream.fileno()
            opened_info = os.fstat(opened)
    except OSError:
        fail("executable could not be hashed")
    if not stat.S_ISREG(opened_info.st_mode) or size != info.st_size or opened_info.st_size != size:
        fail("executable changed while being hashed")
    return digest.hexdigest(), size, first_bytes


def read_metadata(path: Path, expected_keys: set[str]) -> dict[str, str]:
    raw = read_regular(path, 4096, 0o600)
    try:
        lines = raw.decode("ascii", "strict").splitlines()
    except UnicodeError:
        fail("build metadata is not ASCII")
    result = {}
    for line in lines:
        key, separator, value = line.partition("=")
        if not separator or not key or key in result or not value:
            fail("build metadata is malformed")
        result[key] = value
    if set(result) != expected_keys:
        fail("build metadata fields are incomplete or unexpected")
    return result


def source_inventory(path: Path) -> list[str]:
    raw = read_regular(path, 8 * 1024 * 1024, 0o600)
    if not raw.endswith(b"\n") or b"\0" in raw or b"\r" in raw:
        fail("source path list has invalid separators")
    try:
        values = raw.decode("utf-8", "strict")[:-1].split("\n")
    except UnicodeError:
        fail("source path list is not UTF-8")
    if not values or values != sorted(set(values)) or any(
        not safe_relative_path(value) for value in values
    ):
        fail("source path list is empty, unsorted, duplicated, or unsafe")
    return values


def source_hash_map(path: Path, expected_paths: list[str]) -> dict[str, str]:
    raw = read_regular(path, 8 * 1024 * 1024, 0o600)
    if not raw.endswith(b"\n") or b"\0" in raw or b"\r" in raw:
        fail("source hash list has invalid separators")
    try:
        lines = raw.decode("utf-8", "strict").splitlines()
    except UnicodeError:
        fail("source hash list is not UTF-8")
    values = []
    result = {}
    for line in lines:
        path_value, separator, digest = line.partition("\t")
        if (
            not separator
            or "\t" in digest
            or not safe_relative_path(path_value)
            or not re.fullmatch(r"[0-9a-f]{64}", digest)
            or path_value in result
        ):
            fail("source hash list contains a malformed or duplicate row")
        values.append(path_value)
        result[path_value] = digest
    if values != expected_paths or not result:
        fail("source hash list does not match the declared input set")
    return result


def check_source(root: Path, value: str) -> str:
    if not safe_relative_path(value):
        fail("source hash path is unsafe")
    current = root
    parts = value.split("/")
    for index, part in enumerate(parts):
        current = current / part
        try:
            info = current.lstat()
        except OSError:
            fail("hashed source input is unavailable")
        if stat.S_ISLNK(info.st_mode):
            fail("hashed source input contains a symlink")
        if index < len(parts) - 1:
            if not stat.S_ISDIR(info.st_mode):
                fail("hashed source path has a non-directory parent")
        elif not stat.S_ISREG(info.st_mode):
            fail("hashed source input is not a regular file")
    try:
        current.resolve(strict=True).relative_to(root)
        digest = hashlib.sha256()
        with current.open("rb") as stream:
            for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(chunk)
    except (OSError, ValueError):
        fail("hashed source input could not be verified")
    return digest.hexdigest()


def main() -> None:
    if len(sys.argv) != 8:
        fail("invalid receipt arguments")
    build_root = Path(sys.argv[1])
    proof_dir = Path(sys.argv[2])
    runner_receipt_path = Path(sys.argv[3])
    source_root = Path(sys.argv[4]).resolve(strict=True)
    expected_commit, expected_run_id, expected_attempt = sys.argv[5:]

    proof_dir_info = proof_dir.lstat()
    if (
        not stat.S_ISDIR(proof_dir_info.st_mode)
        or stat.S_ISLNK(proof_dir_info.st_mode)
        or stat.S_IMODE(proof_dir_info.st_mode) != 0o700
    ):
        fail("proof delivery directory is not private")

    proof_binary = proof_dir / "verify_approved_simulation_connection"
    before_names = sorted(path.name for path in proof_dir.iterdir())
    if before_names != [proof_binary.name]:
        fail("proof delivery directory has unexpected pre-receipt contents")
    proof_binary_hash, proof_binary_size, proof_binary_magic = digest_regular(
        proof_binary, 1024 * 1024 * 1024, 0o755
    )
    if not os.access(proof_binary, os.X_OK) or proof_binary_magic != b"\x7fELF":
        fail("proof executable is not an executable ELF file")

    metadata = read_metadata(
        build_root / "proof-build-metadata.txt",
        {
            "git_commit", "toolchain", "target", "profile", "workflow_run_id",
            "workflow_run_attempt", "binary_sha256", "binary_size_bytes",
        },
    )
    if not re.fullmatch(r"[0-9a-f]{40}", expected_commit):
        fail("expected source SHA is malformed")
    for value in (expected_run_id, expected_attempt):
        if not value.isascii() or not value.isdigit() or not any(ch != "0" for ch in value):
            fail("expected workflow identity is not a positive ASCII decimal")
    if (
        metadata["git_commit"] != expected_commit
        or metadata["workflow_run_id"] != expected_run_id
        or metadata["workflow_run_attempt"] != expected_attempt
        or not re.fullmatch(r"[0-9a-f]{40}", metadata["git_commit"])
        or metadata["target"] != "x86_64-unknown-linux-musl"
        or metadata["profile"] != "release"
        or not re.fullmatch(r"rustc-[A-Za-z0-9._-]+", metadata["toolchain"])
        or not re.fullmatch(r"[0-9a-f]{64}", metadata["binary_sha256"])
        or not metadata["binary_size_bytes"].isascii()
        or not metadata["binary_size_bytes"].isdigit()
        or int(metadata["binary_size_bytes"]) != proof_binary_size
        or proof_binary_hash != metadata["binary_sha256"]
    ):
        fail("proof build metadata does not bind the expected source and binary")

    runner_bytes = read_regular(runner_receipt_path, 1024 * 1024, 0o600)
    try:
        runner_receipt = json.loads(
            runner_bytes.decode("utf-8", "strict"), object_pairs_hook=reject_duplicate_keys
        )
    except (UnicodeError, json.JSONDecodeError, ValueError):
        fail("runner provenance receipt is malformed")
    runner_fields = {
        "schema", "git_commit", "toolchain", "target", "profile",
        "workflow_run_id", "workflow_run_attempt", "binary_sha256", "source_hashes",
    }
    if not isinstance(runner_receipt, dict) or set(runner_receipt) != runner_fields:
        fail("runner provenance fields are incomplete or unexpected")
    if runner_receipt["schema"] != "owner-beta-simulation-build-v1":
        fail("runner provenance schema is unexpected")
    runner_sources = runner_receipt["source_hashes"]
    if (
        not isinstance(runner_sources, dict)
        or not runner_sources
        or list(runner_sources) != sorted(runner_sources)
        or any(
            not isinstance(path, str) or not safe_relative_path(path)
            or not isinstance(digest, str)
            or not re.fullmatch(r"[0-9a-f]{64}", digest)
            for path, digest in runner_sources.items()
        )
    ):
        fail("runner provenance source map is invalid")
    for field in (
        "git_commit", "toolchain", "target", "profile", "workflow_run_id", "workflow_run_attempt"
    ):
        if runner_receipt[field] != metadata[field]:
            fail("proof and runner build identities do not match")
    runner_binary = runner_receipt_path.parent / "owner-beta-simulation-runner"
    runner_binary_hash, _, runner_binary_magic = digest_regular(
        runner_binary, 1024 * 1024 * 1024, 0o755
    )
    if (
        not os.access(runner_binary, os.X_OK)
        or runner_binary_magic != b"\x7fELF"
        or runner_binary_hash != runner_receipt["binary_sha256"]
    ):
        fail("runner receipt does not bind the separately delivered runner")

    path_inputs = (
        build_root / "proof-source-paths.txt",
        build_root / "proof-source-paths-before.txt",
        build_root / "proof-source-paths-after.txt",
        build_root / "proof-source-paths-after-host.txt",
    )
    inventories = [source_inventory(path) for path in path_inputs]
    source_paths = inventories[0]
    if any(paths != source_paths for paths in inventories[1:]):
        fail("source path inventory changed during the proof build")
    before_hashes = source_hash_map(build_root / "proof-source-hashes-before.tsv", source_paths)
    after_hashes = source_hash_map(build_root / "proof-source-hashes-after.tsv", source_paths)
    if before_hashes != after_hashes:
        fail("source hashes changed during the proof build")
    for path, digest in before_hashes.items():
        if check_source(source_root, path) != digest:
            fail("host source hash does not match the builder inventory")

    receipt = {
        "schema": "owner-beta-simulation-connection-proof-build-v1",
        "git_commit": metadata["git_commit"],
        "toolchain": metadata["toolchain"],
        "target": metadata["target"],
        "profile": metadata["profile"],
        "workflow_run_id": metadata["workflow_run_id"],
        "workflow_run_attempt": metadata["workflow_run_attempt"],
        "binary_sha256": metadata["binary_sha256"],
        "source_hashes": before_hashes,
    }
    if set(receipt) != {
        "schema", "git_commit", "toolchain", "target", "profile",
        "workflow_run_id", "workflow_run_attempt", "binary_sha256", "source_hashes",
    }:
        fail("connection proof receipt fields are incomplete or unexpected")
    receipt_path = proof_dir / "connection-proof-provenance.json"
    receipt_bytes = (json.dumps(receipt, sort_keys=True, separators=(",", ":")) + "\n").encode("utf-8")
    if len(receipt_bytes) > 1024 * 1024:
        fail("connection proof receipt exceeds one mebibyte")
    try:
        descriptor = os.open(receipt_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(receipt_bytes)
    except OSError:
        fail("connection proof receipt could not be created without overwrite")
    os.chmod(receipt_path, 0o600)

    expected_entries = sorted((proof_binary.name, receipt_path.name))
    actual_entries = sorted(path.name for path in proof_dir.iterdir())
    if actual_entries != expected_entries:
        fail("proof delivery directory does not contain exactly two files")
    read_regular(receipt_path, 1024 * 1024, 0o600)
    for path in (proof_binary, receipt_path):
        info = path.lstat()
        if not stat.S_ISREG(info.st_mode) or stat.S_ISLNK(info.st_mode):
            fail("proof delivery contains a non-regular file")
    print("R4_CONNECTION_PROOF_RECEIPT:PASS")


if __name__ == "__main__":
    main()
PYTHON
# R4_CONNECTION_PROOF_RECEIPT_END

delivery_entries="$(find "$delivery_dir" -mindepth 1 -maxdepth 1 -printf '%f\n' | sort)"
expected_delivery_entries=$'owner-beta-simulation-runner\nprovenance.json\nsimulation-runner-provenance.json\nverify_approved_simulation'
if [[ "$delivery_entries" != "$expected_delivery_entries" ]]; then
  echo 'R4 verification build delivery must contain exactly the four approved files.' >&2
  exit 1
fi
proof_delivery_entries="$(find "$proof_delivery_dir" -mindepth 1 -maxdepth 1 -printf '%f\n' | sort)"
expected_proof_delivery_entries=$'connection-proof-provenance.json\nverify_approved_simulation_connection'
if [[ "$proof_delivery_entries" != "$expected_proof_delivery_entries" ]]; then
  echo 'R4 connection proof delivery must contain exactly its two approved files.' >&2
  exit 1
fi

rm -f "${build_root}/${BUILDER_SCRIPT_NAME}" "${build_root}/build-metadata.txt" \
  "${build_root}/simulation-build-metadata.txt" \
  "${build_root}/proof-build-metadata.txt" \
  "${build_root}/rustc-version.txt" "${build_root}/rustc-vV.txt" \
  "${build_root}/toolchain.txt" "${build_root}/target.txt" "${build_root}/profile.txt" \
  "${build_root}/simulation-source-paths-before.txt" \
  "${build_root}/simulation-source-paths-after.txt" \
  "${build_root}/simulation-source-hashes-before.tsv" \
  "${build_root}/simulation-source-hashes-after.tsv" \
  "${build_root}/proof-source-paths.txt" \
  "${build_root}/proof-source-paths-before.txt" \
  "${build_root}/proof-source-paths-after.txt" \
  "${build_root}/proof-source-paths-after-host.txt" \
  "${build_root}/proof-source-hashes-before.tsv" \
  "${build_root}/proof-source-hashes-after.tsv" \
  "${build_root}/proof-source-inventory.py" \
  "${build_root}/verify-rust-test-log.py"
rm -rf "${build_root}/test-logs"
rm -rf "${build_root}/proof-test-logs"
