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


class PaperOperationsDecisionTests(unittest.TestCase):
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


if __name__ == "__main__":
    unittest.main()
