-- 0041: durable Paper settlement obligations and idempotent notification
-- delivery.
--
-- A terminal pending_target is an auditable fact, not a best-effort event.
-- The deferred constraint triggers below require one durable active outbox row
-- (or an archived delivered row) for every terminal target at COMMIT.  The
-- target transition and the outbox insert can therefore be separate SQL
-- statements in one transaction, but neither can commit on its own.

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

-- Exact-lineage foreign-key validation must see every existing tenant row even
-- though FORCE RLS would otherwise make a no-GUC migration connection see
-- none.  These policies exist only for constraint validation and are removed
-- before the migration commits.
CREATE POLICY paper_settlement_migration_validate_recommendation_runs
    ON public.recommendation_runs FOR SELECT TO migration_owner USING (true);
CREATE POLICY paper_settlement_migration_validate_pending_targets
    ON public.pending_targets FOR SELECT TO migration_owner USING (true);

-- Older pending_targets policies cast an empty actor GUC directly to uuid.
-- A migration connection (and a freshly reset pool connection) legitimately
-- has no actor, so harden both serving policies before any 0041 backfill or
-- constraint validation can touch this FORCE-RLS table.  Keep the policy
-- names and tenant semantics unchanged while making the empty-GUC path fail
-- closed as an ordinary NULL comparison.
DROP POLICY tenant_all_app_pending_targets ON public.pending_targets;
DROP POLICY tenant_all_owner_pending_targets ON public.pending_targets;
CREATE POLICY tenant_all_app_pending_targets ON public.pending_targets
    FOR ALL TO app
    USING (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    )
    WITH CHECK (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    );
CREATE POLICY tenant_all_owner_pending_targets ON public.pending_targets
    FOR ALL TO migration_owner
    USING (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    )
    WITH CHECK (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    );

-- The exact-lineage validation policy below is permissive, but PostgreSQL may
-- still evaluate the pre-existing recommendation_runs policies while planning
-- constraint checks. Harden those policies too, so a no-actor migration never
-- parses an empty string as uuid while retaining the same tenant boundary.
DROP POLICY tenant_all_app_recommendation_runs ON public.recommendation_runs;
DROP POLICY tenant_all_owner_recommendation_runs ON public.recommendation_runs;
CREATE POLICY tenant_all_app_recommendation_runs ON public.recommendation_runs
    FOR ALL TO app
    USING (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    )
    WITH CHECK (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    );
CREATE POLICY tenant_all_owner_recommendation_runs ON public.recommendation_runs
    FOR ALL TO migration_owner
    USING (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    )
    WITH CHECK (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    );

-- The duplicate-delivery cleanup policy is likewise permissive. Harden the
-- serving policies it coexists with before the cleanup DML runs under FORCE
-- RLS, including the empty-GUC path on a fresh migration connection.
DROP POLICY tenant_all_app_notification_deliveries
    ON public.notification_deliveries;
DROP POLICY tenant_all_owner_notification_deliveries
    ON public.notification_deliveries;
CREATE POLICY tenant_all_app_notification_deliveries
    ON public.notification_deliveries FOR ALL TO app
    USING (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    )
    WITH CHECK (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    );
CREATE POLICY tenant_all_owner_notification_deliveries
    ON public.notification_deliveries FOR ALL TO migration_owner
    USING (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    )
    WITH CHECK (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    );

-- Exact recommendation lineage is part of the target identity.  The
-- recommendation_run_id-only FK from 0038 is insufficient: a caller could
-- pair a run from one tenant/config/date with a target from another tenant.
ALTER TABLE public.recommendation_runs
    ADD CONSTRAINT recommendation_runs_exact_lineage_uq
    UNIQUE (
        id,
        owner_user_id,
        strategy_config_id,
        as_of,
        dataset_version_id,
        dataset_manifest_sha256
    );

ALTER TABLE public.pending_targets
    ADD CONSTRAINT pending_targets_id_owner_uq
    UNIQUE (id, owner_user_id),
    ADD CONSTRAINT pending_targets_recommendation_exact_lineage_fk
    FOREIGN KEY (
        recommendation_run_id,
        owner_user_id,
        strategy_config_id,
        computed_on,
        dataset_version_id,
        dataset_manifest_sha256
    ) REFERENCES public.recommendation_runs (
        id,
        owner_user_id,
        strategy_config_id,
        as_of,
        dataset_version_id,
        dataset_manifest_sha256
    );

DROP POLICY paper_settlement_migration_validate_pending_targets
    ON public.pending_targets;
DROP POLICY paper_settlement_migration_validate_recommendation_runs
    ON public.recommendation_runs;

ALTER TABLE public.notifications
    ADD COLUMN source_key text,
    ADD CONSTRAINT notifications_source_key_shape_check CHECK (
        source_key IS NULL
        OR (
            pg_catalog.length(source_key) BETWEEN 1 AND 128
            AND source_key !~ '[[:cntrl:]]'
        )
    );

CREATE UNIQUE INDEX notifications_owner_source_key_uq
    ON public.notifications (owner_user_id, source_key)
    WHERE source_key IS NOT NULL;

