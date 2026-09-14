Execution skill: $paseo-delegate (required)
Native subagents: prohibited for worker packages

# 운영 빌드 구조 추가 개선 실행 계획

## Goal and boundaries

**목표:** 동일 운영 환경에서 최종 이미지 12개를 만드는 전체 시간을 줄인다. 외부 의존성뿐 아니라 내부 공통 코드의 중복 컴파일을 대상으로 하며, 현재 캐시 방식·공통 빌더·역할별 빌더를 비교한 뒤 구조를 선택한다. 공통 빌더 채택을 미리 확정하지 않는다.

- 작업 디렉터리: `/data/worktrees/3puw275b/noble-crab`.
- 계획 기준 HEAD: `f4eb4f83abb7c3f43ede072d0077a1a74edd0ee3`. 계획 작성 전 작업 트리는 깨끗했다. 이 계획 문서 작성은 구현 변경이나 작업자 실행이 아니다.
- 비교 기준 A: 최초 구조 `d1baf9da9b13fcb61649b1c26de56aed87a83418`.
- 비교 기준 B: 1차 캐시 개선 `f4eb4f83abb7c3f43ede072d0077a1a74edd0ee3`의 제품 코드. 후속 계측 도구를 쓸 때는 도구 SHA도 별도로 기록한다.
- 비교 기준 C: 이 계획으로 구현하고 깨끗한 커밋으로 고정한 선택안.
- 시간 기준: 공통 산출물 준비, Rust/Web/DB 빌드, 이미지 생성, 최종 이미지 검사와 manifest 발행을 포함한 전체 경과 시간. 공유 작업 시간을 서비스마다 중복 합산하거나 측정 밖으로 빼지 않는다. 준비·실행·검증 구간도 각각 기록한다.
- 완료 기준: 정확성·기존 릴리스 계약·운영 자원 조건을 통과하고, 동일한 대표 변경 시나리오에서 B 대비 C의 중복 컴파일과 전체 시간이 감소했음을 실측한다. A도 같은 조건으로 비교해 최초 구조 대비 효과를 제시한다.
- 절대 시간 목표는 미정이다. 과거 Rust 이미지 7종 약 98분, 대화에서 계산한 30~70분은 전체 12개 인수 기준이나 성능 보장이 아니다.

**고정 계약:** 최종 이미지/서비스 12개, 기존 바이너리 목록·실행 경로·기능·feature/profile, 커밋 내장값과 OCI revision, strict V2 manifest, 설치·실행·롤백, 기존 운영 CLI 모드를 유지한다. 다른 커밋의 이미지를 새 릴리스로 사용하는 경로, 이미지 존재만으로 성공 처리하는 경로, 검증 생략 모드는 추가하지 않는다. 검증 가능한 중간 컴파일 캐시는 커밋 사이에 재사용할 수 있다.

**변경 가능한 영역:** Docker 빌드 단계와 그 연결, 중간 산출물 전달, 입력 분리, 내부 코드 캐시 재사용, 빌드 전용 Compose 설정과 운영 빌드 조율, 검사·계측 도구. 현재의 7개 Dockerfile 분리 방식과 이미지마다 `cargo clean --workspace`를 수행하는 방식은 고정 제약이 아니다. 대체 방식은 오래된 산출물을 배제한다는 증거가 필요하다.

**범위 밖:** 서비스 통합, 추가 빌드 서버·레지스트리, 운영 병렬화 확대, 애플리케이션 기능 변경, Cargo 의존성/프로필/feature 변경, Web·DB Dockerfile 변경, 운영 rollout·migration·provider 호출 및 연구 수집 장애 복구. Compose의 runtime service/profile/network/volume/command와 저장소 `.dockerignore`도 이번 구현 범위에서 유지한다. 이 경계를 넘겨야 한다면 해당 분기를 중단하고 이유와 대안을 제시한다.

**운영 제약:** 14GiB 호스트, `CARGO_BUILD_JOBS=2`, `COMPOSE_PARALLEL_LIMIT=1`, Compose 한 호출에 한 서비스. 논리 묶음은 2~3개 서비스이고 묶음 사이에 완료·메모리/swap·OOM·서비스 health를 확인한다. 공통 빌더도 여러 Rust-producing 서비스를 한 동시 그래프로 제출하거나 전체 컴파일을 병렬화할 수 없다. 공통 산출물 생성 단계에도 중단/재개와 자원 확인 지점을 정의해야 한다. 장시간 빌드는 낮은 CPU/I/O 우선순위의 background systemd 실행을 유지한다.

