# R3 병행 최초 대조 — 소스·계약·공개 공식 자료

확인일: 2026-10-21 KST. 소스 기준: `73f25931e79ff199e17358a09ec94f10f53604fb`,
브랜치 `r3-parallel-20261021`. 담당 범위는 [병행 인계](../handoffs/2026-10-21-r3-parallel-workstream.md)의
첫 bounded 실행이다. 이 문서를 추가한 커밋은 문서 조사 결과이며 빌드·배포 후보가 아니다.
§1–7의 저장소 `파일:행`은 이 소스 기준이다. 후속 D1 수정과 당시 검증 제한은 §8, 실제 집중 검증은 §9에 기록한다.

**판정:** R3-01 소스·계약 대조와 R3-06 공개 자료 재확인을 진행했다. 실제 설치 재검증,
시장 연결·수신·Owner 화면 인수는 수행하지 않았다. 상세 화면이 전체 READY 수요를 만드는
소스 차이 1건을 확인했다. 후속 §8의 수정/회귀 `7b7f219`를 §9에서 검증해 수정 전 4실패,
수정 후 해당 suite 14통과를 확인했다. 부모는 D1만 SOURCE_FIX_AND_FOCUSED_REGRESSION_ACCEPTED로 인수했다. 다음 공동창의 도구·전제는 §10에 구체화한다. R2 완료를 기다릴 필요는 없으나 공유 운영 변경은 직렬화한다.

## 1. 승인, 역사 증거, 현재 미확인을 분리

- [ADR-0009](../decisions/0009-kis-owner-directed-websocket-activation.md):6–48의 평문 시장 연결과
  제한된 활성화 승인은 유효하다. G2 미확인 상태의 제한된 배포 예외를 적용하며 같은 Owner
  승인을 다시 묻지 않는다. 시장 연결 1개, 고정 우주 H0STCNT0 최대 30등록, 실제 READY와 정상
  Owner 수요, 당일 proof, 기존 실패 예산은 유지한다. G2를 VERIFIED로 바꾸는 결정은 아니다.
- [WS 계약](../superpowers/specs/2026-09-21-kis-market-stream-contract.md):1163–1186 및
  [10월 1일 readiness](../runbooks/kis-market-stream-live-readiness.md)의 초기 승인 대기 문구는
  역사 상태다. 승인 판단에는 ADR-0009를 우선한다. 보호된 grant와 당일 증거의 실물 검증은 남는다.
- [10월 4일 인수 표](../superpowers/plans/2026-10-01-kis-market-stream-completion.md):1976–2017는
  SOURCE/LOCAL ACCEPT, 실제 역할 DB/API/nginx/Chromium, 합성 상류 30종목·1800초 soak를 기록한다.
  p95 355ms, 추적 allocator superset 5,940,576B는 그 격리 실행의 수치이며 운영 수치가 아니다.
  804개 소스 입력의 독립 검토와 그 뒤 Low 보정 이력도 그 시점의 인수다. 이번에 private 원본
  로그를 다시 회수하거나 그 시험을 반복하지 않았다. 관련 완료 소스 커밋은
  `150bee84c13d3339c6f62637530665e871898019`이다.
- [10월 7일 인계](../handoffs/2026-10-07-kis-websocket-only.md):24–50는 Rust 59개,
  Web 192개와 합성 HTTP/2→실제 Chromium 10 PASS/숨김 1 NOT_RUN을 기록한다.
  전체 browser QA `passed:false`를 보존한다. native visibility 미관측을 JS 덮어쓰기로 대체하지 않는다.
- 같은 인계 :75–104는 `a65f2082f59ba5d66cb7cdf482b1f6cc7e08fc20` quotes-off 실제 설치와
  당시 health/API pool32, 당일 calendar/window·entitlement·persistent domain 확인을 기록한다.
  당시 grant/producer/cache/subscription/lease 0, READY `005930.KRX` 1개는 **10월 7일 관측**이다.
  ADR의 rollback `a68e4dd753271d15f5311e85c1de573e046b7e96`도 역사 지시이며 오늘 바로 실행할
  rollback 대상으로 확정하지 않는다. R2 schema62·reader floor와의 호환성을 먼저 확인해야 한다.
- 선택한 WS transport/state/wire, runtime/producer/repository, runner 진입점, API WS HTTP 모듈,
  Web quote/provider/client, migrations0055–57, WS Compose overlay 및 `compose-release.sh`는
  `git diff e639169..73f2593 -- <해당 경로>`에서 변경이 없었다. 상세 route도 동일하다.
  이는 해당 소스의 동일성이지 설치된 바이트·설정·현재 서비스 상태의 재검증은 아니다.

## 2. R2 읽기 전용 인계 확인

읽은 registry는 `/data/worktrees/3puw275b/excited-dolphin/target/r2-daily-beta/owned-workers.json`이다.
`coordinatorTask.boundedHandoff`가 가리킨 파일:
`target/r2-daily-beta/wp-8/wave137/heartbeat-renewal-20261021T0300-closure.json`.
그 파일의 직접 계산 SHA-256과 registry의 `boundedHandoffSha256`은 모두
`bd5be4eb98e13a88d57e0f0e008bcc8e2610b15f607985ed08c6c370df1840c5`로 일치했다.
closure 시각은 `2026-10-21T03:04:14.238597Z`이며 신규 운영 관측이 아니다.

| 인계가 기록한 사실 | 이 조사에서의 취급 |
| --- | --- |
| 실제 앱 `e639169018519fd6636c5579d1957e323e2c7802`, schema62; G4·공식 백업 전환·실제 암호화 백업/격리 복원 완료 | 기존 R2 인수 보존. 오늘의 서비스 health나 WS grant 인수로 확대하지 않음 |
| 후보 `fccb68354c5352f741c37764ae1d66eb3784a662`, CI39457995014 attempt1 8성공; hosted closure `a15dcbaab003f8d31531dfc5263e9154398fbf978be0459fc7e11115190400bd` | hosted source 인수. 새 이미지/배포 0. 이번에는 CI/로그/실행을 반복하지 않음 |
| 10월20일 16:44:31UTC 관측: dailyOFF/epoch0·실행 job0. 마지막 `lastObservedAt=17:03:00.525235UTC` | 마지막 시각은 a2 제안 유닛 4개에 한정된 관측. 전체 health 재관측이 아님 |
| a2 17:01:50UTC 비대화형 sudo 인증 거부, root Python 실행 전 종료 | 승인 부족으로 바꾸지 않음. a1/a2 소진·재실행 금지. 인증 복구를 이 조사에서 시험하지 않음 |
| 기업행사 13일 검증/40일 누락, 달력 9월24일–10월6일 공백; KIS timer disabled | 과거 입력 inventory. 오늘의 calendar/window 유효성을 증명하지 않음 |
| 첫 자동 세션 null, Owner 사용 false, 실제 거래일0, C_R2 false | 소진된 10월21일 조건부 후보를 소급 사용하지 않음. R3가 R2 시작일이나 완료를 대신 결정하지 않음 |

[Wave137 문서](2026-10-19-r2-g4-compatibility-ci-and-operating-gaps.md):2152–2205와 대조했다.
registry·부모 작업공간·운영 파일은 수정하지 않았다. R2의 DB/파일 writer 없음 선언은
다른 기기의 동일 KIS 키 WebSocket 점유가 없다는 증거가 아니다.

## 3. R3-01~07 근거와 남은 차이

