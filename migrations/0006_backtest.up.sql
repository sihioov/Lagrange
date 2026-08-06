-- 0006: Backtest — backtest_runs, backtest_metrics, backtest_warnings,
-- result_artifacts. Design §6.9-6.10, §7.2 (Backtest); plan Todo 20.
--
-- Large curves/orders/fills stay in Parquet: the DB holds ONLY manifests
-- (path, row count, sha256, summary). result_artifacts.owner_user_id is
-- nullable because ownership is derivable from the parent backtest_run; it
-- exists for direct tenant queries and RLS alignment (Todo 23).

CREATE TABLE backtest_runs (
    id               uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_user_id    uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    job_id           uuid REFERENCES jobs (id),
    strategy_id      text NOT NULL REFERENCES strategies (id),
    strategy_version text NOT NULL,
    dataset_version  text NOT NULL,
    engine           text NOT NULL DEFAULT 'nautilustrader',
    engine_version   text NOT NULL,
    config_sha256    text NOT NULL,
    code_commit      text NOT NULL,
    random_seed      int,
    timezone         text NOT NULL DEFAULT 'Asia/Seoul',
    status           text NOT NULL DEFAULT 'PENDING',
    summary_json     jsonb NOT NULL DEFAULT '{}'::jsonb,
    started_at       timestamptz,
    finished_at      timestamptz,
    created_at       timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT backtest_runs_config_sha256_check CHECK (config_sha256 ~ '^[0-9a-f]{64}$'),
    CONSTRAINT backtest_runs_status_check CHECK (status IN ('PENDING', 'RUNNING', 'SUCCEEDED', 'FAILED', 'CANCELED'))
);
CREATE INDEX backtest_runs_owner_idx ON backtest_runs (owner_user_id);

-- SC-05 metrics as key/value numerics: CAGR, total return, MDD, volatility,
-- Sharpe, turnover, cost, monthly-return stats, benchmark comparisons.
CREATE TABLE backtest_metrics (
    id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    backtest_run_id uuid NOT NULL REFERENCES backtest_runs (id) ON DELETE CASCADE,
    owner_user_id   uuid NOT NULL REFERENCES users (id),
    metric_key      text NOT NULL,
    metric_value    numeric NOT NULL,
    created_at      timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT backtest_metrics_run_metric_key UNIQUE (backtest_run_id, metric_key)
);
CREATE INDEX backtest_metrics_run_idx ON backtest_metrics (backtest_run_id);

CREATE TABLE backtest_warnings (
    id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    backtest_run_id uuid NOT NULL REFERENCES backtest_runs (id) ON DELETE CASCADE,
    owner_user_id   uuid NOT NULL REFERENCES users (id),
    warning_code    text NOT NULL,
    message         text NOT NULL DEFAULT '',
    created_at      timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX backtest_warnings_run_idx ON backtest_warnings (backtest_run_id);

-- Parquet manifest rows for EQUITY_CURVE/DRAWDOWN_CURVE/MONTHLY_RETURNS/
-- ORDERS/FILLS/POSITIONS/CASH_LEDGER/FEES/BENCHMARK.
CREATE TABLE result_artifacts (
    id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    backtest_run_id uuid NOT NULL REFERENCES backtest_runs (id) ON DELETE CASCADE,
    owner_user_id   uuid REFERENCES users (id),   -- tenant ownership (derived from the run)
    artifact_type   text NOT NULL,
    parquet_path    text NOT NULL,
    row_count       bigint NOT NULL,
    sha256          text NOT NULL,
    size_bytes      bigint NOT NULL,
    summary_json    jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at      timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT result_artifacts_sha256_check CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    CONSTRAINT result_artifacts_type_check CHECK (artifact_type IN ('EQUITY_CURVE', 'DRAWDOWN_CURVE', 'MONTHLY_RETURNS', 'ORDERS', 'FILLS', 'POSITIONS', 'CASH_LEDGER', 'FEES', 'BENCHMARK')),
    CONSTRAINT result_artifacts_row_count_check CHECK (row_count >= 0)
);
CREATE INDEX result_artifacts_run_idx ON result_artifacts (backtest_run_id);