적용 지침은 `/home/l1nnx/.codex/AGENTS.md`, 저장소 `AGENTS.md`, 두 Paseo 스킬이다. 저장소 하위 provider 지침은 `apps/web/AGENTS.md`와 이를 참조하는 `apps/web/CLAUDE.md`가 확인됐다. 임시 Web 변경 시나리오를 작성할 때도 Web 지침과 설치된 Next 문서를 적용한다. 실행 직전에 새 지침과 경로 변경을 다시 확인한다.

현재 증거는 `docs/verification/production-build-cache.md`에 있다. 실제 시험은 `008488c...`에서 cold와 동일 입력 반복만 통과했다. warm 시험의 `--no-cache` 문제는 `3b0b88f...`에서 수정됐지만 실제 재시험은 남아 있다. 소스 삭제 오류 판정도 아직 미검증이다. 이를 완료로 간주하고 새 구조를 평가하지 않는다.

이전 관측의 `research-worker` 재시작, swap 부족, 신뢰할 수 없는 OOM 조회는 실제 시험의 선행 확인 항목이다. 현재 상태라고 단정하지 않고 재조회한다. `docs/superpowers/plans/2026-09-07-stock-beta-production-release-readiness.md`의 image-only 예외는 특정 커밋과 만료 시각에 한정됐으므로 자동 승계하지 않는다. 기존 세션의 유효한 권한은 존중하며, 이미 승인된 fixture 범위에 재승인을 요구하지 않는다. 이 계획 자체가 새로운 운영 빌드·서비스 변경 권한을 만들지는 않는다.

## Initial classification

| Package | Complexity | Basis | Confidence | Reclassification or escalation signals |
|---|---|---|---|---|
| WP-1 | intermediate | 기존 시험의 미검증 경로와 계측 정확성을 정해진 계약으로 보완 | medium | binary 출력과 Fresh 증거 충돌, 계측이 빌드 입력을 다르게 변경, 오류를 성공으로 처리 |
| WP-2 | hard | 내부 의존성·feature·커밋 내장·재개 단위가 결합된 구조 비교와 설계 명세 | high | 빌드 DAG/출처가 설명되지 않음, 기능/운영 제약을 바꾸어야만 성립 |
| WP-3 | intermediate | 확정된 두 후보 명세를 소형 fixture로 구현해 판별 | medium | 명세 누락, 공유 산출물 교차 오염, 중단/재개를 재현할 수 없음 |
| WP-4 | intermediate | G2에서 선택·동결된 명세를 제품 빌드 경로에 적용 | medium | 설계 결정을 작업자에게 다시 요구, 두 번 같은 수정 실패, 기존 소비 경로 누락 |
| WP-5 | intermediate | 새 빌드 DAG에 검사·계측·운영 절차를 맞추고 기존 검증을 보존 | medium | 공유 빌드 시간 누락/중복, static/Fake 검사의 허위 통과, 테스트 완화 필요 |
| WP-6 | hard | 실제 출처·정확성·성능·운영 자원을 종합한 독립 인수 판단 | high | OOM/health 저하, 커밋 불일치, 반복 간 비교 불가능, 측정 변동과 개선 폭이 구분되지 않음 |

## Execution graph

모든 provider는 Codex다. WP-1과 WP-5의 Terra/max는 같은 검증 도구 작업에서 이미 관측된 Luna 반복 실패 후의 상향을 이어받는다. 명세가 확정되는 WP-3·4는 Luna/max에서 시작한다. 구조 판단인 WP-2는 전역 지침의 Astra/xhigh, 독립 검토인 WP-6은 Sol/high를 적용한다. 실행 시 지원 여부를 확인하고 미지원 조합을 임의 대체하지 않는다.

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
|---|---:|---|---|---|---|---|---|---|
| WP-1 | 1 | intermediate | 신뢰할 수 있는 기준 시험·계측 준비 | 기존 QA 2개·캐시 fixture·기준 검증 문서 | 없음 | gpt-5.6-terra / max | 시나리오·계측 계약, 보정된 시험과 증거 | self-test, 조건 충족 시 현재 fixture |
| WP-2 | 1 | hard | 세 구조의 비교와 두 실험안 명세 | 신규 설계 문서만; 제품 코드 읽기 전용 | 기준 HEAD | gpt-6-astra / xhigh | 입력/산출물/DAG/출처/재개 명세 | 코드 근거, 공식 문서, 계약 체크리스트 |
| WP-3 | 2 | intermediate | 공통·역할별 빌더를 소형 실험으로 비교 | 신규 layout fixture·probe 도구·실험 보고서 | WP-1·2, G1 | gpt-5.6-luna / max | 동일 입력의 3방식 실험 | binary 출력, Cargo 증거, 실행 순서·실패 재개 |
| WP-4 | 3 | intermediate | 선택한 제품 빌드 구조 구현 | Dockerfile 7개·빌드 설정·운영 빌드 코드 | WP-3, G2 | gpt-5.6-luna / max | 제품 구현, 변경/입력 목록 | 명세 대조, 통합 Fake 검사 |
| WP-5 | 3 | intermediate | 새 구조의 검증·계측·운영 문서 정합화 | 기존 ops 검사·benchmark·runbook·관련 도표 | WP-3, G2 | gpt-5.6-terra / max | 회귀 검사·측정 도구·운영 절차 | 도구 self-test, 통합 검사, 필요한 도표 렌더 |
| WP-6 | 4 | hard | 깨끗한 후보의 독립 인수 | 최종 검증 보고서만; 구현 읽기 전용 | WP-4·5, G3 | gpt-5.6-sol / high | 인수 행렬·실측·미검증 항목 | 실제 fixture, 이미지 12개·V2, A/B/C 비교 |

