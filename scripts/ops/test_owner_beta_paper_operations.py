#!/usr/bin/env python3
"""Decision tests for the retained Paper transport (no Docker/DB authority)."""
from __future__ import annotations

import contextlib
import fcntl
import hashlib
import importlib.util
import inspect
import itertools
import json
import os
import pathlib
import shutil
import stat
import subprocess
import sys
import tempfile
import threading
import types
import uuid
import unittest
from unittest import mock


SOURCE = pathlib.Path(__file__).with_name("lib") / "owner-beta-paper-operations.py"
SPEC = importlib.util.spec_from_file_location("owner_beta_paper_operations_tested", SOURCE)
ops = sys.modules.get(SPEC.name)
if ops is None:
    ops = importlib.util.module_from_spec(SPEC)
    sys.modules[SPEC.name] = ops
    SPEC.loader.exec_module(ops)
elif pathlib.Path(ops.__file__).resolve() != SOURCE.resolve():
    raise ImportError("Paper operations test source changed")


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


class FixtureModuleIdentityTests(unittest.TestCase):
    def test_sibling_fixture_reimport_keeps_the_original_context_module_identity(self):
        backup = ops._load_local("owner_beta_paper_backup", "owner-beta-paper-backup.py")
        context = object.__new__(ops._PaperContext)
        with mock.patch.dict(sys.modules):
            alias = "_paper_operations_reimport_probe"
            spec = importlib.util.spec_from_file_location(alias, __file__)
            duplicate = importlib.util.module_from_spec(spec)
            sys.modules[alias] = duplicate
            spec.loader.exec_module(duplicate)
            self.assertIs(backup._operations(context), ops)
            self.assertIs(duplicate.ops, ops)
            self.assertIs(sys.modules[type(context).__module__]._PaperContext, type(context))


