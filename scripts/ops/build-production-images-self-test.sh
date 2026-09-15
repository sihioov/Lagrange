#!/usr/bin/env bash
# Fake-Docker self-test for the strict V2 owner-beta image build. It exercises
# only throw-away Git fixtures and never contacts a Docker daemon or provider.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
source_root=$(cd "$script_dir/../.." && pwd)
source_helper=$script_dir/build-production-images.sh
source_manifest_lib=$script_dir/lib/release-image-manifest.sh
source_layout_helper=$script_dir/lib/release-build-layout.sh

# Exercise the product gate functions without a live journal or Docker daemon.
# Query bounds must preserve the same microseconds that the parser validates.
python3 - "$source_layout_helper" <<'PY'
import ast
import datetime
import json
from pathlib import Path
import re
import sys

source = Path(sys.argv[1]).read_text()
gate = source.split("release_build_layout_gate() {", 1)[1]
body = gate.split("<<'PY'\n", 1)[1].split("\nPY\n", 1)[0]
names = {"utc_text", "parse_range", "pairs", "reject_constant", "normalized_boot"}
functions = [node for node in ast.parse(body).body
             if isinstance(node, ast.FunctionDef) and node.name in names]
assert {node.name for node in functions} == names
scope = {"datetime": datetime, "json": json, "re": re}
exec(compile(ast.Module(body=functions, type_ignores=[]), sys.argv[1], "exec"), scope)

epoch = datetime.datetime(1970, 1, 1, tzinfo=datetime.timezone.utc)
def parsed_query_us(value):
    stamp = datetime.datetime.strptime(value, "%Y-%m-%d %H:%M:%S.%f UTC").replace(
        tzinfo=datetime.timezone.utc)
    delta = stamp - epoch
    return (delta.days * 86400 + delta.seconds) * 1000000 + delta.microseconds

since = 1789494421803534
until = since + 1800 * 1000000
for stamp in (0, 1, 999999, 1000000, since, until):
    assert parsed_query_us(scope["utc_text"](stamp)) == stamp
assert scope["utc_text"](since) == "2026-09-15 17:47:01.803534 UTC"
assert scope["utc_text"](until) == "2026-09-15 18:17:01.803534 UTC"

boot = "a" * 32
def record(stamp, message="fixture kernel healthy"):
    return (json.dumps({"__REALTIME_TIMESTAMP": str(stamp), "__CURSOR": "fixture",
                        "_BOOT_ID": boot, "_TRANSPORT": "kernel", "MESSAGE": message}) + "\n").encode()

for stamp in (since, since + 1, until - 1, until):
    assert scope["parse_range"](record(stamp), boot, since, until, 1) == (1, 0)
for stamp in (since - 671516, since - 1, until + 1):
    try:
        scope["parse_range"](record(stamp), boot, since, until, 1)
    except ValueError as error:
        assert str(error) == "range-outside-window"
    else:
        raise AssertionError("journal parser accepted a genuinely out-of-window record")

# Simulate journalctl's selection using the actual formatted query bounds.
# A whole-second formatter incorrectly admits the first timestamp here.
stamps = (since - 671516, since, until, until + 1)
selected = [stamp for stamp in stamps
            if parsed_query_us(scope["utc_text"](since)) <= stamp
            <= parsed_query_us(scope["utc_text"](until))]
assert selected == [since, until]
assert scope["parse_range"](b"".join(map(record, selected)), boot, since, until, 2) == (2, 0)
assert scope["parse_range"](record(since, "Out of memory: Killed process fixture"),
                            boot, since, until, 1) == (1, 1)
print("PRODUCTION_IMAGE_BUILD_JOURNAL_PRECISION_SELF_TEST: PASS")
PY

