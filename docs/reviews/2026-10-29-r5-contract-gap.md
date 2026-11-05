# R5 WP-1 — Paper 계약 차이와 G1 결정 보고

작성: 2026-10-29 (Asia/Seoul). 조사 기준 HEAD: `a30edacff987d55b5be7cca25bed7faeec589629`.
cwd: `/data/worktrees/3puw275b/modern-anaconda`, branch `review-recent-work-history`.
산출물: [계약 초안](../contracts/owner-beta-paper-v1.md), 이 보고서.
상태: **WP-1 문서 산출 / G1 OPEN / 제품 구현·운영 검증 NOT RUN**.

## 1. 판단

R5는 기존 Paper 원장을 재사용할 수 있지만 기존 generic dataset 경로에 Daily pins만 넣어서는 실행할 수 없다.
특히 다음 네 차이는 후속 구현의 필수 전제다.

1. 개인 Owner 권리는 이미 채택됐다. `paper_view`도 체크인 구성에 있다. 재승인 문제가 아니라 정확한
   runtime entitlement와 beta 입력·run·target 계보를 검증하는 연결 작업이 남았다.
2. T 신호용 Daily 입력에는 E 가격이 없다. EOD에서 받은 **E 입력을 별도로 검증**해 T prefix와 연결하고,
   E raw open으로 지연 정산해야 한다. 장 시작 처리 완료를 약속할 수 없다.
3. 기존 poll/checkpoint는 무인 수집의 실행 기원을 증명하지 않는다. 신뢰 가능한 origin producer와 DB reader가
   먼저 필요하다. 현재 WP-2의 세 Daily 파일 후보만으로는 end-to-end 증거를 완성할 수 없다.
4. generic `SKIPPED`, 합성 원장 시각, generic 추천 FK를 beta의 정상 무거래·실제 처리시각·추천 identity로
   간주할 수 없다. 새 schema와 좁은 authority 함수가 필요하다.

