Execution skill: $paseo-delegate (required)
Native subagents: prohibited for worker packages

# R4-06 남은 작업 통합 계획

2026-10-27. Owner 지시: “적당히 다시 계획을 좀 조정해봐 너무 비효율적으로 하는 거 같아”.
작업 트리: /data/worktrees/3puw275b/short-bobcat.
이 계획은 2026-10-26 구현 계획의 **이후 실행 순서와 작업 묶음**을 대체한다.
채택된 금융 규칙, 보안·권리 경계, R4 완료 조건, 과거 증거와 미실행 표시는 대체하지 않는다.

## 목표와 현재 위치

다섯 전략의 계산·저장·API/Web 소스를 공식 실행기와 출시 경로에 연결하고,
동일 후보의 실제 검증을 거쳐 설치·Owner 인수까지 닫는다.
R4-01~05 완료와 R4-06 DOING을 유지한다. 소스 수용, 실행 검증, 운영 설치, Owner 확인은 별개다.

D2K correction1은 실제 idle/activeTurn null/pending0에서 회수했다.
원본을 유지한 독립 보존 검사와 F1–F6 변경 diff 직접 검토를 마쳤으며,
39 Rust 회귀는 NOT RUN이다. 수용 근거는
`target/r4-expansion-implementation/replan-20261027/d2k-c1-source-review.md`.
추가 D2K 검증 전용 작업자나 correction2를 만들지 않는다.
manifest의 오래된 final/postreceipt 파일명은 원본을 수정하지 않고 coordinator 회수 색인에서
실제 terminal receipt로 명시적으로 연결했다.

## 즉시 바꾸는 진행 방식

1. 남은 구현을 **실행기 통합 WP-5C3E**, **배포·운영 통합 WP-5C4** 두 묶음으로 만든다.
   독립 파일은 두 작업자가 병렬 처리한다. 같은 파일이나 아직 합의하지 않은 ABI는 병렬 수정하지 않는다.
2. 다음 파일 한 개를 정하기 위한 READONLY 분석 작업자를 반복 추가하지 않는다.
   기존 C0/D0/D2A~K 보고서와 소스를 재사용하고, coordinator가 한 번의 짧은 접점 표로 결정한다.
   필수 정보가 없으면 정확한 미결 항목을 적고 해당 부분만 차단한다.
3. 각 작업자는 구현과 의미 있는 회귀 소스를 함께 제출한다. 작은 수리는 같은 담당자 또는
   coordinator가 처리한다. 같은 원인 두 번 실패·공유 계약 충돌만 해당 분기를 재계획한다.
   하나의 재검토에서 발견된 문제는 묶어 반환한다. 필수 결함을 리뷰 횟수 제한 때문에 남기지 않는다.
4. 현재 SOURCE/offline 실행 한도는 유지한다. 컴파일/DB/hosted 실행을 이 계획이 새로 허용하지 않는다.
   다만 **첫 실행 검증 준비를 뒤로 미루지 않는다**. WP-6B가 현재 후보의 명령·대상·필요 권한을 먼저
   확정하고, 해당 실행 권한이 확인되면 기존 hosted 환경에서 가장 먼저 컴파일/집중 검증을 한다.
   허용되지 않은 실행을 로컬이나 합성 Python 대체 실행으로 우회하지 않는다.
5. heartbeat는 상태·승인 대기·완료 알림 누락을 확인하는 용도다. 실행 중 매 tick 전체 파일 해시를
   재검사하거나 이전 증거 트리를 다시 복사하지 않는다. dispatch/인수 경계에서 필요한 보호 핀을
   한 번 확인하고, 중간 재확인은 변경·실패·이탈 징후가 있을 때만 한다.
6. 실패 원본과 기존 보존 의무는 유지한다. 새 패키지는 기존 manifest를 참조하고 새 변경·명령·실패만
   추가한다. 오래된 전체 snapshot을 재귀 복제하는 새 감사 체계는 만들지 않는다.
