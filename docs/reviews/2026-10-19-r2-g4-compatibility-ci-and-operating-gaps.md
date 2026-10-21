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


## Wave123 retained unstarted failure / a8 restoration — 2026-10-20

R2-04/R2-09 remain DOING. a8 prepared06:35:39UTC with58 queries and a fresh
independent READY.14 original stop commands exited0 (five timers/nine containers,
including normal Owner exit0). PG parent was protected0700; contract construction
failed PREUPGRADE_UNITS before official audit or PG stop. The retained historical
lagrange-r2-build-ab48a79.service is failed with both main-process timestamps0,
ActiveEnter0, positive InactiveEnter, NRestarts0, no PIDs/job/cgroup/activation edges.
The official predicate wrongly required a positive main start for every retained
terminal. Its actual historical failure cause remains UNKNOWN; no unit is reset,
removed, filtered or restarted to satisfy the check.

Original recovery restored PG parent0701 and issued13 normal0 starts for nine
containers/four timers. It then failed NATIVE_TIMER_FAILED at the31st command's
post-start observation. That exact transient state was not recorded; rejection of
an active/running timer is a source defect reproduced separately, not an invented
actual state. Recovery/coordinator exited2 at06:37:08UTC; G4 remains INCOMPLETE.
A sealed restoration-only action verified all27 original command responses and
owned process/cgroup drain, nine original service identities/health, and the
original window. It issued only the previously unissued final timer start once.
Process2639745 exited0 at06:40:23UTC, final start2640323 exited0; exact root result
148035bytes SHA5ff16e9e, recordErrors empty. The initial local validation adapter
omitted canonical newline; its failure is retained, fixture corrected,10 boundary
cases pass without production execution. Independent process2641992 exited0 at
06:41:06UTC:42 queries,13 stable container policies,4 untouched exact snapshots,
actual new9 container runtime snapshots, five active/waiting timers,56 original
units/fragments, samePGparent0701/current/retained and owned cgroups absent.
Closure26511bytes SHAf3a42a5a binds originals, actual terminal, exact collection
bytes and restoration. No PG stop/schema audit/capture/deployment occurred.

The successor official source admits zero main timestamps only for a detached
failed transient in quiescent-observed mode, with ActiveEnter0, positive
InactiveEnter and NRestarts0. Full field/fragment/inventory/cgroup checks and both
native observations remain; started failures and persistent disabled failures
retain their existing positive-time requirements.64 maintenance tests pass,
including4 new tests covering preservation,19 invalid mutations, graph/fragment
requirements, native activity and changed snapshots. This is a new tool revision
requiring its own hosted CI acceptance; accepted b6/790 and app e639 are immutable.
Private post-start timer observation also requires correction before the next
distinct sealed attempt. No consumed attempt/window is replayed. Owner information
and authorization are resolved; G4, transition/deployment, Owner use,3 actual
trading days and C_R2 remain OPEN. Heartbeat40f9f633 tracks the latest registry.

### Wave123 validated successor / active hosted CI — 2026-10-20 07:07 UTC

New official tool `5dcfad5817382bf008cc27d0d2278436f846d80b` was dispatched exactly once as CI `39382296814` attempt1 at06:45:58UTC. Six jobs have succeeded; complete policy/PG/web evidence reconciles202 required Python results,64 maintenance definitions,19 PG processes/146 names,44 wholeflow fixture results and15 image outcomes. Workspace Rust and the final required job are still pending; this is not hosted acceptance.

The separate34-file package `1149f129346f3272ae3fe3c6c6248c587d5185223ee1d6cb8e8bbac459613344` binds the actual a8 restored9container/5timer generations. After an original timer start returns0, its active/running→waiting settling may be observed for30seconds/11observations within the original deadline; invocation, timestamps, static policy and activation edges remain fixed, and no second start is issued.43 isolated timer/stop/sequence checks and28 unchanged-byte filesystem/prepare/admit/bootstrap checks passed at06:50:20UTC. These are not actual host G4 acceptance.

Both `/var/lib/lagrange-r2-g4-evidence/5dcfad5-a1-20261020` and `/opt/lagrange/maintenance/5dcfad5817382bf008cc27d0d2278436f846d80b` must be absent before the sealed installer writes. No new root staging has run. Full new-tool CI acceptance precedes a fresh stage/information/prepare/coordinate sequence. Previous attempts/windows and original ab48 causeUNKNOWN remain preserved. Owner authority and actual no-external-writer declaration are resolved; G4/deployment/Owner use/three actual trading days/final C_R2 remain open.

### Wave123 full hosted acceptance and execution seal — 2026-10-20

CI39382296814 attempt1 completed all8jobs successfully at07:11:18UTC. Full8logs/7exactcheckouts,210required definitions,64maintenance definitions,1464input pins/548tracked blobs and2164tree entries were reconciled. Hosted tool-only acceptance `581f6ffee6980b89e17b6825217ce37e6bf5626a81593b50bd493282501beacb` preserves the separate application acceptance.

The initial workspace parser rejected a zero-test result whose stderr target header arrived after its stdout summary. Its failed reconciliation is retained. The corrected evidence parser binds only an otherwise unbound explicit all-zero successful block to its exact unchanged target; all3367named Rust rows/231blocks and36current/33carried groups match. The actually fragmented nested name and unprinted independent nested results remainUNKNOWN. No CI rerun or source change occurred. Initial input-manifest line-count metadata and changed-path reporting errors are also retained with their correction.

Execution seal `e20bbecdc9caf4ebe33a3cd431afe05b2a485cf578765da92095c8d821f9b172` binds the immutable34-file package,43+28private validations,64official tests and new hosted acceptance. Root stage/information/prepare/coordinate have not yet run for this successor. Both new roots must be absent; actual fresh admission/independentREADY and the one original45minute window govern execution. G4 acceptance, deployment, Owner use and three actual trading days remain open.

### Wave124 actual PG metadata correction — 2026-10-20

Wave123 5dcfad5-a1 staged34files and published actual Owner information, then prepare process2707857 exited2 at07:15:02UTC before original inputs, operating envelope, coordinator or READY existed. Original exception was not recorded and remainsUNKNOWN. Read-only37query diagnostic reproduced PREFLIGHT_PG_VOLUME_CHANGED. One further fixed volume metadata query found only historical CreatedAt changed; that diagnostic later failed with a retained KeyError while assembling ancestry. The separate parent-only check process2727688 exited0 at07:24:59UTC: exact original inode82199533/device66309, full ancestry/mount IDs/ACL guard and mode0701; its ctime1792478158495347608 matches the actual Docker CreatedAt2026-10-20T15:35:58+09:00. No additional Docker query or outage occurred. Closure02e8e21a preserves both the original failure and diagnostic error.

Distinct 5dcfad5-a2 packagebe879479 binds this actual timestamp, six stable volume fields and original parent ancestry while preserving the historical receipt. Eight new boundary tests,17 prepare/admit/bootstrap regressions and10 actual private filesystem reuse tests passed at07:28:15UTC. Initial unexecuted source review caught an extra parent output-mode field and is retained separately. Canonical12tool bytes are reused read-only with exact original publication identities;22AUXfiles require a fresh root. Hosted5dc acceptance581f6ffe and applicatione639 remain unchanged. Execution seal87c4d0b4 allows the fresh one-use stage/information/prepare/coordinate sequence with actual admission/independentREADY before outage and one45minute window. Failed a1 is consumed, never repaired/replayed. G4/deployment/Owner use/three actual trading days/C_R2 remain OPEN; no approval or information wait.


### Wave125 actual audit failure / safe failure evidence — 2026-10-20

R2-04/R2-09 remain DOING. Wave124 a2 prepared07:29:30UTC with58queries, published independent READY and completed five timer/nine container stop clients normally0. The official audit process2747820 exited1 with only the generic71byte failure marker. Its inner cause remainsUNKNOWN. Original coordinator2736876 exited2 at07:31:05UTC; recovery2/requester0 terminals and exact failure bytes were recovered. PGstop/capture intents were absent. Recovery completed all14 restoration commands0, restored the same PGparent0701, original nine service identities and five timers. Independent observation2760682 at07:32:39UTC used43queries to verify13stable container policies, four untouched snapshots includingPG, nine healthy restored services, five active/waiting timers,56original units/fragments and empty owned cgroups. Closure2d1b2166 binds exact actual collection/terminal evidence; G4 remains INCOMPLETE.

Bounded read-only diagnostics checked contract parsing, backup-root/lock metadata, eight Raw lock metadata records, preserved unit state and a fixed PG log/event interval. They did not establish the historical failure cause; absent retained Docker events are not proof that SQL never ran. The first preserved-unit diagnostic was rejected before queries by the observer allowlist; that error remains retained and the corrected four-query diagnostic checked50preserved units. No audit replay or service change occurred during diagnosis.

The official tool now records only an allowlisted error code and up to four owned-source line locations after a validated contract failure. The root0600 receipt uses existing nofollow/O_EXCL storage, binds the maintenance contract and remains explicitly recoveryAccepted=false. It never records arbitrary exception text, keys, source data or locals. Missing capacity or an existing/partial receipt does not retry or replace data; the original generic stderr and nonzero exit remain. Five new tests cover redaction, source locations, actual private exclusive file publication, write failure and the main failure path. The first run's shared umask interference caused28 fixture setup errors; the new entry test now mocks that global side effect. Corrected69tests passed07:49:23UTC, process2792499exit0, stderrSHA82e177eb. Original failed test output is preserved.

This is a new official source revision requiring distinct hosted CI acceptance. Accepted5dc/b6/790tool payloads and app e639169 remain immutable. The next sealed package must bind actual a2 restoration snapshots and PGparentctime1792481392042631451/CreatedAt2026-10-20T16:29:52+09:00, and use fresh roots/admission/READY/window. No consumed attempt/window may be replayed. Owner information and authority are resolved; G4, backup transition/deployment, Owner use, three actual trading days and final C_R2 remain OPEN.


### Wave126 actual admin-role diagnosis and correction — 2026-10-20

R2-04/R2-09 remain DOING. Tool7a99b7e CI39389358426 attempt1 finished all8jobs at08:16:07UTC; full logs,215required results,69maintenance cases and1464pins/548blobs were recovered (hostedc860ed2a). Its new34-file package18f1bdc5 staged and published Owner information normally; preparation2857020 completed58queries at08:20:52UTC. Independent READY preceded14successful stop clients. Official audit2868738 exited1; coordinator2858046 exited2 at08:22:32UTC. All14 restoration clients exited0. Independent43query observation2890373 at08:28:55UTC confirms13stable policies, four untouched snapshots including PG, nine healthy restored services, five active/waiting timers,56original units/fragments, empty owned cgroups and the same PGparent0701. Closureddfdcd00284b84503824686f1ad6a357e56615d0e38666d3c88b75aa34a7f954 preserves original terminals/bytes. PGstop/capture intents are absent and G4 remains INCOMPLETE. Actual parentctime1792484479746789667/CreatedAt2026-10-20T17:21:19+09:00 must bind any successor.

The new sanitized failure receipt reports DAILY_BACKUP_PROCESS at the fixed audit subprocess. A bounded read-only diagnostic at08:30:34UTC classified one FATAL missing-postgres-role message in08:21:17–21UTC, and the exact original container has POSTGRES_USER=lagrange/POSTGRES_DB=lagrange. No raw log/environment, credential, database query or new Docker exec was persisted/executed by diagnosis. Evidence9f9a3f816c3208e15d32d6f23e22004041f02c3edc800d8b4fc404ccf6bd11a8 explains this attempt's connection failure; the older5dc-a2 inner cause remainsUNKNOWN.

