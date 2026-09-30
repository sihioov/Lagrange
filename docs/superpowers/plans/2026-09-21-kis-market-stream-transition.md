Execution skill: $paseo-delegate (required)
Native subagents: prohibited for worker packages

# KIS 시장 시세 구독 전환 — 계획안

작성일: 2026-09-21 KST. 상태: **WS-1 SOURCE/offline 계약 채택; WS-2 로컬 구현 진행**.
최초 요청은 `$paseo-delegate-plan 계획작성 진행해`였으며 계획 작성 후 별도로 실행을
요청했다. 실행 요청은 아래 의존성과 권한 gates를 유지한다. 기존 REST 복구 계획의 성과를
보존하면서 장중 시세 수신 방식과 화면 전달 방식을 전환하기 위한 실행 계약이다.

2026-09-21 수정: 사용자 전달 의견의 공용 수집기·메모리 최신값·내부 공유·후보군과
실시간 구독 집합 분리를 반영했다. 주문 실행, 신규 전략 엔진, 전 종목 수집으로 범위를
확장하지 않는다. 기존 7개 패키지에 계약과 검증을 보완했다. 후속 실행 요청에 따라
공식 문서·소스 계약 조사부터 진행하며 새 API의 정확한 범위와 운영 gates는 별도로 확인한다.

## Goal and boundaries

### 목표와 완료 기준

- 기존 V1 고정 30종목을 정상 Owner API로 자동 등록한다. 파일에 있는 정확한 목록을
  검증하고 1종목이 READY가 된 뒤 나머지 29종목을 순차 처리한다. 사용자가 30번
  추가하지 않는다. 이미 등록된 종목은 재등록하지 않으며 재실행은 멱등이어야 한다.
- Owner가 시세판을 열면 등록·승인된 30종목의 시장 시세를 서버가 구독한다.
  대시보드와 상세 화면, 여러 탭의 같은 종목은 상류 구독 하나를 공유한다.
- KIS 시장 시세 WebSocket → 서버 검증/최신값 저장 → 인증된 SSE → 실제 브라우저
  화면으로 전달한다. 정상 구독 중 주기적 REST 현재가 요청 및 브라우저 5초 quote
  polling은 0이다. SSE heartbeat나 내부 상태 확인은 KIS 시세 요청이 아니다.
- 초기 REST snapshot 또는 장애 시 REST fallback은 기본적으로 끈다. 필요성이
  확인되면 목적·횟수·동시성·예산을 별도로 제시한다. fallback을 몰래 추가하지 않는다.
- 30종목 모두의 구독 승인, 대표 종목의 실제 후속 시세 수신/캐시/SSE/DOM 일치,
  끊김·재연결·권한 철회·종목 변경·화면 종료 동작을 검증한다. 거래가 없는 종목에
  새 체결을 만들어내거나 전 종목에 체결이 반드시 발생한다고 가정하지 않는다.
- 정상 부하에서 서버가 유효 시세를 받은 시각부터 화면 반영까지 p95 ≤ 2초를
  제품 목표로 둔다. 이는 KIS가 전송하기까지의 지연 보장이나 모든 체결 저장 요구가
  아니다. 실제로 측정하고, 실험실·운영 결과를 나눠 보고한다.
- 기존 EOD 가격/기업행동/달력 수집, 권한별 이력, 일봉·지표는 보존한다. 장중 tick을
  일봉 확정값으로 사용하지 않는다. 동일한 유효 날짜의 달력 증거로 EOD 게시까지
  성공해야 전체 복구를 완료로 판정한다.
- 구현·테스트·운영 검증은 코디네이터가 책임진다. 사용자는 완료된 결과를 최종 확인한다.
  사용자만 가능한 로그인/인증이 실제 장애물일 때만 필요한 한 단계의 도움을 요청하며,
  사용자의 수동 테스트를 자동 검증의 대체물로 삼지 않는다.

### 대상과 확인된 기준선

- 작업 경로: `/data/worktrees/3puw275b/hot-chipmunk`.
- 계획 작성 시 HEAD: `7757245553ab8cbed5d6f706e46a87849e0cedc6`, 작업 트리 깨끗함.
- 마지막 운영 확인 기록: 설치된 후보는 `75d28ef10f86509aee86945af8985f109d328a07`.
  서비스 9개는 이전 `74b39fa`에서 동작했고 research/Owner V2 reader 2개는 의도적으로
  중지돼 있었다. 이것은 9월 19일 확인 기록이며, 오늘의 건강 상태로 간주하지 않는다.
- `7757245`의 달력 진단 보강은 독립 리뷰와 집중 테스트 12개를 통과했으나 이미지에
  포함되거나 운영 설치되지 않았다.
- 기존 인수인계: `/tmp/lagrange-kis-live-20260918/root-calendar-diagnostics-integration-handoff.md`.
  기존 작업자 원장: `/tmp/lagrange-kis-live-20260918/owned-workers.json`.
  모두 종료했고 감시는 중지됐다. 새 실행은 새 작업 원장으로 관리하며 옛 heartbeat를
  되살리거나 옛 작업자의 권한을 그대로 상속하지 않는다.

### 재사용과 변경 범위

| 영역 | 처리 | 제한/근거 |
|---|---|---|
| V2 등록, admission, READY, generation | 유지 및 기존 도구 재사용 | 정상 API, 실제 Owner, pilot-first; DB 강제 READY 금지 |
| Owner 인증, 소유권, RLS | 유지 | 지속 연결에서도 재검증/철회 필요 |
| 감사 가능한 entitlement amendment | 결과와 이력 보존 | WS 시장 시세 이용 범위까지 덮는지는 별도 확인; 재승인 위조 금지 |
| 달력 취득/재사용, EOD 권한별 복구 | 유지 | Sep19 미확정 claim은 그대로 보존; WS 전환으로 우회하지 않음 |
| REST token/rate limiter | EOD용 유지 | WS 승인키와 동일 수명·예산으로 취급하지 않음 |
| 시세 DB 캐시, quote version | 개념 재사용, 저장 계약 수정 | 현재 publish가 REST reservation/receipt에 결합돼 있음 |
| 현재가 타입/API/UI | 부분 재사용, 새 stream 계약으로 버전 구분 | REST 전용 base_price/거래정지/시간 필드를 WS에서 지어내지 않음 |
| demand/lease | 구독 집합 관리로 수정 | 현행 최대 5종목/20 consumer 구조를 30종목 board 수요로 재설계 |
| 장중 REST 반복 producer | WS 모드에서 사용 중지 | REST EOD/reference 요청은 별도 정상 유지 |
| 기존 `kis-client/src/websocket.rs` | 시세 수신에 직접 사용 금지 | 계좌 미체결·체결·잔고 재조회가 필수인 별도 상태기계 |
| 이미지 빌드/설치 guard, backup/restore | 유지 | immutable commit/env/manifest/image 일치, 자원 제한 유지 |
| 장중 QA | 재사용 가능한 부분 유지, 수신 증명 변경 | 합성 REST 영수증으로 WS 성공을 증명하지 않음 |

기존 등록/권한/EOD/화면 기반 전체를 버리지 않는다. 다만 **입력 transport만 바꾸면
끝난다는 이전 설명은 범위가 작았다**. 영수증·시간·수요 한도·저장·API 상태·브라우저
전달까지 수정하는 계획이다.

### 공용 수집과 종목 규모에 대한 설계 보완

