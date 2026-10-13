Execution skill: $paseo-delegate (required)
Native subagents: prohibited for worker packages

# 개발 효율 개선 실행 계획

- 작성일: 2026-10-13
- 조사 기준: `b0a2579bef472ae6b604405d0ab6f50255786dd4`, `orange-dugong`의 깨끗한 작업 트리
- 작업 디렉터리: `/data/worktrees/3puw275b/orange-dugong`
- 상태: **1단계 완료, 2단계 구현·focused 검증 완료 — 실제 cold/warm CI 검증 대기**
- 순서: **1단계 — 항목 1~3 → 2단계 — 항목 4~5**
- 실행 단위: 3개 패키지. 각 담당자가 구현, 관련 회귀 검증, 사용 예시까지 완료한다.

## Goal and boundaries

목표는 작은 오류 때문에 위임과 검증을 반복하는 비용을 줄이고, 무거운 개발 검증을 기존 GitHub Actions에서 재현 가능하게 수행하는 것이다. 다음 다섯 항목만 다룬다.

| 단계 | 개선 항목 | 완료를 판단할 증거 |
|---|---|---|
| 1 | 1. 위임·직접 처리·재계획 규칙 | `AGENTS.md`에 충돌 없는 기준, 동일 담당자의 수정·검증 책임, 작은 오류의 즉시 복구 기준 |
| 1 | 2. 빠른 사전 점검 | 정상 환경 통과, 도구·환경변수·경로·권한·포트 오류를 비싼 명령 실행 전에 구분하는 회귀 테스트 |
| 1 | 3. 단계별 검증·결과 재사용 | 변경 경로→검사 대응표, 실행 가능한 공통 명령, 입력 변경 시 재사용 거부, 대상 커밋·명령·결과 기록 |
| 2 | 4. CI 캐시 | 실제 CI에서 최초 저장과 재사용 확인, 양쪽 필수 검사 통과, 적중·크기·소요시간 비교 |
| 2 | 5. 병렬화·실행 환경 | 에이전트 수와 빌드 수를 구분한 정책, 로컬 중복 실행 차단 검증, 격리된 hosted job 실행 기록 |

### 적용 지침과 범위

- `/home/l1nnx/.codex/AGENTS.md`, 저장소 `AGENTS.md`, `$paseo-delegate-plan`의 지침을 확인했다. Web 경로에 적용되는 `apps/web/AGENTS.md`와 `apps/web/CLAUDE.md`도 확인했다.
- 전역 지침 파일과 스킬 파일은 수정하지 않는다. 프로젝트 `AGENTS.md`는 전역 모델 선택 기준을 복제하지 않고 이 저장소의 작업·검증 절차를 보완한다.
- `docs/ROADMAP.md`는 현재 프로젝트 지침 확인용으로 읽었다. 이 개선 작업의 진행은 아래 `WP-*` 체크와 이 문서에서 관리하며 제품 개발 단계와 묶지 않는다.
- 최초 계획 요청의 산출물은 **이 Markdown 파일 하나**였다. 이후 2026-10-13 Owner의 `$paseo-delegate 작업 진행해` 요청에 따라 이 계획의 구현·관련 검증·커밋과 같은 후보의 CI cache 실증을 진행한다. main 병합과 운영 작업은 포함하지 않는다.
- 후속 실행에는 `$paseo-delegate`가 필요하다. 실행 시 스킬을 찾을 수 없으면 worker 실행 전에 그 전제만 보고하며 다른 위임 수단으로 대체하지 않는다.
- 제품 기능, 의존성 버전 업그레이드, 운영 배포·활성화, provider 호출, 거래 기능, 추가 빌드 머신 도입은 범위 밖이다. 계획 자체가 외부 작업의 신규 실행 권한은 아니다.
- 운영 빌드는 기존 규칙을 유지한다: 서비스 하나씩 Compose 실행, `COMPOSE_PARALLEL_LIMIT=1`, `CARGO_BUILD_JOBS=2`, 낮은 우선순위의 background systemd 실행, 배치 사이 메모리·swap·OOM·종료 상태·서비스 health 확인.
- 필수 CI 검사와 `required` 집계를 유지한다. 캐시 적중이나 변경 경로 필터로 최종 검사를 생략하지 않는다. 구조 변경이 예상되지 않으므로 다이어그램은 수정 대상이 아니다. 구현 중 구조 변경이 필요해지면 해당 분기의 범위를 먼저 재검토한다.

### 조사 결과와 재사용 대상

