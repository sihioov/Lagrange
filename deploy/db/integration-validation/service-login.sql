-- Run as each role's own login (not as postgres/SET ROLE). This catches the
-- session_user branches in SECURITY DEFINER functions and proves passwords,
-- CONNECT grants, and role names are wired for the real serving boundary.
\set ON_ERROR_STOP on
SELECT current_user || '=' || session_user;
