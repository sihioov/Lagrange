"""Protected, same-release Paper serving handoff under the owner's release lock."""

from __future__ import annotations

import contextlib
import dataclasses
import json
import os
from pathlib import Path
import re
import subprocess
import time
from types import SimpleNamespace
import uuid


@dataclasses.dataclass(frozen=True)
class Activation:
    generation: str
    account: str
    epoch: int
    event: str
    manifest: str
    process: str


def committed_activation(lifecycle, context, dispatch, readback):
    """Bind current database state to the original dispatched ENABLE bytes."""
    ops, require = lifecycle.ops, lifecycle.require
    generation, proposal, request = dispatch
    process_sha = ops._digest(proposal)
    manifest = context.manifest_sha256
    require(context.verified is not None and context.verified.process_sha256 == process_sha,
            "PAPER_SERVING_PROCESS")
    process = ops.release._validate_process(ops._json(proposal, lifecycle.MAX), context.commit, manifest)
    account, epoch = process["account_id"], process["control_epoch"]
    args = ops._json(request, ops.journal.MAX_NATIVE_EXACT)
    require(args["kind"] == "TRANSITION" and args["account_id"] == account and type(epoch) is int and epoch > 0
            and args["args"]["transition"] == "ENABLE"
            and args["args"]["expected_epoch"] == epoch - 1
            and args["args"]["request_key"] == generation + ":enable", "PAPER_SERVING_REQUEST")
    evidence = ops._json(args["args"]["evidence_canonical"].encode("ascii"), ops.journal.MAX_NATIVE_EXACT)
    require(evidence["process_binding"] == process and evidence["process_binding_sha256"] == process_sha
            and evidence["expected_epoch"] == epoch - 1 and evidence["new_epoch"] == epoch,
            "PAPER_SERVING_REQUEST")
    checked = lifecycle._exact_readback(readback, request)
    require(checked["status"] == "COMMITTED" and checked["current_state"] == "ENABLED"
            and checked["current_epoch"] == checked["event_epoch"] == epoch, "PAPER_SERVING_READBACK")
    event = checked["event_id"]
    result = ops._json(checked["result_canonical"].encode("ascii"), ops.MAX_PAGE)
    require(result == {"schema": "owner-beta-paper-action-v1", "action": "TRANSITION", "outcome": "OK",
                       "control_epoch": epoch, "account_id": account,
                       "data": {"event_id": event, "state": "ENABLED"}}, "PAPER_SERVING_READBACK")
    ops.journal._uuid(generation)
    return Activation(generation, account, epoch, event, manifest, process_sha)


def fresh_activation(lifecycle, owner, result):
    """Consume this invocation's clean native dispatch. Readback alone cannot launch."""
    require, ops = lifecycle.require, lifecycle.ops
    require(result is owner.last and result.outcome == "OK" and result.cleanup == "COMPLETE"
            and owner._enable_dispatch is not None and result.readback is not None
            and result.native_result is not None and result.artifact is not None,
            "PAPER_SERVING_FRESH_ENABLE")
    activation = committed_activation(lifecycle, owner.context, owner._enable_dispatch, result.readback)
    native = ops._validate_result(result.native_result, "enable", 0)
    readback = lifecycle._exact_readback(result.readback, owner._enable_dispatch[2])
    require(native["outcome"] == "OK" and native["error_code"] == "NONE"
            and native["generation"] == activation.generation and native["event_id"] == activation.event
            and native["control_epoch"] == activation.epoch
            and native["artifact_sha256"] == ops._digest(result.artifact)
            and result.artifact == readback["result_canonical"].encode("ascii")
            and owner._journal().epoch_floor == activation.epoch, "PAPER_SERVING_FRESH_ENABLE")
    owner.revalidate()
    return activation

SERVICES = ("api-server", "paper-scheduler")
SERVING_LABEL = "io.lagrange.paper-serving-generation"
PAPER_ENV = {"OWNER_BETA_PAPER_MODE": "enabled",
             "OWNER_BETA_PAPER_OFF_CONSENT_ENABLED": "false"}
STALE = "PAPER_SERVING_GENERATION"
ATTEMPT_FIELDS = {"schema", "enable_generation", "account_id", "expected_epoch",
                  "release_manifest_sha256", "process_sha256", "prior_api_generation",
                  "prior_daily_generations", "prior_proxy_generation", "installed_env_sha256"}


class Records:
    """Append-only attempt evidence. A retained record cannot launch a service."""
    def __init__(self, lifecycle, install_root):
        self.lifecycle = lifecycle
        self.tree = lifecycle.journal._Tree(install_root + "/" + lifecycle.STORE + "/queries")
        self.root = self.tree.fd

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.tree.close()

    def write(self, generation, name, value, *, create=False):
        journal = self.lifecycle.journal
        journal._uuid(generation)
        self.tree.revalidate()
        if create:
            os.mkdir(generation, 0o700, dir_fd=self.root)
            os.fsync(self.root)
        fd = journal._open_child(self.root, generation, 0o700)
        try:
            raw = journal._canonical(value)
            journal._immutable(fd, name, raw, 0o400)
            journal._check_named_dir(self.root, generation, fd)
            self.lifecycle.require(journal._read_named(fd, name, 0o400, journal.MAX_SMALL) == raw,
                                   "PAPER_SERVING_RECEIPT")
        finally:
            os.close(fd)
        self.tree.revalidate()

    def read(self, generation, name):
        journal = self.lifecycle.journal
        journal._uuid(generation)
        self.tree.revalidate()
        fd = journal._open_child(self.root, generation, 0o700)
        try:
            raw = journal._read_named(fd, name, 0o400, journal.MAX_SMALL)
            journal._check_named_dir(self.root, generation, fd)
            self.tree.revalidate()
            return journal._json(raw, journal.MAX_SMALL)
        finally:
            os.close(fd)

    def optional(self, generation, name):
        try:
            return self.read(generation, name)
        except FileNotFoundError:
            return None


