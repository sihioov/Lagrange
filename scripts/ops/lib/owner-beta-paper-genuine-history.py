#!/usr/bin/env python3
"""Private genuine-history preparation and direct archive acceptance.

Imports have no process, privilege, database or provider effects. Historical
material is borrowed from protected originals, never synthesized or approved by
this module. All fixture identities are synthetic/nonproduction identities;
neither HTTP fixture authentication nor these receipts are Owner acceptance.
"""
from __future__ import annotations

import contextlib
import dataclasses
import datetime
import fcntl
import hashlib
import importlib.util
import ipaddress
import json
import os
from pathlib import Path
import re
import secrets
import selectors
import shutil
import stat
import sys
import tempfile
import time
import types
import uuid

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[3]
SCHEMA = "lagrange-paper-genuine-history-v1"
MAX_PHASE_NS = 90 * 60 * 1_000_000_000
MAX_DOCUMENT = 2 * 1024 * 1024
MAX_MATERIAL_BYTES = 2 * 1024**3


class HistoryError(Exception):
    """Closed diagnostics only; no private paths, payloads or credentials."""


def require(value, code):
    if not value:
        raise HistoryError(code)


def load(name, relative):
    path = ROOT / relative
    module = sys.modules.get(name)
    if module is None:
        spec = importlib.util.spec_from_file_location(name, path)
        require(spec is not None and spec.loader is not None, "HISTORY_SOURCE")
        module = importlib.util.module_from_spec(spec)
        sys.modules[name] = module
        spec.loader.exec_module(module)
    require(Path(module.__file__).resolve() == path, "HISTORY_SOURCE")
    return module


def fixture_module():
    return load("paper_genuine_history_fixture", "scripts/ops/owner-beta-paper-genuine-fixture.py")


def source_module():
    return load("owner_beta_paper_genuine_source", "scripts/ops/lib/owner-beta-paper-genuine-source.py")


def base_module():
    return load("paper_genuine_history_base", "scripts/ops/lib/owner-beta-daily-base.py")


def lifecycle_module():
    return load("owner_beta_paper_lifecycle", "scripts/ops/lib/owner-beta-paper-lifecycle.py")


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True,
                      allow_nan=False).encode("ascii")


def digest(data):
    return "sha256:" + hashlib.sha256(data).hexdigest()


def phase_deadline(value):
    now = time.monotonic_ns()
    require(type(value) is int and 120_000_000_000 < value - now <= MAX_PHASE_NS,
            "HISTORY_PHASE_DEADLINE")
    return fixture_module().Deadline(value)


def image_manifest(data, commit):
    """Current V3/13-image grammar; the production reader checks originals too."""
    fixture = fixture_module()
    require(type(commit) is str and re.fullmatch(r"[0-9a-f]{40}", commit)
            and commit != "0" * 40, "HISTORY_COMMIT")
    fixture.validate_v3_bytes(data, commit)
    values = {parts[1]: parts[3] for line in data.decode("ascii").splitlines()[2:]
              if (parts := line.split("|"))}
    require(tuple(values) == tuple(fixture.ops.daily.policy.MANIFEST_V3_SERVICES)
            and len(values) == 13, "HISTORY_MANIFEST")
    return values


def approved_record(source):
    """Reuse the current production registry validator, including its byte pin."""
    simulation = load("paper_genuine_history_simulation", "scripts/ops/lib/owner-beta-simulation-operations.py")
    data = (Path(source) / "configs/evidence" / simulation.APPROVAL_NAME).read_bytes()
    record = simulation.approved_record(data)
    base = base_module()
    require(record["candidate_content_sha256"] == "sha256:" + base.CANDIDATE
            and record["artifact_manifest_sha256"] == base.MANIFEST
            and record["bars_sha256"] == base.BARS and record["bars_size_bytes"] == base.BARS_SIZE,
            "HISTORY_APPROVED_BASE")
    return record


class HistoricalMaterial:
    """Retain actual V3 base and its registry-selected price/action Raw originals.

    The two original Raw commit.lock files stay shared-locked through use. Raw
    content is only read; this owner cannot publish Raw or create Source events.
    """
    def __init__(self, source, approved_base, raw_root, deadline):
        self.source, self.base_path, self.raw_path = Path(source), Path(approved_base), Path(raw_root)
        self.deadline = deadline
        self.stack = None
        self.members = {}
        self.directories = []
        self.bytes = 0
        self.record = None

    def __enter__(self):
        self.stack = contextlib.ExitStack()
        try:
            self.base = base_module()
            self.record = approved_record(self.source)
            self.base_values = self.stack.enter_context(self.base.source_artifact(self.base_path))
            self.raw_fd = self.stack.enter_context(self.base.files.open_root(
                self.base.files.Root("raw", str(self.raw_path), 10001, 10001, 0o750)))
            self._read_sources()
            self.revalidate()
            return self
        except BaseException:
            self.stack.close()
            self.stack = None
            raise

    def _open(self, relative):
        parts = self.base.files.safe_parts(relative)
        fd = self.raw_fd
        for part in parts[:-1]:
            child = os.open(part, self.base.files.DIRECTORY, dir_fd=fd)
            self.stack.callback(os.close, child)
            info = self.base.files.checked(child, directory=True, owners={10001}, groups={10001})
            self.directories.append((fd, part, child, self.base.files.identity(info)[:6]))
            fd = child
        leaf = os.open(parts[-1], self.base.files.REGULAR, dir_fd=fd)
        self.stack.callback(os.close, leaf)
        info = self.base.files.checked(leaf, directory=False, owners={10001}, groups={10001})
        require(stat.S_IMODE(info.st_mode) in (0o440, 0o640), "HISTORY_RAW_MODE")
        return fd, parts[-1], leaf, info

    def _read(self, path, maximum):
        require(path not in self.members, "HISTORY_RAW_DUPLICATE")
        parent, name, fd, info = self._open(path)
        require(info.st_size <= maximum and self.bytes + info.st_size <= MAX_MATERIAL_BYTES,
                "HISTORY_MATERIAL_LIMIT")
        data = os.pread(fd, info.st_size + 1, 0)
        require(len(data) == info.st_size, "HISTORY_MATERIAL_CHANGED")
        self.bytes += len(data)
        self.members[path] = (parent, name, fd, self.base.files.identity(info), digest(data))
        return data

    def _read_sources(self):
        refs = load("paper_genuine_history_references", "scripts/ops/lib/owner-beta-paper-backup-references.py")
        index = refs._RawIndex(refs._Budget(self.base.files.Limits(MAX_MATERIAL_BYTES)))
        for kind, provider in (("price_source", "kis-daily-range"), ("action_source", "kis")):
            self.deadline.seconds()
            expected = self.record[kind]
            scope = "manifests/provider=" + provider + "/market=kr/"
            parent, name, lock, info = self._open(scope + "commit.lock")
            require(info.st_size == 0 and stat.S_IMODE(info.st_mode) == 0o640, "HISTORY_RAW_LOCK")
            fcntl.flock(lock, fcntl.LOCK_SH | fcntl.LOCK_NB)
            self.members[scope + "commit.lock"] = (parent, name, lock, self.base.files.identity(info), digest(b""))
            manifest = self._read(scope + "manifest.jsonl", 64 * 1024 * 1024)
            require(manifest.endswith(b"\n"), "HISTORY_RAW_MANIFEST")
            matched = []  # V3 pins hash the retained physical line with its LF; Daily's exclude it.
            for number, line in enumerate(manifest[:-1].split(b"\n"), 1):  # LF only, not splitlines
                require(number <= 100000 and len(line) <= MAX_DOCUMENT, "HISTORY_RAW_MANIFEST")
                if digest(line + b"\n") != expected["manifest_line_sha256"]:  # original bytes: LF checked
                    continue
                row = refs._raw_json(line, MAX_DOCUMENT)
                require(row["provider"] == provider and row["market"] == "kr"
                        and row["batch_id"] == expected["batch_id"], "HISTORY_RAW_IDENTITY")
                batch = "provider=" + provider + "/market=kr/date=" + row["date"] + "/batch=" + row["batch_id"]
                if not matched:
                    raw = self._read(batch + "/batch.json", MAX_DOCUMENT)
                    require(digest(raw) == expected["batch_json_sha256"]
                            and refs._raw_value(refs._raw_json(raw, MAX_DOCUMENT)) == refs._raw_value(row),
                            "HISTORY_RAW_BATCH")
                    for entry in row["files"]:
                        require(type(entry["file_name"]) is str and "/" not in entry["file_name"], "HISTORY_RAW_NAME")
                        data = self._read(batch + "/" + entry["file_name"], 64 * 1024 * 1024)
                        require(len(data) == entry["size_bytes"] and digest(data) == entry["content_hash"],
                                "HISTORY_RAW_BYTES")
                index.add(row, batch, expected["batch_json_sha256"], digest(line), number, digest(line + b"\n"))
                matched.append(number)
            require(matched, "HISTORY_RAW_MANIFEST")
            refs._historical_raw_batch(index, kind, expected["batch_id"], expected["file_count"],
                                       expected["manifest_line_sha256"], expected["batch_json_sha256"])
        # This validates the pinned selected batches only. Complete transitive
        # Raw/SQL/domain closure remains the production archive consumer's job.

    def revalidate(self):
        require(self.stack is not None, "HISTORY_MATERIAL_CLOSED")
        self.deadline.seconds()
        for parent, name, fd, original in self.directories:
            require(self.base.files.identity(os.fstat(fd))[:6] == original
                    == self.base.files.identity(os.stat(name, dir_fd=parent, follow_symlinks=False))[:6],
                    "HISTORY_MATERIAL_CHANGED")
        for parent, name, fd, original, expected in self.members.values():
            info = os.fstat(fd)
            require(self.base.files.identity(info) == original
                    == self.base.files.identity(os.stat(name, dir_fd=parent, follow_symlinks=False)),
                    "HISTORY_MATERIAL_CHANGED")
            value = hashlib.sha256()
            offset = 0
            while chunk := os.pread(fd, 1024 * 1024, offset):
                self.deadline.seconds()
                value.update(chunk)
                offset += len(chunk)
            require(offset == info.st_size and "sha256:" + value.hexdigest() == expected,
                    "HISTORY_MATERIAL_CHANGED")

    def provision_views(self, daily_root):
        self.revalidate()
        outcome = self.base.provision(self.base_path, daily_root)
        self.base.verify(daily_root)
        self.revalidate()
        return outcome

    def __exit__(self, exc_type, exc, tb):
        try:
            self.revalidate()
        finally:
            stack, self.stack = self.stack, None
            stack.__exit__(exc_type, exc, tb)


