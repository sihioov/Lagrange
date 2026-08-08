"""Todo 41: what the kill switch does to orders already at the broker.

Engaging denies NEW intents unconditionally and instantly -- that is the
gate's job, not this module's. What happens to orders already working is a
configured choice, and the one thing no policy may do is act on an order whose
broker number we do not have.
"""
from __future__ import annotations

import pytest

from live_node.cancel_policy import (
    CancelPolicy,
    OrderDisposition,
    WorkingOrder,
    disposition,
    plan,
)


def working(state="ACCEPTED", no="B-1", filled=0, ref="oi-1"):
    return WorkingOrder(
        intent_ref=ref, state=state, broker_order_no=no, cumulative_filled=filled
    )


@pytest.mark.parametrize("policy", list(CancelPolicy))
def test_an_unresolved_order_is_never_cancelled_by_any_policy(policy):
    # THE rule. We do not know its broker order number -- that is what UNKNOWN
    # means -- so a cancel would fail or, far worse, name the wrong order.
    # Todo 39 forbids acting on UNKNOWN before a lookup, and a kill switch
    # makes that matter more, not less.
    for state in ("SUBMITTING", "SUBMITTED", "UNKNOWN"):
        order = WorkingOrder(intent_ref="oi-x", state=state, broker_order_no=None)
        assert disposition(order, policy) is OrderDisposition.RESOLVE_FIRST

    # Even an UNKNOWN that somehow carries a number is not actionable: the
    # number cannot be trusted until a lookup confirms it.
    assert (
        disposition(working(state="UNKNOWN"), policy) is OrderDisposition.RESOLVE_FIRST
    )


def test_leave_is_the_default_and_cancels_nothing():
    # The right choice when the switch was pulled because OUR system is
    # misbehaving: an order already at the broker is under the broker's
    # control and is not the thing going wrong.
    assert disposition(working(), CancelPolicy.LEAVE) is OrderDisposition.LEAVE
    assert (
        disposition(working(state="PARTIALLY_FILLED", filled=3), CancelPolicy.LEAVE)
        is OrderDisposition.LEAVE
    )


def test_cancel_working_cancels_everything_the_broker_holds():
    assert disposition(working(), CancelPolicy.CANCEL_WORKING) is OrderDisposition.CANCEL
    assert (
        disposition(
            working(state="PARTIALLY_FILLED", filled=3), CancelPolicy.CANCEL_WORKING
        )
        is OrderDisposition.CANCEL
    )


def test_cancel_unfilled_only_leaves_a_partially_filled_order_alone():
    # Cancelling the rest of a partial strands a position that no strategy is
    # now managing, which is usually worse than letting the order complete.
    assert (
        disposition(working(filled=0), CancelPolicy.CANCEL_UNFILLED_ONLY)
        is OrderDisposition.CANCEL
    )
    assert (
        disposition(
            working(state="PARTIALLY_FILLED", filled=1),
            CancelPolicy.CANCEL_UNFILLED_ONLY,
        )
        is OrderDisposition.LEAVE
    )


@pytest.mark.parametrize("policy", list(CancelPolicy))
def test_a_terminal_order_is_left_alone_by_every_policy(policy):
    for state in ("FILLED", "CANCELED", "EXPIRED", "REJECTED", "DENIED"):
        assert disposition(working(state=state), policy) is OrderDisposition.LEAVE, state


def test_the_whole_sweep_is_decided_before_anything_is_sent():
    # Deciding up front makes the plan auditable, and means a failure partway
    # through does not leave an operator guessing which orders were acted on.
    orders = [
        working(ref="a", state="ACCEPTED", no="B-1"),
        working(ref="b", state="PARTIALLY_FILLED", no="B-2", filled=2),
        WorkingOrder(intent_ref="c", state="UNKNOWN", broker_order_no=None),
        working(ref="d", state="FILLED", no="B-4"),
    ]
    result = plan(orders, CancelPolicy.CANCEL_UNFILLED_ONLY)
    assert result.to_cancel == ("a",)
    assert result.to_leave == ("b", "d")
    assert result.to_resolve == ("c",)
    assert result.requires_lookup


def test_a_plan_with_nothing_unresolved_needs_no_lookup():
    result = plan([working()], CancelPolicy.CANCEL_WORKING)
    assert not result.requires_lookup
    assert result.to_cancel == ("oi-1",)