def trusted_model(lifecycle, context, binding_path, before):
    daily = lifecycle.daily
    preflight = lifecycle._load("owner_beta_paper_preflight", "owner-beta-paper-preflight.py")
    source = lifecycle._load("paper_release_source_binding", "owner-beta-paper-evidence.py")
    simulation = lifecycle._load("paper_serving_simulation", "owner-beta-simulation-operations.py")
    simulation._no_pending(context.install_root)
    preflight.verify(context.release_path, binding_path, context.verified, source, daily, lifecycle.ops.release)
    binding, retained = source.binding(binding_path)
    values, overlays = daily.read_context(context.release_path, context.commit, "enable",
                                          expected_env_sha256=binding["production_env_sha256"])
    lifecycle.require(values == context.values, "PAPER_SERVING_RELEASE")
    _, mode = daily.project_binding(context.commit, context.commit)
    env = {**daily.ENVIRONMENT, "COMPOSE_PARALLEL_LIMIT": "1", "COMPOSE_PROFILES": "",
           "RANGE_RAW_BATCH_ID": "compose-config-disabled",
           "LAGRANGE_PAPER_PREFLIGHT_RELEASE": context.release_path,
           "LAGRANGE_PAPER_PREFLIGHT_API_IMAGE": context.images["api-server"],
           "LAGRANGE_PAPER_PREFLIGHT_WORKER_IMAGE": context.images["paper-scheduler"]}
    files = ["--project-name", context.project, "--env-file", context.release_path + "/deploy/compose/.env",
             "-f", context.release_path + "/deploy/compose/compose.yml", *overlays]
    compose = context.release_path + "/deploy/compose/"
    api_env = lifecycle.ops.runtime.environment(before)
    daily_mode = api_env.get("OWNER_BETA_DAILY_MODE")
    lifecycle.require(daily_mode in ("disabled", "owner_only"), "PAPER_SERVING_DAILY")
    if daily_mode == "owner_only":
        _, sha = daily.install.verify_config(context.release_path, context.commit)
        lifecycle.require(sha == binding["daily_config_sha256"], "PAPER_SERVING_DAILY")
        env["LAGRANGE_DAILY_VERIFIED_CONFIG_SHA256"] = sha
        files += ["-f", compose + "compose.owner-beta-daily.yml"]
    if mode != "absent":
        attested = subprocess.run(
            ["/usr/bin/python3", context.release_path + "/scripts/ops/lib/service-status-policy.py",
             "startup", "--install-root", context.install_root, "--release-root", context.release_path,
             "--commit", context.commit], env=daily.ENVIRONMENT, stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=30, check=False)
        lifecycle.require(attested.returncode == 0 and attested.stdout ==
                          (mode + "|" + context.project + "\n").encode("ascii"), "PAPER_SERVING_RELEASE")
        files += ["-f", compose + "compose.service-status.yml"]
    original_model = json.loads(preflight._compose_config(files, env, lambda code: lifecycle.require(False, code), services=SERVICES))
    files += ["-f", compose + preflight.OVERLAY]
    # The preflight file uses !override to replace broad source mounts. The
    # independently validated status overlay therefore follows it as well.
    if mode != "absent":
        files += ["-f", compose + "compose.service-status.yml"]
    lifecycle.require(source.binding(binding_path)[1] == retained, "PAPER_SERVING_RELEASE")
    return files, env, original_model, retained

def image_binding(context, service):
    return SimpleNamespace(project=context.project, commit=context.commit, image_id=context.images[service])


def inspect_services(lifecycle, context):
    runtime = lifecycle.ops.runtime
    observed = {}
    for service in SERVICES:
        runtime.image_owned(context.docker, image_binding(context, service))
        ids = context.docker.call(
            ["ps", "--all", "--no-trunc", "--filter", "label=com.docker.compose.project=" + context.project,
             "--filter", "label=com.docker.compose.service=" + service, "--format", "{{.ID}}"]).splitlines()
        lifecycle.require(len(ids) <= 1, STALE)
        if ids:
            selected = context.docker.object("container", ids[0])
            runtime.owned(selected, image_binding(context, service), service, ids[0])
            observed[service] = selected
    return observed


def mounts_from_model(lifecycle, model, name):
    service = model["services"][name]
    result = {}
    for entry in service.get("volumes", []):
        target = entry["target"]
        lifecycle.require(entry["type"] == "bind" and target not in result, "PAPER_SERVING_MOUNTS")
        result[target] = (entry["source"], not entry.get("read_only", False))
    for entry in service.get("secrets", []):
        key = entry["target"]
        target = key if key.startswith("/") else "/run/secrets/" + key
        item = model["secrets"][entry["source"]]
        lifecycle.require(target not in result and set(item) <= {"file", "name"}
                          and str(item["file"]).startswith("/"), "PAPER_SERVING_MOUNTS")
        result[target] = (item["file"], False)
    return result


def mount_set(container):
    bind = {}
    for item in container["Mounts"]:
        name = item["Destination"]
        if item["Type"] == "bind":
            if name in bind:
                raise ValueError("PAPER_SERVING_MOUNTS")
            bind[name] = (item["Source"], item["RW"])
        elif item["Type"] != "tmpfs":
            raise ValueError("PAPER_SERVING_MOUNTS")
    return bind


