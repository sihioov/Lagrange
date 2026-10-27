#!/usr/bin/env python3
"""Read-only process observation and TERM-only drain for owner-beta simulation."""
from __future__ import annotations

import dataclasses
import hashlib
import importlib.util
import json
import math
import os
import re
import subprocess
import sys
import time
import unicodedata
from functools import wraps
from types import MappingProxyType


SERVICE = "owner-beta-simulation-runner"
ENTRYPOINT = ("/usr/local/bin/owner-beta-simulation-entrypoint",)
USER = "10001:10001"
CONFIG_DESTINATION = "/run/owner-beta-simulation/config.json"
INPUT_DESTINATION = "/data/owner-beta-artifacts"
CONFIG_LABEL = "io.lagrange.owner-beta-simulation.config-sha256"
RECEIPT_LABEL = "io.lagrange.owner-beta-simulation.receipt-sha256"
REVISION_LABEL = "org.opencontainers.image.revision"
PROJECT_LABEL = "com.docker.compose.project"
SERVICE_LABEL = "com.docker.compose.service"
ONEOFF_LABEL = "com.docker.compose.oneoff"
DOCKER_PREFIX = ("/usr/bin/docker", "--host", "unix:///var/run/docker.sock")
DOCKER_ENVIRONMENT = MappingProxyType({
    "PATH": "/usr/sbin:/usr/bin:/sbin:/bin",
    "HOME": "/root",
    "LANG": "C.UTF-8",
})
MAX_RESPONSE_BYTES = 2 * 1024 * 1024
OBSERVATION_TIMEOUT_SECONDS = 15.0
_CONTAINER_ID = re.compile(r"[0-9a-f]{64}\Z")
_IMAGE_ID = re.compile(r"sha256:[0-9a-f]{64}\Z")
_PROJECT = re.compile(r"[a-z0-9][a-z0-9_-]{0,62}\Z")
_COMMIT = re.compile(r"[0-9a-f]{40}\Z")
_RELEASE_MODULE_NAME = "_lagrange_owner_beta_simulation_release_wp5c2"
_PS_ARGS = (
    "ps", "--all", "--no-trunc", "--filter",
    "label=com.docker.compose.service=owner-beta-simulation-runner",
    "--format", "{{.ID}}",
)
_ERROR_CODES = frozenset({
    "SIMULATION_RUNTIME_INVALID",
    "SIMULATION_RUNTIME_DEPENDENCY",
    "SIMULATION_RUNTIME_CONFIG",
    "SIMULATION_RUNTIME_PLATFORM",
    "SIMULATION_RUNTIME_TIMEOUT",
    "SIMULATION_RUNTIME_TRANSPORT",
    "SIMULATION_RUNTIME_RESPONSE",
    "SIMULATION_RUNTIME_IDENTITY",
    "SIMULATION_RUNTIME_MOUNT",
    "SIMULATION_RUNTIME_GENERATION",
    "SIMULATION_RUNTIME_STATE",
    "SIMULATION_RUNTIME_SINGLETON",
    "SIMULATION_RUNTIME_RESTART_POLICY",
    "SIMULATION_RUNTIME_CLOCK",
})


class RuntimeErrorCode(Exception):
    """Public failures expose a fixed code and no provider or host detail."""

    def __init__(self, code):
        safe_code = code if type(code) is str and code in _ERROR_CODES else "SIMULATION_RUNTIME_INVALID"
        self.code = safe_code
        super().__init__(safe_code)


def _fail(code="SIMULATION_RUNTIME_INVALID"):
    raise RuntimeErrorCode(code) from None


def _api_boundary(function):
    @wraps(function)
    def wrapped(*args, **kwargs):
        try:
            return function(*args, **kwargs)
        except RuntimeErrorCode as error:
            raise RuntimeErrorCode(error.code) from None
        except subprocess.TimeoutExpired:
            _fail("SIMULATION_RUNTIME_TIMEOUT")
        except Exception:
            _fail("SIMULATION_RUNTIME_INVALID")
    return wrapped


