-- 0018 Risk Gateway: versioned limits and the immutable per-intent decision.
-- Design §6.13 (the twelve ordered checks), §16 (fail-closed), §15.2 metrics;
-- plan Todo 38. Approved decision: "persisted Live gatekeeper state before any
-- KIS submission path ... must survive restart and remain blocking until green".
--
-- This EXTENDS 0007's `risk_events` rather than introducing a parallel table.
-- 0007 created it as the generic risk/audit sink; what it lacks is everything
-- that makes a gate decision defensible after the fact:
--
--   * no link to the intent the decision was about, so "exactly one decision
--     per intent" could not be enforced and a second, contradictory decision
--     could be written without anything noticing;
--   * no record of WHICH limits produced the verdict, so a decision could not
--     be re-derived once limits changed;
--   * no correlation id, so a denial could not be joined to the audit log and
--     the alert it raised;
--   * and, most importantly, `app` held UPDATE and DELETE (0009). A gate
--     decision that can be edited afterwards is not evidence of anything.

-- ---------------------------------------------------------------------------
-- Versioned risk limits. "limits_version" has to name something that exists.
-- ---------------------------------------------------------------------------
CREATE TABLE risk_limits (
    version               text PRIMARY KEY,
    owner_user_id         uuid REFERENCES users (id) ON DELETE RESTRICT,

    -- §6.13 checks 7-10. Money is scale-4 numeric, never float: these are
    -- compared for ordering and a float comparison can approve an order that
    -- exceeds a limit by a rounding error.
    max_symbol_weight_bp  int            NOT NULL,
    max_order_value       numeric(18, 4) NOT NULL,
    max_daily_order_value numeric(18, 4) NOT NULL,
    max_daily_loss        numeric(18, 4) NOT NULL,

    -- §6.13 check 3. Older than this and the order is blocked (AT-08).
    max_data_age_secs     int            NOT NULL,

    created_at            timestamptz    NOT NULL DEFAULT now(),
    created_by            uuid REFERENCES users (id),
    note                  text,

    -- Every limit is a bound on something that cannot sensibly be negative,
    -- and a zero max_data_age would block every order forever, which is safe
    -- but is a misconfiguration rather than a policy.
    CONSTRAINT risk_limits_weight_range CHECK (max_symbol_weight_bp > 0 AND max_symbol_weight_bp <= 10000),
    CONSTRAINT risk_limits_order_value_positive CHECK (max_order_value > 0),
    CONSTRAINT risk_limits_daily_order_value_positive CHECK (max_daily_order_value > 0),
    CONSTRAINT risk_limits_daily_loss_positive CHECK (max_daily_loss > 0),
    CONSTRAINT risk_limits_data_age_positive CHECK (max_data_age_secs > 0)
);
CREATE INDEX risk_limits_owner_idx ON risk_limits (owner_user_id);

-- §6.13 check 6: the allowlist. A symbol absent from this table is denied, so
-- an empty allowlist denies everything -- the fail-closed direction.
CREATE TABLE risk_instrument_allowlist (
    owner_user_id uuid        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    instrument_id text        NOT NULL REFERENCES instruments (id),
    added_at      timestamptz NOT NULL DEFAULT now(),
    added_by      uuid REFERENCES users (id),
    PRIMARY KEY (owner_user_id, instrument_id)
);

-- ---------------------------------------------------------------------------
-- The decision record.
-- ---------------------------------------------------------------------------
ALTER TABLE risk_events
    ADD COLUMN intent_ref      text,
    ADD COLUMN correlation_id  text,
    ADD COLUMN limits_version  text REFERENCES risk_limits (version),
    ADD COLUMN decision        text,
    ADD COLUMN denied_by_check text,
    ADD COLUMN reason_code     text,
    ADD COLUMN evaluated_at    timestamptz;

