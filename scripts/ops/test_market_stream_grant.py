"""Pure and fake-adapter tests for the private market-stream grant installer."""

from __future__ import annotations

import hashlib
import importlib.util
import io
import json
import os
import pathlib
import signal
import stat
import subprocess
import sys
import tempfile
import types
import unittest
from unittest import mock

_LIBRARY_PATH = pathlib.Path(__file__).with_name("lib") / "market-stream-grant.py"
_SPEC = importlib.util.spec_from_file_location("market_stream_grant_under_test", _LIBRARY_PATH)
assert _SPEC is not None and _SPEC.loader is not None
grant = importlib.util.module_from_spec(_SPEC)
sys.modules[_SPEC.name] = grant
_SPEC.loader.exec_module(grant)


COMMIT = "1234567890abcdef1234567890abcdef12345678"
SLOT = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"
GRANT = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"
REVISION = "cccccccc-cccc-4ccc-8ccc-cccccccccccc"
OWNER = "dddddddd-dddd-4ddd-8ddd-dddddddddddd"
ENTITLEMENT = "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee"
IMAGE_ID = "sha256:" + "1" * 64
IMAGE = "sha256:" + "2" * 64
OPERATION = "0123456789abcdef0123456789abcdef"


def approval_fixture() -> dict[str, object]:
    return {
        "schema_version": 1,
        "scope": "owner-market-stream-grant",
        "grant_id": GRANT,
        "grant_revision": REVISION,
        "credential_slot_id": SLOT,
        "credential_generation": "7",
        "owner_user_id": OWNER,
        "entitlement_id": ENTITLEMENT,
        "entitlement_reference": 'reviewed "owner" reference',
        "entitlement_document_sha256": "a" * 64,
        "tr_id": "H0STCNT0",
        "wire_version": "kis-h0stcnt0-20260914-v1",
        "network_contract_sha256": "b" * 64,
        "identity_list_sha256": "0e6a9b3aef6b310685b9bd5594a39452c2902d11af623197699cf6dc46931e79",
        "effective_from": "2026-01-01",
        "effective_until": "2026-12-31",
        "activation_commit": COMMIT,
    }


def encoded_approval(payload: dict[str, object] | None = None) -> bytes:
    return grant._canonical_json(approval_fixture() if payload is None else payload)


def context_fixture() -> grant.InstalledContext:
    root = pathlib.Path("/opt/lagrange")
    release = root / "releases" / COMMIT
    ops = release / "scripts" / "ops"
    compose = release / "deploy" / "compose"
    return grant.InstalledContext(
        root=root,
        release_dir=release,
        commit=COMMIT,
        wrapper=ops / "install-owner-market-stream-grant.sh",
        helper=ops / "lib" / "market-stream-grant.py",
        sql_file=ops / "lib" / "market-stream-grant.sql",
        compose_file=compose / "compose.yml",
        env_file=compose / ".env",
        manifest_file=release / ".lagrange-release-manifest",
        slot_id=SLOT,
        credential_generation="7",
        configured_grant_id=GRANT,
        contract_sha256="b" * 64,
        db_migrate_image_id=IMAGE_ID,
        db_migrate_revision=COMMIT,
    )


def parser_snapshot(context: grant.InstalledContext) -> tuple[str, ...]:
    return (
        context.commit,
        context.slot_id,
        context.credential_generation,
        context.configured_grant_id,
        context.contract_sha256,
        context.db_migrate_image_id,
        context.db_migrate_revision,
    )