def _release_module():
    loaded = sys.modules.get(_RELEASE_MODULE_NAME)
    if loaded is not None:
        return loaded
    try:
        path = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                            "owner-beta-simulation-release.py")
        spec = importlib.util.spec_from_file_location(_RELEASE_MODULE_NAME, path)
        if spec is None or spec.loader is None:
            _fail("SIMULATION_RUNTIME_DEPENDENCY")
        module = importlib.util.module_from_spec(spec)
        sys.modules[_RELEASE_MODULE_NAME] = module
        try:
            spec.loader.exec_module(module)
        except Exception:
            sys.modules.pop(_RELEASE_MODULE_NAME, None)
            _fail("SIMULATION_RUNTIME_DEPENDENCY")
        return module
    except RuntimeErrorCode:
        raise
    except Exception:
        _fail("SIMULATION_RUNTIME_DEPENDENCY")


def _validated_config(data, commit):
    try:
        value = _release_module().validate_config(data, commit)
    except RuntimeErrorCode:
        raise
    except Exception:
        _fail("SIMULATION_RUNTIME_CONFIG")
    if type(value) is not dict:
        _fail("SIMULATION_RUNTIME_CONFIG")
    return value


def _canonical_host_path(value):
    if type(value) is not str or not value.startswith("/") or value == "/" or value.endswith("/"):
        return False
    if any(unicodedata.category(character) == "Cc"
           or 0xD800 <= ord(character) <= 0xDFFF for character in value):
        return False
    return all(part not in ("", ".", "..") for part in value[1:].split("/"))


def _valid_project(value):
    return type(value) is str and _PROJECT.fullmatch(value) is not None


def _valid_commit(value):
    return (type(value) is str and _COMMIT.fullmatch(value) is not None
            and value != "0" * 40)


def _is_finite_number(value):
    if type(value) not in (int, float):
        return False
    try:
        return math.isfinite(value)
    except (OverflowError, TypeError, ValueError):
        return False


@dataclasses.dataclass(frozen=True, repr=False)
class Binding:
    project: str
    release: str
    commit: str
    config_data: bytes = dataclasses.field(repr=False)
    _image_id: str = dataclasses.field(init=False, repr=False)
    _config_sha256: str = dataclasses.field(init=False, repr=False)
    _receipt_sha256: str = dataclasses.field(init=False, repr=False)
    _stop_timeout_seconds: int = dataclasses.field(init=False, repr=False)
    _historical_root: str = dataclasses.field(init=False, repr=False)

    def __post_init__(self):
        try:
            if not _valid_project(self.project) or not _valid_commit(self.commit):
                _fail("SIMULATION_RUNTIME_INVALID")
            if (not _canonical_host_path(self.release)
                    or self.release.rsplit("/", 1)[-1] != self.commit):
                _fail("SIMULATION_RUNTIME_INVALID")
            if type(self.config_data) is not bytes:
                _fail("SIMULATION_RUNTIME_INVALID")
            config = _validated_config(self.config_data, self.commit)
            limits = config.get("limits")
            if type(limits) is not dict:
                _fail("SIMULATION_RUNTIME_CONFIG")
            image_id = config.get("image_id")
            receipt_sha256 = config.get("receipt_sha256")
            stop_timeout_seconds = limits.get("stop_timeout_seconds")
            historical_root = config.get("historical_root")
            if (type(image_id) is not str or _IMAGE_ID.fullmatch(image_id) is None
                    or type(receipt_sha256) is not str
                    or re.fullmatch(r"sha256:[0-9a-f]{64}", receipt_sha256) is None
                    or type(stop_timeout_seconds) is not int
                    or not 11 <= stop_timeout_seconds <= 2**31 - 1
                    or not _canonical_host_path(historical_root)):
                _fail("SIMULATION_RUNTIME_CONFIG")
            object.__setattr__(self, "_image_id", image_id)
            object.__setattr__(self, "_config_sha256",
                               "sha256:" + hashlib.sha256(self.config_data).hexdigest())
            object.__setattr__(self, "_receipt_sha256", receipt_sha256)
            object.__setattr__(self, "_stop_timeout_seconds", stop_timeout_seconds)
            object.__setattr__(self, "_historical_root", historical_root)
        except RuntimeErrorCode:
            raise
        except Exception:
            _fail("SIMULATION_RUNTIME_CONFIG")

    @property
    def image_id(self):
        return self._image_id

    @property
    def config_sha256(self):
        return self._config_sha256

    @property
    def receipt_sha256(self):
        return self._receipt_sha256

    @property
    def stop_timeout_seconds(self):
        return self._stop_timeout_seconds

    @property
    def historical_root(self):
        return self._historical_root


