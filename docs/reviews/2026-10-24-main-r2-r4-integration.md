# main에 R2/R3 기록과 R4 인계 통합

2026-10-24 (Asia/Seoul). Owner가 인계 저장 뒤 main 병합과 push를 명시적으로 요청했다.
대상은 이 문서를 포함하는 병합 커밋이다. 운영 배포와 R4 구현은 수행하지 않는다.

## 부모와 변경 범위

- 기존 main: `1286e732c098eff307c9537c3cedffc58ec9f930`, 개발 검증·캐시 개선 9커밋.
- 작업 브랜치: `7a396d7b3cfa70d1c460d28716f9d1e06a363d53`, 공통 조상 이후 400커밋.
- 공통 조상: `b0a2579bef472ae6b604405d0ab6f50255786dd4`.
- 별도 통합 작업 트리에서 병합한다. 원본 ROADMAP의 사용자 미스테이징 32행과
  `.archify/`, main 작업 트리의 미추적 파일은 포함하거나 수정하지 않는다.

충돌 4파일은 다음과 같이 결합했다.

1. `.github/workflows/ci.yml`: main의 캐시·preflight·시간 측정과 R2/R3의
   격리 Docker/DB/browser 검증을 모두 유지한다. Rust 전용 toolchain 저장소를
   설치보다 먼저 준비하고 protected artifact 부모 생성·정리 순서를 보존한다.
2. `scripts/ci/test_ci_contract.py`: 캐시·Bake 검증과 R2/R3 필수 명령 검증을
   결합한다. 시간 측정으로 감싼 명령은 명령 줄 단위로 검사하고, 실패 무시는
   명시된 캐시 단계에만 허용한다. cleanup fixture에도 시간 측정 변수를 제공한다.
3. `data-pipelines/collectors/tests/service_status.rs`: 공통 role/bootstrap SQL과
   트랜잭션 수명 전체의 두 advisory lock을 유지한다. 동일 실행 함수를 사용하는
   commit/rollback 잠금 해제 검증을 보존하고 SQLSTATE 진단은 대문자·숫자로 제한한다.
4. `scripts/qa/research-worker-smoke.sh`: main의 직렬 캐시 빌드·시간 결과와
   R2/R3의 정리 종료코드·자원 부재 검증을 결합한다. 원래 실행 실패를 보존한다.

기존 component/runtime PUML·PNG는 R2/R3 브랜치의 바이트를 그대로 병합한다.
충돌 해결은 새로운 앱 구조·서비스·데이터 권한·실행 경로를 추가하지 않는다.

## 검증과 한계

- 로컬 Python 3.12에 PyYAML이 없어 첫 preflight/import는 실패했다.
  기존 uv 격리 환경의 고정 PyYAML 6.0.2로 준비한 뒤 preflight READY를 확인했다.
- 최초 통합 CI 계약 검사에서 cleanup fixture의 시간 변수와 Web 명령 필터 불일치
  6건을 확인하고 위 결합을 수정했다. 실패 기록을 성공으로 바꾸어 해석하지 않는다.
- CI 계약 37개, cache evidence 및 dev-check 45개: 총 82개 PASS.
- 변경 Rust 테스트의 rustfmt 1.97.1 적용·검사, shell `bash -n`, 병합 차이 공백 검사 PASS.
- 로컬에서 대규모 Rust/브라우저/실제 DB·Docker 검증을 반복하지 않는다.
  push 뒤 이 병합 커밋의 기존 CI와 Research Worker Smoke workflow가 전체 검증을 수행한다.
  이 문서 작성 시 원격 push와 새 hosted 결과는 아직 미확인이다.
- 기존 후보의 CI 성공은 보존하되 새 병합 커밋의 CI 통과로 대체하지 않는다.

R2/R3 FINAL, 유예 D-06/D-07 및 Daily OFF는 유지한다. 다음 제품 작업은
[인계 문서](../handoffs/2026-10-23-r2-r3-to-r4.md)에 따른 다음 세션 R4-02/03이다.