-- A delivery's tenant must be the notification's tenant.  The historical
-- single-column notification FK proved existence but allowed an app caller
-- to pair its own owner_user_id with another tenant's notification id.
ALTER TABLE public.notifications
    ADD CONSTRAINT notifications_id_owner_uq UNIQUE (id, owner_user_id);

-- Older installations did not have a uniqueness fence on deliveries.  Keep
-- the first immutable attempt for each channel before adding the fence.  The
-- table is FORCE RLS; make the cleanup's visibility explicit and temporary so
-- a migration connection with no tenant actor cannot silently leave duplicate
-- rows behind and fail later at the unique constraint.
CREATE POLICY paper_settlement_migration_cleanup_notification_deliveries
    ON public.notification_deliveries FOR ALL TO migration_owner
    USING (true) WITH CHECK (true);
DELETE FROM public.notification_deliveries AS duplicate
USING public.notification_deliveries AS keeper
WHERE duplicate.notification_id = keeper.notification_id
  AND duplicate.channel = keeper.channel
  AND duplicate.id > keeper.id;
DROP POLICY paper_settlement_migration_cleanup_notification_deliveries
    ON public.notification_deliveries;

ALTER TABLE public.notification_deliveries
    ADD CONSTRAINT notification_deliveries_notification_owner_fk
    FOREIGN KEY (notification_id, owner_user_id)
    REFERENCES public.notifications (id, owner_user_id) ON DELETE CASCADE;

ALTER TABLE public.notification_deliveries
    ADD CONSTRAINT notification_deliveries_notification_channel_uq
    UNIQUE (notification_id, channel);

-- A transport call can outlive the SQL transaction that records its result.
-- Keep a short durable lease on each channel row so concurrent runners do not
-- invoke the same source-key transport at the same time.  A lease expiry is
-- an explicit at-least-once boundary: a process killed after external send but
-- before the result update may be retried, while the source key still
-- prevents duplicate notification rows.
ALTER TABLE public.notification_deliveries
    ADD COLUMN delivery_token uuid,
    ADD COLUMN delivery_lease_expires_at timestamptz,
    ADD COLUMN delivery_attempts integer NOT NULL DEFAULT 0,
    ADD CONSTRAINT notification_deliveries_delivery_lease_check
        CHECK (
            (delivery_token IS NULL AND delivery_lease_expires_at IS NULL)
            OR (delivery_token IS NOT NULL AND delivery_lease_expires_at IS NOT NULL)
        ),
    ADD CONSTRAINT notification_deliveries_delivery_attempts_check
        CHECK (delivery_attempts >= 0);
CREATE INDEX notification_deliveries_delivery_lease_idx
    ON public.notification_deliveries (delivery_lease_expires_at)
    WHERE delivery_token IS NOT NULL;

CREATE TABLE public.paper_settlement_outbox (
    id                 uuid PRIMARY KEY DEFAULT pg_catalog.gen_random_uuid(),
    pending_target_id  uuid NOT NULL UNIQUE,
    owner_user_id      uuid NOT NULL,
    severity           text NOT NULL,
    kind               text NOT NULL,
    title              text NOT NULL,
    body               text NOT NULL DEFAULT '',
    parity_json        jsonb,
    attempts           integer NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    max_attempts       integer NOT NULL DEFAULT 8 CHECK (max_attempts BETWEEN 1 AND 32),
    available_at       timestamptz NOT NULL DEFAULT pg_catalog.now(),
    delivered_at       timestamptz,
    exhausted_at       timestamptz,
    last_error         text,
    claim_token        uuid,
    claim_expires_at   timestamptz,
    created_at         timestamptz NOT NULL DEFAULT pg_catalog.now(),
    CONSTRAINT paper_settlement_outbox_target_owner_fk
        FOREIGN KEY (pending_target_id, owner_user_id)
        REFERENCES public.pending_targets (id, owner_user_id) ON DELETE CASCADE,
    CONSTRAINT paper_settlement_outbox_severity_check
        CHECK (severity IN ('INFO', 'WARNING', 'CRITICAL')),
    CONSTRAINT paper_settlement_outbox_kind_check
        CHECK (kind IN ('job', 'recommendation', 'backtest', 'alert')),
    CONSTRAINT paper_settlement_outbox_parity_shape_check
        CHECK (parity_json IS NULL OR pg_catalog.jsonb_typeof(parity_json) = 'object'),
    CONSTRAINT paper_settlement_outbox_exhausted_check
        CHECK (exhausted_at IS NULL OR attempts >= max_attempts),
    CONSTRAINT paper_settlement_outbox_delivery_state_check
        CHECK (delivered_at IS NULL OR exhausted_at IS NULL),
    CONSTRAINT paper_settlement_outbox_claim_state_check
        CHECK (
            (claim_token IS NULL AND claim_expires_at IS NULL)
            OR (claim_token IS NOT NULL AND claim_expires_at IS NOT NULL)
        )
);