# Exercise the frozen public image-save parser with private synthetic OCI
# archives. The two positive shapes match the observed Docker `image save`
# forms: an index directly naming a runnable manifest and an index naming a
# provenance index which in turn names one runnable linux/amd64 manifest plus
# an attestation descriptor. No archive is extracted and no Docker daemon is
# contacted. Negative archives cover root and layer digest tampering,
# selected-path whiteouts, opaque/ancestor replacement, links, and escaping
# tar names.
run_archive_parser_tests() (
  set -euo pipefail
  [ -f "$source_layout_helper" ] && [ ! -L "$source_layout_helper" ] || {
    echo 'self-test: G2 archive helper is missing' >&2
    return 1
  }
  # The generic parser is deliberately the product implementation. Do not copy
  # its private receipt schema into this test.
  source "$source_layout_helper"

  local archive_dir request result expected_map direct_archive direct_id index_archive index_id root_record root_archive root_id
  local path_kind archive_arg request_arg result_arg canonical_result
  archive_dir=$(mktemp -d "${TMPDIR:-/tmp}/lagrange-image-archive-self-test.XXXXXX")
  trap 'rm -rf -- "$archive_dir"' RETURN
  chmod 0700 -- "$archive_dir"
  local archive_commit=0123456789abcdef0123456789abcdef01234567

  python3 - "$archive_dir" "$archive_commit" <<'PY'
import gzip
import hashlib
import io
import json
import struct
import sys
import tarfile
from pathlib import Path

root = Path(sys.argv[1])
commit = sys.argv[2]

def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode("utf-8")

def digest(data):
    return hashlib.sha256(data).hexdigest()

def add_layer_entry(handle, name, kind, payload=b"", mode=0o644, linkname=""):
    info = tarfile.TarInfo(name)
    info.mode = mode
    info.uid = 0
    info.gid = 0
    info.mtime = 0
    if kind == "dir":
        info.type = tarfile.DIRTYPE
        info.size = 0
    elif kind == "symlink":
        info.type = tarfile.SYMTYPE
        info.linkname = linkname
        info.size = 0
    else:
        info.type = tarfile.REGTYPE
        info.size = len(payload)
    handle.addfile(info, io.BytesIO(payload) if kind == "file" else None)

def layer(entries):
    raw = io.BytesIO()
    with tarfile.open(fileobj=raw, mode="w", format=tarfile.USTAR_FORMAT) as handle:
        for entry in entries:
            add_layer_entry(handle, *entry)
    return gzip.compress(raw.getvalue(), mtime=0)

elf = bytearray(64)
elf[:4] = b"\x7fELF"
elf[4] = 2  # ELF64
elf[5] = 1  # little endian
elf[6] = 1  # current ELF version
struct.pack_into("<H", elf, 18, 62)  # EM_X86_64
binary = bytes(elf) + b" LAGRANGE-COMMIT=" + commit.encode("ascii") + b"\n"
binary_sha = digest(binary)

def make_archive(name, root_kind, negative=None, single_layer=False):
    # The selected binary deliberately lives only in the lower layer for the
    # normal positive archive.  Consequently a parser that ignores a later
    # whiteout/opaque marker would incorrectly accept the corresponding
    # negative archive by finding this exact valid lower-layer file.
    base_entries = [
        ("usr", "dir"),
        ("usr/local", "dir"),
        ("usr/local/bin", "dir"),
        ("usr/local/bin/test-bin", "file", binary, 0o755),
        ("usr/local/bin/obsolete", "file", b"obsolete"),
    ]
    base = layer(base_entries)
    final_entries = [
        ("usr", "dir"),
        ("usr/local", "dir"),
        ("usr/local/bin", "dir"),
        ("usr/local/bin/.wh.obsolete", "file", b""),
    ]
    if negative == "whiteout":
        final_entries = [("usr", "dir"), ("usr/local", "dir"), ("usr/local/bin", "dir"),
                         ("usr/local/bin/.wh.test-bin", "file", b"")]
    elif negative == "opaque":
        final_entries = [("usr", "dir"), ("usr/local", "dir"), ("usr/local/bin", "dir"),
                         ("usr/local/bin/.wh..wh..opq", "file", b"")]
    elif negative == "ancestor":
        final_entries = [("usr", "dir"), ("usr/local", "dir"),
                         ("usr/local/bin", "file", b"not-a-directory")]
    elif negative == "symlink":
        final_entries = [("usr", "dir"), ("usr/local", "dir"), ("usr/local/bin", "dir"),
                         ("usr/local/bin/test-bin", "symlink", b"", 0o777, "../../etc/passwd")]
    elif negative == "traversal":
        final_entries.append(("../../escape", "file", b"escape"))
    elif negative == "same-layer-whiteout-marker-first":
        final_entries = [
            ("usr", "dir"), ("usr/local", "dir"), ("usr/local/bin", "dir"),
            ("usr/local/bin/.wh.test-bin", "file", b""),
            ("usr/local/bin/test-bin", "file", binary, 0o755),
        ]
    elif negative == "same-layer-whiteout-addition-first":
        final_entries = [
            ("usr", "dir"), ("usr/local", "dir"), ("usr/local/bin", "dir"),
            ("usr/local/bin/test-bin", "file", binary, 0o755),
            ("usr/local/bin/.wh.test-bin", "file", b""),
        ]
    elif negative == "same-layer-opaque-marker-first":
        final_entries = [
            ("usr", "dir"), ("usr/local", "dir"), ("usr/local/bin", "dir"),
            ("usr/local/bin/.wh..wh..opq", "file", b""),
            ("usr/local/bin/test-bin", "file", binary, 0o755),
        ]
    elif negative == "same-layer-opaque-addition-first":
        final_entries = [
            ("usr", "dir"), ("usr/local", "dir"), ("usr/local/bin", "dir"),
            ("usr/local/bin/test-bin", "file", binary, 0o755),
            ("usr/local/bin/.wh..wh..opq", "file", b""),
        ]
    if not any(entry[0] == "usr/local/bin/test-bin" and entry[1] == "file" and entry[2] == binary
               for entry in base_entries):
        raise AssertionError("fixture-selected-lower-binary-missing")
    if negative is None and any(entry[0] == "usr/local/bin/test-bin" for entry in final_entries):
        raise AssertionError("fixture-positive-must-resolve-selected-binary-from-lower-layer")
    if negative in ("whiteout", "opaque", "ancestor") and any(entry[0] == "usr/local/bin/test-bin"
                                                        for entry in final_entries):
        raise AssertionError("fixture-removal-negative-readds-selected-binary")
    if negative and negative.startswith("same-layer-"):
        if not any(entry[0] == "usr/local/bin/test-bin" and entry[1] == "file" and entry[2] == binary
                   for entry in final_entries):
            raise AssertionError("fixture-same-layer-addition-missing")
    required_removal = {
        "whiteout": "usr/local/bin/.wh.test-bin",
        "opaque": "usr/local/bin/.wh..wh..opq",
        "ancestor": "usr/local/bin",
    }.get(negative)
    if required_removal is not None and not any(entry[0] == required_removal for entry in final_entries):
        raise AssertionError("fixture-removal-marker-missing")
    final = layer(final_entries)
    config = canonical({"architecture": "amd64", "config": {"Labels": {
        "org.opencontainers.image.revision": commit}}, "os": "linux", "rootfs": {"type": "layers"}})
    config_digest = digest(config)
    blobs = {config_digest: config}
    layer_digests = []
    for payload in ((base,) if single_layer else (base, final)):
        key = digest(payload)
        layer_digests.append(key)
        blobs[key] = payload
    manifest = canonical({"config": {"digest": "sha256:" + config_digest,
                                      "mediaType": "application/vnd.oci.image.config.v1+json",
                                      "size": len(config)},
                          "layers": [{"digest": "sha256:" + item,
                                      "mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
                                      "size": len(blobs[item])} for item in layer_digests],
                          "mediaType": "application/vnd.oci.image.manifest.v1+json", "schemaVersion": 2})
    manifest_digest = digest(manifest)
    blobs[manifest_digest] = manifest
    runnable = {"digest": "sha256:" + manifest_digest,
                "mediaType": "application/vnd.oci.image.manifest.v1+json", "size": len(manifest),
                "platform": {"architecture": "amd64", "os": "linux"}}
    if root_kind == "manifest":
        root_blob = manifest
        root_digest = manifest_digest
        outer_descriptor = {key: runnable[key] for key in ("digest", "mediaType", "size")}
        outer_index = canonical({"manifests": [outer_descriptor],
                                 "mediaType": "application/vnd.oci.image.index.v1+json", "schemaVersion": 2})
    elif root_kind == "index":
        attestation = canonical({"config": {"digest": "sha256:" + config_digest,
                                               "mediaType": "application/vnd.oci.image.config.v1+json",
                                               "size": len(config)}, "layers": [],
                                  "mediaType": "application/vnd.oci.image.manifest.v1+json", "schemaVersion": 2})
        attestation_digest = digest(attestation)
        blobs[attestation_digest] = attestation
        provenance = canonical({"manifests": [runnable, {
            "annotations": {"vnd.docker.reference.digest": "sha256:" + manifest_digest,
                            "vnd.docker.reference.type": "attestation-manifest"},
            "digest": "sha256:" + attestation_digest,
            "mediaType": "application/vnd.oci.image.manifest.v1+json", "size": len(attestation),
            "platform": {"architecture": "unknown", "os": "unknown"}}],
            "mediaType": "application/vnd.oci.image.index.v1+json", "schemaVersion": 2})
        root_blob = provenance
        root_digest = digest(provenance)
        blobs[root_digest] = provenance
        outer_descriptor = {"digest": "sha256:" + root_digest,
                            "mediaType": "application/vnd.oci.image.index.v1+json", "size": len(provenance)}
        outer_index = canonical({"manifests": [outer_descriptor],
                                 "mediaType": "application/vnd.oci.image.index.v1+json", "schemaVersion": 2})
    elif root_kind == "config":
        # Docker's legacy config-ID fallback names the config under its own
        # digest in manifest.json; it has no OCI index root.
        root_blob = config
        root_digest = config_digest
        outer_index = None
    else:
        raise AssertionError("fixture-root-kind-invalid")
    if negative == "tampered":
        blobs[layer_digests[-1]] = blobs[layer_digests[-1]] + b"tampered-after-descriptor"
    if negative == "legacy-grafted-layer":
        # Retain the selected binary but substitute a different, independently
        # hash-valid layer under the identical config ID. A config-ID-only
        # fallback cannot bind this change through config.rootfs.diff_ids.
        if root_kind != "config" or not single_layer:
            raise AssertionError("fixture-legacy-graft-shape-invalid")
        original_layer = layer_digests[0]
        grafted = layer(base_entries + [("usr/local/bin/grafted", "file", b"grafted-layer")])
        grafted_digest = digest(grafted)
        if grafted_digest == original_layer:
            raise AssertionError("fixture-legacy-graft-not-effective")
        layer_digests[0] = grafted_digest
        blobs[grafted_digest] = grafted
    if negative == "root-content-tampered":
        # Keep the original digest *filename* and image ID. This controls the
        # actual root bytes, unlike a missing/wrong filename test.
        original = blobs[root_digest]
        if digest(original) != root_digest:
            raise AssertionError("fixture-root-digest-not-canonical")
        blobs[root_digest] = original + b" "
        if digest(blobs[root_digest]) == root_digest:
            raise AssertionError("fixture-root-tamper-not-effective")
    docker_manifest = canonical([{"Config": "blobs/sha256/" + config_digest, "RepoTags": None,
                                  "Layers": ["blobs/sha256/" + item for item in layer_digests]}])
    output = root / name
    with tarfile.open(output, mode="w", format=tarfile.USTAR_FORMAT) as archive:
        for directory in ("blobs", "blobs/sha256"):
            info = tarfile.TarInfo(directory)
            info.type = tarfile.DIRTYPE
            info.mode = 0o755
            info.mtime = 0
            archive.addfile(info)
        for blob_digest in sorted(blobs):
            payload = blobs[blob_digest]
            info = tarfile.TarInfo("blobs/sha256/" + blob_digest)
            info.size = len(payload)
            info.mode = 0o644
            info.mtime = 0
            archive.addfile(info, io.BytesIO(payload))
        top_level = [("manifest.json", docker_manifest),
                     ("oci-layout", canonical({"imageLayoutVersion": "1.0.0"}))]
        if outer_index is not None:
            top_level.insert(0, ("index.json", outer_index))
        for filename, payload in top_level:
            info = tarfile.TarInfo(filename)
            info.size = len(payload)
            info.mode = 0o644
            info.mtime = 0
            archive.addfile(info, io.BytesIO(payload))
        if negative == "duplicate-index":
            # The archive-level duplicate is independent of a duplicate JSON
            # key: a streaming reader must not silently take either copy.
            if outer_index is None:
                raise AssertionError("fixture-duplicate-index-without-index")
            info = tarfile.TarInfo("index.json")
            info.size = len(outer_index)
            info.mode = 0o644
            info.mtime = 0
            archive.addfile(info, io.BytesIO(outer_index))
        elif negative == "archive-path":
            info = tarfile.TarInfo("blobs/sha256/../outside")
            info.size = len(b"outside")
            info.mode = 0o644
            info.mtime = 0
            archive.addfile(info, io.BytesIO(b"outside"))
    return {"archive": str(output), "image_id": "sha256:" + root_digest,
            "config_digest": "sha256:" + config_digest,
            "expected_manifest_digest": "sha256:" + manifest_digest}

records = {
    # Single-layer root controls isolate binding of bytes at a correct digest
    # filename from lower-layer merging and run before the overlay regression.
    "root_direct": make_archive("root-direct.tar", "manifest", single_layer=True),
    "root_direct_tampered": make_archive("root-direct-tampered.tar", "manifest", "root-content-tampered", single_layer=True),
    "root_provenance": make_archive("root-provenance.tar", "index", single_layer=True),
    "root_provenance_tampered": make_archive("root-provenance-tampered.tar", "index", "root-content-tampered", single_layer=True),
    # Config-ID-only exports were not observed or validated on this host.
    # They are intentional fail-closed negatives, including a hash-valid
    # layer graft under unchanged config bytes/image ID.
    "legacy_config_only": make_archive("legacy-config-only.tar", "config", single_layer=True),
    "legacy_config_only_tampered": make_archive("legacy-config-only-tampered.tar", "config", "root-content-tampered", single_layer=True),
    "legacy_config_only_grafted": make_archive("legacy-config-only-grafted.tar", "config", "legacy-grafted-layer", single_layer=True),
    # OCI whiteouts hide lower layers, never additions from their own layer.
    # Both member orders must therefore retain the new selected binary.
    "same_layer_whiteout_marker_first": make_archive("same-layer-whiteout-marker-first.tar", "manifest", "same-layer-whiteout-marker-first"),
    "same_layer_whiteout_addition_first": make_archive("same-layer-whiteout-addition-first.tar", "manifest", "same-layer-whiteout-addition-first"),
    "same_layer_opaque_marker_first": make_archive("same-layer-opaque-marker-first.tar", "manifest", "same-layer-opaque-marker-first"),
    "same_layer_opaque_addition_first": make_archive("same-layer-opaque-addition-first.tar", "manifest", "same-layer-opaque-addition-first"),
    "direct": make_archive("direct-manifest.tar", "manifest"),
    "provenance_index": make_archive("provenance-index.tar", "index"),
    "tampered": make_archive("tampered.tar", "manifest", "tampered"),
    "whiteout": make_archive("whiteout.tar", "manifest", "whiteout"),
    "opaque": make_archive("opaque.tar", "manifest", "opaque"),
    "ancestor": make_archive("ancestor.tar", "manifest", "ancestor"),
    "symlink": make_archive("symlink.tar", "manifest", "symlink"),
    "traversal": make_archive("traversal.tar", "manifest", "traversal"),
    "duplicate_index": make_archive("duplicate-index.tar", "manifest", "duplicate-index"),
    "archive_path": make_archive("archive-path.tar", "manifest", "archive-path"),
}
records["binary_sha256"] = binary_sha
(root / "archive-map.json").write_text(json.dumps(records, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
PY

  expected_map=$archive_dir/archive-map.json
  request=$archive_dir/request.json
  python3 - "$expected_map" "$archive_commit" >"$request" <<'PY'
import json, sys
record = json.load(open(sys.argv[1], encoding="utf-8"))
commit = sys.argv[2]
print(json.dumps({"format": "lagrange-image-files-v1", "files": [{
    "contains_hex": [commit.encode("ascii").hex()], "elf": True, "executable": True,
    "path": "usr/local/bin/test-bin", "sha256": record["binary_sha256"]}],
    "nonempty_directories": ["usr/local/bin"]}, sort_keys=True, separators=(",", ":")))
PY

  archive_value() {
    python3 - "$expected_map" "$1" "$2" <<'PY'
import json, sys
value = json.load(open(sys.argv[1], encoding="utf-8"))[sys.argv[2]][sys.argv[3]]
print(value)
PY
  }
  assert_archive_result() {
    local output=$1 expected_id=$2 expected_manifest=$3 expected_config=$4
    python3 - "$output" "$expected_id" "$expected_manifest" "$expected_config" "$archive_commit" <<'PY'
import json, re, sys
path, image_id, manifest_digest, config_digest, commit = sys.argv[1:]
value = json.load(open(path, encoding="utf-8"))
required = {"format", "image_id", "manifest_digest", "config_digest", "platform", "source_commit",
            "archive_sha256", "request_sha256", "files", "nonempty_directories"}
if set(value) != required or value.get("format") != "lagrange-image-files-result-v1": raise SystemExit("result-keys")
if value["image_id"] != image_id or value["platform"] != "linux/amd64" or value["source_commit"] != commit: raise SystemExit("result-binding")
if value["manifest_digest"] != manifest_digest or value["config_digest"] != config_digest: raise SystemExit("result-root-digests")
if not all(isinstance(value.get(key), str) and re.fullmatch(r"[0-9a-f]{64}", value[key]) for key in ("archive_sha256", "request_sha256")): raise SystemExit("result-hashes")
entry = value["files"].get("usr/local/bin/test-bin")
if not isinstance(entry, dict) or set(entry) != {"sha256", "mode", "size", "elf"}: raise SystemExit("result-file-keys")
if entry["mode"] != "0755" or entry["elf"] is not True or not isinstance(entry["size"], int) or entry["size"] <= 64: raise SystemExit("result-file")
if value["nonempty_directories"] != ["usr/local/bin"]: raise SystemExit("result-directories")
PY
  }
  assert_record_result() {
    local record=$1 output=$2
    if ! assert_archive_result "$output" "$(archive_value "$record" image_id)" \
      "$(archive_value "$record" expected_manifest_digest)" "$(archive_value "$record" config_digest)"; then
      echo "self-test: archive result invariant failed: $record" >&2
      return 1
    fi
  }
  expect_archive_reject() {
    local label=$1 archive=$2 image_id=$3 result_path=$4 expected_commit=${5:-$archive_commit}
    if release_build_layout_archive_scan "$archive" "$image_id" linux/amd64 "$expected_commit" "$request" "$result_path" \
      >"$archive_dir/$label.out" 2>"$archive_dir/$label.err"; then
      echo "self-test: archive parser accepted invalid case: $label" >&2
      return 1
    fi
    [ ! -e "$result_path" ] && [ ! -L "$result_path" ] || {
      echo "self-test: archive parser published a result after rejection: $label" >&2
      return 1
    }
  }
  expect_request_reject() {
    local label=$1 bad_request=$2 result_path=$3
    if release_build_layout_archive_scan "$direct_archive" "$direct_id" linux/amd64 "$archive_commit" "$bad_request" "$result_path" \
      >"$archive_dir/$label.out" 2>"$archive_dir/$label.err"; then
      echo "self-test: archive parser accepted invalid request: $label" >&2
      return 1
    fi
    [ ! -e "$result_path" ] && [ ! -L "$result_path" ] || {
      echo "self-test: archive parser published a result after invalid request: $label" >&2
      return 1
    }
  }
  expect_host_path_reject() {
    local label=$1 archive_arg=$2 request_arg=$3 result_arg=$4 canonical_result=$5
    if (
      cd "$archive_dir"
      release_build_layout_archive_scan "$archive_arg" "$direct_id" linux/amd64 "$archive_commit" \
        "$request_arg" "$result_arg"
    ) >"$archive_dir/$label.out" 2>"$archive_dir/$label.err"; then
      echo "self-test: archive parser accepted noncanonical host path: $label" >&2
      return 1
    fi
    grep -Fxq 'host-path-not-canonical' "$archive_dir/$label.err" || {
      echo "self-test: archive parser rejected noncanonical host path for the wrong reason: $label" >&2
      return 1
    }
    [ ! -e "$canonical_result" ] && [ ! -L "$canonical_result" ] || {
      echo "self-test: archive parser published a result after noncanonical host path rejection: $label" >&2
      return 1
    }
  }

  # These root controls are deliberately one-layer archives. The rejection
  # cases append whitespace to root bytes while retaining the old digest
  # filename and image ID; a filename-only check cannot make them reject.
  for root_record in root_direct root_provenance; do
    root_archive=$(archive_value "$root_record" archive)
    root_id=$(archive_value "$root_record" image_id)
    result=$archive_dir/$root_record-result.json
    release_build_layout_archive_scan "$root_archive" "$root_id" linux/amd64 "$archive_commit" "$request" "$result"
    assert_record_result "$root_record" "$result"
  done
  for root_record in root_direct_tampered root_provenance_tampered \
    legacy_config_only legacy_config_only_tampered legacy_config_only_grafted; do
    expect_archive_reject "$root_record" "$(archive_value "$root_record" archive)" \
      "$(archive_value "$root_record" image_id)" "$archive_dir/$root_record-result.json"
  done

  # Whiteouts and opaque markers remove lower-layer entries only. A selected
  # file added in that same layer must survive whether it was emitted before
  # or after the marker in the tar stream.
  for root_record in same_layer_whiteout_marker_first same_layer_whiteout_addition_first \
    same_layer_opaque_marker_first same_layer_opaque_addition_first; do
    root_archive=$(archive_value "$root_record" archive)
    root_id=$(archive_value "$root_record" image_id)
    result=$archive_dir/$root_record-result.json
    release_build_layout_archive_scan "$root_archive" "$root_id" linux/amd64 "$archive_commit" "$request" "$result"
    assert_record_result "$root_record" "$result"
  done

  # This remains the valid lower-layer directory-overlay control: the
  # required directory contains the selected regular file from the base layer.
  direct_archive=$(archive_value direct archive)
  direct_id=$(archive_value direct image_id)
  result=$archive_dir/direct-result.json
  release_build_layout_archive_scan "$direct_archive" "$direct_id" linux/amd64 "$archive_commit" "$request" "$result"
  assert_record_result direct "$result"

  index_archive=$(archive_value provenance_index archive)
  index_id=$(archive_value provenance_index image_id)
  result=$archive_dir/index-result.json
  release_build_layout_archive_scan "$index_archive" "$index_id" linux/amd64 "$archive_commit" "$request" "$result"
  assert_record_result provenance_index "$result"

  # The public helper requires caller text itself to be canonical before it
  # reads either input or creates a result. Exercise every path position with
  # dot/dotdot aliases, relative text, redundant separators, and a symlinked
  # ancestor. A normalization-before-validation regression would publish each
  # otherwise valid direct-manifest result below.
  mkdir "$archive_dir/path-parent"
  ln -s "$archive_dir" "$archive_dir/symlink-ancestor"
  for path_kind in dot dotdot relative redundant-separator symlink-ancestor; do
    case "$path_kind" in
      dot) archive_arg=$archive_dir/./direct-manifest.tar ;;
      dotdot) archive_arg=$archive_dir/path-parent/../direct-manifest.tar ;;
      relative) archive_arg=direct-manifest.tar ;;
      redundant-separator) archive_arg=$archive_dir//direct-manifest.tar ;;
      symlink-ancestor) archive_arg=$archive_dir/symlink-ancestor/direct-manifest.tar ;;
    esac
    canonical_result=$archive_dir/noncanonical-archive-$path_kind-result.json
    expect_host_path_reject "noncanonical-archive-$path_kind" "$archive_arg" "$request" \
      "$canonical_result" "$canonical_result"
  done
  for path_kind in dot dotdot relative redundant-separator symlink-ancestor; do
    case "$path_kind" in
      dot) request_arg=$archive_dir/./request.json ;;
      dotdot) request_arg=$archive_dir/path-parent/../request.json ;;
      relative) request_arg=request.json ;;
      redundant-separator) request_arg=$archive_dir//request.json ;;
      symlink-ancestor) request_arg=$archive_dir/symlink-ancestor/request.json ;;
    esac
    canonical_result=$archive_dir/noncanonical-request-$path_kind-result.json
    expect_host_path_reject "noncanonical-request-$path_kind" "$direct_archive" "$request_arg" \
      "$canonical_result" "$canonical_result"
  done
  for path_kind in dot dotdot relative redundant-separator symlink-ancestor; do
    canonical_result=$archive_dir/noncanonical-result-$path_kind.json
    case "$path_kind" in
      dot) result_arg=$archive_dir/./noncanonical-result-$path_kind.json ;;
      dotdot) result_arg=$archive_dir/path-parent/../noncanonical-result-$path_kind.json ;;
      relative) result_arg=noncanonical-result-$path_kind.json ;;
      redundant-separator) result_arg=$archive_dir//noncanonical-result-$path_kind.json ;;
      symlink-ancestor) result_arg=$archive_dir/symlink-ancestor/noncanonical-result-$path_kind.json ;;
    esac
    expect_host_path_reject "noncanonical-result-$path_kind" "$direct_archive" "$request" \
      "$result_arg" "$canonical_result"
  done

  expect_archive_reject tampered "$(archive_value tampered archive)" "$(archive_value tampered image_id)" "$archive_dir/tampered-result.json"
  expect_archive_reject whiteout "$(archive_value whiteout archive)" "$(archive_value whiteout image_id)" "$archive_dir/whiteout-result.json"
  expect_archive_reject opaque "$(archive_value opaque archive)" "$(archive_value opaque image_id)" "$archive_dir/opaque-result.json"
  expect_archive_reject ancestor "$(archive_value ancestor archive)" "$(archive_value ancestor image_id)" "$archive_dir/ancestor-result.json"
  expect_archive_reject symlink "$(archive_value symlink archive)" "$(archive_value symlink image_id)" "$archive_dir/symlink-result.json"
  expect_archive_reject traversal "$(archive_value traversal archive)" "$(archive_value traversal image_id)" "$archive_dir/traversal-result.json"
  expect_archive_reject duplicate-index "$(archive_value duplicate_index archive)" "$(archive_value duplicate_index image_id)" "$archive_dir/duplicate-index-result.json"
  expect_archive_reject archive-path "$(archive_value archive_path archive)" "$(archive_value archive_path image_id)" "$archive_dir/archive-path-result.json"
  expect_archive_reject wrong-image-id "$direct_archive" sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa "$archive_dir/wrong-image-result.json"
  expect_archive_reject wrong-commit "$direct_archive" "$direct_id" "$archive_dir/wrong-commit-result.json" \
    fedcba9876543210fedcba9876543210fedcba98

  printf 'existing-result\n' >"$archive_dir/no-clobber-result.json"
  if release_build_layout_archive_scan "$direct_archive" "$direct_id" linux/amd64 "$archive_commit" "$request" "$archive_dir/no-clobber-result.json"; then
    echo 'self-test: archive parser overwrote an existing result' >&2
    return 1
  fi
  grep -Fxq existing-result "$archive_dir/no-clobber-result.json"

  printf '%s\n' '{"format":"lagrange-image-files-v1","format":"lagrange-image-files-v1","files":[],"nonempty_directories":[]}' >"$archive_dir/duplicate-request.json"
  expect_request_reject duplicate-request "$archive_dir/duplicate-request.json" "$archive_dir/duplicate-result.json"
  python3 - "$archive_dir/unknown-field-request.json" "$archive_dir/file-ancestor-request.json" <<'PY'
