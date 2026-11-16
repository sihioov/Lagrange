#!/usr/bin/env python3
"""Pure Paper-combined file-bundle codec tests for ordinary Python discovery.

Actual Linux-root fixtures live in owner-beta-paper-backup-files-self-test.py
and run through the existing hosted backup fixture step.
"""
from __future__ import annotations

import contextlib
import hashlib
import importlib.util
import io
import json
import os
import shutil
import stat
import struct
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


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

    def test_paper_raw_lock_predicate_is_exact_and_returns_a_boolean(self):
        self.assertIs(FILES.paper_raw_commit_lock(
            "raw/manifests/provider=kis/market=krx/commit.lock"), True)
        for path in (
                "raw/commit.lock",
                "raw/manifests/provider=kis/commit.lock",
                "raw/manifests/provider=kis/market=krx/nested/commit.lock",
                "raw/manifests/provider=../market=krx/commit.lock",
                "curated/manifests/provider=kis/market=krx/commit.lock",
                "raw/manifests/provider=kis/market=krx/other.lock"):
            with self.subTest(path=path):
                self.assertIs(FILES.paper_raw_commit_lock(path), False)




def raw_entry(path, kind="directory", data=b""):
    if kind == "directory":
        return root_entry(path, uid=10001, gid=10001, mode=0o750)
    return {"path": path, "kind": "file", "uid": 10001, "gid": 10001, "mode": 0o640,
            "size": len(data), "sha256": digest(data)}


