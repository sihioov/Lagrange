#!/usr/bin/env python3
"""Focused synthetic checks for D4's opt-in initializer artifact binding."""

import copy
import json
from pathlib import Path
import subprocess
import tempfile
import tomllib
import unittest


ROOT = Path(__file__).resolve().parents[2]
HELPER = ROOT / "scripts/ops/lib/release-build-layout.sh"
LAYOUT_PATH = ROOT / "deploy/build/release-build-layout.json"
INITIALIZER_SOURCE = "crates/kis-client/src/bin/kis-market-stream-state.rs"
FEATURE = "market-stream-provisioning"


def write_json(path, value):
    path.write_text(json.dumps(value, sort_keys=True) + "\n", encoding="utf-8")


def run_helper(function, *args):
    command = [
        "/bin/sh", "-c",
        'helper=$1; shift; . "$helper"; function=$1; shift; "$function" "$@"',
        "market-stream-build-layout-self-test", str(HELPER), function,
        *(str(arg) for arg in args),
    ]
    return subprocess.run(
        command,
        cwd=ROOT,
        env={"PATH": "/usr/bin:/bin", "LANG": "C", "LC_ALL": "C"},
        check=False,
        capture_output=True,
        text=True,
        timeout=15,
    )


def request_for(layout, recipe_id):
    recipe = layout["recipes"][recipe_id]
    features = recipe.get("features", [])
    argv = ["cargo", "build", "--locked", "--release", "--package", recipe["package"]]
    if features:
        argv.extend(["--features", ",".join(features)])
    argv.append("--bin")
    return {
        "recipe": recipe_id,
        "package": recipe["package"],
        "bins": recipe["bins"],
        "compile_commit": recipe["compile_commit"],
        "clean_packages": recipe["clean_packages"],
        "features": features,
        "resolution_inputs": {
            "argv": argv,
            "default_features": True,
            "locked": True,
            "message_format": "json-render-diagnostics",
            "verbose": True,
        },
    }


def synthetic_metadata(layout, bad_source=False):
    packages = []
    for name, record in layout["packages"].items():
        dependencies = [
            {
                "name": dependency,
                "path": str((ROOT / layout["packages"][dependency]["path"]).resolve()),
                "kind": None,
            }
            for dependency in record["local_dependencies"]
        ]
        package = {
            "name": name,
            "manifest_path": str((ROOT / record["path"] / "Cargo.toml").resolve()),
            "dependencies": dependencies,
        }
        if name == "job-queue":
            alias_source = ROOT / INITIALIZER_SOURCE
            if bad_source:
                alias_source = ROOT / "crates/job-queue/src/bin/owner-equity-v2-runner.rs"
            package["targets"] = [
                {
                    "name": "owner-equity-v2-runner",
                    "kind": ["bin"],
                    "src_path": str((ROOT / "crates/job-queue/src/bin/owner-equity-v2-runner.rs").resolve()),
                    "required-features": [],
                },
                {
                    "name": "kis-market-stream-state",
                    "kind": ["bin"],
                    "src_path": str(alias_source.resolve()),
                    "required-features": [FEATURE],
                },
            ]
        packages.append(package)
    return {"packages": packages, "workspace_members": []}


