# KIS 시장 시세 스트림: 실제 접속 전 준비 상태

작성·공개 자료 재확인: **2026-10-01 KST**. 기준 소스: `17d643e92bd4b5c257c9745cfc6ec188ef9f4d05`.
이 문서는 **WP-2의 읽기 전용 판단 자료**다. 승인키 발급, KIS 연결, 운영 상태 조회, 권한 부여, 설치 또는 활성화를 수행하지 않았다. 현재 판정은 **G1–G5 모두 실제 접속 인수 전 미완료**다. `SOURCE_LOCAL_ACCEPTED` 이력이나 아래 후보 문자열은 라이브 권한이 아니다. [2026-10-01 계획](../superpowers/plans/2026-10-01-kis-market-stream-completion.md)의 `:93-108`, `:218-234`와 기준 커밋의 [WS-1 계약](../superpowers/specs/2026-09-21-kis-market-stream-contract.md)의 `:1-17`, `:1156-1169`를 함께 적용한다. WP-1이 같은 계약을 편집 중일 수 있으므로 본 문서의 계약 인용은 모두 위 **기준 커밋의 `git show` 행 번호**다.

## 출처와 확인 수준

| 구분 | 정확한 출처·개정 | 10월 1일 확인 및 한계 |
| --- | --- | --- |
| 공식 공개 샘플 | KIS [`kis_devlp.yaml` L26–30](https://github.com/koreainvestment/open-trading-api/blob/b4e6249714418aa57833d1cbbbced39cbcc5b125/kis_devlp.yaml#L26-L30), [`kis_auth.py` L475–498, L513–534, L665–784](https://github.com/koreainvestment/open-trading-api/blob/b4e6249714418aa57833d1cbbbced39cbcc5b125/examples_user/kis_auth.py#L475-L498), [WS 예제 L19–20](https://github.com/koreainvestment/open-trading-api/blob/b4e6249714418aa57833d1cbbbced39cbcc5b125/examples_user/domestic_stock/domestic_stock_examples_ws.py#L19-L20). 고정 SHA `b4e6249714418aa57833d1cbbbced39cbcc5b125`, 커밋 2026-08-26. | 고정 개정 페이지를 다시 열어 HTTPS 운영 호스트, `ws://...:21000`, `Approval` 대소문자와 `/tryitout` 결합, 시장 구독·해제 `1`/`2`를 대조했다. [공식 커밋 목록](https://github.com/koreainvestment/open-trading-api/commits/main/)의 조회 화면은 최신으로 8월 26일을 보였지만 캐시 가능성이 있으므로 **오늘의 `main` HEAD를 새로 증명한 것은 아니다**. 샘플은 실행하지 않았다. |
| 공식 시장 채널 | KIS [H0STCNT0 문서](https://apiportal.koreainvestment.com/apiservice-apiservice?/tryitout/H0STCNT0), [속성 문서](https://apiportal.koreainvestment.com/api/apis/guide/property/714d1437-8f62-43db-a73c-cf509d3f6aa7), [9월 9일 변경 공지](https://apiportal.koreainvestment.com/community/10000000-0000-0011-0000-000000000001/post/26dfe350-eb72-48e5-8175-34eb27970f3e). 계약 `:21-28`, `:54-60`, `:157-173`. | **역사 증거:** 9월 21일 포털 필드 개정 `2026-09-11T16:31:27+09:00`, 9월 14일 발효 `MARKET_CLS_CODE` 포함 47필드. **오늘:** 포털 HTML 껍데기는 열렸으나 속성 JSON과 공지 본문은 공개 도구에서 열리지 않았다. 47필드의 오늘 재검증·실제 wire 수신은 아직 없다. 공식 샘플의 46필드 목록과 불일치하므로 샘플을 필드 권위로 삼지 않는다. |
| 공식 접속키 | KIS [Approval 문서](https://apiportal.koreainvestment.com/apiservice-apiservice?/oauth2/Approval), [속성 문서](https://apiportal.koreainvestment.com/api/apis/guide/property/5c87ba63-740a-4166-93ac-803510bb9c02); 계약 `:56-57`, `:79-115`. | **역사 증거:** 상세 개정 2023-06-27, 필드 개정 2024-12-13, 접속키 24시간. **오늘:** 문서의 HTML 껍데기만 열렸다. 후보의 정확한 요청 형태는 고정 공식 샘플과 역사 계약이 서로 맞지만, 포털 세부 계약의 오늘 재조회는 미완료다. |
| 공식 한도 | KIS [유량 공지](https://apiportal.koreainvestment.com/community/10000000-0000-0011-0000-000000000001/post/d0d1a83f-6f8d-4437-9700-6d26702fd989), [공지 목록](https://apiportal.koreainvestment.com/community/10000000-0000-0011-0000-000000000001/post/), [9월 16일 상세 후보](https://apiportal.koreainvestment.com/community/10000000-0000-0011-0000-000000000001/post/2b641ee8-b594-427a-9d11-ee6e764f0453); 계약 `:29-36`, `:58-61`. | **역사 증거:** 2026-04-20 개정 문구는 App Key당 WS 1세션, 여러 상품·채널 합산 41등록. 9월 21일 목록은 2026-09-16 17:43:45+09:00의 `[중요] API 유량제한 적용 안내`를 식별했다. **오늘:** 공지 HTML 본문은 빈 템플릿, 9월 16일 상세·공개 JSON은 접근 실패. 따라서 최신 수치·대상·시행일은 **미확인**이며 41을 현행 보장으로 쓰지 않는다. 샘플의 `open_map` 40개 검사는 종목별 구독 한도가 아니다(계약 `:29-36`, `:63-65`). |
| 공식 데이터 이용 | KIS [제휴 안내의 고객 범위와 시세 API 항목](https://apiportal.koreainvestment.com/provider); [ADR-0005](../decisions/0005-kis-personal-use-entitlement.md) `:3-23`, `:33-38`. | 오늘 공개 페이지를 재확인했다. 개인 본인 자산 이용과 제3자 제공을 구분하고, 제휴기관의 거래소 정보이용계약 문제를 별도로 적는다. 이 페이지 자체가 현재 배포의 WS/cache/SSE 범위나 무기한 보유를 인증하지는 않는다. |

## G1–G5: 확인된 사실, 충돌, 필요한 결정

| Gate | 현재 판단 | 코디네이터가 닫아야 하는 정확한 차이 |
| --- | --- | --- |
| **G1 네트워크** | 공식 샘플과 역사 계약이 `POST https://openapi.koreainvestment.com:9443/oauth2/Approval` 및 `ws://ops.koreainvestment.com:21000/tryitout`를 지지한다. `H0STCNT0`은 **소켓 안의 시장 TR**이며 포털의 `/tryitout/H0STCNT0` 문서 경로를 소켓 URL로 쓰지 않는다. `ws` 구간의 접속키와 시장 메시지는 TLS 없이 전송된다. 계약 `:79-96`. | Owner의 **이 정확한 신규 HTTPS 요청·평문 WS 업그레이드·시장 TR** 수락을 별도 기록하거나 공식적으로 문서화된 암호화 대안을 재검토한다. 기존 REST `POST /oauth2/tokenP`/GET 허용 목록은 새 WS 권한이 아니다. `wss`·포트·경로 변경, 프록시 우회, 계좌/주문 채널은 승인안에 넣지 않는다. |
| **G2 현행 용량** | 4월 개정의 1세션·41등록은 그 공지 시점의 수치일 뿐이다. 9월 16일 공지 본문과 적용 범위를 회수하지 못했다. 실제 App Key/계정의 다른 WS 클라이언트·등록 수는 공개 문서로 알 수 없다. | 공식 최신 공지 본문 또는 KIS의 현재 서면 확인으로 **이 슬롯에 적용되는 세션·등록 한도와 시행일**을 먼저 확보한다. 운영자가 비밀값 없이 동일 슬롯/계정의 다른 프로세스·호스트·앱·등록 채널 사용량을 대조하고, 기존 세션 종료와 잔여 용량 ≥30을 입증해야 한다. 30 미만이면 고정30 제품 목표를 Owner가 다시 결정해야 한다. 종목 순환, 추가 키, 조용한 축소, 추정 명령률은 해법이 아니다. 숫자형 명령률·재연결 한도·ACK 시간 등은 계약 `:73-77`에서도 미발견이다. |
| **G3 권리** | ADR-0005 `:13-23`은 **개인 단일 Owner의 승인된 읽기 전용 시세 이용**을 이미 채택했고 재확인을 요구하지 않는다. 다만 `:33-38`의 기존 허용 목록에 이 WS가 없으며, 계약 `:281-298`은 새로운 ephemeral latest cache·Owner 전용 SSE 바인딩을 G로 남겼다. | 기존 `data_entitlements`의 정확한 ID, reference `kis:owner-attestation:personal-single-user:2026-08-21`, 문서 SHA-256, 유효일, Owner ID에 **H0STCNT0 한 채널·최신값 cache·인증 Owner SSE만** 연결하는 좁은 결정/필요한 amendment를 검토한다. 보관 기간·Member 접근·재배포 확대나 일반 개인 이용 재확인은 요청하지 않는다. 실제 `ACTIVE` grant는 별도 승인·검증된 설치 전까지 비워 둔다. |
| **G4 당일/지속 운영** | 현재 배포·달력·창·백업·reader 소유권은 조회하지 않았다. 기본 session-window 파일은 빈 `entries`이므로 당일 proof가 아니다(계획 `:106-108`; [장중 runbook](stock-beta-intraday-quotes.md) `:82-115`). 역사 XKRX artifact는 2026-08-28까지만이며 공식 당일 proof가 아니다([XKRX runbook](xkrx-calendar-bootstrap.md) `:1-17`, `:25-37`). | 매 유효 거래일에 운영자/Owner가 **당일 공식 KRX 시간창 근거**를 수동 수집·해시 검토·보호 installer에 제공하고, 기존 승인 범위의 **KIS `chk-holiday` 단 한 번**으로 만든 불변 calendar source/claim과 같은 날짜를 결합하는 책임·마감·부재 시 off 절차를 지정한다. 다음 거래일 proof 공급 경로가 없으면 운영 지속성 미완료다. 새 날짜가 9월 19일 미확정 claim의 삭제·재시도 권한은 아니다. exact commit/12 image/env/manifest, 0055 적용 상태, backup/restore, 메모리·reader 소유권은 WP-8 전에 별도 현재 관측이 필요하다([릴리스 runbook](production-release-and-backup.md) `:9-36`, `:289-332`). |
| **G5 실제 프로토콜** | 47필드, ACK, 해제, `PINGPONG` 등은 문서와 synthetic/loopback 구현의 대상이다. [9월 30일 인계](../handoffs/2026-09-30-kis-market-stream.md) `:62-85`, `:183-204`의 original1–26·pacing 결과는 실제 broker 수신·30 ACK·SSE/DOM 인수가 아니다. | G1–G4와 별도 현재 bounded 실행 범위를 충족한 뒤 정상 Owner 세션에서 **005930.KRX 한 종목**을 READY/구독하고 실제 ACK, 47필드, 새 수신, 해제 ACK/종료, heartbeat/재연결 및 관측된 quota를 정형 코드·시각·카운터로 검증한다. 조용한 장이면 미관측으로 기록한다. 충돌·거절·모호한 승인키 결과는 정지하고 임의 재발급/46필드 수용을 하지 않는다. 이 pilot이 통과해야 나머지 29개를 순차 등록한다(계획 `:390-409`). |

## 후속 승인에 제시할 안전한 비밀 제외 입력

아래는 **검토용 필드 목록**이다. 실제 installer 파일 형식이나 생성된 승인값을 주장하지 않는다. WP-6의 정식 helper 계약과 맞춰 코디네이터가 보호된 입력을 별도로 작성·검토한다(기준 계약 `:289-298`, `:1020-1075`). 이 저장소에 실값/보호 입력을 커밋하지 않는다.

| 기록 | 필수 비밀 제외 필드·검증 |
| --- | --- |
| 네트워크 승인 | 승인 주체·시각·유효 범위; 정확한 method/scheme/host/port/path 두 개; `H0STCNT0` 전용, subscribe `1`/unsubscribe `2`, 정확한 30코드 목록의 해시; 평문 `ws`에서 접속키·시세가 TLS 없이 흐른다는 명시적 수락; 출처 URL/개정/확인일; 불변 승인 문서 SHA-256. `appkey`, `secretkey`, `approval_key`, URL query·프레임은 금지. |
| 슬롯·용량 증명 | 운영자가 부여한 opaque `credential_slot_id` UUID, 기존 양의 credential generation, 같은 계정/App Key 범위라는 비밀 제외 확인, 동시 세션·다른 클라이언트/채널 등록 수·관측 시각·책임자, 최신 공식 한도 근거와 잔여 30 이상 계산. 키의 값이나 공개 해시는 금지. 다른 호스트/클라이언트 부재는 코드 검색만으로 증명되지 않는다. |
| 권리·grant 검토 | 정확한 Owner UUID, 기존 entitlement ID/reference/document SHA-256, 유효 시작/종료일, 좁은 WS/cache/SSE 범위 결정, `credential_slot_id`, grant ID/revision UUID, exact30 list SHA-256, wire/endpoint/network-contract SHA-256, 검증된 설치 commit. `ACTIVE`는 빈 기본값이며 승인 문서나 SQL 삽입 결과를 이 파일에서 만들어내지 않는다. |
| 날짜·릴리스 | KST 대상일, 당일 KRX 공식 창의 출처 URL·조회 시각·원본 해시·분류/시각, KIS calendar source/claim UUID·content hash·조회 시각·동일 날짜, 다음 유효일의 수동 공급자와 cutoff, 검증된 installed commit/manifest/12 image/env 일치, 백업 시각·복구 검증 결과. secret·Raw body·세션 쿠키·가격 payload는 제외. |

## 코디네이터의 이후 읽기 전용 preflight

현재 WP-2에서 **실행하지 않은** 절차다. 보호 운영 환경 접근과 live 작업은 별도 범위에서 진행한다.

1. 최신 공식 KIS 공지와 시장/Approval 상세의 개정 시각을 다시 확보한다. 특히 9월 16일 본문이 열리지 않으면 G2를 닫지 않는다. 30 미만·채널 충돌이면 Owner 제품 결정을 받기 전 고정30 인수를 보류한다.
2. 운영자와 함께 동일 credential slot의 비밀 제외 메타데이터만 확인한다: 소유 App Key/계정 범위, 이미 열린 WS 세션·다른 host/client/채널/등록 수, 동일 slot의 다른 사용 계획. 기존 세션이 불명확하면 pilot을 시작하지 않는다.
3. ADR-0005의 reference/해시/기간과 현재 entitlement 상태를 보호된 절차로 대조한다. 새 WS/cache/SSE의 좁은 결정과 네트워크 승인 artifact를 검토한다. grant/approval-state가 실제로 없거나 `ACTIVE`가 아님을 현재 상태로 확인하되 생성·수정하지 않는다.
4. 그날 KST 날짜의 KIS calendar claim/source/hash와 공식 KRX window proof/hash가 같은 날짜에 유효하고 개장 구간에서 서로 모순되지 않는지 읽기 전용으로 검증한다. WS와 EOD가 쓰는 KIS calendar source는 동일해야 한다. `--reuse-existing-source`는 존재하는 일치 source만 재검증하며 새 KIS 호출을 만들지 않는다([장중 runbook](stock-beta-intraday-quotes.md) `:186-232`). 다음 유효일 입력 담당자와 실패 시 off 운영을 기록한다.
5. 설치 전후 각각 exact release commit·env·manifest·12개 image ID, migration, 상태/lock inode·mode·mount, 현재 서비스/reader/빌드 부재, 메모리·swap/OOM, 백업 신선도와 복구 가능성을 확인한다. 9월 19일 report를 오늘의 관측으로 대체하지 않는다. off 상태에서 공식 installer의 plan/check까지만 별도 허용 범위에 따라 검토한다.
6. 정상 Web `/login` → 기존 `/api/v1/auth/session`의 Owner role 확인 경로를 사용한다([Web route](../../apps/web/app/login/route.ts), [session client](../../apps/web/lib/api/server-session.ts#L31-L52), [API path](../../apps/web/lib/api/contracts.ts#L46-L50)). 유효 세션을 얻을 수 없으면 Owner가 직접 로그인하는 한 단계만 요청한다. 쿠키 추출·DB 세션 삽입·auth 우회는 금지한다. 이 확인도 현재 수행하지 않았다.

**다음 단계:** G1·G3의 좁은 Owner 결정과 G2 최신 한도/slot 증명, G4 당일·다음날 공급 절차를 코디네이터가 확정한 뒤, 별도 권한을 가진 WP-9의 한 종목 G5 pilot으로 진행한다. 본 문서는 `LIVE_ACCEPTED` 또는 `FINAL_ACCEPTED` 증거가 아니다.

## 2026-10-07 WebSocket 전용 전환 준비

Owner는 실시간 시세를 WebSocket 전용으로 운영하기로 결정했다. 새 릴리스는
`owner_only`에서 보호된 설정의 명시적 `market_ws`를 요구한다. 누락·`rest`는 활성화
오류이며, 스트림 장애 시 오래된 값/이용 불가를 표시하고 REST 시세로 전환하지 않는다.
EOD 일봉·참조가격 REST 수집은 별도 경로다. 브라우저 전달은 기존 인증 SSE를 사용한다.

공개 문서만 대상으로 한 당일 재조회에서 시장 채널 속성 JSON은 HTTP 200이었다.
`res_b` 47개 필드와 마지막 `MARKET_CLS_CODE`를 확인했고, 최신 `lastModifiedDate`는
`2026-09-11T16:31:27+09:00`으로 기존 wire 계약과 일치한다. 조회 URL은
<https://apiportal.koreainvestment.com/api/apis/guide/property/714d1437-8f62-43db-a73c-cf509d3f6aa7>이며,
응답 SHA-256은 `ea945675204c6394b614124012995daaad5080eec7a62339f72c9aa613d6df68`이다.
Approval 속성 JSON도 HTTP 200으로 5개 속성을 반환했고, 최신 개정은
`2024-12-13T15:21:56+09:00`이었다. 이는 문서 조회이며 접속키 발급이나 시장 소켓
접속을 수행한 결과가 아니다.

9월 16일 유량 공지는 공개 HTML이 본문을 포함하지 않았고, 그 HTML의 조회 경로로
확인한 공개 JSON은 HTTP 400이었다. 최신 한도와 해당 credential slot의 다른
클라이언트 사용 여부는 여전히 확인하지 못했다. WebSocket 전용 제품 결정 자체로
G1의 정확한 평문 연결 수락, G2의 현재 용량 증명, G3의 검토된 권리/grant 입력,
G4의 당일·다음날 증명, G5의 실제 수신 인수를 충족했다고 취급하지 않는다.

### Owner의 평문 연결 수락 — 2026-10-07

Owner는 접속키가 암호화 없이 전송된다는 설명을 받은 뒤 KIS 평문 WebSocket 연결을
명시적으로 허용했다. 이 결정은 기존 시장 시세 계약의 HTTPS
`POST https://openapi.koreainvestment.com:9443/oauth2/Approval`과
`ws://ops.koreainvestment.com:21000/tryitout`, 시장 채널 `H0STCNT0`의 읽기 전용
접속 준비에 적용한다. 같은 평문 연결 수락을 다시 요청할 필요는 없다.
접속 승인 결정과 검토된 grant 입력·설치·실제 수신 증거는 서로 별개다.

Owner는 같은 키를 다른 WebSocket 프로그램이 사용 중인지는 모른다고 답했다.
당일 호스트 TCP 소켓과 실행 중인 11개 컨테이너의 IPv4/IPv6 TCP 테이블을 확인한
시점에는 KIS의 운영/모의 WS 포트 `21000`/`31000`으로 연결된 소켓을 찾지 못했다.
이는 해당 서버의 관측이며, 다른 기기의 연결이나 동일 App Key의 전역 독점 사용을
증명하지 않는다. 다른 클라이언트의 부재와 최신 공식 용량 근거가 확인되지 않은
상태에서 임의 접속으로 기존 세션을 교체하거나 연결 충돌을 시험하지 않는다.

같은 날 공식 포털을 새 격리 브라우저에서 일반 화면으로 확인했다. 공개 공지 목록에는
9월 16일 유량제한 공지가 잠금 표시와 함께 있었고, 제목 링크로 열면 비밀글 안내,
직접 공지 주소로 열면 권한 없음 안내가 표시됐다. 로그인·인증 우회를 시도하지 않았고
브라우저는 모두 종료했다. 따라서 앞선 HTTP 400을 현행 한도 근거로 삼지 않으며,
Owner가 정상 접근으로 확인한 공식 본문 또는 KIS의 공식 답변이 필요하다.
이 조회에서도 현재 숫자 한도나 중복 세션 처리 방식은 확인하지 못했다.
