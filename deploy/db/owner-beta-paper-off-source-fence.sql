ROLLBACK;
BEGIN READ WRITE;
SET LOCAL search_path = pg_catalog, pg_temp;
SET LOCAL lock_timeout = '2s';
SET LOCAL statement_timeout = '5s';
DO $archive_fence$
DECLARE p public.owner_beta_paper_control;
BEGIN
    IF current_user <> 'migration_owner' OR session_user <> current_user
       OR current_setting('role') <> 'none' THEN
        RAISE EXCEPTION 'PAPER_ARCHIVE_FENCE_PRINCIPAL' USING ERRCODE='42501';
    END IF;
    SELECT * INTO STRICT p FROM public.owner_beta_paper_control
     WHERE id='etf11' AND state='OFF' AND process_binding IS NULL
       AND process_binding_sha256 IS NULL AND cohort_sha256 IS NULL AND daily_cohort IS NULL
       AND activation_event_id IS NULL FOR NO KEY UPDATE NOWAIT;
    IF NOT pg_try_advisory_xact_lock(hashtextextended(p.account_id::text,381901)) THEN
        RAISE EXCEPTION 'PAPER_ARCHIVE_FENCE_BUSY' USING ERRCODE='55P03';
    END IF;
    PERFORM 1 FROM public.owner_beta_daily_control WHERE id='etf11'
       AND owner_user_id=p.owner_user_id FOR NO KEY UPDATE NOWAIT;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'PAPER_ARCHIVE_FENCE_DAILY' USING ERRCODE='23514';
    END IF;
END
$archive_fence$;
SELECT 'PAPER_ARCHIVE: CONTROL_LOCKED';
