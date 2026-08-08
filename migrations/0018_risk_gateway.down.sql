-- Reverse of 0018. `risk_events` returns to the generic 0007 shape, including
-- the 0009 grant, so that re-applying 0009 is not required to undo this.

DROP TRIGGER IF EXISTS risk_events_append_only ON risk_events;
DROP FUNCTION IF EXISTS risk_events_reject_mutation();
GRANT UPDATE, DELETE ON TABLE risk_events TO app;

DROP INDEX IF EXISTS risk_events_correlation_idx;
DROP INDEX IF EXISTS risk_events_one_gate_decision_per_intent;

ALTER TABLE risk_events
    DROP CONSTRAINT IF EXISTS risk_events_gate_decision_is_complete;

ALTER TABLE risk_events
    DROP COLUMN IF EXISTS evaluated_at,
    DROP COLUMN IF EXISTS reason_code,
    DROP COLUMN IF EXISTS denied_by_check,
    DROP COLUMN IF EXISTS decision,
    DROP COLUMN IF EXISTS limits_version,
    DROP COLUMN IF EXISTS correlation_id,
    DROP COLUMN IF EXISTS intent_ref;

DROP TABLE IF EXISTS risk_instrument_allowlist;
DROP TABLE IF EXISTS risk_limits;
