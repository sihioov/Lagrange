-- no-transaction
-- 0029: build the populated recommendation-item uniqueness backing index.

CREATE UNIQUE INDEX CONCURRENTLY recommendation_items_run_instrument_key
    ON recommendation_items (recommendation_run_id, instrument_id);