class PaperRawScopeCodecTests(unittest.TestCase):
    """Pure selection grammar and codec rules for Paper's Raw provider scopes."""

    def test_provider_set_is_core_without_retired_v2_scope(self):
        self.assertEqual(FILES.PAPER_RAW_PROVIDERS, frozenset((
            "kis", "kis-calendar", "kis-daily-range",
            "krx", "kis-normalized", "kis-calendar-normalized")))
        self.assertEqual(FILES.PAPER_RAW_V2_PROVIDER, "kis-daily-range-normalized")
        self.assertFalse(hasattr(FILES, "PAPER_RAW_INCLUDES_V2"))
        self.assertNotIn(FILES.PAPER_RAW_V2_PROVIDER, FILES.PAPER_RAW_PROVIDERS)

    def test_child_filter_applies_only_at_raw_and_manifest_area(self):
        scopes = {"provider=" + name for name in FILES.PAPER_RAW_PROVIDERS}
        self.assertEqual(FILES.paper_raw_children("raw"), scopes | {"manifests"})
        self.assertEqual(FILES.paper_raw_children("raw/manifests"), scopes)
        for path in ("raw/provider=kis", "raw/manifests/provider=kis", "curated", "base/raw"):
            self.assertIsNone(FILES.paper_raw_children(path))

    def test_selected_predicate(self):
        for path in ("raw", "raw/manifests", "raw/manifests/provider=krx",
                     "raw/manifests/provider=kis/market=kr/manifest.jsonl",
                     "raw/manifests/provider=kis/market=kr/commit.lock",
                     "raw/provider=kis-normalized/market=kr/date=2026-11-06/batch=b/batch.json"):
            with self.subTest(path=path):
                self.assertIs(FILES.paper_raw_selected(path), True)
        for path in ("raw/raw", "raw/raw/manifests/provider=kis/market=kr/manifest.jsonl",
                     "raw/.calendar-bootstrap", "raw/2026-01-01", "raw/commit.lock",
                     "raw/manifests/commit.lock", "raw/provider=other/market=kr",
                     "raw/manifests/provider=kis-daily-range-normalized/market=kr/manifest.jsonl",
                     "raw/provider=kis-daily-range-normalized", "raw/provider=kis-extra",
                     "curated/provider=kis"):
            with self.subTest(path=path):
                self.assertIs(FILES.paper_raw_selected(path), False)

    def test_partial_raw_class_with_parent_directories_validates(self):
        data = b"{}\n"
        manifest = paper_manifest({"raw": [
            raw_entry("raw/manifests"),
            raw_entry("raw/manifests/provider=kis"),
            raw_entry("raw/manifests/provider=kis/market=kr"),
            raw_entry("raw/manifests/provider=kis/market=kr/manifest.jsonl", "file", data),
            raw_entry("raw/provider=kis"),
            raw_entry("raw/provider=kis/market=kr"),
        ]})
        raw = canonical(manifest)
        self.assertEqual(FILES.validate_manifest(raw, limits(), profile="paper-combined"), manifest)
        archive = paper_archive(manifest, data)
        verified, _ = FILES.verify_archive(io.BytesIO(archive), limits(), profile="paper-combined")
        self.assertEqual(verified, manifest)
        raw_row = [row for row in class_summary(raw) if row["class"] == "raw"][0]
        self.assertEqual((raw_row["entries"], raw_row["bytes"]), (6, len(data)))

    def test_unselected_raw_member_or_file_boundary_fails_codec(self):
        for extra in ([raw_entry("raw/raw")],
                      [raw_entry("raw/.calendar-bootstrap")],
                      [raw_entry("raw/provider=kis-daily-range-normalized")],
                      [raw_entry("raw/manifests"), raw_entry("raw/manifests/provider=other")],
                      [raw_entry("raw/commit.lock", "file")],
                      [raw_entry("raw/manifests", "file")],
                      [raw_entry("raw/provider=kis", "file")],
                      [raw_entry("raw/manifests"), raw_entry("raw/manifests/provider=kis", "file")]):
            with self.subTest(extra=[row["path"] for row in extra]):
                with self.assertRaisesRegex(FILES.SnapshotError, "^DAILY_BACKUP_FILES_RAW_SCOPE$"):
                    FILES.validate_manifest(canonical(paper_manifest({"raw": extra})), limits(),
                                            profile="paper-combined")
        # Other profiles keep their existing whole-root Raw format.
        daily = {"schema": "lagrange-daily-files-v1", "classes": list(FILES.CLASSES),
                 "entries": [root_entry(name) if name != "raw" else root_entry("raw")
                             for name in FILES.CLASSES]}
        daily["entries"].insert(1, raw_entry("raw/raw"))
        FILES.validate_manifest(canonical(daily), limits(), profile="daily")

    def test_scope_selection_is_paper_source_raw_only(self):
        FILES.Root("raw", "/paper-scope/raw", 10001, 10001, 0o750, paper_raw_scopes=True)
        for kwargs in (dict(name="curated"), dict(leaf="manifests"), dict(name="base", parent=(0, 10001, 0o750))):
            values = dict(name="raw", path="/paper-scope/raw", uid=10001, gid=10001, mode=0o750,
                          paper_raw_scopes=True)
            values.update(kwargs)
            with self.subTest(kwargs=kwargs), self.assertRaisesRegex(FILES.SnapshotError, "^BACKUP_FILES_RAW_SCOPE$"):
                FILES.Root(**values)
        with self.assertRaisesRegex(FILES.SnapshotError, "^BACKUP_FILES_RAW_SCOPE$"):
            FILES.Root("raw", "/paper-scope/raw", 10001, 10001, 0o750, paper_raw_scopes=1)
        daily = [FILES.Root(name, "/paper-scope-daily/" + name, 0, 0, 0o700, paper_raw_scopes=name == "raw")
                 for name in FILES.CLASSES]
        with self.assertRaisesRegex(FILES.SnapshotError, "^BACKUP_FILES_RAW_SCOPE$"):
            FILES.Snapshot(daily, limits(), profile="daily")


UNSELECTED_RAW = ("provider=extra-a", "provider=extra-b", "provider=kis-daily-range-normalized",
                  ".calendar-bootstrap", "raw", "2026-01-01", "commit.lock")
UNSELECTED_MANIFESTS = ("provider=extra-a", "provider=extra-b", "provider=lock-only",
                        "provider=kis-daily-range-normalized", "commit.lock")


