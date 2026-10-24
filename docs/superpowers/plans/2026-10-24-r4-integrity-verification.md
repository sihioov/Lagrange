Execution skill: $paseo-delegate (required)
Native subagents: prohibited for worker packages

# R4-04 재현성·무결성·사용자 격리 검증

- 작성/착수: 2026-10-24, Owner의 “이어서 작업 진행해” 지시.
- 기준: `900057c259cd2397c4d41147029930c9be83750b`, 검증된 소스 후보 `9a0b14f`.
- 작업 디렉터리: `/data/worktrees/3puw275b/short-bobcat`, `review-handoff-next-task`.
- 상태: DOING — G3 소스 리뷰 수용; 최종 후보 hosted와 승인 입력 반복 검증 대기.
- 앞 단계: [실제 시뮬레이션 수용](../../reviews/2026-10-24-r4-actual-simulation.md).

## Goal and boundaries

R4-04의 동일 입력 재현, 미래정보 차단, 분할 중복 반영, 반올림/잔여 현금/누락,
pin 변조와 사용자 격리를 요구사항·코드·실행된 테스트로 대응시킨다. 실제 공백을 보완하고
최종 후보의 필수 hosted 검증으로 확인한다. 이미 검증된 항목에 구현을 복제하는 테스트를 추가하지 않는다.

완료 기준은 항목별 검증 근거와 미확인 경계가 명확하고, 필수 보정 및 독립 리뷰가 완료되며,
수정 후보의 기존 CI 전체와 required, same-SHA smoke가 통과하는 것이다. 전체 사용자 격리가
아직 없는 R4-05 저장/API에 의존한다면 현재 순수 계산 함수의 검증으로 대신하지 않는다.
그 의존성을 구체적으로 보고하고 코디네이터가 현 단계의 가능한 검증과 후속 의존을 구분한다.

현재 입력은 고정 v3 승인, OWNER_ONLY/PRICE_RETURN_ONLY/vendor_snapshot=true/strict_pit=false다.
원본 계약 `2026-10-23-r4-price-only-simulation-contract-draft.md`의 SHA-256
`8c5c6bf7f07de09f14f1e5eaa2b6479be4b10e8b98ed5a89a280ac95568e87cd`는 변경하지 않는다.
기존9a0b14f CI9개·smoke 및 실제 두 프로세스×각2회 결과 재현은 해당 후보의 증거로 보존한다.

포함: 범위가 확인된 회귀 테스트·필수 국소 보정, 기존 접근 거절 경계 검증, 증거 대응표.
제외: 새 큐/DB/API/Web 연결(R4-05), 전략 확대/출시(R4-06), 운영 서비스 수정/배포,
provider 수집/입력 업로드, 거래/계좌, R2/R3 유예 작업. 승인 artifact는 복사·재수집하지 않는다.

지침: 사용자 제공 및 저장소 AGENTS.md, docs/ROADMAP.md, development-verification runbook,
$paseo-delegate-plan/$paseo-delegate. 현재 profile은 Astra/high 코디네이터용뿐이며 leaf는
AGENTS 모델 규칙을 따른다. 로컬 heavy Cargo/Web/Docker build는 금지하고 hosted를 사용한다.

## Initial classification

| Package | Complexity | Basis | Confidence | Reclassification or escalation signals |
| --- | --- | --- | --- | --- |
| WP-1 | intermediate | 고정 계약·유한 코드 범위의 증거와 회귀 공백 분석 | high | 사용자 격리에 새 서비스 설계가 필요하거나 계약 모순 발견 |
| WP-2 | intermediate | WP-1 후 확정된 국소 회귀/필수 보정만 구현 | medium | G1 전 정확한 소유 파일 확정 필요; 같은 원인2회 실패 시 상향 |
| WP-3 | intermediate | 변경 영역과 대응표의 독립 리뷰 | high | 설계 변경/새 권한 모델 필요 시 해당 분기 재계획 |
| WP-4 | simple | 기존 hosted 명령과 종료 증거를 지정 후보에 대조 | high | 환경 실패/로그 누락은 원인별 판단, 맹목 재실행 금지 |

