-- The official simulation worker verifies the exact migration ledger at startup.
-- Keep the grant read-only; migration_owner retains all write authority.
GRANT SELECT ON TABLE public._sqlx_migrations TO worker;