import json
import sys
unknown, ancestor = sys.argv[1:]
base = {
    "format": "lagrange-image-files-v1",
    "files": [{
        "path": "usr/local/bin/test-bin", "sha256": None,
        "executable": False, "elf": False, "contains_hex": []
    }],
    "nonempty_directories": ["usr/local/bin"],
}
with open(unknown, "w", encoding="utf-8", newline="\n") as handle:
    value = json.loads(json.dumps(base))
    value["files"][0]["unexpected"] = True
    handle.write(json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n")
with open(ancestor, "w", encoding="utf-8", newline="\n") as handle:
    value = json.loads(json.dumps(base))
    value["files"].append({
        "path": "usr/local/bin", "sha256": None,
        "executable": False, "elf": False, "contains_hex": []
    })
    value["nonempty_directories"] = []
    handle.write(json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n")
PY
  expect_request_reject unknown-request-field "$archive_dir/unknown-field-request.json" "$archive_dir/unknown-field-result.json"
  expect_request_reject file-ancestor-conflict "$archive_dir/file-ancestor-request.json" "$archive_dir/file-ancestor-result.json"
  echo 'PRODUCTION_IMAGE_ARCHIVE_PARSER_SELF_TEST: PASS (synthetic OCI archives only)'
)

# Keep the archive API regression independent from the larger fake-build
# fixture. A parser failure must remain visible rather than being masked by a
# later fixture setup failure.
run_archive_parser_tests

# Exercise the host runtime selector independently of Docker and producer state.
# A real tiny Git index supplies Python/config/data payloads while ignored
# virtualenv/cache entries remain on disk. Selection, copying and source-side
# expectations must use only tracked regular entries; actual copied trees stay
# strict and reject every missing/extra/link/special mutation.
run_runtime_payload_inventory_tests() (
  set -euo pipefail
  source "$source_layout_helper"
  local test_root fixture inventory inventory_hash selected copy_root verified control_hash ignored_before ignored_after
  local case_root changed_hash changed_tree original_tree group
  test_root=$(mktemp -d "${TMPDIR:-/tmp}/lagrange-runtime-payload-self-test.XXXXXX")
  trap 'rm -rf -- "$test_root"' RETURN
  chmod 0700 -- "$test_root"
  fixture=$test_root/source
  mkdir -p -- "$fixture/nt/config" "$fixture/nt/data" \
    "$fixture/data-pipelines/collectors/runtime" "$fixture/migrations" \
    "$fixture/configs/strategies" "$fixture/deploy/db" "$fixture/deploy/runtime"
  cp -p -- "$source_root/.dockerignore" "$fixture/.dockerignore"
  printf '%s\n' '**/.venv/' '**/__pycache__/' '**/*.py[cod]' >"$fixture/.gitignore"
  printf '%s\n' '# tracked worker' >"$fixture/nt/worker.py"
  printf '%s\n' '{"setting":true}' >"$fixture/nt/config/settings.json"
  # A sibling sharing a directory's prefix distinguishes path-string sorting
  # from the depth-first order used by the existing in-image payload hash.
  printf '%s\n' 'tracked-prefix-sibling' >"$fixture/nt/config-old"
  printf '%s\n' 'tracked-data' >"$fixture/nt/data/value.dat"
  printf '%s\n' '# collector runtime' >"$fixture/data-pipelines/collectors/runtime/collector.py"
  printf '%s\n' '-- migration' >"$fixture/migrations/001.sql"
  printf '%s\n' '{"strategy":"fixture"}' >"$fixture/configs/strategies/baseline.json"
  printf '%s\n' '#!/bin/sh' 'exit 0' >"$fixture/deploy/db/run.sh"
  printf '%s\n' '#!/bin/sh' 'exit 0' >"$fixture/deploy/runtime/paper"
  chmod 0755 -- "$fixture/deploy/db/run.sh" "$fixture/deploy/runtime/paper"
  cat >"$fixture/layout.json" <<'JSON'
{"runtime_payloads":{"backtest":[{"image":"/opt/lagrange/nt","source":"nt"}],"collectors":[{"image":"/opt/lagrange/collectors","source":"data-pipelines/collectors/runtime"}],"database":[{"image":"/opt/lagrange/migrations","source":"migrations"},{"image":"/opt/lagrange/configs/baseline.json","source":"configs/strategies/baseline.json"},{"image":"/usr/local/bin/db-run","source":"deploy/db/run.sh"}],"paper":[{"image":"/usr/local/bin/paper","source":"deploy/runtime/paper"}]}}
JSON
  git -C "$fixture" init -q
  git -C "$fixture" config user.email fixture@example.invalid
  git -C "$fixture" config user.name fixture
  git -C "$fixture" add -- .
  git -C "$fixture" commit -qm fixture
  mkdir -p -- "$fixture/nt/.venv/bin" "$fixture/nt/.venv/lib/__pycache__" "$fixture/nt/__pycache__"
  ln -s ../python-fixture-target "$fixture/nt/.venv/bin/python"
  printf '%s\n' 'ignored venv bytes' >"$fixture/nt/.venv/lib/__pycache__/ignored.pyc"
  printf '%s\n' 'ignored cache bytes' >"$fixture/nt/__pycache__/ignored.pyc"
  [ -z "$(git -C "$fixture" status --porcelain=v1 --untracked-files=all)" ] || {
    echo 'self-test: ignored runtime fixture was not Git-clean' >&2
    return 1
  }
  ignored_before=$(python3 - "$fixture" <<'PY'
import hashlib, json, os, stat, sys
root=sys.argv[1]; link=os.path.join(root,"nt/.venv/bin/python")
cache=os.path.join(root,"nt/__pycache__/ignored.pyc")
info=os.lstat(link)
print(json.dumps({"link":os.readlink(link),"link_mode":stat.S_IMODE(info.st_mode),
                  "cache":hashlib.sha256(open(cache,"rb").read()).hexdigest()},sort_keys=True))
PY
  )
  inventory=$(rbl_runtime_payload_inventory "$fixture" "$fixture/layout.json")
  inventory_hash=$(printf '%s\n' "$inventory" | sha256sum | awk '{print $1}')
  [[ "$inventory_hash" =~ ^[0-9a-f]{64}$ ]]
  RBL_TEST_RUNTIME_INVENTORY=$inventory python3 - <<'PY'
import json, os
value=json.loads(os.environ["RBL_TEST_RUNTIME_INVENTORY"])
assert value["format"]=="lagrange-runtime-payload-inventory-v1"
assert set(value["payload_groups"])=={"backtest","collectors","database","paper"}
paths=[entry["path"] for entry in value["payload_groups"]["backtest"][0]["entries"]]
assert {"nt","nt/worker.py","nt/config","nt/config/settings.json","nt/config-old","nt/data","nt/data/value.dat"}==set(paths)
assert not any(".venv" in path or "__pycache__" in path or path.endswith((".pyc",".pyo",".pyd")) for path in paths)
PY
  for group in backtest collectors database paper; do
    copy_root=$test_root/copy-$group
    mkdir -m 0700 -- "$copy_root"
    selected=$(rbl_runtime_payload_copy "$fixture" "$copy_root" "$group" "$inventory")
    verified=$(rbl_runtime_payload_verify_copy "$copy_root" "$group" "$inventory")
    [ "$selected" = "$verified" ] || {
      echo "self-test: selected/copied runtime payload binding differed: $group" >&2
      return 1
    }
  done
  copy_root=$test_root/copy-backtest
  control_hash=$(rbl_entry_hash "$copy_root" nt)
  RBL_TEST_RUNTIME_INVENTORY=$inventory RBL_TEST_CONTROL_HASH=$control_hash python3 - <<'PY'
import json, os
payload=json.loads(os.environ["RBL_TEST_RUNTIME_INVENTORY"])["payload_groups"]["backtest"][0]
assert payload["tree_sha256"]==os.environ["RBL_TEST_CONTROL_HASH"]
PY
  ignored_after=$(python3 - "$fixture" <<'PY'
import hashlib, json, os, stat, sys
root=sys.argv[1]; link=os.path.join(root,"nt/.venv/bin/python")
cache=os.path.join(root,"nt/__pycache__/ignored.pyc")
info=os.lstat(link)
print(json.dumps({"link":os.readlink(link),"link_mode":stat.S_IMODE(info.st_mode),
                  "cache":hashlib.sha256(open(cache,"rb").read()).hexdigest()},sort_keys=True))
PY
  )
  [ "$ignored_before" = "$ignored_after" ] || {
    echo 'self-test: runtime inventory/copy read path modified ignored tooling' >&2
    return 1
  }

  runtime_tree_hash() {
    RBL_TEST_RUNTIME_INVENTORY=$1 python3 - <<'PY'
import json,os
print(json.loads(os.environ["RBL_TEST_RUNTIME_INVENTORY"])["payload_groups"]["backtest"][0]["tree_sha256"])
PY
  }
  original_tree=$(runtime_tree_hash "$inventory")
  case_root=$test_root/ignored-change
  git clone -q --no-local "$fixture" "$case_root"
  mkdir -p -- "$case_root/nt/.venv/bin" "$case_root/nt/__pycache__"
  ln -s ../first-target "$case_root/nt/.venv/bin/python"
  printf '%s\n' before >"$case_root/nt/__pycache__/ignored.pyc"
  changed_hash=$(rbl_runtime_payload_inventory "$case_root" "$case_root/layout.json")
  printf '%s\n' after >"$case_root/nt/__pycache__/ignored.pyc"
  ln -sfn ../second-target "$case_root/nt/.venv/bin/python"
  [ "$(runtime_tree_hash "$changed_hash")" = "$(runtime_tree_hash "$(rbl_runtime_payload_inventory "$case_root" "$case_root/layout.json")")" ] || {
    echo 'self-test: ignored runtime tooling changed the selected payload hash' >&2
    return 1
  }

  case_root=$test_root/byte-change
  git clone -q --no-local "$fixture" "$case_root"
  printf '%s\n' '# changed tracked worker' >"$case_root/nt/worker.py"
  changed_tree=$(runtime_tree_hash "$(rbl_runtime_payload_inventory "$case_root" "$case_root/layout.json")")
  [ "$changed_tree" != "$original_tree" ] || {
    echo 'self-test: tracked runtime byte edit did not change its tree hash' >&2
    return 1
  }
  case_root=$test_root/mode-change
  git clone -q --no-local "$fixture" "$case_root"
  chmod 0600 -- "$case_root/nt/worker.py"
  changed_tree=$(runtime_tree_hash "$(rbl_runtime_payload_inventory "$case_root" "$case_root/layout.json")")
  [ "$changed_tree" != "$original_tree" ] || {
    echo 'self-test: tracked runtime mode edit did not change its tree hash' >&2
    return 1
  }

  expect_runtime_inventory_failure() {
    local label=$1 expected=$2 root=$3 layout=$4
    if rbl_runtime_payload_inventory "$root" "$layout" \
      >"$test_root/$label.out" 2>"$test_root/$label.err"; then
      echo "self-test: invalid runtime inventory unexpectedly passed: $label" >&2
      return 1
    fi
    grep -Fxq "$expected" "$test_root/$label.err" || {
      echo "self-test: invalid runtime inventory failed for the wrong reason: $label" >&2
      return 1
    }
  }
  case_root=$test_root/tracked-symlink
  git clone -q --no-local "$fixture" "$case_root"
  ln -s worker.py "$case_root/nt/tracked-link"
  git -C "$case_root" add -- nt/tracked-link
  expect_runtime_inventory_failure tracked-symlink runtime-payload-tracked-type-invalid "$case_root" "$case_root/layout.json"

  case_root=$test_root/ancestor-symlink
  git clone -q --no-local "$fixture" "$case_root"
  mv -- "$case_root/nt/config" "$case_root/nt/config-real"
  ln -s config-real "$case_root/nt/config"
  expect_runtime_inventory_failure ancestor-symlink runtime-payload-ancestor-invalid "$case_root" "$case_root/layout.json"

  case_root=$test_root/special
  git clone -q --no-local "$fixture" "$case_root"
  unlink -- "$case_root/nt/data/value.dat"
  mkfifo -- "$case_root/nt/data/value.dat"
  expect_runtime_inventory_failure special runtime-payload-entry-invalid "$case_root" "$case_root/layout.json"

  case_root=$test_root/missing
  git clone -q --no-local "$fixture" "$case_root"
  unlink -- "$case_root/nt/data/value.dat"
  expect_runtime_inventory_failure missing runtime-payload-entry-missing "$case_root" "$case_root/layout.json"

  case_root=$test_root/excluded
  git clone -q --no-local "$fixture" "$case_root"
  mkdir -p -- "$case_root/nt/__pycache__"
  printf '%s\n' tracked-excluded >"$case_root/nt/__pycache__/tracked.pyc"
  git -C "$case_root" add -f -- nt/__pycache__/tracked.pyc
  expect_runtime_inventory_failure excluded runtime-payload-docker-excluded "$case_root" "$case_root/layout.json"

  case_root=$test_root/noncanonical
  git clone -q --no-local "$fixture" "$case_root"
  python3 - "$case_root/layout.json" <<'PY'
import json,sys
path=sys.argv[1]; value=json.load(open(path,encoding="utf-8"))
value["runtime_payloads"]["backtest"][0]["source"]="nt/./worker.py"
open(path,"w",encoding="utf-8").write(json.dumps(value,separators=(",",":"))+"\n")
PY
  expect_runtime_inventory_failure noncanonical runtime-payload-path-invalid "$case_root" "$case_root/layout.json"

  case_root=$test_root/dockerignore-drift
  git clone -q --no-local "$fixture" "$case_root"
  printf '%s\n' '# drift' >>"$case_root/.dockerignore"
  expect_runtime_inventory_failure dockerignore-drift runtime-payload-dockerignore-policy-drift "$case_root" "$case_root/layout.json"

  copy_root=$test_root/copy-extra
  cp -a -- "$test_root/copy-backtest" "$copy_root"
  printf '%s\n' unexpected >"$copy_root/nt/unexpected.txt"
  if rbl_runtime_payload_verify_copy "$copy_root" backtest "$inventory" \
    >"$test_root/copy-extra.out" 2>"$test_root/copy-extra.err"; then
    echo 'self-test: unexpected runtime bundle file passed verification' >&2
    return 1
  fi
  copy_root=$test_root/copy-symlink
  cp -a -- "$test_root/copy-backtest" "$copy_root"
  ln -s worker.py "$copy_root/nt/unexpected-link"
  if rbl_runtime_payload_verify_copy "$copy_root" backtest "$inventory" \
    >"$test_root/copy-symlink.out" 2>"$test_root/copy-symlink.err"; then
    echo 'self-test: unexpected runtime bundle symlink passed verification' >&2
    return 1
  fi
  copy_root=$test_root/copy-missing
  cp -a -- "$test_root/copy-backtest" "$copy_root"
  unlink -- "$copy_root/nt/data/value.dat"
  if rbl_runtime_payload_verify_copy "$copy_root" backtest "$inventory" \
    >"$test_root/copy-missing.out" 2>"$test_root/copy-missing.err"; then
    echo 'self-test: missing runtime bundle file passed verification' >&2
    return 1
  fi
  echo 'PRODUCTION_RUNTIME_PAYLOAD_INVENTORY_SELF_TEST: PASS (tiny tracked Git fixture only)'
)

run_runtime_payload_inventory_tests

# The build helper's --apply contract is root-only. Establish one root-like
# fixture process once; commands inside that child run directly, never through
# a nested fakeroot wrapper. A missing runner is a test-environment failure,
# not a passing skipped test.
if [ "$(id -u)" -ne 0 ]; then
  if [ "${LAGRANGE_IMAGE_BUILD_ROOT_FIXTURE_CHILD:-0}" = 1 ]; then
    echo 'TEST_ENVIRONMENT_ERROR: image-build root fixture did not obtain root identity' >&2
    exit 1
  fi
  if unshare -Ur true >/dev/null 2>&1; then
    exec unshare -Ur env LAGRANGE_IMAGE_BUILD_ROOT_FIXTURE_CHILD=1 \
      bash "$script_dir/build-production-images-self-test.sh" "$@"
  fi
  if command -v fakeroot >/dev/null 2>&1; then
    exec fakeroot env LAGRANGE_IMAGE_BUILD_ROOT_FIXTURE_CHILD=1 \
      bash "$script_dir/build-production-images-self-test.sh" "$@"
  fi
  echo 'TEST_ENVIRONMENT_ERROR: image-build root fixture requires user namespaces or fakeroot' >&2
  exit 1
fi

out_dir=$(mktemp -d "${TMPDIR:-/tmp}/lagrange-image-build-self-test.XXXXXX")
if [ "${IMAGE_BUILD_SELFTEST_KEEP_PRIVATE_EVIDENCE:-0}" = 1 ]; then
  trap ':' EXIT
else
  trap 'rm -rf -- "$out_dir"' EXIT
fi
repo_dir=$out_dir/repo
helper=$repo_dir/scripts/ops/build-production-images.sh
manifest_lib=$repo_dir/scripts/ops/lib/release-image-manifest.sh
layout_helper=$repo_dir/scripts/ops/lib/release-build-layout.sh
layout_config=$repo_dir/deploy/build/release-build-layout.json
compose_file=$repo_dir/deploy/compose/compose.yml
env_file=$repo_dir/deploy/compose/.env
fake_bin=$out_dir/fake-bin
docker_log=$out_dir/docker.log
manifest_file=$out_dir/production-images.manifest

mkdir -p "$fake_bin" "$repo_dir" "$(dirname "$compose_file")"
# Build an intentionally small, private clean source tree from the frozen
# layout inventory. It is enough for the real helper to hash/copy/bind its
# inputs, but never reaches Docker or Rust: the fixture Docker command below
# exports deterministic synthetic receipts and OCI archives instead.
python3 - "$source_root" "$repo_dir" <<'PY'
import json
import os
import shutil
import stat
import subprocess
import sys

source, target = map(os.path.abspath, sys.argv[1:])
layout_path = os.path.join(source, "deploy/build/release-build-layout.json")
layout = json.load(open(layout_path, encoding="utf-8"))

def copy_rel(rel):
    if not isinstance(rel, str) or not rel or rel.startswith("/") or ".." in rel.split("/"):
        raise SystemExit("fixture-copy-path-invalid")
    src = os.path.join(source, rel)
    dst = os.path.join(target, rel)
    info = os.lstat(src)
    if stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode):
        raise SystemExit("fixture-copy-source-invalid:" + rel)
    os.makedirs(os.path.dirname(dst), exist_ok=True)
    shutil.copy2(src, dst, follow_symlinks=False)

