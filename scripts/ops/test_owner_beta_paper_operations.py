#!/usr/bin/env python3
"""Decision tests for the retained Paper transport (no Docker/DB authority)."""
from __future__ import annotations

import contextlib
import fcntl
import hashlib
import importlib.util
import inspect
import json
import os
import pathlib
import shutil
import stat
import subprocess
import sys
import tempfile
import types
import uuid
import unittest
from unittest import mock


SOURCE = pathlib.Path(__file__).with_name("lib") / "owner-beta-paper-operations.py"
SPEC = importlib.util.spec_from_file_location("owner_beta_paper_operations_tested", SOURCE)
ops = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = ops
SPEC.loader.exec_module(ops)


def canonical(value):
    return json.dumps(value, ensure_ascii=True, sort_keys=True,
                      separators=(",", ":"), allow_nan=False).encode("ascii")


def digest(data):
    return "sha256:" + hashlib.sha256(data).hexdigest()


def fixture_journal_dir_info(fd, mode=None):
    info = os.fstat(fd)
    if (not stat.S_ISDIR(info.st_mode)
            or info.st_uid not in (0, 65534, os.geteuid())
            or info.st_gid not in (0, 65534, os.getegid())
            or (mode is not None and stat.S_IMODE(info.st_mode) != mode)):
        ops.journal._fail("JOURNAL_DIRECTORY")
    ops.journal._no_acl(fd)
    return info


def fixture_journal_file_info(fd, mode=None):
    info = os.fstat(fd)
    if (not stat.S_ISREG(info.st_mode)
            or info.st_uid not in (0, 65534, os.geteuid())
            or info.st_gid not in (0, 65534, os.getegid()) or info.st_nlink != 1
            or (mode is not None and stat.S_IMODE(info.st_mode) != mode)):
        ops.journal._fail("JOURNAL_FILE")
    ops.journal._no_acl(fd)
    return info


def fixture_operations_dir(fd, mode=0o700, *, uid=0, gid=0):
    info = os.fstat(fd)
    if (not stat.S_ISDIR(info.st_mode)
            or info.st_uid not in (0, 65534, os.geteuid())
            or info.st_gid not in (0, 65534, os.getegid())
            or stat.S_IMODE(info.st_mode) != mode):
        ops._fail("PAPER_OPS_DIRECTORY")
    ops.daily.install.no_acl(fd)
    return info


def fixture_operations_file(fd, mode=0o400, *, uid=0, gid=0):
    info = os.fstat(fd)
    if (not stat.S_ISREG(info.st_mode)
            or info.st_uid not in (0, 65534, os.geteuid())
            or info.st_gid not in (0, 65534, os.getegid()) or info.st_nlink != 1
            or stat.S_IMODE(info.st_mode) != mode):
        ops._fail("PAPER_OPS_FILE")
    ops.daily.install.no_acl(fd)
    return info


@contextlib.contextmanager
def _fixture_root_directory(path):
    fd = os.open(path, ops.journal.DIR_FLAGS)
    try:
        yield fd
    finally:
        os.close(fd)


class _FenceReadbackRun:
    def __init__(self, result_bytes, result, *, cleanup_state="REMOVED"):
        self._bytes = result_bytes
        self.result = result
        self._closed = False
        self._cleanup_state = cleanup_state
        self._cid = self._child = None
        self._preserve = False
        self._imported = True

    def __enter__(self):
        return self

    def __exit__(self, *_exc):
        self._closed = True

    def readback_artifact(self):
        return self._bytes


class _FenceReadbackContext:
    def __init__(self, live, factory):
        self._mode = "custody"
        self.release_path = live.release_path
        self.commit = live.commit
        self.image = live.image
        self.manifest_sha256 = live.manifest_sha256
        self._manifest_bytes = live._manifest_bytes
        self.database_image_id = live.database_image_id
        self.network_id = live.network_id
        self.database_generation = live.database_generation
        self._invocations = set()
        self._closed = False
        self._factory = factory

    def invoke(self, verb, custody):
        if verb != "readback" or not custody._readonly:
            raise AssertionError("readback fixture received an unbound call")
        return self._factory(custody)

    def __enter__(self):
        return self

    def __exit__(self, *_exc):
        self._closed = True


