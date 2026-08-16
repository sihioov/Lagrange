-- 0040: narrowly scoped identity provisioning for the confidential auth
-- router. Serving roles remain read-only on users/user_roles; these functions
-- are the only app capability that can create an invitation or bind a newly
-- redeemed OIDC identity.

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

-- Invitations are mutated only through the capability-bound SECURITY DEFINER
-- functions below.  The serving role may inspect them for authenticated
-- flows, but a caller-controlled actor GUC must never authorize direct DML.
REVOKE INSERT, UPDATE, DELETE ON TABLE public.invitations FROM app;
GRANT SELECT ON TABLE public.invitations TO app;

-- The invitation role is data, not a caller-controlled table name. Keep the
-- check local to this table because the migration-contract scratch database
-- intentionally does not seed role rows until its test fixtures run.
ALTER TABLE public.invitations
    ADD COLUMN role_id text NOT NULL DEFAULT 'member',
    ADD CONSTRAINT invitations_role_id_check
        CHECK (role_id IN ('owner', 'member'));

-- Ensure a freshly migrated database has the two role records required by the
-- auth protocol. ON CONFLICT preserves operator-managed descriptions and
-- existing role assignments.
INSERT INTO public.roles (id, description)
VALUES ('owner', 'Owner'), ('member', 'Member')
ON CONFLICT (id) DO NOTHING;

-- Keep the inviter provenance on a newly created identity so the later role
-- finalization call can re-enter the invitation's FORCE-RLS policy without a
-- broad migration-owner policy. Existing operator-provisioned users remain
-- NULL and cannot use this first-login-only function.
ALTER TABLE public.users
    ADD COLUMN provisioned_by_user_id uuid REFERENCES public.users(id);

-- An app connection is shared by every authenticated request, so a caller
-- supplied actor GUC is not an authentication boundary.  The narrow adapter
-- below mints a transaction-bound capability only after it proves that the
-- presented session hash belongs to the requested, live Owner session.  The
-- capability table has no serving-role grants; only the definer functions can
-- consume it.  `backend_pid` and `transaction_id` make a capability unusable
-- if an application accidentally carries it across pool connections or
-- transactions.
CREATE TABLE public.identity_actor_capabilities (
    id             uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    actor_user_id  uuid NOT NULL REFERENCES public.users(id) ON DELETE CASCADE,
    backend_pid    integer NOT NULL,
    transaction_id bigint NOT NULL,
    issued_at      timestamptz NOT NULL DEFAULT pg_catalog.clock_timestamp(),
    expires_at     timestamptz NOT NULL,
    CONSTRAINT identity_actor_capability_expiry_check
        CHECK (expires_at > issued_at)
);

ALTER TABLE public.identity_actor_capabilities ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.identity_actor_capabilities FORCE ROW LEVEL SECURITY;
CREATE POLICY identity_actor_capabilities_owner_all
    ON public.identity_actor_capabilities FOR ALL TO migration_owner
    USING (true) WITH CHECK (true);
REVOKE ALL ON TABLE public.identity_actor_capabilities
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;

CREATE FUNCTION public.authenticate_identity_actor(
    p_owner_user_id uuid,
    p_session_hash text
)
RETURNS uuid
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
SET lock_timeout = '1s'
SET statement_timeout = '5s'
AS $function$
DECLARE
    v_capability uuid;
BEGIN
    IF p_owner_user_id IS NULL
        OR p_session_hash IS NULL
        OR p_session_hash !~ '^[0-9a-f]{64}$'
    THEN
        RAISE EXCEPTION 'identity actor capability is invalid'
            USING ERRCODE = '42501';
    END IF;

    -- This GUC only selects the requested tenant row for FORCE-RLS.  It is
    -- deliberately not used as proof of identity: the session hash and live
    -- Owner role below are the authentication boundary.
    PERFORM pg_catalog.set_config(
        'app.actor_user_id', p_owner_user_id::text, true
    );
    IF NOT EXISTS (
        SELECT 1
        FROM public.web_sessions AS session
        JOIN public.user_roles AS role
          ON role.user_id = session.user_id AND role.role_id = 'owner'
        WHERE session.user_id = p_owner_user_id
          AND session.session_hash = p_session_hash
          AND session.revoked_at IS NULL
          AND session.expires_at > pg_catalog.clock_timestamp()
    ) THEN
        RAISE EXCEPTION 'identity actor session is not an active Owner session'
            USING ERRCODE = '42501';
    END IF;

    v_capability := pg_catalog.gen_random_uuid();
    INSERT INTO public.identity_actor_capabilities (
        id, actor_user_id, backend_pid, transaction_id, expires_at
    )
    VALUES (
        v_capability,
        p_owner_user_id,
        pg_catalog.pg_backend_pid(),
        pg_catalog.txid_current(),
        pg_catalog.clock_timestamp() + pg_catalog.make_interval(secs => 30)
    );
    RETURN v_capability;