Wave 1과 Wave 3의 작성 작업만 병렬 진행한다. 각 wave의 수정 범위는 분리한다. 실제 Docker/Rust 빌드 슬롯은 전체 작업에서 하나이며, WP-1 → WP-3 → WP-6 순으로 명시적으로 인계한다. 계획 단계에서는 어떤 작업자도 실행하지 않는다.

## Worker briefs

### 공통 전달 계약

실행자는 이 공통 계약, 해당 분류 행, 해당 brief, 의존 산출물의 고정 SHA/경로를 함께 전달한다. 작업자는 대화 이력을 받지 않는다고 가정한다. 아래 상대 경로는 모두 `/data/worktrees/3puw275b/noble-crab` 기준이며 각 프롬프트에 절대 작업 디렉터리를 포함한다.

`Do not use native subagent, Task, Agent, team, or delegation features. Complete this assignment directly and report if it needs further decomposition.`

- 소유 범위 밖 수정, 임의 기능/의존성 변경, 운영 서비스 조작, provider 호출, 자격 증명 출력, global cache prune을 금지한다. 실제 시험은 승인된 범위와 자원 조건을 모두 만족해야 한다.
- 미확정 명세를 추측하지 않는다. 충돌·누락은 근거와 함께 보고하고 해당 분기만 중단한다.
- 기존 도구와 fixture를 재사용하고 필요한 수정만 한다. 이 작업을 범용 빌드/벤치마크 프레임워크 제작으로 확대하지 않는다. 통과한 검사는 새로운 변경·실패·미해결 근거가 있을 때만 반복한다.
- 파일을 commit/stage하지 않는다. 코디네이터가 검토·통합 후 실제 검증용 커밋을 만든다.
- 종료 보고에는 변경 파일과 라인 범위, brief와 다르게 처리한 부분 및 이유, 검사 명령과 결과, 미해결/후속 작업, 찾지 못했거나 검증하지 못한 내용을 포함한다. 빈 항목도 `없음`을 명시한다.
- 실제 실패 증거를 보존하며 assertion이나 자원 제한을 낮춰 통과시키지 않는다. 같은 수정 두 번 실패, 반복·맥락 손실·허위 통과가 관측되면 코디네이터가 해당 분기를 중단한다. Luna → Terra → Sol 순으로 한 단계씩 올리고 모델과 effort를 동시에 바꾸지 않는다.

### WP-1 — 기준 시험과 계측

