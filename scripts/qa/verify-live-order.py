"""Cross-verifies a claimed Live order against what the SYSTEM recorded.

Exists because the first version of the Phase 3 gate could be talked into
APPROVED by two files containing arbitrary JSON. That is the precise failure
the gate is built to prevent -- "missing real credentials yields
BLOCKED_EXTERNAL_CREDENTIALS, never false approval" -- and accepting a
hand-written `{"reconciled": true}` as proof that a real order was placed
against a real brokerage account was a hole big enough to drive a release
through.

The rule this file enforces: **evidence is verified against state the system
itself produced, never against an assertion someone wrote down.** The claim
file supplies only an `intent_ref` -- a pointer. Everything that matters is
then read from the database:

  1. a `risk_events` row for that intent, decision APPROVED. The gate cannot
     have authorised an order it never assessed.
  2. an `order_intents` row for that intent, bound to a broker order number
     and in a state that means the broker really had it.
  3. a `reconciliation_runs` row, PASSED with zero mismatches, that FINISHED
     AFTER the order reached the broker. A green run from before the order
     proves nothing about it.

All three are rows this system writes as a side effect of actually trading.
Forging them means forging the audit trail in a database where `risk_events`
and `order_intent_events` are append-only and the app role holds no UPDATE or
DELETE grant -- which is the point.

Exit codes: 0 = verified; 1 = not verified (reason on stdout); 2 = could not
check (no database, unreadable claim).
"""
from __future__ import annotations

import json
import os
import subprocess
import sys


def fail(reason: str) -> int:
    print(reason)
    return 1


def _run(cmd: list[str]) -> tuple[int, str]:
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=30)
    except (FileNotFoundError, subprocess.TimeoutExpired) as exc:
        return 2, str(exc)
    return proc.returncode, (proc.stdout or "").strip()


def psql(database_url: str, sql: str) -> tuple[int, str]:
    """Runs one query, via a local psql or the QA container if there is none.

    The fallback matters more than it looks. Without it, a host with no psql
    on PATH -- which is this project's normal Windows lane -- could never
    verify anything, so a FALSE claim would come back as "could not check"
    instead of as a denial. Being unable to check is safe only if it also
    blocks approval, and it is much better to actually check.
    """
    rc, out = _run(["psql", database_url, "-At", "-c", sql])
    if rc != 2:
        return rc, out

    container = os.environ.get("LAGRANGE_QA_DB_CONTAINER", "lagrange-qa-qa-db-1")
    return _run(
        [
            "docker", "exec", "-i", container,
            "psql", "postgres://postgres:lagrange@127.0.0.1:5432/postgres",
            "-At", "-c", sql,
        ]
    )


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print("usage: verify-live-order.py <claim.json>")
        return 2
    try:
        claim = json.loads(open(argv[1], encoding="utf-8").read())
    except (OSError, json.JSONDecodeError) as exc:
        print(f"claim file is unreadable: {exc}")
        return 2

    intent_ref = claim.get("intent_ref")
    if not isinstance(intent_ref, str) or not intent_ref:
        # Deliberately NOT falling back to any other field. A claim that does
        # not name an intent cannot be cross-checked at all, and the whole
        # point is that the claim's own assertions are worthless.
        return fail("claim does not name an intent_ref; nothing to verify against")

    database_url = os.environ.get("DATABASE_URL", "")
    if not database_url:
        print("DATABASE_URL is not set; cannot verify")
        return 2

    ref = intent_ref.replace("'", "''")

    rc, approved = psql(
        database_url,
        "SELECT count(*) FROM risk_events "
        f"WHERE intent_ref = '{ref}' AND decision = 'APPROVED' "
        "AND event_type = 'LIVE_ORDER_GATE'",
    )
    if rc == 2:
        print(f"psql unavailable: {approved}")
        return 2
    if rc != 0 or approved != "1":
        return fail(
            f"no APPROVED risk_events row for intent {intent_ref}: "
            "the gate never authorised this order"
        )

    rc, row = psql(
        database_url,
        "SELECT state, coalesce(broker_order_no, '') FROM order_intents "
        f"WHERE intent_ref = '{ref}'",
    )
    if rc != 0 or not row:
        return fail(f"no order_intents row for intent {intent_ref}")
    state, broker_order_no = (row.split("|") + [""])[:2]
    if not broker_order_no:
        return fail(
            f"intent {intent_ref} is bound to no broker order; "
            "it never reached the broker"
        )
    if state not in {"ACCEPTED", "PARTIALLY_FILLED", "FILLED", "CANCELED", "EXPIRED"}:
        return fail(f"intent {intent_ref} is in state {state}; the broker never held it")

    rc, green_after = psql(
        database_url,
        "SELECT count(*) FROM reconciliation_runs r "
        "WHERE r.status = 'PASSED' AND r.mismatch_count = 0 "
        "AND r.finished_at IS NOT NULL "
        "AND r.finished_at > (SELECT o.created_at FROM order_intents o "
        f"                    WHERE o.intent_ref = '{ref}')",
    )
    if rc != 0 or green_after in ("", "0"):
        return fail(
            f"no green reconciliation finished AFTER intent {intent_ref}; "
            "a run from before the order proves nothing about it"
        )

    print(
        f"verified: intent {intent_ref} was gate-approved, reached the broker as "
        f"order {broker_order_no} (state {state}), and a green reconciliation "
        "followed it"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
