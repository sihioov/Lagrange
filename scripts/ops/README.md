# Production operator workflows

These scripts are repository-owned orchestration and preflight helpers. They
do not contain credentials and do not replace the operator's secret manager.

| Script | Default behavior | External action |
|---|---|---|
| `provision-linux.sh` | `--dry-run` | `--apply` creates only the approved account/directories as root |
| `validate-production-config.sh` | strict validation | no network/API call; missing values are `BLOCKED_EXTERNAL` |
| `compose-release.sh` | `--plan` | `--apply` builds/starts Compose after explicit preflight |
| `backfill-production.sh` | `--plan` | `--execute` calls only the read-only research worker after an explicit guard |
| `self-test.sh` | static/no-infrastructure tests | none |

Production execution is intentionally split into two approvals:

1. host and secret provisioning (`provision-linux.sh --apply`, then
   `deploy/secrets/provision-runtime-secrets.sh`), and
2. service/data execution (`compose-release.sh --apply` or the bounded ETF
   backfill command).

No script enables Compose `live`, asks for a KIS account/order credential, or
calls an order endpoint. KOSPI200/KOSDAQ150 candidate backfill is a separate
blocked workflow until its credentialed candidate bridge and entitlement are
available. See [`docs/runbooks/kis-production-backfill.md`](../../docs/runbooks/kis-production-backfill.md).