class MarketStreamGrantTests(unittest.TestCase):
    def test_default_plan_does_not_touch_installed_context_or_process(self) -> None:
        output = io.StringIO()
        with (
            mock.patch.object(grant, "verify_installed_context", side_effect=AssertionError),
            mock.patch.object(grant, "read_trusted_approval_file", side_effect=AssertionError),
            mock.patch.object(grant, "run_db_operation", side_effect=AssertionError),
        ):
            status = grant.main(
                [],
                output=output,
                operation_id_factory=lambda: OPERATION,
            )
        self.assertEqual(status, "PLAN")
        self.assertEqual(
            output.getvalue(),
            "MARKET_STREAM_GRANT: PLAN operation_id=" + OPERATION + "\n",
        )

    def test_canonical_approval_and_exact_digest_are_accepted(self) -> None:
        raw = encoded_approval()
        payload = grant.parse_approval_bytes(raw, hashlib.sha256(raw).hexdigest())
        self.assertEqual(payload, approval_fixture())
        self.assertTrue(raw.endswith(b"\n"))
        self.assertFalse(raw.endswith(b"\n\n"))
        unicode_payload = {**approval_fixture(), "entitlement_reference": "소유자 검토 근거"}
        literal = encoded_approval(unicode_payload)
        self.assertIn("소유자".encode("utf-8"), literal)
        self.assertEqual(grant.parse_approval_bytes(literal, hashlib.sha256(literal).hexdigest()), unicode_payload)
        escaped = json.dumps(unicode_payload, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode() + b"\n"
        with self.assertRaises(grant.GrantFailure):
            grant.parse_approval_bytes(escaped, hashlib.sha256(escaped).hexdigest())

    def test_noncanonical_and_malformed_approvals_fail_closed(self) -> None:
        valid = approval_fixture()
        raw = encoded_approval(valid)
        duplicate = raw[:-2] + b',"scope":"owner-market-stream-grant"}\n'
        with self.assertRaises(grant.GrantFailure):
            grant.parse_approval_bytes(duplicate, hashlib.sha256(duplicate).hexdigest())

        unknown = dict(valid)
        unknown["operator_note"] = "not admitted"
        unknown_raw = encoded_approval(unknown)
        with self.assertRaises(grant.GrantFailure):
            grant.parse_approval_bytes(unknown_raw, hashlib.sha256(unknown_raw).hexdigest())

        spaced = json.dumps(valid, sort_keys=True, indent=2).encode("utf-8") + b"\n"
        with self.assertRaises(grant.GrantFailure):
            grant.parse_approval_bytes(spaced, hashlib.sha256(spaced).hexdigest())

        invalid_values = []
        bad = dict(valid)
        bad["schema_version"] = True
        invalid_values.append(bad)
        bad = dict(valid)
        bad["schema_version"] = 1.0
        invalid_values.append(bad)
        bad = dict(valid)
        bad["entitlement_id"] = None
        invalid_values.append(bad)
        bad = dict(valid)
        bad["grant_id"] = "00000000-0000-0000-0000-000000000000"
        invalid_values.append(bad)
        bad = dict(valid)
        bad["credential_generation"] = "07"
        invalid_values.append(bad)
        bad = dict(valid)
        bad["credential_generation"] = "18446744073709551616"
        invalid_values.append(bad)
        bad = dict(valid)
        bad["effective_from"] = "2026-02-30"
        invalid_values.append(bad)
        bad = dict(valid)
        bad["effective_until"] = "2025-12-31"
        invalid_values.append(bad)
        bad = dict(valid)
        bad["wire_version"] = "other"
        invalid_values.append(bad)
        bad = dict(valid)
        bad["identity_list_sha256"] = "f" * 64
        invalid_values.append(bad)
        for invalid in invalid_values:
            invalid_raw = encoded_approval(invalid)
            with self.subTest(payload=invalid):
                with self.assertRaises(grant.GrantFailure):
                    grant.parse_approval_bytes(
                        invalid_raw, hashlib.sha256(invalid_raw).hexdigest()
                    )
        with self.assertRaises(grant.GrantFailure):
            grant.parse_approval_bytes(raw, "0" * 64)
        nonfinite = raw.replace(b'"schema_version":1', b'"schema_version":NaN')
        with self.assertRaises(grant.GrantFailure):
            grant.parse_approval_bytes(nonfinite, hashlib.sha256(nonfinite).hexdigest())

    def test_input_file_metadata_nofollow_links_and_size_fail_closed(self) -> None:
        good = types.SimpleNamespace(
            st_mode=stat.S_IFREG | 0o600,
            st_uid=0,
            st_gid=0,
            st_nlink=1,
            st_size=17,
            st_dev=1,
            st_ino=2,
        )
        self.assertTrue(grant.approval_file_stat_is_trusted(good))
        for metadata in (
            replace_stat(good, st_mode=stat.S_IFREG | 0o640),
            replace_stat(good, st_uid=1000),
            replace_stat(good, st_gid=1000),
            replace_stat(good, st_nlink=2),
            replace_stat(good, st_size=grant.MAX_APPROVAL_BYTES + 1),
            replace_stat(good, st_mode=stat.S_IFLNK | 0o777),
        ):
            self.assertFalse(grant.approval_file_stat_is_trusted(metadata))

        with tempfile.TemporaryDirectory() as temporary:
            target = pathlib.Path(temporary) / "target.json"
            link = pathlib.Path(temporary) / "input.json"
            target.write_bytes(b"{}\n")
            link.symlink_to(target)
            with self.assertRaises(grant.GrantFailure):
                grant.read_trusted_approval_file(
                    str(link),
                    ancestor_guard=lambda directory: pathlib.Path(directory),
                    lstat_fn=lambda _: good,
                    metadata_guard=lambda _: True,
                    open_fn=os.open,
                )
            fifo = pathlib.Path(temporary) / "raced-fifo"
            os.mkfifo(fifo)
            with self.assertRaises(grant.GrantFailure):
                grant.read_trusted_approval_file(
                    str(fifo),
                    ancestor_guard=lambda directory: pathlib.Path(directory),
                    lstat_fn=lambda _: good,
                    metadata_guard=lambda metadata: stat.S_ISREG(metadata.st_mode),
                )

    def test_installed_binding_rejects_commit_current_image_and_env_drift(self) -> None:
        context = context_fixture()
        payload = approval_fixture()
        self.assertTrue(grant.installed_current_target_matches("releases/" + COMMIT, COMMIT))
        self.assertFalse(grant.installed_current_target_matches("releases/other", COMMIT))
        self.assertTrue(grant.context_snapshot_is_valid(parser_snapshot(context), COMMIT))
        self.assertTrue(
            grant.inherited_overrides_are_absent(
                COMMIT, {"LAGRANGE_CODE_COMMIT": COMMIT}
            )
        )
        self.assertFalse(grant.inherited_overrides_are_absent(COMMIT, {"DOCKER_HOST": "x"}))
        self.assertFalse(
            grant.inherited_overrides_are_absent(
                COMMIT, {"COMPOSE_PROJECT_NAME": "other"}
            )
        )
        self.assertFalse(
            grant.inherited_overrides_are_absent(
                COMMIT, {"LAGRANGE_CODE_COMMIT": "0" * 40}
            )
        )
        for changed in (
            (COMMIT[:-1] + "0", SLOT, "7", GRANT, "b" * 64, IMAGE_ID, COMMIT),
            (COMMIT, SLOT, "7", GRANT, "b" * 64, "invalid-image", COMMIT),
            (COMMIT, SLOT, "7", GRANT, "b" * 64, IMAGE_ID, "0" * 40),
            (COMMIT, "not-a-uuid", "7", GRANT, "b" * 64, IMAGE_ID, COMMIT),
            (COMMIT, SLOT, "07", GRANT, "b" * 64, IMAGE_ID, COMMIT),
        ):
            self.assertFalse(grant.context_snapshot_is_valid(changed, COMMIT))
        snapshot_calls: list[list[str]] = []
        snapshot_bytes = b"\n".join(item.encode("ascii") for item in parser_snapshot(context)) + b"\n"

        def parser_runner(argv: list[str], **_: object) -> subprocess.CompletedProcess[bytes]:
            snapshot_calls.append(argv)
            return subprocess.CompletedProcess(argv, 0, snapshot_bytes, b"")

        parsed = grant._shared_parser_snapshot(
            dotenv_library=context.release_dir / "scripts/ops/lib/dotenv.sh",
            manifest_library=context.release_dir / "scripts/ops/lib/release-image-manifest.sh",
            env_file=context.env_file,
            manifest_file=context.manifest_file,
            expected_commit=COMMIT,
            run_fn=parser_runner,
        )
        self.assertEqual(parsed, parser_snapshot(context))
        self.assertEqual(len(snapshot_calls), 1)
        self.assertEqual(snapshot_calls[0][:3], ["/usr/bin/env", "-i", "PATH=/usr/bin:/bin"])
        self.assertIn("/usr/bin/bash", snapshot_calls[0])
        self.assertIn('source "$1"', snapshot_calls[0][snapshot_calls[0].index("-c") + 1])
        expected_inspection = (IMAGE_ID + "|" + COMMIT + "\n").encode("ascii")
        self.assertTrue(grant.image_inspection_matches(context, 0, expected_inspection, b""))
        self.assertFalse(grant.image_inspection_matches(context, 0, (IMAGE + "|" + COMMIT + "\n").encode(), b""))
        self.assertFalse(grant.image_inspection_matches(context, 0, expected_inspection, b"warning"))
        self.assertTrue(grant.approval_matches_installed(payload, context, COMMIT))
        self.assertFalse(grant.approval_matches_installed(payload, context, "0" * 40))
        self.assertFalse(
            grant.approval_matches_installed(
                {**payload, "credential_slot_id": "ffffffff-ffff-4fff-8fff-ffffffffffff"},
                context,
                COMMIT,
            )
        )
        self.assertFalse(
            grant.approval_matches_installed(
                {**payload, "credential_generation": "8"}, context, COMMIT
            )
        )
        self.assertFalse(
            grant.approval_matches_installed(
                {**payload, "grant_id": "ffffffff-ffff-4fff-8fff-ffffffffffff"},
                context,
                COMMIT,
            )
        )
        self.assertFalse(
            grant.approval_matches_installed(
                {**payload, "network_contract_sha256": "c" * 64},
                context,
                COMMIT,
            )
        )

    def test_adapter_argv_uses_only_pinned_db_migrate_inputs(self) -> None:
        context = context_fixture()
        override = pathlib.Path("/root-owned-temp/compose.override.yml")
        payload = '{"entitlement_reference":"reviewed quote"}'
        argv, name = grant.build_compose_argv(
            context, override, OPERATION, "install", payload, GRANT
        )
        self.assertEqual(name, "market-stream-grant-" + OPERATION)
        self.assertEqual(argv[0:3], ["/usr/bin/docker", "--host", grant.DOCKER_HOST])
        self.assertIn("--project-name", argv)
        self.assertIn("lagrange-station", argv)
        self.assertEqual(argv[argv.index("--pull") + 1], "never")
        self.assertIn("--no-deps", argv)
        self.assertIn("-T", argv)
        self.assertEqual(argv[argv.index("--label") + 1], grant.CONTAINER_LABEL + "=" + OPERATION)
        self.assertEqual(argv[argv.index("--entrypoint") + 1], "/bin/sh")
        self.assertIn("db-migrate", argv)
        self.assertIn("grant_payload=" + payload, argv)
        self.assertIn("grant_mode=install", argv)
        self.assertIn("grant_id=" + GRANT, argv)
        self.assertNotIn("--no-build", argv)
        self.assertFalse(any(word.lower() in {"appkey", "appsecret", "approval_key"} for word in argv))
        override_text = grant.render_compose_override(IMAGE_ID).decode("ascii")
        self.assertIn('image: "' + IMAGE_ID + '"', override_text)
        self.assertIn("build: !reset null", override_text)
        self.assertIn("pull_policy: never", override_text)
        self.assertIn("healthcheck:\n      disable: true", override_text)
        self.assertEqual(grant.image_inspect_argv(context)[-1], IMAGE_ID)

    def test_timeout_cleanup_targets_only_owned_container_without_retry(self) -> None:
        context = context_fixture()
        container_id = "c" * 64
        name = "market-stream-grant-" + OPERATION
        for label, image_id, state, rm_exit, remains, expected in (
            (OPERATION, IMAGE_ID, "true", 0, False, True),
            (OPERATION, IMAGE_ID, "false", 0, False, True),
            ("foreign", IMAGE_ID, "true", 0, False, False),
            (OPERATION, IMAGE, "true", 0, False, False),
            (OPERATION, IMAGE_ID, "true", 1, True, False),
            (OPERATION, IMAGE_ID, "true", 0, True, False),
        ):
            with self.subTest(label=label, image=image_id, state=state, rm_exit=rm_exit, remains=remains):
                calls = []
                removed = False

                def runner(argv, **kwargs):
                    nonlocal removed
                    calls.append(argv)
                    self.assertEqual(kwargs["timeout"], 10)
                    if argv[3] == "ps":
                        out = (container_id + "\n").encode() if not removed or remains else b""
                        return subprocess.CompletedProcess(argv, 0, out, b"")
                    if argv[3:5] == ["container", "inspect"]:
                        self.assertEqual(argv[-1], container_id)
                        self.assertIn("{{.Image}}", argv[-2])
                        self.assertNotIn(".Config.Image", argv[-2])
                        out = f"{container_id}|{image_id}|{label}|/{name}|{state}\n".encode()
                        return subprocess.CompletedProcess(argv, 0, out, b"")
                    self.assertEqual(argv[3:], ["container", "rm", "--force", container_id])
                    removed = True
                    return subprocess.CompletedProcess(argv, rm_exit, (container_id + "\n").encode(), b"")

                self.assertEqual(grant.cleanup_exact_container(context, OPERATION, name, run_fn=runner), expected)
                if label != OPERATION or image_id != IMAGE_ID:
                    self.assertFalse(removed, "foreign identity must never be removed")
                if expected:
                    self.assertEqual([call[-1] for call in calls[-2:]], ["id=" + container_id, "name=^/" + name + "$"])
        for out, err, rc, expected in (
            (b"", b"", 0, True),
            (b"", b"unavailable", 1, False),
            ((container_id + "\n") .encode() * 2, b"", 0, False),
            (b"truncated-id\n", b"", 0, False),
        ):
            calls = []
            def listing(argv, **kwargs):
                calls.append(argv)
                return subprocess.CompletedProcess(argv, rc, out, err)
            self.assertEqual(grant.cleanup_exact_container(context, OPERATION, name, run_fn=listing), expected)
            self.assertEqual(len(calls), 1)

    def test_all_operation_outcomes_require_cleanup_and_never_retry(self) -> None:
        context = context_fixture()
        cases = [
            ("install", b"APPLIED\n", b"", 0, None, "APPLIED"),
            ("install", b"ALREADY_APPLIED\n", b"", 0, None, "ALREADY_APPLIED"),
            ("revoke", b"REVOKED\n", b"", 0, None, "REVOKED"),
            ("revoke", b"ALREADY_REVOKED\n", b"", 0, None, "ALREADY_REVOKED"),
            ("install", b"REVOKED\n", b"", 0, None, "DB_OUTCOME_UNKNOWN"),
            ("revoke", b"APPLIED\n", b"", 0, None, "DB_OUTCOME_UNKNOWN"),
            ("install", b"APPLIED\n", b"private detail", 0, None, "DB_OUTCOME_UNKNOWN"),
            ("install", b"malformed\n", b"", 0, None, "DB_OUTCOME_UNKNOWN"),
            ("install", b"APPLIED\n", b"", 1, None, "DB_OUTCOME_UNKNOWN"),
            ("install", b"", b"", None, "timeout", "DB_OUTCOME_UNKNOWN"),
            ("install", b"", b"", None, "io", "DB_OUTCOME_UNKNOWN"),
            ("install", b"", b"", None, "spawn", "DB_OUTCOME_UNKNOWN"),
        ]
        for mode, stdout, stderr, rc, failure, expected in cases:
            for cleanup_ok in (True, False):
                with self.subTest(mode=mode, rc=rc, failure=failure, stdout=stdout, cleanup=cleanup_ok):
                    sequence = []
                    class Process:
                        pid = 4242
                        returncode = None
                        count = 0
                        def communicate(self, input=None, timeout=None):
                            self.count += 1
                            if self.count == 1:
                                self_test.assertEqual(input, b"BEGIN;\nCOMMIT;\n")
                                self_test.assertEqual(timeout, 90)
                                if failure == "timeout":
                                    raise subprocess.TimeoutExpired(["docker"], timeout)
                                if failure == "io":
                                    raise OSError("synthetic")
                                self.returncode = rc
                                return stdout, stderr
                            sequence.append("process-reaped")
                            self.returncode = -signal.SIGTERM
                            return b"", b""
                    self_test = self
                    process = Process()
                    def spawn(argv, **kwargs):
                        sequence.append("spawn")
                        self.assertTrue(kwargs["start_new_session"])
                        if failure == "spawn":
                            raise OSError("synthetic")
                        return process
                    def cleanup(*args, **kwargs):
                        sequence.append("container-cleanup")
                        return cleanup_ok
                    with (
                        mock.patch.object(grant, "verify_local_image", return_value=True),
                        mock.patch.object(grant, "_write_private_override", return_value=(pathlib.Path("/tmp/owned"), pathlib.Path("/tmp/owned/compose.yml"))),
                        mock.patch.object(grant, "_remove_private_override", return_value=True) as remove,
                        mock.patch.object(grant, "cleanup_exact_container", side_effect=cleanup) as cleanup_mock,
                    ):
                        result = grant.run_db_operation(context, OPERATION, mode, "{}", GRANT,
                            b"BEGIN;\nCOMMIT;\n", popen_factory=spawn,
                            killpg_fn=lambda pid, sig: sequence.append((pid, sig)))
                    self.assertEqual(result, expected if cleanup_ok else "CLEANUP_FAILED")
                    self.assertEqual(sequence.count("spawn"), 1)
                    cleanup_mock.assert_called_once()
                    remove.assert_called_once()
                    if failure in {"timeout", "io"}:
                        self.assertLess(sequence.index("process-reaped"), sequence.index("container-cleanup"))
                        self.assertIn((4242, signal.SIGTERM), sequence)

    def test_real_shared_parser_rejects_missing_empty_or_mismatched_commit(self) -> None:
        libraries = pathlib.Path(__file__).with_name("lib")
        services = ["db-role-bootstrap", "db-migrate", "api-server", "web", "research-worker",
            "recommendation-runner", "candidate-runner", "owner-beta-runner", "owner-equity-v2-runner",
            "nt-backtest-worker-1", "nt-backtest-worker-2", "paper-scheduler"]
        manifest_text = "LAGRANGE_RELEASE_MANIFEST_V2\ncommit|" + COMMIT + "\n" + "".join(
            f"image|{service}|lagrange-station-{service}:{COMMIT}|{IMAGE_ID}|{COMMIT}\n" for service in services)
        values = {"LAGRANGE_CODE_COMMIT": COMMIT, "KIS_MARKET_STREAM_CREDENTIAL_SLOT_ID": SLOT,
            "KIS_READ_CREDENTIAL_GENERATION": "7", "KIS_MARKET_STREAM_GRANT_ID": GRANT,
            "KIS_MARKET_STREAM_CONTRACT_SHA256": "b" * 64}
        with tempfile.TemporaryDirectory() as directory:
            env = pathlib.Path(directory) / "synthetic.env"
            manifest = pathlib.Path(directory) / "synthetic.manifest"
            manifest.write_text(manifest_text)
            def read_snapshot():
                return grant._shared_parser_snapshot(dotenv_library=libraries / "dotenv.sh",
                    manifest_library=libraries / "release-image-manifest.sh", env_file=env,
                    manifest_file=manifest, expected_commit=COMMIT)
            def write_env(mapping):
                env.write_text("".join(f"{key}={value}\n" for key, value in mapping.items()))
            write_env(values)
            self.assertEqual(read_snapshot(), parser_snapshot(context_fixture()))
            for commit in (None, "", "d" * 40):
                modified = {key: value for key, value in values.items() if key != "LAGRANGE_CODE_COMMIT"}
                if commit is not None:
                    modified["LAGRANGE_CODE_COMMIT"] = commit
                write_env(modified)
                with self.assertRaises(grant.GrantFailure) as error:
                    read_snapshot()
                self.assertEqual(error.exception.code, "INSTALLED_CONTEXT_REJECTED")
            write_env(values)
            manifest.write_text(manifest_text.replace("image|web|", "image|unknown|"))
            with self.assertRaises(grant.GrantFailure):
                read_snapshot()

    def test_off_release_can_only_revoke_explicit_grant(self) -> None:
        off = (COMMIT, "", "", "", "", IMAGE_ID, COMMIT)
        self.assertFalse(grant.context_snapshot_is_valid(off, COMMIT))
        self.assertTrue(grant.context_snapshot_is_valid(off, COMMIT, require_binding=False))
        self.assertFalse(grant.context_snapshot_is_valid(off[:-1] + ("c" * 40,), COMMIT, require_binding=False))
        self.assertFalse(grant.context_snapshot_is_valid(off, "c" * 40, require_binding=False))
        args = ["--apply", "--revoke", "--expected-commit", COMMIT, "--grant-id", GRANT]
        output = io.StringIO()
        with (
            mock.patch.object(grant, "verify_installed_context", return_value=context_fixture()) as context,
            mock.patch.object(grant, "read_trusted_approval_file", side_effect=AssertionError),
            mock.patch.object(grant, "_read_trusted_sql", return_value=b"synthetic SQL"),
            mock.patch.object(grant, "run_db_operation", return_value="REVOKED") as operation,
        ):
            self.assertEqual(grant.main(args, output=output, operation_id_factory=lambda: OPERATION), "REVOKED")
        self.assertEqual(context.call_args.kwargs, {"require_binding": False})
        self.assertEqual(operation.call_args.args[2:5], ("revoke", "{}", GRANT))
        with self.assertRaises(grant.GrantFailure):
            grant.parse_cli(args[:-2])

    def test_sql_binds_values_and_limits_mutation_to_revocation(self) -> None:
        sql_path = pathlib.Path(__file__).with_name("lib") / "market-stream-grant.sql"
        sql = sql_path.read_text(encoding="utf-8")
        self.assertIn(":'grant_payload'", sql)
        self.assertIn(":'grant_mode'", sql)
        self.assertIn(":'grant_id'", sql)
        self.assertIn(" AS grant_payload \\gset", sql)
        self.assertIn(" AS grant_mode \\gset", sql)
        self.assertIn("AS grant_id \\gset", sql)
        self.assertIn("SET LOCAL lock_timeout = '5s'", sql)
        self.assertIn("SET LOCAL statement_timeout = '30s'", sql)
        self.assertIn("SET LOCAL search_path = pg_catalog", sql)
        self.assertIn("current_user <> 'migration_owner' OR session_user <> 'migration_owner'", sql)
        self.assertIn("v_mode IS NULL OR v_mode NOT IN ('install', 'revoke')", sql)
        self.assertIn("pg_advisory_xact_lock", sql)
        self.assertIn("FOR SHARE OF entitlement, owner_role", sql)
        immutable = {
            "grant_id": "id",
            "grant_revision": "grant_revision",
            "credential_slot_id": "credential_slot_id",
            "credential_generation": "credential_generation",
            "owner_user_id": "owner_user_id",
            "entitlement_id": "entitlement_id",
            "entitlement_reference": "entitlement_reference",
            "entitlement_document_sha256": "entitlement_document_sha256",
            "tr_id": "tr_id",
            "wire_version": "wire_version",
            "network_contract_sha256": "network_contract_sha256",
            "identity_list_sha256": "identity_list_sha256",
            "effective_from": "effective_from",
            "effective_until": "effective_until",
            "activation_commit": "activation_commit",
        }
        insert_block = sql.split("INSERT INTO public.owner_market_stream_grants (", 1)[1].split(") VALUES", 1)[0]
        for source_key, column in immutable.items():
            self.assertIn("v_payload ->> '" + source_key + "'", sql)
            self.assertIn("v_existing." + column, sql)
            self.assertRegex(insert_block, r"(?m)^\s*" + column + r",?$")
        updates = [
            line.strip()
            for line in sql.splitlines()
            if line.strip().startswith("UPDATE public.owner_market_stream_grants")
        ]
        self.assertEqual(len(updates), 1)
        update_block = sql.split(updates[0], 1)[1].split("WHERE id = v_grant_id", 1)[0]
        self.assertIn("SET state = 'REVOKED'", update_block)
        self.assertIn("revoked_at = v_now", update_block)
        self.assertIn("updated_at = v_now", update_block)
        self.assertNotIn("ON CONFLICT", sql)
        self.assertNotIn("DELETE FROM public.owner_market_stream_grants", sql)
        self.assertEqual(sql.count("BEGIN;"), 1)
        self.assertEqual(sql.count("COMMIT;"), 1)


def replace_stat(metadata: types.SimpleNamespace, **changes: int) -> types.SimpleNamespace:
    return types.SimpleNamespace(**{**vars(metadata), **changes})


if __name__ == "__main__":
    unittest.main()
