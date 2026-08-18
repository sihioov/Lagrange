# Paper runner systemd deployment

`paper-runner.service` is the production deployment unit for the worker-wide
Paper daemon. It launches the secret-file adapter from
`deploy/runtime/paper-runner-entrypoint`, which builds role-scoped PostgreSQL
URLs in memory and then `exec`s the Rust binary. The systemd environment file
contains no database URLs or passwords; credentials are regular files supplied
by the host secret manager.

## Tailscale TLS renewal

`lagrange-tailscale-tls-renewal.service` is a root-only, TLS-only renewal
workflow for the fixed Tailscale name
`l1nnx-sh.taild74a33.ts.net`. The helper defaults to a no-change plan; only
the explicit `--renew` mode can call `tailscale cert`, and it writes the
certificate and key to a private staging directory first. It validates the
exact SAN, public-key match, at least 30 days remaining, and the source
`root:root 0600` / reverse-proxy runtime `101:101 0440` contracts. It never
touches DB, Auth0, KIS, or application credentials.

Copy `tailscale-tls-renewal.conf.example` to a protected, root-owned `0600`
configuration outside the checkout and customize it before installation. In
particular, replace the commit placeholder with the exact 40-character
lowercase `LAGRANGE_CODE_COMMIT` for the deployed Compose checkout; the
installer rejects placeholders and all-zero values. Set `COMPOSE_FILE` and
`COMPOSE_ENV_FILE` to the approved immutable checkout (the production default
expects the checkout under `/opt/lagrange/deploy`), and make both regular,
non-symlink files owned by `root:root` with no group/other write bits. The
parser accepts only the documented absolute paths, rejects `..` and symlink
ancestors, and requires the fixed domain. Do not run the installation sequence
until the approved `/opt/lagrange/deploy` checkout exists, its Compose file/env
paths are regular protected files, and the source/runtime TLS pair has been
provisioned. The pending configuration is deliberately outside the install
target:

```sh
sudo install -o root -g root -m 0600 \
  deploy/systemd/tailscale-tls-renewal.conf.example \
  /etc/lagrange/tailscale-tls-renewal.conf.pending
sudoedit /etc/lagrange/tailscale-tls-renewal.conf.pending
sudo scripts/ops/renew-tailscale-tls.sh --check \
  --config-file /etc/lagrange/tailscale-tls-renewal.conf.pending
scripts/ops/install-tailscale-tls-renewal.sh --dry-run \
  --config-source /etc/lagrange/tailscale-tls-renewal.conf.pending
```

After the pending config check and dry-run pass, apply copies the artifacts and
pending config to the fixed install locations. It refuses to overwrite an
existing protected target config and does not issue a certificate:

```sh
sudo scripts/ops/install-tailscale-tls-renewal.sh --apply \
  --config-source /etc/lagrange/tailscale-tls-renewal.conf.pending
sudo scripts/ops/install-tailscale-tls-renewal.sh --check \
  --config-source /etc/lagrange/tailscale-tls-renewal.conf.pending
sudo /opt/lagrange/bin/renew-tailscale-tls.sh --check
```

`--apply` refuses to overwrite an existing protected config and only enables
the timer; it does not start the timer or issue a certificate. After checking
the installed artifacts and the current TLS pair, activation is a separate,
explicit operator action:

```sh
sudo systemctl start lagrange-tailscale-tls-renewal.timer
```

Because the timer is persistent, this explicit start may run a missed
invocation; perform it only after the source/runtime pair and Compose
configuration have passed their checks.

The timer then runs at 03:15 KST with a one-hour randomized delay and
`Persistent=true`. Renewal takes an `flock` single-run lock. If Compose reports
the `reverse-proxy` service as running, only that service is force-recreated
with `--no-deps`; if it is absent, no service is started. A failed issuance
leaves the existing pair untouched. A failed proxy refresh leaves an already
validated source/runtime pair converged and reports the retryable error.