| 확인한 파일 | 현재 상태와 계획상 처리 |
|---|---|
| `AGENTS.md` | 운영 빌드·provider 경계·로드맵·다이어그램 규칙이 있다. 개발 작업 처리와 검증 절차를 한 섹션으로 추가한다. |
| `scripts/check-pins.sh`, `scripts/validate-foundation.sh` | pin과 저장소 구조 검사를 재사용한다. `check-pins.sh --manifest-only`와 선택 영역의 설치 도구 확인을 구분한다. |
| `scripts/verify-all.sh`, `scripts/verify-all.ps1` | 기존 검사 진입점이다. Rust 검사에 CI의 `--locked`, workspace 테스트 feature를 정합화한다. 기존 검사 항목은 보존한다. 두 파일의 기존 검사 집합이 완전히 같다고 가정하지 않는다. |
| `.github/workflows/ci.yml` | 검사 job 7개와 `required` 1개. Rust·Web·Python·DB 검증이 이미 hosted runner에서 실행된다. 새 검증 시스템으로 교체하지 않는다. 명시적인 다운로드/빌드 캐시 복원·저장 설정이 없다. |
| `scripts/ci/test_ci_contract.py` | 현재 `actions/cache`, `target/`, artifact upload를 금지하고 Web 명령 목록을 정확히 비교한다. 캐시 도입 시 허용 경로와 실행 순서를 검증하는 계약으로 함께 수정해야 한다. |
| `package.json`, `apps/web/package.json` | 기존 npm workspace 명령을 재사용한다. Biome을 저장소 root에서 직접 호출하지 않고 Web package 문맥으로 실행한다. |
| `scripts/qa/candidate-web-e2e.sh`, `candidate-web-e2e-runner.py`, `test_candidate_web_e2e_runner.py` | 기존 포트 예약·프로세스 소유권·browser inventory·cleanup 검증을 유지한다. 공통 preflight가 이 supervisor를 대체하지 않는다. |
| `deploy/db/integration-validation/{static-check.sh,validate.sh}`, `deploy/qa/qa-db.compose.yml` | 기존 DB static/self-test와 disposable 통합 validator를 그대로 호출한다. 운영 DB는 사용하지 않는다. |
| `scripts/ci/prepare_phase0.py`, `nt/{pyproject.toml,uv.lock}` | Python/혼합 workspace 검증의 fixture 준비와 locked 설치를 재사용한다. 이미 생성된 데이터에 준비 스크립트를 무조건 재실행하지 않는다. |
| `scripts/qa/service-status-{qa-lib.sh,unit-runner.sh,unit-receipt.sh}` | resource/exit evidence 참고 구현이다. 특정 task 디렉터리·systemd unit에 결합되어 있어 공통 진입점에서 그대로 source하지 않는다. 기존 전용 검증은 유지한다. |
| `.cargo/config.toml` | 일반 Cargo `jobs=3` 설정이 있다. 운영의 `CARGO_BUILD_JOBS=2`와 다른 범위다. 이 작업에서 전역 값을 올리지 않는다. |
| `.github/workflows/research-smoke.yml`, `scripts/qa/research-worker-smoke.sh` | 기존 90분 job과 두 서비스의 순차 build를 재사용한다. Raw 초기화 probe의 새 Cargo volume, locked fetch 후 offline compile, context 감사의 `--no-cache`를 유지한다. |
| `data-pipelines/collectors/Dockerfile`, `crates/job-queue/Dockerfile`, `deploy/build/Dockerfile.rust-artifacts` | registry/git/target의 `sharing=locked` cache mount가 이미 있다. Dockerfile·운영 builder를 다시 설계하지 않고 hosted 실행 사이의 layer 저장을 보완한다. |
| `scripts/qa/build-cache-smoke.sh`, `build-cache-benchmark.sh` | 앞의 fixture self-test를 보조 검증에 재사용한다. 뒤의 운영 resource/health gate를 사용하는 benchmark는 일반 CI 성능 측정용으로 실행하지 않는다. |

## Initial classification

분류는 모델 선택에 앞서 정한다. 코드 양보다 결합도·판단 불확실성·검증 가능성을 기준으로 삼았다.

| Package | Complexity | Basis | Confidence | Reclassification or escalation signals |
|---|---|---|---|---|
| WP-1 | simple | 확정된 절차를 지침·명령 대응표로 정리하며 직접 diff로 검증 가능 | high | 상위 지침과 실제 충돌하거나 실행 환경 정책을 변경해야 하는 경우 |
| WP-2 | intermediate | 경로 분류, 선행조건 검사, 검증 기록·무효화가 연결되지만 CLI와 실패 계약을 아래에 한정 | medium | 동일 결함 2회 실패, 입력 범위 누락, 새 process supervisor가 필요해지는 경우 |
| WP-3 | intermediate | 기존 두 workflow에 제한된 cache 경로와 실행 정책을 통합하고 실제 cold/warm 결과로 검증 | medium | cache exporter 미지원, 용량·전송 비용 때문에 효과 없음, 운영 builder 변경이나 새 runner가 필요한 경우 |

WP-1은 `gpt-6-luna medium`, 명세가 정해진 WP-2/3은 `gpt-6-luna max`에서 시작한다. 실제 provider/model 가용성은 실행 시 현재 Paseo에서 확인한다. 기계적 검증과 코디네이터의 코드 확인 없이 결과를 채택하지 않는다. 같은 원인으로 두 번 실패하거나 범위가 변하면 해당 분기를 재검토하고, 관찰된 필요에 따라 `gpt-6.1-sol`로 한 단계 올린다. 모델과 effort를 동시에 올리지 않는다.

## Execution graph

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
|---|---:|---|---|---|---|---|---|---|
| WP-1 | 1 | simple | 1단계: 작업 규칙과 검증 사용법 | `AGENTS.md`, 새 `docs/runbooks/development-verification.md` | 이 문서의 공통 계약 | `gpt-6-luna medium` | 지침·변경 경로 대응표·사용 예시 | 지침 대조, 경로·명령 검토, `git diff --check` |
| WP-2 | 1 | intermediate | 1단계: preflight·단계 실행·결과 재사용 | 새 `scripts/qa/dev-check.py`, `check-profiles.json`, `test_dev_check.py`; `scripts/verify-all.{sh,ps1}`, `scripts/ci/test_ci_contract.py` | 이 문서의 공통 계약 | `gpt-6-luna max` | 동작하는 CLI와 회귀 검사 | 아래 단위·negative·재사용 검증, CI 계약 검사 |
| WP-3 | 2 | intermediate | 2단계: CI cache와 실행 환경 통합 | 두 workflow, smoke 스크립트, 새 cache evidence 도구/테스트; WP-1/2 파일 중 아래 명시한 후속 영역 | WP-1/2 수용 및 1단계 commit | `gpt-6-luna max` | 실제 cache·resource 정책·cold/warm 비교 | CI 계약, cold/warm 각 필수 job·smoke, 자원·충돌 증거 |

