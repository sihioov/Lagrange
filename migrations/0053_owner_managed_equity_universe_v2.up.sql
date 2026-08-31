-- 0053: owner-managed Korean equity universe V2 domain foundation.
-- The deployed fixed-30 V1 universe and artifacts remain untouched.

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

CREATE TABLE public.owner_equity_universe_policies (
    owner_user_id uuid PRIMARY KEY
        REFERENCES public.users (id) ON DELETE RESTRICT,
    max_active_instruments integer NOT NULL,
    target_observed_sessions integer NOT NULL,
    minimum_observed_sessions integer NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT pg_catalog.now(),
    CONSTRAINT owner_equity_universe_policies_max_active_check CHECK (
        max_active_instruments > 0
    ),
    CONSTRAINT owner_equity_universe_policies_minimum_history_check CHECK (
        minimum_observed_sessions >= 121
    ),
    CONSTRAINT owner_equity_universe_policies_target_history_check CHECK (
        target_observed_sessions >= minimum_observed_sessions
    )
);

-- Provision every already-established Owner at migration time. The policy is
-- a product default, not mutable request input; owner-specific tuning remains
-- a privileged administrative operation.
INSERT INTO public.owner_equity_universe_policies (
    owner_user_id,
    max_active_instruments,
    target_observed_sessions,
    minimum_observed_sessions
)
SELECT user_role.user_id, 100, 261, 121
  FROM public.user_roles AS user_role
 WHERE user_role.role_id = 'owner'
ON CONFLICT (owner_user_id) DO NOTHING;

CREATE TABLE public.owner_equity_memberships (
    id uuid PRIMARY KEY DEFAULT pg_catalog.gen_random_uuid(),
    owner_user_id uuid NOT NULL
        REFERENCES public.users (id) ON DELETE RESTRICT,
    instrument_id text NOT NULL,
    state text NOT NULL DEFAULT 'REQUESTED',
    transition_actor_user_id uuid NOT NULL
        REFERENCES public.users (id) ON DELETE RESTRICT,
    transition_code_commit text NOT NULL,
    transition_entitlement_sha256 text NOT NULL,
    error_code text,
    error_retryable boolean,
    requested_at timestamptz NOT NULL DEFAULT pg_catalog.now(),
    disabled_at timestamptz,
    updated_at timestamptz NOT NULL DEFAULT pg_catalog.now(),
    CONSTRAINT owner_equity_memberships_owner_identity_key
        UNIQUE (id, owner_user_id, instrument_id),
    CONSTRAINT owner_equity_memberships_instrument_check CHECK (
        instrument_id ~ '^[0-9]{6}[.]KRX$'
    ),
    CONSTRAINT owner_equity_memberships_state_check CHECK (
        state IN (
            'REQUESTED', 'VALIDATING', 'BACKFILLING', 'MATERIALIZING',
            'READY', 'INSUFFICIENT_HISTORY', 'FAILED', 'DISABLED'
        )
    ),
    CONSTRAINT owner_equity_memberships_actor_check CHECK (
        transition_actor_user_id = owner_user_id
    ),
    CONSTRAINT owner_equity_memberships_code_commit_check CHECK (
        transition_code_commit ~ '^[0-9a-f]{7,64}$'
    ),
    CONSTRAINT owner_equity_memberships_entitlement_hash_check CHECK (
        transition_entitlement_sha256 ~ '^sha256:[0-9a-f]{64}$'
    ),
    CONSTRAINT owner_equity_memberships_failure_code_check CHECK (
        error_code IS NULL OR error_code ~ '^[A-Z][A-Z0-9_]{0,63}$'
    ),
    CONSTRAINT owner_equity_memberships_failure_state_check CHECK (
        (state = 'FAILED' AND error_code IS NOT NULL AND error_retryable IS NOT NULL)
        OR (state <> 'FAILED' AND error_code IS NULL AND error_retryable IS NULL)
    ),
    CONSTRAINT owner_equity_memberships_disabled_state_check CHECK (
        (state = 'DISABLED' AND disabled_at IS NOT NULL)
        OR (state <> 'DISABLED' AND disabled_at IS NULL)
    )
);

CREATE UNIQUE INDEX owner_equity_memberships_one_active_instrument
    ON public.owner_equity_memberships (owner_user_id, instrument_id)
    WHERE state <> 'DISABLED';
CREATE INDEX owner_equity_memberships_owner_state_idx
    ON public.owner_equity_memberships (owner_user_id, state, updated_at, id);

CREATE TABLE public.owner_equity_membership_events (
    id uuid PRIMARY KEY DEFAULT pg_catalog.gen_random_uuid(),
    membership_id uuid NOT NULL,
    owner_user_id uuid NOT NULL,
    instrument_id text NOT NULL,
    generation bigint NOT NULL,
    from_state text,
    to_state text NOT NULL,
    actor_user_id uuid NOT NULL
        REFERENCES public.users (id) ON DELETE RESTRICT,
    code_commit text NOT NULL,
    entitlement_sha256 text NOT NULL,
    error_code text,
    error_retryable boolean,
    occurred_at timestamptz NOT NULL DEFAULT pg_catalog.now(),
    CONSTRAINT owner_equity_membership_events_membership_fkey
        FOREIGN KEY (membership_id, owner_user_id, instrument_id)
        REFERENCES public.owner_equity_memberships (id, owner_user_id, instrument_id)
        ON DELETE RESTRICT,
    CONSTRAINT owner_equity_membership_events_generation_check CHECK (generation >= 0),
    CONSTRAINT owner_equity_membership_events_from_state_check CHECK (
        from_state IS NULL OR from_state IN (
            'REQUESTED', 'VALIDATING', 'BACKFILLING', 'MATERIALIZING',
            'READY', 'INSUFFICIENT_HISTORY', 'FAILED', 'DISABLED'
        )
    ),
    CONSTRAINT owner_equity_membership_events_to_state_check CHECK (
        to_state IN (
            'REQUESTED', 'VALIDATING', 'BACKFILLING', 'MATERIALIZING',
            'READY', 'INSUFFICIENT_HISTORY', 'FAILED', 'DISABLED'
        )
    ),
    CONSTRAINT owner_equity_membership_events_actor_check CHECK (
        actor_user_id = owner_user_id
    ),
    CONSTRAINT owner_equity_membership_events_code_commit_check CHECK (
        code_commit ~ '^[0-9a-f]{7,64}$'
    ),
    CONSTRAINT owner_equity_membership_events_entitlement_hash_check CHECK (
        entitlement_sha256 ~ '^sha256:[0-9a-f]{64}$'
    ),
    CONSTRAINT owner_equity_membership_events_failure_code_check CHECK (
        error_code IS NULL OR error_code ~ '^[A-Z][A-Z0-9_]{0,63}$'
    ),
    CONSTRAINT owner_equity_membership_events_failure_state_check CHECK (
        (to_state = 'FAILED' AND error_code IS NOT NULL AND error_retryable IS NOT NULL)
        OR (to_state <> 'FAILED' AND error_code IS NULL AND error_retryable IS NULL)
    )
);

