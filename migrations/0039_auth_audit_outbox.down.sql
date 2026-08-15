REVOKE ALL ON FUNCTION public.prune_auth_audit_outbox(bigint, integer)
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
DROP FUNCTION public.prune_auth_audit_outbox(bigint, integer);
REVOKE ALL ON FUNCTION public.auth_audit_outbox_stats()
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
DROP FUNCTION public.auth_audit_outbox_stats();
REVOKE ALL ON FUNCTION public.deliver_auth_audit_batch(integer)
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
DROP FUNCTION public.deliver_auth_audit_batch(integer);
REVOKE ALL ON FUNCTION public.enqueue_auth_audit(text, text, uuid, text, text, text, bigint)
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
DROP FUNCTION public.enqueue_auth_audit(text, text, uuid, text, text, text, bigint);

DROP POLICY IF EXISTS auth_audit_outbox_owner_delete ON public.auth_audit_outbox;
DROP POLICY IF EXISTS auth_audit_outbox_owner_update ON public.auth_audit_outbox;
DROP POLICY IF EXISTS auth_audit_outbox_owner_read ON public.auth_audit_outbox;
DROP POLICY IF EXISTS auth_audit_outbox_enqueue ON public.auth_audit_outbox;
DROP POLICY IF EXISTS auth_audit_log_insert_migration_owner ON public.audit_logs;
DROP TABLE public.auth_audit_outbox;
