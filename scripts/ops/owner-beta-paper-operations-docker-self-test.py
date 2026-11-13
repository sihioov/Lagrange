#!/usr/bin/env python3
"""Actual selected-image BEGIN/drain/RECONCILE/OFF and exact-readback driver.

The hosted fixture owner prepares a disposable install tree, its real selected
paper-scheduler image and producer receipts, a root-owned journal action, and a
disposable PostgreSQL server. This file accepts those in-process objects only;
it has no CLI path or environment-controlled install/database selector.
"""
from __future__ import annotations

import contextlib
import importlib.util
import json
import os
import pathlib
import sys
import time

sys.dont_write_bytecode = True
ROOT = pathlib.Path(__file__).resolve().parents[2]
OPS_PATH = ROOT / "scripts/ops/lib/owner-beta-paper-operations.py"
spec = importlib.util.spec_from_file_location("paper_operations_fixture_driver_ops", OPS_PATH)
ops = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = ops
spec.loader.exec_module(ops)


def run_genuine_fixture(context, release_lock_fd, action_custody, *, observer=None):
    """Run BEGIN/readback, FENCED/drain, RECONCILE/OFF and no-submission replay.

    `context` and `action_custody` must come from the existing disposable
    hosted Root/PG fixture setup. The installed image, R/A originals, source
    capability, producer receipts and Docker lifecycle are validated by the
    production controller itself. No extracted ELF or injected Docker output
    can satisfy this path.
    """
    ops._root()
    install_root = str(pathlib.Path(context.release).parent.parent)
    _require(install_root.startswith("/var/lib/lagrange-paper-operations-fixture-"),
             "PAPER_DOCKER_FIXTURE_ROOT")
    _require(type(action_custody) is ops.journal.ActionCustody,
             "PAPER_DOCKER_FIXTURE_ACTION")
    _require(action_custody._operation._intent["purpose"] in {"STOP", "BACKUP"},
             "PAPER_DOCKER_FIXTURE_ACTION")
    operation = action_custody._operation
    generation = operation.generation
    sequence = action_custody.sequence

    capture_receipt = None
    with ops.open_context(context, release_lock_fd) as live:
        _require(live.project.startswith("paper-operations-fixture-"),
                 "PAPER_DOCKER_FIXTURE_PROJECT")
        _require(live.verified is not None and live.verified.producer_receipts_sha256
                 and live.verified.capability_sha256, "PAPER_DOCKER_FIXTURE_PRODUCER")

        if observer is not None:
            observer("first", live, None)

        # This probe must report the fixture's configured OFF state before BEGIN.
        with live.probe_history() as inventory:
            _require(inventory._configured is True and
                     ops._json(inventory._index, ops.MAX_INDEX)["state"] == "OFF",
                     "PAPER_DOCKER_FIXTURE_OFF")

        capture_space = contextlib.ExitStack()
        if operation._intent["purpose"] == "BACKUP":
            paper = _paper_backup()
            capture_space.enter_context(paper.reserve_capture(live))
        original_dispatch = None
        original_result = None
        first_identity = None
        imported_custody = None
        original_invocation = None
        try:
            original_invocation = live.invoke("begin", action_custody)
            if observer is not None:
                observer("first", original_invocation, None)
            with original_invocation:
                result = original_invocation.result
                _require(result["outcome"] == "OK" and result["event_id"] is not None
                         and result["control_epoch"] is not None,
                         "PAPER_DOCKER_FIXTURE_BEGIN")
                imported_custody = original_invocation.import_dispatch()
                original_dispatch = action_custody._dispatch_bytes
                original_result = original_invocation._result_bytes
                first_identity = (result["event_id"], result["control_epoch"],
                                  result["artifact_sha256"])
                _require(type(original_result) is bytes
                         and result["artifact_sha256"] == ops._digest(original_result),
                         "PAPER_DOCKER_FIXTURE_RESULT_ARTIFACT")
                if observer is not None:
                    observer("import", original_invocation, dict(result))
                imported_custody.close()
                imported_custody = None
            _require(original_dispatch is not None and first_identity is not None,
                     "PAPER_DOCKER_FIXTURE_IMPORT")

            # The native duplicate branch must return COMMIT_UNKNOWN without a
            # second SQL submission. Exact dispatch bytes are imported again.
            duplicate_action = operation.open_action(0)
            try:
                with live.invoke("begin", duplicate_action) as duplicate:
                    _require(duplicate.result["outcome"] == "COMMIT_UNKNOWN"
                             and duplicate.result["event_id"] is None
                             and duplicate.result["artifact_sha256"] is None,
                             "PAPER_DOCKER_FIXTURE_DUPLICATE")
                    duplicate_custody = duplicate.import_dispatch()
                    duplicate_custody.close()
                    _require(duplicate_action._dispatch_bytes == original_dispatch,
                             "PAPER_DOCKER_FIXTURE_DUPLICATE_IMPORT")
                    if observer is not None:
                        observer("duplicate", duplicate, duplicate.result)
            finally:
                duplicate_action.close()

            historical = operation._journal.open_custody(generation, sequence)
            try:
                with ops.open_custody_context(context, release_lock_fd) as readback_context:
                    if observer is not None:
                        observer("first", readback_context, None)
                    with readback_context.invoke("readback", historical) as readback:
                        _require(readback.result["outcome"] == "REPLAY"
                                 and readback.result["event_id"] == first_identity[0]
                                 and readback.result["generation"] == generation
                                 and readback.result["control_epoch"] == first_identity[1],
                                 "PAPER_DOCKER_FIXTURE_READBACK")
                        artifact = readback.readback_artifact()
                        readback_value = ops._json(artifact, ops.MAX_PAGE)
                        _require(readback.result["artifact_sha256"] == ops._digest(artifact)
                                 and type(readback_value) is dict
                                 and readback_value.get("schema") ==
                                 "owner-beta-paper-exact-readback-v1"
                                 and readback_value.get("kind") == "BEGIN"
                                 and readback_value.get("status") == "COMMITTED"
                                 and readback_value.get("event_id") == first_identity[0]
                                 and readback_value.get("event_epoch") == first_identity[1]
                                 and readback_value.get("result_canonical") ==
                                 original_result.decode("ascii")
                                 and readback_value.get("result_sha256") ==
                                 ops._digest(original_result),
                                 "PAPER_DOCKER_FIXTURE_READBACK_ARTIFACT")
                        if observer is not None:
                            observer("readback", readback, readback.result)
            finally:
                historical.close()

            fenced_digest = live.fence_begin(original_invocation)
            fenced_stage = operation.revalidate()
            _require(operation._last is not None, "PAPER_DOCKER_FIXTURE_FENCED")
            stage_value = operation._last[3]
            intent_stage_bytes = ops.journal._read_named(
                operation._fd, "00000000.json", 0o400, ops.journal.MAX_SMALL)
            intent_stage = ops.journal._json(intent_stage_bytes, ops.journal.MAX_SMALL)
            _require(operation._last[0] == 1 and operation._last[1] == fenced_digest
                     and fenced_stage == "FENCED"
                     and set(stage_value) == ops.journal.STAGE_KEYS
                     and stage_value["sequence"] == 1
                     and stage_value["previous_sha256"] == action_custody._action["stage_sha256"]
                     and stage_value["intent_sha256"] == operation.intent_sha256
                     and stage_value["stage"] == "FENCED"
                     and stage_value["installation_id"] == operation._intent["installation_id"]
                     and stage_value["generation"] == generation
                     and stage_value["account_id"] == operation._intent["account_id"]
                     and stage_value["boot_id"] == intent_stage["boot_id"]
                     and stage_value["artifact_sha256"] == first_identity[2]
                     and stage_value["event_id"] == first_identity[0]
                     and stage_value["control_epoch"] == first_identity[1]
                     and stage_value["error_code"] == "NONE"
                     and operation.read_original(first_identity[2]) == original_result
                     and operation._journal._read_floor() == first_identity[1],
                     "PAPER_DOCKER_FIXTURE_FENCED")
            fenced_bytes = ops.journal._read_named(operation._fd, "00000001.json", 0o400,
                                                   ops.journal.MAX_SMALL)
            _require(live.fence_begin(original_invocation) == fenced_digest
                     and ops.journal._read_named(operation._fd, "00000001.json", 0o400,
                                                 ops.journal.MAX_SMALL) == fenced_bytes,
                     "PAPER_DOCKER_FIXTURE_FENCED_REPLAY")
            if observer is not None:
                observer("stop-ready", live, None)
            drained = live.drain(original_invocation)
            try:
                # One full stop observation only (the idempotent drain below
                # revalidates); inventory1 must stay young for reconcile:1.
                inventory = ops._json(drained._attempt.inventory_bytes, ops.MAX_RESULT)
                _require(operation._last[2] == "STOPPED" and operation._last[0] == 2
                         and operation._last[3]["previous_sha256"] == fenced_digest
                         and operation._last[3]["artifact_sha256"] == ops._digest(drained._attempt.inventory_bytes)
                         and inventory["fence_event_id"] == first_identity[0]
                         and inventory["control_epoch"] == first_identity[1]
                         and inventory["generation"] == generation
                         and live.drain(original_invocation) is drained,
                         "PAPER_DOCKER_FIXTURE_FENCED")
                if observer is not None:
                    observer("stopped", drained, inventory)
                off_event = ops.commit_off(drained)
                # INTENT, FENCED, STOPPED, RECONCILED, refreshed RECONCILED, OFF_COMMITTED.
                refreshed = ops.journal._read_named(operation._fd, "00000004.json", 0o400,
                                                    ops.journal.MAX_SMALL)
                receipt = ops._json(drained._attempt.consumer.actions[-1]._artifact_bytes,
                                    ops.MAX_RESULT)
                _require(type(off_event) is str and ops.journal.UUID_RE.fullmatch(off_event)
                         and operation._last[2] == "OFF_COMMITTED"
                         and operation._last[0] == 5
                         and operation._last[3]["event_id"] == off_event
                         and operation._last[3]["control_epoch"] == first_identity[1] + 1
                         and operation._journal._read_floor() == first_identity[1] + 1
                         and ops._json(refreshed, ops.MAX_RESULT)["stage"] == "RECONCILED"
                         and receipt["stage_sha256"] == ops._digest(refreshed)
                         and receipt["inventory_sha256"] != ops._digest(drained._attempt.inventory_bytes),
                         "PAPER_DOCKER_FIXTURE_FENCED")
                if observer is not None:
                    observer("off", drained, off_event)
                before = ops.journal._read_named(operation._fd, "00000005.json", 0o400,
                                                 ops.journal.MAX_SMALL)
                _require(ops.commit_off(drained) == off_event
                         and ops.journal._read_named(operation._fd, "00000005.json", 0o400,
                                                     ops.journal.MAX_SMALL) == before,
                         "PAPER_DOCKER_FIXTURE_FENCED_REPLAY")
                if observer is not None:
                    observer("off-replay", drained, off_event)
                if operation._intent["purpose"] == "BACKUP":
                    capture_receipt = run_capture_prefix(live, drained)
            finally:
                drained.close()
            try:
                drained.revalidate()
            except ops.PaperOperationsError as error:
                _require(error.code == "PAPER_OPS_CLOSED", "PAPER_DOCKER_FIXTURE_FENCED")
            else:
                _require(False, "PAPER_DOCKER_FIXTURE_FENCED")
            if observer is not None:
                # The installed fixture observer has a closed phase vocabulary.
                # Recheck its no-write counters after both continuation reads.
                observer("readback", readback, readback.result)
        finally:
            try:
                if imported_custody is not None:
                    imported_custody.close()
            finally:
                if not action_custody._closed:
                    action_custody.close()
                capture_space.close()
    return {"status": "PASS", "capture": capture_receipt, "cases": ("BEGIN", "DUPLICATE_NO_RESUBMISSION",
                                         "EXACT_IMPORT", "EXACT_READBACK",
                                         "FENCED_BEGIN_READBACK", "FENCED_IDEMPOTENT_REPLAY",
                                         "STOPPED_NEGATIVE_PROOF", "DRAINED_HANDLE_LIFETIME",
                                         "RECONCILED_ORIGINAL", "OFF_COMMITTED_READBACK",
                                         "OFF_REPLAY_ZERO_SQL",
                                         "CONFIGURED_OFF_PROBE")}


