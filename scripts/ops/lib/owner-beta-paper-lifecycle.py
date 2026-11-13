#!/usr/bin/env python3
"""Retained host custody for Paper's single-action lifecycle and RESET queries.

All returned values describe original native bytes. They confer no eligibility.
A caller must exit open_lifecycle successfully before consuming a result. The
absolute monotonic_ns integer deadline never extends native or host timeouts.
"""
from __future__ import annotations

import argparse
import contextlib
import dataclasses
import fcntl
import importlib.util
import itertools
import os
from pathlib import Path
import stat
import sys
import time
import uuid
import weakref

sys.dont_write_bytecode = True

def _load(name, filename):
    if name in sys.modules:
        return sys.modules[name]
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module

ops = _load("owner_beta_paper_operations", "owner-beta-paper-operations.py")
journal, daily = ops.journal, ops.daily
STORE = ".owner-beta-paper-lifecycle"
MAX = 65_536
_CALLS = weakref.WeakSet()
# Strong local custody after unknown cleanup; never an eligibility input.
_UNRESOLVED = []

class LifecycleError(Exception):
    def __init__(self, code, *, result=None):
        self.code, self.result = code, result
        super().__init__(code)

def require(value, code="LIFECYCLE_INPUT"):
    if not value:
        raise LifecycleError(code)

@dataclasses.dataclass(frozen=True)
class LifecycleResult:
    outcome: str
    native_result: bytes | None
    readback: bytes | None
    artifact: bytes | None
    cleanup: str

class _Deadline:
    def __init__(self, absolute_ns):
        require(type(absolute_ns) is int and absolute_ns > time.monotonic_ns(), "LIFECYCLE_DEADLINE")
        self.end = absolute_ns
        self.tick = None
    def remaining(self, cap=90):
        require(type(cap) in (int, float) and cap > 0, "LIFECYCLE_DEADLINE")
        if self.tick is not None:
            self.tick()
        left = (self.end - time.monotonic_ns()) / 1_000_000_000
        require(left > 0, "LIFECYCLE_DEADLINE")
        return min(cap, left)


def prepare_storage(install_root):
    """Installer-only private directories; no identity, reset or readiness fact."""
    daily.install.require_root()
    with daily.install.root_directory(install_root) as parent:
        root = journal._open_child(parent, STORE, 0o700, create=True)
        try:
            for name in ("queries", "selections", "anchors", "staging"):
                fd = journal._open_child(root, name, 0o700, create=True)
                os.close(fd)
        finally:
            os.close(root)


class _Original:
    def __init__(self, parent, name, *, mode=0o400, uid=0, gid=0, maximum=MAX):
        self.parent, self.name = parent, name
        self.fd, self.raw, self.identity = ops._open_file(parent, name, mode, uid=uid, gid=gid, maximum=maximum)
        self.mode, self.uid, self.gid = mode, uid, gid
    def revalidate(self):
        ops._file(self.fd, self.mode, uid=self.uid, gid=self.gid)
        require(ops._identity(os.fstat(self.fd)) == self.identity
                and ops._identity(os.stat(self.name, dir_fd=self.parent, follow_symlinks=False)) == self.identity
                and os.pread(self.fd, len(self.raw) + 1, 0) == self.raw, "LIFECYCLE_ORIGINAL_CHANGED")
    def close(self):
        self.revalidate()
        os.close(self.fd)


class _Store:
    def __init__(self, context):
        self.context = context
        self.tree = journal._Tree(context.install_root + "/" + STORE)
        journal._dir_info(self.tree.fd, 0o700)
        self.dirs, self.originals = {}, []
        for name in ("queries", "selections", "anchors", "staging"):
            self.dirs[name] = journal._open_child(self.tree.fd, name, 0o700)
        self.stage_root = context.install_root + "/" + STORE + "/staging"
        self.query_dirs = []
        self.pending_at_open = bool(os.listdir(self.dirs["staging"]))
    def retain(self, parent, name, maximum=MAX):
        item = _Original(parent, name, maximum=maximum)
        self.originals.append(item)
        return item.raw
    def publish(self, kind, raw):
        name = ops._digest(raw)[7:] + ".json"
        journal._immutable(self.dirs[kind], name, raw, 0o400)
        require(self.retain(self.dirs[kind], name) == raw, "LIFECYCLE_ORIGINAL_CHANGED")
        return name
    def query(self, intent, evidence=None, sql=None, binding=None):
        name = str(uuid.uuid4())
        fd = journal._open_child(self.dirs["queries"], name, 0o700, create=True)
        self.query_dirs.append((name, fd))
        for leaf, raw in (("request.json", intent), ("evidence.json", evidence), ("sql-request.json", sql), ("action-binding.json", binding)):
            if raw is not None:
                journal._immutable(fd, leaf, raw, 0o400)
                require(self.retain(fd, leaf, maximum=journal.MAX_NATIVE_EXACT if leaf == "sql-request.json" else MAX) == raw, "LIFECYCLE_ORIGINAL_CHANGED")
        output = journal._open_child(fd, "output", 0o700, create=True)
        os.close(output)
        return fd
    def revalidate(self):
        self.tree.revalidate()
        for name, fd in self.dirs.items():
            journal._check_named_dir(self.tree.fd, name, fd)
        for name, fd in self.query_dirs:
            journal._check_named_dir(self.dirs["queries"], name, fd)
        for item in self.originals:
            item.revalidate()
    def close(self):
        self.revalidate()
        for item in self.originals:
            item.close()
        for _, fd in reversed(self.query_dirs):
            os.close(fd)
        for fd in self.dirs.values():
            os.close(fd)
        self.tree.close()


def _intent(installation, manifest, purpose, account, epoch, artifact=None, *, generation=None, key=None, target=None):
    generation = generation or str(uuid.uuid4())
    return dict(schema="owner-beta-paper-operation-intent-v1", installation_id=installation,
                generation=generation, account_id=account, predecessor_generation=None,
                request_key=key or (generation + {"ENABLE": ":enable", "RECOVERY": ":recover"}.get(purpose, ":inspect")),
                purpose=purpose, scope="PAPER_STOP", expected_epoch=epoch, local_epoch_floor=epoch,
                release_manifest_sha256=manifest, artifact_sha256=None if artifact is None else ops._digest(artifact),
                resume_target_id=target)