-- Delivered rows are prunable only after this immutable archive row exists.
-- The archive is the durable obligation that lets us retain a bounded active
-- outbox while keeping the terminal-target invariant true.
CREATE TABLE public.paper_settlement_outbox_archive (
    id                 uuid PRIMARY KEY,
    pending_target_id  uuid NOT NULL UNIQUE,
    owner_user_id      uuid NOT NULL,
    severity           text NOT NULL,
    kind               text NOT NULL,
    title              text NOT NULL,
    body               text NOT NULL,
    parity_json        jsonb,
    attempts           integer NOT NULL CHECK (attempts >= 0),
    max_attempts       integer NOT NULL CHECK (max_attempts BETWEEN 1 AND 32),
    delivered_at       timestamptz NOT NULL,
    created_at         timestamptz NOT NULL,
    archived_at        timestamptz NOT NULL DEFAULT pg_catalog.now(),
    CONSTRAINT paper_settlement_outbox_archive_target_owner_fk
        FOREIGN KEY (pending_target_id, owner_user_id)
        REFERENCES public.pending_targets (id, owner_user_id) ON DELETE CASCADE,
    CONSTRAINT paper_settlement_outbox_archive_severity_check
        CHECK (severity IN ('INFO', 'WARNING', 'CRITICAL')),
    CONSTRAINT paper_settlement_outbox_archive_kind_check
        CHECK (kind IN ('job', 'recommendation', 'backtest', 'alert')),
    CONSTRAINT paper_settlement_outbox_archive_parity_shape_check
        CHECK (parity_json IS NULL OR pg_catalog.jsonb_typeof(parity_json) = 'object')
);

CREATE INDEX paper_settlement_outbox_pending_idx
    ON public.paper_settlement_outbox (available_at, id)
    WHERE delivered_at IS NULL AND exhausted_at IS NULL;
CREATE INDEX paper_settlement_outbox_claim_idx
    ON public.paper_settlement_outbox (claim_expires_at, available_at, id)
    WHERE delivered_at IS NULL AND exhausted_at IS NULL;
CREATE INDEX paper_settlement_outbox_owner_idx
    ON public.paper_settlement_outbox (owner_user_id, created_at);
CREATE INDEX paper_settlement_outbox_archive_owner_idx
    ON public.paper_settlement_outbox_archive (owner_user_id, archived_at);

ALTER TABLE public.paper_settlement_outbox ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.paper_settlement_outbox FORCE ROW LEVEL SECURITY;
ALTER TABLE public.paper_settlement_outbox_archive ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.paper_settlement_outbox_archive FORCE ROW LEVEL SECURITY;

-- Serving roles never receive direct outbox DML.  INSERT/UPDATE transitions
-- go through the SECURITY DEFINER functions below, which lock the target and
-- derive owner_user_id from that locked row.
CREATE POLICY paper_settlement_outbox_app_select
    ON public.paper_settlement_outbox FOR SELECT TO app
    USING (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    );
CREATE POLICY paper_settlement_outbox_owner_all
    ON public.paper_settlement_outbox FOR ALL TO migration_owner
    USING (true) WITH CHECK (true);
CREATE POLICY paper_settlement_outbox_worker_select
    ON public.paper_settlement_outbox FOR SELECT TO worker USING (true);
CREATE POLICY paper_settlement_outbox_admin_select
    ON public.paper_settlement_outbox FOR SELECT TO admin USING (true);

CREATE POLICY paper_settlement_outbox_archive_owner_all
    ON public.paper_settlement_outbox_archive FOR ALL TO migration_owner
    USING (true) WITH CHECK (true);
CREATE POLICY paper_settlement_outbox_archive_app_select
    ON public.paper_settlement_outbox_archive FOR SELECT TO app
    USING (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    );
CREATE POLICY paper_settlement_outbox_archive_worker_select
    ON public.paper_settlement_outbox_archive FOR SELECT TO worker USING (true);
CREATE POLICY paper_settlement_outbox_archive_admin_select
    ON public.paper_settlement_outbox_archive FOR SELECT TO admin USING (true);

REVOKE ALL ON TABLE public.paper_settlement_outbox,
    public.paper_settlement_outbox_archive
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
GRANT SELECT ON TABLE public.paper_settlement_outbox TO app, worker, admin;
GRANT SELECT ON TABLE public.paper_settlement_outbox_archive TO app, worker, admin;

-- The trigger is DEFERRABLE so the target UPDATE may precede the outbox
-- INSERT in one transaction.  It checks both active and archived rows and is
-- installed after the legacy-terminal backfill below.
CREATE FUNCTION public.assert_paper_settlement_obligation()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $function$
DECLARE
    v_target_id uuid;
    v_status text;
