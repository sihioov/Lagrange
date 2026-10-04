Execution skill: $paseo-delegate (required)
Native subagents: prohibited for worker packages

# KIS 30종목 실시간 시세 — 최종 완료 실행 계획

작성일: 2026-10-01 KST. 상태 갱신: 2026-10-04 KST. **C2 실제 역할 DB29, D4 runtime3, HTTP6·0057 rollback, grant SQL27·마이그레이션57 및 격리 initializer UID/mount13 인수. WP-7 실제 runtime/API/nginx/브라우저 양성 경로·30분 soak·자원 계측과 DB/전송/API/수요·권한·세대 장애의 한정 검증 인수. 달력/EOD/app-role 회귀6, 공유 REST 프로세스·V1/rest/off UI·설정 회귀, 복구 회귀13 및 실제 writer 커밋 후 새 자식 프로세스의 exact-source EOD 재개를 추가 인수했다. 독립 본 검토와 보완 검토는 SOURCE/LOCAL ACCEPT이며, Low 문서·OpenAPI 항목도 수정·검증했다. 아래 최신 증거 표의 한계를 적용하며, 최종 이미지·설치 helper·G1–G5 및 운영 인수는 여전히 미완료다. 과거 실패는 보존한다.**
기준 커밋: `17d643e92bd4b5c257c9745cfc6ec188ef9f4d05`.
대상 작업 경로: `/data/worktrees/3puw275b/needy-snake`.

이 문서는 [9월 21일 전환 계획](2026-09-21-kis-market-stream-transition.md)의
잔여 작업을 [9월 30일 인계](../../handoffs/2026-09-30-kis-market-stream.md) 이후부터
실행하도록 구체화한다. 기존 계획의 계약·안전 경계와 검증 이력을 보존한다.
현재 완료 상태와 재개 순서는 최신 인계와 이 문서를 우선하고, 구현 계약은
[market-stream 명세](../specs/2026-09-21-kis-market-stream-contract.md)를 따른다.
상충을 발견하면 코디네이터가 계약과 이 계획을 함께 수정한 뒤 해당 작업을 시작한다.

## Goal and boundaries

### 목표와 최종 완료 기준

목표는 **Owner가 고정 30종목 시세판을 열면, 공용 KIS 시장 WebSocket 수집을 통해
검증된 시세가 인증된 SSE와 실제 화면까지 전달되고 기존 EOD 수집도 유지되는 것**이다.
여기서 최종 완료는 이번 시세 전환의 완료다. 플랫폼 전체의 Paper·실거래 출시는 포함하지 않는다.

다음을 모두 증명해야 `FINAL_ACCEPTED`로 판정한다.

1. 정상 Owner API로 정확한 고정 30종목을 멱등 등록한다. `005930.KRX`가 실제 READY가
   된 다음 나머지 29개를 순차 처리하며, 기존 READY·generation은 재사용한다.
2. 30종목 모두 실제 시장 구독 ACK를 받는다. 제외·거절·등록 미완료가 남으면 전체 완료가 아니다.
3. 같은 30종목을 보는 1개/10개 소비자가 상류 구독을 공유한다. 정상 WS 모드의 반복
   REST 현재가 요청과 브라우저 5초 quote polling은 모두 0이다.
4. 실제 활발한 pilot 종목에서 첫 표시 뒤 최소 두 번의 새 수신을
   `epoch/quote_version/received_at`으로 DB·SSE·DOM까지 대조한다. 가격 숫자가 같아도
   새 수신을 식별한다. 거래가 없는 다른 종목에는 체결을 만들어내지 않는다.
5. 정상 부하에서 서버 수신→화면 반영 p95 ≤ 2초를 측정한다. 최소 30분 로컬 통합
   soak와 별도 운영 관측을 구별하고, 병합된 업데이트 수·누락·지연도 함께 보고한다.
6. 재연결, DB 장애, 느린 소비자, 알림 유실, 로그아웃·권한 철회, 숨김·offline,
   마지막 탭 종료, generation·거래일 변경을 검증한다. 모호한 값은 live로 표시하지 않는다.
7. 같은 유효 거래일·동일 calendar 출처의 실제 EOD 게시를 확인한다. 다음 거래일의
   유효 proof 공급 책임·절차와 로컬 rollover 검증도 갖춘다. 자동 공급이 없다면 수동
   운영 조건을 명시하고, 공급 경로 자체가 없으면 운영 지속성은 미완료다.
8. 검증된 exact commit의 이미지·env·manifest·설치본이 일치하고, migration·복구 절차,
   서비스 health와 두 구조 도면의 근거·PNG가 일치한다. 최종 사용자 확인 가능한 결과를 제시한다.

판정은 `SOURCE_LOCAL_ACCEPTED`, `INSTALLED_OFF`, `LIVE_ACCEPTED`, `FINAL_ACCEPTED`로
구분한다. 로컬 테스트 PASS나 이미지 설치를 전체 완료로 대신하지 않는다.

### 확인한 기준선

| 경계 | 기준 커밋에서 확인한 사실 | 다음 작업 |
|---|---|---|
| WS-2 | 시장 전용 approval/state/transport/wire 구현과 synthetic/loopback 검증 이력 존재 | 기존 계약 보존, 변경 영향만 재검증 |
| C3A/C3B | stream repository, 0055, owning producer 존재. 인계상 original1–26와 pacing 결과 채택 | 전체 WS3B 완료 여부를 별도 판정 |
| 실행 진입점 | `crates/job-queue/src/bin/owner-equity-v2-runner.rs:777`은 기존 `IntradayProducer`만 시작 | transport 선택과 새 runtime 연결 |
| producer 접점 | `market_stream_producer.rs:579`는 수신 후 게시까지 한 경로. 상태 저장 호출은 `:879`의 test 전용 bridge | 취소 안전한 지속 실행·병합 게시·상태 게시 계약 확정 |
| API | `crates/api-server/src/http/mod.rs:343`에 기존 demand/종목별 GET 경로 | schema2 lease/SSE 추가 |
| Web | `apps/web/lib/products/intraday-quotes-contracts.ts:10`은 5초 polling | 탭당 하나의 30행 stream controller |
| Ops | 기존 REST overlay/validator 존재. 신규 WS provisioning/grant/activation 연결 없음 | 별도 보호 상태와 immutable 운영 경로 |
| 운영 관측 | 9월 19일 설치·서비스 기록은 과거 스냅샷 | 새 실행 시 현재 상태 읽기 전용 재확인 |

최종 producer 및 producer-tests의 SHA-256은 각각
`9ebc67d81f8ea27a5d134cb454c00bfa7e56fc11f39af7a93c100f622ce13f62`,
`15db864f07de9c9f52a3868dd357f7cbc32e6cccd93f684d143743026a50d645`다.
인계의 원본 보고서 네 개와 위 두 파일은 계획 전 분석에서 해시 일치를 확인했다.
마지막 original20–22와 23–26의 PASS는 서로 다른 실행 결과이며 전체 suite의 단일 실행이 아니다.

### 적용 지침과 실행 경계

- 읽은 지침: `/home/l1nnx/.codex/AGENTS.md`, 저장소 `AGENTS.md`,
  `apps/web/AGENTS.md`, `apps/web/CLAUDE.md`,
  `/home/l1nnx/.agents/skills/paseo-delegate-plan/SKILL.md`,
  `/home/l1nnx/.agents/skills/paseo-delegate/SKILL.md`.
  상위 `/data` 경로의 별도 AGENTS/CLAUDE와 저장소 루트 CLAUDE는 확인되지 않았다.
- 최초 요청에서는 계획만 작성했다. 2026-10-01 사용자가 `$paseo-delegate 작업 진행 시작해`로
  후속 실행을 지시했다. 모든 WP는 `$paseo-delegate`로 실행하며 아래 gate를 유지한다.
  해당 스킬 파일은 현재 존재한다.
  실행 시 최신 스킬과 Paseo MCP·profiles·모델/effort 지원을 확인한다. MCP가 없으면 작업자
  시작 전에 그 분기만 중단하고 보고하며 native agent나 CLI launch로 대체하지 않는다.
- 코디네이터는 계획·계약 채택·권한 판단·통합·최종 검증을 맡는다. 구현과 실질 조사,
  독립 검증은 아래 WP가 맡으며 작업자는 재위임하지 않는다.
- 기존 권한과 결정을 먼저 대조한다. 동일 범위의 승인을 반복 요청하지 않는다.
  새 운영·네트워크 권한이 필요하면 검증된 후보, 정확한 변경·명령·영향·복구안을 준비한 뒤
  부족한 항목만 요청한다. 계획 문서는 그 권한을 새로 만들지 않는다.
- 계좌·잔고·주문·주문 통보, live-order profile, 신규 전략, 전 종목/분봉 수집,
  자동 KIND/KRX 수집, REST fallback 추가는 범위 밖이다. 기존 KIS/OpenDART 경계를 유지한다.
- 비밀·broker 원문·실제 payload·보호 상태·실행 승인 파일은 Git과 진단에 남기지 않는다.
  실데이터는 기존 권리가 허용한 저장 경로에서만 취급하고 QA는 버전·시각·정형 상태·카운터를 기록한다.
- 인계 시점의 종료·DB 부재·임시 승인·wrapper 기한은 과거 사실이다. 과거 실행기,
  one-use entry, worker/heartbeat를 재사용하지 않는다. 미확인 과거 실패 DB를 임의 조사·삭제하지 않는다.
- original1–26를 작업명 변경 때문에 반복하지 않는다. 새 구현이 영향을 준 경계를 매핑해
  필요한 회귀를 실행한다. 최종 통합 검증에 필요한 영향 범위는 생략하지 않는다.

### 남은 운영 조건 — 코드 작업과 독립적으로 추적

아래는 9월 21일 계약의 잔여 조건이다. 현재의 공식 한도나 승인 여부를 새로 확인했다는 뜻은 아니다.
WP-2는 확인 가능한 사실과 필요한 결정안을 준비하며 코디네이터가 채택한다.

| Gate | 필요한 결과 | 미해결일 때 |
|---|---|---|
| G1 네트워크 | 계약 후보인 `POST https://openapi.koreainvestment.com:9443/oauth2/Approval`와 `ws://ops.koreainvestment.com:21000/tryitout`, 시장 `H0STCNT0`의 정확한 승인. 평문 전송 조건도 명시 | 실제 접속 금지, 로컬 작업 계속 |
| G2 현행 한도 | 당시 미확인 9월 16일 공지와 최신 공식 자료, 단일 slot의 다른 사용 여부, 30구독 수용성 | 수용성 미확정. 30 미만이면 사용자 제품 결정 필요 |
| G3 권리 | 기존 개인 Owner 범위가 신규 WS/cache/SSE를 덮는 정확한 amendment·reference/hash·기간 | synthetic grant만 사용, 실제 grant 설치 금지 |
| G4 운영 입력 | 당일 calendar/window proof, 다음 유효일 공급 절차, exact release, backup/restore, 리소스·reader 소유권 | 해당 배포·활성화 단계 대기 |
| G5 실제 프로토콜 | 허용된 pilot으로 실제 47필드/ACK/해제/heartbeat·quota 확인 | WS-2 synthetic 결과로 대신하지 않음 |

9월 19일 미확정 calendar claim을 재시도·삭제·초기화하지 않는다. 날짜 변경을 새 호출
승인으로 해석하지 않는다. `configs/market-hours/krx-intraday-session-windows-v1.json`의
빈 entries는 현재 거래일 proof를 제공하지 않는다.

## Initial classification

작업 크기보다 불확실성·결합·실패 비용으로 먼저 분류했다. 아래 모델 선택은 그 다음 단계다.

| Package | Complexity | Basis | Confidence | Reclassification or escalation signals |
|---|---|---|---|---|
| WP-1 | hard | 기존 취소 안전 facade와 병합 게시·상태·실행 루프 사이 계약을 고정해야 함 | high | C2/DB 공개 경계 변경이 필요하면 정확한 변경을 별도 채택한 뒤 구현 허용 |
| WP-2 | hard | 공식 자료 상충·현행성 및 기존 권한 적용 범위를 근거로 판정 | medium | 현행 자료 회수 실패·30 미만·기존 승인 충돌은 미해결로 보고, 추정 금지 |
| WP-3 | hard | 계약 고정 후에도 socket/DB lease/fence/취소/부하가 결합됨 | medium | 같은 수정 2회 실패, 반복 race, 계약 밖 변경 필요 시 중단·상향 |
| WP-4 | intermediate | 고정 DTO·기존 인증/RLS에 SSE 수명과 버전 복구를 추가 | high | 인증 철회·snapshot race 검증 반복 실패 시 상향 |
| WP-5 | intermediate | 고정 계약 위에서 30행 UI·단일 controller·정상 등록 경로 연결 | high | 실제 DOM과 fixture 차이, demand 중복, lifecycle 회귀 반복 시 상향 |
| WP-6 | intermediate | 기존 immutable helper에 정해진 mode/state/grant 절차를 추가 | medium | 초기화 계약 부재·권한 보존 실패·수동 설치본 패치 필요 시 반환 |
| WP-7 | hard | 실제 socket→DB→인증 API→nginx→브라우저의 독립 인수 판단 | high | 보고서와 실행 증거 불일치·회귀·자원 초과는 REJECT 후 원 소유자 수정 |
| WP-8 | hard | 운영 호스트의 자원·이미지·env·DB 호환성과 서비스 상태가 결합 | medium | OOM·reader 미확인·commit 불일치·호환 실패면 즉시 영향 작업 중단 |
| WP-9 | hard | 실제 시장·Owner 세션·30구독·수신·EOD를 함께 입증해야 함 | medium | 권한/날짜/한도 불일치·quiet pilot·인증 미확보면 정확한 미완료를 보고 |

모델 규칙: 명세가 고정된 구현·테스트는 `codex/gpt-6-luna`, effort `max`에서 시작한다.
독립 리뷰·판정은 `codex/gpt-6-sol`, `high`; WP-1의 열린 runtime 구조 판단은
`codex/gpt-6-astra`, `xhigh`를 사용한다. WP-8은 WP-7이 검증한 정확한 절차를 실행하고
판단은 코디네이터가 하므로 Luna max로 시작하되, 독립 검증 없이 운영 판단을 맡기지 않는다.
검증 실패·반복·맥락 누락이 관찰되면 Luna→Sol→Astra 한 단계씩 상향한다.
모델과 effort는 동시에 올리지 않는다. 분류가 hard라는 이유만으로 모든 구현을 상위 모델에 주지 않는다.
지원 조합이 없으면 `resolve at execution from current Paseo availability`로 남기고,
현재 profiles/notes와 적용 지침으로 결정하며 조용히 대체하지 않는다.

## Execution graph

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
|---|---:|---|---|---|---|---|---|---|
| WP-1 | 1 | hard | R111 gap과 runtime 구현 계약 확정 | 새 gap 문서, 기존 stream spec의 필요한 계약 보완 | checkpoint·인계 | codex/gpt-6-astra / xhigh | 요구사항→코드→증거→잔여 표, 구현 manifest | 코드/인계 대조, 코디네이터 채택 |
| WP-2 | 1 | hard | live 조건의 사실·결정안 준비 | 새 live-readiness 문서, 공개 공식 자료 읽기 | 기존 G1–G5·권한 이력 | codex/gpt-6-sol / high | 출처/미확인/정확한 승인 차이·운영 입력 목록 | 공식 근거와 기존 결정 대조, provider 호출 0 |
| WP-3 | 2 | hard | WS3B runtime 완성 | job-queue runtime/runner/repository, 한정 kis config·초기화, 관련 tests | WP-1 채택 | codex/gpt-6-luna / max | 실행·상태·병합 게시·lifecycle와 부하 증거 | 실제 loopback + 실제 역할 DB·영향 회귀 |
| WP-4 | 3 | intermediate | 인증 lease/SSE와 nginx 연결 | Rust API, OpenAPI, nginx, API tests | WP-3 채택·고정 schema2 | codex/gpt-6-luna / max | 실제 인증 SSE·resync·철회 | 실제 HTTP/RLS/nginx tests |
| WP-5 | 3 | intermediate | 30행 화면·등록·단일 탭 controller | Web, Owner acceptance QA trio, Web tests | WP-3 채택·고정 schema2 | codex/gpt-6-luna / max | 30행/상세·자동 등록·lifecycle | typecheck/Vitest·실제 React/Chromium |
| WP-6 | 3 | intermediate | 운영 설정·초기화·grant·복구 도구 | ops/compose/env/Docker·runbook·helper tests | WP-3의 mode·초기화 인터페이스 | codex/gpt-6-luna / max | 공식 helper와 운영 실행안 | disposable 실제 helper·fake Docker·권한 검증 |
| WP-7 | 4 | hard | 통합 후보 독립 인수 | QA harness/tests와 비공개 증거, 제품 코드는 읽기 전용 | WP-4/5/6, 코디네이터 통합·도면 | codex/gpt-6-sol / high | ACCEPT/REJECT·종단/soak/회귀 matrix | 실제 통합 환경, >=30분 soak |
| WP-8 | 5 | hard | 검증 후보 이미지·설치·off 상태 준비 | 승인된 exact commit의 운영 build/install/provision/migration만 | WP-7 ACCEPT, 최신 운영 preflight·필요 권한 | codex/gpt-6-luna / max | 12-image manifest, 설치·호환·off 증거 | 이미지/env/commit, 자원/DB/서비스 검증 |
| WP-9 | 6 | hard | 실제 30종목 시세와 EOD 최종 인수 | 승인된 grant/activation·정상 Owner API·실화면·EOD·복구 | WP-8, G1–G4 충족 | codex/gpt-6-sol / high | G5 및 운영 인수 matrix, 사용자 확인 결과 | pilot→30 ACK→2수신/DOM→lifecycle→EOD |

WP-2의 live 조건이 미해결이어도 WP-1/3/4/5/6/7은 로컬 범위에서 진행할 수 있다.
Wave 3의 세 작업은 서로 다른 파일을 소유하며 같은 고정 명세를 입력으로 사용한다.
계약 변경이 필요해지면 병렬 작업을 그대로 밀어붙이지 않고 영향 작업을 정지·재계획한다.
동시 작업자 수는 실제 Paseo 가용성과 호스트 자원 안에서 최대 3개로 제한한다.

### 병렬 소유권과 공용 파일

- WP-1만 기존 stream spec을 편집한다. 채택 후 freeze하고 이후 변경은 코디네이터가 순서를 다시 정한다.
- WP-3만 Wave 2의 Rust manifest/lock·새 runtime/초기화 코드를 편집한다. API가 추가 Rust
  의존성을 요구하면 Wave 3에서 WP-4만 `Cargo.lock`과 API manifest를 편집한다.
- WP-4만 `apps/api-server/openapi.json`, `generated/openapi.ts`, generator/checker 및
  `deploy/nginx/nginx.conf`를 소유한다. WP-5는 자기 strict schema2 client 계약을 구현하고
  아직 생성되지 않은 API 타입에 의존하지 않는다. 합치기 전 양쪽 계약의 일치를 검사한다.
- WP-5만 Web·필요 npm lock을 소유한다. 의존성 추가는 먼저 필요성을 보고하며 기본은 기존 의존성 재사용이다.
- WP-6은 nginx/API/Web/Rust/migration을 편집하지 않는다. 초기화 binary의 변경은 WP-3에 반환한다.
- 전체 빌드·공유 계약 검사는 관련 파일 편집이 끝난 안정된 트리에서 수행한다.
  Cargo와 Rust 빌드는 공통 lock으로 직렬화하고 `CARGO_BUILD_JOBS=2`, `--locked --offline`을 유지한다.
- 구조 도면과 최종 커밋 통합은 코디네이터 소유다. 소유권 밖의 수정은 먼저 범위·그래프를 갱신한다.

## Worker briefs

### 모든 배정에 실제로 포함할 공통 문맥

각 배정은 아래 공통 문맥과 해당 brief를 **함께** 전달한다. 대화 이력이나 다른 worker의
메모리를 가정하지 않는다. 실행자는 현재 accepted commit, 입력 문서의 절대경로,
허용 파일 목록, 검증 대상·시간·종료 범위만 추가할 수 있고 scope를 조용히 넓힐 수 없다.

> Do not use native subagent, Task, Agent, team, or delegation features. Complete this assignment directly and report if it needs further decomposition.

- cwd는 `/data/worktrees/3puw275b/needy-snake`. 격리가 필요하면 `$paseo-delegate`에 따라
  승인된 Paseo workspace를 사용하고 실제 workspace/부모 관계를 검증한다.
- 필수 입력은 이 계획, 절대경로로 전달한 9월 30일 인계와 9월 21일 spec, 적용 AGENTS다.
- source/local 검증에 실제 KIS·운영 credential·운영 DB를 쓰지 않는다. fake provider는
  loopback 전용 test 경계에만 둔다. production URL override나 공개 proof factory를 만들지 않는다.
- DB 검증은 모든 migration과 실제 bootstrap/역할을 적용한 새 task-owned 환경을 쓴다.
  `LAGRANGE_WS3A_SUPERVISOR_URL`의 명시적 전용 입력을 사용하고 `DATABASE_URL` fallback을 금지한다.
  적용 권한·Unix socket/port·cluster identity·source pins·종료/정리 범위를 현재 기준으로 정한다.
- private evidence 경로와 Cargo lock은 실행 시 실제 쓰기 권한·충돌 여부를 확인해 정한다.
  과거 `/data/agent-tmp/lagrange-kis-stream-20260921` 증거는 읽기 전용 이력이며 재실행 입력이 아니다.
  `/tmp` 쿼터/메모리와 build process를 확인하고 무거운 테스트를 중복 실행하지 않는다.
- 누락·모순은 정확히 보고한다. 검사 명령이 0 tests를 선택하거나 DB 없이 skip했다면 PASS가 아니다.
- 필수 보고: ① 변경 파일·라인 범위 ② brief와 다른 처리·이유 ③ 실제 검사 명령·결과·선택된
  테스트 수·증거 위치 ④ 미해결/후속 ⑤ 못 찾았거나 검증하지 못한 것. 빈 항목은 `없음`을 명시한다.
  synthetic/local/production을 구분하고 과거 실패 이력을 소급 수정하지 않는다.

### WP-1 — 실행기 연결 전 계약과 남은 범위 확정

- **cwd/분류/선택:** `/data/worktrees/3puw275b/needy-snake`; hard, high confidence;
  Astra xhigh. 취소·소유권·병합 저장의 설계 판단이 필요한 좁은 작업이다.
- **알려진 사실:** `OwnerMarketStreamProducer`는 `start/reconcile_desired/read_and_publish`를
  제공한다. 지속 실행·재연결은 없고 상태 저장 연결은 test bridge에 한정된다. 계약 §7은
  250ms 병합 게시를 요구한다. runtime을 기존 async 함수에 단순 연결한다고 완성되지 않는다.
- **목표/입력:** 원본 spec §5–9, 인계의 C1/C2/C3A/C3B 증거와 코드로 R111 표를 완성한다.
  구현 없음/구현 있으나 미검증/채택 완료를 나누고 이미 채택한 테스트를 보존한다.
- **소유 파일:** 신규 `docs/reviews/2026-10-01-kis-market-stream-completion-gaps.md`,
  필요한 `docs/superpowers/specs/2026-09-21-kis-market-stream-contract.md` 보완만.
  제품 코드·DB/runtime·운영·다른 계획 문서는 편집하지 않는다.
- **확정할 설계:** socket 수명과 DB lease 갱신, read/command 취소와 terminal facade 처리,
  authentic receipt를 유지한 bounded coalescing/commit, production 상태/gap 게시,
  snapshot/notification seam, 날짜 전환·proof 재취득 없이 정지하는 경계.
- **추가 접점:** production state의 one-shot initializer는 현재 test 초기화와 다르다.
  전용 binary/library 위치·입력·root/runtime UID 분리·원자 초기화 계약을 정확히 고정해
  WP-3 코드/WP-6 installer 사이 소유권 공백을 없앤다. 새 일반 공개 setter는 허용하지 않는다.
- **산출물/검증:** 요구사항별 파일:라인·증거·결손·담당 WP 표, 공개/비공개 signature와
  runtime 상태 전이, WP-3의 정확한 수정 manifest·focused regression 목록. 코드와 인계
  해시 대조만 수행하며 기존 DB tests를 새로 실행하지 않는다. 코디네이터가 채택해야 WP-3를 시작한다.
- **확대 신호/보고:** frozen C2나 0055의 변경이 필요하면 이유·최소 변경·검증을 명시하고
  채택 전 구현하지 않는다. 공통 보고와 미확정 접점 목록을 반환한다.

### WP-2 — 실제 접속·지속 운영 조건 준비

- **cwd/분류/선택:** `/data/worktrees/3puw275b/needy-snake`; hard, medium confidence;
  Sol high. 현행 공식 근거와 기존 승인 적용의 판정이 산출물이다.
- **목표/입력:** G1–G5, ADR-0005, 기존 session/calendar/immutable runbook의 허용 범위를
  대조해 실제 접속 전 필요한 결정과 입력을 한 번에 검토할 수 있게 한다.
- **소유 파일:** 신규 `docs/runbooks/kis-market-stream-live-readiness.md`만. 공개 공식
  문서·공지·공식 sample revision을 읽는다. 보호 운영 입력 접근은 별도 현재 권한 범위에서만 한다.
  spec/AGENTS/entitlement/secret 수정, approval 발급·WS 접속·DB 변경은 금지한다.
- **조사:** 실제 method/host/path/TR, 평문 조건, 47필드, 최신 구독/연결 한도, 단일 slot의
  다른 client 사용, WS/cache/SSE 권리, 당일·다음 유효일 proof 공급과 Owner 정상 인증 경로.
  문서로 알 수 없는 host/client 사실은 실행 시 확인할 항목으로 남긴다.