- 작업 디렉터리: `/data/worktrees/3puw275b/noble-crab`.
- 분류: intermediate, medium. 기존 계약은 명확하지만 실제 warm/삭제 경로와 계측 신뢰성은 미검증이다. binary/Fresh 불일치나 오류 은폐는 상향 신호다.
- 소유 파일: `scripts/qa/build-cache-smoke.sh`, `scripts/qa/build-cache-benchmark.sh`, `tests/fixtures/build-cache/**`, 신규 `docs/verification/production-build-baseline.md`. 전용 임시 증거 디렉터리만 `/tmp`에 추가할 수 있다.
- 입력: 기준 HEAD, 기존 검증 문서, 해당 QA 도구. 제품 Dockerfile·Compose·운영 스크립트·기존 검증 문서는 수정하지 않는다.
- cold는 새 namespace와 레이어 재사용 차단을 함께 사용한다. warm은 현재 consumed ARG 방식으로 RUN만 무효화하고, 같은 입력 반복은 Docker 레이어 캐시를 허용한다. 삭제 실패는 실제 오류 원인과 실패 이미지 부재로 확인한다.
- 현재 binary 출력과 Cargo Fresh/Compiling 증거를 유지한다. commit-only, Rust, 내부 라이브러리, 의존성/lockfile, build script, 내장 데이터, 설정, 오래된 mtime, 브랜치 전환, 소스 삭제/복구, 빈 캐시, 중간 실패를 포함한다.
- benchmark의 현재 단일 probe를 시나리오별로 구분할 계측 계약을 작성한다. Web-only와 Python-only도 포함하고 각 기준에 동일한 의미의 변경을 적용한다. 도구·소스·probe·계측 변환의 SHA, 도구 버전, OS/플랫폼, 캐시 상태를 기록한다.
- 전체 경과 시간, 컴파일 패키지/설정, Cargo 시간, 이미지 생성·검증 시간, peak memory·swap·디스크를 기록할 방법을 정의한다. 링크 시간을 따로 얻을 수 없다면 미분리로 표시하고 Cargo RUN 시간 전체를 링크 시간으로 부르지 않는다. 계측을 추가하면 모든 비교 대상에 동등하게 적용한다.
- `/proc`와 Docker의 제한된 상태 필드, systemd 상태, kernel journal 접근 결과를 확인한다. journal 오류를 숨긴 빈 출력은 OOM 없음이 아니다. 기존 하한 2GiB MemAvailable/512MiB SwapFree를 낮추지 않는다. 이전 서비스 장애의 복구는 하지 않는다.
- 검증: `bash scripts/qa/build-cache-smoke.sh --self-test`, `bash scripts/qa/build-cache-benchmark.sh --self-test`, 해당 파일 `bash -n`. 조건 충족 시에만 `build-cache-smoke.sh --apply --output-dir <새 절대 경로>`로 실제 검증한다. 불가하면 도구 작업·명세는 완료하되 실제 시험과 성능은 미검증으로 분리한다.
- 산출물: 계측/시나리오 명세, 수정한 시험, 재현 명령과 증거 경로, 호스트 선행 조건. 공통 종료 보고 형식을 따른다.

### WP-2 — 구조 비교와 실험 명세

- 작업 디렉터리: `/data/worktrees/3puw275b/noble-crab`.
- 분류: hard, high. 빌드 입력과 산출물 출처·운영 중단 단위가 결합된 아키텍처 판단이다. 기존 기능/자원 계약 변경을 전제로 한 결론은 거부한다.
- 소유 파일: 신규 `docs/design/production-build-layout.md`만. 제품 코드, WP-1의 변경 파일, 실험 코드는 수정하지 않는다.
- 읽기 입력: 기준 HEAD의 Dockerfile 7개(아래 D7), `Cargo.toml`/`Cargo.lock`, 관련 workspace manifests/build scripts, `deploy/compose/compose.yml`, `scripts/ops/build-production-images.sh`, `scripts/ops/compose-release.sh`, `scripts/ops/lib/release-image-manifest.sh`, 기존 런타임 소비 경로와 도표. 필요 시 고정 HEAD 스냅샷에서 조사한다.
- 서비스 → Dockerfile → package/bin → 내부/외부 의존성 → feature/profile/toolchain → 런타임 COPY → 커밋 검증의 매핑을 작성한다. Dockerfile 7개를 실제 Rust 컴파일 7회와 동일시하지 말고 Compose 이미지별 중복/공유를 구분한다.
- 반드시 세 방식을 비교한다: 현행 캐시 유지/보완, 공통 Rust 산출물 빌더, 역할/의존성 그룹별 산출물 빌더. 같은 입력의 재사용, Web/Python-only 변경, cold 비용, feature 차이, 최종 링크 비용, 재개 단위, 캐시 증가, 유지보수 비용을 평가한다. 이 세 구조안과 문서 앞부분의 제품 커밋 기준 A/B/C는 서로 다른 비교 축이다.
- 공통 빌더라는 이름으로 모든 바이너리를 매번 재빌드하거나 feature를 임의 통합하지 않는다. 바이너리별 입력 해시와 invalidation 범위, 커밋 내장값 갱신, runtime 이미지 출처 검증, 중간 산출물의 불변 참조·완료 표시·실패 시 비가시성을 명시한다.
- 공통/그룹 산출물에도 순차 실행과 2~3개 소비 서비스 단위의 확인 지점이 있어야 한다. 이 조건을 만족하지 못하는 단일 거대 RUN은 이유와 함께 부적합으로 분류한다.
- 공통안·그룹안에 대해 WP-3이 판단 없이 구현할 소형 실험 명세를 작성한다. 각 빌드 단계, 입력 파일/설정, 출력 바이너리, 소비 이미지, 예상 재컴파일 집합, 실패 주입·재개 방법을 고정한다. 현행안은 동일 입력의 기준 실험으로 둔다.
- 제품 적용 명세 초안에는 사용할 기존/신규 파일의 정확한 목록과 helper/Compose build 인터페이스를 포함한다. 아래 WP-4 허용 파일 집합 안에서 설계한다. 새로운 경로가 필수라면 구현 전에 코디네이터에게 명시한다.
- 검증: 모든 구조 주장에 코드 `file:line` 근거를 붙이고, Docker/Cargo 기능을 사용할 때 실행 버전과 공식 문서로 확인한다. 수치가 없는 이득은 가설로 표시한다. 최종 구조 선택은 코디네이터가 한다.
- 산출물: 세 대안 비교, 공통안·그룹안 실험 명세, 제품 적용 초안, 확인 불가 항목. 공통 종료 보고 형식을 따른다.