| 작업 | 확인한 소스·역사 증거 | 남은 차이와 정확한 다음 실행 | 공유 운영 의존성 |
| --- | --- | --- | --- |
| R3-01 DOING | ADR-0009; `market_stream.rs`:1562–1626의 ACTIVE grant/slot/generation/network/rights 결합; `market_stream_runtime.rs`:312–386의 DB 시간·당일 calendar/window 결합; 공식 grant/provision helper | 소스 대조 완료 범위만 기록. 실제 Owner·권리·grant·state/anchor·release/image·pool·당일 proof의 최신 제한 조회안을 부모에게 전달하고 조율 후 확인 | 같은 PG·Owner runner·release lock·credential slot·calendar. 과거 grant0을 현재 부재로 단정 금지 |
| R3-02 TODO | `market_stream_wire.rs`:12–17,465–552의 47×record 수/필드/날짜 검증; transport :32–56의 정확한 URL·cap30; 실제 수신 인수 없음 | §5의 1종목 pilot. ACK, 최초 수신 뒤 두 새 receipt, 해제/닫힘을 별도 기록. §4 D1 수정 또는 실제 수요 합집합이 정확히 1이라는 전제 필요 | 정상 로그인·READY/admission·당일 proof·grant·하나의 market socket |
| R3-03 TODO | API `owner_market_stream.rs`:48–112,424–436의 session/Owner/origin 재검증; nginx :86–99; Web controller :130–207,394–441; 10월7일 숨김 NOT_RUN | 최초 DB→SSE→DOM 상관관계 후 native 새로고침/이동/로그아웃/숨김·복귀/offline·복귀. 소비자 종료·새 lease와 TTL 확인 | 같은 API/Web/nginx/Owner 세션. 쿠키 추출·세션 삽입 금지 |
| R3-04 TODO | 고정30 config, `intraday-stream-universe.ts`:23–39 READY 필터; 실제 역할/합성 full30·공유 수요의 10월4일 검증 | pilot·인증 수명주기 인수 뒤 나머지29 정상 순차 onboarding. 30 실제 admission/ACK 및 1탭/10탭 상류30 유지 확인 | admission/EOD REST의 기존 shared budget, DB 작업량, R2 입력 수집 시간. fixture·SQL READY 생성 금지 |
| R3-05 TODO | `market_stream_state.rs`:150–185,226–287의 영속 명령/연결 예산; producer :2968–2970의 수요 freshness/clean drain; 계약 §7 및 10월4일 fault/soak | 격리 장애 이력을 보존하고 필요한 변경 영향만 후속 검증. 실제 단절/노후/장 종료/다음날 proof 공급은 별도 운영 관측. throttling을 얻으려고 한도 초과 요청하지 않음 | R2 calendar/EOD와 서비스 중단. 생산 DB 장애·권리 철회 강제 주입은 이번 범위가 아님 |
| R3-06 DOING | §6의 오늘 공식 필드·Approval·새 GitHub SHA 확인. 공개 공지 접근 한계 기록 | G2 UNVERIFIED 유지. 정상 접근으로 회수 가능한 공식 한도/적용일·슬롯 사용 사실 확보. ADR-0009 제한적 pilot은 새 Owner 승인 대기로 되돌리지 않음 | 키 값 없이 운영자가 확인할 다른 host/client 사용. 호스트 내부 lock이 전역 독점은 아님 |
| R3-07 TODO | SOURCE/LOCAL·역사 설치 증거와 [최종 matrix](../superpowers/plans/2026-10-01-kis-market-stream-completion.md):1615–1632 | exact 후보·설치·pilot·full30·수명주기·p95·동일 출처 EOD·다음날 proof·Owner 확인을 연결한 최종 기록 | R2 실제 EOD는 날짜/source/hash가 같은 증거만 재사용. R2 3거래일 완료와 R3 인수는 서로 대체 불가 |

위 표의 Rust `market_stream*.rs`는 달리 명시하지 않으면
`crates/job-queue/src/owner_equity_v2/`, wire/transport/state는 `crates/kis-client/src/` 아래다.
API 파일은 `crates/api-server/src/http/`, Web controller는
`apps/web/components/stock-beta/quote/`, universe는 `apps/web/lib/products/` 아래다.

## 4. 코드 결함과 운영 관측 공백

### D1 — 상세 화면 수요 범위가 계약보다 넓음 (소스 확인, 실행 재현 미실시)

계약 :302–305는 board 수요와 detail 1종목 수요를 구분한다. 현재 호출 경로는 다음과 같다.

1. `apps/web/app/(authenticated)/stock-beta/[instrument]/page.tsx`:59–78는 조회한 **전체**
   `memberships.memberships`를 `streamMemberships`에 넣는다. 같은 자리의
   `matchReadyIntradayQuoteMembership`은 별도 `intradayMembership`만 제한한다.
2. `apps/web/components/stock-beta/stock-beta-detail.tsx`:59–63는 전체 배열을 provider로 넘긴다.
3. `apps/web/components/stock-beta/quote/intraday-stream-provider.tsx`:55–71와
   `apps/web/lib/products/intraday-stream-universe.ts`:23–39는 선택 종목 인자를 받지 않고
   고정30 안의 모든 READY identity를 만든다.
4. `apps/web/components/stock-beta/quote/intraday-stream-controller.ts`:260–270는 그 identity
   전체를 lease 요청으로 보낸다. 따라서 선택 종목 외 READY가 있으면 상세 화면도 추가 수요를 만든다.

이는 허가되지 않은 종목을 우회하는 결함으로 단정한 것이 아니다. 기존 READY·고정30·권리 검사는
유지되지만, 정상 상세 화면으로 **단일 종목 pilot을 제한하려는 계약을 충족하지 못한다**.
선택 종목의 generation이 맞지 않을 때 다른 READY 종목만 요청할 수도 있다.
해당 route의 마지막 변경 커밋은 `150bee84c13d3339c6f62637530665e871898019`이며
`e639169..73f2593` 사이 그 파일 변경은 없다. 운영에서 발생했다는 주장은 하지 않는다.

후속 구현은 위 route와 `apps/web/tests/stock-beta-intraday-page-seam.test.tsx` 두 파일로 한정한다.
기존 matcher의 선택 종목·generation·READY 결과를 단일 배열/빈 배열로 전달하고 board의 전체 수요는
보존한다. 검증은 READY 두 종목에서 선택 하나만, generation 불일치/미READY에서 수요0,
board에서는 두 READY 유지, off/Member에서 비활성이라는 실제 전달 경계를 확인한다.
기존 SSR markup 검사만으로 lease 크기를 증명하지 말고 provider에 전달되는 identity 또는
클라이언트 lease 요청을 관측한다. 이번 실행에서는 구현·테스트 작성/실행을 하지 않았다.

### D2 — 옛 문서의 개정·운영 전제가 현재 판정으로 오인될 여지 (문서 정리 대상)

- 공식 샘플이 46필드라는 WS 계약의 9월21일/10월1일 관측은 당시 SHA의 역사 증거다.
  오늘 확인한 새 SHA에서는 47필드다(§6). 과거 내용을 삭제하지 말고 후속 개정으로 연결한다.
- 초기 승인 대기는 ADR-0009로 해소된 범위가 있다. 반대로 옛 quotes-off rollback 경로는
  이후 R2 reader floor·schema62 호환성의 현재 증거가 아니다.
- `compose-release.sh`:703–709는 WS refresh에도 release lock과 `guard-processes`/`guard`를
  적용한다. `owner-beta-daily-operations.py`:672–692는 실제 daily cohort 정지와 DB disabled,
  state 검증을 요구한다. `owner-beta-daily-release.py`:529–549는 reader floor를 보존한다.
  이 guard를 우회하거나 R3용 두 번째 배포 경로를 만들 필요는 없다. R2와 현재 source/target,
  disable/drain·복귀 책임을 함께 바인딩해야 한다.

D1 외 대조한 경로에서 새 구현 결함을 확정하지 않았다. 전체 코드 무결성을 보증하는 전수 리뷰는
아니다. grant 부재/만료, 현재 state 손상, 현재 당일 proof 누락, 실제 wire 불일치나 전달 실패는
이번 운영 미조회 때문에 **UNKNOWN**이며 코드 수정 근거로 삼지 않는다.

## 5. 다음 실행 순서 — 부모의 운영 시간 조율 뒤

아래는 실행안이며 이번에 실행한 절차가 아니다. 실행 전 대상·시각·종료 한도·관측 횟수·복구 담당을
고정한 packet을 부모가 R2 작업과 대조한다. 기존 승인의 재확인이 아니라 공유 자원 직렬화다.

1. **소스 후속:** D1의 두 파일 수정·영향 검증을 먼저 묶는다. 실제 READY가 정확히 1개일 때만
   현 코드로 1종목이 될 수 있으나 현재 개수는 미확인이다. 역사 READY1을 근거로 진행하지 않는다.
   변경 후보 검증은 R2의 이미 성공한 CI를 재실행하는 이유가 되지 않는다.
2. **현재 전제 한정 확인:** 부모가 운영 실행 가능 시점에 exact current/manifest/image,
   schema0055–57 및62 이력, API pool≥24(기본32), WS mode/slot/generation/grant,
   ACTIVE entitlement·Owner·기간·network/wire/universe pin, domain/anchor identity와
   영속 예산 상태를 보호 절차로 확인한다. 기록은 hash·metadata·typed result만 남긴다.
   다른 기기의 키 점유/현행 quota 미확인은 별도 G2로 남기며 충돌·거절이면 닫는다.
3. **당일 입력과 안전한 복귀 준비:** 실제 KST 거래일의 KIS calendar source/version/batch/hash와
   운영 window의 같은 날짜·반개구간을 검증한다. EOD와 동일 source를 재사용하고
   `chk-holiday` 중복 호출·미확정 claim 재시도를 하지 않는다. 다음날 수동 proof 담당·cutoff·
   부재 시 off를 지정한다. R2 reader floor/현재 schema에 맞는 quotes-off release를 먼저
   검증해 rollback 대상으로 고정한다. 과거 a68을 자동 선택하지 않는다.