BEGIN
    IF TG_TABLE_NAME = 'pending_targets' THEN
        -- Read the transition row directly.  A trusted worker may update a
        -- target without an actor GUC, and FORCE RLS must not turn this
        -- invariant into a visibility-dependent check.
        IF TG_OP = 'DELETE' THEN
            RETURN NULL;
        END IF;
        v_target_id := NEW.id;
        v_status := NEW.status;
    ELSIF TG_TABLE_NAME = 'paper_settlement_outbox' THEN
        v_target_id := CASE WHEN TG_OP = 'DELETE' THEN OLD.pending_target_id ELSE NEW.pending_target_id END;
    ELSE
        v_target_id := CASE WHEN TG_OP = 'DELETE' THEN OLD.pending_target_id ELSE NEW.pending_target_id END;
    END IF;

    IF v_status IS NULL THEN
        SELECT target.status INTO v_status
          FROM public.pending_targets AS target
         WHERE target.id = v_target_id;
    END IF;
    IF v_status IS NOT NULL
       AND v_status <> 'PENDING'
       AND NOT EXISTS (
           SELECT 1 FROM public.paper_settlement_outbox AS outbox
            WHERE outbox.pending_target_id = v_target_id
       )
       AND NOT EXISTS (
           SELECT 1 FROM public.paper_settlement_outbox_archive AS archive
            WHERE archive.pending_target_id = v_target_id
       )
    THEN
        RAISE EXCEPTION 'terminal Paper target has no durable settlement outbox obligation'
            USING ERRCODE = '23514';
    END IF;
    RETURN NULL;
END
$function$;

ALTER FUNCTION public.assert_paper_settlement_obligation() OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.assert_paper_settlement_obligation() FROM PUBLIC;

-- Backfill every terminal row created before 0041.  The payload deliberately
-- says that it is a legacy backfill: no historical parity is invented.  The
-- existing pending_targets policy is actor-scoped even for migration_owner,
-- so visit each tenant explicitly rather than accidentally backfilling zero
-- rows from a migration connection with no actor GUC.
DO $backfill$
DECLARE
    v_owner_user_id uuid;
BEGIN
    FOR v_owner_user_id IN SELECT id FROM public.users LOOP
        PERFORM pg_catalog.set_config(
            'app.actor_user_id', v_owner_user_id::text, true
        );
        INSERT INTO public.paper_settlement_outbox (
            pending_target_id, owner_user_id, severity, kind, title, body
        )
        SELECT target.id,
               target.owner_user_id,
               CASE WHEN target.status = 'EXECUTED' THEN 'INFO' ELSE 'WARNING' END,
               CASE WHEN target.status = 'EXECUTED' THEN 'job' ELSE 'alert' END,
               CASE WHEN target.status = 'EXECUTED'
                    THEN 'Paper target legacy settlement recovered'
                    ELSE 'Paper target legacy non-execution recovered' END,
               CASE WHEN target.status = 'EXECUTED'
                    THEN 'A terminal Paper target was found without a notification intent during the 0041 upgrade; the completion obligation was backfilled.'
                    ELSE 'A terminal Paper target was found without a notification intent during the 0041 upgrade; the non-execution obligation was backfilled.' END
          FROM public.pending_targets AS target
         WHERE target.owner_user_id = v_owner_user_id
           AND target.status <> 'PENDING'
        ON CONFLICT (pending_target_id) DO NOTHING;
    END LOOP;
END
$backfill$;

CREATE CONSTRAINT TRIGGER pending_targets_require_settlement_outbox
AFTER INSERT OR UPDATE OF status OR DELETE ON public.pending_targets
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION public.assert_paper_settlement_obligation();

CREATE CONSTRAINT TRIGGER paper_settlement_outbox_require_target_obligation
AFTER INSERT OR UPDATE OR DELETE ON public.paper_settlement_outbox
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION public.assert_paper_settlement_obligation();

CREATE CONSTRAINT TRIGGER paper_settlement_outbox_archive_require_target_obligation
AFTER INSERT OR UPDATE OR DELETE ON public.paper_settlement_outbox_archive
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION public.assert_paper_settlement_obligation();

