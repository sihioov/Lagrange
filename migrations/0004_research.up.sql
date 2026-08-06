-- 0004: Research — universe snapshots, factor definitions/manifests,
-- recommendation runs/items, target portfolios. Design §7.2 (Research),
-- §8; plan Todos 12, 15-16. Snapshots/definitions/manifests are system-owned
-- shared (SELECT only for serving roles); recommendation_runs/items and
-- target_portfolios are tenant tables (owner_user_id).

CREATE TABLE universe_snapshots (
    id                       uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    snapshot_id              text NOT NULL,      -- immutable universe_snapshot_id
    universe_manifest_sha256 text NOT NULL,
    instruments_json         jsonb NOT NULL,     -- canonical instrument ids
    published_by             uuid NOT NULL REFERENCES users (id),
    published_at             timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT universe_snapshots_manifest_sha256_check CHECK (universe_manifest_sha256 ~ '^[0-9a-f]{64}$'),
    CONSTRAINT universe_snapshots_snapshot_id_key UNIQUE (snapshot_id)
);

CREATE TABLE factor_definitions (
    id            text PRIMARY KEY,   -- e.g. 'momentum_12_1'
    name          text NOT NULL,
    description   text NOT NULL DEFAULT '',
    version       text NOT NULL,
    lookback_days int,
    null_policy   text NOT NULL DEFAULT 'EXCLUDE',
    created_at    timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT factor_definitions_null_policy_check CHECK (null_policy IN ('EXCLUDE', 'NULL'))
);

-- Deterministic factor snapshots: bytes live in Parquet; the DB keeps the
-- immutable content hash, path, and row count (plan Todo 15).
CREATE TABLE factor_snapshot_manifests (
    id                   uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    factor_definition_id text NOT NULL REFERENCES factor_definitions (id) ON DELETE CASCADE,
    snapshot_date        date NOT NULL,
    content_sha256       text NOT NULL,
    storage_path         text NOT NULL,
    row_count            bigint NOT NULL,
    created_at           timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT factor_snapshot_manifests_sha256_check CHECK (content_sha256 ~ '^[0-9a-f]{64}$'),
    CONSTRAINT factor_snapshot_manifests_factor_date_key UNIQUE (factor_definition_id, snapshot_date)
);

CREATE TABLE recommendation_runs (
    id                 uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_user_id      uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    strategy_config_id uuid REFERENCES user_strategy_configs (id),
    as_of              date NOT NULL,         -- data as-of; signals never leak post-as-of
    status             text NOT NULL DEFAULT 'SUCCEEDED',
    summary_json       jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at         timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT recommendation_runs_status_check CHECK (status IN ('PENDING', 'SUCCEEDED', 'FAILED', 'BLOCKED'))
);
CREATE INDEX recommendation_runs_owner_idx ON recommendation_runs (owner_user_id);

-- One row per ranked/selected/excluded instrument with structured evidence:
-- reasons, factor scores, rank, and target weight (plan SC-06).
CREATE TABLE recommendation_items (
    id                    uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    recommendation_run_id uuid NOT NULL REFERENCES recommendation_runs (id) ON DELETE CASCADE,
    owner_user_id         uuid NOT NULL REFERENCES users (id),
    instrument_id         text NOT NULL REFERENCES instruments (id),
    rank                  int,
    target_weight         numeric(18, 6),
    reason_codes          jsonb NOT NULL DEFAULT '[]'::jsonb,
    factors_json          jsonb NOT NULL DEFAULT '{}'::jsonb,
    excluded              boolean NOT NULL DEFAULT false,
    exclusion_reason      text,
    created_at            timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT recommendation_items_weight_check CHECK (target_weight IS NULL OR target_weight >= 0)
);
CREATE INDEX recommendation_items_run_idx ON recommendation_items (recommendation_run_id);

-- Selector output: constrained target weights + cash floor; targets only,
-- never orders (plan Todo 16).
CREATE TABLE target_portfolios (
    id                    uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_user_id         uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    recommendation_run_id uuid REFERENCES recommendation_runs (id),
    universe_snapshot_id  text REFERENCES universe_snapshots (snapshot_id),
    as_of                 date NOT NULL,
    cash_weight           numeric(18, 6) NOT NULL DEFAULT 0,
    weights_json          jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at            timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT target_portfolios_cash_weight_check CHECK (cash_weight >= 0)
);
CREATE INDEX target_portfolios_owner_idx ON target_portfolios (owner_user_id);