END
$function$;

ALTER FUNCTION public.authenticate_identity_actor(uuid, text)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.authenticate_identity_actor(uuid, text)
    FROM PUBLIC, worker, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.authenticate_identity_actor(uuid, text) TO app;

-- Consume the capability inside the same transaction in which it was minted.
-- This is intentionally private (no EXECUTE grant) and is called only by the
-- two mutation functions below.
CREATE FUNCTION public.consume_identity_actor_capability(
    p_owner_user_id uuid,
    p_capability uuid
)
RETURNS void
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
SET lock_timeout = '1s'
SET statement_timeout = '5s'
AS $function$
DECLARE
    v_actor_user_id uuid;
BEGIN
    SELECT capability.actor_user_id
      INTO v_actor_user_id
      FROM public.identity_actor_capabilities AS capability
     WHERE capability.id = p_capability
       AND capability.backend_pid = pg_catalog.pg_backend_pid()
       AND capability.transaction_id = pg_catalog.txid_current()
       AND capability.expires_at > pg_catalog.clock_timestamp()
     FOR UPDATE;
    IF NOT FOUND OR v_actor_user_id IS DISTINCT FROM p_owner_user_id THEN
        RAISE EXCEPTION 'identity actor capability is invalid'
            USING ERRCODE = '42501';
    END IF;
    DELETE FROM public.identity_actor_capabilities
     WHERE id = p_capability;
END
$function$;

ALTER FUNCTION public.consume_identity_actor_capability(uuid, uuid)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.consume_identity_actor_capability(uuid, uuid)
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;

-- RLS intentionally hides another Owner's invitation rows from the serving
-- path; the unique index is the global race-proof boundary that still stops
-- two Owners from issuing simultaneous pending invites for one email.  Fail
-- closed if an older database already contains duplicate pending addresses;
-- silently choosing one row would make the migration non-deterministic.
-- The migration connection has no tenant actor, and invitations is FORCE-RLS;
-- temporarily remove FORCE only for this global invariant check.  Restore it
-- before creating any serving function so the policy remains actor-scoped.
ALTER TABLE public.invitations NO FORCE ROW LEVEL SECURITY;
DO $guard$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM public.invitations
        WHERE status = 'PENDING'
        GROUP BY pg_catalog.lower(pg_catalog.btrim(email))
        HAVING pg_catalog.count(*) > 1
    ) THEN
        RAISE EXCEPTION
            'duplicate pending invitation emails require manual resolution'
            USING ERRCODE = '23505';
    END IF;
END
$guard$;
ALTER TABLE public.invitations FORCE ROW LEVEL SECURITY;

CREATE UNIQUE INDEX invitations_pending_email_uq
    ON public.invitations (pg_catalog.lower(pg_catalog.btrim(email)))
    WHERE status = 'PENDING';

-- A private, transaction-bound marker lets the definer-only expiry helper
-- perform the one cross-owner PENDING -> EXPIRED transition required by the
-- global email index.  No serving role can mint or read this marker.  Binding
-- it to the backend, transaction, and normalized email prevents a caller from
-- turning it into a general invitation read/update capability.
CREATE TABLE public.identity_invitation_expiry_capabilities (
    id             uuid PRIMARY KEY DEFAULT pg_catalog.gen_random_uuid(),
    email          text NOT NULL,
    backend_pid    integer NOT NULL,
    transaction_id bigint NOT NULL,
    expires_at     timestamptz NOT NULL,
    CONSTRAINT identity_invitation_expiry_capability_expiry_check
        CHECK (expires_at > pg_catalog.clock_timestamp())
);
ALTER TABLE public.identity_invitation_expiry_capabilities ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.identity_invitation_expiry_capabilities FORCE ROW LEVEL SECURITY;
CREATE POLICY identity_invitation_expiry_capabilities_owner_all
    ON public.identity_invitation_expiry_capabilities FOR ALL TO migration_owner
    USING (true) WITH CHECK (true);
