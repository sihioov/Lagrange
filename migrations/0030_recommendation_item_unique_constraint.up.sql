-- 0030: attach the already-built unique index as the named table constraint.

ALTER TABLE recommendation_items
    ADD CONSTRAINT recommendation_items_run_instrument_key
    UNIQUE USING INDEX recommendation_items_run_instrument_key;
