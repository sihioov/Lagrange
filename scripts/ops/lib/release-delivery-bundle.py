#!/usr/bin/env python3
"""Export and import authenticated, content-pinned release image bundles.

The external release workflow authenticates each flat asset with GitHub's
reported digest before invoking import. This tool validates the complete
closed bundle contract, imports images only after every byte and identity is
checked, and never starts a container.
"""

from __future__ import annotations

import hashlib
import importlib.util
import json
import os
import re
import runpy
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path
from typing import Any, BinaryIO


BUNDLE_FORMAT = "LAGRANGE_RELEASE_DELIVERY_BUNDLE_V1"
TRANSPORT_FORMAT = "docker-save-zstd-v1"
PLATFORM = "linux/amd64"
MAX_ASSET_BYTES = 1_800_000_000
MAX_TRANSPORT_BYTES = 64 * 1024 * 1024 * 1024
MAX_JSON_BYTES = 2 * 1024 * 1024
CHUNK_PREFIX = "images.tar.zst.part"
RELEASE_MANIFEST = "release.manifest"
BUNDLE_MANIFEST = "release-bundle.json"
D8_ARTIFACT_ASSET = "d8-artifact.json"
D8_COMPLETE_ASSET = "d8-COMPLETE"
COMPOSE_OVERRIDE = "compose-offline.override.yml"
V3_SERVICES = (
    "db-role-bootstrap",
    "db-migrate",
    "api-server",
    "web",
    "research-worker",
    "recommendation-runner",
    "candidate-runner",
    "owner-beta-runner",
    "owner-equity-v2-runner",
    "nt-backtest-worker-1",
    "nt-backtest-worker-2",
    "paper-scheduler",
    "owner-beta-simulation-runner",
)
DEPENDENCIES = (
    {
        "name": "postgres",
        "source_ref": "postgres@sha256:3a82e1f56c8f0f5616a11103ac3d47e632c3938698946a7ad26da0df1334744a",
        "archive_ref": "lagrange-delivery/postgres:18.4",
        "compose_services": ("postgres", "research-schema-check"),
    },
    {
        "name": "reverse-proxy",
        "source_ref": "nginxinc/nginx-unprivileged@sha256:7377697a821c131a924a7105fafbe7414db4e9fcc77a6f08f776f33f141ec3f8",
        "archive_ref": "lagrange-delivery/nginx-unprivileged:1.28-alpine",
        "compose_services": ("reverse-proxy",),
    },
    {
        "name": "alpine",
        "source_ref": "alpine@sha256:48b0309ca019d89d40f670aa1bc06e426dc0931948452e8491e3d65087abc07d",
        "archive_ref": "lagrange-delivery/alpine:3.21",
        "compose_services": ("research-raw-init",),
    },
)
IMAGE_LABEL = "org.opencontainers.image.revision"
IMAGE_ID_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
HASH_RE = re.compile(r"^[0-9a-f]{64}$")
DOCKER_CONFIG_RE = re.compile(r"^(?:[0-9a-f]{64}\.json|blobs/sha256/[0-9a-f]{64})$")
OCI_INDEX_MEDIA_TYPES = {
    "application/vnd.oci.image.index.v1+json",
    "application/vnd.docker.distribution.manifest.list.v2+json",
}
OCI_MANIFEST_MEDIA_TYPES = {
    "application/vnd.oci.image.manifest.v1+json",
    "application/vnd.docker.distribution.manifest.v2+json",
}


class BundleError(Exception):
    pass


def fail(reason: str) -> None:
    raise BundleError(reason)


def canonical_json(value: Any) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True) + "\n").encode("ascii")


def create_file(path: Path, data: bytes, mode: int) -> None:
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, mode)
    with os.fdopen(fd, "wb") as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())
    os.chmod(path, mode)


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def checked_commit(value: str) -> str:
    if not COMMIT_RE.fullmatch(value) or value == "0" * 40:
        fail("commit-invalid")
    return value


def path_arg(value: str, label: str) -> Path:
    if not value or not value.startswith("/") or value == "/" or value.endswith("/"):
        fail(f"{label}-path-invalid")
    if "\n" in value or "\r" in value or "//" in value:
        fail(f"{label}-path-invalid")
    if any(part in (".", "..", "") for part in value.split("/")[1:]):
        fail(f"{label}-path-invalid")
    return Path(value)


def check_trusted_path_components(path: Path, label: str) -> None:
    current = Path("/")
    for component in path.parts[1:]:
        current /= component
        try:
            info = os.lstat(current)
        except OSError:
            fail(f"{label}-path-unavailable")
        if stat.S_ISLNK(info.st_mode) or not stat.S_ISDIR(info.st_mode):
            fail(f"{label}-path-component-invalid")
        if info.st_uid != 0 or info.st_mode & 0o022:
            fail(f"{label}-path-untrusted")


def trusted_directory(path: Path, label: str, require_private: bool = True) -> None:
    check_trusted_path_components(path, label)
    info = os.lstat(path)
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != 0:
        fail(f"{label}-directory-invalid")
    if info.st_mode & 0o022:
        fail(f"{label}-directory-writable-by-group-or-other")
    if require_private and info.st_mode & 0o077:
        fail(f"{label}-directory-not-private")


def trusted_file(path: Path, label: str, max_size: int) -> bytes:
    check_trusted_path_components(path.parent, label)
    try:
        info = os.lstat(path)
    except OSError:
        fail(f"{label}-missing")
    if not stat.S_ISREG(info.st_mode) or info.st_uid != 0 or info.st_nlink != 1:
        fail(f"{label}-file-invalid")
    if info.st_mode & 0o022 or info.st_size > max_size:
        fail(f"{label}-file-untrusted")
    try:
        return path.read_bytes()
    except OSError:
        fail(f"{label}-unreadable")


def trusted_file_size(path: Path, label: str, max_size: int) -> int:
    check_trusted_path_components(path.parent, label)
    try:
        info = os.lstat(path)
    except OSError:
        fail(f"{label}-missing")
    if not stat.S_ISREG(info.st_mode) or info.st_uid != 0 or info.st_nlink != 1:
        fail(f"{label}-file-invalid")
    if info.st_mode & 0o022 or info.st_size > max_size:
        fail(f"{label}-file-untrusted")
    return info.st_size


