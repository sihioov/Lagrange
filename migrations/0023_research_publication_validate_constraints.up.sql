-- 0023: validate 0022's populated-table checks after its metadata transaction
-- commits. Validation scans use a lighter lock than the ALTER TABLE changes.

SET LOCAL lock_timeout = '5s';

ALTER TABLE data_batches
    VALIDATE CONSTRAINT data_batches_fetch_mode_check,
    VALIDATE CONSTRAINT data_batches_provenance_all_or_none_check;

ALTER TABLE trading_calendars
    VALIDATE CONSTRAINT trading_calendars_content_sha256_check,
    VALIDATE CONSTRAINT trading_calendars_provenance_all_or_none_check;