class MarketStreamBuildLayoutSelfTest(unittest.TestCase):
    def setUp(self):
        self.layout = json.loads(LAYOUT_PATH.read_text(encoding="utf-8"))

    def layout_check(self, layout):
        with tempfile.TemporaryDirectory(prefix="wp6b-layout-") as directory:
            path = Path(directory) / "layout.json"
            write_json(path, layout)
            return run_helper("rbl_validate_feature_contract", "layout", path)

    def request_check(self, layout, request):
        with tempfile.TemporaryDirectory(prefix="wp6b-request-") as directory:
            layout_path = Path(directory) / "layout.json"
            request_path = Path(directory) / "request.json"
            write_json(layout_path, layout)
            write_json(request_path, request)
            return run_helper("rbl_validate_feature_contract", "request", layout_path, request_path)

    def test_d4_alias_target_binds_two_bins_feature_and_source(self):
        # Cargo accepts a pre-existing multiline inline dependency table that
        # this Python version's TOML parser does not. These assertions concern
        # only the alias/features tables; Cargo metadata validates the full file.
        manifest = (ROOT / "crates/job-queue/Cargo.toml").read_text(encoding="utf-8")
        self.assertEqual(manifest.count("\n[dependencies]\n"), 1)
        cargo = tomllib.loads(manifest.split("\n[dependencies]\n", 1)[0])
        self.assertEqual(cargo["features"][FEATURE], ["kis-client/market-stream-provisioning"])
        alias = next(item for item in cargo["bin"] if item["name"] == "kis-market-stream-state")
        self.assertEqual(alias["path"], "../kis-client/src/bin/kis-market-stream-state.rs")
        self.assertEqual(alias["required-features"], [FEATURE])
        d4 = self.layout["recipes"]["D4"]
        self.assertEqual(d4["bins"], ["owner-equity-v2-runner", "kis-market-stream-state"])
        self.assertEqual(d4["features"], [FEATURE])
        self.assertEqual(
            d4["runtime_binaries"][-1],
            {"bin": "kis-market-stream-state", "image": "/usr/local/bin/kis-market-stream-state"},
        )
        self.assertEqual(self.layout_check(self.layout).returncode, 0)
        with tempfile.TemporaryDirectory(prefix="wp6b-metadata-") as directory:
            layout_path = Path(directory) / "layout.json"
            metadata_path = Path(directory) / "metadata.json"
            write_json(layout_path, self.layout)
            write_json(metadata_path, synthetic_metadata(self.layout))
            valid = run_helper(
                "rbl_validate_d4_cargo_targets", metadata_path, layout_path, ROOT
            )
            self.assertEqual(valid.returncode, 0, valid.stderr)
            write_json(metadata_path, synthetic_metadata(self.layout, bad_source=True))
            invalid = run_helper(
                "rbl_validate_d4_cargo_targets", metadata_path, layout_path, ROOT
            )
            self.assertNotEqual(invalid.returncode, 0)

    def test_default_recipes_retain_empty_feature_selection(self):
        for recipe_id, recipe in self.layout["recipes"].items():
            if recipe_id != "D4":
                self.assertNotIn("features", recipe)
                self.assertEqual(recipe.get("features", []), [])
        self.assertEqual(self.layout["builder"]["features"], [])
        request = request_for(self.layout, "D1")
        self.assertNotIn("--features", request["resolution_inputs"]["argv"])
        self.assertEqual(self.request_check(self.layout, request).returncode, 0)

    def test_general_layout_failure_is_not_masked_by_valid_feature_selection(self):
        with tempfile.TemporaryDirectory(prefix="wp6b-general-layout-") as directory:
            path = Path(directory) / "layout.json"
            write_json(path, self.layout)
            self.assertEqual(run_helper("rbl_validate_layout", path).returncode, 0)
            altered = copy.deepcopy(self.layout)
            altered["guard_version"] = "invalid-guard"
            write_json(path, altered)
            self.assertEqual(self.layout_check(altered).returncode, 0)
            rejected = run_helper("rbl_validate_layout", path)
            self.assertNotEqual(rejected.returncode, 0)
            self.assertIn("layout-selection-invalid", rejected.stderr)

    def test_unknown_and_test_features_are_rejected(self):
        for features in (["unknown-feature"], ["market-stream-db-tests"], [FEATURE, FEATURE]):
            with self.subTest(features=features):
                layout = copy.deepcopy(self.layout)
                layout["recipes"]["D4"]["features"] = features
                self.assertNotEqual(self.layout_check(layout).returncode, 0)

    def test_request_must_match_trusted_recipe(self):
        request = request_for(self.layout, "D4")
        self.assertEqual(self.request_check(self.layout, request).returncode, 0)
        for mutate in (
            lambda value: value.update(features=[]),
            lambda value: value.update(package="api-server"),
            lambda value: value.update(bins=["owner-equity-v2-runner"]),
        ):
            altered = copy.deepcopy(request)
            mutate(altered)
            self.assertNotEqual(self.request_check(self.layout, altered).returncode, 0)

    def test_resolution_argv_must_include_only_the_canonical_feature_argument(self):
        request = request_for(self.layout, "D4")
        self.assertEqual(self.request_check(self.layout, request).returncode, 0)
        for argv in (
            ["cargo", "build", "--locked", "--release", "--package", "job-queue", "--bin"],
            ["cargo", "build", "--locked", "--release", "--package", "job-queue", "--bin", "--features", FEATURE],
            ["cargo", "build", "--locked", "--release", "--package", "job-queue", "--features", "market-stream-db-tests", "--bin"],
        ):
            altered = copy.deepcopy(request)
            altered["resolution_inputs"]["argv"] = argv
            self.assertNotEqual(self.request_check(self.layout, altered).returncode, 0)

    def test_producer_bundle_and_consumer_feature_mismatches_fail_closed(self):
        request = request_for(self.layout, "D4")
        recipe = self.layout["recipes"]["D4"]
        artifact = {"bin": recipe["bins"][0], "features": [FEATURE]}
        with tempfile.TemporaryDirectory(prefix="wp6b-artifacts-") as directory:
            layout_path = Path(directory) / "layout.json"
            request_path = Path(directory) / "request.json"
            artifact_path = Path(directory) / "artifact.json"
            write_json(layout_path, self.layout)
            write_json(request_path, request)
            write_json(artifact_path, artifact)
            valid_artifact = run_helper(
                "rbl_validate_feature_contract", "artifact", layout_path, request_path, artifact_path
            )
            self.assertEqual(valid_artifact.returncode, 0, valid_artifact.stderr)
            write_json(artifact_path, {"bin": recipe["bins"][0], "features": []})
            invalid_artifact = run_helper(
                "rbl_validate_feature_contract", "artifact", layout_path, request_path, artifact_path
            )
            self.assertNotEqual(invalid_artifact.returncode, 0)

            producer_root = Path(directory) / "producer"
            for bin_name in recipe["bins"]:
                artifact_dir = producer_root / bin_name
                artifact_dir.mkdir(parents=True)
                write_json(artifact_dir / "artifact.json", {"bin": bin_name, "features": [FEATURE]})
            bundle_path = Path(directory) / "bundle.json"
            write_json(bundle_path, {"bins": recipe["bins"], "features": [FEATURE]})
            valid_bundle = run_helper(
                "rbl_validate_feature_contract", "bundle", layout_path, request_path,
                bundle_path, producer_root,
            )
            self.assertEqual(valid_bundle.returncode, 0, valid_bundle.stderr)
            write_json(bundle_path, {"bins": recipe["bins"], "features": []})
            invalid_bundle = run_helper(
                "rbl_validate_feature_contract", "bundle", layout_path, request_path,
                bundle_path, producer_root,
            )
            self.assertNotEqual(invalid_bundle.returncode, 0)
            write_json(bundle_path, {"bins": recipe["bins"], "features": [FEATURE]})
            write_json(
                producer_root / recipe["bins"][1] / "artifact.json",
                {"bin": recipe["bins"][1], "features": []},
            )
            invalid_consumer = run_helper(
                "rbl_validate_feature_contract", "bundle", layout_path, request_path,
                bundle_path, producer_root,
            )
            self.assertNotEqual(invalid_consumer.returncode, 0)


if __name__ == "__main__":
    unittest.main()