### WP-3 — 소형 구조 실험

- 작업 디렉터리: `/data/worktrees/3puw275b/noble-crab`.
- 분류: intermediate, medium. G1이 동결한 명세만 구현한다. 명세가 비어 있거나 추가 구조 판단이 필요하면 착수하지 않는다.
- 소유 파일: 신규 `tests/fixtures/build-layout/**`, `scripts/qa/build-layout-probe.sh`, `docs/verification/production-build-layout-probe.md`. 기존 smoke/benchmark와 제품 코드에는 쓰지 않는다.
- 입력: WP-1 계측 계약과 고정된 fixture, WP-2의 현행안·공통안·그룹안 실험 명세. fixture를 복사/참조하되 원본을 변경하지 않는다.
- 두 바이너리 이상과 여러 소비 이미지를 가진 소형 workload에 세 구조안을 적용한다. 내부 라이브러리·외부 의존성·feature 차이·커밋 내장·build script·내장 데이터를 포함한다. 실제 provider/운영 이미지는 사용하지 않는다.
- 계획 출력이 기본이며 실제 실행은 명시적 `--apply`로 구분한다. 임시 태그/checkout/캐시 namespace를 분리하고, 서비스/빌드 단계는 순차로 실행한다. 공통 단계의 생성 비용과 실패를 결과에서 빠뜨리지 않는다.
- 정상 출력뿐 아니라 중간 빌드 실패, 잘못된 산출물 커밋, 누락된 바이너리, 중단 후 재개, 오래된 캐시/삭제 입력을 검증한다. 실패한 공통 산출물에서 소비 이미지를 성공 처리하면 안 된다.
- 검증: 신규 도구 `--self-test`, `bash -n`, 조건 충족 시 실제 `--apply`. 예상 재컴파일 집합과 실제 Cargo 기록, 각 소비 이미지의 binary 출력, 실패/재개 순서를 대조한다.
- 산출물: 재현 가능한 세 구조의 소형 실험, 정확성·중복·소요 시간·자원·회귀 비교. 소형 결과를 12개 제품 이미지의 시간 예측이나 최종 성능 증명으로 확대하지 않는다. 공통 종료 보고 형식을 따른다.

### WP-4 — 선택안의 제품 구현

- 작업 디렉터리: `/data/worktrees/3puw275b/noble-crab`.
- 분류: intermediate, medium. G2에서 코디네이터가 선택한 구조와 상세 인터페이스가 완전히 동결된 경우에만 이 분류로 시작한다. 미정 설계를 Luna에게 맡기지 않는다.
- 입력: G2의 선택 근거·정확한 입력/산출물/빌드 DAG·재개/검증 명세, WP-3 결과, 고정 후보 SHA. 구현자는 구조를 다시 선택하지 않는다.
- 소유 범위 D7:
  - `crates/api-server/Dockerfile`
  - `crates/job-queue/Dockerfile`
  - `crates/job-queue/Dockerfile.owner-beta-runner`
  - `crates/job-queue/Dockerfile.owner-equity-v2-runner`
  - `crates/job-queue/Dockerfile.backtest-runner`
  - `data-pipelines/collectors/Dockerfile`
  - `deploy/runtime/Dockerfile.paper-runner`
