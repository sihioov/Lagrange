# R4 승인 입력 실제 시뮬레이션 검증

2026-10-24, R4-02/03 후속. Owner의 “진행해 머뭇거리지.말고” 지시에 따라
확인된 기존 승인 v3 artifact를 실제 Rust 경로로 검증한다. 이 문서를 포함하는
후보의 hosted 빌드·CI 및 실제 실행은 아직 PENDING이다.

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
현재 성공 결과·수익률·재현성 수용은 주장하지 않는다. R4-04~06, 운영 배포와 Owner 인수는 별도다.
