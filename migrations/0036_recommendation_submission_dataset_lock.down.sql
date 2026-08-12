SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

REVOKE EXECUTE ON FUNCTION public.lock_recommendation_submission_dataset(uuid, text, text, text)
    FROM app;
DROP FUNCTION public.lock_recommendation_submission_dataset(uuid, text, text, text);