def mount_digest(lifecycle, mounts):
    return lifecycle.ops._digest(lifecycle.ops._canonical(mounts))


def validate_runtime(lifecycle, context, name, container, model):
    runtime = lifecycle.ops.runtime
    expected = model["services"][name]
    actual = container.get("HostConfig") or {}
    lifecycle.require(actual.get("ReadonlyRootfs") is True and not actual.get("Privileged")
        and not any(actual.get(key) for key in ("CapAdd", "Devices", "GroupAdd", "VolumesFrom",
                                               "PidMode", "PortBindings")), STALE)
    config = container["Config"]
    image = context.docker.object("image", context.images[name])
    defaults = image["Config"]
    lifecycle.require((config.get("User") or "") == expected.get("user", defaults.get("User") or "")
                      and config.get("Entrypoint") == expected.get("entrypoint")
                      and config.get("Cmd") == expected.get("command", defaults.get("Cmd")), STALE)
    environment = runtime.environment(image)
    environment.update({key: value for key, value in expected["environment"].items() if value is not None})
    lifecycle.require(runtime.environment(container) == environment, "PAPER_SERVING_ENV")
    lifecycle.require(mount_set(container) == mounts_from_model(lifecycle, model, name), "PAPER_SERVING_MOUNTS")
    configured = set((container["NetworkSettings"].get("Networks") or {}).keys())
    expected_networks = {model["networks"][label]["name"] for label in expected["networks"]}
    lifecycle.require(configured == expected_networks, "PAPER_SERVING_NETWORKS")


def daily_cohort(lifecycle, context, mode):
    daily, runtime = lifecycle.daily, lifecycle.ops.runtime
    binding = image_binding(context, "owner-beta-runner")
    members = runtime.capture(context.docker, binding)
    if mode == "owner_only":
        settings = context.values
        _, config_sha = daily.install.verify_config(context.release_path, context.commit)
        active = runtime.Binding(context.project, context.commit, binding.image_id, config_sha, context.release_path,
            settings["LAGRANGE_DATA_DIR"], settings["LAGRANGE_ARTIFACTS_DIR"], settings["LAGRANGE_RUNTIME_SECRET_DIR"],
            context.database["database"], settings["MAX_JOBS_PER_OWNER"], settings["PRICE_BETA_LEASE_SECS"],
            settings["PRICE_BETA_BACKOFF_SECS"])
        lifecycle.require(set(members) == set(runtime.SERVICES), "PAPER_SERVING_DAILY")
        for service, member in members.items():
            runtime.verify_policy(context.docker, active, service, member["value"])
    return {service: list(info["generation"]) for service, info in members.items()}


def proxy_generation(lifecycle, context, api):
    daily, docker = lifecycle.daily, context.docker
    source = daily.install.read_root_file(context.release_path + "/deploy/compose/compose.yml",
                                          gid=0, mode=0o644, maximum=256 * 1024)
    pins = re.findall(rb"(?m)^  reverse-proxy:\n    image: (nginxinc/nginx-unprivileged@sha256:[0-9a-f]{64}) ", source)
    lifecycle.require(len(pins) == 1, "PAPER_SERVING_EDGE")
    pinned = pins[0].decode("ascii")
    image = docker.object("image", pinned)
    ids = docker.call(["ps","--all","--no-trunc","--filter","label=com.docker.compose.project="+context.project,
                       "--filter","label=com.docker.compose.service=reverse-proxy","--format","{{.ID}}"]).splitlines()
    lifecycle.require(len(ids) == 1 and lifecycle.ops.runtime.CONTAINER.fullmatch(ids[0]), "PAPER_SERVING_EDGE")
    proxy = docker.object("container", ids[0])
    cfg = proxy["Config"]
    mount = mount_set(proxy)
    mount_path = context.release_path + "/deploy/nginx/nginx.conf"
    labels = cfg["Labels"]
    lifecycle.require(proxy["Id"] == ids[0] and proxy["Image"] == image["Id"]
                      and pinned in image["RepoDigests"] and cfg["Image"] == pinned and healthy(proxy)
                      and proxy["HostConfig"]["ReadonlyRootfs"] is True
                      and labels["com.docker.compose.project"] == context.project
                      and labels["com.docker.compose.service"] == "reverse-proxy"
                      and labels["com.docker.compose.oneoff"] == "False"
                      and mount.get("/etc/nginx/nginx.conf") == (mount_path,False),
                      "PAPER_SERVING_EDGE")
    edge = proxy["NetworkSettings"]["Networks"]
    lifecycle.require(len(edge) == 1, "PAPER_SERVING_EDGE")
    network_name, endpoint = next(iter(edge.items()))
    network = docker.object("network", endpoint["NetworkID"])
    api_endpoint = api["NetworkSettings"]["Networks"][network_name]
    lifecycle.require(network["Id"] == api_endpoint["NetworkID"] and network["Name"] == network_name
                      and network["Labels"]["com.docker.compose.project"] == context.project
                      and network["Labels"]["com.docker.compose.network"] == "edge"
                      and "api-server" in api_endpoint["Aliases"], "PAPER_SERVING_EDGE")
    for cid in set(network["Containers"]) - {api["Id"]}:
        neighbor = docker.object("container", cid)["NetworkSettings"]["Networks"][network_name]
        lifecycle.require("api-server" not in (neighbor.get("Aliases") or ()), "PAPER_SERVING_EDGE")
    return lifecycle.ops._container_generation(proxy)


def routed_session_response(headers):
    status_lines = re.findall(r"(?mi)^(?:\s*|wget: server returned error: )HTTP/1\.[01] ([0-9]{3})(?: |$)", headers)
    return len(headers) <= 8192 and bool(status_lines) and set(status_lines) == {"401"}


