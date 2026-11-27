from __future__ import annotations

import hashlib
import importlib.util
import io
import json
import subprocess
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
HELPER_PATH = ROOT / "scripts/ops/lib/release-delivery-bundle.py"
COMMIT = "c" * 40
SPEC = importlib.util.spec_from_file_location("release_delivery_bundle", HELPER_PATH)
BUNDLE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(BUNDLE)


def sha(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def json_bytes(value: object) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True) + "\n").encode("ascii")


def canonical_ref(ref: str) -> str:
    repository, tag = ref.rsplit(":", 1)
    prefix = "docker.io/library/" if "/" not in repository else "docker.io/"
    return f"{prefix}{ref}"


def make_oci_image(ref: str, revision: str | None, platform: str = "linux/amd64") -> tuple[str, dict[str, bytes], dict[str, object]]:
    os_name, architecture = platform.split("/", 1)
    labels = {"fixture.ref": ref}
    if revision:
        labels[BUNDLE.IMAGE_LABEL] = revision
    config = {
        "architecture": architecture,
        "os": os_name,
        "config": {"Labels": labels},
        "rootfs": {"type": "layers", "diff_ids": ["sha256:" + "d" * 64]},
    }
    config_raw = json_bytes(config)
    config_hash = sha(config_raw)
    layer_raw = ("fixture layer " + ref).encode("ascii")
    layer_hash = sha(layer_raw)
    manifest = {
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "config": {
            "mediaType": "application/vnd.oci.image.config.v1+json",
            "digest": "sha256:" + config_hash,
            "size": len(config_raw),
        },
        "layers": [{
            "mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
            "digest": "sha256:" + layer_hash,
            "size": len(layer_raw),
        }],
    }
    manifest_raw = json_bytes(manifest)
    manifest_hash = sha(manifest_raw)
    image_index = {
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.index.v1+json",
        "manifests": [{
            "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "digest": "sha256:" + manifest_hash,
            "size": len(manifest_raw),
            "platform": {"os": os_name, "architecture": architecture},
        }],
    }
    image_index_raw = json_bytes(image_index)
    image_id = "sha256:" + sha(image_index_raw)
    index = {
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.index.v1+json",
        "manifests": [{
            "mediaType": "application/vnd.oci.image.index.v1+json",
            "digest": image_id,
            "size": len(image_index_raw),
            "annotations": {"io.containerd.image.name": canonical_ref(ref)},
        }],
    }
    compat = [{
        "Config": "blobs/sha256/" + config_hash,
        "RepoTags": [ref],
        "Layers": ["blobs/sha256/" + layer_hash],
    }]
    files = {
        "blobs/sha256/" + sha(image_index_raw): image_index_raw,
        "blobs/sha256/" + manifest_hash: manifest_raw,
        "blobs/sha256/" + config_hash: config_raw,
        "blobs/sha256/" + layer_hash: layer_raw,
        "oci-layout": json_bytes({"imageLayoutVersion": "1.0.0"}).rstrip(b"\n"),
        "index.json": json_bytes(index),
        "manifest.json": json_bytes(compat),
    }
    return image_id, files, config


def tar_bytes(files: dict[str, bytes]) -> bytes:
    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode="w") as archive:
        # Deliberately put blobs and index before Docker's compatibility
        # manifest to exercise the order emitted by current Engine releases.
        for name in sorted(files, key=lambda item: (item == "manifest.json", item)):
            raw = files[name]
            entry = tarfile.TarInfo(name)
            entry.size = len(raw)
            entry.mode = 0o644
            archive.addfile(entry, io.BytesIO(raw))
    return output.getvalue()