- **산출물/검증:** 출처 날짜·revision·인용 위치, 승인된 항목/부족한 차이, 보호 입력의
  schema와 배치 절차, G1–G4 preflight checklist. 보안값이나 가짜 production pin을 만들지 않는다.
  확인되지 않은 한도를 과거 공지 숫자로 확정하지 않는다. live-provider 호출은 0이어야 한다.
- **확대 신호/보고:** 공식 자료 상충·회수 실패·30 미만·다음날 proof 공급 불가를 명시한다.
  코디네이터가 사용자 결정이 필요한 차이만 채택하며, 이 결과 때문에 독립 로컬 작업을 중단하지 않는다.
  공통 보고에 각 gate의 해결/미해결과 재확인 시점을 추가한다.

### WP-3 — 공용 수집 runtime과 실행기 완성

- **cwd/분류/선택:** `/data/worktrees/3puw275b/needy-snake`; hard, medium confidence;
  Luna max. WP-1에서 빠진 계약이 있으면 시작하지 않는다. 명세가 확정된 구현·테스트만 맡는다.
- **입력/사실:** 기존 producer/DB/transport와 original1–26는 보존할 기준선이다.
  runner는 아직 REST producer를 시작한다. 새 runtime은 기존 EOD/reference 경로와 병존해야 한다.
- **소유 파일:** 신규 `crates/job-queue/src/owner_equity_v2/market_stream_runtime.rs`와 tests,
  기존 `market_stream.rs`, `market_stream_producer.rs`, `owner_equity_v2.rs`,
  `src/bin/owner-equity-v2-runner.rs`, 관련 test/support, Rust manifest/lock,
  `crates/kis-client/src/read_coordination_config.rs` 및 exports.
  초기화 진입점과 frozen C2 최소 변경은 WP-1에서 채택한 정확한 파일·hunk만 허용한다.
  root는 0055 배포 여부를 확인하지 않았으며 새 `0056_owner_market_stream_runtime.{up,down}.sql`
  추가를 채택했다. 기존 migrations0001–0055, API/Web/ops, REST ledger는 변경하지 않는다.
- **구현:** `off|owner_only`와 `rest|market_ws`를 구분하고 기본 off/rest 유지.
  `--once`는 stream을 시작하지 않는다. WS 모드는 REST 반복 quote producer와 배타적이다.
  socket 단일 소유, lease TTL20s/5s 갱신, demand 1s 이내 조정, fresh epoch/ACK,
  bounded backoff, DB/권한/날짜 실패 시 정지, 연결·gap·무체결 상태 게시를 연결한다.
- **처리량:** 30 latest slots, 두 개의 최대30 관측 batch, 상태64개, stream-owned heap ≤8MiB;
  250ms 병합·초당 최대4 quote transaction/120 row updates를 지킨다. 미commit 메모리를
  API로 우회 전달하지 않는다. 취소된 facade를 되살리거나 raw proof factory를 추가하지 않는다.
- **검증:** 기존 lock/jobs2 아래 `cargo test --locked --offline -p job-queue --lib
  --features market-stream-db-tests <확정한_runtime_filter> -- --test-threads=1` 및 실제
  runner target의 focused tests. 필터는 새 실제 test 이름과 개수를 명시한다.
  실제 역할 DB+loopback으로 30종목, 1/10 소비자 동일 upstream, REST quote0, 느린 DB,
  lease/generation/revoke/rollover/실패/취소를 입증한다. initializer와 config의 default-build 경계도 검사한다.
- **산출물/상향/보고:** runtime 구현과 계약 대비 증거, 초기화 binary의 확정 interface,
  source pin·영향 회귀. 동일 수정 두 번 실패·race 반복은 한 단계 상향한다.
  공통 보고에 read/write/queue/memory/command 카운터와 검증하지 못한 runtime 분기를 추가한다.

#### WP-3 순차 실행 단위 — 2026-10-01 root 채택

WP-1의 [R111 검토](../../reviews/2026-10-01-kis-market-stream-completion-gaps.md)와 명세 §13–14를
source/local 계약으로 채택했다. 0055의 미배포를 추정하지 않고 새0056만 허용한다.
WP-2 자료의 최신 공지 회수 실패와 G1–G5 미완료는 로컬 구현을 막지 않는다.
각 단위는 실제 idle·diff·검증을 코디네이터가 회수한 뒤 다음 단위를 배정한다.
아래는 기존 WP-3의 분해이며 새 제품 범위나 운영 권한이 아니다. 모두 Luna max로 시작한다.

| 단위 | 분류/확신 | 목표와 소유 경계 | 선행/검증 |
|---|---|---|---|
| WP-3A | hard/medium | kis-client C2 연결 잠금 선취·bounded idle와 transport parser | 채택 §13.1; V2/V3의 C2·config 부분, 기존 transport/domain/default API 회귀 |
| WP-3B | intermediate/high | §14 initializer module/bin·state의 private 생성 지원·feature | WP-3A 검토; V11의 disposable filesystem/CLI 및 default feature 경계 |
| WP-3C | hard/medium | 새0056, grant/status/retire/snapshot 및 private day/lineage 저장 접점 | WP-3B 검토; V8–V10/V12 실제 역할 DB, 다음 runtime에 필요한 정확한 내부 타입을 배정 때 고정 |
| WP-3D | hard/medium | producer private coalescing/writer, supervisor·runner와 전체 WP-3 검증 | WP-3C 검토; V1–V10 및 변경 영향 original/pacing 회귀 |

WP-3A의 이번 허용 파일은 `crates/kis-client/src/{market_stream,read_coordination_config,lib}.rs`,
`crates/kis-client/tests/{market_stream_transport,market_stream_domain}.rs`뿐이다.
`reserve_connection`→affine owner의 consuming `connect(proof)`와 내부 deadline을 가진
`next_event_until`을 구현한다. 기존 `connect()`/`next_event()` 및3인자 config parser는 보존한다.
partial/fragment 버퍼, 진행 중 control write, poison-before-queued-receipt, authentically captured
ACK/receipt, command·재접속 예산과 terminal 취소를 유지한다. 일반 timeout으로 read future를
끊거나 소켓을 split하지 않는다. `IntradayQuoteTransport::from_optional_str`는 기본 Rest이며
정확한 `rest`/`market_ws` 이외 입력을 거부한다. 새 dependency·state schema·runner·DB 변경은 없다.

이번 검증은 task-owned synthetic 상태와 loopback listener만 허용한다. Cargo는
`/tmp/lagrange-kis-live-cargo.lock`의 배타 잠금, `CARGO_BUILD_JOBS=2`, `--locked --offline`,
workspace의 실제 디스크 `target/`를 사용한다(`/tmp`는 tmpfs이므로 build target 금지).
기존 compiler 부재·메모리/디스크를 확인하고 동일 시점 Rust build는 하나만 실행한다.
kis-client lib/default doctests, test-support transport/domain tests와 새 회귀의 실제 선택 수·결과를
보고한다. 오류 수정에 필요한 재검증만 반복한다. DB, Docker, 운영 상태/자격증명, KIS 접속,
서비스, 배포, commit/push, 추가 worker와 heartbeat 생성은 WP-3A 범위 밖이다.

WP-3A 회수·검토(2026-10-01): 실제 idle/권한 대기0, 허용된4파일 diff와 소스 해시를 확인했다.
private `wp3a-report.md`의 lib174/doc26/transport50/domain9 및 마지막 날짜 바인딩 focused1
통과 보고를 코드·실행 활동과 대조했다. 연결 예약·소비 connect·bounded idle·parser를
이 단위의 source/local 결과로 채택한다. DB lease 결합이나 전체 runtime 인수는 아니다.
state/wire/approval 및0001–0055는 기준선 그대로다. 변경 type seam의 후속 통합은 WP-3D/7이 검증한다.

WP-3B의 허용 파일은 `crates/kis-client/src/market_stream_state.rs`의 feature 한정 private 생성
접점, 신규 `src/market_stream_provisioning.rs`, 신규 `src/bin/kis-market-stream-state.rs`,
`src/lib.rs`, `Cargo.toml`, 신규 `tests/market_stream_provisioning.rs`다. 기존
`tests/market_stream_domain.rs`는 새 private 경로의 영향 회귀에 필요한 경우만 변경한다.
WP-3A transport/config 파일, state schema·runtime replacement/repair semantics,
wire/approval, Cargo.lock·새 dependency, job-queue/DB/API/Web/ops/docs는 변경하지 않는다.

명세 §14의 fixed-path production API와 두 CLI subcommand만 구현한다. 실제 초기화·validate를
현재 host의 `/run/lagrange`에 실행하지 않는다. 양성 filesystem 검증은 `cfg(test)` private
fixture seam에서 task-owned parent와 현재 UID/GID를 주입해 같은 생성/검증 코드를 호출한다.
production feature에는 경로/소유자 override나 test factory를 노출하지 않는다. CLI는 canonical
입력·허용되지 않은 flags·현재 비root 호출의 경로 접근 전 거부를 검증한다. generation 인자는
CLI에서 현재 `KIS_READ_CREDENTIAL_GENERATION`의 canonical positive 값과 일치해야 한다.
실제 root/10001·mount 검증은 WP-6이며 fixture 성공으로 대신하지 않는다.

모든 신규 함수는 default feature에서 제외한다. root 전용 `initialize-new`는 두 leaf 모두
없는 경우에만 descriptor-relative no-follow 생성, 최종 ownership/mode, 서로 다른 anchor,
connection→state lock, temp0600·no-replace 설치·fsync·재검증을 수행한다. `validate-existing`은
읽기 전용이며 bytes/inodes/budget/uncertainty를 보존한다. 부분 실패·존재·손상·symlink/hardlink·
generation/domain 불일치는 typed error로 종료하고 repair/delete/retry하지 않는다.
테스트는 같은 Cargo lock/jobs2/offline/디스크 target을 유지한다. 기존 lib/doc/domain 및
provisioning feature의 lib/bin/integration tests, default-feature closure와 fsync/no-replace 실패
회귀를 실행하고 실제 stdout/exit/선택 수를 mode0600 private logs로 남긴다.

WP-3B 배정 상태(2026-10-01): 코디네이터가 WP-3A 검토를 마치고 위 bounded 로컬 작업을
MCP로 배정하려 했으나 자동 승인 검토가 두 번 거절했다. 기존 사용자 실행 지시·계획과
실제 idle/검토 기록을 재확인해 같은 MCP 요청에 제시했지만, 검토는 이를 WP-3A 복구 범위를
넘는 작업으로 판단하고 코디네이터 gate 기록을 승인 근거로 인정하지 않았다.
두 거절 당시 작업자는 생성되지 않았으며 다른 실행 경로로 대체하지 않았다. 이후 사용자가
“묻지말고 계속 승인 진행해”라고 직접 승인했다. 위 코드·격리 로컬 테스트 범위로 동일
skill/MCP 배정을 재개하고, 기존 계획의 같은 승인 범위는 코디네이터 검증 후 재확인 없이
이어간다. 운영 초기화·실제 접속의 별도 gate는 유지한다.

WP-3B 작업자 `9da61880-3a92-43a9-ba69-af2d696dcad6`를 같은 MCP 경로로 생성했다.
실제 runtime의 `codex/gpt-6-luna`, `max`, `auto-review`, workspace·parent와 실행 상태를
확인했다. 완료 알림 및 2분 간격의 범위 한정 복구 heartbeat를 등록했다. source diff와
실제 테스트 로그 검토 전까지 이 패키지는 미완료다.

WP-3B 회수·검토(2026-10-01): 실제 idle/active turn 없음/권한 대기0을 확인한 뒤 허용6파일의
diff·전체 신규 코드·보고서 해시·실제0600 로그를 대조해 이 단위의 source/local 결과를 채택했다.
provisioning lib186/bin2/integration3/doc28, 기본 lib174/doc26, domain9가 모두 통과했다.
기본/production-feature 바이너리 빌드도 성공했다. root는 Cargo를 재실행하지 않았다.
기존 WP-3A pin과 Cargo.lock·wire/approval·producer·0055가 보존됐다. 실제 UID0/10001,
mount/설치 검증은 WP-6이며 전체 runtime 인수는 아니다. 보고 회수 후 복구 heartbeat를 종료했다.

#### WP-3C source/DB 실행 분리 — 코디네이터 gate

기존 실제 역할 DB fixture는 과거 클러스터의 system identifier/data directory/port에 고정돼
있다. 과거 승인·wrapper·실패 DB를 재사용하지 않기 위해 WP-3C를 다음 두 단위로 순차 수행한다.
사용자의 전체 실행 지시 및 “묻지말고 계속 승인 진행해”의 동일 범위 진행 승인에 따른 분해다.
새 제품·운영·provider 권한은 추가하지 않는다.

- **WP-3C1 (hard/medium, Luna max):** §13의 새0056·저장소·안전한 delivery DTO·private
  runtime config/day 입력 접점과 실제 역할 DB 테스트 소스를 구현한다. 소스/오프라인 빌드와
  DB 없는 검증만 실행한다. DB 테스트는 compile-only이며 실행 PASS로 표현하지 않는다.
- **WP-3C2:** C1 diff를 검토한 뒤 새 작업 전용 PG의 생성·식별·역할 로그인·전체56 migration·
  exact cleanup 실행안을 고정한다. 기존 역사적 guard를 우회하지 않는 새 test-only 입력을
  검토하고 V8–V10/V12 및 실제 영향 회귀를 실행한다. 이 검증 전 WP-3C는 미완료다.
  실행 명세가 정해질 때 모델·effort를 기존 분류 규칙으로 선택한다.

C1 허용 파일: 신규 `migrations/0056_owner_market_stream_runtime.{up,down}.sql`,
`crates/job-queue/src/owner_equity_v2/market_stream.rs`, 신규 `market_stream_runtime.rs`
(config/typed error/private day resolver·types만; daemon/runner 제외), 신규
`market_stream_runtime_storage_tests.rs`, 부모 `owner_equity_v2.rs`의 명시적 module/export,
`intraday.rs`의 기존 lineage validator `pub(super)` visibility만, 기존
`market_stream_c3a_tests.rs`와 `tests/owner_market_stream_boundary.rs`의 직접 영향 테스트만.
기존 DB support의 historical supervisor guard는 C1에서 수정/실행하지 않는다. 그 밖의
migration0001–0055, producer/transport/runner/REST/API/Web/ops/manifest/lock/docs는 보존한다.

내부 API는 §13의 `load_runtime_grant`, `record_runtime_status`, `retire_stream_producer`와
sealed runtime/day 타입으로 고정한다. 새로운 임의 quote/status/proof setter는 공개하지 않는다.
`OwnerMarketStreamRuntimeConfig::from_values`와 typed error만 safe export하고 runtime daemon은
D에 남긴다. runtime publication은 private resolved day를 epoch/context에 결합하며 실제 calendar
lineage와 half-open window를 잠금 후 재검사한다. 기존 focused facade는 보존하고 새 runtime
전용 resolved 경로로 연결한다. 기존 API를 숨기거나 검증을 느슨하게 해 호환성을 맞추지 않는다.

`StreamSnapshot.rows`의 기존 cache 행은 호환을 위해 보존하고 `delivery_rows`를 추가한다.
새 safe DTO에는 허용된 identity/nullable cache/구독 상태/producer delivery metadata만 담는다.
never-quoted 행은 lease UUID/상태 버전0 namespace를 사용한다. SECURITY DEFINER helper와 cache를
동일 actor transaction·일관된 snapshot에서 읽으며 app에 direct producer/subscription/grant
권한을 주지 않는다. 최초 proven epoch는 gap이 아니고 claim/retire는 이전 proven day와 gap을
보존한다. quote_version은 실제 quote commit에서만 증가한다.

C1 Cargo는 기존 lock·jobs2·offline·workspace target으로 직렬 실행한다. DB feature test는
`--no-run`; 실제 실행은 확인된 pure unit/default doctest만이다. DATABASE_URL/과거 supervisor
입력을 읽거나 DB·Docker·서비스·네트워크를 실행하지 않는다. 보고서는 실제 실행/compile-only/
미검증을 구분하고 C2에서 실행할 정확한 test 이름·개수·새 fixture 요구사항을 반환한다.

WP-3C1 작업자 `00a21e2a-937d-4401-8591-0dd8476d7854`를 배정했고 실제
`codex/gpt-6-luna/max/auto-review`, workspace·parent 및 실행 상태를 확인했다.
완료 알림과 해당 작업자만 대상으로 하는 2분 복구 heartbeat를 등록했다.

WP-3C1 회수(2026-10-01): 실제 idle/active turn 없음/권한 대기0과 보고서를 확인했다.
기본 library check, DB 없는 외부 boundary 1개와 포맷 검사는 통과했다. DB feature
`--no-run`은 기존 producer의 test-only 오류 관측 enum에서 새 `ControlWriteTimeout`
분기가 빠져 실패했다. 실제 DB·pure library 테스트·doctest는 아직 실행되지 않았다.
C1은 미채택이며 다음 둘을 기존 사용자 계속 진행 승인 아래 코디네이터가 분리한다.

- **C1-R (코디네이터 직접 검토):** C1의 SQL 권한·snapshot·상태/날짜 전이를 읽기
  전용으로 검토하고 private 보고서에 근거를 남긴다. Sol high 독립 리뷰 작업자 생성은
  자동 승인 검토가 저장소 코드의 미검증 provider 전달 위험을 이유로 거절했다.
  새 작업자나 다른 경로로 재시도하지 않으며 코디네이터가 소스·명세·로그를 대조한다.
  이 검토는 독립된 두 번째 리뷰로 계산하지 않는다.
- **C1-V (simple/high, Luna max):** 코디네이터가 producer의 `cfg(test)` 닫힌 오류 관측
  enum와 정확한 변환 match에 `ControlWriteTimeout`만 추가한다. 이 두 줄은 기존 WP-3
  producer 통합의 좁은 선행 수정이며 runtime/전송 의미는 바꾸지 않는다. 새 producer pin을
  기록한 뒤 C1 작업자는 코드 읽기와 private 검증 보고서만 맡는다. 기존 lock/jobs2/offline
  아래 DB feature 전체 `--no-run`, 정확히 선별한 pure 6개, DB 없는 boundary 및 검토된
  default doctest만 실행한다. DB 테스트 실행과 historical support 사용은 계속 금지한다.

이는 heartbeat의 새 권한이 아니라 완료 콜백 이후 코디네이터 검토·계획 갱신이다.
C1-R 결과와 C1-V 실제 로그를 확인하기 전 C2/3D를 시작하지 않는다. 앞선 rustfmt가
범위 밖 producer 세 파일을 건드린 뒤 복원한 경위도 보존한다. root는 현재 세 파일의
HEAD 일치와 기존 producer/test/0055 pin 일치를 확인했으며 이후 포맷은
`--config skip_children=true`로 수정 파일에 한정한다.

코디네이터 직접 리뷰에서 C1 보완 세 항목을 확인했다. 신규 DB fixture의 최초 lease 순번은
현재 `1`이어서 저장소의 `0` 계약과 충돌한다. resolved epoch/publication은 첫 잠금 후
시각 검사를 통과한 뒤 DB 작업 중 close를 넘을 수 있으므로 commit 직전 fresh clock의
날짜·half-open window·lineage와 producer 잔여 수명을 재검사한다. 새 snapshot 테스트는
never-quoted 거절만 다루므로 authentic ACK/receipt와 명시적 test window로 정상 LIVE를
먼저 증명하고 heartbeat/expiry/epoch/ACK/window/lineage가 이를 무효화함을 검증해야 한다.
상태/retire 테스트에는 quote_version뿐 아니라 수신·commit 시각 불변 assertion도 추가한다.

**C1-F (intermediate/high, Luna max)**는 C1-V가 실제 idle이 된 뒤 기존 C1 작업자에게
위 확정된 수정만 맡긴다. 소유 범위는 `market_stream.rs`,
`market_stream_runtime_storage_tests.rs`, synthetic window seam이 필요한 경우의
`market_stream_runtime.rs`로 한정한다. 코디네이터는 추가로 발견된 producer test의
닫힌 진단 match와 기존 순수 진단 table에 `ControlWriteTimeout`을 각각 한 줄 추가하고
새 pin을 기록한다. 테스트 전용 수정이며 producer runtime과 기존 protected support는 보존한다.
원래 producer의 기존 포맷 차이는 별도로 기록하며 전체 파일 재포맷으로 변경 범위를 넓히지 않는다.
신규 DB 회귀는 소스 작성/전체 opt-in `--no-run`까지만 허용한다. 검토된 pure 6개와
수정한 순수 진단 table 1개, DB 없는 boundary와 검토된 default doctest만 오프라인 실행한다.
C2 actual-role은 여전히 미실행이며 root의 수정 소스·실제 로그 검토 전에는 채택하지 않는다.

C1-F 회수(2026-10-01): 실제 idle·active turn 없음·승인 대기0을 확인하고 보고서와
25개 로그 해시, 최종 소스 pin을 대조했다. library check, DB feature 전체 `--no-run`,
지정 pure 6개, DB 없는 boundary 1개, compile-only doctest 26개 및 포맷 검사는 통과했다.
나머지 pure 1개는 기존 synthetic 입력의 `special`/`SPECIAL` 불일치로 실패했다.
DB 회귀 15개는 컴파일만 했으며 실제 실행 결과로 취급하지 않는다.

코디네이터 검토에서 등록되지 않은 test commit gate가 `InvalidInput`을 반환해
독립 실행한 resolved snapshot/일부 close 테스트가 검증 전에 실패하는 문제도 확인했다.
close 테스트의 오류·assert 경로 역시 gate 해제와 자식 task join을 마친 뒤 DB를 정리하도록
보완해야 한다. C1은 **미채택**이며 위 세 항목을 기존 코디네이터 수정 gate로 반환한다.
상세 근거는 private `wp3c1-fix-root-review.md`에 보존한다. 이 회수 heartbeat에서
새 followup이나 C2/3D를 시작하지 않는다. 완료 작업자 감시는 결과 회수 후 종료한다.

**C1-F2 (intermediate/high, 기존 Luna max 유지)** — 이후 사용자의 진행 상황 확인에
코디네이터가 기존 계속 진행 승인을 재확인하고 source/offline 수정 gate를 재개한다.
이는 회수 heartbeat의 자동 실행이 아니다. root가 두 파일의 작은 테스트 전용 오류를
직접 수정한다: synthetic disposition을 `SPECIAL`로 고치고, 미등록 commit gate는
즉시 성공하도록 하며 DB 없는 정확한 회귀 테스트 1개를 추가한다. 그 뒤 두 소스의
새 pin을 고정하고 기존 C1 작업자에게 storage test 파일 하나의 close-case 정리만 맡긴다.

작업자 쓰기 범위는 `market_stream_runtime_storage_tests.rs`의 두 close-crossing case와
그 task/gate 정리 helper뿐이다. gate 도달 뒤 실패할 수 있는 조회는 Result로 모으고,
성공·오류 모두 release/abort 후 join과 loopback close를 마친 뒤 오류·assert를 전달한다.
검증은 기존 lock/jobs2/offline으로 opt-in library `--no-run`, 기존 exact pure 7개와
root가 추가·검토한 `unregistered_resolved_commit_gate_is_noop` 1개를 실행한다.
새 정리 경로를 위한 no-DB 테스트는 소스 작성 후 root가 읽기 전용으로 확인하기 전에는
실행하지 않는다. 기존 boundary/doctest는 관련 소스가 보존되면 반복하지 않는다.
DB·historical supervisor·provider·운영·서비스·계정/주문 실행과 다른 파일 수정은 금지한다.
보고서·실제 idle·소스/원본 로그를 root가 확인한 뒤 C1을 다시 판단한다.

C1-F2 회수·검토(2026-10-01): 실제 idle/active turn 없음/승인 대기0을 확인하고,
한정된 **C1 소스·오프라인 결과를 채택**했다. 앞선 세 지적은 테스트 입력 수정,
미등록 gate 회귀, 두 close case의 child join·오류 전달 순서 보완으로 해소했다.
최종 소스에 대한 opt-in library `--no-run`, exact pure 8개, 포맷·diff 검사가 통과했고
보고서의 최종 로그 해시 13개와 보호된 소스 pin 10개가 일치했다. DB 테스트 15개는
컴파일만 했으며 WP-3C 실제 역할 검증과 전체 runtime 인수는 여전히 미완료다.

새 C2 harness를 검토할 때 다음 소스상 문제를 먼저 해결한다. 기존 loopback 서버는
trade 전송 후 2초 내 client close를 요구하지만 publication-close case는 약 7.9초의
경계 도달을 기다린다. 또한 기존 `LoopbackMarketSession::close`는 첫 child의 오류에서
두 번째 join을 건너뛸 수 있고, setup 오류 경로는 close 오류를 버린다. 관찰용 SQL await도
각각의 deadline이 필요하다. 새 fixture의 명시적 수명·전체 child 정리·DB 관찰 제한을
고정한 뒤 실행하며, 실제 receipt·lease·window 검증 조건을 약화하지 않는다.
이는 소스에서 확인한 후속 harness 요구사항이며 DB 실행 실패를 관측한 것은 아니다.
상세 근거는 private `wp3c1-fix2-root-review.md`에 보존한다. 결과 회수 후 F2 heartbeat를
종료하고 기존 코디네이터의 C2 계획 gate로 반환한다. 이 회수 이벤트에서 C2/3D를
시작하지 않는다.

#### C2 재개 — 새 로컬 PostgreSQL과 test-only harness

