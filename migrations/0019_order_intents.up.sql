-- 0019 Live order intents: one identity, one order, an append-only history.
-- Design §6.12, §16; requirements FR-LIVE-003, AT-09; plan Todo 39.
--
-- `intent_ref` is THE client-side identity of a Live order. The same string
-- appears in `risk_events.intent_ref` (0018) and, once the broker
-- acknowledges, in `orders.order_ref` (0007, already documented there as the
-- "idempotent per-account order intent"). Three tables, one identity.
--
-- It is GLOBALLY unique, not unique per account, and it is server-generated.
-- 0018's index (`ON risk_events (intent_ref) WHERE event_type =
-- 'LIVE_ORDER_GATE'`) carries no account column, so two accounts that chose
-- the same ref would both create intents while only one could ever record a
-- gate decision — the other would fail persistence and be graded CRITICAL for
-- what is really a naming collision. Server-generated uniqueness removes the
-- possibility rather than detecting it.

CREATE TABLE order_intents (
    intent_ref     text PRIMARY KEY,
    owner_user_id  uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    account_id     uuid NOT NULL REFERENCES accounts (id) ON DELETE RESTRICT,
    instrument_id  text NOT NULL REFERENCES instruments (id),
    side           text NOT NULL,
    quantity       numeric(18, 4) NOT NULL,
    price          numeric(18, 4),          -- NULL = market order
    correlation_id text NOT NULL,

    -- The state machine (`kis_client::order_state::OrderIntentState`). These
    -- strings are that enum's `name()`; renaming one there orphans every row
    -- here, which is why both sides call them stable.
    state          text NOT NULL DEFAULT 'INTENT_CREATED',
    broker_order_no text,
    cumulative_filled numeric(18, 4) NOT NULL DEFAULT 0,
    state_reason   text,

    created_at     timestamptz NOT NULL DEFAULT now(),
    updated_at     timestamptz NOT NULL DEFAULT now(),

    CONSTRAINT order_intents_side_check CHECK (side IN ('BUY', 'SELL')),
    CONSTRAINT order_intents_state_check CHECK (state IN (
        'INTENT_CREATED', 'RISK_APPROVED', 'SUBMITTING', 'SUBMITTED',
        'ACCEPTED', 'REJECTED', 'UNKNOWN', 'PARTIALLY_FILLED',
        'FILLED', 'CANCELED', 'EXPIRED', 'DENIED'
    )),
    -- A state that names a broker order must have one, and one that cannot
    -- have one must not. A row claiming ACCEPTED with no order number is a
    -- row nobody can reconcile against the broker.
    CONSTRAINT order_intents_broker_order_matches_state CHECK (
        (state IN ('ACCEPTED', 'PARTIALLY_FILLED', 'FILLED', 'CANCELED', 'EXPIRED')
             AND broker_order_no IS NOT NULL)
        OR (state IN ('INTENT_CREATED', 'RISK_APPROVED', 'SUBMITTING', 'SUBMITTED',
                      'REJECTED', 'UNKNOWN', 'DENIED'))
    ),
    CONSTRAINT order_intents_quantity_positive CHECK (quantity > 0),
    CONSTRAINT order_intents_price_positive CHECK (price IS NULL OR price > 0),
    CONSTRAINT order_intents_fill_within_quantity CHECK (
        cumulative_filled >= 0 AND cumulative_filled <= quantity
    )
);
CREATE INDEX order_intents_owner_idx ON order_intents (owner_user_id);
CREATE INDEX order_intents_account_idx ON order_intents (account_id);

-- One broker order per intent, and one intent per broker order. Both
-- directions matter: the first stops a duplicate submission being recorded,
-- the second stops two intents claiming the same broker order during
-- reconciliation (Todo 40 reads this).
CREATE UNIQUE INDEX order_intents_one_broker_order
    ON order_intents (broker_order_no) WHERE broker_order_no IS NOT NULL;

-- Intents that cannot be left alone: UNKNOWN needs a broker lookup, and
-- SUBMITTING/SUBMITTED are in flight. Reconciliation scans this.
CREATE INDEX order_intents_unresolved_idx ON order_intents (state)
    WHERE state IN ('SUBMITTING', 'SUBMITTED', 'UNKNOWN');

-- ---------------------------------------------------------------------------
-- The event log. This is the durable truth; `order_intents.state` is a
-- derived cache of it, so a disagreement is resolved by replaying the log.
-- ---------------------------------------------------------------------------
CREATE TABLE order_intent_events (
    id           bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    intent_ref   text NOT NULL REFERENCES order_intents (intent_ref) ON DELETE RESTRICT,
    owner_user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    seq          int NOT NULL,
    event_type   text NOT NULL,
    payload_json jsonb NOT NULL DEFAULT '{}'::jsonb,
    -- The state this event produced, so the log can be read without replaying
    -- it. Redundant BY DESIGN: a mismatch between this and a replay is a
    -- detectable corruption rather than an invisible one.
    resulting_state text NOT NULL,
    recorded_at  timestamptz NOT NULL DEFAULT now(),

    CONSTRAINT order_intent_events_seq_positive CHECK (seq > 0),
    CONSTRAINT order_intent_events_type_check CHECK (event_type IN (
        'RISK_APPROVED', 'RISK_DENIED', 'SUBMISSION_STARTED', 'SUBMISSION_SENT',
        'BROKER_ACCEPTED', 'BROKER_REJECTED', 'SUBMISSION_TIMED_OUT', 'FILL',
        'BROKER_CANCELED', 'BROKER_EXPIRED', 'BROKER_LOOKUP_RESOLVED'
    )),
    -- Gapless, strictly increasing per intent. A gap would mean an event was
    -- lost, and the replay that derives state would silently produce a
    -- different answer.
    CONSTRAINT order_intent_events_seq_unique UNIQUE (intent_ref, seq)
);
CREATE INDEX order_intent_events_intent_idx ON order_intent_events (intent_ref, seq);
CREATE INDEX order_intent_events_owner_idx ON order_intent_events (owner_user_id);

-- ---------------------------------------------------------------------------
-- Append-only, the 0018 fence. `order_intents` itself is deliberately NOT
-- fenced: its state legitimately changes, and its guarantee is that only
-- legal transitions happen (enforced above this layer by the state machine,
-- and by the CHECK constraints here). The EVENTS are the history, and history
-- does not change.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION order_intent_events_reject_mutation() RETURNS trigger
LANGUAGE plpgsql AS $fn$
BEGIN
    RAISE EXCEPTION
        'order_intent_events is append-only: % on a recorded order event is refused', TG_OP
        USING ERRCODE = 'insufficient_privilege';
END
$fn$;

CREATE TRIGGER order_intent_events_append_only
    BEFORE UPDATE OR DELETE ON order_intent_events
    FOR EACH ROW EXECUTE FUNCTION order_intent_events_reject_mutation();

-- ---------------------------------------------------------------------------
-- Grants and RLS. Both are tenant tables.
-- ---------------------------------------------------------------------------
GRANT SELECT, INSERT, UPDATE ON TABLE order_intents TO app;
GRANT SELECT, INSERT ON TABLE order_intent_events TO app;
GRANT SELECT ON TABLE order_intents, order_intent_events TO worker;
GRANT SELECT ON TABLE order_intents, order_intent_events TO admin;

ALTER TABLE order_intents ENABLE ROW LEVEL SECURITY;
ALTER TABLE order_intents FORCE ROW LEVEL SECURITY;
ALTER TABLE order_intent_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE order_intent_events FORCE ROW LEVEL SECURITY;

DO $rls$
DECLARE
  t text;
BEGIN
  FOREACH t IN ARRAY ARRAY['order_intents', 'order_intent_events'] LOOP
    EXECUTE format(
      'CREATE POLICY tenant_all_app_%s ON %I FOR ALL TO app '
      'USING (owner_user_id = current_setting(''app.actor_user_id'', true)::uuid) '
      'WITH CHECK (owner_user_id = current_setting(''app.actor_user_id'', true)::uuid)',
      t, t);
    EXECUTE format(
      'CREATE POLICY tenant_all_migration_%s ON %I FOR ALL TO migration_owner '
      'USING (owner_user_id = current_setting(''app.actor_user_id'', true)::uuid) '
      'WITH CHECK (owner_user_id = current_setting(''app.actor_user_id'', true)::uuid)',
      t, t);
    EXECUTE format(
      'CREATE POLICY tenant_select_admin_%s ON %I FOR SELECT TO admin USING (true)',
      t, t);
    EXECUTE format(
      'CREATE POLICY tenant_select_worker_%s ON %I FOR SELECT TO worker USING (true)',
      t, t);
  END LOOP;
END
$rls$;
