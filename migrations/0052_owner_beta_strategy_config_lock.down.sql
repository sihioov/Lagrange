SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

DROP TRIGGER owner_beta_recommendation_runs_lock_strategy_config_on_success
    ON public.owner_beta_recommendation_runs;
DROP FUNCTION public.owner_beta_recommendation_runs_lock_strategy_config_on_success();