## Execution graph

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
| --- | ---: | --- | --- | --- | --- | --- | --- | --- |
| WP-1 | 1 | intermediate | R4-04 증거 대응·필수 공백 확정 | private `target/r4-integrity/wp-1/` 보고서만 | G0 | gpt-6.1-sol/high, 결론이 산출물인 분석 | 파일/행·테스트·실행 근거와 최소 보정 명세 | coordinator 직접 소스/receipt 대조 |
| WP-2 | 2 | intermediate | 확정 회귀·국소 보정 | `simulation/contract.rs` cfg(test)와 private WP-2 보고서 | WP-1/G1 | gpt-6-luna/max, 명세 확정 구현 | 코드·회귀·private 보고서 | 로컬 fast, hosted는 WP-4 |
| WP-3 | 3 | intermediate | 필수 수정 여부 독립 검토 | private `target/r4-integrity/wp-3/` 보고서만 | WP-2/G2 | gpt-6.1-sol/high | 근거 있는 수용/필수 수정 | coordinator diff/evidence 확인 |
| WP-4 | 4 | simple | 최종 동일 후보 검증 | private `target/r4-integrity/wp-4/` receipt/report만 | WP-3/G3 | gpt-6-luna/medium, 기계적 회수 | run/SHA/attempt/job·기능/정리 근거 | coordinator 독립 대조 |

코디네이터는 계획·로드맵·최종 통합 및 작은1~2파일 보정을 담당한다. 현재 독립 구현 범위가
확정되지 않아 첫 wave는 WP-1 한 명이다. 이후 분리 가능한 소유 범위가 확인되면 비중복 병렬화한다.

## Worker briefs

### WP-1 — 증거와 공백 분석

