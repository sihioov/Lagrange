Execution skill: $paseo-delegate (required)
Native subagents: prohibited for worker packages

# R2 일일 연구 베타 실행 계획

- 작성일: 2026-10-13 KST
- 계획 기준 커밋: `0519c03bf566359592e18f0f77133c059c332f68`
- 작업 디렉터리: `/data/worktrees/3puw275b/excited-dolphin`
- 상위 작업: [로드맵 R2-01~R2-09](../../ROADMAP.md#6-r2--일일-연구-베타-완성)
- 상태: **Owner 실행 요청 수신; WP-1 DB 집계·복구 범위 검토와 WP-2 분석 수용; WP-3/F1·독립 WP-5/F2 보정본 `f8b9296` CI 성공·독립 P2 리뷰 종료. WP-4는 U2 채택 대기.** R2-01/02 운영 책임·실행 전제 판정은 남음.
- 계획 작성 커밋 `cea7baa`에서는 worker·운영 진단·코드/운영 변경을 실행하지 않았다.
  이후 2026-10-13 22:47 KST WP-1/2를 시작했다. [실행 기록](../../reviews/2026-10-13-r2-execution.md)에
  현재 범위·담당·수용 상태를 기록한다. 빌드·배포·migration·timer 변경·provider 호출·거래일 관찰은 미착수다.
- 아래 계획은 작업 분담·검증 계약이다. 새 데이터 입력 계약이나 운영 조치의 승인을 대신하지 않는다.

## Goal and boundaries

### 목표와 완료 기준

Owner가 정상 로그인하여 종목을 추가하고 준비 상태·일봉·신호를 확인하며,
ETF 전략 설정과 추천을 매 거래일 최신의 승인된 입력으로 사용할 수 있게 한다.
첫 베타 출시 묶음은 R1+R2이며 R3의 실제 WebSocket 인수와 독립적이다.

1. 설치된 릴리스·모드에 맞는 실행 경로, 마지막 성공, 비활성 이유와 누락 구간을 실제 근거로 구분한다.
2. 예약 수집, ETF 데이터·추천 발행, Owner V2 generation·신호·차트와 API·화면의 기준일·계보를 연결한다.
3. 정상 Auth0 Owner 세션에서 추가→상태→READY→신호·차트, 필터·상세, 재시도·비활성화,
   ETF 전략 설정·추천 조회를 확인한다. 세션 추출·가장으로 이 인수를 대신하지 않는다.
4. 누락·노후·미래 데이터와 실패·세션 만료를 정확히 표시하고, 격리 환경에서 재실행·재시작의
   중복 발행 방지와 복구를 검증한다.
5. **연속 3거래일** 예약 실행부터 데이터·추천·등록 종목 갱신까지 무인 성공을 기록한다.
   수동 보충은 그날의 무인 성공이 아니다. 실패하면 수정 후 연속 관찰을 다시 시작한다.
6. 같은 대상 revision의 CI·기능 smoke, 실제 이미지·manifest, 운영 적용·health·백업/복구,
   실제 Owner 사용 근거를 모아 R2-01~09를 각각 인수한다. 부분 성공으로 전체를 완료하지 않는다.

### 범위와 승인 경계

- 범위: 기존 daily 실행과 calendar 계약 진단·복구, 승인된 ETF EOD·추천과 Owner V2 갱신,
  해당 API·Web의 정합성·오류 표시, 운영 인수와 관찰 문서.
- 기존 승인: once-daily KIS EOD, Owner V2의 제한된 read-only rollout, 이미 결정된 데이터 권리와
  [로드맵](../../ROADMAP.md)의 범위는 유지한다. 단계 이동만으로 같은 승인을 다시 요구하지 않는다.
- 새로 확인할 사항: 누락 날짜·요청 예산, ETF 추천의 현재 날짜를 지원하는 입력의 승인·갱신 경로,
  실제 upgrade·중단/재개 영향과 복구 조건. 기존 승인으로 해결되지 않는 차이만 구체적인 결정안으로 제시한다.
- 이번 계획 작성은 R1의 한시적 Owner 정지·복귀 승인을 새로운 배포·migration·정지에 재사용하지 않는다.
  실행 요청을 받은 뒤 이미 승인된 작업은 진행하고, 추가 조치가 필요하면 준비·검증을 마친 실행안으로 판단한다.
- 제외: R3 실시간 시세 인수·WS 구독 확대, R4 백테스트 구현, R5 Paper 활성화, 실거래,
  계좌/주문 API, 새 provider, 권리 확대, 자동 KIND 수집, 승인 밖 과거 일괄 수집,
  앞서 논의한 개발 속도 개선 5가지와 무관한 리팩터링.
- KIS allowlist·기존 TokenManager·한 endpoint/TR당 1 rps·순차 실행·기존 pagination 상한을 유지한다.
  broker 응답 본문·메시지, 자격증명·쿠키·사용자 세션은 보고서/로그/Git/Mem0에 남기지 않는다.

### 적용 지침

- `/home/l1nnx/.codex/AGENTS.md`: 작업 성격별 모델·effort, 낮은 티어부터 시작, 한 단계씩 상향,
  기계적 근거/직접 확인에 의한 결과 수용, 관련 Mem0 조회와 검증된 장기 정보만 저장.
- [저장소 AGENTS.md](../../../AGENTS.md): 로드맵 동시 갱신, 서비스별 순차 운영 빌드,
  데이터·거래·비밀 경계, 구조 변경 시 두 도표의 해당 소스·근거·PNG 갱신.
- [Web AGENTS.md](../../../apps/web/AGENTS.md), [CLAUDE.md](../../../apps/web/CLAUDE.md):
  Web 수정 전 `apps/web/node_modules/next/dist/docs/`의 해당 버전 문서를 읽는다.
- [계획 스킬](/home/l1nnx/.agents/skills/paseo-delegate-plan/SKILL.md).
  실행자는 시작 전에 `$paseo-delegate`를 찾아 읽고 현재 launch·알림·heartbeat·회수 규칙을 따른다.
  그 스킬이 없으면 worker를 시작하지 않으며 native subagent로 대체하지 않는다.
- 계획 시 조상 경로의 지침과 저장소 내 AGENTS/CLAUDE 계열을 확인했다. 저장소의 추가 지침은
  Web 하위 위 두 파일이었다. 대상 경로/provider가 바뀌면 그 경로의 지침을 다시 확인한다.

### 확인한 출발점과 남은 입력

| 항목 | 계획에 사용할 근거와 한계 |
| --- | --- |
| R1 완료 | C3 `862fa954c16d28a617271fe6f613d34226e2d558`; [실제 12개 이미지·V2 manifest·복귀 인수](../../reviews/2026-10-13-r1-c3-images.md). 문서 커밋을 C3 대신 이미지 revision으로 쓰지 않는다. |
| Git | 계획 시작 HEAD `0519c03`; R1 완료 문서의 로컬 커밋이다. 실행 시 원격 main과 비교해 문서 통합 상태를 확인한다. 이 계획에서 fetch/push하지 않았다. |
| 마지막 운영 관측 | R1 종료 시 current=`fce9b90542cf9b1a911d46e4c4d3530db7efba4c`, 기존 Owner V2 healthy 복귀, 설치된 KIS timer 10개 disabled/inactive. 이번 시점의 새 진단 결과가 아니다. |
| research-worker | 과거 exited2만으로 일일 수집 실패 원인을 단정하지 않는다. daily 모드의 상주 daemon 정지는 정상일 수 있다. WP-1에서 설치 모드와 실제 경로를 대조한다. |
| daily 설치·실행 | [installer](../../../scripts/ops/install-kis-daily.sh)는 commit 접미 unit을 생성한다. `--apply`는 16:30 KST 이전만 허용하며 enable만 하고 시작하지 않는다. `--install-disabled`는 별도 비활성 설치 경로다. [runner](../../../scripts/ops/kis-daily-production.sh)의 `--check`는 DB의 정확한 누락 세션을 읽고 `--execute`는 그 목록을 처리한다. |
| calendar의 서로 다른 근거 | [XKRX 날짜 artifact 갱신](../../../scripts/ops/kis-daily-calendar-refresh.sh)은 provider-free다. 이 artifact가 KIS `chk-holiday`의 credentialed calendar나 당일 `operational_v1` window를 대신하지 않는다. [상태 계약](../../contracts/service-status-v1.md)의 EOD/장중 판정을 각각 유지한다. |
| ETF 추천 | [봉인 입력 승인 코드](../../../crates/market-data/src/historical_price_only_approval.rs)는 immutable 5-pin/registry 경로다. 과거 v2 문서의 지원 종료일을 현재 v3 종료일로 재사용하지 않는다. 현재 지원 세션과 갱신 권한은 WP-1/2에서 대조한다. |
| Owner V2 | [runbook](../../runbooks/owner-managed-equity-universe-v2.md): 기존 READY generation의 일일 증분은 provenance가 있는 거래일과 16:30 KST gate, job당 최대 2 GET, 순차 처리·중복 방지를 따른다. Add/Retry의 별도 7 GET 상한과 구분한다. |
| upgrade·복구 | [R1 인계](../../reviews/2026-10-12-r1-release-readiness.md#migrationbackuprollback-인계-범위): 실제 migration ledger/checksum/index와 데이터 의존 조건, DB+Raw/Curated 정합성, role/RLS·secret 복구·이전 release 호환성은 별도 확인 대상이다. |
| 실행 환경 | 실제 root 접근·보호 경로·잔여 디스크/메모리·작업/lease·timer 상태를 실행 직전에 확인한다. 기존 sudo 만료 예정은 2026-10-16 05:11:48 UTC이며, 유효성을 가정하거나 자동 연장하지 않는다. |

미확정 입력은 없던 값으로 채우지 않는다. 특히 아래 세 항목은 G1/G4의 필수 판정이다.

- **U1 — 달력과 실행 책임:** 날짜 artifact, credentialed calendar, 당일 window 각각의 공급 경로,
  확인 담당, 준비 마감, 결손 시 동작. EOD와 장중 표시의 의존성을 구분하며 R3 연결 성공을 요구하지 않는다.
- **U2 — ETF 현재 입력:** 지원 세션 이후의 승인된 갱신 경로와 일별 추천 생성 trigger·중복 방지 계약.
  기존 owner-beta가 요청 기반이라는 사실만으로 무인 일일 추천을 약속하지 않는다.
  현재 계약이 부족하면 후보 경로·권리·pin·발행 영향이 있는 결정안을 만들고, 해결 전 WP-4의 해당 구현과
  R2-04/R2-08/R2-09 완료를 막는다. 기존 승인이 있는지 먼저 찾고 중복 승인을 요구하지 않는다.
- **U3 — 실제 운영 변경:** 정확한 대상 commit·env/manifest 바인딩, migration 차이와 데이터 조건,
  일시 중단 대상·상한·복구 책임, 누락 세션 목록·최대 요청 수. 자료가 부족하면 영향을 받는 분기만 보류한다.

## Initial classification

요구사항이 비어 있다는 이유만으로 `hard` worker를 먼저 배정하지 않는다. WP-3~5는 G1에서
수정 계약과 파일 목록이 확정된 경우의 초기 분류다. 열린 설계·재현 불가 진단이 실제로 드러나면
코디네이터가 그 부분을 `hard`로 재분류하고 그래프·brief부터 개정한다.

| Package | Complexity | Basis | Confidence | Reclassification or escalation signals |
| --- | --- | --- | --- | --- |
| WP-1 | simple | 고정 필드·선택된 DB 집계·공식 check 결과를 모으는 읽기 전용 증거 수집; 직접 대조 가능 | medium | 승인된 읽기 인터페이스가 없거나 상태가 모순되면 해당 조회 중단; 원인 추론·새 SQL 설계는 G1로 반환 |
| WP-2 | intermediate | 기존 calendar·입력·추천·운영 계약을 소스와 비교하는 범위가 정해진 분석 | medium | 기존 계약으로 최신 입력/무인 추천을 구성할 수 없거나 아키텍처 선택이 필요하면 별도 결정안·hard 재분류 |
| WP-3 | intermediate | 확정된 daily helper/installer 결함의 국소 수정, fake host 회귀로 검증 가능 | medium | provider 범위·예약 정책·공용 배포 구조 변경이 필요하거나 같은 수정 2회 실패 시 재분류 |
| WP-4 | intermediate | G1에서 승인·확정된 ETF 입력/추천 경로 수정, artifact·queue·publish 회귀 가능 | medium | 새 승인 방식·데이터 모델·권리 계약이 필요하면 구현 금지, U2와 그래프 개정으로 반환 |
| WP-5 | intermediate | 확정된 Owner 갱신 및 API/Web 표시 계약 수정, DB·HTTP·브라우저 회귀 가능 | medium | 인증/사용자 격리 변경, 공유 schema 또는 DTO 소유권 충돌 시 재분류·순서 조정 |
| WP-6 | simple | 고정 후보의 기존 CI·smoke·지정 통합 검사를 실행/회수하고 근거를 분류; 수정 없음 | high | 새 실패는 담당 WP로 반환; 증거를 위해 새 harness가 필요하면 명세·소유권부터 개정 |
| WP-7 | intermediate | 구현·회귀와 실제 upgrade/복구 준비의 독립 리뷰; 결론 자체가 산출물 | high | 재현 불가 오류, 데이터 손실·권한 설계 문제는 해당 분기를 hard 분석으로 전환 |
| WP-8 | intermediate | 공식 builder/validator의 정해진 순차 절차와 실제 byte·manifest 검증; 자원 제약 존재 | medium | fresh health gate 불충족·OOM·새 유지보수 필요·공식 도구 불일치이면 중단 후 G3로 반환 |
| WP-9 | intermediate | G4에서 확정한 명령·중단/복구 절차만 실행하고 고정 근거를 회수 | medium | ledger/backup/health·대상 identity가 변경되거나 복구 절차가 성립하지 않으면 중단; 현장 임의 수정 금지 |
| WP-10 | simple | 정의된 일별 표와 정상 Owner 시나리오의 증거 수집; 성공 기준 고정 | high | 날짜·계보 불일치/수동 보충/Owner 미확인이면 미완료 기록, 결함은 담당 WP로 반환 |

### Worker routing

- 반복 관측·실행/요약은 `codex / gpt-6-luna / medium`: WP-1, WP-6, WP-8, WP-9, WP-10.
  WP-8/9는 명령이 확정되고 실제 결과를 직접 대조할 수 있을 때만 이 배정을 적용한다.
- 명세가 확정된 구현·회귀 작성은 `codex / gpt-6-luna / max`: WP-3, WP-4, WP-5.
- 결론이 산출물인 분석·독립 리뷰는 `codex / gpt-6.1-sol / high`: WP-2, WP-7.
- 명세 모순·미확정을 effort로 덮지 않는다. 두 번 실패, 반복 맴돎, 맥락 위반, 검증 반복 실패가
  관측되면 모델/effort 중 한 축만 한 단계씩 조정한다. Luna→Sol→Astra 순서를 따른다.
  새로 분리한 재현 불가 진단·아키텍처 판단은 적용 지침에 따라 `gpt-6-astra / xhigh` 후보로 재분류한다.
- provider/model/profile ID·effort 지원의 실제 매핑은
  **resolve at execution from current Paseo availability**. 이 계획 때문에 가용성을 추정하거나 worker를 띄우지 않는다.

## Execution graph

G0에서 시작, G1에서 진단 통합과 수정 계약을 확정한다. WP-1/2와 WP-3/4만 병렬 wave다.
WP-5는 DTO·API·queue 접점 때문에 WP-4 뒤에 둔다. DB·대형 테스트·운영 자원을 동시에 사용하지 않는다.
결함이 없는 구현 WP는 재현·검증 근거를 남겨 `NO_CHANGE`로 수용할 수 있으며 불필요한 수정을 만들지 않는다.
U2 때문에 WP-4가 시작되지 못하면, G1에서 현행 API 계약과 독립 파일 범위를 고정한
WP-5의 Owner/현재 화면 작업은 진행할 수 있다. 이 분기는 새 ETF 입력·미정 DTO 구현을 포함하지 않는다.
U2 해소 뒤에는 WP-4 결과와의 통합 회귀를 수행한다. 부분 검증 결과를 전체 R2 출시 준비 완료로 표시하지 않는다.

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
| --- | ---: | --- | --- | --- | --- | --- | --- | --- |
| WP-1 | 1 | simple | 현재 설치 모드·일일 실행·데이터 상태 수집 | 운영 읽기 전용, baseline 보고서·private receipt | G0 | Luna medium | 현재 시각의 상태·누락/미확인 표 | 고정 필드·공식 check·승인된 READ ONLY 집계 대조 |
| WP-2 | 1 | intermediate | 소스 계약과 필요한 결정·수정 명세 도출 | contract 보고서·운영 runbook 초안 | G0 | Sol high | U1/U2/U3 표, 데이터 흐름, 조건부 수정 명세 | 현재 소스·기존 승인·로드맵의 대응표 |
| WP-3 | 2 | intermediate | daily 실행 helper/설치 결함 수정 | 지정 ops·calendar 파일과 격리 회귀 | WP-1, WP-2, G1 | Luna max | daily 경로 수정 또는 NO_CHANGE | fake host self-test·calendar 회귀 |
| WP-4 | 2 | intermediate | 승인된 ETF 입력→추천 갱신 경로 완성 | 지정 collector/market-data/owner-beta backend·회귀 | WP-1, WP-2, G1/U2 해소 | Luna max | 입력·as_of·trigger·중복 방지 근거 | fixture/임시 DB의 lineage·publish 회귀 |
| WP-5 | 3 | intermediate | Owner 갱신·실제 화면 계약 완성 | 지정 Owner V2 backend, DTO, Web·회귀 | WP-3와 WP-4 수용, 또는 G1의 U2 대기 분리 계약 | Luna max | Owner/ETF 기준일·상태·사용 흐름 회귀 | HTTP·DB·Web·격리 브라우저 |
| WP-6 | 4 | simple | 동일 후보의 통합 검증 회수 | 검증 자원, CI/QA 보고서만 | WP-3~5, G2 후보 고정 | Luna medium | job/step·기능 smoke·통합 근거 | 실제 checkout SHA·terminal 결과·실행된 검증 |
| WP-7 | 5 | intermediate | 독립 출시 전 리뷰 | 리뷰 보고서만 | WP-1~6 | Sol high | 결함·권한·upgrade·복구 판정 | diff/테스트/계약·운영 준비안 대조 |
| WP-8 | 6 | intermediate | 실제 후보 이미지·배포 준비 근거 확보 | 후보 전용 build state/manifest, 준비 보고서 | WP-7 수용, G3 | Luna medium | image ID/bytes/manifest, 구체적 적용·복구안 | 공식 validator·hash·resource/health·backup 준비 증거 |
| WP-9 | 7 | intermediate | 승인 범위의 배포·daily 실행 복구 | G4가 지정한 release/DB/unit/calendar·정확한 명령 | WP-8, G4 | Luna medium | 적용·복구 상태, 예약·최초 데이터 결과 | fresh identity/health, 실제 날짜·발행, timer/daemon 배타성 |
| WP-10 | 8 | simple | 실제 Owner 사용·연속 3거래일 증거 수집 | 읽기 관찰·정상 Owner 시나리오·인수 보고서 | WP-9, G5 | Luna medium | 일별 관찰 표와 Owner 확인 | 세션별 입력→추천→API/화면 대조; 수동 성공 제외 |

### 로드맵 대응

| 로드맵 | 주된 패키지 | 완료에 필요한 증거 |
| --- | --- | --- |
| R2-01 | WP-1/2, G1 | 현재 경로·비활성 이유·마지막 성공·누락을 근거와 함께 분류 |
| R2-02 | WP-2, G1 | 세 종류 달력/시간창의 출처·책임·마감·부재 시 동작 확정 |
| R2-03 | WP-3/6~9 | 필요한 수정 검증과 공식 설치 경로의 실제 일일 실행 복구 |
| R2-04 | WP-2/4/6/9/10 | 승인된 최신 ETF 데이터와 추천의 실행·발행·입력 기준일/as_of 일치 |
| R2-05 | WP-5/6/9/10 | 실제 등록 Owner 종목의 수집·generation·artifact·신호·차트 일치 |
| R2-06 | WP-5/10 | 격리 QA와 구분한 정상 Auth0 Owner 사용 확인 |
| R2-07 | WP-3~7/10 | 격리 장애·중복 방지·복구 검증 및 실제 화면의 오류/만료 표시 |
| R2-08 | WP-10 | 연속 3거래일 예약·ETF 추천·Owner 갱신·기준일 증거 |
| R2-09 | WP-7~10, G6 | exact revision/manifest/모드·운영·복구·Owner 인수 묶음 |

## Worker briefs

### 공통 전달 계약

코디네이터는 각 worker에 아래 공통 계약, 해당 brief, 수용된 의존 산출물의 **절대경로와 hash**를
함께 전달한다. 대화 이력이나 다른 worker의 중간 과정을 볼 수 있다고 가정하지 않는다.

- 모든 WP는 `$paseo-delegate`로 실행한다. 하위 위임 없이 한 목표를 직접 완료한다.
  모순·누락·소유권 충돌은 채워 넣지 말고 보고한다. 계획 변경·인접 리팩터링·임의 운영 조치를 금지한다.
- cwd는 `/data/worktrees/3puw275b/excited-dolphin`이다. 격리가 필요하면 코디네이터가 Paseo workspace를
  만들고 G0 기록의 절대경로로 brief를 갱신한다. worker가 별도 worktree나 worker를 임의 생성하지 않는다.
- private receipt는 `target/r2-daily-beta/wp-N/`(directory 0700/file 0600)에 쓴다.
  tracked 보고서는 아래 지정 파일에만 쓴다. `docs/ROADMAP.md`, `docs/STATUS.md`, 이 계획,
  공용 실행 기록 `docs/reviews/2026-10-13-r2-execution.md`는 코디네이터만 갱신한다.
- 공용 runbook `docs/runbooks/r2-daily-beta-operations.md`는 WP-2 초안 후 소유권을 코디네이터에게 넘긴다.
  WP-8/9는 제안/실측을 자신의 보고서에 반환하고 이 파일을 동시에 편집하지 않는다.
- 코드 수정 WP는 G1의 정확한 파일 allowlist 안에서만 수정한다. 아래 목록은 최대 범위이며 디렉터리
  전체 리팩터링 권한이 아니다. 추가 파일·migration·lockfile·공유 설정 변경은 먼저 G1을 개정한다.
- SQL 검증은 실행 직전 코디네이터가 승인한 SELECT/READ ONLY transaction, 최소 role/범위,
  statement timeout·행/기간 상한·RLS 의미를 사용한다. 임의 운영 UPDATE, enqueue, DDL은 금지한다.
- synthetic fixture·임시 DB·네트워크 없는 검증과 실제 운영·Owner 근거를 구분한다.
  `skip`, 권한 없음, 날짜 부재, not-found unit을 성공으로 바꾸지 않는다.
- 필수 최종 보고 형식: **변경 파일·라인 범위 / brief와의 차이 및 이유 / 실행한 검사와 결과 /
  미해결·후속 / 찾지 못했거나 검증하지 못한 것**. 빈 항목은 `없음`을 명시한다.
  no-change도 근거를 붙이고, 실행/대기 권한/남은 background 작업을 함께 보고한다.

### WP-1 — 현재 일일 실행과 데이터 상태 수집

- cwd: `/data/worktrees/3puw275b/excited-dolphin`.
- 분류: `simple`, confidence `medium`. 고정 읽기 증거만 수집한다. 새 조회 설계/원인 추론이 필요하면
  G1로 반환한다. 초기 worker는 `gpt-6-luna / medium`.
- 목표·known facts: R1은 C3 이미지 준비까지 완료했다. 마지막 R1 관측은 설치 current `fce9b90`,
  daily timer 비활성, Owner V2 복귀다. 모두 과거값이며 현재 설치 모드·마지막 발행을 새로 확인한다.
- 입력: 로드맵, C3 이미지 인수, service-status-v1, daily installer/runner, Owner V2 runbook,
  G0의 읽기 명령·DB 집계 승인 목록. 제품 소스와 보호 설정은 읽기만 한다.
- 소유: `docs/reviews/2026-10-13-r2-runtime-baseline.md`, `target/r2-daily-beta/wp-1/`.
- 수집: exact current/release/manifest·image identity, deployment-policy의 mode, 그 commit의 unit과
  전체 관련 timer inventory, LoadState/ActiveState/SubState/Result/ExecMainStatus/last/next 실행 시각,
  daemon·one-shot·compiler·lock·job/attempt 상태, calendar/window metadata와 보호 경로 신뢰.
  env 전체·Docker 전체 inspect·원문 journal은 출력하지 않는다.
- DB: 승인된 집계로 ETF 마지막 실제 발행일/누락 세션, 추천의 성공·입력일/as_of·trigger,
  Owner membership/READY·generation·최신 신호 커버리지를 수집한다. 사용자 식별자는 비식별화한다.
  ledger/checksum/index·0050 기존 run 등 upgrade 선행 조건도 읽기 근거만 반환한다.
- `--check`라는 이름만으로 안전하다고 가정하지 않는다. 현재 설치 소스를 검토한 공식 check만
  허용한다. daily check의 DB read와 provider-free artifact/calendar check를 구분한다.
  일반 observer 실행이 state를 쓰면 이 읽기 패키지에서 실행하지 않는다.
- 검증: 관측 UTC/KST·대상 commit·명령 scope·exit/hash를 기록하고 DB transaction 읽기 전용을 확인한다.
  종료 로그/receipt 없이 unloaded unit의 기본 `Result=success`를 과거 종료 증거로 쓰지 않는다.
- 산출물/보고: 확인됨·미확인·충돌 표와 위 공통 5항목. timer 변경, collector 실행, provider 호출,
  서비스 lifecycle, artifact/queue/state 수정은 금지한다.

### WP-2 — 기존 계약·복구 명세와 미확정 결정안

- cwd: `/data/worktrees/3puw275b/excited-dolphin`.
- 분류: `intermediate`, confidence `medium`; 기존 계약 대조 분석, `gpt-6.1-sol / high`.
  새 아키텍처나 입력 승인 설계가 필요하면 그 분기를 구현하지 않고 재분류한다.
- 목표: runtime→날짜 artifact/credentialed calendar/window→EOD→ETF 추천과 Owner V2→API/화면의
  기존 계약을 한 장의 근거 표로 연결하고 G1에서 판단할 U1/U2/U3와 조건부 수정 명세를 만든다.
- 입력: 로드맵 R2 전체, daily/calendar helper, service-status-v1, Owner V2 runbook,
  `crates/market-data/src/historical_price_only_approval.rs`,
  `crates/job-queue/src/owner_beta/`, `crates/job-queue/src/owner_equity_v2/schedule.rs`,
  `crates/api-server/src/http/owner_beta.rs`, `docs/superpowers/specs/2026-08-28-owner-recommendation-remediation.md`,
  production release/backup runbook과 R1 upgrade 인계. 오래된 예시값은 현재 소스로 대조한다.
- 소유: `docs/reviews/2026-10-13-r2-contract-and-recovery.md`,
  `docs/runbooks/r2-daily-beta-operations.md`, `target/r2-daily-beta/wp-2/`. 제품 코드는 읽기만 한다.
- 명세: day eligibility/cutoff/deadline, 재실행 idempotency, token/daemon 배타성, 누락 재처리의 날짜·예산,
  ETF historical-v3 지원 세션과 daily 새 입력 승격 경로, 추천 예약/발행의 기존 권한을 구분한다.
  새 승인 필요 여부는 근거와 함께 적고 Owner의 선택을 대신하지 않는다.
- runbook 초안: exact revision 선택, helper별 plan/check/apply의 효과, backup·upgrade 검사,
  배포/current 전환·migration·Compose·daily 설치/활성화를 서로 다른 단계로 표시한다.
  적용 실패 시 이전 release 호환성·작업 정산·복구 책임을 명시하고 명령의 미정값을 표시한다.
- WP-1과 독립적으로 소스 근거를 작성한다. 현재 운영 원인 판정은 두 결과를 합치는 G1에서 한다.
- 검증/보고: 각 주장에 현재 `file:line` 또는 승인 문서 근거를 붙인다. 증거가 없는 값·진단은 미확인으로
  남기고 공통 5항목 보고. 운영 조회/변경, provider 요청, 권리·계약의 임의 확대는 금지한다.

### WP-3 — daily helper·calendar·설치 경로 수정

- cwd: `/data/worktrees/3puw275b/excited-dolphin`.
- 분류: `intermediate`, confidence `medium`; G1에서 재현과 변경 명세가 확정된 경우에만 실행한다.
  `gpt-6-luna / max`; 두 번 실패/예약·provider 정책 변경 필요 시 G1로 반환한다.
- 목표: 발견된 daily 실행 결함을 격리 환경에서 수정하거나 기존 동작이 맞다는 NO_CHANGE 근거를 낸다.
- 입력: WP-1/2 수용 결과, G1의 재현·필수 테스트·정확한 파일 목록, 저장소 운영 정책.
- 최대 소유: `scripts/ops/kis-daily-production.sh`, `scripts/ops/install-kis-daily.sh`,
  `scripts/ops/kis-daily-calendar-refresh.sh`, `scripts/ops/xkrx-calendar-bootstrap.py`,
  `scripts/ops/lib/kis-daily-state.py`, `scripts/ops/kis-daily-production-self-test.sh`,
  `scripts/ops/test_xkrx_calendar_bootstrap.py`; G1에서 필요한 파일만 활성화한다.
  2026-10-13 G1 F1 결정으로 `scripts/ops/backfill-production.sh`와
  `scripts/ops/backfill-production-self-test.sh`를 추가했다. 정확한 활성 범위와 재현은
  [실행 기록](../../reviews/2026-10-13-r2-execution.md)의 G1 부분 결정을 따른다.
  보고서는 `docs/reviews/2026-10-13-r2-daily-repair.md`.
- 보존: commit/env/manifest 바인딩, 16:30 설치 gate·Persistent catch-up 영향, daemon/one-shot 배타성,
  실패 state·Raw 보존, 정확한 누락 세션 목록, root 소유권·lock, calendar lineage, typed 실패/74·75 의미.
  실패 state 삭제·PUBLISHED 수동 표기·시간 guard 우회·legacy static activation을 금지한다.
- 검증 후보: `bash -n`의 변경 shell, `bash scripts/ops/kis-daily-production-self-test.sh`,
  `python3 scripts/ops/test_xkrx_calendar_bootstrap.py` 및 G1의 focused tests.
  실행 전에 이 검사들이 fake root/명령 또는 임시 경로만 쓰는지 확인한다.
- 운영 unit/root 파일, Compose·migration, Rust 데이터 계약은 수정하지 않는다.
  산출물은 패치/회귀와 공통 5항목 보고이며 운영 복구 완료라고 주장하지 않는다.

### WP-4 — 승인된 ETF 일일 입력과 추천 연결

- cwd: `/data/worktrees/3puw275b/excited-dolphin`.
- 분류: `intermediate`, confidence `medium`, `gpt-6-luna / max`.
  **U2 해소와 구체적 구현 명세가 launch 조건**이다. 새 설계가 필요하면 별도 `hard` 결정안으로 분리한다.
- 목표: 승인된 입력을 사용해 ETF EOD의 기준일과 추천의 입력/as_of·실행/발행을 일관되게 연결한다.
- 입력: G1에서 채택한 날짜별 입력·5-pin/승인·trigger·idempotency 계약, WP-1/2 근거.
  sealed historical artifact의 등록 여부와 generic dataset READY는 서로 다른 계약이다.
- 최대 소유: G1이 선택한 `data-pipelines/collectors/src/{pipeline,worker}.rs`와 해당 tests,
  `crates/market-data/src/`의 EOD normalization/curation 및 historical-price approval 관련 파일,
  `crates/job-queue/src/owner_beta/`, `crates/job-queue/src/bin/owner-beta-runner.rs`,
  `crates/api-server/src/{repos,http}/owner_beta.rs`, 해당 ETF/owner-beta 회귀 파일.
  Owner V2 파일·ops shell·Web은 제외한다. 공유 DTO/schema가 필요하면 G1에서 별도로 예약한다.
  보고서: `docs/reviews/2026-10-13-r2-etf-recommendation.md`.
- 보존: Raw→정규화→Curated→발행/API의 실제 날짜·출처·hash, ETF11·OWNER_ONLY·PRICE_RETURN_ONLY,
  vendor_snapshot/strict_pit 표지, 승인 registry 및 과거 결과 불변성. 과거 추천을 오늘의 결과로 바꾸지 않는다.
- 검증: target date 지원/미지원, 누락·혼합·미래 입력, pin 불일치, 중복 예약·실패 후 재시도·발행,
  승인되지 않은 날짜의 fail-closed, 같은 입력의 lineage/as_of 일치. 실제 provider 없이 fixture/임시 DB 사용.
  기존 `collectors`의 `kis_recovery`, `job-queue`의 `owner_beta_publish`, `api-server`의
  `http_owner_beta_price_only`/`http_owner_beta_access`를 변경 범위에 맞춰 실행한다.
- 운영 데이터 수집·새 registry 실데이터 승인·DB 수동 등록·generic recommendation 대체는 금지한다.
  산출물/보고는 패치·회귀 및 공통 5항목; U2 미해소 시 구현하지 않고 필요한 결정만 남긴다.

### WP-5 — Owner 갱신과 API·Web 실제 사용 계약

- cwd: `/data/worktrees/3puw275b/excited-dolphin`.
- 분류: `intermediate`, confidence `medium`, `gpt-6-luna / max`.
  인증·RLS·schema 설계 변경이나 공유 파일 충돌을 발견하면 수정 전 G1로 반환한다.
- 목표: Owner 종목의 generation/신호/차트와 ETF 추천 화면이 올바른 기준일·실패·만료를 표시하고
  정상 사용자 흐름을 유지하도록 확정된 결함만 수정한다.
- 입력: WP-3/4 수용 결과와 DTO 계약, G1 명세, Web AGENTS/CLAUDE와 설치된 Next.js 로컬 문서.
  U2 대기 분기에서는 WP-4 대신 G1이 동결한 현행 계약·독립 범위를 입력으로 받는다.
  미정 ETF 부분을 구현하지 않고 남은 통합 검증을 보고하며, WP-4와 동시에 공유 파일을 수정하지 않는다.
- 최대 소유: `crates/job-queue/src/owner_equity_v2.rs`, 그 하위 EOD/schedule 파일과 runner,
  `crates/market-data/src/owner_equity_v2.rs`, `crates/factor-engine/src/owner_equity_v2.rs`,
  `crates/api-server/src/{repos,http}/owner_equity_v2.rs`, 예약된 `crates/api-server/src/http/dto.rs`,
  `apps/web`의 owner-beta/recommendations/stock-beta/service-status 관련 route·component·lib·test.
  market-stream/WS transport·인증 정책·공용 schema는 최초 범위에서 제외한다.
  보고서: `docs/reviews/2026-10-13-r2-owner-and-web.md`.
- 보존: READY의 정상 API/queue admission, 소유자 격리, ETF 공용 기준일과 Owner 기준일의 분리,
  CURRENT/STALE/UNVERIFIABLE·미래 날짜 차단. 오래된 공용 reference가 더 최신 Owner snapshot을
  미래 데이터로 오판하는 회귀를 다시 만들지 않는다.
- 검증: `http_owner_equity_v2_chart`, `owner_equity_v2_db`, 관련 owner-beta HTTP 테스트,
  Web `typecheck`·단위·Playwright의 정상/실패/만료 시나리오. fixture session은 격리 QA에만 사용한다.
  추가·필터·상세·재시도·비활성화·ETF 설정/추천, stale 응답 도착과 새 generation 전환도 검사한다.
- 운영 종목을 자동 추가하거나 사용자 세션을 추출하지 않는다. 실제 Auth0 인수는 WP-10에서 구분한다.
  산출물/보고는 패치·격리 회귀와 공통 5항목이다.

### WP-6 — 고정 후보 CI·통합 근거 회수

- cwd: `/data/worktrees/3puw275b/excited-dolphin` 또는 G2가 기록한 고정 후보 checkout.
- 분류: `simple`, confidence `high`, `gpt-6-luna / medium`; 정해진 검사와 결과 수집만 수행한다.
  실패를 고치거나 테스트를 약화하지 않고 담당 구현 WP로 반환한다.
- 목표·입력: G2의 exact 40자리 후보와 WP-3~5의 focused 회귀 결과를 받아 동일 후보의 전체 검증을 확보한다.
  R1 C3와 main `b0a2579`의 과거 성공을 새 코드 후보의 결과로 재사용하지 않는다.
- 소유: `docs/reviews/2026-10-13-r2-validation.md`, `target/r2-daily-beta/wp-6/`, 배정된 격리 검증 자원.
  소스·workflow·test·production 파일은 편집하지 않는다.
- 검증: 먼저 동일 SHA의 기존 CI/smoke run을 회수하고, 중복 실행 전에 terminal 상태와 실제 checkout을 확인한다.
  없을 때만 실행 요청의 Git/CI 권한 안에서 시작한다. 권한이 없으면 준비 가능한 로컬 검증과 구체적 후속을 남긴다.
  `.github/workflows/ci.yml`의 format/clippy/policy/workspace/DB/Web/Python/required와 별도
  `research-smoke.yml`의 **functional PASS**를 확인한다. R1에서 정비한 공식 wrapper를 유지한다.
- focused 검증 예: `CARGO_BUILD_JOBS=2 cargo test --locked -p collectors --test kis_recovery`,
  `cargo test --locked -p api-server --test http_owner_equity_v2_chart`,
  `npm run typecheck --workspace @lagrange/web`, `npm test --workspace @lagrange/web`,
  `bash scripts/qa/candidate-web-e2e.sh`. DB·fixture·브라우저 전제는 해당 workflow와 G2 목록을 따른다.
  환경 부족으로 미실행된 테스트를 PASS로 세지 않는다.
- 운영 DB/공유 포트를 재사용하지 않으며 대형 Rust/DB/browser 검증은 같은 호스트에서 순차 실행한다.
  새 통합 harness가 필요하면 소유 WP와 검증 명세를 먼저 개정한다.
- 산출물/보고: run URL·SHA·attempt·필수 step 결과·skip 사유·행동/발행 검증 근거와 공통 5항목.

### WP-7 — 독립 코드·운영 준비 리뷰

- cwd: `/data/worktrees/3puw275b/excited-dolphin`.
- 분류: `intermediate`, confidence `high`, `gpt-6.1-sol / high`.
  결론 자체가 산출물이며 구현자의 요약만으로 수용하지 않는다.
- 목표·입력: WP-1~6 원문 근거, exact candidate diff, R1 release 인계, 운영 runbook 초안을 독립 대조한다.
- 소유: `docs/reviews/2026-10-13-r2-release-review.md`, `target/r2-daily-beta/wp-7/`만.
  제품 수정·운영 명령·하위 위임은 금지한다.
- 검토: R2 전체 대응, fresh 날짜·출처·승인·역사 결과 불변, Auth0/CSRF/RLS 경계,
  raw/queue/state 보존·중복 방지, 모드/timer 충돌, 새 코드 후보의 CI·이미지 요구,
  ledger/checksum/index·0050 데이터 조건과 backup/복구 한계·이전 release 호환성.
- 명령 효과를 구분한다. release installer의 current 전환은 서비스 전환이나 DB rollback이 아니다.
  DB DOWN·실데이터 복원은 기존 배포 실패의 자동 회복 명령으로 넣지 않는다.
- 구조 변경이 있다면 component/runtime 도표의 실제 file:line과 로컬 PNG 동시 변경을 확인한다.
- 검증/보고: 각 finding에 대상/증거/영향/해소 조건을 붙이고 공통 5항목을 보고한다.
  발견 사항 없음도 검토한 정확한 SHA·범위·미검증 한계를 명시한다. G3 판단은 코디네이터 소유다.

### WP-8 — 후보 이미지와 구체적인 운영 적용 준비

- cwd: G3에서 기록한 clean detached candidate checkout; 초기 준비 cwd는
  `/data/worktrees/3puw275b/excited-dolphin`.
- 분류: `intermediate`, confidence `medium`, `gpt-6-luna / medium`.
  검증 가능한 공식 절차만 실행한다. health/자원/backup/복구 전제가 다르면 G3로 반환한다.
- 목표: WP-7 수용 후보의 실제 이미지·manifest를 확보하고 WP-9의 검토 가능한 정확한 적용/복구안을 완성한다.
- 입력: G2의 후보, WP-6/7, 공식 builder·release/backup runbook, G3의 build 승인 범위와 자원 전제.
- 소유: `docs/reviews/2026-10-13-r2-release-preparation.md`, `target/r2-daily-beta/wp-8/`,
  G3가 배정한 후보 전용 root build state/manifest 경로. C3의 기존 출력은 덮어쓰지 않는다.
- 재사용 분기: 제품·release source-input이 C3와 같고 G2가 C3를 선택했다면 12개 image ID·revision·
  byte 증거·공식 manifest를 재검증하여 재사용한다. 문서 변경을 이유로 C3를 다시 빌드하지 않는다.
  source-input이 바뀌면 별도 R2 후보를 고정해 그 SHA의 CI와 새 image/manifest를 확보한다.
- 신규 빌드: 서비스 2~3개씩 batch를 구성하되 Compose 호출은 **항상 한 서비스**다.
  `COMPOSE_PARALLEL_LIMIT=1`, `CARGO_BUILD_JOBS=2`, background systemd·낮은 CPU/I/O priority를 유지한다.
  각 batch 후 memory/swap·OOM·exit·서비스 health와 이전 compiler 종료를 확인한다.
  추가 stop/drain이 필요하면 실행 전에 대상·영향·공유 절대 deadline·복귀 책임이 있는 조치를 G3에 제시한다.
  과거 attestation/유지보수 승인·미실행 계획을 재사용하지 않는다.
- 배포 준비: secret을 출력하지 않고 exact protected env 바인딩과 migration delta를 확인하고,
  백업·복원 준비 상태/기존 증거의 적용 가능성을 평가한다. backup 실행이나 disposable 복원에도
  새 시스템 쓰기가 있으면 G4의 정확한 단계로 포함한다. 보유 receipt를 실제 복원 검증으로 과장하지 않는다.
- 검증/보고: 실제 12개 local image ID/OCI revision/saved-byte 검사와 V2 manifest hash, builder의
  실제 terminal exit, 모든 batch gate, 구체적 적용/중단/복구 단계·각 효과를 공통 5항목과 함께 반환한다.
  serving 배포, migration, calendar/timer 활성화, provider 호출은 이 WP에서 실행하지 않는다.

### WP-9 — 승인된 운영 적용과 일일 실행 복구

- cwd: G4가 기록한 검증된 소스 checkout와 exact `/opt/lagrange/releases/<commit>` 경로.
  작업 보고 cwd는 `/data/worktrees/3puw275b/excited-dolphin`.
- 분류: `intermediate`, confidence `medium`, `gpt-6-luna / medium`.
  승인된 명령 목록·복구 동작이 모두 확정돼야 실행한다. 예상 밖 실패를 현장에서 고치지 않는다.
- 목표·입력: WP-8의 검증된 이미지/manifest와 G4의 조치별 권한·preflight·복구안을 사용해
  실제 release와 승인된 daily 실행을 복구하고 초기 데이터 근거를 확보한다.
- 소유: G4에 이름이 지정된 release/current link·Compose service·migration·calendar/unit/state만.
  보고서 `docs/reviews/2026-10-13-r2-production-activation.md`, private `target/r2-daily-beta/wp-9/`.
  root/system 범위는 allowlist의 정확한 명령과 효과에 한정한다.
- 순서: fresh 상태/권한/자원 확인 → 필요 backup 및 격리 복원·정합성 gate → migration 적용 가능성
  최종 대조 → 공식 release 설치/전환·모드에 맞는 공식 Compose/helper → health·기존 작업 정산 확인 →
  검증된 calendar/window 준비 → 해당 release의 daily 설치·활성화 → 실제 날짜·발행 확인.
  실제 순서와 서비스 정지/재개 지점은 G4에서 현재 helper에 맞춰 확정한다.
- 16:30 KST 이후 installer apply를 우회하지 않는다. 비활성 설치와 활성화는 구분한다.
  timer enable만으로 실행을 완료했다고 하지 않는다. 최초 activation의 Persistent catch-up으로
  처리될 정확한 누락 세션/예산을 먼저 확인하고 승인 밖 과거 수집이 생기면 해당 단계를 멈춘다.
- daily daemon/one-shot 중복과 겹친 token 사용을 막는다. 실패 state·Raw·불완료 queue를 보존하고,
  수동 SQL로 READY/PUBLISHED를 만들지 않는다. 새 API/거래/WS 수요를 추가하지 않는다.
- 검증/보고: exact installed release/manifest/image·mode, ledger/backup/복구 근거,
  실제 health·종료 receipt, 예약 활성 상태, 첫 실행 결과와 실제 발행일을 구분해 공통 5항목에 기록한다.
  초기 수동 복구가 성공해도 3거래일 무인 관찰은 별도로 시작한다.

### WP-10 — 실제 Owner 인수와 연속 거래일 관찰

- cwd: `/data/worktrees/3puw275b/excited-dolphin`.
- 분류: `simple`, confidence `high`, `gpt-6-luna / medium`.
  기준일/계보 불일치나 수동 보충은 실패/미완료로 기록하고 원인 수정은 담당 WP에 반환한다.
- 목표·입력: WP-9/G5의 exact 대상·관찰 시작 자격·거래일 판정·Owner 시나리오로 실사용 근거를 모은다.
- 소유: `docs/reviews/2026-10-13-r2-observation-and-acceptance.md`, `target/r2-daily-beta/wp-10/`.
  정상 Owner 사용 외에는 read-only다. 진단·장애 주입·코드/운영 설정 수정은 하지 않는다.
- 각 관찰은 짧은 실행으로 종료·회수한다. 다음 관찰 시각과 남은 항목을 코디네이터가 기록한다.
  worker를 3일 동안 sleep시키지 않는다. 새 schedule/timer를 이 계획만으로 설치하지 않는다.
  실제 모니터링·알림·재개는 실행 시 `$paseo-delegate`의 수명주기 규칙을 따른다.
- 일별 최소 열: 거래일, 대상 commit/manifest/모드, 예약 실행 시작·완료·결과, ETF 발행 기준일,
  추천 실행·발행/입력 기준일/as_of, 등록 Owner 갱신 커버리지·기준일, API·화면 확인,
  수동 개입·실패·복구 여부, 안전한 증거 링크.
- 기존 계약의 eligibility와 마감으로 거래일 성공을 판단한다. 휴장·주말을 성공 일수에 포함하지 않고,
  임의의 72시간으로 대체하지 않는다. 데이터 기준일·입력·추천·API를 같은 기대 세션과 대조한다.
- 정상 Auth0 로그인은 Owner가 수행한다. 이후 허용된 화면에서 추가/상태/READY/신호·차트,
  필터/상세/재시도/비활성화와 ETF 설정/추천을 확인한다. 운영 Add/Retry는 정해진 종목·요청 예산과
  기존 승인 안에서만 수행한다. 실패·만료 주입은 WP-5/6의 격리 근거를 사용한다.
- 가용한 정상 세션이나 Owner 확인이 없으면 R2-06을 미완료로 남긴다. 세션 위조·fixture·서버 health로
  채우지 않는다. 관찰 도중 수정/재배포·실패가 발생하면 G5에서 동일 후보로 연속 구간을 다시 정한다.
- 산출물/보고: 날짜별 실제 근거와 공통 5항목. G6에서 코디네이터가 최종 R2 완료를 판단한다.

## Coordinator gates

### G0 — Pre-launch

1. 현재 실행 요청의 범위와 `$paseo-delegate` 가용성을 확인하고 지침을 읽는다. 계획만 요청된 이번에는
   WP를 시작하지 않았으며 후속 Owner 실행 요청에 따라 WP-1/2를 시작했다. native subagent 실행은 금지한다.
2. HEAD/worktree·원격 main과 R1 문서 통합 상태를 확인한다. 기존 작업을 보존하며 제품·이미지 후보와
   문서 commit을 구분한다. actual workspace/cwd·모델/effort·소유 파일·증거 경로를 worker별로 기록한다.
3. WP-1에 줄 고정 읽기 명령과 SQL 집계를 검토한다. 필요한 read-only 접근이 없으면 독립적인
   WP-2 소스 분석을 먼저 진행하고 접근 문제를 구체적으로 기록한다.
4. 이번에 새로 필요한 사용자 결정은 U1/U2/U3의 실제 차이가 드러난 뒤 요청한다.
   값이 없다는 이유로 계획 작성·기존 권한 내 준비를 멈추거나 과거 승인을 반복 요청하지 않는다.

### G1 — 진단 통합과 구현 계약

1. WP-1/2의 실제 파일·hash·관측 시각과 현재 소스를 직접 대조한다. 과거 timer/daemon 상태만으로
   원인을 확정하지 않는다. 원인마다 증거·재현/반증·수정 필요 여부를 기록해 R2-01을 판단한다.
2. U1의 공급·담당·마감·부재 동작을 명시해 R2-02를 판단한다. U2의 현재 지원 세션·승인·갱신·trigger와
   U3의 누락 날짜·예산/upgrade 조건을 분리한다. 미확정 분기는 BLOCKED와 해소 조건/담당을 기록한다.
3. WP-3~5의 단일 목표, 재현, 정확한 파일 allowlist·expected outcome·테스트를 확정한다.
   공유 `dto.rs`·app wiring·migration 번호·catalog·lockfile이 필요하면 순서를 먼저 개정한다.
   DB schema 변경은 별도 migration 안전 명세가 없으면 시작하지 않는다.
   U2 대기 중 독립 WP-5 작업을 진행할 때는 현행 계약·반납된 공유 파일 소유권·후속 통합 검증을
   실행 기록에 먼저 고정한다. WP-4를 재개할 때 두 worker의 공유 파일 쓰기가 겹치지 않게 순서를 정한다.
4. 각 모델 배정의 검증 가능성을 확인한다. worker의 추측이 필요한 열린 문제는 코디네이터가 분리해
   brief/복잡도/그래프를 고친다. 동시 worker의 mutable scope는 겹치지 않아야 한다.

### G2 — 패치 통합과 후보 고정

1. WP-3/4를 통합한 뒤 WP-5를 실행한다. focused 회귀와 diff를 수용하고 필요 없는 패치는 제외한다.
   U2 대기 분기로 WP-5를 먼저 수행했다면 WP-4 재개 후 DTO·기준일·화면 통합 회귀를 다시 확인한다.
2. 구조 변경이 있으면 두 architecture 파일 중 영향받은 소스/evidence와 로컬 PNG를 같은 변경에 포함한다.
   공용 문서·도표·migration 통합은 코디네이터 소유로 처리해 동시 편집을 막는다.
3. `C_R2`를 exact 40자리 commit으로 고정한다. 실제 제품/build-input 변경이 없으면 C3 재사용의
   소스 동일성 근거를 남긴다. 변경이 있으면 새 R2 후보를 검증하며 C3 증거는 보존한다.
4. WP-6의 기존 run 조회 후 필요한 검증만 수행한다. 대상/CI 환경/브라우저/임시 DB가 고정돼야 한다.
   새 패치가 생기면 후보와 영향받은 검증을 갱신하고 필요 없이 동일 검사를 반복하지 않는다.

### G3 — 리뷰 수용과 이미지 준비

1. WP-7의 미해결 finding과 WP-6의 필수 미실행/실패가 없고 upgrade/복구 절차가 검토 가능해야 한다.
2. WP-8의 실행 직전 host health·동시 작업·메모리/swap/OOM·root 접근·no-clobber 경로를 검증한다.
   R1의 성공 관측이나 만료된 attestation은 admission이 아니다.
3. 이미지 작업의 기존 승인 범위를 확인한다. 추가 유지보수가 필요하면 코드·회귀·구체적 명령·영향·
   복귀안을 준비한 뒤 필요한 차이만 판단한다. 구현자의 임의 서비스 정지로 gate를 통과시키지 않는다.
4. 실제 12개 이미지·strict manifest·saved-byte 근거와 실제 terminal 종료를 직접 대조한다.
   release 설치와 별개인 이미지 검증 완료를 기록한다.

### G4 — 운영 적용 직전 판정

1. WP-8 결과를 바탕으로 **대상 revision/manifest, 조치별 정확한 명령과 효과, 실행 시각/예산,
   서비스 영향·중단 상한, 담당/복구 절차, 성공·중단 기준**이 채워진 runbook을 완성한다.
2. 기존 일일 수집/V2 승인은 유지한다. 새 maintenance·배포·migration·입력 승인 등 실제 요청 범위를
   넘는 차이가 있으면 준비 가능한 작업을 모두 마친 뒤 이 구체적인 실행안으로 Owner 결정을 받는다.
   root 권한 보유와 이미지 준비 승인을 서비스 전환·DB 변경의 승인으로 해석하지 않는다.
3. 실제 ledger/checksum/index와 데이터 의존 조건을 확인한다. DB dump·Raw/Curated의 서로 다른 생성
   시점을 하나의 일관된 snapshot으로 가정하지 않는다. 필요한 backup/격리 restore의 수행·정합성 확인을
   첫 변경 전에 끝내고, schema/role/RLS·secret·artifact 호환성의 부족한 근거는 명시한다.
4. 이전 release 선택만으로 DB·이미지·작업·설정이 모두 복구된다고 가정하지 않는다.
   파괴적 DOWN이나 운영 데이터 복원은 자동 rollback에 포함하지 않고 별도 판단 대상으로 남긴다.
5. 그날 KST 설치/활성화 가능 시각, exact missing sessions·외부 요청 상한, token/worker 충돌,
   현재 건강 상태와 승인 범위를 재확인한다. 조건이 바뀌면 영향을 받는 조치만 보류하고 문서를 갱신한다.

### G5 — 관찰 시작

1. WP-9의 실제 적용·health·작업 정산과 첫 데이터/추천 발행 증거를 수용한다.
   exit 0/74/75, healthy container, enabled timer 각각만으로 데이터 갱신을 완료 처리하지 않는다.
2. 예상 거래일을 현재 계약으로 정하고 대상 commit·모드·입력 계약·관찰 시작일·매일 확인 시각을 기록한다.
   최종 성공한 연속 구간은 같은 인수 후보로 묶는다. 수정/실패/수동 보충은 연속 구간 재시작 사유다.
3. Owner의 정상 로그인/허용된 시나리오 진행 시점을 조율한다. 사람의 사용 확인을 자동 성공으로 가정하지 않는다.

### G6 — Final end-to-end acceptance

1. R2-01~09 대응표의 각 행에 완료일·대상 revision·검증 링크·실제 결과·남은 한계를 붙인다.
   코드 구현, 격리 QA, 운영 적용, 실제 데이터 수신/발행, Owner 인수를 각각 판정한다.
2. 연속 **3거래일**의 예약 실행과 ETF 데이터·추천·등록 Owner 종목 갱신, 입력/as_of·API/화면 기준일이
   모두 일치해야 R2-08을 완료한다. 관찰의 누락/수동 보충/실패/Owner 미확인을 완료로 메우지 않는다.
3. 실제 설치 revision/image/manifest/모드와 backup/복구·Owner 사용 근거를 묶어 R2-09와 전체 R2를 판단한다.
   R3의 미완료는 독립적으로 남기며 Paper/실거래를 시작하지 않는다.
4. `docs/ROADMAP.md`, `docs/STATUS.md`, 이 계획과 실행 기록을 함께 갱신·커밋한다.
   운영 후 증거는 후속 문서 커밋으로 남기고 설치된 immutable release를 수정하지 않는다.
5. 모든 owned worker의 최종 보고와 실제 종료/대기 권한/남은 background 상태를 확인한다.
   실행 스킬에 따라 불필요해진 heartbeat를 정리하고 미완료 관찰은 다음 실행 시각·담당과 함께 인계한다.
   재사용할 검증된 결정/교훈만 Mem0에 저장하며 일일 로그·임시 진행 상태는 저장하지 않는다.

## 계획 변경과 일정

- WP 실행 중 계약·의존성·소유권이 달라지면 해당 분기를 보류하고 이 문서를 먼저 개정한다.
  대체 WP도 `$paseo-delegate`로 실행한다. worker에게 원래 범위 밖 설계를 조용히 맡기지 않는다.
- 첫 실행 묶음은 WP-1/2의 읽기 진단·계약 대조다. 결과를 수용하기 전 원인·배포일·완료일을 확정하지 않는다.
- 코드 수정량보다 U2 입력 승인, 실제 설치 시간창, 정상 Owner 사용 시점과 3거래일 관찰이 일정의 조건이다.
  휴장·시간창 경과를 테스트 실패나 수신 성공으로 처리하지 않는다.
- 이 계획 작성의 검증은 문서 형식·작업 ID/의존성/소유권·로컬 링크·Git diff 검사에 한정한다.
  계획에 적힌 코드/운영/브라우저 명령을 이번 요청에서 실행했다는 뜻이 아니다.
