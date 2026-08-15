"""Live node lifecycle (plan Todo 41).

Design §6.12 and §16; requirements FR-LIVE-004 and FR-LIVE-006.

The node's states, and the one rule that shapes them all:

    STARTING -> RECONCILING -> READY -> STOPPING -> STOPPED
                     ^                      |
                     +----------------------+  (degraded / kill switch)

**A node never starts READY.** It starts in RECONCILING and can only reach
READY by a reconciliation that actually came back green. That is what makes a
crash safe: a process that died mid-order comes back up unable to trade, and
has to re-establish agreement with the broker before it may place anything.
"Resume where we left off" is the behaviour this module exists to prevent.

Everything here is pure. The node's real work (NautilusTrader, sockets,
databases) happens around it, but the decision of whether this node may submit
an order is a function of recorded state, so it can be tested without a broker
and reproduced from a log after an incident.
"""
from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum


class NodeState(str, Enum):
    """Where the node is in its life."""

    STARTING = "STARTING"
    """Process is up; nothing has been checked yet."""

    RECONCILING = "RECONCILING"
    """Comparing our books with the broker's. Cannot submit."""

    READY = "READY"
    """Reconciled green and permitted to trade."""

    DEGRADED = "DEGRADED"
    """Running, but something is wrong: stale data, a socket gap, a database
    failure. Cannot submit, and returns to RECONCILING rather than straight to
    READY, because whatever went wrong may have happened while orders were in
    flight."""

    STOPPING = "STOPPING"
    """Graceful stop in progress; no new submissions, existing state flushed."""

    STOPPED = "STOPPED"
    """Terminal for this process."""


class Reason(str, Enum):
    """Why the node may not submit. Every one of these is a refusal."""

    NOT_READY = "NODE_NOT_READY"
    KILL_SWITCH = "LIVE_KILL_SWITCH_ENGAGED"
    RISK_GATE = "LIVE_RISK_GATE_BLOCKED"
    NOT_RECONCILED = "LIVE_RECONCILIATION_REQUIRED"
    STALE_DATA = "DATA_STALE"
    DRY_RUN = "LIVE_DRY_RUN"
    STOPPING = "NODE_STOPPING"
    DEGRADED = "NODE_DEGRADED"


@dataclass(frozen=True)
class NodeStatus:
    """Everything that decides whether this node may act.

    Frozen: a status is a reading taken at a moment, and a mutable one invites
    a caller to "fix" the reading instead of the condition.
    """

    state: NodeState
    kill_switch_engaged: bool
    reconciliation_green: bool
    data_fresh: bool
    account_id: str
    """The single account this process owns. One node, one account, ever."""
    # These defaults preserve the original pure lifecycle contract for callers
    # that do not model the wider runtime yet. The production simulator passes
    # both explicitly; an absent/blocked risk gate can never grant admission.
    risk_green: bool = True
    execution_enabled: bool = True
    degraded_reasons: tuple[str, ...] = field(default=())

    def may_submit(self) -> bool:
        """Whether an order may be placed right now."""
        return self.refusal() is None

    def refusal(self) -> Reason | None:
        """Why not, in the order an operator most needs to hear it.

        The kill switch is reported FIRST even though `state` would also
        refuse, because "someone pulled the switch" and "the node is still
        reconciling" call for completely different responses, and telling an
        operator the second when the first is true wastes the minutes that
        matter.
        """
        if self.kill_switch_engaged:
            return Reason.KILL_SWITCH
        if self.state is NodeState.STOPPING or self.state is NodeState.STOPPED:
            return Reason.STOPPING
        if self.state is NodeState.DEGRADED:
            return Reason.DEGRADED
        if not self.risk_green:
            return Reason.RISK_GATE
        if not self.reconciliation_green:
            return Reason.NOT_RECONCILED
        if not self.data_fresh:
            return Reason.STALE_DATA
        if not self.execution_enabled:
            return Reason.DRY_RUN
        if self.state is not NodeState.READY:
            return Reason.NOT_READY
        return None


