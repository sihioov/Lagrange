#!/usr/bin/env python3
"""Synthetic operations regressions. No Docker, DB or host filesystem access."""
import copy
import contextlib
import importlib.util
import io
import sys
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


ops = load("simulation_operations_test", ROOT / "scripts/ops/lib/owner-beta-simulation-operations.py")
fixture = load("simulation_runtime_fixture", ROOT / "scripts/ops/test_owner_beta_simulation_runtime.py")
REAL_DISPOSITION_CREATE = ops._disposition_create  # setUp replaces the module attribute
API_ID = "d" * 64
API_IMAGE = "sha256:" + "e" * 64


def api_container():
    return {
        "Id": API_ID, "Image": API_IMAGE, "Created": "created", "RestartCount": 0,
        "Path": "/usr/local/bin/api-server", "Args": [], "Mounts": [],
        "Config": {"User": "10001:10001", "Entrypoint": ["/usr/local/bin/api-server"], "Cmd": None, "Labels": {
            "com.docker.compose.project": fixture.PROJECT, "com.docker.compose.service": "api-server",
            "com.docker.compose.oneoff": "False", "org.opencontainers.image.revision": fixture.COMMIT},
            "Env": ["OWNER_BETA_ACCESS_MODE=disabled", "OWNER_BETA_PRICE_INPUT_MODE=disabled"]},
        "HostConfig": {"RestartPolicy": {"Name": "unless-stopped", "MaximumRetryCount": 0}},
        "State": {"Running": True, "Restarting": False, "Paused": False, "OOMKilled": False,
                  "Error": "", "ExitCode": 0, "StartedAt": "started", "Status": "running",
                  "Health": {"Status": "healthy"}},
    }


class ApiDocker:
    def __init__(self):
        self.container = api_container()
        self.calls = []
        self.fail_update = False
        self.replace_after_update = False
        self.image_capability = "v1"
        self.wait_result = "0\n"

    def object(self, kind, identity, timeout):
        self.calls.append(("inspect", kind, identity))
        if kind == "image":
            return {"Id": API_IMAGE, "Config": {"User": "10001:10001", "Entrypoint": ["/usr/local/bin/api-server"],
                    "Cmd": None, "Labels": {"org.opencontainers.image.revision": fixture.COMMIT,
                    "org.lagrange.api.http-drain-contract": self.image_capability}}}
        return copy.deepcopy(self.container)

    def call(self, args, timeout):
        self.calls.append(tuple(args))
        if args[0] == "ps":
            return self.container["Id"] + "\n"
        if args[:2] == ["update", "--restart=no"]:
            if self.fail_update:
                raise RuntimeError("synthetic transport failure")
            self.container["HostConfig"]["RestartPolicy"]["Name"] = "no"
            if self.replace_after_update:
                self.container["Created"] = "replaced"
            return ""
        if args[:2] == ["kill", "--signal=TERM"]:
            self.container["State"].update(Running=False, Status="exited")
            return ""
        if args[0] == "wait":
            return self.wait_result
        raise AssertionError("unexpected API command")


def worker_value(binding, *, state="created", identity=fixture.CONTAINER_ID):
    value = fixture.make_container(binding, state=state, policy="no", identity=identity,
                                   created="worker-created", started="worker-before")
    value["Config"].update({"StopTimeout": 17,
        "Env": ["APP_ENV=production", "OWNER_BETA_SIMULATION_MODE=owner_only",
                "OWNER_BETA_ACCESS_MODE=owner_only", "OWNER_BETA_PRICE_INPUT_MODE=sealed_v1",
                "LAGRANGE_CODE_COMMIT=" + binding.commit,
                "OWNER_BETA_SIMULATION_CONFIG_SHA256=" + binding.config_sha256,
                "OWNER_BETA_SIMULATION_RECEIPT_SHA256=" + binding.receipt_sha256[7:],
                "DB_HOST=postgres", "DB_PORT=5432", "DB_NAME=fixture", "DB_USER=worker",
                "DB_PASSWORD_FILE=/run/secrets/db_worker_password"],
        "Healthcheck": {"Test": ["CMD", "/usr/local/bin/owner-beta-simulation-runner", "--health",
            "--expected-commit", binding.commit, "--expected-config-sha256", binding.config_sha256,
            "--expected-receipt-sha256", binding.receipt_sha256[7:]],
            "Interval": 1000000000, "Timeout": 1000000000, "StartPeriod": 1000000000, "Retries": 1}})
    value["HostConfig"].update({"CapDrop": ["ALL"], "CapAdd": None,
        "SecurityOpt": ["no-new-privileges:true"], "IpcMode": "private", "Memory": 1,
        "NanoCpus": 1, "PidsLimit": 1, "NetworkMode": "network-id",
        "Tmpfs": {"/run/owner-beta-simulation": "uid=0,gid=10001,mode=0750,size=4096",
                  "/run/owner-beta-simulation-health/records": "uid=10001,gid=10001,mode=0700,size=1"}})
    value["Mounts"].append({"Type": "bind", "Source": "/synthetic/secrets/owner-beta-simulation-runner/db_worker_password",
        "Destination": "/run/secrets/db_worker_password", "RW": False, "Propagation": "rprivate"})
    value["State"]["Health"] = {"Status": "healthy"}
    value["NetworkSettings"] = {"Networks": {"fixture": {"NetworkID": "network-id"}}}
    return value


class WorkerTransport:
    """Mutable transport fixture; the real C2 capture/drain checks every view."""
    def __init__(self, binding):
        self.binding, self.container, self.calls = binding, None, []
        self.fail_observation_once = False
        self.fail_cleanup = False

    def service_ids(self, timeout):
        self.calls.append(("ps",))
        if self.fail_observation_once:
            self.fail_observation_once = False
            raise RuntimeError("synthetic observation loss")
        return [] if self.container is None else [self.container["Id"]]

    def object(self, kind, identity, timeout):
        self.calls.append(("inspect", kind, identity))
        if kind == "image":
            return fixture.make_image(self.binding)
        if self.container is None or identity != self.container["Id"]:
            raise RuntimeError("synthetic replacement")
        return copy.deepcopy(self.container)

    def call(self, args, timeout):
        self.calls.append(tuple(args))
        if self.container is None or args[-1] != self.container["Id"]:
            raise RuntimeError("synthetic foreign target")
        if args[:2] == ["update", "--restart=no"]:
            self.container["HostConfig"]["RestartPolicy"]["Name"] = "no"
            return ""
        if args[:2] == ["kill", "--signal=TERM"]:
            self.container["State"].update(Running=False, Status="exited", ExitCode=0)
            return ""
        if args[0] == "wait":
            if self.fail_cleanup:
                raise RuntimeError("synthetic cleanup loss")
            return "0\n"
        raise AssertionError("unexpected worker command")


