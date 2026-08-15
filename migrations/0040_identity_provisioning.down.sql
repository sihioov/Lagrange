-- 0040 rollback: remove only the identity-provisioning capability. Existing
-- users/invitations are retained; operators must not lose identity data while
-- rolling back the serving function boundary.

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

-- The guard must inspect every tenant's retained invitation, while the
-- migration-owner FORCE-RLS policy intentionally exposes only the current
-- actor during serving operations. Temporarily disabling FORCE-RLS is scoped
-- to this transactional migration-owner rollback and is restored immediately
-- after the guard (or automatically by transaction rollback on failure).
ALTER TABLE public.invitations NO FORCE ROW LEVEL SECURITY;

-- Never turn a partially provisioned identity into an established identity by
-- dropping its provenance marker. Operators must either finish or explicitly
-- clean up the provisional row before rolling this migration back.
DO $guard$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM public.invitations
        WHERE role_id <> 'member'
    ) THEN
        RAISE EXCEPTION
            'cannot roll back identity provisioning while Owner invitations remain'
            USING ERRCODE = '55000';
    END IF;
    IF EXISTS (
        SELECT 1
        FROM public.users
        WHERE provisioned_by_user_id IS NOT NULL
    ) THEN
        RAISE EXCEPTION
            'cannot roll back identity provisioning while provisional identities exist'
            USING ERRCODE = '55000';
    END IF;
END
$guard$;

ALTER TABLE public.invitations FORCE ROW LEVEL SECURITY;

REVOKE ALL ON FUNCTION public.bind_redeemed_identity(text, text, text, text)
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
DROP FUNCTION public.bind_redeemed_identity(text, text, text, text);

REVOKE ALL ON FUNCTION public.claim_invitation(uuid, uuid, text, text)
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
DROP FUNCTION public.claim_invitation(uuid, uuid, text, text);

DROP FUNCTION public.expire_pending_invitations(text);

DROP POLICY IF EXISTS tenant_all_owner_invitations ON public.invitations;
CREATE POLICY tenant_all_owner_invitations ON public.invitations
    FOR ALL TO migration_owner
    USING (user_id = current_setting('app.actor_user_id', true)::uuid)
    WITH CHECK (user_id = current_setting('app.actor_user_id', true)::uuid);

REVOKE ALL ON FUNCTION public.create_invitation(uuid, text, text, text, bigint)
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
DROP FUNCTION public.create_invitation(uuid, text, text, text, bigint);

ALTER TABLE public.invitations
    DROP CONSTRAINT invitations_role_id_check,
    DROP COLUMN role_id;

DROP INDEX public.invitations_pending_email_uq;

ALTER TABLE public.users
    DROP COLUMN provisioned_by_user_id;