REVOKE ALL ON TABLE public.identity_invitation_expiry_capabilities
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;

-- FORCE RLS normally limits migration_owner to the current actor.  Expired
-- pending rows are the one cross-owner cleanup exception needed by the global
-- email uniqueness index: the policy permits only a PENDING -> EXPIRED
-- transition for rows already past expiry, never reads or other mutations.
DROP POLICY IF EXISTS tenant_all_owner_invitations ON public.invitations;
CREATE POLICY tenant_all_owner_invitations ON public.invitations
    FOR ALL TO migration_owner
    USING (
        user_id = NULLIF(current_setting('app.actor_user_id', true), '')::uuid
        OR (status = 'PENDING' AND expires_at <= pg_catalog.clock_timestamp())
        OR EXISTS (
            SELECT 1
            FROM public.identity_invitation_expiry_capabilities AS capability
            WHERE capability.email = pg_catalog.lower(pg_catalog.btrim(public.invitations.email))
              AND capability.backend_pid = pg_catalog.pg_backend_pid()
              AND capability.transaction_id = pg_catalog.txid_current()
              AND capability.expires_at > pg_catalog.clock_timestamp()
        )
    )
    WITH CHECK (
        user_id = NULLIF(current_setting('app.actor_user_id', true), '')::uuid
        OR (status = 'EXPIRED' AND expires_at <= pg_catalog.clock_timestamp())
        OR EXISTS (
            SELECT 1
            FROM public.identity_invitation_expiry_capabilities AS capability
            WHERE capability.email = pg_catalog.lower(pg_catalog.btrim(public.invitations.email))
              AND capability.backend_pid = pg_catalog.pg_backend_pid()
              AND capability.transaction_id = pg_catalog.txid_current()
              AND capability.expires_at > pg_catalog.clock_timestamp()
        )
    );

-- Narrow SECURITY DEFINER cleanup used only by create_invitation. It returns
-- no row data and takes the same transaction advisory lock as duplicate
-- detection, so an Owner cannot observe or select another tenant's invite.
CREATE FUNCTION public.expire_pending_invitations(p_email text)
RETURNS void
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
SET lock_timeout = '1s'
SET statement_timeout = '5s'
AS $function$
DECLARE
    v_email text;
    v_capability uuid;
BEGIN
    v_email := pg_catalog.lower(pg_catalog.btrim(coalesce(p_email, '')));
    IF v_email !~ '^[^[:space:]@]+@[^[:space:]@]+$'
        OR pg_catalog.length(v_email) > 320
    THEN
        RAISE EXCEPTION 'invitation email is invalid'
            USING ERRCODE = '22023';
    END IF;

    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended(v_email, 39039)
    );
    INSERT INTO public.identity_invitation_expiry_capabilities (
        email, backend_pid, transaction_id, expires_at
    )
    VALUES (
        v_email,
        pg_catalog.pg_backend_pid(),
        pg_catalog.txid_current(),
        pg_catalog.clock_timestamp() + pg_catalog.make_interval(secs => 5)
    )
    RETURNING id INTO v_capability;
    UPDATE public.invitations
    SET status = 'EXPIRED'
    WHERE pg_catalog.lower(pg_catalog.btrim(email)) = v_email
      AND status = 'PENDING'
      AND expires_at <= pg_catalog.clock_timestamp();
    DELETE FROM public.identity_invitation_expiry_capabilities
     WHERE id = v_capability;
END
$function$;

ALTER FUNCTION public.expire_pending_invitations(text)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.expire_pending_invitations(text)
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;

-- Owner-only invitation creation. The caller supplies the actor explicitly,
-- but authorization is re-checked against users/user_roles inside this
-- SECURITY DEFINER function. The invitation tenant policy is satisfied by a
-- transaction-local actor GUC set only after input validation.
CREATE FUNCTION public.create_invitation(
    p_owner_user_id uuid,
    p_email text,
    p_role_id text,
    p_invite_hash text,
    p_expires_at_epoch bigint,
    p_actor_capability uuid
)
RETURNS uuid
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
SET lock_timeout = '1s'
SET statement_timeout = '5s'
AS $function$
DECLARE
    v_email text;
    v_expires_at timestamptz;
    v_invitation_id uuid;
