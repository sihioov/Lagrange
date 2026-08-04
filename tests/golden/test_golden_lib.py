"""Red/green unit tests for the golden-manifest core (scripts/golden/golden_lib.py).

Contract pinned here:
- canonical JSON bytes: key-order independent, byte-deterministic, compact ASCII.
- hash_file: JSON -> canonical-JSON hash; Parquet -> metadata-stripped canonical
  projection hash; anything else (incl. corrupt parquet) -> raw-bytes hash.
- field_diff: leaf-path field-level diffs for dict/list nesting, added/removed
  included, deterministic ordering, empty when equal.
- GoldenManifest versions MUST carry data/strategy/engine/code/config/seed/timezone.
- manifest serialization MUST be byte-deterministic (no timestamps).
- verify: PASS on unchanged tree; FAIL pinning the exact field path on mutation.
- evidence writer: sanitized, never contains secret/proprietary markers.
"""
from __future__ import annotations

import json
from pathlib import Path

import pytest

import golden_lib as gl


# --------------------------------------------------------------------------- #
# Canonical JSON hashing
# --------------------------------------------------------------------------- #

def test_canonical_json_stable_under_key_order() -> None:
    a = {"b": 1, "a": [2, 3], "c": {"z": True, "y": None}}
    b = {"c": {"y": None, "z": True}, "a": [2, 3], "b": 1}
    assert gl.canonical_json_bytes(a) == gl.canonical_json_bytes(b)
    assert gl.hash_bytes(gl.canonical_json_bytes(a)) == gl.hash_bytes(gl.canonical_json_bytes(b))


def test_canonical_json_detects_value_change() -> None:
    base = {"fills": [{"price": "10300.00", "quantity": 400}]}
    changed = {"fills": [{"price": "10400.00", "quantity": 400}]}
    assert gl.hash_bytes(gl.canonical_json_bytes(base)) != gl.hash_bytes(gl.canonical_json_bytes(changed))


def test_canonical_json_bytes_are_utf8_ascii() -> None:
    data = {"name": "코덱스 200", "symbol": "069500.KRX"}
    raw = gl.canonical_json_bytes(data)
    assert isinstance(raw, bytes)
    raw.decode("ascii")  # must not raise (ensure_ascii canonical form)


# --------------------------------------------------------------------------- #
# hash_file: json / binary / parquet
# --------------------------------------------------------------------------- #

def test_hash_file_json_reports_kind_and_stable(golden_tree: Path) -> None:
    bars = golden_tree / "fixtures" / "kr-etf" / "2020-01-31" / "bars.json"
    h1, meta1 = gl.hash_file(bars)
    h2, meta2 = gl.hash_file(bars)
    assert h1 == h2
    assert meta1["kind"] == "json"
    assert h1.startswith("sha256:")
    assert len(h1) == len("sha256:") + 64


def test_hash_file_json_canonical_across_key_order(golden_tree: Path, tmp_path: Path) -> None:
    src = golden_tree / "fixtures" / "kr-etf" / "2020-01-31" / "bars.json"
    data = json.loads(src.read_text(encoding="utf-8"))
    reordered = tmp_path / "reordered.json"
    # reverse top-level key order -> different file bytes, same canonical hash
    reordered.write_text(json.dumps(dict(reversed(list(data.items())))), encoding="utf-8")
    h_src, _ = gl.hash_file(src)
    h_ro, meta = gl.hash_file(reordered)
    assert h_src == h_ro
    assert meta["kind"] == "json"


def test_hash_file_binary_reports_kind_binary(golden_tree: Path) -> None:
    corrupt = golden_tree / "fixtures" / "kr-etf" / "variants" / "corrupt" / "corrupt_bars.bin"
    h1, meta1 = gl.hash_file(corrupt)
    h2, meta2 = gl.hash_file(corrupt)
    assert h1 == h2
    assert meta1["kind"] == "binary"