사용자는 C1-F2 회수 결과 뒤 **“승인해 이어서 진행해”**라고 직접 승인했다. 이 승인으로
기존 코디네이터의 C2 gate를 재개하며, 새로운 운영·provider·계정/주문 범위는 추가하지 않는다.
코디네이터는 기존 로컬 PostgreSQL 배포본의 `root/usr` 소프트웨어 파일만 읽어 새 작업
디렉터리에 복사하고 641개 파일 해시를 대조했다. 과거 data/supervisor/proof/wrapper는
읽거나 재사용하지 않았다. UID:GID 1000:1000으로 PostgreSQL 18.6 새 클러스터를 초기화했고,
새 system identifier는 `7691641528624668685`다. 아직 서버를 시작하거나 SQL을 실행하지 않았다.

실행 root는 `/tmp/lagrange-kis-c2-4b4fed7166084d97a6b67ae3ba294be0`, data와 socket은
그 아래 `data`/`socket`, Unix socket port는 55471이다. TCP listen은 빈 값으로 고정한다.
비밀이 없는 identity binding은 private `wp3c2-runtime-binding.json`에 mode0600으로 두고
SHA-256 `f173e4eed80a6543535f574fa003d6d77d521bfb41d6c532f6c44442a2f4f268`로 고정했다.
`wp3c2-preparation.json`과 `wp3c2-software-hashes.json`은 도구·소유권·새 identity 근거다.

| Package | Complexity | Basis | Confidence | Escalation signal |
|---|---|---|---|---|
| WP-3C2-H | intermediate | 확정된 test-only identity·종료·deadline·소켓 전환과 실행 스크립트 작성 | high | 범위 밖 코드 변경 필요, 동일 수정 반복 실패, cleanup 소유권 누락 |
| WP-3C2-E | hard | 실제 역할·잠금·commit 경계와 새 PG 종료/정리 증거를 통합 | medium | fixture identity 불일치, 보호 pin 변화, DB 테스트/정리 실패 |

H와 E는 순차 실행하며 기존 `00a21e2a-937d-4401-8591-0dd8476d7854` 작업자의
`codex/gpt-6-luna / max / auto-review`를 유지한다. 현재 profiles의 Astra high는
코디네이터용이며 작업자 모델·mode를 바꾸지 않는다. 새로운 리뷰 작업자를 재시도하지 않는다.

**C2-H 소유 범위:** `tests/owner_market_stream_boundary_support/mod.rs`, 그 아래 신규
`c2_fixture.rs`, `src/owner_equity_v2/market_stream_runtime_storage_tests.rs`,
`src/owner_equity_v2/market_stream_producer_test_support.rs`의 새 fixture 소켓 전달 접점만
(경로는 모두 `crates/job-queue/` 기준). private `wp3c2-run.py`와 보고서·로그도 작성한다.
제품 저장소/runtime/producer, producer tests, C3A tests, 0001–0056, manifest/lock은 보존한다.

H는 explicit C2 binding path+SHA 입력으로 새 identity를 검증하는 별도 test-only 분기를
추가한다. 둘 중 하나만 있거나 부적합하면 fail closed하고, 새 분기는 접속 전 URL/Unix
socket/user/database/port와 owner0700/0600·고정 경로 구조를 검사한다. 접속 후에도
system-id/data-dir/socket/listen/port/server-version/current-user를 대조한 뒤 DDL을 허용한다.
기존 입력이 없는 legacy guard와 상수는 보존한다. `DATABASE_URL` fallback은 금지한다.
producer PG relay도 같은 검증된 소켓/port를 capture해 쓰며 과거 소켓으로 돌아가지 않는다.
생성 DB 이름은 기존 `lagrange_ws3a_<pid>_<counter>`를 유지하되 새 클러스터 안에서만 허용한다.

close-crossing case에만 명시적 20초 loopback 수명을 부여하고, 기존 기본 2초 제한은 유지한다.
모든 loopback child를 오류/panic/timeout에도 join 또는 abort-and-join하며 setup/cleanup 오류를
함께 보존한다. DB 시각 관찰의 각 await는 2초, 전체 시각 대기는 15초로 제한한다.
SessionInvalid·receipt/lease/window·capture 불변 assertion은 약화하지 않는다.

H에서는 **DB 연결/서버 시작/SQL/loopback 실행을 하지 않는다**. 새 테스트와 DB feature lib는
`--no-run`으로만 검증하고, private runner는 Python 문법 검사까지만 수행한다. 코드와 새
no-DB/loopback 테스트를 root가 먼저 검토한다. root의 source pin·guard·cleanup 검토 뒤 E를
같은 직접 승인 아래 시작한다. H 회수 heartbeat 자체가 E 실행 권한을 만들지는 않는다.

**C2-E 실행안:** 검토한 private one-shot runner가 위의 새 PG만 실행한다. 실제 역할
bootstrap·모든56 migration을 사용하는 신규 DB15개, 기존 C3A3개, producer original20–26
7개와 pacing4개를 각각 exact test 한 개씩 직렬 실행한다. 정확한29개 이름은 private
`wp3c2-db-cases.json`으로 고정한다. 모든 Cargo는 기존 lock/jobs2/target/`--locked --offline`을
사용하며 한 테스트 최대180초, 실패하면 후속 테스트를 중지한다. 각 테스트 뒤 생성 DB의
정확한 부재·연결0을 확인한다. finally에서 owned child를 끝내고 새 PG를 종료·wait하고 socket/
postmaster 부재를 확인한다. 실패 증거는 보존하고 임의 DB 삭제·과거 cluster 복구는 하지 않는다.
E 실행 전 root가 새 helper 테스트와 runner 본문·source pins·검증 목록을 읽고 실행 gate를
닫는다. 실제 PASS/FAIL과 compile-only, 미실행을 구분하며 WP-3D는 C2 인수 뒤에 진행한다.

C2-H 배정·기동 확인(2026-10-01): 기존 작업자에게 MCP followup과 완료 알림을 등록했고,
실제 `codex-turn-4` 실행·Luna/max/auto-review·parent/workspace·승인 대기0을 확인했다.
소유 작업자 하나만 회수하는 2분 heartbeat `4aab1642`를 2시간 한도로 등록했다.
새 PG는 초기화 상태로 정지해 있으며, C2-H 보고서·코드·로그·runner 검토 전에는 E를 실행하지 않는다.

#### C2-H 회수·root 검토 — 보완 후 실행 gate

2026-10-01 실제 idle/active turn 없음/승인 대기0과 완료 보고서를 확인했다.
최종 DB feature lib `--no-run`, Python 문법 검사, 세 소스의 포맷 검사와 `git diff --check`는
통과했다. 151개 보호 pin과 보고서가 인용한 원본 로그 9개의 해시도 일치했다.
producer support 전체 포맷 검사의 기존 실패는 PgRelay 수정 밖의 차이로 확인했다.
이는 컴파일·소스 증거이며 신규 helper 9개와 실제 DB 29개를 실행한 결과가 아니다.

root 검토에서 다음 두 보완점을 확인했으므로 **H 인수와 C2-E 실행은 아직 미완료**다.

1. private runner가 강제 child 정리, 최종 DB catalog 확인 실패·잔존, PostgreSQL 비정상·강제
   종료를 기록하면서도 `all_29_passed`/exit0을 반환할 수 있다. 모든 case와 최종 정리 조건을
   확인한 뒤 성공을 확정하고, primary/cleanup 오류를 함께 보존해야 한다. 각 실패 조건과
   정상 경로를 실제 PG를 시작하지 않는 synthetic 판정 테스트로 검증한다.
2. `c2_fixture.rs`의 canonical URL 테스트가 SQLx `get_host()`의 빈 문자열을 기대한다.
   현재 SQLx 0.9.0의 Unix socket 설정은 기존 host 필드를 지우지 않는다. 실제 socket과
   port/user/database/SSL 계약을 검증하도록 보완하고 endpoint guard는 유지한다.
   실행할 때는 task-owned HOME과 PG 설정이 없는 환경으로 암묵적인 passfile 접근을 막는다.

상세 소스 위치·pin·로그와 미검증 범위는 private `wp3c2-h-root-review.md`에 기록했다.
다음 코디네이터 gate의 수정 범위는 `c2_fixture.rs`의 해당 assertion, private runner의
성공/정리 판정, private no-DB 판정 테스트·보고서로 한정한다. 나머지 H 소스 3개와
151개 보호 pin, 준비 입력과 정확한 29개 목록은 보존한다. 필요한 컴파일·문법 검사와
root가 읽은 no-DB 검증 뒤 다시 인수한다. 이 회수 이벤트에서는 수정 followup이나
C2-E/WP-3D를 배정하지 않았다. 실제 idle·승인 대기0·결과 회수 뒤 H heartbeat
`4aab1642` 삭제를 확인하고 기존 직접 승인에 따른 코디네이터 gate로 반환했다.

#### C2-HF — 사용자 재개 지시에 따른 두 보완점 수정

사용자가 **“이어서 진행해”**라고 직접 지시했으므로 위의 보완 작업과 검토 후 C2-E를
기존 코디네이터 gate에서 이어간다. HF는 intermediate/high: 두 원인과 변경 접점이
확정됐고 순수 테스트·root 소스 검토로 확인할 수 있다. 기존 Luna max/auto-review를
유지하며 같은 수정의 반복 실패나 범위 밖 변경 필요 시 코디네이터에 반환한다.

기존 작업자가 순차로 소유할 범위는 `c2_fixture.rs`의 canonical URL 테스트 assertion,
private `wp3c2-run.py`의 성공/최종 정리 판정과 필요한 순수 함수, private
`wp3c2-hf-runner-tests.py`·보고서·로그·합성 임시 파일뿐이다. 정상 성공은 정확한29개
통과와 모든 command/최종 catalog/PG 종료 조건이 확인된 뒤에만 확정한다. 강제 child
종료·비정상 exit·확인 실패·잔존을 실패로 보존하며 primary 오류를 cleanup 오류로 덮지 않는다.
검증은 순수 판정 함수의 정상/각 실패 입력, DB feature lib compile-only와 root가 이미
읽은 기존 helper 9개를 정확히 하나씩 실행하는 것이다. helper 실행 환경은 별도의
task-owned home/temp를 사용하고 PG·credential 환경을 상속하지 않는다. 새 runner의
실제 진입점, PG·DB·SQL·네트워크 실행은 HF에서 금지한다.

나머지 H 소스3개·보호151개·준비/바인딩/29-case 입력은 읽기 전용이다. 파일별 변경 줄,
원인/편차, exact 명령과 선택 수·exit·원본 로그·해시, 미해결/미검증 항목을 보고한다.
HF actual idle 뒤 root가 실제 호출부까지 포함해 판정 함수와 로그를 검토해야 C2-E를
시작한다. helper와 DB 테스트의 증거를 구별하며 같은 범위의 승인을 다시 요청하지 않는다.

#### C2-HF 인수 — 실제 DB 실행 전 마지막 소스 검토

2026-10-01 13:15:07 UTC의 실제 idle/active turn 없음과 승인 대기0, 완료 callback을
확인했다. root는 최종 보고서·runner 전체와 실제 판정 호출부·순수 테스트·원본 로그를
검토해 위의 두 보완점을 인수했다. helper9개는 각각1개 선택·통과, 합성 판정 테스트6개와
DB-feature `--no-run`도 통과했다. 151개 보호 pin과 나머지 H 소스3개는 유지됐고,
canonical URL assertion 이외의 Rust 변경이 없음을 이전 해시 재구성으로 확인했다.

helper 실행은 dispatch의 직접 binary 호출 대신 동일 binary를 선택하는 exact-filter
offline Cargo를 사용했다. 기존 lock/jobs2/target·격리 HOME·9개 선택 범위를 지켰으므로
root가 동등한 호출로 채택했다. 상세 근거와 편차는 private `wp3c2-hf-root-review.md`에
기록했다. 이는 **실제 DB29개나 서버 종료 동작의 통과가 아니다**. 해당 검증은 다음
C2-E의 실제 원본 결과와 정리 증거로만 인수한다. 기존 직접 재개 지시에 따라 HF 회수용
heartbeat를 종료한 뒤 별도 E 실행 gate를 기록하고 진행한다.

#### C2-E — HF 인수 후 기존 승인으로 실행 gate 개방

HF heartbeat `c548d258` 삭제를 확인했다. 기존 직접 재개 승인에 따라, 동일 작업자
`00a21e2a-937d-4401-8591-0dd8476d7854`의 Luna max/auto-review를 유지해 검토된
`wp3c2-run.py`를 한 번 실행하도록 준비했다. runner SHA-256은
`8ea33933446ceb74ccc7fe2d576dc7f561f84bbeab39e5998139640116728983`이다.
private `wp3c2-e-reviewed-pins.json`은 보호151개와 H 소스4개 및 실행 입력을 고정한다.

준비된 새 PG 하나의 Unix socket과 기존 합성 loopback fixture만 사용하며, 정확한29개를
각180초 이내에 직렬 실행한다. 첫 실패 시 후속을 중지하고 finally의 실제 정리 결과를
보존한다. 코드/runner/명세 수정, 재실행·재초기화·과거 fixture 재사용은 이 작업에 포함되지
않는다. 원본 결과와 실제 역할·catalog·PID/socket·종료 증거를 root가 검토하고 실제 idle을
확인해야 C2를 인수한다. 실행 전 컴파일 증거를 DB 통과로 바꾸지 않는다.

배정 후 실제 `codex-turn-6` 실행과 기존 parent/workspace·Luna/max/auto-review,
승인 대기0을 확인했다. 완료/오류/권한 callback을 등록했고 5분 간격·2시간 한도의
소유 작업자 회수 heartbeat `5f7b5635`를 보조로 등록했다. 이 기록은 기동 확인이며,
DB 테스트나 C2 전체의 통과 판정은 아직 아니다.

#### C2-E 실패 회수와 C2-U 로컬 보완 gate

E runner를 한 번 실행한 원본 결과는 **1번 PASS, 2번 `UnsafePath` FAIL, 3–29번
NOT_RUN**, exit1이다. 새 PG 식별 검증과 전후·최종 빈 generated-DB catalog를 확인했다.
PG는 SIGINT 후 exit0으로 종료했고 강제 종료 없이 PID/socket/소유 process group이 사라졌다.
root는 최종 보고서와 raw 로그·155개 소스/7개 private pin을 대조했고, 13:37:28 UTC의
실제 idle·승인 대기0과 완료 callback을 확인했다. 실패 보고서는 회수했지만 **C2는 인수하지
않았다**. heartbeat `5f7b5635`를 삭제했고, 상세 근거는 private `wp3c2-e-root-review.md`에 있다.

기존 직접 완료·계속 지시에 따른 코디네이터 gate에서 **WP-3C2-U**를 분리한다.
intermediate/high confidence이며, 이미 읽은 코드가 원인 후보를 좁힌 상태이고 DB 없는
재현과 전후 exact 테스트로 확인할 수 있다. 기존 Luna max/auto-review를 유지한다.
E 실행 중 inherited `umask077`은 테스트 생성 anchor의 요청 mode0440을0400으로 만들 수
있고, 현재 `provision_test_anchor_exclusive`는 descriptor 권한을 확정하지 않은 채
`open_anchor`의 exact0440 검사를 받는다. 이는 아직 원인 후보이며 재현 실패 시 수정하지 않는다.

U 소유 범위는 `crates/kis-client/src/market_stream_state.rs`의 test/test-support 전용
`provision_test_anchor_exclusive`와 같은 파일의 focused regression뿐이다. 먼저 기존
`state_is_atomic_and_reloads_attempt_history`를 별도 프로세스의 umask022/077에서 대조한다.
해당 실패가 재현되면 새로 독점 생성한 file descriptor에만 exact0440을 설정하고 sync한다.
기존 파일, production initializer/validator, 경로·소유자·link-count 검사, 모드 상수와 다른
소스는 보존한다. 멀티스레드 테스트 프로세스의 전역 umask를 바꾸지 않고 새 자식 프로세스에서
재현하는 회귀를 추가한다. 새 subprocess 회귀는 두 anchor의 mode0440과 단일 link를 검증한다.

Cargo는 기존 lock/jobs2/target/`--locked --offline`이며, 실행은 해당 재현·신규 회귀와
읽기 완료된 state 원자 저장·잠금 배타성·symlink 거부·slot mismatch·기존 layout 보존
exact 테스트에 한정한다. opt-in job-queue DB lib는 필요 시 compile-only다. DB/PG/SQL,
loopback, 실제 보호 경로·키, provider·운영, E runner 재실행·fixture 재초기화는 U에 없다.
E의 과거 pin·입력·raw 결과를 덮지 않고 U 전용 새 pin 경계를 기록한다. 수정 전후 원본 결과와
소스 diff를 root가 인수한 뒤 실제 DB 재검증의 별도 코디네이터 gate를 준비한다.

U 배정 후 기존 작업자의 `codex-turn-7`, Luna max/auto-review 및 parent/workspace와
승인 대기0을 확인했다. 해당 followup의 완료/오류/권한 알림과 5분 간격·1시간 한도의
회수 heartbeat `41502781`을 등록했다. 원인 재현·수정 결과는 아직 인수하지 않았다.

후속 root 검토에서 U를 source/offline 범위로 인수했다. 수정 전 같은 exact 테스트가
umask022에서 통과하고077에서 `UnsafePath`로 실패해 원인이 확인됐다. 새로 생성한 테스트
anchor descriptor에만0440을 확정했고, 수정 후 지정된7회 exact 실행이 각각1개 선택·통과했다.
kis-client와 opt-in job-queue lib는 compile-only 성공이며 DB 통과 증거는 아니다.
최종 소스·보고서·raw 로그, 실제 idle/activeTurn null·승인 대기0과 보호154개 소스/27개
private pin을 root가 확인했다. 보고서의 보호 소스153개 표기는154개로 정정해 root 검토에
기록했다. E의 실패·미실행 결과는 유지하며 별도 현재 코드 DB 검증 gate를 준비한다.

#### C2-E2 — U 수정 후 별도 DB 검증

U의 실제 완료·idle 인수와 heartbeat41502781 삭제 뒤, 기존 사용자의 완료·계속 지시에
따라 코디네이터가 E2를 준비했다. hard/medium confidence의 실제 역할 검증이며, 명령이
고정된 기존 Luna max/auto-review 작업자가 수행하고 판정은 root가 맡는다.
소스 수정은 없고 U에서 인수한 test-helper 변경만 새155개 pin에 반영한다.

E2는 이번10월1일 작업이 생성한 동일한 격리 클러스터7691641528624668685를 정상 정지
상태에서 한 번 시작·검증·종료한다. 첫 E에서 정상 종료와 빈 최종 generated-DB catalog가
확인됐고 현재 PID/socket도 없다. 클러스터를 처음 시작한다고 주장하지 않는다. 각 case는
검토된 fixture로 새 합성 DB를 생성·삭제한다. 과거9월의 클러스터·proof·wrapper와
운영 DB는 사용하지 않으며, 초기화·reset·잔류 DB 수동 삭제도 허용하지 않는다.

private `wp3c2-e2-run.py` SHA
`2ec619eb842578d04c2f41ee1015689d93a43422fb285c6390e8457b089d30d0`는 기존 검토 runner의
새 사본이다. 설명·소스 pin 입력·결과 파일 prefix만 바뀌었고 역변환으로 원본과 바이트 단위
일치를 확인했다. py_compile도 통과했다. 실행·판정·정리 로직과 정확한29개 case는 동일하다.
원본 E runner·입력·실패 증거는 보존한다. E2는 별도의 현재 실행 gate이며 소비된 E 권한을
연장하지 않는다. 전체 명령·pin·권한은 private E2 준비 검토/dispatch와 ledger에 기록했다.

Unix-only PG와 정확한29개 case에 필요한 합성 loopback만 사용한다. Cargo는 기존 lock,
jobs2/target/offline이며 각180초, 직렬·첫 실패 중단·1회 실행이다. 실패·미실행을 그대로
보존하고 finally의 catalog/PG 정상 종료/자식 group/PID/socket을 검토한다. 소스·runner
수정이나 임의 재시도 없이 root에 반환하며, 실제 idle과 raw 증거 검토 전 C2를 인수하지 않는다.

E2 배정 후 기존 작업자의 `codex-turn-8` 실행(14:12:56 UTC), Luna max/auto-review,
parent/workspace/cwd·session 및 승인 대기0을 확인했다. 완료/오류/권한 callback과
5분 간격·2시간 한도의 회수 heartbeat `16f8a356`을 등록했다. 이는 작업자 기동 확인이며
DB 결과 인수나 전체 완료를 뜻하지 않는다.

E2는 한 번 실행해1번 통과,2번 `WindowUnavailable` 실패,3–29번 미실행으로 끝났다.
실제 idle(14:27:32 UTC), pending0, 최종 보고서·소스155/private33 pin·원본 결과를 root가
확인했다. PG 신원과 빈 최종 catalog, 정상SIGINT/exit0 및 PID/socket/자식 종료도 확인했다.
실패 보고서만 회수했으며 C2는 인수하지 않았다. heartbeat16f8a356을 삭제했고,
private `wp3c2-e2-root-review.md`에 기록했다. E와 E2의 실행 권한은 각각 소비됐다.

#### C2-W — 테스트용 snapshot 시간 범위 보완

E2 완료 callback을 회수한 후 원래 사람의 계속 지시에 따른 root gate에서 C2-W를 준비했다.
소스 검토상 `seeded_resolved_snapshot`은 DB 시각에서60초를 빼고1시간을 더해 시간 범위를
만든다. E2의23:16 KST에서 종료가 다음날00:16이 되어, 기존 `IntradaySessionWindow::new`의
동일 날짜 검사가 거부한다. 자정 직후에는 시작 시각도 전날로 넘어간다. raw 오류만으로
내부 실패 위치를 관측했다고 주장하지 않고, 실제 pure constructor 회귀로 확인한다.

소유 범위는 `market_stream_runtime_storage_tests.rs`의 공용 snapshot 준비 helper/callsite와
순수 회귀2개뿐이다. 원하는 `now-60초`/`now+1시간`을 해당 KST 날짜00:00:00–23:59:59 안으로
제한한다. 날짜 불일치, 반개구간 밖의 now, 종료 동일 시각은 명시적으로 거부하며 현재 시각이나
session_date를 조작하지 않는다. production/transport/day 검증과 LIVE·negative·capture
assertion, 별도 close-crossing 사례와 cleanup은 그대로 둔다.

기존 Luna max/auto-review의 intermediate/high confidence 패키지다. 고정된23:16·00:00:30·
정상 장중 값을 기존 `fixture_day`로 검사하는 pure 회귀와 날짜/종료 경계 거부 회귀,
읽기 완료된 half-open 순수 테스트만 실행한다. DB feature는 compile-only이며 DB/PG/SQL/
loopback/runner 실행은 없다. 다른154개 소스와 E/E2/U 비공개 증거54개를 고정한다.
정확한 새 manifest 수량·해시, 구현 조건과 명령은 private C2-W root finding/dispatch와 ledger가
기준이다. 실제 idle·소스·raw 로그를 인수한 뒤에만 별도 DB 검증 gate를 준비한다.

C2-W는 2026-10-01 14:37:42 UTC에 기존 소유 worker의 `codex-turn-9`로 시작했다.
workspace·parent·Luna max/auto-review 및 실제 running, 승인 대기 0건을 확인했다.
완료 알림과 recovery heartbeat `feb54a6b`(15:38:25 UTC 만료)를 등록했으며,
이 예약은 C2-W 회수만 수행하고 다음 DB 검증이나 WP-3D를 시작하지 않는다.

#### C2-W 결과와 WF — 기본 테스트 기능 경계·기대값 보완

W의 실제 idle(14:51:24 UTC)과 완료 callback, final 보고서 및 raw 로그를 회수했다.
기본 lib test compile이 producer의 test-support 전용 snapshot import/method 때문에
E0432/E0599로 중단되어 순수3개와 DB-feature compile은 모두 미실행이다. root는 별도로
23:16 KST 회귀의 open 기대값이14:15:50Z/231550 대신13:16:50Z/221650인 오류를 확인했다.
154/54 pin과 HEAD는 유지됐고, 새 helper/import/tests/callsite만 제거하면 정확히 이전 소스
hash로 복원된다. W는 인수하지 않았고 heartbeat `feb54a6b`는 삭제했다.

기존 사람의 계속 승인과 실제 완료 callback을 근거로 별도 WF gate를 준비했다. root는
producer snapshot import/method 두 곳만 `all(test,feature="market-stream-db-tests")`로
한정하고, 위 테스트 기대값 두 곳을 바로잡는다. 해당 메서드의 호출은 이미 DB-feature
테스트 안에만 있다. production 코드·feature manifest·기존 assertion·DB case는 보존한다.

WF는 simple/high confidence의 읽기 전용 검증 패키지이며 기존 Luna max/auto-review를
유지한다. root 변경 후 모든 소스를 pin하고 기본 lib compile, 지정 pure3개(각1선택·통과),
DB-feature lib compile-only를 offline/lock/jobs2/workspace target에서 확인한다. 오류가 나면
증거를 보존하고 중단한다. source 수정·DB/PG/SQL/loopback 실행이나 새로운 reviewer 생성은
권한이 없다. 실제 idle와 raw 로그를 root가 검토한 뒤에만 별도 DB 검증 gate로 넘어간다.

root 통합의 네 줄을 역변환해 이전 해시와 일치함을 확인했고, 현재155개 소스·68개 private
입력을 고정했다. WF는14:58:07 UTC에 기존 worker `codex-turn-10`으로 시작했으며 실제 running,
승인 대기0건과 기존 model/mode를 확인했다. 완료 알림과 recovery heartbeat `e08c311b`
(15:58:36 UTC 만료)를 등록했다. DB 실행은 허용하지 않는다.

#### WF 인수와 C2-E3 — 보완 후 현재 소스 DB 검증