BEGIN
    IF p_owner_user_id IS NULL
        OR p_role_id IS NULL
        OR p_invite_hash IS NULL
        OR p_expires_at_epoch IS NULL
        OR p_actor_capability IS NULL
    THEN
        RAISE EXCEPTION 'identity provisioning input is incomplete'
            USING ERRCODE = '22023';
    END IF;

    v_email := pg_catalog.lower(pg_catalog.btrim(coalesce(p_email, '')));
    IF v_email !~ '^[^[:space:]@]+@[^[:space:]@]+$'
        OR pg_catalog.length(v_email) > 320
    THEN
        RAISE EXCEPTION 'invitation email is invalid'
            USING ERRCODE = '22023';
    END IF;
    IF p_role_id NOT IN ('owner', 'member') THEN
        RAISE EXCEPTION 'invitation role is invalid'
            USING ERRCODE = '22023';
    END IF;
    IF p_invite_hash !~ '^[0-9a-f]{64}$' THEN
        RAISE EXCEPTION 'invitation hash is invalid'
            USING ERRCODE = '22023';
    END IF;

    v_expires_at := pg_catalog.to_timestamp(p_expires_at_epoch::double precision);
    IF v_expires_at <= pg_catalog.clock_timestamp()
        OR v_expires_at > pg_catalog.clock_timestamp() + pg_catalog.make_interval(days => 31)
    THEN
        RAISE EXCEPTION 'invitation expiry is outside the permitted window'
            USING ERRCODE = '22023';
    END IF;

    -- The supplied owner is bound to an authenticated, transaction-local
    -- capability minted by authenticate_identity_actor. A caller may set
    -- app.actor_user_id to any UUID, but that alone cannot satisfy this check.
    PERFORM public.consume_identity_actor_capability(
        p_owner_user_id, p_actor_capability
    );

    -- Every global identity/provisional check below must observe one
    -- normalized-email serialization point.  This lock is transaction-scoped,
    -- so a concurrent claim cannot insert or finalize an identity between the
    -- checks and the invitation insert.  The function is SECURITY DEFINER and
    -- owned by migration_owner; it reads shared users data without granting
    -- the serving role any cross-tenant table capability.
    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended(v_email, 39039)
    );

    IF NOT EXISTS (
        SELECT 1
        FROM public.roles
        WHERE id = p_role_id
    ) THEN
        RAISE EXCEPTION 'invitation role is not provisioned'
            USING ERRCODE = '55000';
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM public.user_roles
        WHERE user_id = p_owner_user_id AND role_id = 'owner'
    ) THEN
        RAISE EXCEPTION 'only an Owner may create invitations'
            USING ERRCODE = '42501';
    END IF;

    -- User identity is globally unique, including provisional identities
    -- redeemed by another Owner. This check runs inside the narrowly scoped
    -- SECURITY DEFINER function so FORCE-RLS on tenant invitations cannot
    -- turn a cross-Owner existing-user check into an information leak.
    IF EXISTS (
        SELECT 1
        FROM public.users AS existing_user
        WHERE pg_catalog.lower(pg_catalog.btrim(existing_user.email)) = v_email
    ) THEN
        RAISE EXCEPTION 'an identity already exists for this email'
            USING ERRCODE = '23505';
    END IF;

    PERFORM pg_catalog.set_config(
        'app.actor_user_id', p_owner_user_id::text, true
    );

    -- Expired pending rows no longer reserve an address. The helper acquires
    -- the email advisory lock before this duplicate check and insert.
    PERFORM public.expire_pending_invitations(v_email);
    IF EXISTS (
        SELECT 1
        FROM public.invitations AS invitation
        WHERE pg_catalog.lower(pg_catalog.btrim(invitation.email)) = v_email
          AND (
              invitation.status = 'PENDING'
              OR (
                  invitation.status = 'REDEEMED'
                  AND EXISTS (
                      SELECT 1
                      FROM public.users AS provisional_user
                      WHERE provisional_user.id = invitation.redeemed_by_user_id
                        AND provisional_user.provisioned_by_user_id IS NOT NULL
                  )
              )
          )
    ) THEN
        RAISE EXCEPTION 'an active invitation already exists for this email'
            USING ERRCODE = '23505';
    END IF;

    INSERT INTO public.invitations (
        user_id, email, invite_hash, status, expires_at, role_id
    )
    VALUES (
        p_owner_user_id, v_email, p_invite_hash, 'PENDING', v_expires_at, p_role_id
    )
    RETURNING id INTO v_invitation_id;
    PERFORM public.enqueue_auth_audit(
        'invite:' || v_invitation_id::text || ':created',
        'auth.invite_created', p_owner_user_id, 'invitation',
        v_invitation_id::text, NULL,
        EXTRACT(EPOCH FROM pg_catalog.clock_timestamp())::bigint
    );
    RETURN v_invitation_id;
