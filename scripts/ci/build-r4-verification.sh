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
if [[ -e "$build_root" ]]; then
  echo 'R4 verification build output directory already exists.' >&2
  exit 2
fi
mkdir -m 700 "$build_root"
mkdir -m 700 "$delivery_dir"

checkout_status_before="$(git -C "$GITHUB_WORKSPACE" status --porcelain --untracked-files=all)"
if [[ -n "$checkout_status_before" ]]; then
  echo 'R4 verification build requires a clean source checkout.' >&2
  exit 2
fi

cat >"${build_root}/${BUILDER_SCRIPT_NAME}" <<'BUILDER'
#!/bin/sh
set -eu
umask 077

cd /source
apk add --no-cache build-base musl-dev pkgconf openssl-dev postgresql-dev

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

export CARGO_BUILD_JOBS=2
export CARGO_TARGET_DIR=/out/cargo-target
export CARGO_INCREMENTAL=0
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

cargo fmt --all --check
cargo clippy --locked --release -p job-queue --example verify_approved_simulation -- -D warnings
cargo test --locked --release -p job-queue --example verify_approved_simulation
cargo build --locked --release -p job-queue --example verify_approved_simulation

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
if entries != ["verify_approved_simulation"]:
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
PYTHON

delivery_entries="$(find "$delivery_dir" -mindepth 1 -maxdepth 1 -printf '%f\n' | sort)"
expected_delivery_entries=$'provenance.json\nverify_approved_simulation'
if [[ "$delivery_entries" != "$expected_delivery_entries" ]]; then
  echo 'R4 verification build delivery must contain only the executable and provenance JSON.' >&2
  exit 1
fi

rm -f "${build_root}/${BUILDER_SCRIPT_NAME}" "${build_root}/build-metadata.txt" \
  "${build_root}/rustc-version.txt" "${build_root}/rustc-vV.txt" \
  "${build_root}/toolchain.txt" "${build_root}/target.txt" "${build_root}/profile.txt"