def _exact_readback(raw, request):
    """Correlate selected native readback bytes; never use them as authority."""
    value = ops._json(raw, 1_048_576)
    q = ops._json(request, journal.MAX_NATIVE_EXACT)
    require(type(value) is dict and set(value) == {"schema", "kind", "request_sha256", "status", "account_id",
            "current_epoch", "current_state", "event_id", "event_epoch", "result_canonical", "result_sha256", "reset"}
            and value["schema"] == "owner-beta-paper-exact-readback-v1"
            and value["kind"] == q["kind"] and value["request_sha256"] == ops._digest(request), "LIFECYCLE_READBACK")
    require(q["account_id"] is None or q["account_id"] == value["account_id"], "LIFECYCLE_READBACK")
    if value["status"] == "RESET":
        require(q["kind"] == "RESET" and value["current_epoch"] == q["args"]["expected_epoch"]
                and value["current_state"] in ("OFF", "RESTORED_OFF", "ENABLED", "DRAINING", "BLOCKED")
                and all(value[k] is None for k in ("event_id", "event_epoch", "result_canonical", "result_sha256")), "LIFECYCLE_RESET")
        _reset(value["reset"], q["account_id"], q["args"]["expected_epoch"])
    elif value["status"] == "COMMITTED":
        require(q["kind"] != "RESET" and value["reset"] is None, "LIFECYCLE_READBACK")
        journal._uuid(value["event_id"])
        require(type(value["event_epoch"]) is int and type(value["current_epoch"]) is int
                and 0 <= value["event_epoch"] <= value["current_epoch"], "LIFECYCLE_READBACK")
        require(type(value["result_canonical"]) is str, "LIFECYCLE_READBACK")
        original = value["result_canonical"].encode("ascii")
        ops._json(original, ops.MAX_PAGE)
        require(ops._digest(original) == value["result_sha256"], "LIFECYCLE_READBACK")
    else:
        require(q["kind"] != "RESET" and value["status"] in ("ABSENT", "UNPROVEN")
                and all(value[k] is None for k in ("event_id", "event_epoch", "result_canonical", "result_sha256", "reset")), "LIFECYCLE_READBACK")
    return value


def _reset(value, account, epoch):
    require(type(value) is dict and set(value) == {"account_id", "event_id", "kind", "control_epoch", "occurred_at_us", "payload_sha256"}
            and value["account_id"] == account and type(value["control_epoch"]) is int
            and 0 <= value["control_epoch"] <= epoch and value["kind"] in ("CONFIGURED", "OFF", "RESTORED_OFF")
            and (value["kind"] == "CONFIGURED") == (value["control_epoch"] == 0)
            and type(value["occurred_at_us"]) is int and value["occurred_at_us"] >= 0, "LIFECYCLE_RESET")
    journal._uuid(account)
    journal._uuid(value["event_id"])
    journal._hash(value["payload_sha256"])


def _anchor(raw, selection):
    value = ops._json(raw, MAX)
    require(type(value) is dict and set(value) == set(selection) | {"boot_id", "observed"}
            and value["schema"] == "owner-beta-paper-reset-anchor-v1"
            and all(value[k] == v for k, v in selection.items() if k != "schema"), "LIFECYCLE_ANCHOR")
    journal._uuid(value["boot_id"])
    clock = value["observed"]
    require(type(clock) is dict and set(clock) == {"realtime_ns", "monotonic_ns", "boottime_ns"}
            and all(type(n) is int and 0 < n <= journal.MAX_I64 for n in clock.values())
            and clock["realtime_ns"] > selection["reset"]["occurred_at_us"] * 1000, "LIFECYCLE_ANCHOR")
    return value