CREATE INDEX owner_equity_membership_events_membership_idx
    ON public.owner_equity_membership_events (membership_id, occurred_at, id);

CREATE TABLE public.owner_equity_instrument_generations (
    id uuid PRIMARY KEY DEFAULT pg_catalog.gen_random_uuid(),
    membership_id uuid NOT NULL,
    owner_user_id uuid NOT NULL,
    instrument_id text NOT NULL,
    generation bigint NOT NULL,
    target_observed_sessions integer NOT NULL,
    minimum_observed_sessions integer NOT NULL,
    observed_sessions integer NOT NULL,
    first_session date,
    last_session date,
    created_at timestamptz NOT NULL DEFAULT pg_catalog.now(),
    CONSTRAINT owner_equity_instrument_generations_membership_fkey
        FOREIGN KEY (membership_id, owner_user_id, instrument_id)
        REFERENCES public.owner_equity_memberships (id, owner_user_id, instrument_id)
        ON DELETE RESTRICT,
    CONSTRAINT owner_equity_instrument_generations_number_key
        UNIQUE (membership_id, generation),
    CONSTRAINT owner_equity_instrument_generations_lineage_key
        UNIQUE (id, owner_user_id, membership_id, instrument_id, generation),
    CONSTRAINT owner_equity_instrument_generations_number_check CHECK (generation > 0),
    CONSTRAINT owner_equity_instrument_generations_minimum_check CHECK (
        minimum_observed_sessions >= 121
    ),
    CONSTRAINT owner_equity_instrument_generations_target_check CHECK (
        target_observed_sessions >= minimum_observed_sessions
    ),
    CONSTRAINT owner_equity_instrument_generations_observed_check CHECK (
        observed_sessions >= 0 AND observed_sessions <= target_observed_sessions
    ),
    CONSTRAINT owner_equity_instrument_generations_coverage_check CHECK (
        (observed_sessions = 0 AND first_session IS NULL AND last_session IS NULL)
        OR (
            observed_sessions > 0
            AND first_session IS NOT NULL
            AND last_session IS NOT NULL
            AND first_session <= last_session
        )
    )
);

CREATE TABLE public.owner_equity_generation_admissions (
    generation_id uuid PRIMARY KEY,
    owner_user_id uuid NOT NULL,
    membership_id uuid NOT NULL,
    instrument_id text NOT NULL,
    generation bigint NOT NULL,
    raw_manifest_sha256 text NOT NULL,
    artifact_manifest_sha256 text NOT NULL,
    entitlement_sha256 text NOT NULL,
    capture_code_commit text NOT NULL,
    materializer_code_commit text NOT NULL,
    admitted_at timestamptz NOT NULL DEFAULT pg_catalog.now(),
    CONSTRAINT owner_equity_generation_admissions_generation_fkey
        FOREIGN KEY (
            generation_id, owner_user_id, membership_id, instrument_id, generation
        ) REFERENCES public.owner_equity_instrument_generations (
            id, owner_user_id, membership_id, instrument_id, generation
        ) ON DELETE RESTRICT,
    CONSTRAINT owner_equity_generation_admissions_lineage_key
        UNIQUE (generation_id, owner_user_id, membership_id, instrument_id, generation),
    CONSTRAINT owner_equity_generation_admissions_raw_hash_check CHECK (
        raw_manifest_sha256 ~ '^sha256:[0-9a-f]{64}$'
    ),
    CONSTRAINT owner_equity_generation_admissions_artifact_hash_check CHECK (
        artifact_manifest_sha256 ~ '^sha256:[0-9a-f]{64}$'
    ),
    CONSTRAINT owner_equity_generation_admissions_entitlement_hash_check CHECK (
        entitlement_sha256 ~ '^sha256:[0-9a-f]{64}$'
    ),
    CONSTRAINT owner_equity_generation_admissions_capture_commit_check CHECK (
        capture_code_commit ~ '^[0-9a-f]{7,64}$'
    ),
    CONSTRAINT owner_equity_generation_admissions_materializer_commit_check CHECK (
        materializer_code_commit ~ '^[0-9a-f]{7,64}$'
    )
);

CREATE TABLE public.owner_equity_signal_snapshots (
    id uuid PRIMARY KEY DEFAULT pg_catalog.gen_random_uuid(),
    owner_user_id uuid NOT NULL
        REFERENCES public.users (id) ON DELETE RESTRICT,
    as_of_session date NOT NULL,
    universe_sha256 text NOT NULL,
    row_count integer NOT NULL,
    signal_code_commit text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT pg_catalog.now(),
    published_at timestamptz,
    CONSTRAINT owner_equity_signal_snapshots_owner_key UNIQUE (id, owner_user_id),
    CONSTRAINT owner_equity_signal_snapshots_universe_hash_check CHECK (
        universe_sha256 ~ '^sha256:[0-9a-f]{64}$'
    ),
    CONSTRAINT owner_equity_signal_snapshots_row_count_check CHECK (row_count >= 0),
    CONSTRAINT owner_equity_signal_snapshots_code_commit_check CHECK (
        signal_code_commit ~ '^[0-9a-f]{7,64}$'
    )
);

CREATE INDEX owner_equity_signal_snapshots_owner_as_of_idx
    ON public.owner_equity_signal_snapshots (
        owner_user_id, as_of_session DESC, published_at DESC, id DESC
    );

