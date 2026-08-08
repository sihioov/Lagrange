-- 0017: carry the authentication context on the session (plan Todo 37).
--
-- `auth::stepup::require_owner_step_up` needs two facts the session did not
-- record: WHICH methods authenticated the user (`amr`) and WHEN (`auth_time`).
-- Without them the loader could only pass `amr: []`, so step-up denied every
-- request with STEP_UP_MFA_REQUIRED -- safe, but it made every Owner Live
-- action permanently impossible. Todo 37 needs step-up to be passable by a
-- genuinely fresh-MFA Owner, so the facts have to reach the session.
--
-- `auth_time` is distinct from `created_at` on purpose: a session can be
-- created from a token that authenticated much earlier (a silent renewal), and
-- treating session age as authentication age would make a stale login look
-- fresh -- exactly the property step-up exists to check.

ALTER TABLE web_sessions
    ADD COLUMN amr text[] NOT NULL DEFAULT '{}',
    -- Defaults to created_at for rows written before this migration; those
    -- sessions carry no MFA claim anyway, so they still cannot pass step-up.
    ADD COLUMN auth_time timestamptz;

UPDATE web_sessions SET auth_time = created_at WHERE auth_time IS NULL;
