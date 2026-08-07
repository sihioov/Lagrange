REVOKE UPDATE ON TABLE accounts FROM worker;
REVOKE INSERT, UPDATE ON TABLE orders, fills, cash_ledger, positions, daily_equity FROM worker;
DROP TABLE IF EXISTS pending_targets;
