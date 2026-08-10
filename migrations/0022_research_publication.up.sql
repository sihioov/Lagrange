-- 0022: Raw-to-PostgreSQL research publication lineage and calendar history.
--
-- Legacy manifests/calendars predate publication provenance, so their new
-- columns stay nullable. New publication writes bind every projection and
-- derived batch to a complete stable Raw lineage.
--
-- This migration stays transactional. `lock_timeout` bounds the brief
-- metadata locks needed for column/constraint changes; the populated-table
-- unique index follows separately in nontransactional 0023.

SET LOCAL lock_timeout = '5s';

ALTER TABLE data_batches
    ADD COLUMN source_batch_id uuid,
    ADD COLUMN source_file_name text,
    ADD COLUMN fetch_mode text,
    ADD CONSTRAINT data_batches_fetch_mode_check
        CHECK (fetch_mode IS NULL OR fetch_mode IN ('synthetic', 'credentialed')) NOT VALID,
    ADD CONSTRAINT data_batches_provenance_all_or_none_check
        CHECK (
            (source_batch_id IS NULL AND source_file_name IS NULL AND fetch_mode IS NULL)
            OR
            (source_batch_id IS NOT NULL AND source_file_name IS NOT NULL AND fetch_mode IS NOT NULL)
        ) NOT VALID;

ALTER TABLE data_batches
    VALIDATE CONSTRAINT data_batches_fetch_mode_check,
    VALIDATE CONSTRAINT data_batches_provenance_all_or_none_check;

CREATE TABLE trading_calendar_versions (
    id              bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    exchange        text NOT NULL,
    session_date    date NOT NULL,
    session_type    text NOT NULL CHECK (session_type IN ('TRADING', 'CLOSED')),
    timezone        text NOT NULL CHECK (timezone = 'Asia/Seoul'),
    source          text NOT NULL,
    source_version  text NOT NULL,
    source_batch_id uuid NOT NULL,
    content_sha256  text NOT NULL CHECK (content_sha256 ~ '^[0-9a-f]{64}$'),
    retrieved_at    timestamptz NOT NULL,
    created_at      timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT trading_calendar_versions_exchange_date_source_version_key
        UNIQUE (exchange, session_date, source_version)
);

ALTER TABLE trading_calendars
    ADD COLUMN source_batch_id uuid,
    ADD COLUMN content_sha256 text,
    ADD COLUMN retrieved_at timestamptz,
    ADD CONSTRAINT trading_calendars_content_sha256_check
        CHECK (content_sha256 IS NULL OR content_sha256 ~ '^[0-9a-f]{64}$') NOT VALID,
    ADD CONSTRAINT trading_calendars_provenance_all_or_none_check
        CHECK (
            (source_batch_id IS NULL AND content_sha256 IS NULL AND retrieved_at IS NULL)
            OR
            (source_batch_id IS NOT NULL AND content_sha256 IS NOT NULL AND retrieved_at IS NOT NULL)
        ) NOT VALID;

ALTER TABLE trading_calendars
    VALIDATE CONSTRAINT trading_calendars_content_sha256_check,
    VALIDATE CONSTRAINT trading_calendars_provenance_all_or_none_check;

CREATE FUNCTION trading_calendar_versions_reject_mutation() RETURNS trigger
LANGUAGE plpgsql AS $fn$
BEGIN
    RAISE EXCEPTION
        'trading_calendar_versions is append-only: % is refused', TG_OP
        USING ERRCODE = '55000';
END
$fn$;

CREATE TRIGGER trading_calendar_versions_append_only
    BEFORE UPDATE OR DELETE ON trading_calendar_versions
    FOR EACH ROW EXECUTE FUNCTION trading_calendar_versions_reject_mutation();

-- The publication role is intentionally narrower than a generic data writer:
-- it can write only the Raw manifest, immutable calendar history, and current
-- calendar projection. RLS remains the inner boundary below these grants.
GRANT SELECT, INSERT ON TABLE data_batches TO research_writer;
GRANT SELECT, INSERT ON TABLE trading_calendar_versions TO research_writer;
GRANT SELECT, INSERT, UPDATE ON TABLE trading_calendars TO research_writer;
GRANT SELECT ON TABLE trading_calendar_versions TO app, worker, admin;

CREATE POLICY data_batches_select_research_writer ON data_batches
    FOR SELECT TO research_writer USING (true);
CREATE POLICY data_batches_insert_research_writer ON data_batches
    FOR INSERT TO research_writer WITH CHECK (true);

CREATE POLICY trading_calendars_select_research_writer ON trading_calendars
    FOR SELECT TO research_writer USING (true);
CREATE POLICY trading_calendars_insert_research_writer ON trading_calendars
    FOR INSERT TO research_writer WITH CHECK (true);
CREATE POLICY trading_calendars_update_research_writer ON trading_calendars
    FOR UPDATE TO research_writer USING (true) WITH CHECK (true);

ALTER TABLE trading_calendar_versions ENABLE ROW LEVEL SECURITY;

CREATE POLICY trading_calendar_versions_select_readers ON trading_calendar_versions
    FOR SELECT TO app, worker, admin USING (true);
CREATE POLICY trading_calendar_versions_select_research_writer ON trading_calendar_versions
    FOR SELECT TO research_writer USING (true);
CREATE POLICY trading_calendar_versions_insert_research_writer ON trading_calendar_versions
    FOR INSERT TO research_writer WITH CHECK (true);
