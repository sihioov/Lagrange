# SUPERSEDED by scripts/backup/create.ps1 (and its .sh twin) in Todo 33.
#
# This placeholder said "implemented in Todo 35"; that was wrong — the plan
# assigns the Raw/Curated/Artifact incremental file backup to Todo 33, which is
# where it landed.
#
# create.* writes one `.increment` per file class into the backup set, hashes
# each with sha256, and records path + hash + size in the manifest. The restore
# side re-hashes every declared file and refuses to verify on any mismatch
# (runbook assertions A3/P5).
#
# A note on classes: the plan prose mentions "Raw/Curated/Feature/Catalog/
# Artifact", but the approved policy and manifest schema
# (deploy/backup/policy/) define exactly five classes with no feature or
# catalog member. The implementation follows the policy — extra classes would
# be schema-rejected at the gate. See decisions.md 2026-08-08.
#
# Run it:
#   scripts/backup/create.ps1 -Out <dir> -Key <passphrase>
#   bash scripts/backup/create.sh --out <dir> --key <passphrase>