사용자 전달 의견의 핵심인 **한 번 수신한 시세를 여러 소비자가 공유**하는 구조를
적용한다. KIS 공식 예제에서도 시장 체결가 `H0STCNT0`의 구독/해제를 확인했다
([공식 시장 WS 예제](https://github.com/koreainvestment/open-trading-api/blob/main/examples_user/domestic_stock/domestic_stock_functions_ws.py),
확인일 2026-09-21). 정확한 운영 한도와 인증 계약은 여전히 WS-1의 확인 대상이다.

아래는 구현 전 목표 구조이며 현재 운영 구조를 나타내지 않는다.

```text
승인된 기존 EOD/일봉 데이터 → 후보 목록 (현재 고정 30종목)
                                  ↓ 허용 목록·READY·활성 demand 대조
KIS 시장 WS ← 공용 구독 조정기 (중복 제거·구독 한도·등록/해제 속도 관리)
     ↓
시장 frame 검증 → 수집기 메모리 최신값 + 크기가 제한된 내부 채널
     ↓ 최신값 병합·게시 시 권한/세대/fence 재검증
기존 PostgreSQL 최신값 캐시 + commit 후 변경 알림
     ↓
인증된 API/SSE → 시세판·상세 화면·여러 탭
```

- **수신과 계산 분리:** 외부 연결은 공용 수집 계층만 소유한다. 같은 종목·채널을
  보는 소비자가 늘어도 추가 broker 구독/REST 조회를 만들지 않는다. 새로운 종목이나
  별도 호가 채널을 요구하면 구독 자원을 추가로 사용하므로 무제한 공유라고 표현하지 않는다.
  공유 범위는 같은 허용된 credential/시장/채널/권리 경계 안이며 다른 Owner로 재배포하지 않는다.
- **메모리와 DB의 역할:** 수집기 안에서는 메모리 최신값과 bounded 채널로 처리한다.
  이미 worker/API가 별도 프로세스인 프로젝트이므로 메모리만으로 API까지 공유할 수는 없다.
  프로세스 간 전달에는 기존 PG 캐시/알림을 사용하고 Redis/Kafka는 우선 추가하지 않는다.
  메모리는 수신 작업 공간이며 권한 검증을 우회하는 공개 source가 아니다. 공개 화면에는
  게시 검증과 commit을 통과한 값만 전달한다. 재시작 후 메모리나 DB의 이전 값을 새 live
  수신으로 간주하지 않는다. DB 지연/실패 시에도 이 경계를 우회하지 않는다.
- **내부 소비자 계약:** snapshot/status/버전 있는 latest-value 알림 인터페이스를 정의하고
  느린 소비자가 수집기나 다른 소비자를 막지 않게 한다. 이후 승인된 읽기 전용 분석이
  붙어도 broker client를 각각 만들 필요 없도록 하되, 이번에는 실제 전략 엔진을 연결하지
  않는다. 모든 tick이 필요한 전략에는 최신값 병합 채널을 그대로 쓰면 안 되며 별도 계약이
  필요하다. 이번 작업에 주문 경로, 계좌 보유종목 조회, 매매 신호 실행을 포함하지 않는다.
- **종목 등록과 구독 분리:** 등록된 후보 목록, READY 집합, 현재 구독 중인 집합을 구분한다.
  이번 제품은 시세판을 열면 고정30 모두가 수요이므로 기존30 인수 기준을 유지한다.
  단순히 의견을 적용한다는 이유로 화면 일부 종목만 실시간으로 바꾸지 않는다. 향후 별도
  승인된 분석 소비자가 생기면 화면 demand와 독립된 lease를 가져야 하며, 마지막 브라우저가
  닫혔다는 이유만으로 그 소비자의 구독을 끊지 않는다. 현재 실행에는 그런 소비자가 없다.
- **큰 후보군은 별도 확장:** 수백~수천 종목의 선별은 권리가 확인된 일봉/EOD 데이터로
  낮은 빈도에 수행하는 방향을 둔다. 이번에는 전 종목 데이터 수집이나 선별 엔진을 구현하지
  않는다. 묶음 조회·분봉 API가 현재 허용됐다고 가정하지 않으며, 신규 source/endpoint는
  별도 조사와 승인 대상이다. 전 종목 모든 체결/호가가 필요하면 전용 데이터 서비스의
  범위·권리·비용부터 검토한다. 구독을 빠르게 순환해 한도를 우회하지 않는다.
- **신선도와 사용 가능성:** 마지막 가격, 연결 상태, 수신 시각, 거래 시각, gap을 함께 전달한다.
  연결이 끊기거나 데이터가 불확실하면 마지막 가격은 참고값으로만 표시한다. heartbeat가
  가격을 새롭게 만들지 않는다. 향후 분석 소비자도 이 상태를 검사하게 하며, 오래된 가격으로
  자동 매매할 수 있는지를 이번 시세판 인수 결과로 보증하지 않는다.
- **총 호출량과 구독 변경량:** EOD·등록 작업·reference 조회 등 같은 App Key를 쓰는 모든
  REST 작업의 합산 예산을 확인한다. 기존 endpoint/TR별 제한도 함께 유지한다. WS 승인키
  발급, 연결 시도, 구독/해제 명령은 별도 계수·제한하며 재연결/탭 전환 폭주를 막는다.
  내부 소비자 수 증가가 기존 동일 종목의 상류 요청량 증가로 이어지지 않는지 검증한다.

### 지침 및 권한 경계

읽은 지침: `/home/l1nnx/.codex/AGENTS.md`, 저장소 `AGENTS.md`,
`apps/web/AGENTS.md`, `apps/web/CLAUDE.md`,
`/home/l1nnx/.agents/skills/paseo-delegate-plan/SKILL.md`.
Web 구현자는 설치된 Next 버전의 `node_modules/next/dist/docs/`를 먼저 읽는다.
해당 문서가 없으면 실제 의존성 위치를 확인하고, 기억에 의존해 새 API를 쓰지 않는다.

실행은 `$paseo-delegate`로만 한다. 현재 스킬 경로
`/home/l1nnx/.agents/skills/paseo-delegate/SKILL.md`가 존재하지만 계획 작성 중에는
실행하지 않았다. 실행자는 최신 스킬 전체와 Paseo profiles/notes를 읽고 모델·effort를
확인한다. 프로필 부재/불일치를 조용히 대체하지 않는다.

저장소의 deny-by-default 규칙은 현재 명시된 REST 메서드/호스트/경로/TR만 허용하며
새 response contract에도 명시적 승인을 요구한다. 이번 **계획 요청은 WS 활성화 승인으로
해석하지 않는다**. 후속 실행 요청도 문서에 없는 네트워크 범위를 임의로 넓히지 않는다.
WS-1이 정확한 시장 시세/승인키 네트워크 범위를 문서화한 다음,
코디네이터는 기존 승인과 대조해 부족한 권한만 구체적으로 요청한다. 계좌/주문/체결통보,
`CANO`/`ACNT_PRDT_CD`/HTS 개인 체결 채널, live-order profile은 계속 제외한다.

보존할 운영 사실:

- Sep19 calendar claim은 존재하지만 확정 Raw source가 없다. token 발급 성공 및
  한 번의 로컬 완료된 calendar dispatch만 확인됐고 정확한 오류는 소실됐다.
  해당 날짜의 claim/UUID/token/state/debt/generation을 삭제·수정·재시도하지 않는다.
- 날짜가 바뀌었다고 새 취득이 자동 승인되는 것은 아니다. 취득 시점의 날짜에 유효한
  공식 증거·기존 claim/source·허용 범위를 다시 확인한다. Sep19 CLOSED 자료 재사용 금지.
- 사전 승인된 하루 EOD 수집과 달력 증거 재사용을 WS 재연결 이유로 다시 호출하지 않는다.
- 기존 단일 빌드 승인과 drained-reader attestations는 이전 커밋/기간에 묶여 있다.
  새 커밋으로 복사·기간 연장하지 않는다. root wrapper의 기존 만료는
  `2026-09-22 08:54:48 KST`; 만료 전후를 실행 때 확인하고 임의 연장하지 않는다.
- 실제 Owner 인증 세션 확보 여부는 미확인이다. 쿠키 위조, DB 세션 삽입, auth bypass로
  해결하지 않는다. 기존 승인된 인증 경로를 먼저 점검한다.

### WS-1에서 확정해야 하는 요구사항

계획 기본값은 **고정 30종목 Owner 전용 시세판, 한 서버 연결 계층, 브라우저 SSE**이다.
KIS가 제공하는 정확한 동시 구독 수는 아직 확인하지 않았으므로 30종목 수용을 사실로
기재하지 않는다. 부족하면 cap을 우회하거나 여러 키/연결로 분산하지 말고 영향과 선택지를
보고한다. 이때 5종목만 보여주고 30종목 목표를 완료로 바꾸지 않는다.

WS-1은 현재 공식 KIS portal/공식 저장소를 확인해 다음을 고정한다.

1. 승인키 발급의 정확한 method/host/path, 인증정보, 유효기간·재발급·저장 정책.
   REST token 발급 규칙을 승인키에 추정 적용하지 않는다.
2. 정확한 WebSocket scheme/host/port/path와 시장 체결가 `H0STCNT0` 후보의 요청/응답,
   구독/해제 명령, ACK, heartbeat, frame packing, 오류, 시장 범위. 암호화 여부와
   네트워크 전제도 기록한다. 공식 예제의 계좌 통보 기능을 통째로 가져오지 않는다.
3. 키/연결/종목/채널별 한도, 구독 등록 속도, 동시 연결 및 재연결 규칙. 문서에 없는
   한도는 미확인으로 남긴다. 프로젝트 한도와 broker 한도를 구분한다.
4. 권리 문서가 시장 WS 시세의 개인 Owner 화면 전달/최신값 캐시에 적용되는지.
5. 필드 매핑: 가격·등락·수량·거래정지·거래일/시각과 REST `base_price`의 비동등성.
   없는 필드는 nullable/별도 타입으로 표현하고 기존 필수 필드에 0이나 추정값을 넣지 않는다.
6. 초기 미체결 종목, 중복/역순/한 frame 다수 레코드, sequence 부재, 재연결 gap,
   오래된 세션 frame 및 날짜 변경의 의미와 실제 가능한 검증.
7. 매일 유효한 달력/운영시간 증거의 공급 책임과 절차. 현행 자동갱신이 없다는 점을
   숨기지 않는다. 새로운 KRX 수집 자동화는 본 계획에 포함하지 않는다. 계속 운영하려면
   기존 허용 절차로 공급 가능한지 검증하고, 불가능하면 장기 운영의 미해결 의존성으로 보고한다.

공식 탐색 출발점(구현 계약 자체가 아님):
`https://github.com/koreainvestment/open-trading-api/blob/main/examples_user/domestic_stock/domestic_stock_functions_ws.py`.
WS-1은 출처 revision/확인일/구체적 코드·문서 위치를 남긴다. 공식 문서 열람과
credentialed API 호출은 구분하며, 조사 중 실제 승인키/구독을 발급하지 않는다.

## Initial classification

| Package | Complexity | Basis | Confidence | Reclassification or escalation signals |
|---|---|---|---|---|
| WS-1 | hard | 공식 wire/인증/권리/한도와 REST 결합 계약을 동시에 결정하는 열린 설계 | medium | 공식 자료 상충, 30구독 수용 불가, 필수 시간·가격 필드 근거 부재면 해당 구현 분기 보류 |
| WS-2 | intermediate | WS-1 고정 계약에 따른 시장 전용 transport/parser 구현 | medium | 계약 누락은 먼저 WS-1로 반환; 재연결/레드랙션 검증 두 번 실패 시 모델 한 단계 상향 |
| WS-3 | hard | 다중 프로세스 구독 소유권, 세대·세션·DB fence, REST 증명 분리 | medium | 실제 DB race 재현 실패, REST 경로 오염, 인터페이스 미정이면 구현 전 계약 보완 |
| WS-4 | intermediate | 고정 stream DTO 기반 Owner API/SSE/화면 30행 구현과 인증 검증 | high | revocation/재접속 누락, fake DOM만으로 검증, 두 번 반복 실패면 상향 |
| WS-5 | intermediate | 기존 immutable 운영 경로에 명시적 WS 설정·차단·rollback 적용 | medium | 설치 파일 수동 패치 필요, 기존 proof/token 초기화 요구, 계좌 채널 혼입 시 중단·재분류 |
| WS-6 | hard | 독립 교차 검증, 완전한 DB 역할/브라우저/부하/복구 증거로 후보 채택 판단 | high | 문서상 PASS와 실제 wire/DB/DOM 불일치면 REJECT, 해당 소유 패키지에 반환 |
| WS-7 | hard | 실제 배포·구독·Owner 화면·EOD의 운영 조건과 비가역 경계 | medium | 날짜/권한/시장/인증 입력 부재, 자원·서비스·broker 한도 실패면 영향 분기 중단 |

## Execution graph

복잡도는 모델 등급을 자동 결정하지 않는다. 명세가 고정된 구현은 우선 Luna max,
열린 아키텍처 판단은 Astra xhigh, 독립 결론·운영 분석은 Sol high로 배정한다.
Luna 산출물은 실제 테스트와 코디네이터 파일 확인을 거쳐 채택한다. 같은 수정을 두 번
실패하거나 루프/맥락 이탈/반복 검증 실패가 관찰될 때만 Luna → Terra → Sol로 한 단계씩
올리고 effort는 동시에 바꾸지 않는다. WS-3의 설계 불확실성은 WS-1에서 먼저 해소한다.

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
|---|---:|---|---|---|---|---|---|---|
| WS-1 | 1 | hard | 공식 계약·기존 결합·30종목/세션 모델 고정 | 새 WS spec, 프로토콜/권한 matrix, 실행 계획의 계약 보완만 | 기준선/기존 보고서 | codex/gpt-6-astra, xhigh | 검증 가능한 wire/type/state 계약과 네트워크 승인안 | 공식 출처 + 현 코드 호출·DB 제약 대조; provider 호출 0 |
| WS-2 | 2 | intermediate | 계좌 기능 없는 KIS WS 수신기 | kis-client 새 market stream/approval/parser 모듈, exports, 관련 Cargo/lock, 집중 tests | WS-1 채택; SOURCE/offline은 synthetic/loopback만, credentialed 분기는 정확한 API 경계 승인 필요 | codex/gpt-5.6-luna, max | fixture/local socket 검증된 transport | 실제 local fake WS server, malformed/reconnect/redaction 테스트 |
| WS-3 | 3 | hard | stream evidence·구독·캐시 게시 통합 | market-data stream 타입, job-queue intraday/runner, 필요한 신규 migration와 DB tests | WS-2 + 고정 인터페이스 | codex/gpt-5.6-luna, max, WS-1 미정이면 실행 금지 | 30종목 중복 제거/단일 소유·안전한 저장 | 전체 migration 실제 역할 DB + 생산 경로/local WS 연계 |
| WS-4 | 4 | intermediate | 인증 SSE와 30행 화면·자동 등록 QA | api-server, Web quote/board, nginx SSE route, Owner QA script/tests | WS-3 | codex/gpt-5.6-luna, max | snapshot+delta 화면 및 세션 종료 증거 | 실제 HTTP/SSE + React/Chromium, 30종목 lifecycle |
| WS-5 | 4 | intermediate | WS 운영 설정/활성화/복구 계약 | ops validators/wrappers/self-tests, compose/env/Docker runtime 설정, 운영 runbook | WS-3 | codex/gpt-5.6-luna, max | immutable mode 전환과 fail-closed preflight | 공식 helper 추출/실행 disposable tests, 이미지 provenance |
| WS-6 | 5 | hard | 통합 후보 독립 리뷰와 실제 로컬 QA | 읽기 전용 source, disposable QA 환경, private report | WS-4/5 및 코디네이터 diagram 통합 | codex/gpt-5.6-sol, high | ACCEPT/REJECT와 증거 matrix | 실제 DB 역할·local WS·nginx/SSE·Chromium·자원·EOD 회귀 |
| WS-7 | 6 | hard | 검증된 후보 운영 전환 및 실사용 인수 | 승인된 pinned 운영 명령·Owner QA, private reports만 | WS-6 ACCEPT + root 운영 gates | codex/gpt-5.6-sol, high | 30종목 구독·시세 수신/DOM·EOD 증명 | 실제 이미지/수신/SSE/DOM/정지/동일 달력 EOD |

Wave 4만 병렬 가능: WS-4는 API/Web/nginx, WS-5는 ops/compose를 수정한다.
공유 Cargo/lock 변경은 WS-4 단독 소유이며 WS-5는 변경하지 않는다. 신규 아키텍처
diagram의 `.puml`/PNG와 evidence line 통합은 코디네이터가 두 작업 종료 후 수행한다.
다른 패키지는 앞선 결과 채택 전 시작하지 않는다. 같은 호스트의 Cargo/DB/browser
무거운 검증은 공통 lock과 자원 확인으로 직렬화한다.

## Worker briefs

### 모든 패키지에 함께 전달할 필수 문맥

각 brief를 실행할 때 이 절, 위 기준선/보존 상태/권한 경계, 해당 WS-1 계약과 선행
보고서를 빠짐없이 함께 제공한다. 작업자는 이전 대화를 본다고 가정하지 않는다.
작업 경로는 항상 `/data/worktrees/3puw275b/hot-chipmunk`이다. 실행 중 HEAD가 바뀌면
기준선과 의존성을 다시 확인하고 보고한다. 완료된 일만 통합하며 worker는 stage/commit,
main merge를 하지 않는다. WS-1~6은 production apply를 하지 않는다. WS-7의 운영 명령은
코디네이터가 기존 권한과 gates를 확인해 명시적으로 넘긴 한정된 단계만 실행한다.

각 프롬프트에 다음 문장을 넣는다:

> Do not use native subagent, Task, Agent, team, or delegation features. Complete this assignment directly and report if it needs further decomposition.

필수 보고 형식: 변경 파일과 라인 범위; 명세와 다른 처리/이유; 실행한 명령과 결과;
미해결/후속 작업; 찾지 못했거나 검증하지 못한 것(없으면 `none`); 추가 분해 필요 여부.
성공한 synthetic 검사와 실제 provider/운영 검사를 구분한다. 미해결이 명시되지 않은
보고서는 완료로 채택하지 않는다. 보고서 위치는 실행 시 생성하는 private 작업 디렉터리의
`ws-N-report.md`이며 mode 0600, 비밀/원문 broker payload/개인 식별자는 넣지 않는다.

### WS-1 — 계약과 이전 구현의 재사용 경계 확정

- **분류:** hard/medium. 열린 시장 프로토콜과 기존 DB/권한/신선도 계약 결합이 이유다.
  한도·권리·필드·프로토콜 근거가 충돌하면 상상해서 채우지 말고 해당 분기를 보류한다.
- **범위:** 새 `docs/superpowers/specs/2026-09-21-kis-market-stream-contract.md`, 필요시
  본 계획의 구체적 계약 보완. 기존 source/AGENTS/운영 파일은 수정하지 않는다.
- **입력:** 위 확인 항목, 기존 intraday spec/runbook, 기존 acceptance script,
  `kis-client/src/websocket.rs`, `job-queue/.../intraday.rs`, 최신 로컬/운영 인수인계.
- **산출:** 정확한 네트워크 allowlist 제안, 승인키와 REST token 분리, wire fixture 계약,
  각 계층 Rust/HTTP 타입과 버전, DB migration 필요성, bounded 자원·구독·SSE 설계.
- **고정할 제품 계약:** board 하나가 30개 READY identity를 요청한다. identity별 내부
  generation 검증을 유지하며 one board=30 browser connections 모델은 금지한다.
  detail/tab/board는 집합 합집합으로 상류 구독을 합친다. 마지막 demand 만료/해제 시
  해제하며, 관찰자가 없을 때 상시 30종목을 받는 것은 기본값이 아니다. consumer ID와
  lease 소유자를 구분하고 종목·채널 단위로 참조를 합친다. future 분석 consumer 계약은
  정의만 하며 실제 전략을 시작하거나 계좌 보유종목을 조회하지 않는다.
- **고정할 저장 계약:** REST attempt reservation을 위조해 WS frame을 통과시키지 않는다.
  stream session epoch, subscription identity, producer fence, 실제 수신 시각을 증명하는
  별도 타입/제약을 설계한다. 기존 REST 값이 WS 연결 성공으로 보이지 않게 source를 구분한다.
- **고정할 시간 계약:** 거래소 event time/거래일의 문서상 의미를 검증하고 `received_at`과
  구분한다. heartbeat/구독 ACK만으로 quote version·last price time을 새로 만들지 않는다.
  연결 정상/마지막 체결 시각/장 상태를 구분해서 거래 없는 종목을 통신 장애로 단정하지 않는다.
  sequence/replay가 없다면 exactly-once tick 또는 완전한 체결 이력을 약속하지 않는다.
- **고정할 전달 계약:** SSE 초기 snapshot + 버전 있는 최신값 delta, connection status,
  bounded queues/coalescing, 재접속 시 최신 snapshot 재동기화. 수집기 내 메모리 최신값과
  내부 채널은 private 작업 공간이며 게시 권한/세대/fence 검증을 통과하지 않은 값을 외부
  소비자에게 배포하지 않는다. 기본안은 PG 최신값 캐시와
  commit 후 알림(알림에 민감 quote payload 없음)을 사용한다. 새 Redis/Kafka는 요구하지
  않는다. 알림 유실은 캐시 재조회로 복구한다. 프로세스 간 invalidation을 명시한다.
- **고정할 부하 예산:** 브라우저당 SSE 하나로 묶고 초당 프레임/최대 크기/queue 상한,
  hot symbol의 bounded coalescing, 종목별 저장 빈도 상한, disconnect timeout을 수치화한다.
  화면은 최신값 시세판이므로 모든 tick의 DB INSERT/DOM render를 요구하지 않는다.
- **고정할 예산 계약:** 같은 App Key의 실제 REST caller 목록과 공통 예산 경로를 대조한다.
  기존 per-channel 제한만으로 key 전체 제한을 충족한다고 추정하지 않는다. WS 승인키/
  재연결/구독 변경 예산과 desired→pending→ACKed 상태를 정하고, 탭 전환 시 해제 유예와
  변경 병합의 유한한 시간을 명시한다. 권한 철회 시 전달 차단은 그 유예를 기다리지 않는다.
  기존 limiter에 합산 보장 누락이 있으면 변경 파일·검증을 먼저 계약에 지정하고 WS-3에
  명시적으로 넘긴다. 현재 안전 제한이나 token 정책을 완화하는 권한은 없다.
- **검증:** 공식 revision을 고정한 source matrix와 실제 코드 제약 대조표. 필요한 새 권한,
  충돌하는 기존 계약, 30종목 수용 근거가 갖춰져야 구현 분기를 연다. 추가 조사 때문에
  provider 요청을 실행하거나 새 승인키를 발급하지 않는다.

#### WS-1 조사에 따른 계약/의존성 보완 (SOURCE/offline 채택, live gates 미해결)

제출 계약은 [2026-09-21-kis-market-stream-contract.md](../specs/2026-09-21-kis-market-stream-contract.md)이다.
이 보완은 live 허용이나 운영 확인이 아니다. 계약의 E(근거)/D(프로젝트 설계)/G(live gate)를
구분해 후속 worker에게 전달한다.

- 9월 9일 공식 공지/9월 11일 portal 필드 문서는 9월 14일부터 `H0STCNT0`에
  `MARKET_CLS_CODE`를 추가한다. 현행은 **47필드**이며 8월 GitHub 예제의 46필드를
  그대로 쓰지 않는다. 해제는 portal/helper의 `tr_type="2"`; 함수 설명의 `"0"`은 상충한다.
- `BSOP_DATE`, `STCK_CNTG_HOUR`, `TRHT_YN`이 제공된다. WS 전용 날짜/관측 거래정지
  타입으로 검증하고 `base_price`는 제공되지 않으므로 null로 둔다. 정규장 계약만 사용한다.
- 전체를 회수한 2026-04-20 유량 공지는 1 App Key당 1세션·합산41구독이지만,
  portal에는 9월 16일 새 유량 공지도 있다. 후자는 상세 HTTP400으로 미확인이다.
  **현행 live 한도 확정은 잔여 gate**이며, 30미만이면 제품 결정을 되묻는다.
- 공유 REST coordinator에는 이미 GET 합산1초/채널1초 제한이 있다. 현 source의
  production constructor 4곳/credentialed service 5개가 shared 모드를 지원한다.
  WS-3은 old ledger/토큰/부채를 교체하지 않고 실제 다중 caller 합산 검증을 수행한다.
- 신규 migration은 현재 next-free **`0055_owner_market_stream.{up,down}.sql`**을
  WS-3에 예약한다. 적용된0054/철회한 audit0055와 별개이며, 시작 시 번호 충돌을 재확인한다.
  새 board lease/item·grant·producer·subscription·cache 테이블로 REST 증명/DTO를 보존한다.
- 후속 인터페이스는 계약의 schema2 `stream-leases`/`market-stream` SSE와 별도
  `StreamQuote`/receipt/epoch/fence로 고정한다. 기존 schema1 GET에 WS 값을 넣지 않는다.
- 소유 범위 보완: WS-2는 신규 WS 보호상태/전용 lifetime lock/예산까지 소유한다.
  WS-3에는 `crates/kis-client/src/read_coordination_config.rs`의 transport 설정만 추가한다.
  WS-5에는 `scripts/ops/provision-linux.sh`의 새 WS leaf 및 별도 commit-pinned
  `scripts/ops/install-owner-market-stream-grant.sh`만 추가한다. 기존 immutable installer의
  DB/provider-free 성질을 보존하며, 새 grant 설치도 별도 명시적 승인 입력/실행 gate를 요구한다.
- WS-1 채택 후 SOURCE/offline 구현·실제 loopback/격리 DB/브라우저 검증은 진행 가능하다.
  `POST https://openapi.koreainvestment.com:9443/oauth2/Approval` 및
  `ws://ops.koreainvestment.com:21000/tryitout`/`H0STCNT0`의 정확한 승인,
  plaintext 전송 수용, 기존 권리의 좁은 WS 적용, 새 유량 공지 확인, 당일 proof/immutable
  activation gates가 모두 필요한 credentialed 분기는 따로 잠근다. 자세한 소유/검증은 계약10~12절.

### WS-2 — 시장 시세 전용 승인키·WebSocket transport

- **분류:** intermediate/medium, WS-1 계약 고정 후 명세 구현. 계약 미정은 반환하며
  framing/reconnect/redaction 검증 반복 실패 시 저장소 모델 규칙에 따라 Luna max → Terra medium.
  2026-09-21 두 번의 독립 검증 실패 후 Terra medium으로 상향했다.
- **범위:** `crates/kis-client/src/market_stream*.rs` 등 시장 전용 신규 모듈,
  `crates/kis-client/src/lib.rs`, 해당 crate와 workspace Cargo 의존성/lock,
  `crates/kis-client/tests/market_stream*.rs` 및 synthetic fixtures.
  기존 계좌 `websocket.rs`/execution/order 모듈과 REST token ledger 동작은 수정 금지.
- **2026-09-21 잠금 계약 보완:** WS-2는 고정 production 경로의 state와 별도 root-owned
  connection/state anchor 두 개를 하나의 credential slot에 묶는 opaque `MarketStreamDomain`을
  구현한다. 매 획득마다 fresh open descriptor로 잠그며 approval/socket은 같은 domain을
  공유한다. default build에 임의 경로·분리 store·UID override를 노출하지 않는다.
  test-support에만 synthetic temp-root/loopback 생성을 둔다. 구체 모드/경로/재설치 조건은
  계약 5·9절을 따른다. 이는 SOURCE/offline 명세 채택이며 실제 provisioning 권한이 아니다.
- **남은 검증:** buffered text/control을 동일 decoder로 wire 순서대로 처리해 이미 도착한
  poison ACK 뒤 queued receipt가 나가지 않게 한다. 검사 상한에 도달하고 complete frame이
  남으면 fail closed; 미래의 incomplete frame을 예측하지 않는다. 정상 PINGPONG/후속 data를
  보존하고 ingress/liveness budget을 공통 적용한다. ACK 승인 직전 monotonic deadline을
  검사한다. malformed Close, exact duplicate, restart budget, canonical domain, framing/count/
  forbidden TR를 실제 socket/process fixture로 검증한다. packed frame의 receipt timestamp는
  불변이고 WS-3가 commit 시 session/3초 지연 제한을 다시 검사한다.
- **2026-09-21 전체 리뷰 N1–N3 보완:** 계약 4.2절의 fragmented message당 64-frame
  상한(빈 continuation/중간 control 포함, partial read 간 누적)과 synchronous drain당
  64 successful read 상한을 적용하고 소진 시 epoch/queue를 fail closed한다. 다른 종목의
  명령이 pending이어도 이미 ACK된 종목은 per-symbol 검사 후 보존한다. packed message는
  wall-clock와 monotonic 수신 시각을 각각 한 번만 채취하며 지연 dequeue에도 불변이다.
  실제 streaming fragment, 두 종목 ACK 교차, packed monotonic 동일성 회귀를 검증한다.
- **입력:** WS-1의 literal allowlist, auth/frame/state/types와 synthetic fixture 목록.
- **구현:** 서버 비밀 source 사용, 승인키 재사용/만료/재발급 fence, 지정 endpoint 외
  접속 거부, 구독 ACK/해제 ACK 분리, 시장 frame allowlist, heartbeat 처리, frame 크기
  상한/record count 일치/UTF-8·숫자 검증, bounded jitter backoff와 중단 상태.
  미확정 연결/중복 연결 시 보호 동작을 수행한다. 새 연결은 새 epoch이며 이전 epoch
  데이터는 최신 캐시에 게시할 수 없다. 문서상 재발급이 필요한 경우만 승인키 갱신한다.
- **검증:** 실제 loopback fake WS 서버와 fake approval HTTP를 통해 정상/거절/끊김/
  인증 만료/ACK 누락/다중 record/partial frame/oversize/계좌 TR/비정상 scheme/레드랙션
  경로를 실행한다. 순수 상태기계 테스트만으로 network transport 구현을 PASS하지 않는다.
  live key/실제 KIS 접속은 금지한다.
- **명령:** `CARGO_BUILD_JOBS=2 cargo test --locked --offline -p kis-client --test <추가한_target>`;
  필요한 새 의존성이 offline cache에 없으면 정확한 잠금 의존성 설치를 승인 범위 내 처리하고
  통과 전 누락을 보고한다. source fetch 실패를 성공으로 넘기지 않는다.
- **보고:** 공통 형식과 승인키/연결/구독 시도 횟수의 synthetic 증거, unverified live 범위.

### WS-3 — 30종목 구독 조정·캐시·실제 DB 경계

- **2026-09-21 실행 순서:** full WS-2 SOURCE/offline 독립 리뷰와 root 소스 해시 검증을
  통과했다. WS-3A는 별도 stream 타입·0055 migration·repository·실제 역할 DB 경계 tests를
  먼저 구현한다. root 검토 후 WS-3B가 producer·runner·transport config와 실제
  loopback→DB 통합/부하 증거를 연결한다. 두 부분은 순차 실행하며 하나의 WS-3 채택
  기준을 유지한다. 기존 운영 DB·서비스 사용 권한은 없고 disposable DB만 계획 범위다.

- **2026-09-22 보정 순서:** WS-3A 실제 역할 DB 실행은 55개 migration 적용 뒤 첫
  app lease 행 잠금 권한에서 실패했다. 독립 gap review를 root가 소스와 대조한 뒤
  C1(명세6.3의 최소권한 lock helper·publication owner mutex·활성 demand만 선택)을
  우선 채택한다. unpublished0055 up/down, stream repository, focused actual-role tests만
  수정하며 기존0001–0054·broad UPDATE grant·WS-2는 변경하지 않는다. 실제 app/worker/
  research_writer/admin 로그인, wrong-role/cross-owner/zero-row/GUC 복원/RLS와 revoke·
  generation·lease 교체 경쟁을 검증한다. C1 독립 인수는 전체 WS-3 인수가 아니다.
  C2는 frozen WS-2의 실제 command/ACK 증빙 seam을 별도 root 검토·범위 확정 후에만
  수정한다. 현재 gap review의 제안은 아직 C2 구현 허가가 아니다. C3는 C1/C2 독립
  검증 뒤 private producer adapter와 실제 loopback→DB ACK commit→receipt 게시를
  연결한다. 공개 bool/date/ordinal로 만든 ACK는 해당 통합의 증거가 될 수 없다.

- **2026-09-22 C2 계약 채택:** C1 최신 actual-role suite와 독립 잠금 검증·root 해시/
  격리 DB 정리 확인을 통과했다. 별도 C2 SOURCE 계약 검토를 반영한 명세6.4A를 root가
  채택하고, 기존 WS-2 중 아래 증빙 seam만 SOURCE/offline 구현 범위로 다시 연다.
  `crates/kis-client/src/market_stream.rs`의 opaque Prepared/ACK 타입·동기 prepare·
  소비형 send·단일 command phase, `src/lib.rs`의 세 타입 export,
  `src/market_stream_state.rs`의 기존 검증된 domain UUID를 반환하는 `pub(crate)`
  읽기 전용 getter 하나, 기존 `tests/market_stream_transport.rs`의 집중 증거만 수정한다.
  필요하면 기존 의존성만 쓰는 별도 transport integration test 파일 하나를 추가할 수 있다.
  wire/approval/Cargo/lock/state schema·I/O·잠금·권한·경로·예산·C1 repository/migration은
  변경하지 않는다. 구현은 명세가 확정된 Luna max 작업 하나이며, 이후 Sol high 독립
  SOURCE/실제 loopback 검증과 root 채택을 거친다. C2 구현은 아직 인수된 것이 아니다.
  DB·provider·production·runner/config·C3 작업 권한은 포함하지 않는다.
  두 시계의 예약 시점/기한은 영구 예약 전에 고정하고 DB 대기를 포함한다. ACK는 임시
  AckObserved 뒤 기존 bounded buffered 검사를 통과하고 정확한 durable clear가 성공한
  경우에만 반환한다. 취소·버린 capability는 pending 모호성을 지우지 않는다.
  default compile-fail, actual loopback의 전송 전 예약/expiry/취소/잘못된·중복·늦은 ACK/
  poison 우선 처리와 기존 library/domain/socket/process/doc 회귀를 `--locked --offline`으로
  검증한다. C3는 C2 독립 인수 뒤 raw storage ACK 입력 제거와 private adapter,
  실제 역할 DB의 pending commit → socket send → ACK commit → receipt 게시를 검증한다.
  전체 WS-3·WS-3B와 G1–G5 gate는 계속 열려 있다.

- **2026-09-22 C3 계약 채택:** C2-I1/I2 수정은 현재 소스 및 독립 검증
  166/166/9/42/19 증거를 root가 확인해 SOURCE/offline으로 인수했다. C3 계약 검토에서
  현재 DB에 예약 시각이 없음을 확인하여 명세6.4B를 채택한다. unpublished0055의
  `pending_reserved_at` 열·all-or-none 제약·정확한 worker column grant만 추가하고,
  C1 helper/RLS/잠금과 C2·Cargo·lock·market-data·runner/config는 그대로 유지한다.
  공개 scalar ACK/proof 입력은 제거하고 실제 Prepared/ACK만 private 저장소 경계로 받는다.
  ACK의 두 capture 시계에 `[reserved, deadline)`를 적용하며, timely capture 이후 DB
  commit 지연은 허용하되 최신 producer/grant/session/fence 검증을 유지한다.
  구현은 다음 두 패키지로 순차 분리한다. 병렬 준비 조사 결과는 이후 WS4/5의 입력이며
  해당 구현 gate를 제거하지 않는다.
  1. **C3A storage/schema/API closure — hard/medium, Luna max:**
     0055 up/down audit, `owner_equity_v2/market_stream.rs`, 부모의 명시적 safe exports,
     기존 boundary/support와 신규 내부 `market_stream_c3a_tests.rs`만 소유한다.
     외부 safe-DTO test는 유지하고 기존 실제 역할 case는 feature-gated 내부 library
     test로 옮겨 모든 C1 assertion을 보존한다. 공개 test factory는 만들지 않는다.
     실제 C2 capability로 setup을 교체하고 모든55 migration·실제bootstrap·direct roles,
     예약 열/제약/권한·pending identity·strict capture·지연 ACK 저장·default compile-fail을
     검증한다. 분리 후 library 실제 역할 case 실행 없이 외부 target PASS만으로 인수하지 않는다.
  2. **C3B owning producer integration — hard/medium, Luna max:**
     C3A 독립/root 인수 후 신규 `market_stream_producer.rs`, narrow exports 및 집중
     내부/loopback 역할 tests로 연결한다. private `Ready/InFlight/Terminal`과 session
     소유권 이동으로 모든 await 취소/실패/commit-unknown 후 재개를 막는다. pending commit
     전 bytes0, ACK commit 전 게시0, packed 데이터 보존, 실제 connection failure 및
     quote commit 결과의 정확한 재조회 증거를 검증한다. raw session/receipt/proof를 반환하지 않는다.
     C3A 최종 인수 후 root가 확인한 연결 보완: 이미 ACKED인 종목의 양수 수요 수만
     바뀔 때 기존 ACK/epoch/revision을 보존한다(명세6.4B). 이 한정된 저장 분기와
     신규 producer/private tests, 부모의 명시적 exports만 변경하며 C1/C2 및 기존
     C3A 검증은 보존한다. 수요0 경유 재구독에는 새 authentic ACK가 필요하다.
  두 패키지는 별도 Sol high 독립 검토와 root 채택을 거친다. worker는 명세 공백을
  임의로 메우지 않고 보고한다. DB 실행은 새 bounded dispatch에서 exact task-owned
  cluster와 종료·정리 범위를 명시한 경우만 허용하며, 이 문서 수정 자체는 실행 허가가 아니다.
  전체 WS3/WS3B·G1–G5는 아직 인수되지 않았다.

- **분류:** hard/medium. 구현 선택은 WS-1에서 확정하며 model은 명세 구현 규칙에 따라
  Luna max. 계약이 빠졌으면 worker가 아키텍처를 임의 결정하지 않는다. race 반복 실패면 상향.
- **범위:** `crates/market-data/src/`의 별도 stream quote 타입/exports,
  `crates/job-queue/src/owner_equity_v2/{intraday,intraday_producer}.rs`와 신규 stream 모듈,
  `owner_equity_v2.rs`, `src/bin/owner-equity-v2-runner.rs`, 필요한 config/exports,
  해당 Cargo/lock과 집중 tests. 필요한 신규 migration은 WS-1에서 예약한 번호/이름만
  사용한다. 이미 적용된 0054 수정이나 과거 철회한 audit 0055 재도입은 금지한다.
- **입력:** WS-2 transport, WS-1 stream receipt/DB/API 계약, 정상 admission/달력 계약.
- **구현:** READY identity 합집합을 구독하고 상류 ACK 상태와 desired set을 대조한다.
  키/host의 단일 연결 소유권을 프로세스 간 보장하고 한도 내에서 차분 subscribe/unsubscribe.
  consumer별 lease/참조와 pending/ACKed 집합을 관리하고, 제한된 해제 유예·변경 병합으로
  화면 전환 때 반복 등록을 억제한다. cache의 snapshot/status/버전 알림 계약을 공용으로
  제공하며 실제 전략 엔진을 연결하지 않는다.
  REST 공통 OS lock을 소켓 생애 전체에 잡아 EOD를 막지 않는다. 기존 REST limiter와
  token/state/debt는 보존한다. WS 모드에서는 REST 장중 producer가 함께 돌지 않게 한다.
- **게시:** owner/generation/session-date/source/epoch/fence/활성 demand를 transaction에서
  재검증한다. retired producer/old socket/이전 거래일 frame은 거부한다. 가격 전용 RLS와
  실제 writer role을 유지하고 새 입증 방식에 필요한 제약만 추가한다. REST reservation
  dummy를 만들거나 validation을 삭제하지 않는다. base_price 미제공과 아직 관측하지 못한
  halted 상태를 명시적으로 표현한다. WS의 TRHT_YN을 검증하며 REST에서 추정 이식하지 않는다.
- **부하/실패:** 수집기 메모리 최신값·bounded 내부 채널에서 30종목 latest-value coalescing,
  메모리 상한, 독립 status 이벤트, 느린
  DB/consumer 시 backpressure/재동기화. 연결이 끊기면 last-known을 남기되 live 상태를
  철회하고 gap을 표시한다. heartbeat로 price 수신 시각을 갱신하지 않는다.
- **검증:** 모든 repository migrations/실제 bootstrap roles를 적용한 disposable PostgreSQL에서
  30종목·중복 탭·동시 등록 상한·세대 변경·권한 철회·lease takeover·old epoch·commit
  실패·알림 유실·date rollover·quiet symbol·기존 REST source 격리를 검증한다.
  실제 producer + WS-2 loopback transport까지 잇고, 정상 stream 중 REST quote 호출 0을 센다.
  같은 종목 집합을 소비하는 local consumer 1개와 10개에서 broker 구독 수/REST 호출 수가
  동일한지, 한 slow consumer가 다른 consumer를 막지 않는지 검증한다. 등록·EOD 동시
  수요의 합산 예산과 WS 구독 변경 폭주 제한도 fake clock/실제 transport로 확인한다.
  서로 다른 아침/EOD 시간의 기존 calendar/sink/resolver regression을 보존한다.
- **명령:** jobs2/공통 lock으로 job-queue stream tests 및 collectors calendar/EOD focused
  tests. DB 환경변수 부재만으로 실제 DB 검증을 생략하지 말고 격리 DB를 준비한다.
- **보고:** migration/역할 변화, race 증거, 호출·write·메모리 budget 결과와 unverified 항목.

### WS-4 — Owner 인증 SSE·30종목 시세판·자동 등록 검증

- **분류:** intermediate/high. 고정 DTO와 게시 계약을 사용하는 API/Web 구현.
  실제 인증 철회/브라우저 검증이 반복 실패하면 한 단계 상향한다.
- **범위:** `crates/api-server/src/{http,repos}/owner_intraday_quotes.rs` 및 새 stream 모듈,
  router/state/session 연동에 필요한 한정 부분과 HTTP tests; `apps/web/lib/products/intraday-*`,
  `apps/web/components/stock-beta/quote/*`, 관련 dashboard/detail wiring와 tests;
  `deploy/nginx/nginx.conf`의 stream route와 해당 static tests;
  `scripts/qa/owner-equity-v2-live-acceptance.{mjs,test.mjs,d.mts}`.
  필요한 API Cargo/workspace lock은 이 패키지 단독 소유. ops/compose/migrations는 수정 금지.
- **입력:** WS-3 실제 cache/repository와 WS-1 freeze DTO. Next 로컬 버전 문서를 먼저 읽는다.
- **구현:** same-origin authenticated SSE 1개로 30행 snapshot/delta/status를 전달한다.
  외부 URL/proxy/cross-owner 전달을 허용하지 않는다. session expiry/logout/role 및
  membership 철회를 계속 확인하고 이벤트 전송을 중단한다. 쿠키/CSRF/origin 계약은
  기존 인증 방식에 맞추며 SSE URL에 session/승인키를 넣지 않는다.
- **재접속:** snapshot과 delta 사이 race를 버전으로 해소하고 중복 버전을 제거한다.
  gap/overflow/Last-Event-ID 보존 범위 초과 시 cache snapshot으로 다시 맞춘다.
  proxy buffering/cache를 끄고 heartbeat/idle timeout을 맞춘다. 브라우저 재접속은
  broker 연결을 종목 수나 탭 수만큼 늘리지 않는다.
- **UI/등록:** 기존 고정30 목록을 파일/정책과 대조한 뒤 정상 Owner API로 pilot-first
  자동 등록. 반복 실행/부분 성공은 안전하게 이어간다. 위젯 자동 demand와 QA가 서로
  duplicate demand를 만들지 않게 실제 화면 lifecycle을 따른다. 배치된30행, 선택
  상세, offline/connecting/awaiting-first-trade/closed/stale 상태와 마지막 체결·수신 시각을
  구분한다. 화면 종료/숨김/offline/logout 시 demand 해제 또는 bounded expiry.
- **검증:** 실제 API의 HTTP/SSE, nginx 경유 스트림, React와 local Chromium에서 30행
  업데이트/중복 탭/재접속/로그아웃/다른 Owner 접근/hidden/offline/초기 무시세를 확인한다.
  fixture sender/DOM 객체를 테스트 대상 자체로 대체하지 않는다. browser direct-KIS 차단,
  반복 GET quote 0, allowed same-origin SSE 관찰, 정지 후 quiet period를 실제 시간으로 검증한다.
- **명령:** `npm run typecheck`, 관련 Vitest/Playwright owner_only/off tests, targeted Biome;
  API 대상 tests. 일반 `test:e2e`만 나열하고 실제 환경 준비/실행을 생략하지 않는다.
- **보고:** synthetic와 실제 Owner/실제 provider 미검증을 분리하고, 사용자가 QA를 대신
  해야 한다고 완료 처리하지 않는다.

### WS-5 — 운영 모드·immutable 배포·runbook

- **분류:** intermediate/medium. 기존 공식 운영 경로 확장. immutable 수동 패치나 상태
  초기화가 필요해 보이면 멈추고 문제를 보고한다.
- **범위:** `scripts/ops/validate-production-config.sh`, `compose-release.sh`,
  `lib/kis-read-compose.sh`, 관련 intraday/runtime static/self-tests,
  `deploy/compose/compose*.yml`, `.env.example`, 필요한 기존 runner Dockerfile 설정,
  `docs/runbooks/stock-beta-intraday-quotes.md`와 release runbook의 실제 변경 부분.
  API/Web/nginx/Rust/Cargo/migrations와 diagram 파일은 수정하지 않는다.
- **2026-09-21 잠금 계약 보완:** WS-5는 `scripts/ops/provision-linux.sh`의 첫 설치·preflight·
  재실행 검사, root-owned persistent anchor 두 개와 writable state leaf, Owner runner 전용
  두 bind mount, ancestor/mount 조건 및 inode 보존 테스트를 소유한다. reapply는 기존
  anchor를 truncate/chmod/chown/replace하지 않는다. nonempty state/불확실성과 missing 또는
  mismatched anchor 조합은 hard stop이며 자동 migration/reset은 없다. 실제 root UID/GID와
  read-only mount 증거는 이후 승인된 disposable WS-5 검사로만 입증한다.
  최초 설치는 새로 만든 빈 domain에만 검토된 one-shot initializer를 실행해 nonempty
  slot/domain-bound 상태를 생성·fsync한다. preflight/reapply/runtime은 그 결과만 검증하고
  재생성하지 않는다. 이후 missing/zero-length 상태는 신규 초기화 신호가 아니라 중단 사유다.
- **입력:** WS-1 mode/env/네트워크 계약과 WS-3 binary/config 실제 동작. 기존 guard와
  새 WS 모드/transport를 명시적으로 검증하고 off 기본값을 유지한다.
- **구현:** 서버 측 승인키·연결 소유권 상태의 보호 경로와 mount 최소화, 정확한 envcommit,
  manifest/image/daemon 검증, calendar/session proof 검증, REST/WS producer 동시 실행 차단.
  호스트/date/proof/권한 부족을 구체적 static code로 보고한다. 웹/API에 KIS secret을
  주지 않는다. 운영계 접속 없는 계획/검사 모드를 유지한다.
- **rollback:** subscription/demand 종료→새 소켓 종료 확인→필요한 서비스만 off/허용된
  이전 호환 후보로 복원하는 절차를 만든다. 이력과 DB를 파괴하거나 구독 소유권을
  두 reader가 공유하지 않는다. old74를 DB/권한 호환 확인 없이 안전한 rollback이라고
  부르지 않는다. 후보 설치와 서비스 활성화는 별도 단계로 남긴다.
- **검증:** 공식 script 본문을 실제 disposable 경로/fake Docker에서 실행해 wrong envcommit,
  wrong image/daemon, expired proof, duplicate reader, 금지채널, 상태권한 오류를 차단한다.
  문서 코드를 복사해 비슷한 테스트를 만드는 것은 부족하다. 운영 디렉터리는 건드리지 않는다.
- **보고:** 정확한 변경·검사 결과 및 root용 preflight/build/install/activation/rollback 순서.
  diagram에 필요한 새 의존성/volume/route edge의 실제 파일·라인 목록도 제출한다.

### WS-6 — 독립 통합 검토와 로컬 인수

- **분류:** hard/high. 결론 자체가 산출물인 독립 검증; Sol high. 소스 수정은 하지 않고
  결함을 담당 패키지로 반환한다. 검증 실패를 새 요구사항이라고 완화하지 않는다.
- **범위:** 후보 전체 읽기, 격리 fake-provider/DB/browser 환경과 private report만.
  root가 완료한 diagram evidence/PNG, 잠금 의존성, network allowlist도 검토한다.
- **입력:** 각 worker report, root 통합 exact commit, WS-1 승인된 계약과 fixture 출처.
- **필수 검증:** transport fake socket → 생산 producer → 실제 writer role DB → 실제
  인증 API/SSE → nginx → 실제 Chromium까지 30종목을 연결한다. 다중 탭의 upstream
  구독 중복 0, 정상 REST quote 0, ACK 미승인 source 차단, slow consumer, noisy symbol,
  새 admission/generation, 계정/주문 TR 거부, revocation/expiry, reconnect/gap,
  notification-loss 복구, KST rollover, off 상태를 검증한다.
- **내구/자원:** 30분 이상의 bounded local stream soak, fixture burst와 단일 hot symbol,
  queue/메모리 상한, DB write 상한, 화면 latency 목표를 검증한다. 실제 시장 전체 트래픽을
  시험했다는 표현은 금지한다. 운영 호스트의 자원 상황이 나쁘면 무거운 검증을 격리한다.
  동일 종목의 소비자를 늘려도 상류 요청량이 증가하지 않는 증거, consumer별 해제/만료,
  느린 소비자 격리, 메모리 초기화 후 재동기화, 동시 REST 작업의 합산 예산을 검증한다.
- **회귀:** 기존 calendar 양방향 acquisition order/actual app-role resolver, exact-reference
  EOD 재시작 복구, entitlement amendment replay와 full-role RLS, release env/image guard,
  REST EOD token 공유, 기존 V1과 off-mode UI를 확인한다. 새로운 migration은 append-only
  업그레이드/호환성 검사; source-only rollback이 DB downgrade를 강제하지 않는지 검토한다.
- **판정:** 소스·DB·브라우저·운영 helper evidence matrix로 ACCEPT/REJECT. baseline 실패는
  비교 증거로 구분하고 신규 회귀는 면제하지 않는다. 실제 KIS/live acceptance는 미실행으로 남긴다.
- **보고:** 공통 형식과 후보 commit/diff 범위, 재현 가능한 실패, 운영 진입 전 잔여 조건.

### WS-7 — 운영 전환 및 실제 수신 검증

- **분류:** hard/medium. 실제 외부 상태를 다루므로 root가 각 단계의 증거와 기존 권한을
  확인한 후 다음 bounded 작업을 보낸다. 하나의 무제한 production prompt를 주지 않는다.
- **범위:** 명시적으로 승인된 후보·운영 시스템·30종목 정상 API·QA만. source 수정,
  추가 분해, 임의 재시도, API/한도 확대, account/order, legacy reader 임의 재시작 금지.
- **첫 단계는 읽기 전용:** 실제 root wrapper 만료, installed/current/env/12images/daemon,
  서비스·drained reader, pending jobs/timers, token/claim 비밀 제외 메타데이터,
  새 날짜의 proof, 권한/소유자/구독 한도, backup freshness/복구 가능성을 재확인한다.
  예전 report를 오늘의 측정값으로 쓰지 않는다.
- **빌드/설치:** root가 준비한 새 exact-commit inputs와 한정된 실행 권한으로 official
  image builder를 사용한다. 2~3서비스 논리 batch, 실제 Compose 호출은 1서비스씩,
  `COMPOSE_PARALLEL_LIMIT=1`, `CARGO_BUILD_JOBS=2`, 낮은 우선순위 systemd.
  batch마다 memory/PSI/OOM/서비스/프로세스/출처 검사. SwapFree는 telemetry이며
  승인된 현행 gate를 임의 변경하지 않는다. 실패 시 stop, 재시도는 root 판단 후.
  새 manifest actual12 IDs/OCI/env literal commit을 확인해 새 immutable 경로에 설치한다.
- **활성화:** 유효한 당일 calendar source/proof와 backup/restore gate를 먼저 충족한다.
  Sep19 재시도 금지를 유지한다. 기존 공식 installer의16:30KST cutoff는 우회하지 않는다.
  승인 범위의 시장 WS 한 종목을 pilot으로 연결해 ACK와 실제 후속시세를 확인한 뒤
  고정30 집합으로 확대한다. 인증/구독 오류는 고정 코드로 기록하고 무한 재발급하지 않는다.
- **실제 Owner QA:** 기존 인증 세션/정상 로그인 경로를 사용한다. agent가 유효 세션을
  확보할 수 없을 때만 사용자의 단회 인증이 필요한 이유와 정확한 절차를 제시한다.
  30개 정상 admission/READY 확인은 별도 단계로 먼저 진행 가능하면 진행한다.
  브라우저는 synthetic가 아닌 실제 서비스 페이지를 사용한다.
- **live 인수:** 30개 구독 ACK 또는 typed exclusion 상태를 각각 기록한다. 최종30 목표에
  exclusion이 남으면 전체 완료가 아니다. 실제 활발한 pilot 종목에서 첫 표시 이후
  최소2개의 새 WS 시세 이벤트를 `epoch + quote_version + received_at`으로 연결해
  캐시/SSE/DOM이 일치함을 증명한다. 가격 숫자가 우연히 같아도 새 수신 여부를 증명한다.
  다른29종목은 실제 거래 유무에 따라 새 시세/아직 체결 없음/거래정지를 정직하게 표시한다.
  원문 frame·credential·broker prose는 report에 남기지 않는다.
- **복구/종료:** 통제된 연결 끊김 후 재구독/첫 새 frame까지 stale 상태, 한 탭 종료 시
  다른 탭 유지, 마지막 탭 종료 후 bounded 해제/상류구독0, 재연결 폭주0을 확인한다.
  세션중 모드 전환/실제 장애 주입은 미리 승인된 좁은 테스트 범위만 수행한다.
- **EOD/마감:** WS 수신이 REST calendar/EOD 토큰/lock을 굶기지 않는지 확인하고 동일한
  당일 calendar 증거를 사용한 실제 EOD 게시를 검증한다. 시장이 닫혀 live 검증이
  불가능하면 시간·미실행 항목을 보고하고 승인된 bounded monitoring으로 이어간다.
  개발 완료와 운영/live/EOD 완료를 분리하며 사용자에게 최종 확인 가능한 결과를 전달한다.

## Coordinator gates

### 1. Pre-launch

1. 후속 실행 요청을 확인했다. `$paseo-delegate`를 읽고 profiles·범위·기준선·변경 파일을
   확인한 후 WS-1부터 시작한다. 원장은 `/tmp/lagrange-kis-stream-20260921/owned-workers.json`이다.
2. WS-1에서 미확인 protocol/한도/권리를 조사하게 하고 먼저 계약을 채택한다.
   현재 승인된 고정30 개인 시세판 목표와 충돌하는 결과가 나오면 제품 선택을 명시적으로
   되묻는다. 단순 구현 선택은 코디네이터가 결정한다.
3. 필요한 신규 API 허용 변경은 정확한 literal 표/위험/검증안을 만들어 요청한다.
   계획 승인이나 기존 REST 권한을 새 account/WS 네트워크의 포괄 승인으로 간주하지 않는다.
   이후 이미 승인된 동일 범위는 재승인을 반복 요구하지 않는다.
4. source/offline 작업과 credentialed operation의 권한을 구분한다. 운영 입력이
   미비하더라도 독립적인 source/test 준비를 중단하지 않는다.

### 2. Per-wave integration

1. 각 package의 실제 idle과 pending permission을 확인한 후 전체 보고서/diff/검사 증거를
   검토한다. 자동 검토 거절은 정확한 작업과 이유를 회수하고 승인 우회로 처리하지 않는다.
2. source wire/types/SQL 제약/route/환경 계약을 다음 패키지에 고정해서 넘긴다. 변경이
   필요한 경우 영향 받은 그래프·brief부터 수정한다. 관성으로 adjacent scope를 확장하지 않는다.
3. Cargo는 `/tmp/lagrange-kis-live-cargo.lock` 등 실행 시 확인한 공통 lock으로 직렬화한다.
   disposable DB는 고유 컨테이너/network/path로 격리하고 제거 증거를 남긴다.
4. Wave4 종료 후 coordinator가 `.puml` 두 파일의 실제 구조·파일:라인 evidence를
   갱신하고 로컬 PlantUML로 PNG 두 개를 렌더링한다. 존재하지 않는 계획상의 edge를
   운영 구조처럼 그리지 않는다. `.puml`과 PNG를 같은 통합 커밋에 포함한다.
5. 필요한 새로운 시장 시세 allowlist 승인 후 해당 AGENTS/runbook/spec의 좁은 예외를
   일치시킨다. 기존 계좌/주문 금지는 유지한다. worker가 정책을 스스로 넓히지 않는다.
6. root만 exact reviewed files를 커밋한다. 독립 WS-6가 실패하면 원 소유자 수정 후
   변경 영향을 재검증한다. PASS하지 않은 코드로 이미지를 빌드하지 않는다.
7. 실행자는 callback과 bounded heartbeat를 사용하고 owned worker 원장에 상태/정확한
   pending request/보고서/채택을 기록한다. 고정 heartbeat가 끝나기 전에 회수·갱신 또는
   명시적 외부 차단 인수인계를 한다. 계획 단계에서는 heartbeat를 만들지 않는다.

### 3. Final end-to-end acceptance

아래 항목을 별도 칸으로 기록하고 하나를 다른 하나의 증거로 대체하지 않는다.

| Gate | 완료 증거 |
|---|---|
| Source/local | 독립 ACCEPT, 전체 역할 DB, loopback WS, 실제 local Chromium/SSE, 부하·회귀 통과 |
| Deployment | 새 exact commit의 manifest/실제12image IDs/OCI/env, 정상 서비스/reader ownership |
| Onboarding | 정확한 기존30 목록 자동 등록 및 각 READY/admission; 사용자 수동30회 입력 없음 |
| Provider subscriptions | 실제 시장 채널만 사용, 30구독 ACK, 연결/구독 한도 준수, REST quote 반복0 |
| Shared collection | 동일 종목 다중 소비자에 상류 중복0, bounded 메모리/채널, 느린 소비자 격리, REST 합산·WS 명령 예산 준수 |
| Live screen | 두 후속 실제 시세의 수신→cache→SSE→DOM 일치, latency 측정, nontrading/quiet 상태 정직 표시 |
| Lifecycle/security | logout/권한철회/세대변경/종료/재연결/gap, no duplicate upstream, credential/원문 노출0 |
| EOD | 같은 날짜·동일 calendar 출처의 실제 EOD 게시, 기존 권한별 가격 이력 유지 |
| Handoff | agent가 검증한 증거와 한계를 제시한 뒤 사용자 최종 확인; 미완료 시 정확한 재개 조건 |

계획 문서 작성은 완료했고 후속 실행 요청을 받았다. WS 구현/운영 전환/30종목 실시간
시세판은 아직 완료되지 않았다. 기존 코드를 보존하고 WS-1의 계약과 명시된 gates를
기준으로 위 패키지를 순서대로 수행한다.