Wave 1의 쓰기 범위는 겹치지 않는다. 인터페이스 변경은 코디네이터가 이 문서와 양쪽 brief에 먼저 반영한다. Wave 2는 Wave 1 종료 후 같은 파일을 인계받는다. 같은 파일을 두 담당자가 동시에 수정하지 않는다. 이 계획 문서의 체크와 범위 변경은 코디네이터가 관리한다.

## 공통 구현 계약

### A. 단계와 CLI

새 진입점은 Python 표준 라이브러리 중심의 얇은 dispatcher다. 기존 검증 명령을 대체하거나 새 범용 workflow 엔진을 만들지 않는다. 문서에 쓰는 아래 명령은 **구현 후 제공할 인터페이스**다.

```bash
# 변경 영역과 실행할 검사를 출력한다. 검사·설치·서버 실행은 하지 않는다.
python3 scripts/qa/dev-check.py plan --base origin/main --format json

# 선택한 단계의 도구·입력·경로·권한·포트를 먼저 확인한다.
python3 scripts/qa/dev-check.py preflight --profile web --stage focused --environment local

# 빠른 검사를 수행하고, 적격인 동일 입력 결과만 재사용한다.
python3 scripts/qa/dev-check.py run --profile web --stage fast --reuse

# 변경 영역의 검증. 실제 실행 명령은 profile에 명시한다.
python3 scripts/qa/dev-check.py run --profile qa --stage focused
```

- profile은 `docs`, `rust`, `web`, `python`, `qa`, `db`, `all`; stage는 `fast`, `focused`, `integration`이다. `plan`은 변경 파일에 따라 복수 profile을 제시한다.
- `plan`의 base 기본값은 `HEAD`, environment 기본값은 `local`이다. 명시적 `run`/`preflight` profile은 호출 위치에 관계없이 해당 profile의 단계별 검사를 선택한다. 하위 디렉터리에서 CLI를 부를 때에는 스크립트의 절대 경로를 사용한다.
- 경로는 스크립트 위치에서 저장소 root를 구해 정규화한다. root와 `apps/web`에서 호출해도 같은 argv/cwd를 선택한다. 존재하지 않는 base ref는 구체적인 오류로 종료하고 자동 fetch하지 않는다.
- 변경 집합에는 base와 HEAD 차이, staged/unstaged 변경, Git이 무시하지 않는 새 파일, 삭제·rename의 양쪽 경로를 포함한다. 비밀 파일과 ignored 산출물을 읽어 fingerprint에 넣지 않는다.
- 알 수 없는 경로, workspace/lock/toolchain/공통 설정 변경은 검사 범위를 넓힌다. `plan`은 미분류 경로와 전체 필수 CI 필요성을 표시하며 이를 무검사 통과로 바꾸지 않는다.
- 실행 순서는 `preflight → fast → focused → integration → 확정된 커밋의 CI`다. `run`은 선행 검사를 실행하거나 유효한 선행 결과를 확인한다. 실패 이후 비싼 단계를 시작하지 않는다.
- Rust focused 선택은 `--package <실제 workspace package>`를 지원한다. 의존 crate 영향이 불명확하면 reverse dependency를 임의 추정하지 말고 workspace 검증으로 넓힌다.
- `integration`은 기존 CI job/harness의 정확한 명령과 전제조건을 제시한다. 전체 Rust/DB/browser/smoke를 운영 호스트에서 기본 실행하지 않는다. 기존 hosted job은 자기 단계의 preflight 후 기존 명령을 실행하며, dispatcher로 동일 검사를 한 번 더 실행하지 않는다.
- 성공 0, 검사 실패 1, 선행조건 미충족/잘못된 입력 2로 구분한다. `preflight` 성공을 테스트 통과로 기록하지 않는다. 하위 명령의 원래 exit code도 기록한다.

### B. 변경 경로와 검증 대응

이 표가 `check-profiles.json`과 사용 문서의 기준이다. 현재 존재하는 명령은 유지하고 새로운 빠른 검사는 해당 파일에만 적용한다.

| 변경 영역 | fast | focused / integration | 최종 CI |
|---|---|---|---|
| 문서·지침 | diff 공백·참조 경로·범위 검사 | 제품 구조 변경을 포함하면 해당 코드 profile 추가 | 기존 required 집계 유지 |
| `crates/**`, `data-pipelines/collectors/**`, `apps/api-server/auth/**` | `cargo fmt --all -- --check` | `cargo clippy -p <package> --all-targets --all-features --locked -- -D warnings`, 관련 `cargo test -p <package> --locked`; DB 필요 대상은 disposable 환경으로 전달 | workspace Clippy, chart acceptance, 전체 workspace tests |
| `apps/web/**`, Web 관련 npm 설정 | `npm run lint --workspace @lagrange/web`, `npm run typecheck --workspace @lagrange/web` | OpenAPI check, `npm test --workspace @lagrange/web`, build, `bash scripts/qa/candidate-web-e2e.sh` | 기존 Web job |
| `nt/**`, Python 검사·fixture | 변경 Python 파일의 AST 구문 검사 | 기존 locked 환경과 Phase0 전제 확인 후 `uv run --project nt pytest -q`; 필요한 focused selection은 argv로 기록 | Python job와 혼합 Rust workspace job |
| `scripts/qa/**`, `scripts/ci/**`, workflow | 변경 shell의 `bash -n`, Python AST, workflow YAML/계약 검사 | 해당 unittest/self-test; 영향받는 기존 harness | policy 및 영향받는 job, 최종 required 전체 |
| `deploy/db/**`, migration 계약 | `bash deploy/db/integration-validation/static-check.sh`, `bash deploy/db/integration-validation/validate.sh --self-test` | `bash deploy/db/integration-validation/validate.sh --evidence-dir <새 전용 경로>` | 기존 PostgreSQL job, runtime SKIP 금지 유지 |
| `Cargo.lock`, workspace manifests, `.cargo/**`, toolchain 또는 미분류 공통 파일 | 적용 가능한 fast 전체 | 모든 영향 영역. 불확실하면 full integration 계획 | 모든 필수 job |

