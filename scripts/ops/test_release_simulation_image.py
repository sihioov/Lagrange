#!/usr/bin/env python3
"""Offline contract tests for the guarded owner-beta simulation image."""

from __future__ import annotations

import hashlib
import importlib.util
import shlex
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
DOCKERFILE = ROOT / "crates/job-queue/Dockerfile.owner-beta-simulation-runner"
ENTRYPOINT = ROOT / "deploy/runtime/owner-beta-simulation-entrypoint"
PINNED_ALPINE = (
    "alpine@sha256:48b0309ca019d89d40f670aa1bc06e426dc0931948452e8491e3d65087abc07d"
)
RUNNER_PATH = "/usr/local/bin/owner-beta-simulation-runner"
ARTIFACT_ROOT = "/data/owner-beta-artifacts"
ARTIFACT_JSON = "/usr/share/lagrange/owner-beta-simulation/artifact.json"
COMPLETE = "/usr/share/lagrange/owner-beta-simulation/COMPLETE"
WORKER_ID = "owner-beta-simulation-runner"
STATIC_ERROR = b"owner-beta-simulation-entrypoint: rejected\n"
SYNTHETIC_RECEIPT = b'{"synthetic":"offline fixture"}\n'


class DockerfileContractTests(unittest.TestCase):
    HEALTH_ROOT = "/run/owner-beta-simulation-health"
    HEALTH_RECORDS = f"{HEALTH_ROOT}/records"
    HEALTH_LOCK = f"{HEALTH_ROOT}/writer.lock"
    HEALTH_COMMENT = "# Prepare the image-owned health root and its non-writable records mountpoint."
    HEALTH_LAYOUT_LINES = (
        f"RUN mkdir {HEALTH_ROOT} \\",
        f"    && mkdir {HEALTH_RECORDS} \\",
        f"    && touch {HEALTH_LOCK} \\",
        f"    && chown 0:10001 {HEALTH_ROOT} {HEALTH_RECORDS} {HEALTH_LOCK} \\",
        f"    && chmod 0750 {HEALTH_ROOT} {HEALTH_RECORDS} \\",
        f"    && chmod 0440 {HEALTH_LOCK}",
    )

    @classmethod
    def _validated_health_layout_body(cls, source: str) -> str:
        lines = source.splitlines()

        def reject_unless(condition: bool, message: str) -> None:
            if not condition:
                raise ValueError(message)

        reject_unless(source.count(cls.HEALTH_COMMENT) == 1, "health layout comment must occur once")
        comment_index = lines.index(cls.HEALTH_COMMENT)
        run_start = comment_index + 1
        expected_lines = list(cls.HEALTH_LAYOUT_LINES)
        block_end = run_start + len(expected_lines) - 1
        reject_unless(lines[run_start:block_end + 1] == expected_lines, "health RUN differs from the closed command allowlist")

        from_indices = [index for index, line in enumerate(lines) if line.startswith("FROM ")]
        reject_unless(
            [lines[index] for index in from_indices]
            == [f"FROM {PINNED_ALPINE} AS verified-artifacts", f"FROM {PINNED_ALPINE}"],
            "pinned build stages changed",
        )
        final_from = from_indices[-1]
        reject_unless(comment_index > final_from, "health layout must be in the final stage")

        run_indices = [index for index, line in enumerate(lines) if line.startswith("RUN ")]
        final_run_indices = [index for index in run_indices if index > final_from]
        reject_unless(len(final_run_indices) == 2, "final stage must contain only setup and health RUN instructions")
        reject_unless(final_run_indices[1] == run_start, "health layout must follow final-stage setup")
        setup_start = final_run_indices[0]
        setup_lines = [
            "RUN test -n \"$LAGRANGE_CODE_COMMIT\" \\",
            "    && test \"${#LAGRANGE_CODE_COMMIT}\" -eq 40 \\",
            "    && printf '%s' \"$LAGRANGE_CODE_COMMIT\" | grep -Eq '^[0-9a-f]{40}$' \\",
            "    && test \"$LAGRANGE_CODE_COMMIT\" != 0000000000000000000000000000000000000000 \\",
            "    && apk add --no-cache ca-certificates libgcc libstdc++ libpq libssl3 \\",
            "    && addgroup -S -g 10001 lagrange \\",
            "    && adduser -S -D -H -u 10001 -G lagrange lagrange \\",
            "    && mkdir -p /usr/share/lagrange/owner-beta-simulation",
        ]
        reject_unless(lines[setup_start:comment_index - 1] == setup_lines, "final-stage setup RUN changed or moved")
        reject_unless(lines[setup_start - 1] == "ARG LAGRANGE_CODE_COMMIT", "final-stage revision ARG moved")
        reject_unless(lines[block_end + 1] == "", "health RUN must end before the following label")
        reject_unless(lines[block_end + 2].startswith("LABEL "), "health RUN must precede LABEL/COPY/ENV/USER")

        commands = []
        for index, line in enumerate(expected_lines):
            item = line.removesuffix("\\").strip()
            prefix = "RUN " if index == 0 else "&& "
            reject_unless(item.startswith(prefix), "health RUN must use six &&-chained commands")
            commands.append(shlex.split(item[len(prefix):], posix=True))
        expected_commands = [
            ["mkdir", cls.HEALTH_ROOT],
            ["mkdir", cls.HEALTH_RECORDS],
            ["touch", cls.HEALTH_LOCK],
            ["chown", "0:10001", cls.HEALTH_ROOT, cls.HEALTH_RECORDS, cls.HEALTH_LOCK],
            ["chmod", "0750", cls.HEALTH_ROOT, cls.HEALTH_RECORDS],
            ["chmod", "0440", cls.HEALTH_LOCK],
        ]
        reject_unless(commands == expected_commands, "health RUN tokens differ from the closed allowlist")

        expected_env = [
            "ENV APP_ENV=production",
            "ENV OWNER_BETA_SIMULATION_MODE=disabled",
            "ENV OWNER_BETA_ACCESS_MODE=disabled",
            "ENV OWNER_BETA_PRICE_INPUT_MODE=disabled",
        ]
        actual_env = [line for line in lines if line.startswith("ENV ")]
        reject_unless(actual_env == expected_env, "image ENV settings changed")
        user_lines = [line for line in lines if line.startswith("USER ")]
        reject_unless(user_lines == ["USER 10001:10001"], "image must retain its sole non-root USER")
        reject_unless(lines.index(user_lines[0]) > block_end, "health layout must precede USER")
        reject_unless(not any(line.startswith(("HEALTHCHECK ", "VOLUME ")) for line in lines), "healthcheck or volume is out of scope")
        reject_unless("STOPSIGNAL SIGTERM" in lines, "fixed stop signal changed")
        reject_unless('ENTRYPOINT ["/usr/local/bin/owner-beta-simulation-entrypoint"]' in lines, "fixed entrypoint changed")

        return "\n".join([expected_lines[0].removeprefix("RUN "), *expected_lines[1:]])

    @staticmethod
    def _replace_once(source: str, old: str, new: str) -> str:
        if source.count(old) != 1:
            raise AssertionError(f"mutation anchor must occur exactly once: {old}")
        return source.replace(old, new, 1)

    def _capture_health_layout(self, body: str, *, fail_at: str = ""):
        with tempfile.TemporaryDirectory(prefix="wp5c3c3-health-layout-") as temporary:
            private = Path(temporary)
            private_bin = private / "bin"
            private_bin.mkdir()
            capture_log = private / "argv.tsv"
            commands = ("mkdir", "touch", "chown", "chmod")
            for command in commands:
                stub = private_bin / command
                stub.write_text(
                    "#!/bin/sh\n"
                    f"printf '%s' '{command}' >> \"$CAPTURE_LOG\" || exit 91\n"
                    "for arg do printf '\\t%s' \"$arg\" >> \"$CAPTURE_LOG\" || exit 91; done\n"
                    "printf '\\n' >> \"$CAPTURE_LOG\" || exit 91\n"
                    f"if [ \"${{FAIL_AT:-}}\" = \"{command}\" ]; then exit 37; fi\n"
                    "exit 0\n",
                    encoding="utf-8",
                )
                stub.chmod(0o700)
            environment = {
                "PATH": str(private_bin),
                "CAPTURE_LOG": str(capture_log),
                "FAIL_AT": fail_at,
            }
            result = subprocess.run(
                ["/bin/sh", "-c", body],
                env=environment,
                capture_output=True,
                timeout=10,
                check=False,
            )
            rows = []
            if capture_log.exists():
                rows = [tuple(line.split("\t")) for line in capture_log.read_text(encoding="utf-8").splitlines()]
            return result, rows, sorted(path.name for path in private_bin.iterdir())

    def test_image_health_layout_has_closed_final_stage_contract(self) -> None:
        source = DOCKERFILE.read_text(encoding="utf-8")
        self.assertTrue(self._validated_health_layout_body(source))

        layout = "\n".join((self.HEALTH_COMMENT, *self.HEALTH_LAYOUT_LINES))
        without_layout = self._replace_once(source, layout + "\n\n", "")
        guard_stage = self._replace_once(
            without_layout,
            f"FROM {PINNED_ALPINE} AS verified-artifacts\n",
            f"FROM {PINNED_ALPINE} AS verified-artifacts\n\n{layout}\n",
        )
        after_user = self._replace_once(
            without_layout,
            "USER 10001:10001",
            f"USER 10001:10001\n\n{layout}",
        )
        mutations = {
            "mkdir-p": self._replace_once(
                source,
                f"RUN mkdir {self.HEALTH_ROOT} \\",
                f"RUN mkdir -p {self.HEALTH_ROOT} \\",
            ),
            "wrong-path": self._replace_once(
                source,
                f"RUN mkdir {self.HEALTH_ROOT} \\",
                "RUN mkdir /run/other-health \\",
            ),
            "missing-touch": self._replace_once(
                source,
                f"    && touch {self.HEALTH_LOCK} \\",
                "",
            ),
            "redirection": self._replace_once(
                source,
                f"    && touch {self.HEALTH_LOCK} \\",
                f"    && touch {self.HEALTH_LOCK} > /dev/null \\",
            ),
            "pipeline": self._replace_once(
                source,
                f"    && touch {self.HEALTH_LOCK} \\",
                f"    && touch {self.HEALTH_LOCK} | true \\",
            ),
            "expansion": self._replace_once(
                source,
                f"RUN mkdir {self.HEALTH_ROOT} \\",
                "RUN mkdir \"$HEALTH_ROOT\" \\",
            ),
            "extra-command": self._replace_once(
                source,
                f"    && chmod 0440 {self.HEALTH_LOCK}",
                f"    && chmod 0440 {self.HEALTH_LOCK} && true",
            ),
            "semicolon-chain": self._replace_once(
                source,
                f"    && chmod 0440 {self.HEALTH_LOCK}",
                f"    ; chmod 0440 {self.HEALTH_LOCK}",
            ),
            "owner": self._replace_once(source, "&& chown 0:10001", "&& chown 10001:10001"),
            "root-writable-records": self._replace_once(
                source,
                f"&& chmod 0750 {self.HEALTH_ROOT} {self.HEALTH_RECORDS}",
                f"&& chmod 0770 {self.HEALTH_ROOT} {self.HEALTH_RECORDS}",
            ),
            "writable-lock": self._replace_once(
                source,
                f"&& chmod 0440 {self.HEALTH_LOCK}",
                f"&& chmod 0660 {self.HEALTH_LOCK}",
            ),
            "inside-guard-stage": guard_stage,
            "after-user": after_user,
            "enabled-mode": self._replace_once(
                source,
                "ENV OWNER_BETA_SIMULATION_MODE=disabled",
                "ENV OWNER_BETA_SIMULATION_MODE=enabled",
            ),
            "root-user": self._replace_once(source, "USER 10001:10001", "USER 0:0"),
        }
        for name, mutated in mutations.items():
            with self.subTest(mutation=name), self.assertRaises(ValueError):
                self._validated_health_layout_body(mutated)

    def test_image_health_layout_capture_stubs_prove_order_and_short_circuit(self) -> None:
        body = self._validated_health_layout_body(DOCKERFILE.read_text(encoding="utf-8"))
        expected = [
            ("mkdir", self.HEALTH_ROOT),
            ("mkdir", self.HEALTH_RECORDS),
            ("touch", self.HEALTH_LOCK),
            ("chown", "0:10001", self.HEALTH_ROOT, self.HEALTH_RECORDS, self.HEALTH_LOCK),
            ("chmod", "0750", self.HEALTH_ROOT, self.HEALTH_RECORDS),
            ("chmod", "0440", self.HEALTH_LOCK),
        ]

        result, rows, stub_names = self._capture_health_layout(body)
        self.assertEqual(result.returncode, 0, result.stderr.decode(errors="replace"))
        self.assertEqual(result.stdout, b"")
        self.assertEqual(result.stderr, b"")
        self.assertEqual(stub_names, ["chmod", "chown", "mkdir", "touch"])
        self.assertEqual(rows, expected)

        failed, stopped_rows, failure_stub_names = self._capture_health_layout(body, fail_at="touch")
        self.assertEqual(failed.returncode, 37)
        self.assertEqual(failed.stdout, b"")
        self.assertEqual(failed.stderr, b"")
        self.assertEqual(failure_stub_names, ["chmod", "chown", "mkdir", "touch"])
        self.assertEqual(stopped_rows, expected[:3])

    def test_image_copy_sources_exist_in_real_guard_output(self) -> None:
        spec = importlib.util.spec_from_file_location(
            "simulation_artifact_fixtures",
            ROOT / "scripts/ops/test_release_simulation_artifact.py",
        )
        fixtures = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(fixtures)
        with tempfile.TemporaryDirectory(prefix="wp5b2-guard-image-") as temporary:
            fixture = fixtures.D8BundleFixture(Path(temporary))
            guarded = fixture.guard()
            self.assertEqual(guarded.returncode, 0, guarded.stderr.decode(errors="replace"))
            expected = {
                RUNNER_PATH: (fixture.binary, 0o755),
                ARTIFACT_JSON: (fixture.artifact_path.read_bytes(), 0o444),
                COMPLETE: (fixture.producer_complete_path.read_bytes(), 0o444),
                "/usr/local/bin/owner-beta-simulation-entrypoint": (fixture.entry_bytes, 0o755),
            }
            copies = [shlex.split(line) for line in DOCKERFILE.read_text().splitlines()
                      if line.startswith("COPY --from=verified-artifacts ")]
            self.assertEqual({parts[-1] for parts in copies}, set(expected))
            for parts in copies:
                source, destination = parts[-2:]
                with self.subTest(source=source):
                    self.assertTrue(source.startswith("/build/"))
                    output_file = fixture.output / source.removeprefix("/build/")
                    self.assertTrue(output_file.is_file(), "COPY source missing from real guard output")
                    content, mode = expected[destination]
                    self.assertEqual(output_file.read_bytes(), content)
                    self.assertEqual(stat.S_IMODE(output_file.stat().st_mode), mode)

    def test_revision_guard_rejects_noncanonical_commit_values(self) -> None:
        dockerfile = DOCKERFILE.read_text(encoding="utf-8")
        guard = dockerfile.split("ARG LAGRANGE_CODE_COMMIT\n", 1)[1].split(
            "    && apk add ", 1
        )[0].strip()
        self.assertTrue(guard.startswith("RUN "))
        guard = guard.removeprefix("RUN ").removesuffix("\\").rstrip()

        def check(value: str) -> subprocess.CompletedProcess:
            return subprocess.run(
                ["/bin/sh", "-c", guard],
                env={"PATH": "/usr/bin:/bin", "LAGRANGE_CODE_COMMIT": value},
                capture_output=True,
                timeout=5,
                check=False,
            )

        valid = "0123456789abcdef" * 2 + "01234567"
        self.assertEqual(check(valid).returncode, 0)
        for value in (
            "", "0" * 40, "a" * 39, "a" * 41, "A" * 40, "g" * 40,
            valid + "\n", "\n" + valid, valid + "\ninvalid",
            "invalid\n" + valid, valid + "\n" + valid, valid + "\r",
            " " + valid, valid + " ", valid + "\t",
        ):
            with self.subTest(value=value):
                self.assertNotEqual(check(value).returncode, 0)

    def test_modes_and_exact_pinned_guarded_image_contract(self) -> None:
        self.assertEqual(stat.S_IMODE(DOCKERFILE.stat().st_mode), 0o644)
        self.assertEqual(stat.S_IMODE(ENTRYPOINT.stat().st_mode), 0o755)
        self.assertEqual(stat.S_IMODE(Path(__file__).stat().st_mode), 0o644)

        dockerfile = DOCKERFILE.read_text(encoding="utf-8")
        lines = dockerfile.splitlines()
        normalized_lines = [line.strip() for line in lines]
        self.assertEqual(
            [line for line in lines if line.startswith("FROM ")],
            [
                f"FROM {PINNED_ALPINE} AS verified-artifacts",
                f"FROM {PINNED_ALPINE}",
            ],
        )
        self.assertNotIn("RUST_ARTIFACT_SOURCE", dockerfile)
        self.assertNotIn("source-builder", dockerfile)
        self.assertNotIn("cargo", dockerfile.lower())
        self.assertNotIn("rustc", dockerfile.lower())

        self.assertIn("COPY --from=release_artifacts / /release-artifacts", lines)
        self.assertEqual(dockerfile.count("ARG RUST_ARTIFACT_BUNDLE_SHA256"), 1)
        self.assertEqual(dockerfile.count("ARG RUST_ARTIFACT_HELPER_SHA256"), 1)
        self.assertIn(
            "&& test -n \"$RUST_ARTIFACT_BUNDLE_SHA256\" \\",
            normalized_lines,
        )
        self.assertIn(
            "&& test -n \"$RUST_ARTIFACT_HELPER_SHA256\" \\",
            normalized_lines,
        )
        self.assertIn(
            "&& test \"$(sha256sum /release-artifacts/.release-build/release-build-layout.sh | awk '{print $1}')\" = \"$RUST_ARTIFACT_HELPER_SHA256\" \\",
            normalized_lines,
        )
        self.assertIn(
            "--guard-build /release-artifacts/.release-build/request.json /release-artifacts /build",
            dockerfile,
        )
        self.assertIn("RELEASE_BUILD_LAYOUT_EXECUTION=1", dockerfile)

        expected_copies = [
            "COPY --from=verified-artifacts --chown=0:0 --chmod=0755 "
            "/build/target/release/owner-beta-simulation-runner "
            "/usr/local/bin/owner-beta-simulation-runner",
            "COPY --from=verified-artifacts --chown=0:0 --chmod=0444 "
            "/build/provenance/owner-beta-simulation-runner/artifact.json "
            "/usr/share/lagrange/owner-beta-simulation/artifact.json",
            "COPY --from=verified-artifacts --chown=0:0 --chmod=0444 "
            "/build/provenance/owner-beta-simulation-runner/COMPLETE "
            "/usr/share/lagrange/owner-beta-simulation/COMPLETE",
            "COPY --from=verified-artifacts --chown=0:0 --chmod=0755 "
            "/build/deploy/runtime/owner-beta-simulation-entrypoint "
            "/usr/local/bin/owner-beta-simulation-entrypoint",
        ]
        self.assertEqual(
            [line for line in lines if line.startswith("COPY --from=verified-artifacts ")],
            expected_copies,
        )
        self.assertNotIn("COPY deploy/runtime/owner-beta-simulation-entrypoint", lines)

        self.assertIn("&& test \"$LAGRANGE_CODE_COMMIT\" != 0000000000000000000000000000000000000000 \\", normalized_lines)
        self.assertIn(
            "&& printf '%s' \"$LAGRANGE_CODE_COMMIT\" | grep -Eq '^[0-9a-f]{40}$' \\",
            normalized_lines,
        )
        self.assertIn('LABEL org.opencontainers.image.revision="$LAGRANGE_CODE_COMMIT"', lines)
        self.assertIn("&& apk add --no-cache ca-certificates libgcc libstdc++ libpq libssl3 \\", normalized_lines)
        self.assertIn("&& addgroup -S -g 10001 lagrange \\", normalized_lines)
        self.assertIn("&& adduser -S -D -H -u 10001 -G lagrange lagrange \\", normalized_lines)
        self.assertIn("USER 10001:10001", lines)
        self.assertIn("STOPSIGNAL SIGTERM", lines)
        self.assertIn('ENV APP_ENV=production', lines)
        self.assertIn('ENV OWNER_BETA_SIMULATION_MODE=disabled', lines)
        self.assertIn('ENV OWNER_BETA_ACCESS_MODE=disabled', lines)
        self.assertIn('ENV OWNER_BETA_PRICE_INPUT_MODE=disabled', lines)
        self.assertIn(
            'ENTRYPOINT ["/usr/local/bin/owner-beta-simulation-entrypoint"]',
            lines,
        )
        self.assertNotIn("HEALTHCHECK", dockerfile)


