# R2 G4 compatibility CI acceptance and operating gaps

Status: **790f1ea HOSTED_ACCEPTED_TOOL_ONLY preserved; Owner information resolved; root staging/publication succeeded; first prepare exit2 INCOMPLETE; Wave115 b6e409a HOSTED_ACCEPTED_TOOL_ONLY; distinct fixed package28 connection regressions PASS, successor prepare exit2 before outage; Wave116 large-mount correction passes actual historical binding; a2 prepare exit2 before outage. Wave117 canonical-tool reuse and empty-package correction sealed after16 focused and17 connection checks.**
Roadmap: R2-04/R2-09. Application e639169 remains separately accepted through its images and actual isolated wholeflow, and is not deployed.

2026-10-20 Owner의 외부/수동 writer 없음 답변과 베타 로컬 키 방침을 실제 메시지에 바인딩했다. 정보·승인 대기는 없다. 04:24UTC root34항목 설치와 보호된 정보 저장은 exit0, 04:25UTC prepare는 exit2 INCOMPLETE였다. 실제 READY·중단·백업·배포는 미실행이다. 아래 Wave115 기록에서 제한된 진단으로 재현한 Mounts 순서 의존 오류와 보정 검증을 구분한다. 기존 앱e639169/12이미지/실제통합 및 도구790f1ea의 hosted 인수는 보존한다. G4·백업 전환·배포·Owner·3연속 실제거래일·C_R2는 OPEN이다.

## Exact candidate and recovered evidence

