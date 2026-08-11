-- no-transaction
-- 0028: durable at-most-once identity for scheduled recommendations.

CREATE UNIQUE INDEX CONCURRENTLY recommendation_runs_scheduled_identity_uq
    ON recommendation_runs (
        owner_user_id,
        strategy_config_id,
        as_of,
        dataset_version_id
    )
    WHERE trigger_kind = 'SCHEDULED';