CREATE TABLE public.owner_equity_signal_snapshot_rows (
    snapshot_id uuid NOT NULL,
    owner_user_id uuid NOT NULL,
    instrument_id text NOT NULL,
    membership_id uuid NOT NULL,
    generation_id uuid NOT NULL,
    generation bigint NOT NULL,
    rank integer NOT NULL,
    signals_json jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT pg_catalog.now(),
    PRIMARY KEY (snapshot_id, instrument_id),
    CONSTRAINT owner_equity_signal_snapshot_rows_rank_key UNIQUE (snapshot_id, rank),
    CONSTRAINT owner_equity_signal_snapshot_rows_snapshot_fkey
        FOREIGN KEY (snapshot_id, owner_user_id)
        REFERENCES public.owner_equity_signal_snapshots (id, owner_user_id)
        ON DELETE RESTRICT,
    CONSTRAINT owner_equity_signal_snapshot_rows_admission_fkey
        FOREIGN KEY (
            generation_id, owner_user_id, membership_id, instrument_id, generation
        ) REFERENCES public.owner_equity_generation_admissions (
            generation_id, owner_user_id, membership_id, instrument_id, generation
        ) ON DELETE RESTRICT,
    CONSTRAINT owner_equity_signal_snapshot_rows_rank_check CHECK (rank > 0),
    CONSTRAINT owner_equity_signal_snapshot_rows_signals_check CHECK (
        pg_catalog.jsonb_typeof(signals_json) = 'object'
    )
);

ALTER TABLE public.owner_equity_universe_policies OWNER TO migration_owner;
ALTER TABLE public.owner_equity_memberships OWNER TO migration_owner;
ALTER TABLE public.owner_equity_membership_events OWNER TO migration_owner;
ALTER TABLE public.owner_equity_instrument_generations OWNER TO migration_owner;
ALTER TABLE public.owner_equity_generation_admissions OWNER TO migration_owner;
ALTER TABLE public.owner_equity_signal_snapshots OWNER TO migration_owner;
ALTER TABLE public.owner_equity_signal_snapshot_rows OWNER TO migration_owner;

ALTER TABLE public.owner_equity_universe_policies ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.owner_equity_universe_policies FORCE ROW LEVEL SECURITY;
ALTER TABLE public.owner_equity_memberships ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.owner_equity_memberships FORCE ROW LEVEL SECURITY;
ALTER TABLE public.owner_equity_membership_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.owner_equity_membership_events FORCE ROW LEVEL SECURITY;
ALTER TABLE public.owner_equity_instrument_generations ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.owner_equity_instrument_generations FORCE ROW LEVEL SECURITY;
ALTER TABLE public.owner_equity_generation_admissions ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.owner_equity_generation_admissions FORCE ROW LEVEL SECURITY;
ALTER TABLE public.owner_equity_signal_snapshots ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.owner_equity_signal_snapshots FORCE ROW LEVEL SECURITY;
ALTER TABLE public.owner_equity_signal_snapshot_rows ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.owner_equity_signal_snapshot_rows FORCE ROW LEVEL SECURITY;

REVOKE ALL ON TABLE public.owner_equity_universe_policies
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
REVOKE ALL ON TABLE public.owner_equity_memberships
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
REVOKE ALL ON TABLE public.owner_equity_membership_events
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
REVOKE ALL ON TABLE public.owner_equity_instrument_generations
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
REVOKE ALL ON TABLE public.owner_equity_generation_admissions
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
REVOKE ALL ON TABLE public.owner_equity_signal_snapshots
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
REVOKE ALL ON TABLE public.owner_equity_signal_snapshot_rows
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;

GRANT SELECT ON TABLE public.owner_equity_universe_policies TO app, worker, admin;
GRANT SELECT ON TABLE public.owner_equity_memberships TO app, worker, admin;
GRANT INSERT (
    id, owner_user_id, instrument_id, transition_actor_user_id,
    transition_code_commit, transition_entitlement_sha256
) ON public.owner_equity_memberships TO app;
GRANT UPDATE (
    state, transition_actor_user_id, transition_code_commit,
    transition_entitlement_sha256, error_code, error_retryable,
    disabled_at, updated_at
) ON public.owner_equity_memberships TO worker;
GRANT SELECT ON TABLE public.owner_equity_membership_events TO app, worker, admin;
GRANT SELECT ON TABLE public.owner_equity_instrument_generations TO app, worker, admin;
GRANT INSERT (
    id, membership_id, owner_user_id, instrument_id, generation,
    target_observed_sessions, minimum_observed_sessions, observed_sessions,
    first_session, last_session
) ON public.owner_equity_instrument_generations TO worker;
GRANT SELECT ON TABLE public.owner_equity_generation_admissions TO app, worker, admin;
GRANT INSERT (
    generation_id, owner_user_id, membership_id, instrument_id, generation,
    raw_manifest_sha256, artifact_manifest_sha256, entitlement_sha256,
    capture_code_commit, materializer_code_commit
) ON public.owner_equity_generation_admissions TO worker;
GRANT SELECT ON TABLE public.owner_equity_signal_snapshots TO app, worker, admin;
GRANT INSERT (
    id, owner_user_id, as_of_session, universe_sha256, row_count,
    signal_code_commit
) ON public.owner_equity_signal_snapshots TO worker;
GRANT UPDATE (published_at) ON public.owner_equity_signal_snapshots TO worker;
GRANT SELECT ON TABLE public.owner_equity_signal_snapshot_rows TO app, worker, admin;
GRANT INSERT (
    snapshot_id, owner_user_id, instrument_id, membership_id,
    generation_id, generation, rank, signals_json
) ON public.owner_equity_signal_snapshot_rows TO worker;

CREATE POLICY owner_equity_universe_policies_app_select
    ON public.owner_equity_universe_policies FOR SELECT TO app
    USING (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    );
CREATE POLICY owner_equity_universe_policies_worker_select
    ON public.owner_equity_universe_policies FOR SELECT TO worker USING (true);
CREATE POLICY owner_equity_universe_policies_admin_select
    ON public.owner_equity_universe_policies FOR SELECT TO admin USING (true);
CREATE POLICY owner_equity_universe_policies_owner_all
    ON public.owner_equity_universe_policies FOR ALL TO migration_owner
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

CREATE POLICY owner_equity_memberships_app_select
    ON public.owner_equity_memberships FOR SELECT TO app
    USING (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    );
CREATE POLICY owner_equity_memberships_app_insert
    ON public.owner_equity_memberships FOR INSERT TO app
    WITH CHECK (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    );
CREATE POLICY owner_equity_memberships_worker_all
    ON public.owner_equity_memberships FOR ALL TO worker
    USING (true) WITH CHECK (true);
CREATE POLICY owner_equity_memberships_admin_select
    ON public.owner_equity_memberships FOR SELECT TO admin USING (true);
CREATE POLICY owner_equity_memberships_owner_all
    ON public.owner_equity_memberships FOR ALL TO migration_owner
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

DO $rls$
DECLARE
    v_table text;