위 절대 경로에서 현재 HEAD와 고정 계약을 먼저 확인한다. simulation/*.rs, 기존
owner_beta_simulation 통합 테스트, factor-engine price_only와 target의 관련 회귀,
market-data historical_price_only_approval, 기존 owner-beta API 거절/사용자 경계를 읽는다.
관련 추가 파일은 실제 호출/참조 근거가 있을 때만 좁혀 읽는다. 실데이터 읽기·Docker 실행은 없다.

각 R4-04 항목에 계약 근거, 생산 검증 경로, 테스트 이름/위치, 실제 hosted 실행 근거 또는 NOT_RUN,
누락/최소 수정, R4-05 의존 여부를 표로 기록한다. 기존 WP-5 권고인 가격 증거가 있는 q=0 거절과
q+1 overflow 직접 회귀도 검토한다. owner audience와 실제 user authorization을 구분하고
UUID/run_id 변화 불변성을 사용자 격리 증거로 오인하지 않는다. 계약을 새로 만들거나 소스를 고치지 않는다.

출력은 `target/r4-integrity/wp-1/report.md`와 필요한 hash manifest뿐(0700/0600).
보고: 읽은/변경한 파일과 행, 차이/이유, 검사 결과, 구체적 후속 작업, 찾지 못하거나 검증하지 못한 것
(없으면 명시적으로 없음). 명세 모순은 메꾸지 말고 보고한다.

### WP-2 — 보정 구현 (G1 수용, 범위 확정)

동일 절대 작업 경로, intermediate/high confidence. 소유 파일은
`crates/job-queue/src/owner_beta/simulation/contract.rs`의 `cfg(test)` 영역과
`target/r4-integrity/wp-2/` private 보고서뿐이다. 생산 코드·wire·계약·의존성은 변경하지 않는다.

- C1 `evidenced_zero_share_claim_is_rejected_for_strategy_and_benchmark`:
  `production_trading_result(99_900, 99_900)`의 정상 canonical/JSON roundtrip을 먼저 확인한다.
  첫날100원·10bp·초기10,000,000원에서 한쪽 원장만 기존 cash-only helper로 바꾸고
  정상 반대쪽 원장이 시가 증거를 제공하게 한다. 전략/벤치마크 양방향을 검사한다.
  지표와 차이를 재계산하고 개별 원장·transition·target 검증은 유효함을 확인한다.
  1주+비용100.1000원이라는 독립 oracle과 shared-price 검증의 InvalidLedger를 확인하며,
  기존 helper로 new/from_json/canonical_json 세 경로의 InvalidLedger를 검사한다.
  무가격 q=0 acceptance 회귀는 유지한다.
- C2 `maximum_buy_q_plus_one_overflow_fails_closed`: zero-cost·가격0.0001·q=u64::MAX,
  현금 units=q에서 정상 current event/spend 뒤 q+1이 ArithmeticOverflow임을 확인한다.
  q=u64::MAX-1·현금 units=q는 다음 수량이 표현 가능하나 비싸므로 Ok인 대조다.
  q=1·price units=i128::MAX/2+1·동일 cash에서는 current event가 정상이고
  다음 notional만 overflow함을 별도로 확인한다. 생산 자금 fixture로 표현하지 않는다.

구체 fixture와 관련 helper 근거는 WP-1 private report C1/C2에 있다. 임의로 테스트를
약화하거나 생산 로직을 바꾸지 말고 모순을 보고한다. 누락 import/fixture/format은 직접 수정한다.
로컬 Cargo compile/unit/Clippy·Docker/DB 실행은 금지; plan→preflight→fast만 실행한다.
보고 형식은 WP-1과 같으며 파일/행·해시·검증 결과·미확인 항목을 명시한다.

코디네이터 병렬 소유: `crates/job-queue/examples/verify_approved_simulation.rs`에서
두 번째 실행의 run/config UUID만 nil과 다른 고정 값으로 변경한다. 같은 provenance·승인 입력의
두 결과 바이트/해시 비교와 성공 후 단일 summary는 유지한다. 이는 계약231행의 실행 ID 변화
직접 검증이며 사용자 격리가 아니다. 기존 harness의 계산량(2회)은 늘리지 않는다.
실행 파일이 달라지므로 최종 후보의 source-only optional hosted build와 provenance를 새로
확인한 후 기존 read-only 승인 root에서 제한된 실행1회(내부2회)를 수행한다.
새 private receipt 경로를 쓰며 이전9a0b14f 실행/해시/receipt는 보존한다.

### WP-3 — 독립 리뷰

동일 경로에서 수정 전후 diff와 고정 계약·WP-1 대응표를 검토한다. 구현 파일은 수정하지 않고
private 보고서만 작성한다. 테스트가 실제 위반을 잡는지, 기존 v3 및 가격수익률 제한을 지키는지,
접근 권한을 약화하거나 R4-05를 앞당겨 섞지 않는지 확인한다. 필수 수정과 권고를 구분한다.
보고 형식은 WP-1과 동일. 소스 리뷰로 compile/runtime 인수를 주장하지 않는다.

### WP-4 — hosted 종료 증거

G3 후 coordinator가 고정한 candidate/branch/run ID만 대상으로 기존 CI와 Research smoke의
exact SHA/attempt/workflow·전체 job/required·기능/정리를 회수한다. coordinator가 각1회 dispatch하며
worker는 읽기 전용 회수만 담당한다. 추가 dispatch/rerun/cancel·cache 성공의 성능 주장 금지.
현재 코드/embedded provenance를 바꾼 경우 실제 입력 증거의 재사용 가능성을 G3에서 따로 판단하며,
필요하면 기존 source-only optional build와 제한된 읽기 전용 실제 runtime을 사용한다.
기존 artifact/root와 기존 확인된 경계만 사용하며 새 입력 업로드/수집/권한 변경은 없다.
보고 형식은 WP-1과 동일. polling은2분 이상, 개별 wait60초 이하, 범위 내 bounded 감시를 유지한다.

모든 worker prompt: Do not use native subagent, Task, Agent, team, or delegation features.
Complete this assignment directly and report if it needs further decomposition.

## Coordinator gates

1. G0: clean900057c·기존 완료 증거·고정 계약·현재 profile/model·소유 ledger 확인.
2. G1: WP-1 실제 idle/pending0·보고 회수, source 근거 대조. WP-2 소유 파일/기대값 확정 후 launch.
   R4-05 의존·계약 모순은 자동 보완하지 않고 해당 branch만 재계획한다.
3. G2: 구현 diff·fast/회귀 명세 확인, 독립 리뷰 시작. 작은 통합 보정은 coordinator가 처리한다.
4. G3: 필수 수정 해소·최종 소스 manifest 고정. 기존 CI/smoke 각1회 및 필요한 실제 runtime을 준비한다.
5. G4: 같은 후보 CI/required·smoke·관련 실제 증거를 직접 확인하고 대응표와 roadmap을 갱신한다.
   해당 기준을 만족하지 못한 항목은 완료 처리하지 않는다. 모든 worker/요청/보고/실행 정리 후 감시 종료.

## Progress

WP-1 `524179cd-658a-410c-b51e-fe3a5003f984` (gpt-6.1-sol/high/auto)은 idle/pending0·보고 회수 후 G1 수용.
WP-2 `05091469-ebf3-4009-9f91-2cb60c72b788` (gpt-6-luna/max/auto)은 idle/pending0·보고 회수 후 G2 소스 대조 완료.
완료 알림과 현재 작업 전용5분 감시 `9df45261`를 유지한다. WP-3 `354fef5b-9222-44ec-ab6c-d0c959c5a38a` (gpt-6.1-sol/high/auto) idle/pending0·보고 회수 및 G3 수용. WP-4는 최종 후보 검증 증거를 회수한다.
코디네이터 example UUID 보완은 rustfmt check PASS이며 새 hosted/runtime 실행은 아직 없다.

### G1 수용 — 2026-10-24

WP-1 실제 idle/pending0 및 최종 보고서를 회수했다. manifest32개 해시와27개 source/harness의
검증된9a0b14f 바이트 일치를 코디네이터가 확인했고, 계약204–209/231행과 관련 생산 helper,
기존 fixture·실제 example을 직접 대조했다. 새 생산 버그는 확인하지 못했다.
C1/C2를 최소 필수 회귀로 채택하고 O3의 실행 ID 변화 직접 증거를 위 범위로 보완한다.

R4-04 전체 사용자 격리는 R4-05 owner-scoped job/result 저장·RLS/API가 선행되어야 한다.
현재 Owner admission/추천 replay 검증은 기존 기능의 증거이며 R4 결과의 격리 증거가 아니다.
향후 다른 사용자 간 목록·ID/hash 조회/재사용 거절, 같은 hash의 별도 ownership,
failed/partial pair 비노출을 검증해야 한다. 이번 범위에서 해당 연결은 구현하지 않는다.

### G2 소스 대조 — 2026-10-24

WP-2의 두 named regression 추가 diff를 직접 읽고 cfg(test) 이전 생산 영역 바이트 불변,
source/report/receipt 해시와0700/0600을 확인했다. 소스 SHA-256은
`eef985e4abf5ffc78887dea4febc7c494642d1debb96bb4b826501c04ec8fdc3`다.
Rust fast receipt `20261024T103048Z-a6299026016e`의 cargo-format PASS를 회수했다.
새 회귀 compile/test/Clippy는 NOT_RUN이며, 독립 리뷰 완료 후 hosted에서 검증한다.

### G3 수용 — 2026-10-24

독립 리뷰는 READY_FOR_HOSTED(source-only), 필수 수정·선택 권고 없음이다. 코디네이터가
실제 idle/pending0·보고서/receipt·전후 소스 해시·고정 계약 해시를 확인했고, 새287행
테스트를 제거하면 기준 contract.rs와 바이트 일치함을 확인했다. 보고서 SHA-256은
`8916bc6e595146ce377a589ea8ae0db03e35f90bc4c001fa1de274dbaf6e30de`다.
이 변경 묶음의 후보를 고정해 CI(optional R4 verification build 포함)·smoke를 각1회 실행한다.
별도 private runtime 준비는 기존 lifecycle 함수를 유지하고 프로세스1회로 제한했다.
정리·지연 생성·소유권·종료 모의6개 PASS이며 실제 Docker/승인 입력은 아직 실행하지 않았다.