def tracked(spec):
    raw = subprocess.run(
        ["git", "-C", source, "ls-files", "-z", "--", spec],
        check=True, stdout=subprocess.PIPE).stdout
    values = [item.decode("utf-8") for item in raw.split(b"\0") if item]
    if not values:
        raise SystemExit("fixture-copy-untracked-or-empty:" + spec)
    for rel in values:
        copy_rel(rel)

inventory = layout["source_inventory"]
for spec in inventory["root_files"] + inventory["package_dirs"] + inventory["external_compile_inputs"]:
    tracked(spec)
for payloads in layout["runtime_payloads"].values():
    for item in payloads:
        tracked(item["source"])
for recipe in layout["recipes"].values():
    tracked(recipe["dockerfile"])
for rel in (
    ".dockerignore",
    ".gitignore",
    "deploy/build/Dockerfile.rust-artifacts",
    "deploy/build/release-build-layout.json",
    "scripts/ops/build-production-images.sh",
    "scripts/ops/lib/release-image-manifest.sh",
    "scripts/ops/lib/release-build-layout.sh",
):
    copy_rel(rel)
PY
chmod 0755 "$helper" "$layout_helper"
printf 'services: {}\n' >"$compose_file"
printf 'COMPOSE_TEST=1\n' >"$env_file"
git -C "$repo_dir" init -q
git -C "$repo_dir" config user.email fixture@example.invalid
git -C "$repo_dir" config user.name fixture
git -C "$repo_dir" add -- .
git -C "$repo_dir" commit -qm fixture
commit=$(git -C "$repo_dir" rev-parse HEAD)
# Model the original clean-checkout defect without ever copying or inspecting
# the user's real virtualenv. These synthetic ignored entries must remain
# absent from tracked payload bundles and image expectations.
mkdir -p -- "$repo_dir/nt/.venv/bin" "$repo_dir/nt/__pycache__"
ln -s ../python-fixture-target "$repo_dir/nt/.venv/bin/python"
printf '%s\n' 'ignored image fixture virtualenv bytes' >"$repo_dir/nt/.venv/ignored.txt"
printf '%s\n' 'ignored image fixture pycache bytes' >"$repo_dir/nt/__pycache__/ignored.pyc"
[ -z "$(git -C "$repo_dir" status --porcelain=v1 --untracked-files=all)" ] || {
  echo 'self-test: synthetic ignored NT tooling dirtied the source fixture' >&2
  exit 1
}