END
$function$;

ALTER FUNCTION public.create_invitation(uuid, text, text, text, bigint, uuid)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.create_invitation(uuid, text, text, text, bigint, uuid)
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.create_invitation(uuid, text, text, text, bigint, uuid)
    TO app;

-- Atomically consume one pending invitation and create the first durable user
-- binding. The invitation row is locked before any identity is inserted, so
-- concurrent callbacks can produce at most one user and one redemption.
CREATE FUNCTION public.claim_invitation(
    p_owner_user_id uuid,
    p_invitation_id uuid,
    p_invite_hash text,
    p_issuer text,
    p_subject text
)
RETURNS boolean
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
SET lock_timeout = '1s'
SET statement_timeout = '5s'
AS $function$
DECLARE
    v_invitation public.invitations%ROWTYPE;
    v_user_id uuid;
    v_actor uuid;
BEGIN
    IF p_owner_user_id IS NULL
        OR p_invitation_id IS NULL
        OR p_invite_hash IS NULL
        OR p_issuer IS NULL
        OR p_subject IS NULL
        OR pg_catalog.length(p_issuer) = 0
        OR pg_catalog.length(p_subject) = 0
        OR pg_catalog.length(p_issuer) > 512
        OR pg_catalog.length(p_subject) > 512
        OR p_invite_hash !~ '^[0-9a-f]{64}$'
        OR p_issuer ~ '[[:cntrl:]]'
        OR p_subject ~ '[[:cntrl:]]'
    THEN
        RAISE EXCEPTION 'identity binding input is invalid'
            USING ERRCODE = '22023';
    END IF;

    -- If an app pool already carries a request actor, a supplied Owner must
    -- agree with it.  This is only a consistency check (the invitation hash
    -- remains the capability); it prevents an Owner-A direct SQL session from
    -- switching the function's internal GUC to Owner-B.
    IF session_user <> 'migration_owner' THEN
        v_actor := NULLIF(
            pg_catalog.current_setting('app.actor_user_id', true), ''
        )::uuid;
        IF v_actor IS NOT NULL AND v_actor IS DISTINCT FROM p_owner_user_id THEN
            RETURN false;
        END IF;
    END IF;

    -- A legacy/operator-created invitation must not become a capability for
    -- a non-Owner tenant. The normal create path already enforces this, but
    -- re-checking at redemption makes the boundary fail closed after role
    -- changes and across pre-existing rows. The invite hash is an
    -- invitation-scoped capability: an Owner-A request carrying only its own
    -- hash cannot redeem Owner-B's row even if it forges p_owner_user_id or
    -- app.actor_user_id.
    IF NOT EXISTS (
        SELECT 1
        FROM public.user_roles
        WHERE user_id = p_owner_user_id AND role_id = 'owner'
    ) THEN
        RETURN false;
    END IF;

    -- The invitation is tenant-owned by its creating Owner. Set the actor
    -- before reading it because invitations are FORCE-RLS with an owner-local
    -- policy even for migration_owner. Read the email once to acquire the
    -- same advisory lock used by create_invitation, then lock the current row;
    -- this closes the race between a claim and a replacement invitation for a
    -- still-provisional address.
    PERFORM pg_catalog.set_config(
        'app.actor_user_id', p_owner_user_id::text, true
    );
    SELECT * INTO v_invitation
    FROM public.invitations
    WHERE id = p_invitation_id;

    IF NOT FOUND
        OR v_invitation.user_id <> p_owner_user_id
        OR v_invitation.invite_hash IS DISTINCT FROM p_invite_hash
    THEN
        RETURN false;
    END IF;
    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended(
            pg_catalog.lower(pg_catalog.btrim(v_invitation.email)), 39039
        )
    );
    SELECT * INTO v_invitation
    FROM public.invitations
    WHERE id = p_invitation_id
    FOR UPDATE;

    IF NOT FOUND
        OR v_invitation.user_id <> p_owner_user_id
        OR v_invitation.invite_hash IS DISTINCT FROM p_invite_hash
    THEN
        RETURN false;
    END IF;
    -- A callback can fail after claim_invitation has atomically created the
    -- provisional identity (for example, an unknown OIDC role claim).  The
    -- exact same immutable identity may safely retry that claim; a different
    -- identity must never take over the redeemed invitation.
    IF v_invitation.status = 'REDEEMED' THEN
        RETURN EXISTS (
            SELECT 1
            FROM public.users AS provisional_user
            WHERE provisional_user.id = v_invitation.redeemed_by_user_id
              AND provisional_user.provisioned_by_user_id IS NOT NULL
              AND provisional_user.issuer = p_issuer
              AND provisional_user.subject = p_subject
        );
    END IF;
    IF v_invitation.status <> 'PENDING' THEN
        RETURN false;
    END IF;
    IF v_invitation.expires_at <= pg_catalog.clock_timestamp() THEN
        UPDATE public.invitations
        SET status = 'EXPIRED'
        WHERE id = p_invitation_id;
        RETURN false;
    END IF;

    -- The invitation email is normalized by create_invitation (and is
    -- normalized again here for legacy rows).  Check the shared identity table
    -- while holding the same lock used by creation.  This is deliberately
    -- inside this SECURITY DEFINER function: the app role cannot perform a
    -- cross-tenant users/provisional read or bypass invitation FORCE-RLS.
    IF EXISTS (
        SELECT 1 FROM public.users
        WHERE pg_catalog.lower(pg_catalog.btrim(email)) =
              pg_catalog.lower(pg_catalog.btrim(v_invitation.email))
    ) THEN
        RETURN false;
    END IF;

    -- A pre-existing issuer/subject is never rebound through an invitation.
    -- The normal auth path finds it before calling this function; this check
    -- closes the race between two first-login requests on different invites.
    IF EXISTS (
        SELECT 1 FROM public.users
        WHERE issuer = p_issuer AND subject = p_subject
    ) THEN
        RETURN false;
    END IF;

    INSERT INTO public.users (issuer, subject, email, provisioned_by_user_id)
    VALUES (p_issuer, p_subject, v_invitation.email, p_owner_user_id)
    ON CONFLICT (issuer, subject) DO NOTHING
    RETURNING id INTO v_user_id;
    IF NOT FOUND THEN
        RETURN false;
    END IF;

    INSERT INTO public.user_roles (user_id, role_id, granted_by)
    VALUES (v_user_id, v_invitation.role_id, p_owner_user_id);

    UPDATE public.invitations
    SET status = 'REDEEMED',
        redeemed_by_user_id = v_user_id,
        redeemed_at = pg_catalog.clock_timestamp()
    WHERE id = p_invitation_id;
    PERFORM public.enqueue_auth_audit(
        'invite:' || p_invitation_id::text || ':redeemed:' || v_user_id::text,
        'auth.invite_redeemed', p_owner_user_id, 'invitation',
        p_invitation_id::text, NULL,
        EXTRACT(EPOCH FROM pg_catalog.clock_timestamp())::bigint
    );
    RETURN true;
