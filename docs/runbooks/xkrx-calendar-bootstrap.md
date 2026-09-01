# XKRX historical calendar bootstrap

The historical scheduler bootstrap is a checked-in, materialized **dates-only**
artifact built from the third-party `exchange_calendars==4.13.2` `XKRX`
calendar (reviewed upstream revision `dbe38b1`, Apache-2.0).  It is derived
input, not an official KRX/KIS authority, a weekday rule, or a KIS response:
every civil date in the requested inclusive range is present exactly once as
either a session or an explicit non-session.  Generic closure reasons are
prefixed `derived:` and are not official holiday names.

This is a separate `historical-session-dates-only` contract.  Its `sessions`
and `non_sessions` arrays contain dates and weekdays only, so it cannot be
mistaken for the canonical Rust publication/curation `calendar.json` contract
that carries session instants.  The `source_schedule` array is audit-only and
retains the exact upstream open/close and break instants (for example,
2020-12-03 is 10:00–16:30 KST); no consumer may flatten those values to a
standard 09:00–15:30 session.

The reviewed upstream inputs are pinned in `nt/pyproject.toml` and
`nt/uv.lock`:

- wheel SHA-256: `fc5a2ad0d61b5c3a6539a3061cd4cbb55c59f4a903455cec7926e4b798919996`
- source distribution SHA-256: `a9459425dd64142cd54fbc639847403c7e0c33d60fbc326c94fc1d6bd127f002`

The initial fixed-universe artifact is effective from `2016-08-29` and the
current checked-in artifact covers `2016-08-29..2026-08-28`:
`data/calendars/xkrx/calendar.json`, with its source and content manifest in
`data/calendars/xkrx/manifest.json`. The checked-in `overrides.json` is an
operator-reviewed, source-backed ledger hashed into the artifact and manifest.
It removes the 2026-06-03 national election day and 2026-07-17 Constitution
Day from the scheduler session set. The ledger cites the [KRX holiday rule](https://global.krx.co.kr/contents/GLB/06/0602/0602010201/GLB0602010201T1.jsp),
the [National Election Commission's 2026 election date](https://www.nec.go.kr/site/nec/ex/bbs/View.do?bcIdx=289351&cbIdx=1104),
and the [government notice restoring Constitution Day as a public holiday](https://m.korea.kr/news/policyNewsView.do?newsId=148959009).
These are derived operator corrections: `exchange_calendars` remains the raw
audit source and must not be described as having known those later closures.
Its `source_schedule` retains both raw upstream rows for audit, while the
dates-only session/non-session partition applies the reviewed removals.

The upstream XKRX schedule is queried only within its reviewed supported
bounds `1956-01-01..2050-12-31`; this repository's fixed universe further
requires `--start >= 2016-08-29`.

## Operator commands

All commands are local and read-only except `--apply`; none calls KIS, Docker,
an account/order endpoint, or a secret source.

```bash
# Safe default: validate the requested range and show the pinned inputs.
python scripts/ops/xkrx-calendar-bootstrap.py --plan --end 2026-08-28

# Generate or idempotently confirm the artifact. The command always enters the
# checked-in uv lockfile environment; it never uses a globally installed
# exchange_calendars package.
python scripts/ops/xkrx-calendar-bootstrap.py --apply --end 2026-08-28

# Validate only the checked-in files; this needs no Python package.
python scripts/ops/xkrx-calendar-bootstrap.py --check --end 2026-08-28
```

`--start` defaults to `2016-08-29`, cannot precede that fixed-universe
effective date, and `--end` is intentionally required.
Applying a changed existing artifact is refused unless `--replace` is given
after reviewing the new manifest.  A range extension is therefore a visible,
hash-changing artifact update rather than an in-place holiday correction.

Stage 2 wires this artifact into the bounded historical backfill wrapper as a
package-free, fail-closed session-date emitter.  The wrapper accepts only an
in-range session list after validating both artifact and manifest; weekends and
closures are omitted, and an empty selection exits without a Docker, worker, or
KIS call.  A single Rust worker process receives the exact sorted list.  Its
provider makes one allowlisted KIS `chk-holiday (CTCA0903R)` call for the first
needed session day, then validates later dates against the immutable cached
snapshot; an uncovered date makes no second call and fails closed with
`KIS_CALENDAR_SNAPSHOT_MISS`.  That live check is canonical validation, not a
historical bootstrap mechanism, and must not silently extend the materialized
artifact.  The dates-only artifact is never passed to Raw/publication/curation,
and canonical Rust
publication/curation keeps its existing typed calendar contract and evidence;
the artifact does not provide publication session times.

The old Rust `krx_2020()` fixture is not a bootstrap source.  In particular it
does not represent the third-party XKRX closures on 2020-05-01, 2020-08-17,
and 2020-12-31; regression checks keep those dates closed in the materialized
artifact.

## Validation contract

`--check` verifies the schema/contract, package/version/source hashes, reviewed
upstream revision/license/authority, fixed effective date and supported range,
sorted uniqueness, session/non-session disjointness, complete date partition,
weekend exclusion, derived/ledger closure reasons, exact source-schedule
timestamps and optional breaks, the source-backed override ledger hash, and
manifest size/content hash. It does not regenerate
the calendar, so release checks cannot be changed by a local Python dependency.
