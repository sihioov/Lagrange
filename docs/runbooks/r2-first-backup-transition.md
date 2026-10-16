# R2 first backup installation transition

Status: source written, **not execution-ready**. The 22 real-root filesystem
fixtures are WRITTEN_UNEXECUTED. Systemd responses are substituted in those
fixtures; actual installed units require a fresh operational check. This is one
part of [first-rollout preparation](../reviews/2026-10-16-r2-first-rollout-readiness.md),
not permission to perform a production change.

The [transition tool](../../scripts/ops/transition-production-backup.py) upgrades
only the existing helper, protected configuration and four backup service/timer
files. The [fresh installer](../../scripts/ops/install-production-backup.sh)
retains its refusal to overwrite existing targets. No existing backup set is
modified or removed by this transition.

## Prerequisites and ordering

Before the first release/DB change, complete the separate consistent pre-upgrade
recovery proof. The daily schema62 backup path cannot replace that prerequisite.
The target immutable release must then be installed and selected, with its
required daily files and reader floor prepared. The official backup `--check`
must succeed against the new protected configuration. This does not start daily
processing or count as an actual backup/restore result.

Bind an approved maintenance window and stop the exact backup/verification
services and timers through that operation's reviewed command ledger. The tool
does not stop them for the operator: all four must be loaded, inactive/dead,
without drop-ins, and at their exact canonical fragment paths. A normal
preflight additionally refuses pending daemon reloads. Both existing release
and backup lock files must be protected root:root0600 empty single-link files;
the tool acquires them exclusively without waiting.

## Protected transition contract

Create a root:root0600 JSON file outside the release. Duplicate/unknown top-level
keys, additional destinations, malformed paths and unbounded/expired timestamps
are refused. Populate these fields from fresh observations and the reviewed
candidate; no example value is a production binding.

| Field | Required value |
| --- | --- |
| `schema` | `production-backup-transition-v1` |
| `new_commit` | Exact nonzero40-hex installed/current candidate |
| `config_source` | Separate root:root0600 absolute new-config source |
| `issued_at`, `expires_at` | UTC `YYYY-MM-DDTHH:MM:SSZ`, current time inside a window of at most30 minutes |
| `old`, `new` | Objects with exactly `helper`, `config` and the four unit filenames as keys; each value is its `sha256:<64hex>` commitment |
| `unit_file_states` | Exact observed state for each of the four units: enabled, disabled or static |

The new helper and units come only from this installed immutable release. The
new config must bind that commit and its Compose/env files, as well as the
canonical data, backup, key, lock and metrics paths. Config contents and key
values are not printed. A fresh contract time window is an execution bound;
it is **not** a grant of operating authority or an extension of sudo access.

The planned invocations, after all placeholders and operating authority are
resolved, are:

```text
sudo -n /usr/bin/python3 /opt/lagrange/releases/<candidate>/scripts/ops/transition-production-backup.py --mode preflight --contract <protected-contract> --recovery-dir <new-private-recovery-directory>
sudo -n /usr/bin/python3 /opt/lagrange/releases/<candidate>/scripts/ops/transition-production-backup.py --mode apply --contract <protected-contract> --recovery-dir <same-new-private-recovery-directory>
```

Preflight makes no target or journal writes. Apply refuses an existing recovery
directory, saves all six original byte sequences privately, and stages and
journals all six new file identities before replacing the first target. Each
replacement is atomic. The tool then reloads systemd, verifies the same selected
release/inactive units, runs the official installed `--check`, and rechecks all
installed bytes and identities. Success is `APPLIED_TIMERS_INACTIVE`; it neither
enables nor starts timers. Persistent timers can run overdue work when started,
so their later start remains a separate reviewed operating action.

## Failure and file recovery

Ordinary failures attempt file restoration while locks remain held. All targets
and saved originals are checked before the first restoration; a foreign inode,
changed bytes/metadata, a symlink or missing target stops automatic recovery.
After process interruption use the same candidate/bindings and a newly valid
bounded contract window:

```text
sudo -n /usr/bin/python3 /opt/lagrange/releases/<candidate>/scripts/ops/transition-production-backup.py --mode recover --contract <protected-contract> --recovery-dir <existing-private-recovery-directory>
```

The journal supports resuming after destination replacement or restoration.
A crash between creating a staging file and recording its inode leaves the six
old destinations intact; the unidentified staging file requires explicit review
and is never guessed away. Errors, expired execution deadlines or failed reloads
are not reported as successful recovery. Preserve the private directory for
inspection; do not delete a destination or journal to bypass refusal.

`RECOVERED_FILES_TIMERS_INACTIVE` proves only file restoration and inactive unit
observations. It does not restore the database, select an old release, or permit
an old V1 helper to run after daily schema/state exists. Schema compatibility,
operational restore and timer resumption require their own approved disposition.

## Verification still required

[Written fixtures](../../scripts/ops/production-backup-transition-self-test.py)
cover all six replacements, partial failures, actual locks/metadata/ACLs,
two bounded child-process termination boundaries, foreign replacement, corrupt
recovery copies and the official check arguments. The CI policy step has a hard
exit gate and runs despite unrelated earlier fixture failures. None has been run
locally or accepted from source inspection. Batch hosted verification with the
remaining pre-upgrade recovery and approved-base whole-flow preparation; do not
dispatch a CI run solely for this source checkpoint.