class PaperOperationsDecisionTests(unittest.TestCase):
    def test_v3_context_keeps_thirteen_release_rows_and_resolves_pg_separately(self):
        commit = "a" * 40
        services = ops.daily.policy.MANIFEST_V3_SERVICES
        manifest = ("LAGRANGE_RELEASE_MANIFEST_V3\ncommit|" + commit + "\n" + "".join(
            "image|" + name + "|lagrange-station-" + name + ":" + commit +
            "|sha256:" + format(index + 1, "064x") + "|" + commit + "\n"
            for index, name in enumerate(services))).encode("ascii")
        rows = manifest.decode().splitlines()[2:]
        images = {row.split("|")[1]: row.split("|")[3] for row in rows}
        self.assertEqual(len(images), 13)
        self.assertNotIn("postgres", images)
        before = dict(images)
        context = ops.daily.Context("/fixture/releases/" + commit, commit, images, {}, [])
        with mock.patch.object(ops.daily, "read_manifest", return_value=images), \
                mock.patch.object(ops.daily, "project_binding", return_value=("fixture", "daily")), \
                mock.patch.object(ops.daily, "read_context", return_value=({}, [])):
            selected = ops._manifest_and_context(context, types.SimpleNamespace(names=["fixture"]))
        self.assertEqual(selected[3], before)
        docker = mock.Mock()
        docker.object.return_value = {"Id": "sha256:" + "f" * 64,
                                      "RepoDigests": [ops.POSTGRES_IMAGE]}
        self.assertEqual(ops._database_image_id(docker), "sha256:" + "f" * 64)
        docker.object.assert_called_once_with("image", ops.POSTGRES_IMAGE)
        self.assertEqual(context.images, before)

    def test_pg_image_reference_and_id_are_both_required(self):
        for value in ({"Id": "sha256:" + "f" * 64, "RepoDigests": []},
                      {"Id": "postgres:latest", "RepoDigests": [ops.POSTGRES_IMAGE]},
                      {"Id": "sha256:" + "f" * 64, "RepoDigests": "wrong"}):
            docker = mock.Mock()
            docker.object.return_value = value
            with self.subTest(value=value), self.assertRaises(ops.PaperOperationsError):
                ops._database_image_id(docker)

    def test_begin_stages_originals_before_database_and_selected_sources(self):
        source = inspect.getsource(ops._NativeInvocation._prepare)
        action_inputs = source.index("self.custody.stage_inputs(stage.fd)")
        database = source.index('_write_file(stage.fd, "database.json"')
        migration_secret = source.index(
            '_write_file(secret_stage.fd, "db_migration_owner_password"')
        selected_sources = source.index("context.verified.stage_native_inputs(stage.fd)")
        self.assertLess(action_inputs, database)
        self.assertLess(database, migration_secret)
        self.assertLess(migration_secret, selected_sources)

    def test_canonical_installation_id_line_matches_journal_uuid_grammar(self):
        self.assertTrue(ops._canonical_uuid_lf(
            b"00000000-0000-7000-8000-000000000001\n"))
        for value in (b"00000000-0000-7000-8000-000000000001",
                      b"00000000-0000-7000-8000-000000000001\n\n",
                      b"00000000-0000-7000-8000-00000000000A\n",
                      b"00000000-0000-0000-0000-000000000000\n"):
            with self.subTest(value=value):
                self.assertFalse(ops._canonical_uuid_lf(value))

    def test_selected_image_environment_accepts_only_known_values(self):
        commit = "a" * 40
        env = ["LANG=C.UTF-8", "PATH=" + ops.IMAGE_PATH,
               "LAGRANGE_CODE_COMMIT=" + commit]
        self.assertEqual(ops._image_environment({"Env": env}, commit),
                         {"LANG": "C.UTF-8", "PATH": ops.IMAGE_PATH,
                          "LAGRANGE_CODE_COMMIT": commit})
        for bad_env in (
                env + ["LANG=C"],
                env + ["UNKNOWN=value"],
                env + ["HOSTNAME=image-host"],
                ["PATH=/usr/bin"],
                ["PATH=" + ops.IMAGE_PATH + "::/tmp"],
                ["LANG"],
                ["LANG=C", "LANG=C.UTF-8"],
                ["LANG=C\nTZ=UTC"],
                ["LAGRANGE_CODE_COMMIT=" + "b" * 40]):
            with self.subTest(env=bad_env), self.assertRaises(ops.PaperOperationsError):
                ops._image_environment({"Env": bad_env}, commit)
        with self.assertRaises(ops.PaperOperationsError):
            ops._image_environment({"Env": ["LAGRANGE_CODE_COMMIT=" + commit]}, "b" * 40)

    def test_docker_client_environment_is_fixed_and_excludes_forwarded_host_keys(self):
        environment = ops._docker_client_environment()
        self.assertEqual(environment, {"LANG": "C", "LC_ALL": "C", "TZ": "UTC"})
        self.assertIsNone(ops._validate_docker_client_environment(environment))
        for name in ("PATH", "HOME", "HOSTNAME", "LAGRANGE_CODE_COMMIT", "TERM"):
            with self.subTest(name=name), self.assertRaises(ops.PaperOperationsError):
                ops._validate_docker_client_environment({**environment, name: "host-value"})
        with self.assertRaises(ops.PaperOperationsError):
            ops._validate_docker_client_environment({"LANG": "host-locale"})

    def test_owned_docker_calls_and_attach_share_the_private_environment(self):
        init_source = inspect.getsource(ops._PaperContext.__init__)
        run_source = inspect.getsource(ops._NativeInvocation._run)
        self.assertIn("self.docker = runtime.Docker()", init_source)
        self.assertIn("self.docker.environment = _docker_client_environment()", init_source)
        self.assertIn('"/usr/bin/docker", "--host", "unix:///var/run/docker.sock"',
                      run_source)
        self.assertIn("env=context.docker.environment", run_source)

    def test_bare_env_removes_image_and_daemon_keys_while_empty_value_does_not(self):
        commit = "c" * 40
        image_env = ops._image_environment(
            {"Env": ["PATH=" + ops.IMAGE_PATH, "LAGRANGE_CODE_COMMIT=" + commit,
                     "LANG=C.UTF-8"]}, commit)
        client_env = ops._docker_client_environment()
        daemon_env = {"PATH": "daemon-path", "HOSTNAME": "daemon-host"}
        effective = ops._effective_container_environment(
            image_env, ops.EXPECTED_RAW_ENV, daemon_env=daemon_env,
            client_env=client_env, tty=False, links=[])
        self.assertEqual(effective, ops.NATIVE_ENV)

        removed = ops._effective_container_environment(
            {"PATH": ops.IMAGE_PATH}, ["PATH"], daemon_env=daemon_env,
            client_env=client_env, tty=False, links=[])
        empty = ops._effective_container_environment(
            {"PATH": ops.IMAGE_PATH}, ["PATH="], daemon_env=daemon_env,
            client_env=client_env, tty=False, links=[])
        self.assertNotIn("PATH", removed)
        self.assertEqual(empty["PATH"], "")
        self.assertEqual(ops._native_env_args(),
                         [argument for item in ops.EXPECTED_RAW_ENV
                          for argument in ("--env", item)])

    def test_container_runtime_policy_is_closed_over_image_and_host_settings(self):
        config = {"Entrypoint": [ops.NATIVE], "Cmd": ["begin"], "User": "0:0",
                  "WorkingDir": "/", "Tty": False,
                  "Env": list(ops.EXPECTED_RAW_ENV)}
        host = {"ReadonlyRootfs": True, "Privileged": False, "CapAdd": [],
                "CapDrop": ["ALL"], "AutoRemove": False,
                "RestartPolicy": {"Name": "no"}, "LogConfig": {"Type": "none"},
                "Memory": 2_147_483_648, "NanoCpus": 1_000_000_000,
                "NetworkMode": "net-id", "Binds": [], "Tmpfs": {}, "Devices": [],
                "DeviceRequests": [], "VolumesFrom": [], "PortBindings": {},
                "PublishAllPorts": False, "PidMode": "private", "IpcMode": "private",
                "UTSMode": "private", "CgroupnsMode": "private",
                "SecurityOpt": ["no-new-privileges:true"]}
        image_env = {"PATH": ops.IMAGE_PATH,
                     "LAGRANGE_CODE_COMMIT": "d" * 40,
                     "LANG": "C.UTF-8"}
        client_env = ops._docker_client_environment()
        self.assertIsNone(ops._validate_runtime_policy(
            config, host, "begin", image_env, "net-id", client_env))
        for section, key, replacement in (
                (config, "Env", list(ops.EXPECTED_RAW_ENV) + ["DOCKER_HOST=tcp://example"]),
                (config, "Env", ["LANG=host-value", *ops.EXPECTED_RAW_ENV[1:]]),
                (config, "Env", [*ops.EXPECTED_RAW_ENV, "PATH"]),
                (config, "Tty", True),
                (config, "Links", ["db:postgres"]),
                (config, "Cmd", ["readback"]),
                (host, "Privileged", True),
                (host, "ReadonlyRootfs", False),
                (host, "CapDrop", []),
                (host, "SecurityOpt", ["no-new-privileges:true", "seccomp=unconfined"]),
                (host, "NetworkMode", "bridge"),
                (host, "Links", ["db:postgres"]),
                (host, "Memory", 0)):
            changed = dict(section)
            changed[key] = replacement
            with self.subTest(section=section is host, key=key), \
                    self.assertRaises(ops.PaperOperationsError):
                ops._validate_runtime_policy(changed if section is config else config,
                                             changed if section is host else host,
                                             "begin", image_env, "net-id", client_env)

    def test_stage_directory_name_replacement_is_detected_with_fd_open(self):
        with tempfile.TemporaryDirectory(prefix="paper-stage-name-") as directory:
            parent_fd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
            try:
                os.mkdir("output", 0o700, dir_fd=parent_fd)
                output_fd = os.open("output", os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC,
                                    dir_fd=parent_fd)
                try:
                    identity = ops._identity(os.fstat(output_fd))

                    def fixture_dir(fd, mode=0o700, *, uid=0, gid=0):
                        return os.fstat(fd)

                    with mock.patch.object(ops, "_dir", side_effect=fixture_dir):
                        ops._check_named_directory(parent_fd, "output", output_fd, identity)
                        os.rename("output", "output-original", src_dir_fd=parent_fd,
                                  dst_dir_fd=parent_fd)
                        os.mkdir("output", 0o700, dir_fd=parent_fd)
                        with self.assertRaises(ops.PaperOperationsError):
                            ops._check_named_directory(parent_fd, "output", output_fd, identity)
                finally:
                    os.close(output_fd)
            finally:
                os.close(parent_fd)

    def test_metadata_helpers_use_installed_release_acl_checker(self):
        with tempfile.TemporaryDirectory(prefix="paper-metadata-check-") as directory:
            directory_fd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
            try:
                ops._dir(directory_fd, 0o700, uid=os.geteuid(), gid=os.getegid())
                file_fd = os.open("original", os.O_WRONLY | os.O_CREAT | os.O_EXCL |
                                  os.O_CLOEXEC, 0o400, dir_fd=directory_fd)
                try:
                    os.fchmod(file_fd, 0o400)
                finally:
                    os.close(file_fd)
                file_fd = os.open("original", os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC,
                                  dir_fd=directory_fd)
                try:
                    ops._file(file_fd, 0o400, uid=os.geteuid(), gid=os.getegid())
                finally:
                    os.close(file_fd)
            finally:
                os.close(directory_fd)

    def test_native_result_order_and_existing_schema_by_verb(self):
        begin_ok = (
            b'{"schema":"owner-beta-paper-ops-result-v1","verb":"begin",'
            b'"outcome":"OK","event_id":"22222222-2222-4222-8222-222222222222",'
            b'"generation":"11111111-1111-4111-8111-111111111111",'
            b'"control_epoch":8,"artifact_sha256":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
            b'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","error_code":"NONE"}\n')
        inspect_blocked = (
            b'{"schema":"owner-beta-paper-ops-result-v1","verb":"inspect",'
            b'"outcome":"BLOCKED","event_id":null,'
            b'"generation":"11111111-1111-4111-8111-111111111111",'
            b'"control_epoch":8,"artifact_sha256":null,"error_code":"WRITER_UNKNOWN"}\n')
        readback_replay = (
            b'{"schema":"owner-beta-paper-ops-query-result-v1","verb":"readback",'
            b'"outcome":"REPLAY","event_id":"22222222-2222-4222-8222-222222222222",'
            b'"generation":"11111111-1111-4111-8111-111111111111",'
            b'"control_epoch":8,"artifact_sha256":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'
            b'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","error_code":"NONE"}\n')
        readback_unknown = (
            b'{"schema":"owner-beta-paper-ops-query-result-v1","verb":"readback",'
            b'"outcome":"COMMIT_UNKNOWN","event_id":null,'
            b'"generation":"11111111-1111-4111-8111-111111111111",'
            b'"control_epoch":8,"artifact_sha256":null,"error_code":"COMMIT_UNKNOWN"}\n')
        self.assertEqual(ops._validate_result(begin_ok, "begin", 0)["outcome"], "OK")
        self.assertEqual(ops._validate_result(inspect_blocked, "inspect", 3)["outcome"],
                         "BLOCKED")
        self.assertEqual(ops._validate_result(readback_replay, "readback", 0)["outcome"],
                         "REPLAY")
        self.assertEqual(ops._validate_result(readback_unknown, "readback", 4)["outcome"],
                         "COMMIT_UNKNOWN")
        for raw, verb, status in (
                (begin_ok.replace(ops.RESULT_SCHEMA.encode(),
                                  ops.QUERY_RESULT_SCHEMA.encode()), "begin", 0),
                (readback_replay.replace(ops.QUERY_RESULT_SCHEMA.encode(),
                                         ops.RESULT_SCHEMA.encode()), "readback", 0),
                (inspect_blocked.replace(b'"verb":"inspect"', b'"verb":"begin"'),
                 "inspect", 3)):
            with self.subTest(verb=verb), self.assertRaises(ops.PaperOperationsError):
                ops._validate_result(raw, verb, status)

    def test_native_result_decoder_rejects_duplicate_unknown_and_malformed_records(self):
        valid = (
            b'{"schema":"owner-beta-paper-ops-result-v1","verb":"begin",'
            b'"outcome":"OK","event_id":null,'
            b'"generation":"11111111-1111-4111-8111-111111111111",'
            b'"control_epoch":8,"artifact_sha256":null,"error_code":"NONE"}')
        invalid = (
            valid[:-1] + b',"schema":"owner-beta-paper-ops-result-v1"}',
            valid[:-1] + b',"extra":null}',
            valid.replace(b'{"schema":"owner-beta-paper-ops-result-v1","verb":"begin"',
                          b'{"verb":"begin","schema":"owner-beta-paper-ops-result-v1"'),
            valid.replace(b'"control_epoch":8', b'"control_epoch":true'),
            valid.replace(b'"control_epoch":8', b'"control_epoch":8.0'),
            valid.replace(b'"control_epoch":8', b'"control_epoch":NaN'),
            valid.replace(b'"outcome":"OK"', b'"outcome":[]'),
            valid + b'\n{}',
            b' ' * (ops.MAX_RESULT + 1),
            valid.replace(b'"outcome":"OK"', b'"outcome":"OK" '),
        )
        for raw in invalid:
            with self.subTest(raw=raw[:90]), self.assertRaises(ops.PaperOperationsError):
                ops._validate_result(raw, "begin", 0)
        with self.assertRaises(ops.PaperOperationsError):
            ops._validate_result(valid, "begin", 4)
        with self.assertRaises(ops.PaperOperationsError):
            ops._validate_result(valid, [], 0)

    def test_unknown_result_retains_stage_except_exact_preexisting_duplicate(self):
        duplicate = {"outcome": "COMMIT_UNKNOWN", "event_id": None,
                     "artifact_sha256": None}
        committed_unknown = {"outcome": "COMMIT_UNKNOWN",
                             "event_id": "22222222-2222-4222-8222-222222222222",
                             "artifact_sha256": "sha256:" + "a" * 64}
        self.assertFalse(ops._preserve_unknown_result(duplicate, True))
        self.assertTrue(ops._preserve_unknown_result(duplicate, False))
        self.assertTrue(ops._preserve_unknown_result(committed_unknown, True))

    def test_restore_begin_is_rejected_before_invocation_creation(self):
        context = object.__new__(ops._PaperContext)
        context._mode = "live"
        context._invocations = set()
        action = object.__new__(ops.journal.ActionCustody)
        action._action = {"verb": "begin"}
        action._operation = types.SimpleNamespace(_intent={"purpose": "RESTORE"})
        with mock.patch.object(ops._PaperContext, "revalidate"), \
                mock.patch.object(ops.journal.ActionCustody, "revalidate"):
            with self.assertRaises(ops.PaperOperationsError) as caught:
                context.invoke("begin", action)
        self.assertEqual(caught.exception.code, "PAPER_OPS_ACTION")
        self.assertEqual(context._invocations, set())

    def test_container_binding_rejects_wrong_id_nonce_created_restart(self):
        value = {
            "Id": "a" * 64, "Image": "sha256:" + "b" * 64,
            "Created": "2026-11-06T00:00:00Z", "RestartCount": 0,
            "Config": {"Labels": {
                ops.CONTAINER_LABEL: "n" * 32,
                "org.opencontainers.image.revision": "c" * 40,
            }},
        }
        args = (value, "a" * 64, "n" * 32, value["Created"], value["Image"], "c" * 40)
        self.assertIsNone(ops._validate_container_identity(*args))
        for index, replacement in ((1, "d" * 64), (2, "x" * 32),
                                   (3, "changed"), (4, "sha256:" + "e" * 64),
                                   (5, "d" * 40), (0, {**value, "RestartCount": 1})):
            bad = list(args)
            bad[index] = replacement
            with self.subTest(index=index), self.assertRaises(ops.PaperOperationsError):
                ops._validate_container_identity(*bad)

    def test_only_output_mount_is_writable_and_role_aliases_are_fixed(self):
        mounts = [
            ("/fixture/request-root", ops.PRIVATE, True),
            ("/fixture/operation", ops.OPERATION_ALIAS, True),
            ("/fixture/paper-state", ops.PAPER_STATE_ALIAS, True),
            ("/fixture/secrets", "/run/secrets", True),
            ("/fixture/output", ops.OUTPUT_ALIAS, False),
        ]
        actual = [{"Type": "bind", "Source": os.path.realpath(source), "Destination": target,
                   "RW": not readonly} for source, target, readonly in mounts]
        self.assertIsNone(ops._validate_mount_records(actual, mounts))
        swapped = [dict(item) for item in actual]
        swapped[2]["RW"] = True
        with self.assertRaises(ops.PaperOperationsError):
            ops._validate_mount_records(swapped, mounts)
        with self.assertRaises(ops.PaperOperationsError):
            ops._validate_mount_records(actual[:-1], mounts)
        readback = [
            ("/fixture/readback-request", ops.PRIVATE, True),
            ("/fixture/secrets", "/run/secrets", True),
            ("/fixture/original-output", ops.OUTPUT_ALIAS, False),
        ]
        readback_actual = [{"Type": "bind", "Source": os.path.realpath(source),
                            "Destination": target, "RW": not readonly}
                           for source, target, readonly in readback]
        self.assertIsNone(ops._validate_mount_records(readback_actual, readback))
        destinations = {mount["Destination"] for mount in readback_actual}
        self.assertNotIn(ops.OPERATION_ALIAS, destinations)
        self.assertNotIn(ops.PAPER_STATE_ALIAS, destinations)

    def test_generation_null_page_can_carry_the_only_history_marker(self):
        page = {
            "schema": ops.PAGE_SCHEMA, "action": "INSPECT_OPERATION", "outcome": "OK",
            "control_epoch": 4, "account_id": "33333333-3333-4333-8333-333333333333",
            "data": {"generation": None, "state": "OFF", "control_epoch": 4,
                     "history_present": True, "operation_event_id": None,
                     "reconciliation": None, "targets": [], "next_target": None},
        }
        self.assertIsNone(ops._validate_inspect_page(page))
        self.assertIs(ops._history_presence(True, False, False), ops._HISTORY_PRESENT)
        self.assertIs(ops._history_presence(False, False, True), ops._HISTORY_PRESENT)

    def test_empty_unconfigured_generation_null_page_is_not_configured(self):
        page = {
            "schema": ops.PAGE_SCHEMA, "action": "INSPECT_OPERATION", "outcome": "OK",
            "control_epoch": None, "account_id": None,
            "data": {"generation": None, "state": None, "control_epoch": None,
                     "history_present": False, "operation_event_id": None,
                     "reconciliation": None, "targets": [], "next_target": None},
        }
        self.assertIsNone(ops._validate_inspect_page(page))
        page["data"]["history_present"] = True
        with self.assertRaises(ops.PaperOperationsError):
            ops._validate_inspect_page(page)

    def test_generation_null_inventory_reads_exact_index_and_page_bytes(self):
        page = {
            "schema": ops.PAGE_SCHEMA, "action": "INSPECT_OPERATION", "outcome": "OK",
            "control_epoch": 4, "account_id": "33333333-3333-4333-8333-333333333333",
            "data": {"generation": None, "state": "OFF", "control_epoch": 4,
                     "history_present": True, "operation_event_id": None,
                     "reconciliation": None, "targets": [], "next_target": None},
        }
        page_bytes = canonical(page)
        page_hash = digest(page_bytes)
        hashes = [page_hash]
        index = {"schema": ops.INDEX_SCHEMA, "generation": None, "state": "OFF",
                 "control_epoch": 4, "operation_event_id": None,
                 "root_sha256": digest(canonical(hashes)), "pages": hashes}
        index_bytes = canonical(index)
        index_hash = digest(index_bytes)
        with tempfile.TemporaryDirectory(prefix="paper-inventory-") as directory:
            root_fd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
            try:
                for name, data in ((index_hash[7:] + ".json", index_bytes),
                                   (page_hash[7:] + ".json", page_bytes)):
                    fd = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o400,
                                 dir_fd=root_fd)
                    try:
                        os.write(fd, data)
                    finally:
                        os.close(fd)
                invocation = mock.Mock()
                invocation._result = {"outcome": "OK", "error_code": "NONE",
                                      "generation": None, "event_id": None,
                                      "control_epoch": 4,
                                      "artifact_sha256": index_hash}
                invocation._probe_output_fd = root_fd
                with mock.patch.object(ops, "_open_file", side_effect=self._read_fixture_file):
                    stored_index, stored_pages, history, configured = ops._read_inventory(invocation)
                self.assertEqual(stored_index, index_bytes)
                self.assertEqual(stored_pages, ((page_hash, page_bytes),))
                self.assertTrue(history)
                self.assertTrue(configured)
            finally:
                os.close(root_fd)

    @staticmethod
    def _read_fixture_file(parent, name, mode, *, maximum, uid=0, gid=0):
        try:
            fd = os.open(name, os.O_RDONLY | os.O_NONBLOCK | os.O_NOFOLLOW |
                         os.O_CLOEXEC, dir_fd=parent)
        except OSError:
            raise ops.PaperOperationsError("PAPER_OPS_FILE") from None
        data = os.read(fd, maximum + 1)
        info = os.fstat(fd)
        if len(data) > maximum or not stat.S_ISREG(info.st_mode):
            os.close(fd)
            raise ops.PaperOperationsError("PAPER_OPS_SIZE")
        return fd, data, ops._identity(info)

    def test_missing_content_page_fails_closed(self):
        page_hash = "sha256:" + "a" * 64
        index = {"schema": ops.INDEX_SCHEMA, "generation": None, "state": None,
                 "control_epoch": None, "operation_event_id": None,
                 "root_sha256": digest(canonical([page_hash])), "pages": [page_hash]}
        index_bytes = canonical(index)
        index_hash = digest(index_bytes)
        with tempfile.TemporaryDirectory(prefix="paper-missing-page-") as directory:
            root_fd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
            try:
                name = index_hash[7:] + ".json"
                fd = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o400, dir_fd=root_fd)
                os.write(fd, index_bytes)
                os.close(fd)
                invocation = mock.Mock()
                invocation._result = {"outcome": "OK", "error_code": "NONE",
                                      "generation": None, "event_id": None,
                                      "control_epoch": None,
                                      "artifact_sha256": index_hash}
                invocation._probe_output_fd = root_fd
                with mock.patch.object(ops, "_open_file", side_effect=self._read_fixture_file):
                    with self.assertRaises(ops.PaperOperationsError):
                        ops._read_inventory(invocation)
            finally:
                os.close(root_fd)

    def test_history_snapshot_retains_original_page_inode_and_name(self):
        page = {
            "schema": ops.PAGE_SCHEMA, "action": "INSPECT_OPERATION", "outcome": "OK",
            "control_epoch": 4, "account_id": "33333333-3333-4333-8333-333333333333",
            "data": {"generation": None, "state": "OFF", "control_epoch": 4,
                     "history_present": False, "operation_event_id": None,
                     "reconciliation": None, "targets": [], "next_target": None},
        }
        page_bytes = canonical(page)
        page_hash = digest(page_bytes)
        hashes = [page_hash]
        index = {"schema": ops.INDEX_SCHEMA, "generation": None, "state": "OFF",
                 "control_epoch": 4, "operation_event_id": None,
                 "root_sha256": digest(canonical(hashes)), "pages": hashes}
        index_bytes = canonical(index)
        index_hash = digest(index_bytes)
        with tempfile.TemporaryDirectory(prefix="paper-retained-inventory-") as directory:
            root_fd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
            try:
                for name, data in ((index_hash[7:] + ".json", index_bytes),
                                   (page_hash[7:] + ".json", page_bytes)):
                    fd = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o400,
                                 dir_fd=root_fd)
                    os.write(fd, data)
                    os.close(fd)
                invocation = mock.Mock()
                invocation._result = {"outcome": "OK", "error_code": "NONE",
                                      "generation": None, "event_id": None,
                                      "control_epoch": 4, "artifact_sha256": index_hash}
                invocation._probe_output_fd = root_fd
                with mock.patch.object(ops, "_open_file", side_effect=self._read_fixture_file):
                    snapshot, history, configured = ops._read_inventory(invocation, retain=True)
                self.assertFalse(history)
                self.assertTrue(configured)

                def fixture_dir(fd, mode=0o700, *, uid=0, gid=0):
                    return os.fstat(fd)

                def fixture_file(fd, mode=0o400, *, uid=0, gid=0):
                    return os.fstat(fd)

                with mock.patch.object(ops, "_dir", side_effect=fixture_dir), \
                        mock.patch.object(ops, "_file", side_effect=fixture_file):
                    snapshot.revalidate(root_fd)
                    page_name = page_hash[7:] + ".json"
                    replacement = os.open("replacement", os.O_WRONLY | os.O_CREAT | os.O_EXCL,
                                          0o400, dir_fd=root_fd)
                    os.write(replacement, page_bytes)
                    os.close(replacement)
                    os.replace("replacement", page_name, src_dir_fd=root_fd,
                               dst_dir_fd=root_fd)
                    with self.assertRaises(ops.PaperOperationsError):
                        snapshot.revalidate(root_fd)
                snapshot.close()
            finally:
                os.close(root_fd)

    def test_local_floor_and_database_epoch_disagreement_is_unknown(self):
        self.assertIsNone(ops._validate_floor_epoch(7, 7, True))
        for floor, epoch, configured in ((8, 7, True), (None, 7, True),
                                         (7, None, False), (1, None, False)):
            with self.subTest(floor=floor, epoch=epoch), \
                    self.assertRaises(ops.PaperOperationsError):
                ops._validate_floor_epoch(floor, epoch, configured)

    def test_begin_reuses_the_same_named_input_stage_for_one_action_custody(self):
        context = object.__new__(ops._PaperContext)
        context._mode = "live"
        context._input_stages = {}
        action = object.__new__(ops.journal.ActionCustody)
        retained_stage = mock.Mock()
        with mock.patch.object(ops, "_Stage", return_value=retained_stage):
            first, first_use = context._input_stage(action)
            second, second_use = context._input_stage(action)
        self.assertIs(first, retained_stage)
        self.assertIs(second, retained_stage)
        self.assertTrue(first_use)
        self.assertFalse(second_use)
        retained_stage.revalidate.assert_called_once_with()

    def test_history_journal_selector_never_bootstraps_missing_state(self):
        context = mock.Mock(install_root="/fixture/install", lock_fd=73)
        selected = object()
        with mock.patch.object(ops.journal, "open_journal", return_value=selected) as opened:
            self.assertIs(ops._open_existing_journal(context), selected)
        opened.assert_called_once_with("/fixture/install", 73, bootstrap=False)

    def test_attach_client_is_reaped_without_treating_container_state_as_known(self):
        child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(10)"],
                                 stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                 stderr=subprocess.DEVNULL)
        try:
            ops._NativeInvocation._reap_child(child)
            self.assertIsNotNone(child.returncode)
            self.assertIn(child.returncode, (-15, -9))
        finally:
            if child.poll() is None:
                child.kill()
                child.wait(timeout=2)
            if child.stdout is not None:
                child.stdout.close()


