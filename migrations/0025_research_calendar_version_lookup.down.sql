-- no-transaction
-- Reverse 0025 without taking a long blocking index-drop lock.
-- Deployment must set a finite session lock_timeout externally (for example
-- with PGOPTIONS): SQLx sends this file as one command and PostgreSQL requires
-- concurrent DDL to be its sole statement.

DROP INDEX CONCURRENTLY IF EXISTS trading_calendar_versions_source_lookup_idx;