def lifecycle_action(owner, action, *, caller_key=None, proposal=None, generation=None, sequence=0):
    """Consume a descriptive result only after original-owner context exit."""
    require(action in {"configure", "observe_reset", "enable", "recover", "readback"}, "HISTORY_ACTION")
    lifecycle = lifecycle_module()
    # C and the installed lifecycle load their own operations modules. The
    # production boundary requires its exact Context class; independently read
    # the same protected release instead of substituting a module or token.
    context = lifecycle_context(owner, lifecycle)
    with lifecycle.open_lifecycle(context, owner.lock_fd, deadline=owner.deadline.value) as controller:
        owner.contexts.append(controller.context)
        owner.context_modules[controller.context] = lifecycle.ops
        if action == "configure":
            result = controller.configure(caller_key, proposal)
        elif action == "recover":
            result = controller.recover(caller_key, proposal)
        elif action == "enable":
            result = controller.enable(caller_key)
        elif action == "observe_reset":
            result = controller.observe_reset()
        else:
            result = controller.readback(generation, sequence=sequence)
    require(result.cleanup == "COMPLETE" and result.outcome in {"OK", "REPLAY"},
            "HISTORY_LIFECYCLE_UNSETTLED")
    if action in {"configure", "enable", "recover"}:
        require(type(result.readback) is bytes, "HISTORY_READBACK_REQUIRED")
        journal = fixture_module().ops.journal
        document = journal._json(result.readback, journal.MAX_NATIVE_EXACT)
        require(document.get("status") == "COMMITTED", "HISTORY_READBACK_REQUIRED")
    return result


def lifecycle_context(owner, lifecycle):
    daily = lifecycle.daily
    images = daily.read_manifest(str(owner.release), owner.commit)
    values, overlays = daily.read_context(str(owner.release), owner.commit, "disable")
    context = daily.Context(str(owner.release), owner.commit, images, values, overlays)
    require(dataclasses.asdict(context) == dataclasses.asdict(owner.context), "HISTORY_CONTEXT_CHANGED")
    return context


def archive_acceptance(owner, storage_values, *, policy_root):
    """The final consumer is C's real run_locked lifecycle and fault sequence."""
    fixture = fixture_module()
    require(isinstance(owner, fixture.FixtureOwner) and owner.context is not None
            and owner.lock_fd is not None and owner.observer is not None, "HISTORY_OWNER")
    owner.deadline.seconds()
    runner = fixture.FixtureRunner(owner.source, owner.commit, owner.manifest, owner.deadline, policy_root)
    # C independently requires genuine nonempty source rows/originals. There is
    # no ready flag, test-origin ENABLE receipt or caller-supplied success result.
    runner.archive_acceptance(owner, storage_values)
    expected = ["ARCHIVE_LIFECYCLE", *("ARCHIVE_" + name.upper().replace("-", "_")
        for name in fixture.driver.ArchiveAcceptance.FAULTS)]
    require([row.get("case") for row in runner.results] == expected
            and all(row.get("status") == "PASS" for row in runner.results),
            "HISTORY_ACCEPTANCE_UNPROVEN")
    return tuple(runner.results)


def producer_runtime(owner):
    """Reuse existing owned image/container primitives, with the inherited cutoff.

    Only producer containers created by this adapter enter its disposal list.
    The source PostgreSQL/network remain owned by the actual FixtureOwner.
    """
    wholeflow = load("paper_genuine_history_wholeflow", "scripts/ops/verify-owner-beta-daily-wholeflow.py")

    class Runtime(wholeflow.WholeFlow):
        def __init__(self):
            super().__init__(types.SimpleNamespace(candidate=owner.commit, timeout_seconds=1))
            self.owner = owner
            self.root = owner.root
            self.images = image_manifest(owner.manifest, owner.commit)
            self.network, self.server = owner.network, owner.pg
            self.created = []
            self.api_container = None

        def remaining(self, limit=60):
            return owner.deadline.seconds(limit)

        def command(self, args, payload=None, *, limit=60, output=None, cleanup=False,
                    include_stderr=False, failure_service=None):
            require(output is None, "HISTORY_PRODUCER_OUTPUT")
            result = owner.command(args, payload=payload, cap=limit, cleanup=cleanup, allow_failure=True)
            require(result.returncode == 0, "HISTORY_PRODUCER_FAILED")
            require(len(result.stdout) + len(result.stderr) <= MAX_DOCUMENT, "HISTORY_PRODUCER_OUTPUT")
            return result.stdout + (result.stderr if include_stderr else b"")

        def sql(self, statement):
            return owner.observer.sql(statement)

        def create(self, service, image, uid, network, mounts, env, entry, args=(), *, alias=None):
            require(image in self.images.values(), "HISTORY_PRODUCER_IMAGE")
            original = self.inspect("image", image)
            require(original["Id"] == image and (original["Config"].get("Labels") or {}).get(
                "org.opencontainers.image.revision") == owner.commit, "HISTORY_PRODUCER_IMAGE")
            cid = super().create(service, image, uid, network, mounts, env, entry, args, alias=alias)
            self.created.append(cid)
            return cid

        def intent(self, kind, name):
            self.intents.append({"kind": kind, "name": name})
            wholeflow.private_file(self.root / (self.nonce + "-intent-%03d.json" % len(self.intents)),
                canonical({"nonce": self.nonce, **self.intents[-1]}))

        def dataset_environment(self, pin):
            identifier = str(uuid.UUID(pin["id"]))
            generation = owner.observer.json("SELECT to_json(curated_generation) FROM candidate_price_publications "
                "WHERE dataset_version_id='" + identifier + "'::uuid;", role="migration_owner")
            require(type(generation) is int and generation > 0, "HISTORY_GENERIC_GENERATION")
            return {"RECOMMENDATION_DATASET_VERSION_ID": identifier,
                "RECOMMENDATION_DATASET_ID": pin["dataset_id"], "RECOMMENDATION_DATASET_VERSION": pin["version"],
                "RECOMMENDATION_CURATED_VERSION": str(generation),
                "RECOMMENDATION_DATASET_MANIFEST_SHA256": pin["manifest_sha256"]}

        def start_api(self, pin):
            env = {"APP_ENV": "production", "APP_HOST": "0.0.0.0", "APP_PORT": "8080",
                "LAGRANGE_CODE_COMMIT": owner.commit, "CURSOR_SECRET_FILE": "/run/secrets/cursor",
                "AUTH0_DOMAIN": "paper-genuine-fixture.invalid", "AUTH0_CLIENT_ID": "synthetic-fixture",
                "AUTH0_CLIENT_SECRET_FILE": "/run/secrets/auth0",
                "AUTH0_REDIRECT_URI": "https://paper-genuine-fixture.invalid/auth/callback",
                "SESSION_COOKIE_SECURE": "true", "ARTIFACT_ROOT": "/data/artifacts",
                "OWNER_BETA_ACCESS_MODE": "owner_only", "OWNER_BETA_PRICE_INPUT_MODE": "sealed_v1",
                "OWNER_BETA_DAILY_MODE": "owner_only", "OWNER_BETA_PAPER_MODE": "enabled",
                "DB_APP_MAX_CONNECTIONS": "2", "DB_ADMIN_MAX_CONNECTIONS": "1", "DB_AUDIT_MAX_CONNECTIONS": "1",
                **self.dataset_environment(pin)}
            mounts = [self.mount(self.base, "/data/artifacts"),
                self.mount(self.secret_root / "cursor", "/run/secrets/cursor"),
                self.mount(self.secret_root / "auth0", "/run/secrets/auth0")]
            for role, prefix in (("app", "DB"), ("admin", "ADMIN_DB"), ("audit_writer", "AUDIT_DB")):
                values, secrets_mounts = self.db_env(role, 10001, prefix)
                env.update(values)
                mounts += secrets_mounts
            self.api_container = self.create("api", self.images["api-server"], 10001, self.network,
                mounts, env, "/usr/local/bin/api-server")
            self.command(["start", self.api_container])
            observed = self.inspect("container", self.api_container)
            networks = list(observed["NetworkSettings"]["Networks"].values())
            require(len(networks) == 1 and networks[0]["NetworkID"] == self.network, "HISTORY_API_NETWORK")
            self.api_address = networks[0]["IPAddress"]
            address = ipaddress.ip_address(self.api_address)
            require(address.version == 4 and address.is_private and not address.is_loopback, "HISTORY_API_NETWORK")
            until = time.monotonic() + self.remaining(45)
            while time.monotonic() < until:
                try:
                    if self.http("GET", "/readyz", authenticated=False)[0] == 200:
                        return
                except (OSError, wholeflow.http.client.HTTPException):
                    pass
                time.sleep(min(0.1, self.remaining(0.1)))
            raise HistoryError("HISTORY_API_UNREADY")

        def close(self):
            # A create ACK loss is unresolved custody, never proof of absence.
            require(not self.uncertain_creations, "HISTORY_PRODUCER_CREATE_UNKNOWN")
            for cid in reversed(self.created):
                info = self.inspect("container", cid, cleanup=True)
                require(info["Id"] == cid and info["Config"]["Labels"].get(wholeflow.LABEL) == self.nonce,
                        "HISTORY_FOREIGN_PRODUCER")
                if info["State"]["Running"]:
                    self.command(["stop", "--time", "5", cid], cleanup=True)
                self.command(["rm", "--volumes", cid], cleanup=True)
                require(self.command(["ps", "-aq", "--no-trunc", "--filter", "id=" + cid], cleanup=True) == b"",
                        "HISTORY_PRODUCER_CLEANUP")
                self.created.remove(cid)

    return Runtime()


