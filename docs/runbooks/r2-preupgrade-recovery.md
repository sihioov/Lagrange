# R2 pre-upgrade consistent recovery preparation

Status: **Wave114 maintenance-v3 source written; new hosted validation pending. Wave109 v2 remains accepted at3de6420; production recovery not executed**.
Candidate899f259f9c8872fb61f0c95745904f552c8750e9 passed20 filesystem/contract,
28 maintenance,3 preparation and4 actual Docker/PostgreSQL cases in
CI39098366992 attempt1. The complete
[hosted recovery](../reviews/2026-10-18-r2-wholeflow-git-ci.md) binds each named result;
terminal receipt SHA256c87097708cf803ce69ec0e6d63bfe2d4242e20c1d084e489e6da67c8fbd0d31b.
This is the pre-upgrade part of R2-04/R2-09, alongside the
[six-file backup transition](r2-first-backup-transition.md) and actual
approved-base host integration. Historical
[full failure](../reviews/2026-10-16-r2-first-rollout-metadata-ci.md) `6572bff` remains preserved:
the18 filesystem/contract,28 maintenance and3 preparation cases passed; the
native positive errored at PGDATA ownership and the other3 native printed PASS
rows did not establish their intended failure stages. Wave85 adds2 filesystem
definitions and tightened those3 native assertions; the accepted hosted run
above now supplies their fresh results. No production pre-upgrade fence,
capture, clone proof or operating acceptance is established by those fixtures.

## Recovery method and its operating cost

The first upgrade cannot require schema62 daily control tables that do not yet
exist. Its prepared method is a **cold complete PostgreSQL cluster** plus Raw,
Curated, approved historical base and protected old-release configuration at
one quiescent recovery point. Whole-cluster bytes retain databases, WAL, global
roles/password verifiers, ownership, ACLs, RLS and table contents. This replaces
the earlier proposed logical pre-upgrade dump method; it preserves the recovery
requirement and adds an explicit database-outage requirement.

