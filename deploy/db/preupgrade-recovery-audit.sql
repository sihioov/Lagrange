-- Same read-only audit before clean shutdown and in the isolated physical clone.
-- One bounded result: migration checksums and a catalog fingerprint. No passwords,
-- application rows, role-setting values or function bodies cross into the result.
BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '120s';
SET LOCAL idle_in_transaction_session_timeout = '15s';
SET LOCAL search_path = pg_catalog, pg_temp;
SET LOCAL TimeZone = 'UTC';
SET LOCAL DateStyle = 'ISO, YMD';

DO $guard$
BEGIN
    IF current_user <> 'lagrange' OR session_user <> 'lagrange'
       OR NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = current_user AND rolsuper)
       OR current_setting('server_version_num')::integer NOT BETWEEN 180000 AND 189999 THEN
        RAISE EXCEPTION 'PREUPGRADE_DATABASE_IDENTITY' USING ERRCODE='42501';
    END IF;
    -- Other clients (including idle connections or other databases) invalidate
    -- the pre-stop fence. Prepared transactions can commit after a restart.
    IF EXISTS (SELECT 1 FROM pg_stat_activity WHERE backend_type = 'client backend'
               AND pid <> pg_backend_pid())
       OR EXISTS (SELECT 1 FROM pg_prepared_xacts) THEN
        RAISE EXCEPTION 'PREUPGRADE_DATABASE_WRITERS' USING ERRCODE='55006';
    END IF;
    IF (SELECT count(*) FROM public._sqlx_migrations) NOT BETWEEN 1 AND 59
       OR EXISTS (SELECT 1 FROM public._sqlx_migrations WHERE NOT success OR version < 1 OR version > 59
                  OR octet_length(checksum) <> 48) THEN
        RAISE EXCEPTION 'PREUPGRADE_DATABASE_MIGRATIONS' USING ERRCODE='23514';
    END IF;
    IF (SELECT count(*) FROM pg_class) > 20000
       OR (SELECT count(*) FROM pg_attribute) > 200000
       OR (SELECT count(*) FROM pg_proc) > 20000
       OR (SELECT count(*) FROM pg_authid) > 1000
       OR (SELECT count(*) FROM pg_auth_members) > 10000 THEN
        RAISE EXCEPTION 'PREUPGRADE_DATABASE_LIMIT' USING ERRCODE='54000';
    END IF;
END $guard$;

WITH user_schemas AS (
    SELECT oid FROM pg_namespace
    WHERE nspname !~ '^pg_' AND nspname <> 'information_schema'
), user_relations AS (
    SELECT oid FROM pg_class WHERE relnamespace IN (SELECT oid FROM user_schemas)
), catalog AS (
    SELECT jsonb_build_object(
        'schemas', (SELECT jsonb_agg(to_jsonb(n) ORDER BY n.oid) FROM pg_namespace n
                    WHERE n.oid IN (SELECT oid FROM user_schemas)),
        -- Planner/freeze statistics can change on restart; they are not schema,
        -- access policy or data identities. The full cluster bytes are retained.
        'relations', (SELECT jsonb_agg(to_jsonb(c) - ARRAY['relpages','reltuples','relallvisible',
                        'relallfrozen','relfrozenxid','relminmxid'] ORDER BY c.oid) FROM pg_class c
                      WHERE c.oid IN (SELECT oid FROM user_relations)),
        'columns', (SELECT jsonb_agg(to_jsonb(a) ORDER BY a.attrelid,a.attnum) FROM pg_attribute a
                    WHERE a.attrelid IN (SELECT oid FROM user_relations)),
        'defaults', (SELECT jsonb_agg(to_jsonb(d) ORDER BY d.oid) FROM pg_attrdef d
                     WHERE d.adrelid IN (SELECT oid FROM user_relations)),
        'constraints', (SELECT jsonb_agg(to_jsonb(c) ORDER BY c.oid) FROM pg_constraint c
                        WHERE c.connamespace IN (SELECT oid FROM user_schemas)),
        'indexes', (SELECT jsonb_agg(to_jsonb(i) ORDER BY i.indexrelid) FROM pg_index i
                    WHERE i.indrelid IN (SELECT oid FROM user_relations)),
        'policies', (SELECT jsonb_agg(to_jsonb(p) ORDER BY p.oid) FROM pg_policy p
                     WHERE p.polrelid IN (SELECT oid FROM user_relations)),
        'triggers', (SELECT jsonb_agg(to_jsonb(t) ORDER BY t.oid) FROM pg_trigger t
                     WHERE t.tgrelid IN (SELECT oid FROM user_relations)),
        'rules', (SELECT jsonb_agg(to_jsonb(r) ORDER BY r.oid) FROM pg_rewrite r
                  WHERE r.ev_class IN (SELECT oid FROM user_relations)),
        'types', (SELECT jsonb_agg(to_jsonb(t) ORDER BY t.oid) FROM pg_type t
                  WHERE t.typnamespace IN (SELECT oid FROM user_schemas)),
        'sequences', (SELECT jsonb_agg(to_jsonb(s) ORDER BY s.seqrelid) FROM pg_sequence s
                      WHERE s.seqrelid IN (SELECT oid FROM user_relations)),
        'functions', (SELECT jsonb_agg(to_jsonb(p) ORDER BY p.oid) FROM pg_proc p
                      WHERE p.pronamespace IN (SELECT oid FROM user_schemas)),
        'extensions', (SELECT jsonb_agg(to_jsonb(e) ORDER BY e.oid) FROM pg_extension e),
        'default_acl', (SELECT jsonb_agg(to_jsonb(a) ORDER BY a.oid) FROM pg_default_acl a),
        'databases', (SELECT jsonb_agg(to_jsonb(d) - ARRAY['datfrozenxid','datminmxid']
                      ORDER BY d.oid) FROM pg_database d),
        'roles', (SELECT jsonb_agg(to_jsonb(r) - 'rolpassword' ORDER BY r.oid) FROM pg_authid r),
        'memberships', (SELECT jsonb_agg(to_jsonb(m) ORDER BY m.roleid,m.member,m.grantor)
                        FROM pg_auth_members m),
        'role_settings', (SELECT jsonb_agg(to_jsonb(s) ORDER BY s.setdatabase,s.setrole)
                          FROM pg_db_role_setting s)
    ) AS value
)
SELECT jsonb_build_object(
    'schema', 'lagrange-preupgrade-db-v1',
    'database', current_database(),
    'server_version_num', current_setting('server_version_num')::integer,
    'system_identifier', (SELECT system_identifier::text FROM pg_control_system()),
    'migrations', (SELECT jsonb_agg(jsonb_build_object('version',version,
                      'checksum',encode(checksum,'hex'),'success',success) ORDER BY version)
                   FROM public._sqlx_migrations),
    'catalog_sha256', 'sha256:' || encode(sha256(convert_to(catalog.value::text,'UTF8')),'hex')
) FROM catalog;
ROLLBACK;