BEGIN
    FOREACH v_table IN ARRAY ARRAY[
        'owner_equity_membership_events',
        'owner_equity_instrument_generations',
        'owner_equity_generation_admissions'
    ] LOOP
        EXECUTE pg_catalog.format(
            'CREATE POLICY %I ON public.%I FOR SELECT TO app USING '
            || '(owner_user_id = NULLIF(pg_catalog.current_setting('
            || '''app.actor_user_id'', true), '''')::uuid)',
            v_table || '_app_select', v_table
        );
        EXECUTE pg_catalog.format(
            'CREATE POLICY %I ON public.%I FOR ALL TO worker '
            || 'USING (true) WITH CHECK (true)',
            v_table || '_worker_all', v_table
        );
        EXECUTE pg_catalog.format(
            'CREATE POLICY %I ON public.%I FOR SELECT TO admin USING (true)',
            v_table || '_admin_select', v_table
        );
        EXECUTE pg_catalog.format(
            'CREATE POLICY %I ON public.%I FOR ALL TO migration_owner USING '
            || '(owner_user_id = NULLIF(pg_catalog.current_setting('
            || '''app.actor_user_id'', true), '''')::uuid) WITH CHECK '
            || '(owner_user_id = NULLIF(pg_catalog.current_setting('
            || '''app.actor_user_id'', true), '''')::uuid)',
            v_table || '_owner_all', v_table
        );
    END LOOP;
END
$rls$;

CREATE POLICY owner_equity_signal_snapshot_rows_app_select
    ON public.owner_equity_signal_snapshot_rows FOR SELECT TO app
    USING (
        owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
        AND EXISTS (
            SELECT 1
              FROM public.owner_equity_signal_snapshots AS snapshot
             WHERE snapshot.id = owner_equity_signal_snapshot_rows.snapshot_id
               AND snapshot.owner_user_id =
                   owner_equity_signal_snapshot_rows.owner_user_id
               AND snapshot.published_at IS NOT NULL
        )
    );
CREATE POLICY owner_equity_signal_snapshot_rows_worker_all
    ON public.owner_equity_signal_snapshot_rows FOR ALL TO worker
    USING (true) WITH CHECK (true);
CREATE POLICY owner_equity_signal_snapshot_rows_admin_select
    ON public.owner_equity_signal_snapshot_rows FOR SELECT TO admin USING (true);
CREATE POLICY owner_equity_signal_snapshot_rows_owner_all
    ON public.owner_equity_signal_snapshot_rows FOR ALL TO migration_owner
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

CREATE POLICY owner_equity_signal_snapshots_app_select
    ON public.owner_equity_signal_snapshots FOR SELECT TO app
    USING (
        published_at IS NOT NULL
        AND owner_user_id = NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid
    );
CREATE POLICY owner_equity_signal_snapshots_worker_all
    ON public.owner_equity_signal_snapshots FOR ALL TO worker
    USING (true) WITH CHECK (true);
CREATE POLICY owner_equity_signal_snapshots_admin_select
    ON public.owner_equity_signal_snapshots FOR SELECT TO admin USING (true);
CREATE POLICY owner_equity_signal_snapshots_owner_all
    ON public.owner_equity_signal_snapshots FOR ALL TO migration_owner
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

-- Keep policy provisioning structural for future Owner grants. The trigger
-- executes as migration_owner but binds FORCE-RLS to exactly the affected
-- user before inserting the default row.
CREATE FUNCTION public.provision_owner_equity_universe_policy()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $provision_policy$
BEGIN
    IF NEW.role_id = 'owner' THEN
        PERFORM pg_catalog.set_config('app.actor_user_id', NEW.user_id::text, true);
        INSERT INTO public.owner_equity_universe_policies (
            owner_user_id,
            max_active_instruments,
            target_observed_sessions,
            minimum_observed_sessions
        )
        VALUES (NEW.user_id, 100, 261, 121)
        ON CONFLICT (owner_user_id) DO NOTHING;
    END IF;
    RETURN NEW;
END
$provision_policy$;

ALTER FUNCTION public.provision_owner_equity_universe_policy()
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.provision_owner_equity_universe_policy()
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;

CREATE TRIGGER user_roles_provision_owner_equity_universe_policy
    AFTER INSERT OR UPDATE OF role_id ON public.user_roles
    FOR EACH ROW EXECUTE FUNCTION public.provision_owner_equity_universe_policy();