def test_hash_parquet_stable_across_rewrites(tmp_path: Path) -> None:
    pa = pytest.importorskip("pyarrow")
    pq = pytest.importorskip("pyarrow.parquet")
    table = pa.table({
        "instrument": pa.array(["069500.KRX", "069500.KRX"], pa.string()),
        "date": pa.array(["2020-01-31", "2020-02-03"], pa.string()),
        "close": pa.array([10250, 10380], pa.int64()),
    })
    a = tmp_path / "a.parquet"
    b = tmp_path / "b.parquet"
    pq.write_table(table, a, metadata_collector=None)
    # same logical data, different writer/metadata -> canonical hash must match
    pq.write_table(table, b, metadata_collector=None, store_schema=True)
    ha, meta_a = gl.hash_file(a)
    hb, meta_b = gl.hash_file(b)
    assert meta_a["kind"] == "parquet"
    assert meta_b["kind"] == "parquet"
    assert ha == hb


def test_hash_parquet_detects_value_change(tmp_path: Path) -> None:
    pa = pytest.importorskip("pyarrow")
    pq = pytest.importorskip("pyarrow.parquet")
    t1 = pa.table({"close": pa.array([10250, 10380], pa.int64())})
    t2 = pa.table({"close": pa.array([10250, 10400], pa.int64())})
    a = tmp_path / "a.parquet"
    b = tmp_path / "b.parquet"
    pq.write_table(t1, a)
    pq.write_table(t2, b)
    ha, _ = gl.hash_file(a)
    hb, _ = gl.hash_file(b)
    assert ha != hb


def test_hash_corrupt_parquet_falls_back_to_raw_bytes(golden_tree: Path) -> None:
    """Parquet magic but unreadable content must hash raw bytes deterministically."""
    corrupt = golden_tree / "fixtures" / "kr-etf" / "variants" / "corrupt" / "corrupt_bars.bin"
    h1, meta1 = gl.hash_file(corrupt)
    h2, meta2 = gl.hash_file(corrupt)
    assert h1 == h2
    assert meta1["kind"] == "binary"
    assert meta2["kind"] == "binary"
    assert "raw-bytes" in meta1["note"].lower()


def test_hash_file_missing_raises(golden_tree: Path) -> None:
    with pytest.raises(gl.GoldenManifestError):
        gl.hash_file(golden_tree / "fixtures" / "does-not-exist.json")


# --------------------------------------------------------------------------- #
# field_diff
# --------------------------------------------------------------------------- #

def test_field_diff_nested_changed_path(golden_tree: Path) -> None:
    old = {"fills": [{"price": "10300.00", "quantity": 400}]}
    new = {"fills": [{"price": "10400.00", "quantity": 400}]}
    diffs = gl.field_diff(old, new)
    assert len(diffs) == 1
    assert diffs[0].path == "fills[0].price"
    assert diffs[0].kind == "changed"
    assert diffs[0].old == "10300.00"
    assert diffs[0].new == "10400.00"
    assert "fills[0].price" in gl.render_diff(diffs[0])


def test_field_diff_added_and_removed() -> None:
    old = {"equity": {"cash": "1", "positions": "2"}}
    new = {"equity": {"cash": "1", "positions": "2", "fees_paid": "3"}}
    diffs = gl.field_diff(old, new)
    assert diffs == [gl.Diff("equity.fees_paid", "added", None, "3")]

    old2 = {"orders": [{"id": "a"}, {"id": "b"}]}
    new2 = {"orders": [{"id": "a"}]}
    diffs2 = gl.field_diff(old2, new2)
    assert diffs2 == [gl.Diff("orders[1]", "removed", {"id": "b"}, None)]


def test_field_diff_array_index_paths() -> None:
    old = {"fills": [{"price": "1"}, {"price": "2"}, {"price": "3"}]}
    new = {"fills": [{"price": "1"}, {"price": "9"}, {"price": "3"}]}
    diffs = gl.field_diff(old, new)
    assert [d.path for d in diffs] == ["fills[1].price"]
    assert diffs[0].new == "9"