def build_selection_raw_root(raw, scopes):
    """Synthetic Raw root: selected scopes plus every refused shape, poisoned.

    `scopes` maps a provider to {relative path: bytes} below its Raw tree and
    manifest area (keys start with "tree/" or "manifest/"). Every unselected
    entry is made unreadable, a FIFO, a symlink or a noncanonical lock, so
    opening or descending into it fails the walk. Returns the paths to restore.
    """
    def put(path, data):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)

    for provider, members in scopes.items():
        for relative, data in members.items():
            area, rest = relative.split("/", 1)
            base = raw / ("provider=" + provider) if area == "tree" else raw / "manifests" / ("provider=" + provider)
            put(base / rest, data)
    for name in ("extra-a", "extra-b", "kis-daily-range-normalized"):
        put(raw / ("provider=" + name) / "market=kr/date=2026-11-06/batch=x/batch.json", b"{}")
        put(raw / "manifests" / ("provider=" + name) / "market=kr/manifest.jsonl", b"{}\n")
    put(raw / "manifests/provider=lock-only/market=kr/commit.lock", b"")
    put(raw / "manifests/commit.lock", b"")
    put(raw / "commit.lock", b"")
    put(raw / ".calendar-bootstrap/state.json", b"{}")
    put(raw / "raw/manifests/provider=kis/market=kr/manifest.jsonl", b"{}\n")
    (raw / "2026-01-01").mkdir()
    os.mkfifo(raw / "2026-01-01/pipe")
    os.symlink("/etc/passwd", raw / "raw/manifests/provider=kis/market=kr/link")
    poisoned = [raw / name for name in UNSELECTED_RAW if name != "commit.lock"]
    poisoned += [raw / "manifests" / name for name in UNSELECTED_MANIFESTS if name != "commit.lock"]
    poisoned += [raw / "commit.lock", raw / "manifests/commit.lock"]
    for path in poisoned:
        os.chmod(path, 0)
    return poisoned