CREATE FUNCTION public.owner_equity_memberships_guard()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $membership_guard$
DECLARE
    v_max_active integer;
    v_current_active integer;
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'owner equity memberships are soft-disabled, never deleted'
            USING ERRCODE = '42501';
    END IF;

    PERFORM pg_catalog.set_config('app.actor_user_id', NEW.owner_user_id::text, true);

    IF TG_OP = 'INSERT' THEN
        IF NEW.state IS DISTINCT FROM 'REQUESTED'
           OR NEW.transition_actor_user_id IS DISTINCT FROM NEW.owner_user_id
        THEN
            RAISE EXCEPTION 'owner equity membership must begin as REQUESTED'
                USING ERRCODE = '23514';
        END IF;
        SELECT policy.max_active_instruments
          INTO v_max_active
          FROM public.owner_equity_universe_policies AS policy
         WHERE policy.owner_user_id = NEW.owner_user_id
         FOR UPDATE OF policy;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'owner equity universe policy is required'
                USING ERRCODE = '23514';
        END IF;
        SELECT count(*)
          INTO v_current_active
          FROM public.owner_equity_memberships AS membership
         WHERE membership.owner_user_id = NEW.owner_user_id
           AND membership.state <> 'DISABLED';
        IF v_current_active >= v_max_active THEN
            RAISE EXCEPTION 'owner equity active instrument policy limit reached'
                USING ERRCODE = '23514';
        END IF;
        RETURN NEW;
    END IF;

    IF NEW.id IS DISTINCT FROM OLD.id
       OR NEW.owner_user_id IS DISTINCT FROM OLD.owner_user_id
       OR NEW.instrument_id IS DISTINCT FROM OLD.instrument_id
       OR NEW.requested_at IS DISTINCT FROM OLD.requested_at
    THEN
        RAISE EXCEPTION 'owner equity membership identity is immutable'
            USING ERRCODE = '42501';
    END IF;
    IF OLD.state = 'DISABLED' THEN
        RAISE EXCEPTION 'disabled owner equity membership is immutable'
            USING ERRCODE = '42501';
    END IF;
    IF NEW.state IS NOT DISTINCT FROM OLD.state THEN
        IF NEW.transition_actor_user_id IS DISTINCT FROM OLD.transition_actor_user_id
           OR NEW.transition_code_commit IS DISTINCT FROM OLD.transition_code_commit
           OR NEW.transition_entitlement_sha256 IS DISTINCT FROM OLD.transition_entitlement_sha256
           OR NEW.error_code IS DISTINCT FROM OLD.error_code
           OR NEW.error_retryable IS DISTINCT FROM OLD.error_retryable
           OR NEW.disabled_at IS DISTINCT FROM OLD.disabled_at
        THEN
            RAISE EXCEPTION 'owner equity transition evidence changes only with state'
                USING ERRCODE = '42501';
        END IF;
        RETURN NEW;
    END IF;

    IF NOT (CASE OLD.state
        WHEN 'REQUESTED' THEN NEW.state IN ('VALIDATING', 'DISABLED')
        WHEN 'VALIDATING' THEN NEW.state IN ('BACKFILLING', 'FAILED', 'DISABLED')
        WHEN 'BACKFILLING' THEN NEW.state IN (
            'MATERIALIZING', 'INSUFFICIENT_HISTORY', 'FAILED', 'DISABLED'
        )
        WHEN 'MATERIALIZING' THEN NEW.state IN (
            'READY', 'INSUFFICIENT_HISTORY', 'FAILED', 'DISABLED'
        )
        WHEN 'READY' THEN NEW.state = 'DISABLED'
        WHEN 'INSUFFICIENT_HISTORY' THEN NEW.state IN ('REQUESTED', 'DISABLED')
        WHEN 'FAILED' THEN NEW.state IN ('REQUESTED', 'DISABLED')
        ELSE false
    END) THEN
        RAISE EXCEPTION 'illegal owner equity membership state transition'
            USING ERRCODE = '23514';
    END IF;
    IF OLD.state = 'FAILED' AND NEW.state = 'REQUESTED'
       AND OLD.error_retryable IS DISTINCT FROM true
    THEN
        RAISE EXCEPTION 'terminal owner equity failure cannot be retried'
            USING ERRCODE = '23514';
    END IF;
    IF NEW.transition_actor_user_id IS DISTINCT FROM NEW.owner_user_id THEN
        RAISE EXCEPTION 'owner equity transition actor must match owner'
            USING ERRCODE = '23514';
    END IF;
    IF NEW.state = 'READY' AND NOT EXISTS (
        SELECT 1
          FROM public.owner_equity_generation_admissions AS admission
         WHERE admission.membership_id = NEW.id
           AND admission.owner_user_id = NEW.owner_user_id
           AND admission.instrument_id = NEW.instrument_id
    ) THEN
        RAISE EXCEPTION 'READY owner equity membership requires an admitted generation'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END
$membership_guard$;

ALTER FUNCTION public.owner_equity_memberships_guard() OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.owner_equity_memberships_guard()
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;

CREATE TRIGGER owner_equity_memberships_guard
    BEFORE INSERT OR UPDATE OR DELETE ON public.owner_equity_memberships
    FOR EACH ROW EXECUTE FUNCTION public.owner_equity_memberships_guard();

CREATE FUNCTION public.owner_equity_memberships_record_event()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $membership_event$
DECLARE
    v_generation bigint;
BEGIN
    PERFORM pg_catalog.set_config('app.actor_user_id', NEW.owner_user_id::text, true);
    SELECT COALESCE(max(generation.generation), 0)
      INTO v_generation
      FROM public.owner_equity_instrument_generations AS generation
     WHERE generation.membership_id = NEW.id;
    INSERT INTO public.owner_equity_membership_events (
        membership_id, owner_user_id, instrument_id, generation,
        from_state, to_state, actor_user_id, code_commit,
        entitlement_sha256, error_code, error_retryable
    ) VALUES (
        NEW.id, NEW.owner_user_id, NEW.instrument_id, v_generation,
        CASE WHEN TG_OP = 'INSERT' THEN NULL ELSE OLD.state END,
        NEW.state, NEW.transition_actor_user_id, NEW.transition_code_commit,
        NEW.transition_entitlement_sha256, NEW.error_code, NEW.error_retryable
    );
    RETURN NEW;
END
$membership_event$;

ALTER FUNCTION public.owner_equity_memberships_record_event() OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.owner_equity_memberships_record_event()
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;

CREATE TRIGGER owner_equity_memberships_record_insert_event
    AFTER INSERT ON public.owner_equity_memberships
    FOR EACH ROW
    EXECUTE FUNCTION public.owner_equity_memberships_record_event();
CREATE TRIGGER owner_equity_memberships_record_update_event
    AFTER UPDATE OF state ON public.owner_equity_memberships
    FOR EACH ROW
    WHEN (OLD.state IS DISTINCT FROM NEW.state)
    EXECUTE FUNCTION public.owner_equity_memberships_record_event();

CREATE FUNCTION public.owner_equity_instrument_generations_guard()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $generation_guard$
DECLARE
    v_membership public.owner_equity_memberships%ROWTYPE;
    v_previous_generation bigint;
BEGIN
    PERFORM pg_catalog.set_config('app.actor_user_id', NEW.owner_user_id::text, true);
    SELECT membership.*
      INTO v_membership
      FROM public.owner_equity_memberships AS membership
     WHERE membership.id = NEW.membership_id
     FOR UPDATE OF membership;
    IF NOT FOUND
       OR v_membership.owner_user_id IS DISTINCT FROM NEW.owner_user_id
       OR v_membership.instrument_id IS DISTINCT FROM NEW.instrument_id
       OR v_membership.state NOT IN ('BACKFILLING', 'MATERIALIZING', 'READY')
    THEN
        RAISE EXCEPTION 'owner equity generation membership binding is invalid'
            USING ERRCODE = '23514';
    END IF;
    SELECT COALESCE(max(generation.generation), 0)
      INTO v_previous_generation
      FROM public.owner_equity_instrument_generations AS generation
     WHERE generation.membership_id = NEW.membership_id;
    IF NEW.generation IS DISTINCT FROM v_previous_generation + 1 THEN
        RAISE EXCEPTION 'owner equity generation must be monotonically consecutive'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END
$generation_guard$;

ALTER FUNCTION public.owner_equity_instrument_generations_guard() OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.owner_equity_instrument_generations_guard()
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;

