# Owner-beta Paper stop operations

This runbook describes the source controller for R5-04. Production remains
**CLOSED**, `ENABLE=false`. STOPPED is a writer/backend observation boundary;
it does not commit OFF, reconcile economic history, enable a worker, authorize a
backup, or prove restoration. Those continuations remain separate work under
[the adopted operations contract](../contracts/owner-beta-paper-operations-v1.md).

The installed root caller keeps the existing release-lock open description for
the whole operation. It creates the canonical journal intent and actual
ActionCustody, invokes the selected native BEGIN, imports its original dispatch,
and settles its helper resources. `live.fence_begin(invocation)` independently
reads the exact original SQL result, requires current DRAINING, seals FENCED and
advances the local floor. No imported historical result can replace that call.

The consuming Python API is `drained = live.drain(invocation)`. The argument must
be the same live, settled invocation that this context fenced. The caller keeps
the action, operation, journal, verified release and release lock alive. There
is no command-line selector, environment approval, JSON permit or startup path.
The returned `DrainedHandle` has `revalidate()` and `close()` and supports a
context manager. It cannot be copied, pickled or restored from a receipt.

The controller performs these operations under one original 90-second deadline:

1. Recheck FENCED, the retained BEGIN result FD, action/import custody, current
   boot, installation/account/generation, release manifest, event and epoch floor.
2. Enumerate actual Docker generations, all installed/loaded systemd services
   and timers, their source originals and triggers, invocation processes and
   writable roots. Reject installation-linked cron/at jobs rather than editing
   unsanctioned sources. Scan actual PostgreSQL clients through the selected
   local privileged socket. Correlate writers using socket endpoints and
   PID/backend_start; an application name is never a selector or exemption.
3. Disable launch sources before signaling. Disable timers and container restart
   policies. For a running systemd writer, hold its exact MainPID briefly with
   its retained pidfd, verify the invocation again, submit the nonblocking stop
   and always release that exact pause. This prevents stopping an automatically
   restarted replacement between inspection and the unit-name stop request.
   No collector or scheduler lock is held during dispatch or process waits.
4. Wait for captured generations and owned children. Observe retained container
   IDs twice, requiring exit/FinishedAt, unchanged restart generation, restart=no
   and empty owned cgroups. Reap only the controller's actual children. Do not
   remove exited containers before the inventory and STOPPED are sealed.
5. After publisher/children exit, acquire collector locks, then scheduler locks,
   then the existing Daily/Raw publication locks. Source B uses its existing
   SIGTERM/finally shutdown: INVALID publication and OFD closure precede asset
   changes. A missing INVALID/OFD observation remains UNKNOWN even if processes
   exited. Never force-unlock, unlink or replace a borrowed lock.
6. Cancel/terminate only captured PostgreSQL PID/backend_start pairs, with that
   equality repeated inside fixed SQL. Finish each read-only transaction before
   any OS wait. Reject prepared work, masked visibility, unexplained sessions,
   a new backend or a changed fence. Observe complete absence again.
7. Store protected no-replace observation originals and the closed writer
   inventory, append STOPPED, then return the context-bound live handle.

PAPER_STOP explicitly covers API, PAPER_RUNNER and PAPER_DELIVERY, including
proved absence. This implementation conservatively rejects shared writers whose
inability to mutate that subset is not established. It never silently changes
scope. COMBINED_BACKUP inventories all eleven frozen classes; a single generation
may cover multiple classes but is signaled only once. Existing upstream Compose
images are bound to their actual installed source pins; they are not additional
V3 manifest rows or fabricated image IDs.

UNKNOWN/timeout leaves the SQL account DRAINING and retains captured identities
and protected observations. Signal success, shell exit, lease expiry, health
checks and Docker stop status do not substitute for termination. A failed attempt
at FENCED can be observed again only by another explicit `drain` call. No hidden
retry resets the original deadline. A handle remains bounded by that attempt's
deadline and is poisoned on failed revalidation, close, context loss or restart;
serialized STOPPED data never renews it. Later long-lived backup/reconciliation
consumption must establish its own contract-required fresh observations.

The existing required hosted PostgreSQL job owns genuine integration. Its ten
G2 cases remain: STOP, BACKUP, RESTORE rejection and the seven existing fault
cases. STOP exercises actual selected-image BEGIN/readback and proved absence.
BACKUP also creates a real `app` PostgreSQL client inside the official selected
Paper image, with a private password file and internal network, and captures its
actual socket/backend identity. It is classified OTHER_DB_WRITER, never passed
off as an API or paper-runner ELF. The production drain stops it; independent
fixture queries check backend/prepared-work absence and unchanged event counts.
The fixture retains/reaps its attach child and removes only its captured exited
container after the handle closes. The independent PG pin, all thirteen official
images, original producer receipts/ELFs, native environment, canonical custody,
offline readback and cleanup reserve remain required.

Local stdlib tests exercise decisions, protected-file lifetime and one actual
owned nonroot child. They do not establish Root/systemd/Docker/PG behavior or
operational readiness. Genuine running API/Paper delivery shutdown, production
source B/timer coexistence, later RECONCILE/OFF/recovery, P5B encrypted restore,
final same-candidate CI/smoke, pre/post sessions and Owner acceptance remain open
until their actual evidence is recovered. No serving or migration role gains
monitoring/signaling privileges, and no provider/account/order permission changes.
