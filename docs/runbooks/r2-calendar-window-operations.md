# R2 달력·시간창 공급 책임과 마감

2026-10-23, Wave182 / R2-02. 코드 대조 기준은
`b2b9605dc1a63d12928193a2f494d0a25040e425`이며, 마지막 인수된 운영 범위는
API/Web `a6da63c134fa471ff91e28a32c4b6a2ad9c08ba7`와 보존 Owner V2
`7fa779cc7f0ff3cc291f1e444257beb5f7412c7c`다. 다른 worker의 버전까지 같다고
가정하지 않는다. 이 문서는 공급·확인 책임과 결손 처리를 확정한다. 새 자료 수집,
배포, Daily 활성화, 장중 수신 또는 지속 무인 공급의 실행 증거는 아니다.

## 담당과 실행 계기

운영 담당은 이 작업의 코디네이터다. Owner가 지시한 서버 확인·구현·검증의 기존
권한 안에서 자료 준비, 공식 도구 적용과 별도 결과 대조를 직접 수행한다. Owner에게
서버 조회나 같은 로그인·자료 문의를 되돌리지 않는다. 정상 사용자 인증과 실제 화면
사용 인수는 별도이며, 코디네이터가 세션을 추출하거나 대신 만들지 않는다.

실행 계기는 승인된 배포·Daily 활성화 준비, 정상 장중 수요 또는 구체적인 변경 근거다.
현재 D-06/D-07와 가격 충돌 유예를 유지한다. 정기 heartbeat만으로 자료를 재수집하거나
공급자·운영 조회를 반복하지 않는다. 실행 권한·인증의 유효기간을 넘겨 작업하거나
연장하지 않는다. 담당을 이어받을 때에는 날짜, 원래 claim과 시도 결과, 미완료 범위를
검증된 인계 기록에 남긴다. heartbeat는 달력 공급 timer나 장애 보호 장치가 아니다.

## 세 가지 입력

| 입력 | 공급·확인 경로 | 준비 마감·대조 시점 | 없거나 잘못된 경우 |
| --- | --- | --- | --- |
| 예약 날짜 선택용 XKRX | 설치 릴리스의 `kis-daily-calendar-refresh.sh --plan/--check`; 필요한 변경만 검토 후 `--apply`. 고정 환경·검토된 override와 보호 calendar/manifest를 사용한다. | Daily 설치·활성화 전 범위/해시 확인. 운영 담당은 릴리스나 override/요청 범위 변경 때 다시 대조한다. 각 실제 Daily 실행도 요청 범위를 검증한다. 활성 설치가 필요하면 해당 KST 날짜 **16:30 이전**에 완료한다. | 누락·해시 불일치·범위 초과는 해당 예약 경로를 중단한다. 과거 체크인 artifact나 평일 계산으로 대체하지 않는다. |
| KIS 실제 거래일 | 기존 current-day EOD 또는 설치된 calendar-only helper의 단일 획득/정확 원본 재사용. 같은 날짜의 claim·원본 UUID·해시·정규화/DB 계보를 확인한다. | 장중 사용을 준비할 때에는 해당 날짜의 검증된 실제 개장 이전에 시간창과 함께 준비한다. EOD만 필요하면 승인된 **16:30 이후** current-day 실행 안에서 확보·검증할 수 있다. | 요청 날짜 누락·불명확/실패 claim·충돌은 실패로 보존한다. 새 UUID로 같은 날 재수집하거나 continuation을 따라가지 않는다. 날짜 선택용 XKRX를 실제 거래일 증거로 사용하지 않는다. |
| 당일 KRX 시간창 | 운영 담당이 허용된 공식 `https://global.krx.co.kr/` 자료를 그 KST 날짜에 확인하고 URL·실제 조회 시각·원본 해시·시간을 기록한다. 설치된 `install-intraday-session-window.py`로 검증·적용·`--check`한다. | 장중 가용성을 준비하는 날짜의 검증된 개장 이전. `REGULAR`의 코드 계약은 09:00–15:30이고 `SPECIAL`은 검토된 해당 날짜의 실제 구간을 쓴다. KST 날짜가 바뀌면 이전 증거를 재사용하지 않는다. | 누락·미래/다른 날짜·유효하지 않은 창은 `UNKNOWN`/quote 생성 차단으로 유지한다. 유효한 창의 시간 밖은 `CLOSED`다. 창만으로 거래일이라고 판단하지 않는다. |

각 마감은 준비 순서와 기존 코드 경계다. 새로운 timer·가용성 보장·미래 거래일 선언을
추가하지 않는다. 다음 날짜도 실제 그 날짜가 된 뒤에 자료를 확인한다. 다음 장중
사용이 예정되면 담당 코디네이터가 위 순서를 맡고, 제때 완료하지 못하면 수신 인수를
보류하고 결손 이유를 기록한다. 지속 무인 공급은 실제 운영 증거가 생길 때 별도 판단한다.

