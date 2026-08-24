SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

LOCK TABLE public.owner_beta_recommendation_runs IN ACCESS EXCLUSIVE MODE;

-- Pre-0051 successful rows have no durable target hash or cash result. Those
-- values cannot be reconstructed honestly here. Older non-success rows were
-- also allowed to carry a factor hash, which the new atomic result contract
-- forbids.
-- The table is access-exclusively locked above, so the owner can inspect every
-- row without leaving tenant identity in the pooled migration connection.
ALTER TABLE public.owner_beta_recommendation_runs NO FORCE ROW LEVEL SECURITY;
DO $legacy_guard$
BEGIN
    IF EXISTS (
        SELECT 1
          FROM public.owner_beta_recommendation_runs AS run
         WHERE run.status = 'SUCCEEDED'
            OR run.factor_snapshot_sha256 IS NOT NULL
    ) THEN
        RAISE EXCEPTION 'owner beta target publication migration requires unpublished runs'
            USING ERRCODE = '55000';
    END IF;
END
$legacy_guard$;
ALTER TABLE public.owner_beta_recommendation_runs FORCE ROW LEVEL SECURITY;

ALTER TABLE public.owner_beta_recommendation_runs
    DROP CONSTRAINT owner_beta_recommendation_runs_success_factor_check,
    ADD COLUMN target_snapshot_sha256 text,
    ADD COLUMN cash_weight numeric(18, 6),
    ADD CONSTRAINT owner_beta_recommendation_runs_target_hash_check CHECK (
        target_snapshot_sha256 IS NULL
        OR target_snapshot_sha256 ~ '^sha256:[0-9a-f]{64}$'
    ),
    ADD CONSTRAINT owner_beta_recommendation_runs_cash_weight_check CHECK (
        cash_weight IS NULL OR (cash_weight >= 0 AND cash_weight <= 1)
    ),
    ADD CONSTRAINT owner_beta_recommendation_runs_result_state_check CHECK (
        (
            status = 'SUCCEEDED'
            AND factor_snapshot_sha256 IS NOT NULL
            AND target_snapshot_sha256 IS NOT NULL
            AND cash_weight IS NOT NULL
            AND error_code IS NULL
        )
        OR (
            status <> 'SUCCEEDED'
            AND factor_snapshot_sha256 IS NULL
            AND target_snapshot_sha256 IS NULL
            AND cash_weight IS NULL
        )
    );

GRANT UPDATE (
    target_snapshot_sha256, cash_weight
) ON public.owner_beta_recommendation_runs TO worker;