7. 진행 보고는 PASS 개수 대신 “연결된 사용자 기능 / 남은 필수 차단 / 다음 실행 검증”으로 한다.
   전체 작업량이 고정되지 않은 3~7일 추정은 납기 근거로 사용하지 않는다.

## 초기 분류

| Package | Complexity | Basis | Confidence | Reclassification or escalation signals |
|---|---|---|---|---|
| WP-5C3E | hard | 기존 startup 관측·실제 queue transaction·단일 health 소유자·종료를 한 경로로 결합 | medium | exact-attempt/reader/DB 세션 계약 모순, 소유 범위 밖 의미 변경, 같은 원인 두 번 실패 |
| WP-5C4 | hard | 설치·API fence·reader floor·backup/rollback이 같은 운영 상태 전이를 공유 | medium | 기존 Daily/불변 release 의미 변경, 실제 외부값 필요, 같은 원인 두 번 실패 |
| WP-6B | intermediate | 기존 harness/등록을 재사용해 실제 변경 후보와 검증 대상을 연결 | medium | 누락된 실제 테스트 target, harness 계약 모순, 실행 권한·환경 미확보 |

작업자 모델/effort는 dispatch 때 현재 Paseo 목록과 profile notes를 확인해 저장소 규칙을 적용한다.
기존 D2K Luna/max/auto-review 설정은 변경하지 않는다. 새 다중 모듈 통합은 단순 반복 구현과
구분하며, 실행 검증 없이 신뢰해야 하는 구현은 저장소의 Terra/xhigh 선택 근거를 적용한다.
정확한 사용 가능 ID를 확인할 수 없으면 추측하지 않는다. 리뷰는 Sol/xhigh 규칙을 따른다.
모델과 effort를 동시에 올리는 임의 재시도나 full-access 전환은 없다.

## 실행 그래프

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
|---|---:|---|---|---|---|---|---|---|
| WP-5C3E | 24 | hard | 공식 parent가 실제 startup/claim/lease/drain 근거를 소비하는 하나의 경로 | 아래 Rust 명시 목록 | D2K 회수, G0 접점 결정 | 현재 Paseo 확인 후 저장소 구현 규칙 | private startup+parent, 기존 runner 관측 연결, 회귀 SOURCE | diff/고정 rustfmt 및 허용된 static; 이후 hosted Rust/PG |
| WP-5C4 | 24 | hard | 동일 config/health ABI의 OFF→설치→start→drain→backup/rollback 경로 | 아래 운영 명시 목록 | G0 CLI/config/health ABI; E와 파일 비중복 | 현재 Paseo 확인 후 저장소 구현 규칙 | overlay·installer·ops·backup 통합 SOURCE | 허용된 static; fixture/실제 검증은 각 권한 확인 뒤 |
| WP-6B | 24 준비, 25 최종 | intermediate | 현재 코드부터 검증 경로 준비, E/C4 후 동일 후보 최종 검증 | 기존 4개 harness 파일 + 새 private 실행 명세/결과 | 준비는 즉시; 실행은 권한/후보 확정; 최종은 E/C4 | 현재 Paseo 확인 후 검사/리뷰 역할 규칙 | 정확 target/명령/권한 표, 후보별 실제 결과 | 기존 CI required·PG·Web·smoke 전부 보존 |

기본 동시 작업자는 E/C4 두 명이다. WP-6B의 초기 명령·권한 표는 coordinator가 기존
WP-6A 결과에서 준비한다. 대량 실행/로그 회수는 별도 허용된 실행 단계에서 한 담당자가 맡는다.
별도 분석 전용 worker 세 개를 먼저 띄우지 않는다.

## Worker brief — WP-5C3E 실행기 통합

cwd는 위 작업 트리. 목표는 checked config에서 실제 parent 종료까지 연결된 경로 하나다.
SE는 `crates/job-queue/src/owner_beta/simulation_execution`를 뜻한다.

