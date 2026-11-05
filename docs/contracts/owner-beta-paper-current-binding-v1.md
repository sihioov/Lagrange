# Paper current-binding v1 — R5-02 F2

2026-10-30 coordinator technical choice: adopt review option B for source implementation.
Compatibility amendment1: current-only projection and root attestation of private assets.
This contract adds a current selector and live publisher fence. It grants no Paper activation,
provider, data-rights, database-role, or trading permission. ADR-0010 and the frozen Paper/source
contracts remain authoritative. Implementation and actual runtime acceptance are separate;
**F2 remains blocking and G2a OPEN until the corresponding evidence is accepted**.

The triggering defect is the checker comparing a caller's old cohort against that cohort's
preserved files. A matching old Daily config/revision cannot prove that root still selects it.
The fixed publication below supplies that missing authority. Source events, SQL authority,
Raw/Daily admissions, present rights, and three-session proofs remain independently required.

## Wire and locations

- Host publication: `<validated binding.evidence_root>/current/current-binding.json`.
- Consumer publication: **`/data/owner-beta-paper-evidence/current/current-binding.json`**.
  The consumer accepts no alternate caller/env-selected current path.
- Directory root:GID10001 `0750`; file root:GID10001 `0440`, regular, nlink1,
  no ACL/symlink; all ancestors satisfy the existing protected-path rules.
- Original operator binding remains root:root `0600`, `.control` root:root `0700`,
  existing global scheduler lock root:root `0600`. None is exposed to the worker.
- Canonical UTF-8 JSON, sorted keys, no whitespace, floats, bool-as-integer, duplicate or
  unknown keys; at most 65,536 bytes, depth16. Existing source canonical/format rules apply.
- [Envelope schema](owner-beta-paper-current-binding-v1.schema.json) copies the existing
  binding/cohort/process/clock definitions exactly. It does not change immutable event wire.

All ten keys are required:

| Key | Contract |
| --- | --- |
| `schema` | `owner-beta-paper-current-binding-v1` |
| `status` | `CURRENT` or `INVALID` |
| `reason` | CURRENT: `NONE`; INVALID: `STARTING`, `STOPPED`, `CONFIG_CHANGED`, `CLOCK_INVALID`, `REPLACED`, or `UNKNOWN` |
| `generation` | Fresh nonnil canonical UUID per publisher lifecycle, stable across refreshes |
| `boot_id` | Current Linux boot UUID |
| `publisher` | Closed positive signed-i64 `{pid,start_ticks}`; informational, not container PID lookup authority |
| `sequence` | Nonnegative signed-i64; starts0, strictly increases on publication within generation; overflow stops/invalidate, never wraps |
| `observed` | Closed signed-i64 nonnegative `{realtime_ns,monotonic_ns,boottime_ns}` |
| `binding_sha256` | CURRENT: digest of canonical full `binding`; INVALID: last validated binding digest, or null with null binding |
| `binding` | CURRENT: full existing closed `owner-beta-paper-witness-binding-v1` nonsecret selector; INVALID: last validated selector or null |

CURRENT recomputes the full binding hash and existing semantic hash. For the latter remove
only `cohort.semantic_config_sha256` and `cohort.cohort_sha256` from canonical binding material.
Preserve the existing complete financial/consent cohort hash rules; this selector creates no
new financial cohort. INVALID can never become an admission capability, regardless of payload.
The publisher copies only the existing closed binding allowlist. It publishes selector paths
and existing digests, never production-env contents, DB credentials, tokens or secret bytes.

## Atomic publication and live fence

Use a new mutable-publication primitive, leaving immutable journal `publish()` unchanged.
Under the existing exclusive `.control/lock`, root creates a same-directory temp inode with
O_RDWR/O_EXCL/O_NOFOLLOW/CLOEXEC at0600, writes canonical bytes, fsyncs, and acquires a Linux
**OFD whole-file write lock** (`F_OFD_SETLK`, F_WRLCK, SEEK_SET/start0/len0).
Verify ownership/metadata after chown0:10001/chmod0440, then atomic rename to the fixed leaf
and fsync directory. Keep the new writable FD/lock open; close the old publication FD after
successful replacement. Publish failures invalidate/close the old authority before propagating.
Do not hardlink or reuse the immutable journal's no-replace primitive.