CREATE TRIGGER owner_equity_instrument_generations_guard
    BEFORE INSERT ON public.owner_equity_instrument_generations
    FOR EACH ROW EXECUTE FUNCTION public.owner_equity_instrument_generations_guard();

CREATE FUNCTION public.owner_equity_generation_admissions_guard()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $admission_guard$
DECLARE
    v_observed integer;
    v_minimum integer;
    v_membership_state text;
BEGIN
    PERFORM pg_catalog.set_config('app.actor_user_id', NEW.owner_user_id::text, true);
    SELECT generation.observed_sessions, generation.minimum_observed_sessions,
           membership.state
      INTO v_observed, v_minimum, v_membership_state
      FROM public.owner_equity_instrument_generations AS generation
      JOIN public.owner_equity_memberships AS membership
        ON membership.id = generation.membership_id
       AND membership.owner_user_id = generation.owner_user_id
       AND membership.instrument_id = generation.instrument_id
     WHERE generation.id = NEW.generation_id
       AND generation.owner_user_id = NEW.owner_user_id
       AND generation.membership_id = NEW.membership_id
       AND generation.instrument_id = NEW.instrument_id
       AND generation.generation = NEW.generation
     FOR SHARE OF generation, membership;
    IF NOT FOUND
       OR v_membership_state = 'DISABLED'
       OR v_observed < v_minimum
    THEN
        RAISE EXCEPTION 'owner equity generation is not admissible'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END
$admission_guard$;

ALTER FUNCTION public.owner_equity_generation_admissions_guard() OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.owner_equity_generation_admissions_guard()
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;

CREATE TRIGGER owner_equity_generation_admissions_guard
    BEFORE INSERT ON public.owner_equity_generation_admissions
    FOR EACH ROW EXECUTE FUNCTION public.owner_equity_generation_admissions_guard();

CREATE FUNCTION public.owner_equity_signal_snapshot_rows_guard()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $snapshot_row_guard$
DECLARE
    v_published_at timestamptz;
BEGIN
    PERFORM pg_catalog.set_config('app.actor_user_id', NEW.owner_user_id::text, true);
    SELECT snapshot.published_at
      INTO v_published_at
      FROM public.owner_equity_signal_snapshots AS snapshot
     WHERE snapshot.id = NEW.snapshot_id
       AND snapshot.owner_user_id = NEW.owner_user_id
     FOR UPDATE OF snapshot;
    IF NOT FOUND OR v_published_at IS NOT NULL THEN
        RAISE EXCEPTION 'owner equity signal snapshot is not open'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END
$snapshot_row_guard$;

ALTER FUNCTION public.owner_equity_signal_snapshot_rows_guard() OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.owner_equity_signal_snapshot_rows_guard()
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;

CREATE TRIGGER owner_equity_signal_snapshot_rows_guard
    BEFORE INSERT ON public.owner_equity_signal_snapshot_rows
    FOR EACH ROW EXECUTE FUNCTION public.owner_equity_signal_snapshot_rows_guard();

CREATE FUNCTION public.owner_equity_signal_snapshots_guard()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $snapshot_guard$
DECLARE
    v_actual_count integer;
    v_ready_count integer;
    v_actual_hash text;
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'owner equity signal snapshots are immutable'
            USING ERRCODE = '42501';
    END IF;
    IF OLD.published_at IS NOT NULL
       OR NEW.published_at IS NULL
       OR NEW.id IS DISTINCT FROM OLD.id
       OR NEW.owner_user_id IS DISTINCT FROM OLD.owner_user_id
       OR NEW.as_of_session IS DISTINCT FROM OLD.as_of_session
       OR NEW.universe_sha256 IS DISTINCT FROM OLD.universe_sha256
       OR NEW.row_count IS DISTINCT FROM OLD.row_count
       OR NEW.signal_code_commit IS DISTINCT FROM OLD.signal_code_commit
       OR NEW.created_at IS DISTINCT FROM OLD.created_at
    THEN
        RAISE EXCEPTION 'owner equity signal snapshot lineage is immutable'
            USING ERRCODE = '42501';
    END IF;

    PERFORM pg_catalog.set_config('app.actor_user_id', NEW.owner_user_id::text, true);
    SELECT count(*)::integer,
           'sha256:' || pg_catalog.encode(
               pg_catalog.sha256(
                   pg_catalog.convert_to(
                       COALESCE(
                           pg_catalog.string_agg(
                               snapshot_row.instrument_id,
                               E'\n' ORDER BY snapshot_row.instrument_id
                           ),
                           ''
                       ),
                       'UTF8'
                   )
               ),
               'hex'
           )
      INTO v_actual_count, v_actual_hash
      FROM public.owner_equity_signal_snapshot_rows AS snapshot_row
     WHERE snapshot_row.snapshot_id = NEW.id
       AND snapshot_row.owner_user_id = NEW.owner_user_id;
    SELECT count(*)::integer
      INTO v_ready_count
      FROM public.owner_equity_memberships AS membership
     WHERE membership.owner_user_id = NEW.owner_user_id
       AND membership.state = 'READY';
    IF v_actual_count IS DISTINCT FROM NEW.row_count
       OR v_ready_count IS DISTINCT FROM NEW.row_count
       OR v_actual_hash IS DISTINCT FROM NEW.universe_sha256
       OR EXISTS (
            SELECT membership.instrument_id
              FROM public.owner_equity_memberships AS membership
             WHERE membership.owner_user_id = NEW.owner_user_id
               AND membership.state = 'READY'
            EXCEPT
            SELECT snapshot_row.instrument_id
              FROM public.owner_equity_signal_snapshot_rows AS snapshot_row
             WHERE snapshot_row.snapshot_id = NEW.id
               AND snapshot_row.owner_user_id = NEW.owner_user_id
       )
       OR EXISTS (
            SELECT snapshot_row.instrument_id
              FROM public.owner_equity_signal_snapshot_rows AS snapshot_row
             WHERE snapshot_row.snapshot_id = NEW.id
               AND snapshot_row.owner_user_id = NEW.owner_user_id
            EXCEPT
            SELECT membership.instrument_id
              FROM public.owner_equity_memberships AS membership
             WHERE membership.owner_user_id = NEW.owner_user_id
               AND membership.state = 'READY'
       )
    THEN
        RAISE EXCEPTION 'owner equity signal snapshot universe is not exact'
            USING ERRCODE = '23514';
    END IF;
    NEW.published_at := pg_catalog.now();
    RETURN NEW;
END
$snapshot_guard$;

