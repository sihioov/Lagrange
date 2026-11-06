#!/usr/bin/env python3
"""Actual selected-image BEGIN/readback fixture driver.

The hosted fixture owner prepares a disposable install tree, its real selected
paper-scheduler image and producer receipts, a root-owned journal action, and a
disposable PostgreSQL server. This file accepts those in-process objects only;
it has no CLI path or environment-controlled install/database selector.
"""
from __future__ import annotations

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


def run_genuine_fixture(context, release_lock_fd, action_custody):
    """Run one real selected-image BEGIN, duplicate, exact readback and history probe.

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

        # This probe must report the fixture's configured OFF state before BEGIN.
        with live.probe_history() as inventory:
            _require(inventory._configured is True and
                     ops._json(inventory._index, ops.MAX_INDEX)["state"] == "OFF",
                     "PAPER_DOCKER_FIXTURE_OFF")

        original_dispatch = None
        original_result = None
        first_identity = None
        imported_custody = None
        with live.invoke("begin", action_custody) as invocation:
            result = invocation.result
            _require(result["outcome"] == "OK" and result["event_id"] is not None
                     and result["control_epoch"] is not None,
                     "PAPER_DOCKER_FIXTURE_BEGIN")
            imported_custody = invocation.import_dispatch()
            original_dispatch = action_custody._dispatch_bytes
            original_result = invocation._result_bytes
            first_identity = (result["event_id"], result["control_epoch"],
                              result["artifact_sha256"])
            _require(type(original_result) is bytes
                     and result["artifact_sha256"] == ops._digest(original_result),
                     "PAPER_DOCKER_FIXTURE_RESULT_ARTIFACT")
            imported_custody.close()
            imported_custody = None
        _require(original_dispatch is not None and first_identity is not None,
                 "PAPER_DOCKER_FIXTURE_IMPORT")
        action_custody.close()

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
        finally:
            duplicate_action.close()

        historical = operation._journal.open_custody(generation, sequence)
        try:
            with ops.open_custody_context(context, release_lock_fd) as readback_context:
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
        finally:
            historical.close()
            if imported_custody is not None:
                imported_custody.close()
    return {"status": "PASS", "cases": ("BEGIN", "DUPLICATE_NO_RESUBMISSION",
                                         "EXACT_IMPORT", "EXACT_READBACK", "CONFIGURED_OFF_PROBE")}


def run_restore_no_sql_rejection(context, release_lock_fd, restore_action_custody):
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
