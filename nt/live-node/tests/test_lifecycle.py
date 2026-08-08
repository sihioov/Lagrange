"""Todo 41: the live node never trades before it has earned the right to.

The property this file defends: **no submission while killed, degraded,
unreconciled, stale, stopping, or merely started.** Every one of those is an
independent refusal, because an operator who fixes one and finds the node
still refusing for another reason has learned something useful, while a node
that traded because only five of six conditions were checked has not.
"""
from __future__ import annotations

import pytest

from live_node.lifecycle import (
    IllegalTransition,
    NodeLifecycle,
    NodeState,
    Reason,
    resume_after_crash,
)

ACCOUNT = "acct-live-1"


def ready_node() -> NodeLifecycle:
    node = NodeLifecycle(ACCOUNT)
    node.to(NodeState.RECONCILING)
    node.to(NodeState.READY)
    return node


def all_clear(node: NodeLifecycle):
    return node.status(
        kill_switch_engaged=False, reconciliation_green=True, data_fresh=True
    )


def test_a_node_starts_unable_to_trade_and_must_reconcile_first():
    node = NodeLifecycle(ACCOUNT)
    assert node.state is NodeState.STARTING
    assert not all_clear(node).may_submit()

    # There is NO edge from STARTING to READY. Its absence is the rule: a node
    # cannot begin trading on the strength of having started.
    with pytest.raises(IllegalTransition):
        node.to(NodeState.READY)

    node.to(NodeState.RECONCILING)
    assert not all_clear(node).may_submit(), "reconciling is not yet reconciled"
    node.to(NodeState.READY)
    assert all_clear(node).may_submit()


def test_every_unsafe_condition_refuses_independently():
    node = ready_node()
    # The baseline trades, so each refusal below is caused by the one thing
    # that case changes -- otherwise these could all be passing for the same
    # unrelated reason.
    assert all_clear(node).may_submit()

    kill = node.status(
        kill_switch_engaged=True, reconciliation_green=True, data_fresh=True
    )
    assert kill.refusal() is Reason.KILL_SWITCH

    unreconciled = node.status(
        kill_switch_engaged=False, reconciliation_green=False, data_fresh=True
    )
    assert unreconciled.refusal() is Reason.NOT_RECONCILED

    stale = node.status(
        kill_switch_engaged=False, reconciliation_green=True, data_fresh=False
    )
    assert stale.refusal() is Reason.STALE_DATA

    for status in (kill, unreconciled, stale):
        assert not status.may_submit()


def test_the_kill_switch_is_reported_ahead_of_every_other_reason():
    # "Someone pulled the switch" and "the node is still reconciling" call for
    # completely different responses. Telling an operator the second when the
    # first is true wastes the minutes that matter most.
    node = NodeLifecycle(ACCOUNT)
    node.to(NodeState.RECONCILING)
    status = node.status(
        kill_switch_engaged=True, reconciliation_green=False, data_fresh=False
    )
    assert status.refusal() is Reason.KILL_SWITCH


def test_a_degraded_node_cannot_trade_and_returns_via_reconciliation():
    node = ready_node()
    node.degrade("websocket gap")
    assert node.state is NodeState.DEGRADED
    assert not all_clear(node).may_submit()
    assert all_clear(node).refusal() is Reason.DEGRADED

    # There is no DEGRADED -> READY edge. Whatever degraded us may have
    # happened while orders were in flight, so agreement must be
    # re-established rather than assumed.
    with pytest.raises(IllegalTransition):
        node.to(NodeState.READY)

    node.to(NodeState.RECONCILING)
    node.to(NodeState.READY)
    assert all_clear(node).may_submit()


def test_degrading_twice_is_not_an_error_and_keeps_both_reasons():
    # The same fault is often reported by several subsystems at once; the
    # second report is not a bug.
    node = ready_node()
    node.degrade("stale data")
    node.degrade("db write failed")
    assert node.state is NodeState.DEGRADED
    status = all_clear(node)
    assert status.degraded_reasons == ("stale data", "db write failed")


def test_reaching_ready_clears_the_degraded_history():
    node = ready_node()
    node.degrade("websocket gap")
    node.to(NodeState.RECONCILING)
    node.to(NodeState.READY)
    assert all_clear(node).degraded_reasons == ()


def test_a_stopping_node_accepts_nothing_new():
    node = ready_node()
    node.to(NodeState.STOPPING)
    assert all_clear(node).refusal() is Reason.STOPPING
    node.to(NodeState.STOPPED)
    assert all_clear(node).refusal() is Reason.STOPPING
    # Terminal: nothing follows STOPPED, including a hopeful restart in place.
    with pytest.raises(IllegalTransition):
        node.to(NodeState.RECONCILING)


def test_a_crashed_node_comes_back_unable_to_trade():
    # The whole point of the lifecycle. A process that died mid-order must not
    # resume placing orders against books it has not re-checked.
    before = ready_node()
    assert all_clear(before).may_submit()

    after = resume_after_crash(ACCOUNT)
    assert after.state is NodeState.STARTING
    assert not all_clear(after).may_submit()
    with pytest.raises(IllegalTransition):
        after.to(NodeState.READY)


def test_a_node_owns_exactly_one_account():
    node = NodeLifecycle(ACCOUNT)
    assert node.account_id == ACCOUNT
    assert all_clear(node).account_id == ACCOUNT
    with pytest.raises(ValueError):
        NodeLifecycle("")


def test_every_state_refuses_unless_it_is_ready():
    # Exhaustive: only READY may trade, and only with everything else clear.
    node = NodeLifecycle(ACCOUNT)
    seen = {NodeState.STARTING: node}
    node.to(NodeState.RECONCILING)
    for state in NodeState:
        probe = NodeLifecycle(ACCOUNT)
        # Walk to `state` by the shortest legal path.
        path = {
            NodeState.STARTING: [],
            NodeState.RECONCILING: [NodeState.RECONCILING],
            NodeState.READY: [NodeState.RECONCILING, NodeState.READY],
            NodeState.DEGRADED: [NodeState.RECONCILING, NodeState.DEGRADED],
            NodeState.STOPPING: [NodeState.STOPPING],
            NodeState.STOPPED: [NodeState.STOPPING, NodeState.STOPPED],
        }[state]
        for step in path:
            probe.to(step)
        assert probe.state is state
        status = all_clear(probe)
        assert status.may_submit() is (state is NodeState.READY), (
            f"{state.value} must {'permit' if state is NodeState.READY else 'refuse'}"
        )
    assert seen  # the STARTING node above is covered by the loop's first case