def hash_trusted_file(path: Path, label: str, max_size: int, whole_digest: Any | None = None) -> tuple[str, int]:
    expected_size = trusted_file_size(path, label, max_size)
    digest = hashlib.sha256()
    size = 0
    try:
        with path.open("rb") as stream:
            while True:
                block = stream.read(1024 * 1024)
                if not block:
                    break
                size += len(block)
                digest.update(block)
                if whole_digest is not None:
                    whole_digest.update(block)
    except OSError:
        fail(f"{label}-unreadable")
    if size != expected_size:
        fail(f"{label}-size-changed")
    return digest.hexdigest(), size


def private_directory(path: Path, label: str, create: bool = False) -> None:
    parent = path.parent
    trusted_directory(parent, f"{label}-parent")
    if path.exists() or path.is_symlink():
        trusted_directory(path, label)
        return
    if not create:
        fail(f"{label}-missing")
    try:
        path.mkdir(mode=0o700)
        os.chmod(path, 0o700)
    except OSError:
        fail(f"{label}-create-failed")


def check_checkout(commit: str) -> Path:
    helper = Path(__file__).resolve()
    repo = helper.parents[3]
    env = os.environ.copy()
    for name in ("GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR"):
        env.pop(name, None)
    env["GIT_OPTIONAL_LOCKS"] = "0"

    def git(*args: str) -> str:
        result = subprocess.run(
            ["git", "-c", f"safe.directory={repo}", "-C", str(repo), *args],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            check=False,
            env=env,
        )
        if result.returncode != 0:
            fail("source-checkout-git-check-failed")
        return result.stdout.decode("utf-8", errors="strict").rstrip("\n")

    if git("rev-parse", "--show-toplevel") != str(repo):
        fail("source-checkout-root-mismatch")
    if git("rev-parse", "--verify", "HEAD^{commit}") != commit:
        fail("source-checkout-commit-mismatch")
    if git("status", "--porcelain=v1", "--untracked-files=all"):
        fail("source-checkout-not-clean")
    return repo


def load_v3_manifest(path: Path, commit: str) -> list[dict[str, str]]:
    script = r'''
source "$1"
release_image_manifest_load "$2" "$3" || exit 10
[ "$RELEASE_IMAGE_MANIFEST_LOADED_FORMAT" = "$RELEASE_IMAGE_MANIFEST_V3_FORMAT" ] || exit 11
printf 'format\t%s\n' "$RELEASE_IMAGE_MANIFEST_LOADED_FORMAT"
for service in "${RELEASE_IMAGE_SERVICES_V3[@]}"; do
  printf '%s\t%s\t%s\t%s\n' "$service" \
    "${RELEASE_IMAGE_MANIFEST_REFS[$service]}" \
    "${RELEASE_IMAGE_MANIFEST_IDS[$service]}" \
    "${RELEASE_IMAGE_MANIFEST_REVISIONS[$service]}"
done
'''
    library = Path(__file__).with_name("release-image-manifest.sh")
    result = subprocess.run(
        ["bash", "-c", script, "release-delivery-bundle", str(library), str(path), commit],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        check=False,
        env={**os.environ, "LC_ALL": "C"},
    )
    if result.returncode != 0:
        fail("release-manifest-invalid")
    lines = result.stdout.decode("ascii", errors="strict").splitlines()
    if not lines or lines[0] != "format\tLAGRANGE_RELEASE_MANIFEST_V3":
        fail("release-manifest-not-v3")
    records: list[dict[str, str]] = []
    for line in lines[1:]:
        fields = line.split("\t")
        if len(fields) != 4:
            fail("release-manifest-record-invalid")
        records.append({"service": fields[0], "ref": fields[1], "image_id": fields[2], "revision": fields[3]})
    if tuple(record["service"] for record in records) != V3_SERVICES:
        fail("release-manifest-service-set-invalid")
    return records


def validate_d8(artifact_path: Path, complete_path: Path, commit: str) -> tuple[bytes, bytes]:
    artifact_raw = trusted_file(artifact_path, "d8-artifact", MAX_JSON_BYTES)
    complete_raw = trusted_file(complete_path, "d8-complete", 128)
    validator_path = Path(__file__).with_name("owner-beta-simulation-release.py")
    try:
        namespace = runpy.run_path(str(validator_path))
        validator = namespace["validate_build_receipt"]
        validator(artifact_raw, complete_raw, commit)
    except Exception:
        # The shared validator intentionally exposes no provider or free-form
        # build detail at this transport boundary.
        fail("d8-artifact-contract-invalid")
    return artifact_raw, complete_raw


def run_docker(args: list[str], label: str) -> bytes:
    try:
        result = subprocess.run(
            ["docker", *args],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            check=False,
        )
    except OSError:
        fail("docker-unavailable")
    if result.returncode != 0:
        fail(label)
    return result.stdout


def inspect_image(ref: str, require_repo_digest: str | None = None) -> dict[str, str]:
    template = "{{.Id}}|{{.Os}}|{{.Architecture}}|{{index .Config.Labels \"org.opencontainers.image.revision\"}}"
    if require_repo_digest is not None:
        template += "|{{json .RepoDigests}}"
    raw = run_docker(["image", "inspect", "--format", template, ref], "docker-image-inspect-failed")
    try:
        fields = raw.decode("utf-8", errors="strict").rstrip("\n").split("|", 4)
    except UnicodeDecodeError:
        fail("docker-image-inspect-response-invalid")
    expected_fields = 5 if require_repo_digest is not None else 4
    if len(fields) != expected_fields:
        fail("docker-image-inspect-response-invalid")
    image_id, os_name, architecture, revision = fields[:4]
    if not IMAGE_ID_RE.fullmatch(image_id) or (os_name, architecture) != ("linux", "amd64"):
        fail("docker-image-platform-or-id-invalid")
    result = {"image_id": image_id, "revision": revision, "platform": PLATFORM}
    if require_repo_digest is not None:
        try:
            repo_digests = json.loads(fields[4])
        except json.JSONDecodeError:
            fail("docker-image-repodigests-invalid")
        if not isinstance(repo_digests, list) or require_repo_digest not in repo_digests:
            fail("docker-image-pinned-repodigest-missing")
    return result


def dependency_identities() -> list[dict[str, str]]:
    rows: list[dict[str, str]] = []
    for dependency in DEPENDENCIES:
        inspected = inspect_image(dependency["source_ref"], dependency["source_ref"])
        rows.append(
            {
                "name": dependency["name"],
                "source_ref": dependency["source_ref"],
                "archive_ref": dependency["archive_ref"],
                "image_id": inspected["image_id"],
                "platform": PLATFORM,
            }
        )
    return rows


