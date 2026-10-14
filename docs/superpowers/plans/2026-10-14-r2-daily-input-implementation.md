Execution skill: $paseo-delegate (required)
Native subagents: prohibited for worker packages

# R2 WP-4 — 채택된 일일 입력·자동 추천 구현

## Goal and boundaries

- 기준: `619634db32526032aae07ee2024a10f69fc560c4`, branch `excited-dolphin`.
- 작업 디렉터리: `/data/worktrees/3puw275b/excited-dolphin`.
- 상위: [R2 계획](2026-10-13-r2-daily-research-beta.md), [R2-04](../../ROADMAP.md#6-r2--일일-연구-베타-완성).
- Owner는 2026-10-14 “그걸로 진행해”로 [U2 계약](../../reviews/2026-10-13-r2-daily-input-decision.md)을 채택했다.
  동일한 계약 승인을 다시 요청하지 않는다. 제품·권한 규칙은 그 문서가 기준이다.
- 목표: 역사 v2/v3 재현을 보존하는 별도 일일 ETF11 입력, 독립 admission,
  opt-in 전략의 원자적 일일 예약·발행, 정확한 기준일 화면을 구현하고 격리 검증한다.
- 소스 완료 기준: 입력 거부/재현 fixture, actual-role DB 중복·권한·재시도 회귀,
  API/UI 계약 회귀, 독립 리뷰, 동일 후보 CI, 구조 변경 다이어그램의 근거 일치.
  운영 배포·실제 Owner와 3거래일 관찰은 상위 G4~G6에서 별도 인수한다.
- 제외: 새 provider/권리, trading/account, 과거 추천 전체 재생성, 운영 변경·DB migration 실행,
  신규 provider 호출, 서비스 lifecycle/timer 변경, R3~R5와 개발 속도 개선 5가지.
- 적용 지침: repository `AGENTS.md`, 사용자 제공 전역 위임·Mem0 정책,
  `apps/web/AGENTS.md`와 이를 참조하는 `apps/web/CLAUDE.md`, 두 Paseo skill.
  Web 구현은 설치된 `node_modules/next/dist/docs/`의 해당 가이드를 먼저 읽는다.
- 현재 기존 migration은 0059까지다. 새 번호·테이블·역할·DTO·파일 소유권은 D1 후
  코디네이터가 확정한다. 이전 계약을 새 계약으로 임의 재해석하지 않는다.

## Initial classification

| Package | Complexity | Basis | Confidence | Reclassification or escalation signals |
|---|---|---|---|---|
| D1 | hard | artifact/독립 권한/SQL snapshot/queue/replay를 잇는 새 구조 설계; 결론 자체가 산출물 | high | 현행 소스와 채택 규칙 충돌 또는 구현 묶음 하나로 검증 불가 시 누락 접점을 명시해 재분해 |
| D2 | simple | 기존 Web 전략 설정·추천 DTO·테스트의 제한된 경로 목록; 직접 파일 확인 가능 | high | 새 UI 정책 판단이나 저장소 전수 조사 필요 시 중단하고 범위 반환 |
| D2a | simple | lockfile에 고정된 Web 의존성 준비와 설치본 Next 가이드 읽기; lock hash·설치 exit로 확인 | high | 자원 부족·lock 불일치·설치 실패 시 반복 설치하지 않고 정확한 장애 반환 |
| D3 | intermediate | D1 설계의 고정 규칙·권한·중복·복구에 대한 독립 소스 근거 리뷰 | medium | 새 아키텍처 설계를 리뷰어에게 요구하게 되면 코디네이터가 D1 개정 |
| D4 | intermediate | D3-06에 따라 pure byte schema·정렬·hash·오류를 확정하는 제한된 계약 명세 | high | producer/DB/운영 의미 변경이 필요하면 해당 인터페이스를 코디네이터에게 반환 |
| D5 | hard | D3-01~05의 반복 공급·독립 calendar projection·기존 ACL/발행/복구·harness 접점 교정 | high | 채택 경계 밖의 provider 취득이나 새 제품 정책은 실행하지 않고 정확한 G4 차이로 분리 |
| D6 | intermediate | 고정 D5 교정안의 기존 D3 finding 해소와 D4 접점만 독립 대조 | high | 새 제품 정책·전면 재설계가 필요하면 구체적 미해결 접점을 반환 |
| I1a | intermediate | D4에서 고정한 unapproved wire 타입·encoding/hash와 provider-free 회귀 구현 | high | 명세에 없는 key/type/hash 의미가 필요하면 코드로 채우지 않고 D4 보정 요청 |
| I0a | intermediate | D6가 검토한 새 admitter 역할의 격리 harness 준비를 기존 네 파일에 한정 | high | 운영 secret/Compose 변경 또는 새 migration 의미가 필요하면 해당 접점을 반환 |
| I0a-R | intermediate | 고정된 네 harness 파일의 권한·실행 순서·직접 로그인 구현을 독립 검토 | high | 구체적 소스 반례만 반환하며 미실행 DB 검증을 source closure와 구분 |
| I1a-P2 | intermediate | 회수된 I1a의 세 cash code 문법·오류와 고정 회귀를 두 파일에서 보정 | high | 규칙 밖의 의미 변경이 필요하면 해당 접점을 반환 |
| I1a-R | intermediate | 고정 wire 구현 네 파일의 문법·hash·오류·bounded parser와 회귀를 독립 검토 | high | 새 신뢰/운영 정책을 요구하지 않고 도달 가능한 소스 반례를 반환 |
| I1a-CI | simple | 정확한 부분 후보의 기존 hosted CI 결과·checkout·필수 회귀 증거 회수 | high | 실제 로그 누락/모순은 미검증으로 반환하며 새 run을 만들지 않음 |
| I1a-F | intermediate | I1a-R01~06와 실제 컴파일 진단을 두 파일에서 보정 | high | 고정 D4를 바꿔야 하거나 같은 보정이 다시 실패하면 정확한 반례를 반환하고 재분해/상향 |
| I1a-CI-D | simple | 종료된 세 실패 job의 typed 진단과 실패 phase를 추가 회수 | high | 원인 근거가 없으면 UNKNOWN으로 남기고 SQLSTATE만으로 원인을 추정하지 않음 |

구현 패키지는 D1/D2와 D3의 실제 결과를 통합한 뒤 이 문서에 **정확한 파일 소유권·인터페이스·
검증 명령을 추가하고서** 시작한다. 현재 설계 작업을 소스 구현 완료로 세지 않는다.

## Execution graph

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
|---|---:|---|---|---|---|---|---|---|
| D1 | 1 | hard | 채택 계약을 실행 가능한 구조·인터페이스·파일 묶음으로 설계 | `docs/reviews/2026-10-14-r2-daily-input-design.md`만 | Owner 채택·기준 확인 | codex gpt-6-astra xhigh, 열린 아키텍처 판단 규칙 | 타입/DB/권한/예약/API/운영 접점과 정확한 수정 allowlist | 현재 소스 citation, 불변식·실패 행렬, 구현 분해 가능성 |
| D2 | 1 | simple | 기존 설정·추천 Web 접점과 검증 harness 목록 | `docs/reviews/2026-10-14-r2-daily-web-map.md`만 | Owner 채택·기준 확인 | codex gpt-6-luna medium, 직접 대조 가능한 소규모 조사 | 지정 경로의 DTO/화면/테스트 근거와 남은 접점 | 파일·symbol 존재와 source line 직접 확인 |
| D2a | 1 | simple | Web 구현 전 설치본 가이드·의존성 준비 | ignored root/Web `node_modules/`, npm cache, `docs/reviews/2026-10-14-r2-web-prerequisites.md` | D2 인수·코디네이터 자원 확인 | D2 worker Luna medium | exact lock 유지·설치 결과·읽은 guide 경로 | root workspace `npm ci`, manifest/lock 전후 hash |
| D3 | 2 | intermediate | 확정 전 설계의 독립 검토 | `docs/reviews/2026-10-14-r2-daily-input-design-review.md`만 | D1/D2 회수·코디네이터 통합 | codex gpt-6.1-sol high, 결론 산출물 리뷰 규칙 | actionable finding와 scope/증거 한계 | 채택 계약·실제 호출 경로·위협/회귀 행렬 대조 |
| D4 | 3 | intermediate | 첫 pure contract 구현의 모든 byte schema 확정 | `docs/contracts/owner-beta-daily-etf11-input-v1.md`만 | D3-06 수용 | D3 worker Sol high, 제한된 계약 분석 | 정확한 object/field/type/order/validation/hash/fixture 표 | D1/U2·기존 domain 타입·serializer와 직접 대조 |
| D5 | 3 | hard | 영향받은 설계·통합 패키지 교정 | `docs/reviews/2026-10-14-r2-daily-input-design-amendments.md`만 | D3-01~05 수용 | D1 worker Astra xhigh, 기존 열린 구조 교정 | finding별 해결안·exact SQL/공급/projection/harness 범위 | source 근거와 반례·실제 역할 검증 조건 |
| D6 | 4 | intermediate | D5 및 보정 D4와의 연결 검토 | `docs/reviews/2026-10-14-r2-daily-input-amendments-review.md`만 | D4/D5 회수·hash 고정 | codex gpt-6.1-sol high, 독립 리뷰 규칙 | 구체적 미해결 finding 또는 범위 한정 closure | D3 failure path·현재 source·수정 인터페이스 대조; 실행 없음 |
| I1a | 4 | intermediate | 독립 unapproved wire 계약·canonical hash 구현 | `crates/market-data/src/daily_price_only/{mod,contract}.rs`, `src/lib.rs`, `tests/daily_price_only.rs` | D4 코디네이터 인수·launch brief 고정 | Luna max | 새 pure module·golden/rejection 회귀 | rustfmt/diff, 이후 exact 후보 Rust/CI; 실행 전 source만 수용 |
| I0a | 5 | intermediate | 첫 0060 전 격리 role/bootstrap 기반 준비 | 두 `tests/common/mod.rs`, migration-contract의 `role-bootstrap.sql`·`bootstrap.sql`만 | D6의 D3-05 source-design closure 수용 | Luna max | 새 role preflight·DB별 권한·direct-login helper | rustfmt/diff·source 확인; 실제 역할/DB 검증은 이후 hosted 순차 실행 |
| I0a-R | 5 | intermediate | I0a 구현의 독립 소스 리뷰 | `docs/reviews/2026-10-14-r2-isolated-role-harness-review.md`만 | I0a idle·결과 회수·네 파일 hash 고정 | Sol high, 코드 리뷰 규칙 | 구체적 finding 또는 범위 한정 source closure | 소스·SQL 문자열·연결 순서 대조; 실행 없음 |
| I1a-P2 | 5 | intermediate | D6-01 present-empty cash metadata 구현·회귀 | `daily_price_only/contract.rs`, `tests/daily_price_only.rs`만 | I1a idle·보고 회수, 보정 D4 hash 고정 | 같은 Luna max | empty 보존·정확한 오류·515-byte fixture·nonempty bundle | rustfmt/diff; 후속 독립 리뷰·exact 후보 hosted Rust/CI |
| I1a-R | 6 | intermediate | 전체 pure wire와 보정본의 독립 코드 리뷰 | `docs/reviews/2026-10-14-r2-daily-wire-review.md`만 | I1a-P2 idle·최종 hash 고정 | Sol high | concrete finding 또는 범위 한정 source closure | 고정 D4·실제 source·meaningful 회귀·cached dependency 대조 |
| I1a-CI | 6 | simple | I1a/I0a 부분 후보의 hosted CI 증거 회수 | `docs/reviews/2026-10-14-r2-daily-wire-ci.md`, private exact-SHA CI receipts | source 후보 commit/push·기존 run 조회 후 코디네이터가 한 번만 dispatch | Luna medium | 정확한 run/job/step/테스트 결과·미검증 항목 | GitHub read-only logs와 SHA·필수 테스트 marker 대조 |
| I1a-F | 7 | intermediate | pure wire의 검증된 결함·경계 회귀 보정 | `daily_price_only/contract.rs`, `tests/daily_price_only.rs`만 | I1a-R 실제 idle·보고 회수, 기존 CI terminal failure 확인 | 기존 Luna max | 여섯 finding별 수정·필수 경계 회귀와 한계 | rustfmt/diff 후 독립 delta review·새 exact 후보 hosted CI |
| I1a-CI-D | 7 | simple | 기존 실패 run의 원인 증거 보강 | `docs/reviews/2026-10-14-r2-daily-wire-ci-diagnostics.md`, private `ci/779b3fa/diagnostics/`만 | CI worker 실제 idle·final/receipt 회수 | 기존 Luna medium | job별 최초 실패 phase·typed compiler/test 진단 | 기존 run 로그를 메모리에서 필터·candidate source와 대조, 실행 변경 없음 |

## Worker briefs

### 공통

작업 디렉터리는 위 절대 경로다. 작업 전 AGENTS와 ROADMAP, 채택 U2 및 본 계획을 읽는다.
제품·운영 파일, 다른 worker 보고서, ROADMAP/STATUS/상위 실행 문서는 수정하지 않는다.
보고서 전용 범위 예외에 따른 통합 문서 갱신은 코디네이터가 같은 변경에 담당한다.
Git stage/commit/push, root/DB/Docker/provider/production read, 빌드·테스트·대형 명령,
하위 위임은 수행하지 않는다. 출처가 없는 설계 전제를 사실로 채우지 않는다.

Do not use native subagent, Task, Agent, team, or delegation features. Complete this assignment directly and report if it needs further decomposition.

공통 반환: 변경 파일·라인 범위, brief와 다른 처리 및 이유, 실제 검사·결과,
미해결·후속, 찾지 못했거나 검증하지 않은 항목(없으면 `none`). background와 permission wait도 명시한다.
읽기 전용 source 탐색·문서 syntax/link 검사만 가능하며 실행 테스트 통과를 주장하지 않는다.
D2a에 한해 아래 명시된 npm 의존성 준비와 설치본 가이드 읽기를 허용한다. 제품 구현·테스트·
빌드·브라우저 설치·서버 실행은 이 예외에 포함하지 않는다.

### D1 — 입력·예약 아키텍처

- hard/high confidence: 기존 역사 타입·SQL guard와 새 자동 admission의 결합을 풀어야 한다.
- 입력: U2 계약 전체, ADR-0005/0006, contract-and-recovery 보고서,
  `crates/market-data/src/historical_price_only*.rs`, `crates/factor-engine/src/price_only.rs`,
  `crates/job-queue/src/owner_beta/`, `crates/api-server/src/http/owner_beta.rs`,
  migrations 0049~0052, 기존 producer/DB-role/immutable release/Compose 접점.
- 현행 v3 range_end/5-pin, historical replay와 수동 요청은 보존한다.
  producer가 admission을 쓸 수 없는 실제 역할/저장 분리를 제안하고 배포·복구 영향도 명시한다.
  비현금 action/정정/누락은 중단하고 원래 retrieval timestamp를 보존한다.
- 산출물: 현재 구조 근거, 최소한의 새 tagged 계약과 Rust/JSON/SQL 인터페이스,
  정확한 daily dedup key/transaction lock/opt-in·revocation/worker fence,
  DB role·RLS·migration up/down·stale claim/복구, API/화면 필드,
  실행 흐름과 namespace/permission, 배포 전제, 단계별 파일 allowlist·test acceptance.
  기존 code/migration checksum은 변경하지 않는 additive 설계를 우선한다.
- 완료는 코디네이터가 각 구현 묶음을 추측 없이 위임할 수 있을 때다.
  너무 크면 coupling 경계와 미정 사항을 정확히 반환하며 설계를 임의 채택했다고 쓰지 않는다.

### D2 — Web 접점 목록

- simple/high confidence: `apps/web` 안의 owner-beta 전략 설정·추천 화면과 이에 직접 쓰이는
  DTO/route/test에 한정한다. `rg`로 해당 경로를 찾아 직접 읽고 목록을 만든다.
- 입력: Web AGENTS/CLAUDE, U2 화면 규칙, `crates/api-server/src/http/owner_beta.rs`의 응답 DTO.
- 산출물: exact 파일·symbol·line, opt-in default OFF를 넣을 기존 설정 UI,
  expected/support/as_of/published 시각을 보여줄 현재 위치, owner/member/session/오류 상태,
  가용 fixture·unit/browser 명령, installed Next docs의 필요한 부분 위치.
- 새 정책·UI를 디자인하거나 구현하지 않는다. D1과 같은 파일에 쓰지 않는다.
  탐색 한계가 있으면 해당 범위를 명시해 코디네이터가 직접 검증하도록 한다.

### D3 — 독립 설계 리뷰

- intermediate/medium confidence: 이미 좁힌 설계를 소스·채택 규칙에 대조한다.
- 입력: D1/D2 보고서, coordinator의 확정 인터페이스·scope 개정, U2 전체 규칙.
- 검토: producer/admission 권한 분리 실효성, 파일 TOCTOU/불변성/정정·회수,
  역사 재현 보존, snapshot·dedup 원자성, 권한 철회/설정 변경·retry/claim/publication,
  migration/rollback 실제 역할, 기본 OFF·owner isolation·기준일 의미.
- finding에는 구체적 실패 경로·source line·수정 조건을 쓴다. 가설을 실제 결함으로 단정하지 않는다.
  테스트 실행·구현 파일 수정 없이 검증 가능한 인터페이스와 필요한 회귀를 판단한다.

### D2a — 설치본 Web 가이드 준비

- D2에서 `apps/web/node_modules/next/dist/docs/` 부재를 확인했다. 캐시에는 정확한 Next 16.3.0
  tarball이 있다. 읽지 않은 가이드를 읽었다고 주장하거나 다른 버전의 기억으로 구현하지 않는다.
- simple/high confidence. 코디네이터의 현재 가용 메모리·디스크·동시 compiler 부재 확인 후 시작한다.
- 최초 app-prefix 명령은 app-local lockfile 부재로 EUSAGE/exit1이었다. root `package.json`의
  workspace 선언과 실제 root lockfile, `npm ci --help`로 선택자를 확인해 명세 오류를 바로잡았다.
  이 실패는 prerequisite 보고서에 보존하며 worker의 구현 실패로 상향 배정하지 않는다.
- 위 작업 디렉터리에서 보정 명령 `timeout -k 15s 10m env NODE_OPTIONS=--max-old-space-size=512 npm ci --workspace apps/web --include-workspace-root=false --ignore-scripts --no-audit --no-fund`를 한 번 실행한다.
  lockfile의 package registry 접근만 허용하며 시장 provider·인증서비스·운영 API는 금지한다.
  설치 실패 시 root 사용·global 설정 변경·lock 재생성·재시도 없이 결과를 반환한다.
- 허용 파일은 ignored dependency tree·npm cache와 새 prerequisite 보고서뿐이다. package/lock/source,
  다른 보고서·진행 문서·Git은 변경하지 않는다. manifest/lock hash를 전후 대조한다.
- installed Next docs에서 이번 기존 server page/client form와 unit/browser 테스트에 해당하는 가이드를
  읽고 정확한 경로·version·주의점과 source가 아닌 준비 범위를 기록한다.
- 완료 보고는 공통 형식. 설치 exit 0을 제품 테스트 성공으로 표시하지 않는다.

## Coordinator gates

1. 현재 profile/model 지원과 workspace/parent를 확인한다. D1/D2는 disjoint report이므로 병렬 가능.
   `notifyOnFinish=true`, private owned-worker ledger와 bounded fallback을 유지한다.
2. 실제 idle/pending0 후 보고서·source citation을 회수하고, 채택 계약과 scope를 직접 대조한다.
3. 설계 접점과 migration 번호·구현 파일 소유권을 확정하고 D3의 구체적 finding을 영향 범위별로 해소한다.
4. 구현 패키지를 상세 추가한 뒤 Luna/max로 시작한다. D3가 허용한 pure I1a는 D4 인수 후
   D5 교정과 병행할 수 있다. DB/독립 승인/storage/normalizer/운영 경로는 D5 수용 전 구현하지 않는다.
   실패·새 불확실성에만 단계적으로 상향한다.
   공유 파일 변경은 순차로 배정하고 resource-heavy 검증은 hosted/격리 환경에서 순차 실행한다.
5. 정확한 구현 후보에서 Rust/DB/Web/권한/입력 검증 및 구조 다이어그램·독립 리뷰를 통합한다.
   기존 f8b9296 결과를 새 코드의 실행 증거로 재사용하지 않는다.
6. 소스 검증·커밋·진행 기록과 실제 배포·Owner 인수·3거래일 관찰을 분리한다.
   추가 운영 조치는 준비 가능한 산출물을 완성한 뒤 G4에서 명령·영향을 제시한다.

## D1/D2 통합과 D3 입력 고정 — 2026-10-14

D1/D2/D2a의 실제 idle/pending0와 보고 회수를 확인했다. D2의 현재 UI·DTO 및 설치본 Next
가이드는 확인됐고 제품·schema는 아직 바꾸지 않았다. D1의
[구조 제안](../../reviews/2026-10-14-r2-daily-input-design.md)은 926행,
SHA-256 `e74934b7e5909dc69713bac16118a5d816d25e38498ebbe69dffa45304e9c670`으로
D3 입력을 고정한다. 검토 중에는 D1 보고서를 다른 worker가 수정하지 않는다.

코디네이터는 역사 타입·5-pin·수동 요청 보존, 별도 daily 타입, 새 0060 migration,
기존 보호 job type/계산식 재사용, producer/admitter 파일·DB 분리, default OFF와 영구 예약 키의
방향을 채택 계약에 맞는 검토 대상으로 수용한다. 세부 권한·SQL·storage를 검증 완료했다고
주장하지 않으며, D3가 구현을 막는 구체적 설계 모순을 구분한다.

D3는 특히 다음을 확인한다.

1. 휴장일을 포함한 action/calendar 증거 요구를 현행 소스가 채울 수 있는지, 단순 운영 증거
   부재인지 수집 경로의 구조적 누락인지. 자동 일일 동작을 영구 정지시키는 요구를 G4로 숨기지 않는다.
2. generic action normalization과 별도 일일 cash-only 검증의 경계가 U2와 맞는지.
3. 기존 table-level grant/definer/RLS와 새 column/role/epoch 권한의 실효성, lock 순서·발행·복구.
4. snapshot 재봉인·동일 일 두 번째 실행·legacy hash 변경이 가능한 경로와 필요한 회귀.
5. I1 전체가 너무 크거나 스키마의 구체성이 부족하면, 안정된 pure contract/fixture와 source
   검증/storage를 분리해 첫 구현 범위를 제시한다. 운영 값 부재를 소스 전체 착수 금지로 확대하지 않는다.

새 product/source 작업의 exact 인터페이스와 수정 allowlist는 D3를 회수해 확정한다.

## D3 판정·첫 구현 분리 — 2026-10-14

[독립 리뷰](../../reviews/2026-10-14-r2-daily-input-design-review.md) 423행,
SHA-256 `1ea75c9e1f8612475923775c5b2b49c2f3b7a4a2aa395e482d09cda1c4c9a487`를
실제 idle/pending0 후 회수했다. 코디네이터는 기존 0009 config table grant,
0049 worker output UPDATE 권한, calendar normalizer의 target-date 필터와 대조해 6개 finding을
수용한다. U2 채택은 유지하고 R2-04 전체를 다시 승인 대기로 돌리지 않는다.

### D4 — I1a exact byte 계약

공통 보고·금지 규칙을 유지하며, 코디네이터는 다음 범위를 고정한다.

- 새 파일 `docs/contracts/owner-beta-daily-etf11-input-v1.md`만 소유한다. D1/D3·다른 문서를 고치지 않는다.
- D1 §§3~4와 D3-06의 policy/manifest/session/file/admission을 완전한 JSON/Rust field 표로 만든다.
  모든 nested type, key 순서, optional/null/empty 규칙, enum literal, 날짜·UTC 시각 precision,
  redacted request 구조, copied-source chain, action class/scope/interval/page, civil calendar binding,
  normalization descriptor와 evidence hash preimage를 명시한다. `<...>` 같은 미정 wire field는 남기지 않는다.
- I1a는 **unapproved value** parse/encode/hash와 문법·구조 불변식만 구현한다. Raw 의미 검증,
  파일 신뢰·권한·storage, DB, provider, 승인 타입 constructor, runtime 활성화는 포함하지 않는다.
- 기존 v2/v3 JSON·5-pin 의미·hash는 바꾸지 않는다. 정책 fixture의 가짜 hash가 production pin으로
  취급되지 않게 fixture/validating wire/policy trust를 분리한다. 운영 control/Owner/예산은 wire 밖이다.
- canonical serializer는 고정 struct key 순서·UTF-8 compact JSON·명시된 stable array 순서·integer/
  string scalar로 정한다. unknown/duplicate keys와 모호한 값은 거부한다. 입력 map/array 정렬 허용
  여부 및 parser hash 검증과 canonical encoder의 책임을 서로 모순 없이 고정한다.
- API 함수 signature/return/error enum, content preimage의 제외 field, final manifest/admission hash,
  finite error(외부 본문/값 없음)를 명시한다. 현재 domain `ContentHash`, 날짜·timestamp·BatchId와
  serde 코드에 맞춘다. 새 dependency는 요구하지 않는다.
- 정상 synthetic golden fixture와 parent/policy/lineage 변경, duplicate/unknown/불완전 필드,
  날짜/순서/배열 경계, 역사 wire 보존의 정확한 테스트 기대를 정의한다. 순수 Python hash/JSON
  fixture 산출은 허용하되 Rust compile/test와 source 구현은 금지한다.
- 완료 기준은 Luna가 wire field/hash 결정을 발명하지 않고 I1a를 구현할 수 있는 명세다.
  부족한 운영 값은 값 미정 schema가 아니라 typed input 자리로 명시한다.

### D5 — 공급·권한·발행·검증 연결 교정

- 새 `docs/reviews/2026-10-14-r2-daily-input-design-amendments.md`만 쓴다. 고정 D1/D3를 보존하며
  finding별 어떤 절을 대체하는지 명시한다. D4는 wire 계약을 별도 소유한다.
- D3-01: manifest batch date가 아니라 요청 구간으로 이미 저장된 WholeMarket/ETF11 range를
  찾고 civil-day×class×scope×pagination coverage를 검증하도록 확정한다. 한 번의 catch-up이
  다음 주말을 해결한다고 쓰지 않는다. 반복 공급의 구체적 source 제안·허용 기존 surface·변경되는
  query/date 범위·요청 상한·실패 정책을 준비하되 새 네트워크 실행/승인으로 해석하지 않는다.
- D3-02: 이미 취득한 credentialed 첫 페이지의 주변 calendar 행을 원래 hash/retrieval/source와
  결합하는 별도 offline daily projection을 설계한다. generic normalizer나 data_batches를 위조·
  변경하지 않고 새 SQL join·원래 target 포함·충돌·future/copy 검증·receipt 권한을 구체화한다.
  소스가 필요한 civil 날짜를 포함하지 않으면 typed gap; 추가 chk-holiday 호출은 허용하지 않는다.
- D3-03/04: 정확한 config INSERT/UPDATE column grants·DB epoch/ID/owner/opt-in trigger,
  daily terminal run/output/item 불변성·current attempt settlement·direct DML 우회를 닫는다.
  기존 역사 bytes/동작과 적법한 retry/recovery는 보존하고 DOWN의 원래 ACL 복원까지 명시한다.
- D3-05: official `validate.sh`, 두 tests/common role bootstrap, production config validator,
  secret/preflight/UID·membership·CONNECT/USAGE, generated OpenAPI 출력의 소유권을 정확히 추가한다.
  새 role은 첫 0060/harness 실행 전에 준비하는 dependency로 옮긴다.
- D3-06 중 runtime/SQL 부분: ADMITTED/REPLAY/BLOCKED 결과와 committed conflict event,
  독립 pure normalization의 deterministic identity·무쓰기 경로를 확정한다. D4 field 명세를 중복 작성하지 않는다.
- 테스트·DB·root·Docker·provider·설치·서비스 변경 없이 source 근거와 필요한 fixture/실제 역할
  검증을 보고한다. 권한 승인 재요청 없이 source 준비를 계속하고 G4에서 필요한 추가 효과만 분리한다.

### I1a — launch 전 고정할 공통 범위

D4를 코디네이터가 읽고 정확한 SHA·함수/타입 목록·golden 조건을 launch brief에 붙인 뒤 시작한다.
위 네 파일만 수정한다. `mod.rs`/`lib.rs`의 pure module 등록 외 다른 구조는 바꾸지 않는다.
historical·provider·normalizer·Cargo/lock·policy registry·DB·API·Web·배포·diagram은 이 묶음 밖이다.
운영 build/로컬 Cargo 없이 rustfmt와 소스 정합성만 확인하고, 실제 Rust 회귀는 순차 hosted 검증에
맡긴다. 테스트 미실행을 성공으로 표시하지 않는다. 코디네이터가 문서·로드맵·커밋을 통합한다.

## D4/D5 회수·I1a launch 고정 — 2026-10-14

두 worker의 실제 idle/pending0와 최종 보고를 회수했다. D5는 740행·SHA-256
`8a8796d992da533389bac97fab6abf86c7e89e75f0772f6deceb3d4051d651a6`로 고정한다.
D1/D3는 그대로 보존한다. 기존 config SQL·0052 gate·bootstrap/validator·Raw reader와 대조했고,
공급/DB/storage/runtime 구현 전 D6가 구체적인 교정안의 남은 결함을 독립 검토한다.

D4 원본 876행·`541bdfb7…`에서 코디네이터가 다음 세 접점을 직접 보정했다.
SinglePage 응답 marker는 보존하되 따라가지 않으며, 겹치는 action witness 수와 선택한
정규화 행 수를 분리하고, legacy calendar hash는 새 daily document hash와 구분한다.
필드·signature·기본 golden byte는 바뀌지 않는다. 최종
[D4 계약](../../contracts/owner-beta-daily-etf11-input-v1.md)은 904행·SHA-256
`308b9909932a323440557afc06ebd3c3a7fd26e44be38a502a02c25803ce377d`다.
7개 Python golden 계산과 17개 상대 링크·라인 경계 및 고정 D1/D3 hash를 직접 검증했다.
이를 **pure unapproved wire 구현 명세로 수용**하며 Raw/DB/운영 인수로 확대하지 않는다.

### I1a 실행 brief

- cwd는 이 계획의 workspace. intermediate/high confidence; 고정 명세 구현이므로 Luna/max.
- 위 D4의 정확한 hash를 먼저 확인한다. §6의 public struct 25개, enum 4개, private timestamp
  wrapper, payload-free error enum과 public 함수 17개 및 timestamp method 2개가 고정 interface다.
  §2~7의 key/null/lexical/order/bound/hash graph를 구현하며 별도 정책 결정을 만들지 않는다.
- 수정 allowlist는 `crates/market-data/src/daily_price_only/mod.rs`, 같은 디렉터리의 `contract.rs`,
  `crates/market-data/src/lib.rs`(module 선언만), `crates/market-data/tests/daily_price_only.rs` 네 파일뿐이다.
  필드/함수 누락·상충이 있으면 해당 부분을 보고하며 Raw trust/approved type을 만들어 메우지 않는다.
- §8의 Python construction을 Rust fixture로 옮겨 7개 고정 byte/hash를 검증한다. §9의
  duplicate/unknown/missing nullable/type/lexical/depth/size/order/copy/coverage/internal hash,
  parent/policy/code/source 변경 민감도와 보정한 SinglePage·witness·legacy hash 회귀를 넣는다.
  원래 역사 golden을 수정하지 않고, 새 구조가 외부 실제 bytes를 신뢰하지 않는 negative case를 보존한다.
- historical/provider/normalize/storage/Cargo/lock/registry/DB/API/Web/deploy/diagram/문서는 수정하지 않는다.
  root/DB/Docker/provider/network/운영 read·build·Cargo/test 실행·Git 쓰기·하위 위임은 금지한다.
- 로컬 검사: 허용 파일만 `rustfmt --edition 2024`와 `rustfmt --check`, `git diff --check`.
  실제 Rust 검증은 source 검토 뒤 exact candidate hosted/격리 CI에서 코디네이터가 순차 실행한다.
  예상 명령은 `cargo test --locked -p market-data --test daily_price_only`; 현 단계에서 실행하지 않는다.
- 공통 보고 형식을 따른다. 구현/테스트 작성과 실행 PASS를 구분한다. D6와 같은 파일을 쓰지 않는다.

### D6 실행 brief

- cwd는 이 계획의 workspace. intermediate/high confidence; 결론 산출물 리뷰이므로 Sol/high.
- 고정 D5와 보정 D4, D1/D3 및 채택 U2를 읽는다. 소유 파일은
  `docs/reviews/2026-10-14-r2-daily-input-amendments-review.md` 하나다.
- D3-01~05와 runtime 부분 06의 구체적 실패 경로를 교정안이 닫는지, 특히 source-event lock,
  retry/cancel/attempt order, actual-role/harness 선행, calendar projection 순환 제거를 확인한다.
  D4의 세 보정 및 D5 A6 reconstruction 사이에 남은 구체적 모순도 확인한다.
- 전체 설계를 다시 작성하거나 I1a의 pure wire 구현을 운영 입력 부재로 차단하지 않는다.
  새로운 blocker는 도달 가능한 source/interface 반례·수정 조건·영향 package와 함께 반환한다.
- source/계약/다른 보고서/진행 문서는 수정하지 않는다. 컴파일·테스트·DB·root·Docker·provider·
  production read·Git 쓰기·하위 위임 없이 hash/link/라인/syntax 검사만 허용한다.
- 공통 보고 형식과 finding severity를 쓴다. closure는 source-review 한정이며 실행 성공이 아니다.

## D6 인수·D6-01 보정 예약 — 2026-10-14

[D6 보고](../../reviews/2026-10-14-r2-daily-input-amendments-review.md)는 289행·SHA-256
`b95b5e652d2d459cb5de871943f1dcdcf1ffa6ec33b0d9dc324dcbd01b869269`다.
실제 idle/pending0 후 회수하고 66개 링크와 배당 validator·fixture·기존 recovery query를 대조했다.
D3-01~05의 **설계 수준 closure**를 수용한다. D5의 current attempt/claimed_by 확인은
기존 recovery가 하는 일이 아니라 I2/I4가 구현해야 할 요구다. 현재 RecoveryAttempt/query에
claimed_by가 없다는 D6의 근거 한정을 유지한다. 실행/운영 검증은 아직 없다.

D6-01 P2를 수용한다. `normalize.rs:1890`의 required_present_string은 present string을
trim하며 빈 결과를 허용하고, `kis_normalization.rs:85`의 유효 cash fixture도 세 코드가 비어 있다.
다음 **단일 lexical 보정**을 확정한다. 이미 실행 중인 I1a의 입력 문서/hash는 바꾸지 않고,
그 worker가 idle/pending0가 된 뒤 계약 갱신·hash 고정과 별도 follow-up으로 적용한다.

- `dividend_kind`, `stock_kind`, `high_dividend_flag`의 `code`를 ASCII
  `[A-Za-z0-9_-]{0,32}`로 바꾼다. empty string은 보존하되 missing은 MissingField,
  null/non-string은 InvalidType, 그 밖의 문자·길이는 InvalidLiteral이다.
- wire parser는 trim하거나 값을 만들어내지 않는다. I1b가 기존 shared validator의 trimmed
  string을 그대로 매핑하고 이 별도 bounded wire 규칙을 검사한다. 허용 범위 밖은 typed rejection이며
  임의 code/null 대체·행 삭제가 아니다. legacy validator는 바꾸지 않는다.
- 회귀 row: 첫 golden의 dividend page FileKey, symbol069500, record_date2026-08-31,
  세 code 모두 `""`, face_value5000, cash_amount100, cash_rate2, stock_rate0,
  cash_payment_date2026-09-14, stock/odd_lot date null. §4의 기존 key 순서로 직렬화한
  one-row array는 515 bytes·`sha256:0f87c41ba19ba820ee79a2a05f54c27b9a4af5f185a98b080bc429b56eaca864`다.
  nonempty cash chain/manifest/proof/admission round-trip와 missing/null/type/32·33길이 거부를 추가한다.
  기존 empty-cash golden 7개는 그대로다. private recipe는 `target/r2-daily-beta/wp-4-integration/`에 있다.

현재 I1a는 `308b9909…` 명세로 계속 실행 중이다. 결과를 그대로 최종 수용하지 않고 위 보정 후
정확한 후보의 코드 리뷰·Rust/CI를 진행한다. 새 미승인 입력이나 시장 API 동작을 허용하는 수정이 아니다.

### I0a — 격리 role/harness 준비의 고정 실행 brief

- cwd는 이 계획의 workspace. intermediate/high confidence, 명세가 정해진 구현이므로 Luna/max.
  I1a와 파일이 겹치지 않으며 D6-01의 cash 문법에도 의존하지 않는다.
- 소유 파일은 `crates/job-queue/tests/common/mod.rs`, `crates/api-server/tests/common/mod.rs`,
  `tests/integration/migration-contract/role-bootstrap.sql`, 같은 디렉터리의 `bootstrap.sql` 네 개다.
  D5 A5/A8-I0 중 이 네 fixture 경로만 맡는다. 운영 bootstrap/secret/Compose/official validator와
  0060은 후속 I0/I2 소유이며 이번에는 고치거나 실행하지 않는다. 전체 I0 준비 완료를 주장하지 않는다.
- cluster bootstrap에 `daily_input_admitter`를 absent일 때만 LOGIN NOINHERIT NOSUPERUSER
  NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS, 기존 fixture password `lagrange`로 만든다.
  이미 있는 role의 속성/멤버십을 임의 ALTER/REVOKE로 수리하지 않는다. 위 bool 속성을 검증하고,
  role을 member/roleid 어느 쪽으로든 쓰는 pg_auth_members 행은 거부한다. recursive membership
  closure로 app/worker에서 migration_owner/admitter에 닿는 경로도 거부한다(SET ROLE 포함).
- 기존 supervisor bootstrap 잠금/기존 역할·URL·migration-owner 실행 방식은 보존한다.
  새 역할의 DB별 CONNECT/USAGE와 CREATE 금지는 **새 scratch DB에서**, 첫 MIGRATOR 전에 확정한다.
  migration-contract role-bootstrap은 cluster role만, bootstrap.sql은 DB-local grant만 소유한다.
  supervisor public schema에 새 revoke/grant를 넣지 않는다. scratch public의 PUBLIC CREATE를
  회수하고 migration_owner CREATE는 유지한다. 새 역할의 current DB CONNECT·public USAGE 및
  current DB/public CREATE 없음도 검사한다. static SQL과 안전한 identifier 처리를 사용한다.
- JQ ScratchDb와 API Harness에 각각 public async `daily_input_admitter_pool(&self) -> PgPool`
  helper를 추가한다. 기존 연결 정보의 대상 scratch DB와 fixture password를 사용한 **직접 LOGIN**,
  최대 연결 2, current_user=session_user=daily_input_admitter를 확인한다. SET ROLE/Owner actor
  GUC는 쓰지 않고 URL/credential을 출력하지 않는다. 반환 pool은 호출자가 종료하는 계약이다.
  기존 public 메서드/필드 의미는 보존하며 unrelated helper refactor는 하지 않는다.
- 최소한의 기존 helper 패턴·sqlx API를 따르되 모순/필수 인접 수정이 있으면 구현으로 숨기지 않고 반환한다.
  출처는 D5 A5, D6의 first-migration 절 및 현재 네 파일이다. 새 provider/생산 권한은 없다.
- 로컬은 read-only source와 `rustfmt --edition 2024 --config skip_children=true`/check,
  `git diff --check`만. Cargo/rustc/test/DB/root/Docker/network/production/lifecycle/Git write는 금지한다.
  SQL 파서 실행·effective role 검증은 수행하지 않았다고 보고한다. 실제 검증은 official/harness
  source 통합 후 격리 CI에서 순차 진행하며, 나머지 I0 연결 전 첫 0060 실행은 계속 금지한다.
- 공통 최종 보고 형식과 직접 작업 규칙을 따른다. 문서·로드맵·커밋은 코디네이터가 통합한다.

## I0a 회수·I0a-R 독립 검토 — 2026-10-14

I0a worker는 01:48:29 UTC에 실제 idle/pending0가 됐고 최종 보고를 회수했다.
네 파일의 diff에서 role preflight, scratch DB 전용 권한 준비, MIGRATOR 이전 호출,
직접 로그인 helper 연결을 확인했다. 보고된 rustfmt/diff 검사는 소스 검사이며 SQL 실행이나
실제 role 인수가 아니다. 원래 I0a의 source acceptance는 독립 리뷰와 후속 격리 검증을 남긴다.

| 검토 입력 | 행 수 | SHA-256 |
| --- | --- | --- |
| `crates/job-queue/tests/common/mod.rs` | 399 | `973cb28656db8fd59e17f302f022ec167383b291bd1c0dd095b36425b62ac69d` |
| `crates/api-server/tests/common/mod.rs` | 933 | `6735a723f3c874c129ed43685f8633c372a8384ba2d23da407bebb7d18ed7dcb` |
| `tests/integration/migration-contract/role-bootstrap.sql` | 94 | `9dfe008fdd08b49587940e14fc6f441b2a0053da5f48651fec98fdb53c31b85a` |
| `tests/integration/migration-contract/bootstrap.sql` | 33 | `24fcb7f582f927d9d94827597c74989e005c3378530a3b9c5677e6ac29bf3c77` |

### I0a-R 실행 brief

- intermediate/high confidence. 기존 독립 리뷰 worker를 Sol/high로 재사용한다. 소유 출력은
  `docs/reviews/2026-10-14-r2-isolated-role-harness-review.md` 하나다.
- 위 네 파일 hash와 diff 기준 `c073d7c`를 확인하고 I0a의 고정 brief/D5 A5에 맞춰 검토한다.
  신규 role 속성·양방향 membership·cycle-safe transitive path 거부, 불량 기존 role의 무수리,
  supervisor와 scratch의 grant 분리, migration 전 privilege 확인, direct LOGIN·actor GUC 없음,
  기존 잠금·연결 정보·cleanup 의미와 SQLx 호출 정합성을 확인한다.
- 도달 가능한 실패 경로와 정확한 source anchor가 있는 finding만 제시한다. 기존 baseline 문제와
  새 회귀를 구분하며 전체 I0/0060/운영 준비가 안 됐다는 사실만으로 이 한정 변경을 거부하지 않는다.
- 제품 소스와 I1a의 실행 중인 네 파일, D4 계약, 진행 문서·로드맵은 변경하지 않는다.
  컴파일·Cargo·rustc·테스트·SQL parser·DB·root·Docker·network·provider·운영 읽기/실행·Git write 금지.
  source read/hash/link/line 검사와 허용 네 파일의 rustfmt check·diff check만 허용한다.
- 공통 최종 보고 형식을 따른다. SQL 실행·실제 role/pool/ACL은 미검증으로 남기고
  coordinator의 인수 전 완료로 주장하지 않는다. 하위 위임 없이 직접 수행한다.

### I1a/I0a-R 최종 보고 누락 후 재개 — 2026-10-14

02:06 UTC 확인한 두 worker의 idle/pending0에는 최종/오류 보고가 없었다. 이를 완료로
수용하지 않고 원래 고정 brief·모델·파일 소유권으로 각각 notified follow-up을 전달했다.
02:07:37 UTC 새 turn의 running/pending0를 확인했다. I0a 네 hash와 I1a D4 hash는 그대로다.
원인은 미확인이다. 원래 I1a 결과를 회수하기 전 D6-01을 적용하거나 source를 최종 수용하지 않는다.

## I1a 원본 회수·D6-01 구현 follow-up / I0a-R 수용 — 2026-10-14

I1a는 02:09:24 UTC 실제 idle/pending0 후 최종 보고를 회수했다. 네 파일 hash와 module 선언만
추가된 lib.rs diff, 25 wire struct·4 enum·17 free function·2 timestamp method 목록을 직접 확인했다.
원본 contract.rs는 4,058행·`e9fcfe944acf02db9ae9fd2bad6ec0b684cec63c837ada7bc6e7310c5e044ef0`,
test는 1,504행·`c336bf9c4c3798efd469703b1c4fd82f6dc1ee95d1132e0afc2ebe5b5fc9a22f`다.
rustfmt/diff 검사와 테스트 작성은 실제 Rust 실행이 아니다. 원본은 아직 최종 인수하지 않는다.

예약했던 D6-01을 D4에 적용했다. 새 계약은 **950행**, SHA-256
`fa0529033945437e78eca5b0805f0524a4ae3c75fc4f0a36321ec56b052e717f`다.
기존 7개 Python golden은 그대로이며, 새 present-empty cash array도 515 bytes와 기존 고정 hash가
일치했다. 이 계약을 I1a-P2의 입력으로 고정한다. D4 §11의 앞선 보정 기록도 보존했다.

### I1a-P2 고정 실행 brief

- 같은 Luna/max worker의 notified follow-up. 소유 파일은
  `crates/market-data/src/daily_price_only/contract.rs`와
  `crates/market-data/tests/daily_price_only.rs` 두 개뿐이다. module/lib/다른 소스/문서는 변경하지 않는다.
- 위 D4 hash를 먼저 확인한다. 세 cash code에 `[A-Za-z0-9_-]{0,32}`를 적용해 빈 문자열을 그대로
  보존한다. missing은 MissingField, null/non-string은 InvalidType, 허용하지 않은 문자·길이는
  InvalidLiteral이다. 기존 valid_code 및 parse/typed encoder의 두 검증 경로를 같은 규칙으로 맞춘다.
  wire parser의 trim/대체, legacy shared validator 수정, payload-bearing error는 금지한다.
- §8의 별도 515-byte row를 구현하고 row-array preimage/hash, nonempty dividend chain과
  proof/manifest/admission bundle parse→encode·재검증을 검사하는 회귀를 작성한다. witness count와
  선택 정규화 행 수 및 내부 hash를 정확히 갱신한다. 기존 primary golden 7개는 변경하지 않는다.
- 세 필드 각각 empty·정상 32바이트 통과, missing/null/non-string·33바이트·공백/control/non-ASCII/
  허용 밖 문자의 정확한 오류를 확인한다. public typed preimage/encoder와 JSON parse 경로를 모두
  다룬다. invalid row가 단순 HashMismatch 때문에 거부된 것을 lexical 회귀 통과로 세지 않는다.
- 나머지 구현을 리팩터링하거나 original 명세의 별도 의미를 바꾸지 않는다. 범위 밖 결함을 발견하면
  정확한 반례로 보고한다. 로컬 read/pure fixture 계산/rustfmt edition2024/check/diff만 허용한다.
  Cargo/rustc/build/test/SQL/DB/root/Docker/network/provider/운영/Git write/하위 위임은 금지한다.
- 공통 최종 보고 형식에 원본 대비 변경과 정확한 파일 hash·라인을 포함한다. written regression과
  실제 실행을 구분하며 최종 검증은 독립 소스 리뷰와 exact candidate hosted/격리 CI로 넘긴다.

I0a-R은 02:10:13 UTC idle/pending0 후 [157행 보고](../../reviews/2026-10-14-r2-isolated-role-harness-review.md),
SHA-256 `6ad13d8a0f622e3cc5eadc1fb67e1a050340e7e9fcebce12114f1a606838e0dc`를 회수했다.
직접 읽은 네 파일 diff·전후 hash·호출 순서와 일치하며 concrete finding 없음이라는 **소스 검토 한정**
결론을 수용한다. 현재 검토 문서/계약의 45개 링크·라인 경계도 확인했다. 실제 compile/SQL/role/pool/
ACL/cleanup은 미검증이며 나머지 I0 운영·official 연결 전 첫 0060 실행은 계속 금지한다.

## I1a-P2 회수·독립 리뷰와 hosted 검증 준비 — 2026-10-14

02:20:06 UTC 실제 idle/pending0 후 I1a-P2 최종 보고를 회수했다. 코디네이터가
valid_code, JSON lexical 검사, typed cash-row 검사와 새 fixture/bundle/오류 회귀를 직접 읽었다.
세 값의 empty 보존·InvalidLiteral 분류가 같은 규칙을 따르며 문서 hash는 그대로다.
테스트는 작성됐지만 Cargo/rustc/실행은 아직 없다. 이 source 후보를 독립 리뷰와 기계 검증에
넘기며, 리뷰 완료나 CI 성공을 선취하지 않는다. I0a 코드도 함께 포함된 부분 후보이고 최종 C_R2가 아니다.

| 정확한 리뷰 입력 | 행 수 | SHA-256 |
| --- | --- | --- |
| `crates/market-data/src/daily_price_only/mod.rs` | 8 | `e143131e3bc23628179d6f2fe2bff0f0539b960f86d311b5ad9627f1610b2135` |
| `crates/market-data/src/daily_price_only/contract.rs` | 4057 | `4cf96f83b5a91e4891975e6fe577aa29a24fce92ce05b3c118cd9e4531285833` |
| `crates/market-data/src/lib.rs` | 288 | `fc74f278107f2e6835014dffedaff5cc860f89094ca48fa1942e1c87747dfb75` |
| `crates/market-data/tests/daily_price_only.rs` | 1788 | `860c728b9c76c838acbf95d4ab7a15967a0b8c7498d9cbc700ebeda9841973f1` |

### I1a-R 고정 실행 brief

- intermediate/high confidence, 독립 코드 리뷰 규칙에 따라 Sol/high. 네 파일의 exact hash와
  D4 950행 `fa052903…`를 확인한다. 출력은 `docs/reviews/2026-10-14-r2-daily-wire-review.md` 하나다.
- D4 §2~9와 code의 strict JSON duplicate/unknown/missing/type/lexical 검사, 깊이/크기/count 상한,
  timestamp/numeric 보존, set 정렬과 ordered chain 구분, request/copy/coverage/hash graph,
  모든 public parse/typed encode/preimage/bundle 경로를 검토한다. D6-01 보정도 포함한다.
- 소스 한정의 구체적 도달 경로·line·실패 조건이 있는 finding만 반환한다. Raw trust/DB/운영 인수는
  의도적으로 이 pure wire 밖이다. public unapproved type을 승인 타입처럼 해석하지 않는다.
- 컴파일/strict clippy에 걸릴 수 있는 source 문제와 meaningful fixture의 누락도 확인하되
  실제 compiler 결과처럼 보고하지 않는다. 실행 증거는 별도 exact candidate CI에서 회수한다.
- 제품 소스/계약/로드맵/기존 리뷰는 수정하지 않는다. source/hash/link/line/rustfmt check만 허용하며
  Cargo/rustc/test/DB/SQL/root/Docker/network/provider/운영/Git write/하위 위임은 금지한다.
  cached dependency source read는 허용한다. 공통 최종 보고 형식을 따른다.

### I1a-CI 준비와 회수 범위

- 코디네이터가 이 부분 후보의 소스·진행 문서를 커밋해 기존 `excited-dolphin`에 fast-forward push한다.
  main의 별도 개선은 통합하지 않는다. remote branch는 준비 시 `619634d`, 기존 두 CI는 terminal success였다.
- push 후 exact head SHA의 기존 CI를 먼저 조회한다. 없을 때만 기존 `ci.yml` workflow_dispatch를
  한 번 요청한다. feature push는 이 workflow를 자동 시작하지 않는다. run이 있으면 새 실행하지 않는다.
- 독립 리뷰와 hosted 기계 검증은 병행 가능하다. 한 번에 한 후보 workflow만 운영하며 실제 Rust/DB
  명령은 기존 hosted workflow의 순서를 유지한다. 운영 호스트의 Cargo/DB/Docker/build는 실행하지 않는다.
- simple/high confidence 회수는 Luna/medium에 위임한다. 정확한 commit/run ID와 report/receipt 경로는
  실제 dispatch 응답 뒤 brief에 고정한다. workflow 재실행/취소/새 실행·Git 변경 권한은 주지 않는다.
- 7개 실행 job/required, 모든 checkout SHA, daily_price_only 테스트 3개 cash 회귀와 primary golden,
  기존 historical v2/v3 테스트, PG migration/count/checksum/skip marker, Web/Python 결과를 실제 로그로
  확인한다. 단순 step success로 개별 실행을 추정하지 않는다. 미호출 daily_input_admitter_pool의
  실제 direct-login/GUC/ACL 또는 아직 없는 0060 증거를 기존 CI 성공에서 추론하지 않는다.
- 실패는 typed compiler/test/step 진단과 source anchor만 저장하고 raw verbose 로그·secret/provider
  본문은 저장하지 않는다. final report에 미실행/미검증과 stale 기존 CI의 한계를 명시한다.

### 실제 후보·CI 회수 assignment — 2026-10-14

- source 후보는 `779b3faa787922032c682b9cf3fe07675a3d976f`다. I0a와 I1a를 포함한 부분 후보로,
  최종 C_R2가 아니다. feature branch로 fast-forward push했고 remote SHA를 확인했다.
- exact-SHA existing-run 조회는 빈 배열이었다. 기존 ci.yml을 한 번 dispatch해
  [run 38500926595](https://github.com/sihioov/Lagrange/actions/runs/38500926595),
  workflow_dispatch·02:25:19 UTC 생성·해당 full SHA·queued를 확인했다. 중복 실행하지 않는다.
- I1a-R Sol/high `d9ca76a3-b7dc-4913-944f-10ce8cc35d80`와 I1a-CI Luna/medium
  `11a8f223-6f2d-4adb-89ab-86dbc917d2fd`의 실제 running/pending0·workspace·parent를 확인했다.
  둘 다 notified follow-up이다. CI worker는 이 run의 읽기만 하며 새 실행/취소/Git 권한이 없다.
- CI 보고 파일은 위 표의 경로, private receipt는 `target/r2-daily-beta/ci/779b3fa/`다.
  60초보다 자주 poll하지 않고, 90분 또는 04:00 UTC 중 먼저 오는 시각까지 관찰한다.
  terminal 미도달·로그 부재·오류는 INCOMPLETE로 보고하고 coordinator가 명시적으로 인계/연장한다.
- source의 `#[test]` 함수는 16개다. primary golden 및 세 cash 회귀를 포함한 실제 test 이름/결과를
  checkout SHA와 결합한다. helper 미호출이나 아직 없는 0060의 실행을 추정하지 않는다.
- `55390735` 감시는 끝내고 두 소유 worker만 다루는 `1d5a6342`로 교체했다.
  2분·60회·04:27:42 UTC 만료이며 최종 결과/권한 대기를 회수하기 전 방치하지 않는다.

## I1a 리뷰·CI 실패 인수와 보정 범위 — 2026-10-14

기존 run `38500926595`는 `779b3faa787922032c682b9cf3fe07675a3d976f`에서 terminal failure다.
코디네이터가 GitHub run identity/conclusion과 [CI 보고](../../reviews/2026-10-14-r2-daily-wire-ci.md)
58행·`86c31bec…`, private receipt 741행·`aa44df38…`를 직접 대조했다. policy/format/Web/Python은
성공했으나 clippy/workspace/PG/required는 실패했다. workspace 명령이 건너뛰어져 신규 16개
테스트 성공 근거는 없다. PG의 59 migration/checksum/일부 guard는 전체 성공이 아니며 원인 확인이 남는다.

I1a-CI는 02:40:08 UTC, I1a-R은 02:41:50 UTC actual idle/pending0이고 실질 최종 보고를 회수했다.
리뷰 316행·`92da4c046f8653075a4fb8631475eb37d49b7ee4e1eee357ffe856f1538b7540`의
[R01~06](../../reviews/2026-10-14-r2-daily-wire-review.md)을 source anchors와 대조해 수정 대상으로 수용한다.
R02의 timestamp 비교 및 Option inference 3곳은 실제 CI compiler 진단으로도 확인됐다.
D6-01 present-empty 규칙은 소스상 일치하지만 bundle 실행 성공은 아직 아니다.
기존 후보를 수용하거나 같은 run을 재실행하지 않는다. 고정 D4 950행 `fa052903…`는 유지한다.

### I1a-F 고정 실행 brief

- 기존 구현 worker `2ec1c669-0e57-462e-a397-d6b2c3fbd4a0`, Luna/max. 읽을 입력은 이 계획,
  D4, I1a-R 보고 전체와 CI 보고다. 수정은 `crates/market-data/src/daily_price_only/contract.rs`와
  `crates/market-data/tests/daily_price_only.rs` 두 파일만. 원본 hash는 위 exact-input 표와 같다.
  public 25 struct/4 enum/17 function/2 timestamp method·오류·wire bytes·7 golden·515-byte cash fixture,
  기존 historical 코드·I0a 파일·계약·Cargo/lock·module registration은 변경하지 않는다.
- R01: 전체 session/bar 수는 canonical manifest sessions 길이와 checked multiplication으로 구한다.
  added/evidence 수는 daily_sources 기준을 유지한다. fixture의 base 2개를 제품 코드에 고정하지 않는다.
  base가 2개보다 많은 valid bundle과 정확한 count 불일치 거부를 회귀로 추가한다.
- R02: timestamp wrapper의 inner as_timestamp()를 비교해 nanosecond/nonincreasing copy order를
  보존한다. 세 Option accumulator에 구체적 타입을 주고 필요하면 private type alias를 쓴다.
  서로 다른 identity의 valid multi-hop과 시간 역전 거부를 추가한다.
- R03: proof document.session_date도 typed encoder에서 0001..9999를 검증한다. 0/1/9999/10000년
  parity 사례는 내부 commitment를 올바르게 재계산해 stale-hash 거부로 날짜 오류를 숨기지 않는다.
- R04: 같은 visit 분기를 합치고 dead helper/불필요 mut을 제거한다. slice 인자, io::Error::other,
  private tuple alias 등 국소 lint 정리를 한다. public API를 바꾸거나 crate-wide allow로 숨기지 않는다.
  불가피한 private schema/fixture helper의 복잡도만 좁고 이유가 있는 allowance가 가능하다.
- R05: D4 fixed guard 각각의 bound/bound+1을 의미 있게 검사한다. source/document/bar declared size,
  civil dates, action chains, file occurrences, cash rows, sessions, pages/copies, input/output bytes와
  depth를 매핑한다. positive pages/copies는 고유 identity와 유효 continuation/time을 사용한다.
  다른 schema/byte 한계로 nominal positive가 불가능한 경우 guard를 격리하거나 실제 선행 거부를
  명시한다. duplicate/hash/shape 오류를 해당 limit 검증처럼 세지 않는다. 거대 배열·무제한 반복은
  피하고 가능한 경우 작은 내부 pure guard 테스트 또는 declared sizes를 사용한다. private helper
  테스트는 같은 contract.rs의 cfg(test)에 한정할 수 있으나 public entry-point 회귀도 유지한다.
- R06: seal이 반환하기 전 최종 manifest canonical byte bound를 검사한다. preimage/hash/기존 상한은
  변경하지 않는다. near-limit draft/final pair에서 seal 성공이면 encode도 가능함을 검사한다.
  불가능한 fixture나 명세 모순은 임의 규칙으로 메우지 말고 정확히 보고한다.
- 실행은 read-only source, fixture용 순수 로컬 계산, rustfmt edition2024/check와 diff/whitespace만.
  Cargo/rustc/build/test 실행, DB/SQL/root/Docker/network/provider/운영/Git write/위임은 금지한다.
  최종 보고에 finding별 수정 anchor, guard→test matrix/미검증, 정확한 hash·행 수를 포함한다.
  written test와 실제 PASS를 구분한다. 로드맵·문서·후속 CI는 코디네이터 소유다.

### I1a-CI-D 고정 실행 brief

- 기존 collector `11a8f223-6f2d-4adb-89ab-86dbc917d2fd`, Luna/medium. 종료된 run38500926595의
  clippy job115647446770, workspace job115647446703, PG job115647446742만 read-only 재검토한다.
  기존 CI 보고/receipt는 동결한다. 새 보고 한 파일과 private diagnostics 하위 파일만 작성한다.
- 최초 실패 step/phase, compiler/test/SQL의 typed 진단·source anchor·실제 exit를 회수한다.
  chart step과 PG가 같은 market-data compile 실패인지 별개인지 실제 로그로 구분한다.
  예상된 SQL guard 거부와 terminal 실패를 혼동하지 않는다. 근거가 없으면 UNKNOWN이다.
- raw log는 메모리에서만 처리한다. 안전한 Rust diagnostic/source 위치, 명령의 이름·phase,
  typed SQLSTATE/test 이름/상태와 원본 응답 hash만 남기며 URL credential/provider body/free-form
  DB 행·원시 verbose 로그는 저장하거나 출력하지 않는다. 기존 candidate source/workflow read는 가능하다.
- 새 run/재실행/취소/dispatch, 로컬 tests/build/DB/root/Docker/운영/provider/Git write는 금지한다.
  30분 또는 03:30 UTC 중 먼저 도달한 시각까지 결과를 반환한다. 로그 부재나 분석 범위 초과는
  INCOMPLETE로 남긴다. 소스 수정·원인 추정·추가 승인 요청은 하지 않는다.

두 follow-up 모두 공통 직접 작업 문장과 최종 보고 규칙을 따른다. notified assignment 후 실제
상태/범위가 확인되면 기존 heartbeat를 새 두 작업 범위로 교체한다. 기존 성공·실패 근거는 보존한다.

실제 follow-up은 I1a-F 02:44:30 UTC, I1a-CI-D 02:44:55 UTC에 시작했고 running/pending0와
workspace/parent를 확인했다. `1d5a6342`를 삭제하고 `a80e3b3a`(2분·60회·04:45:39 UTC 만료)로
교체했다. CI-D의 이번 반환 경계는 03:14:55 UTC다. 두 범위 밖의 실행 권한은 추가하지 않았다.

### I1a-CI-D 회수 — 2026-10-14

02:49:54 UTC actual idle/pending0 후 [추가 진단](../../reviews/2026-10-14-r2-daily-wire-ci-diagnostics.md)을
회수했다. clippy exit101, chart compile exit101, PG의 `api-tenancy-rls` 단계 compile failure와
enclosing exit1은 동일한 E0369/E0282 네 건이다. 원래 세 log response hash와 candidate workflow/
validator blob을 직접 대조해 이 원인 증거를 수용했다. PG inner Cargo exit는 여전히 unknown이고
후속 DB test·chart/new pure test 실행은 없다. SQLSTATE를 terminal 원인으로 간주하지 않는다.
I1a-F 범위는 유지하며 실행 중 worker에게 새 지시를 보내지 않았다. `a80e3b3a`는 삭제하고
`45e28caf`(2분·60회·04:51:06 UTC 만료)로 I1a-F만 관찰한다. I0/0060/runtime 인수는 미완료다.

## I1a-F 회수와 보정 후보 검증 — 2026-10-14

03:12:53 UTC actual idle/pending0 뒤 두 파일의 최종 보고를 회수했다. 코디네이터가 전체 session 수의
전달·checked bar 계산, proof 날짜 검사, 최종 seal byte 검사 및 관련 회귀를 직접 읽었다.
수정 전후 diff는 두 파일에 한정되며 D4 950행 `fa052903…`와 public interface inventory는 유지한다.
아래 hash를 독립 검토 입력으로 고정한다. 아직 컴파일·Clippy·테스트 성공이나 6개 finding 종결을
주장하지 않는다. 기존 실패 후보와 로그는 보존하며 새 후보를 별도로 검증한다.

| 입력 | 행 수 | SHA-256 |
| --- | --- | --- |
| `crates/market-data/src/daily_price_only/contract.rs` | 4163 | `79370bc25c927aa0d4b19e9646ab1bccfb845b8980a800217282a172901ccf2a` |
| `crates/market-data/tests/daily_price_only.rs` | 2283 | `ea08ac3ae55c7a4b667556cec100cd25ac32298d2d21a7c209b490c412f355e4` |

### 추가 package 분류와 순서

| Package | Complexity | Basis | Confidence | Reclassification or escalation signals |
| --- | --- | --- | --- | --- |
| I1a-F-R | intermediate | 고정된 6개 finding과 두 파일 delta의 의미·경계 회귀 검토 | high | 명세 모순 또는 범위 밖 Raw/DB 설계가 필요하면 해당 지점을 보고하고 코디네이터가 재분해 |
| I1a-F-CI | simple | 정확한 후보의 기존 hosted run과 이름이 정해진 테스트 결과 회수 | high | 로그 부재·추가 분석 필요·반복 CLI 실패는 추정하지 않고 범위와 근거를 보고 |

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
| --- | ---: | --- | --- | --- | --- | --- | --- | --- |
| I1a-F-R | 1 | intermediate | R01~06 보정 및 새 회귀의 독립 검토 | `docs/reviews/2026-10-14-r2-daily-wire-corrections-review.md` 하나 | I1a-F idle·보고 회수·위 hash 고정 | 새 Paseo Sol/high, code-review 규칙 | finding별 판정·재현 경로·검증 한계 | hash·source/diff·line·rustfmt check, 실행 결과 추정 금지 |
| I1a-F-CI | 1 | simple | 보정 commit의 실제 compiler/test 결과 회수 | `docs/reviews/2026-10-14-r2-daily-wire-corrections-ci.md`, 새 candidate의 private CI 하위 경로 | 코디네이터 commit/push·기존 실행 확인·단일 run 식별 | 기존 Paseo Luna/medium collector | job/step/test·checkout SHA·sanitized receipt | 기존 GitHub run 읽기; 로컬 Rust/DB 실행 금지 |

### I1a-F-R 고정 실행 brief

- 작업 디렉터리는 이 계획 상단과 같다. D4, 원래 I1a-R 316행 `92da4c04…`, CI-D 진단과
  `779b3fa` 대비 현재 두 파일의 diff를 검토한다. wrapper/lib 및 D4는 변경하지 않는다.
- R01 전체 manifest session/bar와 added/evidence 집계 분리, R02 nanosecond 비교·세 accumulator,
  R03 typed/JSON 날짜 parity, R04 strict warning 노출, R05 모든 fixed guard의 실제 호출 및
  bound/bound+1 회귀, R06 최종 seal/encoder 일치와 near-limit fixture를 각각 판정한다.
  named helper만 검사한 제한과 public 경로 검증을 구분하고, 대형 fixture의 유효성·자원 크기를 점검한다.
- 7개 primary golden, 515-byte cash와 정확한 오류, 25 struct/4 enum/17 public function/2 method,
  unapproved-only 경계가 보존됐는지 확인한다. concrete source path·조건·영향 없이 결함을 추측하지 않는다.
- 출력은 위 새 review 한 파일뿐이다. source/hash/diff/line 및 rustfmt check만 허용한다.
  Cargo/rustc/Clippy/test/build/DB/SQL/root/Docker/provider/운영/Git write/하위 위임은 금지한다.
  실제 실행은 CI 담당이 회수한다. 공통 보고 형식에 finding별 disposition과 정확한 입력 hash를 포함한다.

### I1a-F-CI 고정 실행 brief

- 코디네이터가 수정 소스와 진행 문서를 한 candidate로 commit/push하고 remote SHA를 확인한다.
  해당 SHA의 기존 run을 먼저 조회하고 없을 때만 기존 ci.yml을 한 번 dispatch한다.
  `38500926595`는 terminal failure로 보존하며 재실행·취소하지 않는다.
- run ID/full SHA/시각/private 경로는 실제 조회 뒤 notified follow-up에 고정한다. collector는
  그 run만 읽고 workflow 변경·추가 실행·Git write를 하지 않는다. 60초보다 자주 poll하지 않으며
  시작 후 90분 또는 전달된 UTC 경계 중 먼저 도달하면 terminal 여부와 미회수 항목을 보고한다.
- 7개 실행 job과 required, checkout-bearing log의 SHA, 신규 24 integration+5 private unit test의
  정확한 이름/결과와 기존 historical v2/v3, chart, PG validation, Web/Python을 회수한다.
  test 이름과 실행 여부는 로그로 확인하고, 미호출 direct-login helper·0060·runtime을 추론하지 않는다.
  raw 로그는 메모리에서 필터링하고 안전한 진단·원본 응답 hash만 보존한다.
- 로컬 Cargo/rustc/build/test/DB/root/Docker/provider/운영은 금지한다. source-only 리뷰와 hosted
  실행 결과를 코디네이터가 함께 확인해야 이 부분 후보를 수용할 수 있다. 최종 C_R2 인수와는 구분한다.

두 assignment 모두 공통 직접 작업 문장과 최종 보고 형식을 따른다. 실제 running·pending0·notifier를
확인한 뒤 `45e28caf`를 새 두 scope의 bounded heartbeat로 교체하고 ledger에 회수·인수 상태를 분리한다.


### 보정 후보의 실제 hosted 검증 시작 — 2026-10-14

소스 후보는 `02a400e0a6f5f413c32ebd7a19f70af2e46dbfff`다. feature branch push와 remote SHA를
확인하고 해당 SHA의 기존 실행 수가 0임을 조회한 뒤 ci.yml을 한 번 dispatch했다.
[CI 38506113191](https://github.com/sihioov/Lagrange/actions/runs/38506113191)는 03:24:45 UTC 생성,
workflow_dispatch·해당 full SHA·in_progress를 확인했다. 이전 `779b3fa` run은 terminal failure로 보존한다.
I1a-F-CI collector `11a8f223-6f2d-4adb-89ab-86dbc917d2fd`는 03:25:39 UTC notified follow-up으로
시작한 running/pending0·workspace·parent·Luna/medium을 확인했다. 결과 반환 경계는 04:55 UTC다.
I1a-F-R과 I1a-F-CI만 감시하는 `70aea161`(2분·60회·05:26:14 UTC 만료)을 등록하고
기존 `45e28caf`를 삭제했다. 실제 결과 회수 전 인수하지 않으며 만료 전 미완료 작업을 재조정한다.
같은 daemon의 heartbeat는 daemon 중단을 보호하지 못한다. 이번 단계의 운영 권한 확대는 없다.

## I1a-F-R 인수와 격리 보정 I1a-F2 — 2026-10-14

03:30:35 UTC actual idle/pending0 뒤 [보정 리뷰](../../reviews/2026-10-14-r2-daily-wire-corrections-review.md)
309행·`72784303a8c92984b9d6e070838b4aaa5094318ee1b7594f8a90568796946546`를 회수했다.
R01/02/03/06의 소스 보정은 수용하지만 실행 인수는 대기한다. R04/R05는 CR-01~04 때문에 열린 상태다.
코디네이터가 date_range의 push 이전 검사, 복사 fixture의 공유 identity 비교, 새 test-only lint 지점과
출력 writer 매핑을 확인했다. 현재 `02a400e` CI는 진행 중이며 이 리뷰를 실제 실행 결과로 대신하지 않는다.

| Package | Complexity | Basis | Confidence | Reclassification or escalation signals |
| --- | --- | --- | --- | --- |
| I1a-F2 | intermediate | 네 finding의 두 파일 국소 보정과 고정 출력 제한 회귀 | high | fixed 계약을 바꿔야 하거나 legal proof fixture를 bounded 구성할 수 없으면 정확한 모순/한계를 보고 |

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
| --- | ---: | --- | --- | --- | --- | --- | --- | --- |
| I1a-F2 | 2 | intermediate | CR-01~04 보정 | 별도 Paseo worktree의 `contract.rs`, `tests/daily_price_only.rs` 두 파일만 | 리뷰 회수·정확한 `02a400e` 작업 사본 | Sol/medium; 앞선 Luna 구현·보정에 남은 R04/R05 문제로 모델 한 단계 상향 | 변경 파일·finding별 판정·guard/test matrix | source/rustfmt/diff만; 통합 뒤 exact-candidate hosted 검증 |

### I1a-F2 고정 실행 brief

- 코디네이터는 `02a400e0a6f5f413c32ebd7a19f70af2e46dbfff`에서 별도 Paseo worktree를 만든다.
  실제 workspace/path는 생성 응답 뒤 assignment에 고정한다. 기존 checkout과 진행 중 CI 입력은
  수정하지 않는다. 공유 원본 계획·리뷰는 read-only로 참고한다. worker는 commit/push하지 않는다.
- 사본에서 소유한 두 파일의 시작 hash는 `79370bc2…`/`ea08ac3a…`, D4는 `fa052903…`다.
  CR-01: date_range는 제안된 다음 길이를 checked arithmetic으로 검사한 뒤 추가하거나, 추가 후
  terminal break 전에 검사한다. 기존 정확한 370/371 assertion과 public LimitExceeded 의미를 유지한다.
- CR-02: 같은 calendar file identity의 모든 occurrence에 동일한 16/17-hop metadata를 적용한다.
  16-hop public 성공과 17-hop LimitExceeded, 10/11-page pair 및 nanosecond inversion은 유지한다.
  금지된 identity 불일치를 허용하도록 production 검사를 약화하지 않는다.
- CR-03: 새 private guard table의 복잡한 tuple/function-pointer 타입에 private alias/entry type을
  사용한다. singleton borrowed slice는 `std::slice::from_ref`로 바꾸되 값 이동이 필요한 호출은
  의미를 보존한다. 같은 두 파일의 관련 strict warning을 정리하며 blanket allowance는 금지한다.
- CR-04: policy/manifest/admission/normalization 네 fixed canonical output limit 각각을 이름과
  실제 값에 결합한 isolated writer exact/+1 회귀로 매핑한다. policy/admission의 legal public
  encoding은 상한에 도달할 수 없음을 정확히 기록하고 기존 public 성공 경로를 유지한다.
  proof encoder에는 bounded legal success/overflow pair를 추가한다. 고유 source FileKeys와
  적법한 날짜·schema·source commitment를 써서 다른 shape/hash/count 거부로 byte guard를 대신하지 않는다.
  한 번의 bounded fixture 구성·길이 산식으로 필요한 수를 정하고 반복 전체 serialize의 이차 비용을
  피한다. 실제 proof bytes가 한계 아래/위인지 확인하고 내부 commitment는 공개 constructor로 재계산한다.
  전체 nested positive 최대 fixture를 임의로 늘리지 않는다. size/guard 도달 경로를 보고한다.
- public wire/API·D4·7 primary golden·515-byte cash·오류 분류·기존 historical/I0a/module/Cargo 파일은
  변경하지 않는다. Raw/DB/운영 동작을 추가하지 않는다. 정확한 경계가 모순되면 임의 정책 없이 보고한다.
- 허용 검사는 source/hash/diff, bounded pure fixture 계산과 rustfmt edition2024/check뿐이다.
  Cargo/rustc/Clippy/test/build/SQL/DB/root/Docker/provider/운영/Git write/하위 위임은 금지한다.
  공통 최종 보고에 각 finding 수정 anchor, 정확한 source hash/행 수, guard→test 매핑, fixture 비용,
  미실행·미검증 및 추가 분해 필요 여부를 포함한다. written test를 PASS로 보고하지 않는다.

코디네이터는 현재 CI의 최종 결과·보고를 먼저 회수하고 worker가 idle인 것을 확인한 뒤 사본 diff를
원본에 통합한다. 범위 밖 변경이 없는지·기존 소스 hash를 확인하고 새 후보를 따로 commit/검증한다.
현재 CI를 재실행하거나 취소하지 않는다. 이후 독립 delta review와 hosted 결과를 함께 판정한다.


### 보정 리뷰 회수와 I1a-F2 착수 — 2026-10-14

03:30:35 UTC actual idle/pending0 후 보정 리뷰 309행·
`72784303a8c92984b9d6e070838b4aaa5094318ee1b7594f8a90568796946546`를 직접 읽고
hash를 확인했다. R01/02/03/06은 소스 수준 해소를 수용한다. date_range가 371번째 항목을
추가한 뒤 terminal break하는 경로와 calendar의 공유 identity 중 첫 occurrence만 16-hop으로
바꾸는 fixture를 직접 대조했다. 두 회귀의 실패 조건 및 test-only lint·출력 경계 누락을
CR-01~04 수정 대상으로 수용했다. 실행 결과나 전체 source 합격으로 해석하지 않는다.

기존 Luna 구현·보정 뒤 R04/R05가 남았으므로 모델을 한 단계 높여 Sol/medium에 국소 수정만 맡겼다.
Paseo 작업 사본 `/data/worktrees/3puw275b/r2-wire-f2`, `wks_183cc4ee7b8bd2a6`, branch `r2-wire-f2`는
정확한 `02a400e0a6f5f413c32ebd7a19f70af2e46dbfff` 기준이며 clean/시작 hash를 확인했다.
worker `54eb1c89-2cf3-41bb-94cb-25dfe7152ee0`의 03:34:41 UTC notified launch와 실제
running/pending0·workspace·parent·model/effort를 확인했다. 소유권은 사본의 두 Rust 파일뿐이며
원본 CI의 고정 입력은 바꾸지 않는다. 로컬 compiler/test/DB/운영 실행은 맡기지 않았다.

기존 CI collector는 계속 exact `02a400e`의 run만 관찰한다. 두 결과의 actual idle/final 회수 후
코디네이터가 원본 hash와 사본 diff를 대조해 통합하고 새 후보를 검증한다. `70aea161`을 삭제하고
두 소유 worker의 `6dd677e4`(2분·60회·05:35:22 UTC 만료)로 감시를 교체했다.

## I1a-F2 회수·실제 CI 진단 추가 보정 — 2026-10-14

I1a-F2는 03:39:43 UTC actual idle/pending0 후 전체 보고를 회수했다. 사본의 두 파일은
4185행 `b21aead5ebd2e5931cb811c469fa0954c932f0526ab265c9e439d4110ccca230`,
2386행 `ee5c70a1dde4ae94ec05f1d4119fc6151d0eb83401f24a6ae1f4c4eb6adfb73d`다.
CR-01~04 delta를 직접 읽었으며 25 integration+5 private test는 아직 작성 상태다.
legal proof의 53,428/53,429 key 구성과 public commitment 재계산, 네 fixed writer 매핑을 확인했다.
독립 검토·실행 성공은 아직 아니며 원본에는 통합하지 않았다.

CI collector는 03:39:58 UTC actual idle/pending0이며 [CI 보고](../../reviews/2026-10-14-r2-daily-wire-corrections-ci.md)
31행·`6fe75a87cb2b67d19544b933bc2e3b06d1cc1d7706e2b3325bb87730034357bd`를 회수했다.
`02a400e` run38506113191은 03:36:21 UTC terminal failure다. chart6 및 PG APPROVED/59 migrations는
통과했지만 세 collapsible-if와 두 test table type-complexity, test의 E0618, Python SIGTERM test 실패가
남았다. 신규 29개 테스트는 result row가 없고 실행 근거가 없다. PG 성공은 최종 후보나 미호출 helper,
0060/runtime 인수를 뜻하지 않는다. 원본·사본 모두 해당 fixture shadowing과 세 if를 아직 포함한다.

| Package | Complexity | Basis | Confidence | Reclassification or escalation signals |
| --- | --- | --- | --- | --- |
| I1a-F2-C | intermediate | 실제 E0618·세 lint 위치가 확정된 두 파일 국소 수정 | high | 추가 public 동작 변경 필요 또는 새 범위 밖 진단은 보고 후 재분해 |
| I1a-PY-D | simple | 단일 Python CI 실패의 로그·source 증거 추출 | high | 로그로 원인을 구분할 수 없으면 UNKNOWN과 필요한 후속을 명시; 구현은 하지 않음 |

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
| --- | ---: | --- | --- | --- | --- | --- | --- | --- |
| I1a-F2-C | 3 | intermediate | 추가 compiler/lint 진단 제거 | 격리 F2 작업 사본의 같은 두 파일 | F2 idle/final·CI 진단 회수 | 기존 Sol/medium | 정확한 delta/hash·보존 확인 | rustfmt/diff; 실제 compiler는 후속 hosted |
| I1a-PY-D | 3 | simple | Python 실패의 typed 증거·source 위치 회수 | 새 Python 진단 보고, `ci/02a400e/python-diagnostics/` | CI collector idle/final | 기존 Luna/medium | 실제 실패 조건·원인 판정 가능성·미확인 구분 | 기존 run의 python job 및 exact candidate source 읽기 |

### I1a-F2-C 고정 실행 brief

- 기존 Sol/medium `54eb1c89-2cf3-41bb-94cb-25dfe7152ee0`의 같은 사본·두 파일 notified follow-up.
  위 F2 hash를 시작 입력으로 확인한다. CR-01~04·D4/API/golden/30 written test 범위를 보존한다.
- contract의 UUID/commit lexical 검사 두 곳과 pair_value의 중첩 if를 의미가 같은 short-circuit
  조건으로 합친다. pair_value의 replace는 name 일치 시에만 실행돼야 한다. strict lane을 약화하지 않는다.
- test 마지막 verifier_commit_only_changes_admission_hash_and_stale_links_fail의 local fixture 변수가
  함수 fixture(false)를 가린다. local binding과 해당 참조만 명확히 rename하고 마지막 새 fixture 호출과
  기존 stale-hash/parent rejection assertion을 유지한다. helper/public 명칭 전체 rename은 금지한다.
- local read/rustfmt/diff만 허용한다. Cargo/rustc/Clippy/test/build/DB/root/Docker/provider/운영/Git write와
  하위 위임은 금지한다. 출력은 같은 파일 및 공통 최종 보고이고, coordinator가 원본 통합을 담당한다.

### I1a-PY-D 고정 실행 brief

- 기존 collector `11a8f223-6f2d-4adb-89ab-86dbc917d2fd`가 run38506113191의 Python job
  `115663895591` 로그만 read-only로 재조회한다. candidate는 exact `02a400e`다. workflow와
  `nt/live-node/tests/test_runtime.py`의 해당 SIGTERM test 및 직접 호출하는 fixture/runtime source를
  읽어 실제 assertion·예외 타입·exit·timeout·준비/종료 동기화 위치를 식별한다.
- 출력은 `docs/reviews/2026-10-14-r2-python-ci-diagnostics.md` 하나와 private leaf뿐이다.
  prior CI 보고/receipt를 바꾸지 않는다. safe diagnostic과 source anchor·log response hash만 저장한다.
  raw 로그·provider/account/credential 내용·운영 파일은 읽거나 기록하지 않는다.
- 불안정한 테스트인지 제품 결함인지 추정하지 않는다. 호출/실패 사실, source로 확인한 제약,
  미확인 원인을 분리한다. source 변경·재현 실행·pytest·프로세스 신호 전송·workflow action·DB/운영/
  Git write·하위 위임은 금지한다. 20분 또는 04:10 UTC 중 먼저 도달하면 결과/INCOMPLETE를 반환한다.
  실제 수정·추가 검증은 결과를 검토한 코디네이터가 별도 범위로 정한다.

## I1a-F2-C 통합·독립 검토와 Python 준비 대기 보정 — 2026-10-14

03:43:54 UTC actual idle/pending0 후 F2-C를 회수했다. 최종 사본은 contract4183행
`f086c065b903a00a022eeef3b37daf3b021691f0fa2fc525b17be1a7ed4ea523`, test2386행
`11a8255d60d5c4ff1e16f136e3246adf046fc7743c8248b7eb7de755e1d3cd74`다. 코디네이터가 네 추가
진단의 소스 보정을 읽고, 원본 hash·사본 base·두 파일만의 delta를 확인해 그대로 원본에 통합했다.
이전 CI turn의 final도 이미 회수됐으며 새 Python 진단은 exact candidate의 별도 source 읽기였다.
30개 written test와 static check는 실제 compiler/Clippy/test 성공을 대신하지 않는다.

Python 진단은 03:45:29 UTC actual idle/pending0 후 [23행 보고](../../reviews/2026-10-14-r2-python-ci-diagnostics.md)
`4ba38a0b24907da1a33cf23a8d01c62f457eb3184d66d43be48bab721a58b7de`를 회수했다.
SIGTERM 전에 wait_for_cycle의 200×0.005초 대기가 실패했고 child exit 정보는 없었다.
원인이 startup 지연인지 child 오류인지는 미확인이다. 아래 test-only 보정은 이를 구분할 수 있는
bounded 대기 계약을 만들며, 기존 운영 runtime 결함이나 flaky 원인 확정을 주장하지 않는다.

| Package | Complexity | Basis | Confidence | Reclassification or escalation signals |
| --- | --- | --- | --- | --- |
| I1a-F2-R | intermediate | CR-01~04와 실제 compiler/lint 진단의 고정 두 파일 delta 검토 | high | source 모순·경계 위반 발견 시 구체적 finding으로 반환 |
| I1a-PY-F | intermediate | 확정된 subprocess readiness 대기·오류 관측의 test-only 보정 | high | production runtime 수정·새 실행 권한이 필요하면 임의 확대 없이 보고 |

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
| --- | ---: | --- | --- | --- | --- | --- | --- | --- |
| I1a-F2-R | 4 | intermediate | 통합 Rust 보정의 독립 검토 | `docs/reviews/2026-10-14-r2-daily-wire-final-review.md` 하나 | F2-C idle/final·위 hash 통합 | 기존 Sol/high reviewer | source-only finding disposition | source/diff/hash·rustfmt check; 실행 추정 금지 |
| I1a-PY-F | 4 | intermediate | Python SIGTERM 테스트의 준비 대기 개선 | `nt/live-node/tests/test_runtime.py` 하나 | 실제 Python 실패 진단 회수 | 새 Luna/max; 고정 명세 구현 | test-only delta·mock 경계 회귀 | Python compile/AST·diff만; pytest 실행은 hosted 후보 |

### I1a-F2-R 고정 실행 brief

- workspace는 원본이다. 위 두 hash와 D4950행 `fa052903…`, 이전 보정 리뷰309행 `72784303…`,
  actual CI 보고31행 `6fe75a87…`를 고정한다. `02a400e` 대비 합친 F2/F2-C delta를 검토한다.
- CR-01~04의 수정·guard 호출·fixture legality/비용, 세 short-circuit if와 fixture shadow 해소,
  기존 API/golden/cash·R01/02/03/06 보존을 판정한다. code/test가 컴파일·통과했다고 추정하지 않는다.
  출력은 위 report 하나이며 source/hash/diff/line/rustfmt check만 허용한다. local compiler/test/DB/
  root/Docker/provider/운영/Git write/하위 위임은 금지한다. Python 범위는 별도 담당이다.

### I1a-PY-F 고정 실행 brief

- 원본 checkout의 위 test 파일 하나만 수정한다. credential-free/dry-run 테스트 계약을 유지한다.
  production runtime/CLI/config·workflow·의존성·문서·Rust 파일은 변경하지 않는다.
- 기존 thread 기반 wait_for_cycle 호출과 의미는 유지한다. 실패한 SIGTERM subprocess 테스트에는
  별도 readiness helper를 추가해 monotonic 기준 10초 deadline과 짧은 polling interval을 사용한다.
  매 poll에서 child.poll()을 확인하고 child가 종료됐으면 즉시 typed phase·returncode로 실패한다.
  살아 있는 child가 유효한 cycle>=1 상태를 발행해야 다음 SIGTERM 검증으로 넘어간다.
  deadline까지 준비되지 않으면 deadline/returncode/마지막으로 관측한 cycle의 안전한 숫자 정보로
  명확히 실패한다. 임의 stderr/stdout·status 원문·provider/account 내용은 실패 메시지에 넣지 않는다.
- 준비 이후 terminate→exit0→lock 부재→STOPPED status/stdout assertions와 finally kill/wait cleanup을
  그대로 유지한다. 무한 재시도, 실패 skip/xfail, 종료 검증 약화, 실제 provider 접근은 금지한다.
- monkeypatch/fake clock·fake child.poll·fake read_status로 결정론적인 회귀를 작성한다.
  이전 1초보다 늦지만 deadline 안인 준비는 성공, deadline 전 조기 종료는 즉시 실패,
  준비가 끝내 없으면 정확한 bounded 실패를 확인한다. 회귀 자체가 새 real subprocess나 sleep을
  수행하지 않게 한다. 원인 미확인을 숨기지 말고 개선된 진단/준비 계약의 범위를 보고한다.
- 허용 검사는 source read, ast.parse/compile(파일 실행·pyc 생성 없음), diff/whitespace뿐이다.
  pytest/Python module runtime·프로세스 생성/신호·Cargo/build/DB/root/Docker/provider/운영/Git write/
  하위 위임은 금지한다. 모순은 임의로 채우지 않는다. 공통 최종 보고와 정확한 파일 hash/라인,
  추가 회귀 이름·미실행 한계를 반환한다. coordinator의 독립 review 후 새 exact 후보 CI에서 실행한다.


### I1a-F2-R 수용·I1a-PY-F 실행 — 2026-10-14

I1a-F2-R은 03:53:49 UTC actual idle/pending0 후 226행 보고 `e87f054c…`를 회수했다.
CR-01~04와 추가 진단의 source-level 해소·신규 finding 없음만 수용한다. 최종 두 Rust hash는
위 f086c065/11a8255d 그대로이며 30개 테스트는 hosted 실행 전이다. Python I1a-PY-F는
Luna/max `ba55a077-2880-408e-9424-29f476ebdd7f`의 03:53:12 UTC notified launch와 실제
running/pending0를 확인했다. 기존 두 Rust 파일 및 D4는 해당 작업의 소유 범위가 아니다.
`f40a8ab8`(2분·60회·05:53:40 UTC 만료)로 기존 감시를 교체했으며 actual idle/final·코디네이터
검토 뒤 새 정확한 source 후보를 hosted CI로 검증한다. 이전 CI run은 재실행하지 않는다.


## 최종 보정 후보 검증 package — 2026-10-14

I1a-PY-F는 03:57:11 UTC actual idle/pending0 후 회수됐다. 한 test 파일479행/hash
`e2121f61d3be269b4b8b15313c6c0e8c4fe8ed34a218bac1c908cf72ae3dfb97`의 제한된 소스 보정과
AST·diff 검토를 수용한다. 기존 함수·종료/cleanup 보존 및 mock 회귀 3개를 직접 확인했다.
Rust는 위 f086c065/11a8255d, D4는 fa052903 그대로다. 로컬 테스트 성공은 없으며
다음은 이 세 파일을 포함한 정확한 후보의 hosted 검증이다.

| Package | Complexity | Basis | Confidence | Reclassification or escalation signals |
| --- | --- | --- | --- | --- |
| I1a-F3-CI | simple | 새 exact 후보의 기존 run에서 정해진 job·test·diagnostic 증거 회수 | high | 새 오류 분석·검증 범위 확대가 필요하면 사실과 UNKNOWN을 분리해 보고 |

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
| --- | ---: | --- | --- | --- | --- | --- | --- | --- |
| I1a-F3-CI | 5 | simple | 최종 보정 후보의 실제 hosted 결과 회수 | 새 final CI 보고와 candidate private leaf | F2-R/PY-F idle/final·코디네이터 소스 검토·정확한 후보 push/1회 dispatch | 기존 Luna/medium collector | 전체 job/step 결과·신규30 Rust와 Python 증거·실패 진단·한계 | read-only gh run/job/log 및 exact source/receipt 대조 |

### I1a-F3-CI 고정 실행 brief

- 실행은 코디네이터가 push·remote SHA·exact SHA 기존 run 조회 후 한 번 dispatch한 ID만 관찰한다.
  full candidate SHA/run ID는 notified assignment에 고정한다. 재실행·dispatch·cancel을 하지 않는다.
- 출력은 `docs/reviews/2026-10-14-r2-daily-wire-final-ci.md`와 private candidate leaf다.
  이전 report/receipt/source/roadmap/Git은 쓰지 않는다. 원시 로그는 저장하지 않고 선택된 증거·hash만 남긴다.
- 모든 7개 checkout SHA, job/step terminal state 및 required를 확인한다. 30개 Rust test 목록은
  `target/r2-daily-beta/wp-4-integration/final-wire-test-inventory.json`과 exact source에 대조한다.
  기존 golden/cash, private guard, public proof/seal 한계와 날짜/copy 보정의 실제 result row를 확인한다.
  Chart6, workspace 합계·fail/ignored, 역사 v2/v3의 발견 여부, strict Clippy와 cleanup을 분리한다.
- Python은 SIGTERM case와 새 mock 회귀 3개의 발견·실행 근거 수준을 정직하게 기록한다.
  pytest -q의 파일별 progress/전체 합계가 개별 node PASS를 직접 보여주지 않으면 그 한계를 남긴다.
  Web/OpenAPI/E2E 및 PG code_commit·59 migrations/checksum·0039 guard·skip marker·DB-gated
  단계의 결과를 회수한다. 미호출 direct-login helper/0060/runtime 성공을 주장하지 않는다.
- 오류가 있으면 typed compiler/lint/test 진단과 job/step/exit/source 위치를 남긴다. 출력을 보지 못한
  테스트를 PASS로 추정하지 않고 NOT_EXECUTED/UNKNOWN으로 구분한다. 기존 10 Python skips 등도 보존한다.
- 90분 또는 05:35 UTC 중 먼저 도달하면 수집된 근거와 INCOMPLETE를 반환한다. 코디네이터가
  terminal까지 관찰 책임을 유지한다. local Cargo/test/build/DB/root/Docker/provider/운영/Git write와
  하위 위임은 금지한다. 새 회귀를 로컬 실행해 빈 로그를 보충하지 않는다.


### I1a-F3-CI launch 확인 — 2026-10-14

검증 source 후보는 `6429d331bef1bd201c33d49047139587a55a1435`다. `excited-dolphin`에
fast-forward push하고 remote SHA를 확인했다. exact SHA 기존 CI 조회가 비어 있는 것을 확인한 뒤
ci.yml을 한 번 dispatch했다. [run38509087712](https://github.com/sihioov/Lagrange/actions/runs/38509087712)는
03:59:36 UTC 생성·workflow_dispatch·해당 full SHA·queued를 확인했다. 앞선 두 failed run은
재실행하거나 취소하지 않았다. 이후 문서 커밋을 이 검증 source로 바꾸지 않는다.

Luna/medium collector `11a8f223-6f2d-4adb-89ab-86dbc917d2fd`의 I1a-F3-CI notified follow-up은
04:00:36 UTC 실제 running/pending0·workspace/parent/model/effort를 확인했다. 90분 또는 05:35 UTC
중 먼저 도달하는 경계까지 실제 결과를 회수하며 미완료라면 코디네이터가 관찰 책임을 유지한다.
`05c2aada`(2분·60회·06:01:14 UTC 만료)로 감시를 교체하고 회수된 review/Python worker 감시는
삭제했다. worker는 이 run의 read-only 증거만 수집하고 raw 로그나 provider 내용을 저장하지 않는다.
30개 Rust test result, Python 파일/전체 suite와 개별 node 근거의 차이, required·Clippy·chart·PG·Web을
구분한다. I0/0060·운영·Owner 인수는 이 부분 후보의 CI로 완료 처리하지 않는다.


## I1a-L 국소 lint 보정·재검증 — 2026-10-14

I1a-F3-CI actual idle/final을 회수했고 `6429d33`의 30개 Rust test는 직접 ok, Clippy 외 6개 실행
job은 성공했다. 남은 test helper913:17의 중첩 조건 한 곳을 코디네이터가 같은 short-circuit
let-chain으로 결합했다. 이는 작은 1-file 수정 직접 처리 규칙에 따른 통합 보정이다. D4·제품 contract·
Python·test assertions는 바꾸지 않았다. 새 test2385행 SHA-256은
`1596ecbe2b673264fdec2f41605e043f3d7fb79ea54463ac5075ad9da3cd0fdd`이며 rustfmt/diff만 확인했다.
기존 full report/receipt는 보존하고 새 exact source 후보를 전체 hosted CI에서 검증한다.

| Package | Complexity | Basis | Confidence | Reclassification or escalation signals |
| --- | --- | --- | --- | --- |
| I1a-L-CI | simple | 진단된 한 곳의 의미 보존 lint 보정 후 새 정확한 후보 결과 회수 | high | 새 진단·누락 로그는 사실과 UNKNOWN을 구분해 보고; 재실행·범위 확대하지 않음 |

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
| --- | ---: | --- | --- | --- | --- | --- | --- | --- |
| I1a-L-CI | 6 | simple | 새 보정 후보의 strict lane 포함 전체 결과 회수 | `docs/reviews/2026-10-14-r2-daily-wire-lint-ci.md`, 새 candidate private leaf | 이전 collector idle/final·위 한 곳 수정·push/1회 dispatch | 기존 Luna/medium collector | candidate/run identity·7개 job/required·30개 회귀·정확한 한계 | I1a-F3-CI와 같은 read-only evidence 검증 |

I1a-L-CI notified brief는 새 full SHA/run ID를 고정한다. 기존 `6429d33` 보고/receipt를 변경하지 않는다.
30개 이름은 `target/r2-daily-beta/wp-4-integration/lint-correction-test-inventory.json`과 대조한다.
Python suite/node 근거 차이, historical name filter의 한계, PG/helper/0060/runtime 구분은 이전과 같다.
출력은 새 보고/leaf뿐이다. raw 로그 저장·로컬 테스트/컴파일/DB/운영·Git write·하위 위임 금지는
유지한다. 90분 또는 06:10 UTC 중 먼저 도달하면 INCOMPLETE를 반환하고 코디네이터가 감시를 유지한다.


### I1a-L-CI 새 후보 launch — 2026-10-14

lint 보정 source 후보는 `d6bb23a6c516c8a9ebffbd93229c0b92190db48b`다. feature branch의
fast-forward push·remote SHA와 exact SHA 기존 CI 부재를 확인한 뒤 ci.yml을 한 번 dispatch했다.
[run38511544759](https://github.com/sihioov/Lagrange/actions/runs/38511544759)는 04:28:16 UTC 생성,
workflow_dispatch·정확한 SHA·in_progress를 확인했다. dispatch 직후 첫 조회는 전파 전 빈 목록이었고
다음 읽기 조회에서 한 run을 찾았다. 두 번째 dispatch나 기존 failed run 재실행은 없었다.

기존 collector `11a8f223-6f2d-4adb-89ab-86dbc917d2fd`에 I1a-L-CI notified follow-up을 맡겼고
04:29:09 UTC actual running/pending0·workspace/parent/Luna medium을 확인했다. 반환 경계는
90분 또는 06:10 UTC 중 이른 시점이다. 이전 report·receipt는 보존하고 새 보고/leaf에만 기록한다.
`7537b40b`(2분·60회·06:29:39 UTC 만료)로 감시를 교체하고 `05c2aada`를 삭제했다.
미완료면 코디네이터가 감시 책임을 유지한다. 같은 daemon heartbeat의 중단 한계는 그대로다.
이 후속 문서 커밋을 source 검증 대상으로 바꾸지 않는다. R2 운영 권한 확대나 로컬 테스트 실행은 없다.


## I0a-L 공용 harness lint 보정·검증 — 2026-10-14

I1a-L-CI actual idle/pending0/final을 회수했다. 정확한 `d6bb23a`의30개 Rust 회귀와6개 실행 job은
성공했으며 I0a의 선택적 `daily_input_admitter_pool` 미사용 경고로 Clippy/required가 실패했다.
코디네이터가 작은1-file 수정 규칙에 따라 메서드에만 이유를 설명한 `allow(dead_code)`를 추가했다.
각 integration binary가 공용 모듈을 별도 컴파일하는 구조와 기존 role-specific helper 관행을
확인했다. 본문·역할 SQL·pool/GUC assertion과 wire/Python 소스는 그대로다. 새 harness401행 hash는
`242977cf561d54c88889feeb695c8de5b90a64910fd3fcbfde346a17a06e68fa`다.
정확한 delta 대조·rustfmt/diff만 통과했으며 실제 Clippy는 새 후보에서 검증한다.

| Package | Complexity | Basis | Confidence | Reclassification or escalation signals |
| --- | --- | --- | --- | --- |
| I0a-L-CI | simple | 한 메서드 lint 범위 보정 후 고정 후보의 기존 run·정해진30개 테스트와 strict 결과 회수 | high | 새 진단/누락 근거는 UNKNOWN으로 구분해 반환; 원인 추정이나 실행 범위 확대 금지 |

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
| --- | ---: | --- | --- | --- | --- | --- | --- | --- |
| I0a-L-CI | 7 | simple | 공용 harness lint 보정 후보의 전체 hosted 결과 회수 | `docs/reviews/2026-10-14-r2-harness-lint-ci.md`, 새 candidate private leaf | 기존 collector idle/final·국소 수정·push/기존 run 확인/1회 dispatch | 기존 Luna/medium collector | exact 후보/run·7개 job/required·30개 회귀·명확한 검증 한계 | read-only gh run/job/log 및 inventory·source·receipt 대조 |

### I0a-L-CI 고정 실행 brief

- notified prompt에서 full SHA와 run ID를 고정한다. 이전 run38511544759와 보고/receipt는 보존한다.
  새 dispatch/rerun/cancel이나 다른 run의 성공을 섞지 않는다.
- 출력은 위 새 보고와 `target/r2-daily-beta/ci/<candidate-short-sha>/`뿐이다. private0700/0600,
  raw 로그 미보존, 선택된 결과와 원본 응답 byte count/hash만 기록한다.
- `target/r2-daily-beta/wp-4-integration/harness-lint-test-inventory.json`의30개 Rust 이름과
  exact source를 대조하고 각 직접 result row를 확인한다. chart6·workspace result 집계와
  고유 테스트 수를 구분한다. historical V2/V3는 실제 발견한 직접 row만 기록한다.
- 모든7개 checkout SHA·job/step terminal state·required·strict Clippy를 회수한다. Python485/10
  수치는 과거 결과이므로 새 결과로 다시 확인한다. quiet 로그의 개별 node 부재를 성공으로 메꾸지 않는다.
  Web/OpenAPI/E2E와 PG code_commit/59 migrations/checksum/0039 guard/skip markers/cleanup을 회수한다.
- 오류가 있으면 typed 위치·명령·exit를 남긴다. helper의 실제 직접 로그인·GUC·max2 pool/0060/
  runtime·Owner·최종C_R2 인수는 해당 실행 근거 없이는 미검증으로 유지한다.
- 90분 또는06:45 UTC 중 먼저 도달하면 INCOMPLETE와 현재 증거를 반환한다. 코디네이터가
  terminal까지 관찰 책임을 유지한다. 로컬 Cargo/rustc/Clippy/test/build/DB/root/Docker/provider/
  운영·Git write·하위 위임은 금지한다. 별도 로컬 테스트로 로그 공백을 채우지 않는다.
- 기존 worker는 실제 idle/pending0 확인 후 notified follow-up을 보낸다. actual startup과 scope를
  ledger에 기록하고 새 bounded heartbeat로 감시한다. 같은 daemon의 중단 한계는 유지한다.


### I0a-L-CI 정확한 후보 검증 시작 — 2026-10-14

검증 소스 후보는 `929fcd68eb99be5ec18bbbfbb585e6cc17655fee`다. feature branch의 fast-forward
push·remote SHA와 exact SHA 기존 실행 부재를 확인한 뒤 ci.yml을 한 번 dispatch했다.
[run38514204062](https://github.com/sihioov/Lagrange/actions/runs/38514204062)는 04:58:56 UTC 생성,
workflow_dispatch·해당 full SHA·in_progress를 확인했다. 기존 실패 run은 재실행/취소하지 않았다.
private launch receipt는 `target/r2-daily-beta/wp-4-integration/harness-lint-ci-launch.json`이다.

기존 Luna/medium collector `11a8f223-6f2d-4adb-89ab-86dbc917d2fd`에 I0a-L-CI notified follow-up을
맡겼고 05:00:12 UTC actual running/pending0·workspace/parent/model/effort를 확인했다.
새 inventory hash는 `71ddf8fd429cda222a0b0c69638d9832d25fc1f75955b143fdbbca5b82c97177`다.
출력은 새 harness CI 보고와 candidate leaf뿐이다. 90분 또는06:45 UTC 중 먼저 도달하는 경계는
06:30:12 UTC이며 미완료면 코디네이터가 관찰 책임을 유지한다. 새 감시`df0714ed`는2분·최대60회,
07:00:41 UTC 만료다. 같은 daemon의 중단 한계는 유지한다.

소스 후보와 이 후속 문서 커밋을 구분한다. 30개 Rust·strict Clippy/required·Python suite/node 근거·
PG/helper/0060/runtime을 분리해 판정한다. 로컬 테스트나 추가 운영 조치 권한은 부여하지 않았다.

## 929fcd6 부분 인수와 다음 소스 패키지 — 2026-10-14

[I0a-L-CI 보고](../../reviews/2026-10-14-r2-harness-lint-ci.md)와 private receipt를 회수했다.
정확한 `929fcd68eb99be5ec18bbbfbb585e6cc17655fee`의 run38514204062는 05:17:46 UTC
success이며 7개 실행 job과 required 모두 성공했다. 코디네이터는 7개 checkout, 현재 파일과
candidate blob/hash, 30개 테스트 inventory와 직접 ok row, chart6, 이전 실패 근거 보존을 대조했다.
collector는 05:19:34 UTC actual idle/pending0/full final 상태로 회수했고 감시 df0714ed는 종료했다.
coordinator receipt는 `target/r2-daily-beta/wp-4-integration/harness-ci-recovery.json`이다.

I1a의 순수 **unapproved** wire와 I0a의 소스/hosted 검증을 부분 인수한다. Python suite는
485 pass/10 skip/7 warnings이며 개별 readiness/SIGTERM node 근거는 여전히 없다. PG는59개
migration까지 통과했으며 미호출 helper의 직접 login/GUC/pool,0060, 실제 입력·운영·Owner·최종
C_R2 인수가 아니다. `d6bb23a`, `6429d33`와 앞선 실패 기록은 그대로 보존한다.

### 다음 wave의 분해와 판정

두 패키지는 기존 U2 소스 구현 권한 안에서 병렬로 실행한다. 아래 명세와 소유 파일을 고정한 뒤
새 Paseo worker를 생성한다. 운영 설정 읽기/쓰기, secret 생성, migration 실행, provider 호출,
local Cargo/rustc/Clippy/test/build/DB/root/Docker 실행은 부여하지 않는다.

| Package | Complexity | Basis | Confidence | Reclassification / escalation |
| --- | --- | --- | --- | --- |
| I0b | intermediate | D5 A5의 확정 역할·secret 배선을 기존 map/validator에 연결; 경계와 UID·mode가 고정됨 | high | 기존 계약과 충돌하거나 allowlist 밖 변경이 필요하면 보고; 반복 검증 실패 시 Luna→Sol 단계 상향 |
| I1b-S1 | intermediate | D5 A6와 D4의 exact-line 규칙에 맞춘 bounded read-only sibling; 아래 API/중복/경로 규칙을 고정함 | high | descriptor/lock 경계가 충돌하거나 필요한 인터페이스가 빠지면 임의 대안 없이 보고; 반복 실패 시 단계 상향 |

| Package | Wave | Objective / ownership | Dependency | Worker | Deliverable / validation |
| --- | --- | --- | --- | --- | --- |
| I0b | 8 | 아래9개 DB bootstrap·secret·Compose·validator·fixture 파일 | 929fcd6 부분 인수, D5 A5, I0a | 새 Luna/max; 확정 구현을 저비용부터 시작 | source delta·hash·권한/secret map; bash-n·정적 대조, 이후 독립 리뷰와 hosted 실제-role 검증 |
| I1b-S1 | 8 | storage.rs와 새 daily_price_only_storage.rs 두 파일 | 929fcd6 부분 인수, D5 A6, D4 exact-line | 새 Luna/max; 확정 구현 | read-only manifest evidence API·경계 회귀; rustfmt/diff, 이후 독립 리뷰와 hosted 테스트 |

의존 관계는 `929fcd6 → (I0b || I1b-S1) → 각 결과 회수/독립 검토 → 새 exact 후보 CI`다.
I0b 이후에도0060의 actual-role/권한 거부/rollback/race 및 no-SKIP 연결이 완료돼야 첫0060을
검증 환경에서 실행할 수 있다. I1b-S1은 전체 CommittedRawSnapshot이 아니다. batch.json과
provider file descriptor/hash, copy chain, deterministic normalization, artifact/admission은
S1 검토 후 다음 bounded source brief로 연결한다. 기존 wire/D4와 역사 v2/v3 동작은 바꾸지 않는다.

### I0b 고정 실행 brief

소유 파일은 다음9개뿐이다. 이 중 self-test.sh의 수정은 새8번째 credential 및 해당 runtime
inventory의 fixture/기대값에 한정한 D5 allowlist의 명시적 확장이다.

- `deploy/db/bootstrap-roles.sh`
- `scripts/ops/provision-db-secrets.sh`
- `deploy/secrets/provision-runtime-secrets.sh`
- `deploy/secrets/runtime-static-check.sh`
- `scripts/ops/validate-production-config.sh`
- `deploy/compose/compose.yml`
- `deploy/compose/.env.example`
- `deploy/db/integration-validation/validate.sh`
- `scripts/ops/self-test.sh`

1. 새 역할은 `daily_input_admitter`, credential은 `db_daily_input_admitter_password`, bootstrap
   env는 `DB_DAILY_INPUT_ADMITTER_PASSWORD_FILE`이다. LOGIN/NOINHERIT/NOSUPERUSER/
   NOCREATEDB/NOCREATEROLE/NOREPLICATION/NOBYPASSRLS를 고정한다. 기존 역할의 동작을
   불필요하게 바꾸지 않는다. 기존 admitter 속성·incoming/outgoing membership와 app/worker의
   transitive migration_owner/admitter 도달 경로는 변경 **전** 검사해 거부한다. 순환 경로를
   종료하는 UNION closure를 사용한다. 전체 bootstrap SQL은 한 transaction에서 처리하여
   거부 시 password/권한의 부분 변경을 남기지 않는다. 기존 잘못된 역할을 ALTER로 정상화하지 않는다.
2. target DB CONNECT/public USAGE만 추가한다. PUBLIC CREATE 제거와 migration_owner CREATE는
   기존 규칙을 유지한다. 기존 admitter의 직접/유효 CREATE 권한은 조용히 revoke하지 않고
   거부하며, 완료 전 DB/schema effective CREATE 부재와 CONNECT/USAGE를 확인한다. 비밀번호는
   file→private SQL/stdin 경로만 사용하고 argv/log/env-value 입력 인터페이스를 추가하지 않는다.
3. source secret inventory는 기존7개+새1개다. 기존 all-or-nothing 생성·비덮어쓰기·newline/
   symlink/format/distinctness 규칙을 보존한다. 기존7개 설치의 보완 provisioning은 운영 후속이며
   소스 작업 중 실행하거나 자동 회전 경로를 추가하지 않는다.
4. bootstrap projection은999:999/0400. 새 admitter projection은
   `owner-beta-daily-admitter/db_daily_input_admitter_password`,10003:10003/0440이며
   serving-prereqs/release에만 미리 준비한다. infrastructure/backfill에는 bootstrap copy만 추가,
   range-raw/recovery의 DB-free 경계는 유지한다. validator의 required source/runtime_specs,
   static inventory/count와 fixture를 같은 규칙으로 맞춘다. producer10002·API·worker에 새 credential을
   주지 않는다. `.env.example`에는 file 경로 설명만 추가하고 비밀번호 값이나 사용하지 않는 env를 만들지 않는다.
5. Compose는 db-role-bootstrap의 env/secret와 top-level bootstrap secret source만 연결한다.
   새 admitter/producer 서비스·profile·mount·network·timer는 I6 이후 범위로 남긴다.
6. official validator의 bootstrap env, 두 role→secret map, 생성 credential 목록, full/staged
   direct-login 목록을 확장한다. 새 역할의 session_user/current_user 직접 login과 제한된
   catalog/privilege 검사를 existing isolated cluster 경로에 넣는다. migration 목록은59로 유지하고
   존재하지 않는0060/test target을 참조하거나 skip으로 우회하지 않는다. I0a 네 파일은 건드리지 않는다.
7. 코드/fixture를 작성하고 bash-n·정적 inventory/SQL 대조만 수행한다. provisioner/validator/self-test를
   실행하지 않는다. negative attribute/membership/CREATE, 실제 direct-login/GUC 및 credential 노출
   거부의 남은 검증은 보고에 명시한다. 실제 hosted 검증과 독립 review 전 I0 완료를 선언하지 않는다.

### I1b-S1 고정 실행 brief

소유 파일은 `crates/market-data/src/storage.rs`와 새
`crates/market-data/tests/daily_price_only_storage.rs`뿐이다. 기존 writer/read_committed_manifest/
orphan recovery 경로를 완화하지 않는 sibling을 추가한다. 새 daily module 파일이나 mod/lib 변경은 없다.

```rust
pub struct CommittedManifestReadLimits {
    pub max_manifest_bytes: usize,
    pub max_records: usize,
}
RawStore::read_committed_manifest_evidence(
    &self, provider: &str, market: &str, trusted_raw_owner_uid: u32,
    limits: CommittedManifestReadLimits,
) -> Result<Vec<CommittedManifestEvidenceRecord>, StoreError>
```

- `CommittedManifestEvidenceRecord`의 private fields는 parsed ManifestEntry, 원본 record bytes,
  ContentHash, 1-based 물리 line number다. `entry()`, `line_bytes()`, `line_sha256()`, `line_number()`의
  읽기 전용 getter를 제공한다. 승인 capability가 아닌 manifest-only evidence이며 provider bytes/
  batch.json 검증을 했다고 표현하지 않는다. Debug/error에 record bytes나 free-form metadata를 출력하지 않는다.
- bytes는 D4에 따라 마지막 LF만 제외한다. CR·공백·JSON key order를 보존한다. 기존 parser와 같이
  blank line은 무시, 완전한 마지막 unterminated JSON은 포함, serde EOF인 불완전한 마지막 tail은
  제외, 다른 malformed/scope/name 오류는 거부한다. 같은 batch의 동일 parsed entry 중복은 기존처럼
  첫 physical record를 반환하며 다른 metadata는 거부한다. max_records는 dedup **전**의 완전한
  nonblank record를 센다. returned order는 첫 등장 순서다.
- caller가 두 한도를 명시한다(Default 없음).0과 byte bound의 checked +1 overflow는 typed error,
  원본 파일 byte bound와 record count 초과도 typed StoreError로 반환한다. max+1까지만 읽고
  검사하여 한도 전 무제한 read/allocation을 하지 않는다. policy의 실제 배포 scan 값은 여기서 정하지 않는다.
- sibling은 RawStore root에 absolute·ParentDir 없는 경로를 요구한다. filesystem root부터 각
  directory component를 descriptor-relative NOFOLLOW/DIRECTORY/CLOEXEC로 열어 root alias를
  따라가지 않는다. Raw root 및 그 아래 manifest control dirs, lock, manifest의 소유자는 explicit
  trusted_raw_owner_uid이고 group/other write가 없어야 한다. Raw 밖 ancestor의 owner/mode를
  Raw owner라고 주장하지 않는다. regular leaf/type·opened inode/path binding을 검사한다.
- 기존 commit.lock을 read-only로 열어 shared lock을 취득한다. lock inode를 open/lock/read 전후
  대조하고 descriptor에서 manifest를 읽는다. 새 sibling은 manifest redirect도 fail closed한다.
  no mkdir/create/chmod/chown/ACL/reconciliation/store/commit. root caller도 owner 확인을 우회하지
  않고 reader euid와 trusted owner가 같다고 요구하지 않는다. OS permission denied는 그대로 실패하며
  unsafe fallback이 없다. non-Unix는 typed unsupported/unsafe-path 오류로 닫는다.
- public fixture에는 LF/CRLF/공백/unterminated/torn tail/중복·충돌, exact/+1 byte·record 한도,
  zero/overflow limits, missing tree/lock, symlink/redirect, UID/mode/type 위반과 no-write를 포함한다.
  기존 private descriptor hook을 sibling의 lock/manifest 교체 회귀에 한정해 재사용할 수 있다.
  replacement, negative-owner와 no-write를 기계적으로 확인하되 실제 다른 UID의 OS ACL 테스트와
  운영 권한 확인은 수행하지 않았다고 구분한다. legacy tests는 삭제/약화하지 않는다.
- scoped rustfmt edition2024 및 diff/정적 검사만 한다. Rust tests/Cargo/rustc·DB·provider·root 실행은
  코디네이터의 후속 exact-candidate hosted 검증으로 남긴다. D4와30개 기존 wire 테스트를 수정하지 않는다.

### 회수와 다음 검증 gate

두 brief에 native delegation 금지 문장과 파일/행·편차·검사 결과·미해결/미검증·추가 분해 필요 여부를
반환하도록 명시한다. current profiles 조회 후 Luna/max를 적용하고 actual startup/workspace/parent/
effort와 notifyOnFinish를 확인한다. ledger와 bounded heartbeat를 갱신한다. actual idle/pending0와
실제 full final을 확보하기 전에 인수하거나 실행 중 worker를 다른 범위로 돌리지 않는다.
독립 source review와 변경에 적합한 hosted 실제 검증 뒤에만 다음 package를 연결한다. 구조 변화의
diagram evidence/render는 I6 통합에서 수행하고 main 병합 전 완료한다. 새 런타임/운영 승인은 없다.


### I0b / I1b-S1 actual startup — 2026-10-14

명세와 부분 CI 인수는 `0a98b9f`에 커밋했다. 현재 profiles의 Astra/high는 코디네이터 전용이고,
확정 구현의 비용 기준은 저장소 지침에 따라 Luna/max를 선택했다. MCP notified 생성 후 아래
실제 startup과 parent/workspace/model/effort, pending0을 각각 확인했다.

| Package | Agent | Actual start UTC | Model / effort | Writable scope |
| --- | --- | --- | --- | --- |
| I0b | `fd32e206-9248-41cd-a86b-de07a33ea464` | 05:40:08 | codex/gpt-6-luna / max | 위9개 DB/secret/validator/fixture 파일 |
| I1b-S1 | `14d3d7c8-892a-4434-baa6-29de925837cb` | 05:40:35 | codex/gpt-6-luna / max | storage.rs와 새 storage fixture |

두 worker의 `notifyOnFinish=true`를 등록했고 `owned-workers.json`과 private wave8 brief/startup
receipt를 남겼다. 새 감시 `ce11f12d`는2분 간격·최대60회·07:41:05 UTC 만료다. 만료 전에 실제
상태를 회수해 갱신/인계하며 같은 daemon의 중단을 보호한다고 주장하지 않는다. 소스 구현은
실행 중이며 아직 결과/리뷰/실제 테스트를 인수하지 않았다. 로컬/운영 실행 권한을 추가하지 않았다.


## Wave 8 source recovery and independent review — 2026-10-14

[Source handoff](../../reviews/2026-10-14-r2-role-and-raw-source-handoff.md) freezes eleven files.
Both implementers are actual idle/pending0 with full finals recovered. Ten reported hashes
match; I0b's validate-production-config.sh digest was malformed (65 characters). Its actual
bytes are pinned in the handoff and private `source-wave-8-final-inputs.json`; the original
report is preserved. Nine reference files remain byte-identical to 929fcd6. Static checks
passed, but neither package is source-accepted or runtime-tested. ce11f12d is closed after
both recoveries. No local compiler/test/DB or new operational action is authorized.

### Review classification and execution graph

| Package | Complexity | Basis | Confidence | Escalation signal |
| --- | --- | --- | --- | --- |
| I0b-R | intermediate | Nine-file review against fixed D5 role/secret maps and isolated validator contract | high | Missing or contradictory interface, or findings needing changes beyond frozen scope: report, do not invent policy |
| I1b-S1-R | intermediate | Two-file review against fixed descriptor, exact-line and bounded-read contract | high | Unresolved race/trust contract or scope conflict: report concrete evidence for coordinator decomposition |

| Package | Wave | Objective / writable scope | Depends on | Worker | Verification |
| --- | --- | --- | --- | --- | --- |
| I0b-R | 9 | Review I0b; only docs/reviews/2026-10-14-r2-daily-role-wiring-review.md | I0b full final and actual-byte freeze | Existing d9ca76a3, gpt-6.1-sol/high under repository code-review rule | Exact hashes, source delta/call paths, syntax/static checks, findings and execution gaps |
| I1b-S1-R | 9 | Review Raw evidence sibling; only docs/reviews/2026-10-14-r2-raw-manifest-evidence-review.md | I1b-S1 full final and actual-byte freeze | Existing a4b5c1b9, gpt-6.1-sol/high under repository code-review rule | Exact hashes, legacy-preservation/directory-lock-parser paths, syntax/static checks, guard-to-test mapping |

Current profile Astra/high is marked coordinator; it does not replace the repository's
Sol/high code-review routing. Each reused reviewer must first be actual idle/pending0 with
its prior final recovered. Notified follow-ups have disjoint report ownership. Reviewers
must not edit frozen source, D4/D5, roadmap, ledger, other reports or Git, and must not invoke
native delegation, Cargo/rustc/Clippy/tests/build/DB/root/Docker/provider/production commands.
They may read relevant adjacent source to check an exact call path, not redesign later packages.

I0b-R checks transactional preflight before mutations, strict attributes and direct/transitive
memberships, effective database/schema CREATE, postflight and legacy bootstrap compatibility;
eight-secret inventory/projection UID/mode/profile consistency and prohibited runtime grants;
Compose binding and both validator maps/direct login lists. Inspect success and failure
credential-log paths and distinguish role GUC, actor GUC and uncalled pool behavior. Determine
which negative/rollback/log cases are written versus missing or unexecuted. Existing migrations
remain 59; no hypothetical 0060 or future service is a passing requirement for this package.

I1b-S1-R checks D4 exact-line hashing/EOF/duplicate ordering and pre-dedup limits; explicit
caller limits/checked counters/bounded reads; absolute descriptor-relative no-follow path walk;
Raw UID/mode/type trust with outside ancestors distinguished; existing read-only shared lock
and inode/path revalidation before/after read; typed redacted errors/no-write and non-Unix
closure. Review written boundary/replacement fixtures for compile/Clippy exposure and useful
assertions without executing them. Preserve legacy reader/writer/orphan and all30 wire tests.

Both reports must give prioritized concrete findings with source lines, trigger, consequence
and bounded fix; state no findings explicitly if none. Record frozen hashes, checked files,
written-test coverage and unverified execution separately. Final output must include changed
file/line range, deviations/reasons, exact checks/results, unresolved/not-found/not-verified
items, further decomposition, background work and permission waits. Missing requirements
must be reported instead of invented. Return INCOMPLETE and the checked scope if unable to
finish within 60 minutes; do not leave background work. Coordinator retains reconciliation.

After actual idle/pending0/full final, the coordinator validates each report against the source,
resolves concrete findings, commits roadmap/progress with the source delta, and prepares one
new exact-candidate hosted CI only after checking existing runs. Successful static review does
not prove compilation, actual SQL/permissions, Raw runtime trust or final R2 acceptance.
Diagram evidence and local PNG rendering remain required in I6 before structural main merge.
A replaced bounded heartbeat names only the assigned reviewers and does not protect daemon outage.


### Wave 9 actual review startup — 2026-10-14

I0b-R `d9ca76a3-b7dc-4913-944f-10ce8cc35d80` and I1b-S1-R
`a4b5c1b9-3354-400b-be82-b8dec4f87278` both started at06:05:10 UTC as
codex/gpt-6.1-sol/high, in the expected workspace/parent with pending0.
Each notified follow-up writes only its assigned review report. Input bytes remain frozen.
The 60-minute reporting boundary is07:05:10 UTC; incomplete work must be recovered explicitly.
Heartbeat `261b0071` runs every2minutes, at most60times, expires08:06:03 UTC.
Notification registration, actual startup and scope are retained in private wave9 receipts
and the ledger. The monitor must be reconciled/replaced before expiry; it cannot protect
against the same daemon stopping. Review outcomes and all new execution evidence remain pending.


## Wave 9 review disposition and bounded corrections — 2026-10-14

[I0b-R](../../reviews/2026-10-14-r2-daily-role-wiring-review.md) (261 lines,
`f83131c8cdbbf6211fe2b6f1f25426a73a4ecdcc6586eacedb6be29ba654db81`) and
[I1b-S1-R](../../reviews/2026-10-14-r2-raw-manifest-evidence-review.md) (290 lines,
`e0322d1774dbd0124d1619f7f69be983dc43a035df8d71ac8ec324d5e43c089b`) were recovered
only after actual idle/pending0/full finals, at06:15:05 and06:16:34 UTC respectively.
The coordinator checked all20 frozen inputs, 78 report links/line bounds, and the exact
source paths behind all eight findings. Both reports are accepted as correction evidence;
source acceptance remains pending. They are source findings, not observed compiler output,
actual leaked credentials or host/runtime failures. Heartbeat261b0071 is closed.

The frozen implementation commit is `f1dfb91cdc0cad7d5ae96a310ff462f40cb738a3`.
The new corrections below are the first bounded repair of these package findings; no
repeat-failure escalation signal has occurred. Repository routing therefore reuses the
idle Luna/max implementers for fixed-specification work. Reviews remain Sol/high afterward.

| Package | Complexity | Basis | Confidence | Reclassification / escalation |
| --- | --- | --- | --- | --- |
| I0b-F | intermediate | Three verified local inventory/fixture/log-gate defects with exact file and outcome boundaries | high | Missing interface/out-of-scope need: report; repeated failed correction or lost constraints: escalate one model tier |
| I1b-S1-F | intermediate | Five verified two-file error/type/fixture corrections with unchanged API and trust contract | high | Contradictory error contract or repeated verification failure: report and escalate one tier |

| Package | Wave | Objective / writable scope | Depends on | Worker | Verification |
| --- | --- | --- | --- | --- | --- |
| I0b-F | 10 | R01–03; runtime-static-check.sh, validate-production-config.sh, integration-validation/validate.sh only | I0b-R and frozen f1dfb91 inputs | fd32e206, gpt-6-luna/max | bash-n, inert source checks, written hosted self-tests, then delta review and exact-candidate CI |
| I1b-S1-F | 10 | S1-R01–05; storage.rs and daily_price_only_storage.rs only | I1b-S1-R and frozen f1dfb91 inputs | 14d3d7c8, gpt-6-luna/max | scoped rustfmt/static checks, written regressions, then delta review and exact-candidate CI |

The three I0b paths are `deploy/secrets/runtime-static-check.sh`,
`scripts/ops/validate-production-config.sh`, and `deploy/db/integration-validation/validate.sh`.
The two Raw paths remain under `crates/market-data/src/` and `crates/market-data/tests/`.
All starting hashes are pinned in private `source-wave-10-inputs.json`. Reports, other source,
D4/D5, four I0a references, pure wire/Python and roadmap/plan/Git remain worker-read-only.
No local test/validator/self-test/provisioner/Cargo/rustc/Clippy/build/DB/root/Docker/provider/
production execution is authorized. Tests are written for hosted execution, not run locally.

### I0b-F fixed correction brief

1. R01: add the eighth DB credential to both crypto and numeric fixture inventories in
   runtime-static-check.sh. Keep crypto's deliberately malformed cursor, its intended
   diagnostic, no-partial-write and no-value-output assertions. Extend the existing fakeroot
   positive fixture to check the admitter file10003:10003/0440 and directory0750, and new
   bootstrap file999:999/0400. Preserve the current rootless boundary and fakeroot availability
   behavior; do not relax inventory validation or turn a failing case into a skip.
2. R02: add `db-role-bootstrap/db_daily_input_admitter_password:999:999:400` to the release
   runtime_specs list. Add inert selected-release inventory parity checking in runtime-static-
   check.sh against the provisioner's release copies, comparing path, UID, GID and numeric mode,
   not only counts. Treat source text as data: do not source/eval/run the provisioner or production
   validator to derive maps. An unrecognized/ambiguous mapping must fail explicitly. Include
   written in-memory removal, wrong-UID and wrong-mode variants that the parity helper rejects;
   keep all other profile/source/DB-free requirements unchanged. Existing Python is available
   to the hosted policy lane if needed; do not install packages or add a dependency manifest.
3. R03: all three bootstrap callers must capture the real command status (avoid losing it with
   `if ! command`), pass success and failure output through the same credential gate before any
   record_log/evidence persistence, and preserve DENIED on bootstrap failure. Scan all8 credential
   file values exactly without putting values in argv, diagnostics or environment. A missing,
   empty or unreadable credential/raw file or scan error fails closed. No raw log is copied on
   disclosure or scan failure: retain only a fixed redacted marker and safe numeric exit/category.
   Clean failed bootstrap output may be sanitized/recorded, but its original failure remains.
4. Keep this minimal gate callable by the existing `--self-test` branch before runtime setup,
   using the same function as real callers, with no Docker/SQL activation. Move only needed pure
   helper definitions above that branch; preserve ordinary non-bootstrap sanitizer behavior.
   The gate takes explicit credential-directory/raw/destination/bootstrap-status inputs, emits no
   raw text, and returns distinguishable clean-success, clean-bootstrap-failure, disclosure and
   scan-error outcomes. Runtime callers translate every non-success into DENIED (not external
   BLOCKED); preserve the original numeric bootstrap status in safe diagnostics if needed.
5. In the existing self-test, write synthetic success/failure cases covering all8 credentials,
   URL/env/bare SQL PASSWORD forms, clean success/failure, missing/unreadable scan input and
   empty credential. Exercise the shared gate itself, capture diagnostics and evidence, and
   assert no sentinel survives either channel. Do not use a copied mock of the implementation,
   eval-extract functions, print fixture values or invoke a DB. These tests remain unexecuted
   locally. SQL, roles, bootstrap-roles.sh, Compose and provisioning semantics stay unchanged.

### I1b-S1-F fixed correction brief

1. S1-R01: in the new sibling parser, consult serde is_eof only for the existing final torn-tail
   rule, then discard the serde error on all rejected rows. Reuse the redacted
   InvalidCommittedManifestRecord { path, line } variant for malformed typed/JSON records.
   No raw serde source may remain reachable via Display, Debug or Error::source. Legacy reader
   CorruptManifest behavior stays unchanged. Update the new malformed assertion and add valid-
   shape rows with invalid FetchMode and typed hash/ID sentinel values; test all three error
   exposure channels. Use explicit Result matching rather than requiring Debug on Ok evidence.
2. S1-R02: make the borrowed &File reader binding mutable (or use a mutable local &File), and
   annotate Vec<CommittedManifestEvidenceRecord>. Keep the same descriptor, read-only flags,
   max+1 allocation/read and public API; do not add a path-based fallback or lint suppression.
3. S1-R03: rename/scope the local fixture bindings that shadow fixture() in both identified
   functions. Preserve fresh independent fixtures and all assertions; do not rename public API.
4. S1-R04: the missing-tree test calls the public method with an explicit expected UID obtained
   from its existing temp parent, not metadata of nonexistent Raw. Keep Raw absent and retain
   missing-lock/no-creation assertions. Do not create Raw to obtain a UID or weaken production
   ownership checks.
5. S1-R05: only the sibling directory-open mapper maps LOOP and NOTDIR to a static UnsafePath
   reason covering a symlink or non-directory component. Keep permission-denied failures and
   no-follow flags without retries. Preserve legacy opener behavior; maintain strict directory-
   alias/ancestor/control-symlink assertions and add an ordinary non-directory component case.
   Keep leaf regular-file/UID/mode checks and existing exact-byte/limits/duplicate/replacement
   regressions. Written tests do not establish actual platform errno or race behavior.

Both notified workers must report every finding's disposition and exact files/line ranges/hashes,
changes versus brief, static checks and guard-to-written-test map, unresolved/not-found/unverified
items, required decomposition, background work and pending permissions. Do not invent missing
policy. Return INCOMPLETE with precise residual scope if not finished in60min. Coordinator verifies
actual idle/full final, reviews the resulting delta independently, and only then prepares one new
hosted candidate with existing-run deduplication. No CI was launched for the rejected f1dfb91 bytes.


### Wave 10 actual correction startup — 2026-10-14

Review disposition and exact correction briefs were committed as `03811d0`.
I0b-F `fd32e206-9248-41cd-a86b-de07a33ea464` and I1b-S1-F
`14d3d7c8-892a-4434-baa6-29de925837cb` both started at06:23:29 UTC,
actual running/pending0, expected workspace/parent and codex/gpt-6-luna/max.
Each notified follow-up has its own5-file split (3 shell / 2 Rust); no operational
or local execution authority was added. Completion boundary is07:23:29 UTC.
Heartbeat `35d4c84a` is every2minutes, maximum60runs, expires08:24:16 UTC.
The ledger and private wave10 startup receipts preserve actual status and notification
registration. Coordinator must recover incomplete work at the reporting boundary and
reconcile/replace the heartbeat before expiry; same-daemon outage remains unprotected.
No correction result, new source acceptance or fresh CI pass is claimed yet.


## Wave 10 Raw correction recovery and delta review — 2026-10-14

I1b-S1-F `14d3d7c8-892a-4434-baa6-29de925837cb` is actual idle/pending0
at06:29:18 UTC. Its callback contained only the memory finish result; the coordinator
recovered the substantive final from activity before accepting the handoff. All five
S1-R01–05 dispositions were checked against the two-file delta. Scoped rustfmt and
diff checks passed; the other15 frozen non-owned inputs are unchanged. I0b-F's three
shell files remain independently in progress. Neither correction package is accepted
from this source inspection, and no new hosted CI or local execution has occurred.

| Frozen source | Lines | SHA-256 |
| --- | ---: | --- |
| crates/market-data/src/storage.rs | 3656 | `b1dca720210216d49d22858ac65b26a8f3cf230428629c9c4ec0ac64238e6fef` |
| crates/market-data/tests/daily_price_only_storage.rs | 644 | `1deec0dc3060fbbf880b3801b78cf640b70aae7a1d0f1c99807fe41476000d45` |

Private `source-wave-11-raw-inputs.json` pins these bytes, the nine unchanged references
and the prior290-line review `e0322d17…`. The complete worker final and recovery receipt
are under `target/r2-daily-beta/wp-4-integration/wave10-I1b-S1-F-*`.
This remains manifest-only unapproved evidence; full snapshot, admission and runtime
truth are outside this package.

| Package | Complexity | Basis | Confidence | Reclassification or escalation |
| --- | --- | --- | --- | --- |
| I1b-S1-F-R | intermediate | Independent two-file delta review against five fixed findings and unchanged trust/API contract | high | A contradictory error/trust requirement or repeated failed correction must be reported with concrete source evidence |

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
| --- | ---: | --- | --- | --- | --- | --- | --- | --- |
| I1b-S1-F-R | 11 | intermediate | Check S1-R01–05 corrections, written regressions and preservation | Only docs/reviews/2026-10-14-r2-raw-manifest-corrections-review.md | Actual idle/full final and frozen two-file bytes | Reuse a4b5c1b9, gpt-6.1-sol/high under repository code-review rule | Findings or explicit no-findings, five dispositions and execution gaps | Hash/diff/source/call-path review, scoped rustfmt/whitespace, report links; no test execution |

The reused reviewer was verified idle/pending0 with its prior final recovered. Current
Paseo profile Astra/high is coordinator-only; the repository routes code review to Sol/high.
The report scope is disjoint from the ongoing I0b-F three-file correction. Source bytes,
D4/D5, other reports, diagrams, roadmap, ledger and Git are read-only to the reviewer.

The fixed review brief is to inspect:
1. All rejected sibling serde rows discard the original error and use the redacted typed
   variant, with final torn-EOF behavior unchanged. Check Display, Debug and Error::source
   sentinels for invalid FetchMode/hash/ID, and preservation of legacy CorruptManifest.
2. The mutable borrowed reader and explicit vector type resolve the cited source defects
   without changing descriptors, limits, allocation/read bounds or public API.
3. The two fixture-shadowing repairs preserve fresh independent fixtures and every assertion.
4. Missing-tree coverage reaches the public method using the existing parent UID and creates
   no Raw tree; missing-lock and read-only assertions remain.
5. Only the sibling directory mapper handles LOOP/NOTDIR as unsafe, with no-follow/no-retry,
   permission-error closure and legacy opener unchanged. Verify ordinary non-directory,
   symlink, redirected control, owner/mode/type, bounds and replacement written regressions.

Inspect compile/strict-lint exposure at source level and report concrete issues; do not claim
compiler success from rustfmt. Adjacent source may be read only to prove a relevant type or
call path. No redesign or full snapshot work is authorized. No Cargo/rustc/Clippy/tests/
self-tests/build/DB/root/Docker/provider/production or Git actions. The reviewer must complete
directly without native or Paseo delegation and report missing/conflicting requirements.

Return one report with prioritized findings, trigger/consequence/source anchors, bounded fix,
five dispositions, exact checked hashes, actual static checks and unexecuted test limits.
The final must include changed file/line range, deviations/reasons, unresolved/not-found/
not-verified items (explicit none when empty), further decomposition, background work and
pending permissions. The reporting bound is60minutes after actual startup; return INCOMPLETE
with checked/residual scope if needed. Notifications and a bounded replacement heartbeat
retain coordinator ownership; same-daemon outage is not protected.

After actual idle/pending0/full result, the coordinator checks concrete findings against frozen
bytes. I0b-F is recovered/reviewed separately. Only then is a combined exact-candidate hosted
CI prepared after existing-run deduplication; no success claim may use929fcd6 for new code.
I6 diagrams before structural main merge and I0/0060/full input/runtime/Owner remain open.


### I0b correction recovery and paired review — 2026-10-14

I0b-F `fd32e206-9248-41cd-a86b-de07a33ea464` also reached actual idle/pending0
at06:34:39 UTC, before either wave11 review started. The substantive final, all three
hashes, the full scoped diff, three bash syntax checks and embedded Python AST were
recovered/checked. The written shared-gate and inventory cases were not executed.

| Frozen source | Lines | SHA-256 |
| --- | ---: | --- |
| deploy/secrets/runtime-static-check.sh | 568 | `24ff74cce03ec8e9f899c2a14b6ae78f09f79a5b5e9e470a59417ddf3703dccd` |
| scripts/ops/validate-production-config.sh | 889 | `a472d2f8079985f124d66d7efddda2c11e2955858014b7325f563dd3b3ff9ba2` |
| deploy/db/integration-validation/validate.sh | 1709 | `d2d4008249f0c79d64da7e00281861dfbc7a533e6e5793f585411e7693f6f6b3` |

Private `source-wave-11-inputs.json` now freezes all five corrections plus15 unchanged
inputs. Both implementers are idle, have no permission requests and have full finals
recovered. Heartbeat35d4c84a was deleted after those recoveries. Source acceptance remains
pending. The two independent reviews below may proceed in parallel with separate report
ownership; their startup and replacement monitoring are recorded after real launch.

| Package | Complexity | Basis | Confidence | Reclassification or escalation |
| --- | --- | --- | --- | --- |
| I0b-F-R | intermediate | Three fixed inventory/projection/log-gate corrections, bounded source and fixture review | high | Concrete unresolved credential/error-path contract or repeated failed correction: report and escalate one tier |

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
| --- | ---: | --- | --- | --- | --- | --- | --- | --- |
| I0b-F-R | 11 | intermediate | Check I0b-R01–03 and shared-gate failure paths without executing them | Only docs/reviews/2026-10-14-r2-daily-role-corrections-review.md | Actual idle/full final and frozen three-file bytes | Reuse d9ca76a3, gpt-6.1-sol/high under repository review rule | Findings or explicit none, three dispositions and written-test map | Hash/source/diff, bash syntax and inert Python AST; no self-tests or runtime |

I0b-F-R reads the previous261-line role review `f83131c8…`, fixed wave10 brief and
twenty-file freeze. Confirm eighth credentials in both fixtures, preserved malformed-cursor
failure/no-value/no-partial-write assertions, and numeric admitter/bootstrap ownership.
Review release33-entry parity by path/UID/GID/numeric mode against source data, unknown/
ambiguous mapping rejection, and same-helper removal/wrong-UID/wrong-mode cases.

Trace all three bootstrap outcomes through the shared eight-credential gate before evidence
recording. Check original exit preservation, fail-closed unreadable/missing/empty inputs and
scanner errors, no credential values in argv/diagnostics, fixed-marker-only disclosure/error
evidence, and DENIED on every non-success. Distinguish the gate's synthetic tests from the
runtime record/sanitizer/fail wrapper, including clean failed output and bare SQL PASSWORD.
Confirm tests exercise the actual gate, with all eight credentials and both success/failure
statuses, and state the limits of directory-based unreadable-input simulations. No actual
credential leak or runtime success may be inferred from source.

Inspect only relevant adjacent bootstrap/provisioner/Compose/helper source for concrete
compatibility; role SQL and other15 frozen inputs must remain unchanged. Report concrete
delta defects, absent coverage and unresolved interfaces precisely without inventing policy.
No source edits or new files beyond the named report; no Cargo/rustc/Clippy/tests/self-tests/
validators/provisioners/build/DB/root/Docker/provider/production/Git actions or delegation.
Use the same complete final-report format and60-minute actual-start reporting bound as the
paired Raw review. Coordinator owns acceptance and incomplete-result recovery.

Both reviews are source-only. After actual idle/pending0/full finals the coordinator resolves
findings, then prepares one exact-candidate hosted CI after checking existing runs. No CI
was dispatched for these five-file corrections yet;929fcd6 is historical baseline evidence.


### Wave 11 actual correction-review startup — 2026-10-14

Five correction files and progress/briefs are committed as `de1889be8a0f3efa9bac2c1ab1a272c9c1609af0`.
I0b-F-R `d9ca76a3-b7dc-4913-944f-10ce8cc35d80` and I1b-S1-F-R
`a4b5c1b9-3354-400b-be82-b8dec4f87278` both actually started at06:38:21 UTC as
codex/gpt-6.1-sol/high, expected workspace/parent, pending0. Notified follow-ups write only
their two distinct reports; frozen source remains read-only. Their60-minute reporting bound
is07:38:21 UTC. Heartbeat `e676d7dc` runs every2minutes, at most60times, expires08:39:02 UTC.
The ledger and private wave11 startup receipts retain notification and lifecycle evidence.
Coordinator must recover incomplete work explicitly and reconcile/replace before expiry;
the same daemon heartbeat does not protect daemon outage. No new CI or execution acceptance.


## Raw correction review recovered — 2026-10-14

I1b-S1-F-R reached actual idle/pending0 at06:43:44 UTC; full final recovered.
[Independent Raw correction review](../../reviews/2026-10-14-r2-raw-manifest-corrections-review.md) is199lines, SHA-256
`f65bb3e3a47f8cea39dfccd74656602639340249f56ddbc76881fc846eb5e88b`.
The coordinator checked the report, fifteen links/line bounds, all20 frozen inputs,
and the five source dispositions against the previously inspected two-file delta.
S1-R01–05 are closed at source-review level; no concrete new delta finding was identified.
There are19 written definitions (16 Unix integration, one alternate non-Unix, two private
Unix), normally18 selected on Unix. None were executed in this review or recovery.

Compiler, strict Clippy, tests, actual errno/locks/races/permissions and non-Unix behavior
remain unverified. Manifest-only evidence does not establish full snapshot/admission truth.
I0b-F-R is still running without pending permissions; existing e676d7dc monitoring remains
for its07:38:21 UTC reporting bound. Combined source acceptance and new exact-candidate
CI await that review and any required correction. No new workflow or operational action.


## Role correction review disposition and literal grammar repair — 2026-10-14

I0b-F-R reached actual idle/pending0 at06:48:56 UTC; full final recovered.
[Role correction review](../../reviews/2026-10-14-r2-daily-role-corrections-review.md)
is229lines, SHA-256 `52d7400dc60c14e744178e59bcf2e31c43e5ca9a5bf299bfe8d4c24aa7f3ab03`.
The coordinator checked all20 input hashes,30 links/line bounds and the claimed
counterexample by applying AST-extracted regex constants to inert strings/tuples.
No source parser function, checker, shell expression or test was executed.
Original R01–03 are corrected at source-review level. I0b-F-R01 is a new P2:
dynamic shell expressions can be interpreted as literal service/target/source/path
fields by the source checker. Current33literal rows do not contain this syntax.

Both reviewers are now idle/pending0 with full finals recovered; heartbeat e676d7dc
was deleted. Raw's five source closures remain accepted. The new one-file repair
below is necessary before a combined candidate CI; no workflow has been dispatched.

| Package | Complexity | Basis | Confidence | Escalation signal |
| --- | --- | --- | --- | --- |
| I0b-F2 | simple | Fixed literal grammar and same-parser negative cases in one inert Python block | high | Current literal mapping cannot fit grammar: report; repeat failed correction: escalate one model tier |

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
| --- | ---: | --- | --- | --- | --- | --- | --- | --- |
| I0b-F2 | 12 | simple | Reject unknown shell token semantics before constructing release maps | deploy/secrets/runtime-static-check.sh only, embedded parser/regression block | I0b-F-R01 and frozen source-wave-12-inputs.json | Reuse idle fd32e206, gpt-6-luna/max for fixed-spec implementation | Exact source hash and grammar-to-written-test map | bash-n/AST/diff only; independent delta review and hosted execution later |

This is the first repair of the newly identified parser gap. The original three
findings closed, so repeated failure of this fixed grammar has not occurred.
Current Astra/high profile is coordinator-only; the repository's fixed implementation
route remains Luna/max. No implementation may be inferred from the future plan.

### I0b-F2 fixed brief

Only `deploy/secrets/runtime-static-check.sh` is writable, starting568lines and
SHA-256 `24ff74cce03ec8e9f899c2a14b6ae78f09f79a5b5e9e470a59417ddf3703dccd`.
Private `source-wave-12-inputs.json` pins the target and19 unchanged references.
Scope is its new embedded parser plus meaningful same-parser regressions; no shell
fixtures, log gate, role/provisioner/Compose behavior, other source/docs/Git changes.

Use an explicit ASCII unquoted-literal grammar consistently:
- component: `[A-Za-z0-9_][A-Za-z0-9_.-]*`;
- service and target: exactly one component each;
- source: components separated by single slashes, supporting `tls/lagrange.crt`
  and `tls/lagrange.key`, excluding leading/trailing/repeated slash or dot/dotdot;
- validator path: exactly service/target under the same component rules;
- UID/GID: `[0-9]+` ASCII decimal; mode: existing `[0-7]{3,4}` converted base8;
  single_line remains exactly yes/no.

Validate the source field even though it is omitted from the parity tuple. Reject
parameter/command/process substitution, quotes, escaping, glob/brace/tilde syntax,
operators, extra/missing fields and unsupported numeric tokens before constructing
records. Do not interpret shell or strip quotes. Keep error messages free of token
dumps, and current path/UID/GID/mode semantics, unique entries and unknown-body checks.

Write actual-parser rejection cases for dynamic tokens in all three copy positions
and both validator path positions, invalid source paths, extra/missing arguments and
dynamic/non-ASCII numeric values. A shared validator-spec parser helper is allowed.
Use the same parser helpers as production source-map construction, not copied regex.
Only InventoryError counts as expected rejection. Include valid ordinary names and
TLS relative paths with exact tuple results, preserving current33-entry parity and
the existing removal/wrong-UID/wrong-mode dictionary cases. All fixture strings stay
inert; no eval/exec or external source fixture is permitted.

Only reads, hashes, scoped bash-n, complete embedded Python AST and diff checks may
run locally. No parser/regression loop/checker/self-test/provisioner/validator/test/
Cargo/rustc/Clippy/build/DB/root/Docker/provider/network/workflow/production action.
No other source or documentation edits, Git writes or delegation. Missing or conflicting
requirements must be reported rather than filled in.

Final report: changedfile/line ranges/hash/count, finding disposition and grammar-to-
written-test map, deviations/reasons, actual commands/results, unresolved/not-found/
not-verified items (explicit none when empty), decomposition, background work and pending
permissions. Report within30minutes of actual startup, or return INCOMPLETE with precise
residual scope. Coordinator retains recovery, review and exact-candidate hosted verification.


### Wave 12 actual literal-repair startup — 2026-10-14

Brief/review disposition commit is `4aed33c8632bbfd8269bccdb02c22b973079480d`.
I0b-F2 `fd32e206-9248-41cd-a86b-de07a33ea464` actually started at06:56:03 UTC,
codex/gpt-6-luna/max, expected workspace/parent, pending0, notified follow-up.
Its only writable scope is the embedded parser/regressions in runtime-static-check.sh.
The30-minute reporting bound is07:26:03 UTC. Heartbeat `843d6474` runs every2minutes,
at most30times, expires07:56:48 UTC. The ledger and private wave12 startup receipt
preserve actual runtime/notification evidence. Recover incomplete scope at the reporting
bound and reconcile/renew/handoff before expiry; same-daemon outage remains unprotected.
No new code result, parser/test execution, hosted CI or operational action is claimed.