WF의 실제 idle(2026-10-01 15:05:11 UTC), 완료 callback과 raw 로그를 root가 확인했다.
기본 lib compile, exact pure3개(각1선택·통과), DB-feature lib compile-only, format/diff가
통과했다.155개 source·68개 private pin과 HEAD가 유지됐고 남은 Cargo/process/temp는 없다.
초기 보조 pin 스크립트의 비교 문자열 오타(exit1, 전용 raw 파일 없음)는 보고서에 보존하며,
수정된 전체 검사는 Cargo 전후와 root 독립 확인에서 일치했다. W/WF source/offline만 인수하고
heartbeat `e08c311b`는 삭제했다. 실제 DB29나 runtime 인수는 아직 아니다.

실제 WF 완료에 따른 별도 E3 gate를 기존 사람의 계속 승인으로 준비한다. October1에 이
작업만을 위해 초기화한 동일한 정지 PG18.6/sysid7691641528624668685를 사용한다. E/E2의
소비된 once-only 실행과 실패 결과는 보존한다. 새 E3 runner는 현재 producer cfg와 snapshot
fixture 변경의 pin 및 증거 출력 namespace만 갱신하고 실행/판정/정리 로직은 유지한다.
현재 KST 날짜는10월2일이며 fixture는 각 생성 때 DB의 KST 날짜를 사용한다. DB clock이나
production window는 조작하지 않는다. 준비 단계에는 소프트웨어641개 hash, 디렉터리,
PID/socket 부재와 pg_controldata의 stopped identity만 읽고 PG를 시작하거나 SQL을 하지 않는다.

E3는 intermediate/high confidence이며 기존 Luna max/auto-review worker를 유지한다.
최종 root review와 runner/source/private pin을 고정한 뒤 exact29를 serial/failfast/180초 상한으로
딱 한 번 검증한다. 새로운 runner의 두 번째 실행·reset/reinit·수동 잔존DB 삭제·소스 수정은
권한이 없다. 기존 Cargo lock/jobs2/workspace target/locked/offline과 loopback 소유권·수명,
최종 catalog·PG 정상 종료·PID/socket/process-group 정리 조건을 그대로 적용한다.

E3 준비에서 PG control은15:11:33 UTC에 `shut down`, sysid 일치를 확인했다. 전용 디렉터리
owner1000/mode0700,641개 software hash, PID/socket/lock 부재를 확인했고 PG 시작·SQL은
하지 않았다. 새 runner SHA `a54e67ea87d4677d26b65824b2637c7cd6f294b5e97ef6da8cf7c65cad2673e6`,
현재155 source·89 private pin을 고정했다. 원본 E2로의 inverse byte 비교와 py_compile도 통과했다.
E3는15:15:53 UTC에 기존 worker `codex-turn-11`으로 dispatch했으며 실제 running/pending0와
기존 workspace/parent/model/mode를 확인했다. 완료 알림과 heartbeat `555bdeb3`
(17:16:22 UTC 만료)을 등록했다. 이 기록 시 실제 runner invocation/DB 결과는 아직 관측하지
않았고, heartbeat는 이 실행 회수만 수행한다.

#### E3 회수와 L/E4 — 제약을 보존하는 만료 fixture 보완

실제 E3 완료 callback과15:25:34 UTC의 idle/activeTurn null/승인 대기0을 확인했다.
155개 source와89개 private pin, HEAD가 유지됐다. 원본 로그에서1–9번은 각1선택·1통과,
10번은 SQLSTATE23514로 실패,11–29번은 미실행이다. live sysid/Unix-only 신원과 최종
빈 generated-DB catalog, PG 정상 exit0, 기록된 PID/process-group/socket/lock 부재를
root가 대조했다. 실패 증거만 회수했고 C2를 인수하지 않았다. Heartbeat555bdeb3는 삭제했다.

실패한 실제 SQL은 test-support의 `set_producer_expired`가 만료 시각만 현재보다1초
과거로 설정한 것이다. 새 producer의 heartbeat가 더 최신이라 기존0055의
`lease_expires_at > heartbeat_at` 제약을 위반했다. 기존 사람의 계속 지시와 실제 완료
callback에 따른 별도 L/E4 gate에서 root가 이 helper의 SQL만 보완했다. 하나의 DB clock
관측을 CTE에 담고 heartbeat는2초, 만료는1초 과거로 함께 갱신한다. 기존 production,
schema/validator/검증 assertion은 유지한다. 해당 문자열만 역치환하면 E3 source SHA가
복원되고, 나머지154개 source pin이 일치한다. rustfmt와 diff check는 통과했다.
기록: private `wp3c2-e3-root-review.json`, `wp3c2-l-source-review.json`.

E4는 intermediate/high confidence로 기존 Luna max/auto-review worker에 배정한다.
먼저 DB-feature library를 locked/offline/jobs2/shared Cargo lock으로 `--no-run` 컴파일하고,
성공과155 source/145 private pin을 확인한 경우에만 최종 pinned E4 runner를 한 번 실행한다.
새 runner SHA는 `f41e0b86d32fe6de5a01598fa8e2e7c9d70419667f3ca15b7b94dc054d577fee`이다.
E3와의 차이는 header/출력 namespace/helper pin이며 실행·판정·정리 로직의 변경은 없다.
역치환 byte 비교와 py_compile을 통과했다.15:32:50 UTC의 read-only 준비에서 동일한 현재
소유 PG18.6/sysid7691641528624668685의 shut down,641개 software hash, 디렉터리 소유권과
PID/socket/lock 부재를 확인했다. 이 준비에서는 PG 시작이나 SQL을 수행하지 않았다.
E/E2/E3의 소비된 실행과 실패 기록은 그대로 둔다. 동일한29개 synthetic DB 사례를 직렬,
fail-fast,각180초 상한으로 검증하며 E4 재실행/reset/reinit/수동 잔존DB 삭제는 허용하지 않는다.
DB29·0056 up/down·runtime/API/browser/live 인수는 실제 결과 확인 전까지 미완료다.
E4는15:37:03 UTC에 기존 worker의 `codex-turn-12`로 시작했다. 실제 running, 기존
workspace/parent/Luna max/auto-review와 승인 대기0을 확인했다. 완료 callback과5분 간격의
recovery heartbeat `0cbfa42b`(17:37:42 UTC 만료)를 등록했다. 이 시점에는 compile/runner
실제 결과를 아직 확인하지 않았으며, heartbeat는 해당 작업의 회수만 수행한다.

#### E4 회수와 M/E5 — 철회된 snapshot 및 rollback 테스트 정합성

E4 실제 완료 callback을 받은 뒤15:45:13 UTC actual idle/activeTurn null/pending0을
확인하고 원본 로그를 검토했다. Compile exit0, 사례1–15 통과,16번
`owner_market_stream_real_role_boundary`는 `MembershipNotReady`로 실패했으며
17–29는 미실행이다. 앞선 lease fixture 수정은10번 통과로 검증됐고,0056 up/down은
15번에서 통과했다. 최종 generated DB catalog는 비어 있었고 PG는 정상 exit0,
기록된 PID/socket/group은 남지 않았다.155 source/145 private pin과 HEAD가 일치했다.
E4 실패 이력은 보존하며 heartbeat `0cbfa42b`는 삭제했다. 전체 C2는 인수하지 않았다.

원본16번 오류에는 호출 위치가 없었다. Root 소스 검토에서 권한 철회 후에도 snapshot을
Ok(empty)로 기대하는 terminal assertion과,0056을 생략한 rollback tail을 확인했다.
현재 storage와0056 helper의 zero authorized rows 계약은 정확한 MembershipNotReady 거부다.
별도 M/E5 gate에서 root는 C3A 테스트 terminal assertion만 이 계약에 맞추고,
기존0056 down을0055 down 앞에서 실행하며0056 helper 부재도 확인하도록 수정했다.
그 외 race/admission/publication assertion, production validator, migration 원문은 보존했다.
정확한 inverse diff가 이전 source hash를 복원하고 rustfmt/diff check는 통과했다.
이 source finding이 E4의 실제 failing callsite였다는 단정은 E5 전까지 하지 않는다.

E5는 기존 Luna max/auto-review worker의 intermediate/high confidence 실행 패키지다.
현재155 source/221 private pin은 모두 읽기 전용이다. 정확한 offline locked DB-feature
`--no-run` 컴파일을 먼저 수행하고 성공할 때만 pinned E5 runner를 한 번 실행한다.
Runner SHA `1be3d0df705b1e7b94a9ad4e99d47f06f55ee722458a07a1a793bfe4a564b065`; reviewed manifest SHA `2a2ac4b60d8528bec799a1ecd4457be957cccff7335b42ff4918230cd15fbd42`.
원본 E4와 executable logic/case/acceptance/cleanup은 동일하며 header/output namespace 및
M source를 포함하는 protected manifest hash만 바뀌었다. 동일 task-owned PG18.6의
sysid7691641528624668685, shut down,641 software hash, PID/socket 부재를 읽기 전용으로
확인했다. 정확한29 synthetic DB 사례를 순차 fail-fast/각180초 이내로 검증한다.
E/E2/E3/E4는 모두 소비된 실패 이력을 보존하고 재실행하지 않는다. E5도 반복/수정/reset/
재초기화/수동 잔존 DB 삭제를 허용하지 않는다. 실제 idle 및 root 원본 결과·정리 검토 뒤에만
C2 인수를 판단하며 runtime/API/browser/live와 G1–G5는 여전히 미완료다.

E5는16:03:22 UTC에 기존 worker의 codex-turn-13으로 실제 시작했다. 기존
workspace/parent/Luna max/auto-review와 승인 대기0을 확인했다. 완료·오류·승인 callback과
5분 recovery heartbeat 3e1e2210(18:03:56 UTC 만료)를 등록했다. 이 시점의 compile/DB
결과는 아직 확인하지 않았으며, heartbeat는 기존 E5 회수만 수행한다.

#### E5 회수와 N/E6 — full-board snapshot 거부 계약 정렬

E5 actual idle16:10:17 UTC와 실제 완료 callback을 회수했다. Compile exit0,
1–15 PASS,16번 MembershipNotReady FAIL,17–29 NOT_RUN이다. 원본 모든16개 count와
catalog, live identity, 최종 empty catalog, PG normal exit0 및55개 기록 PID/group 부재,
155 source/221 private pin/HEAD를 root가 확인했다.3e1e2210 heartbeat는 삭제했다.
M의 terminal assertion 수정만으로 실패는 해소되지 않았으며 C2는 미인수다.

Root의 추가 검토에서0056 complete-set guard는 단 하나의 disabled/stale-generation
lease 항목도 전체 snapshot을 거부하는데, C3A의 더 이른 snapshot_after_generation 및
snapshot_after_membership은 부분 rows를 기대하고 있음을 확인했다. 이 테스트는 먼저
identity1을 DISABLED, identity3과0을 stale generation으로 만들고 full30 lease를 유지한다.
별도 N/E6 gate에서 두 기대값을 정확한 MembershipNotReady로 정렬했다. 정상이어야 하는
single-identity/rollback snapshot 두 곳에는 typed-error 단계명만 추가하고 양성 assertion을
유지했다. Production/storage/0056/다른 race·demand·publication assertion은 바꾸지 않았다.
정확한 inverse bytes 검토, rustfmt/diff check와154개 타 소스/221개 과거 private pin은 통과했다.
E5 raw error에는 callsite가 없으므로 actual E6 전까지 원인 확정·수정 완료로 판정하지 않는다.

E6는 기존 Luna max/auto-review worker의 intermediate/high-confidence 실행 패키지로,
155 source/299 private pin을 읽기 전용으로 두고 offline locked jobs2/shared-lock
DB-feature --no-run compile이 성공할 때만 runner를 한 번 실행한다.
Runner SHA 87de0582c9ebe61d4eafe7f8b2abbfa7a0aaa89230e89a4af18434f182fc652a; reviewed manifest SHA 6ceabb3c0ed93aad31672b6abbc83f05ffe506cd748cfcde25efeb57ecab519b.
Same PG18.6 sysid7691641528624668685의 shut down 및641 software hash/PID/socket 부재를
읽기 전용으로 확인했다. 정확한29 사례와 실행/인수/cleanup logic은 E5와 같고 새 namespace와
현재 source pin만 바뀐다. E/E2/E3/E4/E5는 재실행하지 않으며 E6도 retry/reset/reinit/manual
retainedDB 삭제를 허용하지 않는다. 실제 idle+root 원본 검토 후에만 인수한다.

E6는16:21:37 UTC에 기존 worker의 codex-turn-14로 실제 시작했다. 동일 workspace/parent/
Luna max/auto-review, 승인 대기0을 확인했다. 완료·오류·승인 callback 및 recovery heartbeat
a590d7c2(18:22:14 UTC 만료)를 등록했다. 현재 compile/runner 결과는 미확인이며, heartbeat는
기존 E6 회수만 수행한다.

#### E6 최종 인수와 WP-3D 선행 gate 완료

E6 actual idle16:39:33 UTC, activeTurn 없음, 승인 대기0과 실제 완료 callback을 확인했다.
Root는29개 raw exact log의 각 selected1passed1failed0ignored0,103개 보고서 hash 참조,
155 source/299 private pin/HEAD, live identity, 각 case 전후·최종 empty catalog, PG 정상
SIGINT exit0과94개 PID/group 및 socket 부재를 검토했다. 첫 readiness exit2는 socket
생성 전 bounded polling 관찰이며 다음 확인에서 정확한 identity를 검증했다.
Root review: wp3c2-e6-root-review.json, SHA d4a7ff2828b849799f59e32e4d2c454ec1f331f131e3335a31be66d2212654c3.
C2의 source/local actual-role DB29,0056 up/down,C3A/producer/pacing 경계를 인수한다.
과거 E–E5의 실패·미실행 이력은 보존했다. 복구 heartbeat a590d7c2는 삭제했다.
이 실제 완료 callback에서 기존 사용자 계속 진행 권한에 따라 코디네이터가 WP-3D를
구체화한다. Runtime/API/browser/live와 G1–G5는 별도 미완료이며 새 운영 권한은 없다.

#### WP-3D 순차 세부 단위 — C2 최종 인수 후 root gate

E6 actual idle·raw29·정상 PG 종료·source155/private299·보고서 검토를 마치고 C2를 인수했다.
기존 human continuation으로 아래 단위를 순차화한다. 기존 E–E6 runner는 모두 소비됐으며
새 DB/runtime 검증은 별도 concrete gate 전에는 실행하지 않는다. 새 reviewer 생성 거절은
유지하고 기존 소유 implementation worker를 사용한다. 모든 단위는 $paseo-delegate 필수다.

| 단위 | 분류/확신 | 단일 목표/소유 범위 | 선행/검증 | 배정 |
|---|---|---|---|---|
| WP-3D1 | intermediate/high | market_stream_producer.rs 끝에 private runtime_buffer만 추가. authentic receipt adapter, latest30/단일 outstanding batch, 단조 ordinal·capture·250ms cadence와 순수 테스트8 | C2 인수; 기존41087 bytes 그대로, 다른 source154/private407 pin 보호. default/DB-feature lib 컴파일만. root가 새 테스트를 읽은 뒤 exact8 실행 | 기존 C1 worker, Luna max, auto-review 유지 |
| WP-3D2 | hard/medium | D1을 private start_resolved/run_owned와 serial writer에 연결, 소켓 단일 소유·취소·commit/ACK 경계 | D1 root 인수 뒤 정확한 source/test manifest; 기존 focused facade와 original/pacing 보존; 새 DB/loopback 실행은 별도 gate | Luna max 시작, 반복 실패/소유권 불명확 시 한 단계 상향 검토 |
| WP-3D3 | hard/medium | runtime supervisor와 runner 모드·lease/demand/day·종료 연결 | D2 검토; runner EOD/REST/once 경계와 typed 상태; concrete manifest를 먼저 고정 | Luna max 시작 |
| WP-3D4 | hard/medium | 완성 WP-3 V1–V10/영향 회귀 actual-role local 통합 검증 | D2/D3 source 검토; 새 test/case/runner/fixture/종료 계획을 root가 먼저 고정 | Luna max 시작, 원본 증거로 root 최종 인수 |

이번에 열린 gate는 D1 source/compile-only뿐이다. D2–D4는 의존성 개요이며 자동 실행 권한이
아니다. D1 내부 generic kernel의 non-Clone marker 테스트는 진짜 receipt를 위조하지 않으며
transport authenticity·실제 DB transaction rate·전체 8MiB/soak를 증명한다고 보고하지 않는다.
한 outstanding batch를 사용해 두 batch 상한보다 좁게 유지하고, writer가 busy하면 latest30에
병합한다. 기존 public facade의 read_and_publish와 모든 기존 assertion은 그대로 보존한다.
상세 self-contained brief와 pins는 private wp3d1-dispatch.md / wp3d1-reviewed-pins.json에 기록한다.
root source/raw-log/actual-idle 검토 뒤에만 인수하며 미실행 검사는 계속 미완료로 남긴다.

#### WP-3D1 결과 회수와 D1-F 한정 수정·순수 검증 gate

D1 real callback과 실제 idle/pending0를 확인하고 소스1124–2805행·보고서·raw compile 로그를
검토했다. 기존41087 bytes 및 source154/private407은 보존됐고 default/DB-feature 컴파일은
통과했지만 테스트 실행은0건이다. root는 두 문제로 D1을 인수하지 않았다: offer가 교체마다
queued_at을 갱신해 모든 pending 종목이 계속 갱신되면 250ms handoff가 무기한 밀릴 수 있고,
unit fixture가 canonical instrument005930.KRX 대신005930으로 StreamIdentity를 생성한다.

root가 테스트 fixture만 바로잡고 기존 hot-symbol 테스트에0/100/200/249ms 수신→250ms 게시,
writer busy 중 수신→500ms 두 번째 게시 요구를 추가했다. 현재 kernel은 그대로여서 첫 요구는
예상 실패해야 한다. 변경을 역으로 제거한 바이트가 D1 원본과 같음을 확인했고, exact8 테스트
전체 본문을 읽어 순수 in-memory 경계임을 검토했다. D1 heartbeat295c6d29는 회수 후 삭제했다.

별도 D1-F gate는 기존 Luna max worker가 (A) exact hot test 한 번의 예상 실패 원본을 남기고,
(B) 기존 pending entry의 최초 queued_at만 보존하는 root-specified hunk 하나를 적용하며,
(C) default 및 DB-feature --no-run, (D) 검토한 default-feature exact8을 각각1회 실행하도록 한다.
2026-10-01T17:41:05Z에 기존 worker의 codex-turn-16 시작을 확인했다. 모델·effort·승인 모드와
workspace/parent는 유지됐고 pending0였다. 완료 알림과 복구 전용 heartbeat730c8c5c를 등록했다.
다른 실패/zero selection/예상과 다른 재현이면 fail-stop한다. 원래 prefix, 모든 root 테스트와
기타 소스는 read-only이고 before/expected-after 전체 source hash를 고정했다. DB/loopback/
provider/production 및 D2 실행은 허용하지 않는다. 이 단위는 intermediate/high, 반복 동일 실패
시 재분류하며 기존 provider/model/mode를 유지한다. private wp3d1f-root-gate/dispatch/pins/cases와
root review를 인수 기준으로 사용하고, actual idle/raw tests 검토 전에는 완료로 판정하지 않는다.

#### D1-F 최종 인수와 D2-A/B 분해

D1-F의 실제 완료 callback과 idle/pending0를 확인했다. root는 수정 전 exact 재현의 지정 panic,
단일 queued_at 보존 hunk, default/DB-feature compile-only 및 exact 순수8의 각각1개 통과를
원본 로그로 검토했다. 다른 source154/private428, 기존41087 bytes, HEAD가 유지됐다.
초기 scratch 포맷 검사의 구분 개행 실패와 수정된 scratch 검사는 모두 남겼고 저장소 포맷을
바꾸지 않았음을 확인했다. D1 source/순수 범위를 인수하며 실제 receipt 연결·DB cadence·heap·
soak·runtime 인수와 구분한다. heartbeat730c8c5c는 회수 후 삭제했다.

D2 준비 중 기존 begin_worker_transaction의 lock5s/statement30s와 publication의 pool 직접
exact reread를 확인했다. §13.1의 호출 전체1s 제한은 outer timeout만 추가해서는 충족되지
않는다. 기존 focused facade·actor API의 동작을 보존하기 위해 다음 두 단위로 순차 분리한다.

| 단위 | 분류/확신 | 범위와 산출물 | 검증/선행/선택 |
|---|---|---|---|
| WP-3D2-A | intermediate/high | market_stream.rs 안에만 private runtime repository adapter, 단일 호출 전체1s deadline/취소 latch, runtime 전용 transaction-local1s 제한과 같은 deadline 안의 exact publication reread. 기본 repository와 모든 C1 판정 유지 | D1 인수 및 root가 고정한12개 method forwarding/허용 hunk; pure8 테스트 소스와 default/DB-feature --no-run만. 실행은 root source review 후 별도 gate. 기존 Luna max, auto-review 유지 |
| WP-3D2-B | hard/medium | D2-A adapter와 D1 buffer를 private start_resolved/run_owned, socket-owner·직렬 writer·취소/ACK/commit 경계에 연결 | D2-A 검토 뒤 source/test manifest 확정; 기존 facade와 C1/C2 의미 보존; 실제 DB/loopback은 별도 gate. Luna max 시작 |

D2-A는 public export, actor transaction, 기존 validator/SQL predicate/role/migration/producer/
runner를 변경하지 않는다. timeout·외부 취소·미확정 commit 뒤에는 같은 runtime adapter의
공유 상태를 terminal로 유지하며 다음 호출이 내부 DB future를 poll하지 않는다. 새 generic
future helper는 private 구현용이며 public API나 새 proof factory가 아니다. DB 오류를 숨기거나
exact reread 밖에서 commit을 복구하지 않는다. unknown commit의 기존 exact reread가 남은
원래1s 예산 안에서 증명한 성공만 유지한다. pool 대기와 transaction/commit/reread를 각각 새
1s로 나누거나 서버 연결에 전역 SET을 남기지 않는다. 실제 DB timeout/취소 동작은 미검증으로
남기고 D2/D3/D4의 후속 actual-role 검증에서 확인한다.

상세 단일 파일 hunk와 보존 범위, typed adapter, 순수8 이름, compile 명령은 private
wp3d2a-dispatch.md / wp3d2a-root-gate.json / wp3d2a-reviewed-pins.json에 고정한다.
기존 worker는 2026-10-01T18:05:27Z에 codex-turn-17을 시작했다. Luna max/auto-review,
workspace/parent, pending0를 확인했으며 알림과 복구 heartbeatac2aa51b를 등록했다.
다른 source154/private469는 보호하며 새 순수8도 현재는 소스 작성·컴파일만 허용한다.
같은 수정 두 번 실패·허용 hunk 밖 필요·계약 충돌은 root에 반환하며 추측으로 보완하지 않는다.
heartbeat는 해당 작업 회수만 수행하며 D2-B·DB·runtime을 새로 시작하지 않는다.

#### D2-A 소스·컴파일 인수와 D2-A-V 순수 검증

D2-A의 real callback과 2026-10-01T18:26:33Z 실제 idle, pending0를 확인했다. 최종 소스
`df6aa40e6b9ef55a2fbf647c8c7e8fd648a5bb8ab517380f936cc69f6da97fc4`의 허용 hunk를
역으로 제거해 기존 소스 바이트 전체를 복원했고, 다른 source154/private469 및 HEAD를
대조했다. actor/publication의 기존 제어 흐름과 legacy5s/30s는 보존됐으며 runtime-only
1s 설정·단일 deadline·공유 terminal latch·12개 forwarding과 exact reread를 직접 검토했다.
최종 default/DB-feature no-run 로그는 exit0이고 테스트 실행은0건이다. root checker의
초기 구분 개행 가정 실패와 수정 후 보존 검사도 기록했다. heartbeatac2aa51b는 삭제했다.

root가 여덟 테스트 본문을 읽어 synthetic future/paused clock/poll·drop counter만 사용하고,
생성한 단일 작업을 abort/join함을 확인했다. 별도 D2-A-V는 simple/high 검증 단위로 기존
Luna max/auto-review를 유지하며, 모든 소스를 고정하고 여덟 default-feature exact filter를
각각 한 번만 실행한다. 각 실행은 selected1/passed1/ignored0이어야 하며 실패·선택0·예상 밖
결과는 즉시 중단하고 미실행 항목을 남긴다. lock/jobs2/offline/workspace target과 개별180s
제한을 적용한다. DB-feature 재실행·DB/SQL·runtime·D2-B 구현이나 소스 수정은 포함하지 않는다.
세부 cases/dispatch/pins와 raw 로그는 private wp3d2av 산출물로 고정한다. 순수 검증 성공도
실제 서버의 제한·취소/rollback, 동시 renewal/publication, runtime·live 인수를 대신하지 않는다.
기존 worker의 codex-turn-18은 2026-10-01T18:34:00Z에 시작됐고 실제 running/pending0를
확인했다. source155/private495를 고정했으며 완료 알림과 복구 전용 heartbeata880c006을
등록했다. 이 알림의 회수 범위도 V에 한정되며 다음 구현 단위를 자동으로 시작하지 않는다.
최종 callback 이후 2026-10-01T18:41:32Z 실제 idle/pending0를 확인했고, root가 exact8의
명령·원본 ok/선택 수·exit0와 source155/private495를 다시 대조해 순수 범위를 인수했다.
초기 결과 판독기의 `0 measured` 누락은 테스트 실패가 아니며 실제 로그와 보고서에 그대로
기록했다. 준비 gate JSON은 원래 스냅샷으로 보존하고 실제 dispatch/lifecycle는 ledger와
startup 기록으로 확인했다. 복구 heartbeata880c006은 삭제했다. 실제 DB/runtime 검증은 남는다.

#### D2-B 소켓 소유자·직렬 writer 소스 gate

