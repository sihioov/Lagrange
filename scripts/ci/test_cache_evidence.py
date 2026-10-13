from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from argparse import Namespace
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "ci" / "cache-evidence.py"
SPEC = importlib.util.spec_from_file_location("cache_evidence_under_test", SCRIPT)
assert SPEC and SPEC.loader
cache_evidence = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(cache_evidence)


class CacheEvidenceTests(unittest.TestCase):
    def make_record(
        self,
        directory: Path,
        name: str,
        *,
        sha: str = "a" * 40,
        namespace: str = "wp3-candidate",
        install: str = "10",
        build: str = "20",
        test: str = "30",
        hit: str = "false",
        matched: str = "",
        restore: str = "success",
        save: str = "success",
        path_bytes: str = "1024",
    ) -> Path:
        output = directory / f"{name}.json"
        summary = directory / f"{name}.md"
        summary.touch(mode=0o600)
        args = Namespace(
            workflow="CI",
            job="clippy",
            sha=sha,
            namespace=namespace,
            cache_entry=[
                f"cargo-target|cargo-target-key|{matched}|{hit}|{restore}|{save}|{path_bytes}"
            ],
            duration=[f"install={install}", f"build={build}", f"test={test}"],
            output=output,
            summary=summary,
        )
        cache_evidence.record(args)
        return output

    def test_restore_and_save_states_do_not_overclaim_warning_only_saves(self) -> None:
        primary = "cargo-key"
        self.assertEqual(
            cache_evidence.restore_state(
                hit="true", matched_key=primary, primary_key=primary, outcome="success"
            ),
            "exact",
        )
        self.assertEqual(
            cache_evidence.restore_state(
                hit="false", matched_key="cargo-key-old", primary_key=primary, outcome="success"
            ),
            "prefix",
        )
        self.assertEqual(
            cache_evidence.restore_state(
                hit="false", matched_key="", primary_key=primary, outcome="success"
            ),
            "miss",
        )
        self.assertEqual(
            cache_evidence.restore_state(
                hit="true", matched_key="", primary_key=primary, outcome="failure"
            ),
            "unknown",
        )
        self.assertEqual(cache_evidence.save_state("success"), "attempted_unverified")
        self.assertEqual(cache_evidence.save_state("failure"), "failed")
        self.assertEqual(cache_evidence.save_state("skipped"), "not_attempted")

    def test_record_sums_durations_and_marks_archive_bytes_unavailable(self) -> None:
        with tempfile.TemporaryDirectory(prefix="cache-evidence-") as temp:
            directory = Path(temp)
            output = directory / "job.json"
            summary = directory / "summary.md"
            summary.touch(mode=0o600)
            args = Namespace(
                workflow="CI",
                job="web",
                sha="b" * 40,
                namespace="epoch-2026.10",
                cache_entry=[
                    "npm-download|npm-key||false|success|success|4096",
                    "web-target|target-key|target-prefix-old|false|success|skipped|unavailable",
                ],
                duration=[
                    "install=12",
                    "install=8",
                    "build=45",
                    "test=10",
                    "test=5",
                ],
                output=output,
                summary=summary,
            )
            report = cache_evidence.record(args)
            self.assertEqual(report["durations_seconds"], {
                "install": 20,
                "build": 45,
                "test": 15,
                "total": 80,
                "measured_steps": {"install": 2, "build": 1, "test": 2},
            })
            self.assertEqual([item["restore"] for item in report["caches"]], ["miss", "prefix"])
            self.assertEqual(report["caches"][0]["save"], "attempted_unverified")
            self.assertEqual(report["caches"][0]["bytes"], {"path": 4096, "archive": None})
            self.assertEqual(report["caches"][1]["bytes"], {"path": None, "archive": None})
            self.assertEqual(output.stat().st_mode & 0o777, 0o600)
            summary_text = summary.read_text(encoding="utf-8")
            self.assertIn("attempted_unverified", summary_text)
            self.assertIn('"total": 80', summary_text)

    def test_duration_total_is_unavailable_when_any_required_stage_was_not_measured(self) -> None:
        totals = cache_evidence.duration_totals(
            ["install=unavailable", "build=10", "test=3", "test=unavailable"]
        )
        self.assertIsNone(totals["install"])
        self.assertIsNone(totals["test"])
        self.assertIsNone(totals["total"])
        self.assertEqual(totals["build"], 10)

    def test_namespace_and_cache_fields_reject_unsafe_or_missing_observations(self) -> None:
        for namespace in ("", "époch", "a\nvalue", "$(touch marker)", "a" * 65):
            with self.subTest(namespace=namespace), self.assertRaises(cache_evidence.EvidenceError):
                cache_evidence.validate_namespace(namespace)
        self.assertEqual(cache_evidence.restore_state(hit="true", matched_key="", primary_key="key", outcome="success"), "unknown")
        for raw in ("bad", "cargo|key||invalid|success|success|0", "cargo|key||false|success|success|-1"):
            with self.subTest(raw=raw), self.assertRaises(cache_evidence.EvidenceError):
                cache_evidence.parse_cache_entry(raw)

    def test_compare_rejects_changed_compatibility_and_corrupt_duration_totals(self) -> None:
        with tempfile.TemporaryDirectory(prefix="cache-evidence-invalid-") as temp:
            directory = Path(temp)
            cold = self.make_record(directory, "cold")
            warm = self.make_record(directory, "warm")
            value = json.loads(warm.read_text())
            value["caches"][0]["key"] = "different-toolchain-key"
            warm.write_text(json.dumps(value))
            with self.assertRaisesRegex(cache_evidence.EvidenceError, "same compatibility"):
                cache_evidence.compare(cold, warm)
            for mutation in ("missing", "incorrect", "boolean"):
                value = json.loads(cold.read_text())
                if mutation == "missing":
                    value["durations_seconds"].pop("test")
                elif mutation == "incorrect":
                    value["durations_seconds"]["total"] += 1
                else:
                    value["durations_seconds"]["build"] = True
                warm.write_text(json.dumps(value))
                with self.subTest(mutation=mutation), self.assertRaises(cache_evidence.EvidenceError):
                    cache_evidence.compare(cold, warm)

    def test_compare_rejects_malformed_missing_and_mismatched_sha(self) -> None:
        with tempfile.TemporaryDirectory(prefix="cache-evidence-compare-") as temp:
            directory = Path(temp)
            cold = self.make_record(directory, "cold")
            warm = self.make_record(directory, "warm", hit="true", matched="cargo-target-key")

            malformed = directory / "malformed.json"
            malformed.write_text("{not-json", encoding="utf-8")
            with self.assertRaisesRegex(cache_evidence.EvidenceError, "cannot read valid"):
                cache_evidence.compare(malformed, warm)

            missing_sha = json.loads(cold.read_text(encoding="utf-8"))
            missing_sha.pop("sha")
            missing_sha_path = directory / "missing-sha.json"
            missing_sha_path.write_text(json.dumps(missing_sha), encoding="utf-8")
            with self.assertRaisesRegex(cache_evidence.EvidenceError, "complete 40-character"):
                cache_evidence.compare(missing_sha_path, warm)

            mismatch = self.make_record(directory, "mismatch", sha="c" * 40)
            with self.assertRaisesRegex(cache_evidence.EvidenceError, "same workflow, job, SHA, and namespace"):
                cache_evidence.compare(cold, mismatch)

            malformed_sha = json.loads(warm.read_text(encoding="utf-8"))
            malformed_sha["sha"] = "not-a-sha"
            malformed_sha_path = directory / "malformed-sha.json"
            malformed_sha_path.write_text(json.dumps(malformed_sha), encoding="utf-8")
            with self.assertRaisesRegex(cache_evidence.EvidenceError, "complete 40-character"):
                cache_evidence.compare(cold, malformed_sha_path)

    def test_compare_requires_same_namespace_and_reports_duration_delta(self) -> None:
        with tempfile.TemporaryDirectory(prefix="cache-evidence-delta-") as temp:
            directory = Path(temp)
            cold = self.make_record(directory, "cold", install="100", build="200", test="50")
            warm = self.make_record(
                directory,
                "warm",
                install="20",
                build="40",
                test="10",
                hit="true",
                matched="cargo-target-key",
            )
            result = cache_evidence.compare(cold, warm)
            self.assertEqual(result["durations_seconds"]["cold"]["total"], 350)
            self.assertEqual(result["durations_seconds"]["warm"]["total"], 70)
            self.assertEqual(result["durations_seconds"]["delta_total"], -280)
            self.assertEqual(result["caches"][0]["warm_restore"], "exact")

            other_epoch = self.make_record(directory, "other-epoch", namespace="another-epoch")
            with self.assertRaisesRegex(cache_evidence.EvidenceError, "same workflow, job, SHA, and namespace"):
                cache_evidence.compare(cold, other_epoch)


if __name__ == "__main__":
    unittest.main()