def _paper_backup():
    module = sys.modules.get("owner_beta_paper_backup")
    path = OPS_PATH.with_name("owner-beta-paper-backup.py")
    if module is None:
        selected = importlib.util.spec_from_file_location("owner_beta_paper_backup", path)
        module = importlib.util.module_from_spec(selected)
        sys.modules[selected.name] = module
        selected.loader.exec_module(module)
    _require(pathlib.Path(module.__file__).resolve() == path.resolve(), "PAPER_DOCKER_FIXTURE_PRODUCER")
    return module


def run_capture_prefix(live, drained):
    """G2's synthetic control fixture proves held RR audit/dump custody.

    Its concrete input is a configured OFF control and completed combined OFF
    operation; it has no approved historical/Daily/R4 material. Consume the real
    held source stage directly instead of expecting a now-obsolete incomplete
    archive guard. This receipt never claims file closure or archive acceptance.
    """
    paper = _paper_backup()
    owner = paper.open_snapshot(live, drained, 2 * 1024 * 1024 * 1024)
    try:
        with owner:
            _require(owner._snapshot is not None and owner.exporter.backend is not None,
                     "PAPER_DOCKER_FIXTURE_PRODUCER")
            owner._dump()  # imports the exact live export on the retained exporter
            owner._check_spools()
            _require(set(owner.spools) == {"audit.stream", "postgres.dump"}
                     and len(owner.description["groups"]) == 9, "PAPER_DOCKER_FIXTURE_PRODUCER")
            fd, _identity, size, digest = owner.spools["postgres.dump"]
            _require(size > 5 and os.pread(fd, 5, 0) == b"PGDMP"
                     and owner.ops._hash_fd(fd) == (size, digest), "PAPER_DOCKER_FIXTURE_PRODUCER")
            receipt = {"scope": "HELD_SOURCE_RR_AUDIT_AND_DUMP", "audit_groups": 9,
                       "audit_sha256": owner.spools["audit.stream"][3], "dump_sha256": digest,
                       "archive_acceptance": False, "synthetic": True}
            owner.release_capture_clients()
            owner.revalidate()
    finally:
        # Failed cleanup must retain the actual registered owner, not signal
        # success because an expected capture exception was observed.
        if owner._closed:
            _require(owner._resources_closed(), "PAPER_DOCKER_FIXTURE_PRODUCER")
    _require(owner._closed and owner._snapshot is None, "PAPER_DOCKER_FIXTURE_PRODUCER")
    return receipt