쓰기 소유권은 다음으로 제한한다.
- `crates/job-queue/src/bin/owner-beta-simulation-runner.rs`
- `SE/mod.rs`, `SE/runner.rs`
- NEW `SE/process_startup.rs`, `SE/process_startup/tests.rs`
- NEW `SE/process_parent.rs`, `SE/process_parent/tests.rs`
- `crates/job-queue/src/queue.rs`: 기존 API를 보존하는 exact-attempt lease 경로만
- `crates/job-queue/tests/owner_beta_simulation_process.rs`
- `crates/job-queue/tests/owner_beta_simulation_execution.rs`

입력: 기존 D0 report sections2–4, D2A section3의 no-claim 정정, C3A/B/C1/C2/C4와
D1/D2B/D2D/D2F/D2G/D2I/D2K의 채택된 API 및 G0 접점 표.
기존 reader/provenance/관측 모듈은 재구현하지 않는다. v1/v2 금융 bytes, publication rights,
기존 queue API, migration/권한·런타임 소유권은 유지한다.

필수 행동: sealed 입력의 실제 승인 경로와 config 바인딩, 기존 관측들의 정확한 한계,
성공한 recovery, 단일 !Send writer 소유권, ack 이후 진행, exact-attempt lease,
epoch 무효화, heartbeat 종료 회수, signal/fatal 우선순위, commit/cleanup 불확실성 보존.
generic Idle·pool 연결·timer를 StartupVerified/CycleVerified로 바꾸지 않는다.
실제 no-candidate SELECT와 확인된 rollback은 그 한정된 사실만 나타내며, “commit” 단어를
얻기 위한 별도 빈 transaction은 추가하지 않는다. 서로 다른 세션 관측을 동일 snapshot이나
모든 pool 연결의 권한 증명으로 합치지 않는다.
미충족 startup 의무가 있으면 admission은 닫힌 상태로 남는다.

회귀 SOURCE: 실패 startup의 no-claim, claim/lease commit 실패, old epoch, 세 종류 Idle,
full/closed/lost ack, 취소와 동일 await 오류, 첫 fatal 유지, 미확정 cleanup/commit 후 다음
claim 차단, 정상 종료와 legacy 경로 보존. 사용하지 않는 범용 관측 프레임워크는 만들지 않는다.

## Worker brief — WP-5C4 배포·운영 통합

같은 cwd. C0의 C4–C7을 하나의 운영 흐름으로 묶는다.
기존 C1 release/config와 C2 runtime adapter를 재사용한다.

쓰기 소유권:
- `deploy/runtime/owner-beta-simulation-entrypoint`, `crates/job-queue/Dockerfile.owner-beta-simulation-runner`
- `deploy/compose/compose.yml`, NEW `deploy/compose/compose.owner-beta-simulation.yml`
- `deploy/secrets/provision-runtime-secrets.sh`, `scripts/ops/validate-production-config.sh`
- `deploy/compose/.env.example`, `.env.example`
- `scripts/ops/deploy-production-release.sh`, `scripts/ops/compose-release.sh`
- `scripts/ops/production-ops-self-test.sh`, `scripts/ops/production-ops-static-check.sh`
- NEW `scripts/ops/lib/owner-beta-simulation-operations.py`
- NEW `scripts/ops/test_owner_beta_simulation_operations.py`
- `scripts/ops/test_release_simulation_image.py`, `scripts/ops/test_release_simulation_compose.py`
- `scripts/ops/run-production-backup.sh`, `scripts/ops/lib/production-backup-tool.py`
- `scripts/ops/lib/owner-beta-daily-backup.py`, `scripts/ops/lib/owner-beta-daily-backup-files.py`
- `deploy/db/daily-backup-verify.sh`, `deploy/db/owner-beta-daily-backup-audit.sql`
- `scripts/ops/owner-beta-daily-backup-self-test.py`, `scripts/ops/production-backup-transition-self-test.py`
- NEW `scripts/ops/test_owner_beta_simulation_backup.py`