def test_field_diff_empty_when_equal() -> None:
    doc = {"a": [1, 2, {"x": "y"}], "b": None, "c": True}
    assert gl.field_diff(doc, json.loads(json.dumps(doc))) == []


def test_field_diff_type_change_reported_at_path() -> None:
    old = {"metrics": {"sharpe": "0.00"}}
    new = {"metrics": {"sharpe": 0.0}}
    diffs = gl.field_diff(old, new)
    assert diffs == [gl.Diff("metrics.sharpe", "changed", "0.00", 0.0)]


def test_render_diff_is_deterministic_and_compact() -> None:
    d = gl.Diff("fills[0].price", "changed", "10300.00", "10400.00")
    assert gl.render_diff(d) == "fills[0].price: 10300.00 -> 10400.00"


# --------------------------------------------------------------------------- #
# GoldenManifest: schema, determinism, versions
# --------------------------------------------------------------------------- #

def test_manifest_has_all_version_fields(golden_tree: Path) -> None:
    config = gl.load_json_file(golden_tree / "golden" / "golden.json")
    code = {"commit": "0" * 40, "tree": "0" * 40}
    manifest = gl.manifest_from_config(config, golden_tree / "golden", code)
    assert set(manifest["versions"]) == {"data", "strategy", "engine", "code", "config", "seed", "timezone"}
    assert manifest["versions"]["seed"] == 42
    assert manifest["versions"]["timezone"] == "Asia/Seoul"
    assert manifest["versions"]["config"]["hash"].startswith("sha256:")
    assert set(manifest["versions"]["code"]) == {"commit", "tree"}


def test_manifest_serialization_byte_deterministic(golden_tree: Path) -> None:
    config = gl.load_json_file(golden_tree / "golden" / "golden.json")
    code = {"commit": "0" * 40, "tree": "0" * 40}
    m1 = gl.manifest_from_config(config, golden_tree / "golden", code)
    m2 = gl.manifest_from_config(config, golden_tree / "golden", code)
    assert gl.serialize_manifest(m1) == gl.serialize_manifest(m2)
    # no timestamps anywhere in the deterministic bytes
    assert b"generated_at" not in gl.serialize_manifest(m1)
    assert b"timestamp" not in gl.serialize_manifest(m1)


def test_manifest_embeds_json_content_and_binary_none(golden_tree: Path) -> None:
    config = gl.load_json_file(golden_tree / "golden" / "golden.json")
    manifest = gl.manifest_from_config(config, golden_tree / "golden", {"commit": "0" * 40, "tree": "0" * 40})
    by_path = {e["path"]: e for e in manifest["fixtures"]}
    assert "content" in by_path["../fixtures/kr-etf/2020-01-31/bars.json"]
    assert by_path["../fixtures/kr-etf/variants/corrupt/corrupt_bars.bin"].get("content") is None


# --------------------------------------------------------------------------- #
# verify_manifest
# --------------------------------------------------------------------------- #

def test_verify_ok_on_unchanged(golden_tree: Path) -> None:
    config = gl.load_json_file(golden_tree / "golden" / "golden.json")
    manifest = gl.manifest_from_config(config, golden_tree / "golden", {"commit": "0" * 40, "tree": "0" * 40})
    report = gl.verify_manifest(manifest, golden_tree / "golden")
    assert report.ok is True
    assert all(e.ok for e in report.artifacts)
    assert all(e.ok for e in report.fixtures)


def test_verify_detects_fill_price_field_change(golden_tree: Path) -> None:
    fills = golden_tree / "golden" / "outputs" / "2020-01-31" / "fills.json"
    data = json.loads(fills.read_text(encoding="utf-8"))
    data["fills"][0]["price"] = "10400.00"
    fills.write_text(json.dumps(data), encoding="utf-8")

    config = gl.load_json_file(golden_tree / "golden" / "golden.json")
    manifest = gl.manifest_from_config(config, golden_tree / "golden", {"commit": "0" * 40, "tree": "0" * 40})
    report = gl.verify_manifest(manifest, golden_tree / "golden")
    assert report.ok is False
    fill_entry = next(e for e in report.artifacts if e.category == "fill")
    assert not fill_entry.ok
    assert fill_entry.expected_sha256 != fill_entry.actual_sha256
    paths = [d.path for d in fill_entry.diffs]
    assert "fills[0].price" in paths
    rendered = [gl.render_diff(d) for d in fill_entry.diffs]
    assert any("10300.00 -> 10400.00" in r for r in rendered)