END
$function$;

ALTER FUNCTION public.claim_invitation(uuid, uuid, text, text, text)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.claim_invitation(uuid, uuid, text, text, text)
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.claim_invitation(uuid, uuid, text, text, text)
    TO app;

-- Finalize the role/email chosen by the validated OIDC claims. This function
-- cannot mutate a pre-provisioned identity: it requires the user to be the
-- redeemed target of an invitation created by claim_invitation above.
CREATE FUNCTION public.bind_redeemed_identity(
    p_issuer text,
    p_subject text,
    p_email text,
    p_role_id text
)
RETURNS uuid
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
SET lock_timeout = '1s'
SET statement_timeout = '5s'
AS $function$
DECLARE
    v_email text;
    v_user_id uuid;
    v_granted_by uuid;
    v_inviter uuid;
    v_invitation public.invitations%ROWTYPE;
BEGIN
    IF p_issuer IS NULL
        OR p_subject IS NULL
        OR p_role_id IS NULL
        OR pg_catalog.length(p_issuer) = 0
        OR pg_catalog.length(p_subject) = 0
        OR pg_catalog.length(p_issuer) > 512
        OR pg_catalog.length(p_subject) > 512
        OR p_issuer ~ '[[:cntrl:]]'
        OR p_subject ~ '[[:cntrl:]]'
        OR p_role_id NOT IN ('owner', 'member')
    THEN
        RAISE EXCEPTION 'identity binding input is invalid'
            USING ERRCODE = '22023';
    END IF;
    v_email := pg_catalog.lower(pg_catalog.btrim(coalesce(p_email, '')));
    IF v_email !~ '^[^[:space:]@]+@[^[:space:]@]+$'
        OR pg_catalog.length(v_email) > 320
    THEN
        RAISE EXCEPTION 'identity email is invalid'
            USING ERRCODE = '22023';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM public.roles WHERE id = p_role_id
    ) THEN
        RAISE EXCEPTION 'identity role is not provisioned'
            USING ERRCODE = '55000';
    END IF;

    -- Binding is the final state transition for the same address used by
    -- create_invitation and claim_invitation.  Take the common transaction
    -- lock before every global users/provisional read so a concurrent
    -- invitation cannot interleave with this identity finalization.
    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended(v_email, 39039)
    );

    SELECT id, provisioned_by_user_id
    INTO v_user_id, v_granted_by
    FROM public.users
    WHERE issuer = p_issuer AND subject = p_subject
    FOR UPDATE;
    IF NOT FOUND OR v_granted_by IS NULL THEN
        RAISE EXCEPTION 'identity is not linked to a redeemed invitation'
            USING ERRCODE = '42501';
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM public.user_roles
        WHERE user_id = v_granted_by AND role_id = 'owner'
    ) THEN
        RAISE EXCEPTION 'identity is not linked to a redeemed invitation'
            USING ERRCODE = '42501';
    END IF;

    -- The provenance column supplies the invitation tenant actor. Once set,
    -- the FORCE-RLS policy permits only this user's redeemed invitation row.
    PERFORM pg_catalog.set_config(
        'app.actor_user_id', v_granted_by::text, true
    );
    SELECT i.*
    INTO v_invitation
    FROM public.invitations AS i
    WHERE i.redeemed_by_user_id = v_user_id
      AND i.user_id = v_granted_by
    ORDER BY i.redeemed_at DESC
    LIMIT 1
    FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'identity is not linked to a redeemed invitation'
            USING ERRCODE = '42501';
    END IF;

    IF v_invitation.status <> 'REDEEMED'
        OR v_invitation.redeemed_by_user_id IS DISTINCT FROM v_user_id
        OR pg_catalog.lower(pg_catalog.btrim(v_invitation.email)) <> v_email
    THEN
        RAISE EXCEPTION 'identity is not linked to a redeemed invitation'
            USING ERRCODE = '42501';
    END IF;
    v_inviter := v_invitation.user_id;

    UPDATE public.users
    SET email = v_email,
        provisioned_by_user_id = NULL,
        updated_at = pg_catalog.clock_timestamp()
    WHERE id = v_user_id;
    DELETE FROM public.user_roles WHERE user_id = v_user_id;
    INSERT INTO public.user_roles (user_id, role_id, granted_by)
    VALUES (v_user_id, p_role_id, v_inviter);
    PERFORM public.enqueue_auth_audit(
        'user:' || v_user_id::text || ':bound',
        'auth.identity_bound', v_inviter, 'user', v_user_id::text, NULL,
        EXTRACT(EPOCH FROM pg_catalog.clock_timestamp())::bigint
    );
    RETURN v_user_id;
END
$function$;

ALTER FUNCTION public.bind_redeemed_identity(text, text, text, text)
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.bind_redeemed_identity(text, text, text, text)
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
GRANT EXECUTE ON FUNCTION public.bind_redeemed_identity(text, text, text, text)
    TO app;