class _Inputs:
    """Retained parent/file owners. Current may refresh, its binding may not."""
    def __init__(self, controller, *, financial):
        self.controller, self.financial = controller, financial
        self.stack = contextlib.ExitStack()
        self.fds, self.originals, self.checks = [], [], []
        self.view = None
        self.closed = False
        self.mounts = []
    def root(self, path, uid=0, gid=None, mode=None):
        # Root-protected ancestry is checked by the existing reader. Raw's last
        # component has its own installed owner; it is never chmod/chowned here.
        base = _load("owner_beta_lifecycle_base", "owner-beta-daily-base.py")
        fd = self.stack.enter_context(base.files.open_root(base.files.Root(
            "raw" if uid != 0 else "paper_evidence", path, uid, gid if gid is not None else uid, mode or 0o750)))
        identity = ops._identity(os.fstat(fd))
        parent, leaf = os.path.split(path)
        def check():
            require(ops._identity(os.fstat(fd)) == identity and
                    ops._identity(os.stat(path, follow_symlinks=False)) == identity, "LIFECYCLE_SOURCE_CHANGED")
        self.checks.append(check)
        return fd
    def open(self, call):
        c = self.controller.context
        evidence = _load("owner_beta_lifecycle_evidence", "owner-beta-paper-evidence.py")
        data = c.values["LAGRANGE_DATA_DIR"]
        current_path = data + "/owner-beta-paper-evidence/current"
        current_fd = self.root(current_path, gid=10001)
        def current():
            raw = daily.install.read_file(current_fd, evidence.CURRENT_LEAF, 0, 10001, 0o440, evidence.SOURCE_LIMIT)
            value = evidence.decode(raw)
            evidence.validate_current(value)
            require(value["status"] == "CURRENT" and type(value["binding"]) is dict, "LIFECYCLE_CURRENT")
            return value["binding"]
        self.binding = current()
        b = self.binding
        require(b["release_root"] == c.release_path and b["data_root"] == data
                and b["secret_root"] == c.values["LAGRANGE_RUNTIME_SECRET_DIR"]
                and b["production_env_sha256"] == ops._digest(c._env_bytes)
                and b["cohort"]["release_manifest_sha256"] == c.manifest_sha256
                and b["cohort"]["verifier_revision"] == c.commit, "LIFECYCLE_CURRENT")
        self.checks.append(lambda: require(current() == self.binding, "LIFECYCLE_CURRENT_CHANGED"))
        self.mounts.append((current_path, "/data/owner-beta-paper-evidence/current", True))
        if self.financial:
            event_path = data + "/owner-beta-paper-evidence/events"
            self.events_fd = self.root(event_path, gid=10001)
            self.mounts.append((event_path, "/data/owner-beta-paper-evidence/events", True))
            config, pin = daily.install.verify_config(c.release_path, c.commit)
            require(pin == b["daily_config_sha256"] and config["raw_owner_uid"] == b["raw_owner_uid"], "LIFECYCLE_CURRENT")
            for host, alias, uid, gid in ((data + "/raw", "/data/raw", config["raw_owner_uid"], 10001),
                    (data + "/owner-beta-daily/admitted", "/data/owner-beta-daily/admitted", 10003, 10001)):
                self.root(host, uid, gid)
                self.mounts.append((host, alias, True))
            daily_path = c.release_path + "/.owner-beta-daily"
            fd = self.root(daily_path, gid=10001)
            for name in ("config.json", "policy.json"):
                item = _Original(fd, name, mode=0o440, gid=10001, maximum=MAX)
                self.originals.append(item)
                self.mounts.append((daily_path + "/" + name, "/run/owner-beta-daily/" + name, True))
            process_path = c.release_path + "/" + ops.release.PAPER_DIRECTORY + "/config.json"
            process_dir = self.root(os.path.dirname(process_path), gid=10001)
            original = _Original(process_dir, "config.json", mode=0o440, gid=10001)
            self.originals.append(original)
            require(c.verified is not None and ops._digest(original.raw) == c.verified.process_sha256, "LIFECYCLE_PROCESS_CHANGED")
            self.mounts.append((process_path, "/run/owner-beta-paper/config.json", True))
            base = _load("owner_beta_lifecycle_base", "owner-beta-daily-base.py")
            stage = call._new_stage("financial-")
            source = c.values["LAGRANGE_ARTIFACTS_DIR"] + "/historical-price-beta-root"
            self.view = self.stack.enter_context(base.native_root_view(source, stage.fd))
            stage.snapshot(mutable_children=("historical",))
            self.mounts.append((stage.path + "/historical", "/data/owner-beta-artifacts", True))
            secret_root = c.values["LAGRANGE_RUNTIME_SECRET_DIR"]
            secret_fd = self.root(secret_root + "/owner-beta-runner", 10001, 10001)
            self.worker = _Original(secret_fd, "db_worker_password", uid=10001, gid=10001)
            self.originals.append(self.worker)
            require(0 < len(self.worker.raw) <= 4096 and not any(x in self.worker.raw for x in (0, 10, 13)), "LIFECYCLE_SECRET")
        self.revalidate()
    def anchor_originals(self, selection, originals):
        if len(originals) <= 3:
            return originals
        # More boots may have been observed than the last three completed
        # Source sessions need. Select those boots from retained actual chains;
        # never prune the persistent anchor history or accept a caller list.
        evidence = _load("owner_beta_lifecycle_evidence", "owner-beta-paper-evidence.py")
        with os.scandir(self.events_fd) as entries:
            names = sorted(entry.name for entry in itertools.islice(entries, 4097))
        require(len(names) <= 4096, "LIFECYCLE_SOURCE_BOUND")
        selected, used, files_used = {}, 0, 0
        for name in names:
            self.controller.deadline.remaining()
            require(len(name) == 64 and journal.HASH_RE.fullmatch("sha256:" + name), "LIFECYCLE_SOURCE_CHAIN")
            child = os.open(name, journal.DIR_FLAGS, dir_fd=self.events_fd)
            self.stack.callback(os.close, child)
            daily.install.directory_info(child, 0, 10001, 0o750)
            identity = ops._identity(os.fstat(child))
            self.checks.append(lambda child=child,name=name,identity=identity: require(
                ops._identity(os.stat(name,dir_fd=self.events_fd,follow_symlinks=False)) == identity
                and ops._identity(os.fstat(child)) == identity, "LIFECYCLE_SOURCE_CHANGED"))
            with os.scandir(child) as entries:
                leaves = sorted(entry.name for entry in itertools.islice(entries, 65))
            require(1 <= len(leaves) <= 64 and leaves == ["%06d.json" % i for i in range(len(leaves))], "LIFECYCLE_SOURCE_CHAIN")
            self.checks.append(lambda child=child,leaves=tuple(leaves): require(
                tuple(sorted(os.listdir(child))) == leaves, "LIFECYCLE_SOURCE_CHANGED"))
            chain = []
            for leaf in leaves:
                files_used += 1
                require(files_used <= 4096, "LIFECYCLE_SOURCE_BOUND")
                item = _Original(child, leaf, mode=0o440, gid=10001, maximum=evidence.SOURCE_LIMIT)
                self.originals.append(item)
                used += len(item.raw)
                require(used <= 16 * 1024 * 1024, "LIFECYCLE_SOURCE_BOUND")
                chain.append(evidence.decode(item.raw))
            evidence.verify_chain(chain)
            first = chain[0]
            require(first["slot_id"] == "sha256:" + name, "LIFECYCLE_SOURCE_CHAIN")
            if first["cohort"] != self.binding["cohort"] or chain[-1]["kind"] != "LINKED":
                continue
            arm = next(event for event in chain if event["kind"] == "ARM_INTENT")
            if arm["observed"]["realtime_ns"] <= selection["reset"]["occurred_at_us"] * 1000:
                continue
            require(first["session"] not in selected, "LIFECYCLE_SOURCE_CHAIN")
            selected[first["session"]] = arm
        require(len(selected) >= 3, "LIFECYCLE_SOURCE_SESSIONS_REQUIRED")
        arms = [selected[session] for session in sorted(selected)[-3:]]
        by_boot = {_anchor(raw, selection)["boot_id"]: raw for raw in originals}
        result = []
        for boot in sorted({event["arm"]["boot_id"] for event in arms}):
            require(boot in by_boot, "LIFECYCLE_ANCHOR_REQUIRED")
            original = by_boot[boot]
            anchor = _anchor(original, selection)
            for event in arms:
                if event["arm"]["boot_id"] == boot:
                    require(all(event["observed"][clock] > anchor["observed"][clock]
                                for clock in anchor["observed"]), "LIFECYCLE_LATE_ANCHOR")
            result.append(original)
        self.revalidate()
        return tuple(result)
    def revalidate(self):
        require(not self.closed, "LIFECYCLE_CLOSED")
        self.controller.deadline.remaining()
        for check in self.checks:
            check()
        for item in self.originals:
            item.revalidate()
        if self.view is not None:
            self.view.revalidate()
    def close(self):
        if self.closed:
            return
        self.revalidate()
        for item in self.originals:
            item.close()
        # Normal exit executes source_artifact's final identity/hash checks,
        # including when the consuming native call failed.
        self.stack.close()
        self.closed = True


