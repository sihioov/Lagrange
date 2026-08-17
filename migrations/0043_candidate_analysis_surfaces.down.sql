-- Revert 0043 only before candidate analysis or user screen data exists.

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

DO $rollback_guard$
BEGIN
    IF EXISTS (SELECT 1 FROM public.stock_analysis_runs)
        OR EXISTS (SELECT 1 FROM public.stock_analysis_snapshots)
        OR EXISTS (SELECT 1 FROM public.candidate_feed_snapshots)
        OR EXISTS (SELECT 1 FROM public.candidate_feed_items)
        OR EXISTS (SELECT 1 FROM public.screener_saved_screens)
    THEN
        RAISE EXCEPTION '0043 rollback blocked by candidate analysis or saved screens'
            USING ERRCODE = '55000';
    END IF;
END
$rollback_guard$;

DROP TABLE public.screener_saved_screens;
DROP TABLE public.candidate_feed_items;
DROP TABLE public.candidate_feed_snapshots;
DROP TABLE public.stock_analysis_snapshots;
DROP TABLE public.stock_analysis_runs;
DROP TABLE public.candidate_scoring_configs;
DROP FUNCTION public.screener_saved_screen_touch_updated_at();
DROP FUNCTION public.candidate_feed_validate_item_count();
DROP FUNCTION public.candidate_analysis_reject_mutation();
DROP FUNCTION public.stock_analysis_validate_lineage();