def valid_d8() -> tuple[bytes, bytes]:
    input_hash = "a" * 64
    recipe_hash = "b" * 64
    artifact = {
        "binary_mode": "0755",
        "binary_sha256": "e" * 64,
        "bin": "owner-beta-simulation-runner",
        "cache_key": "d" * 64,
        "cargo_success": True,
        "compile_env": {
            "CARGO_BUILD_JOBS": "2",
            "CARGO_TARGET_DIR": "/cargo-target",
            "RUSTFLAGS": "<unset>",
            "LAGRANGE_CODE_COMMIT": COMMIT,
            "R4_SIMULATION_BUILD_GIT_COMMIT": COMMIT,
            "R4_SIMULATION_BUILD_TOOLCHAIN": "1.97.1",
            "R4_SIMULATION_BUILD_TARGET": "x86_64-unknown-linux-musl",
            "R4_SIMULATION_BUILD_PROFILE": "release",
            "R4_SIMULATION_BUILD_ORIGIN": "official-release-v1",
            "R4_SIMULATION_BUILD_SOURCE_INPUT_SHA256": input_hash,
            "R4_SIMULATION_BUILD_RECIPE_SHA256": recipe_hash,
        },
        "features": [],
        "format": "lagrange-rust-artifact-v1",
        "h_sha256": "f" * 64,
        "host_triple": "x86_64-unknown-linux-musl",
        "input_sha256": input_hash,
        "package": "job-queue",
        "platform": "linux/amd64",
        "profile": "release",
        "recipe_sha256": recipe_hash,
        "source_commit": COMMIT,
    }
    raw = json_bytes(artifact)
    return raw, (sha(raw) + "\n").encode("ascii")


class FakeDocker:
    def __init__(self) -> None:
        self.images: dict[str, dict[str, object]] = {}
        self.saved_images: dict[str, dict[str, object]] = {}
        self.calls: list[tuple[str, ...]] = []
        self.archive_writes: list[tuple[tuple[str, ...], bytes]] = []
        self.omit_combined_compat_refs: set[str] = set()
        self.combined_extra_tag: object | None = None
        self.combined_duplicate_tag: str | None = None
        self.corrupt_combined_index_ref: str | None = None
        self.omit_combined_oci_index = False
        self.bad_single_id_ref: str | None = None
        self.load_count = 0

    def add(self, ref: str, revision: str | None, repo_digests: list[str] | None = None) -> dict[str, object]:
        image_id, _, config = make_oci_image(ref, revision)
        record = {
            "image_id": image_id,
            "os": config["os"],
            "architecture": config["architecture"],
            "revision": revision or "",
            "repo_digests": repo_digests or [],
            "files": make_oci_image(ref, revision)[1],
        }
        self.images[ref] = record
        return record

    def run(self, args: list[str], _label: str) -> bytes:
        self.calls.append(tuple(args))
        if args[:2] == ["image", "inspect"]:
            ref = args[-1]
            record = self.images.get(ref)
            if record is None:
                raise BUNDLE.BundleError("docker-image-inspect-failed")
            template = args[3]
            fields = [str(record["image_id"]), str(record["os"]), str(record["architecture"]), str(record["revision"])]
            if "{{json .RepoDigests}}" in template:
                fields.append(json.dumps(record["repo_digests"], separators=(",", ":")))
            return ("|".join(fields) + "\n").encode("utf-8")
        if args[:2] == ["image", "tag"]:
            image_id, alias = args[2:]
            source = next((item for item in self.images.values() if item["image_id"] == image_id), None)
            if source is None:
                raise BUNDLE.BundleError("docker-image-tag-failed")
            old = self.images.get(alias)
            if old is not None and old["image_id"] != image_id:
                raise BUNDLE.BundleError("docker-image-tag-failed")
            self.images[alias] = dict(source)
            return b""
        if args[:3] == ["image", "save", "--output"]:
            archive_path = Path(args[3])
            refs = args[4:]
            if any(ref not in self.images for ref in refs):
                raise BUNDLE.BundleError("docker-image-save-failed")
            self.saved_images.update({ref: self.images[ref] for ref in refs})
            current_images = {ref: self.images[ref] for ref in refs}
            archive_files: dict[str, bytes] = {}
            manifest_records = []
            index_records = []
            for ref, record in current_images.items():
                files = dict(record["files"])
                compat = json.loads(files.pop("manifest.json"))
                root = json.loads(files.pop("index.json"))
                compat[0]["RepoTags"] = [ref]
                root["manifests"][0]["annotations"]["io.containerd.image.name"] = canonical_ref(ref)
                archive_files.update(files)
                if len(refs) == 1 and ref == self.bad_single_id_ref:
                    root["manifests"][0]["digest"] = "sha256:" + "0" * 64
                if not (len(refs) > 1 and ref in self.omit_combined_compat_refs):
                    manifest_records.extend(compat)
                index_records.extend(root["manifests"])
            if len(refs) > 1 and manifest_records and self.combined_extra_tag is not None:
                manifest_records[0]["RepoTags"].append(self.combined_extra_tag)
            if len(refs) > 1 and manifest_records and self.combined_duplicate_tag is not None:
                manifest_records[0]["RepoTags"].append(self.combined_duplicate_tag)
            if len(refs) > 1 and self.corrupt_combined_index_ref is not None:
                for descriptor in index_records:
                    annotations = descriptor.get("annotations", {})
                    if annotations.get("io.containerd.image.name") == canonical_ref(self.corrupt_combined_index_ref):
                        descriptor["digest"] = "sha256:" + "0" * 64
                        break
            archive_files["manifest.json"] = json_bytes(manifest_records)
            archive_files["index.json"] = json_bytes({
                "schemaVersion": 2,
                "mediaType": "application/vnd.oci.image.index.v1+json",
                "manifests": index_records,
            })
            if len(refs) > 1 and self.omit_combined_oci_index:
                archive_files.pop("index.json")
            raw_archive = tar_bytes(archive_files)
            archive_path.write_bytes(raw_archive)
            self.archive_writes.append((tuple(refs), raw_archive))
            return b""
        raise AssertionError(f"unexpected Docker operation: {args[:2]}")

    def load(self, _paths: list[Path]) -> None:
        self.load_count += 1
        self.images = dict(self.saved_images)


