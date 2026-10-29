#!/usr/bin/env python3
"""Synthetic stdlib tests for the owner-beta simulation process adapter."""
from __future__ import annotations

import contextlib
import copy
import hashlib
import importlib.util
import io
import json
import runpy
import subprocess
import sys
import traceback
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "scripts/ops/lib/owner-beta-simulation-runtime.py"
SPEC = importlib.util.spec_from_file_location("owner_beta_simulation_runtime_test", MODULE_PATH)
runtime = importlib.util.module_from_spec(SPEC)
assert SPEC is not None and SPEC.loader is not None
sys.modules[SPEC.name] = runtime
SPEC.loader.exec_module(runtime)
release = runtime._release_module()

COMMIT = "a1" * 20
PROJECT = "simulation-test"
CONTAINER_ID = "a" * 64
OTHER_CONTAINER_ID = "b" * 64
IMAGE_ID = "sha256:" + "1" * 64
RECEIPT_SHA256 = "sha256:" + "2" * 64
RELEASE = "/srv/lagrange/releases/" + COMMIT
HISTORICAL_ROOT = "/srv/lagrange/inputs/owner-beta"
SECRET_SENTINEL = "synthetic-error-secret-never-expose"
EXPECTED_PS_ARGS = (
    "ps", "--all", "--no-trunc", "--filter",
    "label=com.docker.compose.service=owner-beta-simulation-runner",
    "--format", "{{.ID}}",
)


def config_value(*, stop_timeout_seconds=17):
    return {
        "schema": release.SIMULATION_SCHEMA,
        "code_commit": COMMIT,
        "manifest_format": release.MANIFEST_V3_FORMAT,
        "manifest_sha256": "sha256:" + "3" * 64,
        "image_id": IMAGE_ID,
        "receipt_sha256": RECEIPT_SHA256,
        "binary_sha256": "sha256:" + "4" * 64,
        "source_input_sha256": "sha256:" + "5" * 64,
        "recipe_sha256": "sha256:" + "6" * 64,
        "build_origin": release.BUILD_ORIGIN,
        "package": release.PACKAGE,
        "bin": release.BIN,
        "target": release.TARGET,
        "profile": release.PROFILE,
        "toolchain": release.TOOLCHAIN,
        "owner": "11111111-2222-4333-8444-555555555555",
        "entitlement_reference": "synthetic test entitlement",
        "entitlement_sha256": "sha256:" + "7" * 64,
        "historical_root": HISTORICAL_ROOT,
        "reader_uid": 10001,
        "reader_gid": 10001,
        "paths": dict(release._PATHS),
        "limits": {
            "memory_bytes": 1,
            "nano_cpus": 1,
            "health_tmpfs_bytes": 1,
            "pids_limit": 1,
            "startup_timeout_seconds": 1,
            "health_max_age_seconds": 1,
            "stop_timeout_seconds": stop_timeout_seconds,
        },
    }


def config_bytes(value=None, *, pretty=False):
    body = config_value() if value is None else value
    if pretty:
        return (json.dumps(body, indent=2) + "\n").encode("utf-8")
    return (json.dumps(body, separators=(",", ":")) + "\n").encode("utf-8")


def make_binding(*, project=PROJECT, release_path=RELEASE, commit=COMMIT, data=None):
    return runtime.Binding(project, release_path, commit, config_bytes() if data is None else data)


def make_image(binding, *, revision=None, identity=None, labels=None):
    values = {runtime.REVISION_LABEL: binding.commit if revision is None else revision}
    if labels is not None:
        values.update(labels)
    return {
        "Id": binding.image_id if identity is None else identity,
        "Config": {"Labels": values},
    }


def make_container(
    binding,
    *,
    identity=CONTAINER_ID,
    project=None,
    state="running",
    policy="unless-stopped",
    created="2026-10-27T00:00:00.000000000Z",
    started="2026-10-27T00:01:00.000000000Z",
    restart_count=0,
    exit_code=0,
    changes=None,
):
    state_values = {
        "Running": state == "running",
        "Restarting": False,
        "Paused": False,
        "OOMKilled": False,
        "ExitCode": exit_code,
        "StartedAt": started,
        "Error": "",
        "Status": state,
    }
    labels = {
        runtime.REVISION_LABEL: binding.commit,
        runtime.PROJECT_LABEL: binding.project if project is None else project,
        runtime.SERVICE_LABEL: runtime.SERVICE,
        runtime.ONEOFF_LABEL: "False",
        runtime.CONFIG_LABEL: binding.config_sha256,
        runtime.RECEIPT_LABEL: binding.receipt_sha256,
    }
    config = {
        "Labels": labels,
        "User": runtime.USER,
        "Entrypoint": list(runtime.ENTRYPOINT),
        "Cmd": None,
    }
    host = {
        "AutoRemove": False,
        "Init": True,
        "ReadonlyRootfs": True,
        "Privileged": False,
        "RestartPolicy": {"Name": policy, "MaximumRetryCount": 0},
    }
    mounts = [
        {
            "Type": "bind",
            "Source": binding.release + "/.owner-beta-simulation/config.json",
            "Destination": runtime.CONFIG_DESTINATION,
            "RW": False,
            "Propagation": "rprivate",
        },
        {
            "Type": "bind",
            "Source": binding.historical_root,
            "Destination": runtime.INPUT_DESTINATION,
            "RW": False,
            "Propagation": "rprivate",
        },
    ]
    value = {
        "Id": identity,
        "Created": created,
        "Image": binding.image_id,
        "Config": config,
        "HostConfig": host,
        "Mounts": mounts,
        "State": state_values,
        "RestartCount": restart_count,
    }
    if changes:
        for path, replacement in changes.items():
            if path.startswith("Config.Labels."):
                value["Config"]["Labels"][path[len("Config.Labels."):]] = replacement
                continue
            current = value
            parts = path.split(".")
            for part in parts[:-1]:
                current = current[int(part)] if type(current) is list else current[part]
            last = int(parts[-1]) if type(current) is list else parts[-1]
            current[last] = replacement
    return value