- 추가 허용 파일: `deploy/compose/compose.yml`의 build 관련 키만, `scripts/ops/build-production-images.sh`, `scripts/ops/compose-release.sh`, 신규 `scripts/ops/lib/release-build-layout.sh`, `deploy/build/Dockerfile.rust-artifacts`, `deploy/build/release-build-layout.json`. 실제 사용할 부분집합은 G2에서 고정한다. 선택안에 불필요한 파일은 만들지 않는다.
- 동일 Dockerfile을 소비하는 serving 외 서비스도 WP-2가 작성한 목록과 대조한다. 해당 서비스의 별도 실행/profile을 활성화하지 않고 빌드 호환성만 보존한다.
- 소비 이미지가 검증된 동일 커밋 산출물을 가져오도록 구현한다. 중간 산출물은 최종 12개 manifest 레코드에 추가하지 않는다. shared builder가 Cargo mount 밖으로 실행파일을 내보내는 시점과 불변 식별자를 명시적으로 검사한다.
- 현재 CLI와 전체 12개 순회·최종 검사·원자적 manifest 생성·덮어쓰기 거부를 유지한다. 실패 즉시 후속 작업을 멈춘다. 빌드 완료 여부를 태그 존재만으로 판정하지 않는다.
- 검증: 변경 파일 syntax/명세 대조와 선택안의 Fake 흐름. Wave 3 중 실제 Docker/Rust 빌드는 하지 않는다. WP-5와 통합한 뒤 코디네이터가 전체 회귀 검사를 수행한다.
- 산출물: 제품 구현, 입력/산출물 경로 목록, 기존 동작 보존 근거. 공통 종료 보고 형식을 따른다.

### WP-5 — 검사·계측·운영 절차 통합

- 작업 디렉터리: `/data/worktrees/3puw275b/noble-crab`.
- 분류: intermediate, medium. 기존 검증과 새 DAG의 정합성이 핵심이다. 시간 이중 합산, 단계 누락, assertion 완화는 즉시 반려한다.
- 소유 파일: `scripts/ops/build-production-images-static-check.sh`, `scripts/ops/build-production-images-self-test.sh`, `scripts/ops/production-ops-static-check.sh`, `scripts/ops/production-ops-self-test.sh`, `scripts/qa/build-cache-benchmark.sh`, `scripts/ops/README.md`, `docs/runbooks/production-release-and-backup.md`.
- 필요할 때만 추가 소유: `docs/diagrams/component_architecture.puml`, `docs/diagrams/runtime_deployment.puml`, `docs/diagrams/component_architecture.png`, `docs/diagrams/runtime_deployment.png`. WP-1과 benchmark 소유는 wave가 달라 겹치지 않으며 WP-4의 제품 코드와는 겹치지 않는다.
- 입력: G2 명세, WP-1 계측 계약, WP-3 실험. WP-4와 같은 명세에서 작성하고, 실제 구현과의 최종 검사는 양쪽 작업이 끝난 뒤 수행한다.
- 기존 검사는 현재 Dockerfile의 문자열 모양 대신 보존할 계약과 새 산출물 출처를 검증하도록 바꾼다. 기존 보호 항목을 삭제하거나 expected 값을 실제 결과에 맞춰 낮추지 않는다.
- Fake Docker는 순차 호출, 잘못된 커밋/이미지 ID/산출물, 바이너리 누락, 공유 빌드 실패, 중간 실패 후 재실행, manifest 경로 사전 거부, lifecycle 명령 부재를 검사한다.
- benchmark는 새 공유 단계와 12개 이미지/검사 전체를 계측한다. A/B/C를 비교할 때 같은 도구·입력 변형·리소스 제한·초기 캐시 상태를 사용한다. 공통 단계 실행 시간을 한 번만 합산하고 캐시 유지 비용·이미지 저장 공간도 기록한다. 제품 구조의 차이를 숨기는 사전 준비는 금지한다.
- runbook에 산출물 준비 → 순차 소비 이미지 빌드 → 최종 공식 builder/manifest 검증, 실패 복구·캐시/임시 산출물 보존 범위, background systemd와 묶음 사이 게이트를 명시한다.
- 구조가 변하면 해당 도표와 코드 근거를 함께 갱신하고 로컬 PlantUML로 렌더한다. 구조가 그대로라면 도표를 바꾸지 않는다. Dockerfile 이동 때문에 기존 Paper line-26 근거가 낡으면 현재의 실제 근거로 고친다.
- 검증: 각 변경 shell `bash -n`, 관련 ops static/Fake 검사, benchmark `--self-test`. 실제 Docker/Rust 빌드는 WP-6에 맡긴다. 로컬 도표 렌더가 필요하면 코디네이터가 별도 Docker 사용 시간을 배정한다.
- 산출물: 회귀 검사·정확한 계측·운영 절차·해당 도표. 공통 종료 보고 형식을 따른다.