class NodeLifecycle:
    """The node's state machine.

    Transitions are explicit and total: an illegal one raises rather than
    being ignored, because a node that silently stayed in a state it thought
    it had left is a node whose logs cannot be trusted afterwards.
    """

    #: Legal transitions. STARTING never goes directly to READY - that edge's
    #: absence IS the "no trading before reconciliation" rule.
    _LEGAL: dict[NodeState, frozenset[NodeState]] = {
        NodeState.STARTING: frozenset({NodeState.RECONCILING, NodeState.STOPPING}),
        NodeState.RECONCILING: frozenset(
            {NodeState.READY, NodeState.DEGRADED, NodeState.STOPPING}
        ),
        NodeState.READY: frozenset(
            {NodeState.RECONCILING, NodeState.DEGRADED, NodeState.STOPPING}
        ),
        # Degraded returns to RECONCILING, never straight to READY: whatever
        # degraded us may have happened while orders were in flight, so
        # agreement has to be re-established, not assumed.
        NodeState.DEGRADED: frozenset({NodeState.RECONCILING, NodeState.STOPPING}),
        NodeState.STOPPING: frozenset({NodeState.STOPPED}),
        NodeState.STOPPED: frozenset(),
    }

    def __init__(self, account_id: str) -> None:
        if not account_id:
            raise ValueError("a live node must own exactly one account")
        self._account_id = account_id
        self._state = NodeState.STARTING
        self._degraded_reasons: tuple[str, ...] = ()

    @property
    def state(self) -> NodeState:
        return self._state

    @property
    def account_id(self) -> str:
        return self._account_id

    def to(self, target: NodeState, *, reason: str | None = None) -> NodeState:
        """Moves to `target`, or raises if the edge does not exist."""
        allowed = self._LEGAL[self._state]
        if target not in allowed:
            raise IllegalTransition(self._state, target)
        if target is NodeState.DEGRADED and reason:
            self._degraded_reasons = (*self._degraded_reasons, reason)
        if target is NodeState.READY:
            # Reaching READY clears the degraded history: it is the record of
            # why we could not trade, and we can.
            self._degraded_reasons = ()
        self._state = target
        return self._state

    def degrade(self, reason: str) -> NodeState:
        """Marks the node degraded. Idempotent, because the same fault can be
        reported by several subsystems at once and the second report is not an
        error."""
        if self._state is NodeState.DEGRADED:
            self._degraded_reasons = (*self._degraded_reasons, reason)
            return self._state
        return self.to(NodeState.DEGRADED, reason=reason)

    def status(
        self,
        *,
        kill_switch_engaged: bool,
        reconciliation_green: bool,
        data_fresh: bool,
        risk_green: bool = True,
        execution_enabled: bool = True,
    ) -> NodeStatus:
        return NodeStatus(
            state=self._state,
            kill_switch_engaged=kill_switch_engaged,
            reconciliation_green=reconciliation_green,
            data_fresh=data_fresh,
            risk_green=risk_green,
            execution_enabled=execution_enabled,
            account_id=self._account_id,
            degraded_reasons=self._degraded_reasons,
        )


class IllegalTransition(RuntimeError):
    """A transition the machine does not have an edge for."""

    def __init__(self, current: NodeState, target: NodeState) -> None:
        super().__init__(f"{current.value} cannot become {target.value}")
        self.current = current
        self.target = target


def resume_after_crash(account_id: str) -> NodeLifecycle:
    """The state a restarted process comes back in.

    Deliberately a named function rather than a comment on the constructor:
    "what happens after a crash" is the question someone will ask at 3am, and
    the answer is that the node comes back in STARTING and must reconcile,
    exactly like a first boot. There is no persisted state that could shortcut
    it, because a shortcut is precisely what would let a node resume placing
    orders against books it has not re-checked.
    """
    return NodeLifecycle(account_id)
