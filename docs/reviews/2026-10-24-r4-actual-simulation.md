# R4 승인 입력 실제 시뮬레이션 검증

2026-10-24, R4-02/03 후속. Owner의 “진행해 머뭇거리지.말고” 지시에 따라
확인된 기존 승인 v3 artifact를 실제 Rust 경로로 검증한다. 대상 `9a0b14f`의 hosted 빌드·CI는
통과했고 같은 후보 smoke도 성공했다. 두 독립 프로세스의 실제 승인 입력 실행·반복 결과·정리를
확인해 R4-02/03의 첫 시뮬레이션 구현·검증 범위를 수용했다.

## 실행 대상과 검증 경계

- 기존 core 19개 파일은 CI40071485641·smoke40071496969가 통과한6ada50c와 일치한다.
- 새 Cargo example `verify_approved_simulation`은 기존 public API로 두 비용 프로필을
  두 번 계산하고, canonical bytes/hash·strict JSON round trip을 검증한 뒤에만 요약을 출력한다.
- 컴파일 시 source SHA·실제 toolchain·target·profile과 자기 실행 파일 SHA-256을 결합한다.
  소스 검토는 실제 executable provenance를 대신하지 않는다.
- 기존 CI의 기본 OFF 입력으로 static musl 검증 파일을 빌드한다. 기존7개 검사와 required를
  유지하고 선택한 빌드도 required에 포함한다. hosted 전달물은 binary/provenance 두 파일만이며
  승인 입력·Raw·Curated·로그·전체 target은 업로드하지 않는다.
- 승인 root는 `/var/lib/lagrange/data/artifacts/historical-price-beta-root`다.
  UID10001, network none, read-only root와 두 개의 정확한 읽기 전용 연결, capability 제거,
  384MiB/CPU0.5/PID32/20분 상한으로 일회성 컨테이너 두 개를 순차 실행한다.
  두 프로세스의 전체 결과 hash를 비교한다. 운영 서비스 변경이나 provider 재수집은 없다.
- 강제 SIGKILL까지 정리를 보장하지 않는다. 생성 시도 전 private receipt에 정확한 이름과
  source를 기록하고, 재개 시 해당 소유 자원만 회수한다. 정상 종료·처리된 취소·제어 오류는
  소유 컨테이너 부재와 attach 자식 reap을 확인할 때까지 공유 실행 잠금을 유지한다.

## 소스 검증

Paseo Luna/max 구현 두 명의 actual idle/pending0와 보고서를 회수했다. 독립 Sol/high 검토에서
Rust example과 최종 CI3파일은 READY_FOR_HOSTED다. 런타임 생성 응답 불확실성/정리 중 잠금 해제
두 지적은 코디네이터가 보완했으며 실패 주입4검사가 통과했다. 실제 Docker 실패 검증은 아니다.

코디네이터가 짧은/전체 rustc commit 비교, non-secret metadata 읽기 권한, clean checkout 확인과
테스트 메시지를 보정했다. 임시 PyYAML6.0.2 환경의 CI 계약41검사, shell/embedded-shell 문법,
Python AST, Rust formatter와 공백 검사가 통과했다. 컴파일·실제 입력 실행 결과는 후속으로 기록한다.

실행 증거는 owner-private `target/r4-first-simulation/u3-*` 아래에 보존한다.
작성 당시 성공 결과·재현성 수용은 미확인이었다. 아래 종료 증거가 이를 갱신하며
R4-04~06, 운영 배포와 Owner 인수는 별도로 유지한다.

## 09:06UTC — 빌드 증거 수용·실제 실행 착수

대상 `9a0b14fde51722afd5587e4eabcff070bf151912`, CI40078923911 attempt1의
`r4-verification-build` job120828208045가 성공했다. release Clippy·example4검사·실행 파일
빌드 및 정확한 두 파일 업로드를 확인했다. 전체 CI/workspace와 smoke40078925934는 아직 진행 중이다.

Artifact12555290326의 ZIP SHA-256은 `795e6a0fd595c163a3644778f4f0bdab31d54159547c1472935be98db797ddca`다.
다운로드 크기·digest, 두 파일 목록, candidate/run/attempt,3개 source hash, 실제 rustc 출력과
static ELF를 대조했다. 실행 파일 SHA-256은 `f7ed6422581b470ee563159adab8e1118bef41db66b60536d553e9faa273c564`,
툴체인은 `rustc 1.97.1 (8bab26f4f 2026-07-14)`, target은 `x86_64-unknown-linux-musl`, profile은 release다.

