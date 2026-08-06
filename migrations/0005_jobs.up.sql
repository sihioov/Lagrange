-- 0005: Job queue — jobs and job_attempts. Design §6.8, §7.2 (Backtest);
-- requirements §10.2 NFR-REL-002/003; plan Todo 19.
--
-- PUBLIC job lifecycle has EXACTLY five states (QUEUED|RUNNING|SUCCEEDED|
-- FAILED|CANCELED) — ORPHANED is attempt-level only and must never appear in
-- `jobs.status` (named constraint `jobs_status_check`, asserted by the
-- migration-contract suite). Cancellation is cooperative: a job enters
-- CANCELED only via an audited cancel request; a dead worker surfaces as an
-- ORPHANED attempt, never as a sixth job status.

CREATE TABLE jobs (
    id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_user_id   uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    job_type        text NOT NULL,            -- 'backtest' | 'paper' | 'report' | ...
    status          text NOT NULL DEFAULT 'QUEUED',
    priority        int NOT NULL DEFAULT 10,
    idempotency_key text,                     -- per-owner idempotent submission
    payload_json    jsonb NOT NULL DEFAULT '{}'::jsonb,
    max_attempts    int NOT NULL DEFAULT 3,
    attempt_count   int NOT NULL DEFAULT 0,
    available_at    timestamptz NOT NULL DEFAULT now(),
    locked_by       text,                     -- worker id holding the claim
    locked_at       timestamptz,
    started_at      timestamptz,
    finished_at     timestamptz,
    error_code      text,
    error_message   text,
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT jobs_status_check CHECK (status IN ('QUEUED', 'RUNNING', 'SUCCEEDED', 'FAILED', 'CANCELED')),
    CONSTRAINT jobs_max_attempts_check CHECK (max_attempts >= 1),
    CONSTRAINT jobs_owner_idempotency_key UNIQUE (owner_user_id, idempotency_key)
);
-- Claim scan (design §6.8): WHERE status='QUEUED' AND available_at <= now()
-- ORDER BY priority DESC, created_at FOR UPDATE SKIP LOCKED LIMIT 1.
CREATE INDEX jobs_claim_idx ON jobs (status, available_at, priority DESC, created_at);
CREATE INDEX jobs_owner_idx ON jobs (owner_user_id);

-- Immutable attempt records. `outcome` is attempt-level ONLY: RUNNING while
-- claimed, then SUCCEEDED | FAILED | ORPHANED. CANCELED is NOT an attempt
-- outcome — worker death is detected as ORPHANED and requeued at most once
-- (NFR-REL-003); input/integrity errors never retry (design §6.8).
CREATE TABLE job_attempts (
    id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    job_id        uuid NOT NULL REFERENCES jobs (id) ON DELETE CASCADE,
    attempt_no    int NOT NULL,
    outcome       text NOT NULL,
    claimed_by    text,                     -- worker id that claimed this attempt
    error_code    text,
    error_message text,
    started_at    timestamptz,
    finished_at   timestamptz,
    created_at    timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT job_attempts_outcome_check CHECK (outcome IN ('RUNNING', 'SUCCEEDED', 'FAILED', 'ORPHANED')),
    CONSTRAINT job_attempts_job_attempt_no_key UNIQUE (job_id, attempt_no)
);
CREATE INDEX job_attempts_job_idx ON job_attempts (job_id);
