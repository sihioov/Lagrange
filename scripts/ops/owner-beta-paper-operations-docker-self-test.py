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
import os
import pathlib
import sys

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
                drained.revalidate()
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
                _require(type(off_event) is str and ops.journal.UUID_RE.fullmatch(off_event)
                         and operation._last[2] == "OFF_COMMITTED"
                         and operation._last[0] == 4
                         and operation._last[3]["event_id"] == off_event
                         and operation._last[3]["control_epoch"] == first_identity[1] + 1
                         and operation._journal._read_floor() == first_identity[1] + 1,
                         "PAPER_DOCKER_FIXTURE_FENCED")
                if observer is not None:
                    observer("off", drained, off_event)
                before = ops.journal._read_named(operation._fd, "00000004.json", 0o400,
                                                 ops.journal.MAX_SMALL)
                _require(ops.commit_off(drained) == off_event
                         and ops.journal._read_named(operation._fd, "00000004.json", 0o400,
                                                     ops.journal.MAX_SMALL) == before,
                         "PAPER_DOCKER_FIXTURE_FENCED_REPLAY")
                if observer is not None:
                    observer("off-replay", drained, off_event)
                if operation._intent["purpose"] == "BACKUP":
                    run_capture_prefix(live, drained)
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
    return {"status": "PASS", "cases": ("BEGIN", "DUPLICATE_NO_RESUBMISSION",
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
    """G2 BACKUP consumes real exported resources; no missing-input waiver.

    Only the precise downstream reference boundary is expected. A missing source,
    foreign client, unobserved dump backend or cleanup failure fails this case.
    No COMPLETE, encrypted archive or restored-state acceptance follows.
    """
    paper = _paper_backup()
    owner = paper.open_snapshot(live, drained, 2 * 1024 * 1024 * 1024)
    try:
        with owner:
            files = owner.selection._backup_module().files
            try:
                owner.capture(files.Limits(owner.maximum))
            except paper.PaperBackupError as error:
                _require(str(error) == "PAPER_BACKUP_REFERENCE_CLOSURE_INCOMPLETE",
                         "PAPER_DOCKER_FIXTURE_PRODUCER")
            else:
                _require(False, "PAPER_DOCKER_FIXTURE_PRODUCER")
            owner.revalidate()
    finally:
        # Failed cleanup must retain the actual registered owner, not signal
        # success because an expected capture exception was observed.
        if owner._closed:
            _require(owner._resources_closed(), "PAPER_DOCKER_FIXTURE_PRODUCER")
    _require(owner._closed and owner._snapshot is None, "PAPER_DOCKER_FIXTURE_PRODUCER")


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


def main():
    # No fixture selector is accepted from argv, env, or a production install.
    if len(sys.argv) != 1:
        sys.stdout.write("PAPER_DOCKER_FIXTURE_NOT_RUN:ARGUMENTS_NOT_ALLOWED\n")
        return 77
    sys.stdout.write("PAPER_DOCKER_FIXTURE_NOT_RUN:HOSTED_ROOT_PG_FIXTURE_REQUIRED\n")
    return 77


if __name__ == "__main__":
    raise SystemExit(main())