-- Replace the preflight implementation from 0037/0038 with a read-only
-- gate.  A worker can ask whether execution is allowed, but this function can
-- never commit a terminal target.  Settlement owns the target transition and
-- the outbox insert in one transaction.
CREATE OR REPLACE FUNCTION public.preflight_paper_target(
    p_target_id uuid,
    p_owner_user_id uuid
) RETURNS TABLE (authorized boolean, reason jsonb)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $function$
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
           target.effective_date, target.dataset_version, target.dataset_version_id,
           target.dataset_manifest_sha256, target.status, target.source_kind,
           target.recommendation_run_id
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

    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended(v_target.account_id::text, 381901)
    );

    PERFORM 1
      FROM public.accounts AS account
      JOIN public.account_strategy_bindings AS binding
        ON binding.account_id = account.id
       AND binding.owner_user_id = account.owner_user_id
       AND binding.strategy_config_id = v_target.strategy_config_id
       AND binding.unbound_at IS NULL
       AND (
            v_target.source_kind = 'MANUAL_RECOMMENDATION'
            OR binding.auto_apply_recommendations
       )
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
            'message', 'Active permitted Paper binding is no longer available'
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
           OR v_dataset.version IS DISTINCT FROM v_target.dataset_version THEN
            v_reason := pg_catalog.jsonb_build_object(
                'code', 'PAPER_DATASET_LINEAGE_CHANGED',
                'message', 'Queued recommendation dataset lineage changed'
            );
        ELSIF v_target.source_kind <> 'LEGACY'
          AND NOT EXISTS (
              SELECT 1 FROM public.recommendation_runs AS run
               WHERE run.id = v_target.recommendation_run_id
                 AND run.owner_user_id = p_owner_user_id
                 AND run.strategy_config_id = v_target.strategy_config_id
                 AND run.as_of = v_target.computed_on
                 AND run.dataset_version_id = v_target.dataset_version_id
                 AND run.dataset_manifest_sha256 = v_target.dataset_manifest_sha256
               FOR SHARE
          ) THEN
            v_reason := pg_catalog.jsonb_build_object(
                'code', 'PAPER_RECOMMENDATION_LINEAGE_MISSING',
                'message', 'Exact recommendation run lineage is unavailable'
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
        -- Intentionally no UPDATE.  The caller records this reason through
        -- settle_with_exact_parity, which inserts the outbox in the same
        -- transaction as the terminal transition.
        RETURN QUERY SELECT false, v_reason;
        RETURN;
    END IF;

    RETURN QUERY SELECT true, NULL::jsonb;
END
$function$;

ALTER FUNCTION public.preflight_paper_target(uuid, uuid) OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.preflight_paper_target(uuid, uuid)
    FROM PUBLIC, app, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.preflight_paper_target(uuid, uuid) TO worker;

-- All outbox writes derive owner_user_id from a locked target.  App callers
-- provide only immutable payload fields; a source-key/owner squatting insert
-- is impossible because app has no table INSERT privilege and this function
-- ignores any caller-supplied owner.
CREATE FUNCTION public.enqueue_paper_settlement_outbox(
    p_pending_target_id uuid,
    p_severity text,
    p_kind text,
    p_title text,
    p_body text,
    p_parity_json jsonb
) RETURNS uuid
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
SET lock_timeout = '5s'
SET statement_timeout = '15s'
AS $function$
DECLARE
    v_target record;
    v_archive record;
    v_id uuid;
    v_actor uuid;
BEGIN
    SELECT target.id, target.owner_user_id, target.status
      INTO v_target
      FROM public.pending_targets AS target
     WHERE target.id = p_pending_target_id
     FOR UPDATE;
    IF NOT FOUND THEN
        -- FORCE RLS intentionally hides a foreign target before the function
        -- can learn its owner.  Report that as an authorization failure, not
        -- as a retryable state error that could leak target existence.
        RAISE EXCEPTION 'Paper settlement outbox target is not owned by actor'
            USING ERRCODE = '42501';
    END IF;
    IF v_target.status = 'PENDING' THEN
        RAISE EXCEPTION 'Paper settlement outbox requires a terminal target'
            USING ERRCODE = '55000';
    END IF;
    IF p_severity NOT IN ('INFO', 'WARNING', 'CRITICAL')
       OR p_kind NOT IN ('job', 'recommendation', 'backtest', 'alert')
       OR p_title IS NULL OR pg_catalog.length(p_title) = 0
       OR pg_catalog.length(p_title) > 512
       OR p_body IS NULL OR pg_catalog.length(p_body) > 8192
       OR (p_parity_json IS NOT NULL AND pg_catalog.jsonb_typeof(p_parity_json) <> 'object')
    THEN
        RAISE EXCEPTION 'Paper settlement outbox payload is invalid'
            USING ERRCODE = '22023';
    END IF;

    IF session_user NOT IN ('migration_owner') THEN
        v_actor := NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid;
        IF v_actor IS DISTINCT FROM v_target.owner_user_id THEN
            RAISE EXCEPTION 'Paper settlement outbox actor does not own target'
                USING ERRCODE = '42501';
        END IF;
    END IF;

    -- A delivered row may already have moved to the immutable archive.  Treat
    -- that archive key as the idempotent result instead of recreating an
    -- active obligation and sending the same external notification again.
    SELECT archive.id, archive.owner_user_id, archive.severity, archive.kind,
           archive.title, archive.body, archive.parity_json
      INTO v_archive
      FROM public.paper_settlement_outbox_archive AS archive
     WHERE archive.pending_target_id = v_target.id
     FOR SHARE;
    IF FOUND THEN
        IF v_archive.owner_user_id IS DISTINCT FROM v_target.owner_user_id
           OR v_archive.severity IS DISTINCT FROM p_severity
           OR v_archive.kind IS DISTINCT FROM p_kind
           OR v_archive.title IS DISTINCT FROM p_title
           OR v_archive.body IS DISTINCT FROM p_body
           OR v_archive.parity_json IS DISTINCT FROM p_parity_json
        THEN
            RAISE EXCEPTION 'Paper settlement archive payload mismatch'
                USING ERRCODE = '23505';
        END IF;
        RETURN v_archive.id;
    END IF;

    INSERT INTO public.paper_settlement_outbox (
        pending_target_id, owner_user_id, severity, kind, title, body, parity_json
    ) VALUES (
        v_target.id, v_target.owner_user_id, p_severity, p_kind, p_title, p_body, p_parity_json
    )
    ON CONFLICT (pending_target_id) DO NOTHING
    RETURNING id INTO v_id;
    IF FOUND THEN
        RETURN v_id;
    END IF;

    SELECT outbox.id INTO v_id
      FROM public.paper_settlement_outbox AS outbox
     WHERE outbox.pending_target_id = v_target.id
     FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'Paper settlement outbox disappeared during idempotent enqueue'
            USING ERRCODE = '40001';
    END IF;
    IF EXISTS (
        SELECT 1 FROM public.paper_settlement_outbox AS outbox
         WHERE outbox.id = v_id
           AND (
               outbox.owner_user_id IS DISTINCT FROM v_target.owner_user_id
               OR outbox.severity IS DISTINCT FROM p_severity
               OR outbox.kind IS DISTINCT FROM p_kind
               OR outbox.title IS DISTINCT FROM p_title
               OR outbox.body IS DISTINCT FROM p_body
               OR outbox.parity_json IS DISTINCT FROM p_parity_json
           )
    ) THEN
        RAISE EXCEPTION 'Paper settlement outbox payload mismatch'
            USING ERRCODE = '23505';
    END IF;
    RETURN v_id;
END
$function$;

ALTER FUNCTION public.enqueue_paper_settlement_outbox(uuid, text, text, text, text, jsonb)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.enqueue_paper_settlement_outbox(uuid, text, text, text, text, jsonb)
    FROM PUBLIC, worker, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.enqueue_paper_settlement_outbox(uuid, text, text, text, text, jsonb)
    TO app;

-- Claim a bounded batch before a worker performs any external transport.  The
-- row lease closes the race where two runner replicas both scan the same due
-- row and send it concurrently.  If a worker dies, the lease expires and the
-- durable obligation is retried (at-least-once, never silently discarded).
CREATE FUNCTION public.claim_paper_settlement_outbox(
    p_limit integer DEFAULT 128,
    p_lease_seconds integer DEFAULT 60
) RETURNS TABLE (
    id uuid,
    pending_target_id uuid,
    owner_user_id uuid,
    severity text,
    kind text,
    title text,
    body text,
    parity_json jsonb,
    attempts integer,
    max_attempts integer,
    available_at timestamptz,
    delivered_at timestamptz,
    exhausted_at timestamptz,
    last_error text,
    created_at timestamptz,
    claim_token uuid
)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
SET lock_timeout = '5s'
SET statement_timeout = '15s'
AS $function$
BEGIN
    IF p_limit IS NULL OR p_limit < 1 OR p_limit > 1000
       OR p_lease_seconds IS NULL OR p_lease_seconds < 5 OR p_lease_seconds > 900
    THEN
        RAISE EXCEPTION 'Paper outbox claim parameters are invalid'
            USING ERRCODE = '22023';
    END IF;

    RETURN QUERY
    WITH candidates AS (
        SELECT outbox.id
          FROM public.paper_settlement_outbox AS outbox
         WHERE outbox.delivered_at IS NULL
           AND outbox.exhausted_at IS NULL
           AND outbox.available_at <= pg_catalog.clock_timestamp()
           AND (
               outbox.claim_expires_at IS NULL
               OR outbox.claim_expires_at <= pg_catalog.clock_timestamp()
           )
         ORDER BY outbox.available_at, outbox.id
         FOR UPDATE SKIP LOCKED
         LIMIT p_limit
    )
    UPDATE public.paper_settlement_outbox AS outbox
       SET claim_token = pg_catalog.gen_random_uuid(),
           claim_expires_at = pg_catalog.clock_timestamp()
               + pg_catalog.make_interval(secs => p_lease_seconds)
      FROM candidates
     WHERE outbox.id = candidates.id
    RETURNING outbox.id, outbox.pending_target_id, outbox.owner_user_id,
              outbox.severity, outbox.kind, outbox.title, outbox.body,
              outbox.parity_json, outbox.attempts, outbox.max_attempts,
              outbox.available_at, outbox.delivered_at, outbox.exhausted_at,
              outbox.last_error, outbox.created_at, outbox.claim_token;
END
$function$;

ALTER FUNCTION public.claim_paper_settlement_outbox(integer, integer)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.claim_paper_settlement_outbox(integer, integer)
    FROM PUBLIC, app, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.claim_paper_settlement_outbox(integer, integer)
    TO worker;

CREATE FUNCTION public.mark_paper_settlement_outbox_delivered(
    p_outbox_id uuid,
    p_owner_user_id uuid,
    p_claim_token uuid
) RETURNS boolean
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
SET lock_timeout = '5s'
SET statement_timeout = '15s'
AS $function$
DECLARE
    v_actor uuid;
BEGIN
    IF session_user <> 'migration_owner' THEN
        v_actor := NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid;
        IF v_actor IS NULL OR p_owner_user_id IS DISTINCT FROM v_actor THEN
            RAISE EXCEPTION 'Paper outbox delivery actor is invalid' USING ERRCODE = '42501';
        END IF;
    END IF;
    UPDATE public.paper_settlement_outbox
       SET delivered_at = COALESCE(delivered_at, pg_catalog.now()),
           last_error = NULL,
           exhausted_at = NULL,
           claim_token = NULL,
           claim_expires_at = NULL
     WHERE id = p_outbox_id
       AND owner_user_id = p_owner_user_id
       AND delivered_at IS NULL
       AND claim_token IS NOT DISTINCT FROM p_claim_token
       AND (
           p_claim_token IS NOT NULL
           OR claim_expires_at IS NULL
       );
    RETURN FOUND;
END
$function$;

ALTER FUNCTION public.mark_paper_settlement_outbox_delivered(uuid, uuid, uuid) OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.mark_paper_settlement_outbox_delivered(uuid, uuid, uuid)
    FROM PUBLIC, worker, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.mark_paper_settlement_outbox_delivered(uuid, uuid, uuid) TO app;

CREATE FUNCTION public.mark_paper_settlement_outbox_delivered(
    p_outbox_id uuid,
    p_owner_user_id uuid
) RETURNS boolean
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, public
SET lock_timeout = '5s'
SET statement_timeout = '15s'
AS $function$
    SELECT public.mark_paper_settlement_outbox_delivered(
        p_outbox_id, p_owner_user_id, NULL::uuid
    )
$function$;

ALTER FUNCTION public.mark_paper_settlement_outbox_delivered(uuid, uuid)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.mark_paper_settlement_outbox_delivered(uuid, uuid)
    FROM PUBLIC, worker, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.mark_paper_settlement_outbox_delivered(uuid, uuid) TO app;

CREATE FUNCTION public.fail_paper_settlement_outbox(
    p_outbox_id uuid,
    p_owner_user_id uuid,
    p_error text,
    p_claim_token uuid
) RETURNS TABLE (attempts integer, exhausted boolean)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
SET lock_timeout = '5s'
SET statement_timeout = '15s'
AS $function$
DECLARE
    v_actor uuid;
    v_attempts integer;
    v_max_attempts integer;
    v_exhausted_at timestamptz;
    v_delivered_at timestamptz;
    v_claim_token uuid;
    v_available_at timestamptz;
BEGIN
    IF session_user <> 'migration_owner' THEN
        v_actor := NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid;
        IF v_actor IS NULL OR p_owner_user_id IS DISTINCT FROM v_actor THEN
            RAISE EXCEPTION 'Paper outbox failure actor is invalid' USING ERRCODE = '42501';
        END IF;
    END IF;

    SELECT outbox.attempts, outbox.max_attempts, outbox.exhausted_at,
           outbox.delivered_at, outbox.claim_token, outbox.available_at
      INTO v_attempts, v_max_attempts, v_exhausted_at, v_delivered_at,
           v_claim_token, v_available_at
      FROM public.paper_settlement_outbox AS outbox
     WHERE outbox.id = p_outbox_id
       AND outbox.owner_user_id = p_owner_user_id
     FOR UPDATE;
    IF NOT FOUND OR v_delivered_at IS NOT NULL
       OR v_claim_token IS DISTINCT FROM p_claim_token
       OR (p_claim_token IS NULL AND v_claim_token IS NOT NULL)
       OR (p_claim_token IS NULL
           AND v_available_at > pg_catalog.clock_timestamp())
    THEN
        RETURN;
    END IF;
    -- Exhaustion is terminal.  A late/concurrent failure report is an
    -- idempotent observation, with no increments beyond max_attempts.
    IF v_exhausted_at IS NOT NULL OR v_attempts >= v_max_attempts THEN
        RETURN QUERY SELECT v_attempts, true;
        RETURN;
    END IF;

    RETURN QUERY
    UPDATE public.paper_settlement_outbox AS outbox
       SET attempts = LEAST(outbox.max_attempts, outbox.attempts + 1),
           available_at = pg_catalog.now() + pg_catalog.make_interval(
               secs => LEAST(900, 5 * (2 ^ LEAST(outbox.attempts, 8))::integer)
           ),
           exhausted_at = CASE
               WHEN outbox.attempts + 1 >= outbox.max_attempts THEN pg_catalog.now()
               ELSE NULL
           END,
           last_error = pg_catalog.left(COALESCE(p_error, 'Paper settlement delivery failed'), 2048),
           claim_token = NULL,
           claim_expires_at = NULL
     WHERE outbox.id = p_outbox_id
       AND outbox.owner_user_id = p_owner_user_id
       AND outbox.delivered_at IS NULL
       AND outbox.exhausted_at IS NULL
       AND outbox.claim_token IS NOT DISTINCT FROM p_claim_token
       AND (
           p_claim_token IS NOT NULL
           OR outbox.available_at <= pg_catalog.clock_timestamp()
       )
     RETURNING outbox.attempts, outbox.exhausted_at IS NOT NULL;
END
$function$;

ALTER FUNCTION public.fail_paper_settlement_outbox(uuid, uuid, text, uuid) OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.fail_paper_settlement_outbox(uuid, uuid, text, uuid)
    FROM PUBLIC, worker, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.fail_paper_settlement_outbox(uuid, uuid, text, uuid) TO app;

CREATE FUNCTION public.fail_paper_settlement_outbox(
    p_outbox_id uuid,
    p_owner_user_id uuid,
    p_error text
) RETURNS TABLE (attempts integer, exhausted boolean)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, public
SET lock_timeout = '5s'
SET statement_timeout = '15s'
AS $function$
    SELECT * FROM public.fail_paper_settlement_outbox(
        p_outbox_id, p_owner_user_id, p_error, NULL::uuid
    )
$function$;

ALTER FUNCTION public.fail_paper_settlement_outbox(uuid, uuid, text) OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.fail_paper_settlement_outbox(uuid, uuid, text)
    FROM PUBLIC, worker, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.fail_paper_settlement_outbox(uuid, uuid, text) TO app;

-- Readiness is false when any undelivered obligation is exhausted or older
-- than the operator budget.  The worker has EXECUTE but no table DML.
CREATE FUNCTION public.paper_settlement_outbox_stats(
    p_max_age_seconds bigint DEFAULT 900
) RETURNS TABLE (
    pending_count bigint,
    oldest_pending_age_secs bigint,
    failed_count bigint,
    exhausted_count bigint,
    ready boolean
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, public
SET lock_timeout = '5s'
SET statement_timeout = '5s'
AS $function$
    SELECT count(*)::bigint,
           COALESCE(
               EXTRACT(EPOCH FROM (pg_catalog.clock_timestamp() - min(outbox.created_at)))::bigint,
               0::bigint
           ),
           count(*) FILTER (WHERE outbox.attempts > 0)::bigint,
           count(*) FILTER (WHERE outbox.exhausted_at IS NOT NULL)::bigint,
           (
               count(*) FILTER (WHERE outbox.exhausted_at IS NOT NULL) = 0
               AND (
                   count(*) = 0
                   OR EXTRACT(EPOCH FROM (pg_catalog.clock_timestamp() - min(outbox.created_at)))
                       <= p_max_age_seconds
               )
           )
      FROM public.paper_settlement_outbox AS outbox
     WHERE outbox.delivered_at IS NULL
$function$;

ALTER FUNCTION public.paper_settlement_outbox_stats(bigint) OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.paper_settlement_outbox_stats(bigint)
    FROM PUBLIC, app, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.paper_settlement_outbox_stats(bigint) TO worker;

-- Move delivered rows to the archive before deleting them from the active
-- queue.  Pending/exhausted rows are never pruned, and the obligation trigger
-- sees the archive row before the active row disappears.
CREATE FUNCTION public.prune_paper_settlement_outbox(
    p_keep_seconds bigint DEFAULT 604800,
    p_limit integer DEFAULT 256
) RETURNS bigint
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
SET lock_timeout = '5s'
SET statement_timeout = '15s'
AS $function$
DECLARE
    v_deleted bigint;
BEGIN
    IF p_keep_seconds IS NULL OR p_keep_seconds < 86400
       OR p_keep_seconds > 31536000
       OR p_limit IS NULL OR p_limit < 1 OR p_limit > 10000
    THEN
        RAISE EXCEPTION 'Paper outbox retention parameters are invalid'
            USING ERRCODE = '22023';
    END IF;

    WITH candidates AS (
        SELECT outbox.*
          FROM public.paper_settlement_outbox AS outbox
         WHERE outbox.delivered_at IS NOT NULL
           AND outbox.delivered_at < pg_catalog.clock_timestamp()
               - pg_catalog.make_interval(secs => p_keep_seconds)
         ORDER BY outbox.delivered_at, outbox.id
         FOR UPDATE SKIP LOCKED
         LIMIT p_limit
    )
    INSERT INTO public.paper_settlement_outbox_archive (
        id, pending_target_id, owner_user_id, severity, kind, title, body,
        parity_json, attempts, max_attempts, delivered_at, created_at
    )
    SELECT id, pending_target_id, owner_user_id, severity, kind, title, body,
           parity_json, attempts, max_attempts, delivered_at, created_at
      FROM candidates
    ON CONFLICT (pending_target_id) DO NOTHING;

    DELETE FROM public.paper_settlement_outbox AS outbox
     USING public.paper_settlement_outbox_archive AS archive
     WHERE outbox.pending_target_id = archive.pending_target_id
       AND outbox.delivered_at IS NOT NULL
       AND archive.archived_at >= pg_catalog.clock_timestamp()
           - pg_catalog.make_interval(secs => p_keep_seconds);
    GET DIAGNOSTICS v_deleted = ROW_COUNT;
    RETURN v_deleted;
END
$function$;

ALTER FUNCTION public.prune_paper_settlement_outbox(bigint, integer) OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.prune_paper_settlement_outbox(bigint, integer)
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.prune_paper_settlement_outbox(bigint, integer) TO worker;

GRANT SELECT ON TABLE public.notification_subscriptions TO worker;