### WP-6 — 독립 인수

- 작업 디렉터리: `/data/worktrees/3puw275b/noble-crab`.
- 분류: hard, high. 구현자와 분리된 검토자가 실제 출처·캐시 정확성·성능·운영 자원을 종합해 판정한다. 불분명한 실측은 성공으로 결론 내리지 않는다.
- 소유 파일: 신규 `docs/verification/production-build-architecture.md`와 전용 `/tmp` 증거만. 제품/QA 코드와 기존 검증 문서를 수정하지 않는다.
- 입력: G3의 깨끗한 C 커밋, A/B 커밋, 최종 계측 도구 SHA, G2 명세와 WP-3 증거, 실행 대상·유효한 image-only 권한·자원 관측.
- 순서: 정적/Fake/도구 self-test → 현재 smoke와 선택안 fixture 실제 검증 → 깨끗한 C의 공식 12개 이미지·V2 manifest → 통제된 A/B/C 성능 비교. 실제 실행 조건이 없으면 가능한 읽기 전용/오프라인 검증만 진행한다.
- 여섯 기존 검사, 신규 layout probe, 별도 `scripts/ops/static-check.sh`와 `scripts/ops/self-test.sh`를 구분한다. 기존 fakeroot/chown 또는 파일 모드 문제가 다시 발생하면 정확한 환경 실패로 기록한다. 가짜 root fixture 통과나 테스트 생략을 인수 통과로 표시하지 않는다.
- 캐시·산출물의 정확성을 binary 출력과 Cargo 재사용 증거로 검증한다. 제품 바이너리 검사는 WP-2가 정의한 부작용 없는 방법만 사용한다. provider에 접근하는 실행 옵션은 쓰지 않는다.
- 실제 공식 release manifest 검증용 소스는 깨끗한 고정 커밋이어야 한다. 계측을 위해 바꾼 임시 소스/라벨의 벤치마크 이미지는 공식 릴리스 이미지라고 주장하지 않는다. 같은 태그를 덮어쓰거나 이미 존재하는 manifest를 교체하지 않는다.
- 성능은 먼저 cold와 warm을 분리한다. 모든 시나리오를 무조건 여러 번 전체 빌드하지 않는다. 대표 warm 시나리오 하나를 사전에 고정하고 B/C에 독립 warm-up 후 최소 3쌍을 확보해 순서를 교차한다. A와 cold·다른 변경 유형은 우선 같은 조건의 1회 관측으로 비용/회귀를 확인하며 표본 수를 공개한다. 변동이나 회귀 때문에 필요한 추가 반복은 이유와 실행 비용을 먼저 명시한다. 예산/자원 때문에 반복이 부족하면 확정적 개선율을 제시하지 않는다.
- 전체 시간의 중앙값·범위, 패키지별 재컴파일, 준비/컴파일/링크(분리 가능 시)/이미지/검증 시간, 메모리/swap·OOM·health·캐시 디스크를 비교한다. 개선 폭이 측정 변동과 구분되지 않으면 원인과 추가 측정 필요를 보고한다.
- 구현 결함은 재현과 파일/라인을 소유 작업자에게 반환하고 실제 빌드를 멈춘다. 코디네이터의 수정 커밋이 고정된 뒤 후속 검증을 재개한다. 실행 중인 검토 대상 파일은 코디네이터도 수정하지 않는다.
- 산출물: 커밋별 인수 행렬, 실제 명령/증거 위치, 성능과 회귀, 미검증 항목과 재개 조건. 운영 조건이나 실측이 미충족이면 전체 완료로 보고하지 않는다. 공통 종료 보고 형식을 따른다.

## Coordinator gates

### 1. 착수 전

- 작업 트리/HEAD/적용 지침과 변경 충돌을 확인한다. 이 계획 이후의 정상 변경은 diff를 검토하고 기준을 명시적으로 갱신한다.
- `$paseo-delegate` 파일과 지정 모델/effort 지원을 확인한다. 스킬을 사용할 수 없으면 작업자 실행 전에 중단하며 native subagent로 우회하지 않는다.
- 모든 프롬프트에 공통 계약·분류·파일 소유·의존 산출물·보고 형식을 포함한다. Paseo의 실행/관찰/회수는 실행 시점 `$paseo-delegate`를 따른다.
- 오프라인 작업에 필요한 미결 사용자 결정은 없다. 시간 상한은 미정 상태를 유지한다. 운영 빌드는 현재 유효한 권한과 대상 커밋을 확인한다. 승인이 없다면 먼저 구체적인 명령·태그·systemd 설정·자원/health 기준을 준비한 뒤 필요한 범위만 확인한다. 과거 만료된 예외를 재사용하지 않는다.

