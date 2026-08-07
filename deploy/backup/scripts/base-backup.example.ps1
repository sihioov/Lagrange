# SUPERSEDED by scripts/backup/create.ps1 (and its .sh twin) in Todo 33.
#
# This placeholder said "implemented in Todo 35"; that was wrong — the plan
# assigns PITR and file-backup automation to Todo 33, which is where it landed.
#
# The real implementation does what this stub described and more:
#   * pg_basebackup -Ft -z -X none against the pinned postgres 18.4 image, so
#     WAL comes from the archive and a gap in it is detectable rather than
#     papered over by WAL bundled into the base;
#   * archive_command WAL shipping (deploy/backup/compose/drill.compose.yml);
#   * AES-256-CBC + PBKDF2 encryption of db_base/db_wal, which the policy marks
#     encryption='required';
#   * a manifest that scripts/backup/validate-policy.* must accept before
#     create.* will report success.
#
# Run it:
#   scripts/backup/create.ps1 -Out <dir> -Key <passphrase> [-Metrics <file.prom>]
#   bash scripts/backup/create.sh --out <dir> --key <passphrase>
#
# Restore and prove it:
#   scripts/backup/restore-and-verify.ps1 -SetPath <dir>/set -Sidecar <dir>/backup-sidecar.json
#
# Runbooks: deploy/backup/runbooks/pitr-point-in-time-recovery.md
#           deploy/backup/runbooks/pre-member-restore-drill.md