class _Call:
    def __init__(self, controller, verb, *, action=None, custody=None, intent=None, evidence=None, sql=None,
                 selection=None, anchors=()):
        self.controller, self.context, self.verb = controller, controller.context, verb
        self.action, self.custody = action, custody
        self.intent, self.evidence, self.sql = intent, evidence, sql
        self.selection, self.anchors = selection, anchors
        self.call, self.inputs = None, None
        self.groups = ("10001",) if verb in ("enable", "recover") else ()
        self.output_fd, self.query_fd = None, None
        self.stage_root = controller.store.stage_root
        self.closed = False
        binding = (action._binding_bytes if action is not None else
                   custody._action._binding_bytes if custody is not None and isinstance(custody._action, journal.ActionCustody) else
                   custody._action if custody is not None and isinstance(custody._action, bytes) else None)
        self.receipt_fd = controller.store.query(
            intent or (action._operation._intent_bytes if action is not None else custody._intent),
            evidence if evidence is not None else action._artifact_bytes if action is not None else custody._artifact if custody is not None else None,
            sql if sql is not None else custody._sql if custody is not None else None, binding)
        if action is not None:
            self.output_fd = action.borrow_output_fd()
        elif custody is not None:
            self.output_fd = custody.borrow_output_fd()
        else:
            self.query_fd = self.receipt_fd
            self.output_fd = journal._open_child(self.query_fd, "output", 0o700)
        _CALLS.add(self)
    def revalidate(self):
        require(self in _CALLS and not self.closed, "LIFECYCLE_CLOSED")
        self.controller.revalidate()
        if self.action is not None:
            self.action.revalidate()
        if self.custody is not None:
            self.custody._check()
        if self.inputs is not None and not self.inputs.closed:
            self.inputs.revalidate()
    def prepare(self, call):
        require(self.call is call, "LIFECYCLE_PRIVATE")
        self.revalidate()
        stage = call._new_stage("native-")
        if self.action is not None:
            self.action.stage_inputs(stage.fd)
            call._persistent_stage = stage
            self.context._input_stages[self.action] = {"stage": stage, "imports": []}
        elif self.custody is not None:
            self.custody.stage_readback(stage.fd)
        else:
            for name, raw in (("request.json", self.intent), ("evidence.json", self.evidence), ("sql-request.json", self.sql)):
                if raw is not None:
                    ops._write_file(stage.fd, name, raw)
        ops._write_file(stage.fd, "database.json", ops._canonical(self.context.database))
        mounts = [(stage.path, ops.PRIVATE, True)]
        if self.verb not in ("readback", "inspect"):
            require(self.context.verified is not None, "LIFECYCLE_RELEASE")
            self.context.verified.stage_native_inputs(stage.fd)
        if self.action is not None:
            operation = self.action._operation
            state_path = self.context.install_root + "/" + ops.PAPER_DIR
            state_fd = os.open(ops.PAPER_DIR, journal.DIR_FLAGS, dir_fd=self.context.tree.fd)
            call._paper_state_fd = state_fd
            call._paper_state_identity = ops._identity(ops._dir(state_fd))
            for name, expected in (("installation-id", operation._intent["installation_id"]),
                                   ("epoch-floor", str(operation._intent["local_epoch_floor"]))):
                fd, raw, identity = ops._open_file(state_fd, name, 0o600, maximum=64)
                call._paper_state_files += ((fd, name, identity, raw),)
                require(raw == expected.encode("ascii") + b"\n", "LIFECYCLE_FLOOR")
            for name in ("operation", "paper-state"):
                os.mkdir(name, 0o700, dir_fd=stage.fd)
            mounts += [(ops._path_from_fd(operation.borrow_directory_fd()), ops.OPERATION_ALIAS, True),
                       (state_path, ops.PAPER_STATE_ALIAS, True)]
        if self.selection is not None:
            ops._write_file(stage.fd, "reset-selection.json", ops._canonical(self.selection))
        secret = call._new_stage("secret-")
        password, sources = ops._read_migration_secret(self.context.values["LAGRANGE_RUNTIME_SECRET_DIR"])
        call._secret_sources = sources
        require(0 < len(password) <= 4096 and not any(x in password for x in (0, 10, 13)), "LIFECYCLE_SECRET")
        ops._write_file(secret.fd, "db_migration_owner_password", password)
        password = None
        if self.verb in ("observe-reset", "enable", "recover"):
            self.inputs = _Inputs(self.controller, financial=self.verb != "observe-reset")
            self.inputs.open(call)
            mounts += self.inputs.mounts
            if self.inputs.financial:
                ops._write_file(secret.fd, "db_worker_password", self.inputs.worker.raw)
        if self.verb == "enable":
            selected = self.inputs.anchor_originals(self.selection, self.anchors)
            os.mkdir("reset-anchors", 0o700, dir_fd=stage.fd)
            fd = journal._open_child(stage.fd, "reset-anchors", 0o700)
            try:
                require(1 <= len(selected) <= 3, "LIFECYCLE_ANCHOR")
                for raw in selected:
                    _anchor(raw, self.selection)
                    ops._write_file(fd, ops._digest(raw)[7:] + ".json", raw)
                os.fsync(fd)
            finally:
                os.close(fd)
        ops._ensure_output_mountpoint(stage.fd)
        mounts += [(secret.path, "/run/secrets", True), (ops._path_from_fd(self.output_fd), ops.OUTPUT_ALIAS, False)]
        call._mounts = tuple(mounts)
        stage.snapshot()
        secret.snapshot()
        self.revalidate()
    def finish_inputs(self, *, preserve):
        if self.inputs is not None:
            self.inputs.close()
        require(not preserve, "CLEANUP_UNKNOWN")
        self.controller.deadline.remaining()
    def close(self):
        if self.query_fd is not None:
            os.close(self.output_fd)
        self.closed = True
        _CALLS.discard(self)


