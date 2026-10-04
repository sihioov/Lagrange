REVOKE ALL ON FUNCTION public.owner_market_stream_api_binding_valid(uuid, text, uuid, uuid, text)
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
DROP FUNCTION public.owner_market_stream_api_binding_valid(uuid, text, uuid, uuid, text);
