-- Preserve the existing scheduler's authority, idempotency and grants.
-- Only a non-beta Paper account may provide its generic auto-apply opt-in.

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

DO $scheduler_binding$
DECLARE
    definition text := pg_catalog.pg_get_functiondef(
        'public.schedule_recommendation_run(uuid,uuid,date,uuid,text,integer,text)'::regprocedure
    );
    original constant text := $binding$
      AND binding.auto_apply_recommendations
      AND account.owner_user_id = p_owner_user_id
      AND account.account_type = 'PAPER'
      AND account.status = 'ACTIVE'
    LIMIT 1$binding$;
    replacement constant text := $binding$
      AND binding.auto_apply_recommendations
      AND account.owner_user_id = p_owner_user_id
      AND account.account_type = 'PAPER'
      AND account.status = 'ACTIVE'
      AND NOT public.paper_0067_is_beta_account(account.id)
    LIMIT 1$binding$;
BEGIN
    IF definition IS NULL OR
       length(definition) - length(replace(definition, original, '')) <> length(original)
    THEN
        RAISE EXCEPTION 'generic scheduler account binding definition changed'
            USING ERRCODE = '55000';
    END IF;
    EXECUTE replace(definition, original, replacement);
END
$scheduler_binding$;
