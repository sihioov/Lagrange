SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

-- Serialize the guard with the first writer and retain the lock through DROP.
LOCK TABLE public.research_execution_status IN ACCESS EXCLUSIVE MODE;

DO $down$
BEGIN
    IF EXISTS (
        SELECT 1 FROM public.research_execution_status
    ) THEN
        RAISE EXCEPTION 'refusing to drop nonempty research execution status';
    END IF;
END
$down$;

DROP TABLE public.research_execution_status;
