Execution skill: $paseo-delegate (required)
Native subagents: prohibited for worker packages

# R1 개발·배포 기준 복구 실행 계획

- 작성일: 2026-10-12 KST
- 계획 기준 커밋: `1b8dca3214ee98c4c23844b888dc4e1fde0ba4a2`
- 작업 디렉터리: `/data/worktrees/3puw275b/orange-dugong`
- 상위 작업: [개발 로드맵 R1-01~R1-09](../../ROADMAP.md#5-r1--개발배포-기준-복구)
- 상태: **계획 작성 완료, 실행 미착수.** 이 문서 작성으로 R1 작업을 완료 처리하지 않는다.

## Goal and boundaries

### 목표와 완료 기준

깨끗한 runner에서 Rust·DB·Web·Python·정책 검사와 기능 smoke를 재현하고,
동일한 검증 대상 커밋으로 배포할 수 있는 이미지·manifest·migration·복구 근거를 준비한다.

1. R1-01의 각 실패에 재현 결과, 원인 분류, 수정 소유자, 최소 회귀 검증이 있다.
2. R1-02~R1-07의 수정은 기존 제품·보안·Raw 보존 계약을 유지하며 개별 검증을 통과한다.
3. R1-08은 동일 커밋의 `CI` 필수 job과 `Research worker smoke` 기능 단계가 모두 통과한다.
   실패, 취소, skip, 미실행을 구분하고 기존 skip도 항목과 사유를 기록한다.
   기존 skip이라도 필수 완료 기준을 대신하면 해당 항목은 미완료다.
4. R1-09는 같은 커밋의 실제 이미지 ID·revision과 공식 immutable manifest를 대조하고,
   migration 순서 및 이전 릴리스로 복구할 조건을 검토한다. 빌드 계획만으로 완료하지 않는다.
5. 코디네이터가 근거를 [ROADMAP.md](../../ROADMAP.md)에 연결한 뒤 R1 종료 여부를 판단한다.

실제 운영 배포, 일일 수집·추천 재가동, 실제 Auth0 Owner 인수는 R2다.
새 KIS/OpenDART 요청, WS 인수, API 허용 범위 변경, Paper 활성화, 계좌·주문 호출은 이 계획에 없다.
R2 진단을 병행할 수 있다는 상위 로드맵의 허용은 유지하지만 이 실행 묶음에는 포함하지 않는다.

### 적용 지침과 실행 전 확인할 입력

- `/home/l1nnx/.codex/AGENTS.md`: 작업 성격별 모델·effort, 한 단계씩 상향, 결과 검증, 메모리 정책.
- [저장소 AGENTS.md](../../../AGENTS.md): 로드맵 동시 갱신, 운영 빌드 자원 정책,
  데이터·거래·비밀 경계, 구조 변경 시 다이어그램과 근거 갱신.
- [Web AGENTS.md](../../../apps/web/AGENTS.md)와 [CLAUDE.md](../../../apps/web/CLAUDE.md):
  Web 코드 수정 전 설치된 Next.js의 해당 로컬 문서를 읽는다. `CLAUDE.md`는 `AGENTS.md`를 참조한다.
- `/home/l1nnx/.agents/skills/paseo-delegate-plan/SKILL.md`와
  `/home/l1nnx/.agents/skills/paseo-delegate/SKILL.md`: 계획과 실행의 위임 계약.
  실제 worker 선택·실행·알림·복구는 실행 시점의 `$paseo-delegate` 지침을 따른다.
- 계획 작성 시 저장소 내 추가 지침은 Web 하위 두 파일만 확인했다. 작업 경로나 provider가 바뀌면
  실행자가 새 경로의 상위·하위 지침을 다시 확인한다.

현재 Paseo profile의 provider/model ID와 effort 지원 여부는 실행 시 확인한다.
정책상 모델 선택은 아래 표에 정하고, 실제 profile 매핑은
`resolve at execution from current Paseo availability`로 남긴다. 사용할 수 없는 조합을 임의로 바꾸지 않는다.

WP-1이 현재 실패와 수정 명세를 확정해야 WP-2~WP-4를 시작할 수 있다.
지원 DB upgrade 출발점, 실제 runner의 빈 cache 조건, 빌드 호스트·안전한 증거 경로는 실행 시 확인할 입력이다.
없는 값이나 이전 운영 예외의 유효성을 추측해서 채우지 않는다.

### 출발 증거

[로드맵 §3.3](../../ROADMAP.md#33-ci-관측--2026-10-11-실행-2026-10-12-재확인)의 실패 기록은
코드 기준선 `fce9b90542cf9b1a911d46e4c4d3530db7efba4c`의 관측이다.
계획 기준 커밋은 로드맵 문서 추가 커밋이다. 아래 증상을 새 커밋에서 재현한 것으로 간주하지 않는다.

| 기존 증거 | 관측한 실패 지점 | 담당 |
| --- | --- | --- |
| [CI 38136740553](https://github.com/sihioov/Lagrange/actions/runs/38136740553) | format·`kis-client` Clippy, `market_stream_transport` 컴파일 | WP-2 |
| 같은 CI | migration stage 38의 catalog sync에서 `enqueue_auth_audit` 해석 실패 | WP-3 |
| 같은 CI | Web 단위 테스트가 Chromium 설치보다 먼저 실행 | WP-4 |
| 같은 CI | Python 446 pass / 1 fail / 10 skip; Raw init 처리 문자열을 기대하는 계약 실패 | WP-4 |
| 같은 CI | candidate Compose 검사에 `RESEARCH_ENTITLEMENT_SHA256` 입력 누락 | WP-4 |
| [smoke 38136740521](https://github.com/sihioov/Lagrange/actions/runs/38136740521) | Raw init calendar harness의 호스트 Cargo registry cache 요구로 기능 단계 진입 실패 | WP-4 |

## Initial classification

분류는 코드 양이 아니라 불확실성, 공유 범위, 실패 비용, 검증 가능성을 기준으로 한다.
현재는 재현 가능한 실패 지점과 기존 검증 경로가 있으므로 `hard` 패키지를 미리 만들지 않는다.

| Package | Complexity | Basis | Confidence | Reclassification or escalation signals |
| --- | --- | --- | --- | --- |
| WP-1 | intermediate | 알려진 실패 7개 계열을 현재 커밋에서 분류하고 수정 계약을 확정하는 분석 | medium | 로그와 재현이 불일치하거나 원인을 좁힐 수 없으면 해당 조사만 재분해; 재현 불가능한 진단은 hard 재분류 |
| WP-2 | intermediate | Rust 검사·테스트 계약의 국소 수정, 컴파일과 실행으로 검증 가능 | medium | wire 의미·승인 경계 변경이 필요하거나 명세대로 두 번 수정해도 실패하면 중단·재분류 |
| WP-3 | intermediate | staged migration과 catalog/audit 함수의 순서·계약 복구, disposable DB 검증 가능 | medium | 기존 DB의 데이터 손실 위험, migration 이력 변경, 권한 설계 변경이 필요하면 hard 판단 전환 |
| WP-4 | intermediate | runner 준비·정적 계약·smoke가 workflow와 Python 검사 파일을 공유 | medium | Raw 운영 동작 변경, 새 의존성 설계, 테스트 약화가 필요해지면 명세를 다시 확정 |
| WP-5 | intermediate | 제한된 변경의 독립 코드 리뷰, 요구사항과 회귀 근거 대조 | high | R1 밖의 구조·권한 변경이 발견되면 영향 분석을 별도 패키지로 분리 |
| WP-6 | simple | 고정 커밋의 정해진 CI·smoke 실행과 기계적 결과 수집; 코드 수정 없음 | high | 새 실패는 담당 수정 패키지로 반환; 환경 접근 불가는 외부 차단으로 기록 |
| WP-7 | intermediate | 기존 공식 도구로 이미지·manifest 준비 및 provenance 수집, 운영 자원 제약 있음 | medium | builder/runbook 불일치, OOM, 불명확한 복구 조건은 진행 중단; 설계 변경은 별도 재분류 |

## Execution graph

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
| --- | ---: | --- | --- | --- | --- | --- | --- | --- |
| WP-1 | 0 | intermediate | R1-01 현재 실패 분류와 수정 명세 확정 | 소스 읽기·최소 재현, 전용 기준선 보고서 | 없음 | Codex `gpt-6.1-sol`, high — 결론이 산출물인 분석 | 실패별 원인·허용 파일·수정 계약·검증표 | 재현 명령·exit·커밋과 CI 근거 대조 |
| WP-2 | 1 | intermediate | R1-02·R1-03 Rust 검사·테스트 복구 | `crates/kis-client` Rust 파일, 사전 확정한 포맷 대상 | WP-1 수용 | Codex `gpt-6-luna`, max — 명세 확정 구현 | 최소 Rust 수정과 회귀 근거 | fmt·Clippy·대상 테스트, 전체 검증은 WP-6 |
| WP-3 | 1 | intermediate | R1-04 DB 초기화·upgrade 복구 | `deploy/db`, 승인된 신규 migration, DB runbook | WP-1 수용 | Codex `gpt-6-luna`, max — 명세 확정 구현 | catalog/migration 수정, 새 DB·upgrade 검증 | static·self-test·실제 disposable DB, serving-role 검증 |
| WP-4 | 1 | intermediate | R1-05·R1-06·R1-07 깨끗한 runner 복구 | 두 workflow, CI 계약, smoke·candidate 검사, 지정 Web 테스트 | WP-1 수용 | Codex `gpt-6-luna`, max — 명세 확정 구현 | 환경·harness 수정과 각 단계 결과 | Web 전 단계·Python·policy·기능 smoke |
| WP-5 | 2 | intermediate | 변경 전체의 독립 검토 | 통합 diff 읽기, 전용 리뷰 문서 | WP-2·3·4 수용 | Codex `gpt-6.1-sol`, high — 코드 리뷰 | 위반·누락·회귀 근거와 파일:라인 | 실제 diff·계약·개별 검사 근거 대조 |
| WP-6 | 3 | simple | R1-08 동일 커밋 통합 증거 확보 | 고정 커밋의 CI·격리 QA, 전용 인수 보고서 | WP-5 수용·후보 커밋 확정 | Codex `gpt-6-luna`, medium — 정해진 실행·수집 | 필수 job·smoke 결과와 skip 목록 | 모든 실행의 대상 커밋·실제 단계 결과 확인 |
| WP-7 | 4 | intermediate | R1-09 동일 커밋 배포 후보 준비 | 공식 빌드의 후보 이미지·새 manifest·전용 준비 보고서 | WP-6 수용 | Codex `gpt-6-luna`, medium — 기존 절차 실행; 최종 판단은 코디네이터 | 이미지 ID·revision·manifest·migration/복구 근거 | 공식 manifest 검증, 자원·서비스 건강 상태 대조 |

Wave 1의 소스 편집은 서로 겹치지 않는다. 다만 같은 호스트의 Cargo·Docker build·DB 검증·Web build는
동시에 시작하지 않는다. 코디네이터가 무거운 검증의 실행권을 한 worker에만 주고,
**WP-2 → WP-3 → WP-4 순서로 기본 예약**한다. 예약 변경은 전달된 지시와 기록으로 관리한다.
GitHub의 서로 다른 hosted runner는 이 로컬 자원 예약과 별개다.

## Worker briefs

### 모든 worker에게 포함할 공통 계약

실행자는 **이 공통 계약 전체 + 해당 WP 절 + 실제 선행 보고서와 확정 명세**를 각 프롬프트에 넣는다.
이전 대화를 볼 것이라고 가정하지 않는다. 아래 경로는 모두 작업 루트 기준이며,
전달할 때 `/data/worktrees/3puw275b/orange-dugong/` 아래 절대경로로 확장한다.
격리가 필요해 다른 Paseo workspace를 쓰면 실제 경로와 기준 커밋을 먼저 확정하고 전부 치환한다.

> Do not use native subagent, Task, Agent, team, or delegation features. Complete this assignment directly and report if it needs further decomposition.

- 작업 디렉터리: `/data/worktrees/3puw275b/orange-dugong`. 해당 경로의 지침과 상위 로드맵을 읽는다.
- WP-1 이후 구현 worker는 코디네이터가 확정한 **파일별 수정 명세**를 받는다. 아직 명세가 없는
  원인은 작업하지 않고 보고한다. 주변 리팩터링, 의존성 일괄 갱신, 새 기능을 추가하지 않는다.
- 소유 범위 밖의 파일, 다른 worker 변경, 전역 설정, 운영 `.env`·비밀 파일을 수정하지 않는다.
  `git add -A`, 다른 작업의 commit/reset/checkout, 기존 cache·volume·이미지 일괄 삭제를 하지 않는다.
- worker는 소스 변경을 보고하고 코디네이터가 검토·통합·커밋한다. `docs/ROADMAP.md`, 이 계획,
  구조 다이어그램의 통합 책임은 코디네이터에게 있다. 구조 변경 필요성을 발견하면 먼저 보고한다.
- 테스트용 DB·포트·Docker project·임시 경로는 자신의 실행 전용으로 확보한다. 생성한 자원만 정리한다.
  실제 DB, Raw/Curated, 시스템 unit, `/opt/lagrange/current`를 테스트 대상으로 사용하지 않는다.
- KIS 등 공급자 호출은 없다. fixture와 synthetic 데이터만 사용한다. 비밀·공급자 본문·원시 로그를
  보고서에 복사하지 않는다. 설치된 릴리스와 운영 서비스의 상태를 바꾸지 않는다.
- 검증 실패를 skip·`|| true`·광범위 lint 허용·assertion 삭제로 숨기지 않는다. 기존 skip은 사유를 기록한다.
- 변경 파일과 라인 범위, 명세와 달리 처리한 부분과 이유, 실행 명령·대상 커밋·환경·결과,
  미해결/후속 작업, 찾지 못했거나 검증하지 못한 것을 보고한다. 빈 항목도 명시적으로 `없음 (none)`이라 쓴다.
- 각 보고서는 아래 지정한 `docs/reviews/2026-10-12-r1-*.md`에 안전한 요약만 기록한다.
  상세 증거는 CI URL이나 접근 통제된 산출물의 식별자·hash로 연결한다. `/tmp`만을 유일한 근거로 남기지 않는다.

### WP-1 — 현재 실패 기준선과 수정 계약

- **CWD:** `/data/worktrees/3puw275b/orange-dugong`
- **분류:** intermediate / confidence medium. 알려진 실패를 재현하고 원인을 분리하는 분석이다.
  근거가 모순되거나 안정된 재현이 없으면 해당 조사만 중단하고 재분류 요청을 낸다.
- **목표:** R1-01을 완료할 수 있는 원인·수정 범위·회귀 명세를 만든다.
- **알려진 사실:** 위 출발 증거의 실패들은 이전 커밋 관측이다. Web 설치 순서와 smoke의
  호스트 cache 요구는 현재 workflow/script에서 확인했지만 수정 후 결과는 아직 없다.
- **입력:** 계획 기준 커밋, 출발 CI 두 실행, `.github/workflows/{ci,research-smoke}.yml`,
  `scripts/ci/test_ci_contract.py`, 해당 실패 파일, DB·빌드 runbook.
- **소유:** 제품 코드 변경 없음. 보고서 `docs/reviews/2026-10-12-r1-baseline.md`만 작성한다.
  재현용 disposable 산출물은 별도 작업 경로에 둔다.
- **수행:** 현재 HEAD·작업 트리와 최신 해당 SHA의 CI를 확인한다. 각 실패를 제품 결함,
  테스트 계약 불일치, CI 환경 문제로 분류하고 증상/근본 원인/미확인을 구분한다.
  전체 suite를 반복 실행하기 전에 기존 로그와 가장 작은 재현을 사용한다.
- **검증:** fmt/Clippy/transport 컴파일, stage 38 catalog, Web 브라우저 준비 순서,
  Python Raw 계약, smoke cache, candidate 필수 env의 각각에 명령·exit·도달 단계를 남긴다.
  CI 로그 재사용과 로컬 재현을 구별한다. DB 또는 Docker가 없으면 원인 해결로 보고하지 않는다.
- **산출물:** 실패별로 `원인 / 근거 / R1 ID / 수정 파일 절대경로 / 보존할 계약 /
  수정 후 기대 결과 / 최소 검증 / 남은 입력`을 작성한다. WP-2의 포맷 전용 파일 목록도 확정한다.
- **금지·보고:** 해결 코드를 미리 쓰거나 광범위 설계를 시작하지 않는다. 공통 보고 형식으로 반환한다.

### WP-2 — Rust 검사와 market-stream 테스트 복구

- **CWD:** `/data/worktrees/3puw275b/orange-dugong`
- **분류:** intermediate / confidence medium. WP-1에서 확정한 명세를 구현한다.
  같은 수정이 두 번 검증 실패하거나 wire/승인 의미를 바꿔야 하면 중단한다.
- **목표:** R1-02·R1-03의 Rust 문제를 수정하고 실제 테스트 실행으로 검증한다.
- **입력:** 수용된 WP-1 보고서와 파일별 명세. `crates/kis-client/tests/market_stream_transport.rs`의
  컴파일 실패와 Clippy 지점이 출발점이며 오류 개수를 독립 버그 개수로 해석하지 않는다.
- **소유:** `crates/kis-client/src/**/*.rs`, `crates/kis-client/tests/**/*.rs` 중 WP-1에 명시한 파일.
  그 밖의 Rust 파일은 WP-1에서 확정한 포맷 전용 목록에 한해 포맷만 수정한다.
  보고서는 `docs/reviews/2026-10-12-r1-rust.md`다.
- **수행:** 테스트 호출 계약과 production API를 대조해 실제 계약에 맞춘 최소 수정을 한다.
  허용 endpoint·프레임 의미·권한·로그 redaction을 보존한다. 기존 안전 검증을 제거하지 않는다.
- **검증:** `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`,
  `cargo test -p kis-client --test market_stream_transport --locked`, 변경한 경로의 관련 테스트.
  무거운 명령은 예약 후 실행한다. 전체 workspace 실행은 WP-6의 동일 커밋 검증까지 필요하다.
- **금지·보고:** DB·workflow·의존성·Cargo workspace 경계 변경 금지. 공통 보고 형식에
  컴파일만 확인한 검사와 실제 실행한 검사를 구별한다. 범위 확대는 코디네이터에게 반환한다.

### WP-3 — DB 초기화와 지원 upgrade 복구

- **CWD:** `/data/worktrees/3puw275b/orange-dugong`
- **분류:** intermediate / confidence medium. 기존 disposable 검증으로 확인할 수 있는 계약 복구다.
  배포된 migration 이력 변경·데이터 손실·권한 설계 변경이 필요하면 hard 여부를 다시 판단한다.
- **목표:** R1-04의 새 DB, 기존 DB upgrade, serving-role 경계를 통과시킨다.
- **입력:** WP-1 명세, `deploy/db/sync-baseline-strategy-catalog.sql`, `deploy/db/migrate.sh`,
  `deploy/db/integration-validation/README.md`, `docs/runbooks/postgres-integration-validation.md`,
  관련 migration. 기존 lane은 0038 baseline → 0039 → 0040 → 0041 및 역할 검증을 수행한다.
- **소유:** WP-1 명세에 포함된 `deploy/db/` 파일과
  `docs/runbooks/postgres-integration-validation.md`. 신규 migration이 필요하면 코디네이터가
  번호·파일·적용/복구 계약을 확정한 후 추가한다. 이미 배포된 migration은 임의로 덮어쓰지 않는다.
  보고서는 `docs/reviews/2026-10-12-r1-postgres.md`다.
- **수행:** catalog sync의 실행 시점과 audit 함수 계약을 일치시킨다. 기존 upgrade 검증과
  **빈 DB에서 현재 전체 migration을 적용하는 검증을 별도 확인**한다. 현재 커밋의 최종 migration과
  지원 출발점을 보고서에 적고, 기존 0041 lane만으로 전체 최신 DB 검증을 대신하지 않는다.
- **검증:** `bash deploy/db/integration-validation/static-check.sh`,
  `bash deploy/db/integration-validation/validate.sh --self-test`,
  `bash deploy/db/integration-validation/migration-safety-audit.sh`,
  `bash deploy/db/integration-validation/validate.sh --evidence-dir <실행전용-절대경로>`.
  추가한 새 DB 검증 명령과 근거도 명시한다. SQLx ledger, catalog sync 재실행, audit 보존,
  ownership/search_path/EXECUTE ACL/RLS, 실제 serving-role 접근, rollback guard를 확인한다.
- **금지·보고:** production DB 접속·migration 적용·role 우회·audit 실패 무시 금지.
  Cargo를 호출하는 DB 검증도 자원 예약 대상이다. 공통 보고 형식으로 결과와 `SKIP:` 유무를 반환한다.

### WP-4 — Web·Python·smoke·policy의 runner 계약 복구

- **CWD:** `/data/worktrees/3puw275b/orange-dugong`
- **분류:** intermediate / confidence medium. 공유 workflow와 계약 테스트를 한 worker가 수정한다.
  Raw 운영 동작 변경이나 새 의존성 설계가 필요하면 코디네이터에게 명세 재확정을 요청한다.
- **목표:** R1-05·R1-06·R1-07을 운영 비밀이나 개발자 호스트 cache 없이 재현한다.
- **입력:** WP-1 명세, 두 workflow, `scripts/ci/test_ci_contract.py`,
  `scripts/qa/research-worker-smoke.sh`, `deploy/compose/candidate-static-check.sh`,
  `apps/web/tests/owner-equity-v2-live-acceptance-dom.test.tsx`, Web 지침·package scripts.
- **소유:** `.github/workflows/ci.yml`, `.github/workflows/research-smoke.yml`,
  `scripts/ci/test_ci_contract.py`, `scripts/qa/research-worker-smoke.sh`,
  `deploy/compose/candidate-static-check.sh`. Web 테스트·설정은 WP-1의 파일별 목록에 있을 때만 수정한다.
  새 회귀 fixture 파일도 WP-1 수용 시 경로를 정한다. 보고서는 `docs/reviews/2026-10-12-r1-runner.md`다.
- **수행:** (1) 브라우저를 사용하는 첫 테스트 전에 Chromium과 필요한 시스템 의존성을 준비한다.
  (2) Raw의 기존 내용·권한·symlink 경계를 지키는 실제 동작을 기준으로 테스트와 스크립트를 맞춘다.
  기존 문자열 assertion을 삭제하는 것만으로 완료하지 않는다.
  (3) calendar harness가 깨끗한 runner에서 pinned 입력으로 준비되도록 하고 warm cache는 최적화로만 쓴다.
  사용자 홈 경로나 빈 cache를 다른 호스트 cache 경로로 교체하는 수정을 하지 않는다.
  (4) candidate 정적 검사에는 비밀이 없는 격리 fixture 값을 사용한다. production 필수값 검사는 유지한다.
- **검증:** workflow 순서·필수 job 계약, `python -m unittest scripts.ci.test_ci_contract -v`,
  `uv run --project nt pytest -q`, `bash deploy/compose/candidate-static-check.sh`,
  `bash scripts/qa/research-worker-smoke.sh --self-test`, 같은 script의 기본 기능 실행.
  smoke는 기존 개발자 Cargo cache가 없는 격리 환경에서도 기능 단계까지 통과해야 한다.
  Web은 `npm ci`, `npm run openapi:check --workspace @lagrange/api-server`, Chromium 설치,
  `npm run lint --workspace @lagrange/web`, `npm run typecheck --workspace @lagrange/web`,
  `npm test --workspace @lagrange/web`, `npm run build --workspace @lagrange/web`,
  `bash scripts/qa/candidate-web-e2e.sh`를 각각 기록한다. 버전·준비 단계는 workflow와 맞춘다.
- **금지·보고:** 기능 smoke를 `--static-only`로 바꾸기, skip 추가로 green 만들기, 운영 credential 주입,
  `nt` 계산 로직·Raw 운영 구현·Compose 서비스 구조의 임의 변경 금지. 공통 보고 형식에 준비/실행 결과를 나눈다.

### WP-5 — 통합 변경 독립 리뷰

- **CWD:** `/data/worktrees/3puw275b/orange-dugong`
- **분류:** intermediate / confidence high. 결론이 산출물이므로 코드 리뷰 모델을 사용한다.
  구조나 승인 경계가 달라졌으면 해당 변경을 분리하고 영향 분석을 요청한다.
- **목표:** 테스트 목적과 제품 계약을 보존한 복구인지 독립 검토한다.
- **입력:** 계획 기준점 대비 통합 diff, WP-1 수정 명세, WP-2~WP-4의 보고서와 테스트 근거.
- **소유:** 읽기 전용 리뷰; `docs/reviews/2026-10-12-r1-review.md`만 작성한다.
- **검증:** 모든 변경 파일을 대조한다. 특히 Raw 보존·권한, DB upgrade 순서·audit·RLS,
  secret 없는 Compose 입력, Chromium 설치 순서, cold-cache smoke, required job 집계를 확인한다.
  낮은 티어의 요약만 신뢰하지 않고 diff와 실제 결과를 읽는다. 필요한 재검증은 실행권을 받아 수행한다.
- **산출물:** severity·file:line·구체적 문제·영향·최소 검증을 갖춘 findings와 완료 조건별 누락 목록.
  다이어그램 대상 구조가 바뀌었는지 판정하고 해당하면 코드 근거를 명시한다.
- **금지·보고:** 직접 수정·별도 위임·범위 밖 전체 아키텍처 재설계 금지. 공통 보고 형식으로 반환한다.
  발견 사항은 원래 소유 worker에게 돌리고 수정된 diff를 재검토한다.

### WP-6 — 동일 커밋의 필수 CI·smoke 인수

- **CWD:** `/data/worktrees/3puw275b/orange-dugong`의 코디네이터가 고정한 후보 커밋 `C`
- **분류:** simple / confidence high. 명령 실행과 결과 수집이다. 새 실패는 WP-2~WP-4에 반환한다.
- **목표:** R1-08을 실제 동일 커밋의 결과로 확정한다.
- **입력:** 수용된 독립 리뷰, 깨끗한 후보 커밋 `C`, 그 커밋을 가리키는 remote ref,
  현재 두 workflow의 필수 단계, WP-1~WP-5 증거.
- **소유:** QA 실행 및 `docs/reviews/2026-10-12-r1-acceptance.md`. 코드·workflow 수정 없음.
- **수행:** 먼저 `C`의 기존 실행이 있는지 확인한다. 필요한 경우 실행 요청 범위 안에서
  두 workflow의 `workflow_dispatch`로 고정 ref를 실행한다. PR merge 커밋·옛 main·다른 HEAD의
  결과를 섞지 않는다. 실행 URL, head SHA 및 실제 checkout 대상, 각 단계 결과를 회수한다.
- **검증:** `policy`, `format`, `clippy`, `web`, `workspace-tests`, `python-tests`,
  `postgres-integration-validation`, `required`, 별도 `research-smoke` 모두를 대조한다.
  `cargo test --workspace --locked --no-fail-fast`가 컴파일 이후 실제 실행됐는지,
  DB를 요구하는 검증이 DB 부재로 건너뛰지 않았는지, Web E2E와 smoke 기능 단계가 도달했는지 확인한다.
  clean dependency 설치, 빈 개발자 cache 조건, 기존 skip의 이름·이유를 기록한다.
- **금지·보고:** 자동 main merge, 코드를 고친 뒤 이전 결과 재사용, 미실행을 pass로 기록하는 행위 금지.
  모든 필수 단계가 통과해도 운영/실데이터/Owner 인수라고 보고하지 않는다. 공통 보고 형식으로 반환한다.

### WP-7 — 이미지·manifest와 릴리스 준비 근거

- **CWD:** `/data/worktrees/3puw275b/orange-dugong`에서 승인된 실제 빌드 workspace로 연결한다.
  코디네이터가 **WP-6의 `C`에 고정된 깨끗한 절대경로**를 확정해서 프롬프트에 넣은 뒤 시작한다.
- **분류:** intermediate / confidence medium. 기존 절차의 실행과 증거 수집이며 최종 배포 판단은 하지 않는다.
  OOM·서비스 이상·builder 계약 충돌·복구 불명확 시 즉시 중단하고 원인을 보고한다.
- **목표:** R1-09의 실제 배포 후보를 준비한다. 운영 적용은 하지 않는다.
- **입력:** WP-6 보고서와 `C`, `scripts/ops/build-production-images.sh`,
  `scripts/ops/lib/release-image-manifest.sh`, `scripts/ops/README.md`,
  `docs/runbooks/production-release-and-backup.md`, WP-3의 migration 근거,
  현재 유효한 빌드 권한·호스트·자원 조건·이전 릴리스 식별자.
- **소유:** 해당 `C`의 후보 이미지와 새 공식 manifest 출력 경로, 실행 전용 빌드 상태·안전한 증거,
  `docs/reviews/2026-10-12-r1-release-readiness.md`. 소스·활성 릴리스·systemd 활성화 설정은 수정하지 않는다.
  실제 외부 출력 경로는 실행 전 확정한다. 이미 존재하는 manifest를 덮어쓰지 않는다.
- **수행:** 공식 builder의 `--plan`, `--preflight` 결과와 실행 범위를 확인한 다음,
  이미 허용된 image prebuild 범위에서만 실제 빌드·검증을 수행한다. 이 계획 자체를 운영 작업 허가로 쓰지 않는다.
  필요한 권한·호스트 입력이 없으면 미완료 근거와 해소 조건을 남기며 완료 처리하지 않는다.
- **자원 규칙:** 14 GiB 운영 호스트에서는 2~3서비스 단위로 계획하되 Compose 호출은 매번 단일 서비스다.
  `COMPOSE_PARALLEL_LIMIT=1`, `CARGO_BUILD_JOBS=2`, 낮춘 CPU/I/O 우선순위의 background systemd를 쓴다.
  배치 사이 memory/swap/OOM/build exit/서비스 health와 잔존 compiler를 확인한다.
  기존 runbook의 유효한 lock·resume·공식 검증 경로를 사용하고 이전 예외를 자동 연장하지 않는다.
  캐시 준비 후 공식 builder로 전체 대상 이미지를 검증하고 immutable manifest를 생성한다.
- **검증·산출물:** 공식 builder가 정의한 모든 대상의 image ID·OCI revision·commit tag가 `C`와 일치하는지,
  manifest hash·보관 위치·권한, 실패/재개 여부, 생산 서비스 건강 상태를 기록한다.
  migration은 순서·사전조건·백업/복구 근거·되돌릴 수 없는 경계를 정리하고 이전 릴리스 식별자를 대조한다.
  이미지 ID를 registry digest라고 표현하지 않는다. rollback 명령 존재만으로 DB 복구 가능성을 주장하지 않는다.
- **금지·보고:** 설치 helper의 운영 `--apply`, `--rollback`, 서비스 start/restart, timer 활성화,
  실 DB migration, live profile 사용 금지. 공통 보고 형식으로 반환하고 R2에서 적용할 근거만 연결한다.

## Coordinator gates

### 1. Pre-launch

1. 현재 요청은 계획 작성이다. 구현 worker를 시작하지 않고 모든 WP와 R1 체크를 미완료로 둔다.
   실행 요청이 오면 기존 사용자 승인 범위를 유지하되 계획을 새 권한으로 해석하지 않는다.
2. 실행 시 현재 HEAD·변경 파일·지침을 다시 확인하고 WP-1의 실제 입력 커밋을 고정한다.
   사용자 변경을 보존한다. R1-01을 담당자와 함께 DOING으로 갱신한다.
3. `$paseo-delegate`와 Paseo MCP·현재 profile을 확인한다. 실행 skill 또는 MCP가 없으면
   해당 선행 조건을 보고하고 worker 시작 전에 멈춘다. native worker나 CLI 실행으로 대체하지 않는다.
4. 모델은 위 표의 작업 성격에 따라 선택한다. 필요한 provider/model·effort 조합 지원 여부를 확인한다.
   같은 수정의 반복 실패, 맥락 이탈, 검증 불일치가 발생할 때만 한 단계 상향한다.
   모델과 effort를 동시에 바꾸지 않으며 변경 이유를 기록한다.
5. 코디네이터는 worker ID·실제 모델/effort·workspace·소유 파일·진행 상태·pending 요청·보고서 위치를 관리한다.
   알림·idle 확인·후속 프롬프트·복구는 `$paseo-delegate`의 현행 절차를 따른다.
   이 문서는 자체 실행 명령이나 별도 모니터링 경로를 만들지 않는다.

### 2. Per-wave integration

1. **Wave 0:** 모든 실패 계열에 근거와 수정 계약이 있는지 직접 확인한다. WP-2~WP-4의 허용 파일을
   절대경로로 확정하고 겹침이 없는지 검사한다. 미확정 원인은 해당 branch만 보류한다.
   해소 가능한 명세 문제를 사용자에게 그대로 넘기지 말고 코디네이터가 해결한다.
2. **Wave 1:** 독립 편집만 병행하고 무거운 검증은 예약한다. 보고서의 명령·exit·실제 diff를 대조한다.
   낮은 티어 결과는 실제 검사 통과 또는 직접 확인을 근거로 수용한다.
   범위 밖 수정은 제거를 단정하지 말고 원인과 필요한 계획 변경을 판단한다.
3. **Wave 2:** 독립 리뷰의 차단 사항을 원래 worker에게 반환한다. 미해결/미검증이 없는지 명시적으로
   확인한다. 구조 변경이면 영향을 받는 `.puml`과 evidence를 같은 변경에서 갱신하고 PNG를 로컬 재생성한다.
   구조 변화가 없으면 다이어그램은 그대로 둔다.
4. **Wave 3:** 통합 변경과 필요한 로드맵 상태를 커밋한 뒤 검증 후보 `C`를 고정한다.
   WP-6 실행 중 소스나 workflow를 바꾸지 않는다. 실패를 수정하면 새 후보 `C2`로 해당 검사와
   최종 필수 CI·smoke를 다시 통과시킨다. green을 만들기 위해 미리 main에 merge하지 않는다.
5. **Wave 4:** WP-6의 동일 `C`만 빌드한다. 이미 허용된 빌드 범위와 구체적인 입력을 확인하며,
   필요한 새 외부 권한이 생긴 경우에만 그 작업을 분리한다. 단순히 R1로 이동했다는 이유로 재승인을 요구하지 않는다.
   manifest와 이미지가 없는 상태에서 R1-09를 완료하지 않는다.

### 3. Final end-to-end acceptance

- 모든 owned worker가 실제 idle/종료 상태이고 결과·오류 보고서가 회수됐으며 pending 요청이 해결됐다.
- R1-01~R1-09의 각 완료 기준과 증거가 충족됐다. 특정 검증이 미완료면 해당 항목과 R1을 열어 둔다.
- 동일 `C`의 CI·기능 smoke·이미지·manifest를 연결하고 commit 혼용이 없는지 직접 확인했다.
- R1-02~R1-07은 개별 수정 근거와 최종 `C`의 검증을 함께 연결한다. 검증 이전에는 완료를 과장하지 않는다.
- 코디네이터가 ROADMAP의 체크박스, 다음 작업, 단계 상태, 변경 이력을 실제 결과대로 갱신한다.
  커밋 후 얻은 CI·빌드 결과는 **후속 문서 커밋에서 검증 대상 `C`를 명시**한다.
  문서 커밋의 HEAD를 검증 대상 `C`로 바꾸어 쓰거나 자기 해시를 맞추려고 반복 amend하지 않는다.
- R2 전달물은 후보 `C`, CI·smoke URL, manifest 위치/hash와 이미지 ID, migration/복구 조건,
  남은 운영 전제다. R1 종료는 실제 배포·실데이터 수신·Owner 인수 완료를 뜻하지 않는다.

## Execution tracking

체크는 계획 작성 여부가 아니라 해당 worker 결과가 수용됐는지 표시한다.
R1의 최종 상태는 상위 ROADMAP에서 관리하며 두 문서가 어긋나면 근거를 확인해 함께 고친다.

- [ ] WP-1 — R1-01 실패 기준선·수정 명세 수용
- [ ] WP-2 — R1-02·R1-03 Rust 수정·개별 검증 수용
- [ ] WP-3 — R1-04 DB 수정·개별 검증 수용
- [ ] WP-4 — R1-05·R1-06·R1-07 runner 수정·개별 검증 수용
- [ ] WP-5 — 독립 리뷰 수용
- [ ] WP-6 — R1-08 동일 커밋 CI·기능 smoke 인수
- [ ] WP-7 — R1-09 실제 배포 후보 준비 인수

| 날짜 (KST) | 변경 | 근거 |
| --- | --- | --- |
| 2026-10-12 | R1을 7개 패키지·5개 wave로 계획. 구현·worker 실행은 미착수 | Owner의 `$paseo-delegate-plan` 요청; 상위 ROADMAP R1; 계획 기준 커밋과 출발 CI |
