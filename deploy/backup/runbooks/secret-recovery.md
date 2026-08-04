# Secret Recovery — SEPARATE PROCEDURE (never via ordinary backup archives)

**Status:** CONTRACT SKELETON. Secrets (KIS app key/secret, Auth0 client secret, DB master
key, any credential) follow this dedicated procedure. They are NEVER written into ordinary
backup archives (db_base, db_wal, file_raw, file_curated, file_artifact). This is a hard
policy rule (System Design §13.4 / §14.2; NFR-SEC-002/003).

---

## 1. Where secrets live

- Runtime injection only: Docker Secrets or an external Secret Store (NFR-SEC-003).
- The database stores only **secret references** (and minimal encrypted metadata), never
  plaintext secret values (design §14.2). Ordinary archives therefore contain references at
  most — never recoverable plaintext.
- Recovery of a secret is a **reference rotation/re-issue**, not an archive restore.

## 2. What the policy gate enforces (machine-checkable, runs NOW)

`scripts/backup/validate-policy.ps1 -SetPath <set>` (any gate) rejects a set when:

- any archive file path matches a forbidden segment (e.g. `secrets/`, `credentials/`, `.env`,
  `kis-credentials`, `id_rsa`, `id_ed25519`), or
- any archive file content contains a `secret_markers` entry (e.g. `kis_app_secret`,
  `LAGRANGE_SECRET_MARKER`, `BEGIN OPENSSH PRIVATE KEY`, ...), or
- the manifest itself contains a marker.

**Secret rule:** a rejected archive is treated as **potentially compromised** — do NOT use it
for any restore. Rotate the exposed secret, quarantine the archive, and record an incident.
The rejection transcript (naming the marker and file) is the evidence.

## 3. Secret-recovery procedure (reference-based, separate from archives)

1. Identify the affected secret reference (DB secret-reference columns / Secret Store key).
2. Re-issue or rotate the secret at the provider (KIS/Auth0) or generate a new key.
3. Inject the new value via Docker Secret / Secret Store at runtime.
4. Restart the consuming service and verify via its healthcheck.

## 4. Exact success assertions (ALL must be true)

| # | Assertion | Machine check |
|---|-----------|---------------|
| C1 | No secret value written to disk or archive during recovery | post-recovery `validate-policy` re-run on the backup set → exit `0` (0 markers) |
| C2 | Recovered secret injected without plaintext persistence | container env/secret inspection shows only the injected reference; no secret in repo/archive/log scan |
| C3 | Consuming service healthy with the new secret | service healthcheck exits `0` |
| C4 | Incident recorded if an archive contained a marker | archive quarantined + audit/incident row exists |

## 5. Evidence to record

- The `validate-policy` rejection transcript (naming the exact marker + path) and the
  post-recovery clean re-run (C1), healthcheck output (C3), quarantine/incident reference (C4).