def initialize_runtime(owner, material, config_source, *, existing=False):
    """Prepare disposable Daily/user inputs using the established fixture rules.

    No admissions, jobs, run links, ledger entries or consent rows are inserted.
    Producer-independent user/entitlement/control setup follows the existing
    private wholeflow fixture and is explicitly synthetic/nonproduction.
    """
    return _initialize_runtime_material(owner, material, config_source,
        owner.release / "configs/evidence/owner-beta-daily-price-policy-v1.json", existing=existing)


def _initialize_runtime_material(owner, material, config_source, policy_source, *, existing=False):
    """Prepare role views/credentials without constructing an installed Context.

    The ordinary caller supplies the published policy. The native fixture's
    pre-publication callback supplies its held, owned stage's policy instead.
    No helper may run until the normal release publication has completed.
    """
    wholeflow = load("paper_genuine_history_wholeflow", "scripts/ops/verify-owner-beta-daily-wholeflow.py")
    runtime = producer_runtime(owner)
    install = wholeflow.release
    config_bytes = install.read_root_file(str(config_source), gid=0, mode=0o600, maximum=16384)
    config = install.validate_config(config_bytes, owner.commit)
    require(config["entitlement_reference"].startswith("fixture-only:"), "HISTORY_SYNTHETIC_ENTITLEMENT")
    require(config["raw_owner_uid"] == 10001, "HISTORY_RAW_OWNER")
    runtime.config, runtime.config_bytes = config, config_bytes
    runtime.policy = Path(policy_source).read_bytes()
    install.validate_policy(runtime.policy)
    runtime.current = owner.root / "data"
    runtime.raw = runtime.current / "raw"
    runtime.generic_raw = runtime.raw
    runtime.curated = runtime.current / "curated"
    runtime.daily = runtime.current / "owner-beta-daily"
    runtime.config_root = (owner.release / install.CONFIG_DIRECTORY if existing
                           else runtime.current / "genuine-config")
    runtime.base = owner.root / "artifacts/historical-price-beta-root"
    runtime.secret_root = owner.root / "genuine-secrets"
    runtime.secret_paths = {}
    if not existing:
        wholeflow.directory(runtime.current, 0, 10001, 0o750)
        wholeflow.directory(runtime.config_root, 0, 10001, 0o750)
        wholeflow.directory(runtime.curated, 10001, 10001, 0o750)
        wholeflow.directory(runtime.secret_root)
        wholeflow.private_file(runtime.config_root / "config.json", config_bytes, 0, 10001, 0o440)
        wholeflow.private_file(runtime.config_root / "policy.json", runtime.policy, 0, 10001, 0o440)
        install.provision_state(str(runtime.current), config)
        material.provision_views(runtime.daily)
        copy_raw(material, runtime.raw)
        wholeflow.directory(owner.root / "artifacts", 0, 10001, 0o750)
        wholeflow.directory(runtime.base, 10001, 10001, 0o750)
        candidate = wholeflow.directory(runtime.base / material.base.CANDIDATE, 10001, 10001, 0o700)
        for leaf, data in material.base_values.items():
            wholeflow.private_file(candidate / leaf, data, 10001, 10001, 0o600)
        with material.base.source_artifact(runtime.base):
            pass
    else:
        require(install.read_root_file(str(runtime.config_root / "config.json"), gid=10001, mode=0o440)
                == config_bytes, "HISTORY_CONFIG_CHANGED")
        require(install.read_root_file(str(runtime.config_root / "policy.json"), gid=10001, mode=0o440)
                == runtime.policy, "HISTORY_POLICY_CHANGED")
        material.base.verify(runtime.daily)
    for role in wholeflow.ROLES:
        # The migration owner already has an actual protected secret from C's
        # database setup. Other roles get disposable credentials, never literals.
        if role == "migration_owner":
            # Its source is UID999 rather than a root file; use the same bound
            # service reader as the real fixture, without changing source mode.
            with install.root_directory(str(owner.root / "secrets")) as parent:
                with install.child_directory(parent, "db-migrate", 999, 999, 0o750) as service:
                    password = install.read_file(service, "db_migration_owner_password", 999, 999, 0o400)
        elif not existing:
            password = secrets.token_hex(32).encode()
            owner.observer.sql("ALTER ROLE " + role + " PASSWORD '" + password.decode("ascii") + "';")
        else:
            password = None
        for uid in (999, 10001, 10003):
            path = runtime.secret_root / (role + "-" + str(uid))
            if not existing:
                wholeflow.private_file(path, password, uid, uid, 0o400)
            runtime.secret_paths[(role, uid)] = path
        if not existing and role != "migration_owner":
            role_leaf = "db_" + wholeflow.ROLES[role] + "_password"
            services = {"api-server": {"app", "admin", "audit_writer"},
                "paper-scheduler": {"app", "worker", "admin", "audit_writer"},
                "owner-beta-runner": {"worker"}, "owner-beta-simulation-runner": {"worker"},
                "research-worker": {"research_writer"}}
            for service, roles in services.items():
                if role in roles:
                    service_root = owner.root / "secrets" / service
                    if not service_root.exists():
                        wholeflow.directory(service_root, 10001, 10001, 0o750)
                    wholeflow.private_file(service_root / role_leaf, password, 10001, 10001, 0o400)
        password = None
    for name in ("cursor", "auth0"):
        if not existing:
            wholeflow.private_file(runtime.secret_root / name, secrets.token_hex(32).encode(), 10001, 10001, 0o400)
    # Synthetic session setup is the same explicit private-fixture exception as
    # WholeFlow.seed; it is never an Auth0 or actual Owner consent assertion.
    runtime.cookie, runtime.csrf = secrets.token_urlsafe(32), secrets.token_hex(32)
    return runtime


def copy_raw(material, destination):
    """Copy protected originals to a new private root while all writers lock out."""
    base = material.base
    require(not destination.exists() and not destination.is_symlink(), "HISTORY_RAW_DESTINATION")
    # Discover and retain all existing coordination locks before copying. The
    # two approved-history scopes are already held by HistoricalMaterial.
    manifests = material.raw_path / "manifests"
    for provider in sorted(manifests.iterdir()):
        require(provider.is_dir() and not provider.is_symlink(), "HISTORY_RAW_SCOPE")
        for market in sorted(provider.iterdir()):
            path = str((market / "commit.lock").relative_to(material.raw_path))
            require(base.files.paper_raw_commit_lock("raw/" + path), "HISTORY_RAW_SCOPE")
            if path not in material.members:
                parent, name, fd, info = material._open(path)
                require(info.st_size == 0, "HISTORY_RAW_LOCK")
                fcntl.flock(fd, fcntl.LOCK_SH | fcntl.LOCK_NB)
                material.members[path] = (parent, name, fd, base.files.identity(info), digest(b""))
    total = entries = 0
    def copy(source_fd, target):
        nonlocal total, entries
        material.deadline.seconds()
        info = base.files.checked(source_fd, directory=True, owners={10001}, groups={10001})
        target.mkdir(mode=0o700)
        names = sorted(os.listdir(source_fd))
        for name in names:
            entries += 1
            require(entries <= 200000 and len(base.files.safe_parts(name)) == 1, "HISTORY_MATERIAL_LIMIT")
            original = os.stat(name, dir_fd=source_fd, follow_symlinks=False)
            directory = stat.S_ISDIR(original.st_mode)
            fd = os.open(name, base.files.DIRECTORY if directory else base.files.REGULAR, dir_fd=source_fd)
            try:
                before = base.files.checked(fd, directory=directory, owners={10001}, groups={10001})
                require(base.files.identity(before) == base.files.identity(original), "HISTORY_MATERIAL_CHANGED")
                if directory:
                    copy(fd, target / name)
                else:
                    total += before.st_size
                    require(total <= MAX_MATERIAL_BYTES, "HISTORY_MATERIAL_LIMIT")
                    with (target / name).open("xb") as output:
                        offset = 0
                        while chunk := os.pread(fd, 1024 * 1024, offset):
                            material.deadline.seconds()
                            output.write(chunk)
                            offset += len(chunk)
                        require(offset == before.st_size, "HISTORY_MATERIAL_CHANGED")
                        output.flush()
                        os.fchown(output.fileno(), before.st_uid, before.st_gid)
                        os.fchmod(output.fileno(), stat.S_IMODE(before.st_mode))
                        os.fsync(output.fileno())
                require(base.files.identity(os.fstat(fd)) == base.files.identity(before)
                    == base.files.identity(os.stat(name, dir_fd=source_fd, follow_symlinks=False)), "HISTORY_MATERIAL_CHANGED")
            finally:
                os.close(fd)
        require(sorted(os.listdir(source_fd)) == names, "HISTORY_MATERIAL_CHANGED")
        os.chown(target, info.st_uid, info.st_gid)
        target.chmod(stat.S_IMODE(info.st_mode))
        fd = os.open(target, base.files.DIRECTORY)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)
    copy(material.raw_fd, destination)
    material.revalidate()


