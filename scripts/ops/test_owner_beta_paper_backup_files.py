#!/usr/bin/env python3
"""Pure Paper-combined file-bundle codec tests for ordinary Python discovery.

Actual Linux-root fixtures live in owner-beta-paper-backup-files-self-test.py
and run through the existing hosted backup fixture step.
"""
from __future__ import annotations

import hashlib
import importlib.util
import io
import json
import os
import struct
import sys
import unittest
from pathlib import Path


sys.dont_write_bytecode = True
SOURCE = Path(__file__).with_name("lib") / "owner-beta-daily-backup-files.py"
ARCHIVE_SCHEMA = Path(__file__).resolve().parents[2] / "docs" / "contracts" / "owner-beta-paper-operations-archive-v1.schema.json"
SPEC = importlib.util.spec_from_file_location("owner_beta_daily_backup_files", SOURCE)
FILES = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = FILES
SPEC.loader.exec_module(FILES)

PAPER_CLASSES = (
    "base", "config", "curated", "daily", "floor", "paper_config",
    "paper_contract", "paper_evidence", "paper_floor", "paper_manifest",
    "paper_operations", "paper_reader", "raw", "simulation_approval",
    "simulation_config", "simulation_floor", "simulation_input",
    "simulation_manifest", "simulation_reader", "simulation_receipt",
)
PAPER_MAGIC = b"LAGRANGE_PAPER_COMBINED_FILES_V1\n"
PAPER_SCHEMA = "lagrange-paper-combined-files-v1"
EMPTY_SHA256 = "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"

OLD_GOLDENS = {
    "daily": (656, "de689c5b261e54ca06d876df63c18c8e7afa9fc124b91ae440cf6f74fad997fa"),
    "preupgrade": (578, "0f4a73dd4d9960ce3cc59d95fbf7e06c4fc2773999c23b988bda6ad21c36dcef"),
    "simulation": (1122, "2a6db99584407cec4c6af52e5e8ee093ade265547e0c9132542040f1173979e4"),
    "daily-simulation": (1510, "61ff2a1b82d147aa2948e8c3e049177b98a3f1a3401792192eeec408f155074b"),
}
OLD_PAYLOAD_GOLDENS = {
    "daily": (831, "f087e78f77bebf4dec4d68f0530753644352a85d4856ecdd6035082399414158"),
    "preupgrade": (753, "8a14e48916586e3cb32756115c68034177ef28f47b84c2c91452347e468b63d6"),
    "simulation": (1297, "ccf56add360051c92e975a61ae046bb3548f813063b7c076404f50ffc2ad2b5d"),
    "daily-simulation": (1685, "9614a946fddfe23a1091a2bc41f367ee734f0df532f112f1f7ab1782934762a2"),
}


def limits(*, total=1 << 20, entries=1000, manifest=1 << 20):
    return FILES.Limits(total_bytes=total, entries=entries, manifest_bytes=manifest)


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"),
                      ensure_ascii=True, allow_nan=False).encode("ascii")


def digest(data):
    return "sha256:" + hashlib.sha256(data).hexdigest()


def root_entry(name, *, uid=0, gid=0, mode=0o700):
    return {"path": name, "kind": "directory", "uid": uid, "gid": gid,
            "mode": mode, "size": 0, "sha256": None}


def paper_manifest(extra=None, *, classes=PAPER_CLASSES, schema=PAPER_SCHEMA):
    extra = extra or {}
    entries = []
    for name in classes:
        entries.append(root_entry(name))
        entries.extend(extra.get(name, ()))
    return {"schema": schema, "classes": list(classes), "entries": entries}


def paper_manifest_bytes(extra=None, **kwargs):
    return canonical(paper_manifest(extra, **kwargs))


def paper_archive(manifest=None, payload=b""):
    raw = paper_manifest_bytes() if manifest is None else canonical(manifest)
    return PAPER_MAGIC + struct.pack(">Q", len(raw)) + raw + payload


def sample_manifest(content=b"abc"):
    return paper_manifest({"base": [
        root_entry("base/dir"),
        {"path": "base/dir/item.bin", "kind": "file", "uid": 10001,
         "gid": 10001, "mode": 0o440, "size": len(content),
         "sha256": digest(content)},
    ]})


