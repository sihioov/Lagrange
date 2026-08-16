-- Run directly as app with Owner A's actor context. Both calls deliberately
-- supply Owner B's UUID; the expected result is an authorization error. The
-- surrounding transaction is rolled back so a buggy function cannot leave a
-- user, invitation, or audit row in the disposable database.
\set ON_ERROR_STOP on
BEGIN;
SELECT set_config('app.actor_user_id', '00000000-0000-4000-8000-000000000039', true);

DO $$
DECLARE
    v_message text;
    v_sqlstate text;
    v_arity integer;
    v_capability uuid;
    v_created uuid;
BEGIN
    BEGIN
        SELECT p.pronargs
          INTO v_arity
          FROM pg_catalog.pg_proc AS p
          JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace
         WHERE n.nspname = 'public'
           AND p.proname = 'create_invitation';
        IF v_arity = 5 THEN
            v_created := public.create_invitation(
                '00000000-0000-4000-8000-00000000003a',
                'cross-owner-create@example.test',
                'member',
                repeat('c', 64),
                extract(epoch FROM (clock_timestamp() + interval '1 day'))::bigint
            );
        ELSIF v_arity = 6 THEN
            -- The capability form binds Owner A before the mutation receives
            -- Owner B. It is still one transaction and one direct app login.
            v_capability := public.authenticate_identity_actor(
                '00000000-0000-4000-8000-000000000039', repeat('d', 64)
            );
            EXECUTE 'SELECT public.create_invitation($1::uuid, $2::text, $3::text, $4::text, $5::bigint, $6::uuid)'
                INTO v_created
                USING
                    '00000000-0000-4000-8000-00000000003a',
                    'cross-owner-create@example.test',
                    'member',
                    repeat('c', 64),
                    extract(epoch FROM (clock_timestamp() + interval '1 day'))::bigint,
                    v_capability;
        ELSE
            RAISE EXCEPTION 'validator_create_invitation_arity_%', v_arity;
        END IF;
        RAISE EXCEPTION 'validator_cross_owner_create_accepted';
    EXCEPTION WHEN OTHERS THEN
        GET STACKED DIAGNOSTICS
            v_message = MESSAGE_TEXT,
            v_sqlstate = RETURNED_SQLSTATE;
        IF v_message = 'validator_cross_owner_create_accepted'
           OR v_sqlstate <> '42501'
        THEN
            RAISE;
        END IF;
    END;
END
$$;

DO $$
DECLARE
    v_message text;
    v_sqlstate text;
    v_arity integer;
    v_argnames text[];
    v_capability uuid;
    v_claimed boolean;
BEGIN
    BEGIN
        SELECT p.pronargs, p.proargnames
          INTO v_arity, v_argnames
          FROM pg_catalog.pg_proc AS p
          JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace
         WHERE n.nspname = 'public'
           AND p.proname = 'claim_invitation';
        IF v_arity = 4 THEN
            v_claimed := public.claim_invitation(
                '00000000-0000-4000-8000-00000000003a',
                '00000000-0000-4000-8000-000000000395',
                'https://validation.invalid',
                'cross-owner-claim'
            );
        ELSIF v_arity = 5
          AND 'p_actor_capability' = ANY (v_argnames)
        THEN
            v_capability := public.authenticate_identity_actor(
                '00000000-0000-4000-8000-000000000039', repeat('d', 64)
            );
            EXECUTE 'SELECT public.claim_invitation($1::uuid, $2::uuid, $3::text, $4::text, $5::uuid)'
                INTO v_claimed
                USING
                    '00000000-0000-4000-8000-00000000003a',
                    '00000000-0000-4000-8000-000000000395',
                    'https://validation.invalid',
                    'cross-owner-claim',
                    v_capability;
        ELSIF v_arity = 5
          AND 'p_invite_hash' = ANY (v_argnames)
        THEN
            -- Some finalized lanes bind the invitation hash as the fifth
            -- boundary input before adopting the actor capability adapter.
            v_claimed := public.claim_invitation(
                '00000000-0000-4000-8000-00000000003a',
                '00000000-0000-4000-8000-000000000395',
                repeat('b', 64),
                'https://validation.invalid',
                'cross-owner-claim'
            );
        ELSE
            RAISE EXCEPTION 'validator_claim_invitation_arity_%', v_arity;
        END IF;
        IF v_claimed IS DISTINCT FROM false THEN
            RAISE EXCEPTION 'validator_cross_owner_claim_accepted';
        END IF;
    EXCEPTION WHEN OTHERS THEN
        GET STACKED DIAGNOSTICS
            v_message = MESSAGE_TEXT,
            v_sqlstate = RETURNED_SQLSTATE;
        IF v_message = 'validator_cross_owner_claim_accepted'
           OR v_sqlstate <> '42501'
        THEN
            RAISE;
        END IF;
    END;
END
$$;

ROLLBACK;
SELECT 'identity_boundary=pass';
