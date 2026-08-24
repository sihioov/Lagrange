SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

-- PostgreSQL row locks require UPDATE privilege.  The owner-beta publisher
-- must hold the exact active strategy config stable through publication, but
-- the cross-tenant worker must never be able to mutate owner-managed configs
-- or choose an arbitrary tenant through a callable SECURITY DEFINER function.
-- Derive the lock identity only from the immutable run row as it transitions
-- to SUCCEEDED; the 0050 binding trigger and column grants prevent worker from
-- rewriting any of these lineage fields.
CREATE FUNCTION public.owner_beta_recommendation_runs_lock_strategy_config_on_success()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, pg_temp
AS $lock$
BEGIN
    IF NEW.status IS DISTINCT FROM 'SUCCEEDED'
       OR OLD.status IS NOT DISTINCT FROM 'SUCCEEDED'
    THEN
        RETURN NEW;
    END IF;

    PERFORM pg_catalog.set_config('app.actor_user_id', NEW.owner_user_id::text, true);
    PERFORM 1
      FROM public.user_strategy_configs AS config
     WHERE config.id = NEW.strategy_config_id
       AND config.owner_user_id = NEW.owner_user_id
       AND config.is_active
       AND config.strategy_id = NEW.strategy_id
       AND config.strategy_version = NEW.strategy_version
       AND config.config_json = NEW.strategy_config_json
     FOR SHARE OF config;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'owner beta recommendation strategy config is unavailable'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END
$lock$;

ALTER FUNCTION public.owner_beta_recommendation_runs_lock_strategy_config_on_success()
    OWNER TO migration_owner;
REVOKE ALL ON FUNCTION public.owner_beta_recommendation_runs_lock_strategy_config_on_success()
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;

CREATE TRIGGER owner_beta_recommendation_runs_lock_strategy_config_on_success
    BEFORE UPDATE OF status ON public.owner_beta_recommendation_runs
    FOR EACH ROW
    EXECUTE FUNCTION public.owner_beta_recommendation_runs_lock_strategy_config_on_success();