def seed_user_and_control(runtime, *, existing=False):
    owner = str(uuid.UUID(runtime.config["owner"]))
    literal = lambda text: "'" + str(text).replace("'", "''") + "'"
    if not existing:
        instruments = ",".join("('%s.KRX','%s','KRX','KRW','SYNTHETIC_FIXTURE','ETF','ACTIVE')" % (s, s)
            for s in runtime.config["instruments"]) if "instruments" in runtime.config else ",".join(
                "('%s.KRX','%s','KRX','KRW','SYNTHETIC_FIXTURE','ETF','ACTIVE')" % (s, s)
                for s in load("paper_genuine_history_wholeflow", "scripts/ops/verify-owner-beta-daily-wholeflow.py").fixture.SYMBOLS)
        runtime.sql("BEGIN; INSERT INTO instruments(id,symbol,venue,currency,name,asset_class,status) VALUES "
            + instruments + ";INSERT INTO users(id,issuer,subject,email) VALUES('" + owner
            + "','paper-genuine-fixture.invalid','synthetic','synthetic@paper-genuine-fixture.invalid');"
            "INSERT INTO user_roles(user_id,role_id,granted_by) VALUES('" + owner + "','owner','" + owner + "');"
            "INSERT INTO data_entitlements(contract_reference,contract_document_sha256,status,covered_datasets,"
            "covered_uses,effective_from,effective_until,managed_by) VALUES("
            + literal(runtime.config["entitlement_reference"]) + "," + literal(runtime.config["entitlement_sha256"][7:])
            + ",'ACTIVE','[\"krx_eod_bars\"]','[\"recommendation\",\"paper_view\",\"backtest\"]',"
            "'2016-08-29','2099-12-31','" + owner + "'); COMMIT;")
        template = (runtime.owner.release / "deploy/db/owner-beta-daily-control.sql").read_text()
        # A source preparer initially configures Daily OFF. Its real immutable
        # Source activation remains a separate authorized existing service step.
        replacements = {"__ACTION__": "prepare", "__CONFIG_HEX__": runtime.config_bytes.hex(),
            "__POLICY_HEX__": runtime.policy.hex(), "__COMMIT__": runtime.owner.commit,
            "__CAPACITY__": "10", "__LEASE__": "60", "__BACKOFF__": "30"}
        for key, value in replacements.items():
            template = template.replace(key, value)
        env, mounts = runtime.db_env("migration_owner", 999)
        result = runtime.once("control", runtime.images["db-migrate"], 999, runtime.network, mounts, env,
            "/usr/local/bin/lagrange-daily-control", payload=template.encode())
        require(result.strip() == b"DAILY_CONTROL: DISABLED", "HISTORY_DAILY_CONTROL")
    runtime.sql("INSERT INTO web_sessions(id,user_id,session_hash,csrf_hash,expires_at) VALUES(gen_random_uuid(),'"
        + owner + "','" + hashlib.sha256(runtime.cookie.encode()).hexdigest() + "','"
        + hashlib.sha256(runtime.csrf.encode()).hexdigest() + "',now()+interval '90 minutes');")


def dataset_pin(owner, batch_id):
    """Read the actual catalog publication linked to this collector output."""
    require(type(batch_id) is str and str(uuid.UUID(batch_id)) == batch_id, "HISTORY_GENERIC_BATCH")
    # SQL shape is the catalog's six-field public pin. The batch identity is
    # supplied by the actual collector output, never a fabricated dataset row.
    rows = owner.observer.json("SELECT coalesce(json_agg(json_build_object('id',id,'dataset_id',dataset_id,"
        "'version',version,'status',status,'manifest_sha256',manifest_sha256,'storage_path',storage_path)),"
        "'[]'::json) FROM dataset_versions WHERE id IN (SELECT dataset_version_id "
        "FROM candidate_raw_batch_datasets WHERE batch_id='" + batch_id + "'::uuid "
        "AND surface='price' AND response_kind='bars');", role="migration_owner")
    require(type(rows) is list and len(rows) == 1, "HISTORY_GENERIC_DATASET")
    row = rows[0]
    require(set(row) == {"id", "dataset_id", "version", "status", "manifest_sha256", "storage_path"}
            and row["status"] in {"READY", "WARNING"}, "HISTORY_GENERIC_DATASET")
    return row


def collect_generic(runtime, session, bundle_root):
    """Real synthetic nonproduction collector -> Raw -> Curated -> catalog.

    This generic lane never emits beta Source events and never calls the
    credentialed EOD/session command. The runtime owns only its disposable roots.
    """
    require(datetime.date.fromisoformat(session).isoformat() == session, "HISTORY_GENERIC_DATE")
    env, mounts = runtime.db_env("research_writer", 10001)
    env.update(APP_ENV="qa", RESEARCH_FETCH_MODE="synthetic",
        RESEARCH_RAW_ROOT="/data/raw", RESEARCH_CURATED_ROOT="/data/curated",
        RESEARCH_SYNTHETIC_BUNDLE="/data/synthetic", RESEARCH_CANDIDATE_ENABLED="false",
        RESEARCH_ENTITLEMENT_REFERENCE=runtime.config["entitlement_reference"])
    mounts += [runtime.mount(runtime.generic_raw, "/data/raw", False),
               runtime.mount(runtime.curated, "/data/curated", False),
               runtime.mount(bundle_root, "/data/synthetic")]
    output = runtime.once("generic-collector", runtime.images["research-worker"], 10001,
        runtime.network, mounts, env, "/usr/local/bin/research-worker", ["--once", "--date", session])
    records = [json.loads(line) for line in output.splitlines() if line.startswith(b"{")]
    completed = [row for row in records if row.get("status") == "ok" and row.get("phase") == "complete"]
    require(len(completed) == 1 and type(completed[0].get("batch_id")) is str, "HISTORY_GENERIC_PUBLICATION")
    return dataset_pin(runtime.owner, completed[0]["batch_id"])


def daily_produce(runtime):
    """Run actual Daily producers; return the existing Source reservation.

    This does not create Source provenance. Only the immutable-release Source
    service may acquire/arm genuine future sessions; existing admitted history
    is checked later by the native financial and archive consumers.
    """
    owner = str(uuid.UUID(runtime.config["owner"]))
    # Existing Source's real scheduler owns reservation and linking. Running
    # these independent Daily workers cannot create either authority. A slot
    # row is only a prerequisite here; native validation later checks originals.
    slot = runtime.owner.observer.json("SELECT to_json(slot_id) FROM owner_beta_paper_source_slots "
        "WHERE owner_user_id='" + owner + "'::uuid AND session_date=(clock_timestamp() AT TIME ZONE 'Asia/Seoul')::date "
        "AND cohort->>'verifier_revision'='" + runtime.owner.commit + "';", role="migration_owner")
    require(type(slot) is str and re.fullmatch(r"sha256:[0-9a-f]{64}", slot), "HISTORY_SOURCE_RESERVATION_REQUIRED")
    runtime.verify_base_views()
    runtime.daily_once("producer", "CANDIDATE_READY")
    runtime.daily_once("admitter", "ADMITTED")
    runtime.daily_once("worker", "SUCCEEDED")
    runtime.verify_base_views()
    return slot


def daily_cycle(runtime):
    """Produce Daily work and wait for the independently running Source link."""
    slot = daily_produce(runtime)
    until = time.monotonic() + runtime.remaining(180)
    while time.monotonic() < until:
        linked = runtime.owner.observer.json("SELECT to_json(count(*)) FROM owner_beta_paper_source_runs r "
            "JOIN owner_beta_paper_source_admissions a USING(slot_id) WHERE r.slot_id='" + slot + "';",
            role="migration_owner")
        if linked == 1:
            return
        time.sleep(min(0.2, runtime.remaining(0.2)))
    raise HistoryError("HISTORY_SOURCE_LINK_PENDING")


def http_result(runtime, method, path, body, statuses):
    status, value = runtime.http(method, path, body)
    require(status in statuses and type(value) is dict, "HISTORY_HTTP_REJECTED")
    return value


