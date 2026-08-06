-- 0002: Strategy registry metadata — strategies, versions, parameter
-- schemas, user strategy configs, promotions. Design §7.2 (Strategy) and
-- §6.7; plan Todo 17. strategies/versions/schemas/promotions are system-owned
-- shared metadata (Owner publishes, everyone reads); user_strategy_configs is
-- a tenant table (owner_user_id).

CREATE TABLE strategies (
    id               text PRIMARY KEY,       -- immutable strategy ID, e.g. 'dual_momentum'
    display_name     text NOT NULL,
    description      text NOT NULL DEFAULT '',
    risk_description text NOT NULL DEFAULT '',
    state            text NOT NULL DEFAULT 'Draft',
    created_at       timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT strategies_state_check CHECK (state IN ('Draft', 'Validated', 'Paper', 'LiveCandidate', 'Retired'))
);

CREATE TABLE strategy_versions (
    id               uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    strategy_id      text NOT NULL REFERENCES strategies (id) ON DELETE CASCADE,
    version          text NOT NULL,          -- SemVer; immutable once published
    required_factors jsonb NOT NULL DEFAULT '[]'::jsonb,
    min_lookback     int,
    supported_market text NOT NULL,
    cadence          text NOT NULL,
    created_at       timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT strategy_versions_strategy_version_key UNIQUE (strategy_id, version)
);

CREATE TABLE strategy_parameter_schemas (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    strategy_id text NOT NULL REFERENCES strategies (id) ON DELETE CASCADE,
    version     text NOT NULL,
    schema_json jsonb NOT NULL,              -- JSON Schema; Member params are schema-bound
    created_at  timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT strategy_parameter_schemas_strategy_version_key UNIQUE (strategy_id, version)
);

CREATE TABLE user_strategy_configs (
    id               uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_user_id    uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    strategy_id      text NOT NULL REFERENCES strategies (id),
    strategy_version text NOT NULL,
    config_json      jsonb NOT NULL,
    is_active        boolean NOT NULL DEFAULT true,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX user_strategy_configs_owner_idx ON user_strategy_configs (owner_user_id);

-- Promotion registry: every state transition is recorded with its evidence.
CREATE TABLE strategy_promotions (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    strategy_id  text NOT NULL REFERENCES strategies (id) ON DELETE CASCADE,
    from_state   text NOT NULL,
    to_state     text NOT NULL,
    promoted_by  uuid NOT NULL REFERENCES users (id),
    evidence_ref text NOT NULL,
    promoted_at  timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT strategy_promotions_transition_check CHECK (from_state <> to_state)
);
