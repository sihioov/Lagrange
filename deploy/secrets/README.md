# Lagrange Station — secrets skeleton.
#
# Rules (NFR-SEC-002/003, design §14.2):
#   * Real secret values NEVER appear in this repository. The files listed
#     below are gitignored; only *.example placeholders are committed.
#   * Compose mounts each secret at /run/secrets/<name> at runtime; no
#     plaintext secret ever appears in compose files, images, or logs.
#   * Generate values locally, e.g.:
#       pwsh -c "[Convert]::ToBase64String((1..32 | ForEach-Object { Get-Random -Max 256 }))"
#     or `openssl rand -base64 32 | tr -d '\r\n'`. Use >= 256 bits for secrets. The cursor
#     signing key has an explicit 32-byte minimum/fixed width; see below.
#   * TLS: place the fullchain at tls/lagrange.crt and the key at
#     tls/lagrange.key (PEM). Self-signed is acceptable for testing only.
#   * Secret recovery is a SEPARATE encrypted procedure; secrets must never
#     enter ordinary backup archives (design §13.4).
#
# Secret inventory (mapped to compose secrets in deploy/compose/compose.yml):
#
#   postgres_password        PostgreSQL cluster administrator/bootstrap password
#   db_migration_owner_password
#                            separate migration_owner role password
#   db_app_password          non-owner application role (RLS) password
#   db_worker_password       worker role password (candidate/recommendation/backtest workers)
#   db_research_password     research_writer role password (research-worker)
#   db_audit_password        audit-writer role password
#   db_admin_password        admin read-only worker/ops role password
#   cursor_secret             API cursor-signing key (exactly 32 random bytes)
#   session_secret           opaque session signing/hashing key (api-server)
#   csrf_secret              CSRF synchronizer-token key (api-server)
#   auth0_client_secret      Auth0 confidential client secret (api-server)
#   kis_app_key              KIS app key (research-worker and live-node-owner)
#   kis_app_secret           KIS app secret (research-worker and live-node-owner)
#   kis_account_ref          KIS account reference (live-node-owner)
#   backup_encryption_key    backup archive encryption key (deploy/backup/)
#   tls/lagrange.crt         TLS certificate (reverse-proxy)
#   tls/lagrange.key         TLS private key (reverse-proxy)
#
# To provision a local development set (gitignored), copy each *.example to
# its real name and fill it in. For native-Linux Compose, supply the source
# values through the operator's secret manager and then run
# `deploy/secrets/provision-runtime-secrets.sh` as root to create the
# service-specific UID/mode copies. The provisioner copies and validates
# supplied values; it deliberately does not generate production credentials.

## Database role credentials and cursor key

The PostgreSQL administrator credential and the migration-owner credential are
intentionally separate. Store the administrator password in
`postgres_password` only for the bootstrap connection. Store the distinct
`migration_owner` role password in `db_migration_owner_password`; Compose
mounts it separately for role bootstrap and the migration one-shot. Never
reuse `postgres_password` for `db_migration_owner_password`, and never put
either value in `deploy/compose/.env` or a command argument.

Provision both files outside Git by copying their `.example` files, replacing
the placeholders with independently generated values, and restricting each
real file to the operator account:

```sh
umask 077
openssl rand -base64 32 | tr -d '\r\n' > deploy/secrets/postgres_password
openssl rand -base64 32 | tr -d '\r\n' > deploy/secrets/db_migration_owner_password
chmod 600 deploy/secrets/postgres_password deploy/secrets/db_migration_owner_password
```

The API cursor key must contain exactly 32 random bytes (the minimum and fixed
width) on one logical line. Hex is a convenient unambiguous representation:

```sh
umask 077
openssl rand -hex 32 | tr -d '\r\n' > deploy/secrets/cursor_secret
chmod 600 deploy/secrets/cursor_secret
```