cat >"$fake_bin/docker" <<'PY'
#!/usr/bin/env python3
# Private test-double only. It never starts a daemon/client; every Buildx,
# Compose, inspect, and image-save response is synthesized below.
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import stat
import struct
import sys
import tarfile

args = sys.argv[1:]
if not args:
    raise SystemExit("fake-docker-empty-command")

repo = Path(os.environ["IMAGE_BUILD_FAKE_REPO"]).resolve()
images = Path(os.environ["IMAGE_BUILD_FAKE_IMAGES"]).resolve()
images.mkdir(mode=0o700, parents=True, exist_ok=True)
log_path = Path(os.environ["IMAGE_BUILD_DOCKER_LOG"])
commit = os.environ.get("LAGRANGE_CODE_COMMIT", "missing")

def log(detail):
    with log_path.open("a", encoding="utf-8", newline="\n") as handle:
        handle.write(f"commit={commit} args={' '.join(args)} {detail}\n")

def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode("utf-8")

def digest(value):
    return hashlib.sha256(value).hexdigest()

def write_bytes(path, value, mode):
    path = Path(path)
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    with path.open("wb") as handle:
        handle.write(value)
    os.chmod(path, mode)

def write_json(path, value, mode):
    write_bytes(path, canonical(value) + b"\n", mode)

def native_identity():
    return {
        "format": "lagrange-build-layout-native-v2",
        "target_platform": "linux/amd64",
        "host_triple": "x86_64-unknown-linux-musl",
        "rustc_vv": "rustc 1.97.1\nhost: x86_64-unknown-linux-musl\n",
        "cargo_version": "cargo 1.97.1 (fixture)",
        "apk_info_vv": "build-base-1\nmusl-dev-1\nopenssl-dev-1\npkgconf-1\npostgresql-dev-1\n",
        "native_packages": ["build-base", "musl-dev", "openssl-dev", "pkgconf", "postgresql-dev"],
        "compiler_env": {"CARGO_BUILD_JOBS": "2", "CARGO_TARGET_DIR": "/cargo-target", "RUSTFLAGS": "<unset>"},
    }

def write_native(destination):
    write_json(Path(destination) / ".release-build/native-identity.json", native_identity(), 0o600)

def arg_value(prefix):
    for item in args:
        if item.startswith(prefix):
            return item[len(prefix):]
    raise SystemExit("fake-docker-argument-missing:" + prefix)

def option_value(name):
    for index, item in enumerate(args):
        if item == name and index + 1 < len(args):
            return args[index + 1]
        if item.startswith(name + "="):
            return item[len(name) + 1:]
    raise SystemExit("fake-docker-option-missing:" + name)

def output_destination():
    raw = arg_value("type=local,dest=") if any(x.startswith("type=local,dest=") for x in args) else option_value("--output")
    if raw.startswith("type=local,dest="):
        raw = raw.split("dest=", 1)[1]
    return Path(raw).resolve()

def targeted_mode(recipe, binary):
    requested = os.environ.get("IMAGE_BUILD_PRODUCER_TARGET", "")
    if requested and requested != f"{recipe}/{binary}":
        return ""
    return os.environ.get("IMAGE_BUILD_PRODUCER_MODE", "")

def elf_binary(source_commit, binary):
    value = bytearray(64)
    value[:4] = b"\x7fELF"
    value[4] = 2
    value[5] = 1
    value[6] = 1
    struct.pack_into("<H", value, 18, 62)
    return bytes(value) + b" LAGRANGE-COMMIT=" + source_commit.encode("ascii") + b"\n" + binary.encode("ascii") + b"\n"

def write_producer(destination, context):
    request = json.loads((Path(context) / ".release-build/request.json").read_text(encoding="utf-8"))
    binary = arg_value("--build-arg=CARGO_BIN=")
    package = arg_value("--build-arg=CARGO_PACKAGE=")
    if package != request["package"] or binary not in request["bins"]:
        raise SystemExit("fake-docker-producer-request-mismatch")
    mode = targeted_mode(request["recipe"], binary)
    log(f"producer={request['recipe']}/{binary} mode={mode or 'normal'}")
    if mode == "producer-failure":
        raise SystemExit(42)
    destination = Path(destination)
    write_native(destination)
    producer = destination / ".release-build/producer" / binary
    bindir = producer / "bin"
    bindir.mkdir(mode=0o700, parents=True, exist_ok=True)
    payload = elf_binary(request["source_commit"], binary)
    write_bytes(bindir / binary, payload, 0o755)
    artifact = {
        "format": "lagrange-rust-artifact-v1",
        "source_commit": request["source_commit"],
        "package": request["package"],
        "bin": binary,
        "platform": "linux/amd64",
        "host_triple": "x86_64-unknown-linux-musl",
        "profile": "release",
        "features": [],
        "compile_env": request["compile_env"],
        "cache_key": request["cache_key"],
        "input_sha256": request["input_sha256"],
        "recipe_sha256": request["recipe_sha256"],
        "h_sha256": request["h_sha256"],
        "cargo_success": True,
        "binary_mode": "0755",
        "binary_sha256": digest(payload),
    }
    if mode == "wrong-commit-receipt":
        artifact["source_commit"] = "f" * 40
    elif mode == "tampered-receipt":
        artifact["binary_sha256"] = "0" * 64
    if mode != "missing-receipt":
        raw = canonical(artifact) + b"\n"
        write_bytes(producer / "artifact.json", raw, 0o644)
        write_bytes(producer / "COMPLETE", hashlib.sha256(raw).hexdigest().encode("ascii") + b"\n", 0o644)
    write_bytes(producer / "cargo.jsonl", canonical({
        "reason": "compiler-artifact", "target": {"name": binary},
        "executable": "/cargo-target/release/" + binary,
    }) + b"\n", 0o644)
    write_bytes(producer / "cargo.stderr", b"Compiling fixture\n", 0o644)
    write_json(producer / "cargo-summary.json", {
        "format": "lagrange-cargo-summary-v1", "cargo_success": True,
        "observed_bin": binary, "package": request["package"], "fresh_count": 0,
        "compiling_count": 1, "executable": "/cargo-target/release/" + binary, "cargo_ms": 1,
    }, 0o644)
    write_json(producer / "timing.json", {"format": "lagrange-cargo-timing-v1", "cargo_ms": 1}, 0o644)
    if mode == "tampered-binary":
        with (bindir / binary).open("ab") as handle:
            handle.write(b"tampered")

def validate_override(service, override):
    value = json.loads(Path(override).read_text(encoding="utf-8"))
    if set(value) != {"services"} or set(value["services"]) != {service}:
        raise SystemExit("fake-docker-override-service-invalid")
    build = value["services"][service].get("build")
    if not isinstance(build, dict) or set(build) != {"args", "additional_contexts"}:
        raise SystemExit("fake-docker-override-shape-invalid")
    args_value = build["args"]
    expected = {"LAGRANGE_CODE_COMMIT", "RUST_ARTIFACT_SOURCE", "RUST_ARTIFACT_HELPER_SHA256", "RUST_ARTIFACT_BUNDLE_SHA256"}
    if set(args_value) != expected or args_value["LAGRANGE_CODE_COMMIT"] != commit or args_value["RUST_ARTIFACT_SOURCE"] != "verified-artifacts":
        raise SystemExit("fake-docker-override-args-invalid")
    contexts = build["additional_contexts"]
    if set(contexts) != {"release_artifacts"} or not isinstance(contexts["release_artifacts"], str):
        raise SystemExit("fake-docker-override-context-invalid")
    return contexts["release_artifacts"]