class OperationsTests(unittest.TestCase):
    disposition_create_real = staticmethod(REAL_DISPOSITION_CREATE)

    def setUp(self):
        self.pending = {}
        def read(_root, name):
            return copy.deepcopy(self.pending.get(name))
        def write(_root, name, value, *, previous=None):
            if previous is None:
                if name in self.pending: raise ops.OperationError("pending exists")
            elif self.pending.get(name) != previous:
                raise ops.OperationError("pending changed")
            self.pending[name] = copy.deepcopy(value)
        def remove(_root, name, expected):
            if self.pending.get(name) != expected: raise ops.OperationError("pending changed")
            del self.pending[name]
        def disposition_read(_root, name):
            return copy.deepcopy(self.pending.get(name))
        def disposition_create(_root, name, value):
            if name in self.pending: raise ops.OperationError("disposition exists")
            self.pending[name] = copy.deepcopy(value)
        for name, fn in (("_pending_read", read), ("_pending_write", write), ("_pending_remove", remove),
                         ("_disposition_read", disposition_read), ("_disposition_create", disposition_create)):
            patcher = mock.patch.object(ops, name, side_effect=fn)
            patcher.start()
            self.addCleanup(patcher.stop)

    def binding(self):
        return ops.runtime.Binding(fixture.PROJECT, fixture.RELEASE, fixture.COMMIT, fixture.config_bytes())

    def exercise_start(self, outcome="success", *, existing=False):
        binding = self.binding()
        worker = WorkerTransport(binding)
        if existing:
            worker.container = worker_value(binding, state="exited")
        context = SimpleNamespace(required=True, binding=binding, source_release=fixture.RELEASE,
            target_release=fixture.RELEASE, source_commit=fixture.COMMIT, target_commit=fixture.COMMIT,
            project=fixture.PROJECT, source_images={"api-server": API_IMAGE}, install_root="/synthetic/install",
            values={"OWNER_BETA_ACCESS_MODE": "owner_only", "OWNER_BETA_PRICE_INPUT_MODE": "sealed_v1",
                    "OWNER_BETA_PAPER_MODE": "disabled", "POSTGRES_DB": "fixture",
                    "LAGRANGE_RUNTIME_SECRET_DIR": "/synthetic/secrets"})
        outer = self
        class Docker(ApiDocker):
            def object(self, kind, identity, timeout):
                if kind == "container" and worker.container is not None and identity == worker.container["Id"]:
                    return copy.deepcopy(worker.container)
                return super().object(kind, identity, timeout)
            def call(self, args, timeout):
                if args[0] != "start": return super().call(args, timeout)
                self.calls.append(tuple(args))
                assert outer.pending[ops.START_PENDING]["phase"] == "start-requested"
                assert args[1] == worker.container["Id"]
                worker.container["State"].update(Running=True, Status="running", StartedAt="worker-after")
                worker.container["NetworkSettings"]["Networks"] = {"fixture": {"NetworkID": "network-id"}}
                if outcome in ("health", "cleanup"):
                    worker.container["State"]["Health"]["Status"] = "unhealthy"
                if outcome == "timeout":
                    worker.container["State"]["Health"]["Status"] = "starting"
                if outcome == "cleanup": worker.fail_cleanup = True
                if outcome == "exit-nonzero":
                    worker.container["State"].update(Running=False, Status="exited", ExitCode=9)
                if outcome == "observation": worker.fail_observation_once = True
                if outcome == "replacement": worker.container["Id"] = fixture.OTHER_CONTAINER_ID
                if outcome == "lost": raise TimeoutError("synthetic lost response")
                if outcome == "interrupt": raise KeyboardInterrupt()
                return args[1] + "\n"
        docker = Docker()
        docker.container["Config"]["Env"] = ["OWNER_BETA_ACCESS_MODE=owner_only",
            "OWNER_BETA_PRICE_INPUT_MODE=sealed_v1", "LAGRANGE_CODE_COMMIT=" + fixture.COMMIT]
        database = mock.Mock()
        database.execute.return_value = "SIMULATION_MAINTENANCE: SETTLED"
        class Output(io.BytesIO):
            def fileno(self): return 71
            def close(self): pass
        output = Output()
        calls = []
        def create(argv, **kwargs):
            calls.append((argv, kwargs))
            assert outer.pending[ops.START_PENDING]["phase"] == "creating"
            if outcome == "create-empty": raise TimeoutError("synthetic create response loss")
            worker.container = worker_value(binding)
            worker.container["NetworkSettings"]["Networks"] = {}
            worker.container["Config"]["Labels"]["org.lagrange.simulation-start-attempt"] = (b"a" * 16).hex()
            if outcome == "create-lost": raise TimeoutError("synthetic create response loss")
            if outcome == "create-foreign":
                worker.container["Config"]["Labels"]["org.lagrange.simulation-start-attempt"] = "f" * 32
                raise TimeoutError("synthetic foreign creation")
            if outcome == "create-nonzero": return SimpleNamespace(returncode=1)
            return SimpleNamespace(returncode=0)
        ticks = [0]
        def clock():
            if outcome == "timeout" and worker.container is not None and worker.container["State"]["Running"]:
                ticks[0] += 1
                return 1.0 if ticks[0] < 4 else 3.0
            return 3.0 if ticks[0] >= 4 else 1.0
        with contextlib.ExitStack() as stack:
            stack.enter_context(mock.patch.object(ops, "validate_worker_secret"))
            stack.enter_context(mock.patch.object(ops, "input_guard", return_value=contextlib.nullcontext()))
            stack.enter_context(mock.patch.object(ops.daily, "internal_network", return_value={"Id": "network-id", "Name": "fixture"}))
            stack.enter_context(mock.patch.object(ops.release, "reader_guard"))
            stack.enter_context(mock.patch.object(ops.tempfile, "mkstemp", return_value=(71, "/synthetic/override")))
            stack.enter_context(mock.patch.object(ops.os, "fchmod"))
            stack.enter_context(mock.patch.object(ops.os, "fdopen", return_value=output))
            stack.enter_context(mock.patch.object(ops.os, "fsync"))
            stack.enter_context(mock.patch.object(ops.os, "unlink"))
            stack.enter_context(mock.patch.object(ops.os, "urandom", return_value=b"a" * 16))
            stack.enter_context(mock.patch.object(ops.time, "monotonic", side_effect=clock))
            stack.enter_context(mock.patch.object(ops.time, "sleep"))
            stack.enter_context(mock.patch.object(ops.subprocess, "run", side_effect=create))
            try:
                result = ops.start_simulation(context, docker, worker, database)
                failure = None
            except ops.StartFailure as exc:
                result, failure = None, exc
        return SimpleNamespace(result=result, failure=failure, worker=worker, docker=docker,
                               database=database, calls=calls, output=output)

    def test_overlay_derives_three_independent_pins_and_exact_limits(self):
        binding = self.binding()
        result = ops.overlay_environment(binding)
        self.assertEqual(result["LAGRANGE_SIM_CONFIG_SHA256"], binding.config_sha256)
        self.assertEqual(result["LAGRANGE_SIM_RECEIPT_SHA256"], "2" * 64)
        self.assertEqual(result["LAGRANGE_SIM_RECEIPT_SHA256_PREFIXED"], "sha256:" + "2" * 64)
        self.assertEqual(result["LAGRANGE_SIM_CPUS"], "0.000000001")
        self.assertEqual(result["LAGRANGE_SIM_STOP_SECONDS"], "17")
        self.assertEqual(result["LAGRANGE_SIM_HISTORICAL_ROOT"], fixture.HISTORICAL_ROOT)

    def test_fence_uses_exact_id_update_then_term_then_wait(self):
        docker = ApiDocker()
        result = ops.fence_api(docker, fixture.PROJECT, fixture.COMMIT, API_IMAGE, 17, clock=lambda: 1)
        mutations = [v for v in docker.calls if v[0] in ("update", "kill", "wait")]
        self.assertEqual(mutations, [("update", "--restart=no", API_ID),
                                     ("kill", "--signal=TERM", API_ID), ("wait", API_ID)])
        self.assertEqual(result.identity, API_ID)
        self.assertFalse(docker.container["State"]["Running"])

    def test_foreign_or_replaced_api_never_signalled(self):
        for alteration in ("foreign", "replaced"):
            with self.subTest(alteration=alteration):
                docker = ApiDocker()
                if alteration == "foreign":
                    docker.container["Config"]["Labels"]["com.docker.compose.project"] = "foreign"
                else:
                    docker.replace_after_update = True
                with self.assertRaises(ops.OperationError):
                    ops.fence_api(docker, fixture.PROJECT, fixture.COMMIT, API_IMAGE, 17, clock=lambda: 1)
                self.assertFalse(any(v[0] in ("kill", "wait") for v in docker.calls))

    def test_capability_must_come_from_image_and_executable_cannot_be_shadowed(self):
        for mutation in ("missing", "wrong", "forged-container", "shadow", "argv", "created"):
            with self.subTest(mutation=mutation):
                docker = ApiDocker()
                if mutation in ("missing", "forged-container"):
                    docker.image_capability = None
                    docker.container["Config"]["Labels"]["org.lagrange.api.http-drain-contract"] = "v1"
                elif mutation == "wrong":
                    docker.image_capability = "v2"
                elif mutation == "shadow":
                    docker.container["Mounts"] = [{"Destination": "/usr/local/bin"}]
                elif mutation == "argv":
                    docker.container["Args"] = ["--override"]
                else:
                    docker.container["State"].update(Running=False, Status="created")
                with self.assertRaises(ops.OperationError):
                    ops.fence_api(docker, fixture.PROJECT, fixture.COMMIT, API_IMAGE, 17, clock=lambda: 1)
                self.assertFalse(any(v[0] in ("update", "kill", "wait") for v in docker.calls))

    def test_failed_api_fence_prevents_worker_drain_and_db(self):
        for failure in ("update", "wait"):
            with self.subTest(failure=failure):
                binding = self.binding()
                docker = ApiDocker()
                docker.fail_update = failure == "update"
                if failure == "wait":
                    docker.wait_result = "1\n"
                database = mock.Mock()
                with mock.patch.object(ops.runtime, "capture", return_value=object()), \
                     mock.patch.object(ops.runtime, "drain") as drain:
                    with self.assertRaises(ops.OperationError):
                        ops.drain_for_maintenance(docker, object(), database, {}, binding, API_IMAGE, clock=lambda: 1)
                drain.assert_not_called()
                database.execute.assert_not_called()

    def test_interrupted_fence_does_not_infer_handler_completion_from_stopped_api(self):
        docker, database = ApiDocker(), mock.Mock()
        docker.container["State"].update(Running=False, Status="exited", ExitCode=0)
        context = SimpleNamespace(install_root="/srv/lagrange", source_release=fixture.RELEASE,
                                  source_commit=fixture.COMMIT, project=fixture.PROJECT, binding=None)
        record = {"api_generation": [API_ID, "created", "started", 0], "source_api_image": API_IMAGE,
                  "worker_generation": None, "worker_running": False, "worker_policy": None}
        with mock.patch.object(ops, "_current", return_value=(fixture.RELEASE, fixture.COMMIT)):
            with self.assertRaises(ops.OperationError):
                ops._retry_fencing(context, database, docker, mock.Mock(), record)
        self.assertFalse(any(row[0] in ("update", "kill", "wait", "start", "rm") for row in docker.calls))
        database.execute.assert_not_called()

    def test_exhausted_api_deadline_does_not_drain_or_audit(self):
        binding, docker, database = self.binding(), ApiDocker(), mock.Mock()
        moments = iter((1, 19))
        with mock.patch.object(ops.runtime, "capture", return_value=object()), \
             mock.patch.object(ops.runtime, "drain") as drain:
            with self.assertRaises(ops.OperationError):
                ops.drain_for_maintenance(docker, object(), database, {}, binding, API_IMAGE,
                                          clock=lambda: next(moments))
        drain.assert_not_called()
        database.execute.assert_not_called()
        self.assertFalse(any(v[0] in ("update", "kill", "wait") for v in docker.calls))

    def test_db_unknown_rejects_after_drain_without_resume(self):
        binding = self.binding()
        docker = ApiDocker()
        database = mock.Mock()
        database.execute.side_effect = RuntimeError("private synthetic DB error")
        snapshot = SimpleNamespace(generation=("a" * 64, "created", "started", 0), running=False, restart_policy="no")
        result = SimpleNamespace(generation=snapshot.generation)
        with mock.patch.object(ops.runtime, "capture", return_value=snapshot), \
             mock.patch.object(ops.runtime, "drain", return_value=result):
            with self.assertRaises(ops.OperationError) as caught:
                ops.drain_for_maintenance(docker, object(), database, {}, binding, API_IMAGE, clock=lambda: 1)
        self.assertNotIn("private synthetic DB error", str(caught.exception))
        self.assertFalse(any(v[0] in ("start", "restart", "rm") for v in docker.calls))
        self.assertEqual(database.execute.call_args.args[1], ops.SETTLEMENT_SQL)

    def test_actual_policy_rejects_pin_mount_health_and_limit_drift(self):
        binding = self.binding()
        value = fixture.make_container(binding, policy="no")
        value["Config"].update({
            "StopTimeout": 17,
            "Env": ["APP_ENV=production", "OWNER_BETA_SIMULATION_MODE=owner_only",
                    "OWNER_BETA_ACCESS_MODE=owner_only", "OWNER_BETA_PRICE_INPUT_MODE=sealed_v1",
                    "LAGRANGE_CODE_COMMIT=" + binding.commit,
                    "OWNER_BETA_SIMULATION_CONFIG_SHA256=" + binding.config_sha256,
                    "OWNER_BETA_SIMULATION_RECEIPT_SHA256=" + binding.receipt_sha256[7:],
                    "DB_HOST=postgres", "DB_PORT=5432", "DB_NAME=fixture", "DB_USER=worker",
                    "DB_PASSWORD_FILE=/run/secrets/db_worker_password"],
            "Healthcheck": {"Test": ["CMD", "/usr/local/bin/owner-beta-simulation-runner", "--health",
                "--expected-commit", binding.commit, "--expected-config-sha256", binding.config_sha256,
                "--expected-receipt-sha256", binding.receipt_sha256[7:]],
                "Interval": 1000000000, "Timeout": 1000000000, "StartPeriod": 1000000000, "Retries": 1},
        })
        value["HostConfig"].update({"CapDrop": ["ALL"], "CapAdd": None,
            "SecurityOpt": ["no-new-privileges:true"], "IpcMode": "private", "Memory": 1,
            "NanoCpus": 1, "PidsLimit": 1, "NetworkMode": "network-id",
            "Tmpfs": {"/run/owner-beta-simulation": "uid=0,gid=10001,mode=0750,size=4096",
                      "/run/owner-beta-simulation-health/records": "uid=10001,gid=10001,mode=0700,size=1"}})
        value["Mounts"].append({"Type": "bind", "Source": "/synthetic/secrets/owner-beta-simulation-runner/db_worker_password",
            "Destination": "/run/secrets/db_worker_password", "RW": False, "Propagation": "rprivate"})
        value["State"]["Health"] = {"Status": "healthy"}
        value["NetworkSettings"] = {"Networks": {"fixture": {"NetworkID": "network-id"}}}
        snapshot = SimpleNamespace(generation=(fixture.CONTAINER_ID, "created", "started", 0),
                                   running=True, restart_policy="no")
        mutations = (
            lambda v: v["Config"]["Env"].__setitem__(6, "OWNER_BETA_SIMULATION_RECEIPT_SHA256=" + "f" * 64),
            lambda v: v["Mounts"][-1].__setitem__("RW", True),
            lambda v: v["Config"]["Healthcheck"]["Test"].__setitem__(2, "--once"),
            lambda v: v["HostConfig"].__setitem__("PidsLimit", 2),
            lambda v: v["HostConfig"]["Tmpfs"].pop("/run/owner-beta-simulation"),
            lambda v: v["HostConfig"]["Tmpfs"].__setitem__("/run/owner-beta-simulation", "uid=10001,gid=10001,mode=0750,size=4096"),
            lambda v: v["HostConfig"]["Tmpfs"].__setitem__("/run/owner-beta-simulation", "uid=0,gid=10001,mode=0770,size=4096"),
            lambda v: v["NetworkSettings"]["Networks"]["fixture"].__setitem__("NetworkID", "foreign"),
        )
        docker = mock.Mock()
        with mock.patch.object(ops.daily.install, "root_directory", return_value=contextlib.nullcontext(1)), \
             mock.patch.object(ops.daily.install, "child_directory", return_value=contextlib.nullcontext(2)), \
             mock.patch.object(ops.daily.install, "read_file", return_value=b"synthetic"), \
             mock.patch.object(ops.daily, "internal_network", return_value={"Id": "network-id", "Name": "fixture"}), \
             mock.patch.object(ops.runtime, "capture", return_value=snapshot), \
             mock.patch.object(ops.runtime, "observe_unchanged", return_value=snapshot):
            docker.object.return_value = copy.deepcopy(value)
            self.assertIs(ops.verify_runtime_policy(docker, object(), binding, "/synthetic/secrets", "fixture"), snapshot)
            for mutation in mutations:
                candidate = copy.deepcopy(value)
                mutation(candidate)
                docker.object.return_value = candidate
                with self.assertRaises(ops.OperationError):
                    ops.verify_runtime_policy(docker, object(), binding, "/synthetic/secrets", "fixture")

    def test_real_cli_inspection_runs_inventory_and_preserves_legacy_off(self):
        release_path = "/synthetic/install/releases/" + fixture.COMMIT
        docker = ApiDocker()
        sim_docker = mock.Mock()
        sim_docker.service_ids.return_value = []
        database = mock.Mock()
        database.execute.return_value = "SIMULATION_HISTORY: EMPTY"
        images = {"api-server": API_IMAGE, "db-migrate": "sha256:" + "9" * 64}
        values = {"LAGRANGE_CODE_COMMIT": fixture.COMMIT}
        with contextlib.ExitStack() as stack:
            stack.enter_context(mock.patch.object(ops.daily.install, "require_root"))
            stack.enter_context(mock.patch.object(ops.daily, "inherited_lock", return_value=contextlib.nullcontext()))
            stack.enter_context(mock.patch.object(ops.daily.runtime, "Docker", return_value=docker))
            stack.enter_context(mock.patch.object(ops.runtime, "Docker", return_value=sim_docker))
            stack.enter_context(mock.patch.object(ops.daily, "read_manifest", return_value=images))
            stack.enter_context(mock.patch.object(ops, "_current", return_value=(release_path, fixture.COMMIT)))
            stack.enter_context(mock.patch.object(ops.daily, "project_binding", return_value=(fixture.PROJECT, "absent")))
            stack.enter_context(mock.patch.object(ops.daily, "read_context", return_value=(values, [])))
            stack.enter_context(mock.patch.object(ops.daily, "Database", return_value=database))
            stack.enter_context(mock.patch.object(ops, "_floor_present", return_value=False))
            stack.enter_context(mock.patch.object(ops, "_configured", return_value=False))
            reader = stack.enter_context(mock.patch.object(ops.release, "reader_guard", return_value="LEGACY_OFF"))
            output = stack.enter_context(contextlib.redirect_stdout(io.StringIO()))
            result = ops.main(["transition", "--install-root", "/synthetic/install", "--target-release", release_path,
                               "--commit", fixture.COMMIT, "--lock-fd", "9", "--action", "inspect"])
        self.assertEqual(result, 0)
        self.assertIn("SIMULATION_OPERATION: LEGACY_OFF", output.getvalue())
        self.assertEqual(database.execute.call_args.args[1], ops.HISTORY_SQL)
        self.assertFalse(reader.call_args.kwargs["history_present"])
        self.assertFalse(any(v[0] in ("update", "kill", "wait", "start", "rm") for v in docker.calls))

    def test_unavailable_history_inventory_cannot_fall_through_to_legacy(self):
        release_path = "/synthetic/install/releases/" + fixture.COMMIT
        docker, sim_docker, database = ApiDocker(), mock.Mock(), mock.Mock()
        sim_docker.service_ids.return_value = []
        database.execute.side_effect = RuntimeError("private synthetic database failure")
        with contextlib.ExitStack() as stack:
            stack.enter_context(mock.patch.object(ops.daily.install, "require_root"))
            stack.enter_context(mock.patch.object(ops.daily, "inherited_lock", return_value=contextlib.nullcontext()))
            stack.enter_context(mock.patch.object(ops.daily.runtime, "Docker", return_value=docker))
            stack.enter_context(mock.patch.object(ops.runtime, "Docker", return_value=sim_docker))
            stack.enter_context(mock.patch.object(ops.daily, "read_manifest", return_value={"api-server": API_IMAGE, "db-migrate": "sha256:" + "9" * 64}))
            stack.enter_context(mock.patch.object(ops, "_current", return_value=(release_path, fixture.COMMIT)))
            stack.enter_context(mock.patch.object(ops.daily, "project_binding", return_value=(fixture.PROJECT, "absent")))
            stack.enter_context(mock.patch.object(ops.daily, "read_context", return_value=({}, [])))
            stack.enter_context(mock.patch.object(ops.daily, "Database", return_value=database))
            stack.enter_context(mock.patch.object(ops, "_floor_present", return_value=False))
            stack.enter_context(mock.patch.object(ops, "_configured", return_value=False))
            reader = stack.enter_context(mock.patch.object(ops.release, "reader_guard"))
            errors = stack.enter_context(contextlib.redirect_stderr(io.StringIO()))
            result = ops.main(["transition", "--install-root", "/synthetic/install", "--target-release", release_path,
                               "--commit", fixture.COMMIT, "--lock-fd", "9", "--action", "inspect"])
        self.assertEqual(result, 1)
        reader.assert_not_called()
        self.assertNotIn("private synthetic", errors.getvalue())
        self.assertFalse(any(v[0] in ("update", "kill", "wait", "start", "rm") for v in docker.calls))

    def test_explicit_start_dispatches_pinned_overlay_and_derived_values(self):
        run = self.exercise_start()
        binding = self.binding()
        self.assertIsNone(run.failure)
        self.assertTrue(run.result.running)
        argv, options = run.calls[0]
        self.assertIn("up", argv)
        self.assertIn("--no-start", argv)
        self.assertIn("--no-deps", argv)
        self.assertIn("--no-build", argv)
        self.assertEqual(argv[argv.index("--pull") + 1], "never")
        self.assertNotIn("create", argv)
        self.assertIn(str(Path(ops.__file__).with_name("owner-beta-simulation-runtime-overlay.yml")), argv)
        self.assertEqual(options["env"]["RANGE_RAW_BATCH_ID"], "compose-config-disabled")
        self.assertEqual(options["env"]["COMPOSE_PROFILES"], "")
        self.assertIn("--no-recreate", argv)
        self.assertEqual(argv[-1], "owner-beta-simulation-runner")
        self.assertEqual(options["env"]["LAGRANGE_SIM_CONFIG_SHA256"], binding.config_sha256)
        self.assertEqual(options["env"]["LAGRANGE_SIM_RECEIPT_SHA256"], binding.receipt_sha256[7:])
        self.assertIn(binding.image_id.encode(), run.output.getvalue())
        self.assertNotIn(ops.START_PENDING, self.pending)

    def test_transition_uses_source_api_and_checks_target_reader_before_mutation(self):
        source = fixture.RELEASE
        target_commit = "b2" * 20
        target = "/srv/lagrange/releases/" + target_commit
        docker = ApiDocker()
        sim_docker = mock.Mock()
        sim_docker.service_ids.return_value = []
        source_images = {"api-server": API_IMAGE, "db-migrate": "sha256:" + "9" * 64}
        database = mock.Mock()
        database.execute.return_value = "SIMULATION_HISTORY: PRESENT"
        with contextlib.ExitStack() as stack:
            stack.enter_context(mock.patch.object(ops.daily, "read_manifest", return_value=source_images))
            stack.enter_context(mock.patch.object(ops, "_current", return_value=(source, fixture.COMMIT)))
            stack.enter_context(mock.patch.object(ops.daily, "project_binding", return_value=(fixture.PROJECT, "absent")))
            stack.enter_context(mock.patch.object(ops.daily, "read_context", return_value=({}, [])))
            factory = stack.enter_context(mock.patch.object(ops.daily, "Database", return_value=database))
            stack.enter_context(mock.patch.object(ops, "_floor_present", return_value=True))
            stack.enter_context(mock.patch.object(ops, "_configured", side_effect=[True, False]))
            stack.enter_context(mock.patch.object(ops, "binding_from_release", return_value=self.binding()))
            reader = stack.enter_context(mock.patch.object(ops.release, "reader_guard", return_value="SIMULATION_READER_REQUIRED"))
            result, _ = ops.inspect_transition("/srv/lagrange", target, target_commit, docker, sim_docker)
        self.assertEqual(result.source_commit, fixture.COMMIT)
        self.assertEqual(result.target_commit, target_commit)
        self.assertEqual(factory.call_args.args[2], fixture.COMMIT)
        self.assertEqual(reader.call_args.args[1:3], (target, target_commit))
        self.assertTrue(reader.call_args.kwargs["history_present"])
        self.assertFalse(any(row[0] in ("update", "kill", "wait") for row in docker.calls))

    def test_explicit_worker_start_rejects_unknown_settlement_before_mutation(self):
        context = SimpleNamespace(required=True, binding=self.binding(), source_release=fixture.RELEASE,
            target_release=fixture.RELEASE, source_commit=fixture.COMMIT, target_commit=fixture.COMMIT,
            install_root="/synthetic/install",
            values={"OWNER_BETA_ACCESS_MODE": "owner_only", "OWNER_BETA_PRICE_INPUT_MODE": "sealed_v1",
                    "OWNER_BETA_PAPER_MODE": "disabled", "POSTGRES_DB": "fixture",
                    "LAGRANGE_RUNTIME_SECRET_DIR": "/synthetic/secrets"})
        database = mock.Mock()
        database.execute.side_effect = RuntimeError("unknown")
        with contextlib.ExitStack() as stack:
            stack.enter_context(mock.patch.object(ops, "validate_worker_secret"))
            empty = ops.runtime.Snapshot(ops.runtime._binding_identity(context.binding), None, None, False)
            stack.enter_context(mock.patch.object(ops.runtime, "capture", return_value=empty))
            start = stack.enter_context(mock.patch.object(ops.subprocess, "run"))
            floor = stack.enter_context(mock.patch.object(ops.release, "reader_guard"))
            with self.assertRaises(RuntimeError):
                ops.start_simulation(context, ApiDocker(), mock.Mock(), database)
            start.assert_not_called()
            floor.assert_not_called()

    def test_explicit_worker_start_reuses_only_the_bound_stopped_generation(self):
        run = self.exercise_start(existing=True)
        self.assertIsNone(run.failure)
        self.assertEqual(run.result.generation[0], fixture.CONTAINER_ID)
        self.assertEqual(run.calls, [])
        self.assertEqual([row for row in run.docker.calls if row[0] == "start"], [("start", fixture.CONTAINER_ID)])

    def test_start_failure_matrix_preserves_ownership_and_independent_uncertainty(self):
        for outcome in ("create-lost", "create-nonzero", "create-foreign", "create-empty", "lost", "interrupt", "observation", "health",
                        "timeout", "replacement", "cleanup"):
            with self.subTest(outcome=outcome):
                self.pending.clear()
                run = self.exercise_start(outcome)
                self.assertIsInstance(run.failure, ops.StartFailure)
                record = self.pending[ops.START_PENDING]
                self.assertTrue(record["primary_failure"])
                self.assertEqual(record["interrupted"], outcome == "interrupt")
                self.assertEqual(record["start_uncertain"], outcome in ("lost", "interrupt"))
                self.assertEqual(record["cleanup_uncertain"], outcome in ("create-foreign", "replacement", "cleanup"))
                if outcome in ("create-foreign", "create-empty"):
                    self.assertIsNone(record["generation"])
                if outcome in ("create-lost", "create-nonzero", "create-foreign", "create-empty"):
                    self.assertFalse(any(v[0] == "start" for v in run.docker.calls))
                if outcome == "replacement":
                    self.assertFalse(any(v[0] == "kill" for v in run.worker.calls))
                elif outcome not in ("create-lost", "create-nonzero", "create-foreign", "create-empty"):
                    self.assertIn(("kill", "--signal=TERM", fixture.CONTAINER_ID), run.worker.calls)
                with self.assertRaises(ops.OperationError):
                    ops._no_pending("/synthetic/install")

    def test_explicit_reconciliation_fences_api_and_resolves_only_recorded_worker(self):
        run = self.exercise_start("lost")
        self.assertTrue(run.failure.outcome_unknown)
        binding = self.binding()
        images = {"api-server": API_IMAGE, "db-migrate": "sha256:" + "9" * 64}
        values = {"OWNER_BETA_ACCESS_MODE": "owner_only", "OWNER_BETA_PRICE_INPUT_MODE": "sealed_v1",
                  "OWNER_BETA_PAPER_MODE": "disabled", "POSTGRES_DB": "fixture",
                  "LAGRANGE_RUNTIME_SECRET_DIR": "/synthetic/secrets", "LAGRANGE_CODE_COMMIT": fixture.COMMIT}
        with contextlib.ExitStack() as stack:
            stack.enter_context(mock.patch.object(ops, "_current", return_value=(fixture.RELEASE, fixture.COMMIT)))
            stack.enter_context(mock.patch.object(ops, "_configured", return_value=True))
            stack.enter_context(mock.patch.object(ops.daily, "read_manifest", return_value=images))
            stack.enter_context(mock.patch.object(ops.daily, "read_context", return_value=(values, [])))
            stack.enter_context(mock.patch.object(ops.daily, "project_binding", return_value=(fixture.PROJECT, "absent")))
            stack.enter_context(mock.patch.object(ops.daily, "Database", return_value=run.database))
            stack.enter_context(mock.patch.object(ops, "binding_from_release", return_value=binding))
            stack.enter_context(mock.patch.object(ops.daily.install, "read_root_file", return_value=b"synthetic manifest"))
            stack.enter_context(mock.patch.object(ops.release, "reader_guard"))
            run.docker.container["Created"] = "replacement"
            with self.assertRaises(ops.OperationError):
                ops.reconcile_start("/srv/lagrange", fixture.RELEASE, fixture.COMMIT, run.docker, run.worker)
            self.assertFalse(any(row[0] in ("update", "kill", "wait") for row in run.docker.calls))
            run.docker.container["Created"] = "created"
            self.assertEqual(ops.reconcile_start("/srv/lagrange", fixture.RELEASE, fixture.COMMIT,
                                                 run.docker, run.worker), "RECONCILED_OFF")
        self.assertNotIn(ops.START_PENDING, self.pending)
        self.assertEqual(self.pending[ops.HANDOFF_PENDING]["phase"], "switched")
        self.assertFalse(run.worker.container["State"]["Running"])
        self.assertFalse(run.docker.container["State"]["Running"])
        self.pending.clear()
        unknown = self.exercise_start("create-foreign")
        with self.assertRaises(ops.OperationError):
            ops._no_pending("/srv/lagrange")
        self.assertFalse(any(row[0] in ("update", "kill", "wait") for row in unknown.docker.calls))

    def test_start_diagnostics_keep_primary_response_and_cleanup_facts_separate(self):
        failure = ops.StartFailure(interrupted=True, outcome_unknown=True, cleanup_unknown=True)
        errors = io.StringIO()
        with mock.patch.object(ops, "transition", side_effect=failure), contextlib.redirect_stderr(errors):
            result = ops.main(["transition", "--install-root", "/synthetic/install", "--target-release",
                               fixture.RELEASE, "--commit", fixture.COMMIT, "--lock-fd", "9", "--action", "start"])
        self.assertEqual(result, 1)
        self.assertEqual(errors.getvalue().splitlines(), ["SIMULATION_OPERATION_START_FAILED",
            "SIMULATION_OPERATION_INTERRUPTED", "SIMULATION_OPERATION_START_OUTCOME_UNKNOWN",
            "SIMULATION_OPERATION_CLEANUP_UNCONFIRMED"])

    def test_handoff_accepts_compose_disabled_defaults_for_an_older_reader_target(self):
        docker, worker = ApiDocker(), mock.Mock()
        worker.service_ids.return_value = []
        docker.container["Config"]["Env"] = ["OWNER_BETA_ACCESS_MODE=disabled",
            "OWNER_BETA_PRICE_INPUT_MODE=disabled", "LAGRANGE_CODE_COMMIT=" + fixture.COMMIT]
        docker.container["Config"]["Labels"]["org.lagrange.api-release-attempt"] = "a" * 32
        context = SimpleNamespace(project=fixture.PROJECT, source_commit=fixture.COMMIT,
                                  target_commit=fixture.COMMIT, target_release=fixture.RELEASE, binding=None)
        record = {"target_api_image": API_IMAGE, "api_generation": [API_ID, "created", "started", 0],
                  "target_api_generation": None, "worker_generation": None, "api_attempt": "a" * 32}
        with mock.patch.object(ops.daily, "read_context", return_value=({}, [])):
            result = ops._handoff_target_running(context, docker, worker, record)
        self.assertEqual(result.identity, API_ID)
        self.assertFalse(any(row[0] in ("update", "kill", "wait", "start", "rm") for row in docker.calls))

    def test_same_release_resume_refuses_cross_release_and_stopped_worker_handoffs(self):
        root, commit, prior = "/synthetic/install", fixture.COMMIT, "b2" * 20
        target = root + "/releases/" + commit
        context = object()
        database = object()
        api = (context, database, None)
        handoff = {"source_commit": commit, "target_commit": commit, "phase": "switched",
                   "worker_generation": None, "worker_running": False}
        with mock.patch.object(ops.daily.install, "require_root"), \
                mock.patch.object(ops.daily, "inherited_lock", return_value=contextlib.nullcontext()), \
                mock.patch.object(ops.daily.runtime, "Docker"), mock.patch.object(ops.runtime, "Docker"), \
                mock.patch.object(ops, "_handoff_read") as read, \
                mock.patch.object(ops, "_handoff_context", return_value=api) as bound, \
                mock.patch.object(ops, "_current", return_value=(target, commit)), \
                mock.patch.object(ops, "_handoff_stopped") as stopped:
            for value in (None, dict(handoff, source_commit=prior),
                          dict(handoff, worker_generation=["stopped"])):
                read.return_value = value
                with self.subTest(value=value), self.assertRaisesRegex(
                        ops.OperationError, "SIMULATION_OPERATION_HANDOFF"):
                    ops.transition(root, target, commit, "9", action="ready", same_release_resume=True)
            bound.assert_not_called()
            stopped.assert_not_called()
            read.return_value = handoff
            self.assertEqual(ops.transition(root, target, commit, "9", action="ready", same_release_resume=True),
                             "RELEASE_START_READY")
            bound.assert_called_once()
            stopped.assert_called_once()

    def test_cross_commit_activation_release_start_and_worker_handoff(self):
        first, second = fixture.COMMIT, "b2" * 20
        for source_commit, target_commit in ((first, second), (second, first)):
            for has_worker, create_outcome in ((False, "success"), (True, "success"),
                    (True, "lost-before"), (True, "lost-after"), (True, "nonzero-after"),
                    (True, "handoff-write")):
                with self.subTest(source=source_commit, target=target_commit, worker=has_worker,
                                  create_outcome=create_outcome):
                    self.pending.clear()
                    root = "/synthetic/install"
                    def make_binding(commit):
                        config = fixture.config_value()
                        config["code_commit"] = commit
                        config["image_id"] = "sha256:" + ("1" if commit == first else "2") * 64
                        return ops.runtime.Binding(fixture.PROJECT, root + "/releases/" + commit,
                                                   commit, fixture.config_bytes(config))
                    bindings = {c: make_binding(c) for c in (first, second)}
                    api_images = {c: "sha256:" + ("e" if c == first else "f") * 64 for c in (first, second)}
                    worker = WorkerTransport(bindings[source_commit])
                    events = []
                    worker_call = worker.call
                    def traced_worker_call(args, timeout):
                        events.append(("worker", *args))
                        return worker_call(args, timeout)
                    worker.call = traced_worker_call
                    if has_worker:
                        worker.container = worker_value(bindings[source_commit], state="running")
                    class Docker(ApiDocker):
                        def __init__(self):
                            super().__init__()
                            self.set_api(source_commit, API_ID, running=True)
                        def set_api(self, commit, identity, *, running):
                            self.container = api_container()
                            self.container.update(Id=identity, Image=api_images[commit])
                            self.container["Config"]["Labels"]["org.opencontainers.image.revision"] = commit
                            self.container["Config"]["Env"] = ["OWNER_BETA_ACCESS_MODE=owner_only",
                                "OWNER_BETA_PRICE_INPUT_MODE=sealed_v1", "LAGRANGE_CODE_COMMIT=" + commit]
                            self.container["State"].update(Running=running, Status="running" if running else "exited")
                        def object(self, kind, identity, timeout):
                            if kind == "image":
                                commit = next(c for c, image in api_images.items() if image == identity)
                                return {"Id": identity, "Config": {"User": "10001:10001",
                                    "Entrypoint": ["/usr/local/bin/api-server"], "Cmd": None,
                                    "Labels": {"org.opencontainers.image.revision": commit,
                                               "org.lagrange.api.http-drain-contract": "v1"}}}
                            if worker.container is not None and identity == worker.container["Id"]:
                                return copy.deepcopy(worker.container)
                            return super().object(kind, identity, timeout)
                        def call(self, args, timeout):
                            if args[0] == "start" and worker.container is not None and args[1] == worker.container["Id"]:
                                self.calls.append(tuple(args))
                                worker.container["State"].update(Running=True, Status="running", StartedAt="after")
                                worker.container["NetworkSettings"]["Networks"] = {"fixture": {"NetworkID": "network-id"}}
                                return args[1] + "\n"
                            if args[0] in ("update", "kill", "wait"):
                                events.append(("api", *args))
                            return super().call(args, timeout)
                    docker = Docker()
                    database = mock.Mock()
                    def execute(_values, sql, _allowed):
                        if sql == ops.HISTORY_SQL: return "SIMULATION_HISTORY: PRESENT"
                        events.append(("database-settlement",))
                        return "SIMULATION_MAINTENANCE: SETTLED"
                    database.execute.side_effect = execute
                    current = [root + "/releases/" + source_commit, source_commit]
                    def switch(_src, _dst, **_kwargs): current[:] = [root + "/releases/" + target_commit, target_commit]
                    def values(path, commit, _action):
                        return ({"OWNER_BETA_ACCESS_MODE": "owner_only", "OWNER_BETA_PRICE_INPUT_MODE": "sealed_v1",
                            "OWNER_BETA_PAPER_MODE": "disabled", "POSTGRES_DB": "fixture",
                            "LAGRANGE_RUNTIME_SECRET_DIR": "/synthetic/secrets", "LAGRANGE_CODE_COMMIT": commit}, [])
                    def image_manifest(_path, commit):
                        return {"api-server": api_images[commit], "db-migrate": "sha256:" + "9" * 64}
                    class Output(io.BytesIO):
                        def fileno(self): return 71
                        def close(self): pass
                    create_calls = []
                    def create(argv, **_kwargs):
                        create_calls.append(argv)
                        self.assertIsNone(self.pending[ops.START_PENDING]["generation"])
                        self.assertEqual(self.pending[ops.START_PENDING]["phase"], "creating")
                        if create_outcome == "lost-before":
                            raise TimeoutError("synthetic create response loss before replacement")
                        worker.binding = bindings[target_commit]
                        worker.container = worker_value(worker.binding, identity=fixture.OTHER_CONTAINER_ID)
                        worker.container["NetworkSettings"]["Networks"] = {}
                        worker.container["Config"]["Labels"]["org.lagrange.simulation-start-attempt"] = (b"a" * 16).hex()
                        if create_outcome == "lost-after":
                            raise TimeoutError("synthetic create response loss after replacement")
                        if create_outcome == "nonzero-after":
                            return SimpleNamespace(returncode=1)
                        return SimpleNamespace(returncode=0)
                    with contextlib.ExitStack() as stack:
                        stack.enter_context(mock.patch.object(ops.daily.install, "require_root"))
                        stack.enter_context(mock.patch.object(ops.daily, "inherited_lock", return_value=contextlib.nullcontext()))
                        stack.enter_context(mock.patch.object(ops.daily.runtime, "Docker", return_value=docker))
                        stack.enter_context(mock.patch.object(ops.runtime, "Docker", return_value=worker))
                        stack.enter_context(mock.patch.object(ops, "_current", side_effect=lambda _root: tuple(current)))
                        stack.enter_context(mock.patch.object(ops.daily, "read_manifest", side_effect=image_manifest))
                        stack.enter_context(mock.patch.object(ops.daily, "read_context", side_effect=values))
                        stack.enter_context(mock.patch.object(ops.daily, "project_binding", return_value=(fixture.PROJECT, "absent")))
                        stack.enter_context(mock.patch.object(ops.daily, "Database", return_value=database))
                        stack.enter_context(mock.patch.object(ops.daily.install, "read_root_file", side_effect=lambda p, **_: p.encode()))
                        stack.enter_context(mock.patch.object(ops.daily.install, "root_directory", return_value=contextlib.nullcontext(99)))
                        stack.enter_context(mock.patch.object(ops, "_floor_present", return_value=True))
                        stack.enter_context(mock.patch.object(ops, "_configured", return_value=True))
                        stack.enter_context(mock.patch.object(ops, "binding_from_release", side_effect=lambda _p, _r, c: bindings[c]))
                        reader_guard = stack.enter_context(mock.patch.object(ops.release, "reader_guard"))
                        stack.enter_context(mock.patch.object(ops, "validate_worker_secret"))
                        stack.enter_context(mock.patch.object(ops, "input_guard", return_value=contextlib.nullcontext()))
                        stack.enter_context(mock.patch.object(ops.daily, "internal_network", return_value={"Id": "network-id", "Name": "fixture"}))
                        stack.enter_context(mock.patch.object(ops.os, "symlink"))
                        stack.enter_context(mock.patch.object(ops.os, "replace", side_effect=switch))
                        fsync = stack.enter_context(mock.patch.object(ops.os, "fsync"))
                        stack.enter_context(mock.patch.object(ops.os, "fchmod"))
                        stack.enter_context(mock.patch.object(ops.os, "fdopen", return_value=Output()))
                        stack.enter_context(mock.patch.object(ops.os, "unlink"))
                        stack.enter_context(mock.patch.object(ops.os, "urandom", return_value=b"a" * 16))
                        stack.enter_context(mock.patch.object(ops.tempfile, "mkstemp", return_value=(71, "/synthetic/override")))
                        stack.enter_context(mock.patch.object(ops.time, "monotonic", return_value=1.0))
                        stack.enter_context(mock.patch.object(ops.subprocess, "run", side_effect=create))
                        target = root + "/releases/" + target_commit
                        if source_commit == second and has_worker:
                            docker.fail_update = True
                            with self.assertRaises(ops.OperationError):
                                ops.transition(root, target, target_commit, "9", action="activate")
                            self.assertEqual(self.pending[ops.HANDOFF_PENDING]["phase"], "fencing")
                            self.assertTrue(worker.container["State"]["Running"])
                            docker.fail_update = False
                        if source_commit == first and has_worker:
                            fsync.side_effect = OSError("synthetic interrupted switch")
                            with self.assertRaises(OSError):
                                ops.transition(root, target, target_commit, "9", action="activate")
                            self.assertEqual(tuple(current), (target, target_commit))
                            self.assertEqual(self.pending[ops.HANDOFF_PENDING]["phase"], "prepared")
                            fsync.side_effect = None
                        self.assertEqual(ops.transition(root, target, target_commit, "9", action="activate"), "ACTIVATED_OFF")
                        api_term = events.index(("api", "kill", "--signal=TERM", API_ID))
                        settlement = events.index(("database-settlement",))
                        self.assertLess(api_term, settlement)
                        if has_worker:
                            worker_term = events.index(("worker", "kill", "--signal=TERM", fixture.CONTAINER_ID))
                            self.assertLess(api_term, worker_term)
                            self.assertLess(worker_term, settlement)
                        self.assertEqual(self.pending[ops.HANDOFF_PENDING]["phase"], "switched")
                        saved = copy.deepcopy(self.pending[ops.HANDOFF_PENDING])
                        self.pending[ops.HANDOFF_PENDING]["target_manifest"] = "sha256:" + "0" * 64
                        with self.assertRaises(ops.OperationError):
                            ops.transition(root, target, target_commit, "9", action="ready")
                        self.pending[ops.HANDOFF_PENDING] = copy.deepcopy(saved)
                        reader_guard.side_effect = ops.OperationError("incompatible reader")
                        with self.assertRaises(ops.OperationError):
                            ops.transition(root, target, target_commit, "9", action="ready")
                        reader_guard.side_effect = None
                        if has_worker:
                            worker.container["Created"] = "replacement"
                            with self.assertRaises(ops.OperationError):
                                ops.transition(root, target, target_commit, "9", action="ready")
                            worker.container["Created"] = "worker-created"
                        del self.pending[ops.HANDOFF_PENDING]
                        with self.assertRaises(ops.OperationError):
                            ops.transition(root, target, target_commit, "9", action="ready")
                        self.pending[ops.HANDOFF_PENDING] = saved
                        self.assertEqual(ops.transition(root, target, target_commit, "9", action="ready"), "RELEASE_START_READY")
                        outcome = ops.transition(root, target, target_commit, "9", action="begin-release")
                        self.assertEqual(outcome, "RELEASE_START_BEGUN " + (b"a" * 16).hex())
                        with self.assertRaises(ops.OperationError):
                            ops.transition(root, target, target_commit, "9", action="ready")
                        if source_commit == first:
                            self.assertEqual(ops.transition(root, target, target_commit, "9", action="reconcile-release"),
                                             "RELEASE_RETRY_READY")
                            self.assertEqual(ops.transition(root, target, target_commit, "9", action="begin-release"), outcome)
                        docker.set_api(target_commit, "f" * 64, running=True)
                        docker.container["Config"]["Labels"]["org.lagrange.api-release-attempt"] = "0" * 32
                        with self.assertRaises(ops.OperationError):
                            ops.transition(root, target, target_commit, "9", action="reconcile-release")
                        self.assertEqual(self.pending[ops.HANDOFF_PENDING]["phase"], "api-starting")
                        docker.container["Config"]["Labels"]["org.lagrange.api-release-attempt"] = (b"a" * 16).hex()
                        action = "reconcile-release" if source_commit == second else "finish-release"
                        expected = "RELEASE_START_OBSERVED" if action == "reconcile-release" else "RELEASE_START_FINISHED"
                        self.assertEqual(ops.transition(root, target, target_commit, "9", action=action), expected)
                        if has_worker:
                            self.assertEqual(self.pending[ops.HANDOFF_PENDING]["phase"], "api-started")
                            docker.container["RestartCount"] = 1
                            with self.assertRaises(ops.OperationError):
                                ops.transition(root, target, target_commit, "9", action="start")
                            docker.container["RestartCount"] = 0
                            if create_outcome == "success":
                                self.assertEqual(ops.transition(root, target, target_commit, "9", action="start"), "STARTED")
                            else:
                                original_write = ops._pending_write.side_effect
                                def fail_handoff_write(root_arg, name, value, *, previous=None):
                                    if name == ops.HANDOFF_PENDING and value["phase"] == "worker-replacing":
                                        raise OSError("synthetic interrupted handoff phase write")
                                    return original_write(root_arg, name, value, previous=previous)
                                if create_outcome == "handoff-write":
                                    ops._pending_write.side_effect = fail_handoff_write
                                try:
                                    with self.assertRaises(ops.StartFailure):
                                        ops.transition(root, target, target_commit, "9", action="start")
                                finally:
                                    ops._pending_write.side_effect = original_write
                                self.assertIn(ops.START_PENDING, self.pending)
                                self.assertFalse(any(row[0] == "start" for row in docker.calls))
                                with self.assertRaises(ops.OperationError):
                                    ops.transition(root, target, target_commit, "9", action="start")
                                worker_before = copy.deepcopy(worker.container)
                                before_calls = len(docker.calls)
                                reconciled = ops.transition(root, target, target_commit, "9", action="reconcile-start")
                                if create_outcome in ("lost-before", "handoff-write"):
                                    self.assertEqual(reconciled, "RECONCILED_WORKER_STOPPED")
                                    self.assertEqual(self.pending[ops.HANDOFF_PENDING]["phase"], "api-started")
                                    self.assertEqual(worker.container, worker_before)
                                    self.assertFalse(any(row[0] in ("start", "update", "kill", "wait")
                                                         for row in docker.calls[before_calls:]))
                                else:
                                    self.assertEqual(reconciled, "RECONCILED_OFF")
                                    self.assertEqual(self.pending[ops.HANDOFF_PENDING]["phase"], "switched")
                                    self.assertFalse(worker.container["State"]["Running"])
                                    self.assertFalse(docker.container["State"]["Running"])
                                self.assertNotIn(ops.START_PENDING, self.pending)
                                self.assertFalse(any(row[0] == "start" for row in docker.calls))
                            if create_outcome != "handoff-write":
                                self.assertNotIn("--no-recreate", create_calls[-1])
                        else:
                            self.assertNotIn(ops.HANDOFF_PENDING, self.pending)
                        self.assertNotIn(ops.START_PENDING, self.pending)


    # ---- failed-start disposition -------------------------------------------------

    def dispose_run(self, *, handoff="worker-replacing"):
        """A real exit-9 start failure plus the protected records production holds."""
        run = self.exercise_start("exit-nonzero")
        self.assertIsInstance(run.failure, ops.StartFailure)
        pending = self.pending[ops.START_PENDING]
        run.events = []
        run.rm_calls = []
        run.database.execute.side_effect = lambda *_a: (run.events.append("settle"),
                                                         run.database.execute.result)[1]
        run.database.execute.result = "SIMULATION_MAINTENANCE: SETTLED"
        run.docker.container["Config"]["Labels"]["org.lagrange.api-release-attempt"] = "c" * 32
        if handoff is not None:
            self.pending[ops.HANDOFF_PENDING] = self.make_handoff(pending, handoff)
        real_call = run.docker.call
        def call(args, timeout):
            if args[0] == "rm":
                run.docker.calls.append(tuple(args))
                run.rm_calls.append(tuple(args))
                run.events.append("rm")
                if run.rm_hook is not None: run.rm_hook(args)
                run.worker.container = None
                return args[1] + "\n"
            return real_call(args, timeout)
        run.rm_hook = None
        run.docker.call = call
        return run

    def make_handoff(self, pending, phase="worker-replacing"):
        binding = self.binding()
        return {"schema": "owner-beta-simulation-handoff-v1", "phase": phase, "project": fixture.PROJECT,
                "source_commit": "b2" * 20, "target_commit": fixture.COMMIT,
                "source_manifest": "sha256:" + "a" * 64, "target_manifest": self.manifest_digest(),
                "source_api_image": "sha256:" + "b" * 64, "target_api_image": API_IMAGE,
                "source_binding": {"image": "sha256:" + "1" * 64, "config": "sha256:" + "2" * 64,
                                   "receipt": "sha256:" + "3" * 64},
                "target_binding": ops._binding_pins(binding),
                "api_generation": ["9" * 64, "old-created", "old-started", 0],
                "worker_generation": ["8" * 64, "old-worker-created", "old-worker-started", 0],
                "worker_running": False, "worker_policy": "no",
                "target_api_generation": (list(pending["api_generation"]) if phase in ("api-started", "worker-replacing")
                                          else None),
                "api_attempt": "c" * 32}

    @staticmethod
    def manifest_digest():
        import hashlib
        return "sha256:" + hashlib.sha256(b"synthetic manifest").hexdigest()

    def dispose(self, run, *, current=None, commit=fixture.COMMIT):
        images = {"api-server": API_IMAGE, "db-migrate": "sha256:" + "9" * 64}
        values = {"OWNER_BETA_ACCESS_MODE": "owner_only", "OWNER_BETA_PRICE_INPUT_MODE": "sealed_v1",
                  "OWNER_BETA_PAPER_MODE": "disabled", "POSTGRES_DB": "fixture",
                  "LAGRANGE_RUNTIME_SECRET_DIR": "/synthetic/secrets", "LAGRANGE_CODE_COMMIT": fixture.COMMIT}
        with contextlib.ExitStack() as stack:
            stack.enter_context(mock.patch.object(ops, "_current",
                return_value=current or (fixture.RELEASE, fixture.COMMIT)))
            stack.enter_context(mock.patch.object(ops, "_configured", return_value=True))
            stack.enter_context(mock.patch.object(ops.daily, "read_manifest", return_value=images))
            stack.enter_context(mock.patch.object(ops.daily, "read_context", return_value=(values, [])))
            stack.enter_context(mock.patch.object(ops.daily, "project_binding", return_value=(fixture.PROJECT, "absent")))
            stack.enter_context(mock.patch.object(ops.daily, "Database", return_value=run.database))
            stack.enter_context(mock.patch.object(ops, "binding_from_release", return_value=self.binding()))
            stack.enter_context(mock.patch.object(ops.daily.install, "read_root_file", return_value=b"synthetic manifest"))
            return ops.dispose_failed_start("/srv/lagrange", fixture.RELEASE, commit, run.docker, run.worker)

    READ_ONLY_API_VERBS = ("inspect", "ps")

    def api_mutations(self, run, mark=0):
        """Every API-transport call that is not a read-only verb; any such call fails API-safety."""
        return [row for row in run.docker.calls[mark:] if row[0] not in self.READ_ONLY_API_VERBS]

    def test_nonzero_exit_leaves_exact_failed_record(self):
        run = self.exercise_start("exit-nonzero")
        self.assertIsInstance(run.failure, ops.StartFailure)
        record = self.pending[ops.START_PENDING]
        self.assertEqual(record["phase"], "failed")
        self.assertTrue(record["primary_failure"])
        self.assertFalse(record["interrupted"])
        self.assertFalse(record["start_uncertain"])
        self.assertTrue(record["cleanup_uncertain"])
        self.assertEqual(record["generation"], [fixture.CONTAINER_ID, "worker-created", "worker-before", 0])
        self.assertNotEqual(run.worker.container["State"]["StartedAt"], record["generation"][2])
        self.assertEqual(run.worker.container["State"]["ExitCode"], 9)
        self.assertFalse(any(row[0] in ("kill", "wait", "rm") for row in run.worker.calls))

    def test_failed_start_is_disposed_with_and_without_handoff(self):
        for handoff in ("worker-replacing", None):
            with self.subTest(handoff=handoff):
                self.pending.clear()
                run = self.dispose_run(handoff=handoff)
                pending = copy.deepcopy(self.pending[ops.START_PENDING])
                handoff_value = copy.deepcopy(self.pending.get(ops.HANDOFF_PENDING))
                mark = len(run.docker.calls)
                self.assertEqual(self.dispose(run), "FAILED_START_DISPOSED")
                self.assertNotIn(ops.START_PENDING, self.pending)
                self.assertNotIn(ops.HANDOFF_PENDING, self.pending)
                name = ops._disposition_name(pending["attempt"])
                record = self.pending[name]
                self.assertEqual(record["start_pending"], pending)
                self.assertEqual(record["handoff"], handoff_value)
                self.assertEqual(record["container"]["exit_code"], 9)
                self.assertEqual(record["container"]["generation"][0], fixture.CONTAINER_ID)
                self.assertEqual(record["container"]["labels"]["org.lagrange.simulation-start-attempt"], pending["attempt"])
                self.assertEqual(record["api_generation"], pending["api_generation"])
                self.assertNotIn("time", " ".join(record))
                self.assertEqual(run.rm_calls, [("rm", fixture.CONTAINER_ID)])
                self.assertEqual(run.events, ["settle", "rm", "settle"])
                self.assertEqual(self.api_mutations(run, mark), [("rm", fixture.CONTAINER_ID)])
                self.assertFalse(any(row[0] in ("update", "kill", "wait")
                                     for row in run.worker.calls + run.docker.calls))
                self.assertTrue(run.docker.container["State"]["Running"])
                self.assertEqual(run.database.execute.call_args.args[1], ops.SETTLEMENT_SQL)

    DISPOSE_REJECTS = {
        "running": lambda t, r, p, h: r.worker.container["State"].update(Running=True, Status="running"),
        "exit-zero": lambda t, r, p, h: r.worker.container["State"].update(ExitCode=0),
        "created": lambda t, r, p, h: r.worker.container["State"].update(Status="created"),
        "attempt-label": lambda t, r, p, h: r.worker.container["Config"]["Labels"].update(
            {"org.lagrange.simulation-start-attempt": "f" * 32}),
        "generation-id": lambda t, r, p, h: p["generation"].__setitem__(0, fixture.OTHER_CONTAINER_ID),
        "generation-created": lambda t, r, p, h: p["generation"].__setitem__(1, "other"),
        "generation-restarts": lambda t, r, p, h: p["generation"].__setitem__(3, 1),
        "container-restarts": lambda t, r, p, h: r.worker.container.__setitem__("RestartCount", 1),
        "container-created": lambda t, r, p, h: r.worker.container.__setitem__("Created", "other"),
        "restart-policy": lambda t, r, p, h: r.worker.container["HostConfig"]["RestartPolicy"].update(Name="unless-stopped"),
        "pending-missing": lambda t, r, p, h: t.pending.pop(ops.START_PENDING),
        "phase": lambda t, r, p, h: p.update(phase="owned"),
        "start-uncertain": lambda t, r, p, h: p.update(start_uncertain=True),
        "interrupted": lambda t, r, p, h: p.update(interrupted=True),
        "primary-failure": lambda t, r, p, h: p.update(primary_failure=False),
        "null-generation": lambda t, r, p, h: p.update(generation=None),
        "commit": lambda t, r, p, h: p.update(commit="b2" * 20),
        "config-pin": lambda t, r, p, h: p.update(config="sha256:" + "0" * 64),
        "image-pin": lambda t, r, p, h: p.update(image="sha256:" + "0" * 64),
        "receipt-pin": lambda t, r, p, h: p.update(receipt="sha256:" + "0" * 64),
        "api-image-pin": lambda t, r, p, h: p.update(api_image="sha256:" + "0" * 64),
        "handoff-phase": lambda t, r, p, h: t.pending.__setitem__(ops.HANDOFF_PENDING, t.make_handoff(p, "api-started")),
        "handoff-switched": lambda t, r, p, h: t.pending.__setitem__(ops.HANDOFF_PENDING, t.make_handoff(p, "switched")),
        "handoff-target-commit": lambda t, r, p, h: h.update(target_commit="b2" * 20),
        "handoff-api-generation": lambda t, r, p, h: h["target_api_generation"].__setitem__(3, 5),
        "handoff-manifest": lambda t, r, p, h: h.update(target_manifest="sha256:" + "0" * 64),
        "handoff-project": lambda t, r, p, h: h.update(project="other-project"),
        "handoff-api-image": lambda t, r, p, h: h.update(target_api_image="sha256:" + "0" * 64),
        "handoff-binding": lambda t, r, p, h: h.update(target_binding=dict(h["target_binding"], config="sha256:" + "0" * 64)),
        "api-restarted": lambda t, r, p, h: r.docker.container.__setitem__("RestartCount", 1),
        "api-stopped": lambda t, r, p, h: r.docker.container["State"].update(Running=False, Status="exited"),
        "api-attempt-label": lambda t, r, p, h: r.docker.container["Config"]["Labels"].update(
            {"org.lagrange.api-release-attempt": "0" * 32}),
        "api-replaced": lambda t, r, p, h: r.docker.container.__setitem__("Id", "e" * 64),
        "unsettled": lambda t, r, p, h: setattr(r.database.execute, "result", "SIMULATION_MAINTENANCE: OPEN"),
        "foreign-container": lambda t, r, p, h: r.worker.container.__setitem__("Id", fixture.OTHER_CONTAINER_ID),
        "two-containers": lambda t, r, p, h: setattr(r.worker, "service_ids",
            lambda timeout: [fixture.CONTAINER_ID, fixture.OTHER_CONTAINER_ID]),
        "oom": lambda t, r, p, h: r.worker.container["State"].update(OOMKilled=True),
        "error": lambda t, r, p, h: r.worker.container["State"].update(Error="boom"),
        "restarting": lambda t, r, p, h: r.worker.container["State"].update(Restarting=True),
    }

    def test_disposition_rejects_every_unsafe_state_before_any_mutation(self):
        for name, mutate in self.DISPOSE_REJECTS.items():
            with self.subTest(case=name):
                self.pending.clear()
                run = self.dispose_run()
                mutate(self, run, self.pending.get(ops.START_PENDING), self.pending.get(ops.HANDOFF_PENDING))
                before = copy.deepcopy(self.pending)
                mark = len(run.docker.calls)
                container_before = copy.deepcopy(run.worker.container)
                with self.assertRaises((ops.OperationError, ops.runtime.RuntimeErrorCode)):
                    self.dispose(run)
                self.assertEqual(run.rm_calls, [])
                self.assertEqual(self.pending, before)
                self.assertEqual(self.api_mutations(run, mark), [])
                self.assertEqual(run.worker.container, container_before)

    def test_disposition_rerun_resumes_after_each_crash_point(self):
        for crash in ("after-record", "after-rm", "after-handoff-removal"):
            with self.subTest(crash=crash):
                self.pending.clear()
                run = self.dispose_run()
                pending = copy.deepcopy(self.pending[ops.START_PENDING])
                name = ops._disposition_name(pending["attempt"])
                original = ops._pending_remove.side_effect
                if crash == "after-record":
                    def boom(_args): raise OSError("synthetic crash before rm completes")
                    run.rm_hook = boom
                    real_worker = run.worker.container
                    # rm fails before removal: restore the container afterwards.
                    with self.assertRaises(OSError):
                        self.dispose(run)
                    run.worker.container = real_worker
                    run.rm_hook = None
                    self.assertIn(name, self.pending)
                    self.assertIn(ops.START_PENDING, self.pending)
                elif crash == "after-rm":
                    def boom(_args): raise OSError("synthetic crash after rm")
                    run.rm_hook = boom
                    # The hook runs before the fake removes the container; emulate a completed rm.
                    def hook(args):
                        run.worker.container = None
                        boom(args)
                    run.rm_hook = hook
                    with self.assertRaises(OSError):
                        self.dispose(run)
                    run.rm_hook = None
                    self.assertIsNone(run.worker.container)
                    self.assertIn(name, self.pending)
                else:
                    def remove(root, record_name, expected):
                        if record_name == ops.START_PENDING:
                            raise OSError("synthetic crash after handoff removal")
                        return original(root, record_name, expected)
                    with mock.patch.object(ops, "_pending_remove", side_effect=remove):
                        with self.assertRaises(OSError):
                            self.dispose(run)
                    self.assertNotIn(ops.HANDOFF_PENDING, self.pending)
                    self.assertIn(ops.START_PENDING, self.pending)
                    self.assertIsNone(run.worker.container)
                rm_before = list(run.rm_calls)
                self.assertEqual(self.dispose(run), "FAILED_START_DISPOSED")
                self.assertNotIn(ops.START_PENDING, self.pending)
                self.assertNotIn(ops.HANDOFF_PENDING, self.pending)
                self.assertIn(name, self.pending)
                expected_rm = 2 if crash == "after-record" else 1
                self.assertEqual(len(run.rm_calls), expected_rm)
                self.assertEqual(len(run.rm_calls) - len(rm_before), 1 if crash == "after-record" else 0)
                self.assertFalse(any(row[0] in ("update", "kill", "wait") for row in run.docker.calls))

    def test_disposition_rerun_rejects_changed_record_and_orphan_absence(self):
        run = self.dispose_run()
        pending = self.pending[ops.START_PENDING]
        name = ops._disposition_name(pending["attempt"])
        def stop(_args): raise OSError("synthetic crash before rm completes")
        run.rm_hook = stop
        with self.assertRaises(OSError):
            self.dispose(run)
        run.rm_hook = None
        self.assertIn(name, self.pending)
        # Different bytes: the container evidence changed since the record was written.
        self.pending[name]["container"]["exit_code"] = 7
        before = copy.deepcopy(self.pending)
        mark = len(run.docker.calls)
        with self.assertRaises(ops.OperationError):
            self.dispose(run)
        self.assertEqual(self.pending, before)
        self.assertEqual(self.api_mutations(run, mark), [])
        self.pending[name]["container"]["exit_code"] = 9
        # Absent runner with a record that embeds different start-pending evidence is rejected too.
        run.worker.container = None
        self.pending[name]["start_pending"]["phase"] = "owned"
        before = copy.deepcopy(self.pending)
        with self.assertRaises(ops.OperationError):
            self.dispose(run)
        self.assertEqual(self.pending, before)
        self.pending[name]["start_pending"]["phase"] = "failed"
        # Absent runner without any disposition record is never inferred as ours.
        del self.pending[name]
        before = copy.deepcopy(self.pending)
        with self.assertRaises(ops.OperationError):
            self.dispose(run)
        self.assertEqual(self.pending, before)
        self.assertEqual(self.api_mutations(run, mark), [])

    def test_disposition_action_is_dispatched_before_the_handoff_path(self):
        with mock.patch.object(ops, "dispose_failed_start", return_value="FAILED_START_DISPOSED") as dispose, \
             mock.patch.object(ops, "_handoff_read", side_effect=AssertionError("handoff path entered")), \
             mock.patch.object(ops.daily.install, "require_root"), \
             mock.patch.object(ops.daily, "inherited_lock", return_value=contextlib.nullcontext()), \
             mock.patch.object(ops.daily.runtime, "Docker", return_value=ApiDocker()), \
             mock.patch.object(ops.runtime, "Docker", return_value=mock.Mock()), \
             contextlib.redirect_stdout(io.StringIO()) as output:
            result = ops.main(["transition", "--install-root", "/synthetic/install", "--target-release",
                               fixture.RELEASE, "--commit", fixture.COMMIT, "--lock-fd", "9",
                               "--action", "dispose-failed-start"])
        self.assertEqual(result, 0)
        self.assertEqual(output.getvalue(), "SIMULATION_OPERATION: FAILED_START_DISPOSED\n")
        dispose.assert_called_once()
        with self.assertRaises(SystemExit), contextlib.redirect_stderr(io.StringIO()):
            ops.main(["transition", "--install-root", "/x", "--target-release", "/x", "--commit", "a" * 40,
                      "--lock-fd", "9", "--action", "dispose-failed-starts"])

    def test_disposition_then_next_release_activates_starts_and_runs(self):
        first, second = fixture.COMMIT, "b2" * 20
        root = "/synthetic/install"
        run = self.dispose_run()
        def make_binding(commit):
            config = fixture.config_value()
            config["code_commit"] = commit
            config["image_id"] = "sha256:" + ("1" if commit == first else "2") * 64
            return ops.runtime.Binding(fixture.PROJECT, root + "/releases/" + commit, commit, fixture.config_bytes(config))
        bindings = {c: make_binding(c) for c in (first, second)}
        api_images = {first: API_IMAGE, second: "sha256:" + "f" * 64}
        # Reuse the failed-start fixtures: the worker transport and API belong to the first release.
        worker, docker = run.worker, run.docker
        worker.binding = bindings[first]
        database = mock.Mock()
        database.execute.side_effect = lambda _v, sql, _a: ("SIMULATION_HISTORY: PRESENT" if sql == ops.HISTORY_SQL
                                                             else "SIMULATION_MAINTENANCE: SETTLED")
        current = [root + "/releases/" + first, first]
        images = lambda _p, c: {"api-server": api_images[c], "db-migrate": "sha256:" + "9" * 64}
        values = lambda _p, c, _a: ({"OWNER_BETA_ACCESS_MODE": "owner_only", "OWNER_BETA_PRICE_INPUT_MODE": "sealed_v1",
            "OWNER_BETA_PAPER_MODE": "disabled", "POSTGRES_DB": "fixture",
            "LAGRANGE_RUNTIME_SECRET_DIR": "/synthetic/secrets", "LAGRANGE_CODE_COMMIT": c}, [])
        api_object = docker.object
        def object_(kind, identity, timeout):
            if kind == "image":
                commit = next(c for c, image in api_images.items() if image == identity)
                return {"Id": identity, "Config": {"User": "10001:10001", "Entrypoint": ["/usr/local/bin/api-server"],
                    "Cmd": None, "Labels": {"org.opencontainers.image.revision": commit,
                                            "org.lagrange.api.http-drain-contract": "v1"}}}
            if worker.container is not None and identity == worker.container["Id"]:
                return copy.deepcopy(worker.container)
            return api_object(kind, identity, timeout)
        docker.object = object_
        api_call = docker.call
        def call(args, timeout):
            if args[0] == "start" and worker.container is not None and args[1] == worker.container["Id"]:
                docker.calls.append(tuple(args))
                worker.container["State"].update(Running=True, Status="running", StartedAt="after")
                worker.container["NetworkSettings"]["Networks"] = {"fixture": {"NetworkID": "network-id"}}
                return args[1] + "\n"
            return api_call(args, timeout)
        docker.call = call
        class Output(io.BytesIO):
            def fileno(self): return 71
            def close(self): pass
        def create(argv, **_kwargs):
            self.assertIsNone(self.pending[ops.START_PENDING]["generation"])
            worker.binding = bindings[second]
            worker.container = worker_value(worker.binding, identity=fixture.OTHER_CONTAINER_ID)
            worker.container["NetworkSettings"]["Networks"] = {}
            worker.container["Config"]["Labels"]["org.lagrange.simulation-start-attempt"] = (b"a" * 16).hex()
            return SimpleNamespace(returncode=0)
        def switch(_src, _dst, **_kwargs): current[:] = [root + "/releases/" + second, second]
        self.assertEqual(self.dispose(run), "FAILED_START_DISPOSED")
        self.assertIsNone(worker.container)
        self.assertEqual(run.rm_calls, [("rm", fixture.CONTAINER_ID)])
        target = root + "/releases/" + second
        with contextlib.ExitStack() as stack:
            stack.enter_context(mock.patch.object(ops.daily.install, "require_root"))
            stack.enter_context(mock.patch.object(ops.daily, "inherited_lock", return_value=contextlib.nullcontext()))
            stack.enter_context(mock.patch.object(ops.daily.runtime, "Docker", return_value=docker))
            stack.enter_context(mock.patch.object(ops.runtime, "Docker", return_value=worker))
            stack.enter_context(mock.patch.object(ops, "_current", side_effect=lambda _root: tuple(current)))
            stack.enter_context(mock.patch.object(ops.daily, "read_manifest", side_effect=images))
            stack.enter_context(mock.patch.object(ops.daily, "read_context", side_effect=values))
            stack.enter_context(mock.patch.object(ops.daily, "project_binding", return_value=(fixture.PROJECT, "absent")))
            stack.enter_context(mock.patch.object(ops.daily, "Database", return_value=database))
            stack.enter_context(mock.patch.object(ops.daily.install, "read_root_file", side_effect=lambda p, **_: p.encode()))
            stack.enter_context(mock.patch.object(ops.daily.install, "root_directory", return_value=contextlib.nullcontext(99)))
            stack.enter_context(mock.patch.object(ops, "_floor_present", return_value=True))
            stack.enter_context(mock.patch.object(ops, "_configured", return_value=True))
            stack.enter_context(mock.patch.object(ops, "binding_from_release", side_effect=lambda _p, _r, c: bindings[c]))
            stack.enter_context(mock.patch.object(ops.release, "reader_guard"))
            stack.enter_context(mock.patch.object(ops, "validate_worker_secret"))
            stack.enter_context(mock.patch.object(ops, "input_guard", return_value=contextlib.nullcontext()))
            stack.enter_context(mock.patch.object(ops.daily, "internal_network", return_value={"Id": "network-id", "Name": "fixture"}))
            stack.enter_context(mock.patch.object(ops.os, "symlink"))
            stack.enter_context(mock.patch.object(ops.os, "replace", side_effect=switch))
            stack.enter_context(mock.patch.object(ops.os, "fsync"))
            stack.enter_context(mock.patch.object(ops.os, "fchmod"))
            stack.enter_context(mock.patch.object(ops.os, "fdopen", return_value=Output()))
            stack.enter_context(mock.patch.object(ops.os, "unlink"))
            stack.enter_context(mock.patch.object(ops.os, "urandom", return_value=b"a" * 16))
            stack.enter_context(mock.patch.object(ops.tempfile, "mkstemp", return_value=(71, "/synthetic/override")))
            stack.enter_context(mock.patch.object(ops.time, "monotonic", return_value=1.0))
            stack.enter_context(mock.patch.object(ops.time, "sleep"))
            stack.enter_context(mock.patch.object(ops.subprocess, "run", side_effect=create))
            def api_for(commit, identity):
                docker.container = api_container()
                docker.container.update(Id=identity, Image=api_images[commit])
                docker.container["Config"]["Labels"]["org.opencontainers.image.revision"] = commit
                docker.container["Config"]["Env"] = ["OWNER_BETA_ACCESS_MODE=owner_only",
                    "OWNER_BETA_PRICE_INPUT_MODE=sealed_v1", "LAGRANGE_CODE_COMMIT=" + commit]
            api_for(first, API_ID)
            self.assertEqual(ops.transition(root, target, second, "9", action="activate"), "ACTIVATED_OFF")
            self.assertEqual(tuple(current), (target, second))
            handoff = self.pending[ops.HANDOFF_PENDING]
            self.assertEqual(handoff["phase"], "switched")
            self.assertIsNone(handoff["worker_generation"])
            self.assertNotIn(ops.START_PENDING, self.pending)
            self.assertEqual(ops.transition(root, target, second, "9", action="ready"), "RELEASE_START_READY")
            begun = ops.transition(root, target, second, "9", action="begin-release")
            self.assertEqual(begun, "RELEASE_START_BEGUN " + handoff["api_attempt"])
            api_for(second, "f" * 64)
            docker.container["Config"]["Labels"]["org.lagrange.api-release-attempt"] = handoff["api_attempt"]
            self.assertEqual(ops.transition(root, target, second, "9", action="finish-release"),
                             "RELEASE_START_FINISHED")
            self.assertNotIn(ops.HANDOFF_PENDING, self.pending)
            worker.binding = bindings[second]
            self.assertEqual(ops.transition(root, target, second, "9", action="start"), "STARTED")
            self.assertNotIn(ops.START_PENDING, self.pending)
            self.assertTrue(worker.container["State"]["Running"])
            self.assertTrue(docker.container["State"]["Running"])

    def crash_after_rm(self, run):
        def hook(args):
            run.worker.container = None
            raise OSError("synthetic crash after rm")
        run.rm_hook = hook
        with self.assertRaises(OSError):
            self.dispose(run)
        run.rm_hook = None
        self.assertIsNone(run.worker.container)

    def test_post_rm_resume_tolerates_only_an_api_restart(self):
        cases = {
            "restarted": (lambda r: (r.docker.container["State"].update(StartedAt="rebooted"),
                                     r.docker.container.__setitem__("RestartCount", 2)), True),
            "other-id": (lambda r: r.docker.container.__setitem__("Id", "e" * 64), False),
            "not-running": (lambda r: r.docker.container["State"].update(Running=False, Status="exited"), False),
            "other-created": (lambda r: r.docker.container.__setitem__("Created", "replaced"), False),
            "attempt-label": (lambda r: r.docker.container["Config"]["Labels"].update(
                {"org.lagrange.api-release-attempt": "0" * 32}), False),
        }
        for name, (mutate, accepted) in cases.items():
            with self.subTest(case=name):
                self.pending.clear()
                run = self.dispose_run()
                self.crash_after_rm(run)
                mutate(run)
                before = copy.deepcopy(self.pending)
                mark = len(run.docker.calls)
                if accepted:
                    self.assertEqual(self.dispose(run), "FAILED_START_DISPOSED")
                    self.assertNotIn(ops.START_PENDING, self.pending)
                else:
                    with self.assertRaises(ops.OperationError):
                        self.dispose(run)
                    self.assertEqual(self.pending, before)
                self.assertEqual(self.api_mutations(run, mark), [])
                self.assertEqual(len(run.rm_calls), 1)

    def test_strict_api_check_still_applies_while_the_runner_is_present(self):
        run = self.dispose_run()
        run.docker.container["State"]["StartedAt"] = "rebooted"
        before = copy.deepcopy(self.pending)
        with self.assertRaises(ops.OperationError):
            self.dispose(run)
        self.assertEqual(self.pending, before)
        self.assertEqual(run.rm_calls, [])

    def test_post_rm_resume_rejects_unsettled_or_changed_handoff(self):
        for name in ("unsettled", "handoff-changed"):
            with self.subTest(case=name):
                self.pending.clear()
                run = self.dispose_run()
                self.crash_after_rm(run)
                if name == "unsettled":
                    run.database.execute.result = "SIMULATION_MAINTENANCE: OPEN"
                else:
                    self.pending[ops.HANDOFF_PENDING]["worker_policy"] = "unless-stopped"
                before = copy.deepcopy(self.pending)
                with self.assertRaises(ops.OperationError):
                    self.dispose(run)
                self.assertEqual(self.pending, before)
                self.assertEqual(len(run.rm_calls), 1)

    def test_present_runner_with_record_rejects_a_handoff_changed_since_the_record(self):
        run = self.dispose_run()
        def stop(_args): raise OSError("synthetic crash before rm completes")
        run.rm_hook = stop
        with self.assertRaises(OSError):
            self.dispose(run)
        run.rm_hook = None
        self.pending[ops.HANDOFF_PENDING]["worker_policy"] = "unless-stopped"
        before = copy.deepcopy(self.pending)
        with self.assertRaises(ops.OperationError):
            self.dispose(run)
        self.assertEqual(self.pending, before)
        self.assertEqual(len(run.rm_calls), 1)

    def test_current_or_commit_mismatch_is_refused_before_any_mutation(self):
        other = "b2" * 20
        for kwargs in ({"current": ("/srv/lagrange/releases/" + other, other)}, {"commit": other}):
            with self.subTest(kwargs=sorted(kwargs)):
                self.pending.clear()
                run = self.dispose_run()
                before = copy.deepcopy(self.pending)
                mark = len(run.docker.calls)
                with self.assertRaises(ops.OperationError):
                    self.dispose(run, **kwargs)
                self.assertEqual(self.pending, before)
                self.assertEqual(run.rm_calls, [])
                self.assertEqual(self.api_mutations(run, mark), [])

    def test_disposition_create_publishes_by_rename_and_refuses_an_existing_name(self):
        name = ops.DISPOSED_PREFIX + "a" * 32
        calls = []
        def create(parent, temporary, data, uid, gid, mode):
            calls.append(("create", temporary))
        def replace(src, dst, **kwargs):
            calls.append(("replace", src, dst))
        with contextlib.ExitStack() as stack:
            ops_patch = lambda target, attr, **kw: stack.enter_context(mock.patch.object(target, attr, **kw))
            ops_patch(ops.daily.install, "root_directory", return_value=contextlib.nullcontext(5))
            ops_patch(ops.daily.install, "create_file", side_effect=create)
            ops_patch(ops.os, "replace", side_effect=replace)
            ops_patch(ops.os, "fsync")
            ops_patch(ops.os, "link", side_effect=AssertionError("link must not be used"))
            ops_patch(ops.os, "unlink", side_effect=AssertionError("unlink must not be used"))
            stat_patch = ops_patch(ops.os, "stat", side_effect=FileNotFoundError)
            self.disposition_create_real("/synthetic/install", name, {"schema": "x"})
            self.assertEqual([c[0] for c in calls], ["create", "replace"])
            self.assertEqual(calls[1][2], name)
            self.assertTrue(calls[1][1].endswith(".tmp"))
            calls.clear()
            stat_patch.side_effect = None
            stat_patch.return_value = object()
            with self.assertRaises(ops.OperationError):
                self.disposition_create_real("/synthetic/install", name, {"schema": "x"})
            self.assertEqual(calls, [])
            with self.assertRaises(ops.OperationError):
                self.disposition_create_real("/synthetic/install", "../x", {"schema": "x"})

    def test_incompatible_rollback_rejects_before_any_api_mutation(self):
        target_commit = "b2" * 20
        target = "/srv/lagrange/releases/" + target_commit
        docker, sim_docker, database = ApiDocker(), mock.Mock(), mock.Mock()
        sim_docker.service_ids.return_value = []
        database.execute.return_value = "SIMULATION_HISTORY: PRESENT"
        with contextlib.ExitStack() as stack:
            stack.enter_context(mock.patch.object(ops.daily, "read_manifest", return_value={"api-server": API_IMAGE, "db-migrate": "sha256:" + "9" * 64}))
            stack.enter_context(mock.patch.object(ops, "_current", return_value=(fixture.RELEASE, fixture.COMMIT)))
            stack.enter_context(mock.patch.object(ops.daily, "project_binding", return_value=(fixture.PROJECT, "absent")))
            stack.enter_context(mock.patch.object(ops.daily, "read_context", return_value=({}, [])))
            stack.enter_context(mock.patch.object(ops.daily, "Database", return_value=database))
            stack.enter_context(mock.patch.object(ops, "_floor_present", return_value=True))
            stack.enter_context(mock.patch.object(ops, "_configured", side_effect=[True, False]))
            stack.enter_context(mock.patch.object(ops.release, "reader_guard", side_effect=ops.OperationError("incompatible")))
            with self.assertRaises(ops.OperationError):
                ops.inspect_transition("/srv/lagrange", target, target_commit, docker, sim_docker)
        self.assertEqual(docker.calls, [])

    def test_backup_tool_dispatch_requires_matching_controller_source(self):
        tool_commit = "b" * 40
        root = "/srv/lagrange"
        app = root + "/releases/" + fixture.COMMIT
        name = "scripts/ops/lib/owner-beta-daily-backup.py"
        verifier = SimpleNamespace(REQUIRED={name}, MAX_FILE=1024, read_manifest=mock.Mock())
        argv = ["verify-backup-tool", "--install-root", root, "--application-release", app,
                "--commit", fixture.COMMIT, "--tool-commit", tool_commit,
                "--manifest-hash", "sha256:" + "c" * 64]
        for changed, expected in ((False, 0), (True, 1)):
            with self.subTest(changed=changed), contextlib.ExitStack() as stack:
                stack.enter_context(mock.patch.object(ops.daily.install, "require_root"))
                stack.enter_context(mock.patch.object(ops, "_local", return_value=verifier))
                stack.enter_context(mock.patch.object(ops.daily.install, "read_root_file",
                    side_effect=[b"simulation-aware", b"older" if changed else b"simulation-aware"]))
                stack.enter_context(contextlib.redirect_stdout(io.StringIO()))
                stack.enter_context(contextlib.redirect_stderr(io.StringIO()))
                self.assertEqual(ops.main(argv), expected)
        self.assertEqual(verifier.read_manifest.call_count, 2)

    def test_explicit_release_start_requires_capability_stopped_worker_and_settlement(self):
        binding = self.binding()
        context = SimpleNamespace(required=True, source_release=fixture.RELEASE,
            target_release=fixture.RELEASE, source_commit=fixture.COMMIT, target_commit=fixture.COMMIT,
            project=fixture.PROJECT, source_images={"api-server": API_IMAGE}, binding=binding,
            values={}, history=True)
        worker = SimpleNamespace(generation=None, running=False, restart_policy=None)
        database, docker, sim_docker = mock.Mock(), ApiDocker(), mock.Mock()
        docker.container["State"].update(Running=False, Status="exited")
        docker.container["HostConfig"]["RestartPolicy"]["Name"] = "no"
        database.execute.return_value = "SIMULATION_MAINTENANCE: SETTLED"
        with contextlib.ExitStack() as stack:
            stack.enter_context(mock.patch.object(ops.runtime, "capture", return_value=worker))
            ops.release_start_ready(context, database, docker, sim_docker)
            database.execute.assert_called_once()
            self.assertFalse(any(row[0] in ("update", "kill", "wait", "start", "rm") for row in docker.calls))
            worker.running = True
            with self.assertRaises(ops.OperationError):
                ops.release_start_ready(context, database, docker, sim_docker)
            worker.running = False
            docker.image_capability = "missing"
            with self.assertRaises(ops.OperationError):
                ops.release_start_ready(context, database, docker, sim_docker)
        self.assertEqual(database.execute.call_count, 1)