최종 Rust 명령은 다음과 같다. legacy verify-all의 대응 명령도 이 옵션에 맞추되 다른 기존 gate를 삭제하지 않는다.

```bash
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test -p api-server --test http_owner_equity_v2_chart --locked
cargo test --workspace --locked --no-fail-fast --features kis-client/test-support
```

### C. Preflight의 책임

- profile/stage에 필요한 도구만 확인한다. 기준 파일은 `rust-toolchain.toml`의 Rust 1.97.1, `.python-version`의 Python 3.12, npm의 Node 24, 현재 workflow의 uv 0.12.1·pyarrow 25.0.0이다. 별도 승인 없이 버전을 바꾸지 않는다.
- `check-pins.sh --manifest-only`를 재사용하고 설치 버전은 선택 영역에 맞게 검사한다. 문서 검사 때문에 Docker/Chromium/DB를 요구하지 않는다.
- npm/uv 설치·Chromium 준비가 없으면 필요한 준비 명령을 표시한다. preflight가 설치하거나 무거운 작업으로 대신 확인하지 않는다. Chromium은 기존 CI처럼 Vitest/browser 검사 전에 준비한다.
- 환경변수는 profile별 필수 이름·허용된 형태만 검사한다. workspace DB는 기존 테스트가 요구하는 disposable loopback endpoint를 확인한다. 기존 harness가 내부에서 만드는 synthetic 값은 사용자가 또 제공하도록 요구하지 않는다. 값 전체나 credentials는 출력하지 않는다.
- 출력 디렉터리의 parent·owner·mode·symlink와 쓰기 가능 여부를 확인한다. 검사 자체가 chown/chmod로 남의 경로를 고치지 않는다. receipt는 새 private 디렉터리에 쓴다.
- 포트는 선택한 harness가 쓸 것만 검사한다. Web의 두 설정 가능 포트와 고정 stream 포트, DB validator의 세 포트를 구분한다. HTTP health 요청이나 다른 listener 종료는 하지 않는다.
- 포트 실패에는 포트 번호와 `EADDRINUSE`/`EACCES`/`EPERM` 등 관찰된 errno를 남긴다. 사전 bind 후 release는 예약 보장이 아니다. 실행 시점의 예약·identity·cleanup은 기존 harness가 다시 확인한다.
- 환경변수 누락, 사용 중인 포트, 권한 부족, Docker daemon 접근 불가를 제품 테스트 실패와 구분한다. 하위 검사 실행 수가 0임을 negative test로 확인한다.

### D. 결과 기록과 재사용

- 저장 경로: ignored `target/dev-check/<run-id>/`; root 전체를 외부로 업로드하거나 cache하지 않는다. 파일은 JSON이며 schema version, check ID, 대상 HEAD, 입력 fingerprint, argv, cwd, 시작/종료·duration, 원래 exit code, 판정, 증거 경로를 포함한다.
- fingerprint에는 해당 소스와 새/삭제 파일, lock·manifest·설정, dispatcher/profile 버전, 실제 도구 버전·OS/arch, 허용된 비밀이 아닌 환경 조건을 포함한다. 설치 상태가 검증에 영향을 주면 그 상태를 확인할 수 없을 때 재사용하지 않는다.
- 입력은 실행 전후에 비교한다. 중간 변경, timeout, 부분 실행, 파싱 실패, 외부조건 미확인 결과는 재사용 불가다. 실패 로그는 보존하며 성공으로 덮어쓰지 않는다.
- 최초 자동 재사용 allowlist는 formatter, shell/Python 구문, manifest-only pin 검사처럼 입력을 완전히 묶을 수 있는 결정적 검사다. 다른 검사는 결과를 기록하되 자동 재사용은 기본 금지한다. 각 검사에서 설치 상태까지 증명할 수 있을 때만 allowlist를 명시적으로 확장한다.
- DB·browser·실서비스 health·네트워크·clock/권한/프로세스 상태에 의존하는 검증과 최종 필수 CI는 매번 실행한다. 같은 코드라는 이유로 동적 상태가 같다고 간주하지 않는다.
- 재사용은 `REUSED`와 원본 receipt·원본 대상 커밋을 표시한다. 현재 커밋에서 새로 실행한 `PASS`로 표시하지 않는다. 관련 입력이 같으면 커밋 SHA 자체가 다르다는 이유만으로 결정적 검사를 다시 실행할 필요는 없다.
- package 설치 cache, 컴파일 cache, 검증 결과 재사용은 서로 다른 기능이다. cache hit가 테스트 성공을 대신하지 않는다.

## Worker briefs

### WP-1 — 지침과 사용 문서

**작업 디렉터리:** `/data/worktrees/3puw275b/orange-dugong` 또는 코디네이터가 지정한 같은 revision의 Paseo 작업 공간. **분류:** simple/high. 규칙과 명령 계약이 위에서 정해져 있고 직접 대조 가능하다. 상위 지침과 충돌하면 임의로 덮지 말고 해당 충돌만 보고한다.

**입력·의존성:** 이 계획, 전역/저장소 지침, 위 재사용 표의 스크립트와 두 workflow. WP-2와 병렬이며 완료를 기다리지 않고 위 CLI 계약으로 문서화한다.

**쓰기 범위:** `AGENTS.md`, 새 `docs/runbooks/development-verification.md`만. workflow·스크립트·제품 코드·전역 설정·로드맵은 수정하지 않는다.

구현할 내용:

1. 이미 맡긴 작업은 같은 담당자가 구현과 focused 검증까지 마친다. import 누락, cwd/인자 오류, 자신의 범위 안 회귀는 직접 고치고 해당 검사부터 이어간다.
2. 코디네이터의 파일 1~2개 소규모 통합 수정과 독립 패키지 위임 기준을 구분한다. 작은 오류마다 별도 명세·검증 담당자·승인 단계를 만들지 않는다.
3. 동일 원인 2회 실패, 명세 모순, 수정 범위 확장 때만 해당 분기의 진단·계획을 재검토한다. 다른 독립 작업은 계속한다. 모델 선택은 전역 규칙을 참조한다.
4. 공통 계약 A~D의 사용 명령·변경 경로 대응·재사용 제한을 문서화한다. hosted 검증이 최종 기준이며 legacy verify-all 하나가 CI 전체와 동일하다고 설명하지 않는다.
5. 이 개선 작업의 진행표는 이 계획에서 관리하도록 연결한다. 운영·provider·다이어그램 지침을 보존하고, 기존 문구와 중복되는 설명을 합친다.

**검증:** `git diff --check`; 모든 기존 파일 경로를 확인하고 새 CLI 표기는 “구현 후”로 구분한다. 전역 위임 기준·운영 제한과 문구를 대조한다. 테스트 없는 문서 검토를 런타임 검증으로 보고하지 않는다.

**완료 기준:** 누가 직접 처리/위임/수정/재계획하는지 한 번 읽고 판단할 수 있고, 대응표와 CLI가 WP-2의 공통 계약과 같다. 2단계의 자원·cache 실측 결과는 WP-3가 인계받아 보완한다.

**보고:** 변경 파일·라인, 계약과 다른 점/이유, 확인 명령·결과, 미해결·후속, 찾지 못했거나 검증하지 못한 항목을 적는다. 빈 항목은 `없음`으로 명시한다.

### WP-2 — 공통 preflight·검증 단계·receipt 구현

**작업 디렉터리:** `/data/worktrees/3puw275b/orange-dugong` 또는 지정된 동등 Paseo 작업 공간. **분류:** intermediate/medium. 입력 무효화와 환경 진단의 경계를 기계적으로 검증한다. 같은 수정 2회 실패, 범위 누락 또는 새로운 supervisor 필요가 관찰되면 재분류한다.

**입력·의존성:** 이 계획의 A~D, 현재 CI/verify-all/전용 harness. WP-1 결과와 무관하게 확정된 CLI를 구현하며 새 요구를 추측해서 추가하지 않는다.

**쓰기 범위:**

- 신규: `scripts/qa/dev-check.py`, `scripts/qa/check-profiles.json`, `scripts/qa/test_dev_check.py`.
- 수정: `scripts/verify-all.sh`, `scripts/verify-all.ps1`의 Rust 옵션 정합화; `scripts/ci/test_ci_contract.py`의 기존 gate 대응·두 스크립트 옵션 일치 계약. 새 CI 실행 step을 요구하는 계약은 workflow를 수정하는 WP-3에서 함께 추가한다.
- 제외: workflow, 기존 Web supervisor/테스트, DB validator, Dockerfile, 제품 코드, WP-1 문서. shell/PowerShell의 기존 비-Rust gate를 삭제하지 않는다.

구현은 기존 명령을 argv 배열로 호출한다. shell 문자열 보간·환경 전체 dump·광역 프로세스 종료·자동 설치를 넣지 않는다. 긴 프로세스의 소유권과 정리는 기존 harness에 맡기며 두 번째 process tree 관리기를 만들지 않는다.

**담당자가 수행할 검증:**

```bash
python3 -B -m unittest discover -s scripts/qa -p 'test_dev_check.py' -v
python3 -B -m unittest scripts.ci.test_ci_contract -v
bash -n scripts/verify-all.sh
python3 scripts/qa/dev-check.py plan --base HEAD --format json
python3 scripts/qa/dev-check.py run --profile qa --stage fast
git diff --check
```

PowerShell 파일은 사용 가능한 `pwsh`의 Parser로 구문 검사를 하고 두 legacy 파일의 Rust argv 일치를 계약 테스트로 확인한다. Parser 부재는 미검증으로 기록하고, 설치된 hosted 환경에서 마무리한다. 전체 verify-all을 문법 확인 목적으로 실행하지 않는다.

회귀 테스트는 tempfile·fake command·허용된 loopback fixture로 다음 실제 실패 경계를 검사한다.

- root/하위 cwd에서 같은 검사 선택; staged/unstaged/untracked/deleted/rename 포함; 알 수 없는 경로의 검증 확대.
- 도구 없음·버전 불일치·환경변수 누락·경로 권한/위험한 symlink·Docker 불가·포트 충돌/권한 실패 각각의 원인과 exit 2. sentinel로 무거운 하위 명령이 시작되지 않았음을 확인.
- 같은 입력을 두 번 검사하면 적격 검사만 재사용. 소스/lock/설정/도구/환경/argv 중 하나를 바꾸면 무효화. 관련 없는 문서 변경만으로 format 결과가 무효화되지 않는 사례.
- 실패·손상 receipt·중간 소스 변경·runtime 검사의 재사용 거부. 원래 exit code 및 첫 실패 뒤 실행 중단 유지.
- 외부 listener, 기존 파일 mode/owner, 호출자 프로세스에는 변경 없음. 포트 probe의 성공을 예약·browser 검증 성공으로 주장하지 않음.

**완료 기준:** 정상 환경 실제 fast 검사 통과, 모든 negative 및 재사용 경계 테스트 통과, JSON receipt에서 커밋·명령·입력·결과를 추적 가능, 기존 CI gate 대응이 유지됨. 느린 전체 검증을 이 패키지에서 반복하지 않는다.

**보고:** 변경 파일·라인, 차이/이유, 명령·exit·검사 수·receipt 경로, 미해결/후속, 찾지 못함/미검증을 각각 명시한다. 문제를 자신의 범위에서 수정한 뒤 보고하며 사소한 오류를 검증 전용 패키지로 넘기지 않는다.

