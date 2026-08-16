-- The failed down must leave both the obligation and the migration ledger
-- intact. Remove the synthetic row after checking it so later stages start
-- cleanly.
\set ON_ERROR_STOP on
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
          FROM public.auth_audit_outbox
         WHERE event_key = 'validation:0039-rollback-guard'
           AND delivered_at IS NULL
    ) THEN
        RAISE EXCEPTION '0039 rollback guard did not preserve undelivered row';
    END IF;
    IF (SELECT count(*) FROM public._sqlx_migrations) <> 39 THEN
        RAISE EXCEPTION '0039 rollback guard changed the migration ledger';
    END IF;
END
$$;
DELETE FROM public.auth_audit_outbox
 WHERE event_key = 'validation:0039-rollback-guard';
SELECT 'rollback_guard=preserved_and_cleaned';