ALTER FUNCTION public.owner_equity_signal_snapshots_guard() OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.owner_equity_signal_snapshots_guard()
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;

CREATE TRIGGER owner_equity_signal_snapshots_guard
    BEFORE UPDATE OR DELETE ON public.owner_equity_signal_snapshots
    FOR EACH ROW EXECUTE FUNCTION public.owner_equity_signal_snapshots_guard();

CREATE FUNCTION public.owner_equity_append_only_guard()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $append_only$
BEGIN
    RAISE EXCEPTION 'owner equity lineage is append-only'
        USING ERRCODE = '42501';
END
$append_only$;

ALTER FUNCTION public.owner_equity_append_only_guard() OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.owner_equity_append_only_guard()
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;

CREATE TRIGGER owner_equity_membership_events_immutable
    BEFORE UPDATE OR DELETE ON public.owner_equity_membership_events
    FOR EACH ROW EXECUTE FUNCTION public.owner_equity_append_only_guard();
CREATE TRIGGER owner_equity_instrument_generations_immutable
    BEFORE UPDATE OR DELETE ON public.owner_equity_instrument_generations
    FOR EACH ROW EXECUTE FUNCTION public.owner_equity_append_only_guard();
CREATE TRIGGER owner_equity_generation_admissions_immutable
    BEFORE UPDATE OR DELETE ON public.owner_equity_generation_admissions
    FOR EACH ROW EXECUTE FUNCTION public.owner_equity_append_only_guard();
CREATE TRIGGER owner_equity_signal_snapshot_rows_immutable
    BEFORE UPDATE OR DELETE ON public.owner_equity_signal_snapshot_rows
    FOR EACH ROW EXECUTE FUNCTION public.owner_equity_append_only_guard();

CREATE FUNCTION public.retry_owner_equity_membership(
    p_membership_id uuid,
    p_code_commit text,
    p_entitlement_sha256 text
)
RETURNS void
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $retry$
DECLARE
    v_actor uuid;
BEGIN
    v_actor := NULLIF(
        pg_catalog.current_setting('app.actor_user_id', true), ''
    )::uuid;
    IF v_actor IS NULL THEN
        RAISE EXCEPTION 'owner equity retry requires an actor'
            USING ERRCODE = '42501';
    END IF;
    UPDATE public.owner_equity_memberships
       SET state = 'REQUESTED',
           transition_actor_user_id = v_actor,
           transition_code_commit = p_code_commit,
           transition_entitlement_sha256 = p_entitlement_sha256,
           error_code = NULL,
           error_retryable = NULL,
           disabled_at = NULL,
           updated_at = pg_catalog.now()
     WHERE id = p_membership_id
       AND owner_user_id = v_actor
       AND (
            state = 'INSUFFICIENT_HISTORY'
            OR (state = 'FAILED' AND error_retryable)
       );
    IF NOT FOUND THEN
        RAISE EXCEPTION 'owner equity membership is not retryable'
            USING ERRCODE = '42501';
    END IF;
END
$retry$;

ALTER FUNCTION public.retry_owner_equity_membership(uuid, text, text)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.retry_owner_equity_membership(uuid, text, text)
    FROM PUBLIC, worker, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.retry_owner_equity_membership(uuid, text, text) TO app;

CREATE FUNCTION public.disable_owner_equity_membership(
    p_membership_id uuid,
    p_code_commit text,
    p_entitlement_sha256 text
)
RETURNS void
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $disable$
DECLARE
    v_actor uuid;
BEGIN
    v_actor := NULLIF(
        pg_catalog.current_setting('app.actor_user_id', true), ''
    )::uuid;
    IF v_actor IS NULL THEN
        RAISE EXCEPTION 'owner equity disable requires an actor'
            USING ERRCODE = '42501';
    END IF;
    UPDATE public.owner_equity_memberships
       SET state = 'DISABLED',
           transition_actor_user_id = v_actor,
           transition_code_commit = p_code_commit,
           transition_entitlement_sha256 = p_entitlement_sha256,
           error_code = NULL,
           error_retryable = NULL,
           disabled_at = pg_catalog.now(),
           updated_at = pg_catalog.now()
     WHERE id = p_membership_id
       AND owner_user_id = v_actor
       AND state <> 'DISABLED';
    IF NOT FOUND THEN
        RAISE EXCEPTION 'owner equity membership is not active'
            USING ERRCODE = '42501';
    END IF;
END
$disable$;

ALTER FUNCTION public.disable_owner_equity_membership(uuid, text, text)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.disable_owner_equity_membership(uuid, text, text)
    FROM PUBLIC, worker, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.disable_owner_equity_membership(uuid, text, text) TO app;

-- Enqueue one exact close-driven incremental generation. The worker receives
-- no INSERT privilege on jobs; this narrow function revalidates every owner,
-- membership, calendar, entitlement, policy, generation, and payload pin.
CREATE FUNCTION public.schedule_owner_equity_incremental(
    p_owner_user_id uuid,
    p_membership_id uuid,
    p_as_of date,
    p_code_commit text,
    p_entitlement_reference text,
    p_entitlement_sha256 text
)
RETURNS TABLE (job_id uuid, inserted boolean)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $schedule_incremental$
DECLARE
    v_now_kst timestamp;
    v_latest_eligible date;
    v_confirmed_close date;
    v_instrument_id text;
    v_prior_generation bigint;
    v_prior_last_session date;
    v_max_active integer;
    v_target integer;
    v_minimum integer;
    v_identity text;
    v_body_hash text;
    v_key text;
    v_payload jsonb;
    v_job_id uuid;
    v_existing_payload jsonb;