### WP-3 — CI cache와 실행 환경 통합

**작업 디렉터리:** `/data/worktrees/3puw275b/orange-dugong` 또는 지정된 동등 Paseo 작업 공간. **분류:** intermediate/medium. 기존 hosted job와 아래 저장 경로로 한정한다. exporter 호환성·용량·효과 문제가 운영 builder 변경까지 요구하면 해당 분기를 멈추고 근거와 대안을 보고한다.

**입력·의존성:** 수용·커밋된 WP-1/2, 두 workflow, `test_ci_contract.py`, 기존 Docker cache mount/순차 smoke, 아래 공식 자료. 모델/API 가용성·action SHA는 실행 시 확인하며 문서에 없는 action 버전을 지어내지 않는다.

**쓰기 범위:**

- `.github/workflows/ci.yml`, `.github/workflows/research-smoke.yml`.
- `scripts/ci/test_ci_contract.py`; 신규 `scripts/ci/cache-evidence.py`, `scripts/ci/test_cache_evidence.py`.
- `scripts/qa/research-worker-smoke.sh`: hosted cache 선택 및 기존 build에 한정된 연결.
- WP-2에서 인계받은 `scripts/qa/{dev-check.py,check-profiles.json,test_dev_check.py}`: CI preflight·heavy slot 연동만.
- WP-1에서 인계받은 `AGENTS.md`, `docs/runbooks/development-verification.md`: 아래 자원 표·cache 사용법·실측 결과만.
- Dockerfile, production Compose, 운영 builder/resource helper, 제품 코드, lockfile은 제외한다. CI 전용 Compose override는 smoke의 private 임시 디렉터리에 생성하고 기존 두 서비스의 build-cache 필드만 추가한다.

#### Cache 구현