The API accepts a 32-byte printable value and base64 or hex encodings that
decode to exactly 32 bytes. Secret files should use base64 or hex so they stay
portable UTF-8 one-line files. Keep `cursor_secret` independent from database,
session, and CSRF secrets; changing it invalidates existing API cursors.

## Auth0 confidential client

Configure Auth0 as a first-party Regular Web Application using Client Secret
Post, Authorization Code, PKCE S256, and RS256 ID tokens. The exact Auth0
Client Secret must be the sole line of `auth0_client_secret`; never copy it
into an environment file or command argument. Compose mounts the secret
read-only, and the API server reads it through
`AUTH0_CLIENT_SECRET_FILE=/run/secrets/auth0_client_secret`.

Put the non-secret tenant domain and Client ID in `deploy/compose/.env`. The
current operator-selected tenant is in the Auth0 Japan region. PAR, JAR,
refresh tokens, and additional credentials are outside this deployment
contract.

After creating the file on Windows, disable inheritance and restrict its ACL
to the current operator, SYSTEM, and BUILTIN Administrators. This workflow uses
SIDs so localized account names do not change the result, and never passes the
credential itself as an argument:

```powershell
$secretPath = (Resolve-Path -LiteralPath 'deploy/secrets/auth0_client_secret').Path
$operatorSid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$allowedSids = @($operatorSid, 'S-1-5-18', 'S-1-5-32-544')

& icacls.exe $secretPath /inheritance:r
if ($LASTEXITCODE -ne 0) {
    throw 'icacls failed while disabling secret ACL inheritance'
}
foreach ($rule in (Get-Acl -LiteralPath $secretPath).Access) {
    $sid = $rule.IdentityReference.Translate(
        [Security.Principal.SecurityIdentifier]
    ).Value
    if ($sid -notin $allowedSids) {
        & icacls.exe $secretPath /remove:g "*$sid"
        if ($LASTEXITCODE -ne 0) {
            throw 'icacls failed while removing an allow ACL entry'
        }
        & icacls.exe $secretPath /remove:d "*$sid"
        if ($LASTEXITCODE -ne 0) {
            throw 'icacls failed while removing a deny ACL entry'
        }
    }
}
& icacls.exe $secretPath /grant:r "*${operatorSid}:(F)" '*S-1-5-18:(F)' '*S-1-5-32-544:(F)'
if ($LASTEXITCODE -ne 0) {
    throw 'icacls failed while granting the required secret ACL entries'
}

$acl = Get-Acl -LiteralPath $secretPath
$rules = @($acl.Access)
$actualSids = @($rules | ForEach-Object {
    $_.IdentityReference.Translate(
        [Security.Principal.SecurityIdentifier]
    ).Value
} | Sort-Object -Unique)
$sidDifference = @(Compare-Object ($allowedSids | Sort-Object) $actualSids)
$rulesAreExact = $rules.Count -eq 3 -and -not ($rules | Where-Object {
    $_.IsInherited -or
    $_.AccessControlType -ne [Security.AccessControl.AccessControlType]::Allow -or
    $_.FileSystemRights -ne [Security.AccessControl.FileSystemRights]::FullControl
})
if (-not $acl.AreAccessRulesProtected -or $sidDifference.Count -ne 0 -or -not $rulesAreExact) {
    throw 'secret ACL does not match the required protected three-principal policy'
}
```

On Unix hosts, restrict the file to its operator account:

```sh
chmod 600 deploy/secrets/auth0_client_secret
```

## Research worker database credential

Provision `db_research_password` outside Git by copying
`db_research_password.example` to `db_research_password`, replacing the
placeholder with a randomly generated password, and restricting the file to
the operator account. Then use an interactive administrator `psql` session to
create the role when absent, ensure it can log in, and set its password:

```psql
SELECT 'CREATE ROLE research_writer LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE'
WHERE NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'research_writer')
\gexec
ALTER ROLE research_writer LOGIN;
\password research_writer
```