def share_oci_compat_child(fake: FakeDocker, source_ref: str, alias_ref: str) -> str:
    source = fake.images[source_ref]
    alias = fake.images[alias_ref]
    source_files = source["files"]
    alias_files = dict(source_files)
    compat = json.loads(alias_files["manifest.json"])
    compat[0]["RepoTags"] = [alias_ref]

    root = json.loads(alias_files["index.json"])
    root_descriptor = root["manifests"][0]
    source_root_id = root_descriptor["digest"]
    source_index_raw = alias_files["blobs/sha256/" + source_root_id[7:]]
    source_index = json.loads(source_index_raw)
    source_index["manifests"].append({
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "digest": "sha256:" + "1" * 64,
        "size": 1,
        "platform": {"os": "linux", "architecture": "arm64"},
    })
    alias_index_raw = json_bytes(source_index)
    alias_image_id = "sha256:" + sha(alias_index_raw)
    alias_files["blobs/sha256/" + alias_image_id[7:]] = alias_index_raw
    root_descriptor["digest"] = alias_image_id
    root_descriptor["size"] = len(alias_index_raw)
    root_descriptor["annotations"]["io.containerd.image.name"] = canonical_ref(alias_ref)
    alias_files["manifest.json"] = json_bytes(compat)
    alias_files["index.json"] = json_bytes(root)
    alias["files"] = alias_files
    alias["image_id"] = alias_image_id
    return alias_image_id


def local_trusted_file(path: Path, label: str, limit: int) -> bytes:
    info = path.lstat()
    if not path.is_file() or path.is_symlink() or info.st_size > limit:
        raise BUNDLE.BundleError(f"{label}-file-invalid")
    return path.read_bytes()


def local_trusted_size(path: Path, label: str, limit: int) -> int:
    info = path.lstat()
    if not path.is_file() or path.is_symlink() or info.st_size > limit:
        raise BUNDLE.BundleError(f"{label}-file-invalid")
    return info.st_size


