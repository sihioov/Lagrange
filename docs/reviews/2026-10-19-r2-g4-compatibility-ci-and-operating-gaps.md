# R2 G4 compatibility CI acceptance and operating gaps

Status: **790f1ea HOSTED_ACCEPTED_TOOL_ONLY; G4 operating packet incomplete, not authorized or executed.**
Roadmap: R2-04/R2-09. Application e639169 remains separately accepted through its images and actual isolated wholeflow, and is not deployed.

Wave114 복구 도구790f1ea의 CI39204199932 attempt1 8작업이05:45:47UTC 성공했고05:46:39UTC 전체8로그/7실제checkout·1464pin/548blob·201필수 결과를 회수했다. 기존41개+신규14개 maintenance55개와 실제 Docker cold restore4개가 통과했다. 승인된 Wave113 조회는12단계까지 회수했으며 유일한 gap인 release lock 부재를 그대로 보존한다. 운영 중단·백업·배포는 실행하지 않았다. 키의 외부 보관 식별명과 외부/수동 writer 유무는 이미 한 번 질문했고 답변 대기다. 정확한 G4 admission/main/recovery 제어와 실행안을 완성한 뒤 필요한 운영 결정을 받는다. 앱e639169/12이미지/실제 통합 인수는 보존하며 G4·백업 전환·배포·Owner·3연속 실제거래일·C_R2는 미완료다.

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

The existing asynchronous information request remains pending for an off-server encryption-key custody identifier, without key values, and external/manual writers beyond known containers and schedules. The observed439 processes/4336 descriptors do not prove a global future writer fence. No answer is assumed and the question is not repeated.

The exact admission/main/recovery programs and readiness handshake still need completion against those operating facts before a final concrete packet and one operating decision. No new host approval question is pending. The consumed survey approval is not transferred to an outage. Existing read-only KIS and Owner V2 permissions remain preserved; they are not asked again merely because the roadmap stage changes.

G4 consistent cold five-class capture/private-clone proof/same-ID restoration, backup transition, application deployment, actual Owner use and three consecutive actual trading days remain open. Synthetic fixture dates do not satisfy those days. The application installer must use e639169, not the maintenance-tool commit. The prior anonymous volume remains retained with ownership unproven and0120 cleanup INCOMPLETE; no mount/content/delete/prune is permitted by this work.

후속 추적e3e092a9를05:48:08UTC 별도 active ID로 확인하고 인계0613d80d/registry 저장 후4786839b 삭제를 확인했다.17:00UTC 전/47번째 tick 전 갱신,17:48:08UTC 만료. Native Ralph blocked/no resume API; 동일 에이전트의 제한된 후속 추적이며 worker나 장애 보호가 아니다.
