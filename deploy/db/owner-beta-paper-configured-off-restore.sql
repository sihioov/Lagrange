-- Installer for a disposable Paper restore guest after native archive BEGIN.
-- This file is outside the SQLx migration ledger and creates no new role,
-- grant, table or serving mutator. The isolated caller pins the exact bytes.
BEGIN READ WRITE;
SET LOCAL search_path=pg_catalog,pg_temp;
SET LOCAL statement_timeout='15s';
SET LOCAL lock_timeout='5s';

DO $guest$
DECLARE
    original text;
    updated text;
    definition text;
BEGIN
    PERFORM public.paper_0068_principal();
    IF current_user <> 'migration_owner' OR session_user <> 'migration_owner'
       OR current_setting('role') <> 'none'
       OR inet_server_addr() IS NOT NULL OR inet_client_addr() IS NOT NULL
       OR current_setting('listen_addresses') <> ''
       OR pg_is_in_recovery()
    THEN RAISE EXCEPTION 'configured-off restore helper requires a socket-only guest' USING ERRCODE='42501'; END IF;

    SELECT prosrc INTO STRICT original FROM pg_proc WHERE oid='public.paper_0068_exact_request(text,text)'::regprocedure
      AND proowner='migration_owner'::regrole AND NOT has_function_privilege('app',oid,'EXECUTE')
      AND NOT has_function_privilege('worker',oid,'EXECUTE')
      AND NOT has_function_privilege('admin',oid,'EXECUTE')
      AND NOT has_function_privilege('daily_input_admitter',oid,'EXECUTE');
    IF encode(sha256(convert_to(original,'UTF8')),'hex') IS DISTINCT FROM '1de0b11cefbd8cb1efc43bf17250b1f1cf1e5907e0c993bb3870bde6ca5eb2c8'
    THEN RAISE EXCEPTION 'configured-off helper unexpected predecessor 1' USING ERRCODE='55000'; END IF;
    updated := original;
    updated := replace(updated,$old11$      IF variant='RESTORED_OFF' THEN keys:=keys||ARRAY['source_installation_id','source_generation','source_epoch','archive_sha256','source_audit_sha256','restored_audit_sha256','destination_epoch_floor']; END IF;
      IF e->>'schema' IS DISTINCT FROM (CASE variant WHEN 'OFF' THEN 'owner-beta-paper-drain-receipt-v1' ELSE 'owner-beta-paper-restore-receipt-v1' END)$old11$,$new11$      IF variant='RESTORED_OFF' THEN
        keys:=keys||ARRAY['source_installation_id','source_generation','source_epoch','archive_sha256','source_audit_sha256','restored_audit_sha256','destination_epoch_floor'];
        IF e->>'schema'='owner-beta-paper-restore-receipt-v2' THEN keys:=keys||ARRAY['source_proof_sha256']; END IF;
      END IF;
      IF e->>'schema' IS DISTINCT FROM (CASE
           WHEN variant='OFF' THEN 'owner-beta-paper-drain-receipt-v1'
           WHEN e->>'schema'='owner-beta-paper-restore-receipt-v2' AND e->>'scope'='COMBINED_BACKUP' THEN 'owner-beta-paper-restore-receipt-v2'
           ELSE 'owner-beta-paper-restore-receipt-v1' END)$new11$);
    IF encode(sha256(convert_to(updated,'UTF8')),'hex') IS DISTINCT FROM 'ecd6a8c58d902fe35f6202d672ed76751f6694668c44b6a3ee746501b00cdb1b'
    THEN RAISE EXCEPTION 'configured-off helper replacement mismatch 1' USING ERRCODE='55000'; END IF;
    definition := pg_get_functiondef('public.paper_0068_exact_request(text,text)'::regprocedure);
    IF length(definition)-length(replace(definition,original,'')) <> length(original)
    THEN RAISE EXCEPTION 'configured-off helper definition mismatch 1' USING ERRCODE='55000'; END IF;
    EXECUTE replace(definition,original,updated);
    IF (SELECT encode(sha256(convert_to(prosrc,'UTF8')),'hex') FROM pg_proc
        WHERE oid='public.paper_0068_exact_request(text,text)'::regprocedure) IS DISTINCT FROM 'ecd6a8c58d902fe35f6202d672ed76751f6694668c44b6a3ee746501b00cdb1b'
    THEN RAISE EXCEPTION 'configured-off helper installed body mismatch 1' USING ERRCODE='55000'; END IF;

    SELECT prosrc INTO STRICT original FROM pg_proc WHERE oid='public.transition_owner_beta_paper_v1(bigint,text,jsonb,text)'::regprocedure
      AND proowner='migration_owner'::regrole AND NOT has_function_privilege('app',oid,'EXECUTE')
      AND NOT has_function_privilege('worker',oid,'EXECUTE')
      AND NOT has_function_privilege('admin',oid,'EXECUTE')
      AND NOT has_function_privilege('daily_input_admitter',oid,'EXECUTE');
    IF encode(sha256(convert_to(original,'UTF8')),'hex') IS DISTINCT FROM '6f94d0cc8eab950b6bb9663ebd8fe854ebec2917bf4379302bc7abb1d5220211'
    THEN RAISE EXCEPTION 'configured-off helper unexpected predecessor 2' USING ERRCODE='55000'; END IF;
    updated := original;
    updated := replace(updated,$old21$    IF transition='OFF' AND NOT public.daily_0060_keys(evidence,ARRAY['schema','installation_id','generation','account_id','request_key','scope','expected_epoch','resulting_epoch','intent_sha256','stage_sha256','inventory_sha256','reconciliation_sha256','observation_id','release_manifest_sha256','boot_id','observed'])
       OR transition='RESTORED_OFF' AND NOT public.daily_0060_keys(evidence,ARRAY['schema','installation_id','generation','account_id','request_key','scope','expected_epoch','resulting_epoch','intent_sha256','stage_sha256','inventory_sha256','reconciliation_sha256','observation_id','release_manifest_sha256','boot_id','observed','source_installation_id','source_generation','source_epoch','archive_sha256','source_audit_sha256','restored_audit_sha256','destination_epoch_floor'])$old21$,$new21$    IF transition='OFF' AND NOT public.daily_0060_keys(evidence,ARRAY['schema','installation_id','generation','account_id','request_key','scope','expected_epoch','resulting_epoch','intent_sha256','stage_sha256','inventory_sha256','reconciliation_sha256','observation_id','release_manifest_sha256','boot_id','observed'])
       OR transition='RESTORED_OFF' AND NOT public.daily_0060_keys(evidence,ARRAY['schema','installation_id','generation','account_id','request_key','scope','expected_epoch','resulting_epoch','intent_sha256','stage_sha256','inventory_sha256','reconciliation_sha256','observation_id','release_manifest_sha256','boot_id','observed','source_installation_id','source_generation','source_epoch','archive_sha256','source_audit_sha256','restored_audit_sha256','destination_epoch_floor']
         || CASE WHEN evidence->>'schema'='owner-beta-paper-restore-receipt-v2'
                 THEN ARRAY['source_proof_sha256'] ELSE ARRAY[]::text[] END)$new21$);
    updated := replace(updated,$old22$    IF evidence->>'schema' IS DISTINCT FROM (CASE transition WHEN 'OFF' THEN 'owner-beta-paper-drain-receipt-v1' ELSE 'owner-beta-paper-restore-receipt-v1' END)$old22$,$new22$    IF evidence->>'schema' IS DISTINCT FROM (CASE
         WHEN transition='OFF' THEN 'owner-beta-paper-drain-receipt-v1'
         WHEN evidence->>'schema'='owner-beta-paper-restore-receipt-v2' THEN 'owner-beta-paper-restore-receipt-v2'
         ELSE 'owner-beta-paper-restore-receipt-v1' END)$new22$);
    updated := replace(updated,$old23$      SELECT * INTO source_o FROM public.owner_beta_paper_operations WHERE generation=(evidence->>'source_generation')::uuid AND account_id=c.account_id FOR SHARE;
      IF NOT FOUND OR source_o.installation_id IS DISTINCT FROM (evidence->>'source_installation_id')::uuid
         OR source_o.installation_id=o.installation_id OR source_o.generation IS DISTINCT FROM NULLIF(o.intent->>'predecessor_generation','null')::uuid
         OR o.intent->>'artifact_sha256' IS DISTINCT FROM evidence->>'archive_sha256'
         OR (evidence->>'destination_epoch_floor')::bigint<>o.local_epoch_floor
         OR (evidence->>'source_epoch')::bigint>(evidence->>'destination_epoch_floor')::bigint
         OR NOT EXISTS(SELECT 1 FROM public.owner_beta_paper_events se WHERE se.account_id=c.account_id AND se.request_key=source_o.generation::text||':off' AND se.kind='OFF' AND (se.payload->>'resulting_epoch')::bigint=(evidence->>'source_epoch')::bigint)
         OR evidence->>'scope'<>'COMBINED_BACKUP' OR evidence->>'archive_sha256' !~ '^sha256:[0-9a-f]{64}$'
       OR evidence->>'source_audit_sha256' !~ '^sha256:[0-9a-f]{64}$' OR evidence->>'restored_audit_sha256' IS DISTINCT FROM evidence->>'source_audit_sha256'
       OR evidence->>'source_installation_id' !~ '^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
       OR evidence->>'source_generation' !~ '^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
         OR (evidence->>'destination_epoch_floor')::bigint>=c.control_epoch
      THEN RAISE EXCEPTION 'restore source/destination audit binding invalid' USING ERRCODE='23514'; END IF;$old23$,$new23$      IF evidence->>'schema'='owner-beta-paper-restore-receipt-v2' THEN
        IF current_user <> 'migration_owner' OR session_user <> 'migration_owner'
           OR current_setting('role') <> 'none'
           OR inet_server_addr() IS NOT NULL OR inet_client_addr() IS NOT NULL
           OR current_setting('listen_addresses') <> ''
           OR pg_is_in_recovery()
        THEN RAISE EXCEPTION 'configured-off restore requires the isolated guest' USING ERRCODE='42501'; END IF;
        -- Native BEGIN verified the original canonical audit rows before
        -- writing the fence. Its exact archive hash remains durable in o.intent.
        IF o.prior_state <> 'OFF' OR o.expected_epoch<>(evidence->>'source_epoch')::bigint
           OR c.process_binding IS NOT NULL OR c.process_binding_sha256 IS NOT NULL
           OR c.daily_cohort IS NOT NULL OR c.cohort_sha256 IS NOT NULL OR c.activation_event_id IS NOT NULL
           OR NOT public.paper_0068_uuid(evidence->'source_installation_id')
           OR (evidence->>'source_installation_id')::uuid=o.installation_id
           OR NOT public.paper_0068_uuid(evidence->'source_generation')
           OR (evidence->>'source_generation')::uuid=o.generation
           OR evidence->>'scope'<>'COMBINED_BACKUP'
           OR NOT public.daily_0060_hash(evidence->'archive_sha256')
           OR o.intent->>'artifact_sha256' IS DISTINCT FROM evidence->>'archive_sha256'
           OR NOT public.daily_0060_hash(evidence->'source_proof_sha256')
           OR NOT public.daily_0060_hash(evidence->'source_audit_sha256')
           OR evidence->'restored_audit_sha256' IS DISTINCT FROM evidence->'source_audit_sha256'
           OR (evidence->>'destination_epoch_floor')::bigint<>o.local_epoch_floor
           OR o.expected_epoch>o.local_epoch_floor OR o.local_epoch_floor>=c.control_epoch
        THEN RAISE EXCEPTION 'configured-off restore source proof invalid' USING ERRCODE='23514'; END IF;
      ELSE
        SELECT * INTO source_o FROM public.owner_beta_paper_operations WHERE generation=(evidence->>'source_generation')::uuid AND account_id=c.account_id FOR SHARE;
        IF NOT FOUND OR source_o.installation_id IS DISTINCT FROM (evidence->>'source_installation_id')::uuid
           OR source_o.installation_id=o.installation_id OR source_o.generation IS DISTINCT FROM NULLIF(o.intent->>'predecessor_generation','null')::uuid
           OR o.intent->>'artifact_sha256' IS DISTINCT FROM evidence->>'archive_sha256'
           OR (evidence->>'destination_epoch_floor')::bigint<>o.local_epoch_floor
           OR (evidence->>'source_epoch')::bigint>(evidence->>'destination_epoch_floor')::bigint
           OR NOT EXISTS(SELECT 1 FROM public.owner_beta_paper_events se WHERE se.account_id=c.account_id AND se.request_key=source_o.generation::text||':off' AND se.kind='OFF' AND (se.payload->>'resulting_epoch')::bigint=(evidence->>'source_epoch')::bigint)
           OR evidence->>'scope'<>'COMBINED_BACKUP' OR evidence->>'archive_sha256' !~ '^sha256:[0-9a-f]{64}$'
         OR evidence->>'source_audit_sha256' !~ '^sha256:[0-9a-f]{64}$' OR evidence->>'restored_audit_sha256' IS DISTINCT FROM evidence->>'source_audit_sha256'
         OR evidence->>'source_installation_id' !~ '^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
         OR evidence->>'source_generation' !~ '^(?!00000000-0000-0000-0000-000000000000$)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
           OR (evidence->>'destination_epoch_floor')::bigint>=c.control_epoch
        THEN RAISE EXCEPTION 'restore source/destination audit binding invalid' USING ERRCODE='23514'; END IF;
      END IF;$new23$);
    IF encode(sha256(convert_to(updated,'UTF8')),'hex') IS DISTINCT FROM 'ea70cbb5067b24d888db1fcd81cbdc57909bbef42202c8cf16c3ff574205d241'
    THEN RAISE EXCEPTION 'configured-off helper replacement mismatch 2' USING ERRCODE='55000'; END IF;
    definition := pg_get_functiondef('public.transition_owner_beta_paper_v1(bigint,text,jsonb,text)'::regprocedure);
    IF length(definition)-length(replace(definition,original,'')) <> length(original)
    THEN RAISE EXCEPTION 'configured-off helper definition mismatch 2' USING ERRCODE='55000'; END IF;
    EXECUTE replace(definition,original,updated);
    IF (SELECT encode(sha256(convert_to(prosrc,'UTF8')),'hex') FROM pg_proc
        WHERE oid='public.transition_owner_beta_paper_v1(bigint,text,jsonb,text)'::regprocedure) IS DISTINCT FROM 'ea70cbb5067b24d888db1fcd81cbdc57909bbef42202c8cf16c3ff574205d241'
    THEN RAISE EXCEPTION 'configured-off helper installed body mismatch 2' USING ERRCODE='55000'; END IF;
END
$guest$;
COMMIT;
SELECT 'PAPER_CONFIGURED_OFF_RESTORE_GUEST: INSTALLED';