def local_output_matches(directory: Path, files: dict[str, bytes]) -> bool:
    expected = {"release.manifest", "release-bundle.json", BUNDLE.COMPOSE_OVERRIDE, "d8"}
    if set(directory.iterdir()) != {directory / item for item in expected}:
        return False
    if set((directory / "d8").iterdir()) != {directory / "d8/artifact.json", directory / "d8/COMPLETE"}:
        return False
    for relative, payload in files.items():
        path = directory / relative
        mode = 0o444 if relative.startswith("d8/") else 0o600
        if path.read_bytes() != payload or path.stat().st_mode & 0o777 != mode:
            return False
    return True


class BundleFlowTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="release-delivery-bundle-")
        self.root = Path(self.temp.name)
        self.build = self.root / "build"
        self.build.mkdir(mode=0o700)
        (self.build / "d8").mkdir(mode=0o700)
        self.bundle_dir = self.root / "bundle"
        self.output_dir = self.root / "imported"
        self.fake = FakeDocker()
        self.image_rows = []
        for service in BUNDLE.V3_SERVICES:
            ref = f"lagrange-station-{service}:{COMMIT}"
            record = self.fake.add(ref, COMMIT)
            self.image_rows.append({"service": service, "ref": ref, "image_id": record["image_id"], "revision": COMMIT})
        self.dependency_rows = []
        for dependency in BUNDLE.DEPENDENCIES:
            record = self.fake.add(dependency["source_ref"], None, [dependency["source_ref"]])
            self.dependency_rows.append({
                "name": dependency["name"],
                "source_ref": dependency["source_ref"],
                "archive_ref": dependency["archive_ref"],
                "image_id": record["image_id"],
                "platform": BUNDLE.PLATFORM,
            })
        manifest = ["LAGRANGE_RELEASE_MANIFEST_V3", f"commit|{COMMIT}"]
        manifest.extend(f"image|{row['service']}|{row['ref']}|{row['image_id']}|{COMMIT}" for row in self.image_rows)
        (self.build / BUNDLE.RELEASE_MANIFEST).write_text("\n".join(manifest) + "\n", encoding="ascii")
        artifact, complete = valid_d8()
        (self.build / "d8/artifact.json").write_bytes(artifact)
        (self.build / "d8/COMPLETE").write_bytes(complete)

    def tearDown(self) -> None:
        self.temp.cleanup()

    def run_with_patches(self, function, *args):
        with patch.object(BUNDLE, "check_checkout", return_value=ROOT), \
             patch.object(BUNDLE, "trusted_directory", side_effect=lambda path, label, require_private=True: None), \
             patch.object(BUNDLE, "trusted_file", side_effect=local_trusted_file), \
             patch.object(BUNDLE, "trusted_file_size", side_effect=local_trusted_size), \
             patch.object(BUNDLE, "run_docker", side_effect=self.fake.run), \
             patch.object(BUNDLE, "output_matches", side_effect=local_output_matches):
            function(*args)

    def export(self) -> None:
        self.run_with_patches(BUNDLE.export_bundle, COMMIT, str(self.build), str(self.bundle_dir))

    def import_bundle(self) -> None:
        with patch.object(BUNDLE, "load_docker_archive", side_effect=self.fake.load):
            self.run_with_patches(BUNDLE.import_bundle, COMMIT, str(self.bundle_dir), str(self.output_dir))

    def expected_archive(self) -> dict[str, dict[str, object]]:
        with patch.object(BUNDLE, "run_docker", side_effect=self.fake.run):
            BUNDLE.ensure_dependency_aliases(self.dependency_rows)
        return BUNDLE.expected_archive_images(self.image_rows, self.dependency_rows)

    def rewrite_release_manifest(self) -> None:
        manifest = ["LAGRANGE_RELEASE_MANIFEST_V3", f"commit|{COMMIT}"]
        manifest.extend(
            f"image|{row['service']}|{row['ref']}|{row['image_id']}|{COMMIT}"
            for row in self.image_rows
        )
        (self.build / BUNDLE.RELEASE_MANIFEST).write_text("\n".join(manifest) + "\n", encoding="ascii")

    def test_export_import_retry_and_duplicate_import(self) -> None:
        self.export()
        assets = {item.name for item in self.bundle_dir.iterdir()}
        self.assertEqual(assets, {
            BUNDLE.BUNDLE_MANIFEST,
            BUNDLE.RELEASE_MANIFEST,
            BUNDLE.D8_ARTIFACT_ASSET,
            BUNDLE.D8_COMPLETE_ASSET,
            "images.tar.zst.part000000",
        })
        self.assertTrue(all(item.is_file() for item in self.bundle_dir.iterdir()))

        chunk = self.bundle_dir / "images.tar.zst.part000000"
        original = chunk.read_bytes()
        chunk.write_bytes(original[:-1] + bytes([original[-1] ^ 1]))
        with self.assertRaisesRegex(BUNDLE.BundleError, "chunk-digest-mismatch"):
            self.import_bundle()
        self.assertEqual(self.fake.load_count, 0)
        self.assertFalse(self.output_dir.exists())
        chunk.write_bytes(original)

        with patch.object(BUNDLE, "materialize_output", side_effect=BUNDLE.BundleError("retry-after-load")):
            with self.assertRaisesRegex(BUNDLE.BundleError, "retry-after-load"):
                self.import_bundle()
        self.assertEqual(self.fake.load_count, 1)
        self.assertFalse(self.output_dir.exists())

        self.import_bundle()
        self.assertEqual(self.fake.load_count, 2)
        self.assertEqual((self.output_dir / "release.manifest").read_bytes(), (self.bundle_dir / "release.manifest").read_bytes())
        self.assertEqual((self.output_dir / "d8/artifact.json").stat().st_mode & 0o777, 0o444)
        self.assertEqual((self.output_dir / "d8/COMPLETE").stat().st_mode & 0o777, 0o444)
        self.assertEqual((self.output_dir / "release.manifest").stat().st_mode & 0o777, 0o600)
        override = (self.output_dir / BUNDLE.COMPOSE_OVERRIDE).read_text(encoding="ascii")
        self.assertIn(f"postgres:\n    image: {self.dependency_rows[0]['image_id']}", override)
        self.assertIn(f"research-schema-check:\n    image: {self.dependency_rows[0]['image_id']}", override)
        self.assertIn(f"reverse-proxy:\n    image: {self.dependency_rows[1]['image_id']}", override)
        self.assertIn(f"research-raw-init:\n    image: {self.dependency_rows[2]['image_id']}", override)

        self.import_bundle()
        self.assertEqual(self.fake.load_count, 2)

    def test_export_repairs_missing_oci_compat_alias_and_import_remains_strict(self) -> None:
        source_ref = self.image_rows[0]["ref"]
        missing_ref = self.image_rows[1]["ref"]
        self.image_rows[1]["image_id"] = share_oci_compat_child(self.fake, source_ref, missing_ref)
        self.assertNotEqual(self.image_rows[0]["image_id"], self.image_rows[1]["image_id"])
        self.rewrite_release_manifest()
        self.fake.omit_combined_compat_refs.add(missing_ref)

        validate_archive_file = BUNDLE.validate_archive_file
        repaired: dict[str, bytes] = {}

        def capture_repaired_archive(path, expected, **kwargs):
            if path.name == "repaired.tar":
                repaired["raw"] = path.read_bytes()
            return validate_archive_file(path, expected, **kwargs)

        with patch.object(BUNDLE, "validate_archive_file", side_effect=capture_repaired_archive):
            self.export()

        expected = self.expected_archive()
        self.assertEqual(self.fake.archive_writes[0][0], tuple(expected))
        with self.assertRaisesRegex(BUNDLE.BundleError, "docker-archive-tag-set-invalid"):
            BUNDLE.validate_archive_stream(io.BytesIO(self.fake.archive_writes[0][1]), expected)
        self.assertEqual(self.fake.archive_writes[1][0], (missing_ref,))
        BUNDLE.validate_archive_stream(io.BytesIO(repaired["raw"]), expected)

        chunk_path = self.bundle_dir / "images.tar.zst.part000000"
        metadata_path = self.bundle_dir / BUNDLE.BUNDLE_MANIFEST
        original_chunk = chunk_path.read_bytes()
        original_metadata = metadata_path.read_bytes()
        incomplete_archive = self.fake.archive_writes[0][1]
        incomplete_chunk = subprocess.check_output(["zstd", "-q", "-T1", "-4", "-c"], input=incomplete_archive)
        metadata = json.loads(original_metadata)
        metadata["chunks"] = [{"name": chunk_path.name, "bytes": len(incomplete_chunk), "sha256": sha(incomplete_chunk)}]
        metadata["transport_bytes"] = len(incomplete_chunk)
        metadata["transport_sha256"] = sha(incomplete_chunk)
        chunk_path.write_bytes(incomplete_chunk)
        metadata_path.write_bytes(BUNDLE.canonical_json(metadata))
        with self.assertRaisesRegex(BUNDLE.BundleError, "docker-archive-tag-set-invalid"):
            self.import_bundle()
        self.assertEqual(self.fake.load_count, 0)
        self.assertFalse(self.output_dir.exists())
        chunk_path.write_bytes(original_chunk)
        metadata_path.write_bytes(original_metadata)

        self.import_bundle()
        self.assertEqual(self.fake.load_count, 1)

    def test_export_does_not_repair_invalid_compatibility_tags_or_crosswired_index(self) -> None:
        missing_ref = self.image_rows[2]["ref"]
        expected = self.expected_archive()
        archive = self.root / "images.tar"

        for mutation in ("nonstring", "unknown", "duplicate", "index", "missing-index"):
            with self.subTest(mutation=mutation):
                self.fake.archive_writes.clear()
                self.fake.calls.clear()
                self.fake.omit_combined_compat_refs = {missing_ref}
                self.fake.combined_extra_tag = None
                self.fake.combined_duplicate_tag = None
                self.fake.corrupt_combined_index_ref = None
                self.fake.omit_combined_oci_index = False
                if mutation == "nonstring":
                    self.fake.combined_extra_tag = 17
                elif mutation == "unknown":
                    self.fake.combined_extra_tag = "foreign.example/unknown:tag"
                elif mutation == "duplicate":
                    self.fake.combined_duplicate_tag = self.image_rows[0]["ref"]
                elif mutation == "missing-index":
                    self.fake.omit_combined_oci_index = True
                else:
                    self.fake.corrupt_combined_index_ref = self.image_rows[0]["ref"]

                expected_error = {
                    "nonstring": "docker-archive-compat-tag-nonstring",
                    "unknown": "docker-archive-compat-tag-unexpected",
                    "duplicate": "docker-archive-compat-tag-duplicate",
                    "index": "docker-archive-oci-image-id-mismatch",
                    "missing-index": "docker-archive-tag-set-unrepairable",
                }[mutation]
                with self.assertRaisesRegex(BUNDLE.BundleError, expected_error):
                    self.run_with_patches(BUNDLE.save_verified_archive, archive, expected)
                self.assertEqual(len(self.fake.archive_writes), 1)
                self.assertEqual(
                    len([call for call in self.fake.calls if call[:3] == ("image", "save", "--output")]),
                    1,
                )
                self.assertEqual(archive.read_bytes(), self.fake.archive_writes[0][1])
                with self.assertRaisesRegex(BUNDLE.BundleError, "docker-archive-tag-set-invalid"):
                    BUNDLE.validate_archive_stream(io.BytesIO(archive.read_bytes()), expected)
                self.assertEqual(set(self.root.iterdir()), {self.build, archive})

    def test_export_rejects_missing_compat_match_without_replacing_raw_archive(self) -> None:
        missing_ref = self.image_rows[2]["ref"]
        expected = self.expected_archive()
        self.fake.omit_combined_compat_refs.add(missing_ref)
        archive = self.root / "images.tar"

        with self.assertRaisesRegex(BUNDLE.BundleError, "docker-archive-compat-tag-match-missing"):
            self.run_with_patches(BUNDLE.save_verified_archive, archive, expected)

        self.assertEqual(archive.read_bytes(), self.fake.archive_writes[0][1])
        self.assertEqual(len(self.fake.archive_writes), 2)
        self.assertEqual(set(self.root.iterdir()), {self.build, archive})

    def test_export_rejects_ambiguous_compat_match(self) -> None:
        source_ref = self.image_rows[0]["ref"]
        second_ref = self.image_rows[1]["ref"]
        missing_ref = self.image_rows[2]["ref"]
        self.image_rows[1]["image_id"] = share_oci_compat_child(self.fake, source_ref, second_ref)
        self.image_rows[2]["image_id"] = share_oci_compat_child(self.fake, source_ref, missing_ref)
        self.rewrite_release_manifest()
        expected = self.expected_archive()
        self.fake.omit_combined_compat_refs.add(missing_ref)
        archive = self.root / "images.tar"

        with self.assertRaisesRegex(BUNDLE.BundleError, "docker-archive-compat-tag-match-ambiguous"):
            self.run_with_patches(BUNDLE.save_verified_archive, archive, expected)

        self.assertEqual(archive.read_bytes(), self.fake.archive_writes[0][1])
        self.assertEqual(len(self.fake.archive_writes), 2)
        self.assertEqual(set(self.root.iterdir()), {self.build, archive})

    def test_export_rejects_single_ref_identity_mismatch_and_cleans_private_work(self) -> None:
        source_ref = self.image_rows[0]["ref"]
        missing_ref = self.image_rows[1]["ref"]
        self.image_rows[1]["image_id"] = share_oci_compat_child(self.fake, source_ref, missing_ref)
        self.rewrite_release_manifest()
        expected = self.expected_archive()
        self.fake.omit_combined_compat_refs.add(missing_ref)
        self.fake.bad_single_id_ref = missing_ref
        archive = self.root / "images.tar"

        with self.assertRaisesRegex(BUNDLE.BundleError, "docker-archive-oci-image-id-mismatch"):
            self.run_with_patches(BUNDLE.save_verified_archive, archive, expected)

        self.assertEqual(archive.read_bytes(), self.fake.archive_writes[0][1])
        self.assertEqual(len(self.fake.archive_writes), 2)
        self.assertEqual(set(self.root.iterdir()), {self.build, archive})

    def test_extra_asset_and_wrong_commit_fail_before_load(self) -> None:
        self.export()
        (self.bundle_dir / "unexpected.bin").write_bytes(b"not allowed")
        with self.assertRaisesRegex(BUNDLE.BundleError, "directory-entry-set-invalid"):
            self.import_bundle()
        (self.bundle_dir / "unexpected.bin").unlink()
        with self.assertRaisesRegex(BUNDLE.BundleError, "source-or-platform-mismatch"):
            self.run_with_patches(BUNDLE.import_bundle, "d" * 40, str(self.bundle_dir), str(self.output_dir))
        self.assertEqual(self.fake.load_count, 0)
        self.assertFalse(self.output_dir.exists())

    def test_d8_receipt_rejects_unallowlisted_field_before_docker(self) -> None:
        artifact_raw, complete_raw = valid_d8()
        artifact = json.loads(artifact_raw)
        artifact["secret_marker"] = "fixture-must-not-be-transported"
        altered = json_bytes(artifact)
        (self.build / "d8/artifact.json").write_bytes(altered)
        (self.build / "d8/COMPLETE").write_bytes((sha(altered) + "\n").encode("ascii"))
        with self.assertRaisesRegex(BUNDLE.BundleError, "d8-artifact-contract-invalid"):
            self.export()
        self.assertEqual(self.fake.calls, [])
        self.assertEqual((self.build / "d8/COMPLETE").read_bytes(), (sha(altered) + "\n").encode("ascii"))


