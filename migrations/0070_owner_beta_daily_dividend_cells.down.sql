-- Admission acquires a lock on Daily control before manifest validation.
-- Keep both that entry barrier and the immutable admission scan fenced
-- through the transactional downgrade. An in-flight caller blocks rollback.
LOCK TABLE public.owner_beta_daily_control, public.owner_beta_daily_admissions
    IN ACCESS EXCLUSIVE MODE NOWAIT;

DO $guard$
BEGIN
    IF EXISTS(
        SELECT 1 FROM public.owner_beta_daily_admissions stored
        CROSS JOIN LATERAL jsonb_array_elements(stored.evidence_json#>'{manifest,daily_sources}') AS source(source_day)
        CROSS JOIN LATERAL jsonb_array_elements(source_day->'normalized_documents') AS document(doc)
        WHERE doc->>'kind'='corporate_actions'
          AND (doc->'row_count' IS DISTINCT FROM to_jsonb((
              SELECT coalesce(sum(jsonb_array_length(chain->'cash_dividends')),0)
              FROM jsonb_array_elements(source_day->'actions') AS action(chain)
              WHERE chain->>'logical_class'='dividend'
          )) OR EXISTS (
              SELECT 1
              FROM jsonb_array_elements(source_day->'actions') AS action(chain)
              CROSS JOIN LATERAL jsonb_array_elements(chain->'cash_dividends') AS dividend(cash)
              WHERE (cash->>'stock_kind') COLLATE "C" !~ '^[A-Za-z0-9_-]{0,32}$'
          ))
    ) THEN RAISE EXCEPTION '0070 rollback blocked by retained Daily evidence' USING ERRCODE='55000'; END IF;
END
$guard$;

DO $migration$
DECLARE
    definition text := pg_get_functiondef('public.daily_0060_manifest(jsonb,jsonb)'::regprocedure);
    old_check constant text := $check$(doc->>'kind'='corporate_actions' AND doc->'row_count' IS DISTINCT FROM to_jsonb(public.daily_0070_selected_cash_count(s->'actions')))$check$;
    new_check constant text := $check$(doc->>'kind'='corporate_actions' AND doc->'row_count' IS DISTINCT FROM to_jsonb(session_cash))$check$;
    old_codes constant text := $codes$                FOREACH symbol IN ARRAY ARRAY['dividend_kind','high_dividend_flag'] LOOP
                    IF jsonb_typeof(cash->symbol) IS DISTINCT FROM 'string' OR (cash->>symbol) COLLATE "C" !~ '^[A-Za-z0-9_-]{0,32}$' THEN RAISE EXCEPTION 'daily cash code' USING ERRCODE='23514'; END IF;
                END LOOP;
                IF jsonb_typeof(cash->'stock_kind') IS DISTINCT FROM 'string'
                   OR NOT public.daily_0070_stock_kind(cash->>'stock_kind')
                THEN RAISE EXCEPTION 'daily cash code' USING ERRCODE='23514'; END IF;$codes$;
    new_codes constant text := $codes$                FOREACH symbol IN ARRAY ARRAY['dividend_kind','stock_kind','high_dividend_flag'] LOOP
                    IF jsonb_typeof(cash->symbol) IS DISTINCT FROM 'string' OR cash->>symbol !~ '^[A-Za-z0-9_-]{0,32}$' THEN RAISE EXCEPTION 'daily cash code' USING ERRCODE='23514'; END IF;
                END LOOP;$codes$;
BEGIN
    IF current_setting('server_encoding')<>'UTF8'
       OR (length(definition)-length(replace(definition,old_check,''))) <> length(old_check)
       OR strpos(definition,new_check) <> 0
       OR (length(definition)-length(replace(definition,old_codes,''))) <> length(old_codes)
       OR strpos(definition,new_codes) <> 0
    THEN RAISE EXCEPTION '0070 unexpected Daily manifest predecessor' USING ERRCODE='55000'; END IF;
    IF (SELECT encode(sha256(convert_to(prosrc,'UTF8')),'hex') FROM pg_proc
        WHERE oid='public.daily_0060_manifest(jsonb,jsonb)'::regprocedure)
       IS DISTINCT FROM '0a672187dc0fe9501a638294f9a3894c931832c719ee2e6a9948f5fddf153520'
    THEN RAISE EXCEPTION '0070 Daily manifest predecessor body mismatch' USING ERRCODE='55000'; END IF;
    EXECUTE replace(replace(definition,old_check,new_check),old_codes,new_codes);
    IF (SELECT encode(sha256(convert_to(prosrc,'UTF8')),'hex') FROM pg_proc
        WHERE oid='public.daily_0060_manifest(jsonb,jsonb)'::regprocedure)
       IS DISTINCT FROM '4326ce2ab15704927d0a7bcb0cc8f96d29a8f7649f46906b6e0dcc444fc8d9a5'
    THEN RAISE EXCEPTION '0070 Daily manifest replacement body mismatch' USING ERRCODE='55000'; END IF;
END
$migration$;

DROP FUNCTION public.daily_0070_selected_cash_count(jsonb);
DROP FUNCTION public.daily_0070_stock_kind(text);