def run_restore_no_sql_rejection(context, release_lock_fd, restore_action_custody, *, observer=None):
    """Reject RESTORE before constructing any helper invocation.

    The hosted Root/PG fixture should also compare its disposable database's
    BEGIN-call/event counts around this call; the controller path itself creates
    no helper and has no SQL transport entry point after the rejection.
    """
    ops._root()
    install_root = str(pathlib.Path(context.release).parent.parent)
    _require(install_root.startswith("/var/lib/lagrange-paper-operations-fixture-"),
             "PAPER_DOCKER_FIXTURE_ROOT")
    _require(type(restore_action_custody) is ops.journal.ActionCustody
             and restore_action_custody._operation._intent["purpose"] == "RESTORE",
             "PAPER_DOCKER_FIXTURE_RESTORE")
    with ops.open_context(context, release_lock_fd) as live:
        _require(live.project.startswith("paper-operations-fixture-"),
                 "PAPER_DOCKER_FIXTURE_PROJECT")
        if observer is not None:
            observer("first", live, None)
        try:
            live.invoke("begin", restore_action_custody)
        except ops.PaperOperationsError as error:
            _require(error.code == "PAPER_OPS_ACTION", "PAPER_DOCKER_FIXTURE_RESTORE")
        else:
            _require(False, "PAPER_DOCKER_FIXTURE_RESTORE")
        _require(not live._invocations, "PAPER_DOCKER_FIXTURE_RESTORE_HELPER_CREATED")
    return {"status": "PASS", "cases": ("RESTORE_REJECTED_BEFORE_HELPER_OR_SQL",)}


def _require(condition, code):
    if not condition:
        raise ops.PaperOperationsError(code)



