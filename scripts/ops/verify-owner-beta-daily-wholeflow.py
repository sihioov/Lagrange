#!/usr/bin/env python3
"""Host-local integration using approved V3 bytes and exact production images.

Explicit execution only. Never builds/pulls images or contacts a provider.
Creates a private disposable DB, two synthetic sessions and a synthetic Owner
session. The latter checks real HTTP authorization/projection, NOT Auth0 login,
browser TLS, actual Owner acceptance, market receipt or three trading days.
Raw output, DB contents and session credentials never enter the result receipt.
"""
from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import http.client
import importlib.util
import ipaddress
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import uuid

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[2]
ENV = {"PATH": "/usr/sbin:/usr/bin:/sbin:/bin", "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8"}
POSTGRES = "postgres@sha256:3a82e1f56c8f0f5616a11103ac3d47e632c3938698946a7ad26da0df1334744a"
LABEL = "io.lagrange.daily-wholeflow"
ROLES = {"migration_owner": "migration_owner", "app": "app", "worker": "worker",
         "audit_writer": "audit", "research_writer": "research", "admin": "admin",
         "daily_input_admitter": "daily_input_admitter"}
DB_CLIENTS = {
    "bootstrap": ("db-role-bootstrap", "/usr/local/bin/lagrange-bootstrap-roles"),
    "migrate": ("db-migrate", "/usr/local/bin/lagrange-migrate"),
    "control": ("db-migrate", "/usr/local/bin/lagrange-daily-control"),
}
# Exact finite diagnostics emitted by owner-beta-runner/daily.rs. These are
# evidence only: a reported outcome never substitutes for exit/state guards.
DAILY_DIAGNOSTICS = {
    ("owner_beta_daily_startup", "code"): (
        "ARGUMENTS_INVALID", "MODE_INVALID", "IDENTITY_INVALID", "CREDENTIAL_BOUNDARY",
        "ENVIRONMENT_INVALID", "CONFIGURATION_INVALID", "HEALTH_CONFIGURATION_INVALID",
        "DATABASE_CONFIG_INVALID", "DATABASE_UNAVAILABLE", "HEALTH_WRITER_UNAVAILABLE"),
    ("owner_beta_daily_cycle", "code"): (
        "CONFIGURATION", "STATE", "INTERRUPTED", "INPUT_PENDING", "INPUT_BLOCKED",
        "INVALID_ARTIFACT", "CONFLICT", "DATABASE", "COMMIT_UNKNOWN"),
    ("owner_beta_daily_cycle", "outcome"): (
        "DISABLED", "BUSY", "NOT_DUE", "PENDING", "CANDIDATE_READY", "ADMITTED", "REPLAY", "BLOCKED"),
}


