-- Invoke once per direct serving login, never through SET ROLE.
\set ON_ERROR_STOP on
DO $$
DECLARE statement text; label text;
BEGIN
    IF current_user<>session_user OR current_user NOT IN ('app','worker','research_writer','admin','daily_input_admitter') THEN
        RAISE EXCEPTION '0060 actual role identity'; END IF;
    FOREACH statement IN ARRAY ARRAY[
      'CREATE TABLE public.validation_0060_forbidden(id integer)',
      'SET ROLE migration_owner',
      'DELETE FROM public.owner_beta_daily_poll_receipts',
      'INSERT INTO public.owner_beta_daily_control(id) VALUES(''etf11'')',
      'SELECT public.daily_0060_take_slot(''ADMISSION'')',
      'SELECT public.revoke_owner_beta_daily_input(NULL,NULL)'] LOOP
        BEGIN EXECUTE statement; RAISE EXCEPTION '0060 unexpected privilege';
        EXCEPTION WHEN insufficient_privilege THEN NULL; END;
    END LOOP;
    IF current_user<>'daily_input_admitter' THEN
        BEGIN PERFORM public.admit_owner_beta_daily_input(NULL,'{}'); RAISE EXCEPTION '0060 unexpected admit privilege';
        EXCEPTION WHEN insufficient_privilege THEN NULL; END;
    ELSE
        IF EXISTS(SELECT 1 FROM public.take_owner_beta_daily_admission_poll_slot()) THEN RAISE EXCEPTION '0060 missing control slot'; END IF;
    END IF;
    IF current_user<>'worker' THEN
        BEGIN PERFORM public.reserve_owner_beta_daily_run(NULL,NULL,NULL,0,'{}'); RAISE EXCEPTION '0060 unexpected reserve privilege';
        EXCEPTION WHEN insufficient_privilege THEN NULL; END;
    ELSE
        IF EXISTS(SELECT 1 FROM public.owner_beta_daily_schedule_candidates()) THEN RAISE EXCEPTION '0060 missing control candidate'; END IF;
    END IF;
END $$;
SELECT 'DAILY_0060_HARNESS:actual_'||current_user||':PASS';