def map_path(service):
    return images / f"service-{service}.json"

def remember_service(service, bundle):
    write_json(map_path(service), {"service": service, "commit": commit, "bundle": bundle}, 0o600)

def service_for_ref(reference):
    if ":" not in reference:
        raise SystemExit("fake-docker-image-ref-invalid")
    name, tag = reference.rsplit(":", 1)
    if tag != commit or not name.startswith("lagrange-station-"):
        raise SystemExit("fake-docker-image-ref-binding-invalid")
    service = name[len("lagrange-station-"):]
    allowed = {
        "db-role-bootstrap", "db-migrate", "api-server", "web", "research-worker",
        "recommendation-runner", "candidate-runner", "owner-beta-runner", "owner-equity-v2-runner",
        "nt-backtest-worker-1", "nt-backtest-worker-2", "paper-scheduler",
    }
    if service not in allowed:
        raise SystemExit("fake-docker-image-service-invalid")
    return service

def add_source_tree(entries, source, image_root):
    source = Path(source)
    image_root = image_root.strip("/")
    if source.is_file():
        entries[image_root] = (source.read_bytes(), stat.S_IMODE(source.stat().st_mode))
        return
    for path in sorted(source.rglob("*")):
        if path.is_dir():
            continue
        if not path.is_file() or path.is_symlink():
            raise SystemExit("fake-docker-payload-invalid")
        relative = path.relative_to(source).as_posix()
        entries[image_root + "/" + relative] = (path.read_bytes(), stat.S_IMODE(path.stat().st_mode))

def write_archive(service, source_commit):
    cache_marker = images / f"service-{service}.json"
    mapping = json.loads(cache_marker.read_text(encoding="utf-8"))
    if mapping != {"bundle": mapping["bundle"], "commit": source_commit, "service": service}:
        raise SystemExit("fake-docker-service-map-invalid")
    layout = json.loads((repo / "deploy/build/release-build-layout.json").read_text(encoding="utf-8"))
    record = layout["services"][service]
    entries = {}
    if record["kind"] == "rust":
        bundle = mapping["bundle"]
        if not isinstance(bundle, str) or not bundle:
            raise SystemExit("fake-docker-rust-bundle-missing")
        recipe = layout["recipes"][record["recipe"]]
        for item in recipe["runtime_binaries"]:
            path = Path(bundle) / "target/release" / item["bin"]
            entries[item["image"].lstrip("/")] = (path.read_bytes(), 0o755)
        payload_name = recipe.get("runtime_payload")
        for item in layout["runtime_payloads"].get(payload_name, []):
            # The verified-artifacts route consumes the host-selected bundle,
            # which mirrors Docker's frozen tracked/excluded context rather
            # than recursively walking ignored tooling from the checkout.
            add_source_tree(entries, Path(bundle) / item["source"], item["image"])
    elif record["kind"] == "database":
        entries["usr/local/bin/sqlx"] = (elf_binary(source_commit, "sqlx"), 0o755)
        for item in layout["runtime_payloads"]["database"]:
            add_source_tree(entries, repo / item["source"], item["image"])
    elif record["kind"] == "web":
        entries["app/apps/web/server.js"] = (b"fixture standalone server\n", 0o644)
        entries["app/apps/web/.next/fixture"] = (b"fixture\n", 0o644)
        entries["app/apps/web/.next/static/fixture"] = (b"fixture\n", 0o644)
    else:
        raise SystemExit("fake-docker-service-kind-invalid")
    directories = {""}
    for name in entries:
        parent = name.rsplit("/", 1)[0] if "/" in name else ""
        while parent:
            directories.add(parent)
            parent = parent.rsplit("/", 1)[0] if "/" in parent else ""
    raw_layer = io.BytesIO()
    with tarfile.open(fileobj=raw_layer, mode="w", format=tarfile.USTAR_FORMAT) as layer:
        for name in sorted(directories - {""}):
            info = tarfile.TarInfo(name)
            info.type = tarfile.DIRTYPE
            info.mode = 0o755
            info.mtime = 0
            layer.addfile(info)
        for name, (payload, mode) in sorted(entries.items()):
            info = tarfile.TarInfo(name)
            info.size = len(payload)
            info.mode = mode
            info.mtime = 0
            layer.addfile(info, io.BytesIO(payload))
    layer_bytes = raw_layer.getvalue()
    layer_digest = digest(layer_bytes)
    config = canonical({"architecture": "amd64", "os": "linux", "config": {"Labels": {
        "org.opencontainers.image.revision": source_commit}}, "rootfs": {"type": "layers"}})
    config_digest = digest(config)
    manifest = canonical({"schemaVersion": 2, "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "config": {"digest": "sha256:" + config_digest, "mediaType": "application/vnd.oci.image.config.v1+json", "size": len(config)},
        "layers": [{"digest": "sha256:" + layer_digest, "mediaType": "application/vnd.oci.image.layer.v1.tar", "size": len(layer_bytes)}]})
    manifest_digest = digest(manifest)
    runnable = {"digest": "sha256:" + manifest_digest, "mediaType": "application/vnd.oci.image.manifest.v1+json", "size": len(manifest), "platform": {"os": "linux", "architecture": "amd64"}}
    blobs = {config_digest: config, layer_digest: layer_bytes, manifest_digest: manifest}
    # Alternate the two observed root forms so product verification exercises
    # both direct manifest and provenance-index bindings with coherent IDs.
    ordinal = ["db-role-bootstrap", "db-migrate", "api-server", "web", "research-worker", "recommendation-runner", "candidate-runner", "owner-beta-runner", "owner-equity-v2-runner", "nt-backtest-worker-1", "nt-backtest-worker-2", "paper-scheduler"].index(service)
    if ordinal % 2 == 0:
        root_digest = manifest_digest
        outer = {key: runnable[key] for key in ("digest", "mediaType", "size")}
    else:
        attestation = canonical({"schemaVersion": 2, "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "config": {"digest": "sha256:" + config_digest, "mediaType": "application/vnd.oci.image.config.v1+json", "size": len(config)}, "layers": []})
        attestation_digest = digest(attestation)
        blobs[attestation_digest] = attestation
        provenance = canonical({"schemaVersion": 2, "mediaType": "application/vnd.oci.image.index.v1+json", "manifests": [runnable, {
            "digest": "sha256:" + attestation_digest, "mediaType": "application/vnd.oci.image.manifest.v1+json", "size": len(attestation),
            "platform": {"os": "unknown", "architecture": "unknown"},
            "annotations": {"vnd.docker.reference.type": "attestation-manifest", "vnd.docker.reference.digest": "sha256:" + manifest_digest}}]})
        root_digest = digest(provenance)
        blobs[root_digest] = provenance
        outer = {"digest": "sha256:" + root_digest, "mediaType": "application/vnd.oci.image.index.v1+json", "size": len(provenance)}
    index = canonical({"schemaVersion": 2, "mediaType": "application/vnd.oci.image.index.v1+json", "manifests": [outer]})
    docker_manifest = canonical([{ "Config": "blobs/sha256/" + config_digest, "RepoTags": None, "Layers": ["blobs/sha256/" + layer_digest]}])
    archive = images / (root_digest + ".tar")
    if not archive.exists():
        with tarfile.open(archive, "w", format=tarfile.USTAR_FORMAT) as output:
            for name in ("blobs", "blobs/sha256"):
                info = tarfile.TarInfo(name)
                info.type = tarfile.DIRTYPE
                info.mode = 0o755
                info.mtime = 0
                output.addfile(info)
            for key, payload in sorted(blobs.items()):
                info = tarfile.TarInfo("blobs/sha256/" + key)
                info.size = len(payload)
                info.mode = 0o644
                info.mtime = 0
                output.addfile(info, io.BytesIO(payload))
            for name, payload in (("index.json", index), ("manifest.json", docker_manifest), ("oci-layout", canonical({"imageLayoutVersion": "1.0.0"}))):
                info = tarfile.TarInfo(name)
                info.size = len(payload)
                info.mode = 0o644
                info.mtime = 0
                output.addfile(info, io.BytesIO(payload))
    return "sha256:" + root_digest

def compose_command():
    if "version" in args:
        return 0
    if "config" in args:
        return 0
    if "build" not in args:
        raise SystemExit("fake-docker-compose-command-invalid")
    build_at = args.index("build")
    if args[build_at + 1:] != ["--pull=false", args[-1]]:
        raise SystemExit("fake-docker-compose-not-one-service")
    service = args[-1]
    if os.environ.get("COMPOSE_PARALLEL_LIMIT") != "1":
        raise SystemExit("fake-docker-compose-parallel-invalid")
    override = os.environ.get("COMPOSE_BUILD_OVERRIDE_FILE", "")
    bundle = validate_override(service, override) if override else None
    remember_service(service, bundle)
    log(f"parallel=1 service={service} bundle={'yes' if bundle else 'none'}")
    if os.environ.get("IMAGE_BUILD_FAIL_SERVICE", "") == service:
        return 42
    return 0

if args[0] == "compose":
    log("compose")
    raise SystemExit(compose_command())
if args[0] == "buildx" and len(args) >= 2 and args[1] == "build":
    target = option_value("--target")
    destination = output_destination()
    context = Path(args[-1]).resolve()
    log(f"buildx target={target}")
    if target == "native":
        write_native(destination)
    elif target == "artifacts":
        write_producer(destination, context)
    else:
        raise SystemExit("fake-docker-buildx-target-invalid")
    raise SystemExit(0)
if args[0:2] == ["image", "inspect"]:
    reference = args[-1]
    service = service_for_ref(reference)
    override_id = os.environ.get("IMAGE_BUILD_FAKE_IMAGE_ID", "")
    resolved_service = service
    swap_service = os.environ.get("IMAGE_BUILD_FAKE_TAG_SWAP_SERVICE", "")
    if not override_id and swap_service == service:
        prior = sum(
            f" image-inspect service={service} image_id=" in line
            for line in log_path.read_text(encoding="utf-8").splitlines()
        ) if log_path.exists() else 0
        if prior >= 1:
            resolved_service = os.environ.get("IMAGE_BUILD_FAKE_TAG_SWAP_TO_SERVICE", "")
            if not resolved_service or resolved_service == service:
                raise SystemExit("fake-docker-tag-swap-target-invalid")
            service_for_ref(f"lagrange-station-{resolved_service}:{commit}")
    image_id = override_id or write_archive(resolved_service, commit)
    revision = os.environ.get("IMAGE_BUILD_FAKE_REVISION", commit)
    log(f"image-inspect service={service} image_id={image_id}")
    if resolved_service != service:
        log(f"tag-swap service={service} resolved_service={resolved_service} image_id={image_id}")
    print(image_id + "|" + revision)
    raise SystemExit(0)
if args[0:2] == ["image", "save"]:
    output = Path(option_value("--output")).resolve()
    image_id = args[-1]
    if not image_id.startswith("sha256:"):
        raise SystemExit("fake-docker-save-image-id-invalid")
    archive = images / (image_id[7:] + ".tar")
    if not archive.is_file():
        raise SystemExit("fake-docker-save-archive-missing")
    output.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    shutil.copyfile(archive, output)
    log(f"image-save image_id={image_id}")
    raise SystemExit(0)
if args[0] == "inspect":
    # The gate accepts a tightly bounded container-health projection only.
    name = args[-1]
    if name != "lagrange-station-research-worker-1":
        raise SystemExit("fake-docker-container-unexpected")
    print("1" * 64 + "\ttrue\tfalse\tfalse\thealthy\t0\tlagrange-station\tsha256:" + "2" * 64 + "\t0")
    log("container-inspect")
    raise SystemExit(0)
raise SystemExit("fake-docker-command-unsupported:" + " ".join(args))
PY
chmod 0755 "$fake_bin/docker"
cat >"$fake_bin/systemctl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
[ "${1:-}" = show ] || exit 64
unit=${2:-}
property=
for argument in "$@"; do
  case "$argument" in --property=*) property=${argument#--property=} ;; esac
done
[ -n "$property" ] || exit 64
case "$unit" in lagrange-build.service|lagrange-health.service) ;; *) exit 5 ;; esac
IFS=, read -r -a properties <<<"$property"
for key in "${properties[@]}"; do
  case "$key" in
    LoadState) value=loaded ;;
    ActiveState) value=active ;;
    SubState) value=running ;;
    ExecMainStatus) value=0 ;;
    MainPID) value=1 ;;
    ControlGroup)
      # The product gate must prove that this actual apply process is inside
      # the selected build unit cgroup.  This fake models that positive input
      # with the test process's real cgroup instead of an unrelated fixture
      # path; the separate outside-cgroup negative proof remains unchanged.
      value=$(awk -F: 'NF == 3 && $3 ~ /^\// { print $3; exit }' /proc/self/cgroup)
      [ -n "$value" ] || exit 66
      ;;
    Nice) value=10 ;;
    IOSchedulingClass) value=idle ;;
    IOSchedulingPriority) value=0 ;;
    NRestarts) value=0 ;;
    *) exit 65 ;;
  esac
  printf '%s=%s\n' "$key" "$value"
