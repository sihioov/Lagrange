# KIS historical daily-bars Stage5 (Raw-only)

Stage5 is an isolated intermediate operation for the fixed 11 ETF symbols. It
captures the official KIS `FHKST03010100`
`/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice` response
under `provider=kis-daily-range`, then runs the Stage4A v2 session normalizer
under `provider=kis-daily-range-normalized`.

The operation is deliberately not the EOD worker path. Its dedicated
`research-range-raw` Compose service has only the KIS app-key/app-secret
runtime mounts and the Raw data mount. It has no DB password, DB dependency,
Curated mount, publication sink, healthcheck, or restart policy. It makes no
reference-price, holiday, corporate-action, candidate, account, or order
request. The result is an acquisition-time current vendor snapshot: KIS does
not provide availability, revision, or knowledge-time evidence here, so this
operation cannot claim strict historical PIT and must not backdate `available_at`.

An explicit `--existing-source-batch-id` recovery is a separate
`research-range-raw-recovery` Compose service and `range-raw-recovery` validator
scope. It mounts only the Raw tree, uses `network_mode: none`, has no Compose
secrets or KIS environment variables, and does not require runtime secret
provisioning. The Rust recovery path reads the named immutable Raw manifest and
never constructs a provider or falls through to a KIS fetch.

The approved scheduler range is the dates-only XKRX artifact and its hashed
override ledger. The 2026-06-03 election-day and 2026-07-17 Constitution-Day
corrections are source-backed calendar overrides; they are not claims that the
KIS endpoint or `exchange_calendars` supplied those closure facts.

## Gates and resume

The wrapper defaults to a local plan:

```sh
scripts/ops/kis-range-raw-backfill.sh \
  --start 2020-01-31 --end 2020-02-03 --plan
```

`--preflight` validates the isolated range-raw production configuration and Compose
expansion without building or starting anything. `--execute` requires the
explicit process-local confirmation:

```sh
sudo env LAGRANGE_CODE_COMMIT="$(git rev-parse HEAD)" \
  KIS_RANGE_RAW_CONFIRM=I_UNDERSTAND_READ_ONLY_DAILY_RANGE_KIS_CALLS \
  scripts/ops/kis-range-raw-backfill.sh \
  --start 2020-01-31 --end 2020-02-03 --execute
```

The wrapper refuses to run while the ordinary `research-worker` daemon is
running. It takes a protected root-owned lock and writes an atomic state
identity binding the date range, fixed universe, source/normalized scopes,
Stage4A v2 normalizer, code commit, and an entitlement hash. State contains no
secret or response body. The state record's source `BatchId` is durably written
before the image build or one-shot invocation; a crash can therefore resume
with the same source identity. The worker image is built with the exact
40-hex `LAGRANGE_CODE_COMMIT`, validated in the Dockerfile, and records it in
the OCI revision label and runtime environment. The worker first reconciles the immutable Raw
manifest. If an exact range/entitlement source batch already exists, it is
reused and normalized without another KIS request; a new source batch is
created only when no matching immutable evidence exists. A failed run may be
resumed with the same identity. If the state record is lost, an existing
different batch for the exact range/entitlement is a permanent conflict; the
wrapper never guesses or refetches it. A valid multi-window source may contain
several windows per symbol, but every window must use the exact daily endpoint,
TR, credentialed mode, six documented query keys, `date1=start`, a bounded
`date2`, and an empty `tr_cont`; the fixed 11-symbol set must be complete and
at least one window must end at the requested `end`. If state is lost and this
conflict is reported, the operator must restore the protected state record or
review/quarantine the existing Raw batch before retrying; do not delete evidence
or fetch a replacement batch.

For the owner-approved ten-year ETF11 capture, use the worker-preserving
wrapper from a root-owned transient service. It discovers exactly one running
`lagrange-station/research-worker` Compose container by labels, stops that
container, invokes the same Stage5 wrapper, and starts the identical container
again through its `EXIT`/signal cleanup path. It never starts a replacement
image. The exact command is recorded in the transient unit and survives the
operator terminal closing:

```sh
commit=$(git rev-parse HEAD)
release_root=$(readlink -f /opt/lagrange/current)
[[ "$release_root" =~ ^/opt/lagrange/releases/[0-9a-f]{40}$ ]] || exit 1
env_file=$release_root/deploy/compose/.env
sudo systemd-run --unit="lagrange-etf11-10y-raw-${commit%${commit#????????}}" \
  --collect --property=Type=exec --property=KillMode=control-group \
  "$(pwd)/scripts/ops/kis-range-raw-with-worker-pause.sh" \
  --start 2016-08-29 --end 2026-08-28 --commit "$commit" \
  --env-file "$env_file" \
  --state-file /var/lib/lagrange/state/range-raw/etf11-10y.tsv
```

Run the ordinary Stage5 `--plan` and root `--preflight` first. Do not edit the
build worktree while the unit is active: the exact commit and clean tracked
tree are part of the Raw identity. Inspect progress with `systemctl status`
and `journalctl -u UNIT`; neither command should print a credential or response
body. A completed Raw capture remains non-published and does not replace the
historical-price-v2 artifact.

For recovery of an already captured immutable Stage3 batch, use a separate
execute-only state identity and the explicit source ID:

```sh
sudo env LAGRANGE_CODE_COMMIT="$(git rev-parse HEAD)" \
  scripts/ops/kis-range-raw-backfill.sh \
  --start 2020-01-31 --end 2026-08-19 \
  --existing-source-batch-id 3d4f061f-8b8c-54f3-bb44-4d491b3ad256 \
  --execute
```

The wrapper binds the source ID into a V3 state record (or an explicitly
provided separate `--state-file`), verifies scope, entitlement, exact 11 ETF
symbols, daily endpoint/query/header contract, and bounded multi-window
coverage, then invokes only Raw readback/normalization. Missing, malformed, or
conflicting evidence is a permanent stop; this path never constructs a
provider, reads KIS credential values, or falls through to refetch. Completion
JSON includes `reused_existing_source=true`. Do not point this mode at the
ordinary default state file or alter the old state/evidence.

The process emits one machine-readable completion record with
`vendor_snapshot=true`, `strict_pit=false`, `ready=false`,
`publication=false`, `curated=false`, and `db=false`, together with the
source batch ID and normalized count/range. These flags are an explicit
non-publishable contract, not dataset approval.

The KIS daily endpoint is bounded manually at 100 rows per request. It uses
fixed `date1`, moves each next `date2` to the preceding response's oldest
civil date minus one day, and rejects gaps/overlaps/out-of-range rows. The
request uses original prices (`FID_ORG_ADJ_PRC=1`) and never uses `tr_cont`.
One process-owned token manager is used for the run; normally it makes one
OAuth token POST within that token's lifetime, but the contract does not claim
an absolute at-most-once issuance under expiry/retry.
Any non-empty continuation marker or continuation-like body field is a
permanent single-page contract failure; multi-page continuation is not
implemented because the raw contract does not preserve an approved cursor.

Successful Stage5 output is still not a production-ready dataset. Calendar
session completeness, listing intervals, action evidence/mapping, lineage
review, canonical publication, Curated generation, dataset approval, and
recommendation/backtest/Paper pins remain separate future gates.