def require_proxy_route(lifecycle, context, proxy):
    # No Owner cookie, provider or application credentials. This exact edge
    # namespace/loopback TLS probe expects the stock API's early JSON 401.
    probe = ["/bin/sh","-c",
             "/usr/bin/wget -S -T 2 -Y off -O /dev/null --no-check-certificate "
             "https://127.0.0.1:8443/api/v1/auth/session 2>&1; exit 0"]
    deadline = time.monotonic() + 12
    while time.monotonic() < deadline:
        headers = context.docker.call(["exec","--user","101:101",proxy["Id"],*probe],timeout=4)
        if routed_session_response(headers):
            return
        time.sleep(.25)
    lifecycle.require(False, "PAPER_SERVING_EDGE")


class Plan:
    """Hold the actual same-release renderer and old disabled generation under the lock."""
    def __init__(self, lifecycle, context, binding_path):
        self.l, self.context = lifecycle, context
        self.source = lifecycle._load("paper_release_source_binding", "owner-beta-paper-evidence.py")
        self.binding_path = binding_path
        self.preflight = lifecycle._load("owner_beta_paper_preflight", "owner-beta-paper-preflight.py")
        before = inspect_services(lifecycle, context)
        lifecycle.require(set(before) == {"api-server"}, STALE)
        old = before["api-server"]
        lifecycle.require(healthy(old) and lifecycle.ops.runtime.environment(old).get("OWNER_BETA_PAPER_MODE") ==
                          "disabled", "PAPER_SERVING_DISABLED_API")
        self.files, self.env, previous, self.binding = trusted_model(lifecycle, context, binding_path, old)
        validate_runtime(lifecycle, context, "api-server", old, previous)
        self.generation = lifecycle.ops._container_generation(old)
        self.proxy = proxy_generation(lifecycle, context, old)
        require_proxy_route(lifecycle, context, self.proxy)
        self.daily_mode = lifecycle.ops.runtime.environment(old)["OWNER_BETA_DAILY_MODE"]
        self.daily_generations = daily_cohort(lifecycle, context, self.daily_mode)
        self.original = previous
        self.revalidate()

    def revalidate(self):
        self.context.revalidate()
        self.l.require(self.source.binding(self.binding_path)[1] == self.binding, "PAPER_SERVING_RELEASE")
        self.l.require(daily_cohort(self.l, self.context, self.daily_mode) == self.daily_generations,
                       "PAPER_SERVING_DAILY")

    def payload(self, activation):
        return json.dumps({"services": {role: {"environment": {
                            **PAPER_ENV, "OWNER_BETA_ACCESS_MODE": "owner_only"},
                            "labels": {SERVING_LABEL: activation.generation}} for role in SERVICES}},
                          separators=(",", ":")).encode("ascii")

    def render_enabled(self, activation):
        payload = self.payload(activation)
        fail = lambda code: self.l.require(False, code)
        model = json.loads(self.preflight._compose_config(self.files + ["-f", "-"], self.env,
                           fail, model=payload, services=SERVICES))
        base_api = self.original["services"]["api-server"]
        api, worker = (model["services"][service] for service in SERVICES)
        bind_options = self.preflight.no_create_options(self.env, fail)
        values = self.context.values
        protected = self.preflight.mounts(self.context.release_path,
                                         values["LAGRANGE_DATA_DIR"], values["LAGRANGE_ARTIFACTS_DIR"])
        self.l.require(model["name"] == self.context.project, "PAPER_SERVING_RELEASE")
        for name, selected in zip(SERVICES, (api, worker), strict=True):
            self.l.require(selected["image"] == self.context.images[name]
                and selected["labels"][SERVING_LABEL] == activation.generation and selected["user"] == "10001:10001"
                and selected.get("build") is None and selected["read_only"]
                and selected["environment"]["OWNER_BETA_ACCESS_MODE"] == "owner_only"
                and all(selected["environment"][k] == v for k,v in PAPER_ENV.items())
                and all(not selected.get(k) for k in ("privileged", "cap_add", "devices", "group_add",
                    "volumes_from", "pid", "ipc", "network_mode", "security_opt", "post_start", "pre_stop", "ports")),
                "PAPER_SERVING_MODEL")
            actual = {item["target"]: item for item in selected["volumes"]}
            self.l.require(len(actual) == len(selected["volumes"]), "PAPER_SERVING_MOUNTS")
            for target, source in protected.items():
                self.l.require(actual.get(target) == {"type":"bind","source":source,"target":target,
                    "read_only":True,"bind":bind_options}, "PAPER_SERVING_MOUNTS")
        old_env = base_api["environment"]
        self.l.require(old_env["OWNER_BETA_ACCESS_MODE"] == "owner_only" and
            {key: value for key,value in api["environment"].items() if key not in PAPER_ENV}
            == {key: value for key,value in old_env.items() if key not in PAPER_ENV}
            and api["networks"] == base_api["networks"], "PAPER_SERVING_DAILY")
        old_mounts = mounts_from_model(self.l, self.original, "api-server")
        self.l.require(old_mounts.pop("/run/owner-beta-paper", None) ==
                       (self.context.install_root + "/current/deploy/owner-beta-paper", False),
                       "PAPER_SERVING_MOUNTS")
        required_api = {**old_mounts, **{path:(source,False) for path,source in protected.items()}}
        self.l.require(mounts_from_model(self.l, model, "api-server") == required_api, "PAPER_SERVING_MOUNTS")
        scheduler = self.original["services"]["paper-scheduler"]
        scheduler_mounts = mounts_from_model(self.l, self.original, "paper-scheduler")
        for removed in ("/data", "/run/owner-beta-paper"):
            self.l.require(removed in scheduler_mounts, "PAPER_SERVING_MOUNTS")
            del scheduler_mounts[removed]
        self.l.require(worker["networks"] == scheduler["networks"] and
                       mounts_from_model(self.l, model, "paper-scheduler") == {
                           **scheduler_mounts, **{path:(source,False) for path,source in protected.items()}},
                       "PAPER_SERVING_MOUNTS")
        return model, payload

    def start(self, activation, current, record):
        self.revalidate()
        model, payload = self.render_enabled(activation)
        current()
        original = inspect_services(self.l, self.context)
        self.l.require(set(original) == {"api-server"} and
                         self.l.ops._container_generation(original["api-server"]) == self.generation, STALE)
        record("creating", {"mounts_sha256": {
            service: mount_digest(self.l, mounts_from_model(self.l, model, service)) for service in SERVICES}})
        result = subprocess.run(
            ["/usr/bin/docker", "--host", "unix:///var/run/docker.sock", "compose",
             *self.files, "-f", "-", "up", "--no-start", "--no-build", "--pull", "never",
             "--no-deps", "--force-recreate", *SERVICES],
            env=self.env, input=payload, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            timeout=70, check=False)
        self.l.require(result.returncode == 0, STALE)
        made = inspect_services(self.l, self.context)
        self.l.require(set(made) == set(SERVICES), STALE)
        identities = {}
        for service, candidate in made.items():
            identities[service] = candidate["Id"]
            generation = self.l.ops._container_generation(candidate)
            self.l.require(generation["Id"] != self.generation["Id"] and generation["RestartCount"] == 0
                            and candidate["State"]["Status"] == "created", STALE)
            self.validate(service, candidate, model, activation)
        record("created", {"container_ids": identities})
        current()
        running = {}
        for service in reversed(SERVICES):
            self.l.require(self.validate_set(identities, model, activation), STALE)
            self.context.docker.call(["start", identities[service]], timeout=20)
            until = time.monotonic() + 55
            while True:
                value = self.validate_set(identities, model, activation)[service]
                generation = self.l.ops._container_generation(value)
                self.l.require(generation["RestartCount"] == 0 and not value["State"].get("OOMKilled")
                                 and value["State"]["Status"] not in ("dead", "exited", "restarting")
                                 and time.monotonic() < until, STALE)
                if healthy(value):
                    running[service] = generation
                    record(service, {"generation": generation})
                    break
                time.sleep(.25)
            current()
        self.revalidate()
        all_ = self.validate_set(identities, model, activation)
        self.l.require(all(healthy(value) and self.l.ops._container_generation(value) == running[key]
                           for key,value in all_.items()), STALE)
        self.l.require(proxy_generation(self.l, self.context, all_["api-server"]) == self.proxy,
                       "PAPER_SERVING_EDGE")
        require_proxy_route(self.l, self.context, self.proxy)
        current()
        final = self.validate_set(identities, model, activation)
        self.l.require(proxy_generation(self.l, self.context, final["api-server"]) == self.proxy and
                       all(healthy(value) and self.l.ops._container_generation(value) == running[key]
                           for key,value in final.items()), STALE)
        return running

    def validate_set(self, identities, model, activation):
        self.revalidate()
        observed = inspect_services(self.l, self.context)
        self.l.require({key:value["Id"] for key,value in observed.items()} == identities, STALE)
        for service, item in observed.items():
            self.validate(service, item, model, activation)
        return observed

    def validate(self, service, value, model, activation):
        validate_runtime(self.l, self.context, service, value, model)
        self.l.require(value["Config"]["Labels"].get(SERVING_LABEL) == activation.generation, STALE)


