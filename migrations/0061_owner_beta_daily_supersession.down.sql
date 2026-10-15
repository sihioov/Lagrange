-- Remove only the added authority. Permanent runs/reservations are unchanged.
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';
DROP FUNCTION public.cancel_superseded_owner_beta_daily_claim(uuid,uuid,uuid,integer,text);