The fixed audit command and physical-clone verifier now connect as the existing lagrange administrator while retaining OS999:999. SQL requires that exact session/current role and superuser status, the original PG18 range and all existing writer/migration/catalog bounds. There is no role creation, fallback or privilege change on production. The disposable physical-recovery fixture initializes POSTGRES_USER=lagrange, proves postgres is absent, exercises the exact maintenance command and rejects a different login. Its inherited fixture helpers select an explicit class admin role; the separate daily logical-backup fixture retains its previous postgres default.

Local69maintenance tests passed08:34:09UTC, process2900542exit0, stderra60d72054e9b4799827889089692a4a03e7bb4157c73615b139eed4ace4ce29e. The initial wrapper inherited umask0077 into fixtures and28setups failed; that receipt is retained. Only the test child's umask was corrected to0022; source bytes were unchanged and production/receipt umask0077 remains. Actual full-cluster fixture runs in fresh hosted CI; the production Docker ancestry is not relaxed for tests. The accepted e639169 application and prior tool payloads/CI remain immutable. Daily logical-backup live dump also contains a postgres-role assumption; its deployment compatibility remains explicitly unresolved, not covered by this G4-only correction.

Next: distinct hosted acceptance, new sealed package bound to the actual restored generations, then fresh admission/READY and one original45minute window. The consumed7a-a1 attempt is never replayed. Authority and Owner information remain resolved. G4, backup transition/deployment, Owner use, three actual consecutive trading days and C_R2 remain OPEN.


### Wave126 fixed package and hosted execution — 2026-10-20 08:41 UTC

Correction commit `b7751b017b2962b870c69d5d95d647ec663276fc` was pushed and CI `39394825862` attempt1 was dispatched once at08:38:13UTC, after freezing217required definitions and1464input pins/548Git blobs. At08:41:05UTC clippy/format/Python had succeeded; workspace, policy, PostgreSQL and web were still running. This is not hosted acceptance. Exact input/inventory hashes are69a6b2cf/50e5bd62. Retrieve this original run without redispatch.

Distinct34-file package `e0247cd25ad3eaecdb5e945330572f56dc1b381280503b3ebdfbd536ccf9f219` binds the actual restored9container/5timer generations and PGparentctime1792484479746789667. Only the official audit command, audit SQL and physical-clone verifier differ; nine other official members match the prior tool. Reconciliation9949e882 proves all private behavior unchanged after path/hash and actual baseline replacement, so43prior timer/stop checks are carried without repetition. New package28filesystem/prepare/admit/bootstrap and8volume-boundary tests passed08:39:37UTC, processes2912243/2912262exit0. No new root staging or operational admission has occurred. The new AUX/tool roots must be absent and complete hosted acceptance plus execution sealing precede stage/information/prepare/coordinate. Previous7a-a1 failure/restoration closureddfdcd00 remains immutable.


### Wave127 protected verifier code staging — 2026-10-20

R2-04/R2-09 remain DOING. Tool b7751b017b2962b870c69d5d95d647ec663276fc completed CI39394825862 attempt1 at08:58:58UTC, all8jobs/217required results/69maintenance cases, full8logs/7checkouts and1464pins/548blobs (hosted7d70b630). The actual new34-file installation and Owner-information publication exited0; preparation2962192 completed58queries at09:05:53UTC. Independent READY preceded14writer/timer stops, audit2973611exit0 and normal PGstop2974745exit0. This is the first actual old-schema audit and normal PG stop in these attempts. Capture2975336 exited1 while starting the private copied-cluster verifier; all15restoration commands exited0. Coordinator2962813 exited2 at09:08:41UTC. Independent43query observation2988188 at09:09:52UTC proves13stable policies, ten healthy restored containers including PG, three untouched snapshots, five active/waiting timers,56original units/fragments, empty owned cgroups and same PGparent0701. Actual parentctime1792487240642206461/CreatedAt2026-10-20T18:07:20+09:00 and new PG StartedAt must be retained. Closure0b86ebcc259daa06977f971c74f2f897a3b0b7af4654e6a3460f6ecf068174d1 preserves all32command terminals and consumed attempt/window. No G4 acceptance or deployment.

The sanitized failure0873ad14 reports DAILY_BACKUP_PROCESS at production-preupgrade-recovery.py:272, the clone's Docker start --attach. Read-only metadata diagnosticc08d4ecf confirms both installed verifier sources are root:root0600, while the clone runs as999:999. Direct file bind mounts preserve those permissions, so the clone cannot read the installed scripts. The bounded historical Docker event query returned no events; the original clone's exact exit/stderr and completed cleanup are not inferred from that absence. Earlier causes and failed units remain preserved.

The correction leaves installed sources unchanged and creates two byte-identical root:root0444 code files under the existing root0700 disposable stage, using held ancestry, no-follow reads, exclusive creation and readback. Only those files are mounted read-only into the existing networkless clone; no secret or production directory is newly exposed. Existing cleanup/ambiguous-intent retention covers the staged code. Four new filesystem cases verify protected-source identity, exact copies, no overwrite, symlink/writable-source rejection and private-stage scope; all73maintenance tests passed09:15:45UTC (process3009332exit0, stderr589fd655). The real Docker fixture now installs its source scripts0600, verifies full restore with staged copies, and adds a direct-mount regression expecting the clone's actual exit126. Those actual Docker results await fresh hosted CI; no production reproduction or accepted CI rerun is authorized by this evidence.

Next: new candidate hosted acceptance and sealed package bound to the actual ten restored container generations, then one new admission/READY/window. Previous b775-a1 is consumed and cannot be replayed. Owner authority/information remain resolved. Daily logical-backup administrator compatibility, backup transition/deployment, Owner use, three actual trading days and C_R2 remain OPEN.


## Wave127 actual G4 accepted — 2026-10-20 09:47 UTC

R2-04/R2-09 remain DOING. Tool `b4f8543cab300ed385a234103330c5882be6c2a8` completed CI39399897233 attempt1 with all8jobs at09:41:56UTC. Full8logs/7checkouts,222required results,73maintenance cases,1464pins/548blobs and the protected-script Docker regression/full encrypted recovery were accepted. Hosted evidence SHA256 `539fb1a01f894e73b4e120996e8159ad358862f2f8b92bfb4e941ee837b231d3` preserves exceptions and initial corrected metadata; no accepted CI was rerun.

Actual b4f8543-a1 staged34files and prepared58queries. Independent recovery READY preceded the outage. All32 original command clients exited0: five timers/nine writers stopped, old-schema audit succeeded, original PG stopped normally, encrypted five-class capture and private physical PG restoration/audit succeeded, and original services/timers were restored. Original root coordinator3077098 (transport3077096) exited0 at09:45:42UTC. Recovery/requester both exited0 with MainPID/ControlPID0 and recursively absent cgroups. The collector verified13stable container policies, ten healthy restored services includingPG, three untouched containers,56units/fragments, five active/waiting timers, unchanged current/retained targets and the same PGparent restored to0701. Newly observed container/volume timestamps are preserved rather than overwritten by earlier generations.

The exact67782byte collection SHA256 `8e3ef4c72075f094e25a7dce7ce115b5937dbfe0664d631683afc2fb0f885647` is bound to that original successful process. A separate read at09:47:52UTC confirmed its exact bytes and file identity. Closure `target/r2-daily-beta/wp-8/wave127/actual-g4-accepted-closure.json`, SHA256 `8f053aff369d515f908316f8027bd495bb29155e576b35bd1d612322403cbb85`, accepts actual G4. The encrypted accepted set is `/srv/backups/lagrange/preupgrade-edbdf59e094d86d4e8fb7a07683ff320`. The attempt/window is consumed; no replay. Failed attempts, original unknown causes, historical units and0120 cleanup INCOMPLETE remain preserved. The successful path does not establish production fault-injection coverage for G4-DRAFT-01..04; their focused integration review remains separate.

Next: correct the official daily logical-backup assumption that a postgres login exists; the actual administrator is lagrange. A three-file proposal exists but is not yet applied, tested or accepted. The installed-release-only backup dispatch and commit-bound image manifest require a supported integration path; neither may be bypassed. Accepted application e639169 and its12images remain immutable and undeployed. Official backup transition/deployment, Owner use, three consecutive actual trading days and final C_R2 remain OPEN. Existing blanket authority and actual Owner information remain valid; no approval/information wait.


## Wave128 daily backup administrator and independent tool package — 2026-10-20

Actual G4 remains accepted under b4f8543 closure8f053aff. The daily logical dump now uses the existing lagrange administrator through the same read-only local socket command; no production role is created. The disposable logical restore creates only a lagrange NOLOGIN/NOSUPERUSER identity to preserve ownership and ACL references. Its own freshly initialized postgres administrator remains correct. The Docker fixture now mirrors the actual source cluster (lagrange administrator, no postgres login), audits through migration_owner and adds actual no-role-creation dump and separate-tool verifier restoration cases.

The official backup path now permits a separately versioned immutable maintenance package without editing accepted application e639169, its12images or its strict image manifest. `scripts/ops/lib/production-backup-tool.py` reads only an exact Git archive of operations/database/configuration sources and four backup units. It publishes a new `/opt/lagrange/maintenance/<tool-commit>` with exclusive files and a manifest last; existing/partial roots are retained and never repaired. Runtime validates root-owned nofollow ancestry, every member's bytes/hash/mode and the exact application commit. Protected backup config must provide both DAILY_BACKUP_TOOL_COMMIT and DAILY_BACKUP_TOOL_MANIFEST. The runner then dispatches the new controller with the explicit current application release; its original installed image manifest, current-link check, reader floor, release/backup locks and data/config bindings remain mandatory. Partial tool configuration cannot fall back to old or legacy code. Preflight performs no Docker action. Restore scripts come from the validated tool; application configuration and images continue to come from the accepted application.

Transition contract v2 adds exact tool commit/manifest binding while preserving the existing six destinations, old/new byte hashes, bounded contract, identity-based recovery and inactive timers. Its source helper/checker/units come from the validated tool; COMPOSE_FILE/ENV and new_commit still identify the current application. Version1 remains supported unchanged. Installation and transition do not start timers, call a provider, build application images or switch current.

Local real-root isolated filesystem/controller tests:84backup cases passed10:08:55UTC process3146116exit0 (stderr81dc1798ec38cf2b3b4b093a79841eff85b7d2edc208f1d46c2efd61935d9317);26transition cases passed10:06:10UTC process3140078exit0 (stderr9b0832f43210f269158f5044915c5e350b53f6f238014763df5105bd18cdf432). Earlier75/83case receipts remain preserved. New coverage includes exact Git bytes rather than working-tree bytes, archive link rejection, existing/partial retention, source tampering/links/modes, application/config mismatch, fixed runner dispatch and six-file recovery without application mutation. Production ops static checks and syntax checks pass. Actual Docker tests are pending fresh candidate CI; no new root tool installation, backup transition, application deployment or provider call occurred. R2-04/R2-09 remain DOING; Owner use, three actual trading days and C_R2 remain OPEN. No approval/information wait.


## Wave129 separate backup-tool verifier mount correction — 2026-10-20

Candidate d56be05 CI39406284330 attempt1 exposed one actual Docker failure in DailyBackupDockerTests.test_separate_tool_verifier_restores_without_editing_application. The policy job printed410 named cases:409passed and this oneERROR. Its create arguments selected the separate tool scripts, but isolated_policy still expected the application script paths, so the exact mount check correctly rejected the mismatch before verifier startup. Original log hash b75e3487fc17f1509b13e225042f6752dd1419703d83bbd31f21e2db8f275e1a and the failed candidate/package remain preserved; no original workflow is rerun or cancelled. Other jobs may still finish independently.

The correction computes one verifier_root and uses it for both create arguments and inspected mount expectations. Exact mount equality, readonly flags, image/user/network/resource isolation and cleanup checks remain mandatory. No accepted application or installed source is changed. Local actual-root Docker suite13cases passed at10:23:45UTC, process3174381exit0, stderra309f9b45425b7a5b19f612b0d7ef8a5704a3e49b37c88a6959939fe55cc1991, including the original failing case, source-admin/no-role-creation check and full encrypted schema restore. The existing failing case is the regression; no duplicate test is added.