def healthy(value):
    state = value["State"]
    return (state["Status"] == "running" and state["Running"] is True and
            state.get("Health", {}).get("Status") == "healthy"
            and not any(state.get(key) for key in ("OOMKilled", "Restarting", "Paused", "Dead")))

def current_activation(lifecycle, context, lock, dispatch, *, floor):
    with lifecycle.open_lifecycle(context, lock, deadline=time.monotonic_ns() + 90_000_000_000) as read:
        result = read.readback(dispatch[0])
        lifecycle.require(result.outcome == "REPLAY" and result.cleanup == "COMPLETE"
                          and result.readback is not None and result.native_result is not None,
                          "PAPER_SERVING_READBACK")
        activation = committed_activation(lifecycle, read.context, dispatch, result.readback)
        native = lifecycle.ops._validate_result(result.native_result, "readback", 0)
        lifecycle.require(native["generation"] == activation.generation and native["event_id"] == activation.event
                          and native["control_epoch"] == activation.epoch
                          and native["artifact_sha256"] == lifecycle.ops._digest(result.readback),
                          "PAPER_SERVING_READBACK")
        if floor:
            lifecycle.require(read._journal().epoch_floor == activation.epoch, "PAPER_SERVING_FLOOR")
    return activation


def begin_failure_fence(lifecycle, context, lock, activation, record):
    """Attempt only stock BEGIN/PAPER_STOP and its independent FENCED readback."""
    ops, j = lifecycle.ops, lifecycle.journal
    with ops.open_context(context, lock) as live, j.open_journal(live.install_root, lock) as store:
        lifecycle.require(live.manifest_sha256 == activation.manifest
                          and live.verified.process_sha256 == activation.process, "PAPER_SERVING_RELEASE")
        # Clean ENABLE has already advanced this floor. After lost acknowledgement,
        # only the exact independent current ENABLE event may advance its one step.
        lifecycle.require(store.epoch_floor in (activation.epoch - 1, activation.epoch), "PAPER_SERVING_FLOOR")
        if store.epoch_floor != activation.epoch:
            lifecycle.require(store.advance_floor(activation.epoch) == activation.epoch, "PAPER_SERVING_FLOOR")
        names = os.listdir(store._ops)
        lifecycle.require(len(names) <= 4096, "PAPER_SERVING_HISTORY")
        allowed = {"NONE"}
        for name in names:
            j._uuid(name)
            allowed.add(name)
        template = ("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;\n"
                    "SELECT CASE WHEN current_user='migration_owner' AND session_user=current_user "
                    "AND current_setting('role')='none' AND EXISTS(SELECT 1 "
                    "FROM public.owner_beta_paper_control WHERE account_id='{acct}'::uuid "
                    "AND state='ENABLED' AND control_epoch={epoch}) THEN COALESCE("
                    "(SELECT generation::text FROM public.owner_beta_paper_operations "
                    "WHERE account_id='{acct}'::uuid ORDER BY created_at DESC,fence_epoch DESC,generation DESC LIMIT 1),"
                    "'NONE') ELSE 'STALE' END;\nROLLBACK;\n")
        j._uuid(activation.account)
        database = lifecycle.daily.Database(live.docker, live.project, live.commit, live.images["db-migrate"])
        predecessor = database.execute(live.values,
            template.format(acct=activation.account, epoch=activation.epoch), allowed)
        live.revalidate()
        generation = str(uuid.uuid4())
        root = dict(schema="owner-beta-paper-operation-intent-v1", installation_id=store.installation_id,
                    generation=generation, account_id=activation.account,
                    predecessor_generation=None if predecessor == "NONE" else predecessor,
                    request_key=generation + ":begin", purpose="STOP", scope="PAPER_STOP",
                    expected_epoch=activation.epoch, local_epoch_floor=activation.epoch,
                    release_manifest_sha256=activation.manifest, artifact_sha256=None, resume_target_id=None)
        with contextlib.closing(store.bind_caller("serving-failure:" + activation.generation,
                                                 ops._canonical(root))) as operation:
            lifecycle.require(operation.generation == generation, "PAPER_SERVING_FENCE_REPLAY")
            record("stop-intent", {"generation":generation,"intent_sha256":operation.intent_sha256})
            stage = dict(schema="owner-beta-paper-operation-stage-v1", installation_id=store.installation_id,
                generation=generation, account_id=activation.account, sequence=0, previous_sha256=None,
                intent_sha256=operation.intent_sha256, stage="INTENT", boot_id=ops._current_boot_id(),
                observed=ops._stage_observation(), control_epoch=activation.epoch, artifact_sha256=None,
                event_id=None, error_code="NONE")
            binding = dict(schema="owner-beta-paper-action-binding-v1", installation_id=store.installation_id,
                generation=generation, root_intent_sha256=operation.intent_sha256, sequence=0,
                previous_binding_sha256=None, verb="begin", request_key=root["request_key"],
                expected_epoch=activation.epoch, artifact_sha256=None,
                stage_sha256=operation.append(ops._canonical(stage)))
            with contextlib.closing(operation.prepare_action(ops._canonical(binding))) as action:
                invocation = live.invoke("begin", action)
                with invocation:
                    invocation.import_dispatch().close()
                live.fence_begin(invocation)
                lifecycle.require(operation.revalidate() == "FENCED"
                                  and store.epoch_floor == activation.epoch + 1,
                                  "PAPER_SERVING_FENCE_UNKNOWN")
                head = operation._last[3]
                return {"generation":generation,"event_id":head["event_id"],"epoch":head["control_epoch"]}