Enter the exact same password stored in `db_research_password` at the prompt.
This avoids putting the credential in shell history, process arguments,
Compose configuration, or logs. Never add the real file to Git; the directory
`.gitignore` intentionally preserves only `*.example` files.

The Compose service runs at 16:30 Asia/Seoul by default and considers the
latest EOD publication healthy for four days. Override
`RESEARCH_RUN_AT_KST` or `RESEARCH_MAX_PUBLICATION_AGE_SECS` operationally when
needed. `EOD_UNAVAILABLE` means the provider has not published the requested
trading day's EOD data yet; retry it after the provider's publication window.

## Research worker deployment order

Apply all SQL migrations with the migration-owner/admin procedure before
starting `research-worker`. Concurrent migrations require a finite session
lock timeout supplied outside the migration file:

```sh
PGOPTIONS='-c lock_timeout=5s' sqlx migrate run
```

Compose first runs `db-migrate` to apply every checked-in SQLx migration
through `0045`, then runs `research-schema-check` as a fail-closed one-shot.
The worker is not launched unless that gate finds successful migrations
`22–25`, `33–35`, `42`, and `45` (the research schema versions it checks), exact normalized
PK/unique/CHECK definitions, the required publication column
type/nullability/identity/default contract, exact valid/ready indexes, RLS
policies, append-only enforcement, and the exact `research_writer` role/grant
contract.
After applying or repairing migrations, restart with:

```sh
docker compose -f deploy/compose/compose.yml up -d research-worker
```

The Raw bind tree must be readable by UID/GID `10001:10001`. Immutable evidence
and `batch.json` are owner-read-only (`0440`), while directories (`0750`) plus
`manifest.jsonl` and `commit.lock` (`0640`) remain writable. Unix recovery opens
immutable files read-only when it re-establishes their durability. Compose
enforces these modes recursively through the isolated `research-raw-init`
one-shot. The initializer drops all Linux capabilities and adds back only
`CHOWN`, `FOWNER`, and `DAC_OVERRIDE`; it has no network or secrets and does not
follow symlinks or cross filesystems. For manual pre-provisioning on Linux,
create the deployment data path before Compose:

```sh
install -d -m 0750 /srv/lagrange/data/raw
chown 10001:10001 /srv/lagrange/data/raw
chmod 0750 /srv/lagrange/data/raw
```

If a host-side collector writes nested Raw content after Compose has transferred
ownership, rerun `research-raw-init` or use an operator-approved shared group/
ACL workflow. Do not recursively follow symlinks while repairing ownership.

Set `LAGRANGE_DATA_DIR=/srv/lagrange/data` (or the appropriate parent) before
starting Compose. Compose binds its `raw` child at `/data/raw` and sets
`RESEARCH_RAW_ROOT=/data`; `RawStore` appends `/raw`, so evidence appears at the
host's `/srv/lagrange/data/raw/provider=...`, never under `raw/raw`. The
long-running worker remains unprivileged and never needs root.

## Native-Linux Compose delivery

Compose uses service-specific copies because file-backed Compose secrets are
bind mounts on native Linux; a Compose `mode` field alone does not reliably
change the host file's ownership. After creating the operator source files,
run the provisioning helper as root:

```sh
sudo deploy/secrets/provision-runtime-secrets.sh
```

It fails closed if a source is missing, symlinked, or (for credential files)
contains CR/LF. It writes only `0440` copies owned by the consuming UID,
including an independent path for `candidate-runner`:
`10001:10001` for API/workers, `999:999` for PostgreSQL/schema checks and the
non-root bootstrap/migration one-shots, and `101:101` for the unprivileged
Nginx image. The bootstrap/migration copies are stricter `0400` files owned by
`999:999`, matching `USER 999:999` in `deploy/db/Dockerfile`; changing either
one-shot to root would violate the deployment contract. Compose refuses to
start until these copies exist under `LAGRANGE_RUNTIME_SECRET_DIR` (default
`deploy/secrets/runtime`). The runtime directory is gitignored and must be
re-provisioned after rotating a source secret.
