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
| 소스 변경 | `git diff --check` 통과, 구조 변경이 없어 아키텍처 도표 변경 없음 |

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

현재 운영 릴리스는 기존 REST 방식이다. 조회 당시 WS grant·producer·cache는
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
root 전용이다. 현재 세션은 UID 1000이고 `sudo -n true`가
`interactive authentication is required`로 실패했다. root 경계를 Docker 등으로
우회하지 않는다. 정확한 명령·보호 입력 계약은
[운영 절차](../runbooks/kis-market-stream-operations.md)와
[릴리스 절차](../runbooks/production-release-and-backup.md)를 따른다.
