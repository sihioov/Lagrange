SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

DROP INDEX public.owner_intraday_quote_cache_identity_session_idx;
DROP INDEX public.owner_intraday_quote_cache_owner_attempt_idx;
DROP INDEX public.owner_intraday_quote_demands_merge_identity_idx;
DROP INDEX public.owner_intraday_quote_demands_owner_state_expiry_idx;

DROP TABLE public.owner_intraday_quote_cache;
DROP TABLE public.owner_intraday_quote_demands;
DROP TABLE public.owner_intraday_quote_producers;
