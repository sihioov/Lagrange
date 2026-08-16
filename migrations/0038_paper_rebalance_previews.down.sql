-- 0038 rollback: refuse to discard active preview/application lineage, restore
-- the exact 0037 scheduled-only execution preflight, then remove new objects.

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

-- The rollback guard must inspect every tenant's recommendation lineage, while
-- the serving policies on both tables intentionally require an actor GUC.
-- Temporarily remove FORCE-RLS for this owner-only transactional check and
-- restore it immediately after the guard (or via transaction rollback on
-- failure).
ALTER TABLE public.paper_rebalance_previews NO FORCE ROW LEVEL SECURITY;
ALTER TABLE public.pending_targets NO FORCE ROW LEVEL SECURITY;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM public.paper_rebalance_previews
         WHERE status IN ('PENDING', 'RUNNING', 'READY')
    ) OR EXISTS (
        SELECT 1 FROM public.pending_targets
         WHERE source_kind <> 'LEGACY'
    ) THEN
        RAISE EXCEPTION 'preview rollback blocked: active preview or recommendation target lineage exists'
            USING ERRCODE = '55000';
    END IF;
END;
$$;

ALTER TABLE public.paper_rebalance_previews FORCE ROW LEVEL SECURITY;
ALTER TABLE public.pending_targets FORCE ROW LEVEL SECURITY;

REVOKE EXECUTE ON FUNCTION public.apply_paper_rebalance_preview(uuid, uuid, text, date)
    FROM app;
DROP FUNCTION public.apply_paper_rebalance_preview(uuid, uuid, text, date);

-- Restore the exact 0037 rule: every queued target requires the automatic
-- binding opt-in because 0037 has no manual recommendation source kind.
CREATE OR REPLACE FUNCTION public.preflight_paper_target(
    p_target_id uuid,
    p_owner_user_id uuid
) RETURNS TABLE (authorized boolean, reason jsonb)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    v_target record;
    v_dataset record;
    v_reason jsonb;
BEGIN
    IF p_target_id IS NULL OR p_owner_user_id IS NULL THEN
        RETURN QUERY SELECT false, pg_catalog.jsonb_build_object(
            'code', 'PAPER_TARGET_INVALID', 'message', 'Paper target identity is incomplete'
        );
        RETURN;
    END IF;

    PERFORM pg_catalog.set_config('app.actor_user_id', p_owner_user_id::text, true);
    SELECT target.account_id, target.strategy_config_id, target.computed_on,
           target.effective_date, target.dataset_version_id,
           target.dataset_manifest_sha256, target.status
      INTO v_target
      FROM public.pending_targets AS target
     WHERE target.id = p_target_id
       AND target.owner_user_id = p_owner_user_id
     FOR UPDATE OF target;
    IF NOT FOUND OR v_target.status <> 'PENDING' THEN
        RETURN QUERY SELECT false, pg_catalog.jsonb_build_object(
            'code', 'PAPER_TARGET_NOT_PENDING', 'message', 'Paper target is not pending'
        );
        RETURN;
    END IF;

    PERFORM 1
      FROM public.accounts AS account
      JOIN public.account_strategy_bindings AS binding
        ON binding.account_id = account.id
       AND binding.owner_user_id = account.owner_user_id
       AND binding.strategy_config_id = v_target.strategy_config_id
       AND binding.unbound_at IS NULL
       AND binding.auto_apply_recommendations
      JOIN public.user_strategy_configs AS config
        ON config.id = binding.strategy_config_id
       AND config.owner_user_id = binding.owner_user_id
       AND config.is_active
     WHERE account.id = v_target.account_id
       AND account.owner_user_id = p_owner_user_id
       AND account.account_type = 'PAPER'
       AND account.status = 'ACTIVE'
     FOR SHARE OF account, binding, config;
    IF NOT FOUND THEN
        v_reason := pg_catalog.jsonb_build_object(
            'code', 'PAPER_BINDING_INACTIVE',
            'message', 'Active opted-in Paper binding is no longer available'
        );
    ELSIF v_target.dataset_version_id IS NULL
       OR v_target.dataset_manifest_sha256 IS NULL THEN
        v_reason := pg_catalog.jsonb_build_object(
            'code', 'PAPER_DATA_LINEAGE_MISSING',
            'message', 'Queued target has no exact dataset lineage'
        );
    ELSE
        SELECT dataset.dataset_id, dataset.version, dataset.status,
               dataset.manifest_sha256
          INTO v_dataset
          FROM public.dataset_versions AS dataset
         WHERE dataset.id = v_target.dataset_version_id
         FOR SHARE OF dataset;
        IF NOT FOUND THEN
            v_reason := pg_catalog.jsonb_build_object(
                'code', 'PAPER_DATASET_MISSING',
                'message', 'Queued recommendation dataset no longer exists'
            );
        ELSIF v_dataset.status NOT IN ('READY', 'WARNING') THEN
            v_reason := pg_catalog.jsonb_build_object(
                'code', 'PAPER_DATASET_BLOCKED',
                'message', 'Queued recommendation dataset is blocked'
            );
        ELSIF v_dataset.manifest_sha256 IS DISTINCT FROM v_target.dataset_manifest_sha256
           OR v_dataset.version IS DISTINCT FROM (
                SELECT target.dataset_version FROM public.pending_targets AS target
                 WHERE target.id = p_target_id
           ) THEN
            v_reason := pg_catalog.jsonb_build_object(
                'code', 'PAPER_DATASET_LINEAGE_CHANGED',
                'message', 'Queued recommendation dataset lineage changed'
            );
        ELSE
            PERFORM 1
              FROM public.data_entitlements AS entitlement
             WHERE entitlement.status = 'ACTIVE'
               AND entitlement.effective_from <= v_target.effective_date
               AND entitlement.effective_until >= v_target.effective_date
               AND entitlement.covered_datasets
                   @> pg_catalog.jsonb_build_array(v_dataset.dataset_id)
               AND entitlement.covered_uses @> '["recommendation"]'::jsonb
             LIMIT 1
             FOR SHARE OF entitlement;
            IF NOT FOUND THEN
                v_reason := pg_catalog.jsonb_build_object(
                    'code', 'PAPER_ENTITLEMENT_INACTIVE',
                    'message', 'Recommendation entitlement is inactive for this session'
                );
            END IF;
        END IF;
    END IF;

    IF v_reason IS NOT NULL THEN
        UPDATE public.pending_targets
           SET status = 'SKIPPED', executed_at = pg_catalog.now(),
               non_execution_reason = v_reason
         WHERE id = p_target_id AND owner_user_id = p_owner_user_id
           AND status = 'PENDING';
        RETURN QUERY SELECT false, v_reason;
        RETURN;
    END IF;

    RETURN QUERY SELECT true, NULL::jsonb;
