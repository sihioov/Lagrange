# R2 pre-upgrade consistent recovery preparation

Status: **source in progress; not execution-ready**. This is the pre-upgrade
part of Wave82/R2-04/R2-09, alongside the [six-file backup transition](r2-first-backup-transition.md)
and the pending approved-base whole-flow fixture. The18 filesystem/contract,
28 maintenance definitions passed on `2db7e48`; its four actual-Docker recovery
definitions were NOT_EXECUTED because class setup failed. Three additional
private-directory preparation regressions are written and unexecuted. See the
[failed candidate's full recovery](../reviews/2026-10-16-r2-first-rollout-correction-ci.md).
The corrected candidate requires fresh verification. No production stop,
capture, restore, database query or operational acceptance has occurred.

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
unprinted changed metadata field remains UNKNOWN. The successor has53 recovery
definitions, including the three new preparation regressions; fresh results
are required. CI wiring retains independent hard exits.

## Bounded maintenance entry point (written, unexecuted)

[prepare-production-recovery.py](../../scripts/ops/prepare-production-recovery.py)
has four closed modes. The default `plan` makes no host observations. The other
modes require root, a protected root:root0600 JSON contract and an independently
prepared, protected `/opt/lagrange/maintenance/<tool-commit>` payload. Its eleven
Python/SQL/shell dependency files must match the reviewed contract pins.
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
or an actual production fence. All28 are WRITTEN_UNEXECUTED.

## Remaining work before operating use

1. Complete the actual approved-base whole-flow fixture and source review, then
   verify the coherent Wave82 candidate once after the existing-run query.
   No per-file CI dispatch; all72 Wave82 definitions are still unexecuted.
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