4. **필요한 설치만:** 현재 상태를 확인한 뒤 변경이 필요하면 별도 clean activation commit,
   exact12-image V2 manifest, immutable env를 준비한다. 빌드는 기존 1서비스/Compose,
   jobs2·parallel1·저우선순위 background와 batch별 자원/health 규칙을 적용한다.
   기존 state는 공식 `provision-owner-market-stream.py --check` 대상으로 보존하고
   initialize-new를 재실행하지 않는다. 검토된 grant는 공식 installer로만 처리한다.
   정상 existing ACTIVE grant의 정확한 replay와 revoked/conflict/불명 결과를 구분한다.
   설치본을 직접 편집하거나 수동 SQL로 admission/grant를 만들지 않는다.
5. **1종목 pilot:** 정상 `/login`→Owner session→실제 READY `005930.KRX` 확인 후 실제 페이지
   하나의 수요를 사용한다. 추가 QA lease/숨은 탭은 만들지 않는다. D1 보정된 상세 route는 해당
   종목의 실제 신호 상세가 있어야 열리므로 그 전제도 확인한다. 신호가 없고 READY1만 있다면
   전체 board의 정상 수요 합집합1을 확인한 경로를 쓴다. 요청할 identity와 다른 활성 수요의 합집합이1임을 확인할 절차를 먼저 고정하고
   보호된 공식 refresh 경로로 API→Web→runner를 순차 갱신한다. 이미 활성화된 설치라면
   불필요한 재설치/재발급 없이 같은 검증된 경로에서 관측한다.
6. **프로토콜·새 수신:** 정확한 symbol/operation/epoch의 ACK 뒤 유효 47필드 메시지와 DB 게시를
   확인한다. 최초 표시 뒤 **최소 두 번의 후속 fresh receipt**를
   `epoch/receive_ordinal/quote_version/received_at/committed_at/DOM 시각`으로 연결한다.
   가격이 같아도 새 receipt는 구분한다. heartbeat/status sequence 증가만으로 통과시키지 않는다.
   실제 PINGPONG·control close는 관측한 것만 기록한다. quiet·stale·미ACK는 미완료다.
7. **pilot 종료와 인증 수명주기:** 정상 페이지 이동/닫힘으로 lease release, renew 중단,
   마지막 demand의 게시 중단과 socket/task 종료를 확인한다. 계약 :324–327은 전체 수요0이면
   5초 안의 소켓 종료를 허용해 종목별 해제 ACK가 필수인 경로와 다르다. 실제 unsubscribe를
   보낸 경우 ACK를 기록하고, whole-socket close를 사용했다면 `unsubscribe ACK NOT_OBSERVED`
   와 definite close를 각각 남긴다. 못 본 ACK를 성공으로 쓰거나 이를 얻으려고 두 번째 종목을
   조기 등록하지 않는다. 명시적 해제 ACK는 후속 다종목 단계에서 한 수요만 제거할 때 확인한다.
   실제 새로고침·native hide/restore·offline/online·logout에서 기존 SSE 종료/가격 제거,
   새 consumer/lease·snapshot을 확인한다. release 응답 유실은 TTL 정산이며 자동 retry 금지다.
8. **나머지29→full30:** pilot과 인증 경로 확인 후 정상 Owner onboarding으로 순차 등록한다.
   기존 READY/generation을 재사용하고 indeterminate 작업 재시도·강제 READY를 하지 않는다.
   각 admission/ACK 상태를 모두 기록한다. 1탭→10탭의 동일30 합집합/상류30을 확인하고,
   1초 command 간격 때문에 걸리는 시간을 고려한다. pilot 종료/재개가 소비한 예산도 누적한다.
   30행 UI가 보인다는 사실과 30 ACK는 다르다. 조용한 종목의 체결은 만들지 않는다.
9. **운영 인수:** latency p95≤2s와 병합/누락 표본수, stale/장 종료·다음날 proof,
   동일 날짜/source EOD 게시를 별도 기록한다. 단절/권한·DB 장애 주입은 기존 격리 증거와
   실제 관측을 구분하고, 추가 운영 변경은 부모와 다시 시간·영향을 조율한다. R2 입력/실행/
   release 변경이 3거래일 관찰 재시작 조건에 해당하는지 R2 담당이 판단한다.

**실행 예산의 상한(새 허가나 목표 호출량이 아님):** Approval는 유효 키 재사용,
60초 간격·rolling24h 최대2, 불명 dispatch 자동 재발급0. 연결은 동시1·최소10초 간격,
10/20/40/60초 backoff+0–20% jitter·6회/10분·20회/24h. 명령은 전역 pending1·간격≥1초,
120회/10분·1000회/24h, desired/pending/ACKED≤30. 실제 남은 예산은 영속 상태에서 확인해야
하며 이 문서가 초기화하지 않는다. ACK deadline5초, 마지막 수요0 close≤5초,
브라우저 lease30초/renew15초, cleanup 유실은30+1+5초 경계를 관측한다.
수치 근거는 계약 :113–115,134–153,300–367 및 소스 approval :19–25/state :150–287이다.
pilot 총 관측 시간·명령/조회 횟수는 운영 packet에서 부모와 고정할 미완료 항목이다.

**복구:** 정확한 producer와 socket/child task 종료를 증명하고 불명 상태·counter·anchor를
보존한다. 검증한 호환 quotes-off release로 공식 rollback/refresh를 수행한다.
`compose-release.sh`:567–648의 source manifest와 running-container guard 때문에 혼합/정지
상태는 일반 refresh 성공 전제가 아니다. 정확한 서비스 복구안을 먼저 조율하며 guard 우회,
REST fallback, down migration, grant/state 삭제나 quota 초기화는 하지 않는다.

## 6. 공개 공식 문서 — 2026-10-21 재확인

인증·쿠키 없는 공개 GET만 사용했다. 시장 데이터 호스트·Approval 발급·WebSocket은 호출하지 않았다.
web 도구는 포털 속성 두 URL과 9월16일 상세를 열지 못했다. sandbox Python의 최초 공개 조회도
URLError로 실패했다. 정확한 공개 조회 명령의 native permission 후 포털 속성/공식 GitHub 읽기에
성공했다. 이 접근 복구는 운영 sudo 인증이나 provider 권한의 복구가 아니다.

