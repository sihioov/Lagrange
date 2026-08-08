"""Todo 41: the startup sequence, in the one order that is safe.

Todo 40 documented a contract on `reconcile()` -- sweep in-flight intents to
UNKNOWN before a startup pass -- and left it for this module to honour. Every
wrong ordering here fails SILENTLY, which is why the order itself is asserted
rather than only its outcome.
"""
from __future__ import annotations

from live_node.startup import IN_FLIGHT, StartupOutcome, plan_startup, sweep_targets


def test_in_flight_intents_are_swept_so_a_crash_cannot_go_green():
    # The hazard Todo 40 named. After a crash the intent sits in SUBMITTING,
    # positions and cash may agree because the order may never have landed,
    # and an unswept reconciliation returns GREEN while the intent is still
    # unresolved -- the node would go READY with an order it cannot account
    # for.
    states = {
        "oi-inflight": "SUBMITTING",
        "oi-sent": "SUBMITTED",
        "oi-working": "ACCEPTED",
    }
    assert sweep_targets(states) == ("oi-inflight", "oi-sent")


def test_an_already_unknown_intent_is_not_swept_again():
    # It is already where the sweep would put it, and appending a second
    # SubmissionTimedOut is an illegal transition -- which the repo refuses,
    # turning a safe restart into a startup failure.
    assert sweep_targets({"oi-1": "UNKNOWN"}) == ()
    assert "UNKNOWN" not in IN_FLIGHT


def test_settled_intents_are_left_alone():
    for state in ("FILLED", "CANCELED", "EXPIRED", "REJECTED", "DENIED", "ACCEPTED"):
        assert sweep_targets({"oi-1": state}) == (), state


def test_the_sweep_order_is_stable_so_a_plan_is_reviewable():
    # Two restarts of the same state must produce the same plan, or an
    # incident review cannot compare them.
    states = {"oi-c": "SUBMITTING", "oi-a": "SUBMITTED", "oi-b": "SUBMITTING"}
    assert sweep_targets(states) == ("oi-a", "oi-b", "oi-c")
    assert sweep_targets(states) == sweep_targets(dict(reversed(list(states.items()))))


def test_a_clean_startup_reaches_ready():
    plan = plan_startup(
        {"oi-done": "FILLED"},
        mismatch_kinds=(),
        fills_to_apply=(),
        lookups_required=(),
    )
    assert plan.outcome is StartupOutcome.READY
    assert plan.may_trade
    assert plan.to_sweep == ()


def test_a_blocking_mismatch_keeps_the_node_from_trading():
    plan = plan_startup(
        {},
        mismatch_kinds=("POSITION", "CASH"),
        fills_to_apply=(),
        lookups_required=(),
    )
    assert plan.outcome is StartupOutcome.BLOCKED
    assert not plan.may_trade
    assert plan.blocking_reasons == ("POSITION", "CASH")


def test_required_lookups_are_reported_ahead_of_a_generic_block():
    # Both block, but the next action differs: a lookup is something an
    # operator can just do, while a position mismatch needs a judgement call.
    # Reporting the generic block would hide the actionable one.
    plan = plan_startup(
        {"oi-1": "SUBMITTING"},
        mismatch_kinds=("UNRESOLVED_INTENT",),
        fills_to_apply=(),
        lookups_required=("oi-1",),
    )
    assert plan.outcome is StartupOutcome.LOOKUPS_REQUIRED
    assert not plan.may_trade
    assert plan.lookups_required == ("oi-1",)
    # And it is still swept, so the reconciliation that produced this saw it.
    assert plan.to_sweep == ("oi-1",)


def test_fills_to_apply_are_carried_so_they_precede_the_recorded_run():
    # A completed-while-offline order appears BOTH as an unapplied fill
    # (auto-resolvable) and a position mismatch (blocking). Applying the fill
    # first makes the position agree, so the run we RECORD is the true one --
    # reconciling first would record a blocking mismatch that the very next
    # step resolves, leaving an alert about a problem that no longer exists.
    plan = plan_startup(
        {},
        mismatch_kinds=(),
        fills_to_apply=("E-1", "E-2"),
        lookups_required=(),
    )
    assert plan.fills_to_apply == ("E-1", "E-2")
    # With the fills applied, nothing blocks.
    assert plan.outcome is StartupOutcome.READY


def test_nothing_is_executed_by_planning():
    # The plan is returned, not performed, so it can be inspected in a test
    # and in an incident review before anything irreversible happens.
    states = {"oi-1": "SUBMITTING"}
    plan_startup(states, mismatch_kinds=(), fills_to_apply=(), lookups_required=())
    assert states == {"oi-1": "SUBMITTING"}, "planning must not mutate its input"