컨트롤러의 생성 응답 불확실성을 다시 보완해, 생성 완료 증거 없이 빈 조회만으로 잠금을 풀지 않게 했다.
지연 생성과 실행 전 실패를 포함한 모의6검사가 통과했다. 09:06:25UTC 첫 격리 프로세스를 시작했고
설정한 UID·network none·읽기 전용 연결·384MiB/CPU0.5/PID32 경계를 실제 inspect로 확인했다.
09:07 관측 사용 메모리는 약29.75MiB, PID3, OOM false다. 이는 실행 중 관측값이며 전체 성능 측정이 아니다.
실제 결과·반복 hash·두 프로세스 정리는 아직 PENDING이다.

## 09:15UTC — 전체 CI 종료 증거

[CI40078923911](https://github.com/sihioov/Lagrange/actions/runs/40078923911) attempt1은
정확한9a0b14f·review-handoff-next-task·workflow_dispatch로 SUCCESS 종료했다. 기존7개 검사,
선택한 r4-verification-build, required의9개 job이 모두 성공했고 집계 입력도 대조했다.
workspace 결과 줄 합계는3,522 pass/0 fail/6 ignored/386 filtered이며 고유 테스트 수와 구분한다.
시뮬레이션 unit41개와 integration2개 모두 통과했고, 후보/현재 소스23개 hash가 일치한다.

코디네이터 receipt는 `target/r4-first-simulation/u3-hosted/candidate-9a0b14f/coordinator-ci-terminal-verification.json`,
SHA-256 `1059330e577778d5ddfcf6794a1f551a7906d197006e20670583ef5980763b7f`다.
같은 후보 smoke와 실제 입력 계산·반복 결과·정리는 별도 진행 중이다. CI 성공을 실제 실행 성공으로
대체하지 않는다.

## 09:25UTC — 첫 실제 승인 입력 실행 성공

첫 프로세스는 09:24:26UTC exit0/OOM false로 종료했고 경과 시간은1,081.655초였다.
승인된 기존 artifact의5개 pin·bars hash와 실행 파일 identity가 일치했으며 평가 기간은
2017-06-22~2026-08-28,2,252세션이다. 두 비용 프로필의 canonical 검증·strict JSON round trip과
프로세스 내부 반복 계산이 성공했다. 이 시간은 CPU0.5 제한 아래 검증 프로그램 전체 경과 시간이며
운영 성능 인수 지표가 아니다.

- 10bp/side 결과 SHA-256: `e67afd944015d1f5494739d8d5544117e4d8280c5ab2b2ec120b86653a35073c`
- zero-cost 결과 SHA-256: `0f2f3f1c9ed76b5c37ecf5fe0de866af6494d2119b7c54450642d334cc2c6162`

첫 컨테이너의 remove exit0·부재·attach reap을 확인했고 코디네이터의 별도 이름 조회도 비어 있었다.
원본 `runtime-1.json` SHA-256은 `1297fb3f9bfe1488b5bcfd5d301405b929859ae1cb4943b33cfc5f0d97347e35`,
코디네이터 확인 receipt `coordinator-first-runtime-verification.json`은
`88558118f6a11a943eaaa719c5e455b2724b397455dd2ad888c4f5b317cf54af`다.
상세 요약과 지표는 owner-private 실행 receipt에 보존한다.

같은 감독 프로세스가09:24:27UTC 두 번째 독립 컨테이너를 순차 시작했다. UID·network none·읽기 전용
연결·자원 제한을 다시 확인했다. 두 프로세스 간 일치, 두 번째 정리·운영 컨테이너 identity 관측과
same-SHA smoke는 아직 PENDING이다. R4-02/03 전체 인수는 이 후속 증거와 구분한다.

## 09:40UTC — 동일 후보 hosted 검증 수용

[smoke40078925934](https://github.com/sihioov/Lagrange/actions/runs/40078925934) attempt1은
09:37:55UTC SUCCESS로 종료했다. 정확한9a0b14f·branch·workflow를 직접 확인했으며20개 step이
모두 성공했다. static/functional PASS, Raw-probe와 전체 cleanup exit0, 프로젝트 컨테이너·볼륨·
네트워크 및 context image 부재, 전용 builder 제거 출력을 대조했다. Raw-probe6개 볼륨은 제거
명령 성공까지만 확인됐으며 별도의 후속 부재 조회는 없다. 캐시 저장 성공을 성능 인수로 해석하지 않는다.

WP-6 actual idle/pending0, 최종 보고서와 receipt를 회수했고 후보/현재 소스23개 및 HEAD/remote가
9a0b14f에 일치했다. CI9개 job과 same-SHA smoke 범위를 수용한다. 코디네이터 smoke receipt
SHA-256은 `96893c9e9d5d068142d12d06b8e7b7f3eb1e31cf5e426b13c8a9355bac8d5823`,
worker 보고서 hash는 `cbc715cc5ec224be40085d668f3448fdc36c546fd19b3632af25b9390fed0e72`다.
두 번째 실제 실행·교차 프로세스 결과·최종 정리는 별도 진행 중이다.

## 09:43UTC — 실제 입력·실행 파일·결과 재현 최종 수용

검증 대상은 `9a0b14fde51722afd5587e4eabcff070bf151912`다. 두 번째 독립 프로세스도
exit0/OOM false로 완료했고 경과 시간은1,078.588초였다. 서로 다른 두 컨테이너에서 각2회씩
계산한 두 비용 프로필의 결과가 일치했다. 각 프로세스는 canonical bytes/hash와 strict JSON
round trip을 검증했고, 코디네이터는 두 summary의 전체 일치·실제 stdout hash·5개 승인 pin·
bars hash·실행 파일 hash·평가 기간을 다시 대조했다. 초기 자금에 대한 NAV/수익률,
벤치마크 차이, 매수·매도 금액에 대한 비용 합계도 독립 Decimal 산술과 일치했다.

두 컨테이너의 정상 종료·제거·부재와 attach 자식 reap을 확인했고 감독 프로세스도 exit0으로
끝났다. 동일 source/scope label의 잔존 컨테이너 조회는 비어 있다. 실행 전후 기존 운영 컨테이너의
ID/image/StartedAt/restart count가 동일했다. 이는 운영 건강 상태나 운영 배포 인수를 의미하지 않는다.
검증은 network none·UID10001·RO 입력/바이너리·384MiB/CPU0.5/PID32 경계 안에서 끝났다.
새 수집·거래·운영 설치는 수행하지 않았다.

| 최종 private 증거 | SHA-256 |
| --- | --- |
| `u3-runtime/acceptance.json` | `136202d9963053ccf84d049612fecf9b9b84c78b3ebe4aaf9eafb55ea5e516a7` |
| `u3-runtime/runtime-2.json` | `3426b21a65609a4fca44f8d3d615befaa04030e578faf36917a7eb8448e878f6` |
| `u3-runtime/production-observation.json` | `58d9020131b72b9eca035a97b64fad0c6feb7a893b89a4de6a4739faa81b11f3` |
| `u3-runtime/coordinator-final-acceptance.json` | `ab4680d8931f4677c809bcfdc4fae17a2b8032d1500f670a72d3914dd5a448cf` |

위 경로는 모두 `target/r4-first-simulation/` 아래다. 실제 지표는 private acceptance summary에
보존하며 Git/hosted artifact에 입력 데이터나 성과 원장을 올리지 않는다. 결과의 의미는 채택된
`PRICE_RETURN_ONLY`, `vendor_snapshot=true`, `strict_pit=false` 범위다. 배당 포함 총수익률이나
엄격한 PIT 검증으로 확장하지 않는다.

이로써 U2 실제 승인 artifact 실행과 U3 실제 executable provenance·반복 결과를 확인했고,
R4-02/03 첫 구현·실제 기준 전략 검증을 완료했다. R4-04의 전체 무결성/사용자 격리 인수,
R4-05 큐·저장·API·Web 연결과 R4-06 전략 확대·운영 출시·Owner 사용 확인은 남아 있다.
모든 소유 작업자10명의 actual idle/pending0와 보고서를 회수했다. 기존 실패 이력과 smoke의
Raw-probe 볼륨 별도 부재 조회 한계는 보존한다.

종료 문서의 docs fast preflight와 git whitespace 검사가 통과했다. 모든 소유 실행과 보고서가 정리된 뒤
09:44UTC 현재 작업의 감시 `e23b0881` 삭제 성공을 확인했다. 추가 실행이나 후속 범위는 자동 시작하지 않았다.