R4 완료는 유지한다. [최종 인수:370](2026-10-28-r4-g3-readonly-preflight.md#L370)의
application312/maintenance439·7개 실제 결과 쌍·복원2회·Owner 화면 확인은 R4의 증거다.
Daily 활성화·R5 전후 soak·generic 데이터셋 등록의 증거로 바꾸지 않는다.
R2 입력/실제3일, R3 FINAL, D-06/D-07/005930 유예는 별도다.

## 2. 직접 확인한 근거와 gap

모든 `file:line`은 위 기준 SHA의 소스/체크인 문서다. 운영 파일·DB·provider는 열지 않았다.
기존 문서의 과거 운영 인수는 **기록 회수**이며 이번 WP의 직접 운영 관측이 아니다.

| ID | 근거 | 실제 사실 | gap/결론 | 증거 수준 |
|---|---|---|---|---|
| E01 | [ADR-0005:13](../decisions/0005-kis-personal-use-entitlement.md#L13), [KIS 권리 구성:23](../../configs/data-rights/kis.entitlement.json#L23) | 개인 Owner read-only 수집/보존/사용 승인, 권리 재확인 금지. `recommendation/backtest/benchmark/paper_view` 포함 | 새 권리 질문 불필요. 현재 DB/보호 config의 동일 ref/hash·ACTIVE·기간·철회는 후속 readback 필요 | 채택 문서 + SOURCE |
| E02 | [Owner launch:20](../superpowers/plans/2026-08-23-owner-readonly-beta-launch.md#L20), [soak:334](../superpowers/plans/2026-08-23-owner-readonly-beta-launch.md#L334) | 2차 Paper, Owner1·ETF11·price-only·3거래일 무인. 실패는 reset; 수동 제외 | 기간은 후속 v3/Daily 계약을 따름. 당시 계획의 2020 시작일을 최신 base로 복사하지 않음 | 채택 문서 |
| E03 | [historical pins:69](../../crates/market-data/src/historical_price_only_approval.rs#L69), [Daily pins/trigger:59](../../crates/job-queue/src/owner_beta/daily_input.rs#L59), [DatasetPin:22](../../crates/job-queue/src/recommendation/input.rs#L22) | 세 별도 identity. Daily에는 kind 및 네 해시와 별도 trigger epoch | 명시 beta adapter 필요. fake dataset UUID/manifest·READY 등록 금지 | SOURCE |
| E04 | [Daily input capability:1](../../crates/market-data/src/daily_price_only/input.rs#L1), [Daily authority:406](../../migrations/0060_owner_beta_daily_inputs.up.sql#L406) | sealed reader와 DB 현재 권한은 별도. Daily rights는 exact reference/hash+recommendation | Paper purpose와 현재 권리/role/epoch를 worker commit에서도 확인. parser PASS는 권한 없음 | SOURCE |
| E05 | [pending FK:96](../../migrations/0038_paper_rebalance_previews.up.sql#L96), [preview FK:165](../../migrations/0038_paper_rebalance_previews.up.sql#L165), [beta target:20](../../crates/job-queue/src/owner_beta/daily_target.rs#L20) | generic run/target FK와 beta factor/target hash 표현이 다름 | beta origin·FK·lineage schema 필요. `LEGACY`/NULL pin fallback 금지 | SOURCE |
| E06 | [Paper daemon:443](../../crates/api-server/src/bin/paper-runner.rs#L443), [daemon config:464](../../crates/api-server/src/bin/paper-runner.rs#L464), [middleware:100](../../crates/api-server/src/http/middleware.rs#L100) | generic dataset root, placeholder code_commit, non-beta 모드. HTTP는 Owner+Paper enum 판정 | 실제 release/config/Owner를 daemon으로 전달, beta scan 제한과 DB admission 필요. enum 변경만으로 충분하지 않음 | SOURCE |
| E07 | [validator:353](../../scripts/ops/validate-production-config.sh#L353), [compose gate:546](../../scripts/ops/compose-release.sh#L546) | trusted checker 부재로 enabled 거절; 역사 artifact gate는 UNREGISTERED/NOT_PUBLISHED | 두 gate를 동일 검증 결과로 연결. shell 독자 판정/수동 boolean 금지 | SOURCE |
| E08 | [Paper 승인 설계:101](../superpowers/specs/2026-08-13-recommendation-paper-rebalance-preview-design.md#L101), [apply:306](../superpowers/specs/2026-08-13-recommendation-paper-rebalance-preview-design.md#L306) | 추천일 raw close preview, target-only apply, next open 재산정, stale 거절 | R5의 latest T→E와 EOD 지연 처리·두 pin 연결 필요 | 설계 승인 + SOURCE 접점 |
| E09 | [EOD cutoff:29](../runbooks/owner-beta-daily-input.md#L29), [wrapper:512](../../scripts/ops/kis-daily-production.sh#L512), [open read:720](../../crates/job-queue/src/paper_execution.rs#L720) | EOD는 16:30 이후; generic open reader는 날짜로 bar 선택 | E 장 시작 available-at 근거 없음. 단순 날짜/market_close_ts 검사를 원본 수집·admission 시각 대신 사용 불가 | SOURCE |
| E10 | [stamp:107](../../crates/job-queue/src/paper_execution.rs#L107), [session idempotency:412](../../crates/job-queue/src/paper_execution.rs#L412), [valuation:398](../../crates/job-queue/src/paper_valuation.rs#L398) | 세션 행을 00:30 UTC로 stamp, created_at::date로 실행 감지; valuation은 미래 close 거절 | beta 경제적 세션키와 실제 처리시각 추가. 과거 stamp 재작성 금지 | SOURCE |
| E11 | [SessionOutcome:45](../../crates/api-server/src/paper_session.rs#L45), [NoTrade:133](../../crates/api-server/src/paper_session.rs#L133), [fallback:158](../../crates/api-server/src/paper_session.rs#L158) | 정상 무거래·오류/차단은 SKIPPED. pins 없으면 generic executor 호출 | WAITING/NO_TRADE/FAILED/HELD 구분 및 beta typed dispatch. zero-trade에도 원자 session evidence 필요 | SOURCE |
| E12 | [비용:46](../../crates/portfolio-model/src/cost.rs#L46), [resolver:119](../../crates/portfolio-model/src/cost.rs#L119), [sizer:377](../../crates/portfolio-model/src/sizing.rs#L377) | KRX_ETF_DEFAULT@1은 확인 필요 placeholder; CUSTOM 불가; 마지막 종목 예산 잔여 | 운영 금융 채택 기록 미회수. R4 금융값 자동 복사 금지. 계약 O1 결정 필요 | SOURCE |
| E13 | [R4 v2 채택:3](../superpowers/plans/2026-10-26-r4-baseline-expansion-adoption.md#L3), [v1 비용:119](../superpowers/plans/2026-10-23-r4-price-only-simulation-contract-draft.md#L119), [v2 수량:111](../superpowers/plans/2026-10-26-r4-baseline-expansion-contract-draft.md#L111) | 두 R4 비용 프로필·정수주·기간·목표 수량/예산 규칙 채택 | R5 권장 Paper 모형과 비용·rounding·sizing 차이 표시. 기간 다르면 NOT_COMPARABLE | 채택 문서 |
| E14 | [poll receipt:160](../../migrations/0060_owner_beta_daily_inputs.up.sql#L160), [reservation:188](../../migrations/0060_owner_beta_daily_inputs.up.sql#L188), [claim:203](../../migrations/0060_owner_beta_daily_inputs.up.sql#L203) | Owner/policy/epoch·started/completed·result·admission, run/job/attempt/claim identity 존재 | timer origin·실제 service/image/config·end-to-end receipt 부재. CHECKED는 추천 SUCCEEDED 아님 | SOURCE |
| E15 | [checkpoint:19](../../crates/job-queue/src/owner_beta/runner/daily_local_state.rs#L19), [scheduler:85](../../crates/job-queue/src/owner_beta/runner/daily_schedule.rs#L85), [timer:442](../../scripts/ops/install-kis-daily.sh#L442) | checkpoint는 restart 힌트; scheduler는 latest-only 예약; systemd 직접 wrapper 실행·Persistent timer | timer active/INVOCATION_ID로 수동·catch-up 배제 못함. 새 trusted origin producer 필요 | SOURCE |
| E16 | [Daily expected/freshness:703](../../migrations/0060_owner_beta_daily_inputs.up.sql#L703), [reservation replay:1136](../../migrations/0060_owner_beta_daily_inputs.up.sql#L1136), [supersession:49](../../migrations/0061_owner_beta_daily_supersession.up.sql#L49) | 검증 달력+16:30 기준, 180초 admission poll, unique reservation replay, epoch binding | 최신3일을 현재 검증해야 함. 데이터 hash를 공통 cohort로 고정하면 정상 일일 증가를 잘못 거절함 | SOURCE |
| E17 | [권한 부여:1775](../../migrations/0060_owner_beta_daily_inputs.up.sql#L1775), [admitter readback](../../migrations/0062_owner_beta_daily_admitter_readback.up.sql) | Daily 테이블 권한을 제한하고 stage별 함수를 부여. reader를 임의 admin query로 대체할 수 없음 | R5 evidence reader용 narrowly scoped SQL 함수·role 검사·RLS 검증 필요 | SOURCE |
| E18 | [target+outbox:309](../../crates/api-server/src/repos/pending_targets.rs#L309), [archive recovery:215](../../crates/api-server/src/paper_session.rs#L215), [Daily rollback:167](../runbooks/owner-beta-daily-input.md#L167) | durable outbox와 archive, reader floor/history 보존 설계 존재 | beta control/drain/session evidence·복원 OFF·floor 확장 필요. 원장/알림을 다시 실행해 복구 금지 | SOURCE |
| E19 | [Daily release:39](../../scripts/ops/lib/owner-beta-daily-release.sh#L39), [activation:154](../../scripts/ops/lib/owner-beta-daily-operations.py#L154), [API 실제 env:413](../../scripts/ops/lib/owner-beta-daily-operations.py#L413) | Daily ON의 parser·activation·API generation 검증도 Paper disabled를 요구 | validator/compose 두 gate 외에 공존 전환·drain·관측 계약 확장 필요. 전역 disabled 검사를 무조건 삭제하지 않음 | SOURCE |

## 3. downstream 인터페이스와 의존성 판단

[계약 §7](../contracts/owner-beta-paper-v1.md#7-d5--증거의-권위와-checker-공개-인터페이스-초안)의 타입/거절코드가
ABI 초안이다. G1은 이름뿐 아니라 다음 입출력·authority를 채택해야 한다.

- `UntrustedPaperContractV1`과 `VerifiedPaperInputV1`을 분리한다. 후자는 실제 sealed reader+DB live gate만 생성한다.
  historical/Daily/generic enum의 서로 다른 branch를 유지하고 등록되지 않은 입력은 generic 실행에 보내지 않는다.
- 새 SQL reader는 현재 인증 Owner와 보호된 운영 Owner, 기존 Daily original IDs, revocation/source events와
  run/job/attempt terminal을 동일 일관 snapshot에서 읽는다. client-selected Owner를 신뢰하거나 admin pool을
  사용하는 shortcut은 금지한다. immutable historical evidence와 현재 authorization을 모두 반환해야 한다.
- root witness producer는 source execution origin·실제 image/config를 증명한다. Rust checker가 신뢰할 경로/
  소유권과 canonical schema·한도·event identity를 고정한다. Python ops는 독립 eligibility 구현을 만들지 않는다.
- API 읽기 DTO 초안: `operation_state`, `eligibility {eligible,reasons,checked_at,expires_at}`, `pre_sessions`,
  `post_sessions`, `consent_state`, `signal_date`, `effective_session`, `processing_date`, `price_basis`,
  `input_kind`, `capability`, `valuation_status`, `comparison_status`, `difference_codes`.
  기존 Paper routes에 beta mode branch를 추가하고 status 조회를 `GET /api/v1/paper/status`로 제안한다.
  preview request에는 서버가 검증하는 `recommendation_input_kind` discriminator를 추가할 수 있으나
  실제 pins·Owner UUID·source receipt를 public body로 받지 않는다. 응답 run ID에는 종류를 반드시 동반한다.
- 새 consent API는 기존 bind-strategy 의미를 보존하면서 beta 계정·계약 hash에 동의 event를 남기는 좁은
  transaction이어야 한다. exact endpoint/DTO version은 WP-3 시작 전에 G1/G2a에서 고정한다.
  Web은 fixed financial option과 설명을 표시하며 pin·운영 경로를 사용자가 입력하게 하지 않는다.

**의존성 gap:** 계획의 WP-2는 Daily execution/schedule/local_state만 receipt 접점 후보로 둔다.
그러나 EOD 수집은 `scripts/ops/kis-daily-production.sh`/`install-kis-daily.sh`의 별도 실행이다.
WP-5까지 producer·migration을 미루면 WP-2의 “실제 receipt reader” 인수와 WP-3 의존성에 순환이 생긴다.
권장은 coordinator가 G1에서 **증거 producer/reader의 선행 부분만** WP-2 이전 또는 WP-2의 확정 범위로
옮기고 migration 소유권을 고정하는 것이다. 운영 실행을 앞당기는 것이 아니라 소스·fixture 작업 순서를 조정한다.
WP-1은 재위임·계획 수정·구현을 하지 않았다. 나머지 독립 문서/fixture 준비까지 전면 차단할 이유는 없다.

후속 보완에서는 **B: 미래 slot을 arm하고 직접 dispatch하는 root scheduler/witness** 한 개를 권장한다.
아래 WP-2A producer/reader → WP-2B 기존 checker의 한 분기 분할을 제안한다. 전체 계획 변경과 채택은
coordinator 소유이며, 나머지 WP와 실제 활성화 gate는 유지한다. G1은 여전히 **OPEN**이다.

### 3.1 D5 보완 — B 단일 권장안의 근거와 검증

**기존 사실과 A/B 비교.** 현재 [installer:442](../../scripts/ops/install-kis-daily.sh#L442)는 root wrapper를
직접 실행하고 [timer:474](../../scripts/ops/install-kis-daily.sh#L474)는 `Persistent=true`다.
로컬 systemd 259.9 man의 `StartUnit/StartTransientUnit`은 job path를 반환하고, `JobNew/JobRemoved`는
job/unit/result를 전달하지만 timer/manual 원인을 전달하지 않는다. `TriggeredBy`는 정적 관계다.
timer man은 missed trigger의 즉시 실행과 이미 active인 service의 미재시작을 명시한다. 따라서 **A의
시간 근접 join은 manual start·catch-up·동시 job을 배제하는 원인이 될 수 없다**. 조사한 로컬 API에는
필요한 causal token이 없었다. systemd의 모든 가능한 계측 방법이 불가능하다는 결론은 아니다.

| 로컬 원문(네트워크 미사용) | 확인 위치: 압축 해제한 roff 원본 line | 확인 범위 |
|---|---|---|
| `/usr/share/man/man5/org.freedesktop.systemd1.5.gz` (259.9) | 71/141 StartUnit 계열, 289/292 Job 신호, 1837/1839 Triggers | 위 인자/정적 property 계약. 실행 origin API나 운영 관측 PASS 아님 |
| `/usr/share/man/man5/systemd.timer.5.gz` (259.9) | 44 active service, 200 AccuracySec, 359–361 Persistent | trigger 시간과 service invocation의 일대일 대응을 가정할 수 없음 |
| `/usr/share/man/man2/timerfd_create.2.gz` (Linux man-pages 6.17, 2026-02-08) | 57/65 clocks, 190–205 CANCEL_ON_SET/ECANCELED | B의 절대 REALTIME timer와 clock-step 거절을 지원하는 로컬 API 문서. 대상 호스트 지원은 미실증 |

위 gzip bytes SHA-256은 순서대로
`d5422a87cabc3016325a08de8529f18b13d3e04305e8336207d1522af1f879db`,
`678baa17da9ecd446ea33d69d1d6d73c1afb140235217e621e89eddbf8073654`,
`d0c84f0d2b2ac28a6f03aac633891d1e009eacff8ffdafb8ace93cd682168896`다.
이는 개발 환경 문서의 재현 identity이며 운영 버전/동작 보증이 아니다.

**B의 공개 계약(PROPOSED).** [계약 §7.2](../contracts/owner-beta-paper-v1.md#72-b-권장--미래-slot을-직접-소유하는-schedulerwitness-proposed)에
arm/slot/reservation/invocation, STARTED→ARMED→RESERVED→LAUNCH_INTENT→RUNNING→SOURCE_FINISHED→LINKED,
실패·crash·restart·수동·no-claim 의미와 join 순서를 고정했다. systemd 원인을 추측하지 않고 살아 있는
부모가 arm한 timerfd expiry와 자기 자식 실행을 원본으로 남긴다. root/kernel/Docker는 신뢰 주체다.
root가 거짓 원본을 만들면 탐지할 수 없다는 한계를 수용해야 한다. 임의 JSON/env/token은 이 신뢰를 얻지 못한다.

현재 [worker:1688](../../data-pipelines/collectors/src/worker.rs#L1688)는 batch/date/reused를 출력하나
[progress:254](../../scripts/ops/lib/backfill-progress.py#L254)는 날짜/상태/run_identity만 저장한다.
[wrapper:595](../../scripts/ops/kis-daily-production.sh#L595)는 기존 source에서도 exit0이므로 성공 exit만으로
새 원본을 주장할 수 없다. [CalendarDayLock:79](../../data-pipelines/collectors/src/eod_raw.rs#L79)의
재사용 판정으로 동시 manual EOD source capture를 거절하고, [SupplyResult:129](../../data-pipelines/collectors/src/action_supply.rs#L129)의
outcome/range/batch/reserved_wire_gets를 별도로 보존한다. `reserved_wire_gets`는 실제 호출 횟수가 아니다.
새 capture를 위해 이미 소비한 calendar/action claim을 재발급하지 않는다. 이 소스들이 이미 root witness를
남긴다고 쓰지 않는다. witness mode의 FD 전달·named container 관측·보호 원본은 모두 신규 구현이다.
특히 EOD source의 reused=false만으로 당일 calendar 원본도 신규라고 할 수 없다. 같은 CalendarDayLock 안의
기존 `has_claim` 판정과 실제 calendar batch를 `calendar_claim_preexisting`/`calendar_batch_id`로 전달하는
두 신규 필드가 필요하다. 선행 calendar claim은 무인 성공에서 거절한다. collector 두 파일의 이 결과 전달만
2A 후보에 추가하며 provider 요청·lock·재사용/실패 처리·calendar 호출 예산을 변경하지 않는다.

**구현 인터페이스/DB 소유권(PROPOSED).** 새 binary/image를 추가하는 대신 기존
[owner-beta-runner dispatch:351](../../crates/job-queue/src/bin/owner-beta-runner.rs#L351)에
`paper-evidence reserve|link-admission|link-run|read` 전용 subcommand를 제안한다. root scheduler가 공식
installer의 고정 one-shot helper 구성을 사용한다. 기존 image의 실제 digest, 고정 entrypoint/argv, read-only
원본 mount를 관측한다. 기존 [admitter UID/role 구성:199](../../scripts/ops/lib/owner-beta-daily-runtime.py#L199)를
참조해 reserve/link-admission은 UID10003·`daily_input_admitter`, link-run/read는 UID10001·`worker`로 분리한다.
전자는 해당 helper에만 supplementary GID10001 read 권한을 부여한다. 기존 secret staging에서 해당 role의
DB credential만 mount하며 broker credential·Docker socket·원본 쓰기 권한은 주지 않는다. root scheduler가
직접 DB 비밀번호를 로그/원본에 넣거나 admin pool을 쓰지 않는다. app에는 이 helper 쓰기 권한이 없다.

| 제안 SQL 함수(모두 0066, `migration_owner` 소유) | 실행 role / DB가 확인·저장할 내용 |
|---|---|
| `reserve_owner_beta_paper_source_slot(slot, reservation, root_event_hash, cohort, session)` | `daily_input_admitter`; 보호된 현재 Owner/policy/epoch와 기대 날짜 대조, 일일 dispatch 유일키, append-only 예약. 동일 identity만 멱등 readback. 성공 bool/임의 Owner 인자 없음 |
| `link_owner_beta_paper_source_admission(slot, source_event_hash, original_set_hash, admission)` | `daily_input_admitter`; 예약·실제 admitted row의 policy/base/parent/source identity를 대조, original의 다른 slot 귀속 거절. Rust helper가 sealed bytes/root event를 먼저 검증. SQL은 파일/시계를 검증했다고 주장하지 않음 |
| `link_owner_beta_paper_daily_run(slot, run_id)` | `worker`; 기존 DAILY reservation→run/job/attempt/claim/target 관계에서 결과를 도출. caller의 SUCCEEDED 문자열·F/W hash를 받아 대체하지 않음 |
| `read_owner_beta_paper_session_evidence(session, cohort)` | `worker`; 현재 보호 Owner에 한정해 immutable IDs/저장 root hashes, 현재 rights/revokes/epochs, 실제 terminal을 일관 snapshot으로 반환. root FS 검증은 Rust checker 책임 |

각 함수는 fixed search_path·session_user·FORCE RLS와 좁은 EXECUTE만 부여한다. 직접 테이블 쓰기/PUBLIC grant는
없다. 기존 admitter가 SQL 예약을 위조해도 protected root 원본·live arm·실제 child 관측이 없으면 checker는
거절한다. SQL row 자체를 origin 권위로 승격하지 않는다. 장애 유발 권한과 성공 증명 권한을 구별한다.
디스크 RESERVED를 먼저 fsync한 뒤 DB 예약/readback, 그 다음 launch한다. DB 실패/불명 결과면 launch하지 않고
그날 claim을 보존한다. admission/run 연결은 원본 수집 뒤이며, DB 연결 완료 뒤 root LINKED를 게시한다.
이중 기록 도중 crash는 UNKNOWN이다. 재시작 시 기존 identity의 대조/종결만 허용하고 수집 재발사는 금지한다.
admission 또는 run 성공을 뒤늦게 찾더라도 launch 증거가 없으면 성공으로 복구하지 않는다.

**WP-2A/2B 범위.** 아래 §4의 WP-2A가 보호 producer·원본 전달·실제 narrow reader·0066을 소유하고,
그 인수 뒤 WP-2B가 기존 streak/eligibility checker와 consumer를 연결한다. `owner_beta/mod.rs`, 새
`paper/mod.rs`, migration harness 등 공유 파일은 2A 종료/인계 후 2B/3만 수정한다. installer/helper wiring의
소스도 2A에 포함하되 배포·실제 활성화·backup/restore 인수는 기존 WP-5/7 gate에 남긴다.

| 필요한 검증 | 결정적인 거절 사례 / 합격 관측 | 증거 수준·현재 결과 |
|---|---|---|
| deterministic parser/state | stale/replayed nonce, wrong slot/arm/boot/cohort/config/image, duplicate original, unknown/빠진 terminal, 위조 LINKED, 파일 chain 삭제/교체 | unit/fixture; NOT RUN |
| 실제 systemd + timerfd | manual wrapper 실행은 MANUAL; scheduler 수동 시작 후 미래 arm→due만 인정. 부팅/재시작 때 과거·미arm·기존 intent slot은 미실행/비성공; 두 프로세스 중 한 개만 lock | 격리 Linux VM의 실제 service/process/FD 원본 필수; 순수 fixture 대체 금지. NOT RUN |
| 실제 시간·crash | disposable VM의 clock step→ECANCELED, suspend/늦은 wake→무효, reserve/spawn/raw commit/DB link 직후 SIGKILL→UNKNOWN; missing failure는 성공 아님. 같은 slot 재발사 0회 | 공유/운영 호스트 시계 변경 금지; 실제 kernel API/프로세스 통합. NOT RUN |
| 실제 Docker lifetime | `--rm` 없는 고정 named invocation의 ID/create/start/exit/image readback; 잘못된 image/config, 기존/중복 container, 조기 삭제/교체·cleanup 전 관측 누락 거절 | 로컬 Docker 통합과 owned cleanup 확인; Docker 동작을 mock만으로 인수하지 않음. NOT RUN |
| 실제 PG/RLS + 원본 | 역할별 grant/다른 Owner/철회·epoch race·예약 동시성, SQL 위조 row와 root event 없는 source 거절; original/admission/run 잘못된 join 거절 | 실제 disposable PG와 보호 filesystem 함께 검사. NOT RUN |
| wrapper 통합·budget | precheck 뒤 다른 writer의 capture→reused 거절; manual calendar-only claim 뒤 신규 EOD도 거절; 소비 claim 재호출·실패 후 무단 retry 0회; batch/reused/calendar/outcome가 FD로 동일 전달 | 실제 Linux process/pipe + offline provider fixture; fixture source는 SYNTHETIC로 production soak 거절. NOT RUN |
| 최종 운영 | 실제 image·config·권리·원본으로 pre/post 각3거래일, 정상 calendar/cutoff·기존 요청 예산 유지 | 별도 G4/G5; 위 통합 PASS가 실거래일 성공을 대체하지 않음. NOT RUN |

**G1 기술 OPEN은 다음 채택에 한정한다:** B/root 신뢰 경계 및 미래 arm60초·launch60초·clock/suspend1초
거절 정책, 위 helper/role/SQL/파일 소유권을 포함한 WP-2A→2B 분할. 이 기술안 채택은 O1/O2/O3 응답을
대신하지 않는다. 대상 Linux의 실제 timerfd·FD 전달·Docker 객체 보존/readback/cleanup·DB role 동작은 **미검증 수용 조건**으로
남겨 2A에서 실증한다. 실패 시 해당 실행은 ORIGIN_UNPROVEN이며 gate를 닫지 않는다. A용 causal API 추가
조사를 선행 blocker로 늘리지 않는다. 이 문서 보완에서 새 서비스·연결·권한을 실제로 만들거나 활성화하지 않았다.

## 4. 신규 migration과 정확한 파일 후보

현재 `rg --files migrations | sort -V`로 확인한 최대 번호는
[0065](../../migrations/0065_owner_beta_simulation_ledger_read.up.sql)다.
아래 **0066/0067은 후보 이름일 뿐 예약/생성하지 않았다**. G1 직전 실제 HEAD/max를 다시 확인한다.

| 후보 | 필요한 내용 | 근거/실행 전제 |
|---|---|---|
| NEW `migrations/0066_owner_beta_paper_evidence_v1.up.sql` / `.down.sql` | §3.1의 4개 함수·slot/original/admission/run 유일성과 append-only 관계, 현재 revoke/state 조회·제한 grants | WP-2A 소유 제안. 0060 receipt에 origin/runtime binding이 없고 직접 reader 권한도 가정할 수 없음 |
| NEW `migrations/0067_owner_beta_paper_v1.up.sql` / `.down.sql` | Paper control/consent epochs·운영 전이 event, beta preview/pending lineage discriminant와 FK, session/price/valuation provenance, 거래 유무 독립 유일키·typed terminal·실제 처리시각, 보존형 rollback guard | generic FK·SKIPPED·created_at::date는 요구를 충족하지 않음. 기존 migrations는 수정 금지 |

권장 새 DB identity는 `(owner,account,effective_session)` 유일성을 유지한다. terminal success와 실제 ledger
증거를 하나의 transaction 또는 검증된 recoverable commit protocol로 결합하고 zero-trade에도 durable 행을 만든다.
economic session·target/source ids를 주문/체결과 연결하며 시간 기반 중복 검사를 beta 경로에서 대체한다.
FORCE RLS와 직접 app/worker login 검사, fixed search_path, narrow SECURITY DEFINER grants, Owner/권리/current
epoch·source fence 잠금 순서를 명시해야 한다. migration down은 beta 이력·pending·outbox가 있으면 거절한다.
down PASS를 위해 원장을 비우거나 구 migration을 고치지 않는다. 두 migration 통합 여부도 coordinator 기술 결정이다.

아래는 **후속 구현의 정확한 파일 allowlist 제안**이다. 현재 WP-1 쓰기 권한은 여전히 문서 두 개뿐이다.
`NEW`는 실제로 만들지 않은 경로다. 기존 계획 밖 추가는 별도로 표시했고 자동 승인하지 않았다.
디렉터리 wildcard 권한으로 해석하지 않는다. 파일 내부도 Paper 접점으로 제한한다.

| 담당/분기 | 기존 파일 후보 | NEW 파일 후보 | 이유/범위 |
|---|---|---|---|
| WP-2A producer/wrapper — 추가·선행 후보 | `scripts/ops/kis-daily-production.sh`, `scripts/ops/backfill-production.sh`, `scripts/ops/lib/backfill-progress.py`, `scripts/ops/kis-daily-production-self-test.sh`; `data-pipelines/collectors/src/eod_raw.rs`, `data-pipelines/collectors/src/worker.rs` | `scripts/ops/lib/owner-beta-paper-evidence.py`, `scripts/ops/test_owner_beta_paper_evidence.py`, `scripts/ops/owner-beta-paper-evidence-local-integration.py` | B timerfd/claim/child/원본 producer, FD 전달·container identity/cleanup. collector는 calendar claim/batch 증거 필드와 해당 회귀만; provider API·예산·cutoff 변경 없음 |
| WP-2A installer/helper wiring — 추가·선행 후보 | `scripts/ops/install-kis-daily.sh`, `scripts/ops/deploy-production-release.sh`, `scripts/ops/lib/release-build-layout.sh`, `scripts/ops/lib/owner-beta-daily-runtime.py`, `scripts/ops/lib/owner-beta-daily-operations.py`, `scripts/ops/owner-beta-daily-operations-self-test.py` | 없음 (새 unit도 공식 installer가 render) | 보호 root service와 role별 one-shot helper의 고정 launch/mount/관측; old timer drain/disable. source 변경만 선행, 실제 activation은 후속 gate. WP-5와 순차 인계 |
| WP-2A Rust reader/DB — 추가·선행 후보 | `crates/job-queue/src/owner_beta/mod.rs`, `crates/job-queue/src/bin/owner-beta-runner.rs`, `tests/integration/migration-contract/tests/migration_contract.rs` | `crates/job-queue/src/owner_beta/paper/mod.rs`, `crates/job-queue/src/owner_beta/paper/source_evidence.rs`, `crates/job-queue/src/bin/owner-beta-runner/paper_evidence.rs`, `crates/job-queue/tests/owner_beta_paper_source_evidence.rs`; 위 0066 up/down | 동일 runner binary에 좁은 evidence subcommand; 권위 원본 reader, 실제 SQL/role/RLS. 새 DB role·credential 불필요, 새 함수 grants는 필요 |
| WP-2B pure contract/checker (2A 인수 후) | `crates/job-queue/src/owner_beta/mod.rs` | `crates/job-queue/src/owner_beta/paper/contract.rs`, `input.rs`, `evidence.rs` (같은 paper 디렉터리); `crates/job-queue/tests/owner_beta_paper_admission.rs`; `crates/job-queue/tests/fixtures/owner_beta_paper/three_sessions.json`, `rejections.json` | sealed adapter, typed parser/capability, streak/chain 검증. 2A의 새 paper/mod.rs를 인계받아 확장 |
| WP-2B runtime binding | `crates/job-queue/src/owner_beta/runner/daily_execution.rs` | `crates/job-queue/src/bin/owner-beta-paper-check.rs` | 실제 consumer revision/claim evidence 연결, ops/API 공통 checker CLI. **신규 CLI는 계획 추가 후보**, 2A helper와 역할 분리 |
| WP-3 beta core (WP-2 종료 후) | `crates/job-queue/src/paper_execution.rs`, `paper_preview.rs`, `paper_valuation.rs`, `paper_io.rs` (동일 src 디렉터리) | `crates/job-queue/src/owner_beta/paper/settlement.rs`, `comparison.rs` | 기존 planner·bounded IO·원장 write 재사용, beta price/typed session 분기. R4 엔진 변경 없음 |
| WP-3 API/daemon | `crates/api-server/src/paper_runner.rs`, `paper_session.rs`, `runtime.rs`; `crates/api-server/src/bin/paper-runner.rs`; `crates/api-server/src/repos/accounts.rs`, `paper.rs`, `pending_targets.rs`, `recommendations.rs`, `rebalance_previews.rs` (각 repos 디렉터리); `crates/api-server/src/http/paper.rs`, `state.rs`, `middleware.rs`, `dto.rs`, `mod.rs` (각 http 디렉터리) | 없음 | 실제 config/Owner·status·consent·DTO·beta FK 및 recovery 접점만 |
| WP-3 schema/API contract | `tests/integration/migration-contract/tests/migration_contract.rs`; `apps/api-server/scripts/openapi-spec.mjs`, `apps/api-server/openapi.json`, `apps/api-server/generated/openapi.ts` | 위 0067 up/down | evidence migration 이후 순차. migration harness 동시 수정 금지 |
| WP-3 focused 회귀 | `crates/api-server/tests/http_paper.rs`, `http_paper_rebalance.rs`, `paper_account.rs`, `paper_runner.rs`, `paper_scheduler.rs`, `paper_valuation.rs`, `paper_notifications.rs`, `paper_execution_seam.rs` (같은 tests); `crates/job-queue/tests/paper_preview.rs` | `crates/api-server/tests/owner_beta_paper.rs` | generic 회귀·실제 PG·Owner/RLS·restart·zero-trade |
| WP-4 Web | `apps/web/app/(authenticated)/paper/page.tsx`; `apps/web/lib/products/paper-contracts.ts`; `apps/web/lib/i18n/dictionaries/paper.ts`; `apps/web/lib/api/contracts.ts`, `product-client.ts`; `apps/web/tests/paper-surface.test.tsx`, `apps/web/tests/e2e/paper.spec.ts`, `apps/web/tests/e2e/support/paper-fixture.mjs` | `apps/web/tests/owner-beta-paper-contracts.test.ts` | 기존 계획의 디렉터리 범위 `components/paper/`는 아래 제한처럼 실제 수정 파일을 WP-4 시작 전 재고정 |
| WP-5 release/ops | `deploy/compose/compose.yml`; `deploy/runtime/Dockerfile.paper-runner`, `paper-runner-entrypoint`, `paper-runner-healthcheck-test.sh`; `scripts/ops/validate-production-config.sh`, `compose-release.sh`, `deploy-production-release.sh`, `run-production-backup.sh`, `README.md`, `production-ops-self-test.sh`, `production-ops-static-check.sh`, `owner-beta-daily-backup-self-test.py`; `scripts/ops/lib/release-build-layout.sh`, `release-image-manifest.sh`, `owner-beta-daily-backup.py`, `owner-beta-daily-backup-files.py`, `service-status-policy.py` | `scripts/ops/lib/owner-beta-paper-release.py`, `owner-beta-paper-operations.py`; `scripts/ops/test_owner_beta_paper_release.py`, `test_owner_beta_paper_operations.py`, `test_owner_beta_paper_backup.py`; `docs/runbooks/owner-beta-paper.md` | 기존 계획 접점. evidence producer와 겹치는 시점은 coordinator가 먼저 종료/인계 확인 |
| WP-5 Daily/Paper 공존 — 추가 후보 | `scripts/ops/lib/owner-beta-daily-release.sh`, `scripts/ops/lib/owner-beta-daily-operations.py`; `scripts/ops/owner-beta-daily-release-self-test.py`, `scripts/ops/owner-beta-daily-operations-self-test.py`; `docs/runbooks/owner-beta-daily-input.md` | 없음 | **기존 계획 밖 추가**. E19의 disabled 전제를 fresh Paper authority와 결합한 공존 검사로 확장. Daily-only/OFF/disable 동작 보존 |

위 표의 짧은 파일명은 같은 묶음에서 바로 앞에 명시한 repo-relative 디렉터리 prefix와 결합한다.
**현재 권장안의 제외:** `crates/portfolio-model/src/cost.rs`·`sizing.rs`, R4 simulation 코드, market-data
wire 계약, 기존 migration, generic dataset 등록 도구는 읽기 전용 참조다. O1 대안(R4 동일 계산)을 택하면
그 금융 분기만 새 파일 allowlist/회귀로 재조정한다. 기존 cost profile의 의미를 바꾸지 않는다.

Web component 추가 후보는 다음 현존 7개 파일로 한정한다. WP-1은 경로와 Web 지침만 읽었고 수정하지 않았다.
WP-4 dispatcher는 이 목록에서 실제 DTO에 필요한 파일을 선택하며 디렉터리 전체 권한으로 넓히지 않는다.

- `apps/web/components/paper/paper-bind-form.tsx`
- `apps/web/components/paper/paper-holdings.tsx`
- `apps/web/components/paper/paper-lineage.tsx`
- `apps/web/components/paper/paper-notifications.tsx`
- `apps/web/components/paper/paper-parity-panel.tsx`
- `apps/web/components/paper/paper-performance.tsx`
- `apps/web/components/paper/paper-rebalance-preview.tsx`

coordinator 소유는 `docs/ROADMAP.md`, R5 계획,
`docs/diagrams/component_architecture.puml`, `runtime_deployment.puml` 및 각 PNG다.
후속 schema/runtime/API 구조 변화 시 정확한 소스 evidence line과 로컬 PNG를 같은 변경에 포함한다.
이번 문서-only WP는 구조를 바꾸지 않아 도표를 건드리지 않았다.

## 5. 최소 Owner 결정 묶음과 G1 종료 조건

아래는 **coordinator가 전달할 결정안**이며 이번 worker가 Owner 응답이나 권한을 생성한 것이 아니다.
같은 질문을 과거 승인된 권리에 다시 묻지 않는다.

| 묶음 | 최소 결정 질문 | 권장 | 대안·영향 |
|---|---|---|---|
| O1 — 첫 운용 금융 | 첫 Paper를 069500 추세50/200·1천만원·정수주·기존 Paper 예시비용으로 시작할 것인가? | 계약 §4의 전체 표 채택: KRW·lot1·전환 시 재조정·KRX_ETF_DEFAULT@1의 정확한 가정·현금/배당 제외·동일 모형 benchmark. 실제 비용 주장 없음 | R4 10bp/0비용과 동일 규칙을 원하면 별도 profile/sizing 구현 범위가 필요. 검증 없이 기존 결과와 동일하다고 표시하지 않음 |
| O2 — 처리 시점과 자동 적용 | EOD 입수 뒤 그날 시가로 지연 모의 정산하고, 명시 opt-in한 한 계정/전략에 매일 자동 적용할 것인가? | T 추천·적용과 E 정산/평가를 각 KST civil-day 종료 전 완료; T→E 고정. 실제 개장 체결 완료 주장 없음. manual/catch-up은 soak 제외 | 수동만이면 R5-05 충족 불가. 개장 즉시 처리는 현재 승인된 입력으로 증명 불가하여 이 범위에서 제공 불가 |
| O3 — 중지·복구와 연속성 | OFF/복원·관련 release/config 변경 후 미정산을 보존하고 새 관찰/명시 재개를 요구할 것인가? | RESTORED_OFF, pending HELD/RECOVERY_REQUIRED, 신규 체결 중지·기존 commit/outbox 정산, 원장 보존. cohort 변경 reset, 복원 후 자동 resume 없음 | 호환 이력 이월/자동 resume는 새 동등성·복구 계약과 별도 검증 필요 |

**Owner에게 묻지 않을 것:** 개인 KIS 권리·read-only 일일 수집·ETF11 경계·R4 계산 재승인,
기존 수동 preview/apply 설계 승인. 기존 승인은 그대로 회수했다.
계정 UUID·실제 초기 날짜·운영 limits·image digest는 deployment packet의 실제 관측/구성값이며
임의 예시값이나 worker 판단으로 채우지 않는다.

G1을 닫으려면 coordinator가 다음을 함께 기록해야 한다.

1. O1~O3의 실제 채택/수정 응답과 계약 원문 hash. 미응답 값은 OPEN 유지.
2. D1 sealed adapter·beta FK, D3 두 input pin/시각, D4 typed settlement/cancel/일관 잠금,
   D5 §3.1의 B/role/SQL/시간 정책·schema·한도, D6 cohort/TTL, D7 floor/보존을 기술 결정으로 확정.
3. 두 migration 후보의 현재 max 확인·소유권 확정, evidence producer 선행 범위 조정. WP-2가 실제 reader를
   검증하기 전 G2a를 완전 PASS로 처리하지 않음. 신규 JSON field는 producer 구현 전 AVAILABLE이라고 표시 금지.
4. 정확한 후속 인터페이스·파일 목록을 worker brief에 포함하고 해당 분기만 시작. 미확인 운영 입력/3거래일은
   G4/G5의 별도 blocker로 남기며 소스 구현 완료와 혼동하지 않음.

## 6. 수용 행렬과 증거 구분

상세 행렬은 [계약 §10](../contracts/owner-beta-paper-v1.md#10-r4-비교와-수용-기준)이다. 본 WP의 결과는 다음과 같다.

| 검증 | 본 WP 결과 | 후속 요구 |
|---|---|---|
| 문서 상대 link·source 인용 경로/라인, 소유 scope, whitespace | 정적 검사 결과를 아래 기록 | coordinator가 D1~D7 의미와 판단을 독립 대조 |
| deterministic unit / 음성 회귀 | NOT RUN | fake/manual origin, missing/failed 날짜, 정상 다른 data hash, TTL·revoke·prefix·epoch, 비용/정수주/현금 |
| 실제 PG/RLS / migration | NOT RUN | 좁은 reader 권한·다른 Owner·stale/version race·zero-trade·commit unknown·down 보존 guard |
| API/worker / generic Paper 회귀 | NOT RUN | 실제 process config/revision, legacy fallback 금지, WAITING→settled, 실제 시간과 E 분리 |
| Web OpenAPI/Vitest/공식 E2E | NOT RUN | 권한·동의·대기/실패/복구·가격수익률/비교 설명. fixture PASS를 실제 Owner로 대체 금지 |
| hosted CI/required·smoke | NOT RUN | 같은 최종 후보 SHA. 기존 전체 required 보존 |
| root 운영 전환·암호화 backup·실제 restore | NOT RUN | fresh drain/cohort·정확한 image·원장/outbox 동일·RESTORED_OFF·old-reader 거절 |
| pre 3실제거래일 / post 3실제거래일 / Owner 확인 | NOT RUN / 실제 성공 기록 미확인 | 서로 다른 목록. 수동·catch-up·fixture·healthy 제외 |

SOURCE는 구현 파일을 읽었다는 뜻이다. fixture가 저장돼 있어도 실행하지 않았다면 PASS라고 쓰지 않는다.
기존 R4 운영 인수는 해당 기록의 대상 SHA/기간에서만 유효하다. R4와 Daily Paper의 input/signal/기간/가격/
비용이 다른 것은 차이 코드 또는 NOT_COMPARABLE이며 결과 숫자의 단순 유사성으로 비교 가능 판정하지 않는다.

## 7. 실행 기록·미해결·미확인

### 실제 수행과 제한

- cwd와 target SHA는 문서 머리의 값이다. `git status --short`, `git rev-parse HEAD`로 dirty 기준선을 확인했다.
  기존 dirty `docs/ROADMAP.md`와 untracked R5 계획은 coordinator 소유로 읽기만 했다.
- 루트 AGENTS 및 상위 경로 적용 지침, 지정된 paseo-delegate skill, ROADMAP, R5 계획의 목표/brief,
  Daily 계약/runbook, 8월 Paper 설계 두 개, Owner 출시 계획, R4 v1/v2 원본·채택·최종 인수를 읽었다.
- `rg --files`, `rg -n`, `nl -ba ... | sed -n ...`, `wc -l`로 필요한 코드/문서/마이그레이션 근거를 조사했다.
  탐색 중 존재하지 않는 후보 경로(`infra`, `db`, `recommendation_paper.rs`, `daily_price_only/wire.rs`,
  `sizer.rs`, `domain/money.rs` 등)는 `rg --files`로 실제 경로를 찾아 확인했다. 해당 탐색 오류를 검증 PASS로 세지 않는다.
- `apply_patch`로 위 신규 문서 두 파일만 작성했다. source 구현, migration, 설정·운영 파일, git index/commit,
  네트워크/provider·DB·서비스·secret·memory 접근, 재위임을 하지 않았다.
- 정적 검사: 로컬 stdlib 검사로 상대 링크/라인 범위·파일 참조를 검증하고, `git diff --check` 및
  두 신규 파일 각각 `git diff --no-index --check /dev/null <file>`를 실행했다. 결과/명령은 private receipt
  `target/r5-paper/WP-1/static-checks/receipt.json`에 남겼다. 제품 테스트나 hosted 실행은 허용 범위 밖이라 NOT RUN이다.

최초 산출물의 정적 검사 명령은 `python3 target/r5-paper/WP-1/static-checks/check_docs.py`다. 상대 link/source 인용
101개와 정확한 기존 파일 후보 73개가 존재하고, line 인용 범위 및 내부 heading이 일치했다.
`git diff --check`는 exit0/진단 없음이다. 두 `git diff --no-index --check /dev/null`은 신규 파일과의
차이 때문에 exit1이고 stdout/stderr가 모두 비었다. 최초 검사기는 이 exit1을 실패로 분류했으나 실제
whitespace 진단이 없음을 확인해 판정만 수정했다. 최초 receipt는 `receipt-initial.json`에 보존한다.
문서 링크·범위·whitespace 최종 정적 결과는 **PASS**이며 금융/권한 계약의 독립 인수나 제품 실행 PASS는 아니다.

**D5 후속 보완 기록:** 같은 cwd/SHA에서 `rg -n`, `sed -n`으로 위 source 접점을 읽고 Python stdlib
`gzip.decompress`/`hashlib.sha256`으로 §3.1의 로컬 man 원문·line·hash를 확인했다. 실제 systemd/D-Bus/
timerfd/Docker/PG를 호출한 검증은 아니다. 잘못 추정한 `deploy/docker/Dockerfile.owner-beta-runner`,
`crates/owner-beta/`, `collectors/src/r2_action_supply.rs`는 없었으며 `rg --files`/소스 참조로 실제
`crates/job-queue/Dockerfile.owner-beta-runner`, `data-pipelines/collectors/src/action_supply.rs`를 확인했다.
후속 문서 검사 명령은 `python3 target/r5-paper/WP-1/origin-followup/check_docs.py`이며 정확한 실행 결과와
각 문서 hash·link/source 대상·whitespace 명령은 같은 디렉터리의 private `receipt.json`에 기록했다.
후속 정적 검사 결과는 **PASS**: link/source 인용112개와 기존 파일 후보79개 존재, 인용 line/heading 유효,
`git diff --check` exit0 및 두 신규 문서 no-index check exit1/진단 없음(신규 파일 차이)이다.
후속 구현/제품·hosted·실제 local integration·운영은 권한 범위 밖이므로 모두 **NOT RUN**이다.

### brief와 다르게 처리한 부분

제품 쓰기 범위·금지 작업의 이탈은 **없음(none)**.
계획의 예상 WP-2 범위로 전체 증거 생산을 끝낼 수 없다는 점을 발견해 추가 파일·migration 후보와
의존성 조정 필요를 보고했다. 직접 계획을 재작성하거나 다른 worker를 실행하지 않았다.
G1은 Owner와 coordinator의 결정 사항이 남아 OPEN으로 두었다. 미채택 권장값을 기존 승인으로 기록하지 않았다.

### 미해결/후속 필요 작업

- O1/O2/O3의 실제 Owner 선택, coordinator 기술 결정 및 G1 종료.
- B scheduler/witness·60초/1초 정책·role/SQL/ABI의 기술 채택, WP-2A 구현 및 §3.1 실제 음성 통합검증.
- migration·입력 adapter·beta recommendation 연결·실제 처리시각·typed 상태/무거래 identity·복구/backup 구현.
- WP-2A producer/reader→WP-2B checker의 해당 분기 소유권 확정 및 WP-4 exact component 소유권 고정.
- Daily ON의 Paper disabled 전제(E19)를 공식 공존 전환·drain 계약으로 확장하는 WP-5 파일 소유권 추가.
- 실제 권리/입력/달력/배포 config의 현재 readback과 pre/post 실제 관찰은 후속 운영 패키지에서 수행.
- coordinator의 계획/로드맵 진행 기록 및 독립 인수. 본 worker는 해당 파일을 수정할 수 없다.

### 찾지 못했거나 확인하지 못한 것

- 기존 Owner-beta Paper의 구체적인 첫 전략·자금·비용·EOD 지연 정산·자동 적용 설정 채택 기록은 조사한 소스/문서에서 찾지 못했다.
  generic 설계 승인과 R4 금융 채택은 존재하며 이를 R5 운영값 승인으로 확대하지 않았다.
- 현 운영 entitlement row·보호 config·실제 계정/동의 상태·Daily 입력 admission/원본 bytes·실제3거래일 성공은
  접근 금지 범위여서 확인하지 않았다. 부재라고 단정하지 않고 미확인으로 기록한다.
- 기존 조사 경로에서 trusted end-to-end unattended checker/receipt producer를 찾지 못했다.
  두 운영 gate도 checker 미존재를 명시한다. 단순 field 추가로 진위 증명이 생긴다고 가정하지 않는다.
- A를 뒷받침하는 timer causal token은 확인한 로컬 systemd API에 없었다. B의 대상 플랫폼 timerfd/clock,
  실제 child FD 전달·Docker 객체 보존/readback·cleanup·helper DB role 동작은 이번 문서 조사에서 실증하지 않았다.
  각각 §3.1의 유한한 수용 조건이며 현재 기능이 존재하거나 PASS라고 가정하지 않는다.
- actual market publication/open available-at은 현 날짜형 EOD 입력에서 확인할 수 없다. batch retrieval과
  admission/처리 관측만 사용할 수 있고, 새 `usable_at` producer는 아직 없다.
- 제품 테스트·실제 PG/RLS·hosted CI·브라우저·운영복원·배포/관측은 전부 미실행이다.

문서 산출 완료와 R5-01/G1 채택 완료는 별개다. commit/push/배포는 수행하지 않았다.
