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

The initial fixed-universe artifact is effective from `2020-01-31` and the
current checked-in artifact covers `2020-01-31..2026-08-19`:
`data/calendars/xkrx/calendar.json`, with its source and content manifest in
`data/calendars/xkrx/manifest.json`.

The upstream XKRX schedule is queried only within its reviewed supported
bounds `1956-01-01..2050-12-31`; this repository's fixed universe further
requires `--start >= 2020-01-31`.

## Operator commands

All commands are local and read-only except `--apply`; none calls KIS, Docker,
an account/order endpoint, or a secret source.

```bash
# Safe default: validate the requested range and show the pinned inputs.
python scripts/ops/xkrx-calendar-bootstrap.py --plan --end 2026-08-19

# Generate or idempotently confirm the artifact. The command always enters the
# checked-in uv lockfile environment; it never uses a globally installed
# exchange_calendars package.
python scripts/ops/xkrx-calendar-bootstrap.py --apply --end 2026-08-19

# Validate only the checked-in files; this needs no Python package.
python scripts/ops/xkrx-calendar-bootstrap.py --check --end 2026-08-19
```

`--start` defaults to `2020-01-31`, cannot precede that fixed-universe
effective date, and `--end` is intentionally required.
Applying a changed existing artifact is refused unless `--replace` is given
after reviewing the new manifest.  A range extension is therefore a visible,
hash-changing artifact update rather than an in-place holiday correction.

Stage 1 intentionally does not wire this artifact into the Rust worker or
backfill.  The existing historical worker therefore remains on its current
KIS path until a separately reviewed Stage 2 integration adds a typed
dates-only scheduler loader.  When that integration is made, it must use this
artifact only for an in-range session/non-session decision, fail closed
outside the manifest range, and never pass it to publication or curation.
Canonical Rust publication/curation keeps its existing typed calendar contract
and evidence; this artifact does not provide publication session times.  A
current daily/live run remains allowed to validate its target through the
allowlisted KIS `chk-holiday (CTCA0903R)` endpoint; that live check is not a
historical bootstrap mechanism and must not silently extend this artifact.

The old Rust `krx_2020()` fixture is not a bootstrap source.  In particular it
does not represent the third-party XKRX closures on 2020-05-01, 2020-08-17,
and 2020-12-31; regression checks keep those dates closed in the materialized
artifact.

## Validation contract

`--check` verifies the schema/contract, package/version/source hashes, reviewed
upstream revision/license/authority, fixed effective date and supported range,
sorted uniqueness, session/non-session disjointness, complete date partition,
weekend exclusion, derived closure reasons, exact source-schedule timestamps
and optional breaks, and manifest size/content hash.  It does not regenerate
the calendar, so release checks cannot be changed by a local Python dependency.
