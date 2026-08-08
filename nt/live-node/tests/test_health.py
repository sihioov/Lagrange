"""Todo 41: health and readiness answer different questions."""
from __future__ import annotations

from live_node.health import METRIC_NAMES, metrics, report
from live_node.lifecycle import NodeLifecycle, NodeState


def node_in(state: NodeState) -> NodeLifecycle:
    node = NodeLifecycle("acct-1")
    path = {
        NodeState.STARTING: [],
        NodeState.RECONCILING: [NodeState.RECONCILING],
        NodeState.READY: [NodeState.RECONCILING, NodeState.READY],
        NodeState.DEGRADED: [NodeState.RECONCILING, NodeState.DEGRADED],
        NodeState.STOPPING: [NodeState.STOPPING],
        NodeState.STOPPED: [NodeState.STOPPING, NodeState.STOPPED],
    }[state]
    for step in path:
        node.to(step)
    return node


def test_a_killed_node_is_healthy_but_not_ready():
    # The classic operational mistake. A node deliberately halted by the kill
    # switch has nothing wrong with it; reporting unhealthy would make an
    # orchestrator restart it, achieving nothing and destroying the in-memory
    # record of why it stopped.
    node = node_in(NodeState.READY)
    status = node.status(
        kill_switch_engaged=True, reconciliation_green=True, data_fresh=True
    )
    r = report(status)
    assert r.healthy is True
    assert r.ready is False
    assert r.refusal == "LIVE_KILL_SWITCH_ENGAGED"


def test_an_unreconciled_node_is_healthy_but_not_ready():
    node = node_in(NodeState.READY)
    status = node.status(
        kill_switch_engaged=False, reconciliation_green=False, data_fresh=True
    )
    r = report(status)
    assert r.healthy is True
    assert r.ready is False
    assert r.refusal == "LIVE_RECONCILIATION_REQUIRED"


def test_only_a_stopped_process_is_unhealthy():
    for state in (
        NodeState.STARTING,
        NodeState.RECONCILING,
        NodeState.READY,
        NodeState.DEGRADED,
        NodeState.STOPPING,
    ):
        status = node_in(state).status(
            kill_switch_engaged=False, reconciliation_green=True, data_fresh=True
        )
        assert report(status).healthy is True, state
    stopped = node_in(NodeState.STOPPED).status(
        kill_switch_engaged=False, reconciliation_green=True, data_fresh=True
    )
    assert report(stopped).healthy is False


def test_the_report_carries_why_it_is_not_ready():
    node = node_in(NodeState.READY)
    node.degrade("websocket gap")
    status = node.status(
        kill_switch_engaged=False, reconciliation_green=True, data_fresh=True
    )
    r = report(status)
    assert r.ready is False
    assert r.degraded_reasons == ("websocket gap",)
    assert r.to_dict()["degraded_reasons"] == ["websocket gap"]


def test_the_kill_switch_metric_is_derived_not_tracked():
    # A gauge read from the truth every time. A counter that drifted here
    # would tell a dashboard Live was enabled when it was not.
    node = node_in(NodeState.READY)
    engaged = node.status(
        kill_switch_engaged=True, reconciliation_green=True, data_fresh=True
    )
    out = metrics(engaged, {"kill_switch_state": 0, "orders_submitted_total": 7})
    assert out["kill_switch_state"] == 1, "the status wins over the counter"
    assert out["orders_submitted_total"] == 7
    assert set(out) == set(METRIC_NAMES)


def test_every_documented_metric_is_present_even_at_zero():
    # A metric absent from a scrape is indistinguishable from a scrape that
    # failed; an explicit zero is not.
    node = node_in(NodeState.READY)
    status = node.status(
        kill_switch_engaged=False, reconciliation_green=True, data_fresh=True
    )
    out = metrics(status, {})
    assert set(out) == set(METRIC_NAMES)
    assert all(isinstance(v, int) for v in out.values())
