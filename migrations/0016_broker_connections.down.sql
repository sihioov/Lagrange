-- 0016 down: remove the Phase-3 additions, leaving 0007's broker_connections.
DROP TABLE IF EXISTS live_kill_switch;
DROP TABLE IF EXISTS broker_nodes;
ALTER TABLE broker_connections
    DROP CONSTRAINT IF EXISTS broker_connections_account_is_masked,
    DROP CONSTRAINT IF EXISTS broker_connections_secret_is_a_reference,
    DROP CONSTRAINT IF EXISTS broker_connections_app_key_is_a_reference,
    DROP CONSTRAINT IF EXISTS broker_connections_profile_check;
ALTER TABLE broker_connections
    DROP COLUMN IF EXISTS account_product_code,
    DROP COLUMN IF EXISTS account_no_masked,
    DROP COLUMN IF EXISTS app_key_ref,
    DROP COLUMN IF EXISTS profile,
    DROP COLUMN IF EXISTS label;
