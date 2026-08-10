-- Reverse 0023 to 0022's NOT VALID boundary without changing publication data.

SET LOCAL lock_timeout = '5s';

ALTER TABLE data_batches
    DROP CONSTRAINT data_batches_fetch_mode_check,
    DROP CONSTRAINT data_batches_provenance_all_or_none_check,
    ADD CONSTRAINT data_batches_fetch_mode_check
        CHECK (fetch_mode IS NULL OR fetch_mode IN ('synthetic', 'credentialed')) NOT VALID,
    ADD CONSTRAINT data_batches_provenance_all_or_none_check
        CHECK (
            (source_batch_id IS NULL AND source_file_name IS NULL AND fetch_mode IS NULL)
            OR
            (source_batch_id IS NOT NULL AND source_file_name IS NOT NULL AND fetch_mode IS NOT NULL)
        ) NOT VALID;

ALTER TABLE trading_calendars
    DROP CONSTRAINT trading_calendars_content_sha256_check,
    DROP CONSTRAINT trading_calendars_provenance_all_or_none_check,
    ADD CONSTRAINT trading_calendars_content_sha256_check
        CHECK (content_sha256 IS NULL OR content_sha256 ~ '^[0-9a-f]{64}$') NOT VALID,
    ADD CONSTRAINT trading_calendars_provenance_all_or_none_check
        CHECK (
            (source_batch_id IS NULL AND content_sha256 IS NULL AND retrieved_at IS NULL)
            OR
            (source_batch_id IS NOT NULL AND content_sha256 IS NOT NULL AND retrieved_at IS NOT NULL)
        ) NOT VALID;
