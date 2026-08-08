"""What happens to working orders when the kill switch is engaged (Todo 41).

FR-LIVE-006: "Kill Switch 활성화 시 신규 주문은 즉시 거부되고 필요 시 취소
정책이 실행된다."

Two things happen on engage, and they are not the same thing:

1. **New intents are denied immediately.** That is unconditional, instant, and
   handled by the gate (check 1) plus `lifecycle.NodeStatus.refusal`. Nothing
   here is involved and nothing can defer it.
2. **Working orders are handled per policy.** That is *not* unconditional,
   because cancelling is itself an action against a live broker, and the right
   answer differs by situation.

The policies below are the configured choices. The important one is what NONE
of them do: **an UNKNOWN order is never cancelled.** We do not know its broker
order number — that is what UNKNOWN means — so a cancel would either fail or,
worse, name the wrong order. It has to be resolved by lookup first (Todo 39's
rule, arriving here intact).
"""
from __future__ import annotations

from dataclasses import dataclass
from enum import Enum


class CancelPolicy(str, Enum):
    """What to do with orders already working at the broker."""

    LEAVE = "LEAVE"
    """Cancel nothing. The default, and the right choice when the kill switch
    was pulled because our own system is misbehaving: an order already at the
    broker is under the broker's control and is not the thing going wrong."""

    CANCEL_WORKING = "CANCEL_WORKING"
    """Cancel every order the broker is working for us. For "get me flat and
    stop", where leaving resting orders to fill unattended is the risk."""

    CANCEL_UNFILLED_ONLY = "CANCEL_UNFILLED_ONLY"
    """Cancel only orders with nothing filled yet. A partially filled order is
    left alone because cancelling one strands a position that no strategy is
    now managing, which is usually worse than letting it complete."""


class OrderDisposition(str, Enum):
    CANCEL = "CANCEL"
    LEAVE = "LEAVE"
    RESOLVE_FIRST = "RESOLVE_FIRST"
    """Cannot be acted on until a broker lookup says what it is."""


@dataclass(frozen=True)
class WorkingOrder:
    """An order as the node believes it stands."""

    intent_ref: str
    state: str
    """A `kis_client::order_state::OrderIntentState` name."""
    broker_order_no: str | None
    cumulative_filled: int = 0


#: States that mean an order may be sitting at the broker right now.
_WORKING = frozenset({"ACCEPTED", "PARTIALLY_FILLED"})

#: States where we do not know whether an order exists.
_UNRESOLVED = frozenset({"SUBMITTING", "SUBMITTED", "UNKNOWN"})


def disposition(order: WorkingOrder, policy: CancelPolicy) -> OrderDisposition:
    """What the policy says to do with one order.

    Pure and total, so the whole kill-switch cancel sweep is a decision that
    can be reviewed after the fact from the order list alone.
    """
    # Unresolved first, ahead of every policy. An order whose broker number we
    # do not have cannot be cancelled by any policy: the cancel would name
    # nothing, or name the wrong order. Todo 39 forbids acting on UNKNOWN
    # before a lookup, and that rule does not weaken because a kill switch was
    # pulled -- it matters MORE then.
    if order.state in _UNRESOLVED or order.broker_order_no is None:
        return OrderDisposition.RESOLVE_FIRST

    if order.state not in _WORKING:
        # Terminal, or never reached the broker. Nothing to cancel.
        return OrderDisposition.LEAVE

    if policy is CancelPolicy.LEAVE:
        return OrderDisposition.LEAVE
    if policy is CancelPolicy.CANCEL_WORKING:
        return OrderDisposition.CANCEL
    if policy is CancelPolicy.CANCEL_UNFILLED_ONLY:
        # A partial fill is a position someone now holds. Cancelling the rest
        # strands it outside any strategy's management, which is usually worse
        # than letting the order finish.
        return (
            OrderDisposition.LEAVE
            if order.cumulative_filled > 0
            else OrderDisposition.CANCEL
        )
    raise ValueError(f"unhandled cancel policy {policy}")


@dataclass(frozen=True)
class CancelPlan:
    """The full sweep, decided before anything is sent."""

    to_cancel: tuple[str, ...]
    to_leave: tuple[str, ...]
    to_resolve: tuple[str, ...]

    @property
    def requires_lookup(self) -> bool:
        return bool(self.to_resolve)


def plan(orders: list[WorkingOrder], policy: CancelPolicy) -> CancelPlan:
    """Decides the whole sweep up front.

    Deciding everything before sending anything means the plan is auditable,
    and a failure partway through does not leave the operator guessing which
    orders were already acted on.
    """
    cancel: list[str] = []
    leave: list[str] = []
    resolve: list[str] = []
    for order in orders:
        match disposition(order, policy):
            case OrderDisposition.CANCEL:
                cancel.append(order.intent_ref)
            case OrderDisposition.RESOLVE_FIRST:
                resolve.append(order.intent_ref)
            case OrderDisposition.LEAVE:
                leave.append(order.intent_ref)
    return CancelPlan(
        to_cancel=tuple(cancel), to_leave=tuple(leave), to_resolve=tuple(resolve)
    )
