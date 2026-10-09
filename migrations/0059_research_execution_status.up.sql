SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

CREATE TABLE public.research_execution_status (
    id text PRIMARY KEY,
    attempt_started_at timestamptz NOT NULL,
    heartbeat_at timestamptz NOT NULL,
    attempt_finished_at timestamptz,
    attempt_state text NOT NULL,
    last_success_at timestamptz,
    last_failure_at timestamptz,
    failure_code text,
    target_session date,
    published_session date,
    updated_at timestamptz NOT NULL,
    CONSTRAINT research_execution_status_singleton_check
        CHECK (id = 'kis-eod'),
    CONSTRAINT research_execution_status_state_check
        CHECK (attempt_state IN ('RUNNING', 'SUCCEEDED', 'FAILED', 'NO_WORK')),
    CONSTRAINT research_execution_status_heartbeat_check
        CHECK (heartbeat_at >= attempt_started_at),
    CONSTRAINT research_execution_status_finish_check
        CHECK (
            (attempt_state = 'RUNNING' AND attempt_finished_at IS NULL)
            OR (attempt_state <> 'RUNNING'
                AND attempt_finished_at IS NOT NULL
                AND attempt_finished_at >= attempt_started_at)
        ),
    CONSTRAINT research_execution_status_failure_code_check
        CHECK (
            (attempt_state = 'FAILED'
             AND failure_code IN (
                 'PIPELINE_FAILED', 'PUBLICATION_FAILED',
                 'RIGHTS_UNAVAILABLE', 'EXECUTION_INTERRUPTED'
             ))
            OR (attempt_state <> 'FAILED' AND failure_code IS NULL)
        ),
    CONSTRAINT research_execution_status_clock_check
        CHECK (
            attempt_started_at <= pg_catalog.clock_timestamp()
            AND heartbeat_at <= pg_catalog.clock_timestamp()
            AND (attempt_finished_at IS NULL
                 OR attempt_finished_at <= pg_catalog.clock_timestamp())
            AND (last_success_at IS NULL
                 OR last_success_at <= pg_catalog.clock_timestamp())
            AND (last_failure_at IS NULL
                 OR last_failure_at <= pg_catalog.clock_timestamp())
            AND updated_at <= pg_catalog.clock_timestamp()
        )
);

ALTER TABLE public.research_execution_status OWNER TO migration_owner;
ALTER TABLE public.research_execution_status ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.research_execution_status FORCE ROW LEVEL SECURITY;

REVOKE ALL ON TABLE public.research_execution_status
    FROM PUBLIC, app, worker, admin, audit_writer, research_writer;
GRANT SELECT ON TABLE public.research_execution_status TO admin;
GRANT SELECT, INSERT, UPDATE ON TABLE public.research_execution_status
    TO research_writer;

CREATE POLICY research_execution_status_admin_select
    ON public.research_execution_status FOR SELECT TO admin
    USING (id = 'kis-eod');
CREATE POLICY research_execution_status_research_writer_select
    ON public.research_execution_status FOR SELECT TO research_writer
    USING (id = 'kis-eod');
CREATE POLICY research_execution_status_research_writer_insert
    ON public.research_execution_status FOR INSERT TO research_writer
    WITH CHECK (id = 'kis-eod');
CREATE POLICY research_execution_status_research_writer_update
    ON public.research_execution_status FOR UPDATE TO research_writer
    USING (id = 'kis-eod')
    WITH CHECK (id = 'kis-eod');
CREATE POLICY research_execution_status_migration_owner_all
    ON public.research_execution_status FOR ALL TO migration_owner
    USING (id = 'kis-eod')
    WITH CHECK (id = 'kis-eod');