입력: C0 보고서의 기존 채택된 운영 흐름, C1/C2 interface, G0 ABI, 기존 release/backup harness.
base는 OFF/build-only로 유지하고 명시적 overlay만 사용한다. 독립 commit/config/receipt pin,
worker secret, 고정 input mount, 단조 reader floor, 정확한 process/generation 식별,
API 선행 maintenance fence, 실패 뒤 OFF/no-auto-resume, v1/v2 이력을 보존한 호환 rollback,
simulation-only backup을 연결한다. 새 프레임워크, 기존 Daily 의미 변경, 이력 삭제,
기존 down 실행은 범위 밖이다. SOURCE 편집과 허용된 static만 수행하며 실제 secret·입력·
Docker·DB·API 중단·설치·backup은 실행하지 않는다. 외부값은 반드시 제공되어야 하는
입력으로 취급하고, 없으면 거절한다.

coordinator는 E/C4 통합 때 기존 production/backup runbook과 구조도를 실제 배선 근거에
맞춰 같은 변경으로 갱신한다. PNG 생성에 필요한 Docker는 허용된 단계에서 실행하며
미생성 상태를 숨기지 않는다. 새 범용 builder/backup 형식을 만들지 않고 기존 형식의
지원 여부를 명시한다.

## Worker brief — WP-6B 검증 통합

같은 cwd. 기존 WP-6A 네 파일만 쓰기 후보:
`deploy/db/integration-validation/validate.sh`,
`deploy/db/integration-validation/static-check.sh`,
`scripts/ci/build-r4-verification.sh`,
`scripts/ci/test_ci_contract.py`.
나머지는 읽기 전용이며 새 private 검증 명세·원본 결과만 추가한다.
새 workflow가 필요하다고 추측하지 않는다. 현 harness로 해결 불가능하면 정확한 계약 충돌을 보고한다.

D2K까지의 현재 코드로 먼저 컴파일/집중 테스트 명령과 필요한 환경·권한을 준비한다.
E/C4 후 같은 후보 SHA에서 기존 CI required, Clippy/workspace, 전용 PG v1/v2,
API/Web/OpenAPI, research smoke·빌드 출처를 확인한다. 최종 CI는 캐시나 정적 검사로 대체하지 않는다.
실행 실패는 같은 담당자가 원인 하나를 고쳐 해당 검사를 재실행한다.
기존 결과를 재사용할 때는 동일 입력과 허용된 결정적 검사만 REUSED로 표시한다.
동적/DB/browser/final CI는 해당 실행 단계에서 실제 수행한다.
승인 입력 검증은 새 전달물·실행 수·자원·정리 조건이 구체화된 별도 실행 범위를 사용한다.
소진된 fakeDocker2/2와 과거 R4-05 실행 한도를 재사용하지 않는다.

## Coordinator gates와 미결 항목의 처리

**G0 — dispatch 전 한 번:** 전체 재분석 대신 현재 소스의 정확한 접점만 표로 확정한다.
official CLI의 독립 pin 전달, startup 필수 관측/승인 reader, DB 연결별 설정 범위,
Owner 불일치 처리, attempt lease predicate, ack/epoch/종료, C4 호출·health ABI가 대상이다.
해결되지 않은 필수 의무는 명시적으로 BLOCKED로 남기고 unrelated 소스 작업은 진행한다.
이 계획은 아직 정의되지 않은 함수 시그니처나 실제 host/config 값을 발명하지 않는다.
정확한 ownership/freeze·모드를 확인한 뒤 notified Paseo로 dispatch한다.

**G1 — 두 통합 묶음 인수:** 보고와 실제 diff/회귀를 한 번의 통합 리뷰에서 대조한다.
작은 수리는 기존 담당자가 처리한다. 동결 위반·실패 원본·미검증 항목은 보존한다.
D2I의 command execute/discard 한계는 WP-6B의 명시적 검증 차단으로 넘기며, 실제 경로에 같은
계약이 필요하면 E dispatch 전 그 두 source만 별도 허용하는 coordinator disposition을 남긴다.
해결된 것으로 숨기거나 D2I 전체 분석을 다시 하지 않는다.

**G2 — 실행 검증:** SOURCE/offline만 허용된 상태와 실제 컴파일/hosted/DB 실행 권한을 분리한다.
권한이 없으면 준비된 정확 명령/후보/한도를 한 곳에서 제시한다. 권한 공백을 새 소스 분석으로
덮지 않는다. 필요한 승인 요청은 검토 가능한 실행안이 완성된 뒤 한 번으로 모은다.
새 provider·거래·권리 확대·실제 입력 접근·설치·가용성 영향은 묵시 승인하지 않는다.