D2-A-V 실제 완료·idle와 root의 source155/private495·raw8 검토 이후 기존 사용자 계속
진행 권한으로 별도 D2-B gate를 준비한다. 분류는 hard/medium이다. 세 작업의 취소·ACK·
DB commit과 버퍼 무효화가 결합되지만 root가 private 연결 방식과 종료 판정을 고정하고,
다음 source review·pure·actual-role/loopback 단계에서 검증한다. 기존 Luna max/auto-review를
유지하며 같은 수정 두 번 실패, 소유권 모순 또는 보호된 API 변경 필요 시 root에 반환한다.

소유 파일은 market_stream_producer.rs 하나다. 기존 focused facade와 D1/F 원본 전체를
보존하고, ReceiptBuffer에 기존 refresh_context만 호출하는 한 개의 private wrapper를 추가한
뒤 private runtime owner 코드를 append한다. 다른 source154 및 역사적 private 증거는 pin으로
고정한다. start_resolved는 D2-A의 같은 shared-latch adapter와 resolved day를 받아 별도
RuntimeOwnedProducer를 반환한다. 기존 public facade를 그대로 두기 위한 private signature
구체화는 spec13.1에 반영했으며 public surface나 증명 생성 권한을 추가하지 않는다.

소켓 작업만 session/command를 구동하고, 직렬 writer는 D1의 latest30·단일 outstanding batch와
실제 transaction-start 250ms 제한을 그대로 사용한다. 부모의 bounded controller가 최신
demand·lease·shutdown과 날짜/5초 lease margin을 확인하고 shared buffer를 즉시 무효화한다.
자식 작업은 abort-on-drop으로 소유하며 정상 경로는 모두 join한다. 외부 drop·강제 abort·
불확정 DB/close는 clean 재시작으로 보고하지 않는다. 이후 D3만 1초 demand/rights/day 검사와
5초 renewal, 유한 reconnect budget 및 public exit 매핑을 연결한다.

root는 RuntimeTransition의 gap 기록이 producer.gap_generation을 바꾸고 C1의 정확한 epoch
검사가 이를 구별함을 확인했다. gap 요청부터 게시를 억제하고 현 incarnation을 정리하며
status 결과로 proof scalar를 바꾸지 않는다. 소켓 clean close·child join·fenced retirement가
모두 알려진 성공일 때만 D3에 fresh epoch 필요를 반환한다. in-flight batch의 demand 무효화도
재생·강제 complete_success하지 않고 알려진 DB 결과를 보존한 뒤 같은 종료 경계를 따른다.

이번 gate는 구현과 새 pure8 소스, default 및 DB-feature lib --no-run까지만 허용한다.
테스트 실행은0건이어야 하며 root가 새 코드를 읽은 후 별도 exact gate를 연다. 실제 session,
DB/PG/SQL, loopback, network/provider, runtime/runner, 배포·운영 입력은 실행하지 않는다.
구체적인 ownership·유한 상태·명령 순서·종료·검증과 pin은 private wp3d2b-dispatch.md,
wp3d2b-root-gate.json, wp3d2b-reviewed-pins.json에 기록한다. heartbeat는 이 작업 회수만
담당하며 D3 또는 새 검증을 시작하지 않는다. 전체 runtime/API/browser/live는 계속 미완료다.
기존 worker의 codex-turn-19가 2026-10-01T19:07:49Z에 시작됐고 실제 running/pending0,
동일 workspace/parent와 Luna max/auto-review를 확인했다. 다른 source154/private526과
전체105373-byte before source를 고정했고 완료·오류·권한 알림을 등록했다. 복구 heartbeat
256b62de는 5분 간격, 2026-10-01T20:08:22Z까지로 제한한다. 만료 전 미완료면 현재 권한만
재확인해 감시를 갱신하고, 실제 idle와 최종 보고서 회수 뒤 삭제한다.

#### D2-B-F 명령 허용·종료 확인 보완

D2-B의 실제 완료 콜백·idle/pending0, 최종 소스와 default/DB-feature `--no-run`,
source154/private526 핀을 root가 확인했고 heartbeat256b62de를 삭제했다. 컴파일은
통과했지만 명령 선택 후 최신 수요·중지 상태를 다시 검증하지 않는 경로와, 강제 취소 후
1초 timeout으로 child join 전에 반환하는 경로 때문에 소스 인수를 보류했다. 원본 실패
컴파일도 보존한다. 근거는 private `wp3d2b-root-review.json`이다.

사용자의 직접 지시 **“이어서 진행해 승인하니”**에 따라 기존 worker의 별도 보완 gate를
연다. 기존 `paseo-delegate` 실행 계약과 권한 경계가 유지된다.

| Package | Complexity | Basis | Confidence | Escalation signals |
|---|---|---|---|---|
| WP-3D2-B-F | hard | 명령 허용과 취소·종료 순서가 동시 작업과 연결되지만 root가 두 수정 경계를 확정함 | medium | 같은 수정 두 번 실패, 보호 코드 변경 필요, ownership 모순이면 root로 반환 |
| WP-3D2-B-F-V | intermediate | 수정 소스를 root가 읽은 뒤 exact 순수 10개를 직렬로 실행하고 원본 로그를 확인 | high | 미선택·실패·timeout이면 중단하고 root가 원인 검토 |

| Package | Wave | Objective | Owned scope | Depends on | Worker | Deliverable | Verification |
|---|---:|---|---|---|---|---|---|
| D2-B-F | 1 | 모든 새 명령의 현재 수요/중지 검증 및 강제 취소 후 child join 완료 보장 | producer 파일의 runtime_owner 내 지정 helper/caller와 새 pure2 소스만 | D2-B root review와 사용자 계속 승인 | 기존 Luna max/auto-review worker | private wp3d2bf-report.md와 원본 로그 | prefix·기존 pure8·보호 핀 보존, default/DB-feature lib compile-only |
| D2-B-F-V | 2 | 기존 pure8와 새 회귀2 확인 | 소스 불변, private 증거와 Cargo target만 | 실제 idle 및 root 수정 소스 검토 | 같은 worker 재사용 | 별도 검증 보고서 | root가 읽은 정확한 default 테스트만 각각1선택·1통과 |

명령은 짧은 동기 임계 구역에서 최신 lifecycle·수요 identity/count·ACK/pending/tombstone
상태를 다시 확인한 뒤에만 허용한다. CountChange도 같은 검사를 통과해야 하며 positive
count 변경으로 ACK proof를 바꾸거나 불필요하게 pending으로 만들지 않는다. 이미 허용된
command는 기존 prepared→pending commit→pinned send→authentic ACK commit 순서를 유지한다.

5초 cooperative drain 이후에는 남은 작업에 abort를 요청하고 JoinSet이 완전히 비워질
때까지 소유권을 유지한다. 별도 1초 timeout이나 Drop만으로 종료 확인을 대신하지 않는다.
외부 future drop은 여전히 terminal이며 clean exit나 cleanup 완료로 보고하지 않는다.

전체 기존105669-byte prefix, parent wrapper와 기존 pure8의 본문은 보존한다. 다른 source154와
역사적 private 증거는 새 manifest로 고정한다. 새 helper/regression source를 root가 읽기 전에는
pure를 포함한 테스트 실행은0건이다. F-V의 exact 실행은 다음 root gate에서만 연다.
DB·PG·loopback·provider·runtime·D3 실행은 이 gate에 포함되지 않는다. 구체적 함수 범위,
새 테스트의 유한 메모리 정리 조건, 오프라인 Cargo 명령과 실패 중단 기준은 private
`wp3d2bf-dispatch.md` 및 `wp3d2bf-scope.json`에 기록한다.
기존 worker의 codex-turn-20이 2026-10-02T04:29:55Z에 시작됐으며 실제 running/pending0,
동일 workspace/parent 및 Luna max/auto-review를 확인했다. source154/private562 핀과
prefix·parent suffix·기존 pure8을 고정했다. 완료·오류·권한 알림 및 5분 간격 heartbeat
62b5e5e2를 등록했으며, 감시 만료는 2026-10-02T05:30:38Z이다. 실제 idle와 보고서 회수 뒤
삭제하고, 미완료 상태로 만료가 다가오면 같은 보완 권한 안에서만 감시를 갱신한다.

#### D2-B-F 결과 검토와 F-V 검증 gate

실제 F 완료 콜백·idle/pending0와 source154/private562, 원본 default/DB-feature
compile 로그를 root가 대조했다. 명령 admission과 abort 후 전체 join 소스 보완은
확인했으며 heartbeat62b5e5e2를 삭제했다. F 보고서와 최초 실패 기록은 보존한다.

root는 신규 pure2 본문만 보강했다. CountChange 중지/실패 검사는 현재 count3 계획의
정상 허용을 먼저 확인한 뒤 거부를 검증하도록 했고, abort regression이 조기 반환을
검출하는 실패 경로에서도 gate release 뒤 모든 child join을 직접 수거하도록 했다.
운영 코드와 기존 pure8 바이트는 불변이며 scratch rustfmt가 통과했다.

별도 WP-3D2-B-F-V(intermediate/high) gate는 이 최종 소스를 고정한 뒤 default 및
DB-feature lib --no-run을 직렬 확인하고, root가 읽은 기존8+신규2 exact DEFAULT
순수 테스트를 각각 한 번, 1선택·1통과·0무시로 검증한다. 같은 Luna max/auto-review
작업자를 재사용하며 소스·핀은 읽기 전용이다. 컴파일/선택/실행 오류면 이후는 NOT_RUN,
재시도나 수정 없이 root로 반환한다. 상세 brief와155source/private manifest는
private wp3d2bfv-dispatch.md, wp3d2bfv-reviewed-pins.json에 고정한다.
DB 실행·loopback·D3·실제 런타임은 이 gate에 포함되지 않는다.
기존 worker의 codex-turn-21 실제 running 상태와 동일 부모/workspace/Luna max/auto-review를 확인했다.
완료·오류·권한 알림과 heartbeateecf2eae를 등록했으며 2026-10-02T06:21:45.417Z까지 제한한다.
최종 보고서·실제 idle·원본 결과를 회수한 뒤 삭제하고, 필요하면 기존 검증 범위만 갱신한다.

#### F-V 인수와 D3 거래일 조회 선행 경계

F-V의 실제 완료 콜백과 idle/pending0를 확인했다. Root가 원본 compile2/exact10 로그,
155개 소스·608개 private pin, 최종 producer 바이트와 보고서를 대조해 source/compile/pure
범위를 인수했다. 테스트8의 synthetic child panic은 의도한 경로이며 최종1pass/0failed/0ignored다.
Cargo/rustc는 없고 git diff --check가 통과했다. Heartbeateecf2eae는 회수 후 삭제했다.
Root acceptance: private wp3d2bfv-root-review.json, SHA
85a3e82185f4a1c8ea42c4209c81668bca29e5d74b88b68a0a0ef9cfc4784a1d.
실제 transport/DB deadline·rollback·runtime·cadence·heap·soak 인수는 별도로 남는다.

D3 준비 중 기존 resolve_day가 OwnerIntradayQuoteRepository의5s/30s transaction과
손실된 CommitUnknown 분류를 사용함을 확인했다. 이 경로를 그대로 supervisor에 연결하면
§13.1의 전체 호출1s와 공유 terminal latch를 충족하지 못한다. 기존 사용자 계속 진행 권한으로
이 선행 접점부터 분리한다. 기존 REST의 public API·SQL 조건·기본 timeout은 보존한다.

| 단위 | 분류/근거/확신 | 목표와 소유 범위 | 선행·검증 | 배정 |
|---|---|---|---|---|
| WP-3D3-A | intermediate; 기존 조회 경로3파일의 명시적 policy·오류 전파 연결; high | intraday.rs의 두 calendar 읽기와 transaction helper만 내부 policy로 분리, runtime resolver의 기존 의미 보존, RuntimeMarketStreamRepository에 공유 deadline을 쓰는 day 조회1개 추가 | F-V 인수 및 root scope; 신규 pure3 소스와 default/DB-feature lib --no-run만, 실행은 root review 뒤 | 기존 Luna max/auto-review, $paseo-delegate |
| WP-3D3-B | hard; socket·lease·demand·day 수명 결합; medium | D3-A와 D2-B를 사용하는 supervisor 구현 | D3-A 인수 뒤 구체적 source/test manifest를 새로 고정 | 기존 worker, 별도 root gate |
| WP-3D3-C | intermediate; 기존 runner의 정확한 mode 분기 연결; medium | off/once/REST 보존과 market_ws daemon 진입점 | D3-B source 검토 뒤 한정 runner·export·config 범위 확정 | 기존 worker, 별도 root gate |

A는 intraday의 기존 public 읽기2개와 transaction2개의 내부 공통 helper 분리만 허용한다.
Legacy5s/30s와 actor GUC 순서, calendar SQL 문자열·bind·36h/current-day·중복 판정은 그대로다.
새 private calendar reader는 같은 pool의 읽기2개만 Runtime1s/1s로 실행하며 raw pool·policy
setter를 노출하지 않는다. 기존 resolve_day API의 오류·Missing/Closed/Open 의미는 보존하되,
새 runtime 경로는 CommitUnknown을 유실하지 않고 기존 shared latch로 전파한다. 하나의
with_runtime_deadline이 최초 pool acquire부터 세 calendar 읽기와 결과까지 모두 감싼다.
새 timeout/latch/reset이나 spawned query를 만들지 않는다. Producer·runner·kis-client·schema·
다른 소스와 기존 테스트는 읽기 전용이다. 상세 허용 함수와 before bytes는 private
wp3d3a-dispatch.md, scope.json 및 reviewed-pins.json으로 고정한다.

A는 source/compile-only gate이며 DB·SQL·PG·loopback·provider·보호 입력·실제 runtime을
실행하지 않는다. 신규 pure3도 root가 최종 소스를 읽기 전에는 실행하지 않는다. 반복된 같은
실패나 scope/ownership 모순은 root로 반환한다. B/C/D4나 다음 단계는 heartbeat가 시작하지 않는다.

D3-A의 기존 worker codex-turn-22 실제 running과 동일 parent/workspace/Luna max/auto-review, pending0를 확인했다.
보호 source152/private655 및 before3 검사는 모두 일치했다. 완료·오류·권한 알림과 복구 heartbeat
e73a6980를 등록했고 2026-10-02T06:48:11.407Z까지 해당 작업 회수만 수행한다. 신규 pure3 실행은0건이다.


#### D3-A source/compile 인수 및 D3-A-V (2026-10-02)

D3-A 실제 callback 뒤 idle/activeTurn없음/pending0를 확인했다. root가 최종3파일 diff와
legacy SQL/bind/GUC 순서, 원본 market_stream.rs248072byte prefix, 기존 tests, source152/private655,
최종 default/DB-feature compile2 원본 로그를 대조해 SOURCE/COMPILE만 인수했다.
root 검토: wp3d3a-root-review.json SHA ecb17156608d13d40de0e951ca6af64268df81e12ae140769fcb69e287d12e50.
초기 rustfmt1와 수정 후0, pre-format/final compile 이력을 보존했다. 테스트 실행은0건이다.
기존 e73a6980 복구 heartbeat는 보고서 회수와 실제 idle 확인 뒤 삭제했다.

| 단위 | 분류/근거/확신 | 목표·소유 범위 | 검증·상향 신호 | 배정 |
|---|---|---|---|---|
| WP-3D3-A-V | simple; root가 읽은 pure3의 고정 명령 실행; high | 저장소 및 과거 증거 모두 읽기 전용, 새 private V 로그·보고서와 Cargo target만 기록 | default-feature exact3 각각 한 번 selected1/passed1/ignored0; 오류·핀 불일치·미선택 시 즉시 중단 후 root | 기존 codex/gpt-6-luna max/auto-review, $paseo-delegate |

V는 거래 정책 상수, 실제 finite 오류 매핑, 기존 shared deadline의 mapped CommitUnknown/latch
경로만 검증한다. root가 세 최종 테스트 본문을 읽어 DB·FS·환경·서버·네트워크·자식작업 경로가
없음을 확인했다. 세 fully-qualified 이름과 sanitized offline/jobs2/공용 lock/각180s 명령은
wp3d3av-cases.json 및 wp3d3av-dispatch.md에 고정한다. 수정·재시도·DB-feature 실행·broad test는 없다.
새155source 및 누적 private manifest를 동결하고 실제 raw3/idle/pending0/root 인수 전에는 B/C/D4를
열지 않는다. 실제 PostgreSQL1s 제한·rollback·commit uncertainty와 runtime 통합은 계속 미검증이다.
이 V는 지속된 인간 실행 승인 아래 실제 완료 callback 이후 코디네이터가 여는 별도 검증 단계이며,
heartbeat는 해당 owned-worker 복구만 수행한다.


D3-A-V는 기존 worker codex-turn-23 실제 running/pending0/같은 Luna max와 parent를 확인했다.
현재155source/683private pins는 모두 일치한다. 별도 완료·오류·권한 알림을 등록했고 복구 heartbeat
8e018b16는 2026-10-02T07:27:06.234Z까지 이 V 결과 회수만 수행한다. 아직 테스트 통과를 주장하지 않는다.


D3-A-V 실제 완료 callback 후 idle/pending0, 현재155source/683private와 raw3 selected1/passed1을 대조해 인수했다.
root 검토 wp3d3av-root-review.json SHA74de85d99d8ff75834afc265e21103ca889e0fe820f0490079ab5991502b82d1.
사후 감사2건의 syntax/count parser 실패는 별도 보존했고 테스트 재실행은 없었다. 원본 case3개와 최종
감사3의 결과를 root가 직접 대조했다. 8e018b16 heartbeat는 실제 idle/보고서 회수 뒤 삭제했다.
D3-A의 실제 PostgreSQL 제한/rollback과 D3-B/C/D4 runtime은 아직 미완료다.

#### D3-B0 승인 클라이언트 바인딩 선행 단계 (2026-10-02)

D3-A-V 인수 뒤 생성자 입력을 확인했다. 별도로 받은 ApprovalClient와 runtime config의
slot/generation을 상태 파일·네트워크 접근 전에 비교할 공개 API가 없다. snapshot은 파일을
읽고 domain slot getter는 crate-private이다. 잘못 결합된 입력을 DB/approval 이전에 거절할
순수 bool 검사만 먼저 추가한다. §13.1에 이 입력 일치 계약을 명시했다.

| 단위 | 분류/근거/확신 | 목표·소유 범위 | 선행·검증·상향 신호 | 배정 |
|---|---|---|---|---|
| WP-3D3-B0 | simple; 완전히 고정한 비교 함수와 pure2 추가; high | market_stream_approval.rs 끝에만 public bool wrapper/private predicate 및 pure2 SOURCE | 원본18073bytes와 나머지154source 보존; kis-client default lib와 job-queue DB-feature lib --no-run 순서; 반복 실패·범위 충돌은 root 반환 | 기존 codex/gpt-6-luna max/auto-review, $paseo-delegate |
| WP-3D3-B | hard; socket/DB/demand/lease/day 수명 통합; medium | B0 predicate를 생성자에서 사용하고 D3-A/D2-B를 결합하는 supervisor | B0 소스와 별도 pure 검증 인수 후 구체적 범위를 고정 | 기존 worker, 별도 root gate |
| WP-3D3-C | intermediate; runner mode/REST/EOD 연결; medium | 승인된 mode와 safe export 연결 | B 소스 검토 후 runner 범위 확정 | 기존 worker, 별도 root gate |

B0는 기대 slot이 nonnil, 기대 generation이 positive u64, 실제 slot이 동일하고 실제 generation
문자열이 기대 값의 정규 십진수와 정확히 같을 때만 true다. parse/trim/별칭 허용은 없다.
실제 자격 증명·권한·상태의 검증이나 연결 허용을 의미하지 않는다. 기존 client의 opaque
generation 의미와 기존 테스트/코드는 전체 prefix 그대로 남긴다.

private wp3d3b0-dispatch/scope/cases/reviewed-pins에 exact signature, 두 pure 테스트 이름,
컴파일 명령, before bytes 및 입력을 동결한다. 새 pure2도 root가 소스를 읽고 별도 V를
열기 전에는 실행하지 않는다. DB/PG/loopback/provider/보호 입력/runtime 실행은 없다.
기존 승인으로 여는 별도 coordinator 단계이며 heartbeat는 이 작업 회수만 한다.

B 후속 설계 시 실제 fresh demand를 drain 중에도 전달해야 하는 D2 owner 규칙,
내부 child join 완료 전 parent owner를 먼저 abort하지 않아야 하는 수명 규칙,
timer tick마다 연결/DB future를 재생성하지 않고 유지해야 하는 점을 검토한다.
이 관찰은 미완성 B의 설계 입력이며 runtime 검증 결과가 아니다.


D3-B0는 기존 worker codex-turn-24 실제 running/pending0/같은 parent·workspace·Luna max·auto-review를 확인했다.
원본18073byte prefix와 source154/private710이 일치하며 완료·오류·권한 알림을 별도로 등록했다.
복구 heartbeat50e22f2a는 2026-10-02T08:03:06.026Z까지 이 작업 회수만 수행한다. 신규 pure2 실행은0건이다.

#### D3-B0 소스 인수 및 B0-V 순수 검증 (2026-10-02)

실제 B0 완료 callback 이후 idle/activeTurn없음/pending0를 확인했다. 원본18073byte prefix,
보호 source154/private710, 추가된 wrapper/private predicate와 두 테스트 본문을 root가 읽었다.
kis-client 기본 및 job-queue DB-feature --no-run 원본 로그2개는 최종 소스에서 exit0이고 실행은0건이다.
root 검토 wp3d3b0-root-review.json SHA f12b04bfadfbacc7cd391ab5ff32a341402c824c5bb2191e6252ed96254d6fea.
초기 신규 import 순서 rustfmt1와 수정 후0을 보존했다. root의 읽기 전용 regex 감사 오류도
별도 기록했고 수정 감사에서 정확한 명령/소스/로그를 확인했다. 테스트 재실행은 없었다.
보고서 회수와 실제 idle 확인 뒤50e22f2a heartbeat를 삭제했다.

| 단위 | 분류/근거/확신 | 소유 범위와 검증 | 배정 |
|---|---|---|---|
| WP-3D3-B0-V | simple; root가 읽은 pure2의 고정 실행; high | 모든155source 및 과거 private 증거 읽기 전용; default kis-client exact2 각각 한 번 selected1/passed1/ignored0, 실패 시 나머지 NOT_RUN | 기존 codex/gpt-6-luna max/auto-review, $paseo-delegate |

V는 동일한 production predicate의 slot/정규 generation 비교만 실행한다. public wrapper의
실제 인자 전달은 root 소스 검사로 확인했으며, 테스트는 client/domain 생성이나 FS/DB/network를
사용하지 않는다. sanitized offline/jobs2/공용 lock/각180s 명령과 current pin은 private
wp3d3b0v-dispatch/cases/reviewed-pins에 동결한다. 별도 소스 수정/재시도/DB-feature 실행은 없다.
B0-V 통과도 D3-B 생성자 연결·C runner·실제 DB/transport/runtime 통과를 뜻하지 않는다.
현재 실제 완료 callback으로 돌아온 coordinator gate에서 이어가며 heartbeat는 결과 회수만 한다.


B0-V는 기존 worker codex-turn-25 실제 running/pending0/같은 Luna max·auto-review 및 parent를 확인했다.
source155/private733이 일치하고 별도 완료·오류·권한 알림을 등록했다. 복구 heartbeatbc112fd9는
2026-10-02T08:19:18.316Z까지 해당 결과 회수만 수행한다. 아직 pure2 통과를 주장하지 않는다.


B0-V 실제 callback/idle/pending0 이후 raw2 각각 selected1/passed1/ignored0 및 source155/private733을 root가 대조해 인수했다.
root review wp3d3b0v-root-review.json SHAeb21ff6ea026cde1fdb5274882d5d3bd1516b903bb703ade4c79cad6fda53f4a.
preflight의 stale STOP annotation은 보존했고 실행 전 별도 refresh가 live startup_verified 상태를 확인했다.
테스트 재실행은 없었다. bc112fd9 heartbeat를 삭제했으며 D3-B/C/runtime 검증은 계속 미완료다.

#### D3-B supervisor source/compile gate (2026-10-02)

B0-V의 실제 유휴 상태·원시 pure2·155/733 핀을 인수하고 bc112fd9를 삭제한 뒤,
기존 직접 계속 진행 권한으로 supervisor 범위를 구체화했다. 새 테스트/DB 실행 권한을
heartbeat에서 파생하지 않는다. 연결 future를 매 tick 취소하거나, 자식 join 중인
producer future를 바깥 timeout으로 버리면 durable ambiguity/분리된 종료가 생기는
경계를 확인했다. 또 renewal은 현재 gap generation을 반환하지만 D2의 활성 lease
검사는 원래 epoch gap을 요구한다. 따라서 gap 증가·거래일 계보 변화는 기존 epoch를
재바인딩하지 않고 drain시키며, clean close+all joins+fenced retirement 뒤에만
새 claim/day/epoch로 진행한다. spec §13.1–13.2의 수명 보완에 이를 고정했다.

| Package | Complexity | Basis | Confidence | Worker selection / escalation |
|---|---|---|---|---|
| WP-3D3-B | hard | 한 소유자의 연결·관측·lease·거래일·종료를 결합하되 root가 정확한 상태/취소 규칙을 고정 | medium | 기존 codex/gpt-6-luna max/auto-review. 사양 충돌, 같은 오류 2회, 범위 밖 수정 필요, 소유권 불명확 시 변경 확대 없이 root 반환 |