The installer and renewal helper are repository artifacts only until the
operator explicitly runs `--apply`; no certificate, Docker, or systemd command
is run by repository tests. See the official [`tailscale cert` CLI reference](https://tailscale.com/docs/reference/tailscale-cli?tab=macos)
and [Tailscale HTTPS certificate guidance](https://tailscale.com/docs/how-to/set-up-https-certificates)
for the vendor-side issuance semantics.

## Install

1. Build the Rust binary from `crates/api-server` and install it at
   `/usr/local/bin/paper-runner-bin`.
2. Install `deploy/runtime/paper-runner-entrypoint` as
   `/opt/lagrange/bin/paper-runner` and set mode `0755`. Install the wrapper's
   runtime dependencies (`python3` and `psql`) on the host.
3. Run `scripts/ops/provision-linux.sh --apply` as root to create the
   `lagrange` user/group, create the canonical `lagrange-data` group at GID
   `10001` when it is unused, add `lagrange` to that group's supplementary
   membership, and create the directories `/opt/lagrange`,
   `/var/lib/lagrange/data/phase0`, `/etc/lagrange/secrets`, and
   `/etc/lagrange`. A pre-existing `lagrange-data` group is accepted only at
   the exact GID with no unrelated explicit members or primary accounts; a
   foreign group already using GID `10001` is a hard conflict. The
   recommendation unit uses `SupplementaryGroups=10001` to read the
   container-owned curated/catalog directories; those paths remain read-only
   to the service.
4. Provision regular, non-symlink role password files in
   `/etc/lagrange/secrets/` (`db_app_password`, `db_worker_password`,
   `db_admin_password`, and `db_audit_password`). They must be readable by
   `lagrange` but not by untrusted users; the wrapper rejects missing,
   symlinked, empty, or multiline files.
5. Copy `paper-runner.env.example` to
   `/etc/lagrange/paper-runner.env`, set mode `600` with owner `root:root`,
   and adjust only non-secret component/path values.
6. Copy `paper-runner.service` to `/etc/systemd/system/`, then run:

   ```sh
   systemctl daemon-reload
   systemctl enable --now paper-runner.service
   systemctl status paper-runner.service
   journalctl -u paper-runner.service -f
   ```

The service's `ExecStartPre` runs the wrapper's `healthcheck --startup` before
the daemon: all four role-scoped database connections and the immutable
curated dataset contract must be ready. Runtime/container healthchecks omit
`--startup`; they additionally require the daemon's non-secret progress state
file to show a live PID, fresh heartbeat, fresh loop progress, and (when a
cycle is active) a cycle deadline that has not expired. A cycle is explicitly
bounded, so a legitimately slow but progressing cycle is not falsely marked
unhealthy; a stale heartbeat or overdue cycle fails closed. Readiness requires
a version-2 JSON manifest with a positive bar count plus a non-empty
`curated/bars/market=kr/**/version=2/` `bars.parquet` partition; an empty data
directory therefore fails closed. It also fails closed if any setting or
secret file is absent. The service never sets `DATABASE_URL`,
`WORKER_DATABASE_URL`, `ADMIN_DATABASE_URL`, or `AUDIT_DATABASE_URL` directly.

The unit is `Type=notify` with `WatchdogSec=30s`. After the initial health
state writer is running, the daemon sends `READY=1`; its health task sends
`WATCHDOG=1` only while the heartbeat and bounded cycle-progress state remain
live. `NOTIFY_SOCKET` may be the usual filesystem pathname or a Linux abstract
socket written with systemd's leading-`@` convention; both are decoded as
Unix datagram addresses. A wedged Tokio process therefore stops notifying
systemd and is automatically restarted. A cycle that runs past its recorded
deadline also stops watchdog notifications even if the process is still
accepting timer ticks, so this is runtime progress supervision rather than a
startup-only probe.

Paper stages use a 15-second application/statement deadline, a 5-second local
lock deadline, and a 90-second whole-cycle deadline by default. SIGTERM is
observed while a cycle is in flight; cancellation leaves unfinished targets
pending and the process exits within the configured 20-second
`PAPER_SHUTDOWN_GRACE_MS` budget (the unit's `TimeoutStopSec=30s` leaves
systemd margin to deliver and observe that bounded shutdown).

Settlement notifications are durable database obligations.  A terminal
`pending_targets` row and its Paper settlement outbox row commit together;
the migration backfills legacy terminal rows and refuses a rollback while an
undelivered obligation remains.  The runner reports
`outbox_backlog`, `outbox_oldest_age_secs`, `outbox_failed`,
`outbox_exhausted`, and `outbox_ready` in its journal line, while the
`paper_settlement_*` Prometheus metrics expose the same readiness signal.
Delivered rows are retained in the database archive before bounded pruning.
An exhausted retry budget or an over-age backlog makes the worker healthcheck
fail closed; investigate the recorded `last_error` before retrying or pruning.

## Recommendation runner

`lagrange-recommendation-runner.service` runs the fixed 11-ETF recommendation
queue daemon. Copy `recommendation-runner.env.example` to
`/etc/lagrange/recommendation-runner.env` (root-owned, mode 600), then fill
`APP_ENV=production`, `DB_HOST`, `DB_PORT`, `DB_NAME`,
`DB_USER=worker`, and `DB_PASSWORD_FILE` pointing to the worker password
secret, plus the five immutable
`RECOMMENDATION_DATASET_*` pin values, and
`RECOMMENDATION_HEALTH_STATE_PATH=/run/lagrange-recommendation-runner/health.json`.
Install the immutable universe manifest at
`/etc/lagrange/universes/kr-etf-core-v1.yaml` and mount curated data at
`/var/lib/lagrange/data/curated` read-only.

The daemon attempts its schedule at 16:30 KST and performs one startup
catch-up for the latest eligible close. Only active Paper bindings explicitly
opted in with `auto_apply_recommendations` are scheduled; manual runs remain
separate. `recommendation-runner healthcheck` reports process heartbeat,
read-only DB reachability, last schedule attempt (including empty/blocked
cycles), oldest active queue age, and current blocked-run count. The runtime
state file is non-secret and intentionally resets on service restart.

The QA smoke uses a labeled synthetic 11-ETF dataset only. The production
read-only path uses the KIS provider and remains blocked until real KIS
credentials, entitlement evidence, an approved immutable dataset pin, and
operator provisioning are available. KIS account/order credentials are not
required for this EOD path; the Live profile remains disabled.