done
EOF
cat >"$fake_bin/journalctl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
boot=$(tr -d '-' </proc/sys/kernel/random/boot_id)
timestamp_seconds=$(date -u +%s)
timestamp_us=$(( (timestamp_seconds - 60) * 1000000 ))
printf '{"_BOOT_ID":"%s","_TRANSPORT":"kernel","__CURSOR":"fixture","__REALTIME_TIMESTAMP":"%s","MESSAGE":"fixture kernel healthy"}\n' \
  "$boot" "$timestamp_us"
EOF
cat >"$fake_bin/ps" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
[ "${1:-}" = -eo ] && [ "${2:-}" = comm= ] || exit 64
exit 0
EOF
chmod 0755 "$fake_bin/systemctl" "$fake_bin/journalctl" "$fake_bin/ps"
export PATH="$fake_bin:$PATH"
export IMAGE_BUILD_DOCKER_LOG=$docker_log
export IMAGE_BUILD_FAKE_REPO=$repo_dir
export IMAGE_BUILD_FAKE_IMAGES=$out_dir/fake-images
export RBL_LOCK_PREFIX=$out_dir/whole-release-lock
export RELEASE_BUILD_SYSTEMD_UNIT=lagrange-build.service
export RELEASE_BUILD_SYSTEMD_MANAGER=system
export RELEASE_BUILD_HEALTH_UNITS=lagrange-health.service
export RELEASE_BUILD_HEALTH_CONTAINERS=lagrange-station-research-worker-1
export LAGRANGE_CODE_COMMIT=$commit

bash "$helper" --plan --compose-file "$compose_file" --env-file "$env_file" >"$out_dir/plan.out"
grep -Fq 'PRODUCTION_IMAGE_BUILD_PLAN mode=plan' "$out_dir/plan.out"
grep -Fq 'research-range-raw' "$out_dir/plan.out"
grep -Fq 'network caveat:' "$out_dir/plan.out"
[ ! -s "$docker_log" ]

bash "$helper" --preflight --compose-file "$compose_file" --env-file "$env_file" >"$out_dir/preflight.out"
grep -Fq 'PRODUCTION_IMAGE_BUILD_PREFLIGHT: PASS' "$out_dir/preflight.out"
grep -Fq 'config --quiet' "$docker_log"

# Manifest output shape is rejected before any Docker command. Cover existing
# regular files, dangling symlinks, missing/non-directory parents, and symlink
# parents without changing the fixture worktree.
: >"$docker_log"
printf '%s\n' existing >"$out_dir/early-existing.manifest"
if COMPOSE_PARALLEL_LIMIT=37 bash "$helper" --apply --compose-file "$compose_file" \
  --env-file "$env_file" --manifest-file "$out_dir/early-existing.manifest" \
  >"$out_dir/early-existing.out" 2>&1; then
  echo 'self-test: existing manifest output unexpectedly passed' >&2
  exit 1
fi
grep -Fq 'manifest-file already exists; refusing to overwrite it' "$out_dir/early-existing.out"
[ ! -s "$docker_log" ]

ln -s "$out_dir/no-manifest-target" "$out_dir/early-dangling.manifest"
if COMPOSE_PARALLEL_LIMIT=37 bash "$helper" --apply --compose-file "$compose_file" \
  --env-file "$env_file" --manifest-file "$out_dir/early-dangling.manifest" \
  >"$out_dir/early-dangling.out" 2>&1; then
  echo 'self-test: dangling manifest output unexpectedly passed' >&2
  exit 1
fi
grep -Fq 'manifest-file must not traverse a symlink' "$out_dir/early-dangling.out"
[ ! -s "$docker_log" ]

if COMPOSE_PARALLEL_LIMIT=37 bash "$helper" --apply --compose-file "$compose_file" \
  --env-file "$env_file" --manifest-file "$out_dir/missing-parent/manifest" \
  >"$out_dir/missing-parent.out" 2>&1; then
  echo 'self-test: missing manifest parent unexpectedly passed' >&2
  exit 1
fi
grep -Fq 'manifest-file parent directory is missing or a symlink' "$out_dir/missing-parent.out"
[ ! -s "$docker_log" ]

printf '%s\n' not-a-directory >"$out_dir/non-directory-parent"
if COMPOSE_PARALLEL_LIMIT=37 bash "$helper" --apply --compose-file "$compose_file" \
  --env-file "$env_file" --manifest-file "$out_dir/non-directory-parent/manifest" \
  >"$out_dir/non-directory-parent.out" 2>&1; then
  echo 'self-test: non-directory manifest parent unexpectedly passed' >&2
  exit 1
fi
grep -Fq 'manifest-file parent directory is missing or a symlink' "$out_dir/non-directory-parent.out"
[ ! -s "$docker_log" ]

mkdir "$out_dir/real-manifest-parent"
ln -s "$out_dir/real-manifest-parent" "$out_dir/symlink-manifest-parent"
if COMPOSE_PARALLEL_LIMIT=37 bash "$helper" --apply --compose-file "$compose_file" \
  --env-file "$env_file" --manifest-file "$out_dir/symlink-manifest-parent/manifest" \
  >"$out_dir/symlink-parent.out" 2>&1; then
  echo 'self-test: symlinked manifest parent unexpectedly passed' >&2
  exit 1
fi
grep -Fq 'manifest-file must not traverse a symlink' "$out_dir/symlink-parent.out"
[ ! -s "$docker_log" ]

before_env=$(sha256sum "$env_file")
# Product init owns creation of the fresh official state parent.  Keeping it
# absent here ensures the fixture cannot hide a state-initialization defect.
[ ! -e "$out_dir/.lagrange-build-state" ]
if IMAGE_BUILD_FAIL_SERVICE=web COMPOSE_PARALLEL_LIMIT=37 \
  bash "$helper" --apply --compose-file "$compose_file" --env-file "$env_file" \
  --manifest-file "$manifest_file" >"$out_dir/middle-failure.out" 2>&1; then
  echo 'self-test: middle-service failure unexpectedly passed' >&2
  exit 1
fi
grep -Fq 'Compose build failed: web' "$out_dir/middle-failure.out"
if grep -Fq 'service=web status=success' "$out_dir/middle-failure.out" ||
   grep -Fq 'service=research-worker status=start' "$out_dir/middle-failure.out"; then
  echo 'self-test: build loop continued after the first failed service' >&2
  exit 1
fi
grep -Fq 'build --pull=false api-server' "$docker_log"
if grep -Fq 'build --pull=false research-worker' "$docker_log" ||
   grep -Fq ' image inspect ' "$docker_log"; then
  echo 'self-test: failed build inspected images or continued to a later service' >&2
  exit 1
fi
[ ! -e "$manifest_file" ]

: >"$docker_log"
IMAGE_BUILD_FAIL_SERVICE= COMPOSE_PARALLEL_LIMIT=37 \
  bash "$helper" --apply --compose-file "$compose_file" --env-file "$env_file" \
  --manifest-file "$manifest_file" >"$out_dir/apply.out"
grep -Fq 'PRODUCTION_IMAGE_BUILD: PASS' "$out_dir/apply.out"
[ "$before_env" = "$(sha256sum "$env_file")" ]
[ "$(stat -c %a "$manifest_file")" = 600 ]
grep -Fxq 'LAGRANGE_RELEASE_MANIFEST_V2' "$manifest_file"
grep -Fxq "commit|$commit" "$manifest_file"
[ "$(grep -c '^image|' "$manifest_file")" -eq 12 ]
for service in db-role-bootstrap db-migrate api-server web research-worker \
  recommendation-runner candidate-runner owner-beta-runner owner-equity-v2-runner nt-backtest-worker-1 \
  nt-backtest-worker-2 paper-scheduler; do
  grep -Fq "PRODUCTION_IMAGE_BUILD_SERVICE service=$service status=start elapsed_seconds=" \
    "$out_dir/apply.out"
  grep -Fq "PRODUCTION_IMAGE_BUILD_SERVICE service=$service status=success elapsed_seconds=" \
    "$out_dir/apply.out"
done
declare -A observed_image_ids=()
for service in db-role-bootstrap db-migrate api-server web research-worker \
  recommendation-runner candidate-runner owner-beta-runner owner-equity-v2-runner nt-backtest-worker-1 \
  nt-backtest-worker-2 paper-scheduler; do
  image_marker="image-inspect service=$service image_id="
  image_id=$(awk -v marker="$image_marker" '
    index($0, marker) {
      observed=substr($0, index($0, marker) + length(marker))
      if (count > 0 && observed != value) bad=1
      value=observed
      count++
    }
    END {
      if (count != 2 || bad || value == "") exit 1
      print value
    }
  ' "$docker_log") || {
    echo "self-test: image inspection identity evidence is inconsistent: $service" >&2
    exit 1
  }
  [[ "$image_id" =~ ^sha256:[0-9a-f]{64}$ ]] || {
    echo "self-test: image inspection identity is malformed: $service" >&2
    exit 1
  }
  [ -z "${observed_image_ids[$image_id]:-}" ] || {
    echo "self-test: two services received the same fake OCI root: $service" >&2
    exit 1
  }
  observed_image_ids["$image_id"]=$service
  grep -Fxq "image|$service|lagrange-station-$service:$commit|$image_id|$commit" \
    "$manifest_file"
done
[ "${#observed_image_ids[@]}" -eq 12 ]

# The full successful run must have selected only tracked NT payloads for both
# shared D2/D5 bundles while preserving their duplicate-consumer reuse. The
# ignored virtualenv/cache remain byte/link-identical and absent from bundle and
# source-derived archive-request evidence.
ignored_bundle_entry=$(find "$out_dir/.lagrange-build-state/$commit/bundles" \
  \( -path '*/.venv' -o -path '*/.venv/*' -o -path '*/__pycache__' -o -path '*/__pycache__/*' \) \
  -print -quit)
[ -z "$ignored_bundle_entry" ] || {
  echo "self-test: ignored NT tooling entered a runtime bundle: $ignored_bundle_entry" >&2
  exit 1
}
if grep -R -Eq '(^|/)(\.venv|__pycache__)(/|$)|\.py[cod]' \
  "$out_dir/.lagrange-build-state/$commit/verification"; then
  echo 'self-test: ignored NT tooling entered saved-image source expectations' >&2
  exit 1
fi
[ -L "$repo_dir/nt/.venv/bin/python" ] &&
  [ "$(readlink -- "$repo_dir/nt/.venv/bin/python")" = ../python-fixture-target ] &&
  grep -Fxq 'ignored image fixture virtualenv bytes' "$repo_dir/nt/.venv/ignored.txt" &&
  grep -Fxq 'ignored image fixture pycache bytes' "$repo_dir/nt/__pycache__/ignored.pyc" &&
  [ -z "$(git -C "$repo_dir" status --porcelain=v1 --untracked-files=all)" ] || {
    echo 'self-test: runtime payload selection modified ignored NT tooling or dirtied the source fixture' >&2
    exit 1
  }
