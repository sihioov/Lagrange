from __future__ import annotations

import importlib.util
import hashlib
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
LIBRARY = ROOT / "scripts/ops/lib/release-image-manifest.sh"
COMMIT = "a" * 40
V2_FORMAT = "LAGRANGE_RELEASE_MANIFEST_V2"
V3_FORMAT = "LAGRANGE_RELEASE_MANIFEST_V3"


def load_module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError("test-module-unavailable")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


sys.dont_write_bytecode = True
policy = load_module("wp5b1_manifest_policy", ROOT / "scripts/ops/lib/service-status-policy.py")
daily = load_module("wp5b1_daily_operations", ROOT / "scripts/ops/lib/owner-beta-daily-operations.py")


def image_id(index: int) -> str:
    return f"sha256:{index:064x}"


def manifest_bytes(format_name: str, services: tuple[str, ...], *, commit: str = COMMIT) -> bytes:
    rows = [format_name, "commit|" + commit]
    rows.extend(
        f"image|{service}|lagrange-station-{service}:{commit}|{image_id(index)}|{commit}"
        for index, service in enumerate(services, start=1)
    )
    return ("\n".join(rows) + "\n").encode("ascii")


def expected_override(services: tuple[str, ...], images: dict[str, str], *, owner_only: bool) -> str:
    names = (*services, *(daily.runtime.SERVICES[:2] if owner_only else ()))
    rows = ["services:"]
    for service in names:
        canonical = "owner-beta-runner" if service in daily.runtime.SERVICES[:2] else service
        rows.extend((f"  {service}:", f"    image: {images[canonical]}", "    build: !reset null"))
    return "\n".join(rows) + "\n"


class ShellManifestCompatibilityTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="wp5b1-manifest-")
        self.root = Path(self.temp.name)
        self.v2_services = policy.MANIFEST_V2_SERVICES
        self.v3_services = policy.MANIFEST_V3_SERVICES

    def tearDown(self) -> None:
        self.temp.cleanup()

    def put(self, name: str, data: bytes) -> Path:
        path = self.root / name
        path.write_bytes(data)
        path.chmod(0o600)
        return path

    def bash(self, source: str, *args: object) -> subprocess.CompletedProcess[str]:
        env = {**os.environ, "LC_ALL": "C", "LANG": "C"}
        return subprocess.run(
            ["/bin/bash", "--noprofile", "--norc", "-c", source, "wp5b1",
             str(LIBRARY), *(str(arg) for arg in args)],
            cwd=ROOT,
            env=env,
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            check=False,
        )

    def load_document(self, path: Path) -> subprocess.CompletedProcess[str]:
        return self.bash(
            'source "$1"; release_image_manifest_load "$2" "' + COMMIT + '" || exit 1; '
            'printf "%s\\n" "$RELEASE_IMAGE_MANIFEST_LOADED_FORMAT"; '
            'printf "%s\\n" "${RELEASE_IMAGE_MANIFEST_LOADED_SERVICES[@]}"',
            path,
        )

    def test_shell_service_sets_match_the_closed_python_sets(self) -> None:
        result = self.bash(
            'source "$1"; printf "V2\\n"; printf "%s\\n" "${RELEASE_IMAGE_SERVICES_V2[@]}"; '
            'printf "V3\\n"; printf "%s\\n" "${RELEASE_IMAGE_SERVICES_V3[@]}"; '
            'printf "CURRENT\\n"; printf "%s\\n" "${RELEASE_IMAGE_SERVICES[@]}"'
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            result.stdout.splitlines(),
            ["V2", *self.v2_services, "V3", *self.v3_services, "CURRENT", *self.v3_services],
        )

    def test_shell_reads_exact_v2_and_v3_and_writes_v3_round_trip(self) -> None:
        for format_name, services in ((V2_FORMAT, self.v2_services), (V3_FORMAT, self.v3_services)):
            with self.subTest(format=format_name):
                path = self.put(format_name + ".manifest", manifest_bytes(format_name, services))
                parsed = self.load_document(path)
                self.assertEqual(parsed.returncode, 0, parsed.stderr)
                self.assertEqual(parsed.stdout.splitlines(), [format_name, *services])

        output = self.root / "written.manifest"
        written = self.bash(
            'source "$1"; release_image_manifest_reset; index=1; '
            'for service in "${RELEASE_IMAGE_SERVICES[@]}"; do '
            'RELEASE_IMAGE_MANIFEST_REFS["$service"]=$(release_image_manifest_ref_for "$service" "' + COMMIT + '"); '
            'RELEASE_IMAGE_MANIFEST_IDS["$service"]=$(printf "sha256:%064x" "$index"); '
            'RELEASE_IMAGE_MANIFEST_REVISIONS["$service"]="' + COMMIT + '"; index=$((index + 1)); done; '
            'release_image_manifest_write "$2" "' + COMMIT + '" || exit 1; '
            'release_image_manifest_load "$2" "' + COMMIT + '" || exit 1',
            output,
        )
        self.assertEqual(written.returncode, 0, written.stderr)
        self.assertEqual(output.read_bytes(), manifest_bytes(V3_FORMAT, self.v3_services))

    def test_shell_compose_override_uses_loaded_v2_or_v3_set_and_daily_aliases(self) -> None:
        for format_name, services in ((V2_FORMAT, self.v2_services), (V3_FORMAT, self.v3_services)):
            data = manifest_bytes(format_name, services)
            path = self.put(format_name + ".manifest", data)
            images = {service: image_id(index) for index, service in enumerate(services, start=1)}
            for mode, owner_only in (("disabled", False), ("owner_only", True)):
                with self.subTest(format=format_name, mode=mode):
                    override = self.root / (format_name + "-" + mode + ".yml")
                    result = self.bash(
                        'source "$1"; release_image_manifest_load "$2" "' + COMMIT + '" || exit 1; '
                        'release_image_manifest_write_compose_override "$3" "$4" || exit 1; cat "$3"',
                        path, override, mode,
                    )
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(result.stdout, expected_override(services, images, owner_only=owner_only))

    def test_shell_rejects_unknown_mixed_partial_and_malformed_manifests(self) -> None:
        v2 = manifest_bytes(V2_FORMAT, self.v2_services).decode("ascii").splitlines()
        v3 = manifest_bytes(V3_FORMAT, self.v3_services).decode("ascii").splitlines()
        bad: list[tuple[str, bytes]] = []

        unknown = v3.copy()
        unknown[0] = "LAGRANGE_RELEASE_MANIFEST_V9"
        bad.append(("unknown version", ("\n".join(unknown) + "\n").encode()))
        bad.append(("v2 with v3 cardinality", ("\n".join([v2[0], v2[1], *v3[2:]]) + "\n").encode()))
        bad.append(("v3 with v2 cardinality", ("\n".join(v3[:-1]) + "\n").encode()))
        mixed = v2.copy()
        mixed[-1] = (
            f"image|owner-beta-simulation-runner|lagrange-station-owner-beta-simulation-runner:{COMMIT}|"
            f"{image_id(13)}|{COMMIT}"
        )
        bad.append(("mixed v2 service set", ("\n".join(mixed) + "\n").encode()))
        bad.append(("missing service", ("\n".join(v3[:-1]) + "\n").encode()))
        bad.append(("extra service", ("\n".join([*v3, v3[-1]]) + "\n").encode()))
        duplicate = v3.copy()
        duplicate[3] = duplicate[2]
        bad.append(("duplicate service", ("\n".join(duplicate) + "\n").encode()))
        reordered = v3.copy()
        reordered[2], reordered[3] = reordered[3], reordered[2]
        bad.append(("reordered service", ("\n".join(reordered) + "\n").encode()))
        wrong_commit = v3.copy()
        wrong_commit[1] = "commit|" + "b" * 40
        bad.append(("wrong commit", ("\n".join(wrong_commit) + "\n").encode()))
        wrong_ref = v3.copy()
        fields = wrong_ref[2].split("|")
        fields[2] = "lagrange-station-web:" + COMMIT
        wrong_ref[2] = "|".join(fields)
        bad.append(("wrong ref", ("\n".join(wrong_ref) + "\n").encode()))
        wrong_id = v3.copy()
        fields = wrong_id[2].split("|")
        fields[3] = "sha256:ABC"
        wrong_id[2] = "|".join(fields)
        bad.append(("wrong image id", ("\n".join(wrong_id) + "\n").encode()))
        wrong_revision = v3.copy()
        fields = wrong_revision[2].split("|")
        fields[4] = "b" * 40
        wrong_revision[2] = "|".join(fields)
        bad.append(("wrong revision", ("\n".join(wrong_revision) + "\n").encode()))
        bad.append(("trailing image separator", ("\n".join([*v3[:2], v3[2] + "|", *v3[3:]]) + "\n").encode()))
        bad.append(("trailing commit separator", ("\n".join([v3[0], v3[1] + "|", *v3[2:]]) + "\n").encode()))
        bad.append(("missing final newline", manifest_bytes(V3_FORMAT, self.v3_services)[:-1]))
        bad.append(("carriage return", manifest_bytes(V3_FORMAT, self.v3_services).replace(b"\n", b"\r\n")))
        bad.append(("nul byte", manifest_bytes(V3_FORMAT, self.v3_services).replace(b"image|", b"im\x00age|", 1)))
        bad.append(("non-ascii byte", manifest_bytes(V3_FORMAT, self.v3_services).replace(b"image|", b"im\xc3\xa9ge|", 1)))

        for index, (label, data) in enumerate(bad):
            with self.subTest(case=label):
                path = self.put(f"invalid-{index}.manifest", data)
                result = self.load_document(path)
                self.assertNotEqual(result.returncode, 0, result.stdout)

    def test_failed_partial_load_clears_published_maps_and_blocks_override(self) -> None:
        good = self.put("good.manifest", manifest_bytes(V3_FORMAT, self.v3_services))
        bad_rows = manifest_bytes(V3_FORMAT, self.v3_services).decode("ascii").splitlines()
        fields = bad_rows[3].split("|")
        fields[3] = "bad-image-id"
        bad_rows[3] = "|".join(fields)
        bad = self.put("partial.manifest", ("\n".join(bad_rows) + "\n").encode())
        override = self.root / "must-not-exist.yml"
        result = self.bash(
            'source "$1"; release_image_manifest_load "$2" "' + COMMIT + '" || exit 1; '
            'release_image_manifest_load "$3" "' + COMMIT + '" && exit 31; '
            '[[ ${#RELEASE_IMAGE_MANIFEST_REFS[@]} -eq 0 && ${#RELEASE_IMAGE_MANIFEST_IDS[@]} -eq 0 && '
            '${#RELEASE_IMAGE_MANIFEST_REVISIONS[@]} -eq 0 && -z "$RELEASE_IMAGE_MANIFEST_LOADED_FORMAT" && '
            '${#RELEASE_IMAGE_MANIFEST_LOADED_SERVICES[@]} -eq 0 ]] || exit 32; '
            'release_image_manifest_write_compose_override "$4" && exit 33; '
            '[[ ! -e "$4" ]] || exit 34; printf RESET_OK',
            good, bad, override,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "RESET_OK")

    def test_writer_rejects_missing_extra_and_inconsistent_maps(self) -> None:
        cases = ("missing", "extra", "inconsistent")
        for case in cases:
            with self.subTest(case=case):
                output = self.root / f"writer-{case}.manifest"
                mutation = {
                    "missing": 'unset "RELEASE_IMAGE_MANIFEST_REVISIONS[${RELEASE_IMAGE_SERVICES_V3[-1]}]"',
                    "extra": 'RELEASE_IMAGE_MANIFEST_REFS[unknown-service]=bad',
                    "inconsistent": 'unset "RELEASE_IMAGE_MANIFEST_IDS[${RELEASE_IMAGE_SERVICES_V3[-1]}]"',
                }[case]
                result = self.bash(
                    'source "$1"; release_image_manifest_reset; index=1; '
                    'for service in "${RELEASE_IMAGE_SERVICES_V3[@]}"; do '
                    'RELEASE_IMAGE_MANIFEST_REFS["$service"]=$(release_image_manifest_ref_for "$service" "' + COMMIT + '"); '
                    'RELEASE_IMAGE_MANIFEST_IDS["$service"]=$(printf "sha256:%064x" "$index"); '
                    'RELEASE_IMAGE_MANIFEST_REVISIONS["$service"]="' + COMMIT + '"; index=$((index + 1)); done; '
                    + mutation + '; release_image_manifest_write "$2" "' + COMMIT + '" && exit 41; '
                    '[[ ! -e "$2" ]] || exit 42; printf WRITE_REJECTED',
                    output,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout, "WRITE_REJECTED")

    def test_malformed_loaded_state_cannot_create_override(self) -> None:
        manifest = self.put("valid.manifest", manifest_bytes(V2_FORMAT, self.v2_services))
        override = self.root / "malformed-state.yml"
        result = self.bash(
            'source "$1"; release_image_manifest_load "$2" "' + COMMIT + '" || exit 1; '
            'RELEASE_IMAGE_MANIFEST_LOADED_SERVICES[0]=owner-beta-simulation-runner; '
            'release_image_manifest_write_compose_override "$3" && exit 51; '
            '[[ ! -e "$3" ]] || exit 52; printf STATE_REJECTED',
            manifest, override,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "STATE_REJECTED")


class PythonManifestCompatibilityTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="wp5b1-policy-")
        self.root = Path(self.temp.name)
        self.release = self.root / "release"
        self.release.mkdir(mode=0o700)
        self.owner = os.geteuid(), os.getegid()
        self.path = self.release / ".lagrange-release-manifest"

    def tearDown(self) -> None:
        self.temp.cleanup()

    def write(self, data: bytes) -> None:
        self.path.write_bytes(data)
        self.path.chmod(0o600)

    def validate(self, commit: str = COMMIT) -> str:
        return policy.validate_manifest(str(self.path), commit, test_owner=self.owner)

    def test_pure_format_selector_and_trusted_validator_accept_exact_v2_and_v3(self) -> None:
        self.assertEqual(policy.manifest_services_for_format(V2_FORMAT), policy.MANIFEST_V2_SERVICES)
        self.assertEqual(policy.manifest_services_for_format(V3_FORMAT), policy.MANIFEST_V3_SERVICES)
        self.assertEqual(policy.MANIFEST_FORMAT, V3_FORMAT)
        self.assertEqual(policy.MANIFEST_SERVICES, policy.MANIFEST_V3_SERVICES)
        for format_name, services in (
            (V2_FORMAT, policy.MANIFEST_V2_SERVICES),
            (V3_FORMAT, policy.MANIFEST_V3_SERVICES),
        ):
            with self.subTest(format=format_name):
                data = manifest_bytes(format_name, services)
                self.write(data)
                self.assertEqual(self.validate(), hashlib.sha256(data).hexdigest())

    def test_unknown_version_and_crossed_cardinalities_are_rejected(self) -> None:
        documents = (
            manifest_bytes("LAGRANGE_RELEASE_MANIFEST_V9", policy.MANIFEST_V3_SERVICES),
            manifest_bytes(V2_FORMAT, policy.MANIFEST_V3_SERVICES),
            manifest_bytes(V3_FORMAT, policy.MANIFEST_V2_SERVICES),
        )
        for data in documents:
            with self.subTest(header=data.splitlines()[0], rows=len(data.splitlines()) - 2):
                self.write(data)
                with self.assertRaises(policy.PolicyError):
                    self.validate()

    def test_trusted_file_mode_and_symlink_requirements_remain_enforced(self) -> None:
        self.write(manifest_bytes(V2_FORMAT, policy.MANIFEST_V2_SERVICES))
        self.path.chmod(0o644)
        with self.assertRaises(policy.PolicyError):
            self.validate()
        self.path.chmod(0o600)
        with self.assertRaises(policy.PolicyError):
            policy.validate_manifest(str(self.path), COMMIT, test_owner=(self.owner[0] + 1, self.owner[1]))
        self.path.unlink()
        self.path.symlink_to(self.root / "missing-target")
        with self.assertRaises(policy.PolicyError):
            self.validate()


class DailyManifestCompatibilityTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="wp5b1-daily-")
        self.root = Path(self.temp.name)
        self.release = self.root / "release"
        self.release.mkdir(mode=0o700)
        self.owner = os.geteuid(), os.getegid()
        self.manifest = self.release / ".lagrange-release-manifest"

    def tearDown(self) -> None:
        self.temp.cleanup()

    def read_images(self, data: bytes) -> dict[str, str]:
        self.manifest.write_bytes(data)
        self.manifest.chmod(0o600)
        validate = daily.policy.validate_manifest

        def validate_for_fixture(path: str, commit: str) -> str:
            return validate(path, commit, test_owner=self.owner)

        with patch.object(daily.install, "read_root_file", return_value=data), \
             patch.object(daily.policy, "validate_manifest", side_effect=validate_for_fixture):
            return daily.read_manifest(str(self.release), COMMIT)

    def test_daily_read_and_start_override_keep_each_validated_order_and_fixed_cohort(self) -> None:
        for format_name, services in (
            (V2_FORMAT, policy.MANIFEST_V2_SERVICES),
            (V3_FORMAT, policy.MANIFEST_V3_SERVICES),
        ):
            with self.subTest(format=format_name):
                images = self.read_images(manifest_bytes(format_name, services))
                self.assertEqual(tuple(images), services)
                target = SimpleNamespace(
                    commit=COMMIT,
                    images=images,
                    release=str(self.release),
                    overlays=[],
                )
                binding = SimpleNamespace(project="lagrange-station", config_sha256="b" * 64)
                calls: list[tuple[list[str], str]] = []

                def fake_run(argv, **kwargs):
                    override = next(
                        Path(argv[index + 1]) for index, value in enumerate(argv[:-1])
                        if value == "-f" and Path(argv[index + 1]).name.startswith(".daily-image-override.")
                    )
                    calls.append((list(argv), override.read_text()))
                    return SimpleNamespace(returncode=0, stdout=b"")

                with patch.object(daily.runtime, "image_owned", return_value=None), \
                     patch.object(daily.subprocess, "run", side_effect=fake_run):
                    daily.start_target(daily.runtime, target, binding, str(self.root), "absent")

                expected = expected_override(services, images, owner_only=True)
                self.assertEqual([content for _, content in calls], [expected, expected])
                up = next(argv for argv, _ in calls if "up" in argv)
                up_args = up[up.index("up"):]
                self.assertIn("api-server", up_args)
                self.assertTrue(set(daily.runtime.SERVICES).issubset(set(up_args)))
                self.assertNotIn("owner-beta-simulation-runner", up_args)


if __name__ == "__main__":
    unittest.main()