The consumer independently opens the fixed leaf read-only, checks bytes/metadata and the
fresh named inode, and queries `F_OFD_GETLK` with an F_RDLCK request over0..EOF. Require an
actual conflicting whole-file F_WRLCK with l_pid=-1 (OFD lock), not F_UNLCK or a POSIX PID lock.
It never acquires a lock or writes. Plain flock is insufficient because a read-only FD can
take exclusive flock. UID10001/10003 cannot obtain the writable FD required to forge this OFD
write fence on0440. Unsupported kernel/API/filesystem behavior fails closed.

The authority FD is never inherited/passed to child/helper processes. CLOEXEC and the actual
subprocess/fork paths must be checked. Crash closes the FD, so a recent timestamp alone cannot
keep CURRENT alive. A copied/restored inode has no live fence. A stopped/hung publisher is
additionally bounded by freshness. Root/kernel/Docker remain trusted; this is no code-integrity
attestation and no replacement for actual root source origin evidence.

## Publisher lifecycle and guarded refresh

Hold the existing scheduler global lock for the entire publication lifecycle, including
validation, invalidation and refresh shutdown. Identify/validate the root-private path and
control directory sufficiently to acquire that lock before minting CURRENT. Publish a fresh
generation INVALID/STARTING first, then fully validate binding/env/calendar/manifest/Daily
config/image map and boot/clocks before CURRENT. The initial complete validation may fail;
that must leave no current authority. The global lock's inode is never replaced/unlinked.

Refresh at most every30seconds while CURRENT, including idle/future timer/long helper waits.
Every refresh rechecks original protected binding bytes and referenced current assets and
boot/clocks; no blind heartbeat or source/provider redispatch. Factor startup/idle validation
from session guard: it must work before `self.j`/session timer exists. If using a refresh
thread, serialize shared guard/publication state, retain a fatal error checked by the main
path before every next action, and bound/join its stop before releasing the global lock.
Refresh errors close CURRENT immediately and preserve the source failure/owned cleanup.
No parallel provider or source capture is introduced.

OFF/normal shutdown, config/clock failure, restore, rollback and generation replacement first
publish INVALID and close the CURRENT FD, then perform existing owned child/container cleanup.
If INVALID publication fails, still close the old authority FD and propagate the failure.
An old lifecycle's delayed finally cannot overwrite a new generation: finish it under the
global lock and verify expected generation/current inode before replacement.

Installer/config replacement follows the existing collector-lock then scheduler-lock order
and quiesce/fence/drain protocol. It must not overwrite a live owner's selector or force unlock.
Inactive install prepares current/ and INVALID only; it does not start a service or mint CURRENT.
Leaf ReadOnlyPaths can pin an old inode. Therefore sanctioned configuration replacement must
fence/drain the old publisher before replacing host binding/env/calendar, then use a new
generation. Detection of arbitrary concurrent out-of-protocol root changes is not claimed.

## Consumer checks and host-path projections

Every fixed-path snapshot requires CURRENT/live OFD fence, exact schema, hash/semantic match,
current Linux boot, no negative/future age, and age strictly less than180seconds on realtime,
monotonic and boottime. Compare elapsed deltas under the existing1second wall/boot/mono
divergence rule. Validate with trusted clocks; caller timestamps or PID lookup inside a
container are not authority. O3 OFF/restore epochs remain separate from current generation.

Open/revalidate at beginning and end of eligibility evaluation, and before later real use.
Allow at most3 fixed-path snapshot attempts for a concurrent legitimate rename/old-fence
close; persistent churn denies. Stable `(schema,boot_id,generation,binding_sha256,cohort)` must
match across evaluation. Sequence and each observed clock cannot decrease; same sequence
requires identical bytes. A later legitimate refresh changes inode/sequence/observed and is
accepted with its own fresh live fence. Immutable source/Raw/Daily validation remains strict.
Do not demand current publisher boot/generation equal those of all historical source sessions.

The only new helper mount is the **current directory**, read-only:

| Host source | Fixed container destination |
| --- | --- |
| `evidence_root/current` | `/data/owner-beta-paper-evidence/current` |

Keep existing events/Raw/Daily/config/policy and one-file role-secret mounts and their exact
allowlist. Mounting only the mutable current leaf is forbidden because it pins the old inode.
Do not mount the full release, calendar directory, production env or secret_root; do not
chmod/group-grant/hardlink private originals or publish their contents. The original manifest
remains root:root0600 and is not directly readable by the worker. A release can itself contain
private deploy/compose/.env even when binding.production_env_file points elsewhere.

Root startup and **every** refresh attest the current private assets using existing protected
validators: original binding/env root600, manifest root600/closed V2-V3 format/commit/image map,
calendar bytes/hash under the existing protected reader, and Daily config/policy root:GID10001
0440 plus semantic binding/hash/boot/clocks. Reuse read_manifest/verify_config or equivalently
revalidate the same protected original bytes and exact metadata; mtime or cached success is
insufficient. The live capsule's existing full binding includes manifest/calendar hashes and
image IDs. It attests root's bounded current observation, not unattended source success or a
Paper permit. No new bool, asset bytes, field or DB role is needed.

The consumer independently validates capsule authority before and after evaluation, compares
its **entire cohort** against source/SQL evidence and the untrusted caller, and directly
revalidates the fixed `/run/owner-beta-daily/config.json` (and existing policy alias) against
the capsule's daily_config_sha256, Owner/policy/revision and existing Daily contract. It does
not independently reopen the root-private manifest or scheduler calendar. Root guard and
pre-invalidation of sanctioned changes provide their currentness. Independent committed Raw,
resolved originals, verified Calendar projections/latest-three-session civil-day proof and
current SQL Owner/strategy/epoch/rights/revoke/terminal checks remain unchanged. None is
replaced by the capsule, its calendar hash, or a caller summary.

Canonical binding retains original **host** paths and hashes; never rewrite them to aliases
and recompute semantic identity. Root helper exact argv/actual Docker inspect verifies
host daily_config_file -> fixed config alias mapping and the current directory source.
Consumer compares the fixed Daily env pin/revision against current capsule selectors. It
must not require host daily_config_file equal the container alias, nor derive host data_root
from the Raw alias. Protected current authority has private fields and no unchecked public
constructor/Deserialize; decoded JSON remains untrusted.

This compatibility amendment removes the coordinator's initial redundant consumer-side
private-asset rereads and broad mount choice. It preserves the original Owner/G1 root trust
boundary and all independent source/Raw/SQL acceptance gates. It reduces that extra defense
in depth; it does not claim to detect arbitrary out-of-protocol root changes between guarded
refreshes. Sanctioned replacement must invalidate/close the old live fence **before** changing
assets, then use a fully validated new generation. Original600/700 permissions never change.
The ten-key envelope/schema and OFD/clock/generation/revocation rules are unchanged.

## Source packages and acceptance evidence

Coordinator owns this contract/schema and keeps them frozen during implementation. Producer
package owns witness/runtime helper Python and its existing Python regression file; optional
installer edits are confined to its existing witness branch. Consumer package owns Rust
protected-reader/checker sources and named fixtures only after FIX-A's frozen review and PG
run finish. No concurrent writers share these files. Neither worker edits frozen source-event
schema/literal, SQL migrations/roles, collector3, Cargo.lock or adopted financial documents.

Required source checks cover canonical/duplicate/unknown/secret rejection, protected paths,
generation replacement, guarded refresh while waiting, invalidation/write/cleanup errors,
FD noninheritance, legitimate rename, missing/forged/stale/crashed lease, fixed alias mapping,
same-release selector/config/calendar replacement and before/after changes. Shared Python/Rust
fixtures must agree with this envelope and copied definitions. These are SOURCE/SYNTHETIC
checks until executed against actual protected resources.

Actual OFD/rename/RO-directory behavior, root ownership/protected reader, current publisher
crash, pinned helper/Raw and full G2a remain distinct runtime proofs. Ordinary UID timer/pidfd
and SQL fixture passes cannot satisfy those gates. Actual root/VM/clock/service operations
need their own authorized isolated scope; this source contract does not authorize execution.