def expected_archive_images(image_rows: list[dict[str, Any]], dependency_rows: list[dict[str, Any]]) -> dict[str, dict[str, Any]]:
    result = {
        row["ref"]: {"image_id": row["image_id"], "revision": row["revision"]}
        for row in image_rows
    }
    for row in dependency_rows:
        result[row["archive_ref"]] = {"image_id": row["image_id"], "revision": None}
    return result


def safe_tar_name(name: str) -> str:
    if not name or "\\" in name or "\x00" in name or "\n" in name or "\r" in name or name.startswith("/"):
        fail("docker-archive-path-invalid")
    normalized = name.rstrip("/")
    parts = normalized.split("/")
    if not normalized or any(part in ("", ".", "..") for part in parts):
        fail("docker-archive-path-invalid")
    return normalized


def _json(raw: bytes, label: str) -> Any:
    try:
        return json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        fail(f"{label}-json-invalid")


def _archive_records(stream: BinaryIO) -> tuple[dict[str, bytes], dict[str, tuple[int, str]], set[str]]:
    """Read archive metadata in any order; hash every OCI blob while streaming."""
    try:
        tf = tarfile.open(fileobj=stream, mode="r|*", errorlevel=2)
    except (tarfile.TarError, OSError):
        fail("docker-archive-open-failed")
    data: dict[str, bytes] = {}
    files: dict[str, tuple[int, str]] = {}
    directories: set[str] = set()
    total = 0
    try:
        for member in tf:
            name = safe_tar_name(member.name)
            if name in files or name in directories or member.size < 0:
                fail("docker-archive-duplicate-or-invalid-entry")
            if member.isdir():
                directories.add(name)
                continue
            if not member.isreg():
                fail("docker-archive-entry-type-invalid")
            total += member.size
            if total > MAX_TRANSPORT_BYTES:
                fail("docker-archive-too-large")
            source = tf.extractfile(member)
            if source is None:
                fail("docker-archive-entry-unreadable")
            digest = hashlib.sha256()
            chunks: list[bytes] = []
            size = 0
            save_bytes = (
                name in {"index.json", "manifest.json", "oci-layout", "repositories"}
                or (name.startswith("blobs/sha256/") and member.size <= MAX_JSON_BYTES)
                or (name.endswith(".json") and member.size <= MAX_JSON_BYTES)
            )
            while True:
                block = source.read(1024 * 1024)
                if not block:
                    break
                size += len(block)
                digest.update(block)
                if save_bytes:
                    if size > MAX_JSON_BYTES:
                        fail("docker-archive-json-too-large")
                    chunks.append(block)
            if size != member.size:
                fail("docker-archive-entry-size-mismatch")
            files[name] = (size, digest.hexdigest())
            if save_bytes:
                data[name] = b"".join(chunks)
    except (tarfile.TarError, OSError, EOFError):
        fail("docker-archive-read-failed")
    finally:
        tf.close()
    return data, files, directories


def _normalize_archive_ref(value: str) -> str:
    if value.startswith("docker.io/library/"):
        return value[len("docker.io/library/"):]
    if value.startswith("docker.io/"):
        return value[len("docker.io/"):]
    return value


def _validate_config(raw: bytes, expected_revision: str | None) -> str:
    config = _json(raw, "docker-archive-config")
    if not isinstance(config, dict):
        fail("docker-archive-config-invalid")
    if config.get("os") != "linux" or config.get("architecture") != "amd64":
        fail("docker-archive-platform-mismatch")
    docker_config = config.get("config") or {}
    if not isinstance(docker_config, dict):
        fail("docker-archive-config-invalid")
    labels = docker_config.get("Labels") or {}
    if not isinstance(labels, dict):
        fail("docker-archive-labels-invalid")
    revision = labels.get(IMAGE_LABEL, "")
    if expected_revision is not None and revision != expected_revision:
        fail("docker-archive-revision-mismatch")
    return "sha256:" + sha256_bytes(raw)


def _oci_descriptor_path(descriptor: Any, label: str) -> tuple[str, int]:
    if not isinstance(descriptor, dict):
        fail(f"{label}-descriptor-invalid")
    digest = descriptor.get("digest")
    size = descriptor.get("size")
    if not isinstance(digest, str) or not IMAGE_ID_RE.fullmatch(digest):
        fail(f"{label}-digest-invalid")
    if type(size) is not int or size <= 0:
        fail(f"{label}-size-invalid")
    return "blobs/sha256/" + digest[7:], size


def _oci_descriptor_file(
    descriptor: Any,
    label: str,
    files: dict[str, tuple[int, str]],
) -> str:
    path, expected_size = _oci_descriptor_path(descriptor, label)
    digest = path.rsplit("/", 1)[-1]
    record = files.get(path)
    if record != (expected_size, digest):
        fail(f"{label}-blob-invalid")
    return path


def _oci_descriptor_bytes(
    descriptor: Any,
    label: str,
    data: dict[str, bytes],
    files: dict[str, tuple[int, str]],
) -> tuple[str, bytes]:
    path = _oci_descriptor_file(descriptor, label, files)
    raw = data.get(path)
    if raw is None:
        fail(f"{label}-blob-unreadable")
    return path, raw


def _validate_oci_platform_manifest(
    raw: bytes,
    expected_media_type: str,
    config_path: str,
    compatibility_layers: list[str],
    data: dict[str, bytes],
    files: dict[str, tuple[int, str]],
) -> None:
    image_manifest = _json(raw, "docker-archive-oci-platform-manifest")
    if not isinstance(image_manifest, dict) or image_manifest.get("schemaVersion") != 2:
        fail("docker-archive-oci-platform-manifest-invalid")
    if image_manifest.get("mediaType") not in OCI_MANIFEST_MEDIA_TYPES or image_manifest.get("mediaType") != expected_media_type:
        fail("docker-archive-oci-platform-manifest-invalid")

    config_descriptor = image_manifest.get("config")
    selected_config_path = _oci_descriptor_file(config_descriptor, "docker-archive-oci-config", files)
    if selected_config_path not in data:
        fail("docker-archive-oci-config-blob-unreadable")
    if selected_config_path != config_path:
        fail("docker-archive-oci-config-crosswired")

    descriptors = image_manifest.get("layers")
    if not isinstance(descriptors, list):
        fail("docker-archive-oci-layer-descriptors-invalid")
    selected_layers: list[str] = []
    for descriptor in descriptors:
        selected_layers.append(_oci_descriptor_file(descriptor, "docker-archive-oci-layer", files))
    if selected_layers != compatibility_layers:
        fail("docker-archive-oci-layers-crosswired")


