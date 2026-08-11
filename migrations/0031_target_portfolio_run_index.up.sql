-- no-transaction
-- 0031: non-blocking one-target-per-recommendation-run enforcement.

CREATE UNIQUE INDEX CONCURRENTLY target_portfolios_one_per_run
    ON target_portfolios (recommendation_run_id)
    WHERE recommendation_run_id IS NOT NULL;
