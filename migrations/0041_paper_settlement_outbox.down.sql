SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

-- Never remove the only durable obligation for a terminal target. Operators
-- must drain/reconcile pending delivery before asking migrations to roll back.
DO $guard$
DECLARE
    v_owner_user_id uuid;
BEGIN
    IF EXISTS (
        SELECT 1 FROM public.paper_settlement_outbox
         WHERE delivered_at IS NULL
    ) THEN
        RAISE EXCEPTION 'Paper settlement rollback blocked while pending outbox obligations exist'
            USING ERRCODE = '55000';
    END IF;
    -- pending_targets is FORCE RLS and the migration_owner policy is
    -- actor-scoped. Inspect every tenant explicitly; a no-GUC query must not
    -- turn an RLS visibility gap into permission to roll back an orphan.
    FOR v_owner_user_id IN SELECT id FROM public.users LOOP
        PERFORM pg_catalog.set_config(
            'app.actor_user_id', v_owner_user_id::text, true
        );
        IF EXISTS (
            SELECT 1
              FROM public.pending_targets AS target
             WHERE target.owner_user_id = v_owner_user_id
               AND target.status <> 'PENDING'
               AND NOT EXISTS (
                   SELECT 1 FROM public.paper_settlement_outbox AS outbox
                    WHERE outbox.pending_target_id = target.id
               )
               AND NOT EXISTS (
                   SELECT 1 FROM public.paper_settlement_outbox_archive AS archive
                    WHERE archive.pending_target_id = target.id
               )
        ) THEN
            RAISE EXCEPTION 'Paper settlement rollback blocked by terminal target without durable obligation'
                USING ERRCODE = '55000';
        END IF;
    END LOOP;
END
$guard$;

DROP TRIGGER IF EXISTS pending_targets_require_settlement_outbox
    ON public.pending_targets;
DROP TRIGGER IF EXISTS paper_settlement_outbox_require_target_obligation
    ON public.paper_settlement_outbox;
DROP TRIGGER IF EXISTS paper_settlement_outbox_archive_require_target_obligation
    ON public.paper_settlement_outbox_archive;
DROP FUNCTION IF EXISTS public.assert_paper_settlement_obligation();

DROP FUNCTION IF EXISTS public.prune_paper_settlement_outbox(bigint, integer);
DROP FUNCTION IF EXISTS public.paper_settlement_outbox_stats(bigint);
DROP FUNCTION IF EXISTS public.fail_paper_settlement_outbox(uuid, uuid, text);
DROP FUNCTION IF EXISTS public.mark_paper_settlement_outbox_delivered(uuid, uuid);
DROP FUNCTION IF EXISTS public.enqueue_paper_settlement_outbox(uuid, text, text, text, text, jsonb);

REVOKE SELECT ON TABLE public.notification_subscriptions FROM worker;
DROP TABLE public.paper_settlement_outbox_archive;
DROP TABLE public.paper_settlement_outbox;

ALTER TABLE public.pending_targets
    DROP CONSTRAINT IF EXISTS pending_targets_recommendation_exact_lineage_fk,
    DROP CONSTRAINT IF EXISTS pending_targets_id_owner_uq;
ALTER TABLE public.recommendation_runs
    DROP CONSTRAINT IF EXISTS recommendation_runs_exact_lineage_uq;

ALTER TABLE public.notification_deliveries
    DROP CONSTRAINT IF EXISTS notification_deliveries_notification_owner_fk,
    DROP CONSTRAINT IF EXISTS notification_deliveries_notification_channel_uq;
ALTER TABLE public.notifications
    DROP CONSTRAINT IF EXISTS notifications_id_owner_uq;
DROP INDEX IF EXISTS public.notifications_owner_source_key_uq;
ALTER TABLE public.notifications
    DROP CONSTRAINT IF EXISTS notifications_source_key_shape_check,
    DROP COLUMN IF EXISTS source_key;
