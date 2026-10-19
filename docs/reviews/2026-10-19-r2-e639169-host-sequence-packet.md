# R2 e639169 image and conditional integration packet

R2-04/R2-09, Wave108. **PREPARED_NOT_AUTHORIZED_NOT_EXECUTED.**
Candidate `e639169018519fd6636c5579d1957e323e2c7802` has complete hosted acceptance.
One new explicit Owner decision covers a ninth image-maintenance attempt once,
then only after complete image/restoration/independent recovery closure and work
settlement, a sixth actual isolated wholeflow dispatch once. This is the seventh
prepared wholeflow packet; 899a2 never ran. No second question between approved
phases, automatic retry or window renewal.

## Why this is a new decision

The latest approval79bbfcb7 authorized the b6b2ddb image and conditional wholeflow
attempts. Both executed and are closed. Images were accepted; the first producer
and admitter succeeded, then the second producer failed with INPUT_BLOCKED.
[Full recovery and source correction](2026-10-19-r2-daily-discovery-canonical-order.md)
preserve the exact outcome and its unknown inner predicate. The adopted execution
boundary requires a new explicit decision for a new candidate and another Owner V2
interruption, after a concrete packet is ready. Root access is not that decision.

The source correction reuses the existing canonical wire ordering before discovery
returns a session. This removes the verified enum-versus-wire ordering mismatch
between returned sessions and sealed manifests used by frontier comparison. Hash
grammar, source validation, limits, approval, subprocess/retry and all host guards
are unchanged. Four new Linux Rust tests passed, including both exact synthetic
wholeflow sessions through actual Raw capture/discovery and canonical roundtrip.
This is source/regression evidence, not a recovered runtime stack trace or proof
that later host phases pass. Another failure is possible.

## Impact and boundaries

Existing Owner V2 stops normally once, pausing its updates/quote processing during
image preparation. Independent recovery must actually be active before the stop.
Restore the SAME container/image healthy immediately when the build terminates or
fails. Owner ID `f293c7be42fb02f5b20f0bbcf1d7b6568518fdcb52b6c47c20deaba3f73a6af4`,
image `sha256:32bc37e1bffda89baefea466c0ba127c89b0cca69401c10ccf4f984c76cf6c50`.
Normal exit0 is mandatory; no forced-kill fallback. Nine other serving containers
remain healthy; research remains stopped exit2. The shared12h window is a maximum,
not a duration estimate. The isolated driver has20min and full recovery within60min.

The existing fixed V3 role-copy behavior is unchanged. Before copying, freshly
verify original candidate leaf
`b0fb82f6a580f3def13a1e1e34bea68e30d95dc720306e7ee54b6c3199cf402d`:
UID/GID10001:10001, leaf0700, exactly manifest.json and bars.ndjson regular single-link
files0600, no ACL/symlink, stable held descriptors/path identity, bars7672914 bytes
and manifest<=2MiB. Current metadata is UNKNOWN until approved fresh observation;
previous metadata is not a reusable attestation. Mismatch blocks; no permission repair.
The runtime verifies unsigned canonical manifest
`53587f32ce67ae1a8488b9c00096465185d03e35b1b38a04f39ed5462da7f6b6` (not whole-file SHA)
and bars `23354c54708d7a458aadc4f2cb2fca77469969e91256b6d3b119101ebae98ffe`.
Private producer10002/admitter10003 copies have roots0750, leaf0700/files0600 and
per-role readonly mounts. Original bytes/permissions and worker/API base stay intact.
Exclusive atomic complete-pair creation precedes private DB preparation; verify before
enable. No overwrite/repair/retry of uncertain stages. The existing daily backup
class includes both copies; restored metadata/bytes and immutable comparisons are required.

Retain volume `159b052fed63a476fba2c51c8f15f532a837096e4bdd453bb753789672034f07`.
Ownership remains unproven and original0120 deleted Mounts association is missing.
Only approved exact pre/post metadata and reference comparisons; no mount, content
read, deletion or prune. Drift blocks. Prior0120 cleanup stays INCOMPLETE regardless
of new cleanup. Unexpected new volumes retain associated containers and intents.