class IntakeCancellation:
    """Fixture-only supervision of one actual destination intake and its peer.

    Production custody stays unresolved after killing an unimported helper.
    These observations dispose resources; they confer no production authority.
    """
    MIGRATION_QUERY = "SELECT checksum FROM public._sqlx_migrations WHERE version=69 AND success"

    def __init__(self, acceptance, package):
        self.acceptance, self.package = acceptance, package
        self.faults, self.p = acceptance.faults, acceptance.paper
        self.blocker = self.probe = self.call = None
        self.binding = self.helper = self.backend = self.peer = None
        self.correlated = self.disposed = self.clients_absent = False
        self.fixture_disposed = self.fixture_handles_closed = False
        self.deadline = package.owner._custody.deadline

    def bound(self, cap=1, *, cleanup=False):
        outer = self.acceptance.owner.deadline.seconds(cap, cleanup=cleanup)
        inner = (self.deadline.remaining(cap) if cleanup else self.deadline.work_remaining(cap))
        return min(outer, inner)

    def query(self, sql):
        self.bound()
        self.probe.check()
        # Each observation must see current backend state, not a cached RR view.
        return json.loads(self.probe.exchange(('ROLLBACK; BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY; '
                                               + sql + ';\n').encode('ascii')))

    def arm(self):
        package = self.package
        self.bound()
        try:
            self.probe = self.p._GuestClient(package, package.guest.admin)
            self.probe.start()
            self.blocker = self.p._GuestClient(package, package.guest.admin)
            self.blocker.start()
            # ACCESS EXCLUSIVE conflicts with the actual audit/migration SELECT.
            # A row lock would not block this native read-only transaction.
            self.blocker.exchange(b'ROLLBACK; BEGIN READ WRITE; '
                b'LOCK TABLE public._sqlx_migrations IN ACCESS EXCLUSIVE MODE;\n')
            self.blocker.check()
            self.probe.check()
            held = self.query("SELECT count(*) FROM pg_locks WHERE pid=%d AND granted "
                "AND relation='public._sqlx_migrations'::regclass AND mode='AccessExclusiveLock'"
                % self.blocker.backend['pid'])
            _require(held == 1, 'PAPER_DOCKER_FIXTURE_INTAKE_LOCK')
        except BaseException:
            self.close_clients()
            raise

    def inputs(self, call):
        package, destination = self.package, self.package.destination
        _require(type(call) is package.ops._NativeInvocation and call is package.call
            and call._archive_owner is package and call._restore_owner is destination
            and call._context is package.owner.context and call in call._context._invocations
            and call.verb == 'begin' and call._archive_restore_begin()
            and call.custody is destination.active_custody
            and destination.actions == [call.custody]
            and call.custody._operation is destination.operation
            and destination.operation._intent['purpose'] == 'RESTORE'
            and call.custody._action['verb'] == 'begin'
            and not call._imported, 'PAPER_DOCKER_FIXTURE_INTAKE_BINDING')
        call.custody.revalidate()
        return (call.custody, destination.operation, destination.operation._intent_bytes,
                call.custody._binding_bytes, tuple(
                    (original, original.fd, original.identity, original.size, original.sha256)
                    for original in package.originals))

    def helper_identity(self):
        call, f, o = self.call, self.faults, self.package.ops
        value = call._context.docker.object('container', call._cid)
        call._archive_observed_container(value)  # native policy, mount and binding checks
        state = value.get('State') or {}
        _require(state.get('Running') is True and type(state.get('Pid')) is int
            and state['Pid'] > 1, 'PAPER_DOCKER_FIXTURE_INTAKE_HELPER')
        process = o._process_record(state['Pid'])
        _require(call._cid in process['cgroup']
            and os.readlink('/proc/%d/exe' % process['pid']).endswith('/owner-beta-paper-ops-native-bin'),
            'PAPER_DOCKER_FIXTURE_INTAKE_HELPER')
        return (o._container_generation(value), process, f.namespace_identity(process['pid'], 'net'),
                f.namespace_identity(process['pid'], 'pid'), f.unix_socket_inodes(process['pid']))

    def activity(self):
        return self.query("SELECT COALESCE(jsonb_agg(jsonb_build_object('pid',pid,"
            "'start_us',(extract(epoch FROM backend_start)*1000000)::bigint,"
            "'active',state='active' AND xact_start IS NOT NULL AND backend_xid IS NULL,"
            "'audit',query=" + self.p._sql_literal(self.MIGRATION_QUERY) + ","
            "'locked',wait_event_type='Lock' AND %d=ANY(pg_blocking_pids(pid)),"
            "'age_ms',ceil(extract(epoch FROM clock_timestamp()-query_start)*1000)::bigint) ORDER BY pid),"
            "'[]'::jsonb) FROM pg_stat_activity WHERE datname=current_database() "
            "AND backend_type='client backend'" % self.blocker.backend['pid'])

    def backend_candidates(self, rows):
        package, f, o = self.package, self.faults, self.package.ops
        guest = package.guest
        guest.check(ready=True)
        expected = {(client.backend['pid'], client.backend['start_us'])
                    for client in (self.blocker, self.probe)}
        actual = {(r['pid'], r['start_us']) for r in rows}
        _require(len(actual) == len(rows) and expected <= actual,
                 'PAPER_DOCKER_FIXTURE_INTAKE_CLIENTS')
        foreign = [r for r in rows if (r['pid'], r['start_us']) not in expected]
        _require(len(foreign) == 1, 'PAPER_DOCKER_FIXTURE_INTAKE_BACKEND')
        row = foreign[0]
        _require(row['active'] is True and row['audit'] is True and row['locked'] is True
            and type(row['age_ms']) is int and 0 <= row['age_ms'] < 5000,
            'PAPER_DOCKER_FIXTURE_INTAKE_BACKEND')
        candidates = []
        # Resolve SQL PID in the guest PID namespace, then check actual parent,
        # executable, cgroup, namespace and generation. Roles never assign ownership.
        for pid, group in o._proc_cgroups().items():
            if group != guest.process['cgroup']:
                continue
            try:
                status = pathlib.Path('/proc/%d/status' % pid).read_text()
                values = next(v for v in status.splitlines() if v.startswith('NSpid:')).split()[1:]
                if int(values[-1]) != row['pid']:
                    continue
                process = o._process_record(pid)
                _require(int(values[0]) == pid and process['parent'] == guest.process['pid']
                    and process['executable'] == guest.process['executable']
                    and f.namespace_identity(pid, 'pid') == f.namespace_identity(guest.process['pid'], 'pid'),
                    'PAPER_DOCKER_FIXTURE_INTAKE_BACKEND')
                sockets = f.unix_socket_inodes(pid)
                _require(o._process_record(pid) == process, 'PAPER_DOCKER_FIXTURE_INTAKE_RACE')
                candidates.append((row, process, sockets))
            except FileNotFoundError:
                raise self.faults.FaultError('FAULT_DIAG_CHANGED') from None
        _require(len(candidates) == 1, 'PAPER_DOCKER_FIXTURE_INTAKE_BACKEND')
        return candidates

    def observe(self, call, frame):
        _require(frame.f_locals.get('child') is call._child and call._child is not None
            and call._child.poll() is None, 'PAPER_DOCKER_FIXTURE_INTAKE_ATTACH')
        self.call, self.binding = call, self.inputs(call)
        until = time.monotonic() + self.bound(15)
        try:
            # The real attached child may still be starting. Do not manufacture
            # a ready marker or replace _read_attached to get this opportunity.
            while True:
                self.bound()
                value = call._context.docker.object('container', call._cid)
                state = value.get('State') or {}
                _require(value.get('Created') == call._created, 'PAPER_DOCKER_FIXTURE_INTAKE_RACE')
                if state.get('Running') and state.get('Pid', 0) > 1:
                    rows = self.activity()
                    identities = {(c.backend['pid'], c.backend['start_us']) for c in (self.blocker, self.probe)}
                    if any((r['pid'], r['start_us']) not in identities and r['locked'] is True for r in rows):
                        break
                _require(state.get('Status') in {'created', 'running'} and time.monotonic() < until,
                         'PAPER_DOCKER_FIXTURE_INTAKE_TIMEOUT')
                time.sleep(min(0.01, self.bound(0.01)))
            self.helper = self.helper_identity()
            generation, process, _net, _pidns, _sockets = self.helper
            # Stay inside both the native 5s lock window and the inherited clocks.
            left = min(self.bound(2), (5000 - max(r['age_ms'] or 0 for r in rows if r['locked'])) / 1000)
            _require(left > 0, 'PAPER_DOCKER_FIXTURE_INTAKE_TIMEOUT')
            end = time.monotonic_ns() + int(left * 1e9)
            peers = self.faults.unix_peers(process['pid'], process['start_ticks'], call._cid, end)
            first = self.faults.unique_unix_backend(peers, self.backend_candidates(self.activity()))
            second = self.faults.unique_unix_backend(peers, self.backend_candidates(self.activity()))
            # age_ms changes; backend PID/start, actual host process and sockets must not.
            _require(first[:2] == second[:2] and first[2]['pid'] == second[2]['pid']
                and first[2]['start_us'] == second[2]['start_us'] and first[3] == second[3]
                and self.helper_identity() == self.helper and self.inputs(call) == self.binding,
                'PAPER_DOCKER_FIXTURE_INTAKE_RACE')
            self.peer, self.backend = first[:2], (first[2]['pid'], first[2]['start_us'], first[3])
            self.correlated = True
            self.no_dispatch()
            self.bound()
            # Exact owned helper termination is the fault. Let the real attached
            # reader consume EOF and production retain its unimported uncertainty.
            self.package.owner._cleanup_docker(['kill', '--signal=KILL', call._cid])
            self.acceptance.fired = True
            call._child.wait(timeout=self.bound(2, cleanup=True))
            while True:
                value = call._context.docker.object('container', call._cid)
                if value.get('State', {}).get('Running') is False:
                    break
                self.bound(0.01, cleanup=True)
                time.sleep(0.01)
            terminal = self.package.ops._terminal_container(value, generation)
            _require(self.package.ops._terminal_container(call._context.docker.object('container', call._cid),
                generation) == terminal, 'PAPER_DOCKER_FIXTURE_INTAKE_DISPOSITION')
            self.prove_disposition()
            self.no_dispatch()
            self.disposed = True
        finally:
            # If correlation/parser/deadline failed, prevent the independently
            # running owned helper from continuing after release of our lock.
            # Failure to establish exact ownership/disposal keeps UNKNOWN.
            try:
                if not self.disposed:
                    value = call._context.docker.object('container', call._cid)
                    call._archive_observed_container(value)
                    if self.helper is not None:
                        _require(self.package.ops._container_generation(value) == self.helper[0],
                                 'PAPER_DOCKER_FIXTURE_INTAKE_RACE')
                    if value.get('State', {}).get('Running'):
                        self.bound(1, cleanup=True)
                        self.package.owner._cleanup_docker(['kill', '--signal=KILL', call._cid])
            finally:
                self.close_clients()

    def no_dispatch(self):
        _require(self.inputs(self.call) == self.binding, 'PAPER_DOCKER_FIXTURE_INTAKE_BINDING')
        for action in self.package.destination.actions:
            _require(action._dispatch_bytes is None, 'PAPER_DOCKER_FIXTURE_PREINTAKE_DISPATCH')
            try:
                os.stat('dispatch.json', dir_fd=action._output_fd, follow_symlinks=False)
            except FileNotFoundError:
                pass
            else:
                _require(False, 'PAPER_DOCKER_FIXTURE_PREINTAKE_DISPATCH')
        generation = self.package.destination.operation.generation
        _require(self.query("SELECT (SELECT count(*) FROM public.owner_beta_paper_operations WHERE generation='"
            + generation + "') + (SELECT count(*) FROM public.owner_beta_paper_events WHERE payload->>'generation'='"
            + generation + "')") == 0, 'PAPER_DOCKER_FIXTURE_PREINTAKE_DISPATCH')
        self.acceptance.records[self.package]['zero_dispatch'] = True

    def prove_disposition(self):
        f, o = self.faults, self.package.ops
        until = time.monotonic() + self.bound(2, cleanup=True)
        while True:
            alive = False
            for process in (self.helper[1], self.backend[2]):
                try:
                    ticks = f.start_ticks(process['pid'])
                except FileNotFoundError:
                    ticks = None
                alive |= ticks == process['start_ticks']
            groups = o._proc_cgroups().values()
            rows = self.activity()
            clients = {(c.backend['pid'], c.backend['start_us']) for c in (self.blocker, self.probe)}
            actual = {(r['pid'], r['start_us']) for r in rows}
            _require(len(actual) == len(rows) and clients <= actual and
                actual <= clients | {(self.backend[0], self.backend[1])},
                'PAPER_DOCKER_FIXTURE_INTAKE_DISPOSITION')
            if not alive and not any(self.call._cid in group for group in groups) and actual == clients:
                _require(self.query('SELECT count(*) FROM pg_prepared_xacts') == 0,
                         'PAPER_DOCKER_FIXTURE_INTAKE_DISPOSITION')
                return
            _require(time.monotonic() < until, 'PAPER_DOCKER_FIXTURE_INTAKE_DISPOSITION')
            time.sleep(min(0.01, self.bound(0.01, cleanup=True)))

    def close_clients(self):
        errors = []
        for client in (self.blocker, self.probe):
            if client is not None and not client.absent:
                try:
                    client.close()  # retains exact backend/process terminal proof
                except BaseException as error:
                    errors.append(error)
        self.clients_absent = all(c is not None and c.absent and c.terminal is not None
                                  for c in (self.blocker, self.probe))
        if errors:
            raise errors[0]

    def proven(self):
        call = self.call
        eligible = (self.correlated and self.disposed and self.clients_absent and self.acceptance.fired
            and call is not None and not call._imported and not call._closed and call._preserve
            and self.package.owner._poisoned and self.package.completed_destination is None
            and self.acceptance.records[self.package]['calls'] == {'begin': 1}
            and self.acceptance.records[self.package]['zero_dispatch'])
        if not eligible:
            return False
        try:
            self.bound(1, cleanup=True)
            generation = self.helper[0]
            o = self.package.ops
            o._terminal_container(call._context.docker.object('container', call._cid), generation)
            for process in (self.helper[1], self.backend[2]):
                try:
                    ticks = self.faults.start_ticks(process['pid'])
                except FileNotFoundError:
                    ticks = None
                if ticks == process['start_ticks']:
                    return False
            return not any(call._cid in group for group in o._proc_cgroups().values())
        except Exception:
            return False


    def prepare_fixture_disposal(self):
        # This branch is standalone verify, so it owns no source stop attempt.
        _require(self.proven() and type(self.package.owner) is self.p._PersistedArchiveOwner
            and not self.package.ops._STOP_ATTEMPTS.get(self.call._context),
            'PAPER_DOCKER_FIXTURE_INTAKE_DISPOSITION')
        self.close_clients()
        if not self.package.guest.absent:
            self.package.guest.close()  # actual clone process/cgroup/absence checks
        _require(self.package.guest.absent and all(c.absent for c in self.package.readers),
                 'PAPER_DOCKER_FIXTURE_INTAKE_DISPOSITION')

    def dispose_retained_fixture(self):
        """After supervisor helper removal, release original owned FDs/trees.

        Never set production's imported/closed flags, advance stages or invoke
        its success publisher. Poisoned objects remain poisoned and unusable.
        """
        if self.fixture_handles_closed:
            return
        package, call, owner = self.package, self.call, self.package.owner
        _require(self.correlated and self.disposed and self.clients_absent and owner._poisoned
            and not call._imported and not call._closed and package.guest.absent
            and not owner._closed and not package.completed_destination,
            'PAPER_DOCKER_FIXTURE_INTAKE_DISPOSITION')
        _require(not owner._cleanup_docker(['ps', '--all', '--no-trunc', '--filter', 'id='+call._cid,
            '--format', '{{.ID}}']).strip(), 'PAPER_DOCKER_FIXTURE_INTAKE_DISPOSITION')
        # Retain generation facts, but dispose only after both exact host
        # generations have disappeared and the owned CID was removed.
        for process in (self.helper[1], self.backend[2]):
            try:
                ticks = self.faults.start_ticks(process['pid'])
            except FileNotFoundError:
                ticks = None
            _require(ticks != process['start_ticks'], 'PAPER_DOCKER_FIXTURE_INTAKE_DISPOSITION')
        destination = package.destination
        for custody in [*destination.imports, *destination.actions]:
            custody.close()
        for stage in reversed(destination.stages):
            stage.close(remove=True)
        destination.operation.close()
        destination.store.close()
        if destination.lock_fd is not None:
            os.close(destination.lock_fd)
            destination.lock_fd = None
        destination.tree.close()
        if package.custody is not None:
            package.custody.close()
        if package.snapshot is not None:
            package.snapshot.__exit__(None, None, None)
        for original in [*package.originals, *package.ciphertexts.values()]:
            original.close()
        for _parent, _name, fd, _identity in reversed(package.directories):
            os.close(fd)
        package.directories.clear()
        package.tree.close()
        package.work_tree.close()
        if owner.audit is not None:
            owner.audit.close()
        for fd, *_rest in owner.spools.values():
            os.close(fd)
        owner.spools.clear()
        for source in reversed(owner.sources):
            source.close()
        for source in owner.secret_sources or ():
            os.close(source['file_fd'])
            os.close(source['service_fd'])
            source['tree'].close()
        # PersistedArchiveCustody.close requires production resource closure;
        # it owns no FD and must stay unresolved along with the poisoned owner.
        # Revoke fixture-owned registry entries; do not certify close/success.
        self.p._PACKAGES.discard(package)
        self.p._OWNERS.pop(owner._custody, None)
        self.p._SPACES.pop(call._context, None)
        for run in tuple(self.p._RUNS):
            if run.live is call._context:
                self.p._RUNS.discard(run)
        self.fixture_handles_closed = True

    def remove_fixture_trees(self):
        # Native stages retain their own directory FDs until the supervisor's
        # final stage loop. Delete only after that loop, under original roots.
        _require(self.fixture_handles_closed and not self.fixture_disposed,
                 'PAPER_DOCKER_FIXTURE_INTAKE_DISPOSITION')
        space = self.package.owner.space
        space.root_tree.revalidate()
        space.selection._remove_tree(space.root_tree.fd)
        space.root_tree.close()
        space.root.rmdir()
        space.selection._dispose_private_tree(space.selection._backup_module().files, space.tree)
        self.fixture_disposed = True


