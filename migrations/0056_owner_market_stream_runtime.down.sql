-- 0056 down is used only in disposable fresh-up rollback verification.
REVOKE ALL ON FUNCTION public.owner_market_stream_delivery_state(uuid, text, uuid)
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
DROP FUNCTION public.owner_market_stream_delivery_state(uuid, text, uuid);

REVOKE SELECT (credential_generation, network_contract_sha256, identity_list_sha256)
    ON public.owner_market_stream_grants FROM worker;
REVOKE UPDATE (status_code, status_at, gap_since, session_has_gap)
    ON public.owner_market_stream_producers FROM worker;

ALTER TABLE public.owner_market_stream_producers
    DROP CONSTRAINT owner_market_stream_producers_status_at_check,
    DROP CONSTRAINT owner_market_stream_producers_status_check,
    DROP COLUMN session_has_gap,
    DROP COLUMN gap_since,
    DROP COLUMN status_at,
    DROP COLUMN status_code;
