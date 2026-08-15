"""Health and readiness for the live node (plan Todo 41).

Design §15.2 (metrics). The two endpoints answer different questions and
conflating them is the classic operational mistake:

* **health** — is this process alive and its own machinery working? A node
  that is deliberately halted by the kill switch is HEALTHY: nothing is wrong
  with it. Reporting unhealthy would make an orchestrator restart it, which
  achieves nothing and loses the in-memory record of why it stopped.
* **readiness** — may this node trade right now? That is the question with all
  the safety in it, and it is false far more often than health is.
"""
from __future__ import annotations

from dataclasses import dataclass

from .lifecycle import NodeState, NodeStatus


@dataclass(frozen=True)
class HealthReport:
    healthy: bool
    ready: bool
    state: str
    account_id: str
    refusal: str | None
    degraded_reasons: tuple[str, ...]
    risk_green: bool = True
    execution_enabled: bool = True

    def to_dict(self) -> dict[str, object]:
        return {
            "healthy": self.healthy,
            "ready": self.ready,
            "state": self.state,
            "account_id": self.account_id,
            "refusal": self.refusal,
            "degraded_reasons": list(self.degraded_reasons),
            "risk_green": self.risk_green,
            "execution_enabled": self.execution_enabled,
        }


def report(status: NodeStatus) -> HealthReport:
    """Builds both answers from one status reading.

    From ONE reading deliberately: taking health and readiness from two
    separate observations lets them describe different moments, and an
    operator comparing them would be reasoning about a state that never
    existed.
    """
    # Unhealthy means the process itself is broken. A kill switch, a red
    # reconciliation and stale data are all reasons not to TRADE, not reasons
    # to restart the process -- and restarting on any of them would destroy
    # the state that explains them.
    healthy = status.state not in (NodeState.STOPPED,)
    return HealthReport(
        healthy=healthy,
        ready=status.may_submit(),
        state=status.state.value,
        account_id=status.account_id,
        refusal=(r.value if (r := status.refusal()) else None),
        degraded_reasons=status.degraded_reasons,
        risk_green=status.risk_green,
        execution_enabled=status.execution_enabled,
    )


#: The §15.2 trading metrics this node is responsible for emitting.
METRIC_NAMES = (
    "orders_submitted_total",
    "orders_rejected_total",
    "unknown_order_states",
    "reconciliation_mismatches",
    "stale_data_blocks",
    "kill_switch_state",
)


def metrics(status: NodeStatus, counters: dict[str, int]) -> dict[str, int]:
    """The metric set, with the gauge derived rather than tracked.

    `kill_switch_state` is a GAUGE of the current truth, so it is read from
    the status every time instead of being incremented somewhere and hoped to
    be right. A counter that drifted from reality here would tell a dashboard
    Live was enabled when it was not.
    """
    out = {name: int(counters.get(name, 0)) for name in METRIC_NAMES}
    out["kill_switch_state"] = 1 if status.kill_switch_engaged else 0
    return out