def _binding_identity(binding):
    return (
        binding.project,
        binding.release,
        binding.commit,
        binding.image_id,
        binding.config_sha256,
        binding.receipt_sha256,
    )


def _checked_binding(binding):
    if type(binding) is not Binding:
        _fail("SIMULATION_RUNTIME_INVALID")
    fresh = Binding(binding.project, binding.release, binding.commit, binding.config_data)
    if (_binding_identity(binding) != _binding_identity(fresh)
            or binding.stop_timeout_seconds != fresh.stop_timeout_seconds
            or binding.historical_root != fresh.historical_root):
        _fail("SIMULATION_RUNTIME_INVALID")
    return binding


def _valid_container_id(value):
    return type(value) is str and _CONTAINER_ID.fullmatch(value) is not None


def _valid_image_id(value):
    return type(value) is str and _IMAGE_ID.fullmatch(value) is not None


def _validate_docker_args(args):
    if type(args) is not list or any(type(value) is not str for value in args):
        _fail("SIMULATION_RUNTIME_INVALID")
    if tuple(args) == _PS_ARGS:
        return
    if len(args) == 3 and args[0] == "image" and args[1] == "inspect" and _valid_image_id(args[2]):
        return
    if len(args) == 3 and args[0] == "container" and args[1] == "inspect" and _valid_container_id(args[2]):
        return
    if len(args) == 3 and args[0] == "update" and args[1] == "--restart=no" and _valid_container_id(args[2]):
        return
    if len(args) == 3 and args[0] == "kill" and args[1] == "--signal=TERM" and _valid_container_id(args[2]):
        return
    if len(args) == 2 and args[0] == "wait" and _valid_container_id(args[1]):
        return
    _fail("SIMULATION_RUNTIME_INVALID")


def _valid_timeout(timeout):
    return _is_finite_number(timeout) and timeout > 0