class DockerArchiveTests(unittest.TestCase):
    def test_arm64_archive_is_rejected(self) -> None:
        ref = "fixture/test:tag"
        image_id, files, _ = make_oci_image(ref, COMMIT, "linux/arm64")
        with self.assertRaisesRegex(BUNDLE.BundleError, "platform-mismatch"):
            BUNDLE.validate_archive_stream(io.BytesIO(tar_bytes(files)), {
                ref: {"image_id": image_id, "revision": COMMIT},
            })

    def test_oci_index_id_mismatch_fails_before_load(self) -> None:
        ref = "fixture/test:tag"
        image_id, files, _ = make_oci_image(ref, COMMIT)
        wrong_id = "sha256:" + ("0" if image_id[-1] != "0" else "1") * 64
        with self.assertRaisesRegex(BUNDLE.BundleError, "oci-image-id-mismatch"):
            BUNDLE.validate_archive_stream(io.BytesIO(tar_bytes(files)), {
                ref: {"image_id": wrong_id, "revision": COMMIT},
            })

    def test_oci_compatibility_manifest_crosswired_layers_rejected(self) -> None:
        refs = ("fixture/first:tag", "fixture/second:tag")
        expected: dict[str, dict[str, object]] = {}
        archive_files: dict[str, bytes] = {}
        compatibility_records = []
        root_descriptors = []
        for ref in refs:
            image_id, files, _ = make_oci_image(ref, COMMIT)
            compatibility = json.loads(files.pop("manifest.json"))
            root = json.loads(files.pop("index.json"))
            compatibility_records.extend(compatibility)
            root_descriptors.extend(root["manifests"])
            archive_files.update(files)
            expected[ref] = {"image_id": image_id, "revision": COMMIT}
        compatibility_records[0]["Layers"] = list(compatibility_records[1]["Layers"])
        archive_files["manifest.json"] = json_bytes(compatibility_records)
        archive_files["index.json"] = json_bytes({
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.index.v1+json",
            "manifests": root_descriptors,
        })
        with self.assertRaisesRegex(BUNDLE.BundleError, "oci-layers-crosswired"):
            BUNDLE.validate_archive_stream(io.BytesIO(tar_bytes(archive_files)), expected)

    def test_oci_named_root_may_be_direct_schema2_manifest(self) -> None:
        ref = "fixture/direct:tag"
        _, files, _ = make_oci_image(ref, COMMIT)
        root = json.loads(files["index.json"])
        image_index_path = "blobs/sha256/" + root["manifests"][0]["digest"][7:]
        image_index = json.loads(files[image_index_path])
        selected = image_index["manifests"][0]
        root["manifests"][0]["digest"] = selected["digest"]
        root["manifests"][0]["size"] = selected["size"]
        root["manifests"][0]["mediaType"] = selected["mediaType"]
        direct_id = selected["digest"]
        files["index.json"] = json_bytes(root)
        BUNDLE.validate_archive_stream(io.BytesIO(tar_bytes(files)), {
            ref: {"image_id": direct_id, "revision": COMMIT},
        })

    def test_current_docker_save_layout_is_accepted_when_available(self) -> None:
        try:
            image_id = subprocess.check_output(
                ["docker", "image", "inspect", "--format", "{{.Id}}", "hello-world"],
                stderr=subprocess.DEVNULL,
                text=True,
            ).strip()
        except (OSError, subprocess.CalledProcessError):
            self.skipTest("Docker Engine or local hello-world image unavailable")
        with tempfile.TemporaryDirectory(prefix="release-delivery-docker-archive-") as temporary:
            archive = Path(temporary) / "hello-world.tar"
            try:
                subprocess.run(
                    ["docker", "image", "save", "--output", str(archive), "hello-world"],
                    check=True,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                )
            except (OSError, subprocess.CalledProcessError):
                self.skipTest("Docker Engine could not save local hello-world image")
            BUNDLE.validate_archive_file(archive, {
                "hello-world:latest": {"image_id": image_id, "revision": None},
            })


if __name__ == "__main__":
    unittest.main()