PostgreSQL permits filesystem backup only with a stopped server or a genuinely
consistent filesystem snapshot, and requires the whole cluster, including
transaction state. Merely denying client connections does not make copying a
running cluster valid. This implementation uses the stopped-server branch.
[PostgreSQL18 filesystem-backup documentation](https://www.postgresql.org/docs/18/backup-file.html).

The source database and every relevant writer must be stopped or fenced within
the eventual G4 command ledger. That ledger must name the exact services,
timers, containers, volume, roots, interruption deadline and resumption owner.
This document grants no downtime or new root authority. The historical sudo
expiry is not current access evidence; existing KIS read-only/EOD and Owner V2
approvals remain intact.

## Written components

The [file archive implementation](../../scripts/ops/lib/owner-beta-daily-backup-files.py)
has an explicit `preupgrade` profile with its own magic/schema and exactly five
nonoverlapping classes: `raw`, `curated`, `base`, `config`, `postgres`. The daily
profile remains the default six-class format and rejects cold archives. UID999
is permitted only inside the cold `postgres` class. Symlinks, hard links,
unexpected owners, ACLs, ordinary writable ancestors, size/entry overflow,
changed identities/bytes, corruption and trailing bytes remain refusals.

The official Docker image can use a postgres:postgres1777 volume root. The
cold profile permits exactly that root mode only beneath a root:root0700 host
parent, held by descriptor and rechecked. No descendant, other class or daily
archive inherits it. Actual pinned-image metadata still requires observation.
[Official Docker PostgreSQL18 definition](https://github.com/docker-library/postgres/blob/master/18/bookworm/Dockerfile).

The disposable native fixture establishes this parent protection explicitly.
It first binds its freshly created labelled local volume and stopped database
generation, holds the protected shared ancestor, and changes only that volume's
own parent through an open descriptor. It checks root ownership, no writable
group/other access or ACL, and unchanged inode/name binding before and after
setting0700; it keeps the child descriptor open through volume and stopped
generation rechecks, then verifies the final name/inode and access metadata. The
shared Docker directory and production capture guard remain unchanged.
This fixture setup does not authorize changing a production volume's permissions.
Live parent metadata still needs fresh observation and, if incompatible, a
separately authorized exact protection step before capture can proceed.

The examined [Moby local-volume driver](https://github.com/moby/moby/blob/v28.5.1/volume/local/local_unix.go#L214-L220)
derives `CreatedAt` from the volume parent's ctime. The fixture therefore does
not require this derived timestamp to remain equal across its own permission
change. Name, driver, scope, options, labels and mountpoint must remain equal;
directory identity, ownership, ACL and access-mode checks remain mandatory.
Diagnostics contain only a closed list of changed field names and finite mode
values. Regressions distinguish timestamp-only change, changes to each of the
six stable fields, and replacement of the directory during Docker observation.
Those three tests use real private directories with controlled Docker metadata;
the four existing native cases still require actual Docker and PostgreSQL.

The [recovery library](../../scripts/ops/lib/production-preupgrade-recovery.py)
checks the exact stopped container, pinned image, generation and local volume.
It requires the PG18 whole-volume layout, retained WAL/control files, no
postmaster/recovery markers and no external tablespaces. Absence of a pid file
alone is explicitly insufficient evidence of shutdown.

The PGDATA directory accepts only uid/gid/mode tuples999/999/0700,
999/999/0750 and999/0/0700. The last retains owner-only access while supporting
a root-created directory whose user alone was changed to PostgreSQL. The
[official entrypoint](https://github.com/docker-library/postgres/blob/4f9ced003ba58a854656ba150d146243d27ae3ac/docker-entrypoint.sh)
creates and protects PGDATA before changing its user; the
[PG18 image definition](https://github.com/docker-library/postgres/blob/e00e1bd34ec5c8a8e7ad89b273b3d42efaf6d5bc/18/bookworm/Dockerfile)
places it beneath the volume. These sources support the compatibility case;
they do not identify the previous runner's unprinted metadata or establish a
digest-to-source mapping. Root-group0750, other owners/groups and world access
remain rejected. No owner/group/mode normalization occurs during restore.
PG_VERSION/control-file ownership, full manifest equality, ACL checks and the
private volume-parent boundary remain unchanged.

Native negative assertions require their exact intended error. Catalog mismatch
requires the post-audit comparison code; lost-create-response requires an
actually returned container identity before injecting that error. Corrupt
control-file rejection requires the owned clone's reserved exit73 from the
native control/shutdown-state check. Generic command failures, timeouts and
Docker errors do not establish that boundary. Only finite source/restored
uid/gid/mode observations are printed after complete named test-result rows.

`ColdStorage.verify_cluster` accepts a newly restored private staging tree.
It first compares every restored file hash/owner/group/mode with the archive
manifest. It boots only the PostgreSQL copy in a new pinned-image container:
network none, UID999, read-only container root, no capabilities, no new
privileges, bounded memory/CPU/time and only the copied volume and fixed
verification scripts mounted. It cannot activate a production container.

The [clone entrypoint](../../deploy/db/preupgrade-recovery-verify.sh) checks
`pg_controldata` for clean shutdown, overrides network/recovery/preload settings,
performs a complete target-database logical read with bounded `pg_dump`, and
returns the [fixed read-only audit](../../deploy/db/preupgrade-recovery-audit.sql).
The audit binds database/cluster/version, every old migration checksum and a
catalog fingerprint covering ownership, ACLs, roles/membership, policies,
indexes, constraints, types, functions, triggers, extensions and role settings.
Password values and application rows are not emitted. Planner/freeze statistics
are omitted from the catalog comparison; the physical archive still preserves
the original bytes. Full audit equality with the pre-stop observation is
required. The original server/volume is not mounted into the verifier.

## Written verification and limits

[Filesystem tests](../../scripts/ops/production-preupgrade-recovery-self-test.py)
cover both format directions, UID/mode boundaries, overlapping roots, complete
PG layout, WAL/marker/tablespace rejection, corruption, limits, concurrent
changes and exact container/volume/audit bindings. These tests use actual
private Linux filesystem ownership; Docker observations in them are fixtures.

[Docker tests](../../scripts/ops/production-preupgrade-recovery-docker-self-test.py)
prepare a real disposable schema59 with the existing migration bytes, role
membership and an RLS-protected fixture table, capture encrypted full-cluster
files after actual shutdown, restore them into a private tree and require
networkless clone/audit equality. Negative cases cover a mismatched catalog
commitment and corrupt control bytes whose archive hash is otherwise valid.
Cleanup must remove only fixture-owned resources and fail on leftovers.
The manually constructed fixture ledger is not a replacement for the existing
SQLx upgrade-lane evidence. A lost-create-response case recovers only the exact
nonce/name/labels/image it created, removes that container, and still reports
the original operation as failed. The failed `2db7e48` run passed46 filesystem
and maintenance definitions but stopped before all four native methods. Its
unprinted changed metadata field remains UNKNOWN. On `6572bff`, all three
preparation cases passed, the native positive errored and the three native
negative PASS rows did not prove their intended stages. Wave85 established55
recovery definitions:20 filesystem,28 maintenance,3 preparation and4 native.
All55 named definitions passed for899f259 in the hosted evidence above,
including actual source/restored PGDATA metadata for the four native cases.
CI wiring retains independent hard exits; unprinted nested independent
process rows remain UNKNOWN rather than being inferred from parent success.

## Bounded maintenance entry point (hosted-tested, not used on production)

[prepare-production-recovery.py](../../scripts/ops/prepare-production-recovery.py)
has four closed modes. The default `plan` makes no host observations. The other
modes require root, a protected root:root0600 JSON contract and an independently
prepared, protected `/opt/lagrange/maintenance/<tool-commit>` payload. Its twelve
Python/SQL/shell dependency files (including the imported fixed-base helper) must match the reviewed contract pins.
This staging path is necessary because the release installer changes `current`;
installing/activating the new release first would defeat a pre-upgrade backup.
The tool does not stage itself or activate a release. The old release remains
`/opt/lagrange/current` throughout audit and capture.

| Mode | Required state and result |
|---|---|
| `audit` | Relevant non-DB writers are already stopped. Read the fixed audit from the exact still-running PostgreSQL container and publish a new private audit receipt without overwriting one. Other DB client connections or prepared transactions cause refusal. |
| `capture` | Operator has cleanly stopped that same PostgreSQL generation; other writers remain stopped. Require the fresh same-boot audit, archive all five roots, encrypt, decrypt and verify a private clone, recheck source/writer identities, then publish a new set. |
| `verify` | Verify the already published recovery ID into a new isolated clone. A fresh bounded clock/boot envelope is allowed, but its original recovery bindings cannot change. This does not claim that the old live release remains current. |

The future command shape is `/usr/bin/python3
/opt/lagrange/maintenance/<tool-commit>/scripts/ops/prepare-production-recovery.py
--mode <audit|capture|verify> --contract <protected-json-path>`. These placeholders
are not an execution-ready command or host observations. G4 must bind actual
values and the separate operator stop/resumption commands first.

The closed contract binds old/tool revisions, recovery ID, database/project,
container/image/volume generations, five exact roots/owners/modes, ordered old
migration checksums, base inventory hash, protected source/config pins, writer
and systemd observations, byte/free-space limits, a secret-recovery **reference**,
boot ID, and an at-most30-minute window. Old migration SHA384 values must match
the actual installed `.up.sql` bytes, not just a declared schema count. The
base inventory hash binds bytes; it does not independently establish approval.
The `config` class retains the protected old release, including its configuration
and installed manifest. Secret values are never contract references or diagnostics.

The command holds the existing release/backup exclusive locks and existing Raw
publication shared locks. It rechecks all `lagrange-*` systemd units and every
container in the project, sharing a source mount or DB network, or using host
networking. Every selected container must be stopped with unchanged generation
and policy; new/missing/restarted writers fail. This is not a global proof that
no unmanaged host process or external client exists. G4 must inventory and fence
those sources, deny new external access, and bind the actual outage/resumption
plan. No arbitrary shell command or automatic stop/start is accepted by the tool.

Sets are private root:root0700 directories named `preupgrade-<recovery-id>` below
`/srv/backups/lagrange`, with only two0600 encrypted members, sanitized metadata
and a completion marker. The existing64-hex protected key remains outside the
archive and is held/rechecked by file identity. Atomic no-replace publication
does not alter daily backup retention/latest pointers. A failure after publish
retains the set for inspection and returns failure; the marker alone is not
operational acceptance.

Before Docker creation, the verifier writes a private nonce-bound intent. If
creation or cleanup is ambiguous, it retains the intent and copied mount and
blocks subsequent CLI calls. Operators must reconcile that exact resource under
bounded authority; there is no broad cleanup or automatic retry. Normal cleanup
and the lost-response fixture require confirmed removal of only owned resources.

[Maintenance regressions](../../scripts/ops/production-preupgrade-maintenance-self-test.py)
use real private filesystem locks, no-replace publication and OpenSSL encryption,
with fake Docker/systemd observations and a fake clone result. They cover stale
contracts/audits, migration/base mismatch, writer/source/key changes, expiry,
cipher damage, missing/extra members, failed clone, publication races and
ambiguous creation. They do not substitute for the separate native Docker suite
or an actual production fence. All28 named regressions passed in the hosted
run above; production writer fencing and recovery remain unexecuted.

## Remaining work before operating use

1. Preserve the completed e639169 actual approved-base isolated wholeflow:
   both admissions, HTTP publication, restart, capture/restore and restored
   publication passed on2026-10-19 ([acceptance](../reviews/2026-10-19-r2-e639169-host-acceptance.md)).
   Its synthetic sessions do not establish actual market receipt or a
   production recovery point. The distinct Wave109 maintenance tool candidate
   3de6420 passed required CI; prepare the fresh G4 operating packet using
   the [bounded inventory survey](../reviews/2026-10-19-r2-g4-operating-inventory-packet.md).
2. Bind the fresh actual host roots, complete writer inventory, old configuration,
   approved base, secret-recovery reference and reviewed maintenance payload.
   Do not substitute synthetic fixture data or guess paths/owners/volume modes.
3. Complete the concrete G3/G4 command packet and obtain only missing operating
   authority. Fresh host observations, actual recovery success and restoration
   compatibility must precede role bootstrap, migration or release-link change.

This tool does not perform automatic production restore, destructive migration
DOWN or service resumption. A failure preserves the old release/data and is not
permission to restart writers. The later schema62 daily-backup path remains a
separate operation. R2, actual Owner use and the three-trading-day gate stay open.


## Wave109 exact unit inventory and staged payload (historical v2 acceptance)

The Wave109 maintenance contract is `lagrange-preupgrade-maintenance-v2`. It adds
explicit unit categories and complete per-kind observations. Old v1 contracts
are refused by this tool; immutable prior tools/receipts are retained. The
archive member layout and encrypted format remain unchanged. No v2 production
capture has occurred.

Every `lagrange-*` name from the union of `list-unit-files` and `list-units`
must occur in the bounded contract (4–128 names). The tool does not filter,
reset, delete or restart historical units. Normal persistent units remain
`inactive/dead` with canonical `/etc/systemd/system/<name>` fragments. A
separately declared `retained-terminal` category permits only transient services
under `/run/systemd/transient/<name>`, in `active/exited`, `failed/failed` or
`inactive/dead`, with `Restart=no`, zero MainPID/ControlPID, no pending Job,
nonempty invocation ID, completed main-process timestamps and no activation
edges. Mandatory backup units cannot use this category. All fragment bytes
remain mandatory protected pins; drop-ins, daemon reload needs, alternate
slices/paths and missing or duplicate fields refuse.

For all units, outgoing/incoming OnSuccess/OnFailure activation is refused.
Timer/socket/path trigger links must be reciprocal, entirely inside the exact
inventory, and join stopped units. Service-specific process properties are
not incorrectly required from timers. Unit generations and inventory are read
again around read-only cgroup checks. Each default system.slice service group
is checked even when ControlGroup is empty: absent groups or recursively empty
v2 groups only, no symlinks/ACLs/unprotected paths, at most128groups/depth16,
256entries per group and4096bytes per control read. cgroup.procs and
cgroup.threads must be empty and cgroup.events must report populated0/frozen0.
Unreadability, replacement, added descendants, live tasks or expiry refuses.
This remains a bounded observed fence, not protection against arbitrary root
mutation or unmanaged writers; the operating packet must separately fence them.

The property grammar follows the [systemd Unit/Service interfaces](https://www.freedesktop.org/software/systemd/man/latest/org.freedesktop.systemd1.html).
No-job formatting is the empty `Job=` value in
[systemctl's property printer](https://github.com/systemd/systemd/blob/main/src/systemctl/systemctl-show.c);
byte-array invocation IDs are hex in the
[shared printer](https://github.com/systemd/systemd/blob/main/src/shared/bus-print-properties.c).
The recursive populated flag is specified by the
[kernel cgroup-v2 documentation](https://docs.kernel.org/admin-guide/cgroup-v2.html).
These are source references, not observations of the current host. Missing or
unsupported current-host properties block admission instead of being guessed.

Thirteen new named regressions cover isolated exact-payload subprocess import,
missing base dependency, complete retained-terminal inventory/pins, active/job/
restart refusal, persistent trigger guards, reciprocal activation, inventory
and generation drift, actual private recursive cgroup files, live tasks,
symlink/mode/missing-file refusal, bounds/expiry and replacement during reads.
All28 prior maintenance definitions remain. All41 definitions passed in
CI39195322025 attempt1 for3de6420, with full recovery at2026-10-19T04:00:50UTC
([evidence](../reviews/2026-10-19-r2-g4-maintenance-inventory-source.md#hosted-acceptance--2026-10-19-0400-utc)).
Local project execution and new production probes were not performed. The systemd observations in
these fixtures are mocks; they do not claim an actual production writer fence.


## Wave114 explicit observed graph and pinned source ancestry (validation pending)

The new `lagrange-preupgrade-maintenance-v3` contract requires `unit_graph` and a complete `postgres_ancestry` list of device/inode/uid/gid/mode identities. A PostgreSQL source below `/data/docker/volumes/` can explicitly trust its pinned1000:1000/0755 `/data` ancestor. Every other ancestor keeps the protected root-owned/noACL/no writable group-or-other policy. All descriptors remain held with no-follow traversal and exact final identity/name checks. The exact local named volume must match the source mount. Daily and restored private roots keep the old strict defaults. This expresses explicit owner trust and does not replace the operating writer fence.

`reciprocal` keeps v2 activation requirements. Explicit `quiescent-observed` preserves both actual property sets independently; every reported edge must join stopped inventoried endpoints, without jobs or hooks. The `retained-disabled-failure` category permits only an exact disabled persistent failed service with pinned fragment/generation, Restart=no, zero PIDs and empty recursive cgroups. Every reported activation source for that service must also be disabled. Mandatory backup units are excluded. No reverse property is synthesized and no history is reset, deleted, filtered or relaunched.

The actual PG parent was root:root0701, while its999:999/1777 child still requires a root:root0700 parent for cold capture. The tool does not alter permissions. A later separately approved operating ledger must provide independent recovery before protecting that exact same parent and restoring0701. All source ancestry pins used by audit and capture must reflect the protected state. Archive format, locks, fixed SQL, clone isolation, no-clobber publication and cleanup guards remain unchanged.

Fourteen new maintenance definitions join the prior41. Filesystem cases exercise actual UID/ACL/inode/path behavior in private chroots; unit observations are mocks. They are written and directly reviewed, not yet executed for this candidate. Full required hosted CI, including the existing actual Docker cold-restore tests, is required. The [survey closure and source review](../reviews/2026-10-19-r2-g4-host-compatibility-source.md) preserve the one absent release lock, current host limitations and remaining custody/writer information. No new production action is authorized.