def _assert_call(owner, context, verb, custody):
    require(type(owner) is _Call and owner in _CALLS and not owner.closed
            and owner.context is context and owner.call is None and owner.verb == verb
            and custody is (owner.action if owner.action is not None else owner.custody)
            and verb in ("configure", "enable", "recover", "observe-reset", "readback", "inspect"), "LIFECYCLE_PRIVATE")
    owner.revalidate()


class Lifecycle:
    def __init__(self, context, lock_fd, deadline):
        self.deadline = _Deadline(deadline)
        self.context = ops._PaperContext(ops._CONTEXT_TOKEN, context, lock_fd, live=True, deadline=self.deadline)
        self.store = _Store(self.context)
        self.context._lifecycle_stage_root = self.store.stage_root
        self.journal_manager = None
        self.journal = None
        self.actions, self.operations, self.imports, self.calls = [], [], [], []
        self.last = None
        self.pending_anchors = []
        self.closed = False
    def revalidate(self):
        require(not self.closed, "LIFECYCLE_CLOSED")
        self.deadline.remaining()
        self.context.revalidate()
        self.store.revalidate()
        if self.journal is not None:
            self.journal.revalidate()
    def _journal(self, *, bootstrap=False):
        if self.journal is None:
            self.journal_manager = journal.open_journal(self.context.install_root, self.context.lock_fd, bootstrap=bootstrap)
            self.journal = self.journal_manager.__enter__()
        return self.journal
    def _process(self):
        self.revalidate()
        verified = self.context.verified
        require(verified is not None and verified.process_sha256 is not None, "LIFECYCLE_PROCESS_REQUIRED")
        original = _Original(verified._fds[-1], "process.json", mode=0o440, gid=10001)
        self.store.originals.append(original)
        raw = original.raw
        require(ops._digest(raw) == verified.process_sha256, "LIFECYCLE_PROCESS_CHANGED")
        value = ops.release._validate_process(ops._json(raw, MAX), self.context.commit, self.context.manifest_sha256)
        return value, raw
    def _run(self, owner):
        self.calls.append(owner)
        native = artifact = readback = None
        outcome = "COMMIT_UNKNOWN"
        call = self.context._invoke_lifecycle(owner, owner.verb, owner.action or owner.custody)
        failure = None
        try:
            call.__enter__()
            result = call.result
            native, artifact = call._stdout, call._result_bytes
            journal._immutable(owner.receipt_fd, "native-result.json", native, 0o400)
            self.store.retain(owner.receipt_fd, "native-result.json", maximum=ops.MAX_RESULT + 1)
            if artifact is not None:
                journal._immutable(owner.receipt_fd, "artifact.json", artifact, 0o400)
                self.store.retain(owner.receipt_fd, "artifact.json", maximum=1_048_576)
            if owner.action is not None:
                try:
                    os.stat("dispatch.json", dir_fd=owner.output_fd, follow_symlinks=False)
                except FileNotFoundError:
                    require(result["outcome"] == "BLOCKED", "LIFECYCLE_DISPATCH_MISSING")
                    call._imported = True  # retained non-dispatch rejection only
                else:
                    custody = call.import_dispatch()
                    journal._immutable(owner.receipt_fd, "sql-request.json", custody._sql, 0o400)
                    self.store.retain(owner.receipt_fd, "sql-request.json", maximum=journal.MAX_NATIVE_EXACT)
                    self.imports.append(custody)
            else:
                if owner.verb == "readback" and artifact is not None:
                    readback = call.readback_artifact()
                    _exact_readback(readback, owner.sql if owner.sql is not None else owner.custody._sql)
                call._imported = True  # readonly result original retained, including BLOCKED
                call._preserve = False
            outcome = result["outcome"]
        except BaseException as error:
            failure = error
            # ACK loss: import only the existing exact original Q. Never invoke
            # the mutation again. Missing/partial dispatch remains uncertainty.
            if owner.action is not None:
                try:
                    custody = owner.action.import_dispatch()
                    self.context._register_imported_custody(owner.action, custody)
                    journal._immutable(owner.receipt_fd, "sql-request.json", custody._sql, 0o400)
                    self.store.retain(owner.receipt_fd, "sql-request.json", maximum=journal.MAX_NATIVE_EXACT)
                    self.imports.append(custody)
                    call._imported = True
                except Exception:
                    pass
        cleanup = "COMPLETE"
        try:
            call.close()
            owner.close()
            self.deadline.remaining()
        except Exception:
            cleanup = "UNKNOWN"
            outcome = "COMMIT_UNKNOWN"
        result = LifecycleResult(outcome, native, readback, artifact, cleanup)
        self.last = result
        if failure is not None and getattr(call, "_cleanup_state", None) == "NOT_STARTED" and cleanup == "COMPLETE":
            raise LifecycleError("LIFECYCLE_PRE_DISPATCH", result=result) from failure
        if isinstance(failure, (KeyboardInterrupt, SystemExit)):
            raise LifecycleError("LIFECYCLE_INTERRUPTED", result=result) from failure
        return result
    def _read_custody(self, custody):
        return self._run(_Call(self, "readback", custody=custody))
    def readback(self, generation, sequence=0):
        require(type(sequence) is int and sequence >= 0, "LIFECYCLE_SEQUENCE")
        custody = self._journal().open_custody(generation, sequence)
        self.imports.append(custody)
        return self._read_custody(custody)
    def _mutate(self, key, purpose, raw, account, epoch, *, generation=None, target=None, selection=None, anchors=()):
        self.revalidate()
        require(type(key) is str and 1 <= len(key) <= 128, "LIFECYCLE_CALLER")
        original = ops._json(raw, MAX)
        require(type(original) is dict, "LIFECYCLE_INPUT")
        store = self._journal(bootstrap=purpose == "CONFIGURE")
        intent = _intent(store.installation_id, self.context.manifest_sha256, purpose, account, epoch, raw,
                         generation=generation, key=key if purpose == "CONFIGURE" else None, target=target)
        operation = store.bind_caller(key, ops._canonical(intent))
        self.operations.append(operation)
        actual = operation._intent
        verb = {"CONFIGURE": "configure", "ENABLE": "enable", "RECOVERY": "recover"}[purpose]
        binding = dict(schema="owner-beta-paper-action-binding-v1", installation_id=store.installation_id,
                       generation=operation.generation, root_intent_sha256=operation.intent_sha256,
                       sequence=0, previous_binding_sha256=None, verb=verb, request_key=actual["request_key"],
                       expected_epoch=actual["expected_epoch"], artifact_sha256=ops._digest(raw), stage_sha256=None)
        action = operation.prepare_action(ops._canonical(binding), raw)
        self.actions.append(action)
        try:
            os.stat("dispatch.json", dir_fd=action.borrow_output_fd(), follow_symlinks=False)
        except FileNotFoundError:
            pass
        else:
            custody = action.import_dispatch()
            self.imports.append(custody)
            return self._read_custody(custody)
        require(store.epoch_floor == epoch, "LIFECYCLE_FLOOR")
        require(not self.store.pending_at_open, "CLEANUP_UNKNOWN")
        result = self._run(_Call(self, verb, action=action, selection=selection, anchors=anchors))
        # Every actual dispatched mutation gets an independent exact readback.
        candidates = [c for c in self.imports if getattr(c, "_action", None) is action]
        if candidates:
            checked = self._read_custody(candidates[-1])
            outcome = result.outcome
            if checked.cleanup != "COMPLETE" or result.cleanup != "COMPLETE":
                outcome = "COMMIT_UNKNOWN"
            elif checked.readback is None or ops._json(checked.readback, 1_048_576)["status"] != "COMMITTED":
                outcome = "COMMIT_UNKNOWN"
            if outcome in ("OK", "REPLAY"):
                read = ops._json(checked.readback, 1_048_576)
                response = ops._json(read["result_canonical"].encode("ascii"), ops.MAX_PAGE)
                require(response.get("outcome") in ("OK", "REPLAY"), "LIFECYCLE_READBACK")
                require(result.artifact is not None and result.native_result is not None, "LIFECYCLE_READBACK_RESULT")
                native = ops._validate_result(result.native_result, verb, 0)
                require(native["event_id"] == read["event_id"], "LIFECYCLE_READBACK_RESULT")
                if verb == "configure":
                    # Frozen CONFIGURE returns H(E), with no generation in its
                    # ResultWire. Its actual SQL response lives in exact readback.
                    require(result.artifact == raw and native["generation"] is None
                            and native["control_epoch"] == read["event_epoch"], "LIFECYCLE_READBACK_RESULT")
                elif verb == "enable":
                    require(result.artifact == read["result_canonical"].encode("ascii")
                            and native["generation"] == operation.generation
                            and native["control_epoch"] == read["event_epoch"], "LIFECYCLE_READBACK_RESULT")
                else:
                    # RECOVER returns the first actual exact-readback document;
                    # later epochs may advance, but the original fact may not.
                    first = _exact_readback(result.artifact, candidates[-1]._sql)
                    require(first["status"] == "COMMITTED" and native["generation"] == operation.generation
                            and native["control_epoch"] == first["current_epoch"]
                            and all(first[k] == read[k] for k in ("account_id", "event_id", "event_epoch",
                                                                "result_canonical", "result_sha256")), "LIFECYCLE_READBACK_RESULT")
                if read["current_epoch"] > store.epoch_floor:
                    store.advance_floor(read["current_epoch"])
            # An uncertain mutation/cleanup is never upgraded by a later receipt.
            self.last = LifecycleResult(outcome, result.native_result, checked.readback, result.artifact,
                                        "COMPLETE" if checked.cleanup == result.cleanup == "COMPLETE" else "UNKNOWN")
        return self.last
    def configure(self, caller_key, configure_bytes):
        value = ops._json(configure_bytes, MAX)
        require(type(value) is dict and set(value) == {"schema", "strategy_config_id", "account_name", "contract"}
                and value["schema"] == "owner-beta-paper-configure-operation-v1", "LIFECYCLE_CONFIGURE")
        # Initial CONFIGURE intentionally requires no process, Current or worker.
        return self._mutate(caller_key, "CONFIGURE", configure_bytes, None, 0)
    def recover(self, caller_key, recovery_bytes):
        value = ops._json(recovery_bytes, MAX)
        require(type(value) is dict and value.get("schema") == "owner-beta-paper-recovery-v2", "LIFECYCLE_RECOVERY")
        store = self._journal()
        require(value.get("installation_id") == store.installation_id, "LIFECYCLE_RECOVERY")
        generation = value.get("generation")
        journal._uuid(generation)
        require(value.get("request_key") == generation + ":recover", "LIFECYCLE_RECOVERY")
        return self._mutate(caller_key, "RECOVERY", recovery_bytes, value.get("account_id"), value.get("active_epoch"),
                            generation=generation, target=value.get("pending_target_id"))
    def _selection(self, *, for_enable=True):
        process, raw = self._process()
        store = self._journal()
        epoch = process["control_epoch"] - 1 if for_enable else store.epoch_floor
        require(store.epoch_floor == epoch, "LIFECYCLE_FLOOR")
        intent = _intent(store.installation_id, self.context.manifest_sha256, "INSPECT", process["account_id"], epoch)
        q = ops._canonical(dict(schema="owner-beta-paper-exact-request-v1", kind="RESET",
                               account_id=process["account_id"], args={"expected_epoch": epoch}))
        result = self._run(_Call(self, "readback", intent=ops._canonical(intent), sql=q))
        require(result.outcome == "OK" and result.cleanup == "COMPLETE" and result.readback is not None, "LIFECYCLE_RESET_UNPROVEN")
        readback = _exact_readback(result.readback, q)
        require(readback["status"] == "RESET", "LIFECYCLE_RESET_UNPROVEN")
        if for_enable:
            require(readback["current_state"] in ("OFF", "RESTORED_OFF")
                    and readback["reset"]["control_epoch"] == epoch, "LIFECYCLE_DRAIN_RESET_REQUIRED")
        selection = dict(schema="owner-beta-paper-reset-selection-v1", installation_id=store.installation_id,
                         reset=readback["reset"], release_manifest_sha256=self.context.manifest_sha256,
                         cohort_sha256=process["daily_cohort_sha256"])
        self.store.publish("selections", ops._canonical(selection))
        return selection, process, raw, readback
    def _anchors(self, selection):
        selected, boots, used = [], set(), 0
        with os.scandir(self.store.dirs["anchors"]) as entries:
            names = [entry.name for entry in itertools.islice(entries, 4097)]
        require(len(names) <= 4096, "LIFECYCLE_INVENTORY_BOUND")
        for name in sorted(names):
            require(len(name) == 69 and name.endswith(".json") and journal.HASH_RE.fullmatch("sha256:" + name[:-5]), "LIFECYCLE_ANCHOR")
            self.deadline.remaining()
            raw = self.store.retain(self.store.dirs["anchors"], name)
            used += len(raw)
            require(used <= 16 * 1024 * 1024, "LIFECYCLE_INVENTORY_BOUND")
            require(ops._digest(raw)[7:] + ".json" == name, "LIFECYCLE_ANCHOR")
            value = ops._json(raw, MAX)
            if all(value.get(k) == v for k, v in selection.items() if k != "schema"):
                value = _anchor(raw, selection)
                require(value["boot_id"] not in boots, "LIFECYCLE_ANCHOR_AMBIGUOUS")
                boots.add(value["boot_id"])
                selected.append(raw)
        return tuple(selected)
    def observe_reset(self):
        require(not self.store.pending_at_open, "CLEANUP_UNKNOWN")
        selection, process, raw, readback = self._selection(for_enable=False)
        require(readback["current_state"] in ("OFF", "RESTORED_OFF", "ENABLED"), "LIFECYCLE_DRAIN_RESET_REQUIRED")
        boot = Path("/proc/sys/kernel/random/boot_id").read_text().strip()
        for original in self._anchors(selection):
            if _anchor(original, selection)["boot_id"] == boot:
                # Original survives boot/session restarts; no retrospective stamp.
                self.last = LifecycleResult("REPLAY", None, None, original, "COMPLETE")
                return self.last
        require(readback["current_state"] in ("OFF", "RESTORED_OFF"), "LIFECYCLE_DRAIN_RESET_REQUIRED")
        intent = _intent(selection["installation_id"], self.context.manifest_sha256, "INSPECT",
                         process["account_id"], readback["current_epoch"], raw)
        result = self._run(_Call(self, "observe-reset", intent=ops._canonical(intent), evidence=raw, selection=selection))
        if result.outcome == "OK" and result.cleanup == "COMPLETE" and result.artifact is not None:
            value = _anchor(result.artifact, selection)
            require(value["boot_id"] == boot, "LIFECYCLE_ANCHOR_BOOT")
            self.revalidate()
            self.pending_anchors.append(result.artifact)
            self.revalidate()
        return result
    def _prior_enable(self, caller_key):
        """Resolve a caller's original dispatch before Current/RESET eligibility."""
        require(type(caller_key) is str and 1 <= len(caller_key) <= 128, "LIFECYCLE_CALLER")
        store = self._journal()
        store._scan_callers()
        if store._callers is None:
            return None
        names = os.listdir(store._callers)
        require(len(names) <= 4096, "LIFECYCLE_INVENTORY_BOUND")
        matches = []
        for name in names:
            binding = journal._validate_caller(journal._read_named(store._callers, name, 0o400, MAX), store.installation_id)
            if binding["caller_key"] == caller_key:
                require(name == journal._caller_name(store.installation_id, binding["account_id"], caller_key), "LIFECYCLE_CALLER")
                matches.append(binding)
        require(len(matches) <= 1, "LIFECYCLE_CALLER_AMBIGUOUS")
        if not matches:
            return None
        binding = matches[0]
        operation = store.open(binding["generation"])
        self.operations.append(operation)
        require(operation.intent_sha256 == binding["root_intent_sha256"] and operation._intent["purpose"] == "ENABLE",
                "LIFECYCLE_CALLER_CONFLICT")
        # A recorded root/action without dispatch is still not permission to
        # select a new epoch for the same K. Its original inputs must be used.
        action = operation.open_action(0)
        self.actions.append(action)
        try:
            os.stat("dispatch.json", dir_fd=action.borrow_output_fd(), follow_symlinks=False)
        except FileNotFoundError:
            action.close()
            self.actions.remove(action)
            operation.close()
            self.operations.remove(operation)
            return None
        custody = action.import_dispatch()
        self.imports.append(custody)
        return self._read_custody(custody)
    def enable(self, caller_key):
        prior = self._prior_enable(caller_key)
        if prior is not None:
            return prior
        selection, process, raw, _readback = self._selection()
        anchors = self._anchors(selection)
        require(anchors, "LIFECYCLE_ANCHOR_REQUIRED")
        return self._mutate(caller_key, "ENABLE", raw, process["account_id"], selection["reset"]["control_epoch"],
                            selection=selection, anchors=anchors)
    def close(self):
        if self.closed:
            return
        self.revalidate()
        require(all(c.call._closed and c.closed for c in self.calls), "CLEANUP_UNKNOWN")
        for custody in reversed(self.imports):
            custody.close()
        for action in reversed(self.actions):
            action.close()
        for operation in reversed(self.operations):
            operation.close()
        # All native children, imports and persistent stages finish before an
        # anchor becomes visible. The caller's inherited release lock is held.
        if self.journal_manager is not None:
            self.journal_manager.__exit__(None, None, None)
        self.context.close()
        self.store.revalidate()
        self.deadline.remaining()
        published = []
        anchor_fd = os.dup(self.store.dirs["anchors"]) if self.pending_anchors else None
        try:
            for raw in self.pending_anchors:
                name = ops._digest(raw)[7:] + ".json"
                try:
                    os.stat(name, dir_fd=anchor_fd, follow_symlinks=False)
                except FileNotFoundError:
                    # Register rollback before publication; fsync can fail after
                    # the immutable name appears. Existing anchors are untouched.
                    published.append((name, raw))
                self.store.publish("anchors", raw)
            self.store.close()
            self.deadline.remaining()
        except BaseException:
            for name, raw in published:
                try:
                    actual = journal._read_named(anchor_fd, name, 0o400, MAX)
                    require(actual == raw, "LIFECYCLE_ANCHOR_CHANGED")
                    os.unlink(name, dir_fd=anchor_fd)
                    os.fsync(anchor_fd)
                except FileNotFoundError:
                    pass
            raise
        finally:
            if anchor_fd is not None:
                os.close(anchor_fd)
        self.closed = True