def _duplicate_json_key(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate")
        result[key] = value
    return result


def _invalid_json_constant(_value):
    raise ValueError("constant")


class Docker:
    """Fixed root-only Docker socket transport; never selects ambient context."""

    def __init__(self):
        try:
            if sys.platform != "linux" or os.geteuid() != 0:
                _fail("SIMULATION_RUNTIME_PLATFORM")
        except RuntimeErrorCode:
            raise
        except Exception:
            _fail("SIMULATION_RUNTIME_PLATFORM")

    def call(self, args, timeout):
        _validate_docker_args(args)
        if not _valid_timeout(timeout):
            _fail("SIMULATION_RUNTIME_INVALID")
        try:
            result = subprocess.run(
                [*DOCKER_PREFIX, *args],
                shell=False,
                stdin=subprocess.DEVNULL,
                capture_output=True,
                check=False,
                timeout=timeout,
                env=dict(DOCKER_ENVIRONMENT),
            )
        except subprocess.TimeoutExpired:
            _fail("SIMULATION_RUNTIME_TIMEOUT")
        except Exception:
            _fail("SIMULATION_RUNTIME_TRANSPORT")
        try:
            returncode = result.returncode
            stdout = result.stdout
        except Exception:
            _fail("SIMULATION_RUNTIME_TRANSPORT")
        if type(returncode) is not int or returncode != 0:
            _fail("SIMULATION_RUNTIME_TRANSPORT")
        if type(stdout) is not bytes or len(stdout) > MAX_RESPONSE_BYTES:
            _fail("SIMULATION_RUNTIME_RESPONSE")
        try:
            return stdout.decode("utf-8", errors="strict")
        except UnicodeError:
            _fail("SIMULATION_RUNTIME_RESPONSE")

    def object(self, kind, identity, timeout):
        if type(kind) is str and kind == "image" and _valid_image_id(identity):
            args = ["image", "inspect", identity]
        elif type(kind) is str and kind == "container" and _valid_container_id(identity):
            args = ["container", "inspect", identity]
        else:
            _fail("SIMULATION_RUNTIME_INVALID")
        text = self.call(args, timeout)
        try:
            value = json.loads(text, object_pairs_hook=_duplicate_json_key,
                               parse_constant=_invalid_json_constant)
        except Exception:
            _fail("SIMULATION_RUNTIME_RESPONSE")
        if type(value) is not list or len(value) != 1 or type(value[0]) is not dict:
            _fail("SIMULATION_RUNTIME_RESPONSE")
        return value[0]

    def service_ids(self, timeout):
        text = self.call(list(_PS_ARGS), timeout)
        if type(text) is not str:
            _fail("SIMULATION_RUNTIME_RESPONSE")
        if text == "":
            return []
        if text.endswith("\n"):
            row = text[:-1]
            if "\n" in row:
                _fail("SIMULATION_RUNTIME_SINGLETON")
        else:
            row = text
        if "\r" in row or not _valid_container_id(row):
            _fail("SIMULATION_RUNTIME_RESPONSE")
        return [row]


@dataclasses.dataclass(frozen=True, repr=False)
class Snapshot:
    binding_identity: tuple[str, str, str, str, str, str]
    generation: tuple[str, str, str, int] | None
    restart_policy: str | None
    running: bool


@dataclasses.dataclass(frozen=True, repr=False)
class DrainResult:
    status: str
    captured: bool
    signalled: bool
    generation: tuple[str, str, str, int] | None


@dataclasses.dataclass(frozen=True, repr=False)
class _Observation:
    generation: tuple[str, str, str, int]
    restart_policy: str
    running: bool
    status: str
    exit_code: int


def _new_snapshot(binding, observation=None):
    if observation is None:
        return Snapshot(_binding_identity(binding), None, None, False)
    return Snapshot(_binding_identity(binding), observation.generation,
                    observation.restart_policy, observation.running)


def _checked_snapshot(snapshot, binding):
    if type(snapshot) is not Snapshot:
        _fail("SIMULATION_RUNTIME_INVALID")
    identity = snapshot.binding_identity
    if (type(identity) is not tuple or len(identity) != 6
            or any(type(value) is not str for value in identity)
            or identity != _binding_identity(binding)):
        _fail("SIMULATION_RUNTIME_INVALID")
    if type(snapshot.running) is not bool:
        _fail("SIMULATION_RUNTIME_INVALID")
    generation = snapshot.generation
    policy = snapshot.restart_policy
    if generation is None:
        if policy is not None or snapshot.running:
            _fail("SIMULATION_RUNTIME_INVALID")
    else:
        if (type(generation) is not tuple or len(generation) != 4
                or not _valid_container_id(generation[0])
                or type(generation[1]) is not str or not generation[1]
                or type(generation[2]) is not str or not generation[2]
                or type(generation[3]) is not int or generation[3] < 0
                or type(policy) is not str or policy not in ("no", "unless-stopped")):
            _fail("SIMULATION_RUNTIME_INVALID")
    return snapshot


def _invoke(docker, method, *args, deadline=None, clock=None, wait=False):
    timeout = OBSERVATION_TIMEOUT_SECONDS
    if deadline is not None:
        remaining = _remaining(deadline, clock)
        timeout = remaining if wait else min(OBSERVATION_TIMEOUT_SECONDS, remaining)
    try:
        operation = getattr(docker, method, None)
        if not callable(operation):
            _fail("SIMULATION_RUNTIME_TRANSPORT")
        result = operation(*args, timeout=timeout)
    except RuntimeErrorCode as error:
        raise RuntimeErrorCode(error.code) from None
    except subprocess.TimeoutExpired:
        _fail("SIMULATION_RUNTIME_TIMEOUT")
    except Exception:
        _fail("SIMULATION_RUNTIME_TRANSPORT")
    return result


def _service_ids(docker, *, deadline=None, clock=None):
    ids = _invoke(docker, "service_ids", deadline=deadline, clock=clock)
    if type(ids) is not list or len(ids) > 1:
        _fail("SIMULATION_RUNTIME_SINGLETON")
    if any(not _valid_container_id(identity) for identity in ids):
        _fail("SIMULATION_RUNTIME_RESPONSE")
    if len(set(ids)) != len(ids):
        _fail("SIMULATION_RUNTIME_SINGLETON")
    return ids


def _object(docker, kind, identity, *, deadline=None, clock=None):
    value = _invoke(docker, "object", kind, identity, deadline=deadline, clock=clock)
    if type(value) is not dict:
        _fail("SIMULATION_RUNTIME_RESPONSE")
    return value


def _verify_image(docker, binding, *, deadline=None, clock=None):
    image = _object(docker, "image", binding.image_id, deadline=deadline, clock=clock)
    if image.get("Id") != binding.image_id:
        _fail("SIMULATION_RUNTIME_IDENTITY")
    config = image.get("Config")
    if type(config) is not dict:
        _fail("SIMULATION_RUNTIME_RESPONSE")
    labels = config.get("Labels")
    if type(labels) is not dict:
        _fail("SIMULATION_RUNTIME_RESPONSE")
    if labels.get(REVISION_LABEL) != binding.commit:
        _fail("SIMULATION_RUNTIME_IDENTITY")


def _required_dict(parent, key):
    if type(parent) is not dict:
        _fail("SIMULATION_RUNTIME_RESPONSE")
    value = parent.get(key)
    if type(value) is not dict:
        _fail("SIMULATION_RUNTIME_RESPONSE")
    return value


def _validate_mounts(container, binding):
    mounts = container.get("Mounts")
    if type(mounts) is not list:
        _fail("SIMULATION_RUNTIME_RESPONSE")
    destinations = {}
    for mount in mounts:
        if type(mount) is not dict:
            _fail("SIMULATION_RUNTIME_MOUNT")
        destination = mount.get("Destination")
        if type(destination) is not str:
            _fail("SIMULATION_RUNTIME_MOUNT")
        if destination in destinations:
            _fail("SIMULATION_RUNTIME_MOUNT")
        destinations[destination] = mount
    expected = (
        (CONFIG_DESTINATION, binding.release + "/.owner-beta-simulation/config.json"),
        (INPUT_DESTINATION, binding.historical_root),
    )
    for destination, source in expected:
        mount = destinations.get(destination)
        if (type(mount) is not dict or mount.get("Type") != "bind"
                or mount.get("Source") != source
                or type(mount.get("RW")) is not bool or mount["RW"] is not False
                or mount.get("Propagation") != "rprivate"):
            _fail("SIMULATION_RUNTIME_MOUNT")


def _validate_container(container, binding, identity):
    if type(container) is not dict:
        _fail("SIMULATION_RUNTIME_RESPONSE")
    if (not _valid_container_id(identity) or container.get("Id") != identity
            or container.get("Image") != binding.image_id):
        _fail("SIMULATION_RUNTIME_IDENTITY")
    config = _required_dict(container, "Config")
    labels = config.get("Labels")
    if type(labels) is not dict:
        _fail("SIMULATION_RUNTIME_RESPONSE")
    expected_labels = {
        REVISION_LABEL: binding.commit,
        PROJECT_LABEL: binding.project,
        SERVICE_LABEL: SERVICE,
        ONEOFF_LABEL: "False",
        CONFIG_LABEL: binding.config_sha256,
        RECEIPT_LABEL: binding.receipt_sha256,
    }
    if any(labels.get(key) != value for key, value in expected_labels.items()):
        _fail("SIMULATION_RUNTIME_IDENTITY")
    entrypoint = config.get("Entrypoint")
    if (config.get("User") != USER or type(entrypoint) is not list
            or tuple(entrypoint) != ENTRYPOINT
            or not all(type(value) is str for value in entrypoint)):
        _fail("SIMULATION_RUNTIME_IDENTITY")
    command = config.get("Cmd")
    if command is not None and (type(command) is not list or len(command) != 0):
        _fail("SIMULATION_RUNTIME_IDENTITY")

    host = _required_dict(container, "HostConfig")
    for key, expected in (("AutoRemove", False), ("Init", True),
                          ("ReadonlyRootfs", True), ("Privileged", False)):
        if type(host.get(key)) is not bool or host[key] is not expected:
            _fail("SIMULATION_RUNTIME_IDENTITY")
    _validate_mounts(container, binding)

    restart = host.get("RestartPolicy")
    if type(restart) is not dict:
        _fail("SIMULATION_RUNTIME_RESTART_POLICY")
    policy = restart.get("Name")
    maximum = restart.get("MaximumRetryCount")
    if (type(policy) is not str or policy not in ("no", "unless-stopped")
            or type(maximum) is not int or maximum != 0):
        _fail("SIMULATION_RUNTIME_RESTART_POLICY")

    state = _required_dict(container, "State")
    booleans = ("Running", "Restarting", "Paused", "OOMKilled")
    if any(type(state.get(key)) is not bool for key in booleans):
        _fail("SIMULATION_RUNTIME_STATE")
    exit_code = state.get("ExitCode")
    if type(exit_code) is not int:
        _fail("SIMULATION_RUNTIME_STATE")
    if (type(state.get("StartedAt")) is not str or not state["StartedAt"]
            or type(state.get("Error")) is not str or type(container.get("Created")) is not str
            or not container["Created"] or type(container.get("RestartCount")) is not int
            or container["RestartCount"] < 0):
        _fail("SIMULATION_RUNTIME_GENERATION")
    status = state.get("Status")
    if type(status) is not str:
        _fail("SIMULATION_RUNTIME_STATE")
    if (state["Restarting"] or state["Paused"] or state["OOMKilled"] or state["Error"]):
        _fail("SIMULATION_RUNTIME_STATE")
    running = state["Running"]
    if running:
        if status != "running":
            _fail("SIMULATION_RUNTIME_STATE")
    elif status not in ("created", "exited") or exit_code != 0:
        _fail("SIMULATION_RUNTIME_STATE")
    generation = (identity, container["Created"], state["StartedAt"], container["RestartCount"])
    return _Observation(generation, policy, running, status, exit_code)


def _inspect_container(docker, binding, identity, *, deadline=None, clock=None):
    value = _object(docker, "container", identity, deadline=deadline, clock=clock)
    return _validate_container(value, binding, identity)


def _require_generation(expected, observed):
    if expected != observed.generation:
        _fail("SIMULATION_RUNTIME_GENERATION")


def _require_policy(expected, observed):
    if expected != observed.restart_policy:
        _fail("SIMULATION_RUNTIME_RESTART_POLICY")


def _require_running_transition(previous, current):
    if previous is False and current is True:
        _fail("SIMULATION_RUNTIME_GENERATION")


@_api_boundary
def capture(docker, binding):
    binding = _checked_binding(binding)
    _verify_image(docker, binding)
    ids = _service_ids(docker)
    if not ids:
        if _service_ids(docker):
            _fail("SIMULATION_RUNTIME_SINGLETON")
        return _new_snapshot(binding)
    identity = ids[0]
    first = _inspect_container(docker, binding, identity)
    if _service_ids(docker) != [identity]:
        _fail("SIMULATION_RUNTIME_SINGLETON")
    _verify_image(docker, binding)
    second = _inspect_container(docker, binding, identity)
    _require_generation(first.generation, second)
    _require_policy(first.restart_policy, second)
    _require_running_transition(first.running, second.running)
    return _new_snapshot(binding, second)


@_api_boundary
def observe_unchanged(docker, binding, snapshot):
    binding = _checked_binding(binding)
    snapshot = _checked_snapshot(snapshot, binding)
    _verify_image(docker, binding)
    ids = _service_ids(docker)
    if snapshot.generation is None:
        if ids:
            _fail("SIMULATION_RUNTIME_SINGLETON")
        return _new_snapshot(binding)
    identity = snapshot.generation[0]
    if ids != [identity]:
        _fail("SIMULATION_RUNTIME_SINGLETON")
    observed = _inspect_container(docker, binding, identity)
    _require_generation(snapshot.generation, observed)
    _require_policy(snapshot.restart_policy, observed)
    _require_running_transition(snapshot.running, observed.running)
    return _new_snapshot(binding, observed)


def _clock_now(clock):
    if not callable(clock):
        _fail("SIMULATION_RUNTIME_CLOCK")
    try:
        value = clock()
    except Exception:
        _fail("SIMULATION_RUNTIME_CLOCK")
    if not _is_finite_number(value) or value < 0:
        _fail("SIMULATION_RUNTIME_CLOCK")
    return float(value)


def _remaining(deadline, clock):
    now = _clock_now(clock)
    remaining = deadline - now
    if not _is_finite_number(remaining) or remaining <= 0:
        _fail("SIMULATION_RUNTIME_TIMEOUT")
    return remaining


@_api_boundary
def drain(docker, binding, snapshot, *, clock=time.monotonic):
    started = _clock_now(clock)
    binding = _checked_binding(binding)
    snapshot = _checked_snapshot(snapshot, binding)
    deadline = started + binding.stop_timeout_seconds
    if not _is_finite_number(deadline):
        _fail("SIMULATION_RUNTIME_CLOCK")

    _verify_image(docker, binding, deadline=deadline, clock=clock)
    ids = _service_ids(docker, deadline=deadline, clock=clock)
    if snapshot.generation is None:
        if ids:
            _fail("SIMULATION_RUNTIME_SINGLETON")
        if _service_ids(docker, deadline=deadline, clock=clock):
            _fail("SIMULATION_RUNTIME_SINGLETON")
        _remaining(deadline, clock)
        return DrainResult("PROCESS_ABSENCE_OBSERVED", False, False, None)

    identity = snapshot.generation[0]
    if ids != [identity]:
        _fail("SIMULATION_RUNTIME_SINGLETON")
    current = _inspect_container(docker, binding, identity, deadline=deadline, clock=clock)
    _require_generation(snapshot.generation, current)
    _require_policy(snapshot.restart_policy, current)
    _require_running_transition(snapshot.running, current.running)

    _invoke(docker, "call", ["update", "--restart=no", identity],
            deadline=deadline, clock=clock)
    ids = _service_ids(docker, deadline=deadline, clock=clock)
    if ids != [identity]:
        _fail("SIMULATION_RUNTIME_SINGLETON")
    _verify_image(docker, binding, deadline=deadline, clock=clock)
    after_update = _inspect_container(docker, binding, identity, deadline=deadline, clock=clock)
    _require_generation(snapshot.generation, after_update)
    if after_update.restart_policy != "no":
        _fail("SIMULATION_RUNTIME_RESTART_POLICY")
    _require_running_transition(current.running, after_update.running)

    signalled = False
    if after_update.running:
        _invoke(docker, "call", ["kill", "--signal=TERM", identity],
                deadline=deadline, clock=clock)
        signalled = True
        wait_result = _invoke(docker, "call", ["wait", identity],
                              deadline=deadline, clock=clock, wait=True)
        if type(wait_result) is not str or wait_result not in ("0", "0\n"):
            _fail("SIMULATION_RUNTIME_STATE")

    ids = _service_ids(docker, deadline=deadline, clock=clock)
    if ids != [identity]:
        _fail("SIMULATION_RUNTIME_SINGLETON")
    _verify_image(docker, binding, deadline=deadline, clock=clock)
    final = _inspect_container(docker, binding, identity, deadline=deadline, clock=clock)
    _require_generation(snapshot.generation, final)
    if final.restart_policy != "no":
        _fail("SIMULATION_RUNTIME_RESTART_POLICY")
    if final.running or final.status not in ("created", "exited") or final.exit_code != 0:
        _fail("SIMULATION_RUNTIME_STATE")
    _remaining(deadline, clock)
    return DrainResult("PROCESS_EXIT_OBSERVED", True, signalled, final.generation)


if __name__ == "__main__":
    pass