| 공식 출처 | 이번 관측·개정 | 판정 |
| --- | --- | --- |
| [H0STCNT0 속성](https://apiportal.koreainvestment.com/api/apis/guide/property/714d1437-8f62-43db-a73c-cf509d3f6aa7) | 04:15:57Z 및 04:17:46Z HTTP200; 21,487 bytes, 전체53 속성 중 `res_b`47, 순서001–047/끝 `MARKET_CLS_CODE`; 개정 `2026-09-11T16:31:27+09:00`; SHA `ea945675204c6394b614124012995daaad5080eec7a62339f72c9aa613d6df68` | 기존 wire 계약/문서 hash 일치. 실제 wire 수신 증거는 아님 |
| [Approval 속성](https://apiportal.koreainvestment.com/api/apis/guide/property/5c87ba63-740a-4166-93ac-803510bb9c02) | 04:15:57Z/04:17:12Z HTTP200, 2,429 bytes/5속성; 개정 `2024-12-13T15:21:56+09:00`; SHA `a2f5272fe9eb3b86c99900e2ccf0bf3fea8988b8cc70b5ff8d3a800705a4c7ec` | request의 grant_type/appkey/secretkey와 기존 필드 문서 동일. 접속키 발급 아님 |
| [공식 GitHub HEAD API](https://api.github.com/repos/koreainvestment/open-trading-api/commits/main) | 04:17:13Z HTTP200, SHA `2077773a69a428daf63e148182511f068cc45de3`, committer `2026-10-14T05:02:51Z` | web의 cached commit 목록은 여전히8월26일을 보였다. 이번 직접 반환 SHA로 고정하며 이후 HEAD까지 주장하지 않음 |
| [고정 시장 함수](https://github.com/koreainvestment/open-trading-api/blob/2077773a69a428daf63e148182511f068cc45de3/examples_user/domestic_stock/domestic_stock_functions_ws.py#L319-L385) | 04:17:47Z HTTP200, 70,212 bytes; SHA `b282661c47287d5374a7b67256a91503e1b16b2dfeb40490a581a65e80918453`; `ccnl_krx` columns47/끝 MARKET_CLS_CODE | 옛46필드 충돌은 이 SHA에서 해소. docstring :331의 해제0은 여전히 helper의2와 충돌하므로 따르지 않음 |
| [고정 auth helper](https://github.com/koreainvestment/open-trading-api/blob/2077773a69a428daf63e148182511f068cc45de3/examples_user/kis_auth.py#L778-L784) | HTTP200, 24,711 bytes; SHA `2c23617eb9829934dc14f530cfae126426676919b630a07aa2a4832682ffdb9f`; :487 Approval, :513–534 envelope, :698–700 PINGPONG→pong, :784 해제2 | 기존 정확한 경로/해제2 유지. :710의40은 open_map key 검사이므로 broker 등록 quota로 사용하지 않음. 샘플의 원문 로그/AES/계좌 경로를 실행·복사하지 않음 |
| [고정 공개 config](https://github.com/koreainvestment/open-trading-api/blob/2077773a69a428daf63e148182511f068cc45de3/kis_devlp.yaml#L26-L30), [시장 예제](https://github.com/koreainvestment/open-trading-api/blob/2077773a69a428daf63e148182511f068cc45de3/examples_user/domestic_stock/domestic_stock_examples_ws.py#L20) | HTTP200; config SHA `61f036e51e02c4f8b86fb26e361fe98ecb26e0d587160d724f2d62768a60e2a2`, 예제 SHA `6ac6ee8fefc1fa48554b0496caeaef3de95bb2aa4870786e274a584d488c7b73` | 운영 ws:21000와 /tryitout 유지. mock/다른 endpoint로 바꾸지 않음 |
| [공개 공지 목록 JSON](https://apiportal.koreainvestment.com/api/forums/10000000-0000-0011-0000-000000000001/posts?size=10&sort=createdDate,desc) | 04:17:12Z/46Z HTTP200, 10,491 bytes; SHA `7cc335f5d306bd66ef184dbb7dc76a4709e4b56e33f329bcd99b8b7a07f4ecd2`; 반환10개 최상단9월9일, 9월16일 항목 없음 | 첫 추출기는 object를 예상해 AttributeError; list형으로 바로잡아 같은 hash에서 title/date만 대조. 한 페이지의 부재는 공지 철회·quota 복구 증거가 아님 |
| [9월16일 공지 상세](https://apiportal.koreainvestment.com/community/10000000-0000-0011-0000-000000000001/post/2b641ee8-b594-427a-9d11-ee6e764f0453), [옛 유량 공지](https://apiportal.koreainvestment.com/community/10000000-0000-0011-0000-000000000001/post/d0d1a83f-6f8d-4437-9700-6d26702fd989) | 9월16일: web 접근 실패, 직접 공개 HTML HTTP500. 옛 공지: web은 본문 없는 템플릿만 반환 | 본문·현행 적용 수치·시행일 미확인. 과거 1세션/41등록을 현재 보장으로 쓰지 않음. 로그인·잠금 우회·비공개 조회 없음 |

공식 공개 자료의 본문/관리자 metadata는 이 보고서에 저장하지 않고 URL·개정·해시·관련 관측만 남겼다.
프로젝트 보수 예산과 KIS quota는 다르다. 오늘의 공개 확인만으로 G2나 LIVE_ACCEPTED를 닫지 않는다.

## 7. R2와 조율할 대상 및 이번 확인의 한계

| 공유 지점 | 다음 packet에 반드시 결합할 내용 |
| --- | --- |
| API/Web/Owner V2와 nginx | source/target image·manifest, 실제 서비스 ID, refresh 순서와 정상 종료·복귀 책임. R2 배포/빌드/복원과 겹치지 않음 |
| PG와 R2 daily cohort | migration62/권한·daily disabled/quiescence·reader floor·백업 호환성. R3 migration 추가나 수동 DB 수정은 현재 필요 근거 없음 |
| release installer/lock/image builder | 정확한 current와 compatible off 후보. 이전 R2 a1/a2·옛 attestation/서비스 ID 재사용 금지. 단순 docs HEAD를 이미지 후보로 삼지 않음 |
| KIS credential slot/영속 state/anchors | 기존 generation·domain 재사용, REST ledger와 WS 예산 분리·보존. cross-host 독점 미확인 명시. 키/토큰/쿠키 조회·복사 없음 |
| 달력/시간창/EOD·onboarding | 동일 source lineage, 중복 calendar 호출 방지, 다음날 proof 담당·cutoff, 29종목 admission의 REST 예산과 R2 입력 수집 시간 조율 |
| R2 3거래일 관찰 | R3 적용으로 입력·추천 실행·릴리스가 바뀌면 재시작 조건 대조. 날짜와 완료 여부는 R2 담당이 판정 |

수행한 확인은 Git 소스/문서 읽기, registry 지정 파일 SHA 대조, 공개 공식 문서 GET,
문서 링크·행 참조·차이 검토와 `git diff --check`다. 소스·fixture 수정, 컴파일·테스트·CI dispatch,
sudo/DB/Docker/Compose/systemd/서비스/운영 재조회, secret env/session cookie/provider 인증,
Approval 발급·시장 소켓·setup script·추가 agent/heartbeat 실행은 모두 **미실행**이다.
구조 변경이 없어 PUML/PNG도 수정·렌더하지 않는다.

확인하지 못한 사실: 현재 운영 grant/entitlement/state/anchor·mode·health·예산 잔량,
실제 READY/admission 수와 정상 Owner 세션, 오늘/다음날 유효 calendar/window,
현행 공식 quota·전역 동일키 점유, 실제 ACK/47필드 수신/해제/DOM·latency·장 경계·동일출처 EOD,
현재 source에 대한 새 통합·설치 인수 및 D1의 실행 재현. 이유는 첫 실행의 문서 조사 범위와
공개 공지 접근 한계다. R3-01/06은 DOING, R3-02~05/07과 LIVE_ACCEPTED/FINAL_ACCEPTED는 미완료다.


## 8. D1 bounded 소스 후속 — 2026-10-21

기준은 최초 대조 커밋 `d805de1687619367642dc69e94dd7fb84443bb66`다. 부모가 D1을
독립 확인한 뒤 지정한 두 소스 파일만 수정했다. §1–7의 조사 당시 미실행/UNKNOWN은 역사
기록으로 보존하며, 이 절은 후속 소스 변경과 아직 실행하지 못한 회귀를 구분한다.

### 변경과 회귀 관측 경계

- `apps/web/app/(authenticated)/stock-beta/[instrument]/page.tsx`:63–68에서 기존
  `matchReadyIntradayQuoteMembership`의 선택 `instrument_id` + 신호 `generation` + READY
  결과만 `streamMemberships`의 단일 배열로 전달한다. 결과가 null이면 빈 배열이다.
  matcher·provider·controller·API·board·REST/off/Member 정책·패키지는 수정하지 않았다.
- `apps/web/tests/stock-beta-intraday-page-seam.test.tsx`:18–43의 관측 wrapper는
  실제 provider module을 `importOriginal`로 불러와 props를 기록하고 원래 provider를 렌더한다.
  page, detail/workspace, matcher 및 provider 내부는 실제 구현을 사용한다. SSR은 컴포넌트
  실행 수단일 뿐, 수요 배열의 판정은 HTML 문자열이 아닌 provider 호출 인자의 전체 배열이다.
  이는 browser effect/lease HTTP/SSE 실행 시험이 아니며 시장 연결을 만들지 않는다.
- 신규 6개 사례(:136–241): READY 두 종목 중 첫째/둘째 각각 선택한 단일 membership,
  선택 generation 불일치·선택 BACKFILLING(다른 READY 존재)에서 빈 배열,
  board의 READY 두 개 유지, 명시적 off에서 두 provider 비활성이다.
- 기존 Member 사례는 product API 구성과 provider 호출이 모두 없음을 추가 확인한다.
  기존 default-off 및 undefined/rest transport 사례도 provider 비활성 전달을 확인하도록
  보강했다. off의 detail membership 조회는 0, board의 기존 서버 membership 조회는 1로
  유지한다. board 서버 조회는 장중 lease 수요 호출이 아니다.

### 실제 수행 결과와 미실행

| 확인 | 결과 |
| --- | --- |
| `apps/web/package.json`, `vitest.config.ts`, 하위 AGENTS 및 관련 소스 읽기 | Vitest 4.1.10 선언, node 환경/기존 include·mock reset 설정 확인. 실행 도구 버전 확인은 아님 |
| Node `createRequire`를 `apps/web/package.json` 기준으로 실행해 `vitest/package.json`, `next/package.json`, `react/package.json`, `typescript/package.json` resolve | 네 패키지 모두 `MODULE_NOT_FOUND`. 확인 스크립트 자체 exit0(예외를 분류해 출력) |
| `node_modules`, `apps/web/node_modules`, `apps/web/node_modules/next/dist/docs` 존재 확인 | 모두 false. Next 로컬 가이드도 읽을 수 없음. 새 Next API를 도입하지 않고 기존 matcher 결과 대입만 변경 |
| 로컬 `command -v vitest` | 미발견. Node와 pnpm 경로는 존재하지만 pnpm 설치/fetch/exec를 대체 실행하지 않음 |
| 새 회귀 / 기존 page-seam suite / typecheck / formatter | **NOT_RUN** — 프로젝트 의존성 부재. 성공·실패 test count 또는 red→green 실행 증거 없음 |
| 변경 차이·전달 경로 수동 검토, `git diff --check`, R2 절/checkbox 불변 대조 | PASS. 소스 정적 검토이며 테스트 통과나 브라우저 검증으로 대체하지 않음 |

원래 구현에서 singleton/빈 배열 기대가 전체 두 membership과 충돌하는 것은 소스 수준으로
확인했다. 실행된 실패는 없으므로 RED라고 기록하지 않는다. 최초 대조 `d805de1`의 route와
이번 회귀를 결합하면 수정 전 검증 대상으로 재현할 수 있다. 패키지 설치·업데이트·network
fetch·다른 worktree 의존성 변경/빌림·전체 빌드/테스트·CI·운영 조회/연결은 수행하지 않았다.

**최소 후속 검증:** 의존성이 이미 준비된 허용된 격리 환경에서 프로젝트 로컬 Vitest의
`--help`를 먼저 읽어 단일 worker·파일 병렬화 해제 옵션을 확인한다. `nice -n 10`으로
`tests/stock-beta-intraday-page-seam.test.tsx` 한 suite만 순차 실행한다. 옵션은 이번에 도움말을
읽지 못했으므로 확정된 실행 명령으로 적지 않는다. 신규 사례만 수정 전 route에서 실패함을
확인한 뒤 같은 회귀를 수정 후 route에서 실행하고, 기존 사례를 포함한 suite 결과를 남긴다.
추가 의존성 설치나 실행 환경 변경은 부모가 다음 범위로 조율한다. 이 변경은 아직
`SOURCE_LOCAL_ACCEPTED`·배포·실제 수신·Owner 인수가 아니다.

### 인계 조회와 남은 운영 범위

이번 읽기 전용 registry 조회 시 pointer는 안내 메시지 이후 다시 전진해
`target/r2-daily-beta/wp-8/wave137/r3-detail-demand-followup-handoff.json`이었다.
직접 SHA-256과 registry `boundedHandoffSha256`은
`e77555470e63caaf2509ee8aaabad3d2a0dd6f60589084b56ae886e6fac5f263`로 일치했다.
기록 시각은 `2026-10-21T04:27:19.263146+00:00`; predecessor는 안내된
`r3-parallel-owner-decision-handoff.json`/`46e635a9024e3afefd0261107e7645e637015fd752a66cd6b6a2e6a76d72ea45`다.
§2의 최초 `bd5be4eb…` closure 조회를 바꾸지 않는다. 새 pointer는 후속 위임 기록이며
운영 관측이 아니다. R2 status/후보fccb·배포0·인증 차단·`lastObservedAt` 범위는 그대로다.
registry·부모 branch·R2 ROADMAP 절과 체크박스를 수정하지 않았다.

R3-01은 D1 소스 보정/회귀 작성까지 진행했으며 집중 검증 및 실제 설치 전제는 미완료다.
R3-02~05/07의 실제 수신·화면·30종목·장 경계·최종 인수와 R3-06 quota/동일키 점유 공백은
그대로다. §5의 첫 소스 단계는 **집중 검증과 부모 통합**이 다음 작업이다. 이후 운영 packet의
대상·변경·복구·예산·실행 시간을 조율한 다음에만 1종목→인증 수명주기→30종목으로 진행한다.
이번 값 범위 변경은 구조 edge를 바꾸지 않아 PUML/PNG 수정·렌더는 없다.

확인하지 못한 사실: 새 회귀와 기존 suite의 실제 실행 결과, 로컬 도구 도움말/Next 가이드,
수정 전 실패→수정 후 성공. 이유는 의존성 부재다. §7의 운영 미확인 항목도 모두 유지한다.


## 9. D1 집중 검증 — 고정 의존성 준비와 실제 red→green

2026-10-21 부모의 후속 범위 확대에 따라 **이 R3 worktree 내부** npm lockfile 의존성 준비와
해당 page-seam suite만 실행했다. 대상 소스는 `7b7f219720f0e5280b96d319a106adc33085a090`,
수정 전 route는 `d805de1687619367642dc69e94dd7fb84443bb66`다. §8의 NOT_RUN은 당시
기록으로 보존한다. 이번에는 추가 소스 보정이 없고 검증 문서만 커밋한다.

### 도구와 설치

- Node `v24.13.1`, npm `11.8.0`; root 요구 `>=24 <25` 충족. 로컬 `npm ci --help`에서
  workspace/root 포함·ignore-scripts·audit/fund 옵션을 확인했다. pnpm은 사용하지 않았다.
- 모든 설치/테스트는 순차, `nice -n 10`, `NODE_OPTIONS=--max-old-space-size=2048`로 실행했다.
  증거와 npm 캐시는 ignored `target/r3-d1-validation/`에 보존했다. manifest/lock 변경,
  전역 설정·다른 worktree·브라우저 다운로드·lifecycle script 실행은 없다.
- 최초 시도는 `--userconfig=/dev/null --globalconfig=/dev/null`의 동일 설정 경로 때문에
  npm 설정 로딩 전 `double-loading config`로 exit1(0.049초). 원본 로그/결과를 남겼다.
  이는 패키지 충돌/다운로드 실패가 아니며, 서로 다른 worktree 내부 빈 설정 파일로만 수정했다.
- 두 번째 명령은 아래와 같으며 exit0(6.610초), **302 packages added**다. `E`는 실행 당시
  `/data/worktrees/3puw275b/r3-parallel-20261021/target/r3-d1-validation` 절대경로다.
  npm이 알린 새 버전 업데이트는 실행하지 않았다. lock의 다운로드 host는 registry.npmjs.org다.

```sh
NODE_OPTIONS=--max-old-space-size=2048 nice -n 10 npm ci \
  --workspace=@lagrange/web --include-workspace-root --include=dev \
  --ignore-scripts --no-audit --no-fund --cache="$E/npm-cache" \
  --userconfig="$E/npm-user-empty.conf" --globalconfig="$E/npm-global-empty.conf" \
  --fetch-retries=0
```

설치된 Vitest `4.1.10`의 실제 `--help`에서 `--maxWorkers`와 `--no-file-parallelism`,
`--testNamePattern`, JSON reporter/outputFile 옵션을 확인했다. Next `16.3.0`, React `19.2.8`.
로컬 Next `dist/docs/01-app/`의 server-and-client-components, page API, testing/vitest 가이드를
읽었다. async Server Component에 대한 일반 Vitest 지원 한계를 명시한 가이드에 따라 이번
검사는 기존 `renderAsyncPage`로 page 결과와 provider props를 관측한 집중 회귀이며,
Next RSC/hydration·브라우저·네트워크 종단 검증으로 확대하지 않는다.

### 실행과 실패 원인

두 명령 모두 `apps/web`에서 실행했다. 아래 `R`은 worktree 절대경로, `E`는 위 증거 경로다.

```sh
NODE_OPTIONS=--max-old-space-size=2048 nice -n 10 node "$R/node_modules/vitest/vitest.mjs" run \
  tests/stock-beta-intraday-page-seam.test.tsx --maxWorkers=1 --no-file-parallelism \
  '--testNamePattern=passes only selected READY membership|passes no detail demand' \
  --reporter=json --outputFile="$E/red.json"
NODE_OPTIONS=--max-old-space-size=2048 nice -n 10 node "$R/node_modules/vitest/vitest.mjs" run \
  tests/stock-beta-intraday-page-seam.test.tsx --maxWorkers=1 --no-file-parallelism \
  --reporter=json --outputFile="$E/green.json"
```

| 실행 | 실제 exit / 결과 | 판정 |
| --- | --- | --- |
| RED: d805de1 원본 route + 7b7f219 회귀 | exit1, 4 failed / 0 passed / 10 skipped, 총14; 1.216초 | 첫째/둘째 선택 모두 membership 2개가 전달돼 singleton 기대 실패. generation 불일치/미READY도 전체2개를 전달해 빈 배열 기대 실패. 네 실패 모두 provider 호출1회의 인자 불일치 |
| GREEN: 7b7f219 route 복원 + 동일 회귀, suite 전체 | exit0, 14 passed / 0 failed / 0 skipped; 1.166초 | 신규6개 및 기존8개 통과. board2종목, off 비활성, Member API/provider 호출없음 포함 |

Python 실행 wrapper는 RED 동안만 원본 blob을 route에 쓰고 `finally`에서 수정 blob을
byte 단위로 복원했다. 결과에 `routeRestored:true`를 남기고 GREEN 전·후 재대조했다.
원본 SHA-256은 `3b28391a5fe08563301dc86f1ea811972949820ca2e16aa656572e4b7b4be859`,
수정본은 `8f28a049276f273d25a35a330e9d5c7b85fc6c751cae8c9d9f85fc249e0217a0`다.
실패의 diff에서 두 membership 전달과 호출1회를 직접 확인했으며 예상 실패를 대신 기록한 것이 아니다.
기존 커밋/원본 로그를 수정하지 않았다. source/test는 7b7f219와 byte 동일하다.

### 무결성, 증거 및 남은 범위

설치 전 모든 tracked package.json과 root lock의 SHA-256을 저장하고 종료 후 모두 일치함을 확인했다.

| 파일 | 전후 동일 SHA-256 |
| --- | --- |
| `apps/api-server/package.json` | `b01a325c7a6a6d38d7149623ab107a59e229d47870f88a7528e5bcdd87681662` |
| `apps/web/package.json` | `556dd43c173351d59b56f231a7d3393251680f309958d62d2cff11510d3fe0bf` |
| `data-pipelines/kind-capture/package.json` | `49fa719dac556504e6789aaef772765fd41a961152fe364c2389e008edee19fc` |
| `package-lock.json` | `1932c810e7165807f24c9aa3005063c9e8dde3292ac27d8ede9c3735da5e9e76` |
| `package.json` | `171bf43c0afc36e500432e784f103a6881382496b3244087c5c9cbdb3bf9c338` |

원본 로그·결과는 Git에 넣지 않는다. 아래 경로는 `target/r3-d1-validation/` 기준이다.

| 증거 | SHA-256 |
| --- | --- |
| `npm-ci.log` | `250e369d730a46c4d41df37449a5c58c169b4b1d2ab546951a8bed0285805a95` |
| `npm-ci-a2.log` | `addfb56898ea0838e3b43436156f7fb704548c4189223a0ad4f8fabd300e8862` |
| `red.json` | `754ff3375fc3cbe711ace598819b51bd54c824f9c690642777c8d2cb3b0ddda8` |
| `red-result.json` | `052e95ef235818e70f38c37df268b52ce95063cb1bab39eb52924d191e25f0c6` |
| `green.json` | `d818cd860e76b4331f43c947c4440d80e63d718186e7468cbb284ef567621b69` |
| `green-result.json` | `beef8a34360425e18a4f4c33af4ffa04b0cd3de4055f7c453b7e078d78b6e996` |
| `final-integrity.json` | `ab59cd4d4f26826d1652ac065e82e46b80f3dbc71352fbea6792dbfe230ebc12` |

`git diff --check`, R2 ROADMAP 전체 절/체크박스 불변, source/test 무변경, ignored 증거·캐시의
비추적을 확인했다. 구조 변화가 없어 diagrams는 그대로다. 추가 테스트/formatter/full build/
전역 typecheck/Cargo/CI·merge/push·운영 조회/수정·lease HTTP/SSE/provider/시장 연결은 미실행이다.

D1의 한정 소스 회귀 검증은 완료했고 다음은 부모 통합 검토다. R3-01 설치 전제 및 R3-02~07의
운영 수신/인수·quota 공백은 그대로다. 1종목→인증 수명주기→30종목 운영 packet의 대상·변경·
복구·예산·시각은 다음 조율 범위이며 이번 의존성 설치는 R2 인증 복구나 운영 인수 증거가 아니다.
확인하지 못한 사실: 전체 타입/빌드·브라우저/RSC·실제 lease/수신과 §7의 운영 항목(이번 범위 밖).
지정된 D1 집중 회귀에서 미해결 실패는 **없음**이다.


## 10. 부모 D1 인수 이후 — §5 실행 순서의 공식 도구 바인딩

2026-10-21 부모가 `7b7f219` 소스/회귀와 `4f50f3f` 검증 문서·원본 결과를 직접 대조해
**D1 SOURCE_FIX_AND_FOCUSED_REGRESSION_ACCEPTED**로 인수했다. 전체 R3 SOURCE_LOCAL/
LIVE/FINAL 인수나 merge·배포가 아니다. 이 절은 §5를 구체화하며 새 실행 승인을 요청하지 않는다.
아래 소스 행은 `4f50f3f` 기준이고 모든 명령은 **미실행 실행안**이다. 운영값은 모두 UNKNOWN이다.

### A. 통합→새 immutable 후보→동일 revision 설치/실행 (§5.1·4)

1. 부모가 독립 R3의 D1 `7b7f219` 및 검증 `4f50f3f`를 통합 검토한다. R2 frozen `fccb683`,
   그 build worktree·manifest에는 합치거나 덮어쓰지 않는다. 이후 별도 clean R3 source 후보를
   확정하며, 이미 인수된 R2 변경과 reader 호환성을 포함할 기준선은 부모가 당시 상태로 선정한다.
   **후보 SHA는 아직 미정**; 현재 문서 HEAD를 검증된 release 후보로 간주하지 않는다.
2. 새 후보 확정 후 `.github/workflows/ci.yml`:3–8의 PR/main 또는 workflow_dispatch 경로로
   **그 후보** CI를 진행한다. checkout(:27–29), Web job(:133–149) 등 전체 필수 결과가
   정확한 후보 SHA를 검사했는지 부모가 인수한다. R2 CI 성공이나 D1 14통과로 대체하지 않는다.
3. 그 SHA의 clean build root에서 `LAGRANGE_CODE_COMMIT=<candidate>`를 바인딩해
   `scripts/ops/build-production-images.sh --plan/--preflight/--apply --env-file <protected-path>
   --manifest-file <new-absolute-path>` 순서다. mode는 각각 별도 호출이다.
   builder :128–148은 HEAD/clean 일치를, :159–208은 12이미지 ID/OCI revision 및 새 V2 manifest를
   강제한다. :323–365는 1서비스씩 B1–B4 순차 처리와 각 batch gate다. 기존 jobs2/parallel1,
   저우선순위 background·batch별 자원/health 조건은 유지하며 R2 빌드와 직렬 조율한다.
4. CI/이미지 인수 뒤 `deploy-production-release.sh --apply --commit <candidate>
   --env-source <protected-path> --release-manifest <new-manifest>`로 설치한다(:32–57,218–299).
   installer는 exact Git archive·env commit·manifest를 묶어 current를 전환할 뿐 runtime 갱신이 아니다.
   quotes-off와 activation env를 같은 설치 commit에 덮어쓰지 않는다. 별도 activation commit이
   필요하면 그 commit도 자체 CI·12-image manifest를 거친다(runbook market-stream :48–53,177–200).
5. 실제 실행 변경은 아래 전제/복구가 고정된 창에서만 설치본 `compose-release.sh --scope release
   --refresh-market-stream --refresh-from-commit <actual-running-source> --plan`, `--preflight`,
   마지막 `--apply` 순서로 진행한다. :567–648은 running source manifest 검증 후 target current의
   image_id로 API→Web→runner를 순차 교체/재검사한다. `--daily-from-commit`은 필요할 때 실제
   기존 daily 소유 revision에 바인딩한다(:101–106,403–410); WS source와 무조건 같다고 하지 않는다.
   현재와 실행본이 같을 때만 refresh-from 생략 가능. 두 기존 rollout acknowledgement는
   runbook :190–194의 값 그대로 사용하며 새 Owner 승인으로 바꾸지 않는다.

### B. 공동창 전 최소 관측/자료와 기존 진입점 (§5.2·3)

`<current>`, `<running>`, `<off>`, `<date>`, 경로·UUID·grant/slot/generation·잔여 예산 모두
**UNKNOWN**이다. 역사 e639/schema62는 보존할 호환 기준이지 현재 재관측값이 아니다.
root 실행 인증의 실제 사용 가능 근거와 원래 작업/서비스 식별이 먼저 필요하다. 이번에는
인증 확인도 하지 않았다. UNKNOWN은 부모의 한정 관측 항목이며 Owner 재승인/정보 질문이 아니다.

| 필요한 실제 자료 | 기존 공식 command / entrypoint 및 정확한 제한 |
| --- | --- |
| installed commit/env/manifest와 reader floor | `deploy-production-release.sh --check --commit <current>`는 root read-only 검증(:32–57,242–299). 내부 `owner-beta-daily-release.py reader-check --install-root <root> --target-release <release> --mode <actual-mode>`는 reserve=false(:529–549,623–638). 실제 running image는 refresh plan/preflight의 `verify_refresh_source`(:567–590)로 별도 확인; current-link만으로 실행 revision을 판정하지 않음 |
| 실제 DB schema/migration 이력 | reader marker 요구는 `[60,61,62]`(`owner-beta-daily-release.py`:26)이나 **실제 DB 증명은 아님**. 이번 대조에서 단독 schema 조회용 공식 CLI는 확인하지 못했다. 부모의 기존 schema62 설치/백업 관측 경로로 현재 migration/checksum 자료를 재바인딩할 정확한 호출은 미정; integration-validation이나 migration 실행으로 조회를 대신하지 않음 |
| R2 daily·원래 작업/프로세스·shared lock | installed `compose-release.sh --daily-disable --preflight [--daily-from-commit <actual-owner>]`가 기존 cohort/DB를 검사하되 disable은 하지 않음(`owner-beta-daily-operations.py`:718–795). 실제 disable/drain은 나중에 `--apply`. refresh apply는 release lock(:379–400) 후 guard-processes/guard(:703–709), quiescence(:672–692)를 수행. preflight 성공이 OFF/drain/lock 독점 성공은 아니며 원래 연구·백업·타이머 상태/복귀 책임은 부모의 기존 인계·관측 자료가 필요 |
| 당일 KIS calendar source/batch/hash와 operational window | `install-intraday-session-window.py --check --root <protected-window-root> --date <actual-KST-date>`(:713–768). calendar는 `compose-release.sh --scope release --bootstrap-intraday-calendar --reuse-existing-source --preflight`(:651–687)로 중단 전제를 검사하지만 DB proof 유효성까지 증명하지 않음. 이후 apply는 publication **쓰기**이며 기존 source 검증/재사용; 창에서만 실행. 실제 day proof 결합은 runtime `resolve_day_with_calendar`(:312–386). XKRX 파일 check는 KIS lineage 대체가 아님 |
| WS state/domain/anchor와 slot/generation | installed `provision-owner-market-stream.py --check --expected-commit <off>`(:124–157,170–208,343–399). **quotes-off·runner 정지** 전제가 있고 network-none pinned initializer를 실행하므로 일반 online 조회가 아님. 기존 상태가 있는 경우 initialize-new 금지; state/예산/불명 기록 보존. 실제 남은 예산의 출력용 단독 CLI는 확인하지 못해 관측 경로 확정 필요 |
| Owner/ACTIVE entitlement/grant pin | `install-owner-market-stream-grant.sh --plan`은 DB 조회가 아니다. `--apply --expected-commit <activation> --approval-input <reviewed-file> --approval-sha256 <hash>`는 grant **쓰기**이며 창에서만 필요 시 수행(`market-stream-grant.py`:399–440,1116–1148). 기존 runtime `market_stream.rs`:1562–1626이 grant·slot/generation·rights를 결합; 단독 read-only grant CLI는 확인되지 않아 기존 보호 관측 자료 필요 |
| 정상 Owner READY/generation·선택 신호·현재 수요 | 정상 로그인 후 기존 `/api/v1/research/owner-beta/equity-universe-v2/memberships` GET 및 signal-detail GET (`http/mod.rs`:342–343,388–391; `owner_equity_v2.rs`:267–297). 보호 UI/typed metadata로 확인하며 쿠키 export·수동 READY·추가 QA lease 없음. browser가 만든 실제 lease/SSE는 :362–373 기존 경로이고 조회를 위해 별도 lease를 만들지 않음 |
| 한 연결/30등록·G2 예외 | ADR-0009:14–36 및 §5 예산. 실제 quota·타기기 동일키 점유·잔여량은 UNKNOWN. 예외는 H0STCNT0/정상 READY/Owner 제한 pilot 허용이지 전역 독점·G2 VERIFIED가 아님. 충돌/거절/불명 시 fail closed |

### C. 첫 실행·단계 전환·실패 기준 (§5.5–9)

- 첫 활성 실행 범위는 **한 정상 Owner 페이지의 1종목 pilot 및 종료 관측**이다. 실제 선택
  READY/generation·신호가 맞는 D1 detail을 쓴다. 신호가 없어 board를 쓰려면 실제 총수요가1임을
  먼저 증명한다. 다른 탭/lease 존재가 불명이면 단일종목 성공으로 진행하지 않는다.
- 성공은 같은 epoch의 ACK→유효47필드→DB/SSE/DOM 및 최초 표시 뒤 두 fresh receipt를 연결한
  §5.6의 증거다. status/heartbeat만 증가, quiet/stale/미ACK는 미완료; 횟수를 채우려고 재접속하지 않는다.
- 종료 후 native hide/restore·offline/online·새로고침·logout을 §5.7 기준으로 확인한 뒤에만
  나머지29 정상 admission→30 ACK/표시·1탭/10탭 합집합을 §5.8로 진행한다. 각 단계는 첫 창의
  자동 연장이 아니다. 기존 잔여 예산·창·복구 여유가 안 맞으면 그 증거에서 멈춘다.
- 마지막 수요0의 whole-socket close≤5초는 계약상 허용이다. 개별 unsubscribe를 보내지 않았다면
  ACK는 NOT_OBSERVED로 보존한다. 후속 장 종료·다음날 proof 공급/부재 시 closed, 같은 source EOD,
  p95≤2초는 별도 §5.9 인수다. 실제 날짜·관측 횟수/창 총시간은 부모 packet에서 확정할 미정값이다.
- typed 권리/proof 실패·ACK/연결 불명·혼합 image·예산 소진·종료 증명 실패 시 다음 단계 금지.
  기존 backoff/ledger만 적용하고 발급/명령 불명을 재시도로 해소하거나 counter를 초기화하지 않는다.

### D. 호환 quotes-off 복구와 다음 한 가지 조율

`<off>`는 **미정**이다. schema62 및 reader floor/history를 보존하는 후보를 먼저 검증한다.
`deploy-production-release.sh --rollback --commit <off>`는 installed manifest/reader 검사 뒤
current만 전환한다(:43–57,242–299). a68의 나이나 quotes-off라는 이름은 호환 증거가 아니다.
공식 rollback 뒤 off 설치본의 `--refresh-market-stream --refresh-from-commit <activation>`
plan/preflight/apply가 정상 running source를 확인할 수 있을 때만 runtime을 교체한다.
producer를 먼저 정지했거나 일부 교체로 mixed 상태가 되면 :567–648 guard가 거부한다.
그 경우 exact 서비스 복구 명령/대상은 현재 증거로 **미정**이며 부모가 기존 서비스 절차로
별도 바인딩하기 전 활성화하지 않는다. guard 우회·DB down migration·state/grant 삭제는 없다.

**부모의 다음 한 가지 행동:** R2 빌드/배포/백업과 겹치지 않는 **공동 사전관측 창 하나**를
배정하고, root 인증 사용 가능 근거·실제 source/target·호환 off 및 정지/mixed 복구 책임을
같은 packet에 묶는다. 이 창은 B의 미확인값을 채우는 단계이며 pilot 자동 개시 허가가 아니다.
이번에는 소스/기존 문서 읽기·문서 차이 검증만 했다. 명령 실행·추가 공식문서 네트워크 조회·
14테스트 반복·CI/build/push/merge·운영 조회/DB/lease/provider/배포 및 agent/heartbeat는 없다.
미해결/확인 못한 항목은 B의 UNKNOWN, 새 후보/off SHA, exact schema/grant/예산 관측 호출과
stopped/mixed 복구 바인딩이다. 이 공백을 새 스크립트나 승인 대기로 바꾸지 않고 부모에게 인계한다.


## 11. 독립 R3 제안 후보와 OFFLINE hosted-CI 입력 패킷

2026-10-21 부모 지시에 따라 제안 후보를 **`1ec5f1626a2db41eb0854be188292f6ad0534449`**로
고정했다. 이 절을 추가한 문서 HEAD는 후보가 아니다. §10.A의 미정 source 후보를 이 제안으로
구체화했으며 off/activation runtime 후보·이미지·설치 인수는 여전히 미정이다.

- `git merge-base --is-ancestor fccb683 1ec5f16` exit0. 사이6커밋을 패킷에 기록했다.
  tracked 실행 소스 차이는 D1 route와 page-seam 두 파일뿐이며 `7b7f219`의 blob과 동일하다.
- 문서 차이5개는 ROADMAP, R3 handoff/review, R2 G4 review, R2 daily-input plan이다.
  **fccb 대비 R2 절 전체 byte 동일은 아니다**: R3 분기 전 `1c53a1a`/`73f2593`의 역사/상태 문구를
  상속했다. fccb 대비 R2 task/checkbox는 동일하고 R3 분기점73f2593 대비 R2 절 전체가 동일하다.
  diagrams, Cargo/npm lock과 모든 package.json은 fccb와 byte 동일하다. 예상 밖 실행 소스 변경 없음.
- 후보 `.github/workflows/ci.yml`을 기존 설치 YAML2.9.0으로 파싱해 job/step별 행과 run/if/env/needs,
  checkout action SHA·기본 source ref 의미, npm script 확장 및 호출 검사 소스의 해시를 기록했다.
  **8 jobs / required 선행7 / checkout7 / run-step64**. checkout action SHA는 source SHA가 아니며
  미래 hosted checkout이 정확히1ec5f16을 검증해야 한다. 현재 hosted run/checkout은 null이다.
- 선택 회귀는 후보의 실제4파일에서 **Rust57/Python10**을 재추출했다. Cargo workspace 명령과
  build-production-images-self-test→build-disabled-daily-self-test 호출을 연결했다. 과거 R2 packet은
  파일 선택 참고일 뿐 정의를 복사하지 않았고, reconstruction의 실제 `linux_tests::` 한정명을 보존했다.
  직접 참조 검사 파일38개·근거 blob187개·Python AST inventory463정의를 보유한다.
  이 inventory는 동적 호출 전체나 runtime 실행 개수가 아니다. 필수 정의와 실제 성공을 구분한다.
- 기존 npm 설치/원본·수정 route/red/green 등 **13개 원본 증거**의 bytes/hash를 다시 대조했다.
  후보 route/test는 인수된7b7f219와 동일, 원본 route 복원과 red4fail/10skip→green14pass/0skip·exit0,
  lock/package 원래 해시를 연결했다. 이번에는 그 테스트·설치를 반복하지 않았다.

패킷은 ignored `target/r3-ci-inputs/1ec5f16/ci-inputs.json`:
**806,091 bytes**, SHA-256 `205aa1be909028d860288eb5869cf08f5b2f55f4ee019b7b0ebc6c0bb50bb893`.
같은 폴더의 `extract.py`/`extract-workflow.mjs`·README·`blobs/<sha256>`가 추출 근거이며,
`nice -n 10 python3 target/r3-ci-inputs/1ec5f16/extract.py`로 재현한다. 추출기 해시도 패킷 안에 있다.
`verification.json`은 재추출 byte 동일·187blob/13receipt 해시 대조 PASS(exit0)를 기록한다.
첫 추출의 npm manifest 변수명 오류 `NameError`(exit1)는 a1 소스/실패 기록으로 보존하고
해당 변수만 바로잡았다. 제품 코드·CI 정의 변경은 없었다. 패킷·원본 로그·캐시는 Git 비추적이다.

다음은 부모의 패킷 검토 및 **고정 후보1ec5f16의 실제 CI 순서 결정**이다. R2 frozenfccb/build root/
manifest/registry를 변경하지 않았다. 전달받은 a3 실행·OwnerV2 복원 책임은 부모에게 있으며
이 오프라인 패킷은 새로운 운영 관측이나 공동창 개시 근거가 아니다. GitHub 조회/dispatch/poll/
push·CI·이미지·build·DB/provider/lease·배포·운영 인수는 **미실행**이다.
미확인: hosted 실행/정확 checkout/결과, 새 이미지·manifest, 설치 revision과 §10 runtime gap.
정적 dependency inventory의 동적 완전성·branch protection 설정·전체 test 수는 주장하지 않는다.


## 12. 단일 hosted CI — 실행 중 인계, Web 서식 실패

2026-10-21 부모가 hosted CI 한 번을 승인했다. 운영 호스트의 a3와 별개인 GitHub runner만
사용했으며 공유 운영 작업은 열지 않았다. 후보는 계속 **1ec5f1626a2db41eb0854be188292f6ad0534449**다.
문서 HEAD1383489를 게시하지 않았다. origin은 `sihioov/Lagrange`, 정상 `gh auth status` exit0이며
인증 출력/토큰은 저장·출력하지 않았다. 후보의 ci/research-smoke push trigger가 main 전용임을 확인했다.

- 전용 ref `r3-ci-1ec5f16-20261021`의 부재를 `git ls-remote --exit-code` exit2로 확인했다.
  `git push origin 1ec5f1626a2db41eb0854be188292f6ad0534449:refs/heads/r3-ci-1ec5f16-20261021`
  한 번이 exit0, 후속 원격 ref도 정확한 SHA였다. force/main/PR/merge/rebase는 없다.
- 게시 전/후 동일 ref·candidate의 ci.yml run 조회는 각각0이었다. 의도 기록 뒤
  `gh workflow run ci.yml --ref r3-ci-1ec5f16-20261021 --repo sihioov/Lagrange`를 **한 번** 실행,
  05:26:05.726171UTC exit0. 재시도·취소·rerun은 없다.
- [run39549529746](https://github.com/sihioov/Lagrange/actions/runs/39549529746), attempt1,
  event workflow_dispatch, head SHA/ref/path 모두 고정값과 일치한다. 30초 이상 간격으로 최대6회
  관측했다. 마지막 **05:33:15.563778UTC in_progress/conclusion null**; 이후 상태는 주장하지 않는다.

| 마지막 job 상태 | 확인 범위 |
| --- | --- |
| clippy / format / python-tests SUCCESS | 완료된 각각의 전체 로그와 후보 checkout 회수 |
| web FAILURE | 전체82,077bytes 로그, SHA8db4fb09618e4a04ba7b56101abd8360bb2df8a1804f162d17cf7f6ce997407d |
| policy / workspace-tests / postgres-integration-validation IN_PROGRESS | 최종 결과·전체 로그 아직 미회수 |
| required | 아직 job 목록에 생성되지 않음; 최종 aggregate 미확인 |

Web은 npm ci·Chromium 준비·openapi check 뒤 `npm run lint --workspace @lagrange/web`에서 exit1.
`tests/stock-beta-intraday-page-seam.test.tsx`의 `importOriginal`(:33–35)와 첫 `it.each`(:136–163)
서식이 Biome 출력과 달랐다. 294파일 검사에서1error/6warnings/69infos이며 해당 서식 오류가 실패다.
후속 typecheck·npm test·build·candidate E2E는 API step 결과 **skipped**다. 따라서 hosted의
focused14는 실행되지 않았고 기존 로컬 green14를 이 run의 성공으로 대체하지 않는다.
이번 범위에서 제품/테스트 소스 보정·formatter 실행·새 후보 생성·추가 dispatch는 하지 않았다.

원본 증거 디렉터리: ignored `target/r3-hosted-ci/1ec5f16/`. 아래 mutation 영수증은 선행 intent와
실제 결과를 구분하며 원본을 덮어쓰지 않았다.

| 파일(위 디렉터리 기준) | bytes | SHA-256 |
| --- | ---: | --- |
| `publish-intent.json` | 262 | `0a4c4757c5bc44d305015dee43a68a4d13b1698e13fc070091d30e7b762999d4` |
| `publish-result.json` | 422 | `4ffca86daf9e2e051fa3157ef7be22ddce05b332f52dad814b47a8d7b03a37e1` |
| `dispatch-intent.json` | 416 | `e8b4eb619474cee607c6e4d456cbcf2a6a6bd6490e6844a02671f680b086c63a` |
| `dispatch-result.json` | 126 | `829aaa6bcd46f072898acce9c09a674a7f248fdeaccdb419400af756523856da` |
| `receipt-manifest.json` | 10069 | `7fa7922b2164eca68512ec838b198ddae5764e63720a898b5f6ffbf89a694090` |
| `recovery-partial.json` | 28382 | `e0d0d14c72a2e12e64180078bd5a7f8a93e107223a3ccfe3fcecbf4a33d2c4ed` |
| `handoff.json` | 2585 | `b4df8ce9dba25d505e36a92cff54f9d19de96f8a480eebf1b880229ae4f7adbc` |

R3 전용 `observe.py`/`recover.py`는 정확한 candidate/run/attempt/packet에 묶인 읽기 회수 코드다.
wave136 recovery는 소스 참고만 했으며 실행하지 않았다. 완료 job **4개 전체 로그/checkout4개**를
회수했고 원본 로그 SHA/bytes·job step 상태는 recovery-partial에 있다. 나머지 job 로그를
보지 않았으므로 Rust57/Python10의 이 run 검증 수는0(실패57/10이라는 뜻 아님), PG19 exit/no-skip도
미검증이다. 8로그/7checkout 최종 회수와 required 결론은 부모의 원래 attempt 추적에 남겼다.

**판정:** HOSTED_NOT_ACCEPTED — Web FAILURE와 필수 후속 검사 skip; 전체 run은 아직 실행 중.
다음은 부모가 원래 attempt의 terminal/남은 원본 증거를 회수하고, 서식만 고치는 별도 bounded
후속 범위와 새 후보의 CI 순서를 정하는 것이다. 원래 attempt를 취소/재실행하지 않는다.
사전 고정된6회 관측을 끝내고 `handoff.json`으로 인계한다. 로컬 테스트/빌드·R2 운영 조회·DB/
Docker/systemd/sudo·provider/lease·이미지/배포는 미실행이다. R2 절/체크박스와 source/diagrams/
lock/package는 이번 문서 변경 전후 동일하다. 후보/source·원본 packet/로그를 보존한다.