class FailedCaptureTests(unittest.TestCase):
    """capture_failed is opt-in; capture() must keep rejecting every non-zero exit.

    The standalone runtime suite is not registered in the production-ops entrypoint,
    so these run with the operations suite that CI does execute.
    """
    runtime = ops.runtime

    def binding(self):
        return self.runtime.Binding(fixture.PROJECT, fixture.RELEASE, fixture.COMMIT, fixture.config_bytes())

    def failed(self, binding):
        value = worker_value(binding, state="exited")
        value["State"].update(ExitCode=9, StartedAt="worker-after")
        return value

    def transport(self, value):
        transport = WorkerTransport(self.binding())
        transport.container = value
        return transport

    def test_accepts_only_the_failed_exit_shape_and_capture_still_rejects_it(self):
        binding = self.binding()
        transport = self.transport(self.failed(binding))
        snapshot = self.runtime.capture_failed(transport, binding)
        self.assertEqual(snapshot.generation, (fixture.CONTAINER_ID, "worker-created", "worker-after", 0))
        self.assertFalse(snapshot.running)
        self.assertEqual(snapshot.restart_policy, "no")
        self.assertFalse(any(row[0] in ("update", "kill", "wait", "start", "rm") for row in transport.calls))
        with self.assertRaises(self.runtime.RuntimeErrorCode) as caught:
            self.runtime.capture(transport, binding)
        self.assertEqual(caught.exception.code, "SIMULATION_RUNTIME_STATE")
        # The existing clean states are unaffected.
        clean = self.transport(worker_value(binding, state="exited"))
        self.assertFalse(self.runtime.capture(clean, binding).running)
        with self.assertRaises(self.runtime.RuntimeErrorCode):
            self.runtime.capture_failed(clean, binding)

    def test_rejects_every_other_shape(self):
        binding = self.binding()
        def state(**values): return lambda v: v["State"].update(values)
        cases = {
            "running": state(Running=True, Status="running"),
            "exit-zero": state(ExitCode=0),
            "created": state(Status="created"),
            "dead": state(Status="dead"),
            "oom": state(OOMKilled=True),
            "error": state(Error="boom"),
            "restarting": state(Restarting=True),
            "paused": state(Paused=True),
            "policy": lambda v: v["HostConfig"]["RestartPolicy"].update(Name="unless-stopped"),
            "project": lambda v: v["Config"]["Labels"].update({self.runtime.PROJECT_LABEL: "foreign"}),
            "config-pin": lambda v: v["Config"]["Labels"].update({self.runtime.CONFIG_LABEL: "sha256:" + "0" * 64}),
            "mount": lambda v: v["Mounts"][0].update(RW=True),
            "image": lambda v: v.update(Image="sha256:" + "0" * 64),
        }
        for name, mutate in cases.items():
            with self.subTest(case=name):
                value = self.failed(binding)
                mutate(value)
                transport = self.transport(value)
                with self.assertRaises(self.runtime.RuntimeErrorCode):
                    self.runtime.capture_failed(transport, binding)
                self.assertFalse(any(row[0] in ("update", "kill", "wait", "start", "rm") for row in transport.calls))
        absent = WorkerTransport(binding)
        with self.assertRaises(self.runtime.RuntimeErrorCode):
            self.runtime.capture_failed(absent, binding)
        two = self.transport(self.failed(binding))
        two.service_ids = lambda timeout: [fixture.CONTAINER_ID, fixture.OTHER_CONTAINER_ID]
        with self.assertRaises(self.runtime.RuntimeErrorCode):
            self.runtime.capture_failed(two, binding)
        drift = self.transport(self.failed(binding))
        real = drift.object
        views = []
        def object_(kind, identity, timeout):
            value = real(kind, identity, timeout)
            if kind == "container":
                views.append(1)
                if len(views) > 1:
                    value["State"]["StartedAt"] = "changed"
            return value
        drift.object = object_
        with self.assertRaises(self.runtime.RuntimeErrorCode):
            self.runtime.capture_failed(drift, binding)


if __name__ == "__main__":
    unittest.main()