`actions/cache/restore`와 `actions/cache/save`를 분리해 사용하고 action은 기존 규칙처럼 검증한 40자리 commit SHA로 고정한다. 설치·build·test 명령은 cache hit와 무관하게 실행한다. cache miss/서비스 장애는 원래 설치·build 경로로 처리하되 검증 자체의 실패를 감추지 않는다. 분리된 restore/save는 [공식 cache action](https://github.com/actions/cache#using-a-combination-of-restore-and-save-actions)이 지원한다.

| 종류 | 복원·저장 범위 | 호환성 key와 적용 job |
|---|---|---|
| Cargo 다운로드 | 공개 locked 의존성의 registry index/cache/src 및 git db/checkouts만. `CARGO_HOME` 전체와 credentials/config는 제외 | OS·arch·실제 rustc identity·Cargo.lock·manifest hash. clippy/workspace/DB, smoke의 host Cargo 호출 |
| Cargo 빌드 산출물 | job별 `${RUNNER_TEMP}` 아래 전용 `CARGO_TARGET_DIR`. 저장소 `target/` 전체는 금지 | OS image·arch·rustc·profile·feature lane·compiler/RUSTFLAGS·`.cargo` 설정·lock/manifests + source revision. 호환 prefix 아래 이전 산출물 복원 후 Cargo가 다시 검사 |
| npm 다운로드 | `npm config get cache` 아래 `_cacache`만 | OS·arch·실제 Node/npm 버전·`package-lock.json`. Web job. `npm ci`는 유지 |
| uv 다운로드 | 고정된 job 전용 `UV_CACHE_DIR` | OS·arch·Python·uv·`nt/uv.lock`·`nt/pyproject.toml`. Python/workspace job. `uv sync --project nt --locked`는 유지 |
| Docker layer | hosted job 전용 Buildx builder의 두 smoke 서비스별 local cache export | OS·arch·BuildKit/Buildx·base digest·Dockerfile·lock·build 설정 + source revision. 서비스별 cache 경로/key를 분리 |

Node 설치 action의 암묵적 cache와 중복 저장하지 않는다. `node_modules`, `.venv`, `.next`, DB volume, Raw/Curated/Phase0 데이터, 검증 receipt·로그, 비밀 값은 cache 대상이 아니다. npm의 다운로드 cache와 `node_modules`는 [setup-node 문서](https://github.com/actions/setup-node#caching-global-packages-data)에서도 구분한다. uv는 [공식 CI cache 안내](https://docs.astral.sh/uv/guides/integration/github/#caching)를 참고하되 버전과 설치 방식은 현재 pin을 유지한다.

Docker에는 다음 연결만 추가한다.

1. hosted job에서 전용 `docker-container` Buildx builder를 준비하고 두 `compose build`가 그 builder를 사용했는지 확인한다. `--builder`는 [Compose build 옵션](https://docs.docker.com/reference/cli/docker/compose/build/)으로 명시할 수 있다.
2. 복원 cache는 읽기 입력으로 두고 새 export는 다른 디렉터리에 쓴다. 성공 후 owned 경로의 새 export만 저장 대상으로 승격한다. 기존 cache 전체 prune이나 운영 builder 전환을 하지 않는다.
3. 임시 override의 `cache_from`/`cache_to`는 [Compose build 명세](https://docs.docker.com/reference/compose-file/build/#cache_from)에 맞추며 중간 build layer도 내보내도록 `mode=max`를 사용한다. cache가 없는 첫 실행도 가능해야 한다. unsupported exporter를 무시한 빌드를 cache 성공으로 보고하지 않는다.
4. **연구 worker 한 서비스 → 종료 확인 → candidate runner 한 서비스** 순서를 유지한다. 기존 Dockerfile의 locked cache mount·workspace clean·mount 밖 binary 복사는 유지한다.
5. layer export는 Cargo cache mount 내용을 자동 보존하지 않는다. 따라서 이번 성능 주장은 host Cargo cache와 Docker layer cache로 나눠 측정한다. cache-mount 추출 action이나 운영 build 레이아웃 변경은 포함하지 않는다. 이 차이는 [Docker cache 문서](https://docs.docker.com/build/ci/github-actions/cache/#cache-mounts)에 명시되어 있다.
6. Raw 초기화 probe의 새 cache volume과 offline compile, context 감사의 `--no-cache`는 매 실행 유지한다. 일반 layer cache와 cold 재현성 검사는 별도로 통과해야 한다.

PR은 cache 복원만 하고 저장은 성공한 신뢰 가능한 push/명시적 repository workflow dispatch로 제한한다. 전체 권한은 `contents: read`, checkout은 `persist-credentials: false`를 유지한다. `pull_request_target`, 새 secret, registry push, artifact upload를 도입하지 않는다. cache 크기와 현재 저장 한도를 확인하고 초과·축출로 warm 재사용이 깨지면 그 cache는 완료 처리하지 않는다.

기존 “cache 전면 금지” assertion만 정해진 allowlist 검증으로 교체한다. cache 경로 확대, credentials/receipt 포함, 필수 검사 삭제, hit 조건으로 검사 생략, 두 Rust 서비스 병렬 build를 거부하는 negative 계약을 추가한다. Web 명령의 필요한 순서와 일회 실행은 계속 검증한다.

#### 병렬화·자원 실행 정책

| 작업 | 기본 위치 | 동시 실행 정책 | 검증 |
|---|---|---|---|
| 독립 구현·분석 | 로컬 Paseo 작업 공간 | 기본 worker 2개, 가용 slot 내 최대 3개 + coordinator. 쓰기 범위 분리 | Wave 1 파일 충돌 없음 |
| 구문·작은 stdlib 검사·읽기 | 로컬 | 기본 2개까지. 에이전트마다 독립 출력 경로 | 병렬 실행 시 서로 다른 receipt·정상 종료 |
| Cargo build/Clippy, 대규모 tests, Next build/browser, Docker/DB 통합 | 기존 GitHub-hosted job 우선 | 서로 다른 VM의 기존 job 병렬성 유지. 각 job 안 heavy 단계는 순차 | job별 시간·자원 관측, OOM/timeout 여부 |
| 로컬에서 필요한 focused heavy 재현 | 개발 환경, 운영 호스트에서는 최소화 | 동일 호스트·사용자의 작업 간 heavy slot 1개. Rust 재현은 `CARGO_BUILD_JOBS=2` | 첫 명령이 보유 중이면 두 번째는 시작 전 실패, 종료 후 재획득 |
| 운영 이미지 빌드 | 기존 운영 절차 | 서비스 하나씩, Cargo 2, Compose 1 및 기존 배치 health gate | 지침·명령 보존 확인. 이번 작업에서 실행 안 함 |

heavy slot은 Linux의 user-private runtime 디렉터리에 단일 advisory lock으로 구현해 worktree마다 다른 lock을 만드는 실수를 피한다. `XDG_RUNTIME_DIR`의 owner/mode를 검증하고 사용할 수 없으면 명시적 private lock 경로를 요구한다. 실패 시 다른 프로세스를 죽이거나 lock 파일을 임의 삭제하지 않는다. lock은 새 dispatcher를 통해 실행하는 명령끼리의 조정이며 시스템 전체 프로세스를 통제한다고 주장하지 않는다. 기존 전용 harness의 더 엄격한 slot 규칙도 유지한다.

시간 기록은 실제 명령과 연결하고 peak RSS/cgroup 같은 지표가 없으면 `unavailable`로 남긴다. 호스트의 과거 OOM을 현재 검사 탓으로 돌리지 않는다. 새 빌드 머신이나 더 높은 동시성은 측정 결과가 필요하며 이번 구현의 대체 완료 조건으로 쓰지 않는다.

#### 담당자가 수행할 검증과 완료 기준

먼저 영향을 받는 가벼운 검사만 수행한다.

```bash
python3 -B -m unittest scripts.ci.test_ci_contract scripts.ci.test_cache_evidence -v
python3 -B -m unittest discover -s scripts/qa -p 'test_dev_check.py' -v
bash -n scripts/qa/research-worker-smoke.sh
bash scripts/qa/research-worker-smoke.sh --self-test
bash scripts/qa/build-cache-smoke.sh --self-test
bash scripts/ops/build-production-images-static-check.sh
git diff --check
```

- lock 충돌·해제 테스트는 compiler 대신 짧은 fake child로 확인한다. 차단된 두 번째 child의 실행 횟수 0, 첫 child reap, 해제 뒤 실행 성공을 검증한다.
- `cache-evidence.py`는 workflow/job/check SHA, cache 종류·key·exact/prefix/miss·restore/save 결과·크기, 설치/build/test 소요시간을 제한된 JSON과 `$GITHUB_STEP_SUMMARY`로 출력한다. API token이나 환경 전체를 입력으로 받지 않는다. 단위검사는 손상·누락·서로 다른 SHA의 비교 거부와 시간 합산을 검사한다.
- hosted cache 검증은 **최종 구현을 커밋한 같은 SHA**에서 수행한다. 두 workflow에 `workflow_dispatch`용 검증된 `cache_epoch` 문자열 입력을 두고, 새로운 namespace로 cold 1회가 끝난 뒤 같은 namespace/ref로 warm 1회를 실행한다. 기존 cache를 지워 cold를 만들지 않는다. 기존 exact-candidate 실행이 조건을 만족하면 그 run을 사용한다.
- 따라서 최초 실증의 상한은 CI 2회 + research smoke 2회다. 이는 명시적인 최초 생성/재사용 비교다. 실패를 이유로 같은 run을 무작정 반복하지 않는다. 수정 후에는 실패한 관련 local gate부터 확인하고 새 SHA의 필요한 hosted 증거를 만든다.
- cache restore/save와 설치·build·test 시간을 따로 비교한다. warm이 느리면 전송량·cache 범위를 줄이고 재검토한다. “hit”만 있고 저장/복원 효과가 없거나 두 실행 중 검증이 실패하면 완료로 표시하지 않는다. 속도 개선 비율은 미리 약속하지 않는다.
- 두 CI 실행 모두 7개 검사 job와 `required`가 성공하고, 두 smoke 모두 cold probe 및 functional 검사까지 통과해야 한다. DB runtime SKIP gate와 Web의 기존 inventory/cleanup 계약도 유지한다. 기존 출력에 없는 수치를 지어내지 않는다.

**보고:** 변경 파일·라인, 범위 차이/이유, 실행한 정확한 명령·CI URL·checkout SHA, cold/warm 표, resource·cleanup 결과, 미해결/후속, 찾지 못함/미검증. 구현과 검증을 이 담당자가 끝내며 결과 수집만 다른 검증 패키지로 분리하지 않는다.

## Coordinator gates

1. **실행 전:** 사용자의 후속 실행 요청과 `$paseo-delegate` 가용성, 현재 HEAD/diff, 각 수정 경로의 지침을 확인한다. 기준 revision과 달라진 부분만 다시 읽는다. 쓰기 소유권·CLI 계약·기존 필수 job·운영 제한을 worker brief에 담는다. 기본 설계에 추가 사용자 결정은 필요하지 않다. 외부 실행 권한/도구가 실제로 없으면 그 분기만 미검증으로 남긴다.
2. **Wave 1 통합:** WP-1/2를 병렬 진행하고 각 담당자의 focused 검증을 받는다. 코디네이터는 AGENTS의 중복/충돌, 문서와 profile·CLI의 일치, 재사용 무효화·negative 증거를 검토한다. 작은 통합 오류는 기존 담당자 또는 코디네이터가 고친다. source가 변하지 않은 통과 검사를 다시 전부 실행하지 않는다. 이 단계의 코드·사용 문서를 커밋한다.
3. **Wave 2 시작:** WP-1/2의 쓰기 권한을 종료하고 WP-3에 필요한 파일을 인계한다. cache 허용 경로, 신뢰 경계, CI 순서, 실제 cold/warm 실증 방법을 확인한다. 구현·local gate 완료 후 검증할 후보 commit을 확정하고 hosted 실행 동안 입력을 유지한다.
4. **최종 검증:** 두 단계의 사용 명령을 실제 구현과 대조하고, 같은 입력 재사용·변경 시 무효화·환경 실패 조기 종료·heavy 충돌 차단을 확인한다. 같은 후보의 cold/warm CI와 smoke 증거, 필수 job 유지, 비용 대비 효과를 수용한다. CI에서도 설치·검사 명령이 skip되지 않았는지 확인한다.
5. **마무리:** 구현·사용법을 커밋하고 이 문서의 상태와 실제 검증 SHA/URL/제약을 기록한다. 실측 기록만 추가하는 후속 문서 커밋이 있으면 테스트 대상 코드 SHA와 문서 SHA를 구분한다. 계획 작성·설정 추가·local 통과만으로 실제 CI cache 완료를 체크하지 않는다. 미완료가 있으면 정확한 항목과 실행 가능한 다음 명령을 남긴다.

## 진행·완료 기록

작업 시작 시 `TODO → IN_PROGRESS`, 구현과 담당 검증 수용 후 `DONE`으로 갱신한다. 외부 실행 전제가 없으면 `BLOCKED`와 이유를 적는다. 범위 변경은 기존 실패/미검증 기록을 지우지 않고 아래 변경 이력에 추가한다.

| 패키지 | 상태 | 구현 commit | 검증 대상·명령/증거 | 미해결 |
|---|---|---|---|---|
| WP-1 | DONE | `6d117f4` | 지침 충돌·참조 경로 확인, 사용법/CLI 대조, `git diff --check` | 없음 |
| WP-2 | DONE | `6d117f4` | QA 회귀 30개, CI 계약 10개 통과; QA fast 4개 통과, 적격 syntax 2개 재사용 확인; shell syntax·공백 확인 | PowerShell 구문 검증은 WP-3 hosted 환경에서 수행 |
| WP-3 | IN_PROGRESS | 이 문서와 함께 확정하는 후보 | QA 38개, CI·cache 계약 22개, shell syntax·self-test·운영 build static·공백 검사 통과 | 같은 후보의 실제 cold/warm CI·smoke와 PowerShell 구문 검증 대기 |

- [x] 1~3번: 지침·preflight·단계별 검증·결과 재사용 구현과 focused 검증 수용
- [ ] 4~5번: cache·실행 환경 구현과 실제 cold/warm CI 증거 수용
- [ ] 사용 명령·검증 대상 commit·제약을 문서화하고 구현 커밋 완료

| 날짜 | 변경 | 근거 |
|---|---|---|
| 2026-10-13 | 계획 최초 작성 | 기존 지침·스크립트·workflow·cache 계약 조사. 구현과 런타임 검증은 수행하지 않음 |
| 2026-10-13 | 실행 착수 | Owner의 후속 실행 요청. WP-1/2의 쓰기 범위를 나누어 병렬 진행 |
| 2026-10-13 | 1단계 수용 | 16개 workspace package 탐색, symlink 입력 거부, pinned formatter 환경, 실패 진단, endpoint port 충돌, 잘못된 base ref 회귀를 포함한 30개 검사 통과. CI 계약 10개 통과. 제품·운영 구조 변경 없음 |
| 2026-10-13 | 2단계 구현·focused 검증 수용 | 명시적 cache restore/save, hosted preflight, package별 Rust 선택과 로컬 heavy lock 구현. 코디네이터가 로컬 `ci` 라벨의 Cargo 2 유지 및 성공한 Docker export의 이전 store 교체를 보완하고 QA 38개·CI/cache 22개를 재검증. 실제 cache 효과는 아직 미검증 |