def _check_attempt(lifecycle, live, dispatch, intent):
    ops = lifecycle.ops
    generation, proposal, request = dispatch
    q = ops._json(request, lifecycle.journal.MAX_NATIVE_EXACT)
    lifecycle.require(type(intent) is dict and set(intent) == ATTEMPT_FIELDS
        and intent["schema"] == "owner-beta-paper-serving-intent-v1" and intent["enable_generation"] == generation
        and intent["account_id"] == q["account_id"] and type(intent["expected_epoch"]) is int
        and intent["expected_epoch"] == q["args"]["expected_epoch"] and q["args"]["transition"] == "ENABLE"
        and intent["release_manifest_sha256"] == live.manifest_sha256
        and intent["installed_env_sha256"] == ops._digest(live._env_bytes)
        and intent["process_sha256"] == live.verified.process_sha256 == ops._digest(proposal),
        "PAPER_SERVING_RECEIPT")


def _existing_stop(lifecycle, live, lock, dispatch, intent, recorded):
    """Read the exact retained STOP on retry without issuing another BEGIN."""
    ops, j = lifecycle.ops, lifecycle.journal
    caller = "serving-failure:" + dispatch[0]
    with j.open_journal(live.install_root, lock) as store:
        store._scan_callers()
        name = j._caller_name(store.installation_id, intent["account_id"], caller)
        try:
            data = None if store._callers is None else j._read_named(store._callers, name, 0o400, j.MAX_SMALL)
        except FileNotFoundError:
            data = None
        if data is None:
            lifecycle.require(recorded is None, "PAPER_SERVING_FENCE_UNKNOWN")
            return None
        owner = j._validate_caller(data, store.installation_id)
        with contextlib.closing(store.open(owner["generation"])) as stop:
            root = stop._intent
            lifecycle.require(owner["caller_key"] == caller and owner["account_id"] == intent["account_id"]
                and owner["root_intent_sha256"] == stop.intent_sha256
                and (recorded is None or recorded == {"generation":stop.generation,"intent_sha256":stop.intent_sha256})
                and root["scope"] == "PAPER_STOP" and root["purpose"] == "STOP"
                and root["expected_epoch"] == intent["expected_epoch"] + 1
                and root["release_manifest_sha256"] == live.manifest_sha256, "PAPER_SERVING_FENCE_UNKNOWN")
            with contextlib.closing(store.open_custody(stop.generation, 0)) as original:
                with ops.open_custody_context(live.context, lock) as reader:
                    with reader.invoke("readback", original) as call:
                        wire, raw = call.result, call.readback_artifact()
                        read = lifecycle._exact_readback(raw, original._sql)
                        epoch = root["expected_epoch"] + 1
                        lifecycle.require(read["kind"] == "BEGIN" and read["status"] == "COMMITTED"
                            and read["current_state"] == "DRAINING" and read["current_epoch"] == read["event_epoch"] == epoch
                            and wire["outcome"] == "REPLAY" and wire["generation"] == stop.generation
                            and wire["control_epoch"] == epoch and wire["event_id"] == read["event_id"]
                            and wire["artifact_sha256"] == ops._digest(raw),
                            "PAPER_SERVING_FENCE_UNKNOWN")
                    lifecycle.require(call._closed and call._cleanup_state == "REMOVED" and not call._preserve,
                                      "PAPER_SERVING_FENCE_UNKNOWN")
            # A known committed BEGIN is also a SQL fence when its local
            # FENCED record is not safely closed. Only the exact closed
            # journal continuation allows the attended wider workflow.
            if stop.revalidate() != "FENCED" or store.epoch_floor != epoch or stop._last[3]["event_id"] != read["event_id"]:
                return {"state":"DRAINING_JOURNAL_INCOMPLETE","fence":None,"enable_generation":dispatch[0]}
            return {"state":"FENCED_DRAINING","enable_generation":dispatch[0],
                    "fence":{"generation":stop.generation,"event_id":read["event_id"],"epoch":epoch}}