def create_beta_config(runtime):
    """Actual adopted Trend HTTP config, distinct from the generic BuyAndHold.

    The returned ID is independently read through the owned DB. Neither a
    relabeled generic config nor a direct INSERT can satisfy this constructor.
    """
    config = {"benchmark_instrument": "069500.KRX", "fast_ma": 50, "slow_ma": 200}
    result = http_result(runtime, "POST", "/api/v1/strategies/trend_following/configs",
        {"strategy_version": "1.0.0", "config": config, "is_active": True,
         "owner_beta_daily_enabled": True}, {201})
    identifier = str(uuid.UUID(result["id"]))
    owner = str(uuid.UUID(runtime.config["owner"]))
    actual = runtime.owner.observer.json(
        "BEGIN READ ONLY; DO $$ BEGIN PERFORM set_config('app.actor_user_id','" + owner + "',true); END $$;"
        "SELECT jsonb_build_object('id',id,'owner',owner_user_id,'strategy',strategy_id,"
        "'version',strategy_version,'config',config_json,'active',is_active,'daily',daily_enabled,"
        "'epoch',daily_epoch) FROM public.user_strategy_configs WHERE id='" + identifier + "'::uuid; ROLLBACK;",
        role="migration_owner")
    require(type(actual) is dict and actual.get("id") == identifier and actual.get("owner") == owner
            and actual.get("strategy") == "trend_following" and actual.get("version") == "1.0.0"
            and actual.get("config") == config and actual.get("active") is True and actual.get("daily") is True
            and type(actual.get("epoch")) is int and actual["epoch"] >= 0, "HISTORY_BETA_CONFIG")
    return identifier


def generic_preview(runtime, strategy_id, session):
    """Actual HTTP originals; the generic preview is deliberately never applied."""
    account = http_result(runtime, "POST", "/api/v1/paper/accounts",
        {"name": "SYNTHETIC generic preview only", "currency": "KRW", "initial_cash": "10000000"}, {201})
    http_result(runtime, "POST", "/api/v1/paper/accounts/" + str(uuid.UUID(account["id"])) + "/bind-strategy",
        {"strategy_config_id": str(uuid.UUID(strategy_id)), "auto_apply_recommendations": False}, {200})
    run = http_result(runtime, "POST", "/api/v1/recommendations/runs",
        {"strategy_config_id": strategy_id, "as_of": session}, {201, 202})
    require(type(account.get("id")) is str and type(run.get("id")) is str, "HISTORY_HTTP_IDENTITY")
    return account, run


def create_generic_preview(runtime, account_id, run_id):
    return http_result(runtime, "POST", "/api/v1/paper/accounts/" + str(uuid.UUID(account_id))
        + "/recommendation-previews", {"recommendation_run_id": str(uuid.UUID(run_id))}, {201, 202})


def consent(runtime, expected_epoch, *, opt_in):
    require(type(expected_epoch) is int and expected_epoch >= 0 and type(opt_in) is bool, "HISTORY_CONSENT_INPUT")
    return http_result(runtime, "POST", "/api/v1/paper/owner-beta/consent",
        {"opt_in": opt_in, "expected_epoch": expected_epoch}, {200, 201})


def beta_preview(runtime, run_id, expected_epoch):
    preview = http_result(runtime, "POST", "/api/v1/paper/owner-beta/previews",
        {"recommendation_run_id": str(uuid.UUID(run_id)), "expected_epoch": expected_epoch}, {201, 202})
    # The actual worker must settle this preview before any token is usable.
    return preview


def apply_beta_preview(runtime, preview_id, expected_epoch):
    preview = http_result(runtime, "GET", "/api/v1/paper/owner-beta/previews/" + str(uuid.UUID(preview_id)),
        None, {200})
    require(type(preview.get("preview_token")) is str and preview["preview_token"], "HISTORY_PREVIEW_PENDING")
    return http_result(runtime, "POST", "/api/v1/paper/owner-beta/previews/" + str(uuid.UUID(preview_id)) + "/apply",
        {"preview_token": preview["preview_token"], "expected_epoch": expected_epoch}, {200, 201, 202})


def simulation_pair(runtime, strategy_id):
    submitted = http_result(runtime, "POST", "/api/v1/backtests/owner-beta/price-only/runs",
        {"strategy_config_id": strategy_id}, {201, 202})
    require(type(submitted.get("run_id")) is str, "HISTORY_SIMULATION_IDENTITY")
    return submitted


def read_simulation_pair(runtime, run_id):
    return http_result(runtime, "GET", "/api/v1/backtests/owner-beta/price-only/runs/"
        + str(uuid.UUID(run_id)) + "/pair", None, {200})


def recommendation_once(runtime, pin):
    env, mounts = runtime.db_env("worker", 10001)
    env.update(APP_ENV="production", RECOMMENDATION_HEALTH_STATE_PATH="/tmp/recommendation-health.json",
               **runtime.dataset_environment(pin))
    mounts += [runtime.mount(runtime.curated, "/data/curated"),
        runtime.mount(runtime.owner.release / "configs/universes/kr-etf-core-v1.yaml",
                      "/opt/lagrange/configs/universes/kr-etf-core-v1.yaml")]
    runtime.once("recommendation", runtime.images["recommendation-runner"], 10001, runtime.network,
        mounts, env, "/usr/local/bin/recommendation-runner", ["--once", "--worker-id", runtime.nonce,
        "--repo-root", "/opt/lagrange", "--data-root", "/data/curated", "--universe-manifest",
        "/opt/lagrange/configs/universes/kr-etf-core-v1.yaml", "--uv-bin", "/usr/local/bin/uv", "--temp-root", "/tmp"])


def simulation_once(runtime):
    simulation = load("paper_genuine_history_simulation", "scripts/ops/lib/owner-beta-simulation-operations.py")
    binding = simulation.binding_from_release(runtime.owner.project, str(runtime.owner.release), runtime.owner.commit)
    require(binding.image_id == runtime.images["owner-beta-simulation-runner"], "HISTORY_SIMULATION_IMAGE")
    env, mounts = runtime.db_env("worker", 10001)
    env.update(APP_ENV="production", OWNER_BETA_SIMULATION_MODE="owner_only", OWNER_BETA_ACCESS_MODE="owner_only",
        OWNER_BETA_PRICE_INPUT_MODE="sealed_v1", LAGRANGE_CODE_COMMIT=runtime.owner.commit,
        OWNER_BETA_SIMULATION_CONFIG_SHA256=binding.config_sha256,
        OWNER_BETA_SIMULATION_RECEIPT_SHA256=binding.receipt_sha256[7:])
    mounts += [runtime.mount(runtime.owner.release / ".owner-beta-simulation/config.json", "/run/owner-beta-simulation/config.json"),
        runtime.mount(binding.historical_root, "/data/owner-beta-artifacts")]
    runtime.once("simulation", binding.image_id, 10001, runtime.network, mounts, env,
        "/usr/local/bin/owner-beta-simulation-runner", ["--once", "--artifact-root", "/data/owner-beta-artifacts",
        "--build-receipt", "/usr/share/lagrange/owner-beta-simulation/artifact.json",
        "--expected-receipt-sha256", binding.receipt_sha256[7:], "--worker-id", runtime.nonce])


def paper_command(runtime):
    env = {"APP_ENV": "production", "LAGRANGE_CODE_COMMIT": runtime.owner.commit,
        "OWNER_BETA_ACCESS_MODE": "owner_only", "OWNER_BETA_PAPER_MODE": "owner_only",
        "DB_HOST": "postgres", "DB_PORT": "5432", "DB_NAME": "lagrange",
        "LAGRANGE_REPO_ROOT": "/opt/lagrange", "LAGRANGE_DATASET_ROOT": "/data",
        "ARTIFACT_PAPER_DIR": "/data/artifacts/paper", "PAPER_HEALTH_STATE_PATH": "/tmp/paper-health.json"}
    mounts = [runtime.mount(runtime.curated, "/data/curated"),
              runtime.mount(runtime.owner.release, "/opt/lagrange")]
    paper_root = runtime.owner.root / "artifacts/paper"
    if not paper_root.exists():
        load("paper_genuine_history_wholeflow", "scripts/ops/verify-owner-beta-daily-wholeflow.py").directory(paper_root, 10001, 10001, 0o750)
    mounts.append(runtime.mount(paper_root, "/data/artifacts/paper", False))
    for role, prefix in (("app", "PAPER_APP_DB"), ("worker", "PAPER_WORKER_DB"),
                         ("admin", "PAPER_ADMIN_DB"), ("audit_writer", "PAPER_AUDIT_DB")):
        role_env, secret_mounts = runtime.db_env(role, 10001, prefix)
        env.update(role_env)
        mounts += secret_mounts
    return ("paper", runtime.images["paper-scheduler"], 10001, runtime.network, mounts, env,
            "/usr/local/bin/paper-runner", ["--once"])


def expire_owned_outbox_claim(runtime):
    """Observe a committed worker claim, kill only that owned worker, wait its lease.

    No claim function, timestamp UPDATE, clock override or synthetic claim row.
    If the real claim cannot be observed before delivery this attempt fails.
    """
    require(runtime.owner.observer.json("SELECT to_json(count(*)) FROM pg_stat_activity "
        "WHERE backend_type='client backend' AND pid<>pg_backend_pid();", role="migration_owner") == 0,
        "HISTORY_CLAIM_FOREIGN_BACKEND")
    with notification_lock(runtime.owner) as blocker:
        _expire_claim_while_blocked(runtime, blocker)