소유 파일은 market_stream_runtime.rs 하나: 정확한 문서/finite error enum seam과
파일 끝의 safe public runtime/private supervisor/pure8 SOURCE만 허용한다. seam 제거 시
전체 기존 22740byte SHA25c09b6a4406880beacde587c1182b0d1fccb90247d4ae402720a0319165ea45가
그대로 prefix여야 한다. producer·repository·transport·runner·SQL·기존 테스트를 포함한
나머지154source와 과거 private evidence는 고정한다. 기본 job-queue --lib --no-run 후
DB-feature --lib --no-run을 offline/locked/jobs2/workspace target/공유 flock/600s로 실행한다.
신규 pure8도 실행0, DB·소켓·runtime 실행0이며 root source 검토 후 별도 검증 gate가 필요하다.
상세 알고리즘·정확한 seams·cases·명령·핀은 private wp3d3b-dispatch/scope/cases/reviewed-pins에 고정한다.

D3-B는 기존 worker codex-turn-26의 실제 running/pending0, 같은 parent·workspace·Luna max·auto-review를 확인했다. 154개 source/759개 private 핀은 일치했고 완료 알림과 4ead7941 복구 heartbeat(1시간/최대12회)를 연결했다. 아직 source acceptance·새 pure8 실행·DB/runtime 검증은 미완료다.

#### D3-B/R-V2 및 초기 종료 보완 인수; D3-C 실행기 연결 (2026-10-03 KST)

Root는 B의 visibility compile 실패를 보존하고 한정 수정 후 compile2를 확인했다. R의
fence·zero-demand stop·terminal 관측·status 후 renewal·tick 수명 수정은 실제 idle과
원본 compile2/pure9로 인수했다. 잘못된 startup ledger selector 때문에 실행0이었던
R-V는 그대로 보존하고, active_followup/coordinator_plan_gate를 읽는 V2에서만 검증했다.
이어 초기 shutdown이 true여도 active 수요/lease 검증에 먼저 막히던 D2 경로를 보완했다.
이 never-active cleanup 경로는 관측시각을 새로 만들거나 lease를 바꾸지 않으며 기존
소켓 close·5초 cooperative drain·모든 child join·fenced retirement를 유지한다.
wp3d2cleanupv-root-review.json의 실제 idle/pending0, source155/private871와 원본
compile2/pure4를 확인해 source/순수 범위를 인수했고 a238b0cc heartbeat를 삭제했다.
실제 socket/DB cleanup·deadline·rollback·cadence·heap·soak 통과를 뜻하지 않는다.

사용자가 다시 직접 “이어서 진행해”라고 지시했다. 다음 단위는 기존 §13.1의 runner
선택 연결뿐이며, live/production/credential 접근 또는 실제 daemon 실행을 추가 승인하지 않는다.
Execution skill: $paseo-delegate (required)
Native subagents: prohibited for worker packages

| Package | Complexity | Basis | Confidence | Reclassification / escalation |
|---|---|---|---|---|
| WP-3D3-C | intermediate | 기존 runner 두 파일의 한정 분기·공개 타입·불변 credential snapshot 연결; root가 수명/검증 계약 확정 | high | 동일 compile 오류2회, 보존 block/범위 밖 변경 필요, 권한·수명 모순은 root로 반환 |
| WP-3D3-C-V | simple | root가 새 pure8 소스를 읽은 뒤 exact 실행을 고정 | high | 미선택·실패·timeout·핀 변경 시 중단 |

| Package | Wave | Objective | Owned scope | Depends on | Worker | Deliverable | Verification |
|---|---:|---|---|---|---|---|---|
| WP-3D3-C | 1 | off/once/REST/EOD 보존 및 market_ws daemon 진입점 | runner의 지정 config/build/startup hunks와 새 private helpers/pure8; parent의 exact safe export1개 | 인수한 B/R 및 D2 cleanup | 기존 Luna max/auto-review | private wp3d3c-report.md·source patch·raw compile2 | default/DB-feature lib+runner --no-run만 |
| WP-3D3-C-V | 2 | 검토된 startup selection/pin/snapshot 순수 분기 검증 | 모든 소스 읽기 전용 | C 실제 idle/source/raw compile 인수 | 기존 worker, 별도 root gate | exact pure 원본 로그 | 새 소스 검토 후 명령 확정; 현재 실행 권한 없음 |

정확한 소유 파일은 crates/job-queue/src/bin/owner-equity-v2-runner.rs와 부모
crates/job-queue/src/owner_equity_v2.rs다. 부모는 runtime/exit safe export만 추가한다.
실행기의 EOD continuation 전체·기존 startup helper·기존 tests·token/quota/issuer 의미를
보존한다. daemon+owner_only에서 rest는 기존 REST, market_ws는 WS 한 가지를 선택한다.
off/once/healthcheck는 WS pin loader·domain/approval/runtime factory를 호출하지 않는다.
WS만 선택한 경우 정적 비밀 없는 slot/grant/contract pin을 검증하며 기존 shared generation을
재사용한다. ProductionReadCoordination과 key/secret snapshot을 시작 시 한 번 고정하고
EOD와 WS construction이 같은 snapshot을 쓴다. WS factory는 기존 fixed production domain과
ApprovalClient 및 accepted runtime만 연결하며 파일 재독해·URL/path override·새 token manager가 없다.
WS factory/runtime 오류는 REST fallback이나 자동 재시작을 만들지 않고 EOD 흐름을 보존한다.
외부 timeout으로 task를 버리지 않으며 기존 runner 종료 signal과 task await를 보존한다.

새 pure8은 코드만 작성하고 실행0이다. lib와 정확한 runner test target을 default,
그 다음 market-stream-db-tests로 각각 --no-run --locked --offline/CARGO_BUILD_JOBS=2/
공유 lock/600초 이내 컴파일한다. DB-feature는 컴파일만 한다. 내부 소유 hunk의 compile 오류는
원본을 보존하고 제한적으로 수정하되 같은 오류2회/범위 밖 필요 시 중단한다. Root 소스 검토와
별도 C-V gate 전에는 기존/신규 테스트도 실행하지 않는다. D4/DB/loopback/provider/actual run/
protected credentials/production/Docker/services/commit/push와 다음 단위 실행은 포함되지 않는다.

### WP-3D3-C-F-V — Send 계약 보완 후 실행기 검증

2026-10-03 사용자가 실패를 수정하며 완료까지 지속 실행(“ralph”)을 명시했다.
현재 도구/스킬/명령에서 Ralph 전용 기능은 확인되지 않아, 코디네이터의 지속 실행 goal을 활성화했다.
동일 source/offline/local 범위의 실패는 구체적인 수정·검증 gate를 갱신하여 계속 처리한다.
이는 새 live/provider/production/credential/account/order/commit/push 권한을 만들지 않는다.

D3-C는 actual idle/pending0와 root raw-log/source/pins 검토 후 실패 결과를 회수했고,
복구 heartbeat edc3aeb6을 삭제했다. 기본 compile exit101 E0277의 원인은 supervisor의
BoxFuture 타입에서 Send를 지운 것이며 DB-feature compile/pure8은 NOT_RUN이다.
root가 해당 타입 별칭에만 Send를 추가하고, runner의 마지막 합성 자격증명 테스트에
서로 다른 key/secret sentinel 및 비노출 equality 검사를 추가했다. 취소·타이머·executor·
DB/SQL·소켓 정리 동작, 원본 EOD continuation과 기존 runner 테스트는 그대로 보존한다.

Execution skill: $paseo-delegate (required)
Native subagents: prohibited for worker packages

| Package | Complexity | Basis | Confidence | Escalation |
|---|---|---|---|---|
| WP-3D3-C-F-V | simple | root가 소스와 정확한 pure10 본문을 검토한 고정 compile2/test10 검증 | high | 새 컴파일 오류/실패/선택 오류/핀 불일치면 원본을 회수해 root가 다음 bounded 수정 gate를 열며 같은 범위의 사용자 승인을 다시 묻지 않는다 |

실행 그래프: root Send 별칭·test 보완/역변환 확인 → 기본 lib+runner --no-run →
DB-feature lib+runner --no-run → runner pure8 + supervisor pinned-stage/terminal-observation pure2.
각 명령은 sanitized env, --locked --offline, 공용 flock, jobs2, workspace target을 사용한다.
컴파일 각600s, exact 순수 테스트 각180s/1선택1통과이며 실패시 후속은 NOT_RUN으로 보존한다.
작업자는 소스를 수정하지 않는다. root가 actual idle/pending0, 원시 로그·핀·명령 선택을 검토한다.
순수 PASS는 실제 DB/socket/task cleanup/cadence/heap/soak/live 또는 FINAL_ACCEPTED를 증명하지 않는다.

### WP-3D4-S — 실제 runtime 통합 테스트 소스 준비

C-F-V 실제 완료 callback과 idle/pending0를 확인하고 root가 두 컴파일 원본, 정확한 기본
feature 테스트10개의 각1개 통과, source157/private926 및 기존 EOD·REST·runner tests 보존을
검토했다. `BoxFuture + Send` 보완은 required runner spawn 컴파일을 해소했다. 감사 도구의
prefix·출력 판독 오류는 원본과 함께 남겼고 테스트 재실행은 없었다. heartbeat172aa253은
회수 후 삭제했다. 이 인수는 실행기 source/compile/pure 범위이며 실제 runtime 증거가 아니다.

지속 실행 목표 아래 다음 단위는 hard/medium인 D4-S다. 기존 Luna max 작업자가 실제
`run_daemon`을 호출하는 DB-feature 테스트3개의 소스와 private 합성 서버를 작성한다:
무수요에서 연결0, 당일 시간표 누락에서 positive demand에도 연결0, 30종목을 1개에서
10개 소비자로 늘렸다 줄여도 같은 epoch·소켓·구독30을 공유하고 정상 종료하는 경우다.
현재 grant/실제 app·worker 역할, calendar resolver, 실제 ACK/receipt 및 snapshot 경로를 쓴다.

소유 범위는 runtime 파일의 test-only window 공급 분기 한 곳과 child test module 연결,
신규 `market_stream_runtime_tests.rs` 및 `market_stream_runtime_test_support.rs`뿐이다.
분기·연결을 역으로 제거하면 C-F-V runtime104780 bytes가 그대로여야 한다. 합성 window는
task-local의 검증된 bytes 계약으로 공급하고 운영 파일·환경·proof scalar를 바꾸지 않는다.
새 fixture는 SystemClock과 실제 1000ms command spacing을 사용하며 모든 소켓·task를 소유하고
오류가 나도 종료와 join 결과를 회수한다. 나머지 source와 역사적 증거는 고정한다.

D4-S는 default/DB-feature library+runner `--no-run --locked --offline`만 허용한다.
공유 Cargo lock, jobs2, workspace target,600초 제한을 유지하고 내부 범위 컴파일 수정만
각 configuration 최대2회 허용한다. 동일 오류 반복·범위 밖 오류·timeout·핀 변동은 즉시
증거를 반환한다. 테스트·PG·DB·loopback은 이 단위에서 실행하지 않는다. root가 작성 소스를
읽은 뒤 새 fixture/runner/case/종료 계획을 고정해 별도 로컬 실행 단위를 연다.

slow DB·commit uncertainty·권리/generation/lineage·취소·재접속·cadence·heap 및 변경 영향
회귀는 후속 D4 항목으로 남는다. API/UI/30분 soak/live와 전체 완료도 별도 증거가 필요하다.
private wp3d4s-dispatch/scope/exact-seams/root-gate/reviewed-pins를 실행 기준으로 사용한다.
같은 승인 범위의 실패 수정·후속 작업은 root가 구체화해 계속 진행하며 재승인을 묻지 않는다.

### WP-3D4-SF — 테스트 정리·관측 보완과 조회 freshness 수정

D4-S의 실제 idle/pending0와 source156/private969, 원본 prefix, 최종 두 컴파일을 root가
회수했다. DB-feature 첫 컴파일 실패도 보존했다. 테스트3개는 미실행이며, setup/cleanup을
빠뜨린 시간 제한과 nested reader detach 가능성 때문에 실행 인수는 하지 않았다.
추가 검토에서 해제한9개 renewer까지 실행 중으로 요구하는 검사와 shutdown 이전에 잡은
불변 비교 기준, DB 관측4개의2초 제한 누락도 확인했다. Root는 inline reader/writer 소유,
명시적으로 해제된 소비자 구분, daemon 반환 이후 기준, 관측2초 제한을 test-only 두 파일에
보완했다. 나머지 시간 제한·부분 준비 실패의 소유 자원 회수는 별도 SF 소스 gate로 진행한다.

같은 실제-idle 경계에서 `cache_row_is_live`의 receipt3초 검사를 계약상의 receipt/event
각30초로 수정했다. 기본 feature 순수 회귀1개가 수정 전 예상 panic으로 실패하고 수정 후
통과했다. 미래·누락 시각과30초 경계도 검사하며, publication3초 및 나머지 원본 바이트는
유지했다. 실제 조용한 종목의 DB/runtime 검증은 아직 별도다.

SF는 기존 작업자의 SOURCE/COMPILE-ONLY 작업이다. 신규 test 파일2개의 시간/cleanup 접점과
기존 guarded fixture 파일 끝의 bounded 생성·정리 adapter만 허용한다. 기존 fixture 전체
prefix/SQL·역사 상수는 보존한다. 실제 C2 binding 신원 확인, generated name 부재 확인과
소유권 기록 뒤에만 DDL을 준비하며, 준비 실패도 같은 소유 DB의 cleanup 결과와 함께 남긴다.
하나의150초 절대 예산에 setup35초, body120초 시점까지, 나머지 cleanup을 배정한다.
취소된 child는 모두 회수하고 시간 초과/강제 취소/불확실 cleanup은 PASS가 될 수 없다.
미래180초 process watchdog은 깨진 정리의 실패 회수용이며150초 성공 기준을 늘리지 않는다.
현재는 코드 작성과 offline 기본/DB-feature `--no-run`만 수행한다.

Root가 독립 구현한 Web schema2 row/SSE decoder와 메모리 상태기는 focused pure26개,
typecheck와 Biome를 통과했다. 현재 connection cursor, reset/snapshot, 권한 상실 purge,
동일 cache-version의 상태 overlay, 서버 시각30초/침묵5초 처리와30개 상한을 검사했다.
아직 EventSource/lease controller/화면 연결이나 실제 API·브라우저 증거는 아니다.
§8의 초기 renewal_sequence0 및 nested SSE body/membership 정렬은 기존 durable domain과
새 client/API 사이의 명시적 계약으로 정리했다. Source/local/runtime/live 완료를 구분한다.

### WP-4 — 인증 API/SSE와 nginx

- **cwd/분류/선택:** `/data/worktrees/3puw275b/needy-snake`; intermediate, high confidence;
  Luna max. WP-3 채택 및 frozen spec §8이 입력이다.
- **사실/목표:** 기존 API는 schema1 REST GET이다. 새 schema2 stream lease/SSE를 별도
  경로로 구현하며 기존 REST DTO에 WS 값을 넣지 않는다.
- **소유 파일:** 신규 `crates/api-server/src/{http,repos}/owner_market_stream.rs`,
  관련 mod/state/session/config의 한정 접점, API manifest/필요 Cargo.lock, API tests,
  `apps/api-server/scripts/openapi-spec.mjs`, `openapi.json`, `generated/openapi.ts`,
  필요한 contract checker, `deploy/nginx/nginx.conf`와 그 focused test만.
  Web/ops/job-queue/migration 수정은 금지하며 저장소 API 부족은 WP-3에 반환한다.
- **구현:** 정확한 POST/DELETE stream-leases와 GET market-stream, Owner/session 바인딩,
  mutation CSRF·멱등성, LISTEN→RLS snapshot→delta race 해소. 매 전송 직전과 idle 1초마다
  세션·권한을 재검증한다. 재접속은 Last-Event-ID와 무관하게 reset+현재 snapshot이다.
- **자원/보안:** Owner당20 SSE, consumer당30 latest rows+snapshot+16 controls 및256KiB,
  5초 쓰기 불가 시 해당 consumer 종료. 1초마다 authenticated status, 15초 comment heartbeat,
  알림 유실 시1초 내 DB 재조회. 정확한 nginx route에서 buffering/cache/gzip을 끈다.
- **검증:** `cargo test --locked --offline -p api-server --test <신규_stream_target>`와
  `npm run openapi:check --workspace @lagrange/api-server`. 실제 role DB/HTTP/nginx를 통해
  만료·logout·타 Owner/세션·DB outage·버전 역전·slow consumer·notification loss를 검증한다.
  이미 존재하는 일반 job SSE를 시장 SSE의 완료 증거로 쓰지 않는다.
- **산출물/상향/보고:** API/프록시·스키마와 실제 검증 결과. 인증/동기화 결함 반복 시 상향.
  공통 보고에 REST 보존, broker 호출0, revocation 지연과 메모리 상한을 추가한다.

### WP-5 — 30종목 화면과 정상 등록·lifecycle

- **cwd/분류/선택:** `/data/worktrees/3puw275b/needy-snake`; intermediate, high confidence;
  Luna max. WP-3와 frozen spec §8–9를 입력으로 WP-4/6과 독립 구현한다.
- **사실/목표:** 현재 위젯은 선택 종목 하나를 poll한다. 고정30 목록과 pilot-first 로직은
  기존 Owner QA에 존재하지만 30행 실시간 화면을 구현한 것은 아니다.
- **소유 파일:** 신규 `apps/web/lib/products/intraday-stream-{contracts,client}.ts`,
  `apps/web/components/stock-beta/quote/`의 controller/components/hooks와 dashboard/detail
  필요한 wiring, 관련 Web unit/e2e/fixture, 필요 npm manifest/lock,
  `scripts/qa/owner-equity-v2-live-acceptance.{mjs,test.mjs,d.mts}`.
  API/generated OpenAPI/nginx/ops/DB와 고정30 원본 목록은 편집하지 않는다.
- **선행 확인:** Web AGENTS/CLAUDE 및 실제 설치된 Next의 `node_modules/next/dist/docs/`를
  읽고 버전별 API를 확인한다. 로컬 문서 부재는 위치/의존성을 확인하고 추측 구현하지 않는다.
- **구현:** 정상 Owner API로 pilot READY→나머지29 순차·멱등 등록, 부분 성공 재개.
  탭당 controller 하나/lease 하나/EventSource 하나로 board/detail을 공유한다. 30행 전체의
  마지막 체결·수신 시각, 무체결·stale·closed·gap·거래정지 관측 상태를 정직하게 표시한다.
  hidden/offline/이탈/logout 시 SSE·갱신 종료와 bounded release, 복귀 시 새 consumer를 생성한다.
  WS 모드에서 기존 quote GET을 끄고 가격을 브라우저 영구 저장하지 않는다.
- **검증:** `npm run typecheck --workspace @lagrange/web`, focused Vitest와 Biome,
  `node --test scripts/qa/owner-equity-v2-live-acceptance.test.mjs`.
  실제 React/Chromium에서 30행·상세·다중탭·재접속·hidden/offline/logout·REST/off 회귀를 검증한다.
  fake API 환경의 결과는 Web 단계 증거이며 최종 실제 API 통합은 WP-7에서 수행한다.
  QA가 화면과 별도 숨은 demand를 만들거나 fake DOM 객체로 화면 검증을 대체하면 안 된다.
- **산출물/상향/보고:** 사용 가능한 화면과 등록/브라우저 검증. DOM 불일치·중복 demand·
  lifecycle 회귀 반복 시 상향. 공통 보고에 실제 프로세스/URL·테스트 수·미검증 인증 범위를 추가한다.

### WP-6 — 운영 설정·provisioning·grant·rollback

- **cwd/분류/선택:** `/data/worktrees/3puw275b/needy-snake`; intermediate, medium confidence;
  Luna max. WP-3 mode/config/초기화 interface와 spec §9를 고정 입력으로 사용한다.
- **사실/목표:** 기존 immutable release 도구는 존재한다. 새 domain의 첫 초기화와 재적용
  검증을 연결하고, 설치만으로 WS가 활성화되지 않게 한다. WP-2가 미완료면 synthetic 입력만 쓴다.
- **소유 파일:** `scripts/ops/{validate-production-config,compose-release,provision-linux}.sh`,
  `scripts/ops/lib/kis-read-compose.sh`, 관련 static/self-test, 신규
  `scripts/ops/install-owner-market-stream-grant.sh`, `deploy/compose/compose*.yml`,
  관련 `.env.example`, 필요한 기존 runtime/build Dockerfile의 runtime 파일 배치,
  신규 `docs/runbooks/kis-market-stream-operations.md`, 기존 intraday/release runbook의 연결 설명.
  최초 domain 초기화는 spec§16의 별도 `scripts/ops/provision-owner-market-stream.py`와
  `scripts/ops/test_provision_owner_market_stream.py`로 연결한다. 기존 provision-linux는 parent만 유지한다.
  nginx/API/Web/Rust/migration 및 WP-2 readiness 문서는 편집하지 않는다.
- **구현:** runner에만 writable state0700/0600과 read-only root-owned anchor0750/0440
  mount, `create_host_path:false`, 슬롯/domain 검증. 첫 신규 설치에서만 WP-3 initializer를
  실행하고 재적용은 inode·권한·상태를 검증만 한다. missing/zero-length/불일치 상태 자동 수선 금지.
  grant helper는 기본 plan, exact approval input/commit으로만 parameterized insert·정확 replay·
  지정 revoke를 수행한다. immutable release installer의 DB/provider-free 성질은 유지한다.
- **운영안:** 배타적 REST/WS mode, env/image/manifest/daemon/source pin, 현재 proof,
  정지→socket 회수→off/호환 release 복원, migration 이후 backward compatibility를 명시한다.
  기본 rollback은 검증된 off 경로이며 REST fallback을 자동 활성화하지 않는다.
  운영 builder의 서비스별 직렬 실행과 2~3서비스마다 자원·health 확인 방식을 확정한다.
- **검증:** `bash scripts/ops/stock-beta-intraday-static-check.sh`, 해당 self-test와 새
  stream focused tests, shell 구문/필요 Python tests. 공식 helper 본문을 disposable 경로와
  fake Docker에서 직접 실행한다. root UID·실제 bind mount 검증은 승인된 disposable 환경에서
  별도 수행하며 fake 결과로 대체하지 않는다. 실제 운영 디렉터리·DB·provider는 건드리지 않는다.
- **산출물/상향/보고:** exact build/install/provision/grant/activation/rollback 순서,
  입력 manifest와 tests. 초기화 interface 공백·권한 실패는 영향 작업을 WP-3로 반환한다.
  공통 보고에 실제 root/mount 증거와 fake 검증의 차이를 명시한다.

### WP-7 — 전체 경로 독립 검증

- **cwd/분류/선택:** `/data/worktrees/3puw275b/needy-snake`; hard, high confidence;
  Sol high. WP-4/5/6 완료, 코디네이터 계약·도면 통합, 안정된 candidate commit이 필요하다.
- **목표/사실:** 기존 focused DB/loopback 통과는 전체 서비스 인수가 아니다.
  fake market WS→실제 runtime/producer→실제 writer-role DB→인증 API/SSE→nginx→Chromium을 연결한다.
- **소유 범위:** 격리 QA 환경·비공개 결과, 신규 `scripts/qa/kis-market-stream-e2e/`
  harness와 테스트 전용 통합 scenario. 제품 소스·명세·도면은 읽기 전용이다.
  공개 환경을 바꾸거나 production auth bypass를 추가하지 않는다. fixture 로그인은 격리 DB 안에서만 쓴다.
- **검증:** 모든 migration/실제 역할, 30종목, 1/10 탭 동일 upstream30·REST quote0,
  ACK 이전 게시0, noisy symbol/burst, 느린 DB/consumer, 알림 유실·재시작, cache 재생성,
  cancellation/unknown commit, 권한/세대/날짜 변경, 마지막 수요 종료를 실행한다.
  >=30분 soak에서 stream buffer≤8MiB와 RSS 추세, write≤4tx/s·120rows/s, SSE queue 상한,
  latency p95≤2s 및 coalesced 수를 측정한다. API status1Hz는 실제 새 가격으로 세지 않는다.
- **회귀:** 아침/EOD 양방향 calendar 재사용, 실제 app-role resolver, exact-source EOD 재시작,
  entitlement replay/RLS, REST shared token/debt, V1/rest/off UI, env/image/daemon guard,
  migration forward 및 off rollback 호환성. 실행 명령·선택한 test 수를 고정한다.
- **산출물/거절/보고:** 후보 hash와 재현 가능한 ACCEPT/REJECT matrix, 현재 high/medium
  finding 및 남은 gate. 결함은 원 소유자에게 반환하고 변경 영향만 재검증한다.
  G1–G5/운영 인수는 별도다. 공통 보고에 실제 socket/DB/인증/브라우저 환경과 측정 한계를 추가한다.

### WP-8 — 운영 후보 빌드·설치와 off 상태 준비

- **cwd/분류/선택:** `/data/worktrees/3puw275b/needy-snake`; hard, medium confidence;
  검증된 명령만 실행하는 Luna max. 운영 판단·다음 단계 승인은 코디네이터 소유다.
- **입력:** WP-7 ACCEPT, WP-6의 실제 검증된 runbook, 깨끗한 exact commit·필요 push/merge
  권한, 현재 운영 preflight와 bounded 실행 범위. G1/G3 미확정이어도 provider-free off 준비만
  별도 허용 범위에서 가능하며 grant/WS 시작은 WP-9까지 닫는다.
- **소유 범위:** 승인된 build/install/provision/migration/health 점검과 비공개 증거만.
  source 수정·설치본 수동 패치·credential 출력·provider 요청·임의 서비스 복원은 금지한다.
- **순서:** 현재 installed/current/env/image/daemon/readers·jobs/timers·backup/restore·
  메모리/swap/OOM/서비스 확인 → exact inputs → 공식 image builder → immutable installer →
  현재 DB의0055 충돌·호환 검사 → 공식 provisioning/migration 및 off 경로 검증.
  실제 역할별 작업 순서는 WP-6 runbook과 현재 상태를 대조해 고정한 뒤 실행한다.
