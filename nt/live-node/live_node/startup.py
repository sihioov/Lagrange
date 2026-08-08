"""The startup sequence, in the one order that is safe (plan Todo 41).

Todo 40 documented a contract on `reconcile()` and left it for this module to
honour. It is worth restating why the order is not arbitrary, because every
wrong ordering fails silently rather than loudly.

    1. SWEEP in-flight intents to UNKNOWN
    2. APPLY fills the broker reports and we have not
    3. RECONCILE, and record that run
    4. READY only if the run came back green

**Sweep before reconcile.** An intent left in SUBMITTING is epistemically
identical to one in UNKNOWN: the broker may or may not hold the order. But
`reconcile()` deliberately flags only UNKNOWN, because flagging every in-flight
intent would make a runtime pass catch the ordinary submission window and flap
readiness. So after a crash, positions and cash can agree (the order may never
have landed), the pass returns GREEN, and the node goes READY while an intent
is still unresolved. The sweep closes that: `SubmissionTimedOut` is a legal
transition from SUBMITTING to UNKNOWN, and a swept intent then blocks and
demands the broker lookup that can actually settle it.

**Apply fills before reconcile.** A completed-while-offline order shows up as
BOTH an unapplied fill (auto-resolvable) and a position mismatch (blocking).
Applying the fill first makes the position agree, so the reconciliation run we
RECORD is the true one. Reconciling first would record a blocking mismatch
that the very next step resolves, leaving an operator with an alert about a
problem that no longer exists.

**Record the run we acted on.** The recorded run is the evidence for going
READY, so it must be the pass whose result we used -- not an earlier one.
"""
from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum


class StartupOutcome(str, Enum):
    """How startup finished."""

    READY = "READY"
    """Reconciled green; the node may trade."""

    BLOCKED = "BLOCKED"
    """Reconciliation found something. The node runs but cannot submit."""

    LOOKUPS_REQUIRED = "LOOKUPS_REQUIRED"
    """Intents whose state only the broker can settle. Also blocking, but
    distinguished because the next action is a lookup rather than a human
    decision."""


@dataclass
class StartupPlan:
    """What startup decided, before anything irreversible happens.

    Returned rather than executed so the sequence can be inspected in a test
    and in an incident review. The node's actual side effects -- appending
    events, applying fills, recording the run -- are performed by the caller
    against real stores, which is what keeps this module pure.
    """

    to_sweep: tuple[str, ...] = field(default=())
    """Intents to move to UNKNOWN before reconciling."""

    fills_to_apply: tuple[str, ...] = field(default=())
    """Execution ids to apply to the ledger before reconciling."""

    lookups_required: tuple[str, ...] = field(default=())
    """Intents needing a broker status lookup."""

    blocking_reasons: tuple[str, ...] = field(default=())
    """Mismatch kinds no automated pass may clear."""

    outcome: StartupOutcome = StartupOutcome.BLOCKED

    @property
    def may_trade(self) -> bool:
        return self.outcome is StartupOutcome.READY


#: States that mean an order may be in flight with an unknown outcome.
IN_FLIGHT = frozenset({"SUBMITTING", "SUBMITTED"})


def sweep_targets(intent_states: dict[str, str]) -> tuple[str, ...]:
    """Intents that must be swept to UNKNOWN before a startup reconciliation.

    `intent_states` maps `intent_ref` to an `OrderIntentState` name, as
    `OrderIntentRepo::unresolved` returns them.

    UNKNOWN is deliberately NOT swept: it is already where the sweep would put
    it, and appending a second `SubmissionTimedOut` would be an illegal
    transition -- which the repo would refuse, turning a safe restart into a
    startup failure.
    """
    return tuple(
        ref for ref, state in sorted(intent_states.items()) if state in IN_FLIGHT
    )


def plan_startup(
    intent_states: dict[str, str],
    *,
    mismatch_kinds: tuple[str, ...],
    fills_to_apply: tuple[str, ...],
    lookups_required: tuple[str, ...],
) -> StartupPlan:
    """Decides the startup sequence from the post-sweep reconciliation.

    `mismatch_kinds` are the kinds a `ReconciliationOutcome` reported that no
    automated pass may clear -- i.e. `blocking()`, already filtered by the
    caller, because "which kinds are auto-resolvable" is Todo 40's single
    definition and must not be restated here.
    """
    to_sweep = sweep_targets(intent_states)

    if lookups_required:
        # Reported ahead of the generic block: the next action is to ask the
        # broker, which an operator can do, rather than to make a judgement
        # call about a difference in the books.
        outcome = StartupOutcome.LOOKUPS_REQUIRED
    elif mismatch_kinds:
        outcome = StartupOutcome.BLOCKED
    else:
        outcome = StartupOutcome.READY

    return StartupPlan(
        to_sweep=to_sweep,
        fills_to_apply=tuple(fills_to_apply),
        lookups_required=tuple(lookups_required),
        blocking_reasons=tuple(mismatch_kinds),
        outcome=outcome,
    )