-- A gate decision must carry the full set; any other risk event (rate limit,
-- operational notice) carries none of it. Half-populated rows are the shape a
-- partially-written decision would take, so they are refused outright.
ALTER TABLE risk_events
    ADD CONSTRAINT risk_events_gate_decision_is_complete CHECK (
        (event_type <> 'LIVE_ORDER_GATE')
        OR (
            intent_ref IS NOT NULL
            AND correlation_id IS NOT NULL
            AND limits_version IS NOT NULL
            AND decision IN ('APPROVED', 'DENIED')
            AND reason_code IS NOT NULL
            AND evaluated_at IS NOT NULL
            -- An approval denied by a check is a contradiction; a denial that
            -- names no check cannot be acted on.
            AND (decision = 'APPROVED') = (denied_by_check IS NULL)
        )
    );

-- Exactly one gate decision per intent, enforced by the database.
--
-- This is only coherent because an intent is evaluated ONCE: a denial
-- terminates that intent, and a retry is a NEW intent with a new correlation
-- id (the state machine Todo 39 implements). Were re-evaluation allowed, this
-- index would be wrong rather than merely strict.
CREATE UNIQUE INDEX risk_events_one_gate_decision_per_intent
    ON risk_events (intent_ref) WHERE event_type = 'LIVE_ORDER_GATE';
CREATE INDEX risk_events_correlation_idx ON risk_events (correlation_id);

-- ---------------------------------------------------------------------------
-- Immutability. 0009 granted `app` UPDATE and DELETE on risk_events; a gate
-- decision that can be rewritten after the order it authorised is not
-- evidence. Nothing writes this table yet (Todo 38 is its first writer), so
-- narrowing the grant now breaks nothing.
--
-- The REVOKE is the primary fence. The trigger is the second one, because a
-- future migration that re-grants in bulk -- 0009 granted this table by being
-- one name in a list of twenty -- would silently undo the REVOKE alone.
-- ---------------------------------------------------------------------------
REVOKE UPDATE, DELETE ON TABLE risk_events FROM app;

CREATE OR REPLACE FUNCTION risk_events_reject_mutation() RETURNS trigger
LANGUAGE plpgsql AS $fn$
BEGIN
    RAISE EXCEPTION
        'risk_events is append-only: % on a persisted risk decision is refused', TG_OP
        USING ERRCODE = 'insufficient_privilege';
END
$fn$;

CREATE TRIGGER risk_events_append_only
    BEFORE UPDATE OR DELETE ON risk_events
    FOR EACH ROW EXECUTE FUNCTION risk_events_reject_mutation();

-- ---------------------------------------------------------------------------
-- Grants and RLS for the new tables. Limits and the allowlist are read by the
-- gate on every evaluation and written only by an Owner-gated admin path.
-- ---------------------------------------------------------------------------
GRANT SELECT, INSERT ON TABLE risk_limits, risk_instrument_allowlist TO app;
GRANT SELECT ON TABLE risk_limits, risk_instrument_allowlist TO worker;
GRANT SELECT ON TABLE risk_limits, risk_instrument_allowlist TO admin;

ALTER TABLE risk_limits ENABLE ROW LEVEL SECURITY;
ALTER TABLE risk_limits FORCE ROW LEVEL SECURITY;
ALTER TABLE risk_instrument_allowlist ENABLE ROW LEVEL SECURITY;
ALTER TABLE risk_instrument_allowlist FORCE ROW LEVEL SECURITY;

DO $rls$
DECLARE
  t text;
BEGIN
  FOREACH t IN ARRAY ARRAY['risk_limits', 'risk_instrument_allowlist'] LOOP
    EXECUTE format(
      'CREATE POLICY owner_all_app_%s ON %I FOR ALL TO app USING (true) WITH CHECK (true)',
      t, t);
    EXECUTE format(
      'CREATE POLICY owner_all_migration_%s ON %I FOR ALL TO migration_owner USING (true) WITH CHECK (true)',
      t, t);
    EXECUTE format(
      'CREATE POLICY owner_select_admin_%s ON %I FOR SELECT TO admin USING (true)',
      t, t);
    EXECUTE format(
      'CREATE POLICY owner_select_worker_%s ON %I FOR SELECT TO worker USING (true)',
      t, t);
  END LOOP;
END
$rls$;