class FakeDocker:
    """Scripted transport; it has no subprocess, filesystem, or DB behavior."""

    def __init__(self, *, id_rows, images=(), containers=(), mutations=()):
        self.id_rows = list(id_rows)
        self.images = list(images)
        self.containers = list(containers)
        self.mutations = list(mutations)
        self.calls = []
        self.after_call = None

    @staticmethod
    def _take(items, kind):
        if not items:
            raise AssertionError("unexpected " + kind)
        return items.pop(0)

    def service_ids(self, timeout):
        self.calls.append(("ps", EXPECTED_PS_ARGS, timeout))
        value = self._take(self.id_rows, "service listing")
        if self.after_call is not None:
            self.after_call("ps", None)
        return copy.deepcopy(value)

    def object(self, kind, identity, timeout):
        self.calls.append(("inspect", kind, identity, timeout))
        if kind == "image":
            value = self._take(self.images, "image inspect")
        elif kind == "container":
            value = self._take(self.containers, "container inspect")
        else:
            raise AssertionError("unexpected inspect kind")
        if self.after_call is not None:
            self.after_call("inspect", kind)
        return copy.deepcopy(value)

    def call(self, args, timeout):
        if tuple(args) not in {
            ("update", "--restart=no", CONTAINER_ID),
            ("kill", "--signal=TERM", CONTAINER_ID),
            ("wait", CONTAINER_ID),
        }:
            raise AssertionError("unexpected command")
        self.calls.append(("call", tuple(args), timeout))
        value = self._take(self.mutations, "mutation")
        if self.after_call is not None:
            self.after_call("call", tuple(args))
        if isinstance(value, BaseException):
            raise value
        return value

    def operations(self):
        result = []
        for call in self.calls:
            if call[0] == "ps":
                result.append(("ps",))
            elif call[0] == "inspect":
                result.append((call[1], call[2]))
            else:
                result.append(call[1])
        return result


def make_capture_docker(binding, *, first=None, second=None, id_rows=None):
    first = make_container(binding) if first is None else first
    second = copy.deepcopy(first) if second is None else second
    rows = [[CONTAINER_ID], [CONTAINER_ID]] if id_rows is None else id_rows
    return FakeDocker(
        id_rows=rows,
        images=[make_image(binding), make_image(binding)],
        containers=[first, second],
    )


def capture_snapshot(binding, *, state="running", policy="unless-stopped"):
    value = make_container(binding, state=state, policy=policy)
    fake = make_capture_docker(binding, first=value, second=value)
    snapshot = runtime.capture(fake, binding)
    return snapshot


@contextlib.contextmanager
def no_runtime_io(testcase):
    with mock.patch.object(runtime.subprocess, "run", side_effect=AssertionError("subprocess")) as run_mock, \
            mock.patch("builtins.open", side_effect=AssertionError("filesystem")) as open_mock, \
            mock.patch.object(runtime.os, "stat", side_effect=AssertionError("filesystem")) as stat_mock, \
            mock.patch.object(runtime.os, "open", side_effect=AssertionError("filesystem")) as os_open_mock:
        try:
            yield run_mock, open_mock, stat_mock, os_open_mock
        finally:
            run_mock.assert_not_called()
            open_mock.assert_not_called()
            stat_mock.assert_not_called()
            os_open_mock.assert_not_called()


