SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

LOCK TABLE
    public.owner_equity_signal_snapshot_rows,
    public.owner_equity_signal_snapshots,
    public.owner_equity_generation_admissions,
    public.owner_equity_instrument_generations,
    public.owner_equity_membership_events,
    public.owner_equity_memberships,
    public.owner_equity_universe_policies
IN ACCESS EXCLUSIVE MODE;

-- FORCE RLS hides tenant rows from an unscoped migration connection. The
-- access-exclusive locks above make this temporary inspection complete; a
-- raised exception rolls the ALTERs back with the migration transaction.
ALTER TABLE public.owner_equity_signal_snapshot_rows NO FORCE ROW LEVEL SECURITY;
ALTER TABLE public.owner_equity_signal_snapshots NO FORCE ROW LEVEL SECURITY;
ALTER TABLE public.owner_equity_generation_admissions NO FORCE ROW LEVEL SECURITY;
ALTER TABLE public.owner_equity_instrument_generations NO FORCE ROW LEVEL SECURITY;
ALTER TABLE public.owner_equity_membership_events NO FORCE ROW LEVEL SECURITY;
ALTER TABLE public.owner_equity_memberships NO FORCE ROW LEVEL SECURITY;
ALTER TABLE public.owner_equity_universe_policies NO FORCE ROW LEVEL SECURITY;

DO $rollback_guard$
BEGIN
    IF EXISTS (SELECT 1 FROM public.owner_equity_signal_snapshot_rows)
       OR EXISTS (SELECT 1 FROM public.owner_equity_signal_snapshots)
       OR EXISTS (SELECT 1 FROM public.owner_equity_generation_admissions)
       OR EXISTS (SELECT 1 FROM public.owner_equity_instrument_generations)
       OR EXISTS (SELECT 1 FROM public.owner_equity_membership_events)
       OR EXISTS (SELECT 1 FROM public.owner_equity_memberships)
    THEN
        RAISE EXCEPTION 'owner equity universe V2 rollback would discard durable state'
            USING ERRCODE = '55000';
    END IF;
END
$rollback_guard$;

-- Policy rows are derived from Owner role grants and contain no market-data
-- lineage. Once the durable-state guard above passes they may be removed so
-- the reversible migration remains usable.
DELETE FROM public.owner_equity_universe_policies;

DROP FUNCTION public.schedule_owner_equity_incremental(
    uuid, uuid, date, text, text, text
);
DROP TRIGGER user_roles_provision_owner_equity_universe_policy
    ON public.user_roles;
DROP FUNCTION public.provision_owner_equity_universe_policy();

DROP FUNCTION public.disable_owner_equity_membership(uuid, text, text);
DROP FUNCTION public.retry_owner_equity_membership(uuid, text, text);

DROP TRIGGER owner_equity_signal_snapshot_rows_immutable
    ON public.owner_equity_signal_snapshot_rows;
DROP TRIGGER owner_equity_generation_admissions_immutable
    ON public.owner_equity_generation_admissions;
DROP TRIGGER owner_equity_instrument_generations_immutable
    ON public.owner_equity_instrument_generations;
DROP TRIGGER owner_equity_membership_events_immutable
    ON public.owner_equity_membership_events;
DROP FUNCTION public.owner_equity_append_only_guard();

DROP TRIGGER owner_equity_signal_snapshots_guard
    ON public.owner_equity_signal_snapshots;
DROP FUNCTION public.owner_equity_signal_snapshots_guard();
DROP TRIGGER owner_equity_signal_snapshot_rows_guard
    ON public.owner_equity_signal_snapshot_rows;
DROP FUNCTION public.owner_equity_signal_snapshot_rows_guard();
DROP TRIGGER owner_equity_generation_admissions_guard
    ON public.owner_equity_generation_admissions;
DROP FUNCTION public.owner_equity_generation_admissions_guard();
DROP TRIGGER owner_equity_instrument_generations_guard
    ON public.owner_equity_instrument_generations;
DROP FUNCTION public.owner_equity_instrument_generations_guard();
DROP TRIGGER owner_equity_memberships_record_update_event
    ON public.owner_equity_memberships;
DROP TRIGGER owner_equity_memberships_record_insert_event
    ON public.owner_equity_memberships;
DROP FUNCTION public.owner_equity_memberships_record_event();
DROP TRIGGER owner_equity_memberships_guard
    ON public.owner_equity_memberships;
DROP FUNCTION public.owner_equity_memberships_guard();

DROP TABLE public.owner_equity_signal_snapshot_rows;
DROP TABLE public.owner_equity_signal_snapshots;
DROP TABLE public.owner_equity_generation_admissions;
DROP TABLE public.owner_equity_instrument_generations;
DROP TABLE public.owner_equity_membership_events;
DROP TABLE public.owner_equity_memberships;
DROP TABLE public.owner_equity_universe_policies;