G4 backup/version transition, deployment/migration, research activation, provider or
Auth0 access, trading, Owner use and final acceptance are outside this packet. Preserve
failed units/results, all accepted images/manifests and evidence without reset,
relaunch, substitution or overwriting. Raw logs stay in RAM; only approved finite
labels, counts, exit fields and hashes are retained. Unprinted observations stay UNKNOWN.

## Prepared inputs and evidence

[CI39186174905](https://github.com/sihioov/Lagrange/actions/runs/39186174905) attempt1
succeeded all8 jobs02:03:23UTC; [functional39186177169](https://github.com/sihioov/Lagrange/actions/runs/39186177169)
attempt1 succeeded02:20:59UTC. Full recovery02:21:48.111212UTC:9 full logs/8 actual
checkouts,1437 input pins/537 candidate blobs,174 required definitions,15 image
outcomes/44 wholeflow definitions,3367 named Rust rows/231 root blocks/36 groups.
Strict Clippy/fmt, Python497 passed+10 existing skips, PG19 processes/146 names,
0060 32+73/83 bootstrap/62 checksums twice/0039 cleanup and Web143 passed.
Actual functional publication/source lineage, repeat unchanged,10 cleanup exits0,
owned resource absence and original/global exit0 recovered. Preserve3 non-Linux
exclusions,6 ignored Rust cases,10 Python skips and independent nested/unprinted
UNKNOWN. No workflow was rerun, canceled or duplicated. Hosted-only acceptance.

Clean detached `/data/worktrees/3puw275b/r2-release-e639169` has2155 source pins;
previous b6 checkout2152 pins were checked against its own commit and preserved.
Inert image env remains byte-identical0600. Eleven controls use finite candidate,
path/unit/description/dependency-hash substitutions with inverse-byte matches,
expanded argv and unit-template correspondence. Three runtime/work probeSQL files
are byte-identical. Wholeflow has12 unchanged critical source pins. Admission and
observer use finite substitutions only: reversing them matches b6 bytes. The exact
base metadata block, duplicate-key rejection and finite command_failures schema
are unchanged. No local project parser/helper/compiler/tests/control execution,
new root/production reads or new independent review. New host/images/manifest,
source-input hash, protected output/unit absence and work settlement remain UNKNOWN.

An initial coordinator generator substituted a short-name prefix inside the PRIOR
full commit ID. Its identity assertion stopped before exclusive preparation outputs.
The original draft/error record is retained; exact old2152/new2155 verification
completed before writing these outputs. Only a detached checkout had been created;
no project/control or host execution occurred.

| Receipt under `target/r2-daily-beta/` | SHA-256 |
| --- | --- |
| `ci/e639169/hosted-terminal-evidence.json` | `f44b66b1b1ce613fa0836385475cc165d8d021bd16cfd93c63fad2307a9c9314` |
| `ci/e639169/functional-smoke-evidence.json` | `2cd651b96fe22caafd763cf32a276f0e287afd5ef401470f40105f53d79fb983` |
| `ci/e639169/required-ci-evidence.json` | `006273fc662daf091e597f47e9e21d6c39526a9d643d5d41eaffe7921bf1cb6b` |
| `image-preparation-e639169/preparation.json` | `280a3db5edbebeec44a134e59db6869a36f94faa226d76d3371f2c1ce62b139b` |
| `wp-8/maintenance-e639169-a9/preparation.json` | `1da14f1e1ea84366ce62e1cde2194d87d4b39eadc574d3fd8e288aa99e2cde06` |
| `wp-8/maintenance-e639169-a9/command-ledger.json` | `5a72c5193ea257418592547bd743850dc078a81f0826a1515705d81b9615a0f4` |
| `wp-8/wholeflow-e639169-a7/preparation.json` | `49be495db974929b87e4caee47f9a8e1a3470e68fa3441bcf9970a55e9f39120` |
| `wp-8/wholeflow-e639169-a7/command-ledger.json` | `695e4cc965edee72d0dc582f157ba46c9d56b0fba29c378298417b16ad0cb219` |
| `wp-8/wave108/retained-volume-disposition.json` | `c4fcbe4a69c3c7070acf39b9af89304e7ba121de47986312988db9c2448c324f` |
| `wp-8/wave108/prepared-sequence.json` | `361f94a8689caa8481aab4492a7a679c992a685950d66bc426e41503da59ef86` |
| `wp-8/wave108/final-static-reconciliation.json` | `46b770789e0c11bbc06a2bf8f6bc9f25b1177a963f59cee9f183ee18587894a2` |

## Ordered execution after the new decision

1. Bind decision/time. Recheck all2155 source pins,11 controls, inert env and clean
   detached checkout. Fresh bounded host/no-clobber/READ ONLY work checks and official
   plan/preflight establish the new actual source-input hash. No old attestation/C3
   wrapper. Install only new root controls at
   `/etc/lagrange/release-manifests/.r2-maintenance-e639169-a9-20261019`.
2. Establish shared12h window; verify `lagrange-r2-recovery-e639169-a9.service`
   actually active before ONE stop intent and normal SIGTERM. Stop only in first1h;
   exit0 mandatory. Build unit `lagrange-r2-build-e639169-a9.service`, runtime10h,
   recovery watchdog10.5h, attestation11h; no renewal. Nice10, idle IO, weights20,
   Compose parallel1/Cargo jobs2, four batches of3 with EXACTLY ONE service per
   invocation. Check memory/swap/OOM/health/termination between each batch.
3. Accept all12 exact image IDs, OCI revisions, saved bytes/layout and immutable
   manifest. Recover actual successful build AND independent recovery terminal
   results, no processes/cgroup tasks, same Owner healthy, exact11 production
   states and postrestore READ ONLY work settlement. Otherwise do not start wholeflow.
4. Fresh five-minute wholeflow admission binds NEW accepted manifest/12 images,
   2155 source pins/12 critical pins, local PostgreSQL digest, resources,11 states,
   retained volume and fixed source metadata. Base ONLY fixed runner
   `1b8bec839b8592918a4f5785233e64bb73a6ab601ad0309f1f24b5941a3824a5` single readonly
   `/data/owner-beta-artifacts` mount; no guessed path or old-image fallback.
5. Dispatch `lagrange-r2-wholeflow-e639169-a7.service` ONCE before image hour12.
   Result `/var/lib/lagrange-r2-wholeflow-evidence/e639169-a7-20261019/result.json`.
   Driver1200sec/2GiB, private nonce DB/network/resources, no provider/Auth0/production
   SQL; outer50min, SIGINT, stop5min, SendSIGKILL=no. Full recovery within60min of
   dispatch AND by shared hour13. No renewal/retry or second permission between
   approved phases.
6. Recover actual first/second admissions, publication/HTTP, restart, capture/restore,
   62 checksum audit, restored role copies, original base unchanged, new owned cleanup,
   exact11 states/current release unchanged, and finite failure evidence if it fails.
   Deadline or uncertain cleanup remains INCOMPLETE. Do not infer later phases.

## Tracking and remaining R2 work

Current heartbeat68a23346 confirmed02:32:49.330UTC, handoffcfb61e58 persisted.
Renew by13:00UTC/before47th tick/12h expiry14:32:49UTC. The summary result path was
corrected to the exact command ledger. Same-name heartbeat creation returned the
same ID; a mistaken predecessor deletion caused an approximately22-second tracking
gap, immediately corrected with a distinct-name/ID heartbeat. No host action occurred.
Future replacement must verify successor ID differs before deleting predecessor.
Same-agent bounded continuation, not a worker or outage protection; native Ralph
remains blocked with no resume API. No delegation.

G4 consistent pre-upgrade backup/transition/deployment, actual Owner use, three
consecutive actual trading days and final C_R2 remain open. The separate static G4
design02534508 addresses missing staged base helper and complete historical terminal
unit inventory; it is UNIMPLEMENTED/UNEXECUTED, not included in this decision.
No R3 expansion. Preserve32 unrelated ROADMAP additions and .archify.
