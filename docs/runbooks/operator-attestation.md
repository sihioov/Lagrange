# Operator attestation runbook

This runbook is the fail-closed handoff between an operator's external KIS
data-rights evidence and a release dataset.  API credentials or a successful
read-only response do not prove a data-use entitlement.  The operator must
provide the real rights document and its metadata; this repository must never
invent, upload, or commit that document.

The helper stores only a SHA-256, a reference, coverage, dates, and the owner
UUID.  It never stores or prints the document body.  Registration and
activation are separate operations:

```text
scripts/ops/provision-entitlement.sh register --plan \
  --metadata-file /etc/lagrange/universes/kis.entitlement.json \
  --document-file /operator-controlled/path/rights.pdf \
  --managed-by OWNER-USER-UUID

sudo scripts/ops/provision-entitlement.sh register --apply \
  --metadata-file /etc/lagrange/universes/kis.entitlement.json \
  --document-file /operator-controlled/path/rights.pdf \
  --managed-by OWNER-USER-UUID \
  --confirm I_UNDERSTAND_REGISTER_PENDING_ENTITLEMENT

sudo scripts/ops/provision-entitlement.sh activate --check \
  --entitlement-id DB-UUID --managed-by OWNER-USER-UUID \
  --activation-date YYYY-MM-DD

sudo scripts/ops/provision-entitlement.sh activate --apply \
  --entitlement-id DB-UUID --managed-by OWNER-USER-UUID \
  --activation-date YYYY-MM-DD \
  --confirm I_UNDERSTAND_ACTIVATE_ENTITLEMENT
```

Use `--env-file deploy/compose/.env` when the Compose environment is not the
default.  `--plan` is local-only.  `--check` performs a read-only database
check.  `--apply` is root-only, requires its exact confirmation string, and
uses the migration-owner `psql` inside the `db-migrate` Compose image on the
private network.  PostgreSQL is intentionally not published on the host;
the password is a mounted Docker secret and is not placed in host argv,
host environment, SQL text, or logs.  The short-lived container may expose
it only to its own libpq process through its internal password channel.  Do
not replace this with host `psql`, host `PGPASSWORD`, or a password argument.

After an ACTIVE entitlement exists and the curated output has been reviewed,
attest the immutable dataset:

```text
scripts/ops/register-dataset-version.sh --plan \
  --manifest-file /var/lib/lagrange/data/curated/datasets/krx_eod_bars/version=1/manifest.json \
  --dataset-id krx_eod_bars --dataset-version kis-YYYYMMDD.1 \
  --storage-path /data/curated --entitlement-id DB-UUID \
  --as-of-date YYYY-MM-DD --env-file deploy/compose/.env

sudo scripts/ops/register-dataset-version.sh --check \
  --manifest-file /var/lib/lagrange/data/curated/datasets/krx_eod_bars/version=1/manifest.json \
  --dataset-id krx_eod_bars --dataset-version kis-YYYYMMDD.1 \
  --storage-path /data/curated --entitlement-id DB-UUID \
  --as-of-date YYYY-MM-DD --env-file deploy/compose/.env

sudo scripts/ops/register-dataset-version.sh --apply \
  --manifest-file /var/lib/lagrange/data/curated/datasets/krx_eod_bars/version=1/manifest.json \
  --dataset-id krx_eod_bars --dataset-version kis-YYYYMMDD.1 \
  --storage-path /data/curated --entitlement-id DB-UUID \
  --as-of-date YYYY-MM-DD --env-file deploy/compose/.env \
  --write-env-file /etc/lagrange/compose.env.pending \
  --confirm I_UNDERSTAND_REGISTER_READY_DATASET \
  --confirm-write I_UNDERSTAND_WRITE_RELEASE_PINS
```

The dataset helper verifies the manifest self-hash, exact artifact list,
artifact byte sizes and SHA-256 values, Parquet header/footer magic, and the
DB lineage/ACTIVE entitlement before a `READY` row can be registered.  For
`krx_eod_bars` it additionally requires all 11 fixed ETF symbols and the
`bars.parquet`, `adjusted_bars.parquet`, and `total_return_bars.parquet`
partition for every year in the generation.  A broad `version=` glob is not a
valid attestation.  A manifest without the exact `artifacts` hash inventory
is blocked; it must not be upgraded to READY by operator assertion.

Only after a successful explicit apply should the emitted six pins be copied
to the release environment (or use `--pin-file`/`--write-env-file` with their
separate confirmation).  Recommendation, backtest, and Paper must consume
the same emitted dataset version and manifest hash.  Candidate universes
require separate point-in-time source evidence and are not covered by an ETF
EOD entitlement.  Account, balance, order, amend, cancel, WebSocket, and live
profiles remain outside this read-only path.