class ArchiveAcceptance:
    """Effects-only observer around the ordinary production archive caller.

    Construct only inside the existing disposable G2 owner. All guarded objects
    are made by run_locked. Profiling observes their real calls; it neither
    replaces callables nor returns validation results to production.
    """
    REQUIRED_ROWS = frozenset((
        "owner_beta_daily_admissions", "owner_beta_recommendation_runs",
        "owner_beta_recommendation_items", "owner_beta_paper_lineage",
        "owner_beta_paper_source_originals", "owner_beta_paper_sessions",
        "cash_ledger", "orders", "fills", "positions", "daily_equity",
        "paper_settlement_outbox", "notifications", "notification_deliveries",
        "owner_beta_simulation_runs", "owner_beta_simulation_result_pairs",
        "owner_beta_paper_operations", "owner_beta_paper_events"))
    FAULTS = ("ciphertext", "missing-member", "changed-member", "dataset-version",
              "dataset-status", "economics", "destination-identity", "destination-floor",
              "writer-backend", "pre-intake-cancel", "private-reverify", "publish-fsync",
              "publish-rename", "publish-latest", "intake-cancel")

    def __init__(self, owner, fault_module, kind=None):
        _require(kind is None or kind in self.FAULTS, "PAPER_DOCKER_FIXTURE_FAULT")
        self.owner, self.faults, self.kind = owner, fault_module, kind
        self.paper = _paper_backup()
        self.packages, self.events, self.mutations, self.readers = [], [], [], []
        self.records, self.roles = {}, {}
        self.fired, self.publications, self.snapshots = False, [], []
        self.saved_fd = None
        self.namespace_fault = None
        self.intake = None

    @staticmethod
    def source_rows(package):
        rows = {}
        with open(package.path / "audit.stream", "rb") as source:
            package.owner.audit_module.decode(source)
            source.seek(0)
            for line in source:
                row = package.owner.audit_module.parse(line[:-1])
                if len(row) == 5 and row[0] == "row":
                    rows.setdefault(row[2], {})[row[3]] = row[4]
        return rows

    def meaningful_source(self, package):
        rows = self.source_rows(package)
        _require(all(rows.get(name) for name in self.REQUIRED_ROWS), "PAPER_DOCKER_FIXTURE_HISTORY_REQUIRED")
        _require(len(rows["owner_beta_paper_operations"]) >= 2, "PAPER_DOCKER_FIXTURE_PRIOR_ROOT_REQUIRED")
        datasets = [json.loads(line) for line in (package.path / "dataset-rows.jsonl").read_bytes().splitlines()]
        _require(datasets and any(row["dataset_version_id"] in {item["id"] for item in datasets}
            and row["beta_lineage_id"] is None and row["pending_target_id"] is None
            for row in rows.get("paper_rebalance_previews", {}).values()), "PAPER_DOCKER_FIXTURE_PREVIEW_REQUIRED")
        now = time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime())
        _require(any(row["claim_token"] is not None and row["claim_expires_at"] is not None
            and row["claim_expires_at"] < now for row in rows["paper_settlement_outbox"].values()),
            "PAPER_DOCKER_FIXTURE_EXPIRED_CLAIM_REQUIRED")
        # Retain counts/hashes only. No provider bytes, identities or SQL rows
        # leave this observer in a fixture receipt.
        return {name: len(rows[name]) for name in sorted(self.REQUIRED_ROWS)}

    def query(self, package, sql, *, write=False):
        reader = self.paper._GuestClient(package, package.guest.admin)
        self.readers.append(reader)
        try:
            reader.start()
            if write:
                reader.exchange(b"ROLLBACK;\nBEGIN READ WRITE;\n")
            result = reader.exchange(sql.encode("ascii"))
            if write:
                reader.exchange(b"COMMIT;\n")
            return json.loads(result)
        finally:
            reader.close()

    def clone_state(self, destination):
        generation = destination.operation.generation
        return self.query(destination.package,
            "SELECT jsonb_build_object('control',(SELECT jsonb_build_object('state',state,'epoch',control_epoch) "
            "FROM public.owner_beta_paper_control WHERE account_id='" + destination.archive["account_id"] + "'),"
            "'operations',(SELECT count(*) FROM public.owner_beta_paper_operations WHERE generation='" + generation + "'),"
            "'events',(SELECT count(*) FROM public.owner_beta_paper_events WHERE payload->>'generation'='" + generation + "'),"
            "'prepared',(SELECT count(*) FROM pg_prepared_xacts));\n")

    def _belongs(self, obj):
        context = getattr(getattr(obj, "owner", None), "context", None)
        return context is not None and context.install_root == str(self.owner.root)

    def observe(self, phase, event, frame, result):
        obj = frame.f_locals.get("self")
        if phase == "context" and event == "return" and result is not None:
            if result.install_root == str(self.owner.root):
                self.owner.contexts.append(result)
                self.owner.context_modules[result] = self.paper._operations(result)
            return
        package = obj if type(obj) is self.paper._ArchivePackage else getattr(obj, "package", getattr(obj, "_archive_owner", None))
        if package is not None and self._belongs(package):
            if package not in self.packages:
                self.packages.append(package)
                self.records[package] = {"prepared": False, "completed": False, "closed": False,
                    "delta": False, "rls": False, "receipts": {}, "calls": {}, "zero_dispatch": False}
            record = self.records[package]
            if phase == "destination" and event == "call":
                verb = frame.f_locals["verb"]
                record["calls"][verb] = record["calls"].get(verb, 0) + 1
                _require(record["calls"][verb] == 1, "PAPER_DOCKER_FIXTURE_ARCHIVE_REPLAY")
            if phase == "close" and event == "call" and self.fired and self.kind in {
                    "ciphertext", "missing-member", "changed-member", "dataset-version", "dataset-status",
                    "economics", "destination-identity", "destination-floor", "private-reverify"}:
                self.prove_zero_dispatch(package)
                # The consuming failure has occurred. Restore the original
                # owned bytes/name before production's cleanup revalidation;
                # never suppress that failure or publish the repaired input.
                for mutation in reversed(self.mutations):
                    mutation.close()
            if phase == "prepare" and event == "return" and package.guest is not None and package.guest.started:
                package.revalidate()
                record["history"] = self.meaningful_source(package)
                _require(set(package.ciphertexts) == set(self.paper._PAPER_MEMBERS), "PAPER_DOCKER_FIXTURE_CIPHERS")
                record["clone"] = package.guest.cid
                record["prepared"] = True
            elif phase == "destination" and event == "return" and obj.receipts:
                record["receipts"] = dict(obj.receipts)
            elif phase == "delta" and event == "return":
                record["delta"] = (package.work / "after.audit").is_file()
            elif phase == "rls" and event == "return":
                record["rls"] = True
            elif phase == "client" and event == "return" and obj.backend is not None:
                self.roles.setdefault(package, set()).add(obj.role)
            elif phase == "invoke" and event == "return" and package.completed_destination is not None:
                destination = package.completed_destination
                state = self.clone_state(destination)
                _require(state["control"] == {"state": "RESTORED_OFF", "epoch": destination.fence + 1}
                    and state["operations"] == 1 and state["prepared"] == 0,
                    "PAPER_DOCKER_FIXTURE_RESTORED_SQL")
                _require(set(destination.receipts) == {"begin", "reconcile", "restore-off"}
                    and all(value["status"] == "COMMITTED" for value in destination.receipts.values()),
                    "PAPER_DOCKER_FIXTURE_RESTORED_READBACK")
                record["receipts"] = {verb: value["request_sha256"] for verb, value in destination.receipts.items()}
                record["completed"] = True
                self.events.append("restore-complete")
            elif phase == "close" and event == "return" and package.closed:
                _require((package.guest is None or package.guest.absent) and package.process is None and package.call is None
                    and all(reader.absent for reader in package.readers), "PAPER_DOCKER_FIXTURE_ARCHIVE_CLEANUP")
                record["closed"] = True
                self.events.append("clone-closed")
            self.inject(phase, event, obj, package, frame, result)
        elif phase == "snapshot" and event == "return" and self._belongs(obj):
            if obj.head[2] == "SNAPSHOT":
                self.snapshots.append(obj.head[1]); self.events.append("snapshot")
        elif phase == "publish" and self._belongs(obj):
            if event == "call":
                _require(len(self.packages) == 2 and all(self.records[p]["closed"] for p in self.packages)
                    and obj.owner._closed and obj.owner.space.closed, "PAPER_DOCKER_FIXTURE_PUBLICATION_ORDER")
                self.events.append("publish")
            elif event == "return" and obj.published:
                self.publications.append(obj.path)
        elif phase == "candidate" and event == "call" and self.kind == "private-reverify" and not self.fired:
            candidate = frame.f_locals["candidate"]
            if self._belongs(candidate):
                self.mutations.append(self.faults.RetainedMemberFault(
                    candidate.retained, "ciphertext", member="postgres.dump.enc"))
                self.fired = True
        elif phase == "storage" and type(obj) is self.paper._PaperStorage and self._belongs(obj):
            self.publication_fault(event, frame, result)

    def prove_zero_dispatch(self, package):
        destination = package.destination
        if destination is not None and destination.operation is not None:
            for action in destination.actions:
                _require(action._dispatch_bytes is None, "PAPER_DOCKER_FIXTURE_PREINTAKE_DISPATCH")
                try:
                    os.stat("dispatch.json", dir_fd=action._output_fd, follow_symlinks=False)
                except FileNotFoundError:
                    pass
                else:
                    _require(False, "PAPER_DOCKER_FIXTURE_PREINTAKE_DISPATCH")
            # Independent read-only server observation remains possible after a
            # retained archive original changed. It does not call a bypassed
            # production verifier and cannot authorize the destination.
            package.guest.check(ready=True)
            generation = destination.operation.generation
            sql = "SELECT count(*) FROM public.owner_beta_paper_operations WHERE generation='" + generation + "';"
            result = self.owner.command(["exec", "--interactive", "--user", "999:999", package.guest.cid,
                "psql", "-X", "-qAt", "-h", "/run/postgresql", "-U", package.guest.admin,
                "-d", package.database["database"], "-v", "ON_ERROR_STOP=1"], sql.encode(), cap=5)
            _require(result.stdout.strip() == b"0", "PAPER_DOCKER_FIXTURE_PREINTAKE_DISPATCH")
        self.records[package]["zero_dispatch"] = True

    def inject(self, phase, event, obj, package, frame, result):
        if self.fired or self.kind is None or event != "call":
            return
        if phase == "destination" and frame.f_locals.get("verb") == "begin" and self.kind == "intake-cancel":
            _require(self.intake is None, "PAPER_DOCKER_FIXTURE_ARCHIVE_REPLAY")
            self.intake = IntakeCancellation(self, package)
            self.intake.arm()
            return
        if phase == "attached" and self.kind == "intake-cancel" and obj._restore_owner is not None \
                and obj._restore_owner is package.destination and obj.verb == "begin":
            _require(self.intake is not None, "PAPER_DOCKER_FIXTURE_INTAKE_BINDING")
            self.intake.observe(obj, frame)
            return
        if phase == "prepare" and package.persisted is not None and self.kind == "ciphertext":
            self.mutations.append(self.faults.RetainedMemberFault(package.persisted, self.kind,
                                                                member="postgres.dump.enc"))
        elif phase == "invoke" and self.kind in {"missing-member", "changed-member"}:
            original = next(v for v in package.originals if v.path == str(package.path / "files.bundle"))
            self.mutations.append(self.faults.RetainedMemberFault(original, self.kind))
        elif phase == "destination" and frame.f_locals.get("verb") == "begin" and self.kind in {
                "dataset-version", "dataset-status", "economics"}:
            if self.kind == "economics":
                sql = "WITH changed AS (UPDATE public.cash_ledger SET amount=amount+1 WHERE id=(SELECT id FROM public.cash_ledger ORDER BY id LIMIT 1) RETURNING id) SELECT count(*) FROM changed;\n"
            else:
                rows = [json.loads(line) for line in (package.path / "dataset-rows.jsonl").read_bytes().splitlines()]
                _require(rows, "PAPER_DOCKER_FIXTURE_PREVIEW_REQUIRED")
                identity = rows[0]["id"]
                _require(type(identity) is str and package.ops.journal.UUID_RE.fullmatch(identity),
                         "PAPER_DOCKER_FIXTURE_PREVIEW_REQUIRED")
                assignment = "version=version||'-fixture-corrupt'" if self.kind == "dataset-version" else "status=CASE WHEN status='READY' THEN 'WARNING' ELSE 'READY' END"
                sql = "WITH changed AS (UPDATE public.dataset_versions SET " + assignment + " WHERE id=" + self.paper._sql_literal(identity) + "::uuid RETURNING id) SELECT count(*) FROM changed;\n"
            _require(self.query(package, sql, write=True) == 1, "PAPER_DOCKER_FIXTURE_FAULT_EFFECT")
        elif phase == "destination" and frame.f_locals.get("verb") == "begin" and self.kind in {
                "destination-identity", "destination-floor"}:
            leaf = "installation-id" if self.kind == "destination-identity" else "epoch-floor"
            target = obj.path / obj.ops.PAPER_DIR / leaf
            original = self.paper._ArchiveOriginal(package, target, mode=0o600)
            try:
                self.mutations.append(self.faults.RetainedMemberFault(original, "changed-member"))
            finally:
                original.close()
        elif phase == "stop" and self.kind == "writer-backend":
            reader = self.paper._GuestClient(package, "migration_owner")
            self.readers.append(reader)
            reader.start()  # actual extra live backend remains through STOPPED observation
        elif phase == "invoke" and self.kind == "pre-intake-cancel":
            self.fired = True
            raise KeyboardInterrupt()
        else:
            return
        self.fired = True

    def publication_fault(self, event, frame, function):
        # Each failure is a real kernel/filesystem effect at the existing
        # publisher. No os.fsync/rename/replace implementation is patched.
        local = frame.f_locals
        if self.saved_fd is not None and event == "c_exception" and function is os.fsync:
            original, saved = self.saved_fd
            os.dup2(saved, original); os.close(saved); self.saved_fd = None
        if self.fired or event != "c_call":
            return
        if self.kind == "publish-fsync" and function is os.fsync and "fd" in local:
            fd = local["fd"]
            self.saved_fd = (fd, os.dup(fd)); os.close(fd)
        elif self.kind == "publish-rename" and function is os.fsync and "directory" in local and "parent" not in local:
            final = pathlib.Path(local["final"])
            if not final.exists():
                final.mkdir(mode=0o700)
                self.namespace_fault = ("rename", final, self.faults.directory_identity(final))
            else:
                return
        elif self.kind == "publish-latest" and function is os.replace:
            parent = pathlib.Path(local["self"].root)
            latest, saved = parent / "latest", parent / (".fixture-latest-" + os.urandom(8).hex())
            had_latest = latest.is_symlink()
            if had_latest:
                latest.rename(saved)
            latest.mkdir(mode=0o700)
            self.namespace_fault = ("latest", latest, self.faults.directory_identity(latest), saved, had_latest)
        else:
            return
        self.fired = True

    def __enter__(self):
        p = self.paper
        production_ops, _context = p._application_context(self.owner.context)
        functions = dict(context=production_ops.open_context, prepare=p._ArchivePackage.prepare,
            invoke=p._ArchivePackage.invoke, close=p._ArchivePackage.close,
            attached=production_ops._NativeInvocation._read_attached,
            destination=p._DestinationRestore.invoke, stop=p._DestinationRestore.stop_and_restore_off,
            delta=p._DestinationRestore.compare_post_restore, rls=p._DestinationRestore.verify_role_visibility,
            client=p._GuestClient.start, candidate=p._HeldPaperSnapshot.verify_candidate,
            snapshot=production_ops._ArchiveCustody.seal_snapshot, publish=p._PaperPublication.publish,
            storage=p._local("owner_beta_daily_backup", "owner-beta-daily-backup.py").Storage.publish)
        self.profile = self.faults.ProductionPhases(functions, self.observe, self.owner.deadline.value)
        self.profile.__enter__()
        return self

    def __exit__(self, *error):
        self.profile.__exit__(*error)
        if self.intake is not None:
            self.intake.close_clients()
        for reader in reversed(self.readers):
            if not reader.absent:
                reader.close()
        for mutation in reversed(self.mutations):
            mutation.close()
        if self.saved_fd is not None:
            original, saved = self.saved_fd
            os.dup2(saved, original); os.close(saved); self.saved_fd = None
        if self.namespace_fault is not None:
            kind, path, identity, *rest = self.namespace_fault
            _require(self.faults.directory_identity(path) == identity, "PAPER_DOCKER_FIXTURE_FAULT_EFFECT")
            path.rmdir()
            if kind == "latest" and rest[1]:
                rest[0].rename(path)

    def successful(self, action):
        expected = 2 if action == "backup" else 1
        _require(len(self.packages) == expected and len({self.records[p]["clone"] for p in self.packages}) == expected,
                 "PAPER_DOCKER_FIXTURE_FRESH_CLONE")
        for package in self.packages:
            row = self.records[package]
            _require(all(row[key] for key in ("prepared", "completed", "closed", "delta", "rls"))
                and {"migration_owner", "app", "worker", "admin", "audit_writer", "research_writer"} <= self.roles[package]
                and len(set(row["receipts"].values())) == 3, "PAPER_DOCKER_FIXTURE_ARCHIVE_EVIDENCE")
        _require((len(self.snapshots), len(self.publications)) == ((1, 1) if action == "backup" else (0, 0)),
                 "PAPER_DOCKER_FIXTURE_PUBLICATION_ORDER")
        return {"clones": expected, "roles_per_clone": 6, "actual_restore_readbacks": expected * 3,
                "cleanup": "REAPED_AND_ABSENT", "source_history": self.records[self.packages[0]]["history"]}


