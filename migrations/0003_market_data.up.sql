-- 0003: Market-data metadata — instruments, aliases, trading calendars,
-- data_batches, dataset_versions, data_quality_issues, corporate_actions,
-- data_entitlements. Design §7.2 (Market Data Metadata), §5.1; plan Todos
-- 5, 8-11. All are system-owned shared tables: serving roles get SELECT only
-- (grants land in 0009); large bars/actions live in Parquet, the DB keeps
-- immutable manifests (sha256) and metadata.

CREATE TABLE instruments (
    id          text PRIMARY KEY,      -- canonical '{symbol}.KRX'
    symbol      text NOT NULL,
    venue       text NOT NULL,
    currency    text NOT NULL,         -- 'KRW' first
    name        text,
    asset_class text NOT NULL DEFAULT 'ETF',
    status      text NOT NULL DEFAULT 'ACTIVE',
    listed_at   date,
    delisted_at date,
    created_at  timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT instruments_status_check CHECK (status IN ('ACTIVE', 'LISTING', 'DELISTED')),
    CONSTRAINT instruments_symbol_venue_key UNIQUE (symbol, venue)
);

-- Provider aliases with effective intervals: a ticker change updates alias
-- history, never the canonical identity (plan Todo 9).
CREATE TABLE instrument_aliases (
    id             uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    instrument_id  text NOT NULL REFERENCES instruments (id) ON DELETE CASCADE,
    alias_provider text NOT NULL,      -- 'KRX' | 'KIS' | ...
    alias_symbol   text NOT NULL,
    effective_from date NOT NULL,
    effective_until date,
    created_at     timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT instrument_aliases_provider_symbol_from_key UNIQUE (alias_provider, alias_symbol, effective_from)
);
CREATE INDEX instrument_aliases_instrument_idx ON instrument_aliases (instrument_id);

CREATE TABLE trading_calendars (
    id             uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    exchange       text NOT NULL,
    session_date   date NOT NULL,
    session_type   text NOT NULL,      -- 'TRADING' | 'SETTLEMENT' | ...
    timezone       text NOT NULL DEFAULT 'Asia/Seoul',
    source         text NOT NULL,
    source_version text NOT NULL,
    created_at     timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT trading_calendars_exchange_date_key UNIQUE (exchange, session_date)
);

-- Immutable Raw ingestion manifests (plan Todo 8): one row per stored file,
-- content-hash bound; identical bytes re-ingested create a second batch with
-- the same hash and never modify the first.
CREATE TABLE data_batches (
    id             uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    provider       text NOT NULL,      -- 'KRX'
    market         text NOT NULL,      -- 'KR'
    batch_date     date NOT NULL,
    kind           text NOT NULL,      -- 'EOD' | 'REFERENCE' | 'CALENDAR' | ...
    storage_path   text NOT NULL,      -- data/raw/provider=krx/market=kr/date=...
    content_sha256 text NOT NULL,
    bytes_size     bigint NOT NULL,
    retrieved_at   timestamptz NOT NULL,
    created_at     timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT data_batches_content_sha256_check CHECK (content_sha256 ~ '^[0-9a-f]{64}$'),
    CONSTRAINT data_batches_bytes_positive_check CHECK (bytes_size >= 0)
);
CREATE INDEX data_batches_provider_market_date_idx ON data_batches (provider, market, batch_date);

-- Versioned curated dataset manifests (plan Todo 11): READY|WARNING|BLOCKED
-- quality state; a correction rule always creates a NEW version, never a
-- backfill into an existing one.
CREATE TABLE dataset_versions (
    id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    dataset_id      text NOT NULL,
    version         text NOT NULL,
    status          text NOT NULL DEFAULT 'READY',
    manifest_sha256 text NOT NULL,
    storage_path    text NOT NULL,
    created_at      timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT dataset_versions_status_check CHECK (status IN ('READY', 'WARNING', 'BLOCKED')),
    CONSTRAINT dataset_versions_manifest_sha256_check CHECK (manifest_sha256 ~ '^[0-9a-f]{64}$'),
    CONSTRAINT dataset_versions_dataset_version_key UNIQUE (dataset_id, version)
);

CREATE TABLE data_quality_issues (
    id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    dataset_id      text NOT NULL,
    dataset_version text NOT NULL,
    issue_code      text NOT NULL,
    severity        text NOT NULL DEFAULT 'ERROR',
    detail_json     jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at      timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT data_quality_issues_severity_check CHECK (severity IN ('ERROR', 'WARNING', 'INFO'))
);

CREATE TABLE corporate_actions (
    id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    instrument_id text NOT NULL REFERENCES instruments (id) ON DELETE CASCADE,
    action_type   text NOT NULL,
    announced_at  timestamptz NOT NULL,   -- never visible before this instant
    ex_date       date,
    pay_date      date,
    ratio_json    jsonb,                  -- split ratio when SPLIT
    amount        numeric(18, 4),         -- per-share amount when DIVIDEND
    created_at    timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT corporate_actions_type_check CHECK (action_type IN ('SPLIT', 'DIVIDEND')),
    CONSTRAINT corporate_actions_ex_pay_check CHECK (ex_date IS NULL OR pay_date IS NULL OR pay_date >= ex_date)
);
CREATE INDEX corporate_actions_instrument_idx ON corporate_actions (instrument_id);

-- KRX data-rights gate (plan Todo 5): lifecycle CHECK-enforced
-- PENDING|ACTIVE|EXPIRED|REVOKED; the contract document is stored by hash +
-- reference only, never by contents.
CREATE TABLE data_entitlements (
    id                       uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    contract_document_sha256 text NOT NULL,
    contract_reference       text NOT NULL,
    status                   text NOT NULL,
    covered_datasets         jsonb NOT NULL,
    covered_uses             jsonb NOT NULL,
    effective_from           date NOT NULL,
    effective_until          date NOT NULL,
    managed_by               uuid NOT NULL REFERENCES users (id),
    created_at               timestamptz NOT NULL DEFAULT now(),
    updated_at               timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT data_entitlements_document_sha256_check CHECK (contract_document_sha256 ~ '^[0-9a-f]{64}$'),
    CONSTRAINT data_entitlements_status_check CHECK (status IN ('PENDING', 'ACTIVE', 'EXPIRED', 'REVOKED')),
    CONSTRAINT data_entitlements_effective_window_check CHECK (effective_until >= effective_from)
);
CREATE INDEX data_entitlements_status_idx ON data_entitlements (status);
