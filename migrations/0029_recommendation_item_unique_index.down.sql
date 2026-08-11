-- no-transaction
-- 0030 down normally removes the constraint-owned index first. IF EXISTS
-- also makes recovery safe when 0029 failed and left an invalid index behind.

DROP INDEX CONCURRENTLY IF EXISTS recommendation_items_run_instrument_key;
