# R5 / PR12 작업 중지 기록 — 2026-11-13 KST

> **재개 2026-11-13 KST:** 같은 날 Owner가 방안 C(지금 병합)를 승인해 재개했다. 현재 상태는 이 문서 끝의 [재개 결정](#재개-결정-2026-11-13) 절이 우선한다.

Owner가 “스톱 중지하고 지금 너무 오래걸려 전부 정지하고 여태까지 내용 기록해”라고 지시했다.
**작업은 PAUSED다. 자동 재개·CI 재실행·push·merge·운영 전환을 하지 않는다.**
이 문서는 재개 명령이 아니라 현재 상태의 보존 기록이다.

## Git 및 목표 상태

- 저장소: `sihioov/Lagrange`, [PR12](https://github.com/sihioov/Lagrange/pull/12).
- 실제 작업 경로: `/data/worktrees/3puw275b/r5-checkpoint-integration`.
- 브랜치: `integrate/r5-checkpoint-20261110`.
- 마지막 push된 구현/테스트 커밋: **`60e158d42a2341d29a5391f447bde8473e4e2666`**.
- 중지 시 PR은 **OPEN / UNMERGED**, 원격 main은 `92e47a05ad50fe13c576cedb23a91d08b154a93c`다.
- 전체 R5 목표의 도구 상태도 `paused`로 전환했다. 완료로 처리하지 않았다.
- 이 중지 문서와 로드맵 변경은 **로컬 문서 커밋**으로 보존하며 push하지 않는다. 새 CI를 유발하지 않는다.
- Production CLOSED / ENABLE readiness=false를 유지한다. 실제 운영 상태를 변경하거나 조회하지 않았다.

## 전달된 구현과 남은 범위

브랜치에는 P4 lifecycle/CLI/Source barrier, Root 역사 view staging, P5 보유 원본 수집·closure와
실제 material borrow/recompute, journal/release/R4/추천 연결, lifecycle 원본의 archive 투영,
격리 destination restore·readback·정리 후 publication, migration0069, 저장 용량 및 이전 release 호환,
owned RESTORE BEGIN의 제한 DAC_READ_SEARCH와 early additive seccomp 경계가 포함돼 있다.
이는 guarded 구현 checkpoint이며 **R5 전체 인수 완료가 아니다.**

private native4/Source4, 실제 승인 V3와 linked Raw의 해시 검증, genuine nonempty archive/C,
실제 복원과 운영 활성화, 실거래일 pre3/post3, G4 및 Owner 화면 인수는 OPEN/NOT_RUN이다.
기존 false ENABLE 게이트, native15초, Cargo2/Compose1, 모든 필수 CI를 보존한다.
공급자 계좌·주문 API, 실제 운영 서비스·DB·Docker, 원본 권한을 변경하지 않았다.

## 장시간 반복의 경위와 보강 위치

현재 계획의 기준은 [ROADMAP](../ROADMAP.md), 단계별 근거는
[기존 R5 재개 기록](2026-11-11-r5-resume.md)이다. 개발 효율 보강은
[WP1~4 계획](../superpowers/plans/2026-10-13-development-efficiency.md)과
[bounded-work 절차](../runbooks/development-verification.md#bounded-work-and-recovery)에 있다.
자료가 없었던 것이 아니라, 실제 통합검사가 뒤늦게 드러낸 결함과 긴 빌드 반복을 제때 줄이지 못했다.

- 캐시/격리 실행/receipt 및 pending producer marker는 구현됐지만, 이번13이미지 통합의 속도 개선을
  입증한 것은 아니다. 매 새 후보의 공식 바이너리 출처·전체 검증 비용은 계속 발생했다.
- 원래 PG90분 안에 일반DB/SQL52와 G1/G2를 모두 넣어 cutoff에 도달했다. `2252fde`에서 기존
  공식13이미지 G1과 G2를 함께 별도 **필수** job으로 분리했다. 검사 제거는 없다.
- 이미지 job은 실제 콜드 비용을 반영해 최초120분, 이후 최초180분으로 배정했다. 실행 도중 기한 갱신은
  하지 않았다. 일반PG90분/native15초 및 동시성 제한은 유지했다. 이것은 용량 배정이지 성능 개선 증거가 아니다.
- G1이 통과한 뒤 처음 실행된 G2 STOP의 준비·전송 경계에서 아래 결함들이 순차적으로 드러났다.
  초기 사전검사가 전체 실제 생성 정책을 검증하지 않아 매번 긴 빌드 뒤에서야 실패를 회수한 비용도 있었다.

| 후보 | 실제 확인 및 조치 |
| --- | --- |
| `7598b93` | G1 공식13이미지/출처/정리 첫 PASS. G2 진입 UNKNOWN; 독립 고정PG 이미지 preload 전제조건 보강. |
| `753d624` | 초기 Root probe PASS, D1/D6/D2 Cargo만 약82분; D8 중120분 reserve 도달. 다음 실행부터 이미지180분. |
| `8573859` | G1 PASS, STOP의 PG_START_TIMEOUT. 고정PostgreSQL18과 맞지 않던 legacy data mount 수정. |
| `81e32ad` | PG18 parent mount/readiness 및 실제 DB/schema probe 추가. 테스트의 cleanup_phase context-manager 오용을 early probe에서 검출. |
| `658ff47` | cleanup_phase API 호출 보정. 실제 early DB/schema/disposal PASS, G1 PASS. STOP context 오류가 cleanup에 가려 UNKNOWN. |
| `be6451e` | 닫힌 causal diagnostics 추가. 원래 PAPER_OPS_NETWORK(1637), 이후 created helper cleanup UNKNOWN(2007) 회수. |
| `ad7282a` | 생성 전 빈 NetworkID를 실제 고정 network 이름/단일 endpoint에만 허용. 실제 network probe/G1 PASS. STOP은 CONTAINER_ENV(485)에서 실패. |
| `76e1e78` | Docker CLI28의 Go map 순회 때문에 순서가 비결정적인 env를 정확한 multiset으로 검사.5040순열 회귀 PASS. 새 Root probe에서 tagged/untagged RepoDigests 테스트 비교 오류를25.208초에 검출; 긴 빌드는 미시작. |
| `60e158d` | 같은 Alpine SHA를 canonical untagged RepoDigests로 비교. **실제 mock 없는 create/환경/identity/host/network 사전검사 PASS**. 이후 전체 CI는 Owner 요청으로 중지. |

환경의 정확한 key/value/개수와 duplicate 거부는 유지한다. `PATH=`는 bare `PATH` 제거와 다르므로 계속
거부한다. Docker CLI28 [create.go93–102](https://github.com/docker/cli/blob/v28.0.4/cli/command/container/create.go#L93-L102)의
map 순회는 확인했지만 **ad728의 실제 실패 배열은 보존되지 않아 별도 extra 가능성을 배제하지 않는다.**
네트워크/환경 수정 후 full G2가 성공했다는 주장은 아직 하지 않는다.

## 검증 범위

- 마지막 로컬 focused: **272 PASS / 실제 Root1 SKIP**. 환경 순열5040 및 기존 변조 거부 포함.
- `ad7282a`의 CI43233409789: 일반7필수 job, SQL52, 실제 Root8/archive16 및 각각의 정리,
  도표 source2/PNG2, 공식13이미지 G1/출처/정리와 같은 후보 smoke43233468863 PASS.
  G2 첫 STOP 실패이므로 required는 FAILURE다. 이를 새 후보의 최종 통과로 재사용하지 않는다.
- 마지막 `60e158d`의 [CI43252110315](https://github.com/sihioov/Lagrange/actions/runs/43252110315):
  실제 unmocked Docker 사전검사·Format·Clippy·Python은 PASS를 관측했다. 나머지는 중지 요청으로 취소됐으므로
  최종 required/G1/G2/Workspace/PG/Web 통과로 인정하지 않는다.
- 같은 `60e158d`의 [smoke43252806343](https://github.com/sihioov/Lagrange/actions/runs/43252806343)는
  중지 직전 이미 terminal SUCCESS였다. 이 기록에서는 원본 로그/개별 정리 receipt를 추가 회수하지 않았다.
- CI 취소 후 hosted builder/helper/DB 자원의 독립적인 부재 확인은 **UNKNOWN**이다. 플랫폼의 취소 결과를
  실제 cleanup PASS로 바꿔 부르지 않는다. 로컬에서는 무거운 Cargo/DB/Docker/provider 검사를 실행하지 않았다.

## 중지 및 증거 보존

- 현재 CI43252110315는 **terminal completed/cancelled**다. 실행 source job들의 취소와 모든 job의 terminal 상태를 확인했다.
  required는 취소된 의존 job 때문에 FAILURE로 종료됐으며, 새 source 결함의 증거로 분류하지 않는다.
- 직접 실행한 CI/smoke watch 두 프로세스를 정확한 PID/argv로 식별해 종료했다. JS observer도 종료했다.
- 서브에이전트들은 이미 완료 상태였으며 새 작업을 위임하지 않는다.
- 기존 원본은 `target/r5-resume-20261112/merge-first/`(0700/0600)에 보존한다.
  `work.json`이 현재 작업 상태, `stop-ci-run.json`, `stop-ci-jobs.json`, `stop-smoke-run.json`,
  `stop-pr.json`, `stop-main.json`과 `stop-final.json`이 중지 snapshot이다.
- `images-ad728.log`, `env-ad728-cause.json`, `env-order-red.log`, `env-order-green.log`,
  `startup-76e1.log`, `env-canonical-fixture-tests.log`에서 이번 오류·수리·한계를 구분할 수 있다.
- 이전 상세 인계/원본은 `target/r5-resume-20261111/` 및 기존 review 링크에 그대로 둔다.
- 승인된300초 읽기 전용 private material preflight는 실제 sudo 인증이 안 돼 미실행이었다.
  Root/호스트 터미널 우회나 원본 권한 변경을 하지 않았고, 중지 후 실행하지 않는다.

재개하려면 Owner의 명시적 지시가 필요하다. 그때 원격 head/main과 새 변경을 먼저 대조하고,
취소된 실행을 성공으로 취급하지 않으며 기존 같은 후보 필수 CI/G2와 정상 merge 조건을 따른다.
**현재는 자동 재실행·merge를 예약하지 않았다.**

## 재개 결정 (2026-11-13)

중지 뒤 같은 날 Owner가 지연 원인 분석을 요청했고, 결과를 보고받은 뒤 ‘지금 병합’(방안 C)을 승인했다.
이 절이 위의 PAUSED·자동 재개 없음 상태를 대체한다.

- **확인한 사실.** 11/06 이후 R5 브랜치의 CI는 71회 실행 중 통과가 없다(PR12 브랜치 27회 포함).
  11/12의 긴 실행 7회는 매번 공식 13이미지를 처음부터 빌드했다. 5회는 76~95분에 빌드를 마쳤고 2회는
  빌드 시간 한도에 걸렸다. 수명주기 검사는 그 5회를 합쳐 약 3분만 실행됐다. 검사 10단계(STOP, BACKUP,
  복원 거부, 장애 7종) 중 첫 단계인 STOP의 준비 과정에서 멈췄고(1회는 단계 식별 전), 원인은 매번 달랐다.
  뒤 9단계는 실행된 적이 없다. 같은 7회에서 기존 필수 검사 7개는 모두 통과했다.
- **결정.** `paper-image-integration`은 PR12가 새로 추가한 검사이고 main에는 없다. `required`에서
  제외하고 계속 실행한다. 병합 조건은 기존 필수 검사 7개와 같은 후보 smoke의 통과다.
- **보완 조건.** 이 검사가 통과하기 전에는 main에서 운영 릴리스를 만들거나 배포하지 않는다.
  migration 0068/0069는 ENABLE 게이트로 막히지 않으므로 릴리스에 포함되면 운영 DB에 적용될 수 있다.
  통과 뒤 `required`로 되돌린다.
- **이후 작업.** 병합 뒤 작은 후속 PR로 수명주기 검사의 빠른 반복 경로를 만들고 10단계를 통과시킨다.
  같은 검사가 3회 연속 실패하면 원인이 달라도 수리를 멈추고 Owner에게 보고한다.
- **변하지 않는 것.** Production CLOSED/ENABLE=false. 비공개 원본·실제 복원·운영 활성화·실거래일 관측·
  Owner 인수는 OPEN이다. push·CI 실행·merge는 각각 Owner 승인 뒤 수행한다. 이 병합은 R5-04/05 완료가 아니다.
- **검토하지 않은 것.** PR12 소스 전체의 품질은 이 결정에서 따로 검토하지 않았다. 병합 근거는 기존 필수
  검사와 smoke의 통과다.
