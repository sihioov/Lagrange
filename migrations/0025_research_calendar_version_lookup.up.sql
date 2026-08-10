-- no-transaction
-- 0025: support global immutable calendar source-version provenance checks
-- without scanning all calendar history rows.
-- Deployment must set a finite session lock_timeout externally, for example:
--   PGOPTIONS='-c lock_timeout=5s' sqlx migrate run
-- SQLx sends this file as one command and PostgreSQL requires concurrent DDL
-- to be its sole statement.

CREATE INDEX CONCURRENTLY trading_calendar_versions_source_lookup_idx
    ON trading_calendar_versions (exchange, source_version)
    INCLUDE (source, timezone, content_sha256);