def _validate_oci_selected_image(
    root_descriptor: dict[str, Any],
    config_path: str,
    compatibility_layers: list[str],
    data: dict[str, bytes],
    files: dict[str, tuple[int, str]],
) -> None:
    _, image_index_raw = _oci_descriptor_bytes(root_descriptor, "docker-archive-oci-image", data, files)
    image_index = _json(image_index_raw, "docker-archive-oci-image-index")
    if not isinstance(image_index, dict) or image_index.get("schemaVersion") != 2:
        fail("docker-archive-oci-image-index-invalid")
    root_media_type = image_index.get("mediaType")
    if root_media_type in OCI_MANIFEST_MEDIA_TYPES:
        if root_descriptor.get("mediaType") != root_media_type:
            fail("docker-archive-oci-image-media-type-mismatch")
        _validate_oci_platform_manifest(image_index_raw, root_media_type, config_path, compatibility_layers, data, files)
        return
    if root_media_type not in OCI_INDEX_MEDIA_TYPES:
        fail("docker-archive-oci-image-index-invalid")
    if root_descriptor.get("mediaType") != root_media_type:
        fail("docker-archive-oci-image-media-type-mismatch")
    manifests = image_index.get("manifests")
    if not isinstance(manifests, list):
        fail("docker-archive-oci-image-index-invalid")
    selected: list[dict[str, Any]] = []
    for descriptor in manifests:
        platform = descriptor.get("platform") if isinstance(descriptor, dict) else None
        if not isinstance(platform, dict):
            continue
        if platform.get("os") == "linux" and platform.get("architecture") == "amd64":
            selected.append(descriptor)
    if len(selected) != 1:
        fail("docker-archive-oci-platform-descriptor-invalid")

    descriptor_media_type = selected[0].get("mediaType")
    if descriptor_media_type not in OCI_MANIFEST_MEDIA_TYPES:
        fail("docker-archive-oci-platform-descriptor-invalid")
    _, manifest_raw = _oci_descriptor_bytes(selected[0], "docker-archive-oci-platform-manifest", data, files)
    _validate_oci_platform_manifest(manifest_raw, descriptor_media_type, config_path, compatibility_layers, data, files)


def validate_archive_stream(stream: BinaryIO, expected: dict[str, dict[str, Any]]) -> None:
    """Check the exact saved tag set and its platform/revision metadata."""
    data, files, directories = _archive_records(stream)
    if "manifest.json" not in data:
        fail("docker-archive-manifest-missing")
    entries = _json(data["manifest.json"], "docker-archive-manifest")
    if not isinstance(entries, list):
        fail("docker-archive-manifest-invalid")
    oci = "oci-layout" in data or "index.json" in data
    tag_config: dict[str, str] = {}
    tag_layers: dict[str, list[str]] = {}
    expected_files = {"manifest.json"}
    for entry in entries:
        if not isinstance(entry, dict) or set(entry) != {"Config", "RepoTags", "Layers"}:
            fail("docker-archive-image-record-invalid")
        config_path = safe_tar_name(entry["Config"])
        if not DOCKER_CONFIG_RE.fullmatch(config_path):
            fail("docker-archive-config-path-invalid")
        tags = entry["RepoTags"]
        layers = entry["Layers"]
        if not isinstance(tags, list) or not tags or not isinstance(layers, list) or not layers:
            fail("docker-archive-manifest-record-invalid")
        for ref in tags:
            if not isinstance(ref, str) or ref not in expected or ref in tag_config:
                fail("docker-archive-tag-set-invalid")
            tag_config[ref] = config_path
            tag_layers[ref] = list(layers)
        if config_path not in files:
            fail("docker-archive-config-missing")
        for layer in layers:
            layer_path = safe_tar_name(layer) if isinstance(layer, str) else ""
            if oci:
                if not re.fullmatch(r"blobs/sha256/[0-9a-f]{64}", layer_path) or layer_path not in files:
                    fail("docker-archive-layer-missing")
            elif not layer_path.endswith("/layer.tar") or layer_path not in files:
                fail("docker-archive-layer-missing")
        expected_files.add(config_path)
        for layer in layers:
            expected_files.add(layer)
            if not oci:
                layer_dir = layer.rsplit("/", 1)[0]
                expected_files.update((f"{layer_dir}/VERSION", f"{layer_dir}/json"))
    if set(tag_config) != set(expected):
        fail("docker-archive-tag-set-invalid")
    for ref, config_path in tag_config.items():
        config_raw = data.get(config_path)
        if config_raw is None:
            fail("docker-archive-config-missing")
        config_id = _validate_config(config_raw, expected[ref]["revision"])
        expected_config_path = "blobs/sha256/" + config_id[7:] if oci else config_id[7:] + ".json"
        if config_path != expected_config_path:
            fail("docker-archive-config-digest-mismatch")
    if "repositories" in data:
        expected_files.add("repositories")

    if oci:
        if set(("oci-layout", "index.json")) - set(data):
            fail("docker-archive-oci-layout-incomplete")
        layout = _json(data["oci-layout"], "docker-archive-oci-layout")
        if not isinstance(layout, dict) or layout.get("imageLayoutVersion") != "1.0.0":
            fail("docker-archive-oci-layout-invalid")
        root = _json(data["index.json"], "docker-archive-index")
        if not isinstance(root, dict) or root.get("schemaVersion") != 2 or not isinstance(root.get("manifests"), list):
            fail("docker-archive-index-invalid")
        index_refs: set[str] = set()
        index_descriptors: dict[str, dict[str, Any]] = {}
        for descriptor in root["manifests"]:
            annotations = descriptor.get("annotations", {}) if isinstance(descriptor, dict) else {}
            if not isinstance(annotations, dict):
                fail("docker-archive-reference-annotation-invalid")
            canonical_ref = annotations.get("io.containerd.image.name")
            if not isinstance(canonical_ref, str):
                fail("docker-archive-reference-annotation-missing")
            ref = _normalize_archive_ref(canonical_ref)
            if ref not in expected or ref in index_refs:
                fail("docker-archive-oci-reference-set-invalid")
            digest = descriptor.get("digest")
            if not isinstance(digest, str) or not IMAGE_ID_RE.fullmatch(digest):
                fail("docker-archive-oci-image-id-invalid")
            if digest != expected[ref]["image_id"]:
                fail("docker-archive-oci-image-id-mismatch")
            size = descriptor.get("size")
            if type(size) is not int or size <= 0:
                fail("docker-archive-oci-image-size-invalid")
            root_blob = files.get("blobs/sha256/" + digest[7:])
            if root_blob is None or root_blob != (size, digest[7:]):
                fail("docker-archive-oci-image-blob-invalid")
            index_refs.add(ref)
            index_descriptors[ref] = descriptor
        if index_refs != set(expected):
            fail("docker-archive-oci-reference-set-invalid")
        for ref, config_path in tag_config.items():
            _validate_oci_selected_image(
                index_descriptors[ref],
                config_path,
                tag_layers[ref],
                data,
                files,
            )
        blob_files = {name for name in files if name.startswith("blobs/sha256/")}
        if not blob_files or any(not re.fullmatch(r"blobs/sha256/[0-9a-f]{64}", name) for name in blob_files):
            fail("docker-archive-oci-blob-set-invalid")
        expected_files.update(("oci-layout", "index.json", *blob_files))
    else:
        for ref, config_path in tag_config.items():
            if expected[ref]["image_id"] != "sha256:" + sha256_bytes(data[config_path]):
                fail("docker-archive-legacy-image-id-mismatch")
    if "repositories" in data:
        repositories = _json(data["repositories"], "docker-archive-repositories")
        expected_repositories: dict[str, dict[str, str]] = {}
        for ref, config_path in tag_config.items():
            repository, tag = ref.rsplit(":", 1)
            config_id = config_path.rsplit("/", 1)[-1]
            config_id = config_id[:-5] if config_id.endswith(".json") else config_id
            expected_repositories.setdefault(repository, {})[tag] = config_id
        if repositories != expected_repositories:
            fail("docker-archive-repositories-mismatch")
    if set(files) != expected_files:
        fail("docker-archive-unexpected-file")
    allowed_dirs = {
        "/".join(parts[:index])
        for name in expected_files
        for parts in [name.split("/")]
        for index in range(1, len(parts))
    }
    if directories - allowed_dirs:
        fail("docker-archive-unexpected-directory")