def reconcile_attempt(lifecycle, live, lock, records, dispatch):
    """No re-entry may launch. Only reconcile or fence an unfinished exact action."""
    generation = dispatch[0]
    intent = records.optional(generation, "serving-intent.json")
    if intent is None:
        return
    if (records.optional(generation, "serving-started.json") is not None
            or records.optional(generation, "serving-not-started.json") is not None):
        return
    _check_attempt(lifecycle, live, dispatch, intent)
    outcome = {"state":"UNKNOWN","fence":None,"enable_generation":generation}
    def record(stage, value):
        records.write(generation, "serving-" + stage + ".json", value)
    try:
        prior = _existing_stop(lifecycle, live, lock, dispatch, intent,
                               records.optional(generation, "serving-stop-intent.json"))
        if prior is not None:
            outcome = prior
        else:
            active = current_activation(lifecycle, live.context, lock, dispatch, floor=False)
            fence = begin_failure_fence(lifecycle, live.context, lock, active, record)
            outcome.update(state="FENCED_DRAINING", fence=fence)
        if outcome["state"] == "FENCED_DRAINING":
            record("reconciled", outcome)
    except Exception:
        raise lifecycle.LifecycleError("PAPER_SERVING_START_FAILED_UNKNOWN", serving_generation=generation) from None
    raise lifecycle.LifecycleError("PAPER_SERVING_START_FAILED_" + outcome["state"], serving_generation=generation)


def serve(lifecycle, context, lock, caller_key, binding_path):
    """Only the official locked enable command can pair a fresh action with serving."""
    ops = lifecycle.ops
    plan = None
    owner = None
    attempted = None
    activation = None
    install_root = str(Path(context.release).parent.parent)
    with Records(lifecycle, install_root) as records:
        def planned(generation, account, epoch, proposal):
            nonlocal attempted
            records.write(generation, "serving-intent.json",
                {"schema":"owner-beta-paper-serving-intent-v1","enable_generation":generation,
                 "account_id":account,"expected_epoch":epoch,"release_manifest_sha256":plan.context.manifest_sha256,
                 "process_sha256":ops._digest(proposal),"prior_api_generation":plan.generation,
                 "prior_daily_generations":plan.daily_generations,
                 "prior_proxy_generation":plan.proxy,
                 "installed_env_sha256":ops._digest(plan.context._env_bytes)}, create=True)
            attempted = generation

        def record(stage, value):
            records.write(attempted, "serving-" + stage + ".json", value)

        try:
            with ops.open_context(context, lock) as live:
                prior_dispatch = None
                with lifecycle.open_lifecycle(context, lock, deadline=time.monotonic_ns() + 90_000_000_000) as owner:
                    prior = owner._prior_enable(caller_key)
                    if prior is not None:
                        prior_dispatch = owner._prior_enable_dispatch
                    else:
                        plan = Plan(lifecycle, live, binding_path)
                        owner._enable_plan = planned
                        result = owner.enable(caller_key)
                        if owner._enable_dispatch is None:
                            if attempted is not None and result.outcome == "BLOCKED" and result.cleanup == "COMPLETE":
                                record("not-started", {"native_outcome":result.outcome,"native_cleanup":result.cleanup})
                            elif attempted is not None:
                                lifecycle.require(False, "PAPER_SERVING_UNKNOWN_COMMIT")
                            return result
                        activation = fresh_activation(lifecycle, owner, result)
                        lifecycle.require(attempted == activation.generation, "PAPER_SERVING_FRESH_ENABLE")
                if prior is not None:
                    lifecycle.require(prior_dispatch is not None, "PAPER_SERVING_RECEIPT")
                    reconcile_attempt(lifecycle, live, lock, records, prior_dispatch)
                    return prior
                def recheck():
                    live.revalidate()
                    lifecycle.require(current_activation(lifecycle, context, lock, owner._enable_dispatch,
                                                         floor=True) == activation, "PAPER_SERVING_READBACK")
                record("enabled", dataclasses.asdict(activation))
                running = plan.start(activation, recheck, record)
            record("started", {"consumers":running})
            return result
        except BaseException as error:
            if attempted is None:
                raise
            # Even a partially successful daemon start never authorizes more starts.
            # Read the exact current committed ENABLE first, then only fence it.
            disposition = "UNKNOWN"
            fence = None
            try:
                lifecycle.require(owner is not None and owner._enable_dispatch is not None,
                                  "PAPER_SERVING_UNKNOWN_COMMIT")
                current = current_activation(lifecycle, context, lock, owner._enable_dispatch, floor=False)
                lifecycle.require(activation is None or current == activation, "PAPER_SERVING_READBACK")
                fence = begin_failure_fence(lifecycle, context, lock, current, record)
                disposition = "FENCED_DRAINING"
            except BaseException:
                pass
            try:
                record("failed", {"state":disposition,"fence":fence,"enable_generation":attempted})
            except Exception:
                raise lifecycle.LifecycleError("PAPER_SERVING_START_FAILED_UNKNOWN", serving_generation=attempted) from error
            raise lifecycle.LifecycleError("PAPER_SERVING_START_FAILED_" + disposition, serving_generation=attempted) from error


