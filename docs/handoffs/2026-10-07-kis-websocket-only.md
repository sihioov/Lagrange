# KIS 실시간 시세 WebSocket 전용 전환

기준: 2026-10-07 KST, 변경 전 소스·운영 릴리스
`cd23906f399295200d701892c87b654a64dec026`.
Owner는 실시간 시세를 WebSocket 전용으로 전환하도록 지시했고, 기존 계약의
평문 시장 소켓 연결도 명시적으로 허용했다. 이 문서는 소스 변경과 검증의 기록이며
실제 KIS 수신이나 운영 활성화 완료를 주장하지 않는다.

## 변경 결과

- 수집기·API·배포 helper는 `owner_only`에서 명시적 `market_ws`를 요구한다.
  누락·`rest`·잘못된 값은 활성화 전에 거부한다. 수집기의 backend 선택도 REST
  실시간 producer를 시작하지 않는다.
- Web은 같은 설정에서 스트림 화면을 사용한다. 가격 변경은 브라우저 프레임마다
  최신값으로 모으고, 로그아웃·구독 변경·가격 삭제·상태 변경은 즉시 반영한다.
  거래정지 표시는 오래된 값이나 장 종료 표시와 함께 표시할 수 있다.
- 스트림 장애에 REST 시세로 전환하지 않는다. EOD 일봉·참조가격 수집은 기존
  읽기 전용 REST 경로를 유지한다. Once/Healthcheck도 실시간 producer를 시작하지 않는다.
- 시장 구독 범위는 체결시세 `H0STCNT0`이다. 별도 호가 채널은 추가하지 않았다.
  서버에서 브라우저까지는 기존 인증 SSE를 사용한다. 일봉 차트의 마지막 캔들을
  실시간으로 변경하는 기능은 이번 변경에 포함되지 않는다.

## 검증과 한계

| 대상 | 결과 |
| --- | --- |
| Rust 설정·API runtime·runner | 5 + 33 + 21 = 59개 테스트 통과, 실제 runner `cargo check` 통과 |
| Web 관련 회귀 | 21개 파일 192개 테스트 통과, TypeScript 및 변경 파일 Biome 통과 |
| 운영 helper | KIS Compose·production ops·intraday self-test 및 Python Compose 8개 테스트 통과 |
| 실제 React/Chromium, 합성 HTTP/2 서버 | 10개 시나리오 통과, 창 숨김 1개 `NOT_RUN` |
| 소스 변경 | `git diff --check` 통과, main 머지 전 WS 전용 실시간 경로 설명과 아키텍처 도표의 코드 근거를 동기화하고 PNG를 로컬 재생성 |

Chromium 검증은 실제 브라우저와 합성 서버를 사용했다. 정상 스트림, 구독 해제,
연결 단절, offline/online, 로그아웃, unmount, 10개 페이지, 수신 노후화와 전달
중단을 확인했다. 이 환경에서는 창 최소화가 native visibility를 숨김으로 바꾸지
않아 해당 시나리오를 검증하지 못했다. 전체 browser QA 결과는 `passed: false`이며,
JavaScript로 visibility를 덮어써 통과 처리하지 않았다. 실제 인증·API·DB·KIS를
한 번에 연결한 운영 인수는 별도다.

Rust 검증은 단일 background user systemd 서비스, jobs 2로 실행했다. API의 기존
REST 설정 fixture 4개는 새 정책에 맞게 수정한 뒤 최종 33개 모두 통과했다.
임시 node_modules 링크는 검증 후 제거했다.
최종 transport 반환 타입을 `off | market_ws`로 좁힌 상태도 mode/page-seam
2개 파일 13개 테스트, app TypeScript 및 해당 파일 Biome으로 다시 확인했다.

원본 지역 검증 기록:

- `/tmp/lagrange-ws-only-rust-checks-v2.log`: KIS parser 5개 통과, 수정 전 API fixture 실패 포함
- `/tmp/lagrange-ws-only-rust-checks-v4.log`: 최종 API 33개·runner 21개 및 runner check 통과
- `/tmp/kis-market-stream-browser-qa-ws-only-20261007/result.json`: 10 PASS / 1 NOT_RUN, 종료·정리 확인

## 운영 활성화에 남은 단계