python3 - "$out_dir/fake-images" "$out_dir/.lagrange-build-state/$commit/bundles" <<'PY'
import json, pathlib, sys
images=pathlib.Path(sys.argv[1]); bundles=pathlib.Path(sys.argv[2])
def selected(service):
    return json.loads((images/f"service-{service}.json").read_text(encoding="utf-8"))["bundle"]
d2=selected("recommendation-runner")
d5=selected("nt-backtest-worker-1")
assert d2==selected("candidate-runner")==str(bundles/"D2")
assert d5==selected("nt-backtest-worker-2")==str(bundles/"D5")
PY
runtime_inventory_file=$out_dir/runtime-payload-inventory.json
bash -c '. "$1"; rbl_runtime_payload_inventory "$2" "$3"' \
  fixture "$layout_helper" "$repo_dir" "$layout_config" >"$runtime_inventory_file"
python3 - "$runtime_inventory_file" "$out_dir/.lagrange-build-state/$commit/verification" <<'PY'
import json, pathlib, sys
inventory=json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
verification=pathlib.Path(sys.argv[2])
payload=inventory["payload_groups"]["backtest"][0]
source=payload["source"]
image=payload["image"].lstrip("/")
expected=[]
for entry in payload["entries"]:
    if entry["kind"]!="file": continue
    relative=entry["path"][len(source)+1:]
    expected.append(image+"/"+relative)
for service in ("recommendation-runner","candidate-runner","nt-backtest-worker-1","nt-backtest-worker-2"):
    request=json.loads((verification/f"{service}-request.json").read_text(encoding="utf-8"))
    observed=sorted(item["path"] for item in request["files"] if item["path"].startswith(image+"/"))
    assert observed==sorted(expected)
    assert image in request["nonempty_directories"]
    assert not any(".venv" in path or "__pycache__" in path or path.endswith((".pyc",".pyo",".pyd")) for path in observed)
PY

compose_prefix="commit=$commit args=compose --env-file $env_file --file $compose_file"
config_count=$(grep -Fc "$compose_prefix config --quiet compose" "$docker_log")
build_count=$(grep -Ec ' build --pull=false [a-z0-9-]+ compose$' "$docker_log")
[ "$config_count" -eq 1 ]
[ "$build_count" -eq 12 ]
awk -v expected='db-role-bootstrap db-migrate api-server web research-worker recommendation-runner candidate-runner owner-beta-runner owner-equity-v2-runner nt-backtest-worker-1 nt-backtest-worker-2 paper-scheduler' '
  BEGIN { split(expected, services, " ") }
  index($0, " parallel=1 service=") {
    detail=$0
    sub(/^.* parallel=/, "parallel=", detail)
    count++
    bundle=(services[count] == "db-role-bootstrap" || services[count] == "db-migrate" || services[count] == "web") ? "none" : "yes"
    if (detail != "parallel=1 service=" services[count] " bundle=" bundle) bad=1
  }
  END { exit !(count == 12 && !bad) }
' "$docker_log" || {
  echo 'self-test: Compose order, parallel limit, or artifact routing evidence is malformed' >&2
  exit 1
}
for service in db-role-bootstrap db-migrate api-server web research-worker \
  recommendation-runner candidate-runner owner-beta-runner owner-equity-v2-runner nt-backtest-worker-1 \
  nt-backtest-worker-2 paper-scheduler; do
  compose_suffix="build --pull=false $service compose"
  compose_line=$(grep -F " $compose_suffix" "$docker_log")
  [ "$(grep -Fc " $compose_suffix" "$docker_log")" -eq 1 ] || {
    echo "self-test: Compose invocation count is not one: $service" >&2
    exit 1
  }
  case "$service" in
    db-role-bootstrap|db-migrate|web)
      [ "$compose_line" = "$compose_prefix $compose_suffix" ] || {
        echo "self-test: source-only Compose invocation is malformed: $service" >&2
        exit 1
      }
      ;;
    *)
      compose_middle=${compose_line#"$compose_prefix --file "}
      [ "$compose_middle" != "$compose_line" ]
      override_path=${compose_middle%" $compose_suffix"}
      [ "$override_path" != "$compose_middle" ]
      override_prefix="$out_dir/.lagrange-build-state/$commit/overrides/$service-"
      case "$override_path" in "$override_prefix"*.json) ;; *)
        echo "self-test: artifact Compose override path is malformed: $service" >&2
        exit 1
      esac
      override_pid=${override_path#"$override_prefix"}
      override_pid=${override_pid%.json}
      [[ "$override_pid" =~ ^[0-9]+$ ]] || {
        echo "self-test: artifact Compose override identity is malformed: $service" >&2
        exit 1
      }
      ;;
  esac
done
for excluded in reverse-proxy postgres research-range-raw live-node-owner; do
  if grep -Eq "build --pull=false .*($excluded)" "$docker_log"; then
    echo "self-test: excluded service entered local image build: $excluded" >&2
    exit 1
  fi
done
if grep -Eiq ' (up|run|restart|start)( |$)' "$docker_log"; then
  echo 'self-test: image helper invoked a container lifecycle command' >&2
  exit 1
fi

# A same-revision tag may be retargeted after all saved archives have been
# verified but before final V2 publication.  Give every first inspection a
# valid, service-specific saved OCI root, then make only paper-scheduler's
# final lookup resolve to api-server's otherwise valid root.  Exact equality
# with the per-service byte-verified identity must fail closed.
tag_swap_dir=$out_dir/tag-swap
tag_swap_manifest=$tag_swap_dir/production-images.manifest
mkdir -m 0700 -- "$tag_swap_dir"
: >"$docker_log"
if IMAGE_BUILD_FAKE_TAG_SWAP_SERVICE=paper-scheduler \
  IMAGE_BUILD_FAKE_TAG_SWAP_TO_SERVICE=api-server \
  COMPOSE_PARALLEL_LIMIT=37 \
  bash "$helper" --apply --compose-file "$compose_file" --env-file "$env_file" \
  --manifest-file "$tag_swap_manifest" >"$tag_swap_dir/apply.out" 2>&1; then
  echo 'self-test: changed same-revision final image tag unexpectedly passed' >&2
  exit 1
fi
grep -Fq 'final image_id differs from byte-verified image: paper-scheduler' "$tag_swap_dir/apply.out"
[ ! -e "$tag_swap_manifest" ] && [ ! -L "$tag_swap_manifest" ]
awk '
  index($0, " image-inspect service=") {
    detail=$0
    sub(/^.* image-inspect service=/, "", detail)
    split(detail, fields, " image_id=")
    service=fields[1]
    image_id=fields[2]
    inspect_count[service]++
    if (inspect_count[service] == 1) first[service]=image_id
    else if (inspect_count[service] == 2) second[service]=image_id
    else bad=1
  }
  index($0, " image-save image_id=") {
    image_id=$0
    sub(/^.* image-save image_id=/, "", image_id)
    saved[image_id]++
    save_count++
  }
  END {
    split("db-role-bootstrap db-migrate api-server web research-worker recommendation-runner candidate-runner owner-beta-runner owner-equity-v2-runner nt-backtest-worker-1 nt-backtest-worker-2 paper-scheduler", expected, " ")
    for (i=1; i<=12; i++) {
      service=expected[i]
      image_id=first[service]
      if (inspect_count[service] != 2 || image_id !~ /^sha256:[0-9a-f]+$/ || length(image_id) != 71 || saved[image_id] != 1) bad=1
      if (seen_first[image_id]++) bad=1
      if (service == "paper-scheduler") {
        if (second[service] == image_id || second[service] != first["api-server"]) bad=1
      } else if (second[service] != image_id) bad=1
    }
    exit !(save_count == 12 && !bad)
  }
' "$docker_log" || {
  echo 'self-test: tag swap did not preserve twelve valid saved roots and exact per-service final identity binding' >&2
  exit 1
}
grep -Fq "tag-swap service=paper-scheduler resolved_service=api-server image_id=" "$docker_log"
cp -- "$docker_log" "$tag_swap_dir/docker.log"
chmod 0600 -- "$tag_swap_dir/docker.log"

if bash "$helper" --apply --compose-file "$compose_file" --env-file "$env_file" \
  >"$out_dir/missing-manifest.out" 2>&1; then
  echo 'self-test: apply without manifest path unexpectedly passed' >&2
  exit 1
fi
grep -Fq -- '--apply requires --manifest-file' "$out_dir/missing-manifest.out"

if IMAGE_BUILD_FAKE_REVISION=0123456789abcdef0123456789abcdef01234567 \
  bash "$helper" --apply --compose-file "$compose_file" --env-file "$env_file" \
  --manifest-file "$out_dir/revision-mismatch.manifest" \
  >"$out_dir/revision-mismatch.out" 2>&1; then
  echo 'self-test: mismatched image revision unexpectedly passed' >&2
  exit 1
fi
grep -Fq 'built image revision label does not match source commit' "$out_dir/revision-mismatch.out"
[ ! -e "$out_dir/revision-mismatch.manifest" ]

if IMAGE_BUILD_FAKE_IMAGE_ID=sha256:not-a-real-image-id \
  bash "$helper" --apply --compose-file "$compose_file" --env-file "$env_file" \
  --manifest-file "$out_dir/image-id-mismatch.manifest" \
  >"$out_dir/image-id-mismatch.out" 2>&1; then
  echo 'self-test: malformed image_id unexpectedly passed' >&2
  exit 1
fi
grep -Fq 'built image_id is not an exact local Docker image ID' "$out_dir/image-id-mismatch.out"
[ ! -e "$out_dir/image-id-mismatch.manifest" ]

: >"$docker_log"
if LAGRANGE_CODE_COMMIT=0123456789abcdef0123456789abcdef01234567 \
  bash "$helper" --preflight --compose-file "$compose_file" --env-file "$env_file" \
  >"$out_dir/mismatch.out" 2>&1; then
  echo 'self-test: mismatched commit unexpectedly passed' >&2
  exit 1
fi
grep -Fq 'does not match the build root HEAD' "$out_dir/mismatch.out"
[ ! -s "$docker_log" ]

printf 'untracked fixture\n' >"$repo_dir/untracked.fixture"
if LAGRANGE_CODE_COMMIT="$commit" \
  bash "$helper" --preflight --compose-file "$compose_file" --env-file "$env_file" \
  >"$out_dir/dirty.out" 2>&1; then
  echo 'self-test: dirty build root unexpectedly passed' >&2
  exit 1
fi
grep -Fq 'worktree is not clean' "$out_dir/dirty.out"
rm -f -- "$repo_dir/untracked.fixture"

mkdir -p "$repo_dir/docs"
printf 'official workbook fixture\n' >"$repo_dir/docs/kis_openapi_entiredocs_20260818_030007.xlsx"
LAGRANGE_CODE_COMMIT="$commit" \
  bash "$helper" --preflight --compose-file "$compose_file" --env-file "$env_file" \
  >"$out_dir/allowed-workbook.out"
grep -Fq 'PRODUCTION_IMAGE_BUILD_PREFLIGHT: PASS' "$out_dir/allowed-workbook.out"
rm -f -- "$repo_dir/docs/kis_openapi_entiredocs_20260818_030007.xlsx"

if LAGRANGE_CODE_COMMIT=not-a-commit \
  bash "$helper" --preflight --compose-file "$compose_file" --env-file "$env_file" \
  >"$out_dir/invalid.out" 2>&1; then
  echo 'self-test: invalid commit unexpectedly passed' >&2
  exit 1
fi
grep -Fq 'exactly 40 lowercase hexadecimal' "$out_dir/invalid.out"

echo 'PRODUCTION_IMAGE_BUILD_SELF_TEST: PASS (fake Docker/root fixture and synthetic OCI parser coverage only; no actual image or performance acceptance)'