class PaperOperationsJournalBoundaryTests(unittest.TestCase):
    def setUp(self):
        self.base = pathlib.Path(tempfile.mkdtemp(prefix="paper-operations-fixture-", dir="/tmp"))
        self.install = self.base / "install"
        self.install.mkdir(mode=0o700)
        self.lock_path = self.install / ops.journal.LOCK_NAME
        self.lock_path.touch(mode=0o600)
        os.chmod(self.lock_path, 0o600)
        self.lock_fd = os.open(self.lock_path, os.O_RDWR | os.O_CLOEXEC)
        fcntl.flock(self.lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        self.patches = [
            mock.patch.object(ops.journal, "_require_root", lambda: None),
            mock.patch.object(ops.journal, "_dir_info", fixture_journal_dir_info),
            mock.patch.object(ops.journal, "_file_info", fixture_journal_file_info),
        ]
        for patch in self.patches:
            patch.start()
        self.addCleanup(self._cleanup_fixture)

    def _cleanup_fixture(self):
        for patch in reversed(self.patches):
            patch.stop()
        try:
            fcntl.flock(self.lock_fd, fcntl.LOCK_UN)
            os.close(self.lock_fd)
        except OSError:
            pass
        shutil.rmtree(self.base, ignore_errors=True)

    @contextlib.contextmanager
    def begin_action(self, purpose):
        manager = ops.journal.open_journal(str(self.install), self.lock_fd, bootstrap=True)
        store = manager.__enter__()
        operation = custody = None
        try:
            generation = str(uuid.uuid4())
            account = str(uuid.uuid4())
            expected_epoch = 3
            intent = {
                "schema": "owner-beta-paper-operation-intent-v1",
                "installation_id": store.installation_id,
                "generation": generation,
                "account_id": account,
                "predecessor_generation": None,
                "request_key": generation + ":begin",
                "purpose": purpose,
                "scope": "PAPER_STOP" if purpose == "STOP" else "COMBINED_BACKUP",
                "expected_epoch": expected_epoch,
                "local_epoch_floor": expected_epoch,
                "release_manifest_sha256": "sha256:" + "a" * 64,
                "artifact_sha256": None,
                "resume_target_id": None,
            }
            operation = store.bind_caller("fixture:" + generation, canonical(intent))
            stage = {
                "schema": "owner-beta-paper-operation-stage-v1",
                "installation_id": store.installation_id,
                "generation": generation,
                "account_id": account,
                "sequence": 0,
                "previous_sha256": None,
                "intent_sha256": operation.intent_sha256,
                "stage": "INTENT",
                "boot_id": "44444444-4444-4444-8444-444444444444",
                "observed": {"realtime_ns": 1, "monotonic_ns": 2, "boottime_ns": 3},
                "control_epoch": expected_epoch,
                "artifact_sha256": None,
                "event_id": None,
                "error_code": "NONE",
            }
            stage_bytes = canonical(stage)
            stage_sha256 = operation.append(stage_bytes)
            action = {
                "schema": "owner-beta-paper-action-binding-v1",
                "installation_id": store.installation_id,
                "generation": generation,
                "root_intent_sha256": operation.intent_sha256,
                "sequence": 0,
                "previous_binding_sha256": None,
                "verb": "begin",
                "request_key": generation + ":begin",
                "expected_epoch": expected_epoch,
                "artifact_sha256": None,
                "stage_sha256": stage_sha256,
            }
            custody = operation.prepare_action(canonical(action))
            yield operation, custody
        finally:
            if custody is not None:
                custody.close()
            if operation is not None:
                operation.close()
            manager.__exit__(None, None, None)

    def _local_markers_context(self):
        release_path = self.base / "release"
        stage_path = release_path / "deploy" / "owner-beta-paper"
        stage_path.mkdir(mode=0o750, parents=True)
        os.chmod(stage_path, 0o750)
        info = os.stat(stage_path)
        verified = types.SimpleNamespace(
            _identities=[(info.st_dev, info.st_ino, info.st_uid, info.st_gid, info.st_mode)])
        context = types.SimpleNamespace(
            install_root=str(self.install), lock_fd=self.lock_fd,
            release_path=str(release_path), verified=verified)
        return context, str(stage_path)

    def _capture_local_markers(self, context, stage_path):
        patches = (
            mock.patch.object(ops, "_dir", fixture_operations_dir),
            mock.patch.object(ops, "_file", fixture_operations_file),
            mock.patch.object(ops.daily.install, "root_directory",
                              side_effect=_fixture_root_directory),
        )
        with patches[0], patches[1], patches[2]:
            markers = ops._LocalMarkers(context)
            try:
                real_stat = os.stat

                def fixture_stat(path, *args, **kwargs):
                    info = real_stat(path, *args, **kwargs)
                    if markers.ops_fd is not None and kwargs.get("dir_fd") == markers.ops_fd:
                        fields = {name: getattr(info, name) for name in (
                            "st_dev", "st_ino", "st_mode", "st_nlink", "st_size",
                            "st_atime", "st_mtime", "st_ctime")}
                        fields.update(st_uid=0, st_gid=0)
                        return types.SimpleNamespace(**fields)
                    return info

                with mock.patch.object(ops.os, "stat", side_effect=fixture_stat):
                    markers.capture()
                    markers.revalidate()
                self.assertEqual(os.path.realpath(stage_path),
                                 os.path.realpath("/proc/self/fd/%d" % markers.release_stage_fd))
                return markers.present
            finally:
                markers.close()

    def test_begin_invokes_with_real_checked_stop_and_backup_intent_custody(self):
        for purpose in ("STOP", "BACKUP"):
            with self.subTest(purpose=purpose), self.begin_action(purpose) as (operation, custody):
                context = object.__new__(ops._PaperContext)
                context._mode = "live"
                context._invocations = set()
                sentinel = object()
                with mock.patch.object(ops._PaperContext, "revalidate"), \
                        mock.patch.object(ops, "_NativeInvocation", return_value=sentinel) as invoke:
                    returned = context.invoke("begin", custody)
                self.assertIs(returned, sentinel)
                self.assertIn(sentinel, context._invocations)
                invoke.assert_called_once_with(ops._INVOCATION_TOKEN, context, "begin", custody)
                self.assertIsNotNone(operation._last)
                self.assertEqual(operation._last[2], "INTENT")

    def test_begin_rejects_unbound_or_changed_action_fields(self):
        mutations = (
            ("missing-stage-digest", lambda action, root: action.__setitem__("stage_sha256", None)),
            ("altered-stage-digest", lambda action, root: action.__setitem__("stage_sha256", "sha256:" + "f" * 64)),
            ("non-null-artifact", lambda action, root: action.__setitem__("artifact_sha256", "sha256:" + "e" * 64)),
            ("wrong-sequence", lambda action, root: action.__setitem__("sequence", 1)),
            ("wrong-key", lambda action, root: action.__setitem__("request_key", "wrong:begin")),
            ("wrong-scope", lambda action, root: root.__setitem__("scope", "COMBINED_BACKUP")),
            ("wrong-epoch", lambda action, root: action.__setitem__("expected_epoch", 4)),
        )
        for name, mutate in mutations:
            with self.subTest(case=name), self.begin_action("STOP") as (operation, custody):
                original_action = dict(custody._action)
                original_root = dict(operation._intent)
                mutate(custody._action, operation._intent)
                context = object.__new__(ops._PaperContext)
                context._mode = "live"
                context._invocations = set()
                try:
                    with mock.patch.object(ops._PaperContext, "revalidate"), \
                            mock.patch.object(ops, "_NativeInvocation") as invoke:
                        with self.assertRaises((ops.PaperOperationsError,
                                                ops.journal.JournalError, OSError)):
                            context.invoke("begin", custody)
                    self.assertEqual(context._invocations, set())
                    invoke.assert_not_called()
                finally:
                    custody._action = original_action
                    operation._intent = original_root

    def test_local_marker_capture_reads_populated_journal_without_bootstrap(self):
        with self.begin_action("STOP"):
            pass
        context, stage_path = self._local_markers_context()
        with mock.patch.object(ops.journal, "open_journal",
                               wraps=ops.journal.open_journal) as opened:
            present = self._capture_local_markers(context, stage_path)
        self.assertTrue(present)
        opened.assert_called_once_with(str(self.install), self.lock_fd, bootstrap=False)
        self.assertTrue((self.install / ops.PAPER_DIR).is_dir())

    def test_local_marker_capture_keeps_absence_without_creating_paper_journal(self):
        state = self.install / ops.PAPER_DIR
        self.assertFalse(state.exists())
        context, stage_path = self._local_markers_context()
        with mock.patch.object(ops.journal, "open_journal",
                               wraps=ops.journal.open_journal) as opened:
            present = self._capture_local_markers(context, stage_path)
        self.assertFalse(present)
        opened.assert_not_called()
        self.assertFalse(state.exists())

    def test_local_marker_capture_reports_malformed_original_state_as_unknown(self):
        with self.begin_action("STOP"):
            pass
        installation_id = self.install / ops.PAPER_DIR / "installation-id"
        os.chmod(installation_id, 0o600)
        installation_id.write_bytes(b"not-a-canonical-id\n")
        context, _stage_path = self._local_markers_context()
        patches = (
            mock.patch.object(ops, "_dir", fixture_operations_dir),
            mock.patch.object(ops, "_file", fixture_operations_file),
        )
        with patches[0], patches[1]:
            markers = ops._LocalMarkers(context)
            try:
                with self.assertRaises(ops.PaperOperationsError) as caught:
                    markers.capture()
                self.assertEqual(caught.exception.code, "PAPER_OPS_LOCAL_UNKNOWN")
            finally:
                markers.close()

    def _fenced_fixture(self, operation, action, *, readback_mutator=None,
                        response_mutator=None, cleanup_state="REMOVED"):
        root = operation._intent
        operation._journal.advance_floor(root["local_epoch_floor"])
        fence = max(root["expected_epoch"], root["local_epoch_floor"]) + 1
        event_id = str(uuid.uuid4())
        result_bytes = canonical({
            "schema": ops.PAGE_SCHEMA,
            "action": "BEGIN_OPERATION",
            "outcome": "OK",
            "control_epoch": fence,
            "account_id": root["account_id"],
            "data": {
                "generation": operation.generation,
                "operation_event_id": event_id,
                "intent_sha256": operation.intent_sha256,
                "fenced_epoch": fence,
            },
        })
        request_bytes = canonical({
            "schema": "owner-beta-paper-exact-request-v1",
            "kind": "BEGIN",
            "account_id": root["account_id"],
            "args": {
                "expected_epoch": root["expected_epoch"],
                "intent_canonical": operation._intent_bytes.decode("ascii"),
                "request_key": root["generation"] + ":begin",
            },
        })
        dispatch_value = {
            "schema": "owner-beta-paper-native-dispatch-v2",
            "root_intent_canonical": operation._intent_bytes.decode("ascii"),
            "root_intent_sha256": operation.intent_sha256,
            "action_canonical": action._binding_bytes.decode("ascii"),
            "action_sha256": digest(action._binding_bytes),
            "artifact_canonical": None,
            "artifact_sha256": None,
            "sql_request_canonical": request_bytes.decode("ascii"),
            "sql_request_sha256": digest(request_bytes),
        }
        dispatch_bytes = canonical(dispatch_value)
        output_fd = action.borrow_output_fd()
        for data in (operation._intent_bytes, action._binding_bytes, result_bytes,
                     request_bytes, dispatch_bytes):
            ops.journal._immutable(output_fd, digest(data)[7:] + ".json", data, 0o400)
        ops.journal._immutable(output_fd, "dispatch.json", dispatch_bytes, 0o400)
        imported = action.import_dispatch()
        imported.close()

        live = object.__new__(ops._PaperContext)
        live._mode = "live"
        live.context = types.SimpleNamespace()
        live.release_path = "/fixture/release"
        live.commit = "a" * 40
        live.image = "sha256:" + "b" * 64
        live.manifest_sha256 = root["release_manifest_sha256"]
        live._manifest_bytes = b"fixture-manifest"
        live.verified = types.SimpleNamespace(
            manifest_sha256=live.manifest_sha256, revalidate=lambda: None)
        live.lock_fd = self.lock_fd
        live.lock_identity = operation._journal._lock_id
        live.network_id = "fixture-network"
        live.database_generation = "fixture-database-generation"
        live.database_image_id = "sha256:" + "c" * 64
        live._invocations = set()
        live._inventories = set()
        stage_marker = object()
        live._input_stages = {action: {"stage": stage_marker, "imports": [imported]}}
        live._retained_stages = set()

        invocation = object.__new__(ops._NativeInvocation)
        invocation._token = ops._INVOCATION_TOKEN
        invocation._context = live
        invocation.verb = "begin"
        invocation.custody = action
        invocation._entered = True
        invocation._closed = True
        invocation._result = {
            "schema": ops.RESULT_SCHEMA, "verb": "begin", "outcome": "OK",
            "event_id": event_id, "generation": operation.generation,
            "control_epoch": fence, "artifact_sha256": digest(result_bytes),
            "error_code": "NONE",
        }
        invocation._result_bytes = result_bytes
        invocation._result_name = digest(result_bytes)[7:] + ".json"
        invocation._result_parent = output_fd
        invocation._result_identity = ops._identity(os.stat(
            invocation._result_name, dir_fd=output_fd, follow_symlinks=False))
        invocation._imported = True
        invocation._cid = invocation._child = None
        invocation._exit = 0
        invocation._preserve = False
        invocation._cleanup_state = "RETAINED_FOR_ACTION"
        invocation._persistent_stage = stage_marker

        def make_readback(custody):
            readback_value = {
                "schema": "owner-beta-paper-exact-readback-v1",
                "kind": "BEGIN",
                "request_sha256": digest(custody._sql),
                "status": "COMMITTED",
                "account_id": root["account_id"],
                "current_epoch": fence,
                "current_state": "DRAINING",
                "event_id": event_id,
                "event_epoch": fence,
                "result_canonical": result_bytes.decode("ascii"),
                "result_sha256": digest(result_bytes),
                "reset": None,
            }
            if readback_mutator is not None:
                readback_mutator(readback_value)
            response_bytes = canonical(readback_value)
            response = {
                "schema": ops.QUERY_RESULT_SCHEMA, "verb": "readback",
                "outcome": "REPLAY", "event_id": event_id,
                "generation": operation.generation, "control_epoch": fence,
                "artifact_sha256": digest(response_bytes), "error_code": "NONE",
            }
            if response_mutator is not None:
                response_mutator(response)
            return _FenceReadbackRun(response_bytes, response,
                                     cleanup_state=cleanup_state)

        readback_context = _FenceReadbackContext(live, make_readback)
        return live, invocation, readback_context, result_bytes

    def test_settled_begin_continuation_can_seal_fenced_after_readback(self):
        self._assert_fence_positive("STOP")

    def test_backup_begin_continuation_can_seal_fenced_after_readback(self):
        self._assert_fence_positive("BACKUP")

    def test_begin_fence_epoch_uses_maximum_and_checks_overflow(self):
        self.assertEqual(ops._begin_fence_epoch({
            "expected_epoch": 3, "local_epoch_floor": 8}), 9)
        with self.assertRaises(ops.PaperOperationsError):
            ops._begin_fence_epoch({"expected_epoch": True, "local_epoch_floor": 8})
        with self.assertRaises(ops.PaperOperationsError):
            ops._begin_fence_epoch({
                "expected_epoch": ops.journal.MAX_I64,
                "local_epoch_floor": ops.journal.MAX_I64,
            })

    def test_begin_fence_repairs_floor_after_same_call_append(self):
        with self.begin_action("STOP") as (operation, action):
            live, invocation, readback_context, _result_bytes = self._fenced_fixture(
                operation, action)

            @contextlib.contextmanager
            def open_fake_readback(*_args):
                selected = _FenceReadbackContext(live, readback_context._factory)
                try:
                    yield selected
                finally:
                    selected._closed = True

            patches = (
                mock.patch.object(ops._PaperContext, "revalidate"),
                mock.patch.object(ops, "_current_boot_id",
                                  return_value=operation._last[3]["boot_id"]),
                mock.patch.object(ops, "_file", fixture_operations_file),
                mock.patch.object(ops, "open_custody_context",
                                  side_effect=open_fake_readback),
            )
            with contextlib.ExitStack() as stack:
                for patcher in patches:
                    stack.enter_context(patcher)
                with mock.patch.object(
                        type(operation._journal), "advance_floor", autospec=True,
                        side_effect=ops.journal.JournalError("JOURNAL_FLOOR")):
                    with self.assertRaises(ops.journal.JournalError):
                        live.fence_begin(invocation)
                stage_before = ops.journal._read_named(
                    operation._fd, "00000001.json", 0o400, ops.journal.MAX_SMALL)
                self.assertEqual(operation._last[2], "FENCED")
                self.assertEqual(operation._journal._read_floor(), 3)
                with mock.patch.object(ops.journal.Operation, "append", autospec=True,
                                       side_effect=AssertionError("floor repair must not append")):
                    self.assertEqual(live.fence_begin(invocation), digest(stage_before))
                self.assertEqual(ops.journal._read_named(
                    operation._fd, "00000001.json", 0o400, ops.journal.MAX_SMALL),
                                 stage_before)
                self.assertEqual(operation._journal._read_floor(), 4)

    def test_begin_fence_rejects_noncommitted_readback_and_cleanup(self):
        cases = (
            ("absent", lambda value: value.__setitem__("status", "ABSENT"), None, "REMOVED"),
            ("unproven", lambda value: value.__setitem__("status", "UNPROVEN"), None, "REMOVED"),
            ("stale-epoch", lambda value: value.__setitem__("current_epoch", 6), None, "REMOVED"),
            ("stale-state", lambda value: value.__setitem__("current_state", "OFF"), None, "REMOVED"),
            ("foreign-account", lambda value: value.__setitem__(
                "account_id", "55555555-5555-4555-8555-555555555555"), None, "REMOVED"),
            ("wrong-event", lambda value: value.__setitem__(
                "event_id", "55555555-5555-4555-8555-555555555555"), None, "REMOVED"),
            ("request-substitution", lambda value: value.__setitem__(
                "request_sha256", digest(b"foreign-request")), None, "REMOVED"),
            ("rehashed-result-substitution", lambda value: (
                value.__setitem__("result_canonical", "{}"),
                value.__setitem__("result_sha256", digest(b"{}"))), None, "REMOVED"),
            ("outer-generation-substitution", None,
             lambda value: value.__setitem__("generation", str(uuid.uuid4())), "REMOVED"),
            ("outer-event-substitution", None,
             lambda value: value.__setitem__("event_id", str(uuid.uuid4())), "REMOVED"),
            ("outer-epoch-substitution", None,
             lambda value: value.__setitem__("control_epoch", 5), "REMOVED"),
            ("cleanup-unknown", None, None, "UNKNOWN"),
        )
        for name, mutate, mutate_response, cleanup_state in cases:
            with self.subTest(case=name), self.begin_action("STOP") as (operation, action):
                live, invocation, readback_context, _result_bytes = self._fenced_fixture(
                    operation, action, readback_mutator=mutate,
                    response_mutator=mutate_response,
                    cleanup_state=cleanup_state)

                @contextlib.contextmanager
                def open_fake_readback(*_args):
                    selected = _FenceReadbackContext(live, readback_context._factory)
                    try:
                        yield selected
                    finally:
                        selected._closed = True

                with mock.patch.object(ops._PaperContext, "revalidate"), \
                        mock.patch.object(ops, "_current_boot_id",
                                          return_value=operation._last[3]["boot_id"]), \
                        mock.patch.object(ops, "_file", fixture_operations_file), \
                        mock.patch.object(ops, "open_custody_context",
                                          side_effect=open_fake_readback):
                    with self.assertRaises(ops.PaperOperationsError):
                        live.fence_begin(invocation)
                self.assertEqual(operation._last[2], "INTENT")
                self.assertEqual(operation._journal._read_floor(), 3)
                with self.assertRaises(FileNotFoundError):
                    os.stat("00000001.json", dir_fd=operation._fd,
                            follow_symlinks=False)

    def test_begin_fence_detects_original_result_inode_replacement(self):
        with self.begin_action("STOP") as (operation, action):
            live, invocation, readback_context, result_bytes = self._fenced_fixture(
                operation, action)
            normal_factory = readback_context._factory

            def replace_result_name(custody):
                run = normal_factory(custody)
                output_fd = action.borrow_output_fd()
                name = digest(result_bytes)[7:] + ".json"
                os.unlink(name, dir_fd=output_fd)
                ops.journal._immutable(output_fd, name, result_bytes, 0o400)
                return run

            @contextlib.contextmanager
            def open_fake_readback(*_args):
                selected = _FenceReadbackContext(live, replace_result_name)
                try:
                    yield selected
                finally:
                    selected._closed = True

            with mock.patch.object(ops._PaperContext, "revalidate"), \
                    mock.patch.object(ops, "_current_boot_id",
                                      return_value=operation._last[3]["boot_id"]), \
                    mock.patch.object(ops, "_file", fixture_operations_file), \
                    mock.patch.object(ops, "open_custody_context",
                                      side_effect=open_fake_readback):
                with self.assertRaises(ops.PaperOperationsError):
                    live.fence_begin(invocation)
            self.assertEqual(operation._last[2], "INTENT")
            self.assertEqual(operation._journal._read_floor(), 3)

    def test_begin_fence_preserves_stage_if_postappend_identity_recheck_fails(self):
        with self.begin_action("STOP") as (operation, action):
            live, invocation, readback_context, _result_bytes = self._fenced_fixture(
                operation, action)

            @contextlib.contextmanager
            def open_fake_readback(*_args):
                selected = _FenceReadbackContext(live, readback_context._factory)
                try:
                    yield selected
                finally:
                    selected._closed = True

            original_append = ops.journal.Operation.append

            def append_then_replace(operation_value, stage_bytes):
                result = original_append(operation_value, stage_bytes)
                name = "00000001.json"
                os.unlink(name, dir_fd=operation_value._fd)
                ops.journal._immutable(operation_value._fd, name, stage_bytes, 0o400)
                return result

            with mock.patch.object(ops._PaperContext, "revalidate"), \
                    mock.patch.object(ops, "_current_boot_id",
                                      return_value=operation._last[3]["boot_id"]), \
                    mock.patch.object(ops, "_file", fixture_operations_file), \
                    mock.patch.object(ops, "open_custody_context",
                                      side_effect=open_fake_readback), \
                    mock.patch.object(ops.journal.Operation, "append",
                                      append_then_replace):
                with self.assertRaises((ops.journal.JournalError,
                                        ops.PaperOperationsError)):
                    live.fence_begin(invocation)
            self.assertGreaterEqual(operation._journal._read_floor(), 3)
            with os.scandir(operation._fd) as entries:
                self.assertIn("00000001.json", {entry.name for entry in entries})

    def test_begin_fence_rejects_unsettled_context_and_floor_drift(self):
        cases = ("open-invocation", "live-container", "unimported", "wrong-cleanup",
                 "wrong-context", "wrong-token", "wrong-action-type",
                 "foreign-action-custody", "closed-action",
                 "wrong-lock", "wrong-manifest", "boot-drift", "database-generation",
                 "floor-ahead")
        for name in cases:
            with self.subTest(case=name), self.begin_action("STOP") as (operation, action):
                live, invocation, readback_context, _result_bytes = self._fenced_fixture(
                    operation, action)
                if name == "open-invocation":
                    invocation._closed = False
                elif name == "live-container":
                    invocation._cid = "owned-container"
                elif name == "unimported":
                    invocation._imported = False
                elif name == "wrong-cleanup":
                    invocation._cleanup_state = "REMOVED"
                elif name == "wrong-context":
                    invocation._context = object()
                elif name == "wrong-token":
                    invocation._token = object()
                elif name == "wrong-action-type":
                    invocation.custody = object()
                elif name == "foreign-action-custody":
                    invocation.custody = operation.open_action(0)
                elif name == "closed-action":
                    action.close()
                elif name == "wrong-lock":
                    live.lock_identity = ("foreign",)
                elif name == "wrong-manifest":
                    live.manifest_sha256 = "sha256:" + "f" * 64
                    live.verified.manifest_sha256 = live.manifest_sha256
                elif name == "floor-ahead":
                    operation._journal.advance_floor(5)

                @contextlib.contextmanager
                def open_fake_readback(*_args):
                    selected = _FenceReadbackContext(live, readback_context._factory)
                    if name == "database-generation":
                        selected.database_generation = "different-generation"
                    try:
                        yield selected
                    finally:
                        selected._closed = True

                with mock.patch.object(ops._PaperContext, "revalidate"), \
                        mock.patch.object(ops, "_current_boot_id",
                                          side_effect=(
                                              [operation._last[3]["boot_id"],
                                               "00000000-0000-7000-8000-000000000099"]
                                              if name == "boot-drift" else None),
                                          return_value=operation._last[3]["boot_id"]), \
                        mock.patch.object(ops, "_file", fixture_operations_file), \
                        mock.patch.object(ops, "open_custody_context",
                                          side_effect=open_fake_readback) as open_readback:
                    with self.assertRaises((ops.PaperOperationsError,
                                            ops.journal.JournalError)):
                        live.fence_begin(invocation)
                    if name in {"open-invocation", "live-container", "unimported",
                                "wrong-cleanup", "wrong-context", "wrong-token", "wrong-action-type",
                                "foreign-action-custody", "closed-action", "wrong-lock",
                                "wrong-manifest", "floor-ahead"}:
                        open_readback.assert_not_called()
                    elif name in {"boot-drift", "database-generation"}:
                        open_readback.assert_called_once()
                self.assertEqual(operation._last[2], "INTENT")
                self.assertGreaterEqual(operation._journal._read_floor(), 3)
                with self.assertRaises(FileNotFoundError):
                    os.stat("00000001.json", dir_fd=operation._fd,
                            follow_symlinks=False)
                if name == "foreign-action-custody":
                    invocation.custody.close()

    def test_begin_fence_retains_result_identity_through_original_store(self):
        self._assert_fence_result_write_boundary("store")

    def test_begin_fence_retains_original_invocation_result_identity(self):
        self._assert_fence_result_write_boundary("before")

    def test_begin_fence_retains_result_identity_through_stage_append(self):
        self._assert_fence_result_write_boundary("append")

    def test_begin_fence_retains_result_identity_through_floor_write(self):
        self._assert_fence_result_write_boundary("floor")

    def _assert_fence_result_write_boundary(self, boundary):
        with self.begin_action("STOP") as (operation, action):
            live, invocation, readback_context, result_bytes = self._fenced_fixture(
                operation, action)

            def replace_result():
                output_fd = action.borrow_output_fd()
                name = digest(result_bytes)[7:] + ".json"
                os.unlink(name, dir_fd=output_fd)
                ops.journal._immutable(output_fd, name, result_bytes, 0o400)

            @contextlib.contextmanager
            def open_fake_readback(*_args):
                selected = _FenceReadbackContext(live, readback_context._factory)
                try:
                    yield selected
                finally:
                    selected._closed = True

            owner, method = {
                "before": (ops.journal.Operation, "_store_original"),
                "store": (ops.journal.Operation, "_store_original"),
                "append": (ops.journal.Operation, "append"),
                "floor": (ops.journal.Journal, "advance_floor"),
            }[boundary]
            original = getattr(owner, method)

            def mutate_after_write(instance, *args):
                result = original(instance, *args)
                if boundary != "before":
                    replace_result()
                return result

            if boundary == "before":
                replace_result()

            with mock.patch.object(ops._PaperContext, "revalidate"), \
                    mock.patch.object(ops, "_current_boot_id",
                                      return_value=operation._last[3]["boot_id"]), \
                    mock.patch.object(ops, "_file", fixture_operations_file), \
                    mock.patch.object(ops, "open_custody_context",
                                      side_effect=open_fake_readback), \
                    mock.patch.object(owner, method, mutate_after_write):
                with self.assertRaises(ops.PaperOperationsError):
                    live.fence_begin(invocation)
            self.assertEqual(operation._last[2],
                             "INTENT" if boundary in {"before", "store"} else "FENCED")
            self.assertEqual(operation._journal._read_floor(),
                             4 if boundary == "floor" else 3)

    def _assert_fence_positive(self, purpose):
        with self.begin_action(purpose) as (operation, action):
            live, invocation, readback_context, result_bytes = self._fenced_fixture(
                operation, action)

            @contextlib.contextmanager
            def open_fake_readback(*_args):
                selected = _FenceReadbackContext(live, readback_context._factory)
                try:
                    yield selected
                finally:
                    selected._closed = True

            with mock.patch.object(ops._PaperContext, "revalidate"), \
                    mock.patch.object(ops, "_current_boot_id",
                                      return_value=operation._last[3]["boot_id"]), \
                    mock.patch.object(ops, "_file", fixture_operations_file), \
                    mock.patch.object(ops, "open_custody_context",
                                      side_effect=open_fake_readback):
                stage_digest = live.fence_begin(invocation)
                self.assertEqual(stage_digest, digest(
                    ops.journal._read_named(operation._fd, "00000001.json", 0o400,
                                            ops.journal.MAX_SMALL)))
                self.assertEqual(operation._last[2], "FENCED")
                self.assertEqual(operation._last[3]["artifact_sha256"],
                                 digest(result_bytes))
                self.assertEqual(operation._journal._read_floor(), 4)
                self.assertEqual(operation.read_original(digest(result_bytes)), result_bytes)
                stage_before = ops.journal._read_named(
                    operation._fd, "00000001.json", 0o400, ops.journal.MAX_SMALL)
                with mock.patch.object(ops.journal.Operation, "append", autospec=True,
                                       side_effect=AssertionError("replay must not append")), \
                        mock.patch.object(ops.journal, "_atomic_replace",
                                          side_effect=AssertionError("floor already advanced")):
                    self.assertEqual(live.fence_begin(invocation), stage_digest)
                self.assertEqual(ops.journal._read_named(
                    operation._fd, "00000001.json", 0o400, ops.journal.MAX_SMALL),
                                 stage_before)
                other_call = object.__new__(ops._NativeInvocation)
                for name in ops._NativeInvocation.__slots__:
                    if name != "__weakref__" and hasattr(invocation, name):
                        setattr(other_call, name, getattr(invocation, name))
                with self.assertRaises(ops.PaperOperationsError):
                    live.fence_begin(other_call)
                self.assertEqual(ops.journal._read_named(
                    operation._fd, "00000001.json", 0o400, ops.journal.MAX_SMALL),
                                 stage_before)
                self.assertEqual(operation._journal._read_floor(), 4)
                later = dict(operation._last[3])
                later.update(sequence=2, previous_sha256=stage_digest, stage="STOPPED")
                operation.append(canonical(later))
                with self.assertRaises(ops.PaperOperationsError):
                    live.fence_begin(invocation)
                self.assertEqual(operation._last[0], 2)
                self.assertEqual(operation._last[2], "STOPPED")
                self.assertEqual(operation._journal._read_floor(), 4)


if __name__ == "__main__":
    unittest.main()