**G3 — 출시/Owner 인수:** 실제 설치, drain/rollback/backup/복원, 승인 입력 결과와 Owner 사용을
각 증거로 닫는다. 위 단계 통과 전 R4-06/R4 전체 DONE을 표시하지 않는다.

| 남은 항목 | 닫는 곳 | 미완료 때 의미 |
|---|---|---|
| 입력/권리/reader·startup·parent·lease·cleanup | E + 실제 검증 | 공식 admission/활성화 미허용 |
| overlay·secret·installer·API fence·floor·ops·backup | C4 + 실제 검증 | 운영 설치/복구 인수 미완료 |
| compile/SQLx/PG 표현·RLS/RI/ACL·body/WHEN 동작·v1/v2 회귀 | 6B 기존 harness와 필요한 직접 회귀 | 관측 SOURCE만으로 실제 의미 통과 불가 |
| 호스트 mount/권리 독립 근거·freshness·실제 자원/종료시간 | 출시 전 실제 운영 확인 | 미관측 값은 UNKNOWN, 안전한 기본값 발명 금지 |
| 일반 full-catalog/body 정적 의미 증명, 미채택 권고 | 별도 미채택/후속 목록 | 자동 출시 필수 요구로 승격하지 않음; 기존 채택 의무는 유지 |
| 운영 설치·Owner 확인 | G3 | R4-06 DOING 유지 |

각 worker 보고 형식: 변경 파일/라인, brief 이탈과 이유, 실제 실행 검사와 결과,
미해결 후속, 확인하지 못한 사항(없으면 none). 모든 회귀는 실행 전 NOT RUN이다.
Do not use native subagent, Task, Agent, team, or delegation features. Complete this assignment directly and report if it needs further decomposition.

## 감독과 기록

### G0 dispatch 결정 (2026-10-27)

[통합 접점 및 초기 검증표](2026-10-27-r4-integration-interface.md)를 SOURCE 계약으로 채택했다.
공식 CLI의 독립 3개 pin, Owner 한정 claim, 연결별 DB 정책, startup/epoch/ack/drain,
운영 호출 및 WP-6B의 첫 컴파일·집중 검증 대상을 확정했다. 실제 실행 권한은 늘리지 않는다.
E의 queue.rs 범위에는 기존 API를 보존하는 공식 Owner 한정 claim을 추가한다.
확인된 공통 execute/discard 문제는 E 안에서 ACL/migrations/routine/RLS/trigger 다섯 모듈과
각 private tests.rs의 명령 응답 처리만 수정한다. catalog SQL/정책/decoder는 재작성하지 않는다.
trigger와 configured-input-rights의 관측값에는 통합에 필요한 읽기 전용 getter만 추가할 수 있다.
configured-input-rights.rs와 해당 tests.rs를 E 소유 목록에 추가한다. 원본 증거는 유지한다.
두 통합 작업은 hard/medium, 현재 사용 가능한 gpt-6.1-terra/xhigh/auto-review로 배정한다.
등록된 프로필은 Astra/high 코디네이터 전용이라 구현 프로필로 전용하지 않았다.
계약·쓰기 범위·보존 핀은 새 wave24-control에서 한 번 고정하고, 완료 경계에서 한 번 대조한다.

D2K 종료/산출물 회수 후 새 계획을 원장과 로드맵에 반영한다.
후속 heartbeat는 이 계획과 owned-workers의 현재 scope를 먼저 읽는다.
오래된 schedule의 “다음 최소 관측 모듈” 흐름으로 돌아가지 않는다.
알림을 기본으로 유지하고 fallback은 bounded cadence로만 확인하며 마지막 만료 전에 갱신한다.
기존 fallback은 새 감독이 확보된 뒤 해제한다. 작업 완료나 외부 인계 없이 감독을 없애지 않는다.
