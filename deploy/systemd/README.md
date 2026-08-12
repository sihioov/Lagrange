# Paper runner systemd deployment

`paper-runner.service` is the production deployment unit for the worker-wide
Paper daemon. It runs the binary built from `crates/api-server` and keeps the
curated phase-0 dataset read-only. Database credentials are supplied through a
root-owned environment file, never committed to this repository.

## Install

1. Build and install `paper-runner` at `/opt/lagrange/bin/paper-runner`.
2. Create the `lagrange` user/group and the directories
   `/opt/lagrange`, `/var/lib/lagrange/data/phase0`, and `/etc/lagrange`.
3. Copy `paper-runner.env.example` to
   `/etc/lagrange/paper-runner.env`, replace all `REPLACE_ME` values, and set
   mode `600` with owner `root:root`.
4. Copy `paper-runner.service` to `/etc/systemd/system/`, then run:

   ```sh
   systemctl daemon-reload
   systemctl enable --now paper-runner.service
   systemctl status paper-runner.service
   journalctl -u paper-runner.service -f
   ```

The service deliberately requires all four role-scoped URLs and
`LAGRANGE_DATASET_ROOT`; it will fail closed if any required setting is absent.

## Recommendation runner

`lagrange-recommendation-runner.service` runs the fixed 11-ETF recommendation
queue daemon. Create `/etc/lagrange/recommendation-runner.env` (root-owned,
mode 600) with `APP_ENV=production`, `DB_HOST`, `DB_PORT`, `DB_NAME`,
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

The QA smoke uses a labeled synthetic 11-ETF dataset only. Real production
recommendations remain blocked until licensed KRX provider implementation,
credentials, entitlement evidence, and operator provisioning are available.