def run_archive_acceptance(owner, storage_values, fault_module, *, action="backup", fault=None):
    """Normal caller with actual disposable context; no replacement authority."""
    _require(action in {"backup", "verify", "reconcile"}, "PAPER_DOCKER_FIXTURE_ACTION")
    _require(str(owner.root).startswith("/var/lib/lagrange-paper-operations-fixture-")
        and owner.context.commit == owner.commit, "PAPER_DOCKER_FIXTURE_ROOT")
    paper = _paper_backup()
    with ArchiveAcceptance(owner, fault_module, fault) as evidence:
        result = paper.run_locked(owner.context, owner.lock_fd, action,
            storage_values["MAX_TOTAL_BYTES"], storage_values=storage_values)
    _require(fault is None, "PAPER_DOCKER_FIXTURE_FAULT_NOT_REJECTED")
    _require(type(result) is paper.PaperHandledResult and result.status == {
        "backup": "BACKUP_COMPLETE", "verify": "VERIFIED_ISOLATED", "reconcile": "RECONCILED_DISABLED"}[action],
        "PAPER_DOCKER_FIXTURE_ARCHIVE_RESULT")
    return result, evidence.successful(action) if action != "reconcile" else {"status": result.status}


def main():
    # No fixture selector is accepted from argv, env, or a production install.
    if len(sys.argv) != 1:
        sys.stdout.write("PAPER_DOCKER_FIXTURE_NOT_RUN:ARGUMENTS_NOT_ALLOWED\n")
        return 77
    sys.stdout.write("PAPER_DOCKER_FIXTURE_NOT_RUN:HOSTED_ROOT_PG_FIXTURE_REQUIRED\n")
    return 77


if __name__ == "__main__":
    raise SystemExit(main())
