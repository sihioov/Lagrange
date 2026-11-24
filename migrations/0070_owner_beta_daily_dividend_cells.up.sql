-- The native verifier compares all covering original chains. Their selected
-- cash projection contributes once per symbol and record date. Keep the full
-- witness count and source bindings in the existing manifest validator.
CREATE FUNCTION public.daily_0070_selected_cash_count(actions jsonb) RETURNS bigint
LANGUAGE plpgsql STABLE SET search_path=pg_catalog,pg_temp
SET lock_timeout='5s' SET statement_timeout='30s'
AS $$
DECLARE selected_count bigint; disagreed boolean;
BEGIN
    -- Called only after the 0060 validator has checked row shape, each
    -- original page reference, chain scopes/dates and all action coverage.
    WITH chains AS MATERIALIZED (
        SELECT id, a->>'scope' AS scope, a->>'symbol' AS symbol,
               (a->>'range_start')::date AS first_date,
               (a->>'range_end')::date AS last_date, a->'cash_dividends' AS cash
        FROM jsonb_array_elements(actions) WITH ORDINALITY AS action(a,id)
        WHERE a->>'logical_class'='dividend'
    ), witnesses AS (
        SELECT id, cash_row->>'symbol' AS symbol, (cash_row->>'record_date')::date AS record_date,
               jsonb_agg(cash_row-'source_file' ORDER BY cash_row-'source_file') AS cash,
               count(*) AS row_count
        FROM chains CROSS JOIN LATERAL jsonb_array_elements(cash) AS rows(cash_row)
        GROUP BY id, cash_row->>'symbol', (cash_row->>'record_date')::date
    ), cells AS (
        SELECT symbol, record_date, min(row_count) AS row_count,
               count(*) AS positive_witnesses, count(DISTINCT cash) AS variants
        FROM witnesses GROUP BY symbol,record_date
    )
    SELECT coalesce(sum(cell.row_count),0),
           coalesce(bool_or(cell.variants<>1 OR cell.positive_witnesses<>(
               SELECT count(*) FROM chains witness
               WHERE (witness.scope='whole_market' OR witness.symbol=cell.symbol)
                 AND cell.record_date BETWEEN witness.first_date AND witness.last_date
           )),false)
    INTO selected_count,disagreed
    FROM cells cell;
    IF disagreed THEN
        RAISE EXCEPTION 'daily cash cell disagreement' USING ERRCODE='23514';
    END IF;
    RETURN selected_count;
END
$$;

ALTER FUNCTION public.daily_0070_selected_cash_count(jsonb) OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.daily_0070_selected_cash_count(jsonb)
    FROM PUBLIC,app,worker,admin,audit_writer,research_writer,daily_input_admitter;

DO $migration$
DECLARE
    definition text := pg_get_functiondef('public.daily_0060_manifest(jsonb,jsonb)'::regprocedure);
    old_check constant text := $check$(doc->>'kind'='corporate_actions' AND doc->'row_count' IS DISTINCT FROM to_jsonb(session_cash))$check$;
    new_check constant text := $check$(doc->>'kind'='corporate_actions' AND doc->'row_count' IS DISTINCT FROM to_jsonb(public.daily_0070_selected_cash_count(s->'actions')))$check$;
BEGIN
    IF (length(definition)-length(replace(definition,old_check,''))) <> length(old_check)
       OR strpos(definition,new_check) <> 0
    THEN RAISE EXCEPTION '0070 unexpected Daily manifest predecessor' USING ERRCODE='55000'; END IF;
    IF (SELECT encode(sha256(convert_to(prosrc,'UTF8')),'hex') FROM pg_proc
        WHERE oid='public.daily_0060_manifest(jsonb,jsonb)'::regprocedure)
       IS DISTINCT FROM '4326ce2ab15704927d0a7bcb0cc8f96d29a8f7649f46906b6e0dcc444fc8d9a5'
    THEN RAISE EXCEPTION '0070 Daily manifest predecessor body mismatch' USING ERRCODE='55000'; END IF;
    EXECUTE replace(definition,old_check,new_check);
    IF (SELECT encode(sha256(convert_to(prosrc,'UTF8')),'hex') FROM pg_proc
        WHERE oid='public.daily_0060_manifest(jsonb,jsonb)'::regprocedure)
       IS DISTINCT FROM 'cf145836d82eb39854f4c2bfcc0fe886359e936b191f79d7ebc34181f48e50ea'
    THEN RAISE EXCEPTION '0070 Daily manifest replacement body mismatch' USING ERRCODE='55000'; END IF;
END
$migration$;