def unique_object(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise ValueError("duplicate diagnostic key")
        value[key] = item
    return value


def command_failure(service, result):
    """Discard all raw bytes after bounded, exact daily-event selection.

    The exit code belongs to the attached Docker command, not an independent
    container-state observation. Unknown output remains unknown, never echoed.
    DB clients and worker output receive hashes only; their output contracts
    are not the daily producer/admitter diagnostic contract.
    """
    streams = {"stdout": result.stdout or b"", "stderr": result.stderr or b""}
    evidence = {"service": service, "docker_exit_code": result.returncode,
                "streams": {name: {"bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}
                            for name, data in streams.items()},
                "diagnostic_status": "UNKNOWN", "diagnostics": []}
    if service not in ("producer", "admitter") or sum(map(len, streams.values())) > 65536:
        return evidence
    for name, data in streams.items():
        for line in data.splitlines():
            if len(line) > 512:
                continue
            try:
                value = json.loads(line, object_pairs_hook=unique_object)
            except (ValueError, UnicodeError, RecursionError):
                continue
            if not isinstance(value, dict):
                continue
            for (event, field), labels in DAILY_DIAGNOSTICS.items():
                if (set(value) == {"event", field} and value["event"] == event
                        and isinstance(value[field], str) and value[field] in labels):
                    evidence["diagnostics"].append({"stream": name, "event": event, field: value[field]})
            if len(evidence["diagnostics"]) > 16:
                evidence["diagnostics"] = []
                return evidence
    if evidence["diagnostics"]:
        evidence["diagnostic_status"] = "OBSERVED_ALLOWLISTED"
    return evidence


def load(name, relative):
    spec = importlib.util.spec_from_file_location(name, ROOT / relative)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


fixture = load("wholeflow_fixture", "scripts/ops/lib/owner-beta-daily-wholeflow-fixture.py")
files = load("wholeflow_files", "scripts/ops/lib/owner-beta-daily-backup-files.py")
release = load("wholeflow_release", "scripts/ops/lib/owner-beta-daily-release.py")
base_views = load("wholeflow_base_views", "scripts/ops/lib/owner-beta-daily-base.py")
lifecycle = load("wholeflow_lifecycle", "scripts/ops/lib/owner-beta-daily-lifecycle.py")
require = fixture.require


def source_git(*args):
    # This root driver is explicitly launched from the reviewed checkout,
    # which can belong to its non-root operator. Trust only that exact path
    # for this command, as the official image builder does; never alter Git
    # configuration or inherit a wildcard safe.directory from the caller.
    try:
        result = subprocess.run(
            ["git", "-c", "safe.directory=" + str(ROOT), "-C", str(ROOT), *args],
            env=ENV, capture_output=True, timeout=10, check=False)
    except (OSError, subprocess.TimeoutExpired):
        raise fixture.FixtureError("WHOLEFLOW_SOURCE_GIT_FAILED") from None
    require(result.returncode == 0, "WHOLEFLOW_SOURCE_GIT_FAILED")
    return result.stdout


def verify_source(candidate):
    head = source_git("rev-parse", "HEAD").decode("ascii").strip()
    require(head == candidate, "WHOLEFLOW_SOURCE_REVISION")
    for relative in ("configs/evidence/owner-beta-daily-price-policy-v1.json",
                     "deploy/db/owner-beta-daily-control.sql", "scripts/ops/verify-owner-beta-daily-wholeflow.py",
                     "scripts/ops/lib/owner-beta-daily-wholeflow-fixture.py",
                     "scripts/ops/lib/owner-beta-daily-release.py", "scripts/ops/lib/owner-beta-daily-backup-files.py",
                     "scripts/ops/lib/owner-beta-daily-base.py",
                     "scripts/ops/lib/owner-beta-daily-lifecycle.py"):
        pinned = source_git("show", head + ":" + relative)
        require(pinned == (ROOT / relative).read_bytes(), "WHOLEFLOW_SOURCE_CHANGED")


def private_file(path, data, uid=0, gid=0, mode=0o600):
    with open(path, "xb") as stream:
        os.fchown(stream.fileno(), uid, gid)
        os.fchmod(stream.fileno(), mode)
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())


def directory(path, uid=0, gid=0, mode=0o700):
    path.mkdir(mode=0o700)
    os.chown(path, uid, gid)
    path.chmod(mode)
    return path


def fixed_outcome(data, event, expected):
    values = []
    for line in data.splitlines():
        try:
            value = json.loads(line)
        except (ValueError, UnicodeError):
            continue
        if isinstance(value, dict) and value.get("event") == event:
            values.append(value)
    require(len(values) == 1 and values[0].get("outcome") == expected,
            "WHOLEFLOW_EXECUTABLE_OUTCOME")


def expired(_signum, _frame):
    raise fixture.FixtureError("WHOLEFLOW_DEADLINE")


class WholeFlow:
    def __init__(self, args):
        self.args = args
        self.end = time.monotonic() + args.timeout_seconds
        self.nonce = "lagrange-wholeflow-" + uuid.uuid4().hex
        self.root = None
        self.intents = []
        self.uncertain_creations = set()
        self.sequence = 0
        self.events = None
        self.event_objects = []
        self.receipt = {"schema": "lagrange-daily-wholeflow-result-v1", "candidate": args.candidate,
                        "synthetic_extension": True, "actual_owner_login": False,
                        "market_receipt": False, "three_trading_days": False, "phases": []}

    def phase(self, name):
        self.receipt["phases"].append(name)
        print("WHOLEFLOW: " + name, flush=True)

    def remaining(self, limit=60):
        remaining = self.end - time.monotonic()
        require(remaining > 0, "WHOLEFLOW_DEADLINE")
        return min(limit, remaining)

    def command(self, args, payload=None, *, limit=60, output=None, cleanup=False, include_stderr=False,
                failure_service=None):
        if self.events is not None and not cleanup:
            self.events.check()
        require(failure_service is None or (failure_service in (*DB_CLIENTS, "producer", "admitter", "worker")
                and len(args) == 4 and args[:3] == ["start", "--attach", "--interactive"]
                and output is None and not cleanup), "WHOLEFLOW_DIAGNOSTIC_SCOPE")
        try:
            result = subprocess.run(["/usr/bin/docker", "--host", "unix:///var/run/docker.sock", *args],
                env=ENV, input=payload, stdout=output or subprocess.PIPE, stderr=subprocess.PIPE,
                timeout=30 if cleanup else self.remaining(limit), check=False)
        except (OSError, subprocess.TimeoutExpired):
            raise fixture.FixtureError("WHOLEFLOW_DOCKER_UNCERTAIN") from None
        if result.returncode != 0 and failure_service is not None:
            self.receipt.setdefault("command_failures", []).append(command_failure(failure_service, result))
        require(result.returncode == 0, "WHOLEFLOW_DOCKER_FAILED")
        return (result.stdout or b"") + (result.stderr if include_stderr else b"")

    def inspect(self, kind, identity, *, cleanup=False):
        value = json.loads(self.command([kind, "inspect", identity], cleanup=cleanup))
        require(isinstance(value, list) and len(value) == 1, "WHOLEFLOW_INSPECT")
        return value[0]

    def intent(self, kind, name):
        # Record the nonce and exact name BEFORE the mutating daemon request.
        self.intents.append({"kind": kind, "name": name})
        private_file(self.root / f"intent-{len(self.intents):03}.json",
                     fixture.canonical({"nonce": self.nonce, **self.intents[-1]}))

    def create_network(self, suffix):
        name = self.nonce + "-" + suffix
        self.intent("network", name)
        self.uncertain_creations.add(("network", name))
        self.command(["network", "create", "--internal", "--label", LABEL + "=" + self.nonce, name])
        value = self.inspect("network", name)
        require(value["Internal"] is True and value["Labels"].get(LABEL) == self.nonce,
                "WHOLEFLOW_NETWORK")
        self.uncertain_creations.discard(("network", name))
        if self.events is not None:
            self.event_objects.append({"kind": "network", "id": value["Id"], "name": name})
            self.events.wait_for("network", value["Id"], "create")
        return value["Id"]

    @staticmethod
    def mount(source, target, readonly=True):
        require("," not in str(source) and "," not in target, "WHOLEFLOW_MOUNT")
        return "type=bind,src=" + str(source) + ",dst=" + target + (",readonly" if readonly else "")

    def create(self, service, image, uid, network, mounts, env, entry, args=(), *, alias=None):
        self.sequence += 1
        name = f"{self.nonce}-{service}-{self.sequence}"
        tmpfs = {"/tmp": "rw,nosuid,nodev,noexec,size=64m,mode=1777",
                 "/run/owner-beta-health": f"uid={uid},gid={uid},mode=0750,size=1m"}
        if service == "postgres":
            tmpfs["/var/run/postgresql"] = "uid=999,gid=999,mode=3775,size=4m"
        if service in DB_CLIENTS:
            image_service, expected_entry = DB_CLIENTS[service]
            require(uid == 999 and image == self.images[image_service] and entry == expected_entry,
                    "WHOLEFLOW_DB_CLIENT_BOUNDARY")
            # The fixed DB client image inherits PostgreSQL's VOLUME. These
            # one-shot clients never own a database directory: explicitly cover
            # the declaration with bounded read-only memory, not an anonymous
            # persistent volume. Keep the exact bind/no-volume guard below.
            tmpfs["/var/lib/postgresql"] = "ro,nosuid,nodev,noexec,size=1m,uid=999,gid=999,mode=0700"
        expected = {}
        for item in mounts:
            parts = dict(piece.split("=", 1) for piece in item.split(",") if "=" in piece)
            require(parts.get("type") == "bind" and parts["dst"] not in expected
                    and parts["dst"] not in tmpfs, "WHOLEFLOW_MOUNT_TARGETS")
            expected[parts["dst"]] = (parts["src"], not item.endswith(",readonly"))
        declared = self.inspect("image", image)["Config"].get("Volumes")
        declared = {} if declared is None else declared
        require(isinstance(declared, dict) and all(isinstance(k, str) and v == {} for k, v in declared.items())
                and set(declared) <= set(expected) | set(tmpfs), "WHOLEFLOW_IMAGE_VOLUMES")
        command = ["create", "--name", name, "--label", LABEL + "=" + self.nonce,
                   "--label", "io.lagrange.daily-wholeflow-service=" + service,
                   "--network", network, "--user", f"{uid}:{uid}", "--read-only",
                   "--cap-drop", "ALL", "--security-opt", "no-new-privileges:true",
                   "--pids-limit", "256", "--memory", "1g", "--cpus", "1",
                   "--log-driver", "none", "--interactive", "--pull", "never",
                   "--entrypoint", entry]
        for target, options in tmpfs.items():
            command += ["--tmpfs", target + ":" + options]
        if uid in (10001, 10002, 10003):
            command += ["--group-add", "10001"]
        if alias:
            command += ["--network-alias", alias]
        for mount in mounts:
            command += ["--mount", mount]
        for key, value in sorted(env.items()):
            command += ["--env", key + "=" + str(value)]
        command += [image, *args]
        self.intent("container", name)
        self.uncertain_creations.add(("container", name))
        self.command(command)
        value = self.inspect("container", name)
        host = value["HostConfig"]
        require(value["Image"] == image and value["Config"]["User"] == f"{uid}:{uid}"
                and value["Config"]["Labels"].get(LABEL) == self.nonce
                and host["ReadonlyRootfs"] is True and host["Privileged"] is False
                and host["NetworkMode"] == network and host["CapDrop"] == ["ALL"]
                and not host.get("PortBindings") and host["LogConfig"]["Type"] == "none",
                "WHOLEFLOW_CONTAINER_BOUNDARY")
        observed = {m["Destination"]: (m["Source"], m["RW"]) for m in value["Mounts"] if m["Type"] == "bind"}
        require(observed == expected and host.get("Tmpfs") == tmpfs
                and all(m["Type"] in ("bind", "tmpfs") for m in value["Mounts"]),
                "WHOLEFLOW_MOUNTS")
        self.uncertain_creations.discard(("container", name))
        if self.events is not None:
            self.event_objects.append({"kind": "container", "id": value["Id"], "name": name,
                                       "role": service, "image": image})
            # The daemon's create event is durable before any start command.
            self.events.wait_for("container", value["Id"], "create")
        return value["Id"]

    def once(self, *args, payload=None, include_stderr=False, **kwargs):
        container = self.create(*args, **kwargs)
        output = self.command(["start", "--attach", "--interactive", container], payload, limit=360,
                              include_stderr=include_stderr, failure_service=args[0])
        value = self.inspect("container", container)
        require(value["State"]["Running"] is False and value["State"]["ExitCode"] == 0
                and not value["State"]["OOMKilled"], "WHOLEFLOW_PROCESS_FAILED")
        return output

    def stop(self, container):
        self.command(["stop", "--time", "40", container], limit=50)
        value = self.inspect("container", container)
        require(not value["State"]["Running"] and value["State"]["ExitCode"] == 0
                and not value["State"]["OOMKilled"], "WHOLEFLOW_STOP")

    def pause_poll(self):
        # Real elapsed time; never alter checkpoints or SQL next_poll_at.
        require(self.remaining(70) >= 62, "WHOLEFLOW_DEADLINE")
        time.sleep(62)
        require(fixture.eligible(dt.datetime.now(dt.timezone.utc)).isoformat() == self.plan["sessions"][-1],
                "WHOLEFLOW_CUTOFF_CHANGED")

    def prepare(self):
        require(sys.platform == "linux" and os.geteuid() == 0, "WHOLEFLOW_ROOT_REQUIRED")
        require(600 <= self.args.timeout_seconds <= 1800, "WHOLEFLOW_DEADLINE_INPUT")
        manifest = release.read_root_file(str(self.args.image_manifest), maximum=16384)
        self.images = fixture.image_manifest(manifest, self.args.candidate)
        self.receipt["image_manifest_sha256"] = fixture.digest(manifest)
        for image in set(self.images.values()):
            info = self.inspect("image", image)
            require(info["Id"] == image and (info["Config"].get("Labels") or {}).get(
                "org.opencontainers.image.revision") == self.args.candidate, "WHOLEFLOW_IMAGE_REVISION")
        pg = self.inspect("image", POSTGRES)
        require(POSTGRES in pg["RepoDigests"], "WHOLEFLOW_POSTGRES_IMAGE")
        self.pg_image = pg["Id"]
        # A reviewed checkout is pinned before loading its SQL/policy fixtures;
        # source modifications cannot silently ride on a prior image candidate.
        verify_source(self.args.candidate)
        self.policy = (ROOT / "configs/evidence/owner-beta-daily-price-policy-v1.json").read_bytes()
        release.validate_policy(self.policy)
        self.plan = fixture.plan(dt.datetime.now(dt.timezone.utc), str(uuid.uuid4()), self.args.candidate)
        self.config = self.plan["config"]
        self.config_bytes = fixture.canonical(self.config)
        release.validate_config(self.config_bytes, self.args.candidate)
        self.root = Path(tempfile.mkdtemp(prefix=self.nonce + "-", dir="/var/lib"))
        self.root.chmod(0o700)
        self.current = directory(self.root / "initial")
        self.base = Path(self.args.approved_base)
        parent = self.base.parent.stat(follow_symlinks=False)
        self.base_root = files.Root("base", str(self.base), 10001, 10001, 0o750,
                                   parent=(parent.st_uid, 10001, 0o750) if parent.st_uid != 0 else None)
        self.original_base_root = self.base_root
        self.raw = directory(self.current / "raw", 10001, 10001, 0o750)
        for name, data in sorted(self.plan["raw_files"].items()):
            path = self.raw / name
            cursor = self.raw
            for part in path.relative_to(self.raw).parts[:-1]:
                cursor /= part
                if not cursor.exists():
                    directory(cursor, 10001, 10001, 0o750)
            private_file(path, data, 10001, 10001, 0o640)
        self.config_root = directory(self.current / "config", 0, 10001, 0o750)
        private_file(self.config_root / "config.json", self.config_bytes, 0, 10001, 0o440)
        private_file(self.config_root / "policy.json", self.policy, 0, 10001, 0o440)
        release.provision_state(str(self.current), self.config)
        self.daily = self.current / "owner-beta-daily"
        self.prepare_base_views()
        self.curated = directory(self.current / "curated", 10001, 10001, 0o750)
        self.floor = directory(self.current / "floor")
        private_file(self.floor / ".owner-beta-daily-reader-floor", release.FLOOR_BYTES, 0, 0, 0o440)
        self.limits = files.Limits(self.args.max_bytes)
        self.original = self.inventory()
        self.cookie, self.csrf = secrets.token_urlsafe(32), secrets.token_hex(32)
        self.secret_root = directory(self.root / "secrets")
        self.secret_values = {role: secrets.token_hex(32).encode() for role in ("postgres", *ROLES)}
        self.secret_paths = {}
        for role in self.secret_values:
            for uid in (999, 10001, 10003):
                path = self.secret_root / f"{role}-{uid}"
                private_file(path, self.secret_values[role], uid, uid, 0o400)
                self.secret_paths[(role, uid)] = path
        for name, value in (("cursor", secrets.token_hex(32).encode()), ("auth0", secrets.token_hex(32).encode())):
            path = self.secret_root / name
            private_file(path, value, 10001, 10001, 0o400)
        self.receipt["sessions"] = self.plan["sessions"]
        self.receipt["images"] = self.images
        self.phase("PREPARED_SYNTHETIC_EXTENSION")

    def prepare_base_views(self):
        try:
            base_views.provision(self.base, self.daily)
        except Exception:
            raise fixture.FixtureError("WHOLEFLOW_BASE_COPY_REJECTED") from None

    def verify_base_views(self):
        try:
            base_views.verify(self.daily)
        except Exception:
            raise fixture.FixtureError("WHOLEFLOW_BASE_COPY_VERIFY_REJECTED") from None

    def roots(self):
        return (files.Root("raw", str(self.raw), 10001, 10001, 0o750),
                files.Root("curated", str(self.curated), 10001, 10001, 0o750),
                files.Root("daily", str(self.daily), 0, 10001, 0o750),
                files.Root("config", str(self.config_root), 0, 10001, 0o750),
                files.Root("floor", str(self.floor), 0, 0, 0o700), self.base_root)

    def inventory(self):
        with files.Snapshot(self.roots(), self.limits) as snapshot:
            return snapshot.records

    def db_env(self, role, uid, prefix="DB"):
        require(role in ROLES, "WHOLEFLOW_DB_ROLE")
        name = "db_" + ROLES[role] + "_password"
        return {prefix + "_HOST": "postgres", prefix + "_PORT": "5432", prefix + "_NAME": "lagrange",
                prefix + "_USER": role, prefix + "_PASSWORD_FILE": "/run/secrets/" + name}, [
                    self.mount(self.secret_paths[(role, uid)], "/run/secrets/" + name)]

    def database(self, suffix, migrate):
        self.network = self.create_network(suffix)
        volume = directory(self.root / ("postgres-" + suffix), 999, 999, 0o1777)
        self.server = self.create("postgres", self.pg_image, 999, self.network, [
            self.mount(volume, "/var/lib/postgresql", False),
            self.mount(self.secret_paths[("postgres", 999)], "/run/secrets/postgres_password")],
            {"POSTGRES_DB": "lagrange", "POSTGRES_PASSWORD_FILE": "/run/secrets/postgres_password"},
            "/usr/local/bin/docker-entrypoint.sh", ["postgres"], alias="postgres")
        self.command(["start", self.server])
        # The image entrypoint can briefly run a socket-only bootstrap server.
        # Require authenticated TCP identity of the FINAL server, not pg_isready.
        until = time.monotonic() + self.remaining(90)
        while True:
            try:
                identity = self.sql("SELECT current_database()||':'||current_user||':'||session_user||':'||"
                                    "(inet_server_addr() IS NOT NULL)::text;")
                if identity.strip() == b"lagrange:postgres:postgres:true":
                    break
            except fixture.FixtureError:
                pass
            require(time.monotonic() < until, "WHOLEFLOW_DATABASE_START")
            time.sleep(0.5)
        env = {"DB_HOST": "postgres", "DB_PORT": "5432", "DB_NAME": "lagrange", "DB_ADMIN_USER": "postgres",
               "DB_ADMIN_PASSWORD_FILE": "/run/secrets/postgres_password"}
        mounts = [self.mount(self.secret_paths[("postgres", 999)], "/run/secrets/postgres_password")]
        keys = {"migration_owner": "MIGRATION_OWNER", "app": "APP", "worker": "WORKER", "audit_writer": "AUDIT",
                "research_writer": "RESEARCH", "admin": "ADMIN_ROLE", "daily_input_admitter": "DAILY_INPUT_ADMITTER"}
        for role, key in keys.items():
            target = "/run/secrets/db_" + ROLES[role] + "_password"
            env["DB_" + key + "_PASSWORD_FILE"] = target
            mounts.append(self.mount(self.secret_paths[(role, 999)], target))
        self.once("bootstrap", self.images["db-role-bootstrap"], 999, self.network, mounts, env,
                  "/usr/local/bin/lagrange-bootstrap-roles")
        if migrate:
            env, mounts = self.db_env("migration_owner", 999)
            self.once("migrate", self.images["db-migrate"], 999, self.network, mounts, env,
                      "/usr/local/bin/lagrange-migrate")

    def sql(self, statement):
        # Only the unique fixture server can be addressed. Inputs are fixed SQL
        # plus generated bounded scalar literals; stderr never leaves memory.
        return self.command(["exec", "--interactive", "--user", "999:999", self.server,
            "sh", "-c", 'set -eu; PGPASSWORD="$(cat /run/secrets/postgres_password)"; '
            'export PGPASSWORD PGCONNECT_TIMEOUT=3; exec psql -X -w -qAt -h 127.0.0.1 '
            '-U postgres -d lagrange -v ON_ERROR_STOP=1 -v VERBOSITY=sqlstate'],
            statement.encode(), limit=30)

    def schema_audit(self):
        rows = json.loads(self.sql("SELECT json_agg(json_build_object('version',version,'success',success,"
            "'checksum',encode(checksum,'hex')) ORDER BY version) FROM public._sqlx_migrations;"))
        migrations = sorted((ROOT / "migrations").glob("*.up.sql"))
        expected = [{"version": int(path.name[:4]), "success": True,
                     "checksum": hashlib.sha384(path.read_bytes()).hexdigest()} for path in migrations]
        require(len(expected) == 62 and rows == expected, "WHOLEFLOW_MIGRATIONS")

    def seed(self):
        owner = self.config["owner"]
        instruments = ",".join("('" + symbol + ".KRX','" + symbol + "','KRX','KRW','SYNTHETIC_FIXTURE','ETF','ACTIVE')"
                               for symbol in fixture.SYMBOLS)
        self.sql("BEGIN; INSERT INTO instruments(id,symbol,venue,currency,name,asset_class,status) VALUES " + instruments + ";"
            "INSERT INTO users(id,issuer,subject,email) VALUES('" + owner +
            "','wholeflow.invalid','fixture','fixture@wholeflow.invalid');"
            "INSERT INTO user_roles(user_id,role_id,granted_by) VALUES('" + owner + "','owner','" + owner + "');"
            "INSERT INTO web_sessions(id,user_id,session_hash,csrf_hash,expires_at) VALUES(gen_random_uuid(),'" +
            owner + "','" + hashlib.sha256(self.cookie.encode()).hexdigest() + "','" +
            hashlib.sha256(self.csrf.encode()).hexdigest() + "',now()+interval '1 hour');"
            "INSERT INTO data_entitlements(contract_reference,contract_document_sha256,status,covered_datasets,"
            "covered_uses,effective_from,effective_until,managed_by) VALUES('" + fixture.ENTITLEMENT + "','" +
            self.config["entitlement_sha256"][7:] + "','ACTIVE','[\"krx_eod_bars\"]','[\"recommendation\"]',"
            "'2016-08-29','2099-12-31','" + owner + "'); COMMIT;")
        # Use the actual migration-role control transaction. Do not insert
        # admissions, poll receipts, jobs, reservations or recommendation rows.
        template = (ROOT / "deploy/db/owner-beta-daily-control.sql").read_text()
        for action, expected in (("prepare", b"DAILY_CONTROL: DISABLED"), ("enable", b"DAILY_CONTROL: ENABLED")):
            replacements = {"__ACTION__": action, "__CONFIG_HEX__": self.config_bytes.hex(),
                            "__POLICY_HEX__": self.policy.hex(), "__COMMIT__": self.args.candidate,
                            "__CAPACITY__": "10", "__LEASE__": "60", "__BACKOFF__": "30"}
            statement = template
            for key, value in replacements.items():
                statement = statement.replace(key, value)
            env, mounts = self.db_env("migration_owner", 999)
            output = self.once("control", self.images["db-migrate"], 999, self.network, mounts, env,
                               "/usr/local/bin/lagrange-daily-control", payload=statement.encode())
            require(output.strip() == expected, "WHOLEFLOW_CONTROL")

    def daily_once(self, stage, expected):
        require(stage in ("producer", "admitter", "worker"), "WHOLEFLOW_STAGE")
        uid = {"producer": 10002, "admitter": 10003, "worker": 10001}[stage]
        base = self.base if stage == "worker" else self.daily / base_views.DIRECTORY / stage
        mounts = [self.mount(self.config_root / "config.json", "/run/owner-beta-daily/config.json"),
                  self.mount(self.config_root / "policy.json", "/run/owner-beta-daily/policy.json"),
                  self.mount(base, "/data/owner-beta-artifacts"),
                  self.mount(self.daily / "admitted", "/data/owner-beta-daily/admitted", stage != "admitter")]
        env = {"APP_ENV": "production", "PRICE_BETA_DAILY_MODE": "owner_only",
               "PRICE_BETA_DAILY_CONFIG_FILE": "/run/owner-beta-daily/config.json",
               "PRICE_BETA_DAILY_CONFIG_SHA256": fixture.digest(self.config_bytes),
               "PRICE_BETA_HEALTH_STATE_PATH": "/run/owner-beta-health/health.json"}
        network = "none" if stage == "producer" else self.network
        if stage != "worker":
            mounts += [self.mount(self.raw, "/data/raw"),
                       self.mount(self.daily / "candidates", "/data/owner-beta-daily/candidates", stage != "producer"),
                       self.mount(self.daily / "frontiers", "/data/owner-beta-daily/frontiers", stage != "admitter"),
                       self.mount(self.daily / (stage + "-state"), "/data/owner-beta-daily/" + stage + "-state", False)]
        if stage != "producer":
            db_env, db_mounts = self.db_env("daily_input_admitter" if stage == "admitter" else "worker", uid)
            env.update(db_env)
            mounts += db_mounts
        args = ["--once", "--artifact-root", "/data/owner-beta-artifacts"] if stage == "worker" else ["daily-" + stage, "--once"]
        output = self.once(stage, self.images["owner-beta-runner"], uid, network, mounts, env,
                           "/usr/local/bin/owner-beta-runner", args, include_stderr=True)
        fixed_outcome(output, "owner_beta_run" if stage == "worker" else "owner_beta_daily_cycle", expected)

    def api(self):
        env = {"APP_ENV": "production", "APP_HOST": "0.0.0.0", "APP_PORT": "8080",
               "LAGRANGE_CODE_COMMIT": self.args.candidate,
               "CURSOR_SECRET_FILE": "/run/secrets/cursor", "AUTH0_DOMAIN": "wholeflow.invalid",
               "AUTH0_CLIENT_ID": "synthetic-wholeflow", "AUTH0_CLIENT_SECRET_FILE": "/run/secrets/auth0",
               "AUTH0_REDIRECT_URI": "https://wholeflow.invalid/auth/callback", "SESSION_COOKIE_SECURE": "true",
               "ARTIFACT_ROOT": "/data/artifacts", "OWNER_BETA_ACCESS_MODE": "owner_only",
               "OWNER_BETA_PRICE_INPUT_MODE": "sealed_v1", "OWNER_BETA_PAPER_MODE": "disabled",
               "OWNER_BETA_DAILY_MODE": "owner_only", "DB_APP_MAX_CONNECTIONS": "2",
               "DB_ADMIN_MAX_CONNECTIONS": "1", "DB_AUDIT_MAX_CONNECTIONS": "1",
               "RECOMMENDATION_DATASET_VERSION_ID": self.config["owner"],
               "RECOMMENDATION_DATASET_ID": "krx_eod_bars", "RECOMMENDATION_DATASET_VERSION": "fixture-unused-generic-path",
               "RECOMMENDATION_CURATED_VERSION": "1", "RECOMMENDATION_DATASET_MANIFEST_SHA256": "f" * 64}
        mounts = [self.mount(self.base, "/data/artifacts"),
                  self.mount(self.secret_root / "cursor", "/run/secrets/cursor"),
                  self.mount(self.secret_root / "auth0", "/run/secrets/auth0")]
        for role, prefix in (("app", "DB"), ("admin", "ADMIN_DB"), ("audit_writer", "AUDIT_DB")):
            db_env, db_mounts = self.db_env(role, 10001, prefix)
            env.update(db_env)
            mounts += db_mounts
        self.api_container = self.create("api", self.images["api-server"], 10001, self.network,
                                         mounts, env, "/usr/local/bin/api-server")
        self.command(["start", self.api_container])
        value = self.inspect("container", self.api_container)
        addresses = list(value["NetworkSettings"]["Networks"].values())
        require(len(addresses) == 1 and addresses[0]["NetworkID"] == self.network, "WHOLEFLOW_API_NETWORK")
        self.api_address = addresses[0]["IPAddress"]
        address = ipaddress.ip_address(self.api_address)
        require(address.version == 4 and address.is_private and not address.is_loopback,
                "WHOLEFLOW_API_ADDRESS")
        until = time.monotonic() + self.remaining(45)
        while True:
            try:
                status, _ = self.http("GET", "/readyz", authenticated=False)
                if status == 200:
                    return
            except (OSError, http.client.HTTPException):
                pass
            require(time.monotonic() < until, "WHOLEFLOW_API_READY")
            time.sleep(0.5)

    def http(self, method, path, body=None, *, authenticated=True):
        require(method in ("GET", "POST") and path.startswith(("/readyz", "/api/v1/")), "WHOLEFLOW_HTTP_SCOPE")
        headers = {"Content-Type": "application/json", "X-Request-Id": str(uuid.uuid4())}
        if authenticated:
            headers["Cookie"] = "__Host-lagrange_session=" + self.cookie
            if method == "POST":
                headers["X-CSRF-Token"] = self.csrf
                headers["Idempotency-Key"] = str(uuid.uuid4())
        connection = http.client.HTTPConnection(self.api_address, 8080, timeout=self.remaining(10))
        try:
            connection.request(method, path, fixture.canonical(body) if body is not None else None, headers)
            response = connection.getresponse()
            data = response.read(1048577)
            require(len(data) <= 1048576, "WHOLEFLOW_HTTP_LIMIT")
            # No redirects, proxy environment, Auth0 endpoint or public host.
            return response.status, json.loads(data)
        finally:
            connection.close()

    def create_config(self):
        status, value = self.http("POST", "/api/v1/strategies/buy_and_hold/configs",
                                  {"strategy_version": "1.0.0", "config": {"benchmark_instrument": "069500.KRX",
                                   "target_weight": 1.0, "rebalance_cadence": "none"},
                                   "is_active": True, "owner_beta_daily_enabled": True})
        require(status == 201 and value["owner_beta_daily_enabled"] is True, "WHOLEFLOW_HTTP_CONFIG")
        self.strategy_id = str(uuid.UUID(value["id"]))

    def admissions(self):
        return json.loads(self.sql("SELECT coalesce(json_agg(row_to_json(a) ORDER BY range_end),'[]'::json) FROM ("
            "SELECT admission_sha256,parent_admission_sha256,content_sha256,manifest_sha256,range_start,range_end,"
            "session_count,bar_count FROM public.owner_beta_daily_admissions) a;"))

    def invariant(self, *, polls=False):
        # Capture full immutable DB witnesses, including publication items and
        # original job/attempt identities. No row body is persisted to a receipt.
        tables = ("owner_beta_daily_control", "owner_beta_daily_admissions", "owner_beta_daily_admission_events",
                  "owner_beta_daily_source_events", "owner_beta_daily_reservations", "owner_beta_daily_publication_claims",
                  "jobs", "job_attempts", "owner_beta_recommendation_runs", "owner_beta_recommendation_items")
        if polls:
            tables += ("owner_beta_daily_calendar_sources", "owner_beta_daily_calendar_days",
                       "owner_beta_daily_poll_receipts", "owner_beta_daily_poll_state")
        result = {}
        for table in tables:
            result[table] = json.loads(self.sql("SELECT coalesce(json_agg(v ORDER BY v::text),'[]'::json) "
                "FROM (SELECT to_jsonb(t) AS v FROM public." + table + " t) q;"))
        return result

    def publication(self):
        admissions = self.admissions()
        fixture.audit_admissions(admissions, self.plan["sessions"])
        path = "/api/v1/recommendations/owner-beta/price-only/daily-state?strategy_config_id=" + self.strategy_id
        status, state = self.http("GET", path)
        require(status == 200, "WHOLEFLOW_HTTP_STATE")
        runs = json.loads(self.sql("SELECT coalesce(json_agg(row_to_json(r)),'[]'::json) FROM (SELECT id,job_id,status,as_of,"
            "trigger_kind,daily_pins->>'admission_sha256' AS admission_sha256,factor_snapshot_sha256,target_snapshot_sha256 "
            "FROM public.owner_beta_recommendation_runs) r;"))
        require(len(runs) == 1, "WHOLEFLOW_UNIQUE_RUN")
        run = runs[0]
        fixture.audit_publication(state, run, admissions[-1], self.plan["sessions"][-1])
        status, detail = self.http("GET", "/api/v1/recommendations/owner-beta/price-only/runs/" + run["id"])
        require(status == 200 and detail["id"] == run["id"] and detail["job_id"] == run["job_id"]
                and detail["status"] == "SUCCEEDED" and detail["as_of"] == run["as_of"]
                and detail["input"]["admission_sha256"] == admissions[-1]["admission_sha256"]
                and detail["factor_snapshot_sha256"] == run["factor_snapshot_sha256"]
                and detail["target_snapshot_sha256"] == run["target_snapshot_sha256"], "WHOLEFLOW_HTTP_RUN")
        items = json.loads(self.sql("SELECT coalesce(json_agg(instrument_id ORDER BY instrument_id),'[]'::json) "
                                   "FROM public.owner_beta_recommendation_items;"))
        require(len(items) > 0 and sorted(item["instrument_id"] for item in detail["items"]) == items,
                "WHOLEFLOW_HTTP_ITEMS")
        self.receipt["publication"] = {key: run[key] for key in ("id", "job_id", "as_of", "admission_sha256",
                                                                "factor_snapshot_sha256", "target_snapshot_sha256")}

    def verify_sources(self):
        self.verify_base_views()
        current = self.inventory()
        prefix = "daily/" + base_views.DIRECTORY
        copied = lambda entry: entry["path"] == prefix or entry["path"].startswith(prefix + "/")
        require([e for e in self.original if copied(e)] == [e for e in current if copied(e)],
                "WHOLEFLOW_BASE_COPY_CHANGED")
        for name in ("raw", "base", "config", "curated", "floor"):
            before = [entry for entry in self.original if entry["path"].split("/")[0] == name]
            after = [entry for entry in current if entry["path"].split("/")[0] == name]
            require(before == after, "WHOLEFLOW_IMMUTABLE_SOURCE_CHANGED")
        # After restoration also re-observe the originally supplied base path;
        # the recovered copy alone cannot prove that the source was untouched.
        if self.base_root != self.original_base_root:
            roots = (*self.roots()[:-1], self.original_base_root)
            with files.Snapshot(roots, self.limits) as snapshot:
                require([e for e in snapshot.records if e["path"].split("/")[0] == "base"] ==
                        [e for e in self.original if e["path"].split("/")[0] == "base"],
                        "WHOLEFLOW_ORIGINAL_BASE_CHANGED")

    def replay(self, phase):
        before = self.invariant()
        self.pause_poll()
        self.daily_once("producer", "REPLAY")
        self.daily_once("admitter", "REPLAY")
        self.daily_once("worker", "IDLE")
        require(before == self.invariant(), "WHOLEFLOW_REPLAY_IDENTITY")
        self.publication()
        self.verify_sources()
        self.phase(phase)

    def capture_restore(self):
        self.stop(self.api_container)
        # All mutating production processes were one-shots and are terminal.
        # Close the API's audit writer before capture. There is no timer, daemon
        # producer or external network on this fixture database.
        baseline = self.invariant(polls=True)
        archive = self.root / "files.archive"
        with files.Snapshot(self.roots(), self.limits) as snapshot:
            with open(archive, "xb") as output:
                os.fchmod(output.fileno(), 0o600)
                snapshot.write(output)
                output.flush()
                os.fsync(output.fileno())
            captured_records = snapshot.records
        dump = self.root / "database.dump"
        with open(dump, "xb") as output:
            os.fchmod(output.fileno(), 0o600)
            self.command(["exec", "--user", "999:999", self.server, "pg_dump", "-U", "postgres",
                          "-d", "lagrange", "--format=custom", "--no-password"], output=output, limit=120)
            output.flush()
            os.fsync(output.fileno())
        require(0 < dump.stat().st_size <= self.args.max_bytes and baseline == self.invariant(polls=True),
                "WHOLEFLOW_CAPTURE_CHANGED")
        self.stop(self.server)
        self.original_server = self.server
        restored = directory(self.root / "restored")
        with open(archive, "rb") as source:
            files.restore_archive(source, self.limits, str(restored))
        self.current = restored
        self.raw, self.curated, self.daily = restored / "raw", restored / "curated", restored / "daily"
        self.config_root, self.floor, self.base = restored / "config", restored / "floor", restored / "base"
        self.base_root = files.Root("base", str(self.base), 10001, 10001, 0o750)
        self.verify_base_views()
        require(self.inventory() == captured_records, "WHOLEFLOW_RESTORED_FILE_IDENTITY")
        self.database("recovered", migrate=False)
        # Restore owners/ACLs, without --no-owner/--no-acl and without seeding
        # operational history. Roles were bootstrapped by the actual image.
        with open(dump, "rb") as source:
            try:
                result = subprocess.run(["/usr/bin/docker", "--host", "unix:///var/run/docker.sock",
                    "exec", "--interactive", "--user", "999:999", self.server, "pg_restore", "-U", "postgres",
                    "-d", "lagrange", "--exit-on-error", "--clean", "--if-exists", "--no-password"],
                    env=ENV, stdin=source, capture_output=True, timeout=self.remaining(120), check=False)
            except (OSError, subprocess.TimeoutExpired):
                raise fixture.FixtureError("WHOLEFLOW_RESTORE_UNCERTAIN") from None
            require(result.returncode == 0, "WHOLEFLOW_RESTORE_FAILED")
        self.schema_audit()
        require(baseline == self.invariant(polls=True), "WHOLEFLOW_RESTORED_POLL_OR_HISTORY")
        require(not self.inspect("container", self.original_server)["State"]["Running"],
                "WHOLEFLOW_ORIGINAL_RESTARTED")
        self.phase("CAPTURED_STATE_EXACTLY_RESTORED")
        self.api()
        self.replay("RESTORED_PROCESSES_PRESERVE_PUBLICATION")

    def execute(self):
        self.prepare()
        self.events = lifecycle.Recorder(self.nonce, self.args.lifecycle_fd, self.args.timeout_seconds + 300)
        self.events.start()
        self.database("initial", migrate=True)
        self.schema_audit()
        self.seed()
        self.api()
        self.create_config()
        self.daily_once("producer", "CANDIDATE_READY")
        self.daily_once("admitter", "ADMITTED")
        first = self.admissions()
        require(len(first) == 1 and first[0]["range_end"] == self.plan["sessions"][0]
                and first[0]["parent_admission_sha256"] is None, "WHOLEFLOW_FIRST_ADMISSION")
        self.phase("ACTUAL_FIRST_ADMISSION")
        self.pause_poll()
        self.daily_once("producer", "CANDIDATE_READY")
        self.daily_once("admitter", "ADMITTED")
        fixture.audit_admissions(self.admissions(), self.plan["sessions"])
        self.daily_once("worker", "SUCCEEDED")
        self.publication()
        self.verify_sources()
        self.phase("ACTUAL_SUBSEQUENT_ADMISSION_AND_HTTP_PUBLICATION")
        self.stop(self.api_container)
        self.api()
        self.replay("RESTART_PRESERVES_PUBLICATION")
        self.capture_restore()

    def cleanup(self):
        if self.root is None:
            return
        # Resolve any lost create reply by exact nonce-labelled inventory. An
        # unexpected name or label stops cleanup; never touch a foreign object.
        for kind in ("container", "network"):
            args = [kind, "ls", "--quiet", "--filter", "label=" + LABEL + "=" + self.nonce]
            if kind == "container":
                args.append("--all")
            identities = self.command(args, cleanup=True).decode().split()
            for identity in identities:
                value = self.inspect(kind, identity, cleanup=True)
                labels = value["Config"]["Labels"] if kind == "container" else value["Labels"]
                name = value["Name"].lstrip("/")
                require(labels.get(LABEL) == self.nonce and {"kind": kind, "name": name} in self.intents,
                        "WHOLEFLOW_CLEANUP_FOREIGN")
                # Never erase the association to an unexpected persistent
                # volume and then claim complete cleanup. Keep the labelled
                # container and private intent evidence for bounded recovery.
                require(kind != "container" or not any(m["Type"] == "volume" for m in value["Mounts"]),
                        "WHOLEFLOW_CLEANUP_VOLUME")
                self.command([kind, "rm", "--force", value["Id"]] if kind == "container" else
                             ["network", "rm", value["Id"]], cleanup=True)
                self.uncertain_creations.discard((kind, name))
            require(not self.command(args, cleanup=True).strip(), "WHOLEFLOW_CLEANUP_REMAINING")
        require(not self.uncertain_creations, "WHOLEFLOW_CREATE_REPLY_UNRESOLVED")
        self.receipt["cleanup"] = "OWNED_CONTAINERS_NETWORKS_ABSENT"
        shutil.rmtree(self.root)
        self.root = None


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--execute", action="store_true", required=True)
    parser.add_argument("--candidate", required=True)
    parser.add_argument("--image-manifest", required=True, type=Path)
    parser.add_argument("--approved-base", required=True, type=Path)
    parser.add_argument("--receipt", required=True, type=Path)
    parser.add_argument("--timeout-seconds", type=int, default=1200)
    parser.add_argument("--max-bytes", type=int, default=2 * 1024**3)
    args = parser.parse_args(argv)
    test = WholeFlow(args)
    result = 1
    receipt_fd = None
    lifecycle_fd = None
    try:
        require(args.receipt.is_absolute() and not args.receipt.exists(), "WHOLEFLOW_RECEIPT_EXISTS")
        with release.root_directory(str(args.receipt.parent)) as parent:
            receipt_fd = os.open(args.receipt.name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                                 0o600, dir_fd=parent)
            os.fchmod(receipt_fd, 0o600)
            lifecycle_fd = os.open(args.receipt.name + ".events.jsonl",
                                   os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                                   0o600, dir_fd=parent)
            os.fchmod(lifecycle_fd, 0o600)
            args.lifecycle_fd = lifecycle_fd
        require(sys.platform == "linux" and 600 <= args.timeout_seconds <= 1800, "WHOLEFLOW_DEADLINE_INPUT")
        signal.signal(signal.SIGALRM, expired)
        signal.setitimer(signal.ITIMER_REAL, args.timeout_seconds)
        test.execute()
        test.receipt["result"] = "PASS"
        result = 0
    except BaseException as error:
        test.receipt["result"] = "FAIL"
        code = str(error) if isinstance(error, (fixture.FixtureError, lifecycle.EvidenceError)) else "WHOLEFLOW_UNCLASSIFIED"
        test.receipt["code"] = code if re.fullmatch(r"WHOLEFLOW_[A-Z_]+", code) else "WHOLEFLOW_UNCLASSIFIED"
    finally:
        if sys.platform == "linux":
            signal.setitimer(signal.ITIMER_REAL, 0)
        try:
            test.cleanup()
        except BaseException:
            test.receipt["result"], test.receipt["cleanup"] = "FAIL", "UNCONFIRMED_STAGE_RETAINED"
            test.receipt["retained_stage"] = str(test.root) if test.root is not None else None
            result = 1
        try:
            if test.events is not None:
                evidence = test.events.finish(test.event_objects)
                test.receipt["lifecycle"] = evidence
                require(evidence["status"] == "COMPLETE", "WHOLEFLOW_EVENTS_INCOMPLETE")
        except BaseException:
            test.receipt["result"] = "FAIL"
            test.receipt["code"] = "WHOLEFLOW_EVENTS_INCOMPLETE"
            result = 1
        finally:
            if lifecycle_fd is not None and (test.events is None or
                    all(not thread.is_alive() for thread in test.events.threads)):
                os.close(lifecycle_fd)
        # Refuse overwriting any existing result. If writing fails, exit failure
        # without dumping an exception, credential, database row or provider body.
        try:
            require(receipt_fd is not None, "WHOLEFLOW_RECEIPT_UNAVAILABLE")
            with os.fdopen(receipt_fd, "wb") as output:
                receipt_fd = None
                output.write(fixture.canonical(test.receipt) + b"\n")
                output.flush()
                os.fsync(output.fileno())
        except BaseException:
            result = 1
        finally:
            if receipt_fd is not None:
                os.close(receipt_fd)
        print("WHOLEFLOW: " + ("PASS" if result == 0 else "FAIL"), flush=True)
    return result


if __name__ == "__main__":
    raise SystemExit(main())
