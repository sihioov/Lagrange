-- Revert 0042 only when no candidate source observation has been published.

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

DROP POLICY candidate_dataset_versions_select_research_writer
    ON public.dataset_versions;
REVOKE SELECT ON TABLE public.dataset_versions FROM research_writer;

DO $rollback_guard$
BEGIN
    IF EXISTS (SELECT 1 FROM public.candidate_universe_snapshots)
        OR EXISTS (SELECT 1 FROM public.candidate_investor_flows)
        OR EXISTS (SELECT 1 FROM public.candidate_investor_flow_snapshot_rows)
        OR EXISTS (SELECT 1 FROM public.candidate_market_status_observations)
        OR EXISTS (SELECT 1 FROM public.candidate_fundamental_observations)
        OR EXISTS (SELECT 1 FROM public.candidate_sector_versions)
        OR EXISTS (SELECT 1 FROM public.candidate_price_publications)
        OR EXISTS (SELECT 1 FROM public.candidate_price_instrument_coverage)
        OR EXISTS (SELECT 1 FROM public.candidate_price_instrument_sessions)
        OR EXISTS (SELECT 1 FROM public.candidate_instrument_registrations)
        OR EXISTS (SELECT 1 FROM public.candidate_raw_batch_publications)
        OR EXISTS (SELECT 1 FROM public.candidate_raw_batch_datasets)
    THEN
        RAISE EXCEPTION '0042 rollback blocked by published candidate source observations'
            USING ERRCODE = '55000';
    END IF;
END
$rollback_guard$;

DROP FUNCTION public.publish_candidate_price_publication(
    text, text, text, bigint, date, date, jsonb, text, uuid, text, text, uuid, text, text,
    date, timestamptz, timestamptz
);
DROP FUNCTION public.insert_candidate_sector_entry(uuid,text,text,text,text,date,date,timestamptz,text);
DROP FUNCTION public.insert_candidate_sector_version(text,text,date,timestamptz,timestamptz,text,uuid,date,text,text,uuid,text);
DROP FUNCTION public.insert_candidate_universe_member(uuid,text,timestamptz,date,date,timestamptz,text);
DROP FUNCTION public.insert_candidate_universe_snapshot(date,uuid,text,text,uuid,date,text,text,timestamptz,timestamptz,integer);
DROP FUNCTION public.insert_candidate_fundamental(text,date,date,text,text,text,numeric,text,integer,boolean,timestamptz,timestamptz,timestamptz,text,uuid,date,text,text,uuid,uuid,text);
DROP FUNCTION public.insert_candidate_market_status(text,date,boolean,boolean,boolean,boolean,boolean,boolean,text,uuid,date,text,text,timestamptz,timestamptz,uuid,text);
DROP FUNCTION public.insert_candidate_investor_flow(text,date,text,numeric,numeric,text,text,text,uuid,date,text,text,timestamptz,timestamptz,uuid,text);
DROP FUNCTION public.candidate_source_dataset_write_matches(uuid,text,text,uuid,date,text,text);
DROP FUNCTION public.candidate_source_dataset_write_is_open(uuid);
DROP FUNCTION public.block_candidate_raw_batch_for_inactive_rights(uuid,text,text,text,text,date,date,date);
DROP FUNCTION public.seal_candidate_raw_batch(uuid,text,text,text);
DROP FUNCTION public.bind_candidate_raw_dataset(uuid,text,text,uuid,boolean);
DROP FUNCTION public.begin_candidate_raw_batch(uuid,text,text,text,text,date);
DROP FUNCTION public.register_candidate_instrument(
    text, text, text, text, date, uuid, text, date, text, text, timestamptz
);
DROP FUNCTION public.register_candidate_source_dataset(text, text, text, uuid, text, date);
DROP FUNCTION public.resolve_candidate_contract_entitlement(text, date, date);
DROP TABLE public.candidate_instrument_registrations;
DROP TABLE public.candidate_price_instrument_sessions;
DROP TABLE public.candidate_price_instrument_coverage;
DROP TABLE public.candidate_price_publications;
DROP TABLE public.candidate_sector_entries;
DROP TABLE public.candidate_sector_versions;
DROP TABLE public.candidate_fundamental_observations;
DROP TABLE public.candidate_market_status_observations;
DROP TABLE public.candidate_investor_flow_snapshot_rows;
DROP TABLE public.candidate_investor_flows;
DROP TABLE public.candidate_universe_members;
DROP TABLE public.candidate_universe_snapshots;
DROP TABLE public.candidate_raw_batch_datasets;
DROP TABLE public.candidate_raw_batch_publications;
DROP FUNCTION public.candidate_universe_validate_members();
DROP FUNCTION public.candidate_source_validate_dataset_pin();
DROP FUNCTION public.candidate_source_entitlement_is_valid(uuid, text, text, date, date);
DROP FUNCTION public.candidate_source_reject_mutation();