def class_summary(data):
    return FILES.class_records(data, limits(), profile="paper-combined")


class PaperCombinedCodecTests(unittest.TestCase):
    def test_constructor_accepts_only_the_declared_alias_and_disjoint_selections(self):
        def roots():
            return [FILES.Root(name, "/paper-codec-constructor/" + name, 0, 0, 0o700)
                    for name in PAPER_CLASSES]

        values = roots()
        FILES.Snapshot(values, limits(), profile="paper-combined")
        simulation_index = PAPER_CLASSES.index("simulation_input")
        values[simulation_index] = FILES.Root("simulation_input", values[0].path, 0, 0, 0o700)
        FILES.Snapshot(values, limits(), profile="paper-combined")
        for name, path, mode, leaf in (
                ("config", values[0].path, 0o700, None),
                ("config", values[0].path + "/child", 0o700, None),
                ("simulation_input", values[0].path, 0o750, None),
                ("simulation_input", values[0].path, 0o700, "child")):
            candidate = roots()
            index = PAPER_CLASSES.index(name)
            candidate[index] = FILES.Root(name, path, 0, 0, mode, leaf=leaf)
            with self.subTest(name=name, mode=mode, leaf=leaf), self.assertRaises(FILES.SnapshotError):
                FILES.Snapshot(candidate, limits(), profile="paper-combined")

    def test_exact_twenty_class_order_magic_schema_and_empty_records(self):
        public_schema = json.loads(ARCHIVE_SCHEMA.read_bytes())
        archive_def = public_schema["$defs"]["archive"]
        class_def = archive_def["properties"]["classes"]
        self.assertEqual(archive_def["properties"]["profile"]["const"], "paper-combined")
        self.assertEqual((class_def["minItems"], class_def["maxItems"]), (20, 20))
        self.assertEqual(tuple(item["properties"]["class"]["const"]
                               for item in class_def["prefixItems"]), PAPER_CLASSES)
        self.assertEqual(PAPER_CLASSES, tuple(sorted(PAPER_CLASSES)))
        self.assertEqual(FILES.profile_values("paper-combined"),
                         (PAPER_CLASSES, PAPER_MAGIC, PAPER_SCHEMA))
        raw = paper_manifest_bytes()
        manifest = FILES.validate_manifest(raw, limits(), profile="paper-combined")
        self.assertEqual(manifest["classes"], list(PAPER_CLASSES))
        self.assertEqual([entry["path"] for entry in manifest["entries"]], list(PAPER_CLASSES))
        rows = class_summary(raw)
        self.assertEqual([row["class"] for row in rows], list(PAPER_CLASSES))
        self.assertTrue(all(set(row) == {"class", "sha256", "entries", "bytes"} for row in rows))
        self.assertTrue(all((row["sha256"], row["entries"], row["bytes"]) ==
                            (EMPTY_SHA256, 0, 0) for row in rows))

    def test_class_stream_frame_arithmetic_and_preserved_entry_metadata(self):
        raw = canonical(sample_manifest())
        rows = class_summary(raw)
        base = rows[0]
        self.assertEqual(base, {
            "class": "base",
            "sha256": "sha256:26220529a37b8f75fc517413cba45da2502329cde5d8c3afd3addffd7c84b9df",
            "entries": 2,
            "bytes": 3,
        })
        self.assertEqual(rows[1]["sha256"], EMPTY_SHA256)
        self.assertEqual(rows[1]["entries"], 0)
        self.assertEqual(rows[1]["bytes"], 0)

    def test_nonroot_entry_name_content_hash_and_metadata_change_class_digest(self):
        original = sample_manifest()
        original_base = class_summary(canonical(original))[0]
        changes = (
            lambda m: m["entries"][2].update(path="base/dir/renamed.bin"),
            lambda m: m["entries"][2].update(sha256="sha256:" + "a" * 64),
            lambda m: m["entries"][2].update(mode=0o400),
            lambda m: m["entries"][2].update(uid=10002),
            lambda m: m["entries"][2].update(gid=10002),
        )
        for mutate in changes:
            candidate = json.loads(canonical(original))
            mutate(candidate)
            candidate_base = class_summary(canonical(candidate))[0]
            self.assertNotEqual(candidate_base["sha256"], original_base["sha256"])
        root_mode = json.loads(canonical(original))
        root_mode["entries"][0]["mode"] = 0o750
        changed_root = canonical(root_mode)
        self.assertEqual(class_summary(changed_root), class_summary(canonical(original)))
        self.assertNotEqual(digest(paper_archive(original)), digest(paper_archive(root_mode)))

    def test_extra_missing_duplicate_and_misordered_class_arrays_fail(self):
        valid = paper_manifest()
        variants = []
        variants.append({**valid, "classes": list(PAPER_CLASSES[:-1])})
        variants.append({**valid, "classes": [*PAPER_CLASSES, "unexpected"]})
        duplicate = list(PAPER_CLASSES)
        duplicate[-1] = duplicate[-2]
        variants.append({**valid, "classes": duplicate})
        variants.append({**valid, "classes": list(reversed(PAPER_CLASSES))})
        for value in variants:
            with self.subTest(classes=value["classes"]), self.assertRaises(FILES.SnapshotError):
                FILES.validate_manifest(canonical(value), limits(), profile="paper-combined")

    def test_traversal_parent_type_mode_owner_depth_and_count_bounds_fail(self):
        wrong_root = paper_manifest()
        wrong_root["entries"][0]["path"] = "../base"
        wrong_kind = paper_manifest()
        wrong_kind["entries"][0]["kind"] = "socket"
        wrong_mode = paper_manifest()
        wrong_mode["entries"][0]["mode"] = 0o777
        wrong_owner = paper_manifest()
        wrong_owner["entries"][0]["uid"] = 999
        missing_parent = paper_manifest({"base": [{"path": "base/missing/leaf", "kind": "file",
            "uid": 0, "gid": 0, "mode": 0o600, "size": 0, "sha256": EMPTY_SHA256}]})
        file_parent = paper_manifest({"base": [
            {"path": "base/a", "kind": "file", "uid": 0, "gid": 0,
             "mode": 0o600, "size": 0, "sha256": EMPTY_SHA256},
            {"path": "base/a/child", "kind": "file", "uid": 0, "gid": 0,
             "mode": 0o600, "size": 0, "sha256": EMPTY_SHA256},
        ]})
        too_deep = paper_manifest({"base": [{"path": "base/" + "/".join(["x"] * 64),
            "kind": "file", "uid": 0, "gid": 0, "mode": 0o600,
            "size": 0, "sha256": EMPTY_SHA256}]})
        for value in (wrong_root, wrong_kind, wrong_mode, wrong_owner, missing_parent,
                      file_parent, too_deep):
            with self.subTest(value=value), self.assertRaises(FILES.SnapshotError):
                FILES.validate_manifest(canonical(value), limits(), profile="paper-combined")
        count_value = paper_manifest({"base": [{"path": "base/x", "kind": "file", "uid": 0,
            "gid": 0, "mode": 0o600, "size": 0, "sha256": EMPTY_SHA256}]})
        with self.assertRaises(FILES.SnapshotError):
            FILES.validate_manifest(canonical(count_value), limits(entries=20), profile="paper-combined")

    def test_total_bytes_manifest_bytes_and_depth_bounds_fail(self):
        value = sample_manifest(content=b"abcd")
        with self.assertRaises(FILES.SnapshotError):
            FILES.validate_manifest(canonical(value), limits(total=3), profile="paper-combined")
        with self.assertRaises(FILES.SnapshotError):
            FILES.verify_archive(io.BytesIO(paper_archive()), limits(manifest=1024),
                                 profile="paper-combined")
        with self.assertRaises(FILES.SnapshotError):
            FILES.class_records(paper_manifest_bytes(), limits(manifest=1024),
                                profile="paper-combined")
        with self.assertRaises(FILES.SnapshotError):
            FILES.safe_parts("/".join(("base", *(["x"] * 64))))

    def test_truncated_payload_digest_mismatch_and_trailing_bytes_fail(self):
        content = b"paper payload"
        manifest = sample_manifest(content)
        encoded = paper_archive(manifest, content)
        verified, retained = FILES.verify_archive(io.BytesIO(encoded), limits(),
                                                   retain=("base/dir/item.bin",),
                                                   profile="paper-combined")
        self.assertEqual(verified, manifest)
        self.assertEqual(retained, {"base/dir/item.bin": content})
        for broken in (encoded[:-1], encoded[:-1] + bytes([encoded[-1] ^ 1]), encoded + b"extra"):
            with self.subTest(size=len(broken)), self.assertRaises(FILES.SnapshotError):
                FILES.verify_archive(io.BytesIO(broken), limits(), profile="paper-combined")

    def test_cross_profile_decode_fails_even_when_outer_magic_is_rehashed(self):
        paper = paper_archive()
        daily_magic = FILES.profile_values("daily")[1]
        rewrapped_paper = daily_magic + paper[len(PAPER_MAGIC):]
        with self.assertRaises(FILES.SnapshotError):
            FILES.verify_archive(io.BytesIO(rewrapped_paper), limits(), profile="daily")
        with self.assertRaises(FILES.SnapshotError):
            FILES.validate_manifest(paper_manifest_bytes(), limits(), profile="daily")
        with self.assertRaises(FILES.SnapshotError):
            FILES.class_records(paper_manifest_bytes(), limits(), profile="daily")

    def test_existing_four_profile_archives_keep_their_golden_bytes(self):
        expected = {
            "daily": (("raw", "curated", "daily", "config", "floor", "base"),
                      b"LAGRANGE_DAILY_FILES_V1\n", "lagrange-daily-files-v1"),
            "preupgrade": (("raw", "curated", "base", "config", "postgres"),
                           b"LAGRANGE_PREUPGRADE_FILES_V1\n", "lagrange-preupgrade-files-v1"),
            "simulation": (("raw", "curated", "simulation_input", "simulation_config",
                            "simulation_floor", "simulation_receipt", "simulation_manifest",
                            "simulation_reader", "simulation_approval"),
                           b"LAGRANGE_SIMULATION_FILES_V1\n", "lagrange-simulation-files-v1"),
            "daily-simulation": (("raw", "curated", "daily", "config", "floor", "base",
                                  "simulation_input", "simulation_config", "simulation_floor",
                                  "simulation_receipt", "simulation_manifest", "simulation_reader",
                                  "simulation_approval"),
                                 b"LAGRANGE_DAILY_SIMULATION_FILES_V1\n",
                                 "lagrange-daily-simulation-files-v1"),
        }
        for profile, (classes, magic, schema) in expected.items():
            with self.subTest(profile=profile):
                self.assertEqual(FILES.profile_values(profile), (classes, magic, schema))
                entries = [root_entry(name) for name in classes]
                raw = canonical({"schema": schema, "classes": list(classes), "entries": entries})
                archive = magic + struct.pack(">Q", len(raw)) + raw
                expected_size, expected_sha = OLD_GOLDENS[profile]
                self.assertEqual(len(archive), expected_size)
                self.assertEqual(hashlib.sha256(archive).hexdigest(), expected_sha)
                manifest, retained = FILES.verify_archive(io.BytesIO(archive), limits(), profile=profile)
                self.assertEqual(manifest["classes"], list(classes))
                self.assertEqual(retained, {})
                payload = b"legacy payload\n"
                entries = []
                for index, name in enumerate(classes):
                    entries.append(root_entry(name))
                    if index == 0:
                        entries.append({"path": name + "/compat.bin", "kind": "file",
                            "uid": 0, "gid": 0, "mode": 0o600, "size": len(payload),
                            "sha256": digest(payload)})
                raw = canonical({"schema": schema, "classes": list(classes), "entries": entries})
                archive = magic + struct.pack(">Q", len(raw)) + raw + payload
                expected_size, expected_sha = OLD_PAYLOAD_GOLDENS[profile]
                self.assertEqual(len(archive), expected_size)
                self.assertEqual(hashlib.sha256(archive).hexdigest(), expected_sha)
                manifest, retained = FILES.verify_archive(io.BytesIO(archive), limits(),
                    retain=(classes[0] + "/compat.bin",), profile=profile)
                self.assertEqual(manifest["classes"], list(classes))
                self.assertEqual(retained, {classes[0] + "/compat.bin": payload})




if __name__ == "__main__":
    unittest.main()