class SimulationRuntimeContractTests(unittest.TestCase):
    def setUp(self):
        self.subprocess_guard = mock.patch.object(
            runtime.subprocess, "run", side_effect=AssertionError("unexpected subprocess"))
        self.subprocess_guard.start()
        self.addCleanup(self.subprocess_guard.stop)

    def assert_runtime_error(self, function, *, code_prefix="SIMULATION_RUNTIME_"):
        with self.assertRaises(runtime.RuntimeErrorCode) as raised:
            function()
        error = raised.exception
        self.assertTrue(error.code.startswith(code_prefix))
        self.assertEqual(str(error), error.code)
        self.assertIsNone(error.__cause__)
        self.assertTrue(error.__suppress_context__)
        rendered = "".join(traceback.TracebackException.from_exception(
            error, lookup_lines=False).format())
        self.assertNotIn(SECRET_SENTINEL, rendered)
        self.assertNotIn(RELEASE, rendered)
        return error

    def test_binding_validates_inputs_and_hashes_exact_config_bytes(self):
        compact = config_bytes()
        pretty = config_bytes(pretty=True)
        original_hash = hashlib.sha256(compact).hexdigest()
        binding = make_binding(data=compact)
        formatted = make_binding(data=pretty)
        self.assertEqual(binding.image_id, IMAGE_ID)
        self.assertEqual(binding.receipt_sha256, RECEIPT_SHA256)
        self.assertEqual(binding.stop_timeout_seconds, 17)
        self.assertEqual(binding.historical_root, HISTORICAL_ROOT)
        self.assertEqual(binding.config_sha256, "sha256:" + original_hash)
        self.assertNotEqual(binding.config_sha256, formatted.config_sha256)
        self.assertNotIn(compact.decode(), repr(binding))
        self.assertEqual(compact, config_bytes())

    def test_binding_accepts_arbitrary_canonical_synthetic_historical_root(self):
        value = config_value()
        value["historical_root"] = "/synthetic/arbitrary/owner-input-tree"
        binding = make_binding(data=config_bytes(value))
        self.assertEqual(binding.historical_root, "/synthetic/arbitrary/owner-input-tree")

    def test_binding_rejects_noncanonical_project_commit_release_and_mutable_config(self):
        bad_projects = ("", "UPPER", "-leading", "contains.dot", "é", "a" * 64)
        for project in bad_projects:
            with self.subTest(project=project):
                self.assert_runtime_error(lambda project=project: make_binding(project=project))
        bad_commits = ("0" * 40, "A" * 40, "a" * 39, "a" * 41, 17)
        for commit in bad_commits:
            with self.subTest(commit=commit):
                self.assert_runtime_error(lambda commit=commit: make_binding(
                    release_path="/srv/releases/" + str(commit), commit=commit))
        bad_paths = (
            "/", RELEASE + "/", "/srv//releases/" + COMMIT,
            "/srv/releases/./" + COMMIT, "/srv/releases/../" + COMMIT,
            "/srv/releases/other-commit", "/srv/releases/bad\x7f" + COMMIT,
        )
        for path in bad_paths:
            with self.subTest(path=path):
                self.assert_runtime_error(lambda path=path: make_binding(release_path=path))
        self.assert_runtime_error(lambda: runtime.Binding(PROJECT, RELEASE, COMMIT, bytearray(config_bytes())))

    def test_binding_maps_c1_config_rejection_and_bool_timeout_without_leaking(self):
        invalid = config_value()
        invalid["limits"] = dict(invalid["limits"], stop_timeout_seconds=True)
        self.assert_runtime_error(lambda: make_binding(data=config_bytes(invalid)))
        invalid = config_value()
        invalid["historical_root"] = "/secret/path/" + SECRET_SENTINEL + "/"
        error = self.assert_runtime_error(lambda: make_binding(data=config_bytes(invalid)))
        self.assertNotIn(SECRET_SENTINEL, str(error))
        self.assert_runtime_error(lambda: make_binding(data=b"{\"secret\":\"" + SECRET_SENTINEL.encode() + b"\"}"))

    def test_library_import_and_direct_script_execution_are_no_work(self):
        output = io.StringIO()
        errors = io.StringIO()
        with mock.patch.object(subprocess, "run", side_effect=AssertionError("subprocess")) as run_mock, \
                contextlib.redirect_stdout(output), contextlib.redirect_stderr(errors):
            name = "owner_beta_simulation_runtime_no_work"
            spec = importlib.util.spec_from_file_location(name, MODULE_PATH)
            self.assertIsNotNone(spec)
            module = importlib.util.module_from_spec(spec)
            sys.modules[name] = module
            spec.loader.exec_module(module)
            direct = runpy.run_path(str(MODULE_PATH), run_name="__main__")
        self.assertEqual(output.getvalue(), "")
        self.assertEqual(errors.getvalue(), "")
        run_mock.assert_not_called()
        self.assertNotIn("main", direct)
        self.assertFalse(hasattr(module, "main"))

    def test_docker_constructor_rejects_platform_without_real_root_or_socket_reads(self):
        with mock.patch.object(runtime.sys, "platform", "linux"), \
                mock.patch.object(runtime.os, "geteuid", return_value=1000) as geteuid, \
                mock.patch.object(runtime.subprocess, "run") as run_mock:
            self.assert_runtime_error(runtime.Docker)
            geteuid.assert_called_once_with()
            run_mock.assert_not_called()
        with mock.patch.object(runtime.sys, "platform", "win32"), \
                mock.patch.object(runtime.os, "geteuid", return_value=0), \
                mock.patch.object(runtime.subprocess, "run") as run_mock:
            self.assert_runtime_error(runtime.Docker)
            run_mock.assert_not_called()

    def test_docker_transport_uses_fixed_argv_environment_and_timeout(self):
        result = SimpleNamespace(returncode=0, stdout=b"ok", stderr=SECRET_SENTINEL.encode())
        with mock.patch.object(runtime.sys, "platform", "linux"), \
                mock.patch.object(runtime.os, "geteuid", return_value=0), \
                mock.patch.object(runtime.subprocess, "run", return_value=result) as run_mock:
            docker = runtime.Docker()
            output = docker.call(["update", "--restart=no", CONTAINER_ID], 4.25)
        self.assertEqual(output, "ok")
        run_mock.assert_called_once()
        command = run_mock.call_args.args[0]
        kwargs = run_mock.call_args.kwargs
        self.assertEqual(command, [*runtime.DOCKER_PREFIX, "update", "--restart=no", CONTAINER_ID])
        self.assertIs(kwargs["shell"], False)
        self.assertIs(kwargs["stdin"], subprocess.DEVNULL)
        self.assertIs(kwargs["capture_output"], True)
        self.assertIs(kwargs["check"], False)
        self.assertEqual(kwargs["timeout"], 4.25)
        self.assertEqual(kwargs["env"], {
            "PATH": "/usr/sbin:/usr/bin:/sbin:/bin",
            "HOME": "/root",
            "LANG": "C.UTF-8",
        })
        self.assertNotIn(SECRET_SENTINEL, output)

    def test_docker_transport_rejects_invalid_operations_and_timeouts_before_subprocess(self):
        with mock.patch.object(runtime.sys, "platform", "linux"), \
                mock.patch.object(runtime.os, "geteuid", return_value=0), \
                mock.patch.object(runtime.subprocess, "run") as run_mock:
            docker = runtime.Docker()
            for args, timeout in (
                (["kill", "--signal=KILL", CONTAINER_ID], 1),
                (["stop", CONTAINER_ID], 1),
                (["update", "--restart=no", "short"], 1),
                (["wait", CONTAINER_ID, "extra"], 1),
                (["ps", "--filter", "project=anything"], 1),
                (["update", "--restart=no", CONTAINER_ID], True),
                (["update", "--restart=no", CONTAINER_ID], float("nan")),
                (["update", "--restart=no", CONTAINER_ID], float("inf")),
                (["update", "--restart=no", CONTAINER_ID], 0),
            ):
                with self.subTest(args=args, timeout=timeout):
                    self.assert_runtime_error(lambda args=args, timeout=timeout: docker.call(args, timeout))
            run_mock.assert_not_called()

    def test_docker_object_rejects_invalid_json_duplicate_keys_and_bad_shapes(self):
        malformed = (
            b"not-json",
            b"[{\"Id\":\"x\",\"Id\":\"y\"}]",
            b"[{\"Id\":NaN}]",
            b"{}",
            b"[{},{}]",
            b"[[]]",
        )
        with mock.patch.object(runtime.sys, "platform", "linux"), \
                mock.patch.object(runtime.os, "geteuid", return_value=0):
            docker = runtime.Docker()
            for stdout in malformed:
                with self.subTest(stdout=stdout):
                    result = SimpleNamespace(returncode=0, stdout=stdout, stderr=b"raw-provider-body")
                    with mock.patch.object(runtime.subprocess, "run", return_value=result):
                        error = self.assert_runtime_error(lambda: docker.object("image", IMAGE_ID, 1))
                        self.assertNotIn("raw-provider-body", str(error))
            self.assert_runtime_error(lambda: docker.object(object(), IMAGE_ID, 1))

    def test_docker_transport_redacts_exit_timeout_utf8_and_output_size_errors(self):
        bad_results = (
            SimpleNamespace(returncode=17, stdout=SECRET_SENTINEL.encode(), stderr=SECRET_SENTINEL.encode()),
            SimpleNamespace(returncode=0, stdout=b"\xff", stderr=SECRET_SENTINEL.encode()),
            SimpleNamespace(returncode=0, stdout=b"x" * (runtime.MAX_RESPONSE_BYTES + 1),
                             stderr=SECRET_SENTINEL.encode()),
        )
        with mock.patch.object(runtime.sys, "platform", "linux"), \
                mock.patch.object(runtime.os, "geteuid", return_value=0):
            docker = runtime.Docker()
            for result in bad_results:
                with self.subTest(returncode=result.returncode, stdout_len=len(result.stdout)):
                    with mock.patch.object(runtime.subprocess, "run", return_value=result):
                        error = self.assert_runtime_error(
                            lambda: docker.call(["wait", CONTAINER_ID], 1))
                        self.assertNotIn(SECRET_SENTINEL, str(error))
            expired = subprocess.TimeoutExpired(
                [SECRET_SENTINEL], 1, output=SECRET_SENTINEL.encode(), stderr=SECRET_SENTINEL.encode())
            with mock.patch.object(runtime.subprocess, "run", side_effect=expired):
                error = self.assert_runtime_error(lambda: docker.call(["wait", CONTAINER_ID], 1))
                self.assertNotIn(SECRET_SENTINEL, repr(error))

    def test_docker_global_service_listing_accepts_only_zero_or_one_full_id(self):
        with mock.patch.object(runtime.sys, "platform", "linux"), \
                mock.patch.object(runtime.os, "geteuid", return_value=0):
            docker = runtime.Docker()
            for raw, expected in ((b"", []), ((CONTAINER_ID + "\n").encode(), [CONTAINER_ID]),
                                  (CONTAINER_ID.encode(), [CONTAINER_ID])):
                result = SimpleNamespace(returncode=0, stdout=raw, stderr=b"")
                with mock.patch.object(runtime.subprocess, "run", return_value=result) as run_mock:
                    self.assertEqual(docker.service_ids(1), expected)
                    self.assertEqual(run_mock.call_args.args[0], [
                        *runtime.DOCKER_PREFIX, *EXPECTED_PS_ARGS,
                    ])
            bad_rows = (
                b"short\n", (CONTAINER_ID + "\n\n").encode(),
                (CONTAINER_ID + "\r\n").encode(), (CONTAINER_ID + "\n" + OTHER_CONTAINER_ID + "\n").encode(),
                (CONTAINER_ID + "\n" + CONTAINER_ID + "\n").encode(),
                (" " + CONTAINER_ID + "\n").encode(), (CONTAINER_ID.upper() + "\n").encode(),
            )
            for raw in bad_rows:
                with self.subTest(raw=raw):
                    result = SimpleNamespace(returncode=0, stdout=raw, stderr=b"")
                    with mock.patch.object(runtime.subprocess, "run", return_value=result):
                        self.assert_runtime_error(lambda: docker.service_ids(1))

    def test_capture_performs_double_observation_and_keeps_only_immutable_summary(self):
        binding = make_binding()
        config_before = binding.config_data
        first = make_container(binding, policy="unless-stopped")
        second = copy.deepcopy(first)
        fake = make_capture_docker(binding, first=first, second=second)
        with no_runtime_io(self):
            snapshot = runtime.capture(fake, binding)
        self.assertEqual(snapshot.binding_identity, runtime._binding_identity(binding))
        self.assertEqual(snapshot.generation, (CONTAINER_ID, first["Created"],
                                               first["State"]["StartedAt"], 0))
        self.assertEqual(snapshot.restart_policy, "unless-stopped")
        self.assertTrue(snapshot.running)
        self.assertEqual(config_before, binding.config_data)
        self.assertEqual(fake.operations(), [
            ("image", IMAGE_ID), ("ps",), ("container", CONTAINER_ID),
            ("ps",), ("image", IMAGE_ID), ("container", CONTAINER_ID),
        ])
        self.assertFalse(hasattr(snapshot, "inspect"))
        self.assertFalse(hasattr(snapshot, "container"))

    def test_capture_empty_snapshot_relists_without_mutations(self):
        binding = make_binding()
        fake = FakeDocker(id_rows=[[], []], images=[make_image(binding)])
        with no_runtime_io(self):
            snapshot = runtime.capture(fake, binding)
        self.assertIsNone(snapshot.generation)
        self.assertIsNone(snapshot.restart_policy)
        self.assertFalse(snapshot.running)
        self.assertEqual(fake.operations(), [("image", IMAGE_ID), ("ps",), ("ps",)])

    def test_capture_allows_only_clean_same_generation_running_to_exit(self):
        binding = make_binding()
        first = make_container(binding, state="running")
        second = make_container(binding, state="exited")
        fake = make_capture_docker(binding, first=first, second=second)
        with no_runtime_io(self):
            snapshot = runtime.capture(fake, binding)
        self.assertFalse(snapshot.running)
        self.assertEqual(snapshot.generation[0], CONTAINER_ID)

    def test_capture_rejects_appearing_disappearing_replaced_and_malformed_singletons(self):
        binding = make_binding()
        appearances = (
            ("appears", [[], [CONTAINER_ID]]),
            ("disappears", [[CONTAINER_ID], []]),
            ("replaced", [[CONTAINER_ID], [OTHER_CONTAINER_ID]]),
            ("duplicate", [[CONTAINER_ID, CONTAINER_ID]]),
            ("wrong-list-type", ["not-a-list"]),
            ("short-id", [["short"]]),
        )
        for label, id_rows in appearances:
            with self.subTest(label=label):
                fake = FakeDocker(id_rows=id_rows, images=[make_image(binding)],
                                  containers=[make_container(binding)])
                with no_runtime_io(self):
                    self.assert_runtime_error(lambda: runtime.capture(fake, binding))

    def test_capture_rejects_image_identity_and_revision_mismatch(self):
        binding = make_binding()
        for image in (
            make_image(binding, identity="sha256:" + "9" * 64),
            make_image(binding, revision="f" * 40),
        ):
            fake = FakeDocker(id_rows=[], images=[image])
            with self.subTest(image=image):
                with no_runtime_io(self):
                    self.assert_runtime_error(lambda: runtime.capture(fake, binding))

    def test_capture_rejects_foreign_labels_config_and_receipt_pins(self):
        binding = make_binding()
        cases = (
            {"project": "another-project"},
            {"changes": {"Config.Labels." + runtime.REVISION_LABEL: "f" * 40}},
            {"changes": {"Config.Labels." + runtime.SERVICE_LABEL: "another-service"}},
            {"changes": {"Config.Labels." + runtime.ONEOFF_LABEL: "True"}},
            {"changes": {"Config.Labels." + runtime.CONFIG_LABEL: "sha256:" + "8" * 64}},
            {"changes": {"Config.Labels." + runtime.RECEIPT_LABEL: "sha256:" + "9" * 64}},
            {"changes": {"Image": "sha256:" + "8" * 64}},
            {"changes": {"Config.User": "0:0"}},
            {"changes": {"Config.Entrypoint": ["/bin/sh"]}},
            {"changes": {"Config.Cmd": ["--help"]}},
            {"changes": {"HostConfig.AutoRemove": True}},
            {"changes": {"HostConfig.Init": False}},
            {"changes": {"HostConfig.ReadonlyRootfs": False}},
            {"changes": {"HostConfig.Privileged": True}},
        )
        for kwargs in cases:
            container = make_container(binding, **kwargs)
            fake = FakeDocker(id_rows=[[CONTAINER_ID]], images=[make_image(binding)],
                              containers=[container])
            with self.subTest(kwargs=kwargs):
                with no_runtime_io(self):
                    self.assert_runtime_error(lambda: runtime.capture(fake, binding))

    def test_capture_rejects_config_and_input_bind_identity_and_duplicate_destinations(self):
        binding = make_binding()
        mutators = (
            lambda c: c["Mounts"][0].update(Source="/wrong/config.json"),
            lambda c: c["Mounts"][0].update(RW=True),
            lambda c: c["Mounts"][0].update(Type="volume"),
            lambda c: c["Mounts"][0].update(Propagation="shared"),
            lambda c: c["Mounts"][1].update(Source="/wrong/input"),
            lambda c: c["Mounts"][1].update(RW=True),
            lambda c: c["Mounts"][1].update(Type="volume"),
            lambda c: c["Mounts"][1].update(Propagation="shared"),
            lambda c: c["Mounts"].pop(),
            lambda c: c["Mounts"].append("not-a-mount"),
            lambda c: c["Mounts"].append(copy.deepcopy(c["Mounts"][0])),
        )
        for mutate in mutators:
            container = make_container(binding)
            mutate(container)
            fake = FakeDocker(id_rows=[[CONTAINER_ID]], images=[make_image(binding)],
                              containers=[container])
            with no_runtime_io(self):
                self.assert_runtime_error(lambda: runtime.capture(fake, binding))

    def test_capture_rejects_malformed_object_list_boolean_and_generation_shapes(self):
        binding = make_binding()
        containers = [
            [],
            {"Id": CONTAINER_ID, "Config": []},
            make_container(binding, changes={"Mounts": True}),
            make_container(binding, changes={"Config.Labels": []}),
            make_container(binding, changes={"Mounts.0.Destination": 7}),
            make_container(binding, changes={"HostConfig.Init": 1}),
            make_container(binding, changes={"HostConfig.RestartPolicy.MaximumRetryCount": True}),
            make_container(binding, changes={"State.Running": 1}),
            make_container(binding, changes={"State.ExitCode": True}),
            make_container(binding, changes={"State.Status": "exited"}),
            make_container(binding, state="unknown"),
            make_container(binding, changes={"RestartCount": True}),
            make_container(binding, changes={"Created": ""}),
            make_container(binding, changes={"State.StartedAt": ""}),
            make_container(binding, changes={"State.Error": SECRET_SENTINEL}),
            make_container(binding, changes={"State.OOMKilled": True}),
            make_container(binding, changes={"State.Restarting": True}),
            make_container(binding, changes={"State.Paused": True}),
            make_container(binding, state="created", exit_code=1),
        ]
        for container in containers:
            fake = FakeDocker(id_rows=[[CONTAINER_ID]], images=[make_image(binding)],
                              containers=[container])
            with self.subTest(container=type(container).__name__):
                with no_runtime_io(self):
                    error = self.assert_runtime_error(lambda: runtime.capture(fake, binding))
                    self.assertNotIn(SECRET_SENTINEL, str(error))

    def test_capture_rejects_each_generation_field_change_and_policy_change(self):
        binding = make_binding()
        first = make_container(binding)
        changed = []
        for kwargs in (
            {"created": "different-created"},
            {"started": "different-started"},
            {"restart_count": 1},
            {"identity": OTHER_CONTAINER_ID},
            {"policy": "no"},
        ):
            changed.append(make_container(binding, **kwargs))
        for second in changed:
            fake = make_capture_docker(binding, first=first, second=second)
            with self.subTest(generation=(second.get("Id"), second.get("Created"),
                                          second.get("State", {}).get("StartedAt"),
                                          second.get("RestartCount"))):
                with no_runtime_io(self):
                    self.assert_runtime_error(lambda: runtime.capture(fake, binding))

    def test_observe_unchanged_rechecks_binding_generation_and_restart_policy(self):
        binding = make_binding()
        snapshot = capture_snapshot(binding)
        cases = (
            (make_binding(project="another-project"), snapshot, [[CONTAINER_ID]],
             [make_image(binding)], [make_container(binding)]),
            (make_binding(release_path="/srv/another-release-root/" + COMMIT), snapshot,
             [[CONTAINER_ID]], [make_image(binding)], [make_container(binding)]),
            (binding, snapshot, [[OTHER_CONTAINER_ID]], [make_image(binding)], []),
            (binding, snapshot, [[CONTAINER_ID]], [make_image(binding)],
             [make_container(binding, started="different-started")]),
            (binding, snapshot, [[CONTAINER_ID]], [make_image(binding)],
             [make_container(binding, policy="no")]),
        )
        for other_binding, prior, ids, images, containers in cases:
            fake = FakeDocker(id_rows=ids, images=images, containers=containers)
            with no_runtime_io(self):
                self.assert_runtime_error(lambda: runtime.observe_unchanged(fake, other_binding, prior))
        invalid = runtime.Snapshot(snapshot.binding_identity, snapshot.generation,
                                   "invalid-policy", True)
        untouched = FakeDocker(id_rows=[], images=[])
        with no_runtime_io(self):
            self.assert_runtime_error(lambda: runtime.observe_unchanged(untouched, binding, invalid))
        self.assertEqual(untouched.calls, [])

    def test_observe_unchanged_returns_new_clean_exit_observation_for_same_generation(self):
        binding = make_binding()
        snapshot = capture_snapshot(binding)
        exited = make_container(binding, state="exited", policy="unless-stopped")
        fake = FakeDocker(id_rows=[[CONTAINER_ID]], images=[make_image(binding)], containers=[exited])
        with no_runtime_io(self):
            current = runtime.observe_unchanged(fake, binding, snapshot)
        self.assertFalse(current.running)
        self.assertEqual(current.generation, snapshot.generation)
        self.assertEqual(current.restart_policy, snapshot.restart_policy)

    def test_observe_empty_snapshot_requires_service_still_absent(self):
        binding = make_binding()
        snapshot = runtime.capture(FakeDocker(id_rows=[[], []], images=[make_image(binding)]), binding)
        fake = FakeDocker(id_rows=[[CONTAINER_ID]], images=[make_image(binding)])
        with no_runtime_io(self):
            self.assert_runtime_error(lambda: runtime.observe_unchanged(fake, binding, snapshot))
        self.assertEqual(fake.operations(), [("image", IMAGE_ID), ("ps",)])

    def test_drain_running_process_updates_then_terms_waits_and_verifies_exact_generation(self):
        binding = make_binding()
        snapshot = capture_snapshot(binding, state="running", policy="unless-stopped")
        observations = [
            make_container(binding, state="running", policy="unless-stopped"),
            make_container(binding, state="running", policy="no"),
            make_container(binding, state="exited", policy="no"),
        ]
        fake = FakeDocker(
            id_rows=[[CONTAINER_ID], [CONTAINER_ID], [CONTAINER_ID]],
            images=[make_image(binding), make_image(binding), make_image(binding)],
            containers=observations,
            mutations=["", "", "0\n"],
        )
        config_before = binding.config_data
        with no_runtime_io(self):
            result = runtime.drain(fake, binding, snapshot, clock=lambda: 100.0)
        self.assertEqual(result.status, "PROCESS_EXIT_OBSERVED")
        self.assertTrue(result.captured)
        self.assertTrue(result.signalled)
        self.assertEqual(result.generation, snapshot.generation)
        self.assertEqual(fake.operations(), [
            ("image", IMAGE_ID), ("ps",), ("container", CONTAINER_ID),
            ("update", "--restart=no", CONTAINER_ID), ("ps",),
            ("image", IMAGE_ID), ("container", CONTAINER_ID),
            ("kill", "--signal=TERM", CONTAINER_ID), ("wait", CONTAINER_ID),
            ("ps",), ("image", IMAGE_ID), ("container", CONTAINER_ID),
        ])
        for call in fake.calls:
            if call[0] == "call" and call[1][0] == "wait":
                self.assertEqual(call[2], binding.stop_timeout_seconds)
            else:
                self.assertLessEqual(call[-1], runtime.OBSERVATION_TIMEOUT_SECONDS)
        self.assertEqual(binding.config_data, config_before)

    def test_drain_already_stopped_process_updates_but_does_not_signal_or_wait(self):
        binding = make_binding()
        for stopped_state in ("created", "exited"):
            for policy in ("no", "unless-stopped"):
                snapshot = capture_snapshot(binding, state=stopped_state, policy=policy)
                observations = [
                    make_container(binding, state=stopped_state, policy=policy),
                    make_container(binding, state=stopped_state, policy="no"),
                    make_container(binding, state=stopped_state, policy="no"),
                ]
                fake = FakeDocker(
                    id_rows=[[CONTAINER_ID], [CONTAINER_ID], [CONTAINER_ID]],
                    images=[make_image(binding), make_image(binding), make_image(binding)],
                    containers=observations,
                    mutations=[""],
                )
                with self.subTest(state=stopped_state, policy=policy), no_runtime_io(self):
                    result = runtime.drain(fake, binding, snapshot, clock=lambda: 5.0)
                self.assertEqual(result.status, "PROCESS_EXIT_OBSERVED")
                self.assertTrue(result.captured)
                self.assertFalse(result.signalled)
                self.assertNotIn(("kill", "--signal=TERM", CONTAINER_ID), fake.operations())
                self.assertNotIn(("wait", CONTAINER_ID), fake.operations())

    def test_empty_capture_observe_and_drain_repeat_absence_without_mutations(self):
        binding = make_binding()
        empty = runtime.capture(FakeDocker(id_rows=[[], []], images=[make_image(binding)]), binding)
        unchanged = runtime.observe_unchanged(
            FakeDocker(id_rows=[[]], images=[make_image(binding)]), binding, empty)
        fake = FakeDocker(id_rows=[[], []], images=[make_image(binding)])
        with no_runtime_io(self):
            result = runtime.drain(fake, binding, unchanged, clock=lambda: 0.0)
        self.assertEqual(result.status, "PROCESS_ABSENCE_OBSERVED")
        self.assertFalse(result.captured)
        self.assertFalse(result.signalled)
        self.assertIsNone(result.generation)
        self.assertEqual(fake.operations(), [("image", IMAGE_ID), ("ps",), ("ps",)])

        appeared = FakeDocker(id_rows=[[CONTAINER_ID]], images=[make_image(binding)])
        with no_runtime_io(self):
            self.assert_runtime_error(lambda: runtime.drain(
                appeared, binding, empty, clock=lambda: 0.0))
        self.assertEqual(appeared.operations(), [("image", IMAGE_ID), ("ps",)])

    def test_drain_rejects_invalid_snapshot_replacement_duplicate_and_preupdate_restart(self):
        binding = make_binding()
        snapshot = capture_snapshot(binding)
        foreign = runtime.Snapshot(("foreign",) + snapshot.binding_identity[1:],
                                   snapshot.generation, snapshot.restart_policy, snapshot.running)
        no_calls = FakeDocker(id_rows=[], images=[])
        with no_runtime_io(self):
            self.assert_runtime_error(lambda: runtime.drain(
                no_calls, binding, foreign, clock=lambda: 0.0))
        self.assertEqual(no_calls.calls, [])

        cases = (
            ([[]], []),
            ([[OTHER_CONTAINER_ID]], []),
            ([[CONTAINER_ID, OTHER_CONTAINER_ID]], []),
            ([[CONTAINER_ID]], [make_container(binding, started="restarted")]),
            ([[CONTAINER_ID]], [make_container(binding, policy="no")]),
        )
        for ids, containers in cases:
            fake = FakeDocker(id_rows=ids, images=[make_image(binding)], containers=containers)
            with self.subTest(ids=ids):
                with no_runtime_io(self):
                    self.assert_runtime_error(lambda: runtime.drain(
                        fake, binding, snapshot, clock=lambda: 0.0))
                self.assertFalse(any(call[0] == "call" for call in fake.calls))

    def test_drain_update_failure_unconfirmed_restart_policy_and_postupdate_replacement_never_term(self):
        binding = make_binding()
        snapshot = capture_snapshot(binding, state="running", policy="unless-stopped")
        base_ids = [[CONTAINER_ID], [CONTAINER_ID]]
        base_images = [make_image(binding), make_image(binding)]
        running = make_container(binding, state="running", policy="unless-stopped")

        fake = FakeDocker(id_rows=[base_ids[0]], images=[base_images[0]], containers=[running],
                          mutations=[RuntimeError(SECRET_SENTINEL)])
        with no_runtime_io(self):
            error = self.assert_runtime_error(lambda: runtime.drain(
                fake, binding, snapshot, clock=lambda: 0.0))
        self.assertNotIn(SECRET_SENTINEL, str(error))
        self.assertNotIn(("kill", "--signal=TERM", CONTAINER_ID), fake.operations())

        fake = FakeDocker(id_rows=base_ids, images=base_images,
                          containers=[running, make_container(binding, state="running", policy="unless-stopped")],
                          mutations=[""])
        with no_runtime_io(self):
            self.assert_runtime_error(lambda: runtime.drain(fake, binding, snapshot, clock=lambda: 0.0))
        self.assertNotIn(("kill", "--signal=TERM", CONTAINER_ID), fake.operations())

        fake = FakeDocker(id_rows=[[CONTAINER_ID], [OTHER_CONTAINER_ID]],
                          images=[base_images[0]], containers=[running], mutations=[""])
        with no_runtime_io(self):
            self.assert_runtime_error(lambda: runtime.drain(fake, binding, snapshot, clock=lambda: 0.0))
        self.assertNotIn(("kill", "--signal=TERM", CONTAINER_ID), fake.operations())

        fake = FakeDocker(id_rows=[[CONTAINER_ID], [CONTAINER_ID, OTHER_CONTAINER_ID]],
                          images=[base_images[0]], containers=[running], mutations=[""])
        with no_runtime_io(self):
            self.assert_runtime_error(lambda: runtime.drain(fake, binding, snapshot, clock=lambda: 0.0))
        self.assertNotIn(("kill", "--signal=TERM", CONTAINER_ID), fake.operations())

        changed = make_container(binding, state="running", policy="no", restart_count=1)
        fake = FakeDocker(id_rows=base_ids, images=base_images,
                          containers=[running, changed], mutations=[""])
        with no_runtime_io(self):
            self.assert_runtime_error(lambda: runtime.drain(fake, binding, snapshot, clock=lambda: 0.0))
        self.assertNotIn(("kill", "--signal=TERM", CONTAINER_ID), fake.operations())
        self.assertNotIn(("wait", CONTAINER_ID), fake.operations())

        changed_pin = make_container(binding, state="running", policy="no",
                                     changes={"Config.Labels." + runtime.CONFIG_LABEL:
                                              "sha256:" + "f" * 64})
        fake = FakeDocker(id_rows=base_ids, images=base_images,
                          containers=[running, changed_pin], mutations=[""])
        with no_runtime_io(self):
            self.assert_runtime_error(lambda: runtime.drain(fake, binding, snapshot, clock=lambda: 0.0))
        self.assertNotIn(("kill", "--signal=TERM", CONTAINER_ID), fake.operations())

    def test_drain_term_failure_or_wait_failure_never_retries_or_claims_exit(self):
        binding = make_binding()
        snapshot = capture_snapshot(binding)
        pre = make_container(binding, state="running", policy="unless-stopped")
        post = make_container(binding, state="running", policy="no")
        for term_result, wait_result in (
            (RuntimeError(SECRET_SENTINEL), "unused"),
            ("", "1"), ("", "0 "), ("", "0\n\n"),
            ("", subprocess.TimeoutExpired(["docker"], 1, stderr=SECRET_SENTINEL.encode())),
        ):
            fake = FakeDocker(
                id_rows=[[CONTAINER_ID], [CONTAINER_ID]],
                images=[make_image(binding), make_image(binding)],
                containers=[pre, post],
                mutations=["", term_result] if isinstance(term_result, BaseException)
                else ["", term_result, wait_result],
            )
            with self.subTest(term=type(term_result).__name__, wait=wait_result):
                with no_runtime_io(self):
                    error = self.assert_runtime_error(lambda: runtime.drain(
                        fake, binding, snapshot, clock=lambda: 0.0))
                self.assertNotIn(SECRET_SENTINEL, str(error))
                operations = fake.operations()
                self.assertEqual(operations.count(("kill", "--signal=TERM", CONTAINER_ID)), 1)
                if isinstance(term_result, BaseException):
                    self.assertNotIn(("wait", CONTAINER_ID), operations)
                else:
                    self.assertEqual(operations.count(("wait", CONTAINER_ID)), 1)
                self.assertNotIn(("kill", "--signal=KILL", CONTAINER_ID), operations)

    def test_drain_final_replacement_or_unclean_exit_fails_closed(self):
        binding = make_binding()
        snapshot = capture_snapshot(binding)
        pre = make_container(binding, state="running", policy="unless-stopped")
        post = make_container(binding, state="running", policy="no")
        invalid_finals = (
            make_container(binding, state="running", policy="no"),
            make_container(binding, state="exited", policy="no", exit_code=2),
            make_container(binding, state="exited", policy="no",
                           changes={"State.OOMKilled": True}),
            make_container(binding, state="exited", policy="no",
                           changes={"State.Error": "daemon-secret"}),
            make_container(binding, state="exited", policy="unless-stopped"),
            make_container(binding, state="exited", policy="no", started="new-generation"),
        )
        for final in invalid_finals:
            fake = FakeDocker(
                id_rows=[[CONTAINER_ID], [CONTAINER_ID], [CONTAINER_ID]],
                images=[make_image(binding), make_image(binding), make_image(binding)],
                containers=[pre, post, final], mutations=["", "", "0"],
            )
            with self.subTest(final_state=final["State"]):
                with no_runtime_io(self):
                    self.assert_runtime_error(lambda: runtime.drain(
                        fake, binding, snapshot, clock=lambda: 0.0))

        replacement = FakeDocker(
            id_rows=[[CONTAINER_ID], [CONTAINER_ID], [OTHER_CONTAINER_ID]],
            images=[make_image(binding), make_image(binding)], containers=[pre, post],
            mutations=["", "", "0"],
        )
        with no_runtime_io(self):
            self.assert_runtime_error(lambda: runtime.drain(
                replacement, binding, snapshot, clock=lambda: 0.0))
        self.assertNotIn(("wait", OTHER_CONTAINER_ID), replacement.operations())

    def test_drain_uses_one_deadline_and_fails_if_exhausted_at_each_boundary(self):
        binding = make_binding()
        snapshot = capture_snapshot(binding)
        pre = make_container(binding, state="running", policy="unless-stopped")
        post = make_container(binding, state="running", policy="no")
        final = make_container(binding, state="exited", policy="no")

        class MutableClock:
            def __init__(self):
                self.value = 0.0

            def __call__(self):
                return self.value

        expired_before = iter((0.0, float(binding.stop_timeout_seconds)))
        fake = FakeDocker(id_rows=[[CONTAINER_ID]], images=[make_image(binding)], containers=[])
        with no_runtime_io(self):
            self.assert_runtime_error(lambda: runtime.drain(
                fake, binding, snapshot, clock=lambda: next(expired_before)))
        self.assertEqual(fake.calls, [])

        clock = MutableClock()
        fake = FakeDocker(id_rows=[[CONTAINER_ID]], images=[make_image(binding)], containers=[pre],
                          mutations=[""])
        fake.after_call = lambda kind, detail: setattr(clock, "value", float(binding.stop_timeout_seconds)) \
            if kind == "call" and detail == ("update", "--restart=no", CONTAINER_ID) else None
        with no_runtime_io(self):
            self.assert_runtime_error(lambda: runtime.drain(fake, binding, snapshot, clock=clock))
        self.assertNotIn(("kill", "--signal=TERM", CONTAINER_ID), fake.operations())

        clock = MutableClock()
        fake = FakeDocker(
            id_rows=[[CONTAINER_ID], [CONTAINER_ID]],
            images=[make_image(binding), make_image(binding)], containers=[pre, post],
            mutations=["", "", "0"],
        )
        fake.after_call = lambda kind, detail: setattr(clock, "value", float(binding.stop_timeout_seconds)) \
            if kind == "call" and detail == ("wait", CONTAINER_ID) else None
        with no_runtime_io(self):
            self.assert_runtime_error(lambda: runtime.drain(fake, binding, snapshot, clock=clock))
        self.assertEqual(fake.operations().count(("wait", CONTAINER_ID)), 1)

        clock = MutableClock()
        fake = FakeDocker(
            id_rows=[[CONTAINER_ID], [CONTAINER_ID], [CONTAINER_ID]],
            images=[make_image(binding), make_image(binding), make_image(binding)],
            containers=[pre, post, final], mutations=["", "", "0"],
        )
        final_inspects = 0

        def expire_after_final(kind, detail):
            nonlocal final_inspects
            if kind == "inspect" and detail == "container":
                final_inspects += 1
                if final_inspects == 3:
                    clock.value = float(binding.stop_timeout_seconds)

        fake.after_call = expire_after_final
        with no_runtime_io(self):
            self.assert_runtime_error(lambda: runtime.drain(fake, binding, snapshot, clock=clock))
        self.assertEqual(final_inspects, 3)

        invalid_clock = lambda: float("nan")
        no_calls = FakeDocker(id_rows=[], images=[])
        with no_runtime_io(self):
            self.assert_runtime_error(lambda: runtime.drain(
                no_calls, binding, snapshot, clock=invalid_clock))
        self.assertEqual(no_calls.calls, [])

    def test_public_errors_from_fake_transport_are_fixed_and_do_not_leak_details(self):
        binding = make_binding()
        fake = FakeDocker(id_rows=[], images=[make_image(binding)])
        fake.images[0]["Config"]["Labels"][runtime.REVISION_LABEL] = SECRET_SENTINEL
        with no_runtime_io(self):
            error = self.assert_runtime_error(lambda: runtime.capture(fake, binding))
        self.assertNotIn(SECRET_SENTINEL, str(error))

        class ExplodingDocker:
            def object(self, kind, identity, timeout):
                raise OSError(SECRET_SENTINEL + " /host/private/path")

        with no_runtime_io(self):
            error = self.assert_runtime_error(lambda: runtime.capture(ExplodingDocker(), binding))
        self.assertNotIn(SECRET_SENTINEL, str(error))
        self.assertNotIn("/host/private/path", str(error))

    def test_results_claim_only_process_observation_not_settlement_or_readiness(self):
        binding = make_binding()
        absence = runtime.DrainResult("PROCESS_ABSENCE_OBSERVED", False, False, None)
        names = set(absence.__dataclass_fields__)
        forbidden = {"settled", "quiescent", "healthy", "fenced", "safe_to_backup"}
        self.assertFalse(names & forbidden)
        self.assertEqual(absence.status, "PROCESS_ABSENCE_OBSERVED")
        self.assertFalse(hasattr(runtime, "start"))
        self.assertFalse(hasattr(runtime, "resume"))
        self.assertFalse(hasattr(runtime, "main"))
        self.assertEqual(binding.stop_timeout_seconds, 17)
        self.assertEqual(runtime.SERVICE, "owner-beta-simulation-runner")


if __name__ == "__main__":
    unittest.main()