@contextlib.contextmanager
def open_lifecycle(context, release_lock_fd, *, deadline):
    """deadline: built-in int, absolute time.monotonic_ns(); no clock duck typing."""
    owner = Lifecycle(context, release_lock_fd, deadline)
    try:
        yield owner
    finally:
        try:
            owner.close()
        except Exception as error:
            if owner not in _UNRESOLVED:
                _UNRESOLVED.append(owner)
            result = owner.last
            if result is not None:
                result = dataclasses.replace(result, outcome="COMMIT_UNKNOWN", cleanup="UNKNOWN")
            raise LifecycleError("CLEANUP_UNKNOWN", result=result) from error


@contextlib.contextmanager
def _locked_context(install_root, release_root, commit):
    daily.install.require_root()
    tree = journal._Tree(install_root)
    lock = None
    try:
        lock = os.open(ops.LOCK_NAME, os.O_RDWR | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=tree.fd)
        ops._file(lock, 0o600)
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        journal._check_lock(install_root, lock, tree)
        daily.policy.validate_current_release(install_root, release_root, commit)
        values, overlays = daily.read_context(release_root, commit, "disable")
        context = daily.Context(release_root, commit, daily.read_manifest(release_root, commit), values, overlays)
        yield context, lock
        journal._check_lock(install_root, lock, tree)
    finally:
        if lock is not None:
            os.close(lock)
        tree.close()


