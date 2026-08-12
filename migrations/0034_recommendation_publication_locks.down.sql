SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

REVOKE EXECUTE ON FUNCTION public.lock_recommendation_publication_inputs(uuid, uuid, text, text, jsonb, uuid, text, text, text, text, text, text, jsonb) FROM worker;
DROP FUNCTION public.lock_recommendation_publication_inputs(uuid, uuid, text, text, jsonb, uuid, text, text, text, text, text, text, jsonb);