R2-04/R2-09 remain DOING. Actual G4 b4f8543 remains accepted; correction requires a new exact candidate CI before separate tool installation. Previous prepared d56be05 bootstrap is retained and must not execute. Application deployment, official backup transition, Owner use, three actual consecutive trading days and C_R2 remain open. A bounded read of current protected environment confirmed that new daily first-session/budget inputs are not present there; no synthetic values are adopted. Authority and the earlier custody/external-writer answer remain resolved.


## Wave130 accepted backup tool installed and deployment bindings — 2026-10-20 11:03 UTC

R2-04/R2-09 remain DOING. Corrected tool `e5638de80731a7d0fe6b4b89823f1ae660d19505`
completed [CI39407989346 attempt1](https://github.com/sihioov/Lagrange/actions/runs/39407989346)
with all eight jobs successful at11:01:18UTC. Full eight logs and seven exact checkouts were
recovered at11:03:15UTC;237required results,1467input pins,549Git blobs and2165source-tree
entries matched. Policy410named cases, actual logical Docker13cases,73maintenance cases,
15tool-package cases and44wholeflow cases passed. Existing skips, ignored cases, non-Linux
exclusions and the unprinted outcome of one fragmented nested helper remain explicit.
Hosted evidence `ci/e5638de/hosted-terminal-evidence.json` has SHA256
`b1f6377c8fb17a117f5464cca8187a4c2c62bbb8020b5ebb0e5adc073ab1d2b2`.
The earlier d56be05 run finished FAILURE; all eight logs/seven checkouts were recovered in
`wave129/prior-failed-ci-closure.json` (88feaafd). Its uninstalled package must never execute.
Neither workflow was rerun, cancelled or duplicated.

After acceptance, the exact sealed bootstrap de136845 executed once as root Python -I -S -B,
cwd/, umask0077, with both new roots absent. Original root process3258314 (sudo3258312)
exited0 at11:03:46.822863UTC. The tool was installed at
`/opt/lagrange/maintenance/e5638de80731a7d0fe6b4b89823f1ae660d19505`;
172members contain3,245,685bytes. The27,072byte manifest hash is
`13437dc81098c8849d3fa5e8f4e109db1a6b9a83567d22cf4f488532f4441ec4`.
The exclusive installation terminal `wave129/actual-tool-install-terminal.json` has SHA256
`bf2c1fa468647269b86bc6ec850fce3a917d9d24f6222a2caa40e01153fea3b1`.
A separate read at11:03:49UTC verified all172files, permissions, manifest identity, exact
installation receipt and original process identity. Current still names fce9b905.
Installation did not change application files, units, database, current or secrets, and
did not call Docker or a provider. This is tool installation, not application deployment
or backup transition; the original installer is consumed and must not be rerun.

The focused review of G4-DRAFT-01..04 is complete for the consumed b4f8543 G4 execution.
`wave129/controller-focused-review.json`, SHA256
`3337026ce26873a409d80fef517de97215eb51104dddca9b9eb049584360e3b8`,
binds executed source hashes, AST deltas, existing integrated failure receipts and actual
b775 failure/restoration plus b4 success. It confirms terminal retention before persistence,
restoration under the original monotonic deadline after wall-clock rejection, original stop
response/cgroup drain before starts, and restoration independent of capture acceptance or
capacity. Changes since the earlier failure tests were reviewed explicitly. No new tests or
production fault injection ran in this review. This is not hard-crash protection: ambiguous
daemon responses, prior cleanup uncertainty and lost-host recovery remain unproven. Earlier
failed attempts, unknown inner causes and0120 cleanup INCOMPLETE remain preserved.

A bounded protected-environment/two-container read at10:53:33UTC and one READ ONLY database
query at10:55:12UTC confirmed the existing sole Owner UUID, entitlement reference/hash,
lease60/backoff30 and the existing owner_only/market_ws/shared_required branch. The API
omits an explicit MAX_JOBS_PER_OWNER; its exact old source supplies effective10, which is
recorded as a derived default rather than explicit configuration. No credentials or provider
bodies were emitted. The first inventory read failed; its receipt is preserved and its
exact original cause remains UNKNOWN. Reconciliation
`wave129/deployment-binding-reconciliation.json` has SHA256
`acfa59c4dedcc2e2c85b23f5367af20a7f55ed1ae2ec3b6eb7539e65a0eb80db`.

The existing blanket authority, beta local-key decision and none-declared external writers
remain resolved. One new asynchronous question requests the automatic recommendation start
basis: first actual trading day after deployment verification with measured budgets, or an
Owner-specified historical catch-up scope. No answer has arrived; no date, budget or runtime
configuration was invented. This is a configuration selection, not renewed permission or
the old custody/writer question. After that selection, bind measured data limits and preserve
existing approved runtime grants/modes, install accepted application e639169, prepare its
schema/disabled runtime, then use the official version2 backup transition. Normal KIS apply
still requires the before16:30KST window; later install-disabled does not bypass activation.
Application deployment, actual Owner use, three consecutive actual trading days and C_R2
remain OPEN. The four current review findings are closed only in the scope stated above.


## Wave131 actual Owner start rule and measured protected inputs — 2026-10-20 12:40 UTC

R2-04/R2-09 remain DOING. The Owner answered the existing question:
“배포 검증 후 첫 실제 거래일부터 시작. 기존 운영 한도를 유지하고 새 파일 처리 예산은 실제 데이터 규모로 산정”.
`wave131/owner-deployment-start-decision.json` (d02d724d) binds this actual
message without inventing a message ID. Configuration selection, the old
custody/writer questions and operating authority are all resolved. Historical
automatic recommendation catch-up was not selected.

A bounded metadata read at12:33:37UTC held nonblocking shared Raw commit locks.
The two scopes contain14unique batches,415source files and1,372,250retained bytes
(manifest, batch metadata and source sizes combined). Source bodies were not read.
Raw ownership is10001. The five initially inspected daily/artifact paths were
absent; the actual historical artifact root was then resolved from the unchanged
protected env as `/var/lib/lagrange/data/artifacts/historical-price-beta-root`.
Its two entries include the fixed approved V3 base, with2,189manifest bytes and
7,672,914bars bytes. Metadata is not a new content-approval claim.
Measurement receipt4595a101 and binding receipt74d57146 preserve these scopes.

The exact installed calendar validator777a4f31 verified the existing artifact,
manifest and override ledger. Artifactb4caff42 covers2020-01-31..2027-12-31.
It lists2026-10-21 as the next session after the decision day. This is calendar
schedule evidence, not an actual trading-day receipt. The candidate first session
is bound to actual deployment verification before2026-10-21T00:00:00Z
(09:00KST) and actual market/input acceptance. If that bound is missed, no daily
activation is allowed under this candidate date: use a supported new immutable
configuration for the next valid session, never edit an installed release or
backdate verification. `activation-gate.json` is a coordinator requirement;
the application does not automatically consume this gate file.

New scan limits derive from measured current scope sizes, the36calendar sessions
between the approved base and the initial three candidate observation sessions,
and twofold headroom rounded to powers of two. This capacity-planning interval
does not authorize or dispatch historical collection or automatic recommendations.
The resulting per-scope limits are128records/batches,256files per batch,
8,192files,8MiB manifest,256KiB batch metadata,128KiB source file,64MiB retained
bytes,16,384descriptors and JSON depth16. The fixed in-container data-root path
uses5bytes/one component; this limit is pathname size, not disk usage.
Artifact enumeration and replay each allow128entries. The300second cycle guard
retains the existing computation ceiling; the metadata timing is not a Rust
throughput benchmark. Existing capacity10, lease60, backoff30, computation300
and immutable CPU/memory limits are preserved. Exact accepted app e639169's
validator accepted the1,279byte configuration with SHA256
`9e9dc09db87dfee3c9fa6242cbca1ee98c861f17d8b5366a1e634e835c3bea4f`.
Derivation receipt26987e5f records formulas and limits. No passed test, app smoke,
image build, provider call or CI run was repeated.

At12:40:39UTC a single root Python -I -S -B invocation, cwd/, umask0077,
published new root0700 input directories and root0600 files under
`/etc/lagrange/release-inputs/e639169-20261020-wave131`.
Only the release commit and five explicit daily/operating settings changed in
the copied protected env; every other environment line was preserved. The old
env hash and relative current link were rechecked unchanged. New env3499280a,
config9e9dc09d and gate89feb0f7 were read back with exact identities; publication
receipt336799e1 records exit0. Backup key contents were not read, copied, inserted
into env or rotated. Existing app, current, DB and services were not changed.
This publication is consumed; no overwrite, repair or retry.

Next: bind fresh deployment admission and the exact install/disabled preparation/
backup-transition/restoration command ledger, including new DB role/runtime
credential prerequisites and preserved market_ws policy. Actual daily input
completeness and resource behavior remain operational checks. Accepted G4 and
the installed e5638de backup tool are not rerun. KIS normal activation remains
before16:30KST; later install-disabled is allowed, with no timer-start bypass.
Application installation/deployment, official backup transition, Owner use,
three consecutive actual trading days and final C_R2 remain OPEN. All older
failures, unknown causes, retained artifacts and0120cleanup INCOMPLETE remain.


## Wave132 actual deployment, daily backup and legacy floor correction — 2026-10-20

R2-04/R2-09 remain DOING. Owner authority, external-writer declaration, local
beta key choice and the actual first-session/operating-limit decision remain
resolved. No new approval or information question was opened.

New daily DB source/runtime credential files were provisioned at13:07UTC;
existing seven DB credentials were unchanged. The initial static validation
rejected the stale operational-session date. A manual official KRX regular-hours
review and a separate protected same-day window resolved that input; the denied
holiday-page retrieval was not accepted as holiday evidence. Config validation
then passed, with all12existing cached app image identities exact. The protected
backup configuration preserves the20GiB/30GiB/14day/minimum3 limits and binds the
already accepted independent tool e5638de.

The official app installer and six-file backup transition completed at
13:18:11UTC: original root3507971 exit0, result10764bytes SHA256
eb7cc58f4490099a7c7bda0e714046e81f799fb068a9ec7c8b05b21cc9c346af.
The first rollout failed before any stop or apply because an unordered Docker
Mounts comparison was too strict. Its failure/unit remain intact. The corrected
full-policy canonicalization passed three focused new cases, retaining six
earlier controller cases; there was no accepted app/image change.

Distinct rollout a2 stopped three remaining periodic timers and ten original
containers, fully resolved all original clients/drains, retired only the exact
normal-stopped superseded owner-beta container without force or volume removal,
then used the official pinned-image release apply and disabled daily preparation.
Its19mutating commands all exited0. Original root3534564 exited0 at13:31:58UTC.
Exact result52808bytes SHA256
ef384697dcfd7c46e4e5dfa6b06860e0cbe9b9770813d4e14b104e7de47880ae
and the original systemd terminal were independently recovered. The initial
collector failure is retained; access-time sensitivity is an inference from its
code, not a proven recorded difference. The successful collector excludes atime
from identity stability. Migrations0060–62 were applied; a separate read-only DB
observation confirms62successful migrations, zero active jobs/running attempts
and daily control OFF/epoch0/first_auto_session2026-10-21. Current and the reader
floor are e639169. A link-only rollback cannot undo this schema/floor change.
New app container identities and actual restart times are retained. PostgreSQL
kept its original identity; the historical research worker remains exited2.

The corrected installed daily-backup helper ran once,13:34:43–13:35:46UTC.
Original root3546284 and child3546287 exited0, with VERIFIED_DISABLED.
The actual set is
`/srv/backups/lagrange/backup-daily-20261020T133448Z-e639169018519fd6636c5579d1957e323e2c7802`.
Encrypted DB/files/state and the official isolated networkless restore verified;
the exact terminal, COMPLETE/latest metadata and owned-verifier cleanup were
independently recovered (observationf44a351d). No additional restore was run.
Nine other app services are healthy; owner-beta is intentionally stopped with
exit0/restart=no under the OFF backup contract. The two backup timers resumed
at13:37:32UTC, completing restoration of all five original timers.

The first installed KIS coverage check omitted the process-level release commit.
The shared DB helper substituted compose-config-disabled, causing a denied image
pull and a build attempt that failed at its exact-commit check before any Rust
compile. This failure is preserved, not described as “no build attempted.”
The corrected pinned-image read-only check passed and measured33missing EOD
sessions between2026-08-18 and2026-10-20. Despite its “No Docker” display text,
the check actually used one db-migrate query container. The explicit missing list
is data-only maintenance, not historical automatic recommendations.

Manual missing-EOD a1 ended at13:40:00UTC: original root3558473/child3558502
exit1, permanent DATABASE_UNAVAILABLE, before the first date completed. The
bounded PG error is “candidate instrument conflicts with canonical instrument
master” (the function uses SQLSTATE23514). Source order places this error in the
initial price recovery before the date loop and before lazy provider issuance;
this is code-backed attribution, not a packet capture. Its unit/state remain.

Read-only comparison at13:48:54UTC validated all eleven stored normalized
reference files and all eleven DB instruments. Only listed_at differed:
legacy platform floor2020-01-31 versus the already approved2016-08-29 used by
the accepted worker. Historical review and the current catalog explicitly say
this column is a platform coverage floor, not an exchange listing date.
No reference body or instrument name was copied into diagnostics.

A standalone exact-row SQL correction passed six real isolated PostgreSQL cases:
eleven-only success/unrelated-row preservation, replay rejection, name drift,
missing row, differing floor and an extra same-symbol row. Failed cases leave
all rows unchanged. The owned networkless fixture exited0 and was removed.
At13:52:01UTC, under the retained release lock and shared Raw lock, one
serializable transaction compared every original row SHA256 and changed exactly
eleven floor values. All other fields remain identical. Original root3582475 and
client3582511 exited0; root receipt4228bytes SHA256
854b8b7243cbc7715ef135bb792f66d0fd4165aa28f8e2f0dd5f649530e2c69f.
There is no SQL-function/schema/accepted-release modification or listing-date
claim. The earlier encrypted backup and per-row before/after hashes retain the
original state. A distinct a2 data attempt checked all eleven committed hashes
and started at13:52:32UTC; original a1 is not reset or automatically resumed.

Daily recommendations remain OFF. New KIS units remain installed disabled;
normal apply is limited to before16:30KST, without a direct timer-start bypass.
The first2026-10-21 candidate still requires the actual deployment/input gate
before00:00UTC and actual trading-day evidence. Owner use, three consecutive
actual trading days and final C_R2 remain OPEN. Every older failed attempt,
unknown cause, retained unproven volume and0120cleanup INCOMPLETE is preserved.


## Wave133 cash-label correction and explicit EOD Raw source integration — 2026-10-20

R2-04/R2-09 remain DOING. Owner authority and actual deployment-selection reply
remain resolved: first actual trading day after verified deployment, existing
operating ceilings retained, new file budgets based on measured data. The
Oct21 candidate still requires verification before00:00UTC/09:00KST; no
historical automatic recommendations or fabricated three-day acceptance.

Missing-EOD a2's original root3583431/child3583494 ended normally with exit1 at
13:58:30UTC. Eleven dates were published; Sep15 Raw was committed, then legacy
normalization returned KIS_NORMALIZE_UNSUPPORTED_ACTION. Its state/unit and
original failure remain consumed. Cash-only R2 handling does not justify
weakening the legacy normalizer or resetting that failed run.

Bounded hash-verified, read-only source diagnostics found one ETF cash-dividend
row with zero stock-dividend rate and no stock/odd payment date. All complete
range/scope checks passed. The unchanged R2 reconstruction nevertheless failed
INVALID_ACTIONS; a diagnostic copy localized InvalidLiteral to the ASCII-only
stock_kind rule. The actual label comprises two precomposed Hangul characters,
six UTF-8 bytes; no provider body or label value was copied into diagnostics.
The official KIS sample at2077773a69a428daf63e148182511f068cc45de3 identifies
stk_kind as the stock-type field. No official sample was executed.

Only that internal wire field now admits precomposed Hangul U+AC00–U+D7A3 within
its existing32UTF-8-byte bound, alongside the old ASCII alphabet. It stays opaque;
no translation, trimming or financial interpretation was added. Other code
fields, numeric/date guards and noncash rejection retain their prior behavior.
Existing empty/ASCII preimages remain exact. A newly verified matched
producer/admitter/runtime release is required; installed e639169 is untouched.

At14:36:38UTC the corrected private verifier successfully reconstructed the
actual Sep15 evidence: seven witnesses,77selections and one cash row. Document
SHA256 e6c9c0b0691a31d686c124243ee3028295e86ca5d14d273718659a9ca7a9d743;
Raw manifest SHA256
2009b5eac887c89f3bf539a1c554982e95a8b2d12e46b457b41526fcb13fd5a0.
This was a read-only probe, not an artifact admission, DB write or deployment.

The explicit research-worker --eod-raw-session-dates path now captures/reuses
complete EOD Raw without canonical normalization, Curated publication or DB
construction. A conflicting, partial or corrupt source fails before refetch.
Current-day execution holds the existing durable calendar lock/claim through
capture. Historical and current-day dates cannot share one invocation; the
current day is rejected before16:30KST. A separate check command reads committed
bytes only, without provider/DB construction or orphan reconciliation; it does
not create an absent Raw scope. The executing path retains ordinary durable
orphan reconciliation. Shared token/rate coordination and provider allowlists
are unchanged.

backfill-production.sh --raw-source binds a distinct EOD_RAW_STATE_V1 identity
and RAW_COMMITTED status. Legacy V4 and Raw state cannot be interchanged. The
relay validates exact date/UUID/phase and false ready/publication/curated/db
flags; typed failure preserves prior completed dates. A body-free error is
sent through the same stdout protocol. kis-daily-production.sh --raw-source
performs an independent current-session source observation, without consulting
the canonical DB frontier, and captures an absent source after cutoff. It is
an explicit mode pinned by install-kis-daily.sh --raw-source, not a silent
change to installed legacy units. It deliberately does not invent a historical
catchup range. Existing late-install/normal-apply boundaries remain enforced.


The pre-commit integration check exposed a second source issue: the historical
in-memory calendar cache carried the first request into later batches without
copied_from. At15:09UTC bounded committed manifest metadata confirmed13such
calendar records among23. New historical Raw collection now captures one
dedicated committed calendar original and retains exact copy references for
later covered targets. Both original and requested target rows are checked
before non-calendar calls; missing coverage or tampering fails without another
calendar request. The canonical daily API remains same-date. The Raw store's
cross-date exception is limited to an earlier credentialed kis-calendar original
copied as a calendar into kis/kr; other kinds/providers retain their date rule.

Complete discovery, calendar projection and sealed-frontier refresh now exclude
an unlinked cached delivery only when a valid uncopied KIS original in the same
snapshot has identical bytes, filename, hash/size and request/response metadata,
with an earlier batch date and no later retrieval. The original supplies the
witness. The redundant file remains invalid as an explicit evidence claim; no
lineage, timestamp, Raw repair or deletion is invented. Unknown or conflicting
sources continue to block. A six-variant negative fixture retains those failures.

The first actual calendar probe at15:16:20UTC stopped with ENTITLEMENT_MISMATCH.
A separate bounded metadata inspection found9calendar-bearing historical batches
under a different reference. Discovery now selects only the protected configured
entitlement while retaining full snapshot integrity/scan budgets. Foreign
references cannot provide missing coverage or copy-chain evidence, and explicit
references still fail. This adopts no new rights and relabels no historical data.
At15:20:39UTC the corrected read-only probe passed:15eligible calendar candidates,
4verified witnesses,11redundant unlinked deliveries excluded,11total batches outside
the configured entitlement excluded (including noncalendar batches). Both exact
manifest hashes were retained: kis2009b5ea… and kis-calendar d85be240…. This is
calendar-source verification only, not DB projection, daily admission or activation.

Local verification:28wire+33reconstruction+25discovery+22evidence,39Raw-store
+7calendar-publication,8CLI+27collector-recovery and9daily-runner tests pass
(198distinct Rust tests,16new test functions). Seven new fake-host
source integration cases, nine Raw progress protocol cases and the new
installer-mode check pass, along with existing relevant shell fixtures.
Clippy -D warnings for market-data/collectors/job-queue and all targets passes. The full ops static
check passes in a separate source snapshot with Git-normalized modes. The
initial missing-Debug fixture compile failure, installer-preview omission,
workspace0775 mode rejection and two stale literal static assertions are
retained with their corrected runs. The cross-date storage rejection, a test-only
UUID dereference compile error and fixture directory-umask rejection were also
preserved and corrected; the independent R2 reader now resolves both new Raw
deliveries to their actual original in the focused collector test. This is isolated source verification,
not an actual provider capture or new runtime acceptance.

The remaining recurring civil-day action supplier is still only a pure D5
planner: there is no production caller of plan_action_supply_proposal. Current
EOD source collection does not close weekend/holiday action coverage. That
runtime connection, evidence-derived catchup, new candidate hosted acceptance
and immutable deployment, actual daily input admission/activation, Owner use,
three consecutive actual trading days and C_R2 remain OPEN. No old G4/CI/app
build/backup attempt was replayed and no provider request was made in this
correction phase. All retained failures, unknown causes and0120cleanup
INCOMPLETE remain. Source changes add no crate dependency, Compose service,
network/mount or DB-role/table boundary; architecture diagrams are unchanged.

Wave134 validation: all37reconstruction tests (4new) and market-data all-target Clippy pass. At15:40:01UTC the changed read-only verifier resolved all5actual errors without Raw edits or network calls:13civil days verified,40stillmissing; calendar Sep24..Oct6 stillmissing. Actual input admission, supplier wiring and activation remain OPEN.


Wave135 source validation (2026-10-21 KST): the recurring supplier is now wired
through the explicit Raw-mode installer/wrapper and research-worker, with no new
Compose service/mount, crate dependency or DB boundary. Eleven supplier tests (including next-day frontier/independent reservation) and
nine CLI tests pass; all-target collectors Clippy passes. Seven new fake-host
supplier cases and two installer-mode checks pass along with existing relevant
shell fixtures. Calendar/gap/identity/timeout/claim tamper failures and existing
Raw reuse are covered. The first compile's missing iterator/fixture error variant,
Clippy item-order failure and literal-dotenv JSON rejection remain preserved.
The corrected nonsecret lowercase-hex configuration keeps the parser unchanged.
No provider call, actual new capture or runtime activation occurred in this work.

Candidate1cbe487 CI39447903390 attempt1 is completed success for all8jobs; full
logs and exact7checkouts were recovered15:52:39UTC. This observation is separate
from image/runtime acceptance and does not cover the subsequent c8483a4 or Wave135
source. Actual app e639169 remains deployed, dailyOFF. Source gaps from the last
15:40:01UTC read-only observation remain13verified/40missing civil days, with
Sep24..Oct6 calendar absent. The new combined candidate still needs hosted gates,
matched immutable images/runtime, evidence-derived catchup, actual input admission
and activation. Owner use, three real consecutive trading days and C_R2 remain OPEN.

Wave135 final local checks: the normalized source snapshot passes the full ops static
contract, and workspace formatting passes. Source structure is unchanged, so
architecture diagrams are untouched. No old CI/G4/backup operation was replayed.


### Wave136 — build gate after an actual daily-OFF deployment (2026-10-21 KST)

R2-04/09 remain DOING. Read-only host observation at16:19:03UTC found
the deployed beta runner normally exited0, historical research exited2, and
the other9project services healthy. No service or provider action was made.
The original official build gate could admit only the research/OwnerV2 drained
pair, so it would reject the legitimate disabled beta service before a build.
No failed real build was submitted to rediscover this source-level mismatch.

Keep v1 unchanged. Explicit v2 includes exactly beta, OwnerV2, research, sorted,
with normal0 for both Owner runners and historical2 only for research. Bind the
root publisher's retained actual daily-OFF/zero-running-jobs observation, owner,
policy, activation epoch and installed source release. The helper validates this
operator attestation, not an independently queried DB result. Every checkpoint
still observes all11containers and binds all stopped lifecycle/identity fields.
Running/stopped drift, exit143, omitted/extra identities or changed binding fail.
The helper neither drains a service nor authorizes activation. Actual execution
still needs fresh admission, normal OwnerV2 drain and independent restoration.

Ten focused tests pass through the real protected reader/checkpoint functions.
The existing shared gate suite and the official full fake-Docker image suite
pass; the latter now exercises the v2 triple through actual helper initialization
and all build checkpoints. Full normalized ops static checks pass. Initial patch
context mismatch changed no test bytes and is not reported as a test failure.
No source dependency, Compose structure or DB boundary changes; diagrams unchanged.

Candidate18372ab CI39454506091 attempt1 remains independently recoverable; do not
rerun/cancel it or treat its results as covering this subsequent build correction.
Next: freeze the new source, one new exact-candidate CI, matched immutable images
with jobs2/one service per Compose and existing independent recovery, then bounded
actual input recovery/admission. Oct21 auto start remains conditional on validation
before09:00KST, never retroactive. Owner use/three actual days/C_R2 remain OPEN.


### Wave137 — hosted closure and bounded image-build recovery (2026-10-21 KST)

Candidate fccb68354c5352f741c37764ae1d66eb3784a662 CI39457995014 attempt1
completed all8jobs successfully. Full8logs, exact7checkout commits, frozen57Rust
and10Python definitions, and19actual PostgreSQL processes with exit0/no skip
were recovered at16:57:15UTC. Hosted source closure a15dcbaa... (46145bytes)
is source acceptance only; no new images or deployment are claimed. Prior
18372ab CI39454506091 also completed and was independently recovered, closure
c205cf16.... Its required-log delimiter/module qualification recovery corrections
preserve the original frozen inputs and observations. No CI was rerun or canceled.

At16:44:31UTC a read-only preparation observation confirmed deployed e639169,
13total containers/11monitored, beta normally stopped0, research historical2,
other9healthy, dailyOFF/epoch0 and zero running beta recommendation jobs. The
current Owner V2 identity is62d49f86.../image9e994794..., not the predeployment
f293... identity. A narrow independent build supervisor now owns separate original
stop/build/start units and responses, requires normal Owner0 plus original client
termination/MainPID0/ControlPID0/empty recursive cgroups before a single start,
and retains all receipts/partial failures. Receipt failure after a resolved stop
still routes to restoration. Large completed build logs are streamed for hashing.
The fixed12hour window is never renewed; deadline/uncertainty remains INCOMPLETE.
Fourteen isolated ordering/failure cases pass. One actual USER-manager fixture
confirms retained original process exit0, exact output and empty cgroup; it is
not root Docker/build admission. Original fixture unit/evidence remain.

Root preparation a1 ran once16:59:25UTC and failed before staging/outage: its broad
KIS-name check mistook preserved lagrange-kis-ws-a65f208-build.service active/exited
for live collection. Read-only diagnosis17:00:16UTC verified its original Oct7
normal0, MainPID/ControlPID0 and empty cgroup; a1AUX was absent. The a2 correction
admits only that exact historical unit/invocation/PID/terminal tuple and empty
cgroup; positive and11negative boundaries pass alongside14control regressions.
No historical unit is filtered, reset, deleted or restarted.

The sealed a2 bootstrap d357d8a4... was submitted once17:01:50UTC, but sudo returned
`interactive authentication is required` before root Python execution. It did
not create an operating window, issue a stop or start a build. At17:03:00UTC all
4proposed a2units were not-found with PID0; AUX metadata was UNOBSERVABLE from
the unprivileged caller, not claimed absent. Both local launch receipts remain
consumed; no automatic replay. Owner authority remains valid and no new approval
or information question is pending. Resume a newly sealed attempt only after
actual noninteractive execution authentication becomes available; do not bypass it.

Heartbeat d18b4f11 was confirmed active and its exact create receipt plus handoff/
registry persisted before predecessor40f9f633 deletion. Renew by2026-10-21T03:30Z
before04:40:28Z expiry/47thtick. This is same-agent tracking, not native Ralph
or outage protection. Fresh build images/runtime, measured calendar/action/EOD
gap recovery, actual producer/admitter input acceptance and daily activation,
Owner use, three consecutive actual trading days and C_R2 remain OPEN. Existing
G4/app/backup acceptance, all consumed attempts/unknown causes,0120incomplete
cleanup,32unrelated ROADMAP additions and.archify remain unchanged.


### Wave138 — Owner authentication restored and original a3 build running (2026-10-21 KST)

Owner explicitly renewed 72-hour sudo in the prepared interactive terminal.
At2026-10-21T04:47:03UTC, `sudo -k -n /usr/bin/id -u` returned exit0/UID0,
independent of a password timestamp. The revocation timer is enabled/active/waiting
with Persistent=yes and deadline2026-10-24T04:46:30Z (13:46:30KST). This verifies
current execution availability, not that future revocation has already executed.
No password, provider credential or backup key was read or copied. The original
reviewed sudo helper is unchanged; prior authentication failures remain recorded.

Fresh preparation observed13containers/11monitored, deployed e639169/schema62,
dailyOFF/epoch0, zero running beta/Owner jobs and settled stream work. Root-only
Git index readability prevented the first local sealing check; the failure is
retained and the exact clean fccb683 source was verified using read-only root Git,
without chmod/chown. The distinct a3 controls only rename the reviewed a2 unit/
AUX/input paths. Their syntax, exact substitutions, fresh lifecycle/work binding,
source and hosted closure hashes were checked; passed tests/CI were not repeated.

Bootstrap f4025005..., seal8ecccaf3..., was submitted once at04:51:04UTC.
Independent root recovery1136983 published READY before the final request. The
original Owner62d49f86... stopped normally0; original stop process1137237,
response, MainPID/ControlPID0 and empty recursive cgroup were verified. The build
process1137634 runs the official12-service builder, four batches of three with
one Compose service at a time, CARGO_BUILD_JOBS=2/COMPOSE_PARALLEL_LIMIT=1,
Nice10 and idle I/O. The original same-boot window ends16:51:04.248235UTC and
must never be renewed. The independent supervisor owns the single owed restore;
do not issue a competing stop/start or replay any consumed attempt.

The bounded04:55:30UTC observation74b6b506... recorded db-role-bootstrap and
db-migrate service success and api-server still building. The run-start,
prepare:api-server and native:D1 gates passed, with no OOM in their recorded
journal scope. Those gate timestamps precede this progress read. Available
memory was6497600KiB and swap3724492KiB at the progress read. Build and recovery
units were still running; no build/start terminal or restored/result receipt was
present. A completed image set, restoration and deployment are not claimed.
The observation's lowercase ready.json/ready-external.json absence is not a
readiness failure: the actual control files are READY.json and
ready-external-observation.json, and the original bootstrap receipt confirms READY.

Current running handoff8d3f2c52... and owned-workers.json supersede the old sealed
submission instruction and authentication-blocked heartbeat prompt. Heartbeat
0f8c84f7 expires15:01:59UTC; renew by13:50UTC with exact successor receipt and
handoff/registry before predecessor deletion. This is bounded same-agent tracking.
R3 source preparation is separately authorized; shared production actions wait
for original a3 terminal and restoration evidence. No new provider calls, input
admission, daily activation or first trading session occurred here. The Oct21
conditional deadline expired at00:00UTC; firstSession remains null until actual
deployment validation and real calendar evidence. Owner use, three consecutive
actual trading days and C_R2 remain OPEN. All historical failures/unknown causes,
0120 incomplete cleanup, protected lock/volume/PG permissions,32unrelated ROADMAP
additions and .archify are preserved. Source/architecture did not change.


### Wave139 — image closure, immutable installation and daily runner binding correction (2026-10-21 KST)

Original a3 completed at05:38:32UTC. Independent recovery at05:41:16UTC verified
all12 image IDs/revisions, the2597-byte V2 manifest1d6dd39e..., all49 PASS gates,
original stop/build/start process results and empty cgroups, and the same Owner
container62d49f86 restored healthy with its new05:38:26 generation. Closure
cf224e30... (857191bytes) also observed the previous e639169 runtime and daily
OFF/epoch0/zero recommendation jobs. No original attempt was replayed.

Protected deployment-only inputs were published05:36:29UTC; only the env and
JSON release commit changed. Their expired Oct21 lower-bound binding is retained
as historical configuration, NOT automatic activation authorization. firstSession
remains null. At05:44:23UTC the official installer applied fccb683 and its installed
check passed (receipt4b1fe1a6...). This installed a new immutable directory and
switched current; it did not replace running services or change the database.
The first runtime preflight failed operational_session_window_invalid. A fresh
05:34:14UTC public KRX hours retrieval (HTTP200,3490bytes,c8b1bdbc...) supports
regular09:00–15:30 hours only, not an actual trading-day/holiday assertion. The
official current-day window apply/check passed05:45:57UTC and the second release
preflight passed05:46:14UTC (74940d13...). No provider or calendar receipt was
fabricated; the actual-calendar gate remains independent.

Before runtime transition, source review found an integration gap: the daily
installer emits --raw-source [--action-supply] --execute, but service-status
validation accepted only the legacy --execute form and its --check invocation
omitted the selected flags. This blocks the intended recurring producer setup.
The gap was reproduced in isolated fixtures: four new test methods produced two
positive-mode errors and nine negative subcase failures on the original source.
The correction recognizes exactly the three installer-rendered commands, rejects
extra/duplicate/reordered/missing options and duplicate/reset ExecStart lines,
and forwards only the recognized installer flags to the read-only checker.
No mode is inferred from ambient env or enabled by this correction.

Changed source: service-status-policy.py, install-service-status.py and its focused
fixture tests. CI now explicitly runs the53 status installer/collector tests,
with a workflow-contract test requiring that step. Local status53 PASS, CI
contract21 PASS and production-ops static PASS. The initial CI-contract command
failed to import PyYAML in the default interpreter; its original result is retained.
The existing /usr/bin/python3 with PyYAML then ran the21 checks successfully.
These are local fixture/source checks, not hosted or production acceptance of the
correction. No architecture boundary changed; diagrams remain untouched.

At05:49:33UTC, the official compatible current-link rollback/check restored
e639169 while retaining the fccb683 installed directory and all image evidence
(receipt2b5883f2...). No runtime transition had been submitted, so there was no
service or migration rollback and no new stop/restore obligation. Schema62 and
the reader floor remain. Do not patch either installed immutable release. The
next candidate must include the corrected guard, freeze/recover its exact CI,
then use fresh deployment bindings and actual data-derived activation inputs.
The serving runtime remains e639169; dailyOFF, Owner acceptance, actual input
admission/automatic operation, three consecutive actual trading days and C_R2
are still OPEN. No replacement first session is asserted.

The separate R3 original CI39549529746 attempt1 reported a Web formatting failure;
its earlier local14 PASS does not replace skipped hosted tests. The existing R3
agent was given a bounded original-result recovery and formatting-only correction/
new exact-candidate CI task. It has no shared production window. R2 work was not
delegated. Preserve all previous failures, protected lock/volume/PG permissions,
32 unrelated ROADMAP additions and.archify. The same-agent heartbeat remains
0f8c84f7, renew by13:50UTC; it is not native Ralph or outage protection.

### Wave140 — corrected candidate hosted acceptance and original a4 build (2026-10-21 KST)

R2-04/R2-09 remain DOING. Exact source91324123d41c3d4e0057b2cb053fd36b03e2b7f0
fixes the daily raw-source/action-supply service-status binding. CI39552239217
attempt1 was dispatched once05:54:39UTC and accepted06:12:20UTC: all8jobs and
full8logs,7exact checkouts, frozen57Rust/84Python definitions and19actual PG
processes exit0. Closure ci/9132412/hosted-source-closure.json is91221bytes,
SHA256 b117ab67b466d115a64e3c9e213cb5721e8a1993fef6169af30864f9c11ec06a.
No accepted CI/tests were repeated; prior failed probes remain retained.

Fresh06:06UTC root preparation observed old e639169/current, dailyOFF/epoch0,
settled recommendation work and exact11 monitored containers. The new a4 names,
AUX and manifest were absent. Root Git verified the clean9132412 source. An
unexecuted draft with an overly broad name replacement was retained separately;
the actual package uses exact name/path substitutions and unchanged reviewed
restoration semantics. This is a distinct candidate build, not a replay of a3.

The sealed a4 was submitted once06:12:43UTC after actual independent root READY.
Its fixed same-boot window expires18:12:43.411991UTC and is never renewed. The
original Owner62d49f86 stop client1335165 exited0 with exact original response
and empty cgroup. At06:17:45.984495UTC the original build1335726 and independent
recovery1334901 remained running. db-role-bootstrap,db-migrate,api-server,web
completed; research-worker was building. Seven gates passed, no failed gate;
MemAvailable9367468KiB, SwapFree4009840KiB. This observation is limited to the
original units/cgroups, retained gate/terminal records, safe progress and memory.
It is not complete image acceptance or a new full runtime observation.

Evidence: wave140/a4/actual-build-progress-20261021T061745.json123582bytes,
SHA256 004d3db8e818453ba63bc723771953b8716889207dfd690f8a8eafa77a2da62c;
launch receipt1b6c3e5c; bounded handoff d97f250c2269be1d84ed4f96b350b3987432400f97b115e80edc9f7254ffe761.
The supervisor owns the single owed restoration after every original response
and process/cgroup has drained. No competing stop/start/build is permitted.
Next recover all12images/manifest,49gates and exact normal restoration before
new immutable deployment. Existing e639169 and fccb683 installations, schema62,
backup/G4 acceptance and all historical failures remain. Daily activation,
actual inputs, Owner use, three consecutive actual days and C_R2 remain OPEN;
firstSession=null. R3 has no shared production window during this original a4.
No structure changed; diagrams/.archify and32 unrelated ROADMAP additions remain.


### Wave141 — immutable serving deployment accepted; actual inputs remain open (2026-10-21 KST)

R2-04/R2-09 remain DOING. Exact candidate91324123d41c3d4e0057b2cb053fd36b03e2b7f0
completed the original a4 build:12 images/49 PASS gates and original normal Owner
restoration were independently recovered at06:30:22UTC. Closure e050dc200ea45d9761c1d64700e6cb9fb5fe3739e68036d1e7fb16e46f5dc81b
(857169bytes); manifest a0d1d5e33cccc10f006a7efaf98686fb5d14b1adae747f66c2a09858249b5409
(2597bytes). The official immutable installer/applicable installed check passed
06:30:47UTC. This retains prior e639169/fccb683 releases and all original evidence.

Serving a1 stopped before its runtime-transition intent: official status apply
rejected ACTIVATION_STATE_UNAVAILABLE. Retained journal999e38a9 shows the original
status timer ran after current changed but before its policy changed, leaving the
service failed. No serving containers changed in a1. No source patch, repeated
CI/test/build, reset-failed or manual status-service start was used to conceal it.
Distinct a2 restored matching old current, observed the original timer's natural
normal exit0, then stopped/drained only that timer across current/policy transition.
The official installer restored the timer. All a1 failures and files remain.

Original a2 submitted once06:35:35UTC. Official scoped refresh replaced API, Web
and OwnerV2 sequentially; original Owner normal exit0 is retained. The root command
and all original children terminated0 with MainPID/ControlPID0/empty cgroup. Direct
API /readyz and Web /healthz checks passed. Independent06:37:20.234192UTC closure
325ee05d9d25ace08a32e1ab7c03bafb59605599140bedd3537344a623622782 (62188bytes)
verified all three exact9132412 image revisions and healthy lifecycle, eight other
original containers unchanged, actual DailyOFF/epoch0/zero jobs, status timer
restored, KIS units disabled and original release lock retained. No restoration
obligation remains. This is scoped serving acceptance, not all-worker replacement,
authenticated Owner use or actual daily-input admission.

KIS raw-source/action-supply units were officially installed disabled; supplying
validated configuration/inputs and normal-time activation remain separate. No
provider call, fabricated first session or automatic-day observation occurred in
this deployment. The expired Oct21 candidate is not revived: firstSession=null.
Next derive actual source gaps, perform distinct bounded recovery and independent
admission, resolve real calendar/date binding, then official daily activation DBLAST.
Owner use, three consecutive actual trading days and C_R2 remain OPEN.

The existing R3 agent's228a8da CI39552229552 all8 acceptance was independently
checked. It lacks the five9132412 operating-guard source files, so a distinct
combined candidate/frozen hosted CI was authorized in the separate worktree.
No R3 production window or market connection has been granted. Shared operations
remain serialized; R2 frozen candidate/manifest are unchanged. The latest registry
handoff is wave141/serving-accepted-inputs-pending-handoff.json c65585e2...
Heartbeat0f8c84f7 renewBy13:50UTC. No structure changed; diagrams,32 unrelated
ROADMAP additions,.archify, historical failures and all protected paths are retained.


### Wave142 — actual Raw horizon failure corrected; R3 D1 integrated (2026-10-21 KST)

R2-04/R2-09 and R3-01/03 remain DOING; no completion checkbox changes.
Actual06:44:23UTC read-only reconstruction retained13 verified civil days,
40 missing and0 invalid over Aug29..Oct20; calendar Sep24..Oct6 remains missing.
The two original Raw manifests retain hashes2009b5ea... and d85be240... .
The exact installed9132412 research image, network-none and Raw-readonly,
then failed its source-presence check at06:46:05UTC with
KIS_RANGE_CALENDAR_RANGE_INVALID. It made no provider/DB/write operation and
its original process was reaped; retained container exited2/PID0. This is a
source-check failure, not a consumed provider-recovery run or source absence.

Both daily Raw entry points incorrectly used the frozen historical normalizer
calendar ending2026-08-28. Remove only that historical upper-horizon coupling;
retain the2016-08-29 lower floor, sorted/unique/nonempty/max10000/no-future dates,
current-day isolation and16:30KST capture boundary. The official wrapper still
selects the protected operational calendar. Raw capture and reuse independently
require an exact original KIS calendar trading-day row; closed-day calendar
evidence remains valid but cannot authorize EOD. Historical range normalization
and the committed calendar artifact are unchanged. No endpoint/host/TR/rate,
credentials, account/order or publication admission scope was expanded.

The original behavior reproduced9 PASS/2 FAIL. Five new regression tests plus
the existing suite now32 PASS; calendar publication7, KIS normalization30 and
historical range14 PASS. Focused collectors/market-data all-target strict Clippy,
format and diff checks passed. Intermediate fixture compile/umask/manifest and
expected-error mistakes are retained, not counted as product behavior evidence.
Local validation receipt wave142/local-validation.json is5139bytes,
SHA256551c13390dfe80f236f9ed2a8fd36f254f88729b3dc4a3eaabf84ea22380c1ef.
These checks do not establish a new deployed image or actual input admission.

R3 candidate f966b2b9f338b4559ec1a6257393cddcf7aa91c6 CI39555456064 attempt1
completed all8 jobs; full8logs/7exact checkouts/57Rust/84Python/19actualPG and
page-seam14 were recovered. The parent independently verified the packet and
qualified recovery/manifest hashes ccb0d5ee...,ee052557...,e3056ff7... .
The original49-name extractor record is preserved; unique frozen definitions
and complete PASS blocks qualified57 without rerunning CI. The parent reviewed
and imported exactly the two D1 blobs: selected instrument/generation/READY
produces one stream membership, otherwise none; board behavior is retained.
No R3 roadmap/document tree was merged. This combines the approved UI correction
and Raw fix in one new candidate; its exact hosted CI/build/deployment is next.

Additional06:53:08UTC TLS-verified requests through the original proxy at its
observed container address returned /login307 and unauthenticated market-stream401.
No body/cookie persisted and this is not authenticated Owner/live reception.
The preceding localhost443 assumption produced connection refusal and is retained;
actual proxy8080/8443 ports were not host-published, so it is not an app failure.
The accepted9132412 API/Web/OwnerV2 and unchanged eight other containers remain
the installed state; all prior build/restoration obligations are resolved.

Next freeze the combined source and89Rust/84Python definitions, dispatch its
single CI, then build/install with fresh actual identities and independent
restoration. Afterwards repeat source presence in a distinct fixed-image check,
derive bounded missing-input recovery, independently admit inputs and resolve
first-session binding before official daily activation DBLAST. No first session
is asserted. DailyOFF/epoch0, actual days0 and C_R2 remain OPEN. R3 shared operations
remain serialized; source/CI acceptance is not market/Owner acceptance.
No architecture boundary changed; diagrams,32 unrelated ROADMAP additions and
.archify remain untouched. All old failures, original manifests and protected
lock/volume/PG/backup-key boundaries remain. Heartbeat0f8c84f7 renewBy13:50UTC.


### Wave142 deployment — combined866 accepted; actual Raw recovery started (2026-10-21)

R2-04/09 and R3-01/03 remain DOING. Exact combined candidate
866d96f20d29c9b9ff3507e9f290c4729b809a21 completed CI39559138658 attempt1:
all8jobs, full8logs/7exact checkouts,89Rust/84Python,19actualPG exit0/no-skip,
and14hosted page-seam tests. Closure ci/866d96f/hosted-source-closure.json
109774bytes SHA8ea35d78a15a1d0af88d05595c038cf4bc6db77ab3203155cc605099d120f7b2.
The initial strict extraction associated85/89 Rust results because stderr from
the next binary interleaved in one nine-test block. Its original record/logs
are retained. Unique frozen definitions and the complete nine-PASS block were
qualified separately (df89c1f8...); no tests or CI were rerun.

The original a5 built12images sequentially in four logical batches, with
49PASS gates. Original stop/start clients returned0 and the exact prior Owner
was restored; independent closure at07:46:22UTC is857266bytes,
SHAe2a67b107258f74c7443ad41321ce4b0dadd5b88126c151e5a65645ab92500b1.
Manifest2597bytes SHA6c2fb8356b7fd807d5168a89a303ed3e71853f2d536733b4379a31e30da3234c.
All original units are terminal with MainPID/ControlPID0 and empty recursive
cgroups. No build/restoration obligation remains, and no repeated build is needed.

Serving a1 installed and checked the immutable866 release, installed KIS disabled,
and moved the status policy/timer through the official installer. Its refresh
then failed before container changes: private control conflated serving source913
with the stopped daily cohort's actual sourcee639. Preserve failed a1 and all
successful preceding steps. Actual read-only diagnosis07:48:40UTC rejected913
with DAILY_RUNTIME_OWNER and accepted the exact e639 cohort; diagnosis SHA
26f5aa51a29f7bd48e1bfc700001ce0d9059a99f86934ab1169dd3b4db7b335a.

A distinct sealed serving a2 used serving-from913, daily-frome639, target866;
it did not repeat installation or status/KIS transitions. Original supervisor
1590566/invocation52d47e1a8b9b49098dc2fe96442f8f79 and every child exited0.
Independent07:54:23.647211UTC closure42776bytes SHA
8bb4d61ed43bd6f53c623fe5be3618f45822e93fbcb36312903b871005c52ccf
accepts exact API/Web/OwnerV2 images/revisions, health/HTTP, normal prior Owner
exit0 without SIGKILL, eight unchanged containers, existing status timer restored,
KIS disabled, DailyOFF/epoch0 and preserved release-lock identity. This accepts
scoped serving deployment, including R3 D1. It does not accept authenticated
Owner use, market reception, actual input admission or all worker refreshes.

The deployed research image's separate network-none/Raw-readonly check succeeded:
33historical sessions checked,12present and21missing from Sep16 through Oct20.
No credentials, provider or DB were used. Receipt wave142/eod-source-check-result.json
8920bytes SHA33d63b0f3f4e7ab8328d7cf651cd82656bd9f3dc833b81a8ffe876e2b8af00a0.
This demonstrates the deployed Raw horizon correction; it is not a missing-input
recovery result. Original source-check failures remain retained.

A new source-aware recovery a1 was submitted once07:58:08UTC through the official
backfill wrapper with --raw-source. Its six missing sessions Sep16,17,18,21,22,23
are selected from the actual source check and existing original calendar coverage;
no old publication attempt is replayed. Source/systemd/capsule/state identity are
new and pinned to866. The original release lock is held shared, research remains
historically stopped, existing per-channel/token/retry limits remain. One original
calendar lookup and six*(22price/reference+70action pages), each up to4attempts,
give a conservative2212 wire-GET ceiling, not a claim of actual calls. At07:58:36UTC
root1600951/invocatione33def22bf1b493697b27819049f7cf7 was running; the official
plan had exited0, execute had not reached terminal and no date completion was yet
recorded. Observation SHA2cea1ecf0fcb036de096fa8ac46da64aff70ad4afc86b580e420c15e31327e79.
Recover this original process/state, then independently reconstruct committed Raw;
never replay a failed/uncertain invocation or count RAW_COMMITTED as admission.

Next close measured EOD/calendar/action gaps, independently admit inputs and bind
the first actual session from deployment validation plus real calendar evidence.
The old Oct21 configuration is expired; firstSession remains null. The official
current control tool cannot rebind an existing first_auto_session; supported
preactivation forward-rebinding work remains before official daily activation
DBLAST. KIS normal installation deadline has passed today: retain install-disabled,
no direct timer-start bypass. Owner use, three actual days0 and C_R2 remain OPEN.
R3 shared operations stay serialized while R2 source recovery owns the current
window. All previous failures/unknowns, protected volumes/PG/backup key and32unrelated
ROADMAP additions/.archify are preserved. No structural change or diagram update.


### Wave143 — partial Raw recovery and preactivation first-session binding (2026-10-21)

R2-04/09 remain DOING. Serving866 remains accepted; no later runtime change.
Original six-session Raw a1 exited0 at08:01:12UTC, and original fifteen-session
Raw a2 committed11 sessions before exiting1 at08:08:09UTC on Oct15 with
BROKER_AUTH_FAILED. Both original supervisors/children are terminal and drained;
no restoration is owed. Preserve all partials and consumed attempts, without replay.
The fixed safe diagnostic exposes no subordinate cause: UNKNOWN, not evidence
that the broker rejected credentials or that sudo is unavailable.

Independent network-none, Raw-readonly verification at08:08 confirms29/34 EOD
sessions present; Oct15,16,19,20,21 remain missing. Original calendar reconstruction
covers Aug25..Oct30 continuously. Civil action coverage Aug29..Oct20 is30verified,
23missing. RAW_COMMITTED and these checks are not canonical input admission.
Evidence under target/r2-daily-beta/wp-8/wave142:
- eod-raw-a1-progress-20261021T080115.json SHA44e2c27ecdaee405de0e24a8cae66896cf7527dd8999ee607a2b3e224bc3616e.
- eod-raw-a2-progress-20261021T080810.json SHA84bd55364ff93a76722049470f1d36df5085ed5e24a4d94b6d1f443cf0e71dcd.
- eod-source-check-a3-result.json SHA2283623a2ccf7d07fc94881d22b7c836aacfd326b3a1fba556a32293ab67f46c.
- coverage-after-raw-a2.json SHA446bf67a09b8a053d2171fcc6c5258e1dc14750172f6785ea8a79257ece357c3.
Provider writes are paused until an actual cause/change is established. No token
refresh, credential read, inferred success, or same-attempt automatic retry.

New official --daily-rebind-first-session source handles the expired preactivation
binding through two verified immutable configurations. Only revision and future
first-session may differ. Exact original cohort must be stopped, control OFF,
all daily DB history empty, shared queues empty and original checkpoints pristine.
Both poll locks span the atomic SQL transaction. Direct migration_owner, current
Owner/entitlement, fixed policy and original capacity/lease/backoff are checked.
The existing trigger advances epoch0 to1 once; exact repetition is idempotent.
It never resets history/epochs, edits installed files or starts/enables workers.
A future date still requires actual deployment/calendar evidence; no date has
been selected and the production DB remains untouched. Prepare semantics and
schema62 are unchanged; no structural diagram change.

Local validation uses private full-schema PostgreSQL and root filesystem fixtures.
Initial a1 exited1: accidental unittest discovery also ran13 unrelated isolated
backup fixture cases; two new assertions omitted public qualification after the
actual SQL changed search_path. Preserve that original failure. Fixing fixture
discovery/assertions required no product-SQL change. Six new cases passed in a1;
the two corrected plus two newly added ordering cases passed in focused a2,
10distinct new cases total. Existing operation34 and CI contract21 passed.
Exact source pins and original receipts are in wave143/local-validation.json.
Hosted acceptance, immutable images/install and actual rebind remain pending.

R3's separate bounded observation ended08:13:13UTC (report commit566e77d): exact
installed866 tool/data blobs6/6, reader-check exit0/DAILY_READER_REQUIRED, and
08:12:36 public snapshot OwnerV2 healthy. Parent reviewed the recorded evidence;
this is not a fresh parent host observation. Actual grant/state/READY/generation,
current-day operational proof and compatible recovery remain UNKNOWN. No DB,
provider, network, lease, CI, build or deployment occurred in that R3 window.

Next accept the exact new source in hosted CI, diagnose Raw authentication without
replay, and close measured inputs before immutable installation/first-session
binding and official daily activation DBLAST. DailyOFF/epoch0, firstSessionnull,
actual days0, Owner use and C_R2 remain OPEN. Earlier failures, G4 acceptance,
protected lock/volumes/PG/key,32unrelated ROADMAP additions and .archify remain.


### Wave144 — first-session CLI contract and isolated legacy fixture (2026-10-21)

R2-04/09 remain DOING. Candidate0c709749e139801a7a8d481ab5e412ce8347d1bc
was published and CI39569367975 attempt1 dispatched once08:27:11UTC. Frozen
inputs include89Rust/94Python definitions,66run steps and7checkouts. The original
policy job failed: production-ops-static-check.sh still required the exact old
three-option shell case. The new first-session hosted suite was skipped; local
10case success is not hosted success. At08:37UTC workspace/PG remained running;
retain and recover the original terminal/logs without rerun or cancellation.

The static contract now includes the fourth, explicit rebind operation. Its local
correction first encountered pre-existing0775 checkout modes on two scripts whose
static contract requires0755; source bytes and production paths were not changed.
A clean tracked archive extracted with umask022 passed full OPS_STATIC. The first
local legacy fixture then discovered the real host reader-floor under default
/opt/lagrange and failed stale-config before backup execution. The fixture now
sets LAGRANGE_RELEASE_ROOT to its own empty temporary root for legacy backup
calls; it cannot consult the production installation. Its fake-Docker full test
passed. Both initial local failures are retained. No product rebind SQL/controller
change, real backup, provider, installed release or production DB mutation.

Exact originals, corrected source pins and results:
wave143/original-ci-policy-failure.json and
wave143/static-fixture-correction-validation.json. Root/nonroot tests used nice10;
the corrected archive's commands were sequential and both original children reaped.
The two source corrections require a new exact candidate CI, not a retry of0c70974.

Actual08:29:41UTC installed866 backfill validator passed current config names and
protected file metadata. Earlier wrong range-raw scope rejection is preserved.
This does not validate token contents or establish the08:08 authentication cause.
The deployed error mapping discards subordinate coordination/issuer reasons;
source evidence is wave143/raw-auth-source-diagnosis.json. Cause remainsUNKNOWN,
no provider/token request or consumed Raw replay. Serving866, DailyOFF/epoch0,
firstSessionnull and actual days0 remain. R3 read-only window evidence was verified;
no further R3 runtime operation is started. Preserve32unrelated ROADMAP additions,
.archify, all partials/old failures and protected production resources.


#### Wave144 exact corrected candidate dispatched — 2026-10-21 08:39 UTC

Corrected source83d1de3fba0870c6d50acfe7e405edd9bf907212 is nonforce-published
at r2-ci-83d1de3-20261021. CI39570874665 attempt1 was dispatched exactly once
08:39:02UTC and observed in progress08:39:33UTC. Frozen wave144/ci-inputs.json
contains unchanged89Rust/94Python definitions,66run commands and7checkouts;
local source pins bind both the first-session tests and corrected static/legacy
fixture results. Exact publish/dispatch intent/results and sole-run observation
are retained under wave144. No acceptance is claimed before terminal evidence.
Clean source /data/worktrees/3puw275b/r2-release-83d1de3 was created with umask022.
No new images, install, root staging, daily control change or provider call.
Original0c70974 policy failure remains; recover its still-pending workspace/PG
terminals and all logs without rerun/cancellation. This documentation commit is
not the build candidate. Registry points to wave144/corrected-first-session-ci-running-handoff.json;
last host observation remains only08:29:41 metadata validation, not this CI time.


#### Wave145 finite collection diagnostics and original hosted closure — 2026-10-21 09:05 UTC

R2-04/R2-09 remain DOING. Shared KIS read coordination produced finite causes,
but EOD provider and worker diagnostics reduced them to BROKER_AUTH_FAILED.
The new diagnostic_code returns only an exact static allowlist; unknown reasons
remain broad and no broker prose or secret is exposed. Existing audit code, retry
classification, quotas, token flow and network surface are unchanged. The actual
EOD provider boundary, helper decode and serialized worker event are covered.

Five distinct new tests and strict Clippy on the three affected packages PASS.
Original two test enum-name compile failures and the first Clippy invocation
missing the existing test-support feature are preserved with corrected results.
Evidence: target/r2-daily-beta/wp-8/wave145/diagnostic-local-validation.json,
8963 bytes, SHA256 a9fadbc807aa4d2b2fa87a28ca827e3fd6f8dc6c145dcbd642d7621741c1f31d.
The old Raw authentication cause is still UNKNOWN; this source change does not
prove historical recovery or authorize replay of the consumed original request.
No provider/token request, secret read or production change was performed.

Original0c70974 CI39569367975 attempt1 is terminal FAILURE: all8 logs and7
checkouts recovered; policy/required failed, new first-session hosted checks
were skipped. Rejection SHA25666aaa220654df3efaf9f1b87eb929097298ffd67750c878890d54514b7e3661f.
Corrected83d1de3 CI39570874665 attempt1 is fully accepted:8success/8logs/7
checkouts,89 frozen Rust and94 Python definitions,19 real PG exit0/no-skip,
page-seam14. Closure target/r2-daily-beta/ci/83d1de3/hosted-source-closure.json
is114332 bytes/SHA256 d554a0eb7439b2bf3f98c3fa35ef410d639f072bd18189bce26e9f4b45cdbcb4.
This closure covers83 only, not the later diagnostic or R3 source.

R3 edbe228 adds official pinned read-only grant inspection with21 fake/static
tests; actual isolated PostgreSQL validation is separately assigned to the
existing R3 workstream. Production operations remain serialized by the parent.
Serving866/DailyOFF/epoch0/firstSession null/actual trading days0 remain the last
accepted state. Raw29/34 and actions30/53 remain incomplete. Next is exact combined
source acceptance, official images/deployment, actual source/input admission and
DB-last daily activation. Preserve32 unrelated ROADMAP additions, .archify and
all historical/protected resources. No structural diagram change.


#### Wave145 R3 official inspect integration — 2026-10-21 09:16 UTC

Parent reviewed edbe2284f9bdb5820a98d4d691eb64bbfd207273 and
cdb0d62051147bc8ca13d900f4b3db9e8672b1d3. Five exact source/test files
were integrated; fixed SQL uses REPEATABLE READ READ ONLY with operator-role,
immutable grant payload, Owner role and ACTIVE entitlement comparison. Existing
approval evidence is reused. No grant write, new role/schema or provider surface.
The helper reports only finite states and allowlisted metadata. Existing installed
revision and protected original approval file/hash remain mandatory.

Twenty-one fake/static tests passed in the R3 workstream. Its actual network-none
PostgreSQL fixture applied all62 migrations and passed11 distinct tests, including
wrong roles, exact fields/types, eligibility, date boundaries, unchanged rows,
SQLSTATE25006 write rejection and Git archive/trusted-file checks. Exact owned
container/anonymous volumes/temp directory cleanup passed. Initial readiness and
FK reset failures are retained. Parent verified every receipt hash and exact
integrated source. Validation4695bytes/SHA256
99c0768b9f4030215e962b4d8d33e607e0c6bf87a94430c3b5164bec00a09100.

Parent preserved the existing first-session CI step, added separate required
fake21 and actualPG11 steps, and passed the one changed CI contract. Integration
evidence is target/r2-daily-beta/wp-8/wave145/r3-grant-integration-validation.json.
At09:13:46UTC, a bounded filesystem metadata-only observation found the existing
root:root0600 ws-approval JSON under release-manifests; no contents/credentials
were read and no grant/state verdict follows. Raw source-gap derivation from
recorded evidence confirms23missing civil dates across8intervals, recurring
frontier gap53days>14; separate bounded catchup is needed, no automatic chunking.
Next is the exact combined candidate hosted CI, fresh images/install and scoped
actual input/grant admission. DailyOFF/epoch0/firstSessionnull/actual3days0.


#### Wave146 original CI toolchain conflict — 2026-10-21 09:26 UTC

Combined3f2eb414c372caca4af8e93202205858a290e72b was nonforce-published and
CI39575826396 attempt1 dispatched once09:17:41UTC. Frozen Wave145 input
482637bytes/SHA25607ec17428f4cc64773eb6a20f5c2d821bb907a6011f978d004def347da6b3f1b
binds94Rust/127Python definitions and68commands. At09:21:53UTC workspace-tests
was failed before any Rust test: rustup could not install the Clippy component
because bin/cargo-clippy already conflicted. The pinned1.97.1 toolchain was
automatically being prepared. The full original log hash and focused lines are
retained in ci/3f2eb41/workspace-failure-focused.json. This is an installation
failure, not a product assertion failure. Other original jobs remain pending;
no cancellation, replay, or whole-run acceptance is claimed.

The workspace-tests job now has a run/attempt/job-specific RUSTUP_HOME under
runner.temp. Rustup documents that this variable selects the toolchain store;
the unchanged rust-toolchain.toml still selects1.97.1 with rustfmt/Clippy.
References: https://rust-lang.github.io/rustup/environment-variables.html and
https://rust-lang.github.io/rustup/overrides.html. No toolchain-version override,
new action, mirror, cache removal, retry, allowed-failure or weakened test gate.
The focused new workflow contract passes1test/exit0; exact receipt is
wave146/isolation-contract-result.json. Actual fresh-store hosted execution
remains required. Parent integration3f2 and R3 isolatedPG evidence are retained.
No new image/root staging/stop/deployment/provider/token request. Serving866,
DailyOFF/epoch0/firstSessionnull/actualdays0 and source gaps remain unchanged.


#### Wave146 CI context correction — 2026-10-21 09:27 UTC

First isolation source51b9bb216f2f2a7d5bd3e452cd770fc24c5c93a9 was published;
the single workflow_dispatch returned exit1 at09:24:55UTC (stderr295bytes,
hash retained; raw error text not retained). Read-only recovery found automatic
push validation39576766320 attempt1/FAILURE and no check-run annotations.
Do not claim a successful dispatch or executed tests. Both failed sources and
all receipts remain; no retry of either original dispatch.

GitHub's official context availability contract excludes runner from job.env:
https://docs.github.com/en/actions/reference/workflows-and-actions/contexts .
The invalid reference is removed. A first post-checkout shell step creates
a private0700 mktemp directory under RUNNER_TEMP and writes RUSTUP_HOME to
GITHUB_ENV for subsequent steps. The pinned compiler/components and existing
test commands are unchanged. Two focused tests passed: workflow ordering and
no toolchain override, plus actual shell execution with a spaced temporary path
and fail-closed missing RUNNER_TEMP without changing the environment file.
Receipt: wave146/isolation-contract-corrected-result.json. The earlier weak
static-only PASS is preserved and is not evidence of hosted syntax validity.
No production change, new images, provider/token request or old Raw replay.
Next is a distinct fixed corrected source with one original CI invocation.


#### Wave146 exact combined CI accepted; original a6 running — 2026-10-21 09:50 UTC

Candidate5f2d4ddb422628973a20dba9d6c7d80e5bba00ff was dispatched once at
09:27:56UTC as CI39577159289 attempt1. At09:48:34UTC all8 required jobs
succeeded. Full8 logs,7 exact checkouts, frozen94Rust/129Python definitions,
19 actual PostgreSQL processes exit0/no-skip and hosted page-seam14 passed.
Closure175581bytes/SHA25666722ddfe12ad11a0ddfe87ed4190cc75b6907cf3fd5c783284409f924e8de8a.
The original closure extractor failed because a next-target stderr header
appeared before the last3 research-worker stdout rows. Preserve that failure
and every original observation. A separate source-qualified recovery matched
all10 unique frozen definitions, the complete10-PASS block and the following
1-test block; no test/CI rerun. Correction7746bytes/SHA256
ff8cd376ea789e5b139464a19c6f9f007cb80e4751bf1fbb6cbf1a51a4aa8ac4.
Original3f2 CI39575826396 is terminal FAILURE with all8logs/7checkouts;
rejection25803bytes/SHA25638f6d1a1976f0cd5a5b35c2b293d508dbb27d843164754ce4bf006ab034e1a6d.
Invalid51b9 dispatch/push failure and original frozen inputs remain unchanged.

The distinct sealed a6 was submitted once09:50:14UTC after actual independent
root recovery READY. The original bootstrap exited0/reaped. At09:50:28UTC the
original Owner stop client was normal0/drained and the first image was building;
1 gate passed, MemAvailable8787348KiB and SwapFree2140740KiB. Restoration is
still owed; no new image-set or serving acceptance. The fixed same-boot12h
window expires21:50:13.954231UTC and cannot be renewed. Four batches of3 remain
strictly one Compose service at a time, parallel limit1/build jobs2 and lowered
CPU/I/O. Exact original unit PIDs/invocations are bound in the read-only terminal
observer. No competing stop/start/replay or forced kill. Handoff:
wave146/a6/running-handoff-20261021T0950.json,136905bytes/SHA256
9c055125522b6a0b2a512d3665d7cd0b46faf61e87c07c7e50221f18b2386201.

Previously accepted serving866 and independently reconstructed real calendar
identify Oct22 as a conditional future configuration candidate. This is local
planning only; actual firstSession remains null, DailyOFF/epoch0 and actual
consecutive days0. New target deployment and actual input admission must finish
before candidate open; official first-session rebind remains OFF and uses the
actual historical e639 daily cohort. A draft new protected-input publisher and
serving driver exist but have not executed. Raw29/34 and actions30/53 remain
partial, prior authentication cause UNKNOWN. R3 inspect code/isolatedPG tests
are included in this candidate; actual grant/pilot remain unverified. Preserve
all original failures,32 unrelated ROADMAP additions and .archify. No structural
diagram change. Next is original a6 terminal/image/restoration acceptance, then
immutable install/serving validation and independently admitted daily/R3 inputs.

## Wave147 — actual 5f2 image/restoration and serving acceptance; input gaps (2026-10-21)

Candidate `5f2d4ddb422628973a20dba9d6c7d80e5bba00ff` retains its accepted hosted closure. The original a6 completed all12images/49gates and its independent supervisor restored the original Owner normally. Independent observation10:45:15UTC: `wave146/a6/image-restoration-observation-20261021T104515.json`,857640bytes/SHA256522f71b555f7ae8c0af61d72a7e278c6633b7ab4650ad1c20266da3fe066d176. No replay or outstanding restoration debt.

The distinct serving a1 ran once10:45:56UTC. Official install/check, KIS install-disabled/check, status policy/timer restoration, sequential3-serving refresh and HTTP checks all exited0. Original PID2029412/invocationaee2cd7df2c44a81b521cbe5e2a7f27d was terminal/drained. Independent10:46:50.752685UTC closure `wave146/serving-deployment-closure-20261021T104650.json`,61781bytes/SHA2568b697bf8cedb7fb95cfb87e4bfd919c10a4c04e3b665190dde175c944ce04522 proves exact3 healthy5f2 images, other8 unchanged and original Owner normal termination. Schema62 unchanged; this is serving acceptance, not authenticated Owner or daily-input acceptance. Original tracking-field KeyError was separately recorded and corrected locally without rollout replay.

Raw5f2a1 mixed historical/current-day plan was rejected before execute; retained normalexit2 and no execute intent/state. Distinct current-day a2 completedOct21 at10:51:27UTC; independent network-none original-source verification now30/34 (`wave146/eod-source-check-a2-result.json`,SHA256ff5b8f14532488861a1abffcfec902c5f13d3405c4f378e4a551561d4505c1f1). RemainingOct15,16,19,20 were submitted in distincta3. Original execute2048658 exited1/reaped10:52:42UTC; unit2048453/invocationa252e036ca72401ebd9f2f4cabc62c67 terminal/PID0/emptycgroup at10:53:15UTC. Oct15 failed with finite `KIS_READ_CALLBACK_START_MISSED`; no committed dates. Later plan RUNNING rows do not prove calls. Preserve `wave146/eod-raw-5f2-a3-progress-20261021T105315.json`,SHA2565bbc2481f01c28852ee05808c94812293d0d5bff0ca90d2fa4757f43700311ed; no replay. The underlying delay and original old866 BROKER_AUTH_FAILED cause remain UNKNOWN.

R3 official READ ONLY inspect at10:47UTC returned DB_OUTCOME_UNKNOWN, not a grant verdict. A distinct bounded diagnostic of unchanged installed functions at10:56:41UTC returned SQL INSPECT_VALID/exit0, but stderr272bytes made the official outcome remain UNKNOWN. Two stderr lines classified as owned Compose lifecycle and one remains redacted/unclassified. Cleanup reported no owned container; no provider, grant write or lease. Original inspection remains preserved; this new SQL result does not establish READY/dayproof/pilot/Owner acceptance. Evidence `wave146/r3-grant-diagnostic-result.json`; exact candidate/SQL/approval pins checked.

Daily remainsOFF/epoch0 with historicale639cohort, firstSessionnull/actualdays0. ConditionalOct22 config is installed but is not an admitted first session; actual inputs and validation must complete before2026-10-22T00:00Z. KIS remainsdisabled. Actions need an independent refreshed inventory; older30/53 is historical. Next: preserve both failures, correct verified local timing/transport defects, finish measured source gaps and independently admit inputs, then official rebind/activation DB LAST. All original failures/unknowns,32 unrelated ROADMAP additions and .archify remain. No structural diagram changes.