def notification_lock(owner):
    """Real owned SQL contention after claim COMMIT, without changing a row.

    The normal notifier blocks on its actual notifications INSERT. This makes
    the claim/kill observation deterministic without a patched producer hook.
    Existing C RowLock owns child reap and fixture supervisor registration.
    """
    fixture = fixture_module()
    class NotificationLock(fixture.RowLock):
        def __enter__(self):
            self.observer.target()
            self.child = fixture.subprocess.Popen(owner.argv(["exec", "--interactive", "--user", "999:999",
                owner.pg, "psql", "-X", "-qAt", "-U", "lagrange", "-d", "lagrange", "-v", "ON_ERROR_STOP=1"]),
                stdin=fixture.subprocess.PIPE, stdout=fixture.subprocess.PIPE, stderr=fixture.subprocess.DEVNULL,
                env=fixture.ops._docker_client_environment())
            owner.blockers.append(self)
            try:
                self.child.stdin.write(b"BEGIN; SET LOCAL lock_timeout='5s'; SET LOCAL statement_timeout='15s';\n"
                    b"LOCK TABLE public.notifications IN ACCESS EXCLUSIVE MODE;\n"
                    b"SELECT json_build_object('pid',pg_backend_pid(),'start',backend_start::text) "
                    b"FROM pg_stat_activity WHERE pid=pg_backend_pid();\n\\echo HISTORY_NOTIFICATION_LOCKED\n")
                self.child.stdin.flush()
                output = b""
                until = time.monotonic() + owner.deadline.seconds(15)
                with selectors.DefaultSelector() as selector:
                    selector.register(self.child.stdout, selectors.EVENT_READ)
                    while b"HISTORY_NOTIFICATION_LOCKED\n" not in output:
                        require(self.child.poll() is None and time.monotonic() < until, "HISTORY_NOTIFICATION_LOCK_UNKNOWN")
                        if selector.select(min(0.1, max(0, until - time.monotonic()))):
                            output += os.read(self.child.stdout.fileno(), 4096)
                            require(len(output) < 8192, "HISTORY_NOTIFICATION_LOCK_UNKNOWN")
                self.backend = json.loads(output.splitlines()[0])
                require(set(self.backend) == {"pid", "start"} and type(self.backend["pid"]) is int
                    and self.backend["pid"] > 0 and type(self.backend["start"]) is str,
                    "HISTORY_NOTIFICATION_LOCK_UNKNOWN")
                count = self.observer.json("SELECT to_json(count(*)) FROM pg_locks l JOIN pg_stat_activity a USING(pid) "
                    "WHERE l.pid=" + str(self.backend["pid"]) + " AND a.backend_start::text="
                    + fixture.driver._paper_backup()._sql_literal(self.backend["start"])
                    + " AND l.granted AND l.relation='public.notifications'::regclass AND l.mode='AccessExclusiveLock';",
                    role="migration_owner")
                require(count == 1, "HISTORY_NOTIFICATION_LOCK_UNKNOWN")
                self.observer.backends.add((self.backend["pid"], self.backend["start"]))
                self.acquired = time.monotonic()
                return self
            except BaseException:
                self.release()
                raise
        def __exit__(self, *_exc):
            self.release()
            require(self.observer.json("SELECT to_json(count(*)) FROM pg_stat_activity WHERE pid="
                + str(self.backend["pid"]) + " AND backend_start::text="
                + fixture.driver._paper_backup()._sql_literal(self.backend["start"]) + ";",
                role="migration_owner") == 0, "HISTORY_NOTIFICATION_LOCK_UNREAPED")
    return NotificationLock(owner.observer)


def _expire_claim_while_blocked(runtime, blocker):
    before = runtime.owner.observer.json("SELECT coalesce(json_object_agg(id,claim_token),'{}'::json) "
        "FROM paper_settlement_outbox;", role="migration_owner")
    cid = runtime.create(*paper_command(runtime))
    runtime.command(["start", cid])
    image = runtime.inspect("container", cid)
    generation = (image["Created"], image["State"]["StartedAt"], image["RestartCount"], image["Image"])
    networks = list(image["NetworkSettings"]["Networks"].values())
    require(len(networks) == 1 and networks[0]["NetworkID"] == runtime.network, "HISTORY_CLAIM_NETWORK")
    address = str(ipaddress.ip_address(networks[0]["IPAddress"]))
    require(ipaddress.ip_address(address).version == 4, "HISTORY_CLAIM_NETWORK")
    owner_id = str(uuid.UUID(runtime.config["owner"]))
    until = time.monotonic() + runtime.remaining(30)
    claimed = None
    while time.monotonic() < until:
        claimed = runtime.owner.observer.json("SELECT coalesce((SELECT json_build_object('id',id,'token',claim_token) "
            "FROM paper_settlement_outbox WHERE owner_user_id='" + owner_id + "'::uuid "
            "AND claim_token IS NOT NULL AND claim_expires_at>clock_timestamp() AND delivered_at IS NULL "
            "ORDER BY created_at,id LIMIT 1),'null'::json);", role="migration_owner")
        if claimed is not None:
            require(before.get(claimed["id"]) != claimed["token"], "HISTORY_CLAIM_PREEXISTING")
            network = runtime.inspect("network", runtime.network)
            require(set(network.get("Containers", {})) == {runtime.owner.pg, cid}, "HISTORY_CLAIM_FOREIGN_WRITER")
            backends = runtime.owner.observer.json("SELECT coalesce(json_agg(json_build_object('pid',pid,"
                "'start',backend_start::text,'address',host(client_addr),'blocked'," + str(blocker.backend["pid"])
                + "=ANY(pg_blocking_pids(pid)))),'[]'::json) FROM pg_stat_activity "
                "WHERE backend_type='client backend' AND pid<>pg_backend_pid();", role="migration_owner")
            own = [row for row in backends if row["pid"] == blocker.backend["pid"] and row["start"] == blocker.backend["start"]]
            writers = [row for row in backends if row not in own]
            require(len(own) == 1 and writers and all(row["address"] == address and row["pid"] > 0 and row["start"]
                for row in writers) and any(row["blocked"] is True for row in writers), "HISTORY_CLAIM_FOREIGN_BACKEND")
            observed = runtime.inspect("container", cid)
            require((observed["Created"], observed["State"]["StartedAt"], observed["RestartCount"], observed["Image"])
                == generation and observed["State"]["Running"] is True, "HISTORY_CLAIM_WORKER_UNKNOWN")
            break
        require(runtime.inspect("container", cid)["State"]["Running"], "HISTORY_CLAIM_NOT_OBSERVED")
        time.sleep(min(0.01, runtime.remaining(0.01)))
    require(type(claimed) is dict, "HISTORY_CLAIM_NOT_OBSERVED")
    runtime.command(["kill", "--signal=KILL", cid])
    runtime.command(["wait", cid])
    require(runtime.inspect("container", cid)["State"]["Running"] is False, "HISTORY_CLAIM_WORKER_UNKNOWN")
    require(runtime.owner.observer.json("SELECT to_json(count(*)) FROM pg_stat_activity "
        "WHERE backend_type='client backend' AND pid<>pg_backend_pid() AND pid<>"
        + str(blocker.backend["pid"]) + ";", role="migration_owner") == 0,
        "HISTORY_CLAIM_BACKEND_UNREAPED")
    claim_id, token = str(uuid.UUID(claimed["id"])), str(uuid.UUID(claimed["token"]))
    while True:
        state = runtime.owner.observer.json("SELECT json_build_object('token',claim_token,'delivered',delivered_at,"
            "'expired',claim_expires_at<clock_timestamp()) FROM paper_settlement_outbox WHERE id='"
            + claim_id + "'::uuid;", role="migration_owner")
        require(type(state) is dict and state["token"] == token and state["delivered"] is None,
                "HISTORY_CLAIM_NOT_RETAINED")
        if state["expired"] is True:
            return
        time.sleep(min(0.2, runtime.remaining(0.2)))


def checkpoint(owner, facts):
    """Durable description of actual bootstrap originals, never a ready token."""
    fixture = fixture_module()
    owner.observer.target()
    require(owner.lock_fd is not None and fixture.faults.directory_identity(owner.root) == owner.root_id,
            "HISTORY_CHECKPOINT_OWNER")
    with fixture.ops.journal.open_journal(str(owner.root), owner.lock_fd) as journal:
        installation_id, floor = journal.installation_id, journal.epoch_floor
        journal.revalidate()
    value = {"schema": SCHEMA, "root_identity": list(owner.root_id), "bootstrap_commit": owner.commit,
        "manifest_sha256": digest(owner.manifest), "project": owner.project, "network": owner.network,
        "postgres": owner.pg, "postgres_image": owner.pg_image,
        "postgres_generation": list(owner.observer.generation), "account_id": owner.account,
        "release_hashes": owner.release_hashes, "producer_hashes": owner.producer_hashes, "facts": facts,
        "installation_id": installation_id, "bootstrap_floor": floor}
    path = owner.root / "genuine-bootstrap.json"
    fixture.write_new(path, canonical(value), 0o400)
    fd = os.open(owner.root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)
    return path


def release_descriptors(owner, checked):
    ops = fixture_module().ops
    paper = owner.release / "deploy/owner-beta-paper"
    capability = ops._json(ops.daily.install.read_root_file(str(paper / "capability.json"),
        gid=0, mode=0o444, maximum=ops.MAX_RESULT), ops.MAX_RESULT)
    release = {"capability_sha256": checked.capability_sha256,
        "install_receipt_sha256": checked.install_receipt_sha256,
        "producer_receipts_sha256": checked.producer_receipts_sha256,
        **{key: capability[key] for key in ("api_binary_sha256", "worker_binary_sha256", "native_binary_sha256")}}
    producers = {name: {leaf: ops._digest(ops.daily.install.read_root_file(
        str(paper / "provenance" / name / leaf), gid=0, mode=0o444, maximum=65536))
        for leaf in ("artifact.json", "COMPLETE")}
        for name in ("api-server", "paper-runner", "owner-beta-paper-ops-native")}
    checked.revalidate()
    return release, producers


