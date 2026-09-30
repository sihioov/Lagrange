-- 0055 down migration.  This is a disposable-schema rollback only; normal
-- production rollback remains forbidden by the repository release policy.

DROP TRIGGER owner_market_stream_grants_guard
    ON public.owner_market_stream_grants;
DROP FUNCTION public.owner_market_stream_grants_guard();

DROP POLICY owner_market_stream_session_lookup ON public.web_sessions;

DROP TABLE public.owner_market_stream_cache;
DROP TABLE public.owner_market_stream_subscriptions;
DROP TABLE public.owner_market_stream_producers;
DROP TABLE public.owner_market_stream_lease_items;
DROP TABLE public.owner_market_stream_leases;
DROP TABLE public.owner_market_stream_grants;

DROP FUNCTION public.lock_owner_market_stream_worker_demand(uuid, uuid, uuid, uuid, uuid[]);
DROP FUNCTION public.lock_owner_market_stream_app_lease_items(uuid, text, uuid);
DROP FUNCTION public.lock_owner_market_stream_worker_admission(
    uuid, uuid, uuid, uuid, uuid, uuid, text, bigint
);
DROP FUNCTION public.lock_owner_market_stream_app_admission(uuid, text, uuid, text, bigint);
DROP FUNCTION public.lock_owner_market_stream_session(uuid, uuid, uuid, uuid, text);
DROP FUNCTION public.lock_owner_market_stream_rights(uuid, uuid, uuid, uuid, date);
DROP FUNCTION public.owner_market_stream_session_valid(text, uuid);
DROP FUNCTION public.owner_market_stream_rights_valid(uuid, uuid, date);
