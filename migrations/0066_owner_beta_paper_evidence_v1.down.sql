-- Preserve source reservations and their unknown/crash history on rollback.
DO $$ BEGIN
    IF EXISTS(SELECT 1 FROM public.owner_beta_paper_source_slots) THEN
        RAISE EXCEPTION 'paper source history requires compatible OFF reader' USING ERRCODE='23514';
    END IF;
END $$;
DROP FUNCTION public.reserve_owner_beta_paper_source_slot(text,uuid,text,jsonb,date),public.link_owner_beta_paper_source_admission(text,text,text,text),public.link_owner_beta_paper_daily_run(text,uuid),public.read_owner_beta_paper_session_evidence(date,jsonb);
DROP FUNCTION public.paper_0066_read(public.owner_beta_paper_source_slots),public.paper_0066_originals(public.owner_beta_daily_admissions),public.paper_0066_authority(jsonb,date),public.paper_0066_principal(text);
DROP TABLE public.owner_beta_paper_source_runs,public.owner_beta_paper_source_originals,public.owner_beta_paper_source_admissions,public.owner_beta_paper_source_slots;
