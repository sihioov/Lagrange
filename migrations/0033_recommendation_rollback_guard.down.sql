-- Fail before 0032.down can remove the first recommendation-family index.

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

SELECT public.assert_no_scheduled_recommendation_lineage();
DROP FUNCTION public.assert_no_scheduled_recommendation_lineage();