def validate_archive_file(path: Path, expected: dict[str, dict[str, str]]) -> None:
    try:
        with path.open("rb") as stream:
            validate_archive_stream(stream, expected)
    except OSError:
        fail("docker-archive-unreadable")


def split_archive(source: BinaryIO, directory: Path) -> tuple[list[dict[str, Any]], str, int]:
    chunks: list[dict[str, Any]] = []
    whole = hashlib.sha256()
    total = 0
    index = 0
    try:
        while True:
            name = f"{CHUNK_PREFIX}{index:06d}"
            target = directory / name
            fd = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            chunk_hash = hashlib.sha256()
            chunk_bytes = 0
            with os.fdopen(fd, "wb") as output:
                while chunk_bytes < MAX_ASSET_BYTES:
                    block = source.read(min(1024 * 1024, MAX_ASSET_BYTES - chunk_bytes))
                    if not block:
                        break
                    output.write(block)
                    chunk_hash.update(block)
                    whole.update(block)
                    chunk_bytes += len(block)
                    total += len(block)
                output.flush()
                os.fsync(output.fileno())
            if not chunk_bytes:
                target.unlink()
                break
            chunks.append({"name": name, "bytes": chunk_bytes, "sha256": chunk_hash.hexdigest()})
            index += 1
    except OSError:
        fail("archive-chunking-failed")
    if not chunks or any(chunk["bytes"] >= 2_000_000_000 for chunk in chunks):
        fail("archive-chunk-size-invalid")
    return chunks, whole.hexdigest(), total


def verify_chunk_assets(bundle_dir: Path, metadata: dict[str, Any]) -> list[Path]:
    chunks = metadata["chunks"]
    if not isinstance(chunks, list) or not chunks:
        fail("bundle-chunks-invalid")
    paths = []
    total = 0
    digest = hashlib.sha256()
    for index, item in enumerate(chunks):
        if not isinstance(item, dict) or set(item) != {"name", "bytes", "sha256"}:
            fail("bundle-chunk-record-invalid")
        name = f"{CHUNK_PREFIX}{index:06d}"
        if item["name"] != name or type(item["bytes"]) is not int or item["bytes"] <= 0 or item["bytes"] > MAX_ASSET_BYTES:
            fail("bundle-chunk-name-or-size-invalid")
        if not isinstance(item["sha256"], str) or not HASH_RE.fullmatch(item["sha256"]):
            fail("bundle-chunk-hash-invalid")
        path = bundle_dir / name
        actual_hash, actual_size = hash_trusted_file(path, "bundle-chunk", MAX_ASSET_BYTES, digest)
        if actual_size != item["bytes"] or actual_hash != item["sha256"]:
            fail("bundle-chunk-digest-mismatch")
        total += actual_size
        if actual_size >= 2_000_000_000:
            fail("bundle-chunk-too-large")
        paths.append(path)
    if total != metadata["transport_bytes"] or digest.hexdigest() != metadata["transport_sha256"]:
        fail("bundle-transport-digest-mismatch")
    return paths


