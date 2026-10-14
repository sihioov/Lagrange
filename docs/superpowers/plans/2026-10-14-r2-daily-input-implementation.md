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

구현 패키지는 D1/D2와 D3의 실제 결과를 통합한 뒤 이 문서에 **정확한 파일 소유권·인터페이스·
검증 명령을 추가하고서** 시작한다. 현재 설계 작업을 소스 구현 완료로 세지 않는다.

## Execution graph

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
|---|---:|---|---|---|---|---|---|---|
| D1 | 1 | hard | 채택 계약을 실행 가능한 구조·인터페이스·파일 묶음으로 설계 | `docs/reviews/2026-10-14-r2-daily-input-design.md`만 | Owner 채택·기준 확인 | codex gpt-6-astra xhigh, 열린 아키텍처 판단 규칙 | 타입/DB/권한/예약/API/운영 접점과 정확한 수정 allowlist | 현재 소스 citation, 불변식·실패 행렬, 구현 분해 가능성 |
| D2 | 1 | simple | 기존 설정·추천 Web 접점과 검증 harness 목록 | `docs/reviews/2026-10-14-r2-daily-web-map.md`만 | Owner 채택·기준 확인 | codex gpt-6-luna medium, 직접 대조 가능한 소규모 조사 | 지정 경로의 DTO/화면/테스트 근거와 남은 접점 | 파일·symbol 존재와 source line 직접 확인 |
| D2a | 1 | simple | Web 구현 전 설치본 가이드·의존성 준비 | ignored root/Web `node_modules/`, npm cache, `docs/reviews/2026-10-14-r2-web-prerequisites.md` | D2 인수·코디네이터 자원 확인 | D2 worker Luna medium | exact lock 유지·설치 결과·읽은 guide 경로 | root workspace `npm ci`, manifest/lock 전후 hash |
| D3 | 2 | intermediate | 확정 전 설계의 독립 검토 | `docs/reviews/2026-10-14-r2-daily-input-design-review.md`만 | D1/D2 회수·코디네이터 통합 | codex gpt-6.1-sol high, 결론 산출물 리뷰 규칙 | actionable finding와 scope/증거 한계 | 채택 계약·실제 호출 경로·위협/회귀 행렬 대조 |

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
3. 설계 접점과 migration 번호·구현 파일 소유권을 확정하고 D3의 구체적 finding을 해소한다.
4. 구현 패키지를 상세 추가한 뒤 Luna/max로 시작한다. 실패·새 불확실성에만 단계적으로 상향한다.
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
