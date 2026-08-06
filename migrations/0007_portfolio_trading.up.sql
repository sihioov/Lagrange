-- 0007: Portfolio / Trading — accounts, cash_ledger, positions, orders,
-- fills, daily_equity, broker_connections, reconciliation_runs, risk_events.
-- Design §7.2 (Portfolio / Trading), §7.3; plan Todos 18, 31-32, 36-37.
-- All tenant tables (owner_user_id). Orders/fills/cash/positions/equity rows
-- are the DB-side manifests; large arrays stay in Parquet artifacts (0006).
-- broker_connections stores a Secret-Store REFERENCE only, never a value
-- (NFR-SEC-002).

CREATE TABLE accounts (
    id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    account_type  text NOT NULL,
    name          text NOT NULL,
    currency      text NOT NULL DEFAULT 'KRW',
    status        text NOT NULL DEFAULT 'ACTIVE',
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT accounts_type_check CHECK (account_type IN ('PAPER', 'LIVE')),
    CONSTRAINT accounts_status_check CHECK (status IN ('ACTIVE', 'SUSPENDED', 'CLOSED')),
    CONSTRAINT accounts_owner_name_key UNIQUE (owner_user_id, name)
);

-- Deterministic cash ledger with strictly increasing per-account seq
-- (plan Todo 18 replay contract).
CREATE TABLE cash_ledger (
    id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id    uuid NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    owner_user_id uuid NOT NULL REFERENCES users (id),
    seq           bigint NOT NULL,
    event_type    text NOT NULL,          -- DEPOSIT | WITHDRAWAL | SELL | BUY | FEE | TAX | ...
    amount        numeric(18, 4) NOT NULL,
    balance       numeric(18, 4) NOT NULL,
    currency      text NOT NULL DEFAULT 'KRW',
    reference_id  uuid,                   -- order/fill id for replay correlation
    ts            timestamptz NOT NULL DEFAULT now(),
    created_at    timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT cash_ledger_account_seq_key UNIQUE (account_id, seq)
);
CREATE INDEX cash_ledger_account_idx ON cash_ledger (account_id);

CREATE TABLE positions (
    id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id    uuid NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    owner_user_id uuid NOT NULL REFERENCES users (id),
    instrument_id text NOT NULL REFERENCES instruments (id),
    quantity      numeric(18, 4) NOT NULL,
    avg_price     numeric(18, 4),
    updated_at    timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT positions_account_instrument_key UNIQUE (account_id, instrument_id)
);

CREATE TABLE orders (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id   uuid NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    owner_user_id uuid NOT NULL REFERENCES users (id),
    order_ref    text NOT NULL,           -- idempotent per-account order intent
    instrument_id text NOT NULL REFERENCES instruments (id),
    side         text NOT NULL,
    quantity     numeric(18, 4) NOT NULL,
    price        numeric(18, 4),
    status       text NOT NULL DEFAULT 'PENDING',
    submitted_at timestamptz,
    created_at   timestamptz NOT NULL DEFAULT now(),
    updated_at   timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT orders_side_check CHECK (side IN ('BUY', 'SELL')),
    CONSTRAINT orders_status_check CHECK (status IN ('PENDING', 'SUBMITTED', 'PARTIALLY_FILLED', 'FILLED', 'CANCELED', 'REJECTED')),
    CONSTRAINT orders_account_order_ref_key UNIQUE (account_id, order_ref)
);
CREATE INDEX orders_account_idx ON orders (account_id);

CREATE TABLE fills (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id   uuid NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    owner_user_id uuid NOT NULL REFERENCES users (id),
    order_id     uuid REFERENCES orders (id),
    instrument_id text NOT NULL REFERENCES instruments (id),
    fill_ref     text NOT NULL,
    side         text NOT NULL,
    quantity     numeric(18, 4) NOT NULL,
    price        numeric(18, 4) NOT NULL, -- execution price (slippage embedded)
    fees         numeric(18, 4) NOT NULL DEFAULT 0,
    ts           timestamptz NOT NULL,
    created_at   timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT fills_side_check CHECK (side IN ('BUY', 'SELL')),
    CONSTRAINT fills_account_fill_ref_key UNIQUE (account_id, fill_ref)
);
CREATE INDEX fills_account_idx ON fills (account_id);

CREATE TABLE daily_equity (
    id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id      uuid NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    owner_user_id   uuid NOT NULL REFERENCES users (id),
    trading_date    date NOT NULL,
    equity          numeric(18, 4) NOT NULL,
    cash            numeric(18, 4) NOT NULL,
    positions_value numeric(18, 4) NOT NULL,
    currency        text NOT NULL DEFAULT 'KRW',
    created_at      timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT daily_equity_account_date_key UNIQUE (account_id, trading_date)
);

-- Owner-only KIS Live (Phase 3): secret_ref points into the Secret Store.
CREATE TABLE broker_connections (
    id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    broker        text NOT NULL,
    account_ref   text NOT NULL,
    secret_ref    text NOT NULL,          -- reference only; NEVER a secret value
    status        text NOT NULL DEFAULT 'DISCONNECTED',
    enabled       boolean NOT NULL DEFAULT false,
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT broker_connections_broker_check CHECK (broker IN ('KIS')),
    CONSTRAINT broker_connections_status_check CHECK (status IN ('CONNECTED', 'DISCONNECTED', 'ERROR')),
    CONSTRAINT broker_connections_owner_broker_key UNIQUE (owner_user_id, broker, account_ref)
);

-- Pre-Live reconciliation is fail-closed (NFR-REL-004).
CREATE TABLE reconciliation_runs (
    id                   uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_user_id        uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    broker_connection_id uuid REFERENCES broker_connections (id),
    run_type             text NOT NULL,
    status               text NOT NULL DEFAULT 'PENDING',
    mismatch_count       int NOT NULL DEFAULT 0,
    report_path          text,
    started_at           timestamptz,
    finished_at          timestamptz,
    created_at           timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT reconciliation_runs_type_check CHECK (run_type IN ('STARTUP', 'SCHEDULED', 'MANUAL')),
    CONSTRAINT reconciliation_runs_status_check CHECK (status IN ('PENDING', 'RUNNING', 'PASSED', 'FAILED'))
);

CREATE TABLE risk_events (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    account_id   uuid REFERENCES accounts (id),
    event_type   text NOT NULL,          -- kill switch, rate limit, order intent conflict, ...
    severity     text NOT NULL DEFAULT 'INFO',
    payload_json jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at   timestamptz NOT NULL DEFAULT now(),
    created_by   text NOT NULL DEFAULT 'system',
    CONSTRAINT risk_events_severity_check CHECK (severity IN ('INFO', 'WARNING', 'CRITICAL'))
);
CREATE INDEX risk_events_owner_idx ON risk_events (owner_user_id);
