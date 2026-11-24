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
          AND doc->'row_count' IS DISTINCT FROM to_jsonb((
              SELECT coalesce(sum(jsonb_array_length(chain->'cash_dividends')),0)
              FROM jsonb_array_elements(source_day->'actions') AS action(chain)
              WHERE chain->>'logical_class'='dividend'
          ))
    ) THEN RAISE EXCEPTION '0070 rollback blocked by retained Daily cell counts' USING ERRCODE='55000'; END IF;
END
$guard$;

DO $migration$
DECLARE
    definition text := pg_get_functiondef('public.daily_0060_manifest(jsonb,jsonb)'::regprocedure);
    old_check constant text := $check$(doc->>'kind'='corporate_actions' AND doc->'row_count' IS DISTINCT FROM to_jsonb(public.daily_0070_selected_cash_count(s->'actions')))$check$;
    new_check constant text := $check$(doc->>'kind'='corporate_actions' AND doc->'row_count' IS DISTINCT FROM to_jsonb(session_cash))$check$;
BEGIN
    IF (length(definition)-length(replace(definition,old_check,''))) <> length(old_check)
       OR strpos(definition,new_check) <> 0
    THEN RAISE EXCEPTION '0070 unexpected Daily manifest predecessor' USING ERRCODE='55000'; END IF;
    IF (SELECT encode(sha256(convert_to(prosrc,'UTF8')),'hex') FROM pg_proc
        WHERE oid='public.daily_0060_manifest(jsonb,jsonb)'::regprocedure)
       IS DISTINCT FROM 'cf145836d82eb39854f4c2bfcc0fe886359e936b191f79d7ebc34181f48e50ea'
    THEN RAISE EXCEPTION '0070 Daily manifest predecessor body mismatch' USING ERRCODE='55000'; END IF;
    EXECUTE replace(definition,old_check,new_check);
    IF (SELECT encode(sha256(convert_to(prosrc,'UTF8')),'hex') FROM pg_proc
        WHERE oid='public.daily_0060_manifest(jsonb,jsonb)'::regprocedure)
       IS DISTINCT FROM '4326ce2ab15704927d0a7bcb0cc8f96d29a8f7649f46906b6e0dcc444fc8d9a5'
    THEN RAISE EXCEPTION '0070 Daily manifest replacement body mismatch' USING ERRCODE='55000'; END IF;
END
$migration$;

DROP FUNCTION public.daily_0070_selected_cash_count(jsonb);