END;
$$;

ALTER FUNCTION public.preflight_paper_target(uuid, uuid) OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.preflight_paper_target(uuid, uuid)
    FROM PUBLIC, app, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.preflight_paper_target(uuid, uuid) TO worker;

REVOKE EXECUTE ON FUNCTION
    public.queue_scheduled_paper_targets(uuid, uuid, uuid, date, uuid, text, text, jsonb)
    FROM worker;
DROP FUNCTION public.queue_scheduled_paper_targets(uuid, uuid, uuid, date, uuid, text, text, jsonb);
GRANT EXECUTE ON FUNCTION
    public.queue_scheduled_paper_targets(uuid, uuid, date, uuid, text, text, jsonb)
    TO worker;

REVOKE EXECUTE ON FUNCTION public.fail_paper_rebalance_preview(uuid, uuid, jsonb)
    FROM worker;
DROP FUNCTION public.fail_paper_rebalance_preview(uuid, uuid, jsonb);

REVOKE EXECUTE ON FUNCTION public.publish_paper_rebalance_preview(uuid, uuid, bigint, text, text, integer, date, text, jsonb, jsonb)
    FROM worker;
DROP FUNCTION public.publish_paper_rebalance_preview(uuid, uuid, bigint, text, text, integer, date, text, jsonb, jsonb);

REVOKE EXECUTE ON FUNCTION public.snapshot_paper_rebalance_preview(uuid, uuid, date)
    FROM worker;
DROP FUNCTION public.snapshot_paper_rebalance_preview(uuid, uuid, date);

REVOKE EXECUTE ON FUNCTION public.lock_paper_rebalance_preview_submission(uuid, uuid, uuid, date)
    FROM app;
DROP FUNCTION public.lock_paper_rebalance_preview_submission(uuid, uuid, uuid, date);

DROP TABLE public.paper_rebalance_previews;

DROP TRIGGER pending_targets_protect_recommendation_origin
    ON public.pending_targets;
DROP FUNCTION public.pending_targets_protect_recommendation_origin();
DROP INDEX public.pending_targets_recommendation_run_idx;
ALTER TABLE public.pending_targets
    DROP CONSTRAINT pending_targets_recommendation_source_lineage_check,
    DROP CONSTRAINT pending_targets_source_kind_check,
    DROP COLUMN recommendation_run_id,
    DROP COLUMN source_kind;

DROP TRIGGER positions_bump_paper_state_version ON public.positions;
DROP TRIGGER cash_ledger_bump_paper_state_version ON public.cash_ledger;
DROP FUNCTION public.bump_paper_account_state_version();
ALTER TABLE public.positions
    DROP CONSTRAINT positions_account_owner_fkey;
ALTER TABLE public.cash_ledger
    DROP CONSTRAINT cash_ledger_account_owner_fkey;
ALTER TABLE public.accounts
    DROP CONSTRAINT accounts_paper_state_version_check,
    DROP CONSTRAINT accounts_id_owner_user_id_key,
    DROP COLUMN paper_state_version;
