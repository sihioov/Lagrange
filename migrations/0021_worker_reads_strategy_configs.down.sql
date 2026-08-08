-- Revert 0021: `worker` can no longer read strategy configs.
--
-- Reverting this makes the backtest runner unable to resolve any job, which is
-- the correct behaviour for a down migration: it restores 0009's grant matrix
-- exactly rather than leaving a half-privileged role behind.

REVOKE SELECT ON TABLE user_strategy_configs FROM worker;
