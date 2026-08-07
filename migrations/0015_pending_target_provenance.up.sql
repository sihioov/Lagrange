-- 0015: record the dataset a pending target was computed from (plan Todo 32).
--
-- Parity compares a Paper account's executed signals against the backtest's
-- for the SAME strategy/data/as-of inputs. Todo 31's pending_targets
-- captured strategy (via strategy_config_id) and as-of (computed_on) but
-- not the dataset version, so a parity report could not tell "same signals"
-- from "same signals computed off different data" -- exactly the
-- NOT_COMPARABLE case result_model::paper_parity exists to catch.
--
-- Nullable rather than NOT NULL: rows queued before this migration have no
-- honest value to backfill, and inventing one would let a parity report
-- claim comparability it cannot prove. A NULL reads as "unknown dataset"
-- and the report degrades to NOT_COMPARABLE, which is the correct
-- fail-closed behaviour.

ALTER TABLE pending_targets ADD COLUMN dataset_version text;

COMMENT ON COLUMN pending_targets.dataset_version IS
    'Dataset version the target was computed from; NULL means unknown (parity degrades to NOT_COMPARABLE).';
