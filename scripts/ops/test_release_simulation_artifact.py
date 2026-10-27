#!/usr/bin/env python3
"""Offline regressions for the closed D8 producer, guard and image contract."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import struct
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
HELPER = ROOT / "scripts/ops/lib/release-build-layout.sh"
LAYOUT_PATH = ROOT / "deploy/build/release-build-layout.json"
LAYOUT = json.loads(LAYOUT_PATH.read_text(encoding="utf-8"))
# Canonical pre-D8 layout; independent of checkout history and commit position.
LEGACY_LAYOUT_SHA256 = "1a2337572d3743bbd4d85e78a97450dd6f5a559394abedb59945e4df668c1160"
RUNTIME_FORMAT = "lagrange-runtime-payload-inventory-v1"
DOCKERIGNORE_SHA256 = "0b69fbfaf5417fdd0be8ef3bb4b6cf7eaebffb16a1883809e9fe21713adf863e"
D8_BIN = "owner-beta-simulation-runner"
D8_ENTRY = "deploy/runtime/owner-beta-simulation-entrypoint"
D8_IMAGE_ENTRY = "/usr/local/bin/owner-beta-simulation-entrypoint"
D8_GUARD_SIDECAR_ROOT = "/provenance/owner-beta-simulation-runner"
D8_SIDECAR_ROOT = "/usr/share/lagrange/owner-beta-simulation"
SIMULATION_ENV_NAMES = (
    "R4_SIMULATION_BUILD_GIT_COMMIT",
    "R4_SIMULATION_BUILD_TOOLCHAIN",
    "R4_SIMULATION_BUILD_TARGET",
    "R4_SIMULATION_BUILD_PROFILE",
    "R4_SIMULATION_BUILD_WORKFLOW_RUN_ID",
    "R4_SIMULATION_BUILD_WORKFLOW_RUN_ATTEMPT",
    "R4_SIMULATION_BUILD_ORIGIN",
    "R4_SIMULATION_BUILD_SOURCE_INPUT_SHA256",
    "R4_SIMULATION_BUILD_RECIPE_SHA256",
)


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def canonical(value: object) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode("utf-8")


def write_bytes(path: Path, value: bytes, mode: int) -> None:
    path.parent.mkdir(mode=0o755, parents=True, exist_ok=True)
    path.write_bytes(value)
    path.chmod(mode)


def write_json(path: Path, value: object, mode: int) -> bytes:
    raw = canonical(value)
    write_bytes(path, raw, mode)
    return raw


def extract_python(function_name: str) -> str:
    source = HELPER.read_text(encoding="utf-8")
    starts = list(re.finditer(r"(?m)^" + re.escape(function_name) + r"\(\) \{\n", source))
    if len(starts) != 1:
        raise AssertionError(f"expected one function definition: {function_name}")
    start = starts[0].end()
    following = re.search(r"(?m)^[A-Za-z_][A-Za-z0-9_]*\(\) \{\n", source[start:])
    end = start + following.start() if following else len(source)
    block = source[start:end]
    bodies = re.findall(r"(?ms)^[^\n]*python3 - <<'PY'[^\n]*\n(.*?)\nPY(?=\n|$)", block)
    if len(bodies) != 1:
        raise AssertionError(f"expected one embedded Python body in {function_name}, got {len(bodies)}")
    return bodies[0]


def helper_call(script: str, *args: object, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[bytes]:
    process_env = dict(os.environ, GIT_OPTIONAL_LOCKS="0")
    if env:
        process_env.update(env)
    return subprocess.run(
        ["bash", "-c", script, "fixture", str(HELPER), *(str(arg) for arg in args)],
        cwd=ROOT,
        env=process_env,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )


def run_python_body(body: str, env: dict[str, str]) -> subprocess.CompletedProcess[bytes]:
    with tempfile.TemporaryDirectory(prefix="wp5b2a-python-") as temporary:
        script = Path(temporary) / "embedded.py"
        script.write_text(body, encoding="utf-8")
        return subprocess.run(
            ["python3", "-B", str(script)],
            cwd=ROOT,
            env=dict(os.environ, GIT_OPTIONAL_LOCKS="0", **env),
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )


def h_digest(request: dict[str, object]) -> str:
    material = {
        "format": "lagrange-build-layout-h-v1",
        "k_sha256": request["k_sha256"],
        "package": request["package"],
        "bins": request["bins"],
        "recipe": request["recipe"],
        "recipe_sha256": request["recipe_sha256"],
        "transitive_package_hashes": request["transitive_package_hashes"],
        "resolution_inputs": request["resolution_inputs"],
        "compile_env": request["compile_env"],
        "features": request["features"],
        "profile": request["profile"],
        "platform": request["platform"],
        "host_triple": request["host_triple"],
    }
    return sha256_bytes(json.dumps(material, sort_keys=True, separators=(",", ":")).encode("utf-8"))


def package_closure(layout: dict[str, object], name: str) -> list[str]:
    packages = layout["packages"]
    result: set[str] = set()

    def visit(current: str, visiting: set[str]) -> None:
        if current in visiting:
            raise AssertionError("fixture package graph contains a cycle")
        if current in result:
            return
        visiting.add(current)
        for dependency in packages[current]["local_dependencies"]:
            visit(dependency, visiting)
        visiting.remove(current)
        result.add(current)

    visit(name, set())
    return [name] + sorted(result - {name})


def make_request(
    layout: dict[str, object],
    recipe_id: str,
    helper_hash: str,
    layout_hash: str,
    *,
    commit: str = "d" * 40,
    input_hash: str = "a" * 64,
    recipe_hash: str = "b" * 64,
) -> dict[str, object]:
    recipe = layout["recipes"][recipe_id]
    package_hashes = {name: sha256_bytes(name.encode("ascii")) for name in layout["packages"]}
    features = recipe.get("features", [])
    resolution_argv = ["cargo", "build", "--locked", "--release", "--package", recipe["package"]]
    if features:
        resolution_argv.extend(["--features", ",".join(features)])
    resolution_argv.append("--bin")
    compile_env = {
        "CARGO_BUILD_JOBS": "2",
        "CARGO_TARGET_DIR": "/cargo-target",
        "RUSTFLAGS": "<unset>",
        "LAGRANGE_CODE_COMMIT": commit if recipe["compile_commit"] == "present" else "<unset>",
    }
    if recipe_id == "D8":
        compile_env.update(
            {
                "R4_SIMULATION_BUILD_GIT_COMMIT": commit,
                "R4_SIMULATION_BUILD_TOOLCHAIN": "1.97.1",
                "R4_SIMULATION_BUILD_TARGET": "x86_64-unknown-linux-musl",
                "R4_SIMULATION_BUILD_PROFILE": "release",
                "R4_SIMULATION_BUILD_ORIGIN": "official-release-v1",
                "R4_SIMULATION_BUILD_SOURCE_INPUT_SHA256": input_hash,
                "R4_SIMULATION_BUILD_RECIPE_SHA256": recipe_hash,
            }
        )
    request: dict[str, object] = {
        "format": "lagrange-rust-artifact-request-v1",
        "recipe": recipe_id,
        "package": recipe["package"],
        "bins": recipe["bins"],
        "compile_commit": recipe["compile_commit"],
        "source_commit": commit,
        "platform": "linux/amd64",
        "host_triple": "x86_64-unknown-linux-musl",
        "profile": "release",
        "features": features,
        "compile_env": compile_env,
        "clean_packages": recipe["clean_packages"],
        "package_hashes": package_hashes,
        "transitive_package_hashes": [
            {"package": name, "p_sha256": package_hashes[name]}
            for name in package_closure(layout, recipe["package"])
        ],
        "resolution_inputs": {
            "argv": resolution_argv,
            "default_features": True,
            "locked": True,
            "message_format": "json-render-diagnostics",
            "verbose": True,
        },
        "input_sha256": input_hash,
        "recipe_sha256": recipe_hash,
        "k_sha256": "c" * 64,
        "cache_key": "c" * 64,
        "helper_sha256": helper_hash,
        "layout_sha256": layout_hash,
        "guard_version": "common-1",
        "cache_namespace": "product",
        "payloads": [
            {"source": item["source"], "image": item["image"]}
            for item in layout["runtime_payloads"].get(recipe.get("runtime_payload", ""), [])
        ],
    }
    request["h_sha256"] = h_digest(request)
    return request


def make_elf_fixture(name: str) -> bytes:
    header = bytearray(64)
    header[:4] = b"\x7fELF"
    header[4] = 2
    header[5] = 1
    header[6] = 1
    struct.pack_into("<H", header, 18, 62)
    return bytes(header) + b" synthetic bytes only " + name.encode("ascii") + b"\n"


class RequestSourceFixture:
    """Temporary tracked input tree for the real context request generator."""

    def __init__(self, root: Path):
        self.root = root
        self.layout_path = root / "deploy/build/release-build-layout.json"
        self.layout_path.parent.mkdir(mode=0o755, parents=True)
        write_bytes(self.layout_path, LAYOUT_PATH.read_bytes(), 0o644)
        write_bytes(root / ".dockerignore", (ROOT / ".dockerignore").read_bytes(), 0o644)
        write_bytes(root / "scripts/ops/lib/release-build-layout.sh", HELPER.read_bytes(), 0o755)
        write_bytes(root / "deploy/build/Dockerfile.rust-artifacts", b"FROM scratch\n", 0o644)

        for relative in LAYOUT["source_inventory"]["root_files"]:
            path = root / relative
            if not path.exists():
                write_bytes(path, ("fixture " + relative + "\n").encode(), 0o644)
        for relative in LAYOUT["source_inventory"]["package_dirs"]:
            write_bytes(root / relative / "Cargo.toml", ("fixture " + relative + "\n").encode(), 0o644)
        for relative in LAYOUT["source_inventory"]["external_compile_inputs"]:
            if relative == "migrations":
                write_bytes(root / relative / "0001_fixture.sql", b"-- synthetic fixture\n", 0o644)
            else:
                write_bytes(root / relative, ("fixture " + relative + "\n").encode(), 0o644)
        write_bytes(
            root / LAYOUT["recipes"]["D8"]["dockerfile"],
            b"FROM scratch AS synthetic-d8-recipe\n",
            0o644,
        )
        environment = dict(os.environ, GIT_OPTIONAL_LOCKS="0")
        subprocess.run(["git", "init", "--quiet", str(root)], cwd=ROOT, env=environment, check=True)
        subprocess.run(["git", "-C", str(root), "config", "user.name", "WP-5B2A fixture"], env=environment, check=True)
        subprocess.run(
            ["git", "-C", str(root), "config", "user.email", "wp-5b2a-fixture@example.invalid"],
            env=environment,
            check=True,
        )
        subprocess.run(["git", "-C", str(root), "add", "-A"], env=environment, check=True)
        subprocess.run(
            ["git", "-C", str(root), "commit", "--quiet", "-m", "synthetic WP-5B2A inputs"],
            env=environment,
            check=True,
        )

    def shell_value(self, script: str, *args: object) -> str:
        result = helper_call(script, self.root, self.layout_path)
        if result.returncode != 0:
            raise AssertionError(f"helper value command failed: {result.stderr.decode(errors='replace')}")
        return result.stdout.decode("ascii").strip()

    def source_hash(self) -> str:
        return self.shell_value('. "$1"; rbl_hash_source "$2" "$3" 1')

    def recipe_hash(self) -> str:
        result = helper_call('. "$1"; rbl_recipe_hash "$2" D8 "$3"', self.root, self.layout_path)
        if result.returncode != 0:
            raise AssertionError(f"recipe hash failed: {result.stderr.decode(errors='replace')}")
        return result.stdout.decode("ascii").strip()

    def package_hashes(self) -> str:
        result = helper_call('. "$1"; rbl_package_hashes "$2" "$3" 1', self.root, self.layout_path)
        if result.returncode != 0:
            raise AssertionError(f"package hash failed: {result.stderr.decode(errors='replace')}")
        return result.stdout.decode("utf-8").strip()

    def generate(self, recipe_id: str, source_hash: str, recipe_hash: str, commit: str, label: str) -> Path:
        partial = self.root / "synthetic-contexts" / label
        partial.mkdir(mode=0o700, parents=True)
        helper_hash = sha256_bytes(HELPER.read_bytes())
        layout_hash = sha256_bytes(self.layout_path.read_bytes())
        context_hash = "e" * 64
        variables = {
            "RBL_SOURCE_ROOT": str(self.root),
            "RBL_LAYOUT": str(self.layout_path),
            "RBL_PARTIAL": str(partial),
            "RBL_NATIVE_IDENTITY": str(self.root / "state/native-identity.json"),
            "RBL_RECIPE": recipe_id,
            "RBL_COMMIT": commit,
            "RBL_SOURCE_HASH": source_hash,
            "RBL_RECIPE_HASH": recipe_hash,
            "RBL_HELPER_HASH": helper_hash,
            "RBL_LAYOUT_HASH": layout_hash,
            "RBL_NAMESPACE": "product",
            "RBL_K_HASH": context_hash,
            "RBL_CACHE_KEY": context_hash,
            "RBL_PACKAGE_HASHES": self.package_hashes(),
        }
        result = run_python_body(extract_python("rbl_make_context"), variables)
        if result.returncode != 0:
            raise AssertionError(f"request generator failed: {result.stderr.decode(errors='replace')}")
        return partial / ".release-build/request.json"


def write_native_identity(path: Path) -> None:
    inventory = [
        {"name": "build-base", "version": "0.5-r4", "provides": [], "status": ["installed"]},
        {"name": "musl-dev", "version": "1.2.6-r2", "provides": [], "status": ["installed"]},
        {"name": "openssl-dev", "version": "3.5.8-r0", "provides": [], "status": ["installed"]},
        {"name": "pkgconf", "version": "2.5.1-r0", "provides": [], "status": ["installed"]},
        {
            "name": "postgresql18-dev",
            "version": "18.6-r0",
            "provides": ["postgresql-dev"],
            "status": ["installed"],
        },
    ]
    write_json(
        path,
        {
            "format": "lagrange-build-layout-native-v3",
            "target_platform": "linux/amd64",
            "host_triple": "x86_64-unknown-linux-musl",
            "rustc_vv": "rustc 1.97.1\nhost: x86_64-unknown-linux-musl\n",
            "cargo_version": "cargo 1.97.1 (fixture)",
            "apk_info_vv": "synthetic package inventory\n",
            "apk_installed_packages": inventory,
            "native_packages": ["build-base", "musl-dev", "openssl-dev", "pkgconf", "postgresql-dev"],
            "compiler_env": {
                "CARGO_BUILD_JOBS": "2",
                "CARGO_TARGET_DIR": "/cargo-target",
                "RUSTFLAGS": "<unset>",
            },
        },
        0o600,
    )


class D8BundleFixture:
    def __init__(self, root: Path):
        self.root = root
        self.bundle = root / "bundle"
        self.output = root / "guard-output"
        self.layout = LAYOUT
        self.release = self.bundle / ".release-build"
        self.request_path = self.release / "request.json"
        self.producer = self.release / "producer" / D8_BIN
        self.artifact_path = self.producer / "artifact.json"
        self.producer_complete_path = self.producer / "COMPLETE"
        self.binary = make_elf_fixture(D8_BIN)
        helper_hash = sha256_bytes(HELPER.read_bytes())
        layout_hash = sha256_bytes(LAYOUT_PATH.read_bytes())
        self.request = make_request(LAYOUT, "D8", helper_hash, layout_hash)
        self.bundle.mkdir(mode=0o755, parents=True)
        self.release.mkdir(mode=0o755)
        self.producer.mkdir(mode=0o755, parents=True)
        write_bytes(self.release / "release-build-layout.json", LAYOUT_PATH.read_bytes(), 0o644)
        write_bytes(self.release / "release-build-layout.sh", HELPER.read_bytes(), 0o755)
        write_json(self.request_path, self.request, 0o600)
        write_native_identity(self.release / "native-identity.json")
        write_bytes(self.bundle / "target/release" / D8_BIN, self.binary, 0o755)
        write_bytes(self.producer / "bin" / D8_BIN, self.binary, 0o755)
        entry = b"#!/bin/sh\n# synthetic entrypoint bytes; never executed\nexit 0\n"
        write_bytes(self.bundle / D8_ENTRY, entry, 0o755)
        self.entry_bytes = entry
        self.write_artifact()
        write_bytes(
            self.producer / "cargo.jsonl",
            canonical(
                {
                    "reason": "compiler-artifact",
                    "target": {"name": D8_BIN},
                    "executable": "/cargo-target/release/" + D8_BIN,
                }
            ),
            0o644,
        )
        write_bytes(self.producer / "cargo.stderr", b"Compiling synthetic fixture\n", 0o644)
        write_json(
            self.producer / "cargo-summary.json",
            {
                "format": "lagrange-cargo-summary-v1",
                "cargo_success": True,
                "observed_bin": D8_BIN,
                "package": "job-queue",
                "fresh_count": 0,
                "compiling_count": 1,
                "executable": "/cargo-target/release/" + D8_BIN,
                "cargo_ms": 1,
            },
            0o644,
        )
        write_json(self.producer / "timing.json", {"format": "lagrange-cargo-timing-v1", "cargo_ms": 1}, 0o644)
        tree_record = f"f\t0755\t{D8_ENTRY}\t{sha256_bytes(entry)}\n".encode("utf-8")
        payloads = [
            {
                "source": D8_ENTRY,
                "image": D8_IMAGE_ENTRY,
                "tree_sha256": sha256_bytes(tree_record),
            }
        ]
        request_payloads = self.request["payloads"]
        if request_payloads != [{"source": D8_ENTRY, "image": D8_IMAGE_ENTRY}]:
            raise AssertionError("fixture request payload did not bind the D8 entrypoint")
        recipe = LAYOUT["recipes"]["D8"]
        write_json(
            self.release / "bundle.json",
            {
                "format": "lagrange-rust-artifact-bundle-v1",
                "recipe": "D8",
                "bins": recipe["bins"],
                "source_commit": self.request["source_commit"],
                "source_input_sha256": self.request["input_sha256"],
                "recipe_sha256": self.request["recipe_sha256"],
                "h_sha256": self.request["h_sha256"],
                "helper_sha256": self.request["helper_sha256"],
                "layout_sha256": self.request["layout_sha256"],
                "k_sha256": self.request["k_sha256"],
                "cache_key": self.request["cache_key"],
                "cache_namespace": self.request["cache_namespace"],
                "guard_version": "common-1",
                "platform": "linux/amd64",
                "host_triple": "x86_64-unknown-linux-musl",
                "profile": "release",
                "features": [],
                "compile_env": self.request["compile_env"],
                "payloads": payloads,
            },
            0o600,
        )
        self.reseal_bundle()

    def write_artifact(self, transform=None) -> None:
        record = {
            "format": "lagrange-rust-artifact-v1",
            "source_commit": self.request["source_commit"],
            "package": "job-queue",
            "bin": D8_BIN,
            "platform": "linux/amd64",
            "host_triple": "x86_64-unknown-linux-musl",
            "profile": "release",
            "features": [],
            "compile_env": self.request["compile_env"],
            "cache_key": self.request["cache_key"],
            "input_sha256": self.request["input_sha256"],
            "recipe_sha256": self.request["recipe_sha256"],
            "h_sha256": self.request["h_sha256"],
            "cargo_success": True,
            "binary_mode": "0755",
            "binary_sha256": sha256_bytes(self.binary),
        }
        if transform:
            transform(record)
        raw = write_json(self.artifact_path, record, 0o644)
        write_bytes(self.producer_complete_path, (sha256_bytes(raw) + "\n").encode("ascii"), 0o644)

    def reseal_bundle(self) -> None:
        result = helper_call('. "$1"; rbl_bundle_tree_hash "$2"', self.bundle)
        if result.returncode != 0:
            raise AssertionError(f"fixture bundle tree hash failed: {result.stderr.decode(errors='replace')}")
        write_bytes(self.release / "complete", result.stdout.strip() + b"\n", 0o600)

    def validate_producer(self) -> subprocess.CompletedProcess[bytes]:
        environment = dict(os.environ, RELEASE_BUILD_LAYOUT_SOURCE_ROOT=str(ROOT), GIT_OPTIONAL_LOCKS="0")
        return helper_call(
            '. "$1"; rbl_verify_producer "$2" D8 "$3" "$4"',
            self.producer,
            D8_BIN,
            self.request_path,
            env=environment,
        )

    def guard(self) -> subprocess.CompletedProcess[bytes]:
        return helper_call(
            '. "$1"; rbl_guard_build "$2" "$3" "$4"',
            self.request_path,
            self.bundle,
            self.output,
            env={"RUST_ARTIFACT_HELPER_SHA256": self.request["helper_sha256"]},
        )


class LayoutAndRequestTests(unittest.TestCase):
    def test_layout_is_closed_d8_and_preserves_existing_recipes(self) -> None:
        legacy_layout = json.loads(json.dumps(LAYOUT))
        del legacy_layout["runtime_payloads"]["simulation"]
        del legacy_layout["recipes"]["D8"]
        self.assertEqual(
            legacy_layout["services"].pop(D8_BIN), {"kind": "rust", "recipe": "D8"}
        )
        self.assertEqual(
            legacy_layout["batches"].pop(),
            {"id": "B5", "services": ["paper-scheduler", D8_BIN]},
        )
        legacy_layout["batches"][3]["services"].append("paper-scheduler")
        self.assertEqual(legacy_layout["gates"]["consumer_batches"].pop(), "B5")
        self.assertEqual(sha256_bytes(canonical(legacy_layout)), LEGACY_LAYOUT_SHA256)
        self.assertEqual(set(LAYOUT["recipes"]), {"D1", "D2", "D3", "D4", "D5", "D6", "D7", "D8"})
        self.assertEqual(
            LAYOUT["recipes"]["D3"],
            {
                "dockerfile": "crates/job-queue/Dockerfile.owner-beta-runner",
                "package": "job-queue",
                "bins": ["owner-beta-runner"],
                "compile_commit": "present",
                "external_inputs": ["configs/universes/kr-etf-core-v1.yaml"],
                "clean_packages": [
                    "auth",
                    "collectors",
                    "data-go-client",
                    "domain",
                    "factor-engine",
                    "job-queue",
                    "kis-client",
                    "market-data",
                    "opendart-client",
                    "portfolio-model",
                    "selector",
                ],
                "runtime_binaries": [
                    {"bin": "owner-beta-runner", "image": "/usr/local/bin/owner-beta-runner"}
                ],
                "checkpoints": [],
            },
        )
        self.assertEqual(LAYOUT["recipes"]["D4"]["features"], ["market-stream-provisioning"])
        self.assertEqual(LAYOUT["recipes"]["D2"]["runtime_payload"], "backtest")
        self.assertEqual(LAYOUT["recipes"]["D5"]["runtime_payload"], "backtest")
        self.assertEqual(LAYOUT["recipes"]["D6"]["runtime_payload"], "collectors")
        self.assertEqual(LAYOUT["recipes"]["D7"]["runtime_payload"], "paper")
        self.assertEqual(LAYOUT["recipes"]["D8"], {
            "dockerfile": "crates/job-queue/Dockerfile.owner-beta-simulation-runner",
            "package": "job-queue",
            "bins": [D8_BIN],
            "features": [],
            "compile_commit": "present",
            "external_inputs": ["configs/universes/kr-etf-core-v1.yaml"],
            "clean_packages": LAYOUT["recipes"]["D3"]["clean_packages"],
            "runtime_binaries": [{"bin": D8_BIN, "image": "/usr/local/bin/owner-beta-simulation-runner"}],
            "runtime_payload": "simulation",
            "checkpoints": [],
        })
        self.assertEqual(len([name for recipe in LAYOUT["recipes"].values() for name in recipe["bins"]]), 19)
        self.assertEqual(len(LAYOUT["services"]), 13)
        self.assertEqual([batch["id"] for batch in LAYOUT["batches"]], ["B1", "B2", "B3", "B4", "B5"])
        self.assertEqual(
            LAYOUT["batches"][3:],
            [
                {"id": "B4", "services": ["nt-backtest-worker-1", "nt-backtest-worker-2"]},
                {"id": "B5", "services": ["paper-scheduler", "owner-beta-simulation-runner"]},
            ],
        )
        self.assertEqual(LAYOUT["gates"]["consumer_batches"], ["B1", "B2", "B3", "B4", "B5"])
        self.assertEqual(LAYOUT["runtime_payloads"]["simulation"], [{"source": D8_ENTRY, "image": D8_IMAGE_ENTRY}])
        valid = helper_call('. "$1"; rbl_validate_layout "$2"', LAYOUT_PATH)
        self.assertEqual(valid.returncode, 0, valid.stderr.decode(errors="replace"))

        with tempfile.TemporaryDirectory(prefix="wp5b2a-layout-") as temporary:
            path = Path(temporary) / "layout.json"
            for mutation in (
                lambda layout: layout["recipes"]["D8"].update({"optional": True}),
                lambda layout: layout["recipes"].update({"D9": dict(layout["recipes"]["D8"])}),
                lambda layout: layout["recipes"]["D8"].update({"bins": [D8_BIN, "other"]}),
                lambda layout: layout["runtime_payloads"]["simulation"].append(
                    {"source": "other", "image": "/usr/local/bin/other"}
                ),
                lambda layout: layout["batches"][4]["services"].reverse(),
            ):
                mutated = json.loads(json.dumps(LAYOUT))
                mutation(mutated)
                write_json(path, mutated, 0o600)
                rejected = helper_call('. "$1"; rbl_validate_layout "$2"', path)
                self.assertNotEqual(rejected.returncode, 0, "closed D8/layout mutation was accepted")

    def test_real_context_generator_binds_source_recipe_and_compile_identity(self) -> None:
        with tempfile.TemporaryDirectory(prefix="wp5b2a-source-") as temporary:
            fixture = RequestSourceFixture(Path(temporary) / "source")
            input_base = fixture.source_hash()
            recipe_base = fixture.recipe_hash()
            commit_base = "a" * 40
            d8_path = fixture.generate("D8", input_base, recipe_base, commit_base, "d8-base")
            base = json.loads(d8_path.read_text(encoding="utf-8"))
            self.assertEqual(len(base["compile_env"]), 11)
            self.assertEqual(base["compile_env"]["R4_SIMULATION_BUILD_GIT_COMMIT"], commit_base)
            self.assertEqual(base["compile_env"]["R4_SIMULATION_BUILD_SOURCE_INPUT_SHA256"], input_base)
            self.assertEqual(base["compile_env"]["R4_SIMULATION_BUILD_RECIPE_SHA256"], recipe_base)
            valid = helper_call('. "$1"; rbl_validate_feature_contract request "$2" "$3"', fixture.layout_path, d8_path)
            self.assertEqual(valid.returncode, 0, valid.stderr.decode(errors="replace"))

            (fixture.root / "crates/job-queue/Cargo.toml").write_text("fixture source changed\n", encoding="utf-8")
            source_changed = fixture.source_hash()
            self.assertNotEqual(source_changed, input_base)
            source_path = fixture.generate("D8", source_changed, recipe_base, commit_base, "d8-source")
            source_request = json.loads(source_path.read_text(encoding="utf-8"))
            self.assertNotEqual(source_request["h_sha256"], base["h_sha256"])

            dockerfile = fixture.root / LAYOUT["recipes"]["D8"]["dockerfile"]
            dockerfile.write_text("FROM scratch AS changed-recipe\n", encoding="utf-8")
            recipe_changed = fixture.recipe_hash()
            self.assertNotEqual(recipe_changed, recipe_base)
            recipe_path = fixture.generate("D8", source_changed, recipe_changed, commit_base, "d8-recipe")
            recipe_request = json.loads(recipe_path.read_text(encoding="utf-8"))
            self.assertNotEqual(recipe_request["h_sha256"], source_request["h_sha256"])

            commit_changed = "f" * 40
            commit_path = fixture.generate("D8", source_changed, recipe_changed, commit_changed, "d8-commit")
            commit_request = json.loads(commit_path.read_text(encoding="utf-8"))
            self.assertNotEqual(commit_request["h_sha256"], recipe_request["h_sha256"])

            d3_path = fixture.generate("D3", source_changed, "8" * 64, commit_base, "d3-base")
            d3 = json.loads(d3_path.read_text(encoding="utf-8"))
            self.assertEqual(len(d3["compile_env"]), 4)
            self.assertEqual(
                set(d3["compile_env"]),
                {"CARGO_BUILD_JOBS", "CARGO_TARGET_DIR", "RUSTFLAGS", "LAGRANGE_CODE_COMMIT"},
            )
            d3_valid = helper_call('. "$1"; rbl_validate_feature_contract request "$2" "$3"', fixture.layout_path, d3_path)
            self.assertEqual(d3_valid.returncode, 0, d3_valid.stderr.decode(errors="replace"))

    def test_request_rejects_missing_extra_mixed_and_self_consistent_forged_env(self) -> None:
        with tempfile.TemporaryDirectory(prefix="wp5b2a-request-") as temporary:
            fixture = RequestSourceFixture(Path(temporary) / "source")
            request_path = fixture.generate("D8", fixture.source_hash(), fixture.recipe_hash(), "a" * 40, "base")
            original = json.loads(request_path.read_text(encoding="utf-8"))
            mutations = {
                "missing-official-origin": lambda env: env.pop("R4_SIMULATION_BUILD_ORIGIN"),
                "extra-hosted-field": lambda env: env.update({"R4_SIMULATION_BUILD_WORKFLOW_RUN_ID": "123"}),
                "mixed-official-and-hosted": lambda env: env.update(
                    {
                        "R4_SIMULATION_BUILD_WORKFLOW_RUN_ID": "123",
                        "R4_SIMULATION_BUILD_WORKFLOW_RUN_ATTEMPT": "1",
                    }
                ),
                "forged-toolchain-with-rehashed-h": lambda env: env.update(
                    {"R4_SIMULATION_BUILD_TOOLCHAIN": "9.9.9"}
                ),
            }
            for label, mutate in mutations.items():
                with self.subTest(mutation=label):
                    value = json.loads(json.dumps(original))
                    mutate(value["compile_env"])
                    value["h_sha256"] = h_digest(value)
                    path = Path(temporary) / f"{label}.json"
                    write_json(path, value, 0o600)
                    result = helper_call('. "$1"; rbl_validate_feature_contract request "$2" "$3"', fixture.layout_path, path)
                    self.assertNotEqual(result.returncode, 0, f"{label} request was accepted")


class RuntimePayloadTests(unittest.TestCase):
    def test_real_runtime_inventory_requires_simulation_entrypoint_0755(self) -> None:
        with tempfile.TemporaryDirectory(prefix="wp5b2a-runtime-") as temporary:
            root = Path(temporary) / "runtime"
            root.mkdir(mode=0o755)
            (root / ".dockerignore").write_bytes((ROOT / ".dockerignore").read_bytes())
            layout = {
                "runtime_payloads": {
                    "collectors": [{"source": "payload/collectors.txt", "image": "/opt/collectors.txt"}],
                    "backtest": [{"source": "payload/backtest", "image": "/opt/backtest"}],
                    "database": [{"source": "payload/database.txt", "image": "/opt/database.txt"}],
                    "paper": [{"source": "payload/paper-entrypoint", "image": "/usr/local/bin/paper"}],
                    "simulation": [{"source": D8_ENTRY, "image": D8_IMAGE_ENTRY}],
                }
            }
            layout_path = root / "deploy/build/release-build-layout.json"
            write_json(layout_path, layout, 0o644)
            write_bytes(root / "payload/collectors.txt", b"collector fixture\n", 0o644)
            write_bytes(root / "payload/backtest/worker.py", b"# synthetic\n", 0o644)
            write_bytes(root / "payload/database.txt", b"database fixture\n", 0o644)
            write_bytes(root / "payload/paper-entrypoint", b"#!/bin/sh\n", 0o755)
            write_bytes(root / D8_ENTRY, b"#!/bin/sh\n# synthetic\n", 0o755)
            environment = dict(os.environ, GIT_OPTIONAL_LOCKS="0")
            subprocess.run(["git", "init", "--quiet", str(root)], cwd=ROOT, env=environment, check=True)
            subprocess.run(["git", "-C", str(root), "config", "user.name", "WP-5B2A fixture"], env=environment, check=True)
            subprocess.run(
                ["git", "-C", str(root), "config", "user.email", "wp-5b2a-fixture@example.invalid"],
                env=environment,
                check=True,
            )
            subprocess.run(["git", "-C", str(root), "add", "-A"], env=environment, check=True)
            subprocess.run(["git", "-C", str(root), "commit", "--quiet", "-m", "synthetic runtime payloads"], env=environment, check=True)
            valid = helper_call('. "$1"; rbl_runtime_payload_inventory "$2" "$3"', root, layout_path)
            self.assertEqual(valid.returncode, 0, valid.stderr.decode(errors="replace"))
            inventory = json.loads(valid.stdout.decode("utf-8"))
            simulation = inventory["payload_groups"]["simulation"]
            self.assertEqual(len(simulation), 1)
            self.assertEqual(simulation[0]["source"], D8_ENTRY)
            self.assertEqual(simulation[0]["entries"][0]["mode"], "0755")
            (root / D8_ENTRY).chmod(0o644)
            invalid = helper_call('. "$1"; rbl_runtime_payload_inventory "$2" "$3"', root, layout_path)
            self.assertNotEqual(invalid.returncode, 0)


class CompilerEnvironmentTests(unittest.TestCase):
    def test_actual_export_routine_discards_hostile_ambient_identity(self) -> None:
        with tempfile.TemporaryDirectory(prefix="wp5b2a-env-") as temporary:
            fixture = RequestSourceFixture(Path(temporary) / "source")
            source_hash, recipe_hash = fixture.source_hash(), fixture.recipe_hash()
            d8_path = fixture.generate("D8", source_hash, recipe_hash, "a" * 40, "d8")
            d3_path = fixture.generate("D3", source_hash, "8" * 64, "b" * 40, "d3")
            hostile = {name: "hostile-value" for name in SIMULATION_ENV_NAMES}
            hostile.update(
                {
                    "CARGO_BUILD_JOBS": "91",
                    "CARGO_TARGET_DIR": "/hostile-target",
                    "RUSTFLAGS": "-C metadata=hostile",
                    "LAGRANGE_CODE_COMMIT": "f" * 40,
                }
            )
            script = '. "$1"; rbl_builder_export_compile_environment "$2" "$3"; env -0'
            d8_result = helper_call(script, d8_path, fixture.layout_path, env=hostile)
            self.assertEqual(d8_result.returncode, 0, d8_result.stderr.decode(errors="replace"))
            captured = dict(
                item.split(b"=", 1) for item in d8_result.stdout.split(b"\0") if item
            )
            d8 = json.loads(d8_path.read_text(encoding="utf-8"))
            self.assertEqual(captured[b"CARGO_BUILD_JOBS"], b"2")
            self.assertEqual(captured[b"CARGO_TARGET_DIR"], b"/cargo-target")
            self.assertNotIn(b"RUSTFLAGS", captured)
            self.assertEqual(captured[b"LAGRANGE_CODE_COMMIT"], d8["source_commit"].encode())
            for name in SIMULATION_ENV_NAMES:
                if name in d8["compile_env"]:
                    self.assertEqual(captured[name.encode()], d8["compile_env"][name].encode())
                else:
                    self.assertNotIn(name.encode(), captured)

            d3_result = helper_call(script, d3_path, fixture.layout_path, env=hostile)
            self.assertEqual(d3_result.returncode, 0, d3_result.stderr.decode(errors="replace"))
            d3_captured = dict(item.split(b"=", 1) for item in d3_result.stdout.split(b"\0") if item)
            for name in SIMULATION_ENV_NAMES:
                self.assertNotIn(name.encode(), d3_captured)

    def test_native_identity_phase_clears_before_fake_cargo(self) -> None:
        with tempfile.TemporaryDirectory(prefix="wp5b2a-native-env-") as temporary:
            root = Path(temporary)
            fake_bin = root / "bin"
            fake_bin.mkdir(mode=0o700)
            capture = root / "cargo-environment.txt"
            write_bytes(
                fake_bin / "rustc",
                b"#!/bin/sh\nprintf 'rustc 1.97.1\\nhost: x86_64-unknown-linux-musl\\n'\n",
                0o755,
            )
            cargo_script = "#!/bin/bash\n" + "\n".join(
                [
                    "for name in " + " ".join(SIMULATION_ENV_NAMES) + "; do",
                    '  if [[ -v "$name" ]]; then printf "%s=%s\\n" "$name" "${!name}" >>"$WP5B2A_CARGO_ENV_CAPTURE"; else printf "%s=<unset>\\n" "$name" >>"$WP5B2A_CARGO_ENV_CAPTURE"; fi',
                    "done",
                    "printf 'cargo 1.97.1 (synthetic)\\n'",
                ]
            ) + "\n"
            write_bytes(fake_bin / "cargo", cargo_script.encode(), 0o755)
            write_bytes(fake_bin / "apk", b"#!/bin/sh\nexit 70\n", 0o755)
            hostile = {name: "hostile-value" for name in SIMULATION_ENV_NAMES}
            hostile.update(
                {
                    "PATH": f"{fake_bin}:/usr/bin:/bin",
                    "TARGETPLATFORM": "linux/amd64",
                    "WP5B2A_CARGO_ENV_CAPTURE": str(capture),
                }
            )
            output = root / "native-output"
            result = helper_call('. "$1"; rbl_builder_native_identity "$2"', output, env=hostile)
            self.assertNotEqual(result.returncode, 0, "fake APK boundary should stop before identity serialization")
            self.assertIn(b"apk identity export failed", result.stderr)
            captured = capture.read_text(encoding="utf-8").splitlines()
            self.assertEqual(
                captured,
                [f"{name}=<unset>" for name in SIMULATION_ENV_NAMES],
            )


class ProducerGuardTests(unittest.TestCase):
    def test_producer_validation_and_guard_preserve_authenticated_sidecars(self) -> None:
        with tempfile.TemporaryDirectory(prefix="wp5b2a-guard-positive-") as temporary:
            fixture = D8BundleFixture(Path(temporary))
            bundle_contract = helper_call(
                '. "$1"; rbl_validate_feature_contract bundle "$2" "$3" "$4" "$5"',
                LAYOUT_PATH,
                fixture.request_path,
                fixture.release / "bundle.json",
                fixture.release / "producer",
            )
            self.assertEqual(bundle_contract.returncode, 0, bundle_contract.stderr.decode(errors="replace"))
            producer = fixture.validate_producer()
            self.assertEqual(producer.returncode, 0, producer.stderr.decode(errors="replace"))
            guarded = fixture.guard()
            self.assertEqual(guarded.returncode, 0, guarded.stderr.decode(errors="replace"))
            destination = fixture.output / D8_GUARD_SIDECAR_ROOT.lstrip("/")
            self.assertEqual((destination / "artifact.json").read_bytes(), fixture.artifact_path.read_bytes())
            self.assertEqual((destination / "COMPLETE").read_bytes(), fixture.producer_complete_path.read_bytes())
            self.assertEqual(stat.S_IMODE((destination / "artifact.json").stat().st_mode), 0o444)
            self.assertEqual(stat.S_IMODE((destination / "COMPLETE").stat().st_mode), 0o444)
            self.assertEqual((fixture.output / "target/release" / D8_BIN).read_bytes(), fixture.binary)
            self.assertEqual((fixture.output / D8_ENTRY).read_bytes(), fixture.entry_bytes)
            self.assertEqual(stat.S_IMODE((fixture.output / D8_ENTRY).stat().st_mode), 0o755)

    def test_each_guard_mutation_fails_before_sidecar_publication(self) -> None:
        mutations = {
            "wrong-receipt-digest": lambda fixture: fixture.write_artifact(
                lambda record: record.update({"h_sha256": "f" * 64})
            ),
            "wrong-complete": lambda fixture: write_bytes(
                fixture.producer_complete_path, b"0" * 64 + b"\n", 0o644
            ),
            "wrong-source": lambda fixture: fixture.write_artifact(
                lambda record: record.update({"source_commit": "e" * 40})
            ),
            "wrong-recipe": lambda fixture: fixture.write_artifact(
                lambda record: record.update({"recipe_sha256": "f" * 64})
            ),
            "wrong-binary-hash": lambda fixture: fixture.write_artifact(
                lambda record: record.update({"binary_sha256": "0" * 64})
            ),
            "wrong-mode": lambda fixture: fixture.artifact_path.chmod(0o600),
            "extra-producer-entry": lambda fixture: write_bytes(fixture.producer / "extra.json", b"{}\n", 0o644),
        }
        for label, mutate in mutations.items():
            with self.subTest(mutation=label), tempfile.TemporaryDirectory(prefix="wp5b2a-guard-negative-") as temporary:
                fixture = D8BundleFixture(Path(temporary))
                baseline = fixture.guard()
                self.assertEqual(baseline.returncode, 0, baseline.stderr.decode(errors="replace"))
                shutil.rmtree(fixture.output)
                mutate(fixture)
                fixture.reseal_bundle()
                result = fixture.guard()
                self.assertNotEqual(result.returncode, 0, f"{label} guard mutation was accepted")
                self.assertFalse(
                    (fixture.output / D8_GUARD_SIDECAR_ROOT.lstrip("/")).exists(),
                    f"{label} exposed a receipt sidecar before rejection",
                )

        with self.subTest(mutation="receipt-symlink"), tempfile.TemporaryDirectory(prefix="wp5b2a-guard-symlink-") as temporary:
            fixture = D8BundleFixture(Path(temporary))
            baseline = fixture.guard()
            self.assertEqual(baseline.returncode, 0, baseline.stderr.decode(errors="replace"))
            shutil.rmtree(fixture.output)
            linked_bytes = Path(temporary) / "receipt-copy.json"
            linked_bytes.write_bytes(fixture.artifact_path.read_bytes())
            fixture.artifact_path.unlink()
            fixture.artifact_path.symlink_to(linked_bytes)
            result = fixture.guard()
            self.assertNotEqual(result.returncode, 0, "receipt symlink was accepted")
            self.assertFalse((fixture.output / D8_GUARD_SIDECAR_ROOT.lstrip("/")).exists())


class AttemptLogTests(unittest.TestCase):
    def test_exact_d8_and_legacy_labels_preserve_attempt_bytes_and_exit_status(self) -> None:
        labels = ["native-identity", *(f"producer-D{i}-fixture" for i in range(1, 8)), f"producer-D8-{D8_BIN}"]
        for label in labels:
            with self.subTest(label=label), tempfile.TemporaryDirectory(prefix="d8-attempt-") as temporary:
                state = Path(temporary)
                log = state / "logs" / f"{label}.log"
                script = 'source "$1"; rbl_capture_attempt_log "$2" "$3" "$4" sh -c \'printf "%s\\n" "$1"; exit "$2"\' fixture "$5" "$6"'
                first = helper_call(script, state, log, label, "first", 0)
                self.assertEqual(first.returncode, 0, first.stderr.decode(errors="replace"))
                self.assertEqual(log.read_bytes(), b"first\n")
                second = helper_call(script, state, log, label, "second", 7)
                self.assertEqual(second.returncode, 7, second.stderr.decode(errors="replace"))
                self.assertEqual(log.read_bytes(), b"second\n")
                self.assertEqual(stat.S_IMODE(log.stat().st_mode), 0o600)
                history = state / "logs/attempt-history" / label
                for raw in (b"first\n", b"second\n"):
                    self.assertEqual((history / "objects" / (sha256_bytes(raw) + ".bytes")).read_bytes(), raw)
                receipts = [json.loads(p.read_bytes()) for p in (history / "receipts").glob("*.json")]
                self.assertEqual(sorted(r["exit_code"] for r in receipts), [0, 7])
                self.assertTrue(all(r["attempt_label"] == label and r["phase"] == "completed" for r in receipts))

    def test_d8_near_misses_and_wrong_log_path_fail_before_command(self) -> None:
        exact = f"producer-D8-{D8_BIN}"
        labels = ["producer-D8-other-runner", exact + "-extra", exact + "\n", exact + "/../escape", exact.replace("D8", "D9"), exact.replace("D8", "D80")]
        for label in [*labels, exact]:
            with self.subTest(label=label), tempfile.TemporaryDirectory(prefix="d8-attempt-reject-") as temporary:
                state = Path(temporary)
                marker = state / "command-ran"
                log = state / "logs" / ("wrong.log" if label == exact else label + ".log")
                script = 'source "$1"; rbl_capture_attempt_log "$2" "$3" "$4" sh -c \'printf ran > "$1"\' fixture "$5"'
                result = helper_call(script, state, log, label, marker)
                self.assertNotEqual(result.returncode, 0)
                expected = b"attempt log path does not match its label" if label == exact else b"attempt log label is not canonical"
                self.assertIn(expected, result.stderr)
                self.assertFalse(marker.exists())
                self.assertFalse((state / "logs").exists())


class ProductExpectationTests(unittest.TestCase):
    def test_product_request_requires_binary_entrypoint_and_both_original_sidecars(self) -> None:
        body = extract_python("rbl_product_request")
        with tempfile.TemporaryDirectory(prefix="wp5b2a-product-") as temporary:
            root = Path(temporary)
            fixture = D8BundleFixture(root / "fixture")
            source_root = root / "source"
            write_bytes(source_root / "deploy/db/Dockerfile", (
                "RUN chmod 0755 /usr/local/bin/lagrange-migrate "
                "/usr/local/bin/lagrange-bootstrap-roles /usr/local/bin/lagrange-daily-control\n"
            ).encode(), 0o644)
            write_bytes(source_root / "deploy/runtime/Dockerfile.paper-runner", (
                "RUN chmod 0755 /usr/local/bin/paper-runner /usr/local/bin/paper-runner-bin\n"
            ).encode(), 0o644)
            source_entry = source_root / D8_ENTRY
            write_bytes(source_entry, fixture.entry_bytes, 0o755)
            entry_hash = sha256_bytes(fixture.entry_bytes)
            payload_record = f"f\t0755\t{D8_ENTRY}\t{entry_hash}\n".encode("utf-8")
            inventory = {
                "format": RUNTIME_FORMAT,
                "dockerignore_sha256": DOCKERIGNORE_SHA256,
                "payload_groups": {
                    "simulation": [
                        {
                            "source": D8_ENTRY,
                            "image": D8_IMAGE_ENTRY,
                            "entries": [{"kind": "file", "mode": "0755", "path": D8_ENTRY, "sha256": entry_hash}],
                            "tree_sha256": sha256_bytes(payload_record),
                        }
                    ]
                },
            }
            request_parent = root / "requests"
            request_parent.mkdir(mode=0o700)
            request_path = request_parent / "request.json"
            expected_path = request_parent / "expectations.json"

            def generate(bundle: Path, label: str) -> tuple[subprocess.CompletedProcess[bytes], dict[str, object] | None, dict[str, object] | None]:
                current_request = request_parent / f"{label}-request.json"
                current_expected = request_parent / f"{label}-expectations.json"
                environment = {
                    "RBL_SERVICE": "owner-beta-simulation-runner",
                    "RBL_COMMIT": fixture.request["source_commit"],
                    "RBL_SOURCE_ROOT": str(source_root),
                    "RBL_LAYOUT": str(fixture.release / "release-build-layout.json"),
                    "RBL_BUNDLE": str(bundle),
                    "RBL_REQUEST_PATH": str(current_request),
                    "RBL_EXPECTED_PATH": str(current_expected),
                    "RBL_RUNTIME_INVENTORY": json.dumps(inventory, sort_keys=True, separators=(",", ":")),
                    "RBL_RUNTIME_FORMAT": RUNTIME_FORMAT,
                    "RBL_DOCKERIGNORE_HASH": DOCKERIGNORE_SHA256,
                }
                result = run_python_body(body, environment)
                request_value = json.loads(current_request.read_text(encoding="utf-8")) if current_request.exists() else None
                expected_value = json.loads(current_expected.read_text(encoding="utf-8")) if current_expected.exists() else None
                return result, request_value, expected_value

            positive, request_value, expected_value = generate(fixture.bundle, "positive")
            self.assertEqual(positive.returncode, 0, positive.stderr.decode(errors="replace"))
            selected = {item["path"]: item for item in request_value["files"]}
            expected_paths = {
                "usr/local/bin/owner-beta-simulation-runner",
                "usr/local/bin/owner-beta-simulation-entrypoint",
                "usr/share/lagrange/owner-beta-simulation/artifact.json",
                "usr/share/lagrange/owner-beta-simulation/COMPLETE",
            }
            self.assertEqual(set(selected), expected_paths)
            self.assertEqual(expected_value["files"]["usr/share/lagrange/owner-beta-simulation/artifact.json"]["mode"], "0444")
            self.assertEqual(expected_value["files"]["usr/share/lagrange/owner-beta-simulation/COMPLETE"]["mode"], "0444")
            self.assertEqual(expected_value["files"]["usr/local/bin/owner-beta-simulation-runner"]["mode"], "0755")
            self.assertEqual(expected_value["files"]["usr/local/bin/owner-beta-simulation-entrypoint"]["mode"], "0755")
            artifact_raw = fixture.artifact_path.read_bytes()
            complete_raw = fixture.producer_complete_path.read_bytes()
            self.assertEqual(
                selected["usr/share/lagrange/owner-beta-simulation/artifact.json"]["sha256"],
                sha256_bytes(artifact_raw),
            )
            self.assertEqual(
                selected["usr/share/lagrange/owner-beta-simulation/COMPLETE"]["sha256"],
                sha256_bytes(complete_raw),
            )

            for missing in ("artifact.json", "COMPLETE"):
                with self.subTest(missing=missing):
                    broken = root / f"bundle-missing-{missing}"
                    shutil.copytree(fixture.bundle, broken, symlinks=True)
                    (broken / ".release-build/producer" / D8_BIN / missing).unlink()
                    rejected, _, _ = generate(broken, f"missing-{missing}")
                    self.assertNotEqual(rejected.returncode, 0, f"missing {missing} sidecar was accepted")


if __name__ == "__main__":
    unittest.main(verbosity=2)