class EntrypointFixture:
    """Run a relocated copy with synthetic receipt files and a capture-only runner."""

    def __init__(self, root: Path):
        self.root = root
        self.runner = root / "synthetic-runner"
        self.artifact_root = root / "approved-artifact-root"
        self.receipt = root / "receipt" / "artifact.json"
        self.complete = root / "receipt" / "COMPLETE"
        self.alternate = root / "alternate"
        self.capture = root / "captured-argv.bin"
        self.entrypoint = root / "relocated-entrypoint"
        self.launcher = root / "launcher.sh"
        self.hostile_bin = root / "hostile-bin"
        self.hostile_bin.mkdir()
        self.receipt.parent.mkdir()

        self.runner.write_text(
            "#!/bin/sh\n"
            ": \"${CAPTURE_PATH:?}\"\n"
            "printf '%s\\000' \"$@\" > \"$CAPTURE_PATH\"\n",
            encoding="utf-8",
        )
        self.runner.chmod(0o755)
        self.receipt.write_bytes(SYNTHETIC_RECEIPT)
        self.receipt.chmod(0o444)
        self._receipt_sha256 = hashlib.sha256(SYNTHETIC_RECEIPT).hexdigest()
        self.complete.write_bytes(self.receipt_sha256.encode("ascii") + b"\n")
        self.complete.chmod(0o444)
        self.alternate.write_bytes(b"different synthetic bytes\n")

        source = ENTRYPOINT.read_text(encoding="utf-8")
        constants = {
            f"RUNNER={RUNNER_PATH}": f"RUNNER={shlex.quote(str(self.runner))}",
            f"ARTIFACT_JSON={ARTIFACT_JSON}": f"ARTIFACT_JSON={shlex.quote(str(self.receipt))}",
            f"COMPLETE={COMPLETE}": f"COMPLETE={shlex.quote(str(self.complete))}",
        }
        for original, relocated in constants.items():
            self._replace_exactly_once(source, original, relocated)
            source = source.replace(original, relocated, 1)
        self.entrypoint.write_text(source, encoding="utf-8")
        self.entrypoint.chmod(0o644)

        self.launcher.write_text(
            "#!/bin/sh\n"
            "stat() {\n"
            "  case \"$1\" in -c) ;; *) command stat \"$@\"; return $? ;; esac\n"
            "  case \"$3\" in\n"
            "    \"$TEST_ARTIFACT_JSON\"|\"$TEST_COMPLETE\")\n"
            "      [ -f \"$3\" ] && [ ! -L \"$3\" ] || return 1\n"
            "      file_type=$(command stat -c %F \"$3\" 2>/dev/null) || return 1\n"
            "      file_mode=$(command stat -c %a \"$3\" 2>/dev/null) || return 1\n"
            "      owner=${TEST_STAT_OWNER:-0:0}\n"
            "      printf '%s:%s:%s:%s\\n' \"$file_type\" \"${owner%%:*}\" \"${owner#*:}\" \"$file_mode\"\n"
            "      ;;\n"
            "    *) command stat \"$@\" ;;\n"
            "  esac\n"
            "}\n"
            ". \"$TEST_ENTRYPOINT\"\n",
            encoding="utf-8",
        )

    @property
    def receipt_sha256(self) -> str:
        return self._receipt_sha256

    def rewrite_receipt(self, data: bytes) -> None:
        self.receipt.chmod(0o600)
        self.receipt.write_bytes(data)
        self.receipt.chmod(0o444)

    def rewrite_complete(self, data: bytes) -> None:
        self.complete.chmod(0o600)
        self.complete.write_bytes(data)
        self.complete.chmod(0o444)

    @staticmethod
    def _replace_exactly_once(source: str, old: str, new: str) -> None:
        if source.count(old + "\n") != 1:
            raise AssertionError(f"fixed path constant was not unique: {old}")


