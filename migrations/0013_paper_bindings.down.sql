DROP TABLE IF EXISTS account_strategy_bindings;
ALTER TABLE accounts DROP CONSTRAINT IF EXISTS accounts_cost_profile_id_check;
ALTER TABLE accounts DROP COLUMN IF EXISTS cost_profile_version;
ALTER TABLE accounts DROP COLUMN IF EXISTS cost_profile_id;
ALTER TABLE accounts DROP COLUMN IF EXISTS initial_cash;