Candidate `790f1eaf87ab1736d13e6f827cacc1048335c41c` was dispatched once at05:22:17UTC2026-10-19 after exact remote SHA and all-event run absence checks. [CI39204199932](https://github.com/sihioov/Lagrange/actions/runs/39204199932), attempt1, completed all eight jobs at05:45:47UTC. Full evidence recovery ended05:46:39.007085UTC, within the07:07:17UTC reporting bound. No retry, rerun, cancellation, local project test/helper/compiler execution or new independent review occurred.

Eight complete logs were read in memory, hashed and reduced to finite observations. Seven actual checkout commands reported the exact candidate; the aggregate gate has no checkout. All1464 frozen input pins and548 tracked blobs matched, with2163 tree entries. The201 required definitions comprise193 Python policy names and8 carried Rust names. The tool-only change did not request new application smoke or release images.

| Receipt | SHA256 |
|---|---|
| hosted-terminal-evidence.json | b2cc36c28edf8b745950bcc2a230bbe4d5551a76bc414a5c2bd7e3d574284b47 |
| required-ci-input-reconciliation.json | aeb5b32885272dcbc5d22d0f26a48efaecf5c406905e114f2858dc4c45d80b51 |
| policy-pg-web-reconciliation.json | e32f8a888f803388a88c9e4effba3a5dc9c44b5ef560bb3be383b4fd4886c034 |
| workspace-reconciliation.json | f56877772da46a8aab0e5baf0139d99616c976bbf0075f0d1b2a22d28c20ac13 |
| post-ci-operating-readiness.json | 2221d34fd2604ae73eac9566480e567997a574de6cb599a196f6c281a867ea04 |

Private receipts are under `target/r2-daily-beta/ci/790f1ea/` and `target/r2-daily-beta/wp-8/wave114/`. Accepted source, old survey controls and evidence remain immutable. This follow-up records post-commit results without changing candidate source.

## Validation and limits

- All55 maintenance cases passed:41 carried and14 new. The new cases cover observed activation graphs and retained disabled persistent failures, plus actual ownership/mode/ACL/inode/name replacement checks inside isolated hosted chroots. Systemd observations in those cases are mocks.
- Four existing actual Docker/PostgreSQL cold-cluster cases passed, including encrypted round-trip, catalog mismatch, control corruption and lost-create-response cleanup. Eight printed source/restored PGDATA records retained999:0/0700. These hosted fixtures are not production G4 capture or recovery.
- Rust evidence contains3367 actual named rows,231 root blocks,36 workspace groups and33 carried required groups. All expected root summaries and carried required names matched. Strict Clippy and formatting passed. Python reported497 passed and10 existing skips.
- Dedicated PostgreSQL validation recovered19 process exits and146 named results, daily0060's32 harness names and73 subcases,83 bootstrap checks,0039 cleanup and all62 migration checksums matched twice. Web143 cases and the carried supervisor/cleanup evidence matched.
- Fifteen image-related outcomes and44 wholeflow fixture definitions remained accepted. Workspace cleanup printed successful removal of its named QA container and network; no additional absence query is claimed.
- Three non-Linux exclusions, six ignored Rust rows and ten Python skips remain explicit. The fragmented nested test name and unprinted independent nested rows/exits remain UNKNOWN. Parent success does not fill them.

The implementation scope is documented in [the frozen source review](2026-10-19-r2-g4-host-compatibility-source.md): explicit pinned PG ancestry, actual independently observed activation fields and an exact retained-disabled-failure category. Parent0700, complete inventory/generation/fragment/cgroup checks, archive layout, fixed SQL, encryption and cleanup guards remain. No generic ownership relaxation, source permission mutation, historical unit reset/removal or new provider surface was added.

## G4 preparation completed without host execution

Twelve accepted payload files,305106bytes, were copied inertly into a private local staging directory and byte-compared. They have not been installed on the host. Static receipts bind the original56-unit/13-container survey, the11 project containers, five active timers, stopped research state, old59 source migration files and the fixed PG volume. Live metadata and source migrations do not substitute for a consistent backup or actual old-schema audit.

The draft proposes a single45-minute shared window with25 minutes for main work and time for independent recovery. It is not an approved outage. Independent recovery must report actual readiness before any stop or exact PG-parent0701→0700 change. Restoration must return that same parent inode to0701, start the original PG ID first and restore the original writer IDs and prior timer activation states. All history, disabled KIS and research exit2 remain preserved. Fresh admission must reject drift rather than repair it.

The official commands are one `--mode audit` followed by one `--mode capture` against the same fresh contract. Capture already performs encrypted set verification, actual isolated clone audit, restored manifest comparison and source stability checks before exclusive COMPLETE publication. It does not justify an automatic second capture or retry.

Docker stop uses `-t -1` to avoid a daemon timeout escalating to SIGKILL; client timeout is not evidence that a daemon stop was cancelled. Actual owned process/cgroup termination is needed before recovery can restart a container. [Docker stop reference](https://docs.docker.com/reference/cli/docker/container/stop/). Systemd `Type=exec` confirms launch rather than application readiness, and `SendSIGKILL=no` does not prove no processes remain. [systemd.service](https://www.freedesktop.org/software/systemd/man/latest/systemd.service.html), [systemd.kill](https://www.freedesktop.org/software/systemd/man/latest/systemd.kill.html).

The release lock was actually absent in the approved survey. A future operating packet must explicitly create it exclusively as root:root0600 under the held protected parent and retain it. No lock has been created. The official tool acquires release/backup locks itself, so an outer coordinator must not hold another exclusive open-file-description lock on those same files while invoking it. A separate private dispatch lock is needed.

Restoration compares stable container identity, image, Config, HostConfig, Mounts and network membership/IDs while recording real new process timestamps and network endpoints. It must not fabricate unchanged StartedAt, raw NetworkSettings or live PG bytes after a normal restart. Exact stopped-state policy hashes remain required during audit/capture. Deadline or cleanup uncertainty is INCOMPLETE.

## Pending inputs and remaining work

### Static control components — 2026-10-19

Three private draft files were added under `target/r2-daily-beta/wp-8/wave114/`:
`g4-control-protocol.py`, `g4-journal-storage.py` and `g4-readiness-protocol.json`.
They define the main/conditional restoration decisions, exclusive bounded action records,
and the proposed actual-readiness exchange. The decision draft contains22 main actions
and17 restoration/watchdog actions. It has no host entrypoint; the storage draft requires
a future authorized adapter and already held private directory. No draft was imported,
parsed as Python, compiled, tested or executed, and no new independent review occurred.

The direct source review separated capture admission from restoration gates so lack of
space for another capture does not itself prevent restoration. The watchdog's normal
termination request cannot depend on the lock held by the main process. An existing action
intent cannot dispatch again; unresolved Docker stop effects and main/watchdog termination
must be observed before restart. Journal append locks are short-lived and separate from
mutation ownership, avoiding an append-lock deadlock during watchdog termination.

The preparation receipt `g4-control-preparation.json` has SHA256
`136e5cc1b553ce386b6a24fa42df9a0bfac5e102a30b957f7f22b490bc94c11f`.
Literal comparison matched the nine stop-container IDs and five timer names against the
existing draft ledger. Three accepted maintenance source files remained byte-identical to
790f1ea. The prior readiness, ledger and pending-question receipts were unchanged.
These comparisons do not establish executable correctness or a current host admission.

The draft remains **NOT_READY / NOT_AUTHORIZED / NOT_EXECUTED**. Fresh observation and
dispatch adapters, actual readiness server/client, complete recovery/final observer,
partial-journal crash reconciliation and clock-drift recovery are still missing. Partial
records must remain intact without replay; a safe recovery path for that case must be
completed before this can become an operating packet. Readiness booleans in the pure
decision model are obligations for those adapters, not observed production facts.

### Static failure-path review — 2026-10-19 07:00 UTC

The implementing agent directly reviewed the three pinned private drafts. Receipt
`g4-static-recovery-audit.json` has SHA256
`a9f6cbc5b5699ef729bd94ff9edd16250933347605603d0f0e4922bfacaf3428`.
Four findings remain open; these are source-derived counterexamples, not executed tests:

| Finding | Source evidence | Required correction before readiness |
| --- | --- | --- |
| G4-DRAFT-01 | `g4-journal-storage.py:117` reads every final record; `:196` creates the final name before writing its bytes. | A damaged record must not erase access to previously committed restoration obligations. Preserve damaged bytes and prohibit replay while providing a reviewed recovery/reconciliation path. |
| G4-DRAFT-02 | `g4-control-protocol.py:81` rejects clock disagreement; both main and recovery call it through `bound`. | Stop new main work on clock drift while defining safe restoration within the original same-boot monotonic deadline. Never renew the window or reuse authority across a boot change. |
| G4-DRAFT-03 | `g4-control-protocol.py:318` selects restoration for every consumed stop intent; `:154` requires a witnessed stop. | Distinguish witnessed stop, proven no effect and unresolved effect. A proven unchanged instance needs no start command; a healthy instance with a pending daemon stop is still unresolved. |
| G4-DRAFT-04 | `g4-control-protocol.py:175` still includes capture admission in restoration closure. | Record actual restoration separately from backup acceptance. Insufficient space for another capture must not conceal observed restoration or manufacture backup acceptance. Actual safety and cleanup gaps remain INCOMPLETE. |

The receipt defines twelve unexecuted scenarios covering partial writes, lost returns,
clock/boot changes, stop effects and restoration outcomes. It does not fix the drafts.
The accepted790 tool CI does not cover these new private controls. The existing
[WP-7 review requirement](../superpowers/plans/2026-10-13-r2-daily-research-beta.md#wp-7--독립-코드운영-준비-리뷰)
is not satisfied by this same-agent review; no new independent review or delegation
occurred. Validation and exact runtime adapters remain incomplete. These findings do
not reopen the accepted app/tool evidence or authorize a host operation.

### V2 correction draft — 2026-10-19 07:23 UTC

`g4-control-protocol-v2.py` is a new private, unexecuted pure model, pinned by
`g4-v2-static-preparation.json`/`63ab12285d5c5b1baa19ca41e1f7c48a6f9661a67fe6d4d1bc7f7399a38f8e8e`.
It separates wall-clock rejection of main work from the original same-boot monotonic
recovery deadline, distinguishes witnessed stop from proven no effect, and reports
original-instance restoration, settlement and backup acceptance separately. A clock
failure ends the main invocation; time stabilization cannot resume it. Unknown stop
effects still prohibit replay and restart. No-effect states need fresh closure evidence.

The V2 record schema intentionally rejects V1 records. The existing storage and readiness
drafts remain unchanged and do not implement the V2 adapters. A separate storage design
specifies complete-record atomic no-replace publication and independent reconciliation;
the exact primitive, implementation and crash tests are still missing. Sixteen V2 scenario
definitions and twelve storage fault-check requirements are unexecuted. The same-agent
source/diff review found unchanged action targets/order; accepted790 source is byte-identical.
There was no parser, import, compilation, test, control execution or independent review.
**All four findings remain open** until their implementation and validation evidence is
complete; this draft is not an operating packet or a new host authorization.

### V2 storage draft — 2026-10-19 07:43 UTC

A new private `g4-journal-storage-v2.py` draft is pinned at `341d60460e01c311ec4af959d80f90387b80e1762426e220b9a242c4d9b2e5f8`;
`g4-journal-storage-v2-preparation.json` is `ed5adf1020f0ec47ee1c898405a4fc2b78ac839fd4c6c6145d1650d80fcfd696`. The earlier V1 storage and
V2 model/design receipts remain unchanged. The model's earlier statement that no
storage existed describes that earlier stage; this new storage is still unwired.

The draft uses separate permanent attempt slots and contiguous protocol record
sequences. An interrupted staging file retains its slot and bytes. It blocks main
work while allowing inspection of the separately validated committed records.
Recovery can append a different record in a fresh slot only through a future
pinned effect validator; no orphan is promoted, repaired, removed or treated as
proof that a command had no effect. Corrupt committed records and unknown members
reject the complete snapshot. The snapshot hash includes every retained orphan.

The proposed publication primitive is `renameat2(RENAME_NOREPLACE)`, which requires
filesystem support and does not overwrite an existing destination. Matching device
numbers do not establish matching mount points. [Linux rename(2)](https://man7.org/linux/man-pages/man2/rename.2.html).
The draft fsyncs the complete file and both affected directories; file fsync alone
does not make a directory entry durable. [Linux fsync(2)](https://man7.org/linux/man-pages/man2/fsync.2.html).
No primitive was called or host/filesystem compatibility observed. There is no
fallback or retry. Lost return and publication uncertainty still end main work;
reading an intent back is never dispatch authority.

Sixteen specific storage fault conditions are recorded but **none was executed**.
The bytes-to-model bridge, independent effect validator, durable failed-main latch,
actual readiness/observation/dispatch/recovery and final observer remain incomplete.
No parser, import, compilation, test or private-control execution occurred. Direct
source review is not independent review. All four findings remain OPEN, and the
operating packet remains NOT_READY / NOT_AUTHORIZED / NOT_EXECUTED. The accepted790
maintenance source files were byte-compared again; they and the existing question
receipt remain unchanged. No new operating question was issued.

### Journal/model bridge — 2026-10-19 08:14 UTC

`g4-journal-protocol-bridge-v2.py` (`ac96517a541e457f38261772110e48bd49c8182944879f47698684f1cb94f09c`) now provides the strict bytes-to-model
callback and finite intent/terminal receipt binding code. Preparation `ab17dc05f1e57e80d6940685eecb6b7103a081d45c4c98b6e7de30a687f67f73` pins
it with seventeen regression test definitions in `test_g4_journal_protocol_bridge_v2.py`.
The test source uses synthetic bytes and the exact pinned model; **no test was run**.
No parser, import, compiler or private-control execution occurred.

A terminal record must reference the exact consumed intent, snapshot, observation,
operation and result. Timeout/healthy-state claims cannot resolve a lost stop response.
The orphan inventory remains visible, but mutation intents with unresolved orphans are
still rejected. This is not a complete orphan recovery path. The production loader and
actual receipt/host-evidence verifier remain missing; matching labels/hashes alone do
not establish those facts. Readiness, dispatch, recovery and final observer integration
also remain incomplete. All four findings remain OPEN and **zero additional operating
completion conditions passed**. Accepted source/evidence and the existing question were
preserved; no new operating question or authorization occurred.

### Observation and readiness transport source — 2026-10-19 08:38 UTC

Preparation `1481c3f7db6177265fe783f5374d2de9cdf0cd449c1fa4b94b26c2df8fe0b470`
pins `g4-live-observer-v2.py` (`da12aeae`),
`g4-readiness-transport-v2.py` (`9a0d4572`) and 22 unexecuted regression cases
(`ea0cc374`). The source now contains protected held-FD loading of exact control
and evidence bytes, bounded read-only host queries, actual process generation and
recursive cgroup observations, filtered container identity/configuration checks,
and the local readiness transport. No code was imported, parsed, compiled or run.

The transport checks both peer generations around an exchange and propagates one
2-second deadline into the observer's subprocess queries. Truncated packets,
unexpected ancillary data, replayed challenges and changed generations reject the
exchange. The source uses Linux Unix-socket semantics documented in
[unix(7)](https://man7.org/linux/man-pages/man7/unix.7.html), the versioned
[Linux v6.12 seqpacket implementation](https://raw.githubusercontent.com/torvalds/linux/v6.12/net/unix/af_unix.c)
and [Python socket APIs](https://docs.python.org/3/library/socket.html#socket.socket.recvmsg).
These references are source-review inputs; local kernel/interpreter behavior has
not been exercised. No transport change is an observed readiness result.

A peer's READY response remains a claim requiring the actual lock/parent-handle/
restore-plan/watchdog checks. A stopped container snapshot does not resolve a
pending Docker stop. The narrow bridge sample carries only same-boot/window/current
facts; it cannot authorize an action. Full fresh admission, effect reconciliation,
orphan recovery, durable failed-main handling, main/recovery/final-observer
integration, fault validation and independent review remain unfinished. The new
synthetic tests cover metadata filtering, identity drift and message validation;
they do not establish actual filesystem/kernel/process behavior. **All four
findings remain OPEN; zero additional operating completion conditions passed.**
Accepted tool/application sources, old controls, the retained volume and the
existing information question are preserved. No new host approval was requested.

The existing asynchronous information request remains pending for an off-server encryption-key custody identifier, without key values, and external/manual writers beyond known containers and schedules. The observed439 processes/4336 descriptors do not prove a global future writer fence. No answer is assumed and the question is not repeated.

The exact admission/main/recovery programs and readiness handshake still need completion against those operating facts before a final concrete packet and one operating decision. No new host approval question is pending. The consumed survey approval is not transferred to an outage. Existing read-only KIS and Owner V2 permissions remain preserved; they are not asked again merely because the roadmap stage changes.

G4 consistent cold five-class capture/private-clone proof/same-ID restoration, backup transition, application deployment, actual Owner use and three consecutive actual trading days remain open. Synthetic fixture dates do not satisfy those days. The application installer must use e639169, not the maintenance-tool commit. The prior anonymous volume remains retained with ownership unproven and0120 cleanup INCOMPLETE; no mount/content/delete/prune is permitted by this work.

후속 추적e3e092a9를05:48:08UTC 별도 active ID로 확인하고 인계0613d80d/registry 저장 후4786839b 삭제를 확인했다.17:00UTC 전/47번째 tick 전 갱신,17:48:08UTC 만료. Native Ralph blocked/no resume API; 동일 에이전트의 제한된 후속 추적이며 worker나 장애 보호가 아니다.

## Recovery resource monitor source, 2026-10-19 UTC

Preparation `ebcac376` pins the unexecuted resource monitor `cf23e3a7`, static
expected scope `b7c0a9e3`, and 19 synthetic test definitions `6cb3e228`. The monitor
opens the existing recovery singleton lock, checks its own descriptor's lock
record and a separate-open conflict, holds the exact PostgreSQL parent ancestry,
loads the original-instance plan, and registers one absolute monotonic timer with
the local listener. The fresh plan must come from admission; the old survey cannot
populate it. Its attempt fields exclude the admission digest so that plan,
admission and binding can be hashed without a circular self-reference.

The timer has no rearm path. Deadline events take precedence over queued main
requests; delayed kernel delivery cannot reopen main work. Main revocation retains
timer polling, and shared-deadline uncertainty is INCOMPLETE. The narrow `/data`
ancestry exception stays limited to the fixed PG path. No ownership/mode/content
change occurs in this monitor. Source review used the documented
[timerfd semantics](https://man7.org/linux/man-pages/man2/timerfd_create.2.html),
[flock semantics](https://man7.org/linux/man-pages/man2/flock.2.html), and
[Linux fd-specific lock reporting](https://raw.githubusercontent.com/torvalds/linux/v6.12/fs/locks.c).
The implementation requires a verified Linux 64-bit native ABI; host behavior has
not been exercised. CLOCK_MONOTONIC does not provide suspend or host-outage protection.

The resource receipt explicitly reports readiness and dispatch as false. A timer
event is not a main-stop command or a successful restoration. Full fresh admission,
pinned loading of all new controls, READY receipt/response, actual command wait
and daemon-effect/orphan reconciliation, durable failed-main handling, and the
integrated main/recovery/final observer remain unfinished. Only direct source,
byte and whitespace review ran; no parser/import/compiler/test or host query ran.
All four findings remain OPEN and no additional operating completion condition
passed. Accepted source/evidence, retained volume and pending information question
are unchanged. No repeated question or new operating approval request was issued.

## Priority correction: minimize implementation before operating work

The Owner asked to move on. The next task is the smallest complete G4 operating
packet around the accepted tool, rather than further expansion of the private
controller. G4 requires exact actions, effects, limits, consistent backup, actual
isolated recovery and an effective resumption procedure; it does not name the V2
controller as a required implementation. The later independent-recovery-before-
outage requirement remains binding. Reuse must be established by direct source
and contract review; no existing recovery unit/authorization may be relaunched
or transferred. No alternative is claimed ready yet. No host operation, waived
backup gate, new approval or R2 completion is inferred from this direction.


## Existing tool path and deployment bindings

Owner directed continued work on the smallest operating path. The source comparison
is complete. The accepted maintenance tool already performs encryption, an actual
private PostgreSQL restore/audit, restored-file comparison and final publication;
a second backup implementation or automatic extra verification run is unnecessary.
The prior accepted image recovery is limited to one Owner container, uses a12-hour
wall-clock window and may repeat restoration calls. It is not a complete G4 recovery
program for ten original containers, five timers, the PG parent and clone cleanup.
Its old authority, unit and evidence remain immutable and are not reused.

| Static result | Evidence / practical limit |
| --- | --- |
|12 maintenance payloads,305106 bytes match accepted790f1ea | Existing inert copies and Git bytes compared; not host staging or execution. |
|17 deployment source files match accepted e639169 | Current checkout and clean detached e639 checkout both matched; no rebuild or source change. |
|59 old UP migrations are byte-identical; only0060–0062 added | All62 target UP files have SHA256/SHA384 pins; this is not an actual DB ledger audit. |
|34 G4 action entries | Five timer stops, nine writer stops, two local preparation/restoration boundaries, audit, PG stop, capture, ten same-ID restorations and five timer starts. Every entry is unexecuted. |
|10 official deployment command entries | Candidate/path bindings and prerequisites recorded; production values and lifecycle dispositions remain unresolved. This is an effect map, not an executable approval packet. |

Private records:
`target/r2-daily-beta/wp-8/wave114/existing-tools-minimal-sequence.json`
(SHA256 `1ff0729e68bd7cade5ca05aa66bbaa6c43711224c3d05e1273b2e1dbfae5f5ab`)
and `deployment-e639169-source-plan.json`
(SHA256 `b04aa370cf8868b796729ba40f9bef34218003d85b6521821f14c409a877f81b`).
Source references are the unchanged `prepare-production-recovery.py` audit/capture,
`deploy-production-release.sh`, `compose-release.sh`, daily operations helper,
backup transition and KIS/service-status installers. No project/private control
parser, helper, compiler, tests, Docker, SQL, root or provider operation was run.

Required sequencing is explicit: G4 consistent recovery and old-service resumption
must finish before rollout. The immutable installer changes `current` without
starting services. A target daily service-status binding is needed before normal
Compose apply to preserve daemon absence. Compose performs role bootstrap and UP
migrations but leaves daily processing OFF; controlled prepare/backup transition/
verified backup/enable have separate effects. Enable can replace its owned old
cohort, so G4's same-ID resumption promise cannot be carried over to deployment.
An old current-link rollback does not undo0060–0062 or remove the reader floor.

The first automatic session, budgets, fresh target env and mode branches are not
filled from fixtures. KIS install-disabled can be prepared after16:30KST, but the
normal installer apply cutoff is unchanged; no direct timer-start bypass or first
observation date is proposed. Existing KIS/Owner permissions are preserved.
The earlier custody/external-writer question remains pending without repetition.

Further private framework expansion is deferred. Minimal admission, ready-before-
outage recovery and terminal/effect settlement still need an executable connection
and validation; four earlier findings remain OPEN. No ready replacement or fresh
operating authorization is claimed. G4, deployment, Owner acceptance and three
actual trading days remain open.


## Owner authorization and actual isolated validation

The Owner explicitly approved all remaining R2 work: “다 승인하니까 다 진행해 다음
질문때까지 R2가 전부 완료되게 ralph로 진행해”. This is new authorization for
necessary implementation/validation, G4 backup/restoration and deployment. Do not
request the same permission again. `owner-r2-blanket-authorization.json` SHA256
`da47fce95f5f9b29eecf88a57748dc3f9733c51ed074d03ecb5ab0696bb26346`
supersedes earlier *authorization pending* status; old consumed attempts stay closed.
It does not supply the missing off-server key custody identifier or external/manual
writer facts, and does not waive actual Owner use or three actual trading days.

Necessary local validation is now executed: all58 existing synthetic component
cases passed with an audit hook denying subprocesses/network/production paths.
Receipt `authorized-component-tests.json` SHA256
`15ae6a2be7ff47300e384525901e63fb5444fb7b7cf28beccd7c5ead9bcd871d`.
A coordinator copied a transport pin incorrectly before that run; the byte check
failed before any imports/tests. The exact sealed pin was recovered and verified.
No accepted source changed and no failed result was promoted to success.

`g4-fixed-command-runner.py` implements only the32 exact command entries in the
existing34-action inventory; its two local boundaries are not shell commands.
It publishes an exclusive intent through its required adapter before spawning,
retains the same child across observation deadlines, never retries/kills a stop
client and records only complete stream hashes/counts. A fresh exact-target stop
observation plus the original successful client response is required for STOPPED.
Terminal receipt failure preserves the actual reaped result in memory. Fresh
admission, ready-before-outage and drained-main gates remain mandatory adapters.

The final21 tests passed, including four actual unprivileged Python fixture children
for pipe draining, a late original response, nonzero exit and over-limit output.
All four children were reaped. No Docker/systemd/root/SQL/provider/production-path
operation ran. Receipt `fixed-command-validation.json` SHA256
`b3f306c2bd206b410e6fe3960eb25a9efdd2c3548317b7cc5fe20746993a080d`.
These tests do not establish integrated recovery, actual stop semantics or full
G4 admission. Four earlier full-controller findings remain OPEN. The runner has
no operating entrypoint; full admission/READY/local permission-change/restoration
connection and end-to-end fault validation still need completion.

Native Ralph was observed blocked and no resume API is exposed. The same-agent
heartbeat remains continuation tracking, not native Ralph or outage recovery.
Completion by the next question cannot substitute for deployment followed by
three actual trading days. No new approval or information question was sent.


## Fixed sequence, held-file operations and persistent recording failure

The34-action sequence is now connected to the fixed runner, with the15 restores
limited to targets whose original successful stop response and fresh observation
were recovered. Main failure/deadline proceeds to drain and original-state
restoration; unresolved stop responses still prohibit starts. Sixteen integration
cases PASS using simulated Docker/systemd/native readiness. Receipt
`fixed-sequence-validation-a1.json` SHA256
`6d942da44fa910ddf6366e96aaad8a12f0cea3ced3c89a57fb3e7cee5b4bcb41`.

`g4-fixed-local-operations.py` supplies exclusive, held-parent file publication and
the exact held PG parent0701→0700→0701 transition. A failure after chmod is resolved
from the actual held inode/mode. Partial files remain untouched and cannot be
retried. Sixteen tests exercised actual unprivileged temporary filesystem effects;
root admission was tested only for rejection, with explicit private-owner adapters
for the positive cases. Receipt `fixed-local-validation-a1.json` SHA256
`4741753747131f8cc8f6d1fafd83a8a508980310d3e1a9ed67a4437cdac0c4cc`.

Integration review identified a remaining write-failure path: making a new restore
intent depend entirely on a successful disk write could prevent original-service
resumption during persistent ENOSPC. `g4-fixed-restore-journal.py` now publishes
15 exact contingent restore intents **before any outage**. They do not claim an
actual stop/start. Only within the original same-process/same-boot deadline, after
verified stop and fresh native drain/parent/timer prerequisites, may a restore use
its existing durable conditional intent when the new record fails with
ENOSPC/EDQUOT/EIO/EROFS. The actual per-dispatch intent remains in RAM; evidence is
marked incomplete and capture cannot be accepted. Integrity failures such as
EEXIST/owner/name drift are never treated as capacity exceptions. No command is
replayed and these files cannot resume a lost recovery process.

Ten combined tests PASS with real private journal files/permission changes and
simulated daemon/process gates, including persistent recording failure, untouched
PG exclusion, missing/tampered reserve rejection and retained partial chmod.
Receipt `fixed-restore-journal-validation-a1.json` SHA256
`7ed3274e700349d184ce7926b13027fcd17322e5097c0753129b7f4705e1c246`.
The prior79 tests were not repeated; this continuation added42 distinct cases.
No root, Docker, systemd, SQL, provider or production-file operation executed.
Accepted app/tool source and evidence remain unchanged.

There is still no production entrypoint: fresh host admission, actual independent
recovery readiness/requester drainage, protected contract/official-result binding
and final native closure must be connected. The four full-controller findings
remain OPEN until integrated native validation; no independent review is claimed.
Owner-wide authorization remains effective. The existing custody/external-writer
information question is pending without repetition. G4/deployment/Owner use/three
actual trading days/C_R2 remain open; the current heartbeat is1b9f4608, not the
superseded e3e092a9 snapshot that remained in two registry fields.


## Native restore observations and official result recovery

The fixed restoration adapter now checks the original stop response, requester
terminal generation and empty recursive cgroup, exact recovery generation with
only its own PID/thread, retained-volume references and the full13-container
inventory before starts. It restores timers only after all10 original services
are healthy. Final checks preserve the56 historical units and distinguish the
still-running recovery process from its required later external terminal proof.
Twenty-three focused cases PASS with simulated observations and actual private
files through explicit cgroup-directory/owner fixtures. They are not actual
kernel cgroup, root, Docker or systemd verification. Receipt
`fixed-native-restoration-validation-a3.json` SHA256
`7fceabd27c957ee68df7ff633aaee9e86cfd383772861c70518d3753a252073c`.
The first two receipts remain failed: missing explicit mock method and inherited
group-writable fixture directory. Only fixtures changed; runtime guards were
not weakened. No operating action was attempted by either failed test run.

`g4-fixed-official-results.py` pairs the original successful audit/capture child
response with the fresh protected audit receipt and exact published COMPLETE,
metadata and encrypted-file hashes. It reuses accepted790f1ea contract/audit
validation. It does not run another audit, private PG restore or decryption;
the accepted successful capture performs those checks. Missing/changed evidence
rejects capture while the existing sequence proceeds to original-state recovery.
Eleven focused cases PASS using real temporary files, accepted pure validators,
and explicit root-owner/path fixtures. No actual PG/capture/encryption ran.
Receipt `fixed-official-results-validation-a1.json` SHA256
`55f40715e998ab269b4274838514fc7122b21fa2160a0bf8aa75d2d85ca42caf`.

The prior121 passed cases were not repeated. All12 accepted tool payload files
still match their sealed bytes. No production/root/Docker/systemd/SQL/provider
operation or accepted app/tool source change occurred. These are callable native
adapters, not a complete production entrypoint. Fresh admission/protected loading,
actual independent READY/requester release, fresh contract construction and an
external recovery terminal collector still need integration. Four full-controller
findings remain OPEN, with no new independent review. Blanket authorization
remains effective; the existing factual custody/external-writer question remains
pending without another question. G4, deployment, Owner use and three real trading
days/C_R2 remain incomplete. The coordinator's unfinished integration is also a
cause of delay; it is not described solely as awaiting Owner input.


## Recovery readiness and requester release

`g4-fixed-ready-release.py` connects the concrete resource checks to one
authenticated AF_UNIX exchange. Recovery holds the singleton, original monotonic
watchdog, held parents and durable contingent restore intents, and freshly checks
the original13 containers/56 units and requester/recovery generations before
publishing READY. The requester acknowledges the exact receipt once and exits0;
recovery observes that exact successful terminal and empty cgroup before release.
All32 command children belong to recovery. Requester exit does not cancel or lose
a later Docker stop client. Socket connection, Type=exec active, ACK or an intent
alone never authorizes outage. No reconnect, command retry or deadline renewal.

Eleven new tests PASS:9 resource/error gates and2 actual unprivileged requester
processes using kernel AF_UNIX peer PID/UID/GID evidence. Root ownership, systemd
generations and cgroups are explicitly simulated; this is not actual production
readiness. Both private children were reaped. Receipt
`fixed-ready-release-validation-a1.json` SHA256
`73a245c8e9c2f57859d69bbca361283fe62898de991c4490c8097bd05c42d592`.
The prior155 cases were not rerun. No accepted source/evidence or production
resource changed, and no operating command ran.

The fixed packet still needs protected admission/loading, complete fresh official
contract construction, the operating entrypoint and external recovery-terminal
collection. The full pre-outage READY check requires phase NEW and unchanged
original resources; it must not be reused as the local-operation gate after stops
have occurred. Subsequent main gates must check the actual witnessed stopped
state, retain the original window and use same-boot restoration on failure.
The four full-controller findings remain OPEN. This continuation does not mark
G4, deployment, Owner use, actual three-day observation or C_R2 complete. Existing
blanket authority remains effective and the one custody/external-writer factual
question stays pending without repetition.


## Fresh official contract and fixed operating connection

`g4-fixed-contract.py` assembles the accepted maintenance-v3 contract after
actually witnessed writer/timer stops and held-parent protection. It obtains
raw container metadata only in RAM, preserves actual one-way activation fields
and retained disabled failures, and freshly observes PostgreSQL volume CreatedAt
after the parent mode change while requiring all immutable volume fields unchanged.
Its three additional read-only argv are the exact PG-volume inspect and the full
unit-file/unit union; they share the original observer's pending-child tracking.
No lifecycle argv can enter that query path. The contract pins exactly the required
protected paths and the12 accepted790f1ea payload hashes. Missing custody reference
is rejected; an input hash is not itself evidence that the Owner supplied facts.
Fresh protected admission must authenticate those actual inputs before construction.

The base inventory reuses the accepted scanner on the held original base root.
It does not create another backup/restore verifier. The official capture still
performs its complete encrypted five-class backup and actual private PG restore.
The generated contract keeps the original main deadline instead of opening a
new25-minute window. Source migration pins are expectations; the official read-only
audit remains necessary to establish actual old database migration state.

`g4-fixed-operating-connection.py` composes the existing fixed runner, sequence,
local operations, conditional journal, concrete readiness and restoration adapters,
fresh contract builder and official-result reader. It distinguishes the NEW-only
full READY check from subsequent live recovery gates after resources have stopped.
Every dispatch requires the exact preceding step gate and fresh target/liveness
checks; restore never reuses main wall-clock/capacity admission. Contract failure
after chmod leaves that actual mode change for existing held-FD recovery. The
callable connection is not yet a protected host bootstrap or complete packet.
Its result explicitly reports a still-pending command client and the still-required
external recovery terminal proof; a future bootstrap must not exit/kill an owner
with an unresolved stop client or label such a deadline case successful.

Eighteen focused cases PASS, receipt `fixed-contract-connection-validation-a2.json`
SHA256 `5c8a926198da2db388b7b3a481ae7365893064e97ae8cf1ea4dd64fab67e0adf`.
The initial16-case receipt e0cb6f54 remains failed: the pin-drift fixture changed
a detached input dictionary rather than observed bytes. The fixture was corrected;
two tests were added for the explicit fixed pin surface and immutable tool hashes.
The tests exercise accepted pure schema/scanner code and real temporary base files;
root, protected host reads, Docker/systemd generations and command effects remain
explicit mocks. No child, network, production-path or privileged operation occurred.
All12 accepted tool payloads still match their sealed bytes. Prior166 tests were
not repeated, no old CI/host attempt was rerun and no accepted evidence changed.

Remaining: complete protected loading/fresh admission and actual custody/external
writer facts, external terminal collection and full assembled integration including
query-budget/slow-health and unresolved-client deadline paths. All four original
full-controller findings remain OPEN until their integrated evidence exists.
There is no new operating admission or dispatch. G4/deployment/Owner use/three
actual trading days/C_R2 remain open, under the existing blanket authority.


## Composed recovery validation and pre-dispatch state correction

The actual `OperatingConnection`, fixed sequence/runner, native restoration,
contingent journal, READY resources and post-stop contract builder were exercised
together. Kernel observations, protected IO, READY transport and official child
results are explicit in-memory fixtures; this is not actual G4 or backup evidence.
Eleven cases PASS in `fixed-composed-validation-a4.json`, SHA256
`66cb74be3ecb68ce807e244067efb6baebc0e7484026a37d80a53c33d777097d`.
They cover the32-command order, partial main intent, wall drift after a stop,
capture failure, contract failure after protection, storage failure during restore,
unresolved original stop at the shared deadline, delayed health, final result
publication failure and the exact boundary before process creation.

This composition found a real defect in the prior private runner: during a
restore intent write failure, its key was already consumed but had no terminal
record. The journal's fresh drain gate consequently classified this not-yet-launched
restore as unresolved and prevented restoration. The original runner remains
unchanged. `g4-fixed-command-runner-v2.py` records the true in-process
NOT_DISPATCHED state while publishing intent, then removes that record immediately
BEFORE Popen. An ambiguous launch therefore cannot retain a false non-dispatch
claim; it remains consumed and cannot be retried. Existing stop-response, target,
drain, identity, deadline and no-kill guards are unchanged.

The original21 runner tests passed against v2, including four actual unprivileged
Python fixture children, all reaped. Receipt
`fixed-composed-validation-runner-v2-regression.json`, SHA256
`3fb5d30223a59c8ffd879045138b0be450e1a6b572f5efb64467785b565790c5`.
These21 are repeated regression coverage, not21 additional distinct cases. The
current distinct total is195 (166+18+11). Initial import rejection, a1's explicit
mock-method setup error, a2's actual restoration defect and a3's eight passing
cases are retained. The final test/runner pin set is in the a4 receipt.

The fixture query estimator counted3346 reads for immediate healthy recovery
and3946 with60 extra health observations per restored container. These are
estimates, not host measurements or a worst-case guarantee. The4096 bound was
not raised. Fresh admission shares this observer budget, so admission reserve,
slow-health bounds and duplicate gate cost must be resolved before host execution.
The shared-deadline case correctly remains INCOMPLETE with the original stop
client handle retained; protected bootstrap must still preserve that owner and
collect its actual eventual terminal without replay, renewal or forced kill.

Protected fresh admission/loading, actual Owner custody/external-writer facts,
operating bootstrap and external recovery-terminal proof remain unfinished.
The four full-controller findings remain OPEN; the storage failure now has
positive composed-fixture regression evidence but no actual host integration.
No operating attempt, root query, lifecycle action, provider call or accepted
app/tool mutation occurred. Blanket authority and the existing pending factual
question are preserved without another request.


## Recovery query reserve and external terminal reconciliation

The4096 read-query limit remains unchanged. Fixed contract/operating/native v2
files preserve their v1 originals. Two exact `systemctl show --all --property=...`
commands read all56 admitted historical units, grouped only by service/nonservice
property schema. Missing, duplicate, extra or malformed unit/property blocks reject;
there are no synthesized reverse edges or filtered historical failures. The
[official systemctl documentation](https://www.freedesktop.org/software/systemd/man/latest/systemctl.html)
documents multiple explicit unit names, `--all` and property selection for `show`.
Installed-host output still must pass the strict parser before any outage.

READY must leave its observation count at or below512 before creating the release
lock or stopping anything. MAIN checks a256-read step allowance and reserves2300
reads for restoration. The immediate dispatch gate rechecks that reserve. A
restoration dispatch consumes a same-process drain gate no older than5seconds and
rechecks the actual target/parent/current/health; it does not spend another full
duplicate drain. A fallback journal write still performs its own fresh drain.
Health polling waits3seconds and permits at most900 waits across all original
starts, within the same original2700-second deadline. No limit, window, start
retry, forced-kill allowance, target or guard was broadened.

`fixed-query-reserve-validation-a2.json` SHA256
`e310ad39e6b43d1f9d29d6a13772f215379d31664252e11f42f784b9af23690a`
records26 PASS:15new cases and11 prior composed/boundary cases rechecked. Fixture
read counts are1930 for immediate health,2530 for600 additional health reads,
3239 after storage failure with900 waits, and3627 for the maximum512 READY cost
plus capture-terminal storage failure,900 waits and15 contingent restore intents.
Excess READY cost rejects before any command; exhausting MAIN reserve restores
only actual prior stopped targets. Exceeding the health bound remains INCOMPLETE.
These are explicit fixture counts, not host measurements or an unbounded
worst-case proof. The earlier25-case a1 PASS receipt remains preserved.

`g4-fixed-terminal-collector.py` now provides the read-only external reconciliation
method. It requires an external process, the exact original MAIN/recovery
generations, both actual successful terminals and recursive empty cgroups, the
original boot/shared monotonic deadline, and the protected result's exact hash.
All32 intent/terminal pairs are checked in fixed order with attempt/argv/intent
bindings, successful original responses and finite stream hashes. It reuses the
protected member reader, the accepted official contract/audit validator and native
state checks. Audit time must belong to the original contract window and boot;
there is no fresh audit or SQL. It hashes the existing encrypted publication
without decryption, reobserves the13 containers/56 units/current link/retained
volume/PG parent, then checks both terminals and small protected records again.
Missing evidence, pending clients, failed recovery, changed publication/state,
self-certification, deadline expiry and a second collection attempt reject.

`fixed-terminal-collector-validation-a2.json` SHA256
`a1e88ae97328f4ab5b11329f95d01dd10f1dfc97395f912e6e797a0671102327`
records23 PASS, including the accepted actual pure audit-schema validator against
synthetic database receipts. The earlier20-case a1 PASS remains preserved. Host
observations, protected IO, cgroups and publication bytes are explicit fixtures;
no actual root, Docker/systemd, SQL, provider, process or backup operation occurred.
The collector has no protected execution entrypoint and does not itself prove
overall operating admission. No new private child execution occurred this turn.

Distinct private cases now total233 =195+15+23, not195+26+23. The12 accepted tool
payloads and accepted app/source/evidence remain unchanged. Remaining engineering:
protected source loading/fresh complete admission, the actual bootstrap and
retention of recovery ownership while an original stop client is unresolved.
Actual off-server custody/external-writer facts remain pending in the original
information question. All four full-controller findings remain OPEN until the
complete protected integration is validated; G4 operating execution remains0.
Blanket R2 authority remains granted. No repeated approval/question, CI run or
host attempt. Then G4/old-service closure, backup transition/deployment, Owner use
and three consecutive actual trading days remain necessary for final C_R2.


## Protected fixed source and operating inputs

The new private `g4-fixed-source-input-loader.py` preserves the earlier
source-only loader and its receipts. All14 current controls and12 accepted tool
payloads are checked before any project module executes. The loader compiles the
exact verified bytes in memory, restricts explicit local imports to that fixed
bundle, and rechecks every file afterwards. It requires a root isolated Python
process, fixed path, protected ancestors, root0600 single-link regular files,
no-follow/ACL/mount/name identity checks and a separately supplied self hash.
The accepted maintenance source is unchanged. This is not a sandbox for arbitrary
code, interpreter attestation or a production launch.

The input function reads only seven fixed protected files. It binds the existing
blanket authorization, an actual Owner-information receipt, fixed scope/sequence,
original instances and contract input hashes to the loaded controls and original
boot/window. The current pending question or blanket approval cannot substitute
for an information reply. A declared absence of external/manual writers remains
an Owner assertion, not a global writer fence. A present writer requires an
actual fence; no writer absence, custody identifier or answer receipt was created.

Original observation age is at most60seconds within the original window. The
read-only path compares all13 containers, all56 units and their protected
fragments/empty service cgroups, the current link, retained-volume metadata and
held PG-parent identity. Inputs are read again afterwards. Its short120-second
cap and128-query ceiling do not renew the original2700-second window: the same
observer and consumed query counter pass to later gates. No service command,
lock creation, key content, chmod, Docker volume content or staging is performed
by this input reader. The official post-stop contract still performs its full
schema and runtime validation; input binding alone is not operating admission.

`fixed-source-loader-validation-a2.json` SHA256
`bcf1ebe65fd7585231f48b4d171aba4fb241597dfbe0b0271165c33567f7baf4`
records18 PASS, with actual in-memory imports and private filesystem cases. The
initial17-case a1 failure is retained: the test umask made intermediate fixture
directories group-writable, so the real guard correctly rejected them. Only the
fixture setup was corrected; the production guard was not relaxed.

`fixed-source-input-validation-a1.json` SHA256
`d3eea499fa305d1c0dc09666a54f0b49f588d208259dbd33df802d56d610364d`
records40 PASS:18 source cases rechecked against the extended loader and22 new
input cases. They include wrong/missing factual bindings, duplicate JSON,
replayed input attempts, stale originals, changed current/container/unit state,
added units, wall drift, reread drift, pending queries and budget exhaustion.
Host/root/cgroup/mount observations are explicit fixtures. No production path,
child, network, Docker/systemd, SQL, provider or actual backup operation ran.
Distinct cases total273 =233+18+22; the repeated18 are not counted twice.

Protected source loading and input binding are implemented and locally tested;
complete operating admission and the actual requester/recovery/collector launch
remain unfinished. The recovery owner must still retain any unresolved original
stop client without kill, replay, renewed lifecycle authority or false terminal
closure. Four full-controller findings remain OPEN until protected integrated
validation. Actual custody/external-writer facts remain pending in the existing
question. G4 operating execution0; accepted app/tool evidence and blanket R2
authority remain unchanged. G4 closure, backup transition/deployment, Owner use,
three consecutive actual trading days and final C_R2 remain open.


## Original command ownership through incomplete termination

The current fixed runner/connection are v3; v1/v2 sources and receipts remain
unchanged. Recovery's run_once installs SIGINT/SIGTERM handlers before setup.
The handlers revoke MAIN without signalling the original child or raising a
reentrant exception. Within the original window, an outstanding stop response
is reaped and its exact target observed before restoration. A signal during
intent publication consumes that intent without launching its command.

If the sequence becomes INCOMPLETE with an original client still pending,
run_once and its finally path retain ownership. They perform bounded nonblocking
reads/poll on those exact existing handles until the client is reaped and both
pipes reach EOF. No new command, target query, receipt write, kill, replay or
renewed lifecycle window occurs in that passive hold. A lost pipe is not EOF.
The late RAM record is CLIENT_REAPED_AFTER_INCOMPLETE, cannot satisfy the normal
stop/closure validators, and permanently closes dispatch. INCOMPLETE stays
INCOMPLETE. This does not protect against a hard process/host crash; actual unit
startup, force-kill policy and external terminal recovery remain to be verified.

`fixed-retained-client-validation-a2.json` SHA256
`efd8c04675c365f4d8021ba885af27f1fd2504777521192715ec09f29e691cef`
records61 PASS:14 new cases and47 prior runner/composed cases. The old stalled
mock is released only after two passive pending observations; it is the SAME
original mock child and no later command is dispatched. Three new cases use
actual unprivileged Python children (partial output plus self SIGINT/SIGTERM,
setup failure/finally drainage, over-limit output/nonzero exit). Four prior
private-child cases also reran. All seven children were reaped. a1's missing
fixture attributes caused one error; its exact test source and receipt are
retained. Only the fixture was corrected; no runtime guard was weakened.

`fixed-retained-integration-validation-a1.json` SHA256
`067962fab7d371dd41ec4147224dd11088f0db4890725c6a5337c0b58e4d718e`
records four additional composed cancellation cases PASS: signal after a stop,
during the next intent, while the original response is pending, and listener
setup failure. The real sequence/restoration code runs against explicit mocked
root/daemon/host state. Self signals are actual; no host lifecycle was invoked.

`g4-fixed-source-input-loader-v2.py` pins these v3 controls. Its only changes are
the two filenames/hashes and its own fixed path. All26 source pins and14 roles,
seven input paths and previous guards remain. Its40 regression cases PASS in
`fixed-source-input-validation-v2-a1.json`, SHA256
`dbd2cd0151c049cd9616e6a5610764b3d14af2cb9d19872610c1c6132c4647f3`.
Distinct cases total291 =273+14+4; the87 repeated cases add no distinct count.
Including the retained a1 run, this change made14 finite private child executions;
these are not Docker/G4 execution or independent recovery validation.

The operating entrypoint/full fresh admission and actual unit startup/terminal
integration are still unfinished. Actual Owner custody/external-writer facts
remain pending in the existing information question; no answer is synthesized.
All four full-controller findings remain OPEN. G4 actual execution0; accepted
app/tool source and evidence remain unchanged. Blanket approval persists.


## Fixed file preflight before host observation

The protected input loader v3 adds one mandatory file preflight before creating
the live observer. Its14 source roles,12 accepted payloads and seven fixed input
paths remain unchanged. The accepted PostStopContract constructor checks the
fixed input surface; accepted root/file readers check protected hashes and all59
old UP migration bytes. Unexpected newer UP files, missing/duplicate versions,
changed bytes, mismatched unit fragment pins and unsupported custody-reference
syntax are rejected before any operating action. No accepted tool source changed.

First-four-root metadata uses the accepted held-root reader. Original PostgreSQL
parent0701 and its exact _data identity are checked with the existing held-parent
guard; the capture reader's parent0700 requirement is unchanged. The key path is
opened for metadata only, without read/hash/mmap of key bytes. The fixed backup
directory must have the configured minimum free bytes. These live metadata/free
space observations are not a consistent backup, full content-size estimate,
valid-key check, off-server custody proof or global/future writer fence.

Current loader SHA256 `15278ae0c8b3ecf3abe1d08cba083530598eff474b1a32429ad473723988c965`.
`fixed-file-preflight-validation-a2.json` SHA256
`7d73605d7b826e9166150a20a06ac4a4f82d31cd8aeb80c336ea65b746b4181b`
records58 PASS:16 actual private-file/accepted-reader cases,2 mandatory connection
cases and40 source/input regressions. The latter use an explicit preflight stub;
the separate16 cases execute the actual preflight against private byte fixtures.
Root ownership, mounts, root paths, space and host queries are explicit fixtures.
No child, production path, daemon, SQL, provider, key read or lifecycle ran.
The original harness misinterpreted dir_fd-relative names under mocked cwd,
causing25 errors in its58-case run; its receipt and command are retained. Only
the harness's validated private-dir_fd mapping changed, not runtime/test source.
The16 exact owned temporary fixture directories left by its cleanup errors were
removed and recorded. Distinct cases309=291+18; repeats are not recounted.

Full fresh admission still needs the fixed historical baseline, runtime pins,
complete capacity/input/fence checks and actual requester/recovery/collector
startup/terminal composition. Four full-controller findings remain OPEN.
Owner information remains pending; blanket authority persists without a repeated
question. G4 execution0, deployment/Owner use/3actual tradingdays/C_R2 remain open.


## Fixed requester/recovery process body assembly

Protected loader v4 caches only its exact loaded input object for one role body.
The role checks original clock/query budget and distinct actual PID generations.
The requester uses the existing READY exchange and returns exit0 for immediate
termination; recovery still independently proves its actual successful terminal
before outage. The recovery body owns held parents, the existing singleton and
original monotonic watchdog, then the existing OperatingConnection. Its finally
retains the original pending client before resources close in reverse order.
No command, retry, deadline, accepted payload or file-preflight guard changed.

A successful body return requires the exact published result bytes, RESTORED,
captureAccepted, originalServicesRestored, evidenceComplete and no main failure.
Restoring services after a failed capture or missing result publication returns
nonzero. Actual successful independent unit terminal and external collection are
still required; a Python function return does not prove those facts.

Loader SHA256 `51ca4eace79397eaff1706d6f2f235b57511ea5fea3a788d957b429df5a5f039`.
`fixed-process-body-validation-a1.json` SHA256
`758d09f77386427a508cefbb9952a6a0ce9f0fdc32608d145b43d5a50dc9b967`
records56 PASS (15 body cases,1 exact input-cache case,40 source/input regressions).
The body cases use actual private descriptors/files and three actual unprivileged
children, all reaped with expected exit0/0/2. Root, host, READY and the connection
are explicit fixtures there. `fixed-process-body-composition-validation-a2.json`
SHA256 `61118b70d22d448a4b3952f32315180cfa7eae5f460a15c7b7de25242eb87810`
adds two PASS cases using the actual existing connection/runner/sequence/contract/
journal/restoration with daemon/READY/publication fixtures. The first composition
run lacked postgresParent in its old fixture; both errors and exact source are
preserved. Only that explicit fixture field changed, not runtime source.
Distinct tests327=309+18; the40 regressions do not increase that count.

`process-body-source-reconciliation.json` SHA256
`66353f6e177ef380037efb76feede4e59f4053addef14c3b02b24b15e1a157c5`
proves the complete inverse to loader v3 and unchanged26 sources/14 roles/seven
inputs. Bodies remain callable private assembly, with no operating CLI or full
admission. Full fixed historical/runtime/capacity/fence checks, actual unit policy/
startup and external terminal wiring remain unfinished. Four findings OPEN.
G4 host execution0; no staging, outage, SQL, provider or key-content access.
Owner custody/external-writer information stays pending without another question.
Blanket authority remains granted; app/tool acceptance and retained volume stay
unchanged. G4, deployment, Owner use,3actual tradingdays and C_R2 remain OPEN.


## External terminal collection connected to the original input owner

Loader v5 adds collector_body_once after the preserved requester/recovery bodies.
Only the original process that loaded the exact protected input object can use it
once. It retains that observer/query count and the original2700-second deadline.
It reads the fixed protected READY record, binds both process generations and
the original snapshot, then invokes the existing TerminalCollector. That existing
collector checks actual successful independent terminals/cgroups,65 command/result
records, official publication and restored state. The body finally rereads READY
and all seven input pins, without restarting services, running another audit,
renewing a window, or claiming its own terminal/publication.

Current loader SHA256 `15ac89737e9be6d1962592b8cc5a8d9bed9b57be44ff084f8c12b4753cd17dcd`.
`fixed-external-collector-body-validation-a2.json` SHA256
`868e7f00d1daad434f5f361e55efff56761be5ececac91a6013e4521893b0e67`
records10 PASS:8 new collector-body cases,1 original input-owner binding and1
source-import regression. The actual existing collector and official pure receipt
validator run against explicit host/root/READY/file/publication fixtures. Tests
cover collection after the MAIN deadline within the original shared window,
failed independent terminal, self-generation rejection, snapshot/READY/input
drift and expiry. No new private child or production action ran. Distinct336.

The first harness imported a fixture that registered official modules before
source-loader tests; those two checks correctly rejected collisions. Its receipt
`2138cb28e1fcdc250a4b7446272327a428b84487f77c7396beb51c65e2050b69`
and command are preserved. Only harness ordering changed; runtime and test source
are identical between runs. Source reconciliation `7c280b1cfa4a3197692f1ba45b25d0f9d3629192072c8071607a8a052f35ea3d`
proves inverse v4 bytes and unchanged26 sources/14 roles/seven inputs.

This body depends on the original external input owner remaining alive; it does
not implement fresh-process resumption, a full admitted launcher, durable result
delivery or actual unit startup. Historical/runtime/capacity/fence admission and
actual Owner facts remain open. Four findings OPEN; G4 host execution0. Blanket
authority and the single pending information question remain unchanged.


## Actual runtime and retained coordinator terminal — 2026-10-20 KST

An unprivileged fixed observation recovered Python3.14.4, Linux x86_64 with
8-byte long/pointer ABI,72 public executable/mapped-library/loaded-stdlib files,
and one actual private absolute monotonic timer expiry/close. Receipt
`runtime-observation-a1.json` SHA256
`4446c575f11954169a2366ecf68c65cc5faf57123167d51dc480fa02d7c93737`.
No daemon, child, secret or production data was accessed by that observation.
This is not a complete future-import inventory or a fresh root admission.

Two distinct, bounded USER-manager fixture services each ran once. The first
proved main-only SIGINT and the child's normal exit, then exposed immediate
transient-unit garbage collection. Its absent unit cannot establish a retained
InvocationID. Closure `user-unit-policy-fixture-closure.json` SHA256
`38ea5131fff3d443180357fbdb36ee012b70e631270a8b5836b060634925fab8`.
The second uses RemainAfterExit=yes and signals only the main process. It recovered
the same InvocationID, active/exited, normal exit0, ordered execution timestamps,
PID0 and absent cgroup twice, persisted that proof, then stopped only its own
completed fixture. Both parent/child processes and cgroup were absent afterward.
Closure `user-unit-retained-terminal-closure.json` SHA256
`46d7ce20e58328b480718a934920ca9ed5c16f3e10f18e26a3cecfb76b5a165b`.
No failed historical unit was reset, deleted or restarted. These are actual
unprivileged Python fixtures, not root G4 or Docker recovery.

Observer v4 now requires the fixed lifecycle policy only for our two G4 units;
the56 historical schemas and per-observation query count are unchanged. Requester
and external collector require retained normal execution termination, both PIDs0,
no job and the existing recursive cgroup proof; active/exited alone cannot pass.
Loader v7 SHA256 `84d78314dfe3c37de2050c2e9d18d463853bd1490b5c849ffac7ef597cf14aed`
binds those three changes. All other23/26 source pins, all12 accepted payloads,
seven inputs and original1500/2700-second deadlines remain unchanged.
Six focused policy cases passed (`f506041c4c18aeed772c02fba39259cfe4875a36516a8c74544490f350722a87`);
four retained-terminal cases, those six changed-policy regressions and one source
import regression passed (`2f0af5d83a75780c67f0988ec923b1163d0f0c85463b1e8b8d97a78f70450d42`).
Root/host/cgroup are explicit fixtures in that suite. Complete inverse bytes and
scope are recorded in `native-policy-source-reconciliation.json` SHA256
`c93eb8aba4c39105e0d2be8066bbe468970a3b2a47c8af758be4b1113ee640cf`.

The policy follows systemd's documented [main-only initial signal and disabled
final kill](https://www.freedesktop.org/software/systemd/man/latest/systemd.kill.html)
and [retained active/exited state](https://www.freedesktop.org/software/systemd/man/latest/systemd.service.html).
Infinite outer runtime/stop limits retain unresolved original clients; they do
not renew the internal operating window. Cooperative cancellation must target
the exact main process. Stopping the recovery unit itself puts it outside the
current active-generation guard and is not a validated recovery path. Do not
stop retained G4 units before independent terminal evidence is durably recovered.

Full historical/runtime/capacity/fence admission, actual fixed launcher and durable
external delivery remain unfinished. Owner custody/writer facts remain pending;
blanket authority is already granted. Four controller findings OPEN, G4 execution0,
app/tool acceptance unchanged, deployment/Owner use/three real tradingdays/C_R2 OPEN.


## Historical baseline binding before operation — 2026-10-20 KST

Loader v8 binds the exact a4 terminal bytes as the eighth protected input
`historical-survey-a4.json` (SHA256
`94f4c97f4cd427839c4a81df7cfd0771d8deb6a0cfd6da6c6f87ac33d3052cac`).
The old observation remains collected-with-gaps and acceptedObservation=false.
It supplies historical facts, not current operating admission. The copy is local
and inert; no root staging, production observation or outage occurred.

Fresh input records must preserve all56 units and13 container identities. Unit
fragments, six fixed protected files, PG volume/path and ancestry identities,
five root identities and key metadata bind to the preserved observation. New
snapshots cannot redefine these baselines just by agreeing with each other.
Four previously executed periodic services may supply newer completed invocation
timestamps while preserving policy/state/actual edges. The never-run revocation
service and all other historical generations remain pinned. Old unprinted Id,
Result and exit fields are not backfilled; fresh Id must match the old map key.

The actual existing inspect query supplies the legacy container policy digest
(full NetworkSettings, compact JSON without newline) before outage. The existing
restart-safe policy grammar remains unchanged for later restoration; fresh
endpoints/timestamps must not be misrepresented as unchanged. Retained-volume
metadata remains unproven ownership with no references or content access. The
external collector rereads all eight admitted inputs after terminal collection.

Current loader SHA256
`282dffdee13119720b3b25e7a47fb3eeaa6d73f253b802d5f2e042493d950d5e`.
`historical-baseline-validation-a2.json` SHA256
`f220bf10bc34b7558c4a14aca7bb48f9cb1d95304cc562496f59495cfbf18b1d`
records22 new cases plus1 actual source-import regression. Historical bytes are
actual; fresh root/files/host inputs remain explicit fixtures. The first run
exposed that the old survey omitted the Id property; its failed receipt and
source are retained, then the new schema bound fresh Id to the historical key.
`historical-collector-validation-a2.json` SHA256
`07f626277d9c27ebe32e3b6a961c1d6436f8834257c82c476d8d9499e0e78ebd`
records3 new input-preservation cases through the current collector-v2 predicate.
The earlier harness replaced READY inside an older composed fixture too soon;
that failure is preserved and fixture ordering/partial cleanup corrected. This
is not a fresh all-components or actual G4 execution test. New25, distinct373;
no child/daemon/root/network/protected production path was used by these suites.

Source reconciliation `historical-baseline-source-reconciliation.json` SHA256
`fa20ad06598c6326bc528f141080c385f423e7930a252a51a2377592457ff551`
confirms all26 source pins/14 roles/12 official payloads unchanged. Only the
loader's two connection functions and four pure baseline checks changed; all
other prior loader functions are unchanged. Runtime/capacity/writer fencing,
actual launcher and durable external delivery remain required. Owner blanket
approval persists, the existing information question stays pending, all four
controller findings and R2 completion gates remain OPEN.


## Wave114 external collection publication — 2026-10-20 KST

R2-04/R2-09 now has `collector_delivery_once` in loader v9. It takes only the
original in-memory context and calls the existing collector itself; a supplied
success flag or result cannot be published. The prior collector, all previous
loader functions, the26 dependency pins and12 accepted official payloads are
unchanged. Its result includes the two actual unit terminals, restored original
state,65 evidence records and reread8 inputs before reaching publication.

The new boundary reuses the existing fixed local writer, at only
`fixed-external-collection.json` under the held auxiliary directory. Exclusive
creation,0600/single-link metadata, file and directory fsync, and readback precede
a successful return. Failed or partial writes remain at their exact name and
consume the delivery call; no repair, unlink or retry is introduced. The same
original2700-second deadline applies through the final readback. The result
explicitly still requires this external collector process's actual successful
terminal; file existence alone is not G4 closure or admission.

Loader SHA256 `84cc0000fb490b97c73e894f16d79025ab738121962a2bce651bf644144fc7be`.
Validation `collector-delivery-validation-a3.json` SHA256
`0555e0037f4af8ac35a2d584a53c17787209344b1d83b3f05eebc6a9ad9d6550`:9 new cases PASS, with real unprivileged temporary-file
open/write/fsync/readback. Host/root/backup and unit terminals remain explicit
composed fixtures. The first fixture run hit tempfile's default-directory probe;
the second inherited a stubbed os.read from the command fixture and expected an
older terminal error label. Both failed receipts and test sources are retained.
The fixture now selects /tmp explicitly, confines write dir_fd to its private
folder, and uses the captured real read syscall for evidence. The loader source
did not change across these runs. The1 source-import regression passed in a1;
it is retained separately from that run's failed delivery tests. Distinct382.

Source reconciliation SHA256 `51d78e9ccbbb5e8e345d94fb0178037b942a13d75de553919d00975702da077c`. No new child, daemon,
root staging, production observation, backup or lifecycle action occurred.
The original input-owner process still must stay alive. Full native admission,
actual fixed process startup, transport of the saved receipt and independent
collector-terminal recovery remain unfinished. The initial intention to finish
the launcher in this continuation was not achieved; no admission guard was
bypassed to make it callable. All four controller findings remain OPEN. The
blanket authorization and unanswered information question remain unchanged.


### Wave114 — sequential process startup boundary (2026-10-20 KST)

R2-04/R2-09 loader v10 and observer v5 fix a concrete integration race:
the first of the two sequentially started services previously required both
services to be running immediately. The body now binds its own actual process,
then observes only the exact peer name through a bounded read-only list-units
query. Only successful empty output means absent. A loaded peer must pass the
existing full coordinator policy; a running peer still needs the existing
double-read process/invocation/cgroup generation check. Neither absence nor
STARTING grants readiness or dispatch.

The wait is at most60seconds/16observations within the original25minute main
window and existing128query budget. Query errors, terminal peers, disappearance
after observation, own generation drift and exhausted bounds fail closed;
no service start, lifecycle retry or deadline renewal is added. Original READY,
stop-client ownership, restoration, collector delivery and all12 official
payloads are unchanged. The only changed existing functions are the loader's
process_body_context and observer query allowlist; one peer observation method
is added. Old v9/v4 sources remain immutable.

peer-start-validation-a1.json:19 new cases and1 necessary pinned-source import
regression PASS; total401 distinct cases. Sources really load from verified
bytes, but root/systemd/process/clock values are fixtures and no child, socket,
production path or actual service is used. This does not prove root system-manager
startup. peer-start-source-reconciliation.json records the exact26 current pins.
The initial reconciliation command failed before outputs by applying literal_eval
to AUX+literal; its correction and unchanged source pins are retained separately.

Complete fresh operating admission, the fixed protected launcher and external
collector process terminal recovery remain unfinished. G4 execution/root staging0;
all four controller findings remain OPEN. Blanket Owner authorization persists;
the single custody/external-writer information question remains pending.


### Wave114 — pre-outage writer admission connection (2026-10-20 KST)

R2-04/R2-09 loader v11 now makes a known-writer check mandatory in
load_inputs_once, before its final unit/fragment and input reconciliation.
It reuses the thirteen existing raw container observations in RAM, adds the
existing PG volume query and four exact network metadata queries in observer v6.
PG image/labels/mount/volume identity, no published ports/host networking, and
the original four network identities/endpoints must match. Configured but
empty EndpointID attachments are not invented as live endpoints.

Selection uses the accepted official tool's four criteria: same project,
shared PG network, overlapping protected mount or host network. All nine planned
live writers must be selected and healthy; selected preserved containers must
pass the actual official stopped_writer validator. The research exit2 states
remain allowed and unchanged. Extra running writers are rejected before outage;
the official post-stop guards remain unchanged. The returned record explicitly
does not claim actual stops, global/future writer exclusion or complete admission.

writer-preflight-validation-a1/a2:18 new distinct cases and1 source import
regression PASS. The initial3 failures expected ValueError whereas the accepted
validator correctly raised RuntimeErrorCode. Only those3 assertions were fixed
and rerun; source bytes stayed identical and failed receipts remain. The earlier
unvalidated draft is retained: configured disconnected attachments were corrected
to require a nonempty valid endpoint before counting a live member.
Actual immutable historical bytes and accepted pure validators were exercised;
current Docker/root/state/network facts are fixtures. No child/socket/production
path or real outage was used. Distinct419; full controller findings remain OPEN.

The fixed launcher was not completed. Complete runtime/capacity admission,
independent actual root startup and external collector-process terminal recovery
remain engineering work. Blanket approval persists; custody/external/manual-writer
facts remain unanswered. No additional approval/information question was issued.


### Wave114 — fixed entry and runtime admission connection (2026-10-20 KST)

R2-04/R2-09 now has an executable fixed entry source. Loader v12 binds its
entrypoint hash in the operating-input controls digest, verifies the exact
protected entry bytes, and checks both actual process command lines and
unchanged generations before READY. Coordinate starts only the two fixed
transient services, once each, with an empty environment and the existing
retained-terminal policy. The independent recovery body still owns all stop
clients, journal, original watchdog and restoration; official12 payloads,
observer v6, actual READY and all existing restoration functions are unchanged.

The startup runtime gate reuses the existing protected public-file reader for
Python/systemd/Docker/env/OpenSSL and current mapped/loaded system files. It
requires2GiB MemAvailable, backup free space at least minFreeBytes+4*maxTotalBytes,
64MiB evidence space and256 available inodes. These are conservative startup
headroom choices, not reservations, a complete future-import inventory or future
capacity guarantees. Actual swap values are recorded without inventing a swap
minimum. The official per-write reserve is unchanged. The new gate is never
called to block an already owed restoration.

Every unit submission records an exclusive intent before Popen, retains the
original manager client until reaped, and records its actual response; a missing
response is not inferred from a running unit. No second submission, reset,
forced kill, recreation or deadline renewal is available. A first/second startup
failure or post-dispatch receipt failure still attempts bounded terminal and
finite record/state recovery. Missing/partial records and uncertain terminals
remain explicit INCOMPLETE. Successful collection publishes its bounded exact
file metadata/hash in the program's terminal envelope so the external caller can
recover it together with the actual process exit. Neither stdout nor a saved
file certifies that future exit.

runtime-entry-validation-a1/a2, entry-bootstrap-validation-a1 and
entry-terminal-output-validation-a1:40 new distinct tests PASS;4 changed-entry
regressions PASS. Total459 distinct. The bootstrap tests use real private files,
hashes and memory imports with root metadata/held-reader callbacks mocked. Other
root/proc/manager/Docker facts are fixtures. No child/socket/production path,
root staging or actual G4 execution occurred. Prior passed suites were not
repeated. Two earlier entry versions are retained. Source reconciliation pins
27 dependencies (15 private controls +12 accepted payloads), plus loader/entry;
all old loader functions except four named binding functions retain exact ASTs.

The package is SOURCE_PREPARED_NOT_STAGED_NOT_ADMITTED. Remaining work is the
exclusive protected staging/fresh eight-input construction, complete fixed-entry
integration review/validation of the four open controller findings, and actual
external coordinator terminal recovery. Owner custody/external-manual-writer
information is still absent; no new question or permission request was issued.
No actual backup, deployment, Owner use, real three-day observation or C_R2
completion is claimed. Same-agent heartbeat remains continuation only.


### Wave114: fixed staging and fresh-input preparation (2026-10-20 KST)

R2-04/R2-09 now has a fixed34-file installer and a `prepare` branch in the
existing entry. The bundle contains29 source files,4 immutable static inputs
and the empty recovery singleton file. Before its first write, the installer
requires both fixed destination roots absent. It holds and verifies every
ancestor, creates only the fixed roots/subdirectories, writes each file with
O_EXCL/0600, fsyncs, and verifies all bytes again after the whole installation.
It never chmods an existing path, overwrites, removes or repairs partial output.
The hash-bound isolated root invocation is concrete in fixed-stage-invocation;
it has not run. Source and bundle transport are not an operating admission.

`prepare` first requires the actual protected Owner information receipt. It
publishes an exclusive preparation intent, observes the fixed56 units and13
containers, current/retained metadata and fixed PG ancestry, and reads59 old UP
source checksums. Project/database identifiers come from the fixed PG metadata;
missing/ambiguous values reject. No other environment values are persisted.
Existing historical/file/runtime/writer checks are reused. It writes original
and contract inputs, then the operating envelope LAST. Partial input publication
blocks reuse. The original45-minute window starts here, not at later coordinate
startup. These source checksums are not an actual database audit; no SQL, key
bytes, lifecycle dispatch, lock acquisition or permission change occurs here.

The coordinator's unbound invocation state is now accepted only as a loaded
activating/start job with zero PIDs, before any bound generation, under a single
60-second cap within the original main window. It is never terminal or READY;
a lost bound generation, malformed state or expired cap still rejects.

New distinct cases22 PASS (11 installer,3 startup,8 preparation), cumulative481;
5 necessary regressions PASS. Installer cases use actual unprivileged private
files and /proc/self/fdinfo. Root ownership and all production/proc/manager facts
in preparation are explicit fixtures; native gates are fixture callbacks there,
not actual root admission. Initial7 installer-case failures came from test
ancestors inheriting umask002; the fixture now matches production main umask077,
all11 affected filesystem cases passed, and the failure/source are preserved.
No installer behavior was changed for that correction. Unchanged suites were
not repeated. No child, daemon, socket, root staging or actual G4 execution.

fixed-stage-input-source-reconciliation pins the final entry, installer,34-file
bundle and receipts. Loader v12/27 dependencies (including accepted12 payloads)
are unchanged. Apart from the added prepare function/main dispatch and bounded
startup handling, all entry functions retain their prior ASTs. The exact bundle
was installed/read back again in a private fixture after its entry pin changed.
Actual Owner information is still absent. Four full-controller findings and
full fixed-entry integration validation remain open. Actual root installation,
fresh inputs, independent READY, backup/restoration and external coordinator exit
plus exact collection bytes remain unexecuted. Blanket authorization remains
valid; no repeat question, CI/build, accepted-source mutation or new independent
review was performed. After G4, backup transition/deployment/Owner use/three
actual consecutive tradingdays/final C_R2 are still required.


### Wave114: fixed-entry integration disposition (2026-10-20 KST)

The current prepare output enters the current v12 input loader unchanged: all
8 pins, attempt, original shared deadline and actual historical-binding code
agree. Corrupt original bytes are retained and reject admission without repair
or reuse. Host/root/filesystem observations remain fixtures in this connection.

Five new cases connect the current coordinator and current recovery body to the
real fixed sequence/runner/restoration/journal/contract code. They cover partial
terminal bytes, wall-clock drift after stop, failed official capture, a delayed
original stop response, and a consumed but undispatched partial next intent.
Witnessed obligations remain recoverable; no start occurs before the same stop
client resolves. Body exit2 reaches the outer coordinator failure path instead
of accepted collection. Manager/READY/host/resource boundaries are fixtures.

Two new cases connect the current coordinator to the real current external
collector and actual unprivileged private receipt IO. Eight admitted inputs and
65 command/result records are checked using synthetic host/backup observations;
publication uses actual exclusive create/fsync/readback. A final fsync error
retains the file, returns failure and prohibits recollection. This is not proof
of a root coordinator's actual exit, a real encrypted backup or actual READY.

New distinct cases9 PASS, cumulative490; one changed negative-information test
also passes. That test now resets its input before each subcase, so missing
custody, declared external writers and wrong information source are independently
exercised. Two initial fixture integration errors are retained: missing ancestry
validator and a Mock pending-query map. The fixtures were corrected by connecting
the unchanged validator and supplying the concrete empty map; only affected
cases were repeated. No operating source changed. All29 sources and34 fixed
bundle members still match their prior pins. No new actual child, root staging,
Docker/systemd operation, SQL, production observation, CI or rebuild occurred.

`entry-integration-disposition.json` binds the receipts and limits. G4-DRAFT-01
through04 now have current source-path mitigation evidence across these seams;
the complete actual root controller findings remain OPEN pending fresh native
admission, real independent READY and original coordinator terminal recovery.
This is composed fixture evidence, not a single actual root end-to-end run or
an independent review. Do not repeat passed suites or grow the private controls
to occupy the information wait. The next dependency is the actual answer to the
existing custody/external-writer question, followed by binding that concrete
protected information packet and the already-authorized G4 execution. Blanket
authority stays valid; no repeated approval or information question is needed.
Deployment, Owner use, three actual consecutive trading days and C_R2 remain open.


### Wave114: beta local backup-key custody (2026-10-20 KST)

The Owner proposed temporary server-local file storage because this is not yet a
formal service. Adopt that beta scope using the existing official
`/etc/lagrange/secrets/backup_encryption_key` (root:root 0600); no key contents are
read, copied into `.env`, rotated, or placed in evidence. Off-server custody is
deferred to formal-service readiness and no longer blocks beta G4. This local
policy does not establish recovery after loss of both the server and its key.

`owner-beta-local-key-policy.json` retains the exact Owner message/hash and binds
`beta-local:backup_encryption_key` explicitly as a local-only reference. The
existing entry/loader accept this reference without source changes. External or
manual writer presence remains unanswered; do not fabricate `none-declared` or
create the complete operating information receipt. Original question evidence is
unchanged. Actual root admission, independent READY, G4 and deployment remain
unexecuted. Accepted source, tests, CI, keys and host state are unchanged. The
registry points to `beta-local-key-policy-handoff.json`, which supersedes stale
heartbeat text requiring off-server custody before beta execution.


## Wave115 actual preparation failure and mount-order correction

The Owner's actual message “따로 없으니 이어서 진행하라니까?” declares no
separate external/manual writers. Together with the adopted beta-local key policy,
this resolves the factual-information wait. The canonical350-byte information
receipt has SHA256 `2e6fc3b5ab6bafce2cebdac5d84da97a5eec0ebc544d96166ab8d9e9571e2011`;
provenance binds the exact message and the unchanged original question. This is
an Owner declaration, not proof that no unobserved/future writer can exist.

Actual root execution on2026-10-20 UTC:

- 04:24:10: sealed34-member installation exited0, STAGED_SOURCE_ONLY. Both previously
 absent destination roots now exist; preserve them and all files without repair.
- 04:24:39: protected Owner-information publication exited0 with exclusive creation,
 fsync and exact readback. No key contents were read or copied.
- 04:25:00: original prepare process PID2332313 was reaped with exit2 and the exact
72-byte INCOMPLETE terminal. Its710-byte preparation intent remains. Original
instances, contract, operating envelope and coordinator intent were absent in the
04:28:15 bounded read-only inspection. No coordinator/recovery/requester started,
no independent READY or outage occurred, and the old preparation must not replay.

The original entry suppressed exception details, so its exact exception remains
UNKNOWN. A bounded follow-up using the same frozen reader confirmed the56-unit
union, both owned service names absent, and then reproduced
`INPUT_HISTORY_CONTAINER_POLICY` for existing backtest-worker-1. Its ID/state/
restart count were unchanged. A subsequent fixed-container query produced policy
hash `a59e464eed426d0602714498b6d5197807026dd69831992894ffd333a7525d27`;
permuting only its four Mounts entries by `[2,3,0,1]` reproduced the exact historical
hash `f0af6bbf248a435f3ed43c3acff36d22e61528494112b13711742d23447c40c3`.
No Config, HostConfig, network or mount field was removed or modified. Raw inspect
configuration remained in RAM. This proves this observed mismatch was ordering
only; it does not recover the missing original traceback or prove all later gates.

The official `stopped_writer` had the same list-order sensitivity. Its successor
source now sorts Mounts by validated unique destination while retaining every
field. Config/HostConfig arrays and full stopped NetworkSettings remain ordered/
exact as before. Invalid/duplicate destinations fail closed. Private successor
observer-v7 uses the same normalization for restart-policy comparisons. Loader-v13
retains the immutable old survey hash and permits only a bounded exact-hash
permutation proof (at most six mounts/720orders) when the raw legacy hash differs.
No historical evidence or installed source was overwritten or repinned.

Five new official and seven private pure-metadata cases pass, including all24
orders of a four-mount example and refusal of actual configuration, mount-field,
network and generation changes. The existing maintenance suite has60 unique tests
including the five additions. The first run passed32 but its28 filesystem cases
failed in setup because the invocation's077 umask contradicted the fixture's750
expectation. Only those28 were repeated with the fixture's normal022 umask, all
passed; sources were unchanged. Fixture files remained under private0700 temporary
roots/chroots, Docker/systemd were fakes, and production execution retains077.
Both failed and corrected receipts are preserved. The earlier diagnostic wrapper's
incorrect `_LOADED` tuple access is also retained; it failed before observer queries.
One intermediate diagnostic matched the identity permutation and over-labelled
it MOUNT_ORDER_ONLY_PROVEN; that record alone proves only a current hash match.
The later nonidentity permutation receipt above supplies the actual mismatch proof.

Evidence is under `target/r2-daily-beta/wp-8/wave114/`: actual-stage*,
actual-information-publication*, actual-prepare*; mount-order-focused-validation;
mount-order-maintenance-validation-a1/a2. The successor private source is unsealed:
it is not routed by the installed entry, still needs a new accepted tool pin and
an independently scoped execution package, and grants no new preparation window.
The original45-minute window is never renewed. No automatic operational retry,
CI rerun of accepted candidates, application rebuild, provider call or deployment
is included. Next is successor tool hosted validation and the concrete corrected
execution package under existing Owner authority. The four full-controller runtime
findings, G4, deployment, Owner use, three actual trading days and C_R2 remain OPEN.


### Wave115 hosted acceptance and distinct sealed successor — 2026-10-20 05:06 UTC

R2-04/R2-09: `b6e409af4baf0f5af7a0539bda92c9ce3ad62335` completed
[CI39370737972](https://github.com/sihioov/Lagrange/actions/runs/39370737972)
at05:04:57UTC, attempt1, all8 jobs successful. Full8 logs/7 actual checkout SHAs
were recovered by05:06:32UTC. All206 required definitions,60 maintenance cases
(including five new mount-order cases),3367 Rust named rows/231 root blocks,
19 dedicated PG exits/146 names and web143 cases reconciled. Existing skips,
ignored/platform exclusions and independent fragmented nested outcomes remain
explicit; parent success does not fill UNKNOWN.

All1464 byte pins and548 tracked candidate blobs match. The initial assembler
retained the prior tree count2163, but the actual candidate has2164, with this
compatibility review as the sole added path since790f1ea. Original manifest and
failed assembler receipt remain. `ci-input-tree-count-correction.json` pins the
actual Git listing and added blob; no source/workflow change or CI rerun.
Hosted receipt `f7ba48e367b985fcabe28daedd3829415bc0c7b917e20b95992314862d2b43c6`
is maintenance-tool-only acceptance. App e639169/12images/actual isolated
wholeflow and prior790 evidence remain separate and immutable.

The distinct34-member package binds corrected observer-v7/loader-v13 and the
new tool with27 dependency pins/12 payload pins. New roots are
`/var/lib/lagrange-r2-g4-evidence/b6e409a-a1-20261020` and
`/opt/lagrange/maintenance/b6e409af4baf0f5af7a0539bda92c9ce3ad62335`.
Both must be absent before writes. Original790 roots, failed prepare and its
window remain consumed/retained. Original blanket authority bytes stay exact;
their accepted790 field is historical context, while new source/hosted acceptance
are explicitly pinned. Actual Owner information stays bound to the real message
and original question. No additional approval or answer is pending.

Twenty-eight selected isolated regressions pass:34-file staging/readback,
no overwrite/partial reuse, pinned import/bootstrap, prepare and prepare-to-admit
connection. Root/host observations are fixtures; actual admission/READY/G4 remain
open. Source package4e834b7b and execution seal136c0f1e are in private Wave115.
Next: exact sealed installer/entry preparation, actual independent recovery READY,
single bounded G4 and original-process/exact-collection closure. No automatic
retry, old-attempt replay, repair or window renewal. Backup transition/deployment,
Owner use, three consecutive actual tradingdays and C_R2 remain OPEN.


## Wave116 exact large-mount historical binding — 2026-10-20 05:19 UTC

R2-04/R2-09: the distinct b6 installation and actual Owner-information publication
exited0 at05:09UTC. Its preparation exited2 with the generic INCOMPLETE result;
the original exception remains UNKNOWN. Bounded read-only diagnosis reproduced
INPUT_HISTORY_MOUNT_ORDER_BOUND at the API container's13 mounts. The later fixed13
container checks recovered the exact immutable full policy hashes for both the13
and10 mount cases using one metadata read each. No key/config values were persisted.
The original56-unit survey is not replaced and no new complete admission is claimed.

The consumed attempt retains all34 installed files, its preparation intent and
original45-minute window. Original/contract/operating/coordinator files and both
owned units were absent. The closure reader initially rejected the intentional
empty recovery.lock after33 successful source checks; that error remains, and a
single separate empty-file metadata/hash check completed the34-file closure.
No READY, outage, G4, backup or deployment occurred.

Private loader-v14 fixes only those two proven historical mount orders. Every
mount field, Config, HostConfig and NetworkSettings must still reproduce the full
old hash. It neither searches factorial13 orders nor refreshes the baseline.
Five new focused cases plus seven affected regressions pass;28 changed-package
connection cases pass. The assembler's initial26-versus27 source-count assertion
occurred before writes; its corrected accounting includes the separately pinned
runtime module. Both receipts are retained. Official b6 payload/CI and accepted
application evidence remain unchanged; no CI/build rerun is needed for this private
binder correction. New fixed roots b6e409a-a2 and maintenance/b6e409a…-a2 preserve
both earlier attempts. The suffix denotes a distinct copy, not another Git revision.

Evidence under target/r2-daily-beta/wp-8/: wave115/actual-prepare-failure-closure.json
SHA c720cac4a3c0f27eb715222c8cf8ff9422e0b185d87bbd37e5c14ecb3d11609c;
legacy-order proof907f0cad; wave116/source package5985170d and execution seal659c43d2.
Actual a2 preparation/admission/independent READY/G4 and all later R2 gates remain open.


## Wave117 canonical tool reuse and empty package markers — 2026-10-20 05:27 UTC

R2-04/R2-09: Wave116 stage/information exited0; a2 preparation once exited2 at
05:20:10UTC, with original exception still UNKNOWN. Subsequent bounded read-only
construction passed all13 historical container bindings and identified
CONTRACT_FIXED_PIN_SCOPE. The suffixed TOOL copy cannot satisfy the official
commit-derived canonical root, including PREUPGRADE_TOOL_LOCATION. Prospective
canonical12-pin construction then passed file/historical metadata preflight and
reached RUNTIME_FILE_METADATA: two actual root:root0644 zero-byte Python3.14
compression package markers were being rejected. These are separate diagnostics,
not preparation/admission and not the original missing traceback.

Closure05883030 retains all34 a2 files and its original intent/window; four
operating files and both own units were absent. No outage/READY/G4/backup/deploy
occurred. No existing root, failed attempt or official source is repaired/replayed.

The new fixed installer verifies the canonical b6 tool's12 source files against
the exact original publication inode/full metadata and hashes before any new
AUX write, and rechecks them after writing22 exclusive AUX files. TOOL is never
written, removed or recreated. This supersedes the earlier two-absent-roots
staging requirement for this distinct package: only the new AUX must be absent;
the exact already-published canonical TOOL must exist unchanged. Official b6
source, hosted acceptance and app e639 evidence remain immutable. No CI rerun.
The private runtime reader permits zero bytes only for the two observed exact
compression/__init__.py paths; ownership, mode, ACL, regular-file, ancestry,
identity, hash and drift checks remain in effect.

Sixteen focused cases pass:10 actual private filesystem reuse/failure cases,
4 public-file/metadata cases and2 real prepare-to-PostStopContract connections.
Seventeen changed-package entry/input/import regressions pass. The prior fixture
connection mocked file preflight and did not cover the actual path mismatch;
the new connection runs the real constructor. Package7e67a92a and seal23bfbae0
bind these results, Owner facts, prior closure and exact existing-tool publication.
Actual a3 preparation/admission/independent READY/G4 and later R2 gates remain OPEN.

## Wave118 actual preparation and typed empty Exec arrays — 2026-10-20 05:40 UTC

R2-04/R2-09: a3 stage/information and preparation exited0. Preparation at05:28:54UTC
published original625f0251, contract00b23ca0, envelopeedeafc4a after58 queries.
Coordinate at05:29:00UTC exited2; both exact launcher clients exited0 and were reaped.
Both retained actual root units normally exited2 with MainPID/ControlPID0, no Job,
and recursive cgroup paths absent twice. READY/result/collection/contract and all64
command intent/terminal files were absent. No outage or daemon stop was submitted.
Closure2b320c6a preserves original failure, partials and failed units. Original caught
exception remains UNKNOWN. Later read-only checks reproduce OBS_UNIT_PROPERTIES:
systemctl omits six empty Exec arrays even with --all. D-Bus independently returned
six exact a(sasbttttuii) empty arrays for each unit. Initial diagnostic asserted the
old property set and failed; that failure is retained, not counted as success.

New private observer-v8 accepts only missing members of those six fields and only
after all six exact typed-empty manager values are recovered through its bounded,
reaped, read-only argv allowlist. Other missing/unknown values and nonempty hooks
still fail. busctl joins the executable hash/metadata inventory. Coordinator polls
at6 seconds, retaining its original45-minute window,2048 wait budget and4096 total
query cap; worst-case near-deadline fixture consumes1928 including128 admission.
Historical56 unit property schema and policies remain unchanged. No lifecycle retry.

22 focused cases and17 changed-package entry/input regressions pass. The corrected
reader also recovered both actual failed a3 terminals with8 real read-only queries,
all children reaped, and rejected them as successful terminals. This adapter binds
only the two prior names; it is not a4 startup or G4 admission. Package d22808d4 and
seal cce3c53b select distinct a4 AUX with unchanged canonical12-tool read-only reuse.
The first local assembler used the previous WAVE path and failed before writes;
corrected assembly succeeded with that failure preserved. Official b6 hosted and
application acceptance are unchanged. Actual independent READY/G4, backup transition,
deployment, Owner use, three actual consecutive trading days and C_R2 remain OPEN.

## Wave119 actual READY and post-ACK exit transition — 2026-10-20 05:48 UTC

R2-04/R2-09: a4 prepared58 queries successfully at05:41:44UTC. Actual independent
READY7eafc3dd was published; exact requester invocation d57d1cf8 exited normally0.
Recovery5c19aea3 and original coordinator exited2. Failure recovery contains both
actual terminals and no coordinator terminal gaps. No64 command records, operating
result/collection or maintenance contract existed. Closurec9df7052 observes both
recursive cgroups absent, all13 original filtered container snapshots unchanged,
PG-parent same ancestry/inode0701 and official release lock still absent. Preserve
READY, contingent restore intents, socket, failed/exited units and consumed window.
No outage/backup/restore/deploy success. Original caught exception remains UNKNOWN.

A separately reproduced source defect explains a possible post-ACK failure: a
running unit observation can precede the requester's required normal exit, so a
second live-generation query can reject that expected exit. The prior exact loop
fails with OBS_COORDINATOR_NOT_RUNNING in a deterministic real-child interleaving.
This does not retroactively identify a4's missing exception. The new loop binds each
running snapshot to the original full property digest/PID/invocation, then requires
the existing exact normal-exit0/PID0/job-empty/recursive-cgroup terminal verification.
Query errors, replacement, nonzero/signal exit, policy drift and residual children
still reject. No retry/reconnect or renewed deadline. Three actual private child
executions are included in17 focused tests;17 entry/input/import regressions pass.

The entry also retains bounded typed failure code/type/source-frame metadata through
its existing exclusive writer after body cleanup, without exception prose, locals,
keys or provider data. Publication failure preserves the original exception and
cannot grant success or retry. Initial local package c7c57e13 emitted a regex escape
SyntaxWarning and was preserved unexecuted; corrected package9ad5262b passes with
warnings-as-errors. Seal3726d951 selects distinct a5 AUX, same canonical official12
read-only files and actual Owner information. Original a4 READY is consumed; fresh
admission/independent READY remain mandatory for a5. All later R2 gates remain OPEN.

## Wave120 actual timer restoration / periodic service correction — 2026-10-20 06:09 UTC

R2-04/R2-09: a5 reached actual independent READY and normal requester exit0.
Two backup timers stopped and restored; all four original command clients exited0.
Recovery/coordinator exited2 with FINAL_ORIGINAL_STATE_UNVERIFIED. Actual G4 outage
sequence began, but no application/PG container stop, PG chmod, schema audit or
backup capture occurred. Closure a0c3c785 preserves INCOMPLETE, all actual receipts
and original main exception UNKNOWN. Collector failure OBS_OWNED_NOT_NORMAL_TERMINAL
is the subsequent recovery-exit2 rejection, not the original main failure.

Independent42 read queries at05:52:26UTC verified all13 container snapshots exact,
5timers active/waiting,56-unit union/fragments, PG-parent same inode0701 and both own
recursive cgroups absent. Two timers have actual new restored generations. The
normal service-status invocation56399755 ran at05:49:43–44UTC between timer stops
(journal metadata99b99378). Old whole-property equality rejects that completed
invocation; the original thrown main exception was not recorded and stays UNKNOWN.
The official release lock now exists: root0600 empty inode826045/device64512, exact
identity bound in the closure. It is retained, never recreated or removed.

New private a6 package047dad92/sealfcbdafa5 permits only later completed normal
invocations of previously run services belonging to the original active timers.
All policy/edge fields, fragment bytes, full56 inventory, normal exit0 and empty
recursive cgroups remain required. Historical failed/disabled/never-run units stay
exact. An already-running periodic service may settle through read-only bounded
observations for at most5seconds within the original window; query errors fail
immediately. Its generation freezes after its timer is stopped. Actual new values
remain in the original/maintenance/final/collector records. Two backup timer
baselines bind exact a5 restoration; the existing release lock is verified read-only
by exact original identity, ancestry/no ACL/no-follow/mount and empty content.

26 focused tests and4 connected before-step/contract/final/collector cases pass;
17 prepare/admit/bootstrap/import regressions pass. The initial connected fixture
incorrectly expected no frozen services after stopping two backup timers; its
failure is retained and only that failed case rerun after fixing the assertion.
No operating source changed after sealing. Typed bounded main/final failure details
now survive INCOMPLETE without secret prose or blocking owed restoration. Prior
attempts/windows/units remain consumed and retained. Fresh actual a6 prepare,
admission and independent READY are required; no Owner approval/information wait.
G4 acceptance and backup transition/deployment/Owner/three actual days/C_R2 OPEN.


## Wave121 active timer observation / a6 restoration — 2026-10-20

R2-04/R2-09 remain DOING. Actual a6 preparation/READY/requester exit0 succeeded;
three timer stops and their three restorations all exited normally0. Sequence
RESTORED has originalServicesRestored/evidenceComplete true, captureAccepted false.
Recovery/coordinator exit2. Original main error CONNECTION_DISPATCH_TARGET_CHANGED
is now retained at the fourth timer pre-dispatch gate; the differing property was
not captured and remains UNKNOWN. Running-to-waiting is a reproduced source race,
not a claim of the unrecorded actual value. Collector's nonzero-recovery rejection
is subsequent. No container stop, PG chmod, schema audit or capture occurred.
Closure11d34d88 contains42 independent read queries at06:11:03UTC:13 original
containers exact,5 timers active/waiting,56-unit union/fragments, samePGparent0701,
owned cgroups absent, exact retained a5 release-lock inode. The initial result
reader32768 bound error is retained; only its read bound changed to262144.

Wave121 package63f98544/seal23cca6cd adds bounded read-only observation of only
an original active/waiting timer's SubState running→waiting. All other fields,
generation, timestamps, jobs, policy and edges stay exact; errors fail immediately.
The bound remains5seconds/11observations within the original deadline. The real
pre-dispatch gate uses this helper and still requires exact waiting before its
single intent. Three restored baselines bind the actual a6 closure.13 focused
cases (including real dispatch gate success/rejection)+17 prepare/admit/bootstrap
regressions pass. Prior30 periodic cases and post-ACK/typed-array/runtime evidence
are retained; no official payload/application/hosted evidence change. Distinct a7
stage/preparation/admission/actual independent READY precede any G4 operations;
a6 units/windows are consumed, not retried. No Owner information/approval wait.
G4 acceptance, backup transition/deployment, Owner use,3 actual trading days and
C_R2 remain OPEN. Same-agent tracking is not recovery protection.


## Wave122 stop acceptance and owed restoration — 2026-10-20

R2-04/R2-09 remain DOING. Actual a7 preparation and independent READY succeeded;
all seven original stop clients exited normally0 (five timers, proxy, web).
Web actually exited143; the overly strict stop-state predicate raised
OBS_NOT_NORMAL_STOPPED. Recovery/coordinator exited2, leaving original services
stopped. The original failure and result remain INCOMPLETE, not G4 acceptance.
A separate sealed restoration-only action ran once inside the original a7 window,
verified every original stop response and owned process/cgroup drain, then restored
web, proxy and five timers in order with seven normal0 clients. Its original
process2587869 exited0 at06:23:20UTC. Independent process2591597 exited0 at06:23:55UTC
with42 queries:13 stable container policies,11 untouched snapshots, actual new
web/proxy timestamps/endpoints, five active/waiting timers,56 original units and
fragments, samePGparent0701 and owned cgroups absent. Exact root result161880bytes
SHAad1bd5df and closure25034bytes SHA7aca0584 preserve both failure and restoration.
No PG/Owner stop, PG chmod, old schema audit or capture occurred.

Wave122 package0d86bb60/sealcae2ad19 permits143 only for the exact observed web ID
and full policy hash during main execution. Owner and PG still require exit0.
Other noncritical143 can only establish that an original stop completed for owed
restoration, never authorize further main work or capture. The original client
must have exited0; unresolved responses/activity/cgroups still forbid starts.
A stop acceptance failure with verified physical completion enters recovery and
restores all prior effects; original error and nonzero workload exits are retained.
Two container and five timer baselines bind exact actual a7 restoration, preserving
immutable historical records and new runtime metadata.18 focused cases (15 new,
3 sequence regressions) and17 prepare/admit/bootstrap regressions pass.
Official b6 payload, hosted CI and accepted app/image/wholeflow remain unchanged.
Fresh a8 stage/preparation/admission and independent READY precede G4; no consumed
attempt/window is replayed. No Owner information/approval wait. G4 acceptance,
backup transition/deployment, Owner use,3 actual trading days and C_R2 remain OPEN.

Distinct active heartbeat40f9f633 was created06:25:52UTC. Exact create receipt and
renewal handoff5f06d5a0/registry were persisted before predecessor d8d89ce7 deletion.
Expires18:25:52UTC, renew by17:20UTC and before47th tick. Latest bounded handoff
b925273c points at Wave122; tracking is neither native Ralph nor outage protection.