def test_verify_detects_fixture_field_change(golden_tree: Path) -> None:
    bars = golden_tree / "fixtures" / "kr-etf" / "2020-01-31" / "bars.json"
    data = json.loads(bars.read_text(encoding="utf-8"))
    data["bars"][0]["close"] = data["bars"][0]["close"] + 1
    bars.write_text(json.dumps(data), encoding="utf-8")

    config = gl.load_json_file(golden_tree / "golden" / "golden.json")
    manifest = gl.manifest_from_config(config, golden_tree / "golden", {"commit": "0" * 40, "tree": "0" * 40})
    report = gl.verify_manifest(manifest, golden_tree / "golden")
    assert report.ok is False
    bars_entry = next(e for e in report.fixtures if e.category == "data-bars")
    assert not bars_entry.ok
    assert any(d.path == "bars[0].close" for d in bars_entry.diffs)


def test_verify_missing_file_is_failure_not_crash(golden_tree: Path) -> None:
    config = gl.load_json_file(golden_tree / "golden" / "golden.json")
    manifest = gl.manifest_from_config(config, golden_tree / "golden", {"commit": "0" * 40, "tree": "0" * 40})
    (golden_tree / "golden" / "outputs" / "2020-01-31" / "metrics.json").unlink()
    report = gl.verify_manifest(manifest, golden_tree / "golden")
    assert report.ok is False
    metric = next(e for e in report.artifacts if e.category == "metric")
    assert not metric.ok
    assert "missing" in metric.note.lower()


# --------------------------------------------------------------------------- #
# Evidence writer
# --------------------------------------------------------------------------- #

SECRET_MARKERS = ("secret", "password", "token", "api_key", "apikey", "BEGIN PRIVATE KEY", "authorization")


def test_evidence_is_sanitized_no_secrets(golden_tree: Path, tmp_path: Path) -> None:
    config = gl.load_json_file(golden_tree / "golden" / "golden.json")
    manifest = gl.manifest_from_config(config, golden_tree / "golden", {"commit": "0" * 40, "tree": "0" * 40})
    report = gl.verify_manifest(manifest, golden_tree / "golden")
    out = tmp_path / "evidence.txt"
    gl.write_evidence(manifest, report, out)
    text = out.read_text(encoding="utf-8")
    assert "golden_id" in text and "kr-etf-2020-01-31-test" in text
    low = text.lower()
    for marker in SECRET_MARKERS:
        assert marker not in low, f"evidence leaked marker: {marker}"
    # evidence must not embed raw artifact payloads
    assert '"10300.00"' not in text.replace("fills[0].price: 10300.00 -> 10300.00", "")


def test_evidence_contains_failure_diffs(golden_tree: Path, tmp_path: Path) -> None:
    fills = golden_tree / "golden" / "outputs" / "2020-01-31" / "fills.json"
    data = json.loads(fills.read_text(encoding="utf-8"))
    data["fills"][0]["price"] = "10400.00"
    fills.write_text(json.dumps(data), encoding="utf-8")
    config = gl.load_json_file(golden_tree / "golden" / "golden.json")
    manifest = gl.manifest_from_config(config, golden_tree / "golden", {"commit": "0" * 40, "tree": "0" * 40})
    report = gl.verify_manifest(manifest, golden_tree / "golden")
    out = tmp_path / "evidence.txt"
    gl.write_evidence(manifest, report, out)
    text = out.read_text(encoding="utf-8")
    assert "VERDICT: FAIL" in text
    assert "fills[0].price" in text