def source_barrier(binding_path, *, refresh=None):
    """Called after Current starts and before Source claims/ARM_INTENT, each day."""
    evidence = _load("owner_beta_lifecycle_evidence", "owner-beta-paper-evidence.py")
    b, original = evidence.binding(binding_path)
    release_root, commit = b["release_root"], b["cohort"]["verifier_revision"]
    install_root = str(Path(release_root).parent.parent)
    deadline = _Deadline(time.monotonic_ns() + 90_000_000_000)
    deadline.tick = refresh
    with _locked_context(install_root, release_root, commit) as (context, lock):
        # A legacy Daily installation has no prepared Paper release. Query the
        # actual DB inventory with its existing OFF reader before requiring one.
        with ops._PaperContext(ops._CONTEXT_TOKEN, context, lock, live=False, deadline=deadline) as reader:
            reader._lifecycle_stage_root = install_root + "/" + STORE + "/staging"
            with daily.install.root_directory(reader._lifecycle_stage_root) as stage_fd:
                require(not os.listdir(stage_fd), "CLEANUP_UNKNOWN")
            with ops._inspect_generation_null(reader) as call:
                _index, _pages, _history, configured = ops._read_inventory(call)
                call._imported = True
        if not configured:
            require(evidence.binding(binding_path)[1] == original, "LIFECYCLE_CURRENT_CHANGED")
            deadline.remaining()
            return None
        with open_lifecycle(context, lock, deadline=deadline.end) as owner:
            owner.deadline.tick = refresh
            owner.deadline.remaining()
            result = owner.observe_reset()
            require(result.outcome in ("OK", "REPLAY") and result.cleanup == "COMPLETE"
                    and result.artifact is not None, "LIFECYCLE_ANCHOR_REQUIRED")
            require(evidence.binding(binding_path)[1] == original, "LIFECYCLE_CURRENT_CHANGED")
        return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("configure", "observe-reset", "enable", "recover", "readback"))
    parser.add_argument("--install-root", required=True)
    parser.add_argument("--caller-key")
    parser.add_argument("--proposal")
    parser.add_argument("--generation")
    parser.add_argument("--sequence", type=int, default=0)
    args = parser.parse_args(argv)
    release_root = str(Path(__file__).resolve().parents[3])
    commit = Path(release_root).name
    require(daily.install.is_commit(commit), "LIFECYCLE_RELEASE")
    with _locked_context(args.install_root, release_root, commit) as (context, lock):
        with contextlib.ExitStack() as stack:
            raw = None
            if args.command in ("configure", "recover"):
                require(args.proposal is not None and args.caller_key is not None, "LIFECYCLE_ARGUMENTS")
                parent, leaf = os.path.split(args.proposal)
                fd = stack.enter_context(daily.install.root_directory(parent))
                proposal = _Original(fd, leaf)
                stack.callback(proposal.close)
                raw = proposal.raw
            else:
                require(args.proposal is None, "LIFECYCLE_ARGUMENTS")
            with open_lifecycle(context, lock, deadline=time.monotonic_ns() + 90_000_000_000) as owner:
                if args.command == "configure": result = owner.configure(args.caller_key, raw)
                elif args.command == "recover": result = owner.recover(args.caller_key, raw)
                elif args.command == "enable": result = owner.enable(args.caller_key)
                elif args.command == "observe-reset": result = owner.observe_reset()
                else: result = owner.readback(args.generation, args.sequence)
    # Only finite descriptive state is printed; original private bytes stay local.
    print("PAPER_LIFECYCLE_" + result.outcome + ":" + result.cleanup)
    return {"OK": 0, "REPLAY": 0, "BLOCKED": 3, "COMMIT_UNKNOWN": 4}[result.outcome]

if __name__ == "__main__":
    sys.modules["owner_beta_paper_lifecycle"] = sys.modules[__name__]
    try:
        raise SystemExit(main())
    except LifecycleError as error:
        print(error.code, file=sys.stderr)
        raise SystemExit(4 if error.code == "CLEANUP_UNKNOWN" else 3)
    except Exception:
        print("PAPER_LIFECYCLE_REJECTED", file=sys.stderr)
        raise SystemExit(3)