- **자원:** 낮은 CPU/I/O 우선순위 background systemd, `COMPOSE_PARALLEL_LIMIT=1`,
  `CARGO_BUILD_JOBS=2`. 2~3서비스를 논리 batch로 묶되 Compose 호출은 언제나 서비스 하나씩이다.
  batch 사이 memory/swap/OOM/exit/service health와 compiler 잔류0을 확인한다.
  전 이미지 병렬 그래프는 금지한다. OOM/서비스 종료면 중단하고 승인된 복구만 수행한다.
- **검증:** 공식 builder로 최종 12-image IDs/OCI revision과 strict manifest를 검사하고
  `scripts/ops/deploy-production-release.sh`로 exact commit을 설치한다. 설치/env/image 일치,
  보호 상태 소유권·anchor inode, migration/RLS·off 상태 broker 호출0·서비스 health를 검증한다.
  release installer는 DB/provider를 호출하지 않으며 그 밖의 운영 단계와 증거를 분리한다.
- **산출물/중단/보고:** `INSTALLED_OFF`, exact pins, 적용/미적용 변화와 실제 rollback 준비.
  불일치·자원 부족·호환 실패는 임의 재시도 없이 보고한다. 공통 보고에 현재 host 관측 시각을 명시한다.

### WP-9 — 실제 수신·공유·화면·EOD 최종 인수

- **cwd/분류/선택:** `/data/worktrees/3puw275b/needy-snake`; hard, medium confidence;
  Sol high. WP-8와 실제 G1–G4 충족·코디네이터의 현재 bounded dispatch가 선행한다.
- **목표/입력:** 검증된 설치본, 정확한 approval/grant 입력, 당일 proof, 정상 Owner 인증,
  30종목 원본 목록과 WP-5 QA, 실패 시 정지/복구 명령·관측 예산.
- **소유 범위:** 승인된 grant 설치·시장 시세 활성화·정상 등록·브라우저 관측·EOD·종료/
  복구만. source 변경, DB READY 강제, token/claim/state 초기화, API 확대는 금지한다.
- **실행 단위:** ① 최신 read-only preflight ② 정상 pilot READY 및 시장 구독의 G5 확인
  ③ 나머지29 순차 admission/30 ACK ④ 실제 SSE/DOM·공유·lifecycle 측정 ⑤ 동일 출처 EOD.
  각 단위는 현재 증거를 회수한 뒤 진행한다. 하나의 무제한 production 작업으로 보내지 않는다.
- **검증:** 대표 종목의 첫 표시 이후 두 fresh receipt, 실제30 ACK, 다중탭 upstream중복0,
  반복 REST quote0, p95 측정, 정지 후 구독0과 폭주0, logout/철회/재연결/gap을 확인한다.
  실제 장애 주입은 승인된 좁은 범위만 수행한다. 조용한 종목은 무체결 상태로 남긴다.
- **인증/날짜:** 기존 정상 인증 경로를 사용하며 사용자만 가능한 로그인일 때만 한 단계의
  도움을 요청한다. credential/cookie 추출·세션 위조 금지. 거래시간·installer cutoff를
  우회하지 않고 시장 종료·proof 부재면 정확한 다음 실행 조건을 기록한다.
- **마감/보고:** 실제 동일 calendar 출처의 EOD 게시와 서비스 건강, 다음날 proof 공급 절차,
  운영 변경·복구 상태·종료 증거를 회수한다. 공통 보고와 최종 수용 matrix를 반환한다.
  한 항목이라도 미실행이면 `FINAL_ACCEPTED` 대신 완료한 단계와 남은 조건을 명시한다.

## Coordinator gates

### 1. Pre-launch checks and user decisions

1. 후속 실행 요청과 현재 workspace/HEAD/diff·적용 지침을 확인한다. 후속 실행 요청은
   2026-10-01에 확인됐다. 이전 분석용 native agent는 이 계획의 worker로 사용하지 않는다.
2. `$paseo-delegate` 최신 지침과 Paseo profiles/notes·MCP를 읽고 정확한 provider/model/effort를
   확인한다. 스킬이 정한 callback·권한·monitoring을 따르며 이 문서에 별도 launch 명령을 복제하지 않는다.
3. 현재 source pins·기존 검증 증거를 확인하고 WP-1/2를 보낸다. 이미 채택된 결과를 복구할 수
   없으면 그 부분만 미검증으로 표시한다. 출처 없는 PASS나 과거 권한 재사용을 금지한다.
4. 사용자가 결정할 수 있는 항목은 G1의 정확한 신규 접속·평문 조건, G3의 기존 권리 적용
   차이, 현행 한도가30 미만일 때의 제품 범위, 실제 필요한 운영 실행 범위다. 먼저 기존
   승인을 확인하고, 구체적인 검토 결과·후보·실행안을 준비한 뒤 부족한 결정만 요청한다.
5. 로컬 DB/브라우저/빌드 환경의 소유권·자원·종료 범위를 확정한다. runtime 승인이 필요한
   경계는 현재 지침에 맞춰 처리하며 새 범용 proof framework를 만드는 작업으로 확대하지 않는다.

### 2. Per-wave integration and verification

1. WP-1에서 runtime/초기화 인터페이스와 수정 manifest를 채택해야 WP-3를 시작한다.
   WP-2 결과의 미해결 G는 기록하되 독립 local 작업의 시작 조건으로 잘못 추가하지 않는다.
2. WP-3의 실제 역할·loopback·자원·영향 회귀를 검토하고 소스 pin을 고정한다. 기존 C2의
   opaque proof, ACK/commit 선후, 취소 terminal 성질을 약화하면 채택하지 않는다.
3. Wave3의 파일 소유권을 먼저 확인하고 구현을 병행한다. 공용 Cargo lock·자원 한도를
   지키며 안정된 트리에서 계약·빌드를 검증한다. 테스트 환경 부재/skip/0 tests를 통과로 세지 않는다.
4. worker 실제 종료·pending permission·변경 diff·보고서·검증 결과를 회수한 뒤 결과를 채택한다.
   실패는 원 소유자에게 bounded 수정으로 반환한다. 재작업 범위와 필요한 회귀를 기록한다.
5. 코디네이터가 `docs/diagrams/component_architecture.puml`과
   `docs/diagrams/runtime_deployment.puml`의 실제 `file:line` 근거를 갱신하고 아래 명령으로
   로컬 PNG를 렌더한다. 계획상의 연결은 그리지 않는다. 소스·도면·PNG를 같은 통합 변경에 포함한다.

   ```sh
   docker run --rm -e PLANTUML_LIMIT_SIZE=16384 -v "$PWD/docs/diagrams:/data" plantuml/plantuml -tpng /data/component_architecture.puml /data/runtime_deployment.puml
   ```

6. API/Web schema2, mode/env, initializer/installer, migration/roles, 기존 REST 보존을
   대조해 안정된 candidate를 WP-7에 넘긴다. 이 계획만 추가하는 현재 변경은 구조 변경이
   아니므로 도면을 지금 수정하거나 렌더하지 않는다.
7. WP-7 ACCEPT와 필요 회귀 통과 뒤 main 통합·릴리스 후보를 확정한다. merge/rebase로 코드가
   달라지면 영향 검증을 갱신한다. root만 exact reviewed 범위를 커밋하며 main push/배포의 기존
   권한을 대조한다. 구조 문서가 누락된 checkpoint를 그대로 통합 완료로 처리하지 않는다.
8. WP-8 앞에서 모든 운영 사실·권한·자원·백업을 새로 확인한다. WP-9 앞에서는 G1–G4,
   정상 Owner 세션·당일 proof·실제 설치 commit을 다시 확인한다. 과거 만료 wrapper를 연장하지 않는다.

### 3. Final end-to-end acceptance checks

| 항목 | 최종 증거 | 실패/미실행 시 판정 |
|---|---|---|
| Source/local | WP-7 독립 ACCEPT, 실제 역할 DB/실제 API/nginx/Chromium, soak/회귀 | source/local 미완료 |
| Deployment | exact commit/env/manifest/12 images/설치본, migration와 healthy 서비스 | 설치 또는 활성화 미완료 |
| Onboarding | 정확한30의 정상 READY·generation·멱등 등록 | 30종목 목표 미완료 |
| Provider | 허용된 시장 채널, 실제30 ACK, G5 프로토콜 확인 | live 미완료 |
| Shared collection | 1/10 소비자의 상류30 동일, quote REST0, memory/write/queue/명령 상한 | 공유·성능 미완료 |
| Live screen | 두 후속 실제 수신의 DB/SSE/DOM 일치와 p95, quiet/stale 상태 | 화면 인수 미완료 |
| Lifecycle/security | 재접속·정지·권한철회·세대/날짜 변경, 비밀/원문 노출0 | 운영 인수 미완료 |
| EOD/지속성 | 같은 날짜·출처의 EOD 게시, 다음날 proof 공급 절차·rollover 검증 | 당일 또는 지속 운영 미완료 |
| Closeout | 사용자가 확인할 화면/결과, 남은 제한, rollback 위치, worker/자식/환경 정리 | 완료 보고 보류 |

증거에는 실행 시각·source pin·실제 명령·선택 test/관측 수·exit·cleanup을 남긴다.
기술적 PASS와 사용자가 확인한 결과를 구별한다. 사용자의 최종 확인을 자동 검증 대체로 쓰지 않는다.
미완료가 있으면 해당 분기와 재개 조건을 명시하며 나머지 승인된 독립 작업은 끝까지 처리한다.

### 작업량과 갱신 시점

앞선 4~7주 평가는 전담 1인, 현재 계약 유지, 큰 재작업이 없다는 가정의 거친 규모다.
Paseo worker 수로 나눠 완료일을 계산하지 않는다. Wave3는 병행 가능하지만 runtime 채택,
전체 통합, 직렬 운영 빌드, 시장 관측/EOD는 의존성이 있다. WP-1 완료 후 실제 결손과
fixture 재사용성을 기준으로 작업량을 갱신하고, WP-7 완료 후 운영 일정을 확정한다.
공식 근거·권한·인증·당일 입력·시장 개장 대기는 구현 작업량과 따로 보고한다.


### WP-4 API 현재 경계와 H 검증 소스 (2026-10-03)

코디네이터는 WP-4-R-V의 실제 idle/pending0, 고정 source157/private1071,
원시 compile2/exactpure6 로그를 대조했다. app의 private grant SELECT를 추가하는 대신
기존 cache composite FK/RLS를 유지하고, 새0057의 app 전용 boolean helper로
immutable slot/grant/source-contract와 canonical session을 확인하는 구조다.
기존0055/0056와 private grant SELECT 권한은 바꾸지 않는다.0057 적용/rollback은 아직 미검증이다.

API는 schema2 lease POST/consumer-bound DELETE/단일 SSE를 별도로 mount했다.
LISTEN 이후 RLS snapshot을 읽고, body가 실제 batch를 꺼낼 때 canonical session/role,
현재 lease/membership/date/rights와 배포 pin을 다시 읽는다. 별도 watchdog는1초 주기로
재검증하며 mailbox에는 가격 bytes 대신 bounded wake ordinal만 둔다. 신규 source는
DB 권한 철회 이후 stale 직렬화 값을 보내는 경로를 만들지 않는다. 실제 HTTP·DB 수명은
이 구현 설명이나 순수 테스트만으로 인수하지 않는다.

OpenAPI의 새3개 route/strict DTO/SSE4종류/closed reason/canonical string counter와
정확 SSE nginx location을 추가했다. 기존 REST 계약은 numeric literal까지 대조해 보존했다.
root의 API contract/config/delivery/projection target22, OpenAPI19, nginx5가 통과했다.
설정 exact3 중2개는 통과했고 pool-size 테스트는 fixture가 값과 `_FILE`를 동시에
공급해 Ambiguous로 실패했다. 원본 로그를 유지하며 H의 source freeze 종료 뒤 fixture만
키별 lookup으로 보완한다. 실제 pool·LISTEN 용량 검증을 이 순수 결과로 대신하지 않는다.

WP-4-H는 기존 Paseo worker에 source/compile-only로 배정했다. 범위는 신규
`crates/api-server/tests/owner_market_stream_http.rs`와 명시된 새 test support,
private 로그/report뿐이다. 기존 bounded C2 fixture와 실제 역할/real router/정상 synthetic
cookie를 사용한 helper ACL+0057 rollback, lease auth/session/consumer fencing,
SSE30 snapshot/read-only/status,29 replacement/철회,20 consumer cap/drop,
off/잘못된 binding/권리 철회의6개 SOURCE를 작성한다. 테스트 실행 권한은 없고
root가 최종 소스를 읽은 뒤 별도 once-only PG/HTTP gate를 준비한다. no-window/no-price
fixture의 통과도 authentic quote→API/DOM 증명은 아니므로 별도 후속 검증이 남는다.

현재 worker scope/pin/명령/보고서는 private `wp4h-{dispatch,root-gate,reviewed-pins,cases}`와
owned-worker ledger에 고정했다. 코디네이터 소유 contract/OpenAPI/nginx/docs와 쓰기 범위를
분리했으며 모든 Cargo는 같은 lock/jobs2/locked/offline을 사용한다.

### WP-4 HTTP 검증 준비와 WP-6-A source wiring (2026-10-03)

H의 계획 문서 pin 불일치는 동시 root 문서 수정과 manifest 포함 범위가 충돌한 것으로
확인했다. 과거 실패를 보존하고 문서를 제외한 H-C 범위를 새로 고정했다. H-C는 기본
컴파일을 통과했지만 support 파일 이동 후 migration include 경로 두 개와 공유 fixture의
sha1_smol 직접 test dependency가 누락되어 DB-feature 컴파일에서 멈췄다. 실제 idle과
231 source/1108 private pins 및 원시 로그를 회수한 뒤 root가 세 항목만 수정했다.
기존 잠금 버전 그대로 dev-dependency edge만 추가했고 inverse byte proof를 보존했다.
H-C-F는 source233/private1123을 동결한 compile-only 검증이다. HTTP6 실행은 아직0이다.
API 풀 크기 순수 테스트는 lookup fixture가 `_FILE`에도 값을 반환하던 오류만 고쳐
정확히1개 통과했다. 생산 설정 검사 변경은 없다.

WP-6-A는 immutable dotenv의 transport/slot/grant/hash/origin을 검증하고 fixed
compose.market-stream.yml을 active WS에서만 선택한다. API 앱 pool은 기본32/최소24이고,
runner만 state RW와 anchor RO를 받는다. 별도 `--refresh-market-stream`은 exact image
확인 후 API→Web→runner 순서이며 quotes-off 복귀도 같은 순서를 쓴다. 실제 활성화에는
기존 Owner V2 확인과 별도의 WS 확인이 모두 필요하다. 설치/plan/preflight는 provider-free다.
설정7개, 기존 intraday static check, fake Docker REST/WS/off refresh 검증이 통과했다.
plan 출력 이름 변경에 따른 fixture 실패1회는 보존하고 해당 기대만 고친 재검증이 통과했다.
실제 Docker/서비스/DB/provider는 실행하지 않았다. initializer build product·provisioning·
grant helper·실제 UID/mount·runbook·도면 및 전체 통합/운영 인수는 여전히 미완료다.

private 증거: wp4hc-root-review.json, wp4hcf-root-correction.json,
wp4-root-config-03-02.record.json, wp6a-root-source-review.json.


### 2026-10-03 WP-4-H-E 회수 및 WP-6-B 패키징 범위

실제 idle/pending0 및 callback 뒤 root가 단일 runner, compile exit0, exact HTTP6의
각 selected1/passed1,233 source/1139 private pin, 실제 PG identity와 빈 최종 catalog,
SIGINT exit0 및 PID/PGID/socket 부재를 대조했다. `wp4he-root-review.json`에 인수 범위를
기록했다. 인증·lease fencing·app-only binding/0057 rollback·SSE snapshot/철회/연결 제한은
synthetic actual-role 범위에서 통과했다. missing-window fixtures이므로 authentic quote,
TCP/nginx/browser/soak/provider/live 동작을 증명하지 않는다. heartbeat8e04d0f2는 삭제했다.

WP-6-B는 기존 initializer CLI를 runner 이미지에 담는 source/compile 전용 prerequisite다.
코디네이터는 이 패키지에 한해 job-queue Cargo.toml의 opt-in feature forwarding과
required-feature alias target을 허용한다. CLI/library 원본은 수정하지 않는다. D4 recipe만
market-stream-provisioning을 선택하고 daemon과 initializer를 같은 image에 포함한다.
기본 consumer feature와 다른 recipe는 그대로이며, request/H/receipt/bundle/consumer guard가
같은 선택을 검증해야 한다. 해당 feature가 없는 이전 bundle은 새 D4에 재사용할 수 없다.
별도 이미지·서비스·자동 초기화·DB/provider 실행은 추가하지 않는다. 기존 installed release
installer는 계속 DB/provider/Docker-free이며, 실제 provisioning/UID/mount는 별도 검증 대상이다.

### 2026-10-03 WP-6-B 검증 회수와 provisioning 교차 검사

WP-6-B의 metadata와 두 컴파일이 통과했다. Root는 기존 layout 검사의 실패가 새 feature
검사 성공으로 덮이는 shell 반환값 오류를 재현하고, 실패를 즉시 전파하도록 수정했다.
수정된 순수 검사7개의 원시 로그는 모두1개 선택·통과다. Worker case1 기록기는 실행 후
종료 코드 저장 전에 실패했으므로 원본 exit/timestamp는 미상으로 보존했다. 별도 root
WP-6-B-F-V2에서 같은 exact case1만 한 번 실행해 exit0·1개 통과를 기록했다.
source243/private1210와 실제 idle/pending0를 대조하고 복구 heartbeat를 종료했다.
이는 build guard의 source/pure 인수이며 실제 release artifact/image 검증은 아니다.

Provisioning helper의 fake Docker/합성 파일·실제 공통 dotenv/manifest parser 검사15개는
umask077에서 통과했다. 교차 검토에서 host parent는 root:10001 mode0750인데 Rust의
고정 parent 검사는 root:root를 요구하는 불일치를 찾았다. 현재 CLI를 offline build하고
새 private fixture만 마운트한 network-none 컨테이너에서 CLI exit6·새 leaf0을 재현했다.
컨테이너 종료·부재를 확인했다. 실제 운영 경로/DB/provider는 접근하지 않았다.
수정은 `/`와 `/run`의 root:root 검사를 보존하고 `/run/lagrange`만 기존 host parent의
고정 GID10001 계약에 맞춘다. WP-6-D의 보호 source pin 회수 전에는 해당 Rust 파일을
변경하지 않는다. 수정 후 actual UID/mount와 후속 컴파일 검증은 아직 남아 있다.

WP-6-D는 별도 source/syntax-only gate로 신규 grant wrapper/Python/SQL/pure8 SOURCE를
기존 worker에 위임했다. source248/private1249는 읽기 전용이며 실제 DB/Docker/테스트
실행 권한은 없다. Root는 disjoint provisioning helper와 운영 runbook을 계속 검토한다.


### 2026-10-03 WP-6 immutable 전환과 기존 빌드 검사 교차 확인

설치된 env를 바꾸지 않고 다른 commit으로 활성화/quotes-off 복귀하려면 실행 중인 이전
이미지와 새 current 이미지를 구분해야 한다. Root는 `--refresh-from-commit`을
market-stream refresh에만 추가했다. 동일 release root 아래 보호된 이전 V2 manifest가
세 running image의 사전 검사에만 쓰이고, subprocess 밖의 새 manifest는 override와
교체 후 검사에 그대로 남는다. invalid/missing/symlink/foreign/mixed 거부와 active/off
전환·plan/preflight 무변경을 fake Docker로 확인했다. 실제 서비스/생산 동작은 미실행이다.
private 증거는 `wp6transition-root-review.json`이며 운영 runbook에 정확한 사용 범위를 기록했다.

기존 build target guard 16개는 통과했다. generic Cargo graph 검사는 새 D4 target 검사가
작은 정상 작업공간에도 적용되어 실패했고, Cargo 출력 self-test는 새 `"$@"` feature
인자 확장을 아직 처리하지 못해 Cargo 실행 전에 실패했다. 원시 실패를 보존했다.
D4 target 검사를 별도 필수 builder 단계로 분리하고 실제 shell argv 확장을 쓰는 검사를
준비했으며, WP-6-D의 source pin 회수 후 적용/재검증한다. 이 단계는 최종 image나
실제 운영 인수를 뜻하지 않는다.

### 2026-10-03 WP-6 build·initializer·grant의 검증 회수

WP-6-D의 실제 idle/pending0와 보호 pin을 회수한 뒤 root가 준비된 수정을 적용했다.
generic Cargo graph와 D4 전용 target 검사를 분리하고 실제 shell argv를 검사하도록 고쳤다.
WP-6-BC-V의 열 명령은 순서대로 각 한 번 exit0이었다. 기존 graph18 검사, 작은 fixture의
default/opt-in cold/warm Cargo4, 정확한 Python7 및 initializer opt-in build를 원시 로그로
확인했다. selection/audit 형식 오류는 실행 결과를 보존한 채 판독만 수정했다.
source248/private1254와 HEAD가 일치하며 `wp6bcv-root-review.json`에 인수 범위를 기록했다.

Rust provisioning의 `/run/lagrange` parent GID만 기존 host 계약인10001로 맞췄다.
`/`와 `/run`의 root:root 검사, 고정 UID/mode 및 no-follow 조건은 그대로다. 최종 CLI를
network-none 격리 컨테이너의 새 synthetic bind에서 실행해 root 초기화, UID10001 검증,
재초기화/잘못된 slot/상태 소실 거부, state RW·anchor RO mount, inode/bytes 보존과
컨테이너 정리13개를 확인했다. 원본 실패는 별도로 남겼다. 이는 실제 UID/mount/Rust CLI
검증이며, GNU CLI와 필요한 library를 주입한 fixture여서 최종 Alpine release image나
설치 Python helper 전체 실행을 증명하지 않는다. 증거는 `wp6c-uid-green-root-review.json`이다.

Grant helper 검토에서 보호 dotenv에 없는 commit을 외부 값으로 채우는 경로, 정상/오류
종료 후 container 정리 누락, 실제 Docker image 필드와 remove 인자 불일치를 수정했다.
모든 실행 후 exact ID/image/name/operation label을 대조하고 ID와 이름 모두의 부재를
확인한다. 불확실한 결과는 재시도하지 않는다. off release에서 명시한 grant 철회는 기존
WS binding 없이도 current commit/image 보호 검사를 거친다. 정규 JSON은 literal UTF-8로
고정했고 pipe/파일 race 검사와 container 내부 secret-read 실패도 닫힌 오류로 처리한다.
실제 공통 parser와 합성 입력을 쓰는 pure11, 실제 Docker의 소유/부재/foreign-label 정리3이
통과했다. `wp6d-root-correction-review.json`과 `wp6d-cleanup-probe-root-review.json`에 기록했다.

별도 WP-6-D-E runner는 한 번 실행되어 exit0이었다. 현재 task-owned PG18.6에서 새 DB
하나에 원본57 migrations와 role/bootstrap SQL을 적용했다. 원본 grant SQL의 Unicode 설치,
정확 replay, 잘못된 역할6종·불변 필드 충돌·Owner/entitlement 부적합 거부, 철회와 반복 철회
27개가 모두 통과했다. Root는 raw156 명령의 hash/exit, 거부 전후 행 불변, 실제 cluster identity,
생성 DB를 탐지한 catalog와 빈 최종 catalog, 정상 SIGINT exit0 및 PID/group/socket 부재를
독립 대조했다. source/input257와 HEAD도 유지됐다. 실제 idle/pending0와 callback/report를
회수하고 heartbeat2088af8c를 삭제했다. `wp6de-root-review.json`은 synthetic SQL 범위만 인수한다.

도면의 실제 source 연결·file:line 근거와 운영 runbook을 갱신하고 캐시된 로컬 PlantUML로
두 PNG를 렌더했다. 도면은 설치/활성화 증거가 아니다. 다음 WP-7-P는 기존 worker의 읽기 전용
fixture 연결 조사다. production runtime, 실제 인증 API와 browser가 같은 DB/receipt를
사용하도록 필요한 생성·window·cleanup 접점을 특정하며, 아직 소스 수정이나 실행 권한은 없다.


### 2026-10-03 WP-7 준비 중 nginx authority 수정

실제 캐시된 nginx 이미지와 새 개인 Unix upstream/TLS HTTP2 listener를 사용한 재현에서,
명시적 HTTPS 포트가 lease 요청의 `$host`에서 제거되고 SSE의 `$http_host`에는 유지됐다.
lease POST/DELETE 경로에 `$http_host`와 같은 `X-Forwarded-Host`를 전달하는 전용 location을
추가했다. 기존 일반 API 경로와 SSE buffering·timeout 설정은 유지했다.

최종 실제 nginx 검사는 포트 유무 × POST/DELETE/SSE 6개를 통과했고 일반 API Host 동작도
유지됐다. `api-server --test nginx_config --locked --offline`의 6개 검사도 통과했다.
nginx는 SIGTERM 후 exit0, 정확한 container ID 제거와 이름 부재를 확인했다.
원본 재현의 즉시 cleanup 확인 실패와 후속 부재 확인, 첫 green 검증기의 자동 `:443`
추가로 생긴 불일치도 보존했다. 최종 검증은 HTTP2 `:authority`를 명시했다.
근거는 private `wp7n-root-review.json` 및 `wp7n-green2/result.json`이다.
이는 실제 nginx의 전달 동작 검증이며 인증 API·브라우저·runtime의 통합 인수를 대신하지 않는다.


### 2026-10-04 WP-7-E10 양성 경로 및 종료 수요 정리 인수

