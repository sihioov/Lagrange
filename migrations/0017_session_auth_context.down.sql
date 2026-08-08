-- 0017 down.
ALTER TABLE web_sessions DROP COLUMN IF EXISTS amr;
ALTER TABLE web_sessions DROP COLUMN IF EXISTS auth_time;