def read_bundle_assets(bundle_dir: Path, commit: str) -> tuple[dict[str, Any], bytes, bytes, bytes, list[dict[str, str]], list[dict[str, str]], list[Path]]:
    trusted_directory(bundle_dir, "bundle")
    metadata_raw = trusted_file(bundle_dir / BUNDLE_MANIFEST, "bundle-metadata", MAX_JSON_BYTES)
    try:
        metadata = json.loads(metadata_raw.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        fail("bundle-metadata-json-invalid")
    metadata_keys = {
        "chunks",
        "d8_artifact_sha256",
        "d8_complete_sha256",
        "dependencies",
        "format",
        "images",
        "platform",
        "release_manifest_sha256",
        "source_commit",
        "transport_bytes",
        "transport_format",
        "transport_sha256",
    }
    if not isinstance(metadata, dict) or set(metadata) != metadata_keys or canonical_json(metadata) != metadata_raw:
        fail("bundle-metadata-contract-invalid")
    if metadata.get("format") != BUNDLE_FORMAT or metadata.get("transport_format") != TRANSPORT_FORMAT:
        fail("bundle-format-unsupported")
    if metadata.get("source_commit") != commit or metadata.get("platform") != PLATFORM:
        fail("bundle-source-or-platform-mismatch")
    if type(metadata.get("transport_bytes")) is not int or metadata["transport_bytes"] <= 0 or metadata["transport_bytes"] > MAX_TRANSPORT_BYTES:
        fail("bundle-transport-size-invalid")
    for field in ("release_manifest_sha256", "d8_artifact_sha256", "d8_complete_sha256", "transport_sha256"):
        if not isinstance(metadata.get(field), str) or not HASH_RE.fullmatch(metadata[field]):
            fail("bundle-hash-field-invalid")

    manifest_raw = trusted_file(bundle_dir / RELEASE_MANIFEST, "release-manifest", 64 * 1024)
    d8_artifact_raw = trusted_file(bundle_dir / D8_ARTIFACT_ASSET, "d8-artifact-asset", MAX_JSON_BYTES)
    d8_complete_raw = trusted_file(bundle_dir / D8_COMPLETE_ASSET, "d8-complete-asset", 128)
    if sha256_bytes(manifest_raw) != metadata["release_manifest_sha256"]:
        fail("bundle-release-manifest-hash-mismatch")
    if sha256_bytes(d8_artifact_raw) != metadata["d8_artifact_sha256"]:
        fail("bundle-d8-artifact-hash-mismatch")
    if sha256_bytes(d8_complete_raw) != metadata["d8_complete_sha256"]:
        fail("bundle-d8-complete-hash-mismatch")

    # The archive manifest is validated only after all small assets and hashes.
    manifest_path = bundle_dir / RELEASE_MANIFEST
    image_rows = load_v3_manifest(manifest_path, commit)
    d8_path = bundle_dir / D8_ARTIFACT_ASSET
    complete_path = bundle_dir / D8_COMPLETE_ASSET
    validate_d8(d8_path, complete_path, commit)

    metadata_images = metadata.get("images")
    expected_images = [
        {"service": row["service"], "ref": row["ref"], "image_id": row["image_id"], "revision": row["revision"]}
        for row in image_rows
    ]
    if metadata_images != expected_images:
        fail("bundle-image-identity-records-mismatch")
    raw_dependencies = metadata.get("dependencies")
    if not isinstance(raw_dependencies, list) or len(raw_dependencies) != len(DEPENDENCIES):
        fail("bundle-dependency-contract-invalid")
    expected_dependencies = []
    for dependency, row in zip(DEPENDENCIES, raw_dependencies):
        if not isinstance(row, dict) or set(row) != {"name", "source_ref", "archive_ref", "image_id", "platform"}:
            fail("bundle-dependency-contract-invalid")
        expected_dependencies.append(
            {
                "name": dependency["name"],
                "source_ref": dependency["source_ref"],
                "archive_ref": dependency["archive_ref"],
                "image_id": row["image_id"],
                "platform": PLATFORM,
            }
        )
    if metadata["dependencies"] != expected_dependencies:
        fail("bundle-dependency-contract-invalid")
    dependency_rows = expected_dependencies
    if any(not isinstance(row.get("image_id"), str) or not IMAGE_ID_RE.fullmatch(row["image_id"]) for row in dependency_rows):
        fail("bundle-dependency-id-invalid")

    chunk_paths = verify_chunk_assets(bundle_dir, metadata)
    expected_assets = {
        BUNDLE_MANIFEST,
        RELEASE_MANIFEST,
        D8_ARTIFACT_ASSET,
        D8_COMPLETE_ASSET,
        *(path.name for path in chunk_paths),
    }
    actual_assets = set(os.listdir(bundle_dir))
    if actual_assets != expected_assets:
        fail("bundle-directory-entry-set-invalid")
    for name in actual_assets:
        path = bundle_dir / name
        try:
            info = os.lstat(path)
        except OSError:
            fail("bundle-directory-entry-invalid")
        if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1:
            fail("bundle-directory-entry-invalid")
    return metadata, metadata_raw, manifest_raw, d8_artifact_raw, image_rows, dependency_rows, chunk_paths


def verify_zstd(paths: list[Path]) -> None:
    cat = subprocess.Popen(["cat", *(str(path) for path in paths)], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    try:
        result = subprocess.run(["zstd", "-q", "-t", "-"], stdin=cat.stdout, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        if cat.stdout is not None:
            cat.stdout.close()
        cat_status = cat.wait()
    except OSError:
        cat.kill()
        cat.wait()
        fail("zstd-unavailable")
    if result.returncode != 0 or cat_status != 0:
        fail("bundle-compressed-archive-invalid")


def validate_zstd_archive(paths: list[Path], expected: dict[str, dict[str, str]]) -> None:
    cat = subprocess.Popen(["cat", *(str(path) for path in paths)], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    if cat.stdout is None:
        fail("transport-pipe-failed")
    try:
        decoder = subprocess.Popen(["zstd", "-q", "-d", "-c"], stdin=cat.stdout, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        cat.stdout.close()
    except OSError:
        cat.kill()
        cat.wait()
        fail("zstd-unavailable")
    if decoder.stdout is None:
        decoder.kill()
        cat.kill()
        fail("transport-pipe-failed")
    validation_error: BaseException | None = None
    try:
        validate_archive_stream(decoder.stdout, expected)
        while True:
            block = decoder.stdout.read(1024 * 1024)
            if not block:
                break
            if any(block):
                fail("docker-archive-trailing-data")
    except BaseException as exc:
        validation_error = exc
        decoder.kill()
        cat.kill()
    finally:
        decoder.stdout.close()
    decoder_status = decoder.wait()
    cat_status = cat.wait()
    if validation_error is not None:
        raise validation_error
    if decoder_status != 0 or cat_status != 0:
        fail("bundle-compressed-archive-invalid")


def ensure_dependency_aliases(rows: list[dict[str, str]]) -> None:
    for dependency, row in zip(DEPENDENCIES, rows):
        try:
            existing = inspect_image(dependency["archive_ref"])
        except BundleError as exc:
            if str(exc) != "docker-image-inspect-failed":
                raise
            existing = None
        if existing is not None:
            if existing["image_id"] != row["image_id"]:
                fail("dependency-alias-collision")
            continue
        run_docker(["image", "tag", row["image_id"], dependency["archive_ref"]], "dependency-alias-tag-failed")
        tagged = inspect_image(dependency["archive_ref"])
        if tagged["image_id"] != row["image_id"]:
            fail("dependency-alias-identity-mismatch")


def inspect_source_images(image_rows: list[dict[str, str]], dependency_rows: list[dict[str, str]], commit: str) -> None:
    for row in image_rows:
        inspected = inspect_image(row["ref"])
        if inspected["image_id"] != row["image_id"] or inspected["revision"] != commit:
            fail("source-image-identity-mismatch")
    for dependency, row in zip(DEPENDENCIES, dependency_rows):
        inspected = inspect_image(dependency["source_ref"], dependency["source_ref"])
        if inspected["image_id"] != row["image_id"]:
            fail("source-dependency-identity-mismatch")


def export_bundle(commit: str, build_dir_arg: str, bundle_dir_arg: str) -> None:
    commit = checked_commit(commit)
    check_checkout(commit)
    build_dir = path_arg(build_dir_arg, "build").absolute()
    bundle_dir = path_arg(bundle_dir_arg, "bundle").absolute()
    private_directory(build_dir, "build")
    if bundle_dir.exists() or bundle_dir.is_symlink():
        fail("bundle-output-already-exists")
    trusted_directory(bundle_dir.parent, "bundle-output-parent")

    manifest_path = build_dir / RELEASE_MANIFEST
    manifest_raw = trusted_file(manifest_path, "release-manifest", 64 * 1024)
    image_rows = load_v3_manifest(manifest_path, commit)
    d8_artifact_raw, d8_complete_raw = validate_d8(build_dir / "d8" / "artifact.json", build_dir / "d8" / "COMPLETE", commit)
    source_dependencies = dependency_identities()
    image_metadata = [
        {"service": row["service"], "ref": row["ref"], "image_id": row["image_id"], "revision": row["revision"]}
        for row in image_rows
    ]
    dependency_metadata = source_dependencies
    inspect_source_images(image_rows, dependency_metadata, commit)
    ensure_dependency_aliases(dependency_metadata)

    parent = bundle_dir.parent
    try:
        temporary_root = Path(tempfile.mkdtemp(prefix=f".{bundle_dir.name}.tmp.", dir=parent))
        os.chmod(temporary_root, 0o700)
    except OSError:
        fail("bundle-temporary-directory-create-failed")
    archive_path = temporary_root / "images.tar"
    try:
        refs = [row["ref"] for row in image_rows] + [row["archive_ref"] for row in dependency_metadata]
        run_docker(["image", "save", "--output", str(archive_path), *refs], "docker-image-save-failed")
        expected = expected_archive_images(image_metadata, dependency_metadata)
        validate_archive_file(archive_path, expected)
        try:
            with archive_path.open("rb") as archive_stream:
                compressor = subprocess.Popen(
                    ["zstd", "-q", "-T1", "-4", "-c"],
                    stdin=archive_stream,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.DEVNULL,
                )
        except OSError:
            fail("zstd-unavailable")
        if compressor.stdout is None:
            compressor.kill()
            compressor.wait()
            fail("archive-compression-failed")
        try:
            chunks, transport_hash, transport_bytes = split_archive(compressor.stdout, temporary_root)
        except BaseException:
            compressor.kill()
            compressor.stdout.close()
            compressor.wait()
            raise
        finally:
            compressor.stdout.close()
        if compressor.wait() != 0:
            fail("archive-compression-failed")
        os.unlink(archive_path)
        metadata = {
            "chunks": chunks,
            "d8_artifact_sha256": sha256_bytes(d8_artifact_raw),
            "d8_complete_sha256": sha256_bytes(d8_complete_raw),
            "dependencies": dependency_metadata,
            "format": BUNDLE_FORMAT,
            "images": image_metadata,
            "platform": PLATFORM,
            "release_manifest_sha256": sha256_bytes(manifest_raw),
            "source_commit": commit,
            "transport_bytes": transport_bytes,
            "transport_format": TRANSPORT_FORMAT,
            "transport_sha256": transport_hash,
        }
        create_file(temporary_root / RELEASE_MANIFEST, manifest_raw, 0o600)
        create_file(temporary_root / D8_ARTIFACT_ASSET, d8_artifact_raw, 0o600)
        create_file(temporary_root / D8_COMPLETE_ASSET, d8_complete_raw, 0o600)
        create_file(temporary_root / BUNDLE_MANIFEST, canonical_json(metadata), 0o600)
        os.chmod(temporary_root, 0o700)
        if bundle_dir.exists() or bundle_dir.is_symlink():
            fail("bundle-output-already-exists")
        os.rename(temporary_root, bundle_dir)
    except BaseException:
        shutil.rmtree(temporary_root, ignore_errors=True)
        raise


def compose_override(dependency_rows: list[dict[str, str]]) -> bytes:
    ids = {row["name"]: row["image_id"] for row in dependency_rows}
    postgres_id = ids["postgres"]
    nginx_id = ids["reverse-proxy"]
    alpine_id = ids["alpine"]
    text = (
        "# Generated by release-delivery-bundle import after image ID verification.\n"
        "# Source refs and upstream digests are recorded in release-bundle.json.\n"
        "services:\n"
        f"  postgres:\n    image: {postgres_id}\n"
        f"  research-schema-check:\n    image: {postgres_id}\n"
        f"  reverse-proxy:\n    image: {nginx_id}\n"
        f"  research-raw-init:\n    image: {alpine_id}\n"
    )
    return text.encode("ascii")


def expected_output_files(manifest: bytes, bundle_metadata: bytes, artifact: bytes, complete: bytes, override: bytes) -> dict[str, bytes]:
    return {
        RELEASE_MANIFEST: manifest,
        BUNDLE_MANIFEST: bundle_metadata,
        COMPOSE_OVERRIDE: override,
        "d8/artifact.json": artifact,
        "d8/COMPLETE": complete,
    }


def output_matches(output_dir: Path, files: dict[str, bytes]) -> bool:
    expected_top = {"release.manifest", "release-bundle.json", COMPOSE_OVERRIDE, "d8"}
    try:
        if set(os.listdir(output_dir)) != expected_top:
            return False
        d8_dir = output_dir / "d8"
        d8_info = os.lstat(d8_dir)
        if not stat.S_ISDIR(d8_info.st_mode) or d8_info.st_uid != 0 or d8_info.st_mode & 0o077:
            return False
        if set(os.listdir(d8_dir)) != {"artifact.json", "COMPLETE"}:
            return False
        for relative, expected in files.items():
            path = output_dir / relative
            info = os.lstat(path)
            expected_mode = 0o444 if relative in {"d8/artifact.json", "d8/COMPLETE"} else 0o600
            if not stat.S_ISREG(info.st_mode) or info.st_uid != 0 or stat.S_IMODE(info.st_mode) != expected_mode or info.st_nlink != 1:
                return False
            if path.read_bytes() != expected:
                return False
        return True
    except OSError:
        return False


def materialize_output(output_dir: Path, files: dict[str, bytes]) -> None:
    private_directory(output_dir.parent, "import-output-parent")
    temporary = Path(tempfile.mkdtemp(prefix=f".{output_dir.name}.tmp.", dir=output_dir.parent))
    os.chmod(temporary, 0o700)
    try:
        (temporary / "d8").mkdir(mode=0o700)
        for relative, data in files.items():
            target = temporary / relative
            target.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
            mode = 0o444 if relative in {"d8/artifact.json", "d8/COMPLETE"} else 0o600
            fd = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL, mode)
            with os.fdopen(fd, "wb") as stream:
                stream.write(data)
                stream.flush()
                os.fsync(stream.fileno())
            os.chmod(target, mode)
        os.chmod(temporary / "d8", 0o700)
        if output_dir.exists() or output_dir.is_symlink():
            fail("import-output-collision")
        os.rename(temporary, output_dir)
    except BaseException:
        shutil.rmtree(temporary, ignore_errors=True)
        raise


def load_docker_archive(paths: list[Path]) -> None:
    cat: subprocess.Popen[bytes] | None = None
    decoder: subprocess.Popen[bytes] | None = None
    docker: subprocess.Popen[bytes] | None = None
    try:
        cat = subprocess.Popen(["cat", *(str(path) for path in paths)], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        if cat.stdout is None:
            fail("transport-pipe-failed")
        decoder = subprocess.Popen(["zstd", "-q", "-d", "-c"], stdin=cat.stdout, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        cat.stdout.close()
        if decoder.stdout is None:
            fail("transport-pipe-failed")
        docker = subprocess.Popen(["docker", "image", "load"], stdin=decoder.stdout, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        decoder.stdout.close()
    except OSError:
        for process in (docker, decoder, cat):
            if process is not None:
                if process.poll() is None:
                    process.kill()
                process.wait()
        fail("docker-or-zstd-unavailable")
    except BaseException:
        for process in (docker, decoder, cat):
            if process is not None:
                if process.poll() is None:
                    process.kill()
                process.wait()
        raise
    if docker is None or decoder is None or cat is None:
        fail("transport-pipe-failed")
    docker_status = docker.wait()
    decoder_status = decoder.wait()
    cat_status = cat.wait()
    if docker_status != 0 or decoder_status != 0 or cat_status != 0:
        fail("docker-image-load-failed")


def verify_loaded_images(image_rows: list[dict[str, str]], dependency_rows: list[dict[str, str]], commit: str) -> None:
    for row in image_rows:
        inspected = inspect_image(row["ref"])
        if inspected["image_id"] != row["image_id"] or inspected["revision"] != commit:
            fail("loaded-product-image-identity-mismatch")
    for dependency, row in zip(DEPENDENCIES, dependency_rows):
        inspected = inspect_image(dependency["archive_ref"])
        if inspected["image_id"] != row["image_id"]:
            fail("loaded-dependency-image-identity-mismatch")


def import_bundle(commit: str, bundle_dir_arg: str, output_dir_arg: str) -> None:
    commit = checked_commit(commit)
    check_checkout(commit)
    bundle_dir = path_arg(bundle_dir_arg, "bundle").absolute()
    output_dir = path_arg(output_dir_arg, "output").absolute()
    private_directory(bundle_dir, "bundle")
    if output_dir.exists() or output_dir.is_symlink():
        private_directory(output_dir, "import-output")
    else:
        trusted_directory(output_dir.parent, "import-output-parent")

    metadata, metadata_raw, manifest_raw, artifact_raw, image_rows, dependency_rows, chunks = read_bundle_assets(bundle_dir, commit)
    verify_zstd(chunks)
    expected = expected_archive_images(image_rows, dependency_rows)
    validate_zstd_archive(chunks, expected)
    complete_raw = trusted_file(bundle_dir / D8_COMPLETE_ASSET, "d8-complete-asset", 128)
    override = compose_override(dependency_rows)
    files = expected_output_files(manifest_raw, metadata_raw, artifact_raw, complete_raw, override)

    if output_dir.exists():
        if not output_matches(output_dir, files):
            fail("import-output-conflict")
        try:
            verify_loaded_images(image_rows, dependency_rows, commit)
        except BundleError:
            load_docker_archive(chunks)
            verify_loaded_images(image_rows, dependency_rows, commit)
        return

    # Every asset, identity, platform and receipt has passed before Docker can
    # mutate its image store. Loading does not start services.
    load_docker_archive(chunks)
    verify_loaded_images(image_rows, dependency_rows, commit)
    materialize_output(output_dir, files)


def usage() -> str:
    return (
        "Usage:\n"
        "  scripts/ops/release-delivery-bundle.sh export COMMIT BUILD_DIR BUNDLE_DIR\n"
        "  scripts/ops/release-delivery-bundle.sh import COMMIT BUNDLE_DIR OUTPUT_DIR\n"
        "The bundle contains a V3 manifest, allowlisted D8 receipts, and only the\n"
        "13 release images plus fixed pinned offline dependencies. Import requires\n"
        "upstream authentication of every exact release asset digest."
    )


def main(argv: list[str]) -> int:
    if len(argv) == 1 and argv[0] in ("-h", "--help"):
        print(usage())
        return 0
    if os.geteuid() != 0:
        fail("must-run-as-root")
    if not argv or argv[0] not in ("export", "import"):
        fail("usage-invalid")
    if argv[0] == "export":
        if len(argv) != 4:
            fail("usage-invalid")
        export_bundle(argv[1], argv[2], argv[3])
    else:
        if len(argv) != 4:
            fail("usage-invalid")
        import_bundle(argv[1], argv[2], argv[3])
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main(sys.argv[1:]))
    except BundleError as exc:
        print(f"release-delivery-bundle: {exc}", file=sys.stderr)
        raise SystemExit(1)