BEGIN
    IF p_owner_user_id IS NULL
       OR p_membership_id IS NULL
       OR p_as_of IS NULL
       OR p_code_commit !~ '^[0-9a-f]{7,64}$'
       OR p_entitlement_reference IS NULL
       OR pg_catalog.btrim(p_entitlement_reference) = ''
       OR pg_catalog.length(p_entitlement_reference) > 512
       OR p_entitlement_reference ~ '[[:cntrl:]]'
       OR p_entitlement_sha256 !~ '^sha256:[0-9a-f]{64}$'
    THEN
        RAISE EXCEPTION 'owner equity incremental pins are invalid'
            USING ERRCODE = '22023';
    END IF;

    v_now_kst := pg_catalog.clock_timestamp() AT TIME ZONE 'Asia/Seoul';
    v_latest_eligible := v_now_kst::date;
    IF v_now_kst::time < TIME '16:30' THEN
        v_latest_eligible := v_latest_eligible - 1;
    END IF;
    SELECT calendar.session_date
      INTO v_confirmed_close
      FROM public.trading_calendars AS calendar
     WHERE calendar.exchange = 'KRX'
       AND calendar.session_type = 'TRADING'
       AND calendar.timezone = 'Asia/Seoul'
       AND calendar.session_date <= v_latest_eligible
       AND calendar.source_batch_id IS NOT NULL
       AND calendar.content_sha256 IS NOT NULL
       AND calendar.retrieved_at IS NOT NULL
     ORDER BY calendar.session_date DESC
     LIMIT 1;
    IF v_confirmed_close IS NULL OR p_as_of IS DISTINCT FROM v_confirmed_close THEN
        RAISE EXCEPTION 'owner equity confirmed close is unavailable'
            USING ERRCODE = '55000';
    END IF;

    PERFORM pg_catalog.set_config('app.actor_user_id', p_owner_user_id::text, true);
    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended(p_owner_user_id::text || '|' || p_membership_id::text, 0)
    );

    SELECT membership.instrument_id
      INTO v_instrument_id
      FROM public.owner_equity_memberships AS membership
     WHERE membership.id = p_membership_id
       AND membership.owner_user_id = p_owner_user_id
       AND membership.state = 'READY'
     FOR UPDATE OF membership;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'ready owner equity membership is required'
            USING ERRCODE = '42501';
    END IF;

    SELECT policy.max_active_instruments,
           policy.target_observed_sessions,
           policy.minimum_observed_sessions
      INTO v_max_active, v_target, v_minimum
      FROM public.owner_equity_universe_policies AS policy
     WHERE policy.owner_user_id = p_owner_user_id
     FOR SHARE OF policy;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'owner equity policy is unavailable'
            USING ERRCODE = '55000';
    END IF;

    PERFORM 1
     FROM public.data_entitlements AS entitlement
     WHERE entitlement.contract_document_sha256 =
               pg_catalog.substring(p_entitlement_sha256, 8)
       AND entitlement.contract_reference = p_entitlement_reference
       AND entitlement.status = 'ACTIVE'
       AND entitlement.effective_from <= p_as_of
       AND entitlement.effective_until >= p_as_of
     FOR SHARE OF entitlement;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'owner equity entitlement is unavailable'
            USING ERRCODE = '55000';
    END IF;

    SELECT generation.generation, generation.last_session
      INTO v_prior_generation, v_prior_last_session
      FROM public.owner_equity_instrument_generations AS generation
      JOIN public.owner_equity_generation_admissions AS admission
        ON admission.generation_id = generation.id
       AND admission.owner_user_id = generation.owner_user_id
       AND admission.membership_id = generation.membership_id
       AND admission.instrument_id = generation.instrument_id
       AND admission.generation = generation.generation
     WHERE generation.owner_user_id = p_owner_user_id
       AND generation.membership_id = p_membership_id
       AND generation.instrument_id = v_instrument_id
     ORDER BY generation.generation DESC
     LIMIT 1
     FOR SHARE OF generation, admission;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'admitted owner equity generation is unavailable'
            USING ERRCODE = '55000';
    END IF;
    IF v_prior_last_session >= p_as_of THEN
        RETURN;
    END IF;

    -- Do not create a second job for the same membership while a prior daily
    -- generation is still claimable or running.
    IF EXISTS (
        SELECT 1
          FROM public.jobs AS active_job
         WHERE active_job.owner_user_id = p_owner_user_id
           AND active_job.job_type = 'owner_equity_v2'
           AND active_job.status IN ('QUEUED', 'RUNNING')
           AND active_job.payload_json ->> 'action' = 'INCREMENTAL'
           AND active_job.payload_json ->> 'membership_id' = p_membership_id::text
    ) THEN
        RETURN;
    END IF;

    v_identity := pg_catalog.concat_ws(
        '|', p_owner_user_id::text, p_membership_id::text,
        pg_catalog.to_char(p_as_of, 'YYYY-MM-DD'), v_prior_generation::text,
        p_code_commit, p_entitlement_reference, p_entitlement_sha256
    );
    v_body_hash := pg_catalog.encode(
        pg_catalog.sha256(pg_catalog.convert_to(v_identity, 'UTF8')), 'hex'
    );
    v_key := 'oev2:incremental:' || v_body_hash;
    v_payload := pg_catalog.jsonb_build_object(
        'schema_version', 1,
        'action', 'INCREMENTAL',
        'membership_id', p_membership_id,
        'instrument_id', v_instrument_id,
        'expected_generation', v_prior_generation + 1,
        'request_body_sha256', v_body_hash,
        'requested_through', pg_catalog.to_char(p_as_of, 'YYYY-MM-DD'),
        'max_active_instruments', v_max_active,
        'target_observed_sessions', v_target,
        'minimum_observed_sessions', v_minimum,
        'code_commit', p_code_commit,
        'entitlement_reference', p_entitlement_reference,
        'entitlement_sha256', p_entitlement_sha256
    );

    INSERT INTO public.jobs (
        owner_user_id, job_type, status, priority, idempotency_key,
        payload_json, max_attempts
    )
    VALUES (
        p_owner_user_id, 'owner_equity_v2', 'QUEUED', 20, v_key,
        v_payload, 3
    )
    ON CONFLICT (owner_user_id, idempotency_key) DO NOTHING
    RETURNING jobs.id INTO v_job_id;

    IF v_job_id IS NULL THEN
        SELECT existing.id, existing.payload_json
          INTO v_job_id, v_existing_payload
          FROM public.jobs AS existing
         WHERE existing.owner_user_id = p_owner_user_id
           AND existing.idempotency_key = v_key
         FOR UPDATE OF existing;
        IF NOT FOUND OR v_existing_payload IS DISTINCT FROM v_payload THEN
            RAISE EXCEPTION 'owner equity incremental identity conflicts with lineage'
                USING ERRCODE = '23514';
        END IF;
        RETURN QUERY SELECT v_job_id, false;
        RETURN;
    END IF;

    RETURN QUERY SELECT v_job_id, true;
END
$schedule_incremental$;

ALTER FUNCTION public.schedule_owner_equity_incremental(uuid, uuid, date, text, text, text)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION
    public.schedule_owner_equity_incremental(uuid, uuid, date, text, text, text)
    FROM PUBLIC, app, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION
    public.schedule_owner_equity_incremental(uuid, uuid, date, text, text, text)
    TO worker;