def validate_attended_selection(lifecycle, live, enable_generation, stop, current):
    """The operator selected an original failed start. A recorded BEGIN alone is insufficient."""
    require = lifecycle.require
    root, head = stop._intent, stop._last[3]
    key = "serving-failure:" + enable_generation
    with Records(lifecycle, live.install_root) as records:
        intent = records.read(enable_generation, "serving-intent.json")
        failed = records.optional(enable_generation, "serving-reconciled.json")
        if failed is None:
            failed = records.read(enable_generation, "serving-failed.json")
    require(type(intent) is dict and set(intent) == ATTEMPT_FIELDS
            and intent["schema"] == "owner-beta-paper-serving-intent-v1"
            and intent["enable_generation"] == enable_generation
            and intent["account_id"] == root["account_id"]
            and intent["release_manifest_sha256"] == root["release_manifest_sha256"]
            and (current is None or (live.manifest_sha256 == root["release_manifest_sha256"]
                                     and live.verified.process_sha256 == intent["process_sha256"]
                                     and lifecycle.ops._digest(live._env_bytes) == intent["installed_env_sha256"]))
            and type(intent["expected_epoch"]) is int
            and intent["expected_epoch"] + 1 == root["expected_epoch"] == root["local_epoch_floor"]
            and root["purpose"] == "STOP" and root["scope"] == "PAPER_STOP"
            and stop._bound and stop.revalidate() == "FENCED" and head["sequence"] == 1
            and head["control_epoch"] == root["expected_epoch"] + 1
            and failed == {"state":"FENCED_DRAINING","enable_generation":enable_generation,
                          "fence":{"generation":stop.generation,"event_id":head["event_id"],
                                   "epoch":head["control_epoch"]}}, "PAPER_SERVING_RECOVERY")
    store = stop._journal
    name = lifecycle.journal._caller_name(store.installation_id, root["account_id"], key)
    binding = lifecycle.journal._validate_caller(
        lifecycle.journal._read_named(store._callers, name, 0o400, lifecycle.journal.MAX_SMALL), store.installation_id)
    require(binding["caller_key"] == key and binding["generation"] == stop.generation
            and binding["account_id"] == root["account_id"]
            and binding["root_intent_sha256"] == stop.intent_sha256 and
            (current is None or current == (head["control_epoch"], "DRAINING")), "PAPER_SERVING_RECOVERY")
    return head


def clear_unstarted(lifecycle, live, enable_generation):
    """Remove only an exact never-started member of this explicitly attended attempt."""
    require, ops = lifecycle.require, lifecycle.ops
    with Records(lifecycle, live.install_root) as records:
        intent = records.read(enable_generation, "serving-intent.json")
        bound = records.optional(enable_generation, "serving-creating.json")
        created = records.optional(enable_generation, "serving-created.json")
        observed = inspect_services(lifecycle, live)
        for service, value in observed.items():
            state = value["State"]
            if state["Status"] != "created":
                continue  # Actual combined scope owns the normal running/exited writers.
            generation = ops._container_generation(value)
            require(bound is not None and bound["mounts_sha256"].get(service) == mount_digest(lifecycle, mount_set(value))
                and (created is None or created["container_ids"].get(service) == generation["Id"])
                and generation["Id"] != intent["prior_api_generation"]["Id"]
                and generation["RestartCount"] == 0 and generation["StartedAt"] == "0001-01-01T00:00:00Z"
                and state["Running"] is False and state["Pid"] == 0 and not state.get("Restarting")
                and not state.get("Paused") and not state.get("OOMKilled") and not state.get("Error")
                and value["Config"]["Labels"].get(SERVING_LABEL) == enable_generation
                and all(generation["Id"] not in group for group in ops._proc_cgroups().values()), STALE)
            record = {"service":service,"generation":generation}
            records.write(enable_generation, "serving-clearing-" + service + ".json", record)
            live.revalidate()
            require(inspect_services(lifecycle, live).get(service) == value, STALE)
            live.docker.call(["container", "rm", generation["Id"]])
            require(service not in inspect_services(lifecycle, live), STALE)
            records.write(enable_generation, "serving-cleared-" + service + ".json", record)