변경 전 초기 운영 릴리스는 기존 REST 방식이었다. 초기 조회 당시 WS grant·producer·cache는
없었고 API 연결 풀은 16으로, 스트림에 필요한 최소 24보다 작았다. 새 설정의
권장 풀 크기는 32다. 호스트와 실행 중인 11개 컨테이너에서 KIS WS 포트의 연결을
찾지 못했지만 다른 기기의 동일 키 사용은 확인하지 못했다.

1. 동일 키의 다른 클라이언트 사용과 현재 공식 용량 근거를 확인한다.
2. 기존 Owner entitlement에 정확한 WS/cache/SSE 범위를 연결한 grant 입력과
   당일 calendar/window 증명을 검토한다. 승인 fixture를 운영 입력으로 사용하지 않는다.
3. 깨끗한 커밋으로 공식 12-image 빌드·manifest를 만들고 immutable off release를
   설치한다. 서비스별 순차 빌드와 자원·서비스 상태 검사를 적용한다.
4. 보호된 현재 릴리스 helper로 persistent state와 grant를 준비하고 별도 activation
   release로 API·Web·runner를 갱신한다. 운영 `.env`를 직접 수정하지 않는다.
5. 한 종목의 실제 ACK·47필드·수신·해제를 검증한 뒤 나머지 구독과 인증 화면을 인수한다.

공식 image builder의 `--apply`, 설치, provisioning, grant 설치와 Compose 갱신은
root 전용이다. 일반 `sudo -n true`의 실패만으로 허용된 운영 helper까지 사용할 수
없다고 판단하면 안 된다. 이후 실제 허용 목록과 유효한 유지보수 helper를 확인해
아래 배포를 수행했다. root 경계를 Docker 등으로 우회하지 않았다. 보호 입력 계약은
[운영 절차](../runbooks/kis-market-stream-operations.md)와
[릴리스 절차](../runbooks/production-release-and-backup.md)를 따른다.

## 실제 배포와 활성화 요청 후 점검

2026-10-07 12:46 KST, `a65f2082f59ba5d66cb7cdf482b1f6cc7e08fc20`의 공식
12-image 빌드와 strict V2 manifest 검증을 완료하고 immutable quotes-off 릴리스를
설치했다. API·Web·Owner V2를 공식 helper로 순차 교체했다. 세 실제 image ID와
OCI revision은 manifest에 일치했고 모두 healthy, 재시작 0, OOM 없음이었다.
API·Web·프록시 HTTP health도 통과했다. API 연결 풀은 32다. 기존 research-worker의
2주 전 exit 2는 이 배포에서 발생한 장애가 아니다.

Owner는 같은 날 WebSocket 활성화를 명시적으로 지시했다. 기존 평문·시장 채널
승인과 이 지시는 좁은 WS/cache/Owner SSE 운영 입력의 준비·검토·설치를 허용한다.
동일한 활성화 승인을 다시 요청할 필요는 없다. 최신 한도나 키 독점 사용이라는
운영 사실까지 확인됐다는 뜻은 아니다.

13:18 KST 점검에서 당일 운영 window의 정식 설치 검사와 실제 runtime 쿼리와 같은
calendar/version/batch 전체 lineage 조건이 각각 통과했다. 기존 entitlement도
정확한 reference/hash/ACTIVE/유효일 바인딩에 일치한다. persistent WS domain은
기존 상태를 보존한 채 패키지 helper의 검증을 통과했다. WS grant·producer·cache·
subscription·lease는 아직 0이다. 현재 보호 env에는 grant/network pin/origin이 없다.
최신 공식 용량과 다른 기기의 동일 키 사용은 미확인이다.

기존 V2에는 `005930.KRX` READY 1종목과 2026-10-06의 공개 신호 1행이 있다.
quotes-off는 실시간 영역만 숨긴다. 별도로 신호 snapshot이 없거나 조회에 실패하면
profile/chart 영역 전체가 숨겨지던 UI를 수정해 상태·오류 안내를 유지한다.
운영자의 인증된 실제 화면과 실제 broker 수신은 아직 검증하지 않았다.

설치된 a65 릴리스의 env는 변경하지 않는다. 후속 UI 배포 및 활성화에는 새 clean
commit/image/manifest가 필요하며, 활성화는 검토된 grant와 남은 용량 증명을 연결한
별도 보호 입력으로 수행한다. 같은 키의 다른 세션 부재나 최신 숫자를 추정해
ACTIVE grant를 만들지 않는다.