class EntrypointBehaviorTests(unittest.TestCase):
    def launch(
        self,
        *,
        args: tuple[str, ...] = (),
        mutate=None,
        env_overrides: dict[str, str] | None = None,
        stat_owner: str = "0:0",
    ):
        with tempfile.TemporaryDirectory(prefix="wp5b2-simulation-image-") as temporary:
            fixture = EntrypointFixture(Path(temporary))
            if mutate is not None:
                mutate(fixture)

            environment = {
                "APP_ENV": "production",
                "OWNER_BETA_SIMULATION_MODE": "owner_only",
                "OWNER_BETA_ACCESS_MODE": "owner_only",
                "OWNER_BETA_PRICE_INPUT_MODE": "sealed_v1",
                "LAGRANGE_CODE_COMMIT": "a" * 40,
                "OWNER_BETA_SIMULATION_CONFIG_SHA256": "sha256:" + "b" * 64,
                "OWNER_BETA_SIMULATION_RECEIPT_SHA256": fixture.receipt_sha256,
                "CAPTURE_PATH": str(fixture.capture),
                "TEST_ENTRYPOINT": str(fixture.entrypoint),
                "TEST_ARTIFACT_JSON": str(fixture.receipt),
                "TEST_COMPLETE": str(fixture.complete),
                "TEST_STAT_OWNER": stat_owner,
                "PATH": str(fixture.hostile_bin),
                "LC_ALL": "C",
            }
            if env_overrides:
                environment.update(env_overrides)
            result = subprocess.run(
                ["/bin/sh", str(fixture.launcher), *args],
                env=environment,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
                timeout=10,
            )
            captured = fixture.capture.read_bytes() if fixture.capture.exists() else None
            artifact_root_exists = fixture.artifact_root.exists()
            return (
                result,
                captured,
                artifact_root_exists,
                fixture.receipt_sha256,
                str(fixture.artifact_root),
                str(fixture.receipt),
            )

    def assert_rejected(self, result: subprocess.CompletedProcess, captured: bytes | None) -> None:
        self.assertEqual(result.returncode, 64)
        self.assertIsNone(captured, "fake runner must not be reached on rejection")
        self.assertEqual(result.stdout, b"")
        self.assertEqual(result.stderr, STATIC_ERROR)

    def test_valid_fixture_execs_exact_fixed_protocol_without_reading_approved_root(self) -> None:
        result, captured, artifact_root_exists, digest, expected_root, expected_receipt = self.launch()
        self.assertEqual(result.returncode, 0, result.stderr.decode("utf-8", errors="replace"))
        self.assertEqual(result.stderr, b"")
        self.assertIsNotNone(captured)
        argv = [part.decode("utf-8") for part in captured.split(b"\0")[:-1]]
        self.assertEqual(
            argv,
            [
                "--official-daemon",
                "--expected-commit",
                "a" * 40,
                "--expected-config-sha256",
                "sha256:" + "b" * 64,
                "--expected-receipt-sha256",
                digest,
            ],
        )
        self.assertFalse(artifact_root_exists, "the shim passes pins and does not open the input root")

    def test_hostile_ambient_paths_and_hashes_do_not_change_fixed_exec_arguments(self) -> None:
        overrides = {
            "RUNNER": "/hostile/runner",
            "ARTIFACT_ROOT": "/hostile/root",
            "ARTIFACT_JSON": "/hostile/receipt",
            "COMPLETE": "/hostile/COMPLETE",
            "WORKER_ID": "hostile-worker",
            "receipt_sha256": "0" * 64,
            "artifact_sha256": "f" * 64,
            "OWNER_BETA_SIMULATION_RUNNER": "/hostile/runner",
            "OWNER_BETA_SIMULATION_ARTIFACT_ROOT": "/hostile/root",
            "OWNER_BETA_SIMULATION_BUILD_RECEIPT": "/hostile/receipt",
            "OWNER_BETA_SIMULATION_EXPECTED_RECEIPT_SHA256": "0" * 64,
            "OWNER_BETA_SIMULATION_WORKER_ID": "hostile-worker",
        }
        result, captured, artifact_root_exists, digest, expected_root, expected_receipt = self.launch(
            env_overrides=overrides
        )
        self.assertEqual(result.returncode, 0, result.stderr.decode("utf-8", errors="replace"))
        self.assertIsNotNone(captured)
        argv = [part.decode("utf-8") for part in captured.split(b"\0")[:-1]]
        self.assertEqual(
            argv,
            [
                "--official-daemon",
                "--expected-commit",
                "a" * 40,
                "--expected-config-sha256",
                "sha256:" + "b" * 64,
                "--expected-receipt-sha256",
                digest,
            ],
        )
        self.assertFalse(artifact_root_exists)

    def test_closed_configuration_and_argument_checks_reject_from_valid_baseline(self) -> None:
        cases = [
            ("disabled simulation", {"OWNER_BETA_SIMULATION_MODE": "disabled"}),
            ("unknown simulation", {"OWNER_BETA_SIMULATION_MODE": "anything-else"}),
            ("wrong access", {"OWNER_BETA_ACCESS_MODE": "public"}),
            ("wrong input", {"OWNER_BETA_PRICE_INPUT_MODE": "raw"}),
            ("wrong app env", {"APP_ENV": "development"}),
            ("missing commit", {"LAGRANGE_CODE_COMMIT": ""}),
            ("zero commit", {"LAGRANGE_CODE_COMMIT": "0" * 40}),
            ("bad config pin", {"OWNER_BETA_SIMULATION_CONFIG_SHA256": "b" * 64}),
            ("bad receipt pin", {"OWNER_BETA_SIMULATION_RECEIPT_SHA256": "f" * 64}),
        ]
        for label, overrides in cases:
            with self.subTest(label=label):
                result, captured, _, _, _, _ = self.launch(env_overrides=overrides)
                self.assert_rejected(result, captured)

        result, captured, _, _, _, _ = self.launch(args=("--help",))
        self.assert_rejected(result, captured)

    def test_complete_rejects_missing_symlink_directory_and_invalid_bytes(self) -> None:
        valid = hashlib.sha256(SYNTHETIC_RECEIPT).hexdigest().encode("ascii")
        invalid = {
            "missing": lambda f: f.complete.unlink(),
            "symlink": lambda f: (f.complete.unlink(), f.complete.symlink_to(f.alternate)),
            "directory": lambda f: (f.complete.unlink(), f.complete.mkdir()),
            "empty": lambda f: f.rewrite_complete(b""),
            "short": lambda f: f.rewrite_complete(valid[:-1] + b"\n"),
            "long": lambda f: f.rewrite_complete(valid + b"0\n"),
            "nonhex": lambda f: f.rewrite_complete(b"g" * 64 + b"\n"),
            "uppercase": lambda f: f.rewrite_complete(valid.upper() + b"\n"),
            "CRLF": lambda f: f.rewrite_complete(valid + b"\r\n"),
            "missing LF": lambda f: f.rewrite_complete(valid),
            "multiline": lambda f: f.rewrite_complete(valid + b"\n\n"),
            "NUL": lambda f: f.rewrite_complete(valid[:20] + b"\0" + valid[21:] + b"\n"),
        }
        for label, mutate in invalid.items():
            with self.subTest(label=label):
                result, captured, _, _, _, _ = self.launch(mutate=mutate)
                self.assert_rejected(result, captured)

    def test_receipt_rejects_missing_symlink_directory_empty_and_wrong_digest(self) -> None:
        def wrong_digest(f: EntrypointFixture) -> None:
            f.rewrite_complete(
                hashlib.sha256(b"different synthetic bytes").hexdigest().encode("ascii") + b"\n"
            )

        cases = {
            "missing": lambda f: f.receipt.unlink(),
            "symlink": lambda f: (f.receipt.unlink(), f.receipt.symlink_to(f.alternate)),
            "directory": lambda f: (f.receipt.unlink(), f.receipt.mkdir()),
            "empty": lambda f: f.rewrite_receipt(b""),
            "wrong raw hash": wrong_digest,
        }
        for label, mutate in cases.items():
            with self.subTest(label=label):
                result, captured, _, _, _, _ = self.launch(mutate=mutate)
                self.assert_rejected(result, captured)

    def test_self_consistent_replaced_receipt_does_not_replace_independent_pin(self) -> None:
        def replace_both(f: EntrypointFixture) -> None:
            replacement = b"different synthetic receipt\n"
            f.rewrite_receipt(replacement)
            f.rewrite_complete(hashlib.sha256(replacement).hexdigest().encode("ascii") + b"\n")

        result, captured, _, _, _, _ = self.launch(mutate=replace_both)
        self.assert_rejected(result, captured)

    def test_wrong_modes_and_owner_are_rejected(self) -> None:
        cases = [
            ("receipt mode", lambda f: f.receipt.chmod(0o440), "0:0"),
            ("COMPLETE mode", lambda f: f.complete.chmod(0o400), "0:0"),
            ("owner", None, "1000:1000"),
        ]
        for label, mutate, owner in cases:
            with self.subTest(label=label):
                result, captured, _, _, _, _ = self.launch(mutate=mutate, stat_owner=owner)
                self.assert_rejected(result, captured)


if __name__ == "__main__":
    unittest.main()