소스: [날짜 공급기](../../scripts/ops/kis-daily-calendar-refresh.sh#L15),
[Daily 날짜·cutoff 검증](../../scripts/ops/kis-daily-production.sh#L514),
[설치 시각 제한](../../scripts/ops/install-kis-daily.sh#L549),
[시간창 날짜·시각 검증](../../scripts/ops/install-intraday-session-window.py#L170),
[시간창 종류 검증](../../scripts/ops/install-intraday-session-window.py#L284).

## 장중 준비와 EOD 재사용 순서

1. 실제 KST 날짜와 설치 릴리스·manifest·모드·각 서비스 source를 확인한다. API/Web와
   Owner의 서로 다른 승인 버전을 보존한다. 작업별 현재 상태 확인은 해당 실행 범위에
   한정하고 과거 closure를 오늘의 health·수요·거래일 증거로 사용하지 않는다.
2. 장중 분기에서는 공식 당일 시간창을 먼저 검증한다. `operational_v1` 설치는 내용
   주소로 고정된 artifact와 activation pin을 사용한다. 설치기 `--check` 성공만으로
   공식 자료의 의미나 실제 거래일 확인을 대신하지 않는다.
3. calendar-only 분기는 연구 daemon이 정지/부재이고 daily timer가 비활성인 전제를
   독립 확인한다. 공식 Compose helper의 `--preflight`는 daemon을 검사하지만 timer
   비활성까지 증명하지 않는다. 필요한 정지·복구는 별도 검토된 직렬 작업으로 수행한다.
4. 기존 당일 claim과 원본이 있으면 정확한 원본 재사용 경로를 택한다.
   `--reuse-existing-source`는 공급자 호출 없이 기존 claim/source를 검증·발행하지만
   **DB 발행을 포함하므로 읽기 전용 조회가 아니다.** 원본이 없으면 기존 허용 범위와
   미소진 당일 획득 여부를 확인한 뒤 retained UUID 하나를 사용한다. 결과가 불명확하면
   원래 기록을 회수하며 시도 파일·claim을 삭제하거나 새 UUID로 우회하지 않는다.
5. 실제 캡처 시각·exact-date 응답·원본/정규화/DB identity를 별도로 대조한다. 이후
   같은 날 EOD는 같은 원본의 UUID·bytes·해시·조회 시각을 보존해 재사용한다. 단일
   `chk-holiday`의 첫 응답에서 필요한 날짜를 찾지 못해도 추가 요청은 하지 않는다.
6. 그 날짜의 calendar와 window, 현재 Owner binding, 정상 인증 수요가 모두 갖춰졌을 때
   새 수신 범위를 판단한다. 이 문서나 과거 관측으로 수요·ACK·READY·quote를 만들지 않는다.

소스: [공식 calendar helper](../../scripts/ops/compose-release.sh#L756),
[원래 day lock·획득·발행](../../data-pipelines/collectors/src/calendar_bootstrap.rs#L342),
[claim 충돌·불확실성·재사용](../../data-pipelines/collectors/src/calendar_bootstrap.rs#L423).
장중 상세 계약은 [기존 runbook](stock-beta-intraday-quotes.md#calendar-only-publication-and-refresh),
WS 운영은 [market-stream 절차](kis-market-stream-operations.md)를 따른다.

## Daily와의 경계

당일 장중 시간창은 EOD Raw 수집과 추천 입력 승인을 대신하지 않는다. 장중 시간창이나
R3 수신 인수가 없다는 이유로 별도 승인된 EOD 경로를 막지 않는다. 반대로 날짜·시간창이
준비됐어도 기업행사/가격 소스 충돌, 독립 입력 승인, 현재 Daily cohort 바인딩과 공식
DB-LAST 활성화 조건을 건너뛸 수 없다. 현재는 입력 미승인·Daily OFF가 마지막 인수 상태다.

현재 R2 source 모드는 16:30 전이나 비거래일 선택에서 worker/provider를 실행하지 않는다.
16:30 이후 `--raw-source --check`는 provider-free여도 일회 worker를 실행하므로 단순 파일
조회로 취급하지 않는다. `Persistent=true`의 늦은 실행은 첫 거래일이나 관찰 성공을
소급하지 않는다. `--install-disabled`는 CREATE_ONLY이며 기존 대상은 공식 `--check`로
검증한다. 삭제·재설치로 이를 우회하지 않는다. 승인된 recurring action 공급의 기존
14일/280 wire-GET 상한·실패/불확실 시 당일 재실행 금지도 그대로 유지한다.

세부 활성화와 미래 첫 세션 선정은 [Daily 입력 절차](owner-beta-daily-input.md)를 따른다.
공식 시간창이 없다고 운영 설정을 임의로 off로 바꾸지 않는다. 필요한 OFF 복귀는
호환 릴리스·현재 binding·정상 drain·공식 helper·독립 서비스 확인을 갖춘 별도 작업이다.

## 기록과 완료 범위

준비 기록에는 대상 KST 날짜, 준비 목적/마감, 담당 코디네이터, app/Owner/tool commit과
manifest, 원본 출처·실제 조회 시각·해시, calendar claim/source/정규화 identity,
공식 도구 결과, 별도 대조 결과와 미완료 이유를 남긴다. 로컬 검토 시각과 실제 호스트
관측 시각을 구분하고 인증정보·원문 데이터·쿠키는 기록하지 않는다.

R2-02의 공급 경로·담당·마감·부재 시 동작 문서화는 완료다. 실제 날짜별 공급,
Daily 활성화·예약 3거래일·Owner 사용·R3 수신과 전체 R2/R3 FINAL은 별도 미완료다.
D-06/D-07와 가격 충돌 조사를 이 문서로 재개하거나 완료 처리하지 않는다.