def private_policy(owner, policy_root):
    """The existing fixture's namespace-local observer policy, never host policy."""
    fixture = fixture_module()
    policy = {"schema_version": "service-status.policy.v1", "profile": "owner_daily",
        "release_commit": owner.commit, "compose_project": owner.project, "owner_access": "owner_only",
        "quote_transport": "market_ws", "window_source": "operational_v1", "research_mode": "daily",
        "policy_installed_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "job_lease_seconds": {name: 60 for name in ("owner_equity_v2", "owner_beta_price_recommendation",
            "candidate_compute", "recommendation", "backtest")}}
    fixture.ops.daily.policy.validate_policy(policy)
    fixture.write_new(Path(policy_root) / "service-status/policy.json", canonical(policy))


def resume_owner(source, commit, manifest, deadline, checkpoint_path, *, policy_root):
    """Reopen exact root/DB/release originals, not a serialized Python owner.

    The private checkpoint locates resources; actual protected filesystem,
    release, network/container generation and native DB/journal state must all
    validate again. C still establishes complete source history independently.
    """
    fixture = fixture_module()
    ops = fixture.ops
    path = Path(checkpoint_path)
    require(path.is_absolute() and path.name == "genuine-bootstrap.json"
            and path.parent.parent == Path("/var/lib")
            and path.parent.name.startswith("lagrange-paper-operations-fixture-"), "HISTORY_CHECKPOINT_PATH")
    data = ops.daily.install.read_root_file(str(path), gid=0, mode=0o400, maximum=65536)
    value = ops.journal._json(data, 65536)
    require(type(value) is dict and set(value) == {"schema", "root_identity", "bootstrap_commit",
        "manifest_sha256", "project", "network", "postgres", "postgres_image", "postgres_generation",
        "account_id", "release_hashes", "producer_hashes", "facts", "installation_id", "bootstrap_floor"}
        and value["schema"] == SCHEMA, "HISTORY_CHECKPOINT")
    require(tuple(value["root_identity"]) == fixture.faults.directory_identity(path.parent), "HISTORY_ROOT_CHANGED")
    image_manifest(manifest, commit)

    class RetainedFixtureOwner(fixture.FixtureOwner):
        def __init__(self):
            # This is an explicit retained-resource constructor. Each resource
            # is reopened and checked below; no booleans certify preparation.
            self.source, self.commit, self.manifest, self.deadline = Path(source), commit, manifest, deadline
            self.root, self.root_id, self.project = path.parent, tuple(value["root_identity"]), value["project"]
            self.docker = fixture.FixtureDocker(deadline)
            self.pg_image, self.network, self.pg = value["postgres_image"], value["network"], value["postgres"]
            self.network_sent = self.pg_sent = True
            self.context = self.lock_fd = self.observer = None
            self.contexts, self.tracees, self.threads, self.blockers = [], [], [], []
            self.context_modules, self.helpers, self.helper_bindings = {}, {}, {}
            self.stop_writer, self.stop_writer_sent = None, False
            self.cancel_event = fixture.threading.Event()
            self.original_custody_uncertain = False
            self.cleanup = {"containers": "PENDING", "backends": "PENDING", "inputs": "RETAINED", "extractions": "NOT_STARTED"}
            self.extractions = fixture.P3ExtractionCustody(self)
            self.account = str(uuid.UUID(value["account_id"]))
            self.release_hashes, self.producer_hashes = value["release_hashes"], value["producer_hashes"]

    owner = RetainedFixtureOwner()
    private_policy(owner, policy_root)
    owner.lock_fd = os.open(owner.root / ops.LOCK_NAME, os.O_RDWR | os.O_CLOEXEC | os.O_NOFOLLOW)
    try:
        ops._file(owner.lock_fd, 0o600)
        fcntl.flock(owner.lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        with ops.journal.open_journal(str(owner.root), owner.lock_fd) as journal:
            require(journal.installation_id == value["installation_id"]
                and type(value["bootstrap_floor"]) is int and 0 <= value["bootstrap_floor"] <= journal.epoch_floor,
                "HISTORY_JOURNAL_CHANGED")
            journal.revalidate()
        bootstrap = owner.root / "releases" / value["bootstrap_commit"]
        require(digest(ops.daily.install.read_root_file(str(bootstrap / ops.release.MANIFEST_NAME),
            gid=0, mode=0o600, maximum=65536)) == value["manifest_sha256"], "HISTORY_BOOTSTRAP_CHANGED")
        owner.release = bootstrap
        with ops.release.verify(str(owner.root), str(bootstrap), value["bootstrap_commit"]) as old:
            require(release_descriptors(owner, old) == (value["release_hashes"], value["producer_hashes"]),
                "HISTORY_BOOTSTRAP_CHANGED")
            old.revalidate()
        owner.release = owner.root / "releases" / commit
        require(ops.daily.install.read_root_file(str(owner.release / ops.release.MANIFEST_NAME),
            gid=0, mode=0o600, maximum=65536) == manifest, "HISTORY_RELEASE_CHANGED")
        # The later release must already have been published by the ordinary
        # installer. This path cannot add process.json to either immutable tree.
        if commit != value["bootstrap_commit"]:
            ops.daily.policy.validate_current_release(str(owner.root), str(owner.release), commit)
        with ops.release.verify(str(owner.root), str(owner.release), commit) as current:
            current.revalidate()
            owner.release_hashes, owner.producer_hashes = release_descriptors(owner, current)
            owner.prepared_process_sha256 = current.process_sha256
        owner.bootstrap_commit = value["bootstrap_commit"]
        images = ops.daily.read_manifest(str(owner.release), commit)
        values, overlays = ops.daily.read_context(str(owner.release), commit, "disable")
        require(values["LAGRANGE_DATA_DIR"] == str(owner.root / "data")
                and values["LAGRANGE_ARTIFACTS_DIR"] == str(owner.root / "artifacts")
                and values["LAGRANGE_RUNTIME_SECRET_DIR"] == str(owner.root / "secrets"), "HISTORY_RELEASE_ROOTS")
        owner.context = ops.daily.Context(str(owner.release), commit, images, values, overlays)
        owner.observer = fixture.PgObserver(owner)
        owner.observer.generation = tuple(value["postgres_generation"])
        owner.observer.target()
        with ops.open_context(owner.context, owner.lock_fd) as live:
            owner.contexts.append(live)
            with live.probe_history() as inventory:
                require(inventory._configured is True, "HISTORY_CONFIGURED_REQUIRED")
        actual = owner.observer.json("SELECT to_json(account_id) FROM owner_beta_paper_control;", role="migration_owner")
        require(actual == owner.account, "HISTORY_ACCOUNT_CHANGED")
        return owner, value["facts"]
    except BaseException:
        os.close(owner.lock_fd)
        owner.lock_fd = None
        raise


def bootstrap(source, commit, manifest, deadline, *, approved_base, raw_root, config_source,
              generic_bundle, generic_session, policy_root):
    """OFF-reader/native CONFIGURE and genuine HTTP consent, then durable pause.

    Later immutable-release installation and genuine future Source acquisition
    are separate phases. This result is BOOTSTRAPPED, never archive acceptance.
    """
    fixture = fixture_module()
    owner = fixture.FixtureOwner(source, commit, manifest, deadline)
    runtime = None
    retained = False
    with owner.extractions:
        try:
            owner.setup(policy_root)  # Actual original OFF-reader release, process_binding=None.
            with HistoricalMaterial(source, approved_base, raw_root, deadline) as material:
                runtime = initialize_runtime(owner, material, config_source)
                seed_user_and_control(runtime)
                pin = collect_generic(runtime, generic_session, generic_bundle)
                runtime.start_api(pin)
                runtime.create_config()
                generic_strategy = runtime.strategy_id
                beta_strategy = create_beta_config(runtime)
                require(beta_strategy != generic_strategy, "HISTORY_BETA_CONFIG")
                contract = fixture.existing_json_macro(
                    owner.release / "tests/integration/migration-contract/tests/paper_0067.rs", "financial_contract")
                proposal = canonical({"schema": "owner-beta-paper-configure-operation-v1",
                    "strategy_config_id": beta_strategy, "account_name": "Owner-beta Paper", "contract": contract})
                result = lifecycle_action(owner, "configure", caller_key="genuine-bootstrap-configure", proposal=proposal)
                readback = fixture.ops.journal._json(result.readback)
                owner.account = str(uuid.UUID(readback["account_id"]))
                require(readback["current_state"] == "OFF" and readback["current_epoch"] == 0,
                        "HISTORY_BOOTSTRAP_EPOCH")
                consent_result = consent(runtime, 0, opt_in=True)
                # Independent DB view, not the HTTP body, binds the next release.
                control = owner.observer.json("SELECT to_jsonb(c) FROM owner_beta_paper_control c;", role="migration_owner")
                require(control["account_id"] == owner.account and control["control_epoch"] == 0,
                        "HISTORY_BOOTSTRAP_EPOCH")
                generic_account, generic_run = generic_preview(runtime, generic_strategy, generic_session)
                recommendation_once(runtime, pin)
                preview = create_generic_preview(runtime, generic_account["id"], generic_run["id"])
                runtime.once(*paper_command(runtime))
                verify_generic_preview(owner, preview["id"], pin)
                runtime.close()
                material.revalidate()
                facts = {"strategy_config_id": beta_strategy, "generic_strategy_config_id": generic_strategy,
                    "owner_user_id": runtime.config["owner"],
                    "configure_readback_sha256": digest(result.readback), "consent_http_sha256": digest(canonical(consent_result)),
                    "generic_dataset": pin, "generic_account_id": generic_account["id"],
                    "generic_run_id": generic_run["id"], "generic_preview_id": preview["id"],
                    "daily_config_sha256": digest(runtime.config_bytes),
                    "consent_event_id": control["consent_event_id"]}
                owner.extractions.reconcile()
                output = checkpoint(owner, facts)
            retained = True
            return output
        finally:
            if runtime is not None:
                runtime.close()
            if retained:
                os.close(owner.lock_fd)
                owner.lock_fd = None
            else:
                deadline.cleanup_phase()
                source_module().dispose_with_source_custody(owner)


def verify_generic_preview(owner, preview_id, pin):
    identifier = str(uuid.UUID(preview_id))
    row = owner.observer.json("SELECT json_build_object('dataset',dataset_version_id,'lineage',beta_lineage_id,"
        "'target',pending_target_id,'applied',applied_at,'status',status) FROM paper_rebalance_previews "
        "WHERE id='" + identifier + "'::uuid;", role="migration_owner")
    require(type(row) is dict and row["dataset"] == pin["id"] and row["lineage"] is None
            and row["target"] is None and row["applied"] is None and row["status"] == "READY",
            "HISTORY_GENERIC_PREVIEW")


def prepared_release(owner):
    require(owner.commit != owner.bootstrap_commit and owner.prepared_process_sha256 is not None,
            "HISTORY_LATER_PREPARED_RELEASE_REQUIRED")


def produce(owner, facts, phase, *, approved_base, raw_root, config_source):
    """Resume only real durable producer state; this never runs a Source scheduler.

    Real future sessions, Source ARM/Current originals and Daily activation must
    already have been acquired by the existing authorized release service. Each
    subphase stops at an actual committed producer result. No clock is changed.
    """
    require(phase in {"daily", "beta-preview", "paper-cycle", "simulation", "expire-claim"}, "HISTORY_PHASE")
    prepared_release(owner)
    runtime = None
    try:
        with HistoricalMaterial(owner.source, approved_base, raw_root, owner.deadline) as material:
            runtime = initialize_runtime(owner, material, config_source, existing=True)
            require(runtime.config["owner"] == facts["owner_user_id"], "HISTORY_OWNER_CHANGED")
            if phase == "daily":
                daily_cycle(runtime)
            elif phase == "paper-cycle":
                runtime.once(*paper_command(runtime))
            elif phase == "expire-claim":
                expire_owned_outbox_claim(runtime)
            else:
                seed_user_and_control(runtime, existing=True)  # Authentication only.
                pin = facts["generic_dataset"]
                verify_generic_preview(owner, facts["generic_preview_id"], pin)
                runtime.start_api(pin)
                if phase == "simulation":
                    submitted = simulation_pair(runtime, facts["strategy_config_id"])
                    simulation_once(runtime)
                    read_simulation_pair(runtime, submitted["run_id"])
                else:
                    control = owner.observer.json("SELECT to_jsonb(c) FROM owner_beta_paper_control c;",
                        role="migration_owner")
                    require(control["account_id"] == owner.account and control["state"] == "ENABLED",
                            "HISTORY_OFFICIAL_ENABLE_REQUIRED")
                    # Select an actual successful Daily-produced run of this
                    # configured strategy; generic recommendation IDs cannot enter.
                    run = owner.observer.json("SELECT to_json(id) FROM owner_beta_recommendation_runs "
                        "WHERE strategy_config_id='" + str(uuid.UUID(facts["strategy_config_id"]))
                        + "'::uuid AND status='SUCCEEDED' ORDER BY created_at DESC,id DESC LIMIT 1;",
                        role="migration_owner")
                    require(type(run) is str, "HISTORY_DAILY_RUN_REQUIRED")
                    preview = beta_preview(runtime, run, control["control_epoch"])
                    runtime.once(*paper_command(runtime))
                    apply_beta_preview(runtime, preview["preview_id"], control["control_epoch"])
            runtime.close()
            material.revalidate()
    finally:
        if runtime is not None:
            runtime.close()
    return {"phase": phase, "status": "PRODUCER_PHASE_COMPLETE", "archive_acceptance": False}


def retained_phase(owner, facts, args, policy_root, *, source_custody=None):
    action = args.phase
    if action == "source-service":
        prepared_release(owner)
        return source_module().run_source_service(owner, args.binding_file, policy_root, args.action)
    if action in {"observe-reset", "enable", "recover"}:
        prepared_release(owner)
    if action in {"observe-reset", "enable", "recover", "readback"}:
        proposal = None
        if action == "recover":
            proposal = fixture_module().ops.daily.install.read_root_file(args.proposal_file,
                gid=0, mode=0o400, maximum=65536)
        result = lifecycle_action(owner, action.replace("-", "_"), caller_key=args.caller_key,
            proposal=proposal, generation=args.generation, sequence=args.sequence)
        return {"phase": action, "status": "LIFECYCLE_PHASE_COMPLETE", "outcome": result.outcome,
            "readback_sha256": digest(result.readback) if result.readback is not None else None,
            "artifact_sha256": digest(result.artifact) if result.artifact is not None else None,
            "archive_acceptance": False}
    if action == "produce":
        return produce(owner, facts, args.producer, approved_base=args.approved_base,
            raw_root=args.raw_root, config_source=args.daily_config)
    require(action == "accept", "HISTORY_PHASE")
    require(source_custody is not None and source_custody.owner is owner, "HISTORY_SOURCE_CUSTODY")
    source_custody.check()
    prepared_release(owner)
    verify_generic_preview(owner, facts["generic_preview_id"], facts["generic_dataset"])
    selection = load("paper_genuine_history_selection", "scripts/ops/lib/owner-beta-paper-backup-selection.py")
    values = selection._backup_module().config_file(args.storage_config, owner.context, owner.project)
    # Storage belongs to this disposable owner too. Never direct private fault
    # mutation or disposal at an unrelated historical/production backup root.
    root = str(owner.root) + "/"
    require(all(values[name].startswith(root) for name in
        ("BACKUP_ROOT", "KEY_FILE", "LOCK_FILE", "METRICS_FILE")), "HISTORY_STORAGE_OWNER")
    # C retains its actual DrainedHandle/Source publication locks internally;
    # the outer guard preserves the stopped unit originals for its discovery.
    results = archive_acceptance(owner, values, policy_root=policy_root)
    source_custody.check()
    return {"phase": action, "status": "PASS", "cases": results, "archive_acceptance": True}


def run(args):
    """Private host entry. Only bounded observed acceptance returns PASS.

    Bootstrap leaves the owned PostgreSQL/network and native originals for the
    later ordinary prepared-release installer and time-spanning Source service.
    Other preparatory phases also retain them. Acceptance disposes them through
    C's real supervisor before returning success. Failure never emits acceptance.
    """
    fixture = fixture_module()
    require(os.geteuid() == 0 and os.getegid() == 0 and sys.platform == "linux", "HISTORY_PRIVATE_ROOT_REQUIRED")
    # Reuse exact source/official 13-image original validation; no env defaults.
    phase_deadline(args.deadline_monotonic_ns)
    source, manifest, deadline = fixture.validated_inputs(args.source_root, args.commit,
        args.manifest_file, args.deadline_monotonic_ns)
    policy_root = Path(tempfile.mkdtemp(prefix="lagrange-paper-genuine-policy-", dir="/var/tmp"))
    (policy_root / "service-status").mkdir(mode=0o700)
    owner = None
    result = None
    policy_scope = None
    try:
        policy_scope = fixture.ServiceStatusScope(policy_root)
        with fixture.bounded_run(deadline), policy_scope:
            if args.phase == "bootstrap":
                path = bootstrap(source, args.commit, manifest, deadline,
                    approved_base=args.approved_base, raw_root=args.raw_root, config_source=args.daily_config,
                    generic_bundle=args.generic_bundle, generic_session=args.generic_session, policy_root=policy_root)
                # A private local path, never a hosted receipt or uploaded body.
                result = {"phase": "bootstrap", "status": "BOOTSTRAPPED_OFF", "checkpoint": str(path),
                    "archive_acceptance": False}
            else:
                owner, facts = resume_owner(source, args.commit, manifest, deadline, args.checkpoint,
                    policy_root=policy_root)
                with owner.extractions:
                    if args.phase == "accept":
                        with source_module().require_source_custody_for_acceptance(owner) as custody:
                            try:
                                result = retained_phase(owner, facts, args, policy_root, source_custody=custody)
                            finally:
                                deadline.cleanup_phase()
                                source_module().dispose_with_source_custody(owner, custody)
                    else:
                        result = retained_phase(owner, facts, args, policy_root)
                        owner.extractions.reconcile()
                if args.phase == "accept":
                    require(owner.cleanup.get("containers") == "REMOVED", "HISTORY_DISPOSAL_UNPROVEN")
                    result["cleanup"] = dict(owner.cleanup)
                else:
                    owner.observer.target()
            require(result is not None, "HISTORY_PHASE")
        # Namespace restoration is part of successful cleanup as well.
        return {**result, "commit": args.commit, "synthetic_fixture": True,
            "owner_acceptance": False, "production": "CLOSED", "enable": False}
    finally:
        if owner is not None and owner.lock_fd is not None:
            os.close(owner.lock_fd)
            owner.lock_fd = None
        # A failed unmount, setns or identity recheck retains the actual source
        # and descriptor owner. An unresolved scope is never successful cleanup.
        if policy_scope is None or policy_scope not in fixture._UNRESOLVED_POLICY_SCOPES:
            shutil.rmtree(policy_root)
