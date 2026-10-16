# R2 pre-upgrade consistent recovery preparation

Status: **source in progress; not execution-ready**. This is the pre-upgrade
part of Wave82/R2-04/R2-09, alongside the [six-file backup transition](r2-first-backup-transition.md)
and the pending approved-base whole-flow fixture. The18 filesystem/contract and
three actual-Docker fixture definitions are WRITTEN_UNEXECUTED. No production
stop, capture, restore, database query or source acceptance has occurred.

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
SQLx upgrade-lane evidence. All21 new cases remain unexecuted. CI wiring is
written with independent hard exits; no candidate has been dispatched.

## Remaining work before a usable operating command

1. Implement the bounded maintenance entry point: fresh protected contract,
   exact old/tool revisions, image/volume generations, boot/time bounds, release
   and backup locks, complete writer enumeration/fences, pre-stop audit binding,
   cold capture, encryption, isolated restore, and no-clobber publication.
2. Bind the old release's actual migration checksums and actual Raw/Curated/base
   identities, protected configuration and separate secret-recovery references.
   Do not invent host paths or treat the synthetic base in these tests as the
   approved real base. Keep the decryption key outside the archive.
3. Add entry-point interruption, expiry and publication failure cases; finish
   the approved-base whole-flow fixture. Review and verify this coherent
   candidate once, following the existing-run check. No per-file CI dispatch.
4. Complete the concrete G3/G4 packet and obtain only missing operating
   authority. Fresh host observations, actual recovery success and restoration
   compatibility must precede role bootstrap, migration or release-link change.

This tool does not perform automatic production restore, destructive migration
DOWN or service resumption. A failure preserves the old release/data and is not
permission to restart writers. The later schema62 daily-backup path remains a
separate operation. R2, actual Owner use and the three-trading-day gate stay open.
