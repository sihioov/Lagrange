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
