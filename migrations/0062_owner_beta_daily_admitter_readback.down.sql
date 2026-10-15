-- Read-only APIs own no durable data. Keep all prior admissions and receipts.
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';
DROP FUNCTION public.read_owner_beta_daily_admission_receipt(uuid);
DROP FUNCTION public.read_owner_beta_daily_admitted_input(text);
DROP FUNCTION public.read_owner_beta_daily_admission_context(uuid);