E9의 마지막 소비자 해제 후 desired count30 잔류를 실제 로그에서 확인했다. 저장소 retirement가 이미 잠근 현재 수요가 비어 있을 때만 subscription reference count를0으로 정리하도록 수정했다. 재참여 수요 보존, 오래된 epoch 거부, 반복 retirement, quote version/capture 보존을 실제 역할 DB 검사3개로 검증했다. 기존 SQL 조건·권한·lock 순서는 유지했다. RETIRE-C는 읽기 범위 이탈로 Cargo0건을 보존했고, C2의 최종 compile3을 수락하되 첫 로그 잘림·정확한 종료 시각 누락 한계는 남겼다.

E10 한 번의 새 고정 실행에서 DB3·실제 runtime fixture1·nginx/Chromium 검사5가 모두 통과했다. 1/10탭 모두 연결1·상류 구독30, 마지막 화면 해제 후 lease/desired/reference/active WebSocket0을 확인했다. 수신→DOM p95는5,039개 관측에서325ms, version regression/gap 및 음수 clock은0이었다. 실제 초기/최종 클러스터 신원과 빈 generated-DB catalog, SIGINT 뒤 PG exit0, 프로세스·그룹·소켓·컨테이너·임시경로 부재를 root가 원본 증거로 대조했다. 작업자의 실제 idle/pending0와 보고서를 회수하고 heartbeat a7aa1207을 삭제했다. root 보고서는 private wp7e10-root-review.json이며, 작업자 보고서의 첫 case SHA 전사 오류를 원본 해시로 바로잡아 기록했다.

이 인수는 격리 양성 경로에 한정된다. 30분 soak, 장애 matrix, 실제 쓰기 cadence/변경 행 수, stream-owned heap8MiB, 독립 인수 및 installed/live/G1–G5는 남는다. 다음 WP-7-MEASURE-1은 실제 publication transaction 안의 테스트 전용 계측을 추가하는 SOURCE/COMPILE 전용 단계다. 생산/default 코드와 SQL·권한·오류 판정은 보존하고, 계측의 커밋 성공/응답 불명/취소를 구분한다. 신규 pure4는 root가 최종 소스를 읽기 전 실행하지 않는다. 기존 owned Luna max/auto-review 작업자를 사용하며 새 reviewer/native agent 경로는 재시도하지 않는다.


### 2026-10-04 WP-7-MEASURE-1/2/3 및 E12 발행 계측 인수

테스트 전용 publication probe와 fixture 연결을 source/compile 및 pure4+3으로 검증했고, 원시 기록의 독립 rolling-window 판독 pure6도 통과했다. E11은 root 실행 핀 목록에 허용 경로 밖 과거 증거83개를 넣은 오류로 사전 검사에서 중단됐으며 DB·브라우저는 시작하지 않았다. 실패 게이트를 보존하고 E12에서 과거 증거 전체는 root manifest 전후 검사로 유지하며 기존 runner 경로 제한을 그대로 적용했다.

E12 단일 실행에서 실제 역할 DB3, runtime fixture1, 기존 nginx/Chromium5가 모두 통과했다. 약65초 동안 발행 본문 시작217개, 확인된 커밋216개, 변경 행4953개를 기록했다. 최종 수요 해제 시 마지막 zero-row precommit drop1개를 커밋과 구별했다. 임의의 반개방 1초 구간에 본문 시작 최대4, 커밋 응답 최대4, 변경 행 최대120을 root가 원본으로 재계산했다. 수신→DOM p95는333ms였다. 이 시각은 transaction 준비 이후 및 commit 응답 이후의 client monotonic 관측이며 서버 내부 시각이라고 주장하지 않는다. 정상 PG exit0, 빈 최종 catalog, 모든 소유 프로세스·그룹·소켓·컨테이너·임시경로 정리와 source745/private2059 전후 핀을 검증했다. 근거는 private wp7e12-root-review.json이다.

입력 병합/개별 drop 수, stream-owned heap8MiB 및 별도 RSS, 30분 soak, 장애 matrix, 독립 인수·installed/live/G1–G5는 남는다. 다음 계측은 실제 receipt adapter의 분류와 버퍼 최고 점유를 유한 scalar로 기록하며 payload·proof·DB 동작을 변경하지 않는다.

### 2026-10-04 WP-7-MEASURE-4/5/6 계측 소스와 순수 검사 인수

Receipt adapter의 실제 입력 분류와 pending/high-water/outstanding 최고 슬롯 수를 고정 크기 scalar로 계측했다. Producer와 parent의 승인된 삽입을 역으로 제거하면 원본 바이트가 복원된다. M4의 최종 default/DB-feature 컴파일과 기존4·신규4 순수 검사를 수락했다. 보고서의 보호 source 수739는 중복 차감 오류이며 root가 실제 manifest742개를 확인했다.

통합 fixture에 Linux GNU System allocator의 usable block 계측을 테스트 전용으로 연결했다. 측정 구간 이전 할당을 포함한 전체 peak를 사용하고 realloc의 겹침은 보수적으로 센다. 기준값이나 RSS를 빼서 heap 수치를 만들지 않는다. API/mock/probe가 포함된 상위 집합이므로 8MiB 이하는 해당 allocator로 포착한 stream 할당의 상한 근거가 될 수 있지만, 초과 시 stream 자체의 초과라고 단정하지 않는다. Native·직접 allocator 우회·fragmentation·stack/kernel은 별도 한계다.

M5-V의 실제 idle/pending0, source748/private2200, 컴파일2 및 exact pure16 원본 로그를 root가 대조했고 heartbeat e16c6f04를 삭제했다. Fixture/entry 역변환과 기존 cleanup 보존도 확인했다. M6의 최종 저장소 모듈은 순수 판독8과 문법 검사를 통과했다. 실제 coalesced replacement, 분류 합계, 슬롯 상한, 시간·계측 세대·누적값 순서, 전체 allocator peak와 별도 RSS 변화율을 검증한다. 근거는 private wp7measure4-root-review.json, wp7measure4v-root-review.json, wp7measure5v-root-review.json, wp7measure6-installed-root-review.json이다.

이 단계에서는 새 runtime을 실행하지 않았다. 다음 E13의 새 고정 1회 실행에서 기존 DB3·브라우저5와 실제 발행/버퍼/힙 수치를 함께 확인한다. 30분 soak, 장애 matrix 및 installed/live·전체 KIS 인수는 계속 미완료다.

### 2026-10-04 E13 실제 버퍼·힙 계측 인수 및 30분 검증 준비

E13의 단일 실행은 DB3·runtime fixture1·브라우저5를 모두 통과했다. 실제 adapter30390건은 신규 buffer5067건과 pending replacement25323건으로 분류됐고, pending/high-water/outstanding의 최고 슬롯은 각각30이었다. 약73초 측정 구간의 global Rust allocator 보수적 최대치는6042056bytes이며 시작 전 할당1035128bytes도 포함한다. 종료 시565224bytes로 감소했다. 별도 RSS 표본 최대45559808bytes, 전체 짧은 구간 OLS 변화율5352707.645bytes/min을 기록했으며 장기 증가율로 단정하지 않는다.

발행 본문218·확인된 커밋217·변경 행5037과 마지막 zero-row precommit drop1을 구분했다. 원시 기록을 독립 재계산한 rolling1초 최대는 본문4·응답4·변경 행120, 수신→DOM p95는309ms였다. source750/private2267, 초기·최종 live identity, 빈 catalog, 정상 PG exit0 및 소유 process/group/socket/container/temp 부재를 확인했다. 근거는 private wp7e13-root-review.json과 wp7e13-report.md다.

후속 E14는 동일 경로에 실제 monotonic1800초의 연속 DOM/fixture 관측을 추가한다. 기존 양성/종료5검사는 보존하고, 30개 identity·epoch·version·ordinal의 진행과 최대5초 관측/진행 공백을 유한 상태로 검사한다. 시작/종료 histogram 차이로 soak 구간 자체의 p95를 계산한다. 순수6과 controller/runner 문법·역변환 검사가 통과했다. fixture2100초·controller2160초·outer2230초는 이 새 시험의 상한이며 생산 timeout을 바꾸지 않는다. 아직 실제 30분 실행은 미수행이고, 장애 matrix·독립 인수·installed/live는 별도로 남는다.


### 2026-10-04 WP-7-E14 recovery and bounded passive diagnosis

Root recovered the single E14 attempt as INCOMPLETE: DB3 and runtime fixture passed, four initial browser checks passed, the 1800-second continuous stream stopped at `SOAK_NOT_LIVE` near fixture elapsed 213 seconds, and final unmount was NOT_RUN. All 752 source/2371 private pins matched. PostgreSQL exited normally with empty final catalog and owned groups/PIDs/sockets/container/temp absent. The final live identity read was NOT_RUN. E14 is consumed; no retry or result replacement. Root evidence: `wp7e14-root-review.json` (SHA256 `90449debd818cafdae87422eac59026e8b771a5621c32729ca04b3a326eea832`).

The failed row and preceding SSE metadata were not retained; connected backend observations and continuing commits do not establish why the UI lost LIVE. Root added only passive harness metadata diagnostics, reviewed their inverse against the E14 controller, and verified six pure diagnostic cases. Price values, raw response bodies, credentials, and arbitrary provider text are excluded from the new summaries. E15 will be a separately frozen one-shot with the unchanged continuous LIVE/progress, cadence, heap and final cleanup assertions. No production correction is inferred from missing diagnostics.

The existing worker's WP-7-FAULT-P read-only map was recovered after actual idle/pending0 with all 13 pins matching. It identifies still-unexecuted pre-ACK, whole-operation DB stall and notification/cache/restart fault cases; it is planning evidence, not independent or runtime acceptance. The read-only followup heartbeat was deleted.


### 2026-10-04 E15 serialization evidence and SNAPSHOT-1 correction

E15 reproduced E14: a concurrent app lease renewal caused PostgreSQL SQLSTATE40001 on the repeatable-read lease FOR SHARE snapshot, followed by SSE STOPPED/PRODUCER_UNAVAILABLE. The passive evidence showed fresh LIVE deltas immediately before the typed failure. Both attempts remain consumed and incomplete, with normal PostgreSQL cleanup.

Root reproduced this with an actual app-role transaction and observed lock contention before applying one narrow correction. The initial pre-publication lease SELECT may now explicitly roll back and rebuild the full actor/session/transaction context once for40001 only. SQL/binds/predicates and every downstream check remain byte-exact; no timer is renewed and commit ambiguity is not retried. GREEN passed five exact actual-role cases once each: renewal recovery, concurrent-release typed denial, original1s timeout/rollback, pre-ACK non-live rows and actor/session/rights/generation denials. Three final-source compile gates passed. Raw logs, pins, empty catalog and normal PG shutdown were independently reviewed.

E16 is a fresh one-shot full1800-second integrated soak with unchanged LIVE/progress/cadence/heap/final-unmount assertions. Broader fault matrix, independent acceptance, installed/live and overall KIS completion remain open. The read-only FAULT-DB-P worker was canceled after its bounded planning timebox and recovered at actual idle/pending0 without a finished design; no design acceptance is claimed.

### 2026-10-04 E16 30-minute local soak accepted; fault cases remain

Root recovered E16 after its single invocation exited 0 and verified all 754 source, 2,687 private and 3,297 execution pins, raw command hashes, DB3, the actual Rust fixture, all six browser checks and owned cleanup. The continuous thirty-instrument interval lasted 1,800,010 ms; its received-to-DOM p95 was 355 ms and maximum sample/progress gap 1,014 ms. Nineteen PostgreSQL serialization conflicts were observed and handled without terminating delivery.

Rolling publication maxima were 4 body starts/s, 4 acknowledged commits/s and 120 changed rows/s. The run recorded 6,220 commits, 185,086 changed rows and 925,534 coalesced replacements; one final body was dropped before commit with no uncertain commit. The conservative tracked Rust allocator peak was 5,940,576 bytes without baseline subtraction. Separate sampled process RSS was 33,873,920 to 46,034,944 bytes; it is not an 8 MiB RSS claim. Final unmount left no active leases, desired subscriptions or upstream socket. Final identity matched, the generated catalog was empty, PostgreSQL exited normally and recorded owned resources were absent.

This accepts the bounded local positive/soak/resource scenario. Fault-matrix and independent acceptance, installed/off and live/G1–G5 remain incomplete. After recovery, root opened a separate insertion-only test scope for real pool exhaustion/cancellation, advisory-lock deadline, known denial, actual COMMIT loss and pre-ACK wire rejection, plus a pure cache-namespace reset check. These new cases require compile and explicit frozen execution evidence before acceptance; no failure result is replaced or retried under an old runner.

### 2026-10-04 bounded database and pre-ACK fault cases accepted

New exact cases passed against the actual runtime repository adapter for exhausted-pool acquisition deadline, cancellation of a pending pool call, observed advisory-lock timeout/rollback, typed rights denial without poisoning the adapter, COMMIT dropped before forwarding, and committed-but-lost COMMIT response. Shared-clone terminal behavior and independently queried database outcomes were checked. A genuine pre-ACK market frame followed by a causal Ping/Pong barrier produced no cached quote or receipt ordinal; after the authentic ACK, the first admitted record published quote-version1/ordinal1. The pure cache-row namespace recreation check also passed.

The first seven-case invocation remains recorded as six passes and one failed newly authored expectation: a pre-ACK snapshot correctly had no cache row. Root corrected only that test to the established empty-cache/non-live delivery-identity contract, compiled and passed a fresh exact-one invocation. Both runs preserved pins and shut PostgreSQL down normally with empty catalogs and no owned processes/groups/sockets. Existing production code and assertions were preserved. Actual API notification loss/cache recreation/restart/slow-consumer and terminal scenarios, independent acceptance and installed/live remain open.

### 2026-10-04 E17 notification/cache faults and E20 logout lifecycle accepted

E17 passed five browser checks and the actual Rust fixture. Terminating the exact owned LISTEN backend preserved the existing SSE connection and repeated thirty-row progress through the one-second database fallback. Deleting one owned cache row caused the real producer to recreate its namespace; the browser observed reset/full snapshot, a new lower quote version with a greater authentic receipt ordinal, and continued progression. Final cleanup and pins were root-verified (`wp7e17-root-review.json`).

E18's authenticated logout failed with403. A focused expected-red test confirmed overlapping CSRF preflights could rotate the sole session CSRF value between token acquisition and mutation. The browser API and stream mutation clients now share an exclusive same-origin Web Lock over token acquisition and mutation, including their existing total deadlines. No server auth/CSRF bypass or retry was added. Focused20 tests, typecheck and the fresh browser build passed. E19 reached actual logout204 but exposed a pending publication's SessionInvalid abrupt-error path; its failed attempt and runtime cleanup failure remain recorded.

Root reproduced the SessionInvalid path before extending the existing known-denial classifier: only a new rights-validated, same-owner/slot, empty demand may select normal NoDemand cleanup. Failed observations, nonempty or mismatched demand and all genuine commit/deadline uncertainty remain failures; denied batches are never retried. Seven exact pure tests and three compile gates passed. E20 then passed all five browser checks and the actual fixture: native browser offline plus fresh in-process API server/state restart, reconnect with authentic thirty-row progress, normal logout204 with quote/SSE purge, zero leases/desired/upstream, and normal fenced retirement. This is not a deployed OS-process restart claim (`wp7e20-root-review.json`).

### 2026-10-04 E21 actual nginx slow-consumer isolation accepted

An extra authenticated TLS SSE reader used the existing browser lease through the owned nginx endpoint and paused reads. The unchanged production nginx five-second send timeout and API watchdog released only the extra LISTEN consumer. Root verified151 raw observations: the original PID persisted, the extra PID disappeared at7944ms, and no SQL mutation or extra lease was introduced. The interval includes TLS/kernel buffering and is not the exact start of a blocked-write timer. Draining222485 bytes reached normal EOF; the healthy browser retained its original stream and advanced all thirty instruments28 times while the peer was paused. Received-to-DOM p95 was275ms.

All five E21 checks and the Rust fixture passed. Root verified source762/private3188/execution3806 pins, raw hashes, initial/final cluster identity, empty final catalog, normal PostgreSQL exit0 and actual absent owned processes/groups/sockets/container/temp (`wp7e21-root-review.json`). The test changed only opt-in fixture/controller source. Remaining terminal-fault and cross-cutting regression coverage, independent acceptance and installed/off/live gates must still be reconciled; these results do not claim whole KIS completion.

### 2026-10-04 E22–E26 terminal transitions and cross-cutting regressions

E22 withdrew the fixture-owned immutable window, observed socket/price shutdown, then restored
that same window. A fresh epoch received all30 authentic synthetic ACKs over exactly two serial
WebSockets (peak1); all five browser checks and the Rust fixture passed, p95297ms. This is an
actual window-withdrawal/recovery test, not an overnight wall-clock transition.

E23 revoked exactly the synthetic entitlement. The authenticated old stream returned403,
prices disappeared, the daemon joined with typed `GrantUnavailable`, the socket closed normally,
and no quote version advanced. All five browser checks and the Rust fixture passed. Desired
metadata30 and the last `CONNECTED` record remained fenced after rights denial; this is not
evidence of a successful retirement write under revoked rights.

E24 rejected a malformed command argument before creating an attempt or starting PostgreSQL.
E25 is retained as three browser passes followed by `MembershipNotReady`/terminal fixture
failure; later cases were not run. Root traced the publication failure to a committed lease whose
identity had been superseded. The new branch handles only that known precommit denial, performs
one fresh rights-checked demand read, and chooses `NoDemand` or `FreshEpochRequired` only when
the denied identity is absent or replaced. Unchanged/invalid demand and read/commit uncertainty
remain errors. No denied batch retry, proof patch, SQL predicate change or deadline renewal was added.
Three new and seven existing exact pure tests and both final-source compiles passed.

E26 then passed all five browser checks and the Rust fixture. The exact generation1→2 change
closed the old full-board SSE and cleared all30 prices; the old authenticated lease returned404,
the retained old quote version did not advance, and final demand/desired/upstream counts reached0.
The daemon and socket shut down normally. This proves rejection and clean drain of the old
generation; it does not claim fresh generation2 display. E22/E23/E26 all had matching frozen pins,
empty final catalogs, normal PostgreSQL exit0 and actual absent owned process groups/sockets.

REGRESSION-1 passed five exact REST demand/reservation tests and five exact runner/EOD-startup
tests. A separate real owned-loopback transport test used an injected clock: the receipt before
session close was accepted, exact/after-close and next-KST-day receipts were rejected. It joined
its owned transport tasks and did not change the host clock or contact KIS.

CALENDAR-1 changed only the test endpoint factory in `intraday_quotes_support/mod.rs`. Explicit
C2 input must pass the existing binding/URL/live-identity guard before DDL; generated-role
connections retain that Unix endpoint. The historical QA URL constant and default branch, role
SQL, migration set, test bodies, validators and cleanup were preserved. Default and DB-feature
compile-only checks passed. CALENDAR1-E invoked its fresh pinned runner once and passed six
existing actual-role tests: trading/closed calendar reads, morning-calendar/evening-EOD reuse,
credentialed EOD calendar resolution, stale/future lineage rejection, read-only calendar
fingerprints, and current-identity/actor/generation isolation. Source765/private3579/execution4156
pins matched; every catalog was empty and PostgreSQL exited0 normally. The initial readiness
probe exited2 before the socket appeared; the next and final identity checks matched.

REGRESSION-2 passed four exact real-process tests with synthetic REST token issuers, covering
shared issuance/spacing and process death around issuance, token commit and read reservation.
It also passed19 V1/rest/off UI tests, seven Compose configuration tests, the intraday static
checker, and the fake-Docker refresh/rollback self-test. All770 input pins matched. These are
Node render/mock and disposable fake-installation tests, not actual REST/off browser or
installed-service results.

REGRESSION-3 passed one offline compile and thirteen exact filesystem/in-memory recovery
tests. Root checked all raw selection counts, exits,777 frozen inputs, and child cleanup;
the existing owned worker was idle with no pending permissions before acceptance. The
scheduled recovery heartbeat was deleted through the heartbeat API after result recovery.

EOD-RESTART-1 appended only feature-gated tests to `intraday_calendar_read_state.rs`; its
original46291 bytes remain exact. Both final-source compile-only configurations passed.
The separately frozen EOD-RESTART-1-E runner ran once and exited0. Its one selected parent
test started an actual research_writer child which reconciled an interrupted Raw manifest,
published the exact normalized source, synced its checkpoint and deliberately exited73
after commit. A distinct, reaped child replayed that source with exit0. Raw bytes, Raw and
normalized manifests, and full database row fingerprints stayed equal; the publication
remained4 batch rows,1 calendar history and1 projection. No provider request was possible
in either recovery child. All4190 execution pins matched; live identities matched and every
catalog was empty before normal PostgreSQL shutdown. This proves the bounded collector
child restart, not a PostgreSQL server crash, installed daemon or live EOD result.

The two evidence-bound diagrams were reviewed against current source and rendered locally
from the cached PlantUML image with networking disabled. The first deployment PNG hit the
4096px canvas limit; it is preserved privately. The final9548px image shows the full graph,
using `PLANTUML_LIMIT_SIZE=16384`; both owned render containers were removed.

### Current evidence and remaining gates — 2026-10-04

Private reviews below live under `/tmp/lagrange-kis-stream-completion-20261001-653ff5d84407`.
They index raw commands, hashes, exact selection counts and cleanup. Root recovery is distinct
from the independently assigned WP-7 acceptance. Both the original independent review and its
authorized supplement have now completed with SOURCE/LOCAL ACCEPT. Canceled mapping workers
and earlier failed runs remain incomplete and do not count as accepted reviews.

| Requirement | Recovered local evidence | Limit or remaining gate |
|---|---|---|
| Actual-role storage, API/RLS and migrations | `wp3c2-e6-root-review.json`, `wp4he-root-review.json`, `wp6de-root-review.json`; DB29, HTTP6, grant SQL27, migrations57 | Synthetic roles/data; current production migration/backup state uninspected |
| Full30 display and shared collection | E10/E16 real runtime→DB→authenticated API→nginx→Chromium | Synthetic upstream; actual broker30 ACK and normal Owner admission remain G5/WP-9 |
| Soak and resource ceilings | `wp7e16-root-review.json`; actual1800s, rolling≤4tx/120rows per second, conservative tracked global Rust allocator superset peak5940576B, p95355ms | Separate RSS trend; no installed/live latency or production memory proof |
| DB stalls, cancellation, ambiguous commit, pre-ACK | `wp7faultdb1e-root-review.json`, `wp7faultpreackfe-root-review.json` | Exact bounded fault cases; unknown commit is terminal and was not retried |
| Notification loss, cache recreation, API lifecycle | E17/E20 root reviews | API restart rebuilt owned in-process state; not an OS/service restart |
| Slow consumer | `wp7e21-root-review.json`; original healthy SSE progressed while slow connection was removed | Observed7944ms includes TLS/kernel buffering; not an exact server blocked-write timer measurement |
| Day/window boundary | `wp7e22-root-review.json`, `wp7regression1-root-review.json` | Real window withdrawal/restoration plus injected-clock KST transport boundary; no overnight full-stack run |
| Rights and generation supersession | `wp7e23-root-review.json`, `wp7e26-root-review.json` | Revoked-rights metadata remains fenced; generation2 fresh display not part of E26 |
| Calendar/EOD and app-role regressions | `wp7calendar1e-root-review.json`, `wp7regression1-root-review.json`, `wp7regression3-root-review.json`, `wp7eodrestart1e-root-review.json` | Actual writer commit followed by distinct collector child replay of one synthetic source; no PostgreSQL server crash, installed daemon or live EOD proof |
| REST sharing, V1/rest/off, env/image/daemon guards | `wp7regression2-root-review.json` | Synthetic issuers, Node UI, fake Docker; installed off rollback is separate |
| Initializer and grant helper | `wp6c-uid-green-root-review.json`, `wp6d-cleanup-probe-root-review.json`, `wp6de-root-review.json` | Injected CLI UID/mount and direct SQL are not final packaged-image or installed-helper acceptance |
| Independent source/local acceptance | `wp7-independent-review-report.md` and `wp7-independent-review-supplement-report.md`; SOURCE/LOCAL ACCEPT, no High/Medium findings, all 804 current source inputs matched at review | Source/local only; original evidence-count/hash and locator errata are retained in the supplement and root recovery; subsequent Low correction is separately verified |
| Exact candidate installation and live acceptance | `INSTALLED_OFF`, `LIVE_ACCEPTED`, `FINAL_ACCEPTED` not reached | Clean reviewed commit, authorized integration/build/install, current host preflight, G1–G5 and Owner session remain |

The same independently assigned reviewer completed both reviews after the owner explicitly
authorized the original private payload and the 24-file supplement. Actual idle, no active
turn and zero pending permissions were verified before recovery; the recovery heartbeat was
deleted. No alternate agent route or model-mode bypass was used. Historical launch rejections
and both reports are preserved.

The first Low finding was corrected by describing the allocator measurement as a tracked
global Rust allocator superset and separating local operational evidence from installed proof.
The supplement's Low finding was corrected by using the existing canonical, non-nil lease UUID
schema for the DELETE path parameter. The JSON artifact was regenerated; TypeScript generation
produced unchanged bytes. The OpenAPI check linted 91 operations and type-checked generated
types. Focused in-memory checks accepted canonical UUIDs, rejected uppercase/unhyphenated/nil/
malformed/padded values, and proved that no other generated API contract changed. This bounded
schema correction is recorded in `wp7-l2-root-review.json`; runtime validation was unchanged.

The next required gate is WP-8: a clean exact reviewed commit, authorized integration and
provider-free installation scope, with fresh host/resource/backup checks. WP-9 still needs
the separately satisfied G1–G5. Do not repeat already accepted scenarios merely to obtain a
new report. No production, provider, protected credential or commit/push operation is opened
by this source/local acceptance update. Installed/live and overall KIS acceptance remain open.
