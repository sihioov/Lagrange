Execution skill: $paseo-delegate (required)
Native subagents: prohibited for worker packages

# main 검증 정리와 R4 첫 시뮬레이션 구현 계획

- 작성일: 2026-10-24 (Asia/Seoul).
- 기준 revision: `bb906d374f17e25485b440017639894bceb3e1ac`.
- 대상 작업 디렉터리: `/data/worktrees/3puw275b/short-bobcat`, 브랜치 `review-handoff-next-task`.
- 상태: **2026-10-24 완료 — 첫 구현·격리 검증 및 후속 실제 승인 입력 실행 범위.**
  대상9a0b14f의 CI40078923911 attempt1 all9 SUCCESS, 같은 SHA smoke40078925934 PASS,
  실제 승인 v3 입력·executable provenance·두 독립 프로세스×각2회×두 비용 프로필의 결과 일치와
  정리를 코디네이터가 수용했다. 원래 CI 실패 원인 미확정 및 후속 실패/보정 이력은 보존한다.
  [실제 실행 수용](../../reviews/2026-10-24-r4-actual-simulation.md). 운영 출시·성과 API/Web 노출은 범위 밖이다.
- 상위 작업: [ROADMAP R4-02/03](../../ROADMAP.md#8-r4--추천-전용-백테스트).
  R4-04의 계산·무결성 검증 일부를 함께 수행하되 R4-04 전체 완료로 표시하지 않는다.
- 결정 근거: [R2/R3→R4 인계](../../handoffs/2026-10-23-r2-r3-to-r4.md),
  [main 병합 기록](../../reviews/2026-10-24-main-r2-r4-integration.md).

## Goal and boundaries

### 목표와 완료 기준

기존 main 검증의 실패를 분리해 정리하고, 승인된 과거 ETF11 입력과 기존 추천 계산을 사용하는
전용 Rust 시뮬레이션 경로를 구현한다. 먼저 결과 계약과 합성 원장을 검증한 뒤 승인 입력을 연결한다.
이번 묶음의 종료점은 **소스 구현·격리 검증·동일 후보 hosted 검증**이며 성과 공개나 운영 출시가 아니다.

1. 원래 main CI와 smoke의 최종 결과, 실패 명령·원인·필요한 최소 수정, 후속 후보의 검증을 구분한다.
2. 전용 결과 schema·정규 JSON·결과 해시가 채택된 P0-01~12를 표현하고 누락·변조를 거절한다.
3. 전략·벤치마크 각각의 수량·현금·비용·NAV·지표를 작은 수작업 원장과 대조한다.
4. 승인 경계를 통과한 과거 v3 입력만 받으며 기존 factor/target 계산을 그대로 연결한다.
   실제 승인 artifact가 없는 환경의 합성 테스트를 실제 artifact 실행으로 보고하지 않는다.
5. 미래정보 차단, 가격/수량/반올림, 결정적 결과, pin·계보 무결성의 해당 회귀가 통과한다.
6. 독립 리뷰의 필수 수정과 같은 최종 코드 revision의 모든 required CI 및 기능 smoke가 통과한다.
   승인 artifact 실행 근거가 없다면 그 부분과 R4-03 최종 인수는 OPEN으로 남긴다.

### 범위

- 포함: 기존 CI 실패의 증거 회수와 국소 보정, 전용 Rust 입력/결과 계약, 순수 원장과 지표,
  기존 역사 입력·신호 연결, 해당 테스트·문서와 구조 변경이 있을 때의 도표 정합성.
- 제외: R4-05의 job 등록·큐 실행·DB 저장/RLS·API·Web 성과 화면, R4-06 전략 확대·운영 출시,
  R5 Paper, 실제 주문·계좌, 새 provider·새 데이터 수집·권리 확대, Daily 활성화와 운영 배포.
- D-06, 005930 가격 충돌, D-07은 유예를 유지한다. R2/R3 실제 입력 승인·예약 3거래일·Owner 사용·
  FINAL은 별도 미완료이며 이번 독립 구현의 공통 대기 조건으로 삼지 않는다.
- 성과 노출을 거절하는 기존 경로는 유지한다. generic Nautilus/등록 READY 성과를 이 결과로 대체하지 않는다.
- 이 문서는 작업 분담 계약이다. 계획 작성 요청으로 push·workflow dispatch·운영 접근을 새로 실행하지 않는다.
  실행자는 현재 요청과 지속되는 기존 승인을 대조해 이미 허용된 작업을 진행하고 범위가 추가되는 지점만 분리한다.

### 적용 지침과 실행 규칙

- `/home/l1nnx/.codex/AGENTS.md` 및 현재 사용자 지침: 역할 분담, 낮은 티어부터의 모델 선택,
  한 축씩 상향, 기계적 근거 또는 직접 파일 확인을 통한 결과 수용, 보수적인 memory 사용.
- [저장소 AGENTS.md](../../../AGENTS.md): 로드맵 동시 기록, 안전 경계, 도표 evidence,
  검증 단계와 자원 정책. 조상 경로와 저장소의 추가 지침 파일을 확인했다.
- Web 파일을 수정하는 분기로 확장할 경우 [Web AGENTS.md](../../../apps/web/AGENTS.md)와
  [CLAUDE.md](../../../apps/web/CLAUDE.md)를 적용하고 설치된 Next.js의 해당 로컬 문서를 먼저 읽는다.
- [검증 runbook](../../runbooks/development-verification.md)과
  [개발효율 계획](2026-10-13-development-efficiency.md)의 기존 검사·캐시 정책을 보존한다.
- 모든 WP는 `$paseo-delegate`만 사용한다. 로컬 실행 스킬 위치는
  `/home/l1nnx/.agents/skills/paseo-delegate/SKILL.md`이며 계획 시 파일 존재를 확인했다.
  실행자가 그때의 전체 지침과 Paseo 가용 profile/model을 읽는다. 스킬 또는 MCP가 없으면 worker를
  시작하지 않고 그 실행 전제를 보고한다. native collaboration/Task/Agent로 대체하지 않는다.
- 기본 동시 worker는 2명, 최대 3명+코디네이터다. 같은 파일의 소유권은 wave마다 명시적으로 이전한다.
  worker는 재위임하거나 workspace를 임의 생성하지 않는다. 필요하면 코디네이터가 Paseo workspace를 지정한다.
- 대형 Rust·Web build/browser·DB·Docker 검증은 기존 hosted job에서 수행한다.
  명시적으로 선택된 로컬 Rust focused 실행만 공용 private slot과 `CARGO_BUILD_JOBS=2`를 사용한다.
  슬롯을 얻지 못하면 다른 lock을 삭제하거나 독립 Cargo 명령으로 우회하지 않는다.

### 보존할 결정과 출발점

| 항목 | 확인된 내용과 한계 |
| --- | --- |
| R4 승인 | R4-01 제안 뒤 Owner의 “진행해”와 다음 세션 착수 지시가 인계에 기록됐다. 원본의 PROPOSED/미선택 표시는 작성 당시 상태다. 같은 최초 승인을 다시 묻지 않는다. |
| 계약 원본 | [계약](2026-10-23-r4-price-only-simulation-contract-draft.md), 버전 `owner-beta-price-only-simulation-v1-draft-1`, 원본 commit `0d2fa9b13eab86091feef7de17fd3f7579166266`, 22,156 bytes, SHA-256 `8c5c6bf7f07de09f14f1e5eaa2b6479be4b10e8b98ed5a89a280ac95568e87cd`. 이번 계획에서 길이·해시 일치를 확인했다. 원본을 편집하지 않는다. |
| Git | 직전 읽기 확인에서 원격 main=로컬 HEAD=`bb906d3`. 실행 시 다시 확인하며 후속 main 변경을 자동 병합하지 않는다. |
| 원래 hosted 실행 | [CI 39976478432](https://github.com/sihioov/Lagrange/actions/runs/39976478432), [smoke 39976478535](https://github.com/sihioov/Lagrange/actions/runs/39976478535), 각각 attempt 1. 직전 조회에서 모두 진행 중, Web job의 `web-e2e` 단계 실패. 최종 결론·실패 원인은 미확정이다. |
| 실패 단계 | `.github/workflows/ci.yml:383`의 `bash scripts/qa/candidate-web-e2e.sh`. 실패 사실만으로 앱 결함·merge 결함·환경 결함을 단정하지 않는다. |
| machine-local 인계 | 이전 worktree의 `target/r2-daily-beta/owned-workers.json`이 가리키는 Wave185 `handoff-main-pushed.json`은 626,169 bytes, SHA-256 `8f2ff342fa09f5e224e38f54a91078f3cec75451b45ea704024dbab643355565`와 일치했다. `nextAction`은 위 원래 두 실행 회수 후 R4다. 오래된 내부 요약 필드보다 최신 인계/Owner 결정이 우선한다. |
| 입력 | 2016-08-29~2026-08-28, 2,452세션·ETF11·26,972행의 기존 v3 승인 범위. 실제 파일 접근·재실행은 이번 계획에서 하지 않았다. |
| 최초 전략 | `trend_following/1.0.0`, 069500.KRX, fast_ma=50/slow_ma=200. 기존 `trend_50 < trend_200`이면 100% 매수, 그 외 및 동률은 현금이다. 수식을 별도로 근사하지 않는다. |
| 구현 위치 | 기존 `job-queue`에 domain·market-data·factor-engine·selector·serde_json·sha2 의존성이 있다. `owner_beta::simulation` 모듈을 추가하며 기존 계산·발행 runner와 구분한다. 새 crate나 result-model 의존 연결은 기본안에 없다. |

고정 계약은 1천만 원, 정수 주, 200세션 준비, T 종가 신호→다음 승인 세션 시가 체결→종가 평가다.
비용은 매수/매도 각각 0.1% 가정과 0비용 비교를 별도 계산한다. 최종일 강제 청산과 배당은 없다.
v1은 비현금 action 0, 전체 raw/adjusted OHLC 수치 일치만 허용한다.
`OWNER_ONLY`, `PRICE_RETURN_ONLY`, `vendor_snapshot=true`, `strict_pit=false` 및 5개 pin을 보존한다.
정확한 pin 값·12개 항목·세부 계산 규칙은 고정 원본을 입력으로 전달하며 이 계획이 이를 대체하지 않는다.

### 실행 전에 해소하거나 별도 기록할 항목

- U1 — CI 원인: 기존 완료 job의 로그/step과 실행 SHA를 먼저 회수한다. 로그 부재를 성공이나
  재실행 필요로 해석하지 않는다. 알 수 없는 재현 문제면 WP-1 분기만 재분류한다.
- U2 — 실제 artifact 위치와 권리: 기존 승인된 로컬 경로의 읽기 가능 여부만 확인한다. 운영 볼륨을
  새로 mount하거나 provider 재수집·Raw 업로드로 해결하지 않는다. 부재 시 WP-2/3은 진행 가능하고,
  WP-4 실제 데이터 실행만 미확인으로 남긴다. sealed 승인 경계를 느슨하게 만드는 fixture는 금지다.
- U3 — 엔진 identity: 실제 코드 revision·실행 binary 해시·toolchain/target/profile을 어떤 격리
  실행 증거에서 받는지 G2에서 고정한다. 문서 HEAD·운영 앱 해시·임의 상수를 대신 넣지 않는다.
  합성 테스트 identity와 실제 승인 artifact 결과의 provenance를 구별한다.
- U4 — 날짜/세션: 기존 v3 validator가 검증하는 승인 XKRX 세션과 Stage5 계보를 사용한다.
  bar에 존재하는 날짜만 모으거나 평일 목록을 만들어 누락 세션을 숨기지 않는다.

## Initial classification

아래 분류를 먼저 적용하고 그다음 worker를 선택한다. 미확정 입력을 상위 모델로 메우지 않는다.
새로운 증거가 분류를 바꾸면 해당 brief와 소유 범위만 개정한다.

| Package | Complexity | Basis | Confidence | Reclassification or escalation signals |
| --- | --- | --- | --- | --- |
| WP-1 | intermediate | 실행·실패 단계가 특정된 CI 증거 회수와 기존 harness의 제한된 보정. 기존 회귀·hosted 실행으로 검증 가능 | medium | 원인 로그 부재, 재현 불가, 앱/인증/운영 구조 문제면 해당 분기 보류·hard 재분류. 동일 수정 두 번 실패 시 상향 |
| WP-2 | intermediate | 채택된 schema/정규화 규칙을 폐쇄형 Rust 타입과 해시로 표현. 결정적 단위 검증 가능 | high | P0 필드 모순, 유효 결과 표현 불가, 공개 계약 확장이 필요하면 G2로 반환 |
| WP-3 | intermediate | 체결·비용·지표 규칙과 작은 기준값이 확정돼 있고 순수 함수로 검증 가능 | high | 정확한 분수/overflow 처리로 승인 기간을 계산할 수 없음, 반복 산술 실패면 해당 설계 재검토 |
| WP-4 | intermediate | 기존 sealed 승인·factor/target 계산 연결. 입력과 시점 경계의 국소 통합 | medium | 승인 우회, 새로운 signal 산술, engine provenance 위조/불명, 승인 세션 재구성이 필요하면 중단·범위 재분해 |
| WP-5 | intermediate | 계약·diff·테스트와 경계의 독립 검토가 산출물이며 근거를 직접 확인 가능 | high | 계약 모순·새 아키텍처 결정은 코디네이터에게 반환; 열린 문제를 리뷰어가 임의 구현하지 않음 |
| WP-6 | simple | 고정 후보의 기존 hosted job 결과·checkout·필수 검사·정리 증거 회수 | high | 미실행/skip/잘못된 SHA/환경 실패면 미완료. 새 harness 설계나 구현은 담당 WP로 반환 |

모델 배정은 WP-1~4=`codex / gpt-6-luna / max`, WP-5=`codex / gpt-6.1-sol / high`,
WP-6=`codex / gpt-6-luna / medium`이다. 낮은 티어 결과는 테스트 또는 코디네이터 직접 확인으로 수용한다.
Paseo provider/profile ID와 effort 지원은 **resolve at execution from current Paseo availability**.
지원되지 않는 조합을 조용히 대체하지 않는다. 두 번 실패·맴돎·맥락 위반·검증 반복 실패가 관측될 때
Luna→Sol→Astra로 한 단계씩 조정하고 모델과 effort를 동시에 올리지 않는다.
재현 불가 진단·아키텍처 판단으로 재분류된 패키지는 적용 지침의 Astra/xhigh 규칙을 따른다.

## Execution graph

먼저 G0에서 원래 CI 상태와 파일 소유권을 확인한다. WP-1과 WP-2는 파일·시스템 쓰기 범위가 달라
병렬로 진행할 수 있다. 기존 CI가 끝나지 않았다는 이유만으로 순수 계약 작업을 기다리게 하지 않는다.
WP-2→3→4는 같은 모듈의 계약·구현 의존성이 있으므로 순차 실행한다.
WP-1의 결과가 늦어져도 독립 구현은 계속할 수 있지만 최종 후보 검증 G4는 생략하지 않는다.

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
| --- | ---: | --- | --- | --- | --- | --- | --- | --- |
| WP-1 | 1 | intermediate | main CI 실패 정리 | 지정 CI/QA harness·관련 회귀·WP-1 보고서 | G0 | Luna max | 원래 실행 terminal 결과, 원인과 최소 보정 또는 NO_CHANGE | 기존 CI 계약·harness 회귀, 후속 exact-SHA hosted 결과 |
| WP-2 | 1 | intermediate | 전용 결과 계약 | simulation/mod·contract·canonical, owner_beta 모듈 선언·해당 unit test | G0 | Luna max | 폐쇄형 schema, 정규 JSON/hash, 후속 원장 입출력 타입 | unknown/duplicate/missing/scale/order/hash 회귀 |
| WP-3 | 2 | intermediate | 전략·벤치마크 순수 원장 | simulation/ledger·metrics·mod·해당 unit test, 필요한 숫자 의존만 | WP-2, G2 | Luna max | 두 비용 프로필, 정확한 원장·허용 지표 | 작은 수작업 기준값, 경계·긴 분수 합산 |
| WP-4 | 3 | intermediate | 승인 입력·기존 신호 연결 | simulation/input·signals·engine·mod·통합 회귀, 제한된 기존 seam | WP-3, G3 | Luna max | sealed 입력→두 결과의 순수 실행 경로, provenance 증거 | 승인/결손/시점/pin/identity/결정성 회귀, 가능한 경우 승인 artifact 검증 |
| WP-5 | 4 | intermediate | 독립 계약·코드 리뷰 | 리뷰 보고서만 | WP-1~4 통합 | Sol high | 심각도·file:line·재현 근거·잔여 한계 | 계약/P0 대조표, 경계와 테스트 직접 확인 |
| WP-6 | 5 | simple | 최종 후보 검증 회수 | private receipts·검증 보고서, 허용된 hosted 실행 | WP-5 수용, G4 후보 고정 | Luna medium | exact revision의 CI·smoke·실제 실행/미실행 구분 | required 전체, job/step·SHA·종료·정리 증거 |

공유 `docs/ROADMAP.md`, 이 계획, `docs/reviews/2026-10-24-r4-execution.md`, 도표와 공용 검증 설정은
코디네이터 소유다. WP-1의 workflow 변경과 다른 worker의 명세 변경을 동시에 자동 commit하지 않는다.
code commit/push 권한이 현재 실행 범위에 없으면 G4에서 검증된 로컬 결과와 정확한 후보를 준비한 뒤
그 외부 쓰기 단계만 남긴다. 이미 존재하는 동일 후보 실행은 회수하며 무조건 새 dispatch하지 않는다.

## Worker briefs

### 모든 brief에 포함할 공통 계약

코디네이터는 이 절과 해당 WP brief, 수용된 의존 산출물의 절대경로·revision/hash를 함께 전달한다.
worker가 이전 대화나 다른 worker의 과정을 알고 있다고 가정하지 않는다.
worker는 이미 Paseo로 위임된 직접 실행자다. 계획의 launch/monitor 단계는 코디네이터 전용이며,
worker가 `$paseo-delegate`를 다시 실행하거나 Paseo `create_agent`/`send_agent_prompt`를 호출하는
중첩 위임도 금지한다. 이 역할 구분을 모든 assignment/follow-up 첫머리에 명시한다.

> Do not use native subagent, Task, Agent, team, or delegation features. Complete this assignment directly and report if it needs further decomposition.

- cwd는 `/data/worktrees/3puw275b/short-bobcat`이다. 실행 시 다른 Paseo workspace를 선택하면
  코디네이터가 실제 절대경로와 source revision을 brief 전체에 반영한다.
- 각 worker는 자신의 소유 파일만 수정한다. 새 전략·공급원·API·queue·DB·운영 변경, 인접 리팩터링,
  테스트 약화, 검증 결과 조작, provider 본문·비밀 출력은 금지한다.
- private receipt는 `target/r4-first-simulation/wp-N/`의 owner-private 경로에 둔다.
  각 tracked 보고서는 `docs/reviews/2026-10-24-r4-wp-N.md`다. provider 데이터나 원시 로그를 넣지 않는다.
- 첫 출력은 확인한 전제/불일치이며, 구현 담당자는 자신의 focused 검증과 작은 수정을 직접 끝낸다.
  사소한 누락 import·cwd·인자 오류에 별도 worker나 승인 단계를 만들지 않는다.
- 필수 최종 보고: **변경 파일과 라인 범위; brief와 다르게 처리한 부분과 이유; 실행한 명령과 결과;
  미해결/후속 작업; 찾지 못했거나 확인하지 못한 것.** 마지막 두 항목이 비면 각각 `없음 (none)`이라고 쓴다.
  계획만 한 검사는 실행한 검사와 분리한다. 코디네이터는 이 보고 없이 완료로 수용하지 않는다.

### WP-1 — 원래 main CI 증거 회수와 국소 복구

- cwd: `/data/worktrees/3puw275b/short-bobcat`.
- 분류: `intermediate`, confidence `medium`. 실행·실패 단계는 알려져 있지만 원인은 아직 미확정이다.
  재현 불가/설계 문제, 같은 수정 두 번 실패, 소유 범위 밖 결함이 드러나면 해당 분기를 반환한다.
- 목표: `bb906d3`의 CI39976478432·smoke39976478535를 재실행 없이 먼저 회수하고 실제 실패 원인을
  특정한다. 부분 job 실패와 workflow 최종 결론을 구분한다. GitHub plugin이 push 실행을 제공하지
  못하면 `gh` 읽기 조회로 보완할 수 있다. 로그 접근 실패만으로 실행을 다시 만들지 않는다.
- 입력: 병합 기록, `.github/workflows/ci.yml`, 다음 harness와 회귀. 알려진 실패 명령은
  `bash scripts/qa/candidate-web-e2e.sh`다. 원래 main과 현재 후보의 SHA/attempt를 매번 구별한다.
- 소유 파일: `.github/workflows/ci.yml`, `scripts/ci/test_ci_contract.py`,
  `scripts/qa/candidate-web-e2e.sh`, `scripts/qa/candidate-web-e2e-runner.py`,
  `scripts/qa/test_candidate_web_e2e_runner.py`, WP-1 보고서.
  이 목록은 제한된 오류 보정의 최대 범위이며 전체 workflow/runner 재작성 권한이 아니다.
- **G1 후속 범위(2026-10-24):** 원래 CI의 단일 실패는 앱 테스트 `R2_DAILY_SWITCH_VERIFY`로
  좁혀졌으나 assertion 정보가 안전한 요약에서 빠져 원인 확정이 불가능했다. 따라서
  `apps/web/tests/e2e/owner-beta-recommendations.spec.ts`의 해당 step 내부에 정적 assertion 식별자를
  추가하는 변경만 소유 범위에 포함한다. 기존 assertion·상위 5초 timeout·결과 기준을 유지하고,
  runner는 제한된 깊이/개수의 중첩 step에서 allowlist 식별자만 회수한다. 메시지·expected/actual·
  stack·응답 본문은 공개하지 않는다. 해당 sanitizer 회귀를 추가하고 후속 hosted 결과로 실제
  오류를 특정한다. 앱 구현·기대값 수정은 그 근거가 생긴 뒤 해당 분기만 다시 명시한다.
- 증거가 위 범위의 import·cwd·인자·병합 누락을 입증하면 같은 worker가 고치고 해당 회귀를 재개한다.
  앱 소스·다른 harness·인증·DB가 원인이면 정확한 file:line/증거/필요 변경을 반환한다.
  코디네이터가 그 분기의 allowlist와 분류만 개정하고 동일 worker 또는 적절한 후속 worker로 이어간다.
- `required` 집계, 실패 종료코드, cleanup, 기존 browser·DB 검사를 제거하지 않는다.
  진행 중 job을 취소하거나 CI 성공을 만들려고 skip/continue-on-error를 추가하지 않는다.
- 검증: 변경 shell의 `bash -n`; 준비된 고정 Python 환경에서
  `python -m unittest scripts.ci.test_ci_contract -v`와
  `python -m unittest discover -s scripts/qa -p 'test_candidate_web_e2e_runner.py' -v` 중 영향받은 검사.
  실제 Web E2E는 hosted에서 실행한다. 해당 후보의 통합 재검증은 WP-6와 중복하지 않는다.
- 산출물: terminal run/job/step 표, 원인·보정·원래 실패 보존, 코드 변경 또는 근거 있는 NO_CHANGE,
  아직 회수하지 못한 실행. 공통 필수 보고 형식을 따른다.

### WP-2 — 폐쇄형 결과 계약과 정규 해시

- cwd: `/data/worktrees/3puw275b/short-bobcat`.
- 분류: `intermediate`, confidence `high`. 고정 원본의 P0-12와 타입 규칙을 구현한다.
  계약 모순·필드 확장 필요·실제 identity 의미가 불명확하면 추정하지 않고 G2로 반환한다.
- 입력: 고정 계약 전체와 인계의 후속 승인, `crates/domain/src/fixed.rs`,
  `crates/job-queue/src/owner_beta/{input,target,compute}.rs`, v3 승인 등록부.
- 소유: `crates/job-queue/src/owner_beta/mod.rs`의 새 모듈 선언,
  `crates/job-queue/src/owner_beta/simulation/{mod,contract,canonical}.rs`와 해당 모듈 안 unit tests,
  WP-2 보고서. 기존 추천·daily·generic 결과 타입, manifest, Cargo 의존성은 수정하지 않는다.
- 전용 schema `owner-beta-price-only-result-v1`의 contract/input/engine/strategy/schedule/
  conventions/signals/strategy_ledger/benchmark_ledger를 명시 타입으로 작성한다.
  하위 필드까지 unknown·missing·duplicate key·null을 거절한다. JSON Value 변환 과정에서 중복 키가
  사라진 뒤 검사하는 것으로 대체하지 않는다. 금액 4자리·비율 12자리·정수 수량·ppm을 검증한다.
- WP-3에 넘길 신호/세션/체결/원장/지표 타입과 검증 경계도 이 단계에서 확정한다.
  검증되지 않은 DTO를 성공 결과로 생성할 public constructor, 운영 발행 연결은 만들지 않는다.
- 계약 원본 hash를 고정하고 입력 5개 pin·labels·bars hash·전략/팩터 해시·실행 일정·engine identity를
  모두 포함한다. ASCII key ordering, 배열 순서, UTF-8 무공백·무개행, 정규 decimal/hash를 구현한다.
  snapshot 내부 float를 재직렬화하지 않고 기존 검증된 해시로 묶는다.
- result hash 자체·job/request ID·시각·duration·path·Owner ID는 계산 내용에서 제외한다.
  Owner ID 제외는 조회 권한의 면제가 아니며 저장/RLS 인수는 R4-04/05에 남긴다.
- 검증: 같은 의미의 객체 key 순서에는 같은 바이트/hash, 원장/비용/일정/신호/engine 변경에는 다른 hash,
  금지 형식·음의 0·비정규 decimal/중복 키 거절. 합성 결과는 실제 성과로 표현하지 않는다.
  공통 검증 절차의 fast와 `job-queue` focused/hosted를 사용한다.
- 산출물: 컴파일 가능한 계약 모듈·테스트, P0→field/validator 대응표, WP-3 인터페이스.
  공통 필수 보고 형식을 따른다.

G2 검토 보정(2026-10-24): 초안은 원장과 다른 수익률·MDD·회전율을 성공 결과로 받아들였다.
검증 생성자 요구를 충족하도록 WP-3의 공유 정확분수 지표 helper만 WP-2 보완으로 앞당긴다.
추가 소유 범위는 `simulation/metrics.rs`, 모듈 연결, 기존 lock의 num-bigint/num-rational/num-traits를
연결하는 job-queue Cargo 항목뿐이다. 버전·계약 값·schema는 바꾸지 않는다. WP-3는 이 helper를
재사용해 실제 원장을 계산한다. JSON 파서의 깊이 제한과 변조 거절 회귀도 WP-2에서 보완한다.
컴파일·실행 검증은 아직 미수행이며 G2 수용 전에는 WP-3를 시작하지 않는다.

### WP-3 — 두 비용 프로필의 순수 원장과 지표

- cwd: `/data/worktrees/3puw275b/short-bobcat`.
- 분류: `intermediate`, confidence `high`. 숫자·순서·기준값이 고정돼 있다.
  긴 기간의 정확한 계산이 불가능하거나 반복적으로 기준값이 깨지면 해당 산술 분기를 재분류한다.
- 의존: WP-2 계약 수용/G2. 입력은 고정 계약 P0-01~11, 수작업 예제와 WP-2 타입이다.
- 소유: `simulation/{ledger,metrics}.rs`와 unit tests, `simulation/mod.rs`의 모듈 연결,
  필요한 경우 `crates/job-queue/Cargo.toml`과 `Cargo.lock`의 해당 숫자 의존 항목만, WP-3 보고서.
  WP-2 계약 변경이 필요하면 이유를 코디네이터에게 제시해 소유권을 이전받는다.
- 외부 I/O 없는 원장 함수로 100%↔현금 전환, 매도→매수, 다음 세션 open 체결, close 평가를 구현한다.
  최초 매수도 비용 포함 최대 정수 q; q=0이면 이유를 남기고 목표 불변 시 재매수하지 않는다.
  마지막 강제 청산 없음, no-trade 현금 결과, 거래 필요 시 volume=0 실패를 포함한다.
- 금액은 0.0001원 단위 checked 정수 산술. 비용은 원금/1000을 최소 단위로 올림하여 주문당 한 번만
  반영한다. 음수·overflow·양수 아닌 분모·표현 불가 입력은 전체 실패다.
  `ILLUSTRATIVE_10BP_PER_SIDE_V1`와 `ZERO_COST_REFERENCE_V1`을 독립 실행한다.
- 수익률·최대낙폭·회전율은 원본 원장으로 계산하고 최종 단계에서만 12자리 ties-to-even으로 반올림한다.
  특히 회전율의 정확한 일별 분수 합을 f64 또는 매일 반올림한 소수 합으로 대체하지 않는다.
  이 합산에는 현재 lock에 있는 `num-bigint 0.4.8`, `num-rational 0.4.2`, 필요 시
  `num-traits 0.2.19`의 정확한 분수 표현을 재사용한다. 새 버전 업데이트는 하지 않는다.
  원금·현금의 checked overflow 규칙은 그대로 유지하고 승인 최대 기간의 합산 검증을 추가한다.
- 수작업 필수 기준: 1,000원, 첫날 open100/close110 매수, 다음날 open/close120 현금 전환.
  기본 비용에서 9주·현금99.1000→1178.0200, 비용 합1.9800, 벤치마크1179.1000;
  수익률0.178020000000/0.179100000000, 차이-0.001080000000.
  0비용은 10주·두 원장 최종1200.0000. 작은 자금 주입은 test-only이며 실제 요청은 1천만 원 고정이다.
- 추가 검증: cost ceiling 경계, q/q+1 가능성, 잔여 현금·갭 수익, 무거래·최종일·MDD,
  ties-to-even 양/음/0, 정확한 회전율과 긴 기간, overflow·누락·volume 거절.
- 산출물: 순수 엔진·지표·기준값 회귀와 공통 필수 보고. 실제 artifact/시장 성과를 주장하지 않는다.

### WP-4 — 승인 artifact·기존 추천 신호 연결

- cwd: `/data/worktrees/3puw275b/short-bobcat`.
- 분류: `intermediate`, confidence `medium`. 승인과 신호 생성은 기존 경계를 재사용한다.
  그 경계를 우회해야만 연결 가능하거나 실제 engine identity를 입증할 수 없으면 해당 부분을 반환한다.
- 의존: WP-3 수용/G3. 입력은 WP-2/3 산출물, 고정 계약/5개 pin, v3 registry,
  기존 `historical_price_only_approval.rs`, `historical_price_only_v3_artifact.rs`,
  `historical_price_only_v3.rs`, `owner_beta/{input,compute,target}.rs`, `factor-engine/src/price_only.rs`다.
- 소유: `simulation/{input,signals,engine}.rs`와 tests, `simulation/mod.rs`,
  `crates/job-queue/tests/owner_beta_simulation.rs`, WP-4 보고서.
  기존 seam 변경은 `owner_beta/compute.rs`의 동일 검증·산술을 보존하는 최소 추출,
  `market-data/src/historical_price_only_approval.rs`의 이미 검증된 metadata 접근자에 한정한다.
  이를 넘어서는 파일 필요는 증거와 함께 반환한다. 승인 상수·registry·기존 신호 규칙은 수정 금지다.
- runtime 입력은 실제 sealed 승인 함수를 먼저 통과해야 한다. 5개 pin/labels/bars hash,
  action0, 전체 OHLC raw=adjusted, 세션×11종목의 완전성·중복·양수 가격을 확인한다.
  승인된 독립 세션 기준과 Stage5 계보를 유지한다. 검증 후 다른 바이트를 다시 읽어 계산하지 않는다.
- 기본 d[200]~d[2451], 최소 2평가일·200준비일, 범위 밖/역순 거절. 각 T에는 T 이하 데이터만
  기존 factor builder에 제공하고 기존 target builder·snapshot 검증을 거친다.
  prev(S) 최초 신호, 마지막 E 새 신호 없음, 다음 승인 세션 매매를 연결한다.
- factor/target 계산을 별도 SMA로 복제하지 않는다. 미래 가격 변형 테스트는 합성 fixture의
  T 팩터값/목표를 비교하며 전체 artifact pin이 달라졌을 때 lineage hash 동일성을 요구하지 않는다.
  production 승인 생성자를 열거나 가짜 pin을 허용해서 synthetic 통합을 통과시키지 않는다.
- 두 비용 프로필 결과를 모두 완성·검증한 뒤 반환한다. 하나 실패 시 부분 비교 성공을 반환하지 않는다.
  이 함수는 provider·process spawn·job enqueue·DB·HTTP·발행 동작을 갖지 않는다.
- EngineIdentity는 G2에서 정한 검증된 실행 context에서 받고 실제 코드 revision/binary/toolchain/
  target/profile에 결합한다. library 테스트의 synthetic identity만으로 실제 실행 provenance를 주장하지 않는다.
  실제 역사 실행이 가능하면 기존 승인 위치에서 read-only로만 수행하고 입력 pin·코드/binary identity·
  결과 hash·상태만 private receipt에 기록한다. 이 단계 때문에 새 운영 CLI나 서비스는 만들지 않는다.
- 검증: 200일 경계·동률·목표 불변, missing/duplicate/조정 차이/비현금 action 거절,
  pin·snapshot 변조, 동일 입력 결정성, 미래정보 회귀, 비용 쌍 실패 원자성.
  실제 artifact 없으면 그 실행은 NOT_RUN으로 보고한다. U2 부재가 합성/거절 경로 구현을 막지는 않는다.
- 산출물: 승인 입력 연결과 테스트, 실제/합성/미실행의 증거 표, 공통 필수 보고.

### WP-4 보완 — 기존 팩터 정의 identity와 합성 회귀

2026-10-24 코디네이터 검토에서 호출자 제공 digest가 실제 정의와 연결되지 않는 공백을 확인했다.
이 항목의 표현만 다음으로 고정한다. 투자 규칙·원본 계약·결과 schema·팩터 산술은 변경하지 않는다.

- `factor_definition.sha256`는 호출자 문자열을 받지 않고 실제 기존 `factors_for`가 반환하는
  trend_200/50 정의의 descriptor를 만들고 기존 canonical JSON helper로 계산한다.
- descriptor schema는 `owner-beta-trend-factor-definition-v1`. 필드는 `schema`, `factors`,
  `sources`다. factors는 ID ASCII 오름차순이며 각 항목은 `id`, `version`, `required_fields`,
  `lookback`(`kind=trading_days`, `window`, `min_periods`), `null_policy=insufficient_lookback`를
  실제 Factor trait 값에서 읽는다. 정확한 50/200·CLOSE·full-window·현재 정책이 아니면 거절한다.
- sources는 `factor_contract_sha256`, `trend_impl_sha256`, `trend_support_sha256` 세 필드다.
  각각 기존 factor-engine `src/contract.rs`, `src/factors/trend.rs`, `src/lazy_util.rs`의 컴파일 시
  포함한 정확한 바이트 SHA-256이다. runtime 파일 I/O·수작업 hash 상수·경로/시간 입력은 없다.
- result의 기존 `factor_definition.version`은 두 실제 정의의 동일한 semver다. 서로 다르면 거절한다.
  모든 계산된 factor snapshot의 version map을 이 정의와 대조한다. 이 정의 digest는 실행 binary의
  provenance를 대신하지 않으며 U3는 별도로 미실행 상태를 유지한다.
- 원래 WP-4 파일 안에서 producer/연결/변조·결정성 회귀를 구현한다. 추가 소유권은
  `crates/factor-engine/src/price_only.rs`와 `crates/job-queue/src/owner_beta/target.rs`의 기존
  `cfg(test)` 영역에 한정한다. 기존 test-only helper로 실제 TrendFactor50/200의 T 이후 가격 변경
  불변성과 기존 target builder의 50/200 동률→현금·목표 불변 회귀를 추가한다. 두 test seam을 사용한
  합성 근거를 full sealed-artifact 실행으로 표현하지 않는다. production 생성자를 열지 않는다.
- 원본 WP-4 receipt는 보존하고 보완 receipt를 별도로 남긴다. compile/unit test/Clippy는 최종 hosted
  검증 전 NOT_RUN이다. WP-4 보완 검토 뒤 WP-5를 시작한다.

### WP-5 — 독립 계약·코드 리뷰

- cwd: `/data/worktrees/3puw275b/short-bobcat`.
- 분류: `intermediate`, confidence `high`; 리뷰 결론이 산출물이므로 Sol/high를 사용한다.
  계약 충돌이나 새 구조 결정은 코디네이터에게 반환하고 제품 요구를 임의 변경하지 않는다.
- 의존: WP-1~4의 통합 diff와 테스트 근거. 고정 계약 원본/hash, 이 계획, ROADMAP을 함께 받는다.
- 소유: WP-5 보고서만. 구현 파일·테스트·로드맵을 수정하거나 다른 worker를 생성하지 않는다.
- 초점: P0-01~12 누락, 중복 JSON key·알 수 없는 필드, 금액과 분수 반올림, q 최대값,
  원장 보존식, 실제 factor 재사용과 시점, 독립 승인 세션, pin/engine identity,
  두 프로필의 실패 원자성, production 승인 우회 부재, 기존 성과 거절·R2/R3 경계 보존.
- source-only 완료를 공개 성과/운영 인수로 잘못 표현하지 않았는지 확인한다.
  Owner ID가 hash에서 빠지는 것과 실제 사용자 격리가 구현되는 것을 혼동하지 않는다.
- 산출물: 심각도·file:line·재현 또는 논증·해결 조건을 갖춘 발견사항과 남은 검증 한계.
  코디네이터는 필수 수정을 원래 구현 worker에게 돌려보내고 변경된 영역만 다시 검토한다.
  공통 필수 보고 형식을 따른다.

### WP-6 — 동일 후보의 hosted 검증·증거 회수

- cwd: `/data/worktrees/3puw275b/short-bobcat`.
- 분류: `simple`, confidence `high`. 실행·회수 절차가 기존 workflow에 정의돼 있다.
  새 실패나 harness 필요가 드러나면 수정하지 않고 해당 WP와 코디네이터에 반환한다.
- 의존: WP-5 수용, G4가 지정한 정확한 candidate SHA와 허용된 Git/hosted 작업 범위.
- 소유: `target/r4-first-simulation/wp-6/`, WP-6 보고서, 지정된 후보의 기존 CI와 smoke 실행 회수.
  소스·workflow·운영 서비스는 수정하지 않는다. 기존 다른 실행을 취소하지 않는다.
- 동일 SHA/attempt의 기존 실행이 있으면 먼저 회수한다. 새 실행이 허용된 경우만 기존 workflow를 사용한다.
  dispatch 응답·run success만으로 검증 성공을 단정하지 않고 checkout·job/step·required·필수 검사와
  smoke의 실제 기능 단계/cleanup을 대조한다. 캐시 적중은 검사를 대신하지 않는다.
- 기존 workspace Clippy·테스트·chart acceptance, Python, policy, PostgreSQL, Web build/E2E,
  required 집계와 Research worker smoke를 보존한다. 로컬 전체 workspace/browser/DB로 대신하지 않는다.
- 새 승인 artifact를 공개 hosted 환경에 업로드하지 않는다. CI 합성 검증과 별도 권리 범위의 실제
  artifact 검증을 구분하고 소스/합성만으로 실제 성과·배포·Owner 인수를 완료하지 않는다.
- 산출물: 후보 SHA/run/attempt/URL/job/step/결론, skip/실패/미실행, 기능·정리 증거 표,
  공통 필수 보고. 오류 원인은 담당 WP가 수정하고 새 SHA의 영향받은 검증을 다시 수행한다.

## Coordinator gates

### G0 — 실행 전

1. 현재 요청이 계획 전용인지 실행 요청인지 구분한다. 계획 전용인 동안 worker/CI/운영 작업을 시작하지 않는다.
2. `$paseo-delegate` 전체 지침과 실제 Paseo profile/model/effort 지원을 확인한다.
   실제 workspace/branch/HEAD/dirty 파일, applicable instructions, 두 작업자의 쓰기 범위를 기록한다.
3. 고정 계약 바이트/hash와 후속 승인, 원래 CI 두 실행의 실제 상태를 확인한다.
   R4 최초 승인을 다시 요구하지 않는다. 계획 당시 CI 상태를 현재 성공으로 재사용하지 않는다.
4. WP-1/2를 병렬 시작할 수 있다. 산술·계약 구현은 CI 원인 조사와 독립적이다.
   새 앱 수정 범위, 외부 쓰기 또는 운영 조치가 필요할 때만 그 분기를 구체화한다.

### G1 — baseline 결과 통합

원래 CI·smoke의 terminal 결과와 원인을 수용한다. 작은 in-scope 보정은 WP-1이 직접 마친다.
추가 앱 파일이나 재현 불가 진단이 필요하면 해당 brief/분류/소유 범위만 개정한다.
R4-02/03과 독립적인 조사를 전체 개발의 무기한 대기 조건으로 만들지 않는다.
기존 실패 이력과 수정 후보 결과를 구별한다.

### G2 — 결과 계약 수용 / 원장 착수

WP-2의 폐쇄형 필드·단계·정규화·실패 타입·P0 대응표를 직접 확인한다.
성공 결과를 검증 없이 만들 수 없는지, 원장/지표 타입에 필요한 값이 모두 있는지 점검한다.
엔진 provenance의 검증된 실행 context 형식과 합성 테스트의 한계를 기록한다.
WP-3에 타입/API·수용 revision/hash를 전달한다. 미정 계약을 worker가 새 기본값으로 채우게 하지 않는다.

U3 실행 증거 형식: 결과의 기존 `EngineIdentity` 필드만 사용한다. 실제 값은 최종 후보의 격리 빌드에서
확인한 전체 source revision, 그 빌드가 만든 **실제 실행 파일**의 SHA-256, 실제 toolchain/target/profile과
대조해야 한다. private 실행 receipt가 해당 빌드 근거와 binary 해시를 연결하고 결과 안 identity와 일치시킨다.
WP-3의 순수 원장에는 이 I/O를 넣지 않는다. WP-4가 같은 executable의 실행 근거를 확보하지 못하면
실제 provenance/승인 artifact 실행은 NOT_RUN으로 남기며, 단위 테스트의 선언 identity를 승격하지 않는다.
이 형식 확정은 빌드·실행 증거를 이미 확보했다는 뜻이 아니며 새 실행 CLI나 운영 작업을 허용하지 않는다.

### G3 — 순수 원장 수용 / 승인 입력 연결

수작업 1,000원 예제와 0비용 비교, 정수 매수·현금 보존·분수 합산·마지막 날 규칙을 확인한다.
필요한 숫자 의존은 기존 lock 버전만 사용했는지, 공용 계약이나 기존 비용 모델을 바꾸지 않았는지 점검한다.
WP-4에 수용한 원장 인터페이스와 U2/U3 실제 가용성을 전달한다.
실제 입력 부재는 별도로 표시하며 승인 우회·재수집으로 해결하지 않는다.

### G4 — 통합·리뷰·검증 후보 고정

WP-1~4 통합 후 WP-5 리뷰를 받는다. 발견된 필수 오류는 원래 담당 worker가 수정하고 해당 검증을 반복한다.
공용 파일의 작은 통합은 코디네이터가 처리한다. 변경이 확정되면 정확한 candidate SHA를 고정한다.
기존 승인 범위에 맞는 Git/hosted 실행만 WP-6에 맡긴다. 이전 후보 성공을 새 후보 성공으로 재사용하지 않는다.

구조·path dependency·runtime·API 변경이 실제 발생하면 해당 `.puml`의 evidence를 수정하고 로컬에서
두 PNG를 렌더링해 같은 변경에 넣는다. 바뀌지 않은 구조는 그리지 않는다. 새 모듈만 추가되고 기존
의존·runtime 구조가 그대로인 경우에는 도표를 불필요하게 변경하지 않는다. 코드 이동으로 기존 evidence가
틀어졌으면 citation도 수정한다. network renderer는 사용하지 않는다.

### G5 — 최종 인수와 로드맵 기록

- same-SHA required 전체·smoke 성공, 필수 리뷰 해소, 원장/계약 회귀와 기존 추천·성과 거절 경로 보존.
- 실제 승인 artifact 실행 여부, engine identity, 동일 결과 hash 재현 여부를 각각 기록한다.
  실제 실행을 못 했으면 소스·격리 검증까지만 인수하고 R4-03 최종 완료 체크는 남긴다.
- R4-02/03은 각 완료 기준을 만족한 범위만 날짜·revision·증거로 갱신한다.
  R4-04 전체, R4-05/06, 운영 공개·Owner 인수·R2/R3 FINAL은 자동 완료하지 않는다.
- 문서 작성 시점은 계획 준비 상태다. 코드 실행 착수 시 ROADMAP의 해당 작업을 DOING으로 변경하고,
  매 구현·검증 변화와 같은 변경에 현재 상태·blocker·다음 행동·이력을 기록한다.
- 후속 작업은 R4-04 잔여 인수, R4-05 저장/RLS·API·Web, R4-06 전략 확대/출시 순서다.
  별도 승인 artifact·운영 범위가 필요한 부분은 이미 채택된 범위를 먼저 확인한다.

### 검증 명령과 증거 수준

실행 worker는 항상 변경 경로에 맞게 plan→preflight→fast→focused→hosted integration 순서로 진행한다.
다음은 실행 시 사용하는 명령이며 **이번 계획 작성에서 이 코드 검증을 실행했다는 뜻이 아니다**.

```bash
python3 scripts/qa/dev-check.py plan --base bb906d374f17e25485b440017639894bceb3e1ac --format json
python3 scripts/qa/dev-check.py preflight --profile rust --stage fast
python3 scripts/qa/dev-check.py run --profile rust --stage fast

# 명시적으로 로컬 focused를 선택했고 공용 private slot과 package 전제가 준비된 경우만.
python3 scripts/qa/dev-check.py preflight --profile rust --stage focused --package job-queue
python3 scripts/qa/dev-check.py run --profile rust --stage focused --package job-queue

# 기존 hosted 검사 목록 출력; 로컬 integration을 실행하지 않는다.
python3 scripts/qa/dev-check.py run --profile all --stage integration
```

`job-queue` focused에는 준비된 Phase0·locked NT/uv·Python/pyarrow·고정 격리 QA DB 전제가 있다.
이를 피하려고 production DB나 다른 포트를 사용하지 않는다. market-data 변경 시 해당 package의
focused 전제/검사도 적용한다. 필요한 hosted 명령은 다음을 포함하며 기존 workflow 전체가 최종 기준이다.

```bash
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test -p api-server --test http_owner_equity_v2_chart --locked
cargo test --workspace --locked --no-fail-fast --features kis-client/test-support
```

문서 계획의 검증은 로컬 참조·필수 섹션/패키지·의존 순서·소유권·diff 공백 검사에 한정한다.
그 결과는 구현 테스트나 최종 CI 통과를 뜻하지 않는다.

## Progress

| Package | 상태 | 증거 / 다음 행동 |
| --- | --- | --- |
| WP-1 | DONE (G1 증거 범위) | 2026-10-24 exact90f133e CI39990016668 attempt1 all8 SUCCESS, Web143개 통과. 원래 실패 미재현/원인 미확정 보존; 최종 R4 검증은 별도 |
| WP-2 | DONE | 2026-10-24 대상6ada50c 계약·정확 산술 회귀, CI8개 job·same-SHA smoke PASS. 후속9a0b14f 실제 결과/provenance 검증도 수용 |
| WP-3 | DONE | 2026-10-24 대상6ada50c 원장11개 포함 시뮬레이션 unit41 PASS. 후속9a0b14f 실제 승인 입력·반복 결과도 수용 |
| WP-4 | DONE | 2026-10-24 대상6ada50c input/signals/engine 회귀·integration2·CI8개·smoke PASS. 후속9a0b14f U2 실제 실행·U3 executable provenance와 두 프로세스 재현 수용 |
| WP-5 | DONE (소스 리뷰 범위) | 2026-10-24 manifest27ae4b99…의 19개 파일과 report31010885… 직접 대조. F1~F5 해소, READY_FOR_HOSTED; runtime 인수 아님 |
| WP-6 | DONE (same-SHA hosted 검증 범위) | 2026-10-24 대상9a0b14f CI40078923911 attempt1 all9·smoke40078925934 SUCCESS. Coordinator 로그·소스23hash·보고서·receipt 검증 및 idle/pending0 확인. 이전6ada50c 성공과 실패 이력은 보존; 실제 입력 후속 수용은 별도 종료 기록에 연결 |

2026-10-24: Owner의 `$paseo-delegate-plan` 요청으로 계획 작성. 원본 계약 hash·로컬 코드 접점·
검증 절차·실행 스킬 위치를 확인했다. 구현·worker·새 hosted 실행·운영 변경은 시작하지 않았다.

계획 문서 검증: `dev-check.py preflight/run --profile docs --stage fast` 통과.
필수 머리말·5개 섹션·6개 분류/brief·로컬 링크·공백·계약 원본 hash·로드맵 연결 및 TODO 보존을 확인했다.
검증 대상은 이 계획과 로드맵을 추가·갱신한 작업 트리이며 구현 테스트/CI/운영 인수는 수행하지 않았다.

2026-10-24 후속: Owner가 `$paseo-delegate 작업 진행 시작해`라고 지시해 G0을 확인하고 WP-1/2를
Paseo MCP로 실행했다. 모델/effort·작업 트리·parent 관계·알림 등록을 확인했고 전용 bounded 감시를 설정했다.
현재 소스/검증 결과는 미수용이며 운영 조치는 없다.


### WP-5 리뷰 후 수정 배정 — 2026-10-24

F1/F3는 코디네이터가 signals.rs 미사용 import와 input.rs 날짜 fixture를 고친다.
F2/F4/F5는 기존 WP-2가 contract.rs와 해당 테스트만 수정한다. 최대 비용 포함 q/q+1,
같은 날짜 시가 mark/체결 및 두 원장 공통 가격을 검증하며 기존 산술·거절 조건은 보존한다.
현재 schema에 없는 가격을 추측해서 q=0 근거를 만들지 않는다. 양쪽 현금 원장에 opening price가
전혀 없는 경우의 검증 한계는 별도 보고하고 재검토한다. 원본 리뷰·receipt는 고정 보존한다.

2026-10-24 재검토 착수: WP-2 actual idle/pending0·보완 보고서/receipt와 fast 결과를 회수했다.
최초 19개 소스 중 contract/input/signals 세 파일만 바뀌었음을 확인해 새 review-inputs-r2.json을 고정했다.
기존 WP-5 Sol/high에 변경 영역과 가격 증거 없는 q=0의 계약 경계 판단을 맡겼다. 원본 리뷰/manifest는 보존한다.
컴파일·unit·Clippy·실제 입력/binary·최종 hosted 인수는 여전히 OPEN이다.

2026-10-24 G4 소스 수용: WP-5 실제 idle/pending0, 전후 19개 hash와 원본 계약/보고서 보존을 확인했다.
F1~F5가 해소됐고 새 필수 수정은 없다. 무가격 q=0은 입력/provenance 검증 경계로 수용하되 실제 원가
검증을 주장하지 않는다. 가격 증거가 있는 q=0 거절과 q+1 overflow 직접 회귀는 보완 권고로 기록했다.
최종 후보의 기존 CI·smoke는 새로 실행하며 U2/U3 실제 실행은 OPEN을 유지한다.

2026-10-24 종료 회수: WP-6 actual idle/pending0 뒤 보고서59cc9364…와 terminal receipt78b14122…를 회수했다. 코디네이터가 run/job/required 및 로그를 직접 대조해 workspace3,522 pass/0 fail/6 ignored, simulation unit41/integration2, Web143, PG19를 확인했다. Clippy/required 실패로 G5는 미수용이다. 두 파일 lint 보정과 종료 문서를 새 후보로 고정하며, 현재 감시 지시의 추가 smoke 금지 아래 새 후보 CI/smoke 각각1회 허용 범위를 확인한다. U2/U3 실제 실행은 OPEN이다.

2026-10-24 재개: Owner “이어서 진행햐”를 수정 후보7d388a7 CI/smoke 각1회에 대한 진행 승인으로 기록했다. 과거 제한/실패 증거는 보존한다. 기존 WP-6 Luna/medium actual idle/pending0, HEAD/remote/19소스와 빈 실행 목록을 확인한 뒤 notified follow-up과 새 bounded 감시b5cd7be4를 등록했다. 코디네이터가 종료 결과를 검증하기 전 G5는 미수용이다.

2026-10-24 추가 lint 실패: CI40062490973의 factor-engine 테스트 Copy clone14곳을 한 파일에서 보정했다. 생산 코드 prefix와 기대값은 동일하고 formatter PASS다. R4 추가 코드의 남은 clone 사용을 타입별로 제한 검토했으며 이를 Clippy 통과로 주장하지 않는다. 현재7d388a7의 나머지 CI/smoke 회수는 계속하고 새 보정 hosted는 NOT_RUN으로 구분한다.

2026-10-24 05:02UTC CI 종료: exact7d388a7 CI40062490973 attempt1은 Clippy·required FAILURE, 나머지6개 job SUCCESS다. 코디네이터가 workspace3,522 pass/0 fail/6 ignored, simulation unit41/integration2 및 required 원인을 직접 대조했다. 같은 후보 smoke와 WP-6 최종 보고서 회수를 계속하며 추가 미커밋 lint 보정 hosted와 U2/U3는 미검증이다.

2026-10-24 종료 회수: exact7d388a7 smoke40062501852 attempt1 SUCCESS, 기능·정리 exit0 및20개 step 성공을 직접 대조했다. Web143개와 PostgreSQL19suite·62migration NO_OP도 확인했다. WP-6 최종보고서와9개 private receipt, 원본prefix 보존 및6명 actual idle/pending0를 확인했다. 단위테스트 수치는41개로 정정돼 coordinator와 일치한다. 추가 수정은 price_only.rs cfg(test)의Copy clone14개뿐이며 새 후보의 hosted는 NOT_RUN이다. 현재 각1회 승인은 소진됐고 추가 dispatch는 하지 않는다. U2/U3 실제 증거와 G5는 OPEN이다.

2026-10-24 06:47UTC 재개: 추가14곳 테스트 lint 보정을 포함한6ada50c 후보의 CI·smoke 각1회 실행을 Owner가 “그래 진행해”로 승인했다. 기존 WP-6의 actual idle/pending0 확인 뒤 notified follow-up을 시작했고 실제 codex-turn-5 running을 확인했다. CI40071485641·smoke40071496969 attempt1의 workflow·branch·SHA와 개별 HTTP204·exit0를 직접 대조했다. 현재 작업 전용5분 감시99090809(2시간/최대24회)를 등록했다. 코디네이터의 종료 결과 수용 전까지 G5와 U2/U3는 OPEN이다.

2026-10-24 07:25UTC — exact6ada50c CI40071485641 attempt1 all8 SUCCESS를 직접 대조했다. required의7개 입력도 모두 success이고 시뮬레이션 unit41/integration2가 통과했다. 같은 후보 smoke와 WP-6 최종 보고서 회수를 계속하며 G5와 실제 U2/U3는 OPEN을 유지한다.

2026-10-24 07:47UTC — WP-6 완료 알림52b3d2ee와6명 실제 idle/pending0를 회수했다. exact6ada50c의 CI8개 및 smoke 기능·정리 성공을 코디네이터가 직접 검증해 G5의 소스·격리 범위만 수용했다. 전용 builder 제거 출력도 회수해 worker 보고의 제한을 보완했다. 원본 보고 prefix와8개 worker receipt를 보존하고 감시99090809를 삭제했다. U2 기존 승인 artifact 경로를 사용자에게 요청했으며 실제 실행·U3 provenance·R4-03 최종 인수는 OPEN이다.

2026-10-24 08:15UTC — Owner가 기존 artifact 위치를 직접 찾으라고 지시해 사용자 경로 제공 대기를 해소했다. 기록 두 곳과 기존 Compose 읽기 전용 연결을 대조한 root는 `/var/lib/lagrange/data/artifacts/historical-price-beta-root`다. 호스트 일반 읽기와 sudo는 거부됐지만 기존 API의 UID10001·읽기 전용 마운트에서 현재 파일과 승인 candidate/manifest/Stage5/action pin, bars SHA·26,972행·2,452세션을 확인했다. 파일·서비스 설정을 변경하거나 데이터를 복사하지 않았다. 실제 sealed Rust 승인·시뮬레이션·U3 executable provenance는 NOT_RUN이며 기존 API 이미지를 실행 증거로 대신하지 않는다. [위치·해시 기록](../../reviews/2026-10-24-r4-approved-artifact-location.md).


### 2026-10-24 후속 실행 — 실제 승인 입력과 U3

Owner가 위치·해시 확인 및 남은 실행 단계 보고 후 “진행해 머뭇거리지.말고”라고 지시했다.
코디네이터는 실제 실행에 필요한 다음 기술적 절차를 선택했다. 기존 운영 CLI·서비스를
확장하거나 provider 데이터를 hosted 환경에 올리는 작업은 아니다.

- U3 harness: Luna/max, intermediate. 기존 public simulation API만 호출하는 검증용 Cargo example.
  컴파일 시 source/toolchain/target/profile과 실제 자기 binary hash를 결합하고, 두 비용 프로필의
  반복 실행·canonical hash·strict JSON 재검증이 모두 성공해야 aggregate summary를 출력한다.
- U3 hosted build: Luna/max, intermediate. 기존 CI에 기본 OFF인 dispatch 입력과 검증 파일 빌드를
  추가한다. 기존 필수 7개 검사와 required를 유지하며 선택한 빌드도 실패 시 인수하지 않는다.
  별도 소스 전용 artifact 예외는 실행 파일과 provenance 두 경로로 한정한다. Raw·Curated·
  승인 입력·로그·일반 target 디렉터리는 업로드하지 않는다.
- 코디네이터 실행: 당시 가용 메모리 약850MiB/스왑 여유0이므로 로컬 컴파일하지 않는다.
  검증된 static binary만 받아 기존 승인 root를 읽기 전용으로 연결한 일회성 격리 런타임에서
  실행한다. UID10001, network none, read-only root, capability 제거, CPU/메모리/PID 상한,
  소유 자식 종료·정리와 기존 서비스 불변을 확인한다. 기존 승인 데이터의 복사·재수집이나
  운영 서비스 내부에 새 코드를 설치하지 않는다.

이 후속 실행의 제한된 읽기 전용 artifact 연결은 과거 위치 탐색 단계의 새 mount 금지와
구분한다. 실제 실행 지시에 필요한 격리 검증용 연결만 선택했으며 운영 mount/권한을 변경하지 않는다.
검증 example은 운영 CLI가 아니고 job 등록·DB/API/Web 노출도 없다. 동일 코드 SHA의 hosted
검사와 executable/run identity, 승인 pin, 두 비용 결과의 재현을 각각 기록한다.
소유 작업자·실행·알림은 기존 private ledger에 추가하고 코디네이터 검증 뒤 수용한다.

2026-10-24 09:40UTC — 후속9a0b14f의 source-only executable build·CI40078923911 attempt1 9개 job·same-SHA smoke40078925934 기능/정리를 직접 수용했다. source23개·binary f7ed6422…·rustc1.97.1/musl/release provenance를 대조했고 첫 실제 승인 입력2,252세션/두 비용 프로필·내부 반복 hash와 정리를 확인했다. WP-6 idle/pending0·보고서 회수 완료. 두 번째 독립 프로세스의 교차 재현·최종 정리 전까지 실제 실행의 최종 인수는 PENDING이다. [실제 실행 기록](../../reviews/2026-10-24-r4-actual-simulation.md).

2026-10-24 09:43UTC — 실제 실행 후속 완료.9a0b14f의 두 독립 프로세스가 각2회·두 비용 결과를 재현했고5개 pin/bars/executable identity·종료0·컨테이너 부재·자식 reap·운영 identity 불변을 확인했다. CI9개/same-SHA smoke 수용과 함께 U2/U3를 닫고 R4-02/03 첫 구현/기준 전략 검증을 완료했다.10명 actual idle/pending0·보고서 회수 완료. R4-04 전체·R4-05/06·운영/Owner 인수는 범위 밖 미완료다. [최종 근거](../../reviews/2026-10-24-r4-actual-simulation.md).