### 2. G1 — Wave 1 통합과 실험 명세 동결

- WP-1의 허위 통과 방지·cold/warm 구분·증거 보존과 WP-2의 세 대안 비교를 직접 확인한다.
- WP-1 계측 인터페이스와 WP-2 실험 명세가 어긋나면 코디네이터가 정합화하고 고정 SHA/명세를 만든다. 누락된 essential input을 WP-3이 채우게 하지 않는다.
- 현재 fixture의 실제 재시험이 막혔으면 오프라인 prototype 작성은 가능하지만 실제 캐시 정확성/성능 판정은 보류한다. 실제 빌드 슬롯과 조건을 충족한 뒤 WP-3에 인계한다.

### 3. G2 — Wave 2 검토와 제품 구조 선택

- 세 구조 실험의 입력·출력 동등성, invalidation 범위, 커밋 출처, 순차 실행·재개 가능성, 공통 준비 비용 포함 여부를 확인한다.
- 코디네이터가 선택 이유, 탈락 이유, 회귀/불확실성, 제품에 적용할 정확한 파일/인터페이스/검증 명세를 `docs/design/production-build-layout.md`에 확정한다. 소형 실험의 성능은 제품 성능 보장이 아니다.
- 실제 구조 실험이 막혀 필수 정확성을 판단할 수 없다면 이 gate를 통과시키지 않는다. 두 새 구조 모두 이점이 없으면 그 결과를 보고하고 억지로 재구성하지 않는다. 현재 안 유지가 합리적이라는 결론과 추가 단축 목표 달성은 구분한다.
- 고정 경계 안의 설계 선택은 코디네이터가 수행한다. 추가 서버·기능/런타임/자원 정책 변경이 필요하면 해당 분기의 graph/brief를 수정하고 필요한 사용자 결정을 받는다.

### 4. G3 — Wave 3 통합

- 두 작업자가 idle로 완료됐음을 확인하고 보고와 실제 diff를 대조한다. 소유 범위 밖 수정과 검증 완화를 거부한다.
- 관련 기존/신규 static·Fake·도구 self-test를 통과시키고 정확한 C 커밋을 고정한다. 모델과 effort를 동시에 상향하지 않는다.
- 도표가 달라졌거나 코드 근거가 이동했다면 실제 근거와 PNG를 확인한다. 인터페이스 누락 또는 구조 판단이 다시 필요하면 WP-4의 구현을 억지로 계속하지 않고 G2로 반환한다.
- WP-6이 끝나기 전에는 구현 코드를 수정하지 않는다. 결함 수정은 reviewer가 멈추고 결과를 반환한 뒤 소유 작업자가 수행한다. 새 후보 SHA를 다시 전달한다.

### 5. 최종 인수

| 시나리오/계약 | 합격 기준 |
|---|---|
| 동일 커밋 반복 | 완료된 단계 재사용, 바이너리 출력/커밋/이미지 출처 동일 |
| Web/Python-only 변경 | Rust 공통 작업의 불필요한 재실행 감소; 내장 커밋은 최신 값 |
| Rust·내부 라이브러리·의존성/설정·내장 데이터 변경 | 새 결과 반영, 영향 범위 설명 가능, 오래된 산출물 혼입 없음 |
| 오래된 mtime·브랜치 전환·삭제 | 현재 소스 결과 또는 원인이 확인된 명시적 실패 |
| cold·캐시 재생성 | 필요한 모든 산출물 생성, warm cache 존재 가정 없음 |
| 공유/개별 빌드 실패와 재개 | 실패 산출물 비가시, 후속 이미지/manifest 미발행, 안전한 재실행 |
| 공식 릴리스 | 12개 이미지·바이너리/경로·커밋/revision·strict V2·기존 설치/롤백 계약 통과 |
| 성능 | A/B/C 동등 조건, 공통 단계 포함 전체 시간, B 대비 C의 대표 변경 빌드 개선이 측정 변동과 구분됨 |
| 운영 | 순차/jobs=2/묶음 게이트 보존, 비교 가능한 자원 조건, 새 OOM/서비스 영향 없음이 유효한 관측으로 확인됨 |

실제 검증이나 성능 비교가 실행되지 않았다면 구현 완료와 전체 인수 완료를 구분한다. 최종 보고에는 완료·실패·미검증, 각각의 후보 SHA와 재개 조건을 명시한다.
