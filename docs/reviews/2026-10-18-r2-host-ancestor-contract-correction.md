# R2 protected host ancestor contract correction

R2-04/R2-09, Wave99. Source target: the commit containing this record.
Status: SOURCE_WRITTEN_HOSTED_VERIFICATION_PENDING. No local project test,
parser, compiler, helper or additional production execution in this correction.
Coordinator static disposition only; no new independent review is claimed.

## Accepted Wave98 image outcome

Candidate `899f259f9c8872fb61f0c95745904f552c8750e9` retains its complete hosted
acceptance (CI39098366992 and smoke39098368916 attempt1 SUCCESS) and now its
actual image acceptance. The single approved image dispatch at07:18:07.982728UTC
produced12 verified images,704 saved-file checks,49 resource/health gates,
7 complete bundle trees and18 producer binaries with exact source provenance.
The12 saved image archives and OCI revisions,2137 source pins and11 controls
were reconciled. The root-owned2597-byte manifest SHA256 is
`1414f4e845a9358bedd72e8ae36a68dff68bc4c5f9704124933ad3866b350d86`.

Owner V2 returned healthy with the same ID/image at07:35:11.227797UTC;
build successful deactivation was07:35:11.233314, independent recovery
successful deactivation07:35:11.508795. Actual process/cgroup absence,
all11 fixed production states, current releasefce9b90 and postrestore READ ONLY
work/stream settlement were recovered. Ten services were healthy; research
remained stopped(exit2/restarts7935). The bounded readable kernel interval had
611 rows and0 OOM matches; this is not proof about all historical logs.

Evidence under `target/r2-daily-beta/wp-8/maintenance-899f259-a4/`:

| Receipt | SHA256 |
| --- | --- |
| attempt-closure.json | d4ef8fa035e9189c364491f3e11845218dd81aa492be3213068fa8aecb05640a |
| image-set-evidence.json | 8336cc15556f94c388091cc17e438e34976bf85625a3cebf19bd4161efb67306 |
| postbuild-source-reconciliation.json | 95c629a72b529297cda6a625e105f84492ef907d70c78c6bef0f190a2915972f |
| postrestore-work-settlement.json | 97108b93fb8039019a45ecb32316127c8f6a078c72707b5ceda6004397d6866a |
| terminal-host-boundaries.json | 58c8fcc3d2e503d47a2d34d24752123859e336493d88262cfcb9f2dacd33d054 |

The accepted899 checkout, images, manifest and controls remain immutable.
All older2a successes and ab48/2a failed attempt evidence remain historical.

## Conditional wholeflow admission stopped before dispatch

Fresh authorized read-only admission at07:38:06UTC resolved the base from the
fixed owner-beta-runner's actual single readonly artifact mount. Its one failed
predicate was `base-safe-parent:/var/lib/lagrange/data`: this ancestor is
root:9820750, without ACL. The selected data root is10001:10001/0750 and its
immediate artifact parent995:10001/0750. Other admission predicates passed,
including the exact12 images/manifest, local PostgreSQL digest, production11,
resource/OOM observations and new unit/output absence.

The coordinator did not launch the project driver, install a unit/create its
result directory, or create wholeflow resources. Dispatch count is0. This is
an admission failure, not a failed runtime wholeflow. Later phases are UNKNOWN.
Receipt `wp-8/wholeflow-899f259-a2/authorized-host-admission.json` SHA256
`c68a83299784dc1e5ecebcd5b5d08a49f0f40c0f259c0100d2fbd3ea028c45e7`;
unlaunched `admission-closure.json` SHA256
`ee15d94e829bfc9ba6f04c9d5747c8a423862b4437f43829597b886bb2cf720e`.

## Source mismatch and correction

`provision-linux.sh:367` checks the data root as root:service_gid0750, and
line420 creates it with root:service_group0750. The group is host allocated.
`release-image-manifest.sh:124-153` already trusts protected root-owned host
ancestors without limiting their group to archived data identities.
The backup file helper instead applied its fixed content GID allowlist to every
ancestor. That contradicts the installation contract; the observed982 is one
instance, not a newly approved privileged exception.

`checked_parent` now checks root ownership, a directory descriptor, absence of
special/group-other-write bits and absence of access/default ACLs. Traversal
still uses descriptor-relative O_NOFOLLOW. The exact observed device, inode,
UID, GID and mode are retained and rechecked at scope exit along with every name
binding. Archived data roots/content retain their fixed identity sets and exact
root UID/GID/mode. The separately bound immediate artifact parent and the cold
PostgreSQL root/private-parent exception are unchanged. The host is not chmodded,
chgrped or copied elsewhere, and no path override, bypass or test skip is added.

Nine real-root private filesystem cases extend `DailyBackupFilesTests`:

- root service-group ancestor roundtrip preserves bytes and host metadata;
- that ancestor and the exact artifact parent remain separate boundaries;
- non-root ancestor owner is rejected;
- group/other-write and special permission bits are rejected;
- actual access/default ACLs are rejected;
- symlink traversal is rejected;
- a GID change during a snapshot is rejected at scope exit;
- unknown data-root/content groups remain rejected;
- ancestor pathname replacement is rejected.

Fixtures use arbitrary14567/14568 identities, not the production982 group. All
existing content, archive, preupgrade and wholeflow tests remain required. The
new exact candidate must retain122 prior required definitions plus these9,
all15 image outcomes and23 wholeflow fixtures, all Rust/PG/Web/policy gates and
separate functional publication/repeat/cleanup evidence. No result is inferred
from source inspection. Runbooks also replace stale historical unexecuted labels
with the already recovered899 hosted results; actual G4 remains pending.

## Next bounded work

Commit this coherent two-source-file correction with the evidence and roadmap;
freeze inherited inputs, current definition line references and candidate blobs.
Verify remote candidate and all-event/all-workflow exact-head absence before
one new CI and one functional smoke dispatch. Recover all jobs/steps/pages and
full hashes/named results before acceptance or another correction. Do not rerun
any older candidate. Raw logs remain RAM-only; finite receipts are persisted.

After hosted recovery prepare fresh successor image/wholeflow controls and one
concrete Owner decision. Original899 approval does not authorize a fifth image
attempt or changed-candidate host run. No new root/production reads, SQL,
Docker lifecycle or local project execution are authorized during this correction.
G4 consistent preupgrade recovery/backup transition/deployment, actual Owner use,
three consecutive trading days and finalC_R2 remain open.

Successor heartbeatbf7d2b09 was confirmed07:48:45.808UTC and its bounded handoff
persisted before confirmed deletion of5f3cfda2. Renew before18:50UTC, expiry
19:48:45.805UTC or47thtick. Same-agent tracking does not protect daemon outage;
native Ralph remains blocked without an agent resume API. No delegation.
