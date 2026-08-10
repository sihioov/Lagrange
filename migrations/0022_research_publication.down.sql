-- Reverse 0022 in dependency order. The externally provisioned
-- research_writer role is deliberately retained.

DROP POLICY IF EXISTS trading_calendar_versions_insert_research_writer ON trading_calendar_versions;
DROP POLICY IF EXISTS trading_calendar_versions_select_research_writer ON trading_calendar_versions;
DROP POLICY IF EXISTS trading_calendar_versions_select_readers ON trading_calendar_versions;
DROP POLICY IF EXISTS trading_calendars_update_research_writer ON trading_calendars;
DROP POLICY IF EXISTS trading_calendars_insert_research_writer ON trading_calendars;
DROP POLICY IF EXISTS trading_calendars_select_research_writer ON trading_calendars;
DROP POLICY IF EXISTS data_batches_insert_research_writer ON data_batches;
DROP POLICY IF EXISTS data_batches_select_research_writer ON data_batches;

REVOKE USAGE ON SEQUENCE trading_calendar_versions_id_seq FROM research_writer;
REVOKE SELECT, INSERT ON TABLE trading_calendar_versions FROM research_writer;
REVOKE SELECT, INSERT, UPDATE ON TABLE trading_calendars FROM research_writer;
REVOKE SELECT, INSERT ON TABLE data_batches FROM research_writer;
REVOKE SELECT ON TABLE trading_calendar_versions FROM app, worker, admin;

DROP TRIGGER IF EXISTS trading_calendar_versions_append_only ON trading_calendar_versions;
DROP FUNCTION IF EXISTS trading_calendar_versions_reject_mutation();
DROP TABLE IF EXISTS trading_calendar_versions;

DROP INDEX IF EXISTS data_batches_raw_lineage_key;

ALTER TABLE trading_calendars
    DROP CONSTRAINT IF EXISTS trading_calendars_provenance_all_or_none_check,
    DROP CONSTRAINT IF EXISTS trading_calendars_content_sha256_check,
    DROP COLUMN IF EXISTS retrieved_at,
    DROP COLUMN IF EXISTS content_sha256,
    DROP COLUMN IF EXISTS source_batch_id;

ALTER TABLE data_batches
    DROP CONSTRAINT IF EXISTS data_batches_provenance_all_or_none_check,
    DROP CONSTRAINT IF EXISTS data_batches_fetch_mode_check,
    DROP COLUMN IF EXISTS fetch_mode,
    DROP COLUMN IF EXISTS source_file_name,
    DROP COLUMN IF EXISTS source_batch_id;