class PaperOperationsDecisionTests(unittest.TestCase):
    def test_created_native_container_accepts_unallocated_network_only_before_start(self):
        network_id = "a" * 64
        value = {"Config": {}, "HostConfig": {}, "Mounts": [],
                 "NetworkSettings": {"Networks": {"fixture": {"NetworkID": ""}}},
                 "State": {"Running": False, "Status": "created", "ExitCode": 0}}
        context = types.SimpleNamespace(revalidate=mock.Mock(), network_id=network_id,
            image="image", commit="b" * 40, image_environment={},
            docker=types.SimpleNamespace(environment={}, object=mock.Mock(
                side_effect=lambda kind, _: (value if kind == "container" else
                    {"Id": network_id, "Name": "fixture"} if kind == "network" else {"Config": {}}))))
        call = ops._NativeInvocation(ops._INVOCATION_TOKEN, context, "inspect", None)
        call._cid, call._nonce, call._created, call._exit = "cid", "nonce", "created", 0
        with mock.patch.object(ops, "_validate_container_identity"), \
             mock.patch.object(ops, "_validate_runtime_policy"), \
             mock.patch.object(ops, "_validate_mount_records"), \
             mock.patch.object(ops, "_image_environment", return_value={}):
            call._inspect_and_validate(started=False)
            for endpoints in ({"foreign": {"NetworkID": ""}},
                              {"fixture": {"NetworkID": ""}, "foreign": {"NetworkID": ""}}):
                value["NetworkSettings"]["Networks"] = endpoints
                with self.subTest(endpoints=endpoints), self.assertRaisesRegex(
                        ops.PaperOperationsError, "PAPER_OPS_NETWORK"):
                    call._inspect_and_validate(started=False)
            value["NetworkSettings"]["Networks"] = {"fixture": {"NetworkID": ""}}
            for endpoint in ("foreign", None):
                value["NetworkSettings"]["Networks"]["fixture"]["NetworkID"] = endpoint
                with self.subTest(endpoint=endpoint), self.assertRaisesRegex(
                        ops.PaperOperationsError, "PAPER_OPS_NETWORK"):
                    call._inspect_and_validate(started=False)
            value["NetworkSettings"]["Networks"]["fixture"]["NetworkID"] = ""
            for started, status, running in ((None, "running", True), (True, "exited", False)):
                value["State"].update(Status=status, Running=running)
                with self.subTest(status=status), self.assertRaisesRegex(
                        ops.PaperOperationsError, "PAPER_OPS_NETWORK"):
                    call._inspect_and_validate(started=started)
                value["NetworkSettings"]["Networks"]["fixture"]["NetworkID"] = network_id
                call._inspect_and_validate(started=started)
                value["NetworkSettings"]["Networks"]["fixture"]["NetworkID"] = ""

    def test_restore_begin_capability_requires_registered_owner_relationship(self):
        module = ops._load_local("owner_beta_paper_backup", "owner-beta-paper-backup.py")
        context = types.SimpleNamespace(network_id="net", image="image", _invocations=set())
        package = object.__new__(module._ArchivePackage)
        destination = object.__new__(module._DestinationRestore)
        custody = object.__new__(ops.journal.ActionCustody)
        operation = types.SimpleNamespace(_intent={"purpose": "RESTORE"})
        custody._operation, custody._action = operation, {"verb": "begin"}
        destination.package, destination.operation = package, operation
        destination.active_custody, destination.closed = custody, False
        package.owner = types.SimpleNamespace(context=context)
        package.destination = destination
        call = ops._NativeInvocation(ops._INVOCATION_TOKEN, context, "begin", custody)
        call._archive_owner, call._restore_owner = package, destination
        package.call = call
        context._invocations.add(call)
        module._PACKAGES.add(package)
        self.addCleanup(module._PACKAGES.discard, package)
        self.assertTrue(call._archive_restore_begin())
        args = call._create_args()
        self.assertEqual([args[i + 1] for i, value in enumerate(args) if value == "--cap-add"],
                         ["DAC_READ_SEARCH"])
        context.revalidate = mock.Mock()
        context.commit, context.image_environment = "a" * 40, {}
        context.docker = types.SimpleNamespace(environment=ops._docker_client_environment())
        call._cid, call._created, call._nonce = "id", "created", "nonce"
        call._mounts = (("/source", "/target", True),)
        config = {"Entrypoint": [ops.NATIVE], "Cmd": ["begin"], "User": "0:0",
                  "WorkingDir": "/", "Tty": False, "Env": list(ops.EXPECTED_RAW_ENV)}
        host = {"ReadonlyRootfs": True, "Privileged": False, "CapAdd": ["DAC_READ_SEARCH"],
                "CapDrop": ["ALL"], "AutoRemove": False, "RestartPolicy": {"Name": "no"},
                "LogConfig": {"Type": "none"}, "Memory": 2_147_483_648,
                "NanoCpus": 1_000_000_000, "NetworkMode": "none", "PublishAllPorts": False,
                "PidMode": "private", "IpcMode": "private", "UTSMode": "private",
                "CgroupnsMode": "private", "SecurityOpt": ["no-new-privileges"],
                "GroupAdd": ["10001", "10002", "10003"]}
        value = {"Config": config, "HostConfig": host, "Mounts": [
            {"Type": "bind", "Source": "/source", "Destination": "/target", "RW": False}],
            "NetworkSettings": {"Networks": {}}, "State": {"Running": False, "Status": "created"}}
        context.docker.object = mock.Mock(side_effect=lambda kind, _: value if kind == "container" else {"Config": {}})
        with mock.patch.object(module._ArchivePackage, "revalidate"), \
             mock.patch.object(module._DestinationRestore, "revalidate"), \
             mock.patch.object(ops.journal.ActionCustody, "revalidate"), \
             mock.patch.object(ops, "_validate_container_identity"), \
             mock.patch.object(ops, "_image_environment", return_value={}), \
             mock.patch.object(ops, "_effective_container_environment", return_value=ops.NATIVE_ENV):
            call._inspect_and_validate(started=False)
            call._archive_observed_container(value)
            for caps in ([], ["DAC_READ_SEARCH", "SYS_ADMIN"]):
                host["CapAdd"] = caps
                for inspect_call in (lambda: call._inspect_and_validate(started=False),
                                     lambda: call._archive_observed_container(value)):
                    with self.assertRaises(ops.PaperOperationsError):
                        inspect_call()
            host["CapAdd"] = ["DAC_READ_SEARCH"]
            value["Mounts"][0]["RW"] = True
            for inspect_call in (lambda: call._inspect_and_validate(started=False),
                                 lambda: call._archive_observed_container(value)):
                with self.assertRaises(ops.PaperOperationsError):
                    inspect_call()
        for obj, attr, invalid in ((package, "call", object()),
                                   (package, "destination", object()),
                                   (destination, "package", object()),
                                   (destination, "active_custody", object()),
                                   (destination, "operation", object()),
                                   (destination, "closed", True),
                                   (package.owner, "context", object())):
            original = getattr(obj, attr)
            setattr(obj, attr, invalid)
            with self.subTest(attr=attr), self.assertRaises(ops.PaperOperationsError):
                call._archive_restore_begin()
            setattr(obj, attr, original)
        operation._intent["purpose"] = "STOP"
        with self.assertRaises(ops.PaperOperationsError):
            call._archive_restore_begin()
        operation._intent["purpose"] = "RESTORE"
        custody._action["verb"] = "off"
        with self.assertRaises(ops.PaperOperationsError):
            call._archive_restore_begin()
        custody._action["verb"] = "begin"
        module._PACKAGES.remove(package)
        with self.assertRaises(ops.PaperOperationsError):
            call._archive_restore_begin()
        module._PACKAGES.add(package)
        context._invocations.remove(call)
        with self.assertRaises(ops.PaperOperationsError):
            call._archive_restore_begin()
        context._invocations.add(call)
        for verb in ("reconcile", "restore-off", "readback", "off", "enable", "recover", "observe-reset"):
            call.verb = verb
            self.assertFalse(call._archive_restore_begin())
            self.assertNotIn("--cap-add", call._create_args())

    def test_archive_invoker_rejects_caller_attestations(self):
        context = object.__new__(ops._PaperContext)
        for value in ({"verified": True}, object(), None):
            with self.assertRaisesRegex(ops.PaperOperationsError, "PAPER_OPS_PRIVATE"):
                context._invoke_archive(value)

    def test_archive_transport_network_is_local_without_changing_ordinary_network(self):
        context = types.SimpleNamespace(network_id="actual-source-network", image="sha256:"+"a"*64)
        call = ops._NativeInvocation(ops._INVOCATION_TOKEN, context, "readback", object())
        call._mounts = (("/actual/package", ops.PRIVATE+"/archive", True),
                        ("/actual/socket", "/run/postgresql", True))
        ordinary = call._create_args()
        self.assertEqual(ordinary[ordinary.index("--network")+1], "actual-source-network")
        call._archive_owner = object()  # component-only; cannot pass preparation
        archive = call._create_args()
        self.assertEqual(archive[archive.index("--network")+1], "none")
        self.assertEqual(archive[-2:], [context.image, "readback"])
        self.assertEqual([archive[i+1] for i, value in enumerate(archive) if value == "--group-add"],
                         ["10001", "10002", "10003"])
        self.assertNotIn("--group-add", ordinary)
        self.assertIn("type=bind,source=/actual/socket,target=/run/postgresql,readonly", archive)
        for name in ops._native_env_args():
            self.assertIn(name, archive)
        self.assertEqual(context.network_id, "actual-source-network")

    def test_archive_docker_work_preserves_original_cleanup_reserve(self):
        base = types.SimpleNamespace(environment=ops.NATIVE_ENV, call=mock.Mock())
        deadline = types.SimpleNamespace(work_remaining=mock.Mock(return_value=2.5),
                                         remaining=mock.Mock())
        docker = ops._ArchiveWorkDocker(base, deadline)
        docker.call(["inspect", "captured"], timeout=15)
        deadline.work_remaining.assert_called_once_with(15)
        deadline.remaining.assert_not_called()
        base.call.assert_called_once_with(["inspect", "captured"], timeout=2.5)

    def test_archive_terminal_receipt_cannot_be_minted_without_actual_invocation(self):
        call = ops._NativeInvocation(ops._INVOCATION_TOKEN, mock.Mock(), "readback", object())
        with self.assertRaisesRegex(ops.PaperOperationsError, "PAPER_OPS_UNRESOLVED"):
            call._retain_archive_terminal()

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
        # Docker CLI 28 rebuilds --env from a Go map, with no stable order.
        for permutation in itertools.permutations(ops.EXPECTED_RAW_ENV):
            self.assertIsNone(ops._validate_runtime_policy(
                dict(config, Env=list(permutation)), host, "begin", image_env, "net-id", client_env))
        with self.assertRaises(ops.PaperOperationsError):
            ops._validate_runtime_policy(config, host, "begin", image_env, "net-id", client_env,
                                         archive_restore_begin=True)
        for caps in (["DAC_READ_SEARCH"], ["DAC_READ_SEARCH", "SYS_ADMIN"], ["DAC_OVERRIDE"]):
            changed = dict(host, CapAdd=caps)
            if len(caps) == 1 and caps[0] == "DAC_READ_SEARCH":
                self.assertIsNone(ops._validate_runtime_policy(config, changed, "begin", image_env,
                                  "net-id", client_env, archive_restore_begin=True))
            else:
                with self.assertRaises(ops.PaperOperationsError):
                    ops._validate_runtime_policy(config, changed, "begin", image_env, "net-id", client_env,
                                                 archive_restore_begin=True)
            with self.assertRaises(ops.PaperOperationsError):
                ops._validate_runtime_policy(config, changed, "begin", image_env, "net-id", client_env)
        for section, key, replacement in (
                (config, "Env", list(ops.EXPECTED_RAW_ENV) + ["DOCKER_HOST=tcp://example"]),
                (config, "Env", ["LANG=host-value", *ops.EXPECTED_RAW_ENV[1:]]),
                (config, "Env", [*ops.EXPECTED_RAW_ENV, "PATH"]),
                (config, "Env", [*ops.EXPECTED_RAW_ENV[:-1], "PATH="]),
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

    def test_observe_reset_consumes_native_query_schema(self):
        # Native Verb::result_schema selects the query family for ObserveReset.
        # Keep its ordered stdout independent of Python's schema lookup.
        original = (
            b'{"schema":"owner-beta-paper-ops-query-result-v1","verb":"observe-reset",'
            b'"outcome":"OK","event_id":null,'
            b'"generation":"11111111-1111-4111-8111-111111111111",'
            b'"control_epoch":8,"artifact_sha256":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
            b'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","error_code":"NONE"}')
        for raw in (original, original + b'\n'):
            with self.subTest(terminal_lf=raw.endswith(b'\n')):
                result = ops._validate_result(raw, "observe-reset", 0)
                self.assertEqual(result["outcome"], "OK")
                self.assertIsNone(result["event_id"])
                self.assertEqual(result["control_epoch"], 8)
        wrong_family = original.replace(b'owner-beta-paper-ops-query-result-v1',
                                        b'owner-beta-paper-ops-result-v1')
        with self.assertRaises(ops.PaperOperationsError):
            ops._validate_result(wrong_family, "observe-reset", 0)

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

    def test_closed_begin_transfers_original_result_fd_to_context(self):
        invocation = object.__new__(ops._NativeInvocation)
        invocation._restore_owner = None
        context = types.SimpleNamespace(_mode="live", _invocations={invocation},
                                        _begin_result_fds={})
        invocation._context = context
        invocation.verb = "begin"
        invocation._closed = False
        invocation._child = invocation._cid = None
        invocation._preserve = False
        invocation._imported = True
        invocation._exit = 0
        invocation._persistent_stage = object()
        invocation._stages = [invocation._persistent_stage]
        invocation._secret_sources = invocation._paper_state_files = []
        invocation._probe_output_fd = invocation._paper_state_fd = None
        invocation._preexisting_dispatch_fd = None
        with tempfile.TemporaryFile() as original:
            original_fd = os.dup(original.fileno())
            invocation._result_fd = original_fd
            try:
                invocation.close()
                self.assertEqual(context._begin_result_fds.get(invocation), original_fd)
                self.assertIsNone(invocation._result_fd)
                self.assertEqual(os.fstat(original_fd).st_ino,
                                 os.fstat(original.fileno()).st_ino)
                invocation.close()
                self.assertEqual(context._begin_result_fds[invocation], original_fd)
            finally:
                for fd in context._begin_result_fds.values():
                    os.close(fd)
                context._begin_result_fds.clear()

    def test_context_close_releases_retained_original_result_fd(self):
        context = object.__new__(ops._PaperContext)
        context._closed = False
        context._inventories = context._invocations = context._retained_stages = set()
        context._input_stages = {}
        context.verified = None
        context._release_tree = mock.Mock()
        context.tree = mock.Mock()
        with tempfile.TemporaryFile() as original:
            retained_fd = os.dup(original.fileno())
            context._begin_result_fds = {object(): retained_fd}
            context._manifest_fd = os.dup(original.fileno())
            context.lock_fd = os.dup(original.fileno())
            with mock.patch.object(ops._PaperContext, "revalidate"):
                context.close()
                context.close()
            self.assertEqual(context._begin_result_fds, {})
            with self.assertRaises(OSError):
                os.fstat(retained_fd)
            self.assertGreaterEqual(os.fstat(original.fileno()).st_size, 0)

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
        live._begin_result_fds = {}
        self.addCleanup(live._close_begin_results)

        invocation = object.__new__(ops._NativeInvocation)

        invocation._restore_owner = None
        invocation._token = ops._INVOCATION_TOKEN
        invocation._context = live
        invocation.verb = "begin"
        invocation.custody = action
        invocation._entered = True
        invocation._closed = False
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
        invocation._stages = [stage_marker]
        invocation._secret_sources = invocation._paper_state_files = []
        invocation._probe_output_fd = invocation._paper_state_fd = None
        invocation._preexisting_dispatch_fd = None
        invocation._result_fd = os.open(invocation._result_name,
                                        os.O_RDONLY | os.O_NOFOLLOW,
                                        dir_fd=output_fd)
        invocation.close()

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
                original_fd = live._begin_result_fds[invocation]
                self.assertEqual(os.fstat(original_fd).st_nlink, 0)
                self.assertNotEqual(os.fstat(original_fd).st_ino,
                                    os.stat(name, dir_fd=output_fd,
                                            follow_symlinks=False).st_ino)
                self.assertEqual(os.pread(original_fd, len(result_bytes) + 1, 0),
                                 result_bytes)
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
                other_call._restore_owner = None
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


class _ComponentStopObserver:
    """Only unit-test OS/DB substitute. It never enters the hosted driver."""
    def __init__(self, attempt):
        self.attempt = attempt
        self.calls = []
        self.initial = None
        self.backends = []
        self.backend_original = {"backends": [], "prepared": 0}
        self.containers = {"a" * 64: {"pid": 101, "classes": ["API", "PAPER_DELIVERY"],
            "generation": {"Id": "a" * 64, "StartedAt": "component-start"}}}
        self.units = {}

    def capture(self):
        self.calls.append("capture")
        self.initial = {"containers": self.containers, "unit_enumeration": [],
                        "backend_observation": self.backend_original}

    def disable_launches(self):
        self.calls.append("disable")

    def signal(self):
        self.calls.append("signal")

    def wait_exit(self):
        self.calls.append("exit")

    def publication_locks(self):
        self.calls.append("locks")

    def drain_backends(self):
        self.calls.append("backends")
        return self.database("observe")

    def terminal_observation(self):
        self.calls.append("terminal")
        return {"resources": [{"id": "a" * 64, "exit": 0}],
                "launches": {"containers": ["a" * 64], "disabled": True}}

    def database(self, action):
        self.calls.append(action)
        return {"backends": [], "prepared": 0}

    def close(self):
        self.calls.append("close")


class PaperStopProofTests(unittest.TestCase):
    setUp = PaperOperationsJournalBoundaryTests.setUp
    _cleanup_fixture = PaperOperationsJournalBoundaryTests._cleanup_fixture
    begin_action = PaperOperationsJournalBoundaryTests.begin_action
    _fenced_fixture = PaperOperationsJournalBoundaryTests._fenced_fixture

    @contextlib.contextmanager
    def isolated_install(self):
        prior = self.install, self.lock_fd
        self.install = self.base / ("case-" + uuid.uuid4().hex)
        self.install.mkdir(mode=0o700)
        self.lock_fd = os.open(self.install / ops.LOCK_NAME,
                              os.O_CREAT | os.O_EXCL | os.O_RDWR, 0o600)
        fcntl.flock(self.lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        try:
            yield
        finally:
            os.close(self.lock_fd)
            self.install, self.lock_fd = prior

    @contextlib.contextmanager
    def stopped_fixture(self, purpose="STOP"):
        with self.isolated_install(), self.begin_action(purpose) as (operation, action):
            live, invocation, readback, _original = self._fenced_fixture(operation, action)
            live._token, live._closed = ops._CONTEXT_TOKEN, False
            live.docker = types.SimpleNamespace(environment=ops._docker_client_environment())
            stage = mock.Mock()
            live._input_stages[action]["stage"] = stage
            invocation._persistent_stage = stage
            boot = operation._last[3]["boot_id"]
            with mock.patch.object(ops._PaperContext, "revalidate"), \
                    mock.patch.object(ops, "_current_boot_id", return_value=boot), \
                    mock.patch.object(ops, "_file", fixture_operations_file), \
                    mock.patch.object(ops, "open_custody_context", return_value=readback), \
                    mock.patch.object(ops, "_HostStopObserver", _ComponentStopObserver):
                live.fence_begin(invocation)
                try:
                    yield live, invocation, operation
                finally:
                    for attempt in ops._STOP_ATTEMPTS.pop(live, {}).values():
                        attempt.close()

    def test_stop_requires_same_context_settled_fence(self):
        context = object.__new__(ops._PaperContext)
        with self.assertRaises(ops.PaperOperationsError):
            context.drain(object())

    def test_handle_cannot_be_constructed_or_restored_from_report(self):
        with self.assertRaises(ops.PaperOperationsError):
            ops.DrainedHandle(None, None)

    def test_real_journal_stop_and_backup_seal_only_after_exit_and_backend_proof(self):
        for purpose in ("STOP", "BACKUP"):
            with self.subTest(purpose=purpose), self.stopped_fixture(purpose) as (live, call, operation):
                handle = live.drain(call)
                self.assertIs(type(handle), ops.DrainedHandle)
                attempt = handle._attempt
                self.assertEqual(operation._last[2], "STOPPED")
                self.assertEqual(operation._last[0], 2)
                self.assertEqual(attempt.observer.calls[:7],
                                 ["capture", "disable", "signal", "exit", "locks", "backends", "observe"])
                inventory = ops._json(attempt.inventory_bytes, ops.MAX_RESULT)
                required = ops._PAPER_CLASSES if purpose == "STOP" else ops._WRITER_CLASSES
                self.assertEqual({w["writer_class"] for w in inventory["writers"]}, required)
                self.assertEqual(inventory["control_epoch"], operation._journal.epoch_floor)
                self.assertEqual(inventory["fence_event_id"], call._result["event_id"])
                for row in inventory["writers"]:
                    for key in ("identity_sha256", "observation_sha256", "backend_set_sha256"):
                        self.assertIn(row[key], attempt.originals.files)
                        self.assertNotEqual(row[key], digest(b""))
                before = ops.journal._read_named(operation._fd, "00000002.json", 0o400, ops.MAX_RESULT)
                self.assertIs(live.drain(call), handle)
                self.assertEqual(before, ops.journal._read_named(operation._fd, "00000002.json", 0o400, ops.MAX_RESULT))
                handle.close()
                with self.assertRaises(ops.PaperOperationsError):
                    live.drain(call)

    def test_signal_return_without_exit_never_seals_stopped(self):
        with self.stopped_fixture() as (live, call, operation), \
                mock.patch.object(_ComponentStopObserver, "wait_exit",
                                  side_effect=ops.PaperOperationsError("PAPER_OPS_WRITER_NOT_EXITED")):
            with self.assertRaisesRegex(ops.PaperOperationsError, "PAPER_OPS_WRITER_NOT_EXITED"):
                live.drain(call)
            self.assertEqual(operation._last[2], "FENCED")
            attempt = ops._STOP_ATTEMPTS[live][call]
            notes = [json.loads(data) for _fd, _identity, data in attempt.originals.files.values()]
            self.assertTrue(any(v.get("status") == "UNKNOWN" and v["initial_sha256"] for v in notes))

    def test_every_failed_boundary_keeps_fence_and_originals(self):
        for boundary in ("capture", "disable_launches", "signal", "wait_exit",
                         "publication_locks", "drain_backends", "terminal_observation", "database"):
            with self.subTest(boundary=boundary), self.stopped_fixture() as (live, call, operation), \
                    mock.patch.object(_ComponentStopObserver, boundary,
                                      side_effect=ops.PaperOperationsError("PAPER_OPS_BACKEND_UNKNOWN")):
                with self.assertRaises(ops.PaperOperationsError):
                    live.drain(call)
                self.assertEqual(operation._last[2], "FENCED")
                self.assertEqual(operation._journal.epoch_floor, call._result["control_epoch"])
                self.assertIsNone(ops._STOP_ATTEMPTS[live][call].handle)

    def test_final_reconnection_rejects_even_after_prior_negative_observation(self):
        with self.stopped_fixture() as (live, call, operation), \
                mock.patch.object(_ComponentStopObserver, "database",
                                  return_value={"backends": [{"pid": 444}], "prepared": 0}):
            with self.assertRaisesRegex(ops.PaperOperationsError, "PAPER_OPS_BACKEND_RECONNECTED"):
                live.drain(call)
            self.assertEqual(operation._last[2], "FENCED")

    def test_handle_is_nonserializable_and_stays_poisoned_after_identity_loss(self):
        import copy
        import pickle
        with self.stopped_fixture() as (live, call, _operation):
            handle = live.drain(call)
            for save in (copy.copy, copy.deepcopy, pickle.dumps):
                with self.assertRaises(ops.PaperOperationsError):
                    save(handle)
            with mock.patch.object(_ComponentStopObserver, "terminal_observation",
                                   side_effect=ops.PaperOperationsError("PAPER_OPS_WRITER_CHANGED")):
                with self.assertRaises(ops.PaperOperationsError):
                    handle.revalidate()
            with self.assertRaises(ops.PaperOperationsError):
                handle.revalidate()

    def test_boot_manifest_floor_context_and_pending_custody_changes_block(self):
        for boundary in ("boot", "manifest", "floor", "context", "pending", "stage"):
            with self.subTest(boundary=boundary), self.stopped_fixture() as (live, call, operation):
                if boundary == "manifest":
                    live.manifest_sha256 = "sha256:" + "d" * 64
                elif boundary == "floor":
                    operation._journal.advance_floor(operation._journal.epoch_floor + 1)
                elif boundary == "context":
                    live._closed = True
                elif boundary == "pending":
                    live._invocations.add(object())
                elif boundary == "stage":
                    stage = dict(operation._last[3]);stage.update(stage="STOPPED", sequence=2,
                                                                  previous_sha256=operation._last[1])
                    operation.append(canonical(stage))
                with contextlib.ExitStack() as patches:
                    if boundary == "boot":
                        patches.enter_context(mock.patch.object(ops, "_current_boot_id",
                                                               return_value=str(uuid.uuid4())))
                    with self.assertRaises(ops.PaperOperationsError):
                        live.drain(call)

    def test_deadline_is_one_absolute_attempt_and_keeps_draining(self):
        with self.stopped_fixture() as (live, call, operation):
            real_disable = _ComponentStopObserver.disable_launches
            def expire(observer):
                real_disable(observer)
                observer.attempt.deadline.end = 0
            with mock.patch.object(_ComponentStopObserver, "disable_launches", expire):
                with self.assertRaisesRegex(ops.PaperOperationsError, "PAPER_OPS_STOP_TIMEOUT"):
                    live.drain(call)
            self.assertEqual(operation._last[2], "FENCED")

    def test_protected_original_inode_swap_invalidates_handle(self):
        with self.stopped_fixture() as (live, call, operation):
            handle = live.drain(call)
            key, (_fd, _identity, data) = next(iter(handle._attempt.originals.files.items()))
            name = key[7:] + ".json"
            os.unlink(name, dir_fd=operation._fd)
            ops.journal._immutable(operation._fd, name, data, 0o400)
            with self.assertRaises(ops.PaperOperationsError):
                handle.revalidate()

    def test_closed_context_and_replaced_handle_registry_reject(self):
        for changed in ("context", "registry"):
            with self.subTest(changed=changed), self.stopped_fixture() as (live, call, _operation):
                handle = live.drain(call)
                attempt = handle._attempt
                if changed == "context":
                    live._closed = True
                else:
                    ops._STOP_ATTEMPTS[live][call] = None
                try:
                    with self.assertRaises(ops.PaperOperationsError):
                        handle.revalidate()
                finally:
                    ops._STOP_ATTEMPTS[live][call] = attempt

    def test_terminal_container_rejects_restart_replacement_running_and_missing_finish(self):
        created, started, finished = ("2026-11-09T00:00:00Z", "2026-11-09T00:00:01Z", "2026-11-09T00:00:02Z")
        generation = {"Id": "a"*64, "Image": "sha256:"+"b"*64, "Created": created,
                      "RestartCount": 0, "StartedAt": started}
        value = {**generation, "State": {"StartedAt": started, "Running": False,
            "Status": "exited", "Restarting": False, "Paused": False,
            "OOMKilled": False, "Error": "", "ExitCode": 0, "FinishedAt": finished},
            "HostConfig": {"RestartPolicy": {"Name": "no"}}}
        self.assertEqual(ops._terminal_container(value, generation)["exit_code"], 0)
        import copy
        for key, new in (("Id", "c"*64), ("RestartCount", 1), ("Running", True),
                         ("FinishedAt", "0001-01-01T00:00:00Z"), ("StartedAt", "later")):
            changed = copy.deepcopy(value)
            (changed if key in {"Id", "RestartCount"} else changed["State"])[key] = new
            with self.subTest(key=key), self.assertRaises(ops.PaperOperationsError):
                ops._terminal_container(changed, generation)

    def test_source_backend_correlation_uses_socket_and_start_not_application_name(self):
        observer = object.__new__(ops._HostStopObserver)
        observer.context = types.SimpleNamespace(database_generation=("d"*64,), docker=mock.Mock())
        observer.context.docker.object.return_value = {"NetworkSettings": {"Networks": {"n": {"IPAddress": "10.1.0.2"}}}}
        observer.processes = {10: {"pid": 10, "start_ticks": 42, "owner": ["CONTAINER", "a"*64],
            "sockets": [{"local": ["10.1.0.3", 12345], "peer": ["10.1.0.2", 5432], "socket_inode": 77}]}}
        row = {"pid": 20, "start_us": 99, "role": "worker", "addr": "10.1.0.3", "port": 12345}
        self.assertEqual(observer.correlate([row])[0]["process"]["start_ticks"], 42)
        for changed in ({**row, "port": 12346}, {**row, "addr": None}):
            with self.assertRaisesRegex(ops.PaperOperationsError, "PAPER_OPS_BACKEND_UNCORRELATED"):
                observer.correlate([changed])

    def test_owned_nonroot_child_pidfd_exit_and_actual_reap(self):
        import time
        child = subprocess.Popen([sys.executable, "-B", "-c", "import time; time.sleep(20)"],
                                 stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                 stderr=subprocess.DEVNULL)
        fd = None
        try:
            process = ops._process_record(child.pid)
            self.assertEqual(process["parent"], os.getpid())
            fd = os.pidfd_open(child.pid, 0)
            observer = object.__new__(ops._HostStopObserver)
            observer.deadline = ops._StopDeadline()
            observer.processes, observer.pidfds = {child.pid: process}, {child.pid: fd}
            observer.reaped = set()
            observer.terminal_observation = mock.Mock()
            signal_module = __import__("signal")
            signal_module.pidfd_send_signal(fd, signal_module.SIGTERM)
            observer.wait_exit()
            with self.assertRaises(ChildProcessError):
                os.waitpid(child.pid, os.WNOHANG)
            child.returncode = -signal_module.SIGTERM
            observer.terminal_observation.assert_called_once()
        finally:
            if fd is not None:
                os.close(fd)
            if child.poll() is None:
                child.kill();child.wait(timeout=2)

    def database_observer(self):
        observer = object.__new__(ops._HostStopObserver)
        root = {"account_id": "11111111-1111-4111-8111-111111111111",
                "generation": "22222222-2222-4222-8222-222222222222",
                "installation_id": "33333333-3333-4333-8333-333333333333"}
        event = "44444444-4444-4444-8444-444444444444"
        observer.attempt = types.SimpleNamespace(operation=types.SimpleNamespace(
            _intent=root, intent_sha256="sha256:"+"a"*64), fence=4, event=event)
        docker = mock.Mock()
        docker.object.return_value = {"Id": "d"*64, "Config": {"Env": ["POSTGRES_USER=lagrange"]},
                                      "State": {"Pid": 333}}
        observer.context = types.SimpleNamespace(revalidate=mock.Mock(), values={}, docker=docker,
            database_generation=("d"*64,), database={"database": "lagrange"},
            manifest_sha256="sha256:"+"b"*64)
        observer.backends = [{"pid": 123, "start_us": 999}]
        value = {"observer": {"pid": 234, "user": "lagrange", "session": "lagrange",
            "read_only": "on", "role": "none", "superuser": True}, "prepared": 0,
            "unexpected": 0, "backends": [], "fence": {"epoch": 4, "state": "DRAINING",
            "event": event, "generation": root["generation"]}}
        observer.command = mock.Mock(return_value=json.dumps(value).encode()+b"\n")
        return observer, value

    def test_fixed_maintenance_transport_is_readonly_local_and_generation_qualified(self):
        observer, value = self.database_observer()
        with mock.patch.object(ops, "_process_record", return_value={"cgroup": "/docker/"+"d"*64}):
            observer.database("observe")
            argv, payload = observer.command.call_args.args
            self.assertIn("/var/run/postgresql", argv)
            self.assertIn("999:999", argv)
            self.assertIn(b"READ ONLY", payload)
            self.assertNotIn(b"application_name", payload)
            self.assertNotIn(b"pg_terminate_backend", payload)
            observer.command.return_value = b"1\n" + json.dumps(value).encode()+b"\n"
            observer.database("terminate")
            payload = observer.command.call_args.args[1]
            self.assertIn(b"pg_terminate_backend", payload)
            self.assertIn(b"(123,999)", payload)
            self.assertIn(b"a.pid=v.pid", payload)
            self.assertIn(b"a.backend_start", payload)
            self.assertIn(b"v.start_us", payload)

    def test_prepared_masked_principal_and_changed_fence_fail_closed(self):
        import copy
        for case in ("prepared", "unexpected", "readonly", "superuser", "event", "epoch", "backend_start"):
            observer, value = self.database_observer()
            if case in ("prepared", "unexpected"):
                value[case] = 1
            elif case == "readonly":
                value["observer"]["read_only"] = "off"
            elif case == "superuser":
                value["observer"]["superuser"] = False
            elif case == "event":
                value["fence"]["event"] = str(uuid.uuid4())
            elif case == "epoch":
                value["fence"]["epoch"] = 5
            else:
                value["backends"] = [{"pid": 123, "start_us": None, "role": "app", "addr": None, "port": None}]
            observer.command.return_value = json.dumps(value).encode()+b"\n"
            with self.subTest(case=case), mock.patch.object(ops, "_process_record",
                    return_value={"cgroup": "/docker/"+"d"*64}), self.assertRaises(ops.PaperOperationsError):
                observer.database("observe")

    def test_new_backend_identity_is_never_signalled(self):
        observer = object.__new__(ops._HostStopObserver)
        observer.backends = [{"pid": 12, "start_us": 100}]
        observer.database = mock.Mock(return_value={"backends": [{"pid": 12, "start_us": 101}]})
        with self.assertRaisesRegex(ops.PaperOperationsError, "PAPER_OPS_BACKEND_RECONNECTED"):
            observer.drain_backends()
        observer.database.assert_called_once_with("observe")

    def test_launch_replacement_restart_and_enabled_source_are_not_termination(self):
        import copy
        unit = {"originals": [{"sha256": "source"}], "file_state": "disabled",
                "triggers": [], "triggered_by": [], "active": "inactive", "pid": 0,
                "invocation": "invocation", "cgroup": "/owned"}
        container = {"generation": {"Id": "a"*64, "RestartCount": 0}, "mounts": [],
                     "policy": "policy", "restart": "no"}
        for case in ("source", "restart", "replacement", "enabled"):
            observer = object.__new__(ops._HostStopObserver)
            observer.units, observer.containers = {"owned.service": unit}, {"a"*64: container}
            now_units, now_containers = copy.deepcopy(observer.units), copy.deepcopy(observer.containers)
            if case == "source":
                now_units["owned.service"]["originals"] = []
            elif case == "enabled":
                now_units["owned.service"]["file_state"] = "enabled"
            elif case == "restart":
                now_containers["a"*64]["generation"]["RestartCount"] = 1
            else:
                now_containers = {"b"*64: container}
            observer.units_now = mock.Mock(return_value=now_units)
            observer.containers_now = mock.Mock(return_value=now_containers)
            observer.systemctl = mock.Mock(return_value="")
            with self.subTest(case=case), self.assertRaises(ops.PaperOperationsError):
                observer.launch_observation()

    def test_missing_class_unknown_class_and_size_cannot_mint_inventory(self):
        with self.stopped_fixture() as (live, call, operation):
            original_capture = _ComponentStopObserver.capture
            def unknown(observer):
                original_capture(observer)
                observer.containers["a"*64]["classes"] = ["CALLER_SELECTED"]
            with mock.patch.object(_ComponentStopObserver, "capture", unknown):
                with self.assertRaisesRegex(ops.PaperOperationsError, "PAPER_OPS_WRITER_INVENTORY"):
                    live.drain(call)
            self.assertEqual(operation._last[2], "FENCED")

    def test_stop_inventory_wire_uses_native_identity_class_kind_order(self):
        with self.stopped_fixture("BACKUP") as (live, call, _operation):
            handle = live.drain(call)
            rows = json.loads(handle._attempt.inventory_bytes)["writers"]
            keys = [(w["identity_sha256"], w["writer_class"], w["kind"]) for w in rows]
            self.assertEqual(keys, sorted(set(keys)))
            self.assertEqual(len(rows), 11)

    def test_captured_cgroup_does_not_make_unchanged_container_fail_before_signal(self):
        observer = object.__new__(ops._HostStopObserver)
        row = {"generation": {"Id": "a"*64}, "pid": 123, "cgroup": "/owned"}
        observer.units, observer.containers = {}, {"a"*64: row}
        observer.units_now = mock.Mock(return_value={})
        observer.containers_now = mock.Mock(return_value={"a"*64: {"generation": row["generation"], "pid": 123}})
        observer.context = types.SimpleNamespace(docker=mock.Mock())
        observer.attempt = types.SimpleNamespace(originals=mock.Mock())
        observer.launch_observation = mock.Mock(return_value={"disabled": True})
        with mock.patch.object(ops, "_container_generation", return_value=row["generation"]):
            observer.disable_launches()
        observer.context.docker.call.assert_called_once_with(["update", "--restart=no", "a"*64])
        observer.launch_observation.assert_called_once_with(waiting=True)

    def test_unknown_note_failure_preserves_original_stop_error(self):
        with self.stopped_fixture() as (live, call, operation):
            with mock.patch.object(_ComponentStopObserver, "capture", side_effect=ops.PaperOperationsError("PAPER_OPS_PROCESS_CHANGED")), \
                    mock.patch.object(ops._ObservationOriginals, "put", side_effect=OSError()):
                with self.assertRaisesRegex(ops.PaperOperationsError, "PAPER_OPS_PROCESS_CHANGED"):
                    live.drain(call)
            self.assertEqual(operation._last[2], "FENCED")
            self.assertTrue(ops._STOP_ATTEMPTS[live][call].observation_write_unknown)

    def test_cgroup_kernel_populated_is_required_even_with_empty_proc_scan(self):
        observer = object.__new__(ops._HostStopObserver)
        info = os.stat(self.base)
        observer.cgroup_fds = {"/owned": (77, ops._identity(info))}
        with mock.patch.object(ops.os, "fstat", return_value=info), \
                mock.patch.object(ops.Path, "lstat", return_value=info), \
                mock.patch.object(ops.os, "open", return_value=78), \
                mock.patch.object(ops.os, "close"), \
                mock.patch.object(ops.os, "read", return_value=b"populated 1\nfrozen 0\n"):
            with self.assertRaisesRegex(ops.PaperOperationsError, "PAPER_OPS_CGROUP_NOT_EMPTY"):
                observer.empty_cgroup("/owned")
            with mock.patch.object(ops.os, "read", return_value=b"populated 0\nfrozen 0\n"):
                self.assertEqual(observer.empty_cgroup("/owned")["populated"], 0)

    def test_unit_dispatch_failure_resumes_only_captured_pidfd(self):
        observer = object.__new__(ops._HostStopObserver)
        observer.units = {"owned.service": {"pid": 123, "invocation": "same", "cgroup": "/owned"}}
        observer.containers = {}
        observer.processes = {123: {"start_ticks": 100}}
        observer.pidfds = {123: 77}
        observer.launch_observation = mock.Mock()
        observer.units_now = mock.Mock(return_value=observer.units)
        observer.systemctl = mock.Mock(side_effect=ops.PaperOperationsError("PAPER_OPS_STOP_TRANSPORT_UNKNOWN"))
        with mock.patch.object(ops, "_process_generation", return_value=(100, "T")), \
                mock.patch.object(ops.signal, "pidfd_send_signal") as signal_call:
            with self.assertRaisesRegex(ops.PaperOperationsError, "PAPER_OPS_STOP_TRANSPORT_UNKNOWN"):
                observer.signal()
        self.assertEqual(signal_call.call_args_list,
                         [mock.call(77, ops.signal.SIGSTOP), mock.call(77, ops.signal.SIGCONT)])

class PaperCommitOffTests(unittest.TestCase):
    setUp = PaperStopProofTests.setUp
    _cleanup_fixture = PaperStopProofTests._cleanup_fixture
    begin_action = PaperStopProofTests.begin_action
    _fenced_fixture = PaperStopProofTests._fenced_fixture
    isolated_install = PaperStopProofTests.isolated_install
    stopped_fixture = PaperStopProofTests.stopped_fixture

    @contextlib.contextmanager
    def transport(self, live, *, result_mutator=None, readback_mutator=None,
                  lost_stdout=False, unknown=False):
        """Component SQL/OS substitute; real journal/action/result files and FDs."""
        submitted, results = [], {}
        def run(call):
            action = call.custody
            root = action._operation._intent
            submitted.append(call.verb)
            stage = mock.Mock(closed=False)
            call._persistent_stage = stage
            call._stages = [stage]
            live._input_stages[action] = {"stage": stage, "imports": []}
            fence = ops._begin_fence_epoch(root)
            event = str(uuid.uuid4())
            if call.verb == "reconcile":
                args = {"generation": root["generation"], "expected_epoch": fence,
                        "inventory_sha256": digest(action._artifact_bytes),
                        "request_key": action._action["request_key"]}
                summary = {"schema": "owner-beta-paper-reconciliation-v1",
                    "installation_id": root["installation_id"], "generation": root["generation"],
                    "account_id": root["account_id"], "observation_id": event,
                    "control_epoch": fence, "inventory_sha256": digest(action._artifact_bytes),
                    "observed_at": "2026-11-09T12:00:00Z", "unresolved_commits": 0,
                    "live_claims": 0, "held_targets": 2, "quarantined_deliveries": 1,
                    **{k: digest(k.encode()) for k in ("database_sha256", "ledger_sha256",
                       "pending_sha256", "outbox_sha256", "jobs_sha256")}}
                body = {"observation_id": event, "summary": summary,
                        "summary_sha256": digest(canonical(summary))}
            else:
                args = {"expected_epoch": fence, "transition": "OFF",
                        "evidence_canonical": action._artifact_bytes.decode("ascii"),
                        "request_key": action._action["request_key"]}
                body = {"event_id": event, "state": "OFF"}
            value = {"schema": ops.PAGE_SCHEMA,
                     "action": "RECONCILE_OPERATION" if call.verb == "reconcile" else "TRANSITION",
                     "outcome": "OK", "control_epoch": fence + (call.verb == "off"),
                     "account_id": root["account_id"], "data": body}
            if result_mutator:
                result_mutator(call.verb, value)
            raw = canonical(value)
            results[action.sequence] = (raw, event)
            request = canonical({"schema": "owner-beta-paper-exact-request-v1",
                "kind": "RECONCILE" if call.verb == "reconcile" else "TRANSITION",
                "account_id": root["account_id"], "args": args})
            dispatch = canonical({"schema": "owner-beta-paper-native-dispatch-v2",
                "root_intent_canonical": action._operation._intent_bytes.decode("ascii"),
                "root_intent_sha256": action._operation.intent_sha256,
                "action_canonical": action._binding_bytes.decode("ascii"),
                "action_sha256": digest(action._binding_bytes),
                "artifact_canonical": action._artifact_bytes.decode("ascii"),
                "artifact_sha256": digest(action._artifact_bytes),
                "sql_request_canonical": request.decode("ascii"), "sql_request_sha256": digest(request)})
            output = action.borrow_output_fd()
            objects = [action._operation._intent_bytes, action._binding_bytes,
                       action._artifact_bytes, request, dispatch, raw]
            if call.verb == "reconcile":
                objects.append(canonical(body["summary"]))
            for data in objects:
                ops.journal._immutable(output, digest(data)[7:] + ".json", data, 0o400)
            ops.journal._immutable(output, "dispatch.json", dispatch, 0o400)
            call._exit = 0
            call._cleanup_state = "OWNED"
            if lost_stdout:
                call._preserve = True
                raise ops.PaperOperationsError("PAPER_OPS_ATTACH_UNKNOWN")
            call._result = dict(zip(ops.RESULT_FIELDS, (ops.RESULT_SCHEMA, call.verb,
                "COMMIT_UNKNOWN" if unknown else "OK", event, root["generation"],
                value["control_epoch"], digest(raw), "COMMIT_UNKNOWN" if unknown else "NONE")))
            call._capture_result_artifact()
            call._preserve = unknown

        def reader(consumer):
            def read(custody):
                raw, event = results[custody._sequence]
                result = json.loads(raw)
                request = json.loads(custody._sql)
                epoch = consumer.attempt.fence + (request["kind"] == "TRANSITION")
                value = {"schema": "owner-beta-paper-exact-readback-v1", "kind": request["kind"],
                    "request_sha256": digest(custody._sql), "status": "COMMITTED",
                    "account_id": result["account_id"], "current_epoch": epoch,
                    "current_state": "OFF" if request["kind"] == "TRANSITION" else "DRAINING",
                    "event_id": event, "event_epoch": epoch, "result_canonical": raw.decode("ascii"),
                    "result_sha256": digest(raw), "reset": None}
                if readback_mutator:
                    readback_mutator(consumer, value)
                data = canonical(value)
                output = custody.borrow_output_fd()
                name = digest(data)[7:] + ".json"
                ops.journal._immutable(output, name, data, 0o400)
                wire = dict(zip(ops.RESULT_FIELDS, (ops.QUERY_RESULT_SCHEMA, "readback", "REPLAY",
                    event, consumer.operation.generation, epoch, digest(data), "NONE")))
                response = _FenceReadbackRun(data, wire)
                response._result_fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW, dir_fd=output)
                self.addCleanup(os.close, response._result_fd)
                return response
            return _FenceReadbackContext(live, read)
        with mock.patch.object(ops._NativeInvocation, "_run", run), \
                mock.patch.object(ops, "_open_maintenance_reader", side_effect=reader):
            yield submitted

    def test_stop_and_backup_commit_exact_epochs_stages_and_zero_sql_replay(self):
        for purpose in ("STOP", "BACKUP"):
            with self.subTest(purpose=purpose), self.stopped_fixture(purpose) as (live, call, operation):
                handle = live.drain(call)
                with self.transport(live) as submitted:
                    event = ops.commit_off(handle)
                    consumer = handle._attempt.consumer
                    self.assertEqual(submitted, ["reconcile", "off"])
                    self.assertEqual(operation._last[2], "OFF_COMMITTED")
                    self.assertEqual(operation._journal._read_floor(), 5)
                    self.assertEqual(operation._last[3]["event_id"], event)
                    self.assertEqual(operation._last[3]["control_epoch"], 5)
                    drain = json.loads(consumer.actions[-1]._artifact_bytes)
                    self.assertEqual((drain["expected_epoch"], drain["resulting_epoch"]), (3, 4))
                    self.assertEqual(drain["intent_sha256"], operation.intent_sha256)
                    self.assertEqual(operation._last[3]["artifact_sha256"], digest(consumer.actions[-1]._artifact_bytes))
                    self.assertEqual(json.loads(ops.journal._read_named(operation._fd, "00000003.json",
                        0o400, ops.MAX_RESULT))["artifact_sha256"], digest(consumer.summary))
                    self.assertEqual(ops.commit_off(handle), event)
                    self.assertEqual(submitted, ["reconcile", "off"])
                    handle.revalidate()

    def test_lost_stdout_and_commit_unknown_use_original_readback_without_resubmission(self):
        for options in ({"lost_stdout": True}, {"unknown": True}):
            with self.subTest(options=options), self.stopped_fixture() as (live, call, operation):
                handle = live.drain(call)
                with self.transport(live, **options) as submitted:
                    event = ops.commit_off(handle)
                    self.assertEqual(ops.commit_off(handle), event)
                    self.assertEqual(submitted, ["reconcile", "off"])
                    self.assertEqual(operation._last[2], "OFF_COMMITTED")

    def test_unknown_readback_preserves_action_and_floor_and_never_resubmits(self):
        with self.stopped_fixture() as (live, call, operation):
            handle = live.drain(call)
            def unknown(_consumer, value):
                value["status"] = "UNKNOWN"
            with self.transport(live, readback_mutator=unknown) as submitted:
                for _ in range(2):
                    with self.assertRaises(ops.PaperOperationsError):
                        ops.commit_off(handle)
                self.assertEqual(submitted, ["reconcile"])
                self.assertEqual(operation._last[2], "STOPPED")
                self.assertEqual(operation._journal._read_floor(), 4)

    def test_mixed_summary_or_unresolved_work_never_seals_reconciled(self):
        for key, value in (("inventory_sha256", "sha256:" + "f" * 64),
                           ("control_epoch", 5), ("live_claims", 1), ("unresolved_commits", 1)):
            with self.subTest(key=key), self.stopped_fixture() as (live, call, operation):
                handle = live.drain(call)
                def mutate(_verb, result):
                    result["data"]["summary"][key] = value
                    result["data"]["summary_sha256"] = digest(canonical(result["data"]["summary"]))
                with self.transport(live, result_mutator=mutate) as submitted:
                    with self.assertRaises(ops.PaperOperationsError):
                        ops.commit_off(handle)
                    self.assertEqual(operation._last[2], "STOPPED")
                    self.assertEqual(submitted, ["reconcile"])

    def test_mixed_exact_readback_never_advances_stage_or_floor(self):
        for key, value in (("kind", "BEGIN"), ("request_sha256", "sha256:" + "f" * 64),
                           ("current_epoch", True), ("current_state", "OFF"),
                           ("event_epoch", 5), ("event_id", str(uuid.uuid4())),
                           ("reset", {}), ("result_sha256", "sha256:" + "a" * 64)):
            with self.subTest(key=key), self.stopped_fixture() as (live, call, operation):
                handle = live.drain(call)
                def mutate(_consumer, record):
                    record[key] = value
                with self.transport(live, readback_mutator=mutate) as submitted:
                    with self.assertRaises(ops.PaperOperationsError):
                        ops.commit_off(handle)
                    self.assertEqual(submitted, ["reconcile"])
                    self.assertEqual(operation._last[2], "STOPPED")
                    self.assertEqual(operation._journal._read_floor(), 4)

    def test_pending_readback_can_resolve_same_action_without_second_submission(self):
        with self.stopped_fixture() as (live, call, operation):
            handle = live.drain(call)
            first = [True]
            def mutate(_consumer, value):
                if first[0]:
                    first[0] = False
                    value["status"] = "UNKNOWN"
            with self.transport(live, readback_mutator=mutate, unknown=True) as submitted:
                with self.assertRaises(ops.PaperOperationsError):
                    ops.commit_off(handle)
                binding = handle._attempt.consumer.action._binding_bytes
                self.assertEqual(ops.commit_off(handle), operation._last[3]["event_id"])
                self.assertEqual(handle._attempt.consumer.actions[0]._binding_bytes, binding)
                self.assertEqual(submitted, ["reconcile", "off"])

    def test_cleanup_uncertainty_keeps_original_action_until_explicit_resolution(self):
        with self.stopped_fixture() as (live, call, operation):
            handle = live.drain(call)
            first = [True]
            close = ops._NativeInvocation.close
            def fail_once(native):
                if native.verb == "reconcile" and first[0]:
                    first[0] = False
                    native._preserve = True
                    raise ops.PaperOperationsError("PAPER_OPS_CONTAINER_CLEANUP_UNKNOWN")
                return close(native)
            with self.transport(live) as submitted, \
                    mock.patch.object(ops._NativeInvocation, "close", fail_once):
                with self.assertRaises(ops.PaperOperationsError):
                    ops.commit_off(handle)
                consumer = handle._attempt.consumer
                original = consumer.call
                os.fstat(original._result_fd)
                binding = consumer.action._binding_bytes
                self.assertEqual(operation._last[2], "STOPPED")
                self.assertEqual(operation._journal._read_floor(), 4)
                self.assertEqual(submitted, ["reconcile"])
                ops.commit_off(handle)
                self.assertEqual(consumer.actions[0]._binding_bytes, binding)
                self.assertIn(original, live._begin_result_fds)
                self.assertEqual(submitted, ["reconcile", "off"])

    def test_off_commit_readback_timeout_retains_reconciled_floor_and_exact_action(self):
        with self.stopped_fixture() as (live, call, operation):
            handle = live.drain(call)
            def timeout(consumer, value):
                if value["kind"] == "TRANSITION":
                    consumer.attempt.deadline.end = 1
            with self.transport(live, readback_mutator=timeout) as submitted:
                with self.assertRaises(ops.PaperOperationsError):
                    ops.commit_off(handle)
                self.assertEqual(operation._last[2], "RECONCILED")
                self.assertEqual(operation._journal._read_floor(), 4)
                binding = handle._attempt.consumer.action._binding_bytes
                with self.assertRaises(ops.PaperOperationsError):
                    ops.commit_off(handle)
                self.assertEqual(binding, handle._attempt.consumer.action._binding_bytes)
                self.assertEqual(submitted, ["reconcile", "off"])

    def test_retained_result_summary_request_and_old_stage_swaps_reject_replay(self):
        for target in ("result", "summary", "request", "stopped"):
            with self.subTest(target=target), self.stopped_fixture() as (live, call, operation):
                handle = live.drain(call)
                with self.transport(live) as submitted:
                    ops.commit_off(handle)
                    consumer = handle._attempt.consumer
                    action = consumer.actions[0]
                    if target == "stopped":
                        parent, name = operation._fd, "00000002.json"
                    else:
                        parent = action.borrow_output_fd()
                        index = json.loads(action._dispatch_bytes)
                        data = (consumer.summary if target == "summary" else
                                index["sql_request_canonical"].encode() if target == "request" else
                                next(raw for _parent, _name, _fd, _identity, raw in consumer.originals
                                     if b'"action":"RECONCILE_OPERATION"' in raw))
                        name = digest(data)[7:] + ".json"
                    original = ops.journal._read_named(parent, name, 0o400, ops.journal.MAX_NATIVE_EXACT)
                    os.unlink(name, dir_fd=parent)
                    ops.journal._immutable(parent, name, original, 0o400)
                    with self.assertRaises((ops.PaperOperationsError, ops.journal.JournalError)):
                        ops.commit_off(handle)
                    self.assertEqual(submitted, ["reconcile", "off"])

    def test_direct_live_maintenance_cannot_consume_arbitrary_action(self):
        with self.stopped_fixture() as (live, call, _operation):
            live.drain(call)
            with self.assertRaises(ops.PaperOperationsError):
                live.invoke("off", call.custody)
            with self.assertRaises(ops.PaperOperationsError):
                live.invoke("reconcile", call.custody)
            self.assertEqual(live._invocations, set())

    def test_new_backend_blocks_consumption_before_action_and_sql(self):
        with self.stopped_fixture() as (live, call, operation):
            handle = live.drain(call)
            with self.transport(live) as submitted, \
                    mock.patch.object(handle._attempt.observer, "database", return_value={"backends": [123]}):
                with self.assertRaises(ops.PaperOperationsError):
                    ops.commit_off(handle)
                self.assertEqual(submitted, [])
                self.assertEqual(ops.journal._next_action_sequence(operation._fd), 1)
                self.assertEqual(operation._last[2], "STOPPED")

    def test_commit_readback_does_not_override_changed_live_context_or_boot(self):
        for changed in ("boot", "manifest", "lock", "closed"):
            with self.subTest(changed=changed), self.stopped_fixture() as (live, call, operation):
                handle = live.drain(call)
                def mutate(consumer, _value):
                    if changed == "boot":
                        consumer.attempt.boot = str(uuid.uuid4())
                    elif changed == "manifest":
                        live.manifest_sha256 = "sha256:" + "f" * 64
                    elif changed == "lock":
                        live.lock_identity = ()
                    else:
                        live._closed = True
                with self.transport(live, readback_mutator=mutate) as submitted:
                    with self.assertRaises(ops.PaperOperationsError):
                        ops.commit_off(handle)
                    self.assertEqual(operation._last[2], "STOPPED")
                    self.assertEqual(operation._journal._read_floor(), 4)
                    self.assertEqual(submitted, ["reconcile"])

    def test_missing_native_original_is_not_reconstructed_from_committed_readback(self):
        for target in ("result", "summary"):
            with self.subTest(target=target), self.stopped_fixture() as (live, call, operation):
                handle = live.drain(call)
                removed = []
                def remove(consumer, record):
                    raw = record["result_canonical"].encode("ascii")
                    data = raw if target == "result" else canonical(json.loads(raw)["data"]["summary"])
                    parent = consumer.action.borrow_output_fd()
                    name = digest(data)[7:] + ".json"
                    os.unlink(name, dir_fd=parent)
                    removed.append((parent, name))
                with self.transport(live, readback_mutator=remove) as submitted:
                    with self.assertRaises((ops.PaperOperationsError, OSError)):
                        ops.commit_off(handle)
                    self.assertEqual(operation._last[2], "STOPPED")
                    self.assertEqual(submitted, ["reconcile"])
                    for parent, name in removed:
                        with self.assertRaises(FileNotFoundError):
                            os.stat(name, dir_fd=parent, follow_symlinks=False)

    def test_off_epoch_overflow_and_foreign_pending_custody_remain_closed(self):
        with self.stopped_fixture() as (live, call, operation):
            handle = live.drain(call)
            attempt = handle._attempt
            old_fence = attempt.fence
            attempt.fence = ops.journal.MAX_I64
            try:
                with self.assertRaisesRegex(ops.PaperOperationsError, "PAPER_OPS_EPOCH"):
                    ops._OffCommit(ops._STOP_TOKEN, handle)
            finally:
                attempt.fence = old_fence
            live._retained_stages.add(object())
            with self.transport(live) as submitted:
                with self.assertRaises(ops.PaperOperationsError):
                    ops.commit_off(handle)
                self.assertEqual(submitted, [])
            live._retained_stages.clear()

    def test_native_result_fds_outlive_handle_until_context_close(self):
        with self.stopped_fixture() as (live, call, _operation):
            handle = live.drain(call)
            with self.transport(live):
                ops.commit_off(handle)
            fds = list(live._begin_result_fds.values())
            self.assertEqual(len(fds), 3)
            owned = [row[2] for row in handle._attempt.consumer.originals]
            handle.close()
            for fd in owned:
                with self.assertRaises(OSError):
                    os.fstat(fd)
            for fd in fds:
                self.assertGreater(os.fstat(fd).st_size, 0)
            live._close_begin_results()
            for fd in fds:
                with self.assertRaises(OSError):
                    os.fstat(fd)
            with self.assertRaises(ops.PaperOperationsError):
                ops.commit_off(handle)

    def test_off_floor_failure_reuses_sealed_stage_without_refresh_or_sql(self):
        with self.stopped_fixture() as (live, call, operation):
            handle = live.drain(call)
            advance = ops.journal.Journal.advance_floor
            first = [True]
            def fail_before_write(current, value):
                if value == 5 and first[0]:
                    first[0] = False
                    raise OSError("component floor publication fault")
                return advance(current, value)
            with self.transport(live) as submitted, \
                    mock.patch.object(ops.journal.Journal, "advance_floor", fail_before_write):
                with self.assertRaises(OSError):
                    ops.commit_off(handle)
                sealed = ops.journal._read_named(operation._fd, "00000004.json", 0o400, ops.MAX_RESULT)
                self.assertEqual(operation._last[2], "OFF_COMMITTED")
                self.assertEqual(operation._journal._read_floor(), 4)
                event = ops.commit_off(handle)
                self.assertEqual(event, json.loads(sealed)["event_id"])
                self.assertEqual(ops.journal._read_named(operation._fd, "00000004.json", 0o400,
                                                       ops.MAX_RESULT), sealed)
                self.assertEqual(operation._journal._read_floor(), 5)
                self.assertEqual(submitted, ["reconcile", "off"])

    def test_off_observer_queries_original_fence_separately_from_current_epoch(self):
        observer, value = PaperStopProofTests.database_observer(self)
        observer.attempt.consumer = types.SimpleNamespace(database_state=lambda: (5, "OFF"))
        value["fence"].update(epoch=5, state="OFF")
        with mock.patch.object(ops, "_process_record", return_value={"cgroup": "/docker/" + "d" * 64}):
            observer.command.return_value = json.dumps(value).encode() + b"\n"
            observer.database("observe")
            payload = observer.command.call_args.args[1]
            self.assertIn(b"AND o.fence_epoch=4 ", payload)
            self.assertNotIn(b"o.fence_epoch=c.control_epoch", payload)
            self.assertIn(b"'epoch',c.control_epoch,'state',c.state", payload)
            self.assertIn(b"'event',o.fence_event_id", payload)
            self.assertNotIn(b"application_name", payload)

    def test_live_reconcile_preparation_mounts_actual_action_operation_and_fence_floor(self):
        with self.stopped_fixture() as (live, call, operation):
            handle = live.drain(call)
            consumer = ops._OffCommit(ops._STOP_TOKEN, handle)
            handle._attempt.consumer = consumer
            stage_type = ops._Stage
            created = []
            def stage(prefix):
                value = stage_type(prefix, root=str(self.base))
                created.append(value)
                return value
            live.install_root = str(self.install)
            live.tree = operation._journal._tree
            live.database = {"schema": "owner-beta-paper-ops-database-v1", "host": "postgres",
                             "port": 5432, "database": "lagrange"}
            live.values = {"LAGRANGE_RUNTIME_SECRET_DIR": "/component-private-secret"}
            live.verified.stage_native_inputs = mock.Mock()
            def prepare_only(native):
                try:
                    native._prepare()
                except ops.PaperOperationsError as error:
                    raise AssertionError(error.code) from error
            with mock.patch.object(ops, "_Stage", side_effect=stage), \
                    mock.patch.object(ops, "_dir", fixture_operations_dir), \
                    mock.patch.object(ops, "_read_migration_secret", return_value=(b"component-only", ())), \
                    mock.patch.object(ops._NativeInvocation, "_run", prepare_only):
                try:
                    consumer.prepare("reconcile", handle._attempt.inventory_bytes)
                    native = consumer.call
                    self.assertEqual(native.verb, "reconcile")
                    self.assertEqual(dict((name, data) for _fd, name, _id, data in
                        native._paper_state_files)["epoch-floor"], b"4\n")
                    self.assertIn((str(self.install) + "/.owner-beta-paper/operations/" + operation.generation,
                                   ops.OPERATION_ALIAS, True), native._mounts)
                    self.assertEqual(ops.journal._read_named(native._persistent_stage.fd,
                        "action-binding.json", 0o400, ops.MAX_RESULT), consumer.action._binding_bytes)
                    self.assertEqual(ops.journal._read_named(native._persistent_stage.fd,
                        "evidence.json", 0o400, ops.MAX_RESULT), handle._attempt.inventory_bytes)
                    live.verified.stage_native_inputs.assert_called_once_with(native._persistent_stage.fd)
                finally:
                    if consumer.call is not None:
                        # No process/SQL was created in this staging-only component.
                        consumer.call._imported = True
                        consumer.call.close()
                    for value in reversed(created):
                        if not value.closed:
                            value.close(remove=True)

    def test_stage_allows_owned_mountpoint_creation_only_before_snapshot(self):
        with mock.patch.object(ops, "_dir", fixture_operations_dir), \
                mock.patch.object(ops, "_file", fixture_operations_file):
            stage = ops._Stage("component-stage-", root=str(self.base))
            try:
                links = os.fstat(stage.fd).st_nlink
                os.mkdir("operation", 0o700, dir_fd=stage.fd)
                self.assertGreaterEqual(os.fstat(stage.fd).st_nlink, links)
                stage.revalidate()
                stage.snapshot()
                os.mkdir("unlisted", 0o700, dir_fd=stage.fd)
                try:
                    with self.assertRaises(ops.PaperOperationsError):
                        stage.revalidate()
                finally:
                    os.rmdir("unlisted", dir_fd=stage.fd)
                stage.revalidate()
                os.rename(stage.name, stage.name + "-replaced", src_dir_fd=stage.tree.fd,
                          dst_dir_fd=stage.tree.fd)
                os.mkdir(stage.name, 0o700, dir_fd=stage.tree.fd)
                try:
                    with self.assertRaises(ops.PaperOperationsError):
                        stage.revalidate()
                finally:
                    os.rmdir(stage.name, dir_fd=stage.tree.fd)
                    os.rename(stage.name + "-replaced", stage.name, src_dir_fd=stage.tree.fd,
                              dst_dir_fd=stage.tree.fd)
            finally:
                stage.close(remove=True)

    def test_commit_off_rejects_reports_and_foreign_or_closed_handles(self):
        for value in (None, {}, {"stage": "STOPPED"}, object()):
            with self.subTest(value=type(value)), self.assertRaises(ops.PaperOperationsError):
                ops.commit_off(value)
        with self.stopped_fixture() as (live, call, _operation):
            handle = live.drain(call)
            forged = object.__new__(ops.DrainedHandle)
            forged._attempt, forged._closed = handle._attempt, False
            with self.assertRaises(ops.PaperOperationsError):
                ops.commit_off(forged)
            handle.close()
            with self.assertRaises(ops.PaperOperationsError):
                ops.commit_off(handle)

    def test_commit_off_deadline_does_not_publish_an_action_or_reset_attempt(self):
        with self.stopped_fixture() as (live, call, operation):
            handle = live.drain(call)
            end = handle._attempt.deadline.end = 1
            with self.assertRaises(ops.PaperOperationsError):
                ops.commit_off(handle)
            self.assertEqual(handle._attempt.deadline.end, end)
            self.assertEqual(operation._last[2], "STOPPED")
            self.assertEqual(ops.journal._next_action_sequence(operation._fd), 1)



class PaperArchiveCustodyTests(unittest.TestCase):
    setUp = PaperStopProofTests.setUp
    _cleanup_fixture = PaperStopProofTests._cleanup_fixture
    begin_action = PaperStopProofTests.begin_action
    _fenced_fixture = PaperStopProofTests._fenced_fixture
    isolated_install = PaperStopProofTests.isolated_install
    stopped_fixture = PaperStopProofTests.stopped_fixture
    transport = PaperCommitOffTests.transport

    def archive_module(self):
        return ops._load_local("owner_beta_paper_backup", "owner-beta-paper-backup.py")

    def test_archive_active_invocation_exception_is_one_owned_readback_only(self):
        with self.stopped_fixture("BACKUP") as (live, begin, operation), self.transport(live):
            handle = live.drain(begin)
            ops.commit_off(handle)
            owner = self.archive_owner(live, handle)
            module = self.archive_module()
            package = object.__new__(module._ArchivePackage)
            package.owner, package.closed = owner, False
            action = handle._attempt.consumer.actions[-1]
            custody = operation._journal.open_custody(operation.generation, action.sequence)
            package.custody = custody
            call = ops._NativeInvocation(ops._INVOCATION_TOKEN, live, "readback", custody)
            call._archive_owner = package
            package.call = call
            owner.package = package
            module._PACKAGES.add(package)
            live._invocations.add(call)
            try:
                self.assertIs(owner._custody.native_invocation(), call)
                handle._attempt._check_custody(stopped=True)
                call.verb = "off"
                with self.assertRaisesRegex(ops.PaperOperationsError, "PAPER_OPS_PRIVATE"):
                    handle._attempt._check_custody(stopped=True)
                call.verb = "readback"
                extra = object()
                live._invocations.add(extra)
                with self.assertRaisesRegex(ops.PaperOperationsError, "PAPER_OPS_BEGIN_UNSETTLED"):
                    handle._attempt._check_custody(stopped=True)
                live._invocations.remove(extra)
                module._PACKAGES.remove(package)
                with self.assertRaisesRegex(ops.PaperOperationsError, "PAPER_OPS_PRIVATE"):
                    owner._custody.native_invocation()
            finally:
                live._invocations.discard(call)
                module._PACKAGES.discard(package)
                owner.package = None
                custody.close()
                call._closed = True  # no process/SQL exists in this component vector
                owner.close()

    def archive_owner(self, live, handle):
        module = self.archive_module()
        if live not in module._SPACES:
            # Component-only reservation; no exporter, Docker or root I/O.
            space = object.__new__(module._CaptureSpace)
            space.closed, space.owner = False, None
            space.tree = space.root = space.root_tree = space.selection = None
            space.deadline = ops._ArchiveDeadline()
            module._SPACES[live] = space
            self.addCleanup(module._SPACES.pop, live, None)
        return module.open_snapshot(live, handle, 1024)

    def test_archive_clock_is_distinct_and_original_stop_clock_still_expires(self):
        with self.stopped_fixture("BACKUP") as (live, call, operation), self.transport(live):
            handle = live.drain(call)
            ops.commit_off(handle)
            attempt = handle._attempt
            attempt.observer.deadline = attempt.deadline  # component observer retains the real clock
            before, observer_clock = attempt.deadline.end, attempt.observer.deadline
            owner = self.archive_owner(live, handle)
            try:
                self.assertIs(attempt.observer.deadline, observer_clock)
                self.assertEqual(attempt.deadline.end, before)
                self.assertIsNot(owner._custody.deadline, attempt.deadline)
                with mock.patch.object(ops._ArchiveObserver, "terminal_observation"), \
                        mock.patch.object(ops._ArchiveObserver, "database", return_value={"backends": []}), \
                        mock.patch.object(ops.time, "monotonic_ns", return_value=before+1):
                    owner.revalidate()
                    with self.assertRaisesRegex(ops.PaperOperationsError, "PAPER_OPS_STOP_TIMEOUT"):
                        attempt.deadline.remaining()
                self.assertEqual(attempt.deadline.end, before)
                with self.assertRaisesRegex(ops.PaperOperationsError, "PAPER_OPS_ARCHIVE_REPLAY"):
                    self.archive_owner(live, handle)
            finally:
                owner.close()
            with self.assertRaisesRegex(ops.PaperOperationsError, "PAPER_OPS_ARCHIVE_REPLAY"):
                self.archive_owner(live, handle)

    def test_archive_requires_actual_settled_backup_off_not_stop_or_stopped(self):
        for purpose, commit in (("STOP", True), ("BACKUP", False)):
            with self.subTest(purpose=purpose), self.stopped_fixture(purpose) as (live, call, operation), self.transport(live):
                handle = live.drain(call)
                if commit:
                    ops.commit_off(handle)
                with self.assertRaisesRegex(ops.PaperOperationsError, "PAPER_OPS_ARCHIVE_OFF"):
                    self.archive_owner(live, handle)

    def test_archive_rechecks_originals_boot_and_live_registration_after_stop_expiry(self):
        for drift in ("boot", "result", "registration"):
            with self.subTest(drift=drift), self.stopped_fixture("BACKUP") as (live, call, operation), self.transport(live):
                handle = live.drain(call)
                ops.commit_off(handle)
                owner = self.archive_owner(live, handle)
                try:
                    with mock.patch.object(ops._ArchiveObserver, "terminal_observation"), \
                            mock.patch.object(ops._ArchiveObserver, "database", return_value={"backends": []}):
                        owner.revalidate()
                        if drift == "registration":
                            actual = ops._STOP_ATTEMPTS[live].pop(call)
                            try:
                                with self.assertRaises(ops.PaperOperationsError):
                                    owner.revalidate()
                            finally:
                                ops._STOP_ATTEMPTS[live][call] = actual
                        else:
                            target = (mock.patch.object(ops, "_current_boot_id", return_value=str(uuid.uuid4()))
                                      if drift == "boot" else mock.patch.object(ops._OffCommit, "check",
                                      side_effect=ops.PaperOperationsError("PAPER_OPS_RESULT_CHANGED")))
                            with target, self.assertRaises(ops.PaperOperationsError):
                                owner.revalidate()
                    self.assertTrue(owner._poisoned)
                    self.assertTrue(owner._custody.poisoned or drift == "registration")
                finally:
                    owner.close()

    def test_archive_unknown_clients_prevent_releasing_original_locks(self):
        with self.stopped_fixture("BACKUP") as (live, call, operation), self.transport(live):
            handle = live.drain(call)
            ops.commit_off(handle)
            owner = self.archive_owner(live, handle)
            owner.clients.append(types.SimpleNamespace(absent=False))
            try:
                with self.assertRaisesRegex(ops.PaperOperationsError, "PAPER_OPS_ARCHIVE_CLEANUP_UNKNOWN"):
                    handle.close()
                self.assertFalse(handle._attempt.closed)
            finally:
                owner.clients.clear()
                owner.close()


class HostUnitEnumerationTests(unittest.TestCase):
    """Minimal stub for _HostStopObserver.units_now: no reusable fake exists, so these
    tests call the unbound method on a stub providing systemctl, roots, unit_files,
    collector_locks and deadline-free state."""
    PROPS = ("Id", "FragmentPath", "DropInPaths", "ExecStart", "EnvironmentFiles",
             "ReadWritePaths", "WorkingDirectory", "ControlGroup", "MainPID", "InvocationID",
             "ActiveState", "SubState", "UnitFileState", "Triggers", "TriggeredBy")

    @staticmethod
    def block(unit, *, drop=(), extra=(), **override):
        values = {"Id": unit, "FragmentPath": "/nowhere/" + unit, "ActiveState": "inactive",
                  "SubState": "dead", "UnitFileState": "static", "MainPID": "0",
                  "ControlGroup": ""}
        values.update(override)
        for key in drop:
            values.pop(key, None)
        return "\n".join([f"{k}={v}" for k, v in values.items()] + list(extra))

    def run_units(self, files, running=(), blocks=(), roots=("/nonexistent-root",)):
        shown = []

        def systemctl(*args):
            if args[0] == "list-unit-files":
                return "".join(f"{name} static -\n" for name in files)
            if args[0] == "list-units":
                return "".join(f"{name} loaded inactive dead desc\n" for name in running)
            shown.append(args[3:])
            return "\n\n".join(blocks)

        stub = types.SimpleNamespace(systemctl=systemctl, roots=tuple(roots), unit_files={},
                                     collector_locks=set())
        return ops._HostStopObserver.units_now(stub), shown, stub

    def code(self, *args, **kwargs):
        with self.assertRaises(ops.PaperOperationsError) as caught:
            self.run_units(*args, **kwargs)
        return str(caught.exception.args[0]) if caught.exception.args else str(caught.exception)

    def test_template_unit_file_is_not_shown(self):
        result, shown, _ = self.run_units(["getty@.service", "plain.service"],
                                          blocks=[self.block("plain.service")])
        self.assertEqual((result, shown), ({}, [("plain.service",)]))

    def test_template_named_like_installation_is_ambiguous(self):
        for name in ("lagrange-x@.service", "paper-runner@.service"):
            with self.subTest(name=name):
                self.assertIn("PAPER_OPS_LAUNCH_AMBIGUOUS", self.code([name]))

    def test_instance_name_is_shown(self):
        _, shown, _ = self.run_units(["getty@.service"], running=["getty@tty1.service"],
                                     blocks=[self.block("getty@tty1.service")])
        self.assertEqual(shown, [("getty@tty1.service",)])

    def test_two_exec_start_lines_accepted_and_both_recorded(self):
        with tempfile.TemporaryDirectory() as tmp:
            fragment = pathlib.Path(tmp, "paper.service")
            fragment.write_bytes(b"[Service]\n")
            fragment.chmod(0o644)

            class Tree:
                def __init__(self, path):
                    self.fd = os.open(path, os.O_RDONLY)
                def revalidate(self):
                    pass
                def close(self):
                    os.close(self.fd)

            def open_file(parent, name, mode=0o400, **_kw):
                fd = os.open(name, os.O_RDONLY, dir_fd=parent)
                return fd, fragment.read_bytes(), ops._identity(os.fstat(fd))

            def run(exec_lines):
                block = self.block("paper.service", FragmentPath=str(fragment),
                                   drop=("ExecStart",), extra=exec_lines)
                with mock.patch.object(ops.daily.install, "read_root_file",
                                       return_value=b"[Service]\n"), \
                        mock.patch.object(ops.journal, "_Tree", Tree), \
                        mock.patch.object(ops, "_open_file", open_file):
                    return self.run_units(["paper.service"], blocks=[block], roots=(tmp,))
            both, _, stub = run(["ExecStart=/bin/true",
                                 "ExecStart=/x/owner-beta-paper-evidence.py run --binding /b/c"])
            first, _, _ = run(["ExecStart=/bin/true"])
            for tree, _fd, _i, _d, _m in stub.unit_files.values():
                tree.close()
        # The source-publisher class and binding come only from the second line, so
        # they prove the lines were joined; the identity digest differs from one line.
        self.assertEqual(both["paper.service"]["classes"],
                         ["SOURCE_PUBLISHER", "SOURCE_CHILD", "RAW_CURATED"])
        self.assertEqual(both["paper.service"]["source_binding"], "/b/c")
        self.assertNotEqual(both["paper.service"]["identity"], first["paper.service"]["identity"])

    def test_repeated_other_key_is_unknown(self):
        for key in ("Id=other.service", "MainPID=1"):
            with self.subTest(key=key):
                self.assertIn("PAPER_OPS_LAUNCH_UNKNOWN",
                              self.code(["a.service"], blocks=[self.block("a.service", extra=[key])]))

    def test_service_without_exec_start_is_accepted(self):
        result, shown, _ = self.run_units(["a.service"], blocks=[self.block("a.service")])
        self.assertEqual((result, len(shown)), ({}, 1))

    def test_service_without_main_pid_is_unknown(self):
        block = self.block("a.service", drop=("MainPID",))
        self.assertIn("PAPER_OPS_LAUNCH_UNKNOWN", self.code(["a.service"], blocks=[block]))


class LaunchMemberTests(unittest.TestCase):
    """_HostStopObserver._launch_member on real files in a temporary directory."""
    ROOT = "/nonexistent-lagrange-launch-member-root"

    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.tmp = pathlib.Path(tmp.name)
        self.stub = types.SimpleNamespace(roots=(self.ROOT,))

    def member(self, path):
        return ops._HostStopObserver._launch_member(self.stub, path)

    def code(self, path):
        with self.assertRaises(ops.PaperOperationsError) as caught:
            self.member(path)
        return str(caught.exception.args[0]) if caught.exception.args else str(caught.exception)

    def regular(self, name="target", data=b"#!/bin/sh\ntrue\n"):
        path = self.tmp / name
        path.write_bytes(data)
        return path

    def link(self, target, name="entry"):
        path = self.tmp / name
        path.symlink_to(target)
        return path

    def test_regular_file_has_four_keys(self):
        result = self.member(self.regular())
        self.assertEqual(sorted(result), ["bytes", "identity", "mtime_ns", "name"])

    def test_link_to_regular_file_adds_target(self):
        target = self.regular(data=b"x" * 10)
        result = self.member(self.link(target))
        self.assertEqual(sorted(result), ["bytes", "identity", "mtime_ns", "name", "target"])
        self.assertEqual(sorted(result["target"]), ["bytes", "identity", "mtime_ns"])
        self.assertEqual(result["target"]["bytes"], 10)

    def test_link_to_missing_target_is_inert(self):
        result = self.member(self.link(self.tmp / "missing"))
        self.assertIsNone(result["target"])

    def test_link_to_directory_is_unknown(self):
        directory = self.tmp / "dir"
        directory.mkdir()
        self.assertEqual(self.code(self.link(directory)), "PAPER_OPS_LAUNCH_UNKNOWN")

    def test_link_to_oversized_file_is_unknown(self):
        target = self.regular(data=b"x" * 65_537)
        self.assertEqual(self.code(self.link(target)), "PAPER_OPS_LAUNCH_UNKNOWN")

    def test_link_to_multiply_linked_file_is_unknown(self):
        target = self.regular()
        os.link(target, self.tmp / "second")
        self.assertEqual(self.code(self.link(target)), "PAPER_OPS_LAUNCH_UNKNOWN")

    def test_link_text_naming_installation_is_unlisted(self):
        entry = self.link(self.ROOT + "/bin/run")
        self.assertEqual(self.code(entry), "PAPER_OPS_UNLISTED_LAUNCH")

    def test_link_to_file_mentioning_owner_beta_is_unlisted(self):
        target = self.regular(data=b"exec owner-beta-run\n")
        self.assertEqual(self.code(self.link(target)), "PAPER_OPS_UNLISTED_LAUNCH")

    def test_regular_file_mentioning_root_is_unlisted(self):
        path = self.regular(name="job", data=("run " + self.ROOT + "/x\n").encode())
        self.assertEqual(self.code(path), "PAPER_OPS_UNLISTED_LAUNCH")

    def test_regular_file_with_two_links_is_unknown(self):
        path = self.regular(name="job")
        os.link(path, self.tmp / "job2")
        self.assertEqual(self.code(path), "PAPER_OPS_LAUNCH_UNKNOWN")

    def test_link_to_fifo_is_unknown_and_does_not_block(self):
        fifo = self.tmp / "fifo"
        os.mkfifo(fifo)
        entry = self.link(fifo)
        outcome = []

        def run():
            try:
                self.member(entry)
                outcome.append("returned")
            except ops.PaperOperationsError as exc:
                outcome.append(str(exc.args[0]) if exc.args else str(exc))

        thread = threading.Thread(target=run, daemon=True)
        thread.start()
        thread.join(5)
        self.assertFalse(thread.is_alive(), "_launch_member blocked on a FIFO")
        self.assertEqual(outcome, ["PAPER_OPS_LAUNCH_UNKNOWN"])


class LifecycleRoutingTests(unittest.TestCase):
    def test_operator_entry_routes_through_owned_lifecycle_without_widening_invoke(self):
        lifecycle=mock.Mock();lifecycle.main.return_value=3
        with mock.patch.object(ops,"_load_local",return_value=lifecycle) as load:
            self.assertEqual(ops.main(["enable","--install-root","/installed","--caller-key","K"]),3)
        load.assert_called_once_with("owner_beta_paper_lifecycle","owner-beta-paper-lifecycle.py")
        lifecycle.main.assert_called_once_with(["enable","--install-root","/installed","--caller-key","K"])


if __name__ == "__main__":
    unittest.main()