class UnprivilegedRawSelection:
    """Real descriptors as an ordinary user: only ownership and root ancestry are shimmed.

    The Linux-root suite repeats the walk with real 10001 ownership and locks.
    """
    module = FILES

    def setUp(self):
        self.addCleanup(os.umask, os.umask(0o027))  # Raw modes: no group/other write
        self.temp = Path(tempfile.mkdtemp(prefix="paper-raw-scope-"))
        self.poisoned = []
        self.addCleanup(self.cleanup)
        files = self.module
        for patcher in (mock.patch.object(files, "OWNERS", files.OWNERS | {os.getuid(), os.getgid()}),
                        mock.patch.object(files, "open_root", self.open_root)):
            patcher.start()
            self.addCleanup(patcher.stop)

    def cleanup(self):
        for path in self.poisoned:
            os.chmod(path, 0o700)
        shutil.rmtree(self.temp)

    @staticmethod
    @contextlib.contextmanager
    def open_root(root):
        fd = os.open(root.path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
        try:
            yield fd
        finally:
            os.close(fd)

    def roots(self, scopes, *, selective=True):
        files = self.module
        roots = []
        for name in files.PAPER_CLASSES:
            path = self.temp / name
            path.mkdir()
            if name == "raw":
                self.poisoned += build_selection_raw_root(path, scopes)
            info = os.stat(path)
            roots.append(files.Root(name, str(path), info.st_uid, info.st_gid, stat.S_IMODE(info.st_mode),
                                    paper_raw_scopes=selective and name == "raw"))
        return tuple(roots)

    @contextlib.contextmanager
    def spied(self):
        files, calls = self.module, []
        real_open, real_stat = os.open, os.stat

        def spy_open(path, *args, **kwargs):
            calls.append(path)
            return real_open(path, *args, **kwargs)

        def spy_stat(path, *args, **kwargs):
            calls.append(path)
            return real_stat(path, *args, **kwargs)

        with mock.patch.object(files.os, "open", spy_open), mock.patch.object(files.os, "stat", spy_stat):
            yield calls


SELECTED_SCOPES = {
    "kis": {"manifest/market=kr/manifest.jsonl": b"{}\n",
            "tree/market=kr/date=2026-11-06/batch=a/batch.json": b"{}",
            "tree/market=kr/date=2026-11-06/batch=a/bars.json": b"bars"},
    "kis-daily-range": {"manifest/market=kr/manifest.jsonl": b"{}\n",
                        "tree/market=kr/date=2026-11-06/batch=b/batch.json": b"{}"},
}


class PaperRawSelectionWalkTests(UnprivilegedRawSelection, unittest.TestCase):
    def expected_inventory(self, scopes):
        expected = {"raw", "raw/manifests"}
        for provider, members in scopes.items():
            for relative in members:
                area, rest = relative.split("/", 1)
                base = ("raw/provider=" if area == "tree" else "raw/manifests/provider=") + provider
                parts = (base + "/" + rest).split("/")
                expected.update("/".join(parts[:n]) for n in range(2, len(parts) + 1))
        return expected

    def test_only_selected_scopes_are_captured_and_unselected_entries_never_opened(self):
        roots = self.roots(SELECTED_SCOPES)
        with self.spied() as calls:
            with FILES.Snapshot(roots, limits(), profile="paper-combined") as snapshot:
                snapshot.verify_unchanged()
                output = io.BytesIO()
                snapshot.write(output)
        inventory = {row["path"] for row in snapshot.records if row["path"].split("/")[0] == "raw"}
        self.assertEqual(inventory, self.expected_inventory(SELECTED_SCOPES))
        self.assertEqual({path for path in snapshot.identities if path.split("/")[0] == "raw"}, inventory)
        self.assertTrue(all(FILES.paper_raw_selected(path) for path in inventory))
        touched = set(calls)
        self.assertLessEqual({"manifests", "provider=kis", "provider=kis-daily-range", "bars.json"}, touched)
        self.assertFalse(touched & set(UNSELECTED_RAW), touched & set(UNSELECTED_RAW))
        self.assertFalse(touched & set(UNSELECTED_MANIFESTS))
        self.assertFalse(any(isinstance(name, str) and name in {"state.json", "pipe", "link"} for name in touched))
        manifest, _ = FILES.verify_archive(io.BytesIO(output.getvalue()), limits(), profile="paper-combined")
        self.assertEqual(manifest["entries"], snapshot.records)

    def test_absent_selected_scopes_are_not_an_error(self):
        roots = self.roots({})
        with FILES.Snapshot(roots, limits(), profile="paper-combined") as snapshot:
            pass
        self.assertEqual({row["path"] for row in snapshot.records if row["path"].split("/")[0] == "raw"},
                         {"raw", "raw/manifests"})

    def test_whole_root_walk_without_selection_still_reaches_the_refused_entries(self):
        roots = self.roots(SELECTED_SCOPES, selective=False)
        with self.assertRaises((FILES.SnapshotError, PermissionError)):
            with FILES.Snapshot(roots, limits(), profile="paper-combined"):
                pass

    def test_changes_are_fenced_only_inside_selected_scopes(self):
        roots = self.roots(SELECTED_SCOPES)
        extra = self.temp / "raw/provider=extra-a"
        os.chmod(extra, 0o700)
        self.poisoned.remove(extra)
        with FILES.Snapshot(roots, limits(), profile="paper-combined") as snapshot:
            (extra / "market=kr/date=2026-11-06/batch=x/batch.json").write_bytes(b"changed")
            snapshot.verify_unchanged()
        selected = self.temp / "raw/provider=kis/market=kr/date=2026-11-06/batch=a/bars.json"
        with self.assertRaisesRegex(FILES.SnapshotError, "^DAILY_BACKUP_FILES_CHANGED$"):
            with FILES.Snapshot(roots, limits(), profile="paper-combined") as snapshot:
                selected.write_bytes(b"changed bars")
                snapshot.verify_unchanged()

    def test_selected_scope_name_that_is_not_a_directory_fails(self):
        roots = self.roots(SELECTED_SCOPES)
        (self.temp / "raw/provider=krx").write_bytes(b"not a scope")
        with self.assertRaisesRegex(FILES.SnapshotError, "^DAILY_BACKUP_FILES_TYPE_OR_MODE$"):
            with FILES.Snapshot(roots, limits(), profile="paper-combined"):
                pass
        os.unlink(self.temp / "raw/provider=krx")
        os.symlink("provider=kis", self.temp / "raw/provider=krx")
        with self.assertRaisesRegex(FILES.SnapshotError, "^DAILY_BACKUP_FILES_TYPE_OR_MODE$"):
            with FILES.Snapshot(roots, limits(), profile="paper-combined"):
                pass


if __name__ == "__main__":
    unittest.main()
