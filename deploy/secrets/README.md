# Lagrange Station — secrets skeleton.
#
# Rules (NFR-SEC-002/003, design §14.2):
#   * Real secret values NEVER appear in this repository. The files listed
#     below are gitignored; only *.example placeholders are committed.
#   * Compose mounts each secret at /run/secrets/<name> at runtime; no
#     plaintext secret ever appears in compose files, images, or logs.
#   * Generate values locally, e.g.:
#       pwsh -c "[Convert]::ToBase64String((1..32 | ForEach-Object { Get-Random -Max 256 }))"
#     or `openssl rand -base64 32`. Use >= 256 bits for secrets.
#   * TLS: place the fullchain at tls/lagrange.crt and the key at
#     tls/lagrange.key (PEM). Self-signed is acceptable for testing only.
#   * Secret recovery is a SEPARATE encrypted procedure; secrets must never
#     enter ordinary backup archives (design §13.4).
#
# Secret inventory (mapped to compose secrets in deploy/compose/compose.yml):
#
#   postgres_password        superuser / migration-owner password
#   db_app_password          non-owner application role (RLS) password
#   db_worker_password       worker role password (backtest workers)
#   db_audit_password        audit-writer role password
#   session_secret           opaque session signing/hashing key (api-server)
#   csrf_secret              CSRF synchronizer-token key (api-server)
#   auth0_client_secret      Auth0 confidential client secret (api-server)
#   krx_api_key              licensed KRX data-source credential (research-worker)
#   kis_app_key              KIS app key (live-node-owner, profile-gated)
#   kis_app_secret           KIS app secret (live-node-owner, profile-gated)
#   kis_account_ref          KIS account reference (live-node-owner)
#   backup_encryption_key    backup archive encryption key (deploy/backup/)
#   tls/lagrange.crt         TLS certificate (reverse-proxy)
#   tls/lagrange.key         TLS private key (reverse-proxy)
#
# To provision a local development set (gitignored), copy each *.example
# to its real name and fill it in, or run scripts/provision-dev-secrets.ps1
# once it lands with Todo 35.
