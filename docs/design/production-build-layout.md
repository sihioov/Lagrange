# 운영 이미지 빌드 구조 비교와 WP-3 실험 명세

상태: **G1 오프라인 실험 명세 동결. 제품 구조 선택과 G2는 미정이다.**

2026-09-15 실행 재개: 소유자 승인에 따라 [재개 조건](../superpowers/plans/2026-09-15-production-build-resume.md)의
정확한 기존 research-worker 장애만 image-only 시험 중 별도 감시할 수 있다.
이 예외의 명시적 입력이 없으면 아래 §5.7의 엄격한 기본 조건을 그대로 적용한다.
실험 입력·예상 Cargo 재사용·산출물 정확성·G2 인수 조건은 변경하지 않는다.
실제 실행에서 드러난 커널 로그 1,000건 절단 한계는 같은 재개 문서의
`Complete kernel journal capture` 계약으로 보완한다. 전체 구간의 EOF·성공 종료와
자원 상한을 함께 검증하며, §5.7의 OOM·건강·자원·재개 조건은 유지한다.
이 문서는 실험 구현 명세를 제공한다. 실제 Docker/Rust 빌드, 프로토타입 구현,
운영 변경, 성능 인수 또는 G2 선택을 완료했다는 뜻이 아니다.

## 1. 기준, 범위, 현재 실행 가능성

- 작업 경로: `/data/worktrees/3puw275b/noble-crab`.
- 작업 시작 HEAD: `6f2571da8820df092bb4e54da795a487137fbd50`.
  `git diff --stat f4eb4f8 6f2571d`는 승인된 계획 문서 하나, 208행 추가뿐이다.
- 제품 비교 A: `d1baf9da9b13fcb61649b1c26de56aed87a83418`.
- 제품 비교 B: `f4eb4f83abb7c3f43ede072d0077a1a74edd0ee3`.
- 제품 비교 C: G3에서 코디네이터가 만드는 깨끗한 선택안 커밋. 아직 없음.
- 이하 코드 `경로:행`은 별도 표시가 없으면 **B의 `git show` 행 번호**이다.
  조사 시점의 동시 QA 변경을 B의 증거에 섞지 않는다. 이 문서 자체의 SHA는
  코디네이터가 G1에서 고정하고, WP-1 도구 SHA/fixture SHA와 함께 WP-3에 전달한다.
- 아래 구조 L0/L1/L2는 제품 커밋 A/B/C와 다른 비교 축이다. C=L1로 예약하지 않는다.
- 소유 변경은 이 파일뿐이다. D7, Web/DB Dockerfile, `.dockerignore`, Cargo 설정,
  runtime Compose, provider 코드, 기존 QA 파일을 수정하지 않는다.

적용 지침: `/home/l1nnx/.codex/AGENTS.md`, 저장소 `AGENTS.md`,
`/home/l1nnx/.agents/skills/paseo-delegate-plan/SKILL.md`,
`/home/l1nnx/.agents/skills/paseo-delegate/SKILL.md`를 읽었다.
WP-2는 배정된 작업자로 직접 수행하며 추가 위임을 하지 않는다.
하위 provider 지침은 `apps/web/AGENTS.md`, `apps/web/CLAUDE.md`만 발견했다.
본 실험의 web/python 변경은 **합성 fixture의 runtime 파일 변경**이다.
실제 `apps/web` 변경 시나리오의 파일 선택/수정은 WP-1 및 후속 작업의 소유이고,
그 직전에 해당 Web 지침과 설치된 Next 문서를 읽어야 한다.

2026-09-14의 제한된 읽기 관측:

| 항목 | 관측/판정 |
|---|---|
| 메모리 | `free -m`: total 14841 MiB, available 1432 MiB, swap free 0 MiB. 순간 관측이며 과거 값의 승계가 아니다. |
| 서비스 | `docker ps`에서 `lagrange-station-research-worker-1`은 `Restarting (2)`; 다른 표시된 production 상시 서비스 10개는 healthy. 이 상태로 실험 게이트 통과 불가. |
| OOM | `journalctl -k --since '30 minutes ago' -n 80 --no-pager --grep='oom\|Out of memory\|Killed process'`는 `-- No entries --`, exit 1. 필터 없는 읽기/범위 연속성을 입증하지 않았으므로 **신뢰 가능한 OOM 없음 증거로 채택하지 않음**. |
| 실행 버전 | Docker client/server 29.7.2, Compose v5.4.0, Buildx v0.36.1 (`1d8dde89b8aba914e05e45366770736fea1fd690`), default docker driver의 BuildKit v0.32.2, 지원 플랫폼 linux/amd64 계열. 호스트 Cargo/rustc 1.97.1. |
| 빌더 실행 확인 | 실제 pinned Alpine 컨테이너 내부 `rustc -vV`, Cargo, linker, apk package inventory는 미실행. Dockerfile pin과 호스트 버전만 확인했으며 동일하다고 대체하지 않는다. |

Docker 조회와 Mem0 관련 이력 조회는 좁은 native 읽기 권한으로 수행했다.
Docker build/run, Rust compile, provider 호출, 서비스 조작은 하지 않았다.
기존 fixture는 `docs/verification/production-build-cache.md:142`의 cold/동일 입력만
실제 통과했고, warm 도구 수정 재시험과 삭제 판정은 같은 파일 `:172`, `:176`에서
미검증이다. 이를 해결하기 전 새 구조의 실제 캐시 정확성/성능을 합격으로 판정하지 않는다.
기존 image-only 예외의 만료를 해제하거나 이전 세션의 유효한 fixture 승인을 재요구하지 않는다.

## 2. 현재 빌드 DAG와 고정 계약

### 2.1 정확한 D7 및 서비스 매핑

D7는 시작 HEAD의 계획 `docs/superpowers/plans/2026-09-14-production-build-architecture.md:120`
에서 읽은 다음 일곱 파일이다. `bin`은 모두 Cargo의 `--bin` 이름이다.

| ID / Dockerfile | release 소비 서비스 → Compose 근거 | package / bin | runtime COPY·entrypoint 근거 |
|---|---|---|---|
| D1 `crates/api-server/Dockerfile` | `api-server` → `deploy/compose/compose.yml:140` | `api-server / api-server`, D1 `:33` | `/usr/local/bin/api-server`, D1 `:54`, `:61` |
| D2 `crates/job-queue/Dockerfile` | `recommendation-runner`, `candidate-runner` → Compose `:865`, `:947` | `job-queue / recommendation-runner`, `candidate-runner`, D2 `:28` | 두 bin을 `/usr/local/bin/<bin>`에 유지, D2 `:47`; default entrypoint recommendation, `:56`; candidate 실행 선택은 Compose에 유지 |
| D3 `crates/job-queue/Dockerfile.owner-beta-runner` | `owner-beta-runner` → Compose `:1008` | `job-queue / owner-beta-runner`, D3 `:34` | `/usr/local/bin/owner-beta-runner`, D3 `:49`, `:54` |
| D4 `crates/job-queue/Dockerfile.owner-equity-v2-runner` | `owner-equity-v2-runner` → Compose `:1076` | `job-queue / owner-equity-v2-runner`, D4 `:42` | `/usr/local/bin/owner-equity-v2-runner`, D4 `:58`, `:64` |
| D5 `crates/job-queue/Dockerfile.backtest-runner` | `nt-backtest-worker-1`, `nt-backtest-worker-2` → Compose `:1176`, `:1243` | `job-queue / backtest-runner`, D5 `:42` | `/usr/local/bin/backtest-runner`, D5 `:63`, `:72` |
| D6 `data-pipelines/collectors/Dockerfile` | `research-worker` → Compose `:533` | `collectors /` 아래 10개, D6 `:38` | 모두 `/usr/local/bin/<bin>`, D6 `:72`; entrypoint research-worker, `:90` |
| D7 `deploy/runtime/Dockerfile.paper-runner` | `paper-scheduler` → Compose `:1313` | `api-server / paper-runner`, D7 `:26` | Rust 파일 `/usr/local/bin/paper-runner-bin`, wrapper `/usr/local/bin/paper-runner`, D7 `:39`, `:40`, `:46` |

D6 출력 순서는 다음으로 고정한다 (`data-pipelines/collectors/Dockerfile:38–58`):

1. `research-worker`
2. `kis-historical-price-beta-artifact`
3. `kis-historical-price-v3-artifact`
4. `kis-historical-price-beta-approval-check`
5. `kis-action-range-raw`
6. `kis-stock-price-beta-raw`
7. `kis-stock-price-beta-materialize`
8. `kis-historical-price-v3-input-check`
9. `owner-equity-v2-check`
10. `owner-equity-v2-materialize`

나머지 3 release record도 전체 비용과 검사에 포함한다:

| 서비스 | 빌드/산출물 | 근거 |
|---|---|---|
| `db-role-bootstrap`, `db-migrate` | 같은 `deploy/db/Dockerfile`; `cargo install sqlx-cli --version 0.9.0 --locked --no-default-features --features postgres,rustls`; `/usr/local/bin/sqlx`, migrate/bootstrap wrapper 및 SQL | Compose `:285`, `:362`; `deploy/db/Dockerfile:10`, `:22–31` |
| `web` | `apps/web/Dockerfile`; npm deps → Next build → standalone/static → `node apps/web/server.js` | Compose `:99`; Web Dockerfile `:12–15`, `:27–33`, `:53–60` |

즉 **12 service record, 9개의 D7 Rust 소비 서비스, 7개 D7 recipe,
17개 서로 다른 workspace bin**이다. DB도 Rust-producing이지만 D7 workspace cache의
대상은 아니다. D2 두 tag, D5 두 tag, DB 두 tag는 동일 입력이면 build layer를 공유할
수 있다. 같은 image ID가 여러 record에 등장해도 서비스 record를 합치지 않는다.
D7 recipe를 처음 한 번씩 실행하면 `cargo build` 명령은 17개이고, Docker 재사용이
안 되어 9개 소비 서비스의 compile RUN이 각각 실행되면 20개다. 이것은 명령 수이며
rustc unit 수/실행 시간의 실측값이 아니다. 실제 layer cache hit 여부는 BuildKit 기록으로
따로 센다 ([Docker cache](https://docs.docker.com/build/cache/invalidation/)).

D6를 별도로 소비하는 profile 서비스도 삭제/축소하면 안 된다:
`research-range-raw` (`deploy/compose/compose.yml:627`),
`research-action-range-raw` (`:678`), `research-stock-price-beta-raw` (`:723`),
`research-stock-price-beta-materialize` (`:768`), `research-v3-input-check` (`:797`),
`research-range-raw-recovery` (`:832`). 이들은 12 record에 추가하지 않고 실행하지 않는다.
D6의 contract fixtures 두 디렉터리, fixed-stock universe 및 approval registry runtime COPY도
보존한다 (`data-pipelines/collectors/Dockerfile:82–85`).

D2/D5의 Python runtime은 `nt` 전체와 `uv sync --locked`를 사용한다
(D2 `:49–51`, D5 `:64–66`). recommendation의 실제 child는
`crates/job-queue/src/recommendation/child.rs:509–574`의
`python -m strategies.recommendation_cli`이다. Python-only 변경을 빌드에서 빼거나
runtime payload를 줄이는 설계는 제외한다.

### 2.2 내부 의존성과 feature/profile

다음은 normal path dependency 그래프이며 테스트 전용 간선은 제외했다.
경로 의존성의 전이 폐쇄가 각 package/bin의 내부 컴파일 입력이다.

| package | 직접 내부 의존성 | manifest 근거 |
|---|---|---|
| `api-server` | api-server-auth, auth, collectors, domain, factor-engine, risk-gateway, kis-client, job-queue, market-data, portfolio-model, result-model, selector | `crates/api-server/Cargo.toml:22–43` |
| `job-queue` | factor-engine, selector, portfolio-model, market-data, domain, collectors, kis-client | `crates/job-queue/Cargo.toml:13–23` |
| `collectors` | market-data, factor-engine, domain, kis-client, opendart-client | `data-pipelines/collectors/Cargo.toml:9–13` |
| `factor-engine` | domain, market-data | `crates/factor-engine/Cargo.toml:9–10` |
| `selector` | domain, market-data, factor-engine, auth | `crates/selector/Cargo.toml:9–12` |
| `portfolio-model` | domain, selector | `crates/portfolio-model/Cargo.toml:9–10` |
| `market-data` | domain, auth, kis-client, opendart-client, data-go-client | `crates/market-data/Cargo.toml:9–13` |
| `api-server-auth` | auth | `apps/api-server/auth/Cargo.toml:9` |
| `result-model`, `risk-gateway` | 각각 domain | `crates/result-model/Cargo.toml:9`, `crates/risk-gateway/Cargo.toml:9` |
| domain/auth/kis-client/opendart-client/data-go-client | normal 내부 의존성 없음 | 각각의 `Cargo.toml:8` 이하 dependencies |

공통 고비용 외부 가지는 Polars 0.54.4 (`crates/factor-engine/Cargo.toml:15`),
Arrow/Parquet 59.1.0 (`crates/market-data/Cargo.toml:19–20`), SQLx 0.9.0,
Tokio 1.53.1, reqwest/rustls 및 proc-macro/build dependency들이다.
실제 lock은 `Cargo.lock`, SHA-256
`6d5ddab719c794d7f504b97def293041f3a3792a570afc0a5f750818ef8cedc1`이다.
`cargo metadata --locked --offline --no-deps --format-version 1`로 16 workspace member,
production package들의 명시 feature map `{}`를 확인했다. 그러나 **package feature가
없다는 사실은 전이 의존성 feature가 같다는 뜻이 아니다**.

호스트 Cargo 1.97.1의 아래 조회는 compile 없이 성공했다:

```sh
cargo tree --offline --locked -p collectors --edges normal,build --target x86_64-unknown-linux-musl -f '{p} {f}' --prefix none
cargo tree --offline --locked -p job-queue --edges normal,build --target x86_64-unknown-linux-musl -f '{p} {f}' --prefix none
cargo tree --offline --locked -p api-server --edges normal,build --target x86_64-unknown-linux-musl -f '{p} {f}' --prefix none
```

주요 차이: SQLx는 collectors에 `json`/`default`/`any`가 없고, job-queue는 `json`을
추가하며, api-server는 `default,any`도 포함한다. Tokio `test-util`은 job/API 쪽에 있고
collectors 쪽에는 없다. 선언 근거는 collectors manifest `:21–23`, job manifest
`:29–35`, API manifest `:48–50`이다. `serde_json`의 `float_roundtrip`도 조회에서 다르다.
동일 package가 host build dependency와 target dependency로 다른 feature unit을 가질 수 있다.
원문 조회는 `/tmp/wp2-{collectors,job-queue,api-server}-musl-tree.txt`에 두었다.
Cargo 공식 문서는 `cargo tree`가 실제 컴파일 그래프와 정확히 같음을 보장하지 않으므로,
이 결과를 unit 재사용 증명으로 쓰지 않는다
([cargo tree](https://doc.rust-lang.org/cargo/commands/cargo-tree.html)).

D7 모두 같은 pinned Rust Alpine base, `/build`, release, `--locked`, jobs=2를 사용하며
명시적 `--features`, `--all-features`, `--no-default-features`, `--target`을 전달하지 않는다
(D1 `:4–10`, D2 `:1–7`, 각 compile 행). 실제 compile target은 pinned builder의 host
triple이며, 위 `--target ...musl`은 **조회 명령에만** 사용했다.
workspace resolver=3, edition=2024, rust-version=1.97.1은 `Cargo.toml:12–36`이다.
root의 profile 변경은 dev/test에만 있고 release override는 없다 (`Cargo.toml:52–77`).
기존 release 기본값을 유지하고 incremental/LTO/strip/linker 등의 변경을 이득에 포함하지
않는다 ([Cargo profiles](https://doc.rust-lang.org/cargo/reference/profiles.html)).
`.cargo/config.toml:20–21`의 jobs=3은 D7가 COPY하지 않는다. 이를 새 builder에 무심코
복사하여 유효 설정을 바꾸지 않으며 jobs=2 ENV를 유지한다.

`cargo build --workspace` 또는 여러 `-p`를 한 호출에 합쳐 feature union을 만드는 방법은
실험/제품 모두 제외한다. 기존처럼 **한 package의 한 bin씩** 호출한다. feature가 다르면
동일 source의 library라도 다른 unit으로 센다
([Cargo features](https://doc.rust-lang.org/cargo/reference/features.html)).

### 2.3 입력 데이터, 커밋, 릴리스 검사

| 입력/검사 | 실제 구조와 제약 |
|---|---|
| embedded data | market-data의 approval registry 3개: `crates/market-data/src/historical_price_only_approval.rs:54–59`, `src/range_to_canonical.rs:84`; calendar/manifest/overrides 및 ETF universe: `src/range_normalize.rs:57–63`; job-queue ETF universe: `crates/job-queue/src/recommendation/compute.rs:50–70`; API stock approval registry: `crates/api-server/src/http/equity_signals.rs:29`. 관련 package input hash에 포함한다. |
| build script | `crates/api-server/build.rs:11`은 `../../migrations`를 rerun 입력으로 선언한다. B의 D1/D7 COPY에는 migrations가 없다. missing path의 반복 invalidation 가능성은 실제 Cargo 기록으로 확인할 항목이며, L1/L2 초안에서는 migrations를 명시적 API 입력으로 제공한다. 테스트만 사용하는 SQL migration 내장을 새 production 기능으로 추가하지 않는다. |
| 컴파일 내장 커밋 | 전체 Rust source의 `env!`/`option_env!` 조사에서 실제 `LAGRANGE_CODE_COMMIT` compile read는 `crates/job-queue/src/bin/backtest-runner.rs:371–382`에서 확인됐다. release에서 runtime ENV와 다르면 실패한다. D5 `:31–42`가 compile ENV를 공급한다. |
| 기타 commit 전달 | D3 `:26–34`, D4 `:30–42`는 compile ENV를 공급하지만 해당 값의 Rust compile read는 발견되지 않았다. D6 `:30–47`는 compile RUN에 ARG가 유효하고 runtime ENV는 `:64`; D1은 runtime ENV `:47`, D2/D7은 OCI label만 해당 runtime stage에 둔다. 주석만으로 모든 bin의 compile attestation을 주장하지 않는다. 기존 ENV/ARG 범위는 유지한다. |
| final revision | D1 `:43–47`, D2 `:37–40`, D3 `:40–44`, D4 `:48–52`, D5 `:52–56`, D6 `:62–64`, D7 `:30–33`; Web `:40–43`, DB `:17–20`. 새 commit의 최종 tag/label은 반드시 새 commit이다. |
| official builder | `scripts/ops/build-production-images.sh:119–146`은 clean HEAD와 정확한 SHA를 확인한다. `:254–264`는 12 service를 한 번씩 순차 build한다. `:152–177`은 local image ID/revision 검사이며 **현재 binary contents 검사는 아니다**. `:180–203`은 V2 재검증, no-clobber publish, mode 0600이다. |
| 소비/rollback | `scripts/ops/lib/release-image-manifest.sh:8–22`가 canonical V2 목록. `scripts/ops/compose-release.sh:138–187`은 installed release/manifest를 바인딩하고 `:343–352`/`:395–438`은 image-ID 기반 `--no-build` 실행 및 재검사를 한다. 새 cache/artifact record를 V2에 추가하지 않는다. |

따라서 Web/Python-only commit에서 Rust **전부 재컴파일이 필수라는 주장은 틀리다**.
반대로 backtest의 compile commit 갱신까지 생략할 수도 없다. 새 source revision을
검증한 현재 producer가 Cargo를 실행하여 Fresh인 중간 결과를 재사용할 수는 있으나,
이전 commit의 완성 이미지/완료 artifact를 새 release로 relabel하는 경로는 금지한다.
Cargo는 `env!`/`option_env!` 값 변경을 감지한다
([build scripts](https://doc.rust-lang.org/cargo/reference/build-scripts.html#change-detection)).

## 3. 세 구조의 비교

B는 registry/git/target에 동일 `sharing=locked` cache ID를 사용하지만 매 compile RUN의
첫 명령은 `cargo clean --workspace --release --locked`이다 (D1 `:29–35`, D2 `:24–32`,
D3 `:30–36`, D4/D5 `:38–44`, D6 `:34–58`, D7 `:26`). 외부 산출물은 남길 수 있지만
내부 workspace 산출물은 다음 recipe 시작 시 지워진다. 동일 RUN 내부의 순차 bin들은
내부 코드를 재사용할 여지가 있다. `cargo clean --workspace` 의미는
[Cargo clean](https://doc.rust-lang.org/cargo/commands/cargo-clean.html)에 근거한다.
A의 D2 `:10`, `:23–24`, `:41`은 `nt`를 compile 전에 COPY하고 일반 target layer로
빌드한다. B의 cache mounts/직접 runtime NT COPY 효과와 새 구조 효과를 구분한다.

| 평가 축 | L0 현행 cache 유지/보완 | L1 공통 Rust 산출물 builder | L2 역할/의존성 그룹 builder |
|---|---|---|---|
| 구조 | D7 source build 유지. B 그대로가 기준 실험. 보완은 아래 입력 guard를 적용한 별도 후보로 구분 | 하나의 builder recipe/공유 target namespace에서 **한 bin씩 독립 호출/완료**; 소비 이미지가 검증된 산출물 받음 | 같은 recipe/출처 계약, `collectors`, `queue`, `api` target namespace 3개; 그룹 내부 bin은 순차 |
| 동일 입력 | B도 layer cache hit이면 compile 0. source RUN 강제 실행 시 workspace clean 비용 | 완료 record hash 재검증 후 같은 commit producer 재사용; layer miss라도 내부 Fresh 가능 | 동일. 그룹 별 cache와 완료 범위 분리 |
| Web/Python-only | B는 D2/D5 NT COPY가 runtime에 있지만 compile ARG가 있는 recipe는 commit 변경으로 재실행/clean 가능 | 새 commit producer는 다시 확인/Cargo 호출; backtest env unit 갱신, 나머지 Fresh 가설; NT runtime build 유지 | 동일하나 cache 3개와 그룹별 확인 비용 |
| cold | D7 내부 library 중복; DB install 포함 | 같은 feature unit의 중복 감소 가설. 입력 hash/복사/export/검사 비용 추가 | 그룹 경계에서 동일 library/외부 의존성 중복 가능. 그룹별 cold 생성 비용 |
| feature | 현행 package별 호출 보존 | feature union 금지. 호환 unit만 Cargo 재사용. 한 target의 variant 전환 손실 여부 측정 | API/queue/collectors의 다른 해석 그래프를 분리; 공통 부분 중복은 남음 |
| 최종 링크 | B는 매 source RUN에 필요한 bin 링크 재발생 가능 | 변경된 bin은 재링크 필요. common library가 Fresh여도 링크 무료 아님 | 동일. 공유 builder 자체가 링크 수를 줄이는 것은 아님 |
| 재개 | Docker의 완결 layer 단위. D6 긴 RUN은 내부 bin 중단 지점 없음 | bin 단위 원자 완료/export; 2~3 소비 서비스마다 gate, D6는 2~3 bin마다 추가 gate | bin 완료 + 그룹 cache; 그룹 하나를 거대한 RUN으로 만들지 않음 |
| cache 증가 | 현재 target + layer; 보완 시 hash ledger | variant와 내부 artifacts가 누적되고 완료 bundle의 중복 저장 비용 발생 | 3 target cache 및 외부 dependency 복제 가능; 더 큰 disk 가설 |
| 유지보수 | B는 낮음. selective invalidation을 넣으면 입력 inventory/guard 검증 비용은 L1 수준 | D7 경량 소비 경로, producer/receipt/임시 Compose override/실패복구 관리 필요 | L1 관리에 그룹 membership/version 검증 추가 |
| 주요 위험 | clean 삭제만 제거하면 old-mtime/삭제 입력에서 stale 결과 가능 | 공유 target 오염/feature variant 전환/불완전 export를 성공으로 오판 | 그룹 경계 중복이 공유 이득을 상쇄; source closure 누락 |

수치가 없는 모든 시간/컴파일 감소는 **가설**이다. L0 보완안은 L1/L2의 content guard를
D7 source RUN에 적용할 수 있지만, 이것은 B와 다른 후보이다. WP-3 baseline을 이 안으로
조용히 교체하지 않는다. 공유 산출물 export 이득이 없으면 L0 보완 선택도 가능하다.

**부적합안:** 한 `RUN cargo build --workspace`로 전체 산출물을 만들거나, 다수 Rust service를
`additional_contexts: service:...`로 연결해 한 동시 graph에 제출하거나, 모든 bin 완료를
단일 marker로만 나타내는 안. 각각 feature 계약/순차 정책/중단 및 재개 확인점을 충족하지
못한다. Docker가 shared stage나 service context를 지원한다는 사실은 운영 승인과 다르다
([multi-stage](https://docs.docker.com/build/building/multi-stage/),
[Compose build](https://docs.docker.com/reference/compose-file/build/#additional_contexts)).

## 4. L1/L2에 공통으로 적용할 출처·invalidation 계약

아래는 새 설계의 요구사항이다. 현재 코드가 이미 제공한다는 주장이 아니다.

### 4.1 세 종류의 identity

1. **cache compatibility key `K`**: schema version, builder image digest, builder 내부
   rustc/Cargo identity, platform 및 host triple, release profile의 유효값, Cargo.lock과 모든
   workspace manifest의 raw bytes, effective Cargo config/허용 compiler ENV, native toolchain
   package inventory, 고정 `/build`/target 경로, guard 구현 hash. 여기서 compiler ENV는
   target/linker/RUSTFLAGS 같은 호환 설정이며 commit 및 app build-script setting은 제외한다.
   그 값은 아래 H와 Cargo ENV tracking으로 처리한다. 설정/lock/member 변경은
   새 namespace. 제품 Cargo flags/features/profile을 변경하는 옵션은 받지 않는다.
2. **package content `P[p]` / bin input `H[b]`**: 정렬된 상대 경로·파일형식·mode·SHA-256,
   존재/삭제 및 empty-directory 표기를 포함한다. symlink는 원형과 대상 경계까지 검증하며
   바깥 경로는 거부한다. `mtime`은 identity가 아니다. `P`는 package subtree 전체
   (src/build.rs/manifest 포함) + 선언된 외부 입력이다. `H`는 package/bin 선택, feature
   resolution signature, `P`의 전이 폐쇄, 해당 compile ENV, `K`를 포함한다.
   초기 구현은 **package 단위로 보수적 invalidation**한다. 같은 package의 다른 bin 파일이
   바뀌어도 `P`가 달라지는 과잉 invalidation을 허용하되 기록한다. 완전한 Rust module parser나
   Cargo 내부 fingerprint 편집을 만들지 않는다.
3. **release artifact identity `R`**: exact source commit + `H[b]` + 실제 bin bytes hash +
   Cargo 성공 record + recipe/helper hash. cache key에서 release commit을 분리해도
   완료 artifact record의 commit을 생략하지 않는다. commit 내장 target은 commit이 `H`에도
   포함된다. 기존 ARG/ENV 제공 여부도 `H`에 기록하므로 unset과 빈 값을 혼동하지 않는다.

Rust source input은 B D7의 root manifest/toolchain, `crates`, collectors, API auth,
migration-contract manifests/source 및 위 embedded paths의 합집합이다. L1/L2에서는
API build script를 위해 `migrations`도 넣는다. D6 runtime fixture/input files는 compiler
input과 별도의 runtime payload hash로 기록하고 소비 bundle에 원래 상대경로로 보존한다.
`nt`, Web source, DB runtime SQL의 변화는 해당 runtime payload와 release record에만 반영한다
(API build script 입력인 migrations는 예외). `.dockerignore`는 그대로 둔다.
새 임시 source context는 tracked 파일 inventory로 만들고, 별도 비밀/환경 파일은 읽지 않는다.

Cargo config는 repository에 존재하는 것과 builder에 **실제로 적용한 것**을 구분해 기록한다.
B에서는 `.cargo/config.toml`이 빌드 컨텍스트에 있어도 COPY되지 않는다. 새 config가 나타나거나
새 compile-time read를 찾으면 inventory를 검토하기 전 fail closed한다. 생산 input inventory는
G2에서 고정해야 하며 미등록 build script/외부 include를 일반 glob으로 숨기지 않는다.

### 4.2 stale source 방지와 cache state

- target cache는 성능 수단이다. 성공의 source of truth로 사용하지 않는다. Docker cache mount는
  GC/다른 build에 의해 바뀔 수 있고 mount 내용 변경만으로 RUN cache가 무효화되지는 않는다
  ([Dockerfile cache mounts](https://docs.docker.com/reference/dockerfile/#run---mounttypecache)).
- mount **안**에 `K`, 전체 `P` inventory, 상태 version, pending transaction을 둔다.
  mount와 분리된 host의 “마지막 성공” 파일만 믿지 않는다. `sharing=locked` 안에서 guard와
  Cargo를 수행하고, host에서도 전체 build 실행 lock 하나를 잡는다.
- 새 source에서 `P`가 바뀐 package와 reverse local dependency closure를 구한다.
  그 package 각각에만 `cargo clean --locked --release -p <package>`를 실행한 뒤 빌드한다.
  이 conservative closure는 source 삭제/이전 mtime/branch A→B→A에도 동일하다.
  metadata 단계에서 manifest가 없어지거나 참조 입력이 누락되면 즉시 실패한다.
- cleaning 전에 pending을 쓰고, clean 뒤 새 inventory를 원자 교체하되 pending은 Cargo 성공 및 mount 밖 출력/receipt 작성까지 유지한다. 중간 종료/불명확한
  inventory/알 수 없는 guard version/일부 cache 손상 시 해당 `K` target 전체를 재생성한다.
  registry/git 및 다른 namespace를 prune하지 않는다. 이 fallback 비용도 측정한다.
- compile 실패 중 남은 target은 완료 artifact로 공개하지 않는다. 다음 호출은 pending 상태로
  분류하고 위 재생성 규칙을 따른다. source가 동일한 이전 **성공 artifact**는 독립 hash 검증으로
  다시 사용할 수 있지만 실패 직전의 mutable target을 그것으로 간주하지 않는다.
- compiler ENV만 달라진 경우 Cargo의 공식 env tracking을 사용한다. `backtest-runner`의
  commit 변경은 그 target의 rebuild를 요구한다. ENV 변화 때문에 공통 library까지 clean하지
  않는다. fixture에서 이를 입증하지 못하면 G2를 통과하지 못하며 B의 clean을 그냥 제거하지 않는다.
- Layer cache hit는 그 RUN이 읽은 **현재 input identity**의 완결 결과이다. hit가 있으면
  mutable mount가 비어도 mount 밖 output/receipt를 검증할 수 있다. marker가 있더라도
  실제 파일/mode/bytes hash가 틀리면 실패다. 파일을 `touch`하는 것만으로 source freshness를
  보장하거나 mtime을 Cargo fingerprint 대신 쓰는 설계는 제외한다.

### 4.3 출력과 소비

producer 한 번은 정확히 한 bin을 빌드하고 다음 순서를 지킨다:

```text
검증한 source context + K/P/H + exact compile env
  → guarded locked target cache
  → cargo build --locked --release --package P --bin B (jobs=2)
  → Cargo JSON의 해당 package/bin executable 확인
  → /out/bin/B 로 복사 (target cache mount 밖)
  → /out/artifact.json 및 checksum 작성
  → scratch export stage → private partial directory
  → host 재검증 → 내용 hash 이름 디렉터리로 no-clobber publish → COMPLETE
```

`artifact.json` 필수 키: `format=lagrange-rust-artifact-v1`, `source_commit`, `package`,
`bin`, `platform`, `host_triple`, `profile`, `features`, `compile_env`, `cache_key`,
`input_sha256`, `recipe_sha256`, `binary_sha256`, `binary_mode`, `cargo_success=true`.
`COMPLETE`에는 canonical artifact.json bytes의 SHA-256을 적는다. JSON duplicate key,
알 수 없는 format/누락/다른 commit/비정규 경로는 거부한다. 완성된 디렉터리를 다시 쓰지 않는다.
동일 이름이 이미 존재하면 전체 bytes를 대조하고 일치할 때만 재사용한다.

export는 [Docker local exporter](https://docs.docker.com/build/exporters/local-tar/)로
`--target artifacts --output type=local,dest=<private-partial>,platform-split=false`를 사용한다.
플랫폼은 한 개만 지정한다. 실패한 export의 파일을 완료 경로에 병합하지 않는다.
소비는 digest 이름의 로컬 **directory named context**이다. mutable image tag,
remote registry 또는 `service:` context가 아니다
([build contexts](https://docs.docker.com/build/concepts/context/#named-contexts)).

같은 Dockerfile이 여러 bin을 COPY하는 D2/D6에는 현재 commit의 모든 개별 receipt를 검증한
뒤 한 소비 bundle을 원자 생성한다. 하나라도 빠지면 bundle을 공개하지 않는다.
소비 Dockerfile의 검증 stage는 `LAGRANGE_CODE_COMMIT`, bundle hash, 파일 목록/실행 mode,
각 bin hash를 검증한 뒤 기존 `/build/target/release/<bin>` 경로를 제공한다.
D6의 fixtures/configs도 검증된 source payload에서 원래 `/build/...` 경로로 제공한다.
최종 runtime stage의 COPY/기능/user/path는 보존한다. 최종 tag 존재만으로 성공하지 않는다.

### 4.4 순서와 중단 지점

공식 12개 순서는 `scripts/ops/lib/release-image-manifest.sh:9–22`를 유지하고 묶음을 고정한다:

| 묶음 | 소비 서비스 (각각 별도 Compose 호출) | 필요한 producer |
|---|---|---|
| B1 | db-role-bootstrap, db-migrate, api-server | API bin 하나; DB install은 기존 DB recipe 비용 |
| B2 | web, research-worker, recommendation-runner | collectors 10 bin 순서대로, 그 뒤 recommendation/candidate pair |
| B3 | candidate-runner, owner-beta-runner, owner-equity-v2-runner | D2 bundle 재검증, owner bin 각각 |
| B4 | nt-backtest-worker-1, nt-backtest-worker-2, paper-scheduler | backtest bin 하나, paper bin 하나 |

필요한 산출물은 해당 소비 직전에 준비한다. L1의 target cache는 모두 같은 `K`를 공유하고,
L2는 `K-collectors`, `K-queue`, `K-api`로 분리한다. L2의 queue는 job-queue의 5개 bin,
api는 API/paper, collectors는 D6의 10개 bin이다. runtime 역할만 보고 외부 dependency가
서로 없다고 주장하지 않는다. 공통/그룹 모두 한 producer 호출에 하나의 Cargo bin만 낸다.
D6 10개는 **3+3+2+2 bin**마다 추가 확인점이 있다. 배치 사이뿐 아니라 각 producer 종료
뒤에도 exit/남은 rustc·cargo/메모리·swap/OOM·서비스 상태를 확인하며 다음 실행을 멈출 수 있다.

제품 artifact recipe도 위 identity 준비를 먼저 수행하여 native inventory가 cache key와
일치함을 확인해야 한다. 이 준비는 compile과 별도 종료/자원 확인점이며 그 뒤 첫 bin을 시작한다.

긴 실행은 owner-approved background systemd unit, 낮은 CPU/I/O priority, jobs=2,
COMPOSE_PARALLEL_LIMIT=1을 그대로 적용한다 (`AGENTS.md:3–18`). helper는 service lifecycle
명령이나 health 복구를 수행하지 않는다. OOM 조회 권한/기준 관측이 없으면 gate 실패이며
빈 grep 결과로 대신하지 않는다. WP-1에서 정의한 메모리/swap/health threshold를 낮추지 않는다.
stop/failure 후에는 소비 이미지 및 최종 manifest로 진행하지 않는다. 완료된 bin/bundle만
현재 commit/hash로 재검증하여 재개한다. 모든 최종 소비 서비스는 공식 builder를 다시 순회하고
12개 검사와 V2 발행을 마쳐야 한다. 사전 준비와 재개 비용도 전체 경과에 포함한다.

## 5. WP-3용 고정 소형 prototype 명세

이 절은 L0/L1/L2를 **동일 synthetic workload**에서 구현하기 위한 고정 계약이다.
WP-3는 제품 코드를 수정하거나 그룹/feature/명령을 재설계하지 않는다. G1은 WP-1 계측
인터페이스와 이 절을 대조한 뒤 문서/도구/fixture의 고정 SHA를 전달한다. WP-1의 고정 계측 계약과 fixture identity는 §9에서 연결한다. 실제 실행은 기존 fixture warm/삭제 재시험과 자원 게이트
이후이며 오프라인 구현은 가능하다.

### 5.1 fixture 파일과 의미

소유 root `tests/fixtures/build-layout/`에 다음 파일만 만든다. 기존
`tests/fixtures/build-cache` 원본은 수정하지 않는다.

- `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`: B fixture의 두-member resolver=3 workspace,
  toolchain 및 lock을 복사한다 (`tests/fixtures/build-cache/Cargo.toml:1–3`).
- `fixture-lib/Cargo.toml`, `fixture-lib/src/lib.rs`: B fixture를 복사한다.
  `itoa = "=1.0.18"`를 유지한다 (`.../fixture-lib/Cargo.toml:8`).
  features에 `default=[]`, `wide=[]`를 추가한다. `shared_number()`는 기존 itoa 경로를
  사용하여 base면 42, `cfg(feature="wide")`면 84를 반환한다.
- `fixture-app/Cargo.toml`, `fixture-app/build.rs`, `fixture-app/data/embedded.txt`,
  `fixture-app/src/bin/cache-bin-a.rs`, `fixture-app/src/bin/cache-bin-b.rs`: B fixture 복사.
  app features는 `default=[]`, `wide=["build-cache-fixture-lib/wide"]`만 추가한다.
  build.rs의 commit/setting/env rerun 및 generated.txt 규칙은 그대로 사용한다
  (`.../fixture-app/build.rs:7–24`). 새 dependency/build dependency는 없다.
  두 출력의 구조도 그대로 둔다 (`.../src/bin/cache-bin-a.rs:1–14`, b도 동일).
- `runtime/web.txt`, `runtime/python.txt`: 각각 ASCII `web-v1\n`, `python-v1\n`.
  실제 Web/NT 코드나 provider를 사용하지 않는다.
- `Dockerfile.baseline`, `Dockerfile.artifacts`, `Dockerfile.consumer`, `compose.yml`,
  `layout.json`, `layout-helper.sh`: 아래 명령/순서/검증만 구현한다.
- fixture native deps는 없음. Rust base와 Alpine runtime은 B fixture
  `tests/fixtures/build-cache/Dockerfile:3`, `:34`의 exact pins를 복사한다.
  JSON/hash/guard를 위해 artifact/baseline builder에만 `python3`를 설치할 수 있다.
  세 구조에 동일 설치 단계를 적용하고 cold 준비 시간에 포함한다. runtime은 바꾸지 않는다.

package 이름은 기존 `build-cache-fixture-lib`, `build-cache-fixture-app` 그대로다.
앱에 lib target은 없다. bin a/b와 build script뿐이라는 사실도 보존한다.
동일 source/commit/setting/variant는 세 구조에서 byte output이 정확히 같아야 한다.

| 소비 서비스/단계 | 출력 bin | feature argv | runtime file | 그룹 |
|---|---|---|---|---|
| `probe-a-base` / P1 | cache-bin-a | 없음 (default empty) | web.txt | base |
| `probe-b-base` / P2 | cache-bin-b | 없음 | python.txt | base |
| `probe-a-wide` / P3 | cache-bin-a | `--features wide` | web.txt | wide |
| `probe-b-wide` / P4 | cache-bin-b | `--features wide` | python.txt | wide |

순서는 P1 → 소비 a-base → P2 → 소비 b-base → **gate** →
P3 → 소비 a-wide → P4 → 소비 b-wide → **gate** → 최종 네 이미지 검증이다.
각 소비 이미지는 `/usr/local/bin/<bin>` 한 개와 `/opt/fixture/runtime.txt` 한 개,
UID/GID 10001, 해당 bin entrypoint, exact test commit OCI revision을 갖는다.
bin의 stdout는 기존 fixture와 같은 필드를 가지며 `shared=42` 또는 `shared=84`만 variant에
따라 다르다. runtime.txt는 이미지 검증 시 바이트까지 읽고 대조한다.

### 5.2 세 빌드 DAG (임의 선택 없음)

먼저 세 layout 모두 **P0 준비 단계**를 한 번 실행한다. 해당 Dockerfile의
`builder-env` stage는 pinned Rust base + 동일 python3 설치 + `/build`를 정의한다.
`identity` scratch target으로 `rustc -vV`, `cargo -V`, native tool/package inventory를
JSON으로 local export한다. host는 이 값과 manifest/lock/설정 hash로 K를 계산한다.
P1–P4는 같은 builder-env에서 실제 identity를 다시 비교하며 P0와 달라지면 compile 전에
실패한다. P0 출력은 실행파일이 아니며 완료된 producer 개수에 넣지 않는다. P0의 dependency
설치/다운로드/버전 조회/export 시간도 준비 및 전체 시간에 포함한다. L0에도 동일 비용을
부담시켜 artifact 방식에만 숨겨진 사전 준비를 두지 않는다. P0부터 모든 호출은 순차이다.

공통 Cargo 명령은 `cargo build --locked --release --verbose
--message-format=json-render-diagnostics --package build-cache-fixture-app --bin <bin>`이다.
wide인 P3/P4만 `--features wide`를 덧붙인다. ENV는 jobs=2,
`CACHE_FIXTURE_COMMIT=<40hex>`, `CACHE_FIXTURE_BUILD_SETTING=default`이다.
세 layout/P0/source fallback 모두 `WORKDIR=/build`, `CARGO_TARGET_DIR=/cargo-target`,
cache mount target `/cargo-target`를 고정한다. bin 원본은 `/cargo-target/release/<bin>`이며
`/out/bin/<bin>` 또는 baseline의 `/build/target/release/<bin>`으로 mount 밖에 복사한다.
`--target`을 compile에 추가하거나 target 경로를 variant별로 바꾸지 않는다. L2도 경로는 같고
mount **ID만** 그룹별로 다르다. `CARGO_BUILD_JOBS=2`, `COMPOSE_PARALLEL_LIMIT=1`은 고정이다.

- **L0 baseline:** `Dockerfile.baseline`의 builder는 B fixture의 source COPY와 locked
  registry/git/target mounts를 재현한다. 각 P 호출은 `cargo clean --workspace --release
  --locked` 후 그 P의 **한 bin**을 빌드하고 mount 밖에 복사한다. 그 결과를 같은 Dockerfile의
  runtime으로 COPY한다. 네 소비 service는 `BIN`/`VARIANT` args가 다르므로 네 compile RUN을
  구별한다. target namespace 하나를 네 호출이 공유한다. 별도 producer export는 없으며
  source/compile/image 비용을 같은 timeline에 기록한다. 동일 입력 반복은 Docker hit 허용.
- **L1 common:** P1–P4가 `Dockerfile.artifacts`의 같은 recipe를 각각 호출한다.
  registry/git/target namespace 하나, §4의 package hash guard, 한 bin씩 local export 및
  개별 완료 receipt를 쓴다. 각 소비는 완료된 자신의 P directory만 named context로 받고
  `Dockerfile.consumer`에서 commit/hash 검증 후 이미지 생성한다.
- **L2 grouped:** L1과 똑같은 단계/검사이나 target namespace를 `base`와 `wide` 둘로 분리한다.
  registry/git namespace는 두 그룹이 공유한다. compile 산출물의 다른 namespace 간 복사는 없다.
  P1/P2는 base, P3/P4는 wide로 고정한다. 첫 그룹 완료 후 두 번째 그룹을 시작한다.

cache namespace는 `build-layout-<unique-run-id>-<layout>-<case-scope>-<compat-key>[-group]`이다.
case-scope는 §5.5의 01–03에서만 `warm-chain`, 나머지는 해당 case ID다. setup/trial은 같은
case-scope를 사용하고 group suffix는 L2의 base/wide뿐이다.
세 구조 간 target/layer cache를 공유하지 않는다. 독립 cold/warm-up을 수행하고 같은 layout의
연속 warm 시나리오만 namespace를 유지한다. base/frontend cache 상태와 다운로드를 기록한다.
호스트 전체 cache prune, production tag/cache namespace, registry push는 없다.

`CACHE_FIXTURE_RUN_TOKEN`을 실제 compile RUN에서 소비하여 **강제 RUN + warm target**을
만든다. 이 token은 `K/P/H`에서 제외하고 fixture 출력에도 넣지 않는다. `--no-cache`를 warm
수단으로 쓰지 않는다. 동일 입력 layer-hit 시험과 forced warm 시험을 별개로 기록한다.

2026-09-15 실제 `layout-03`의 L0 cold P2가 `unexpected-recompile-set-itoa`로
실패했다. 같은 cache ID인데 두 호출 모두 `Removed 0 files`와 itoa 재컴파일이
관측됐다. 별도 고정 Alpine cache-file 시험은 동일 namespace에 대해 연속
`--no-cache`, `--no-cache`, token 변경만 사용했을 때 `MISS, MISS, HIT`를
재현했다. 증거는
`/data/worktrees/3puw275b/build-verification-20260915-02bthtb5/reports/no-cache-diagnostic-f90688b3acac/result.json`이다.
따라서 cold/setup/empty-cache의 `--no-cache`는 P0와 첫 compile P1에만
적용한다. P2–P4는 새 run/scope/K의 cache ID, 각 bin/feature 및 소비된 token으로
이전 시험의 layer 재사용을 차단하면서 이번 묶음이 생성한 mount를 보존한다.
그룹별 새 target도 고유 namespace로 비어 있음을 보장한다. warm 단계에는
`--no-cache`를 사용하지 않는다. 예상 Cargo 재사용 집합이나 실패 판정은 바꾸지
않으며 fake 실행기도 이 호스트에서 재현한 mount 초기화를 반영해야 한다.
route-contract의 단독 P1 source/producer 호출은 기존 cold 동작을 유지한다.

fixture 소비 이미지의 Compose 호출에는 `--provenance=false`를 명시한다.
실제 `layout-04`는 L0 cold 전체와 Cargo layer 재사용을 통과했지만, 반복 P1의
동일 config/실행 manifest에 새 attestation이 붙어 상위 image ID만 달라졌다.
[Compose 5.4.0의 contentDigest 설명](https://github.com/docker/compose/blob/v5.4.0/pkg/compose/images.go#L176)과
[Docker attestation 문서](https://docs.docker.com/build/metadata/attestations/)도
이 차이를 설명한다. 작은 실제 Compose 시험에서 YAML `provenance: false`만으로는
attestation이 남았고, 명시적인 CLI 옵션으로 두 빌드의 image ID가 같아졌다.
증거: `/data/worktrees/3puw275b/build-verification-20260915-02bthtb5/reports/provenance-diagnostic-5a18ab6d5ca6/result.json`.
이 조건은 소형 시험의 exporter 메타데이터만 고정하며 실제 image ID, binary,
payload, OCI revision의 동일성 비교를 보존한다. 제품 build/provenance/V2 설정에는
적용하지 않는다. 제품 인수는 관측된 실제 image ID를 계속 검증한다.

### 5.3 예상 재컴파일 집합

단위는 `(package, target kind/name, feature set, target/host, profile)`이다.
`Fresh` 문자열만 세지 않고 Cargo `compiler-artifact` JSON의 `fresh`, `features`, `target`,
`executable` 및 exit status를 함께 읽는다
([Cargo external tools](https://doc.rust-lang.org/cargo/reference/external-tools.html#json-messages)).
특히 build-script-executed event는 실제 script 재실행 횟수와 동일시하지 않고 verbose 실행
증거와 구분한다. 아래의 재컴파일 집합/Fresh 횟수는 **검증할 가설 H**이며 실측값 또는
Cargo가 보장하는 횟수가 아니다. 반면 stdout 전체 바이트, commit/setting/embedded/generated,
base/wide별 lib 결과, itoa의 package/version/source와 unit별 JSON/verbose 증거, binary 경로/hash,
완료 표시와 실패 비가시성은 **필수 정확성 assertion C**이다. H가 달라도 C를 완화할 수 없다.

feature 전환에서 app/bin/build-script가 예상보다 더 재실행되거나 lib/itoa Fresh 횟수가
표와 다르면, 출력이 맞아도 `FAILED_UNRESOLVED: unexpected-recompile-set`으로 전체 실험을
멈추고 코디네이터에게 반환한다. 원시 Cargo JSON/verbose, phase/variant/token/cache ID,
기대 집합과 실제 집합의 차이를 보존한다. expected count 자동 변경, 추가 compile을 cache hit로
정규화, lib/itoa 증거 생략, 가설을 사후에 필수 assertion에서 빼는 처리는 금지한다.
수치가 없는 build-script compile/run 횟수는 관측값으로 기록하되, fresh=false/실제 실행을
구별할 증거가 없으면 `FAILED_UNRESOLVED: missing-cargo-evidence`이다. 코디네이터가 명세를
수정하고 새 도구/fixture SHA를 고정한 **새 run**만 재시험할 수 있다.

| 시나리오 | L0 가설 H / 정확성 C | L1/L2 가설 H / 정확성 C |
|---|---|---|
| cold, P1–P4 | 내부 lib base 2회/wide 2회; app은 각 P target 생성. itoa 동일 unit은 첫 compile 1회, 이후 Fresh라는 가설 | 내부 lib base 1회/wide 1회; app bin 네 variant 생성. L1 itoa 1회, L2는 각 target namespace에서 1회씩 예상. variant별 build script compile/run 증거 별도 |
| exact repeat (source/commit/token 모두 같음) | compile vertex cache hit 또는 실제 Fresh 근거 | 완료 producer/소비 재검증, 재컴파일 0; missing cache만으로 실패하면 안 됨 |
| forced warm, token만 변경 | 네 P에서 workspace clean 때문에 lib/app 재빌드, itoa Fresh | lib/bin 모두 Fresh; exporter/검사 시간은 0으로 숨기지 않음 |
| 새 commit만 (`111…1`→`222…2`) | 각 P clean 및 build | env/build-script 영향으로 app a/b의 base/wide 결과 갱신; lib와 itoa Fresh |
| runtime web/python 파일만 + 새 commit | 위와 같은 Rust invalidation + 해당 runtime 이미지 갱신 | 위 commit 범위만 Rust 갱신; web은 a-base/a-wide, python은 b-base/b-wide에서 새 runtime bytes. 나머지 image도 새 commit label |
| app source a marker `source-v1`→`source-v2`, mtime 과거 | package clean으로 app 출력 반영 | package 수준 guard이므로 **a뿐 아니라 b도** app rebuild를 허용/기록. lib/itoa Fresh. b의 stdout source는 v1 |
| lib base 값 42→43, wide 값 84 유지 | lib/app 갱신 | lib의 P 변화로 lib+app 양 variant clean/rebuild; wide 출력도 현재 코드와 일치, itoa Fresh |
| embedded `embedded-v1`→`embedded-v2` 또는 build.rs `generated-v1`→`generated-v2` | 해당 app 출력 갱신 | app 양 bin/variant rebuild, lib/itoa Fresh |
| build setting `default`→`changed` | app 출력 갱신 | app ENV/build script 갱신, lib/itoa Fresh |
| manifest/lock/config compatibility 변경 | 변경을 반영하거나 typed failure | 새 K; cold와 같은 집합. 제품 dependency 변경 허용을 의미하지 않음 |
| feature lane 변화 | 현재 lane 출력 반영 | base/wide 결과 바뀌어 섞이지 않음. feature가 다른 lib unit은 중복으로 세지 않음 |
| A→B→A source bytes, 모두 오래된 mtime | 현재 source 결과 또는 원인이 확인된 실패 | guard가 changed package/reverse closure를 clean; 최초 A의 bytes 출력 복원 |

prototype lock 변경은 새로운 외부 dependency를 추가하지 않고 복사한 lock에 주석 한 줄을
추가하여 raw compatibility key 변화를 시험한다. dependency 버전 변경의 실제 정확성은
WP-1 기존 fixture의 고정 시나리오를 사용한다. 이 둘을 같은 검증이라고 보고하지 않는다.

### 5.4 실패 주입과 재개 (세 구조 모두)

실패 전 결과를 지우거나 assertion을 완화하지 않는다. 아래 실패 이름/위치를 고정한다.

| case | 주입 | 기대 실패/재개 |
|---|---|---|
| `compile-fail-p2` | P2 시작 전 b source에 `compile_error!("layout-probe-injected");` 삽입 | P2 cargo nonzero, 이후 P/소비 없음. 원본 복원 후 재개; 현재 hash가 맞는 P1만 재사용, dirty target guard 적용 |
| `export-fail-p2` | P2 Cargo 성공/복사 뒤 COMPLETE 전에 fixture helper exit 73 | partial만 남음, b 소비 없음. 재개는 P2 재수행/검증 후 공개; partial을 완료로 승격하지 않음 |
| `wrong-commit` | private export의 artifact.json commit만 `333…3`으로 교체 | publish/consumer 검사 거부. 정상 완결 디렉터리는 변조하지 않음. 태그가 이미 있어도 실패 |
| `missing-bin` | private export에서 해당 bin 하나 제거 | hash/목록 검사 실패, 소비 이미지 미발행 |
| `tampered-bin` | private bin에 byte 한 개 추가하되 원 receipt 유지 | checksum 불일치 실패 |
| `missing-complete` | 공개 후보에서 COMPLETE 생성을 생략 | hash가 맞더라도 미완료로 거부 |
| `stop-after-p2` | 첫 두 소비/gate 후 tool이 exit 75로 종료 | 두 완료 결과 보존. 재개에서 재검증 후 P3부터 진행; 앞단 시간/중단 시간을 누락하지 않음 |
| `delete-input` | build script가 읽는 embedded.txt 삭제, mtime 강제 과거 | 명시적 missing-input 또는 Cargo/build-script 실패. 캐시 binary 실행 성공을 기대값으로 삼지 않음. 복원 후 정확 출력 |
| `delete-source` | b source 삭제 | 명시적 missing declared target/input 실패. 원인 없는 임의 nonzero는 합격 아님 |
| `empty-cache` | 새 owned K/namespace로 cold 재생성 | 모든 결과 재생성 및 출력 일치. global prune 없음 |
| `broken-ledger` | fixture 전용 cache ledger를 알 수 없는 version/pending으로 만듦 | L1/L2 owned target 재생성 후 정확성 검사; L0의 명시적 적용 제외는 §5.5 참조. 예상 외 실패는 unresolved |

L0에는 별도 artifact receipt가 없으므로 `export-fail`, `wrong-commit`, `missing-bin`,
`tampered-bin`, `missing-complete`를 대응되는 **runtime COPY 직전의 baseline staging/검증**에
적용한다. L0의 compile policy를 바꾸지 않으며 시험용 완료 record가 B 제품에 있었다고
주장하지 않는다. 이 공통 검증 계층의 비용은 모든 prototype에 포함한다.

### 5.5 시나리오 그래프와 CLI (G1 보완, 이 절이 모호한 이전 표현보다 우선)

원본 `S`는 §5.1의 고정 fixture snapshot이다. `C1`, `C2`, `C3`는 각각 ASCII `1`, `2`,
`3`을 **40개** 반복한 synthetic commit이다. 초기 setting은 `default`, source marker는
`source-v1`, embedded는 `embedded-v1`, generated prefix는 `generated-v1`, lib는 base=42/
wide=84이다. 아래 표의 수정 이외에는 반드시 S/C1로 되돌린 새 private snapshot을 사용한다.
실제 Git HEAD/tool/fixture SHA는 synthetic commit과 별도 저장한다.

`--layout all`의 순서는 **baseline → common → grouped**, 각 layout 안의 case 순서는 아래
01–28이다. 각 case의 순서도 P0 → P1/소비 → P2/소비 → gate → P3/소비 → P4/소비 → gate →
네 결과 검사로 고정한다. 실패/route case의 명시적 예외만 아래 표를 따른다.

| 순번 / `--case` ID | 입력 변경과 실행/리셋 |
|---|---|
| 01 `cold` | S/C1, 새 빈 owned namespace, token=`cold`. P0–P4 전체. |
| 02 `exact-repeat` | 01과 같은 source/commit/token/namespace 및 완료 결과를 그대로 재검증/재빌드. setup 없음. |
| 03 `forced-warm` | 01/02의 namespace와 S/C1, token만 `forced-warm`으로 바꾸어 P1–P4 RUN 강제. |
| 04 `commit-only` | 독립 warm-up W 뒤 C2만 전달. |
| 05 `web-only` | W 뒤 C2 + web.txt를 `web-v2\n`으로 교체. |
| 06 `python-only` | W 뒤 C2 + python.txt를 `python-v2\n`으로 교체. |
| 07 `app-source-old-mtime` | W 뒤 C1 유지, a의 marker만 source-v2; 모든 source mtime을 2000-01-01T00:00:00Z로 설정. |
| 08 `lib-source` | W 뒤 C1, base 42→43만 변경, wide 84 유지. |
| 09 `embedded` | W 뒤 C1, embedded.txt를 `embedded-v2\n`으로 교체. |
| 10 `build-script` | W 뒤 C1, build.rs의 generated-v1→generated-v2만 변경. |
| 11 `build-setting` | W 뒤 C1, setting=changed. |
| 12 `manifest-key` | W 뒤 C1, fixture-app/Cargo.toml 끝에 `# layout-manifest-v2\n` 추가. 새 K. |
| 13 `lock-key` | W 뒤 C1, Cargo.lock 끝에 `# layout-lock-v2\n` 추가. 새 K. |
| 14 `config-key` | W 뒤 C1, Cargo argv에 `--config build.jobs=2`만 추가. 유효 jobs=2 유지, 명시 config 입력으로 새 K. |
| 15 `feature-switch` | W 뒤 S/C1, P1–P4가 base/base/wide/wide 순으로 전환하는 trial을 2회 연속 실행. 각 trial token은 서로 다름. stdout/commit/lib/itoa와 H를 매번 대조. |
| 16 `branch-return` | W 뒤 C1; S→a marker만 source-v2→S의 두 trial, 각 trial 전 모든 source mtime을 위 과거 시각으로 설정. 양쪽 P1–P4 수행. |
| 17 `compile-fail-p2` | W 뒤 S/C1 P1 성공 후 새 private source의 b에 표의 compile_error 삽입, P2 실패. R로 S 복원. |
| 18 `export-fail-p2` | W 뒤 S/C1, P2의 private staging에서 exit 73. R로 주입 없이 P2 재수행. |
| 19 `wrong-commit` | W 뒤 S/C1, P2 private receipt의 commit을 C3로 변경. 검증 거부 후 R. |
| 20 `missing-bin` | W 뒤 S/C1, P2 private bin 제거, 검증 거부 후 R. |
| 21 `tampered-bin` | W 뒤 S/C1, P2 private bin 끝에 ASCII X 한 byte 추가, 검증 거부 후 R. |
| 22 `missing-complete` | W 뒤 S/C1, P2 COMPLETE 생략, 검증 거부 후 R. |
| 23 `stop-after-p2` | W 뒤 S/C1, P1/P2 및 소비/gate 완료 후 exit 75. R은 검증된 P1/P2를 건너뛰고 P3부터. |
| 24 `delete-input` | W 뒤 C1, trial 시작 전 embedded.txt 삭제, 남은 source mtime은 위 과거 시각. missing-input 거부 후 R로 S 복원. |
| 25 `delete-source` | W 뒤 C1, trial 시작 전 b source 삭제. missing-target/input 거부 후 R로 S 복원. |
| 26 `empty-cache` | 별도 W 없이 S/C1, 01과도 다른 새 빈 namespace로 P0–P4. |
| 27 `broken-ledger` | W 뒤 S/C1, L1/L2 각 target cache의 ledger version을 `unknown-probe-version`으로 바꾸고 pending=true. 첫 P가 해당 owned target을 재생성해야 하며 P1–P4 결과 검증. L0는 ledger가 없으므로 `NOT_APPLICABLE: baseline-no-ledger`를 명시하고 clean 기반 P1–P4 정확성은 검사. |
| 28 `route-contract` | W 대신 §5.6의 3단계만 수행. layout 성능 집합과 별도 표기하되 suite 시간에서 빼지 않음. |

**W (독립 warm-up):** case별 새 namespace에서 S/C1, token=`<case>-setup`으로 P0–P4와
네 소비/출력 검증을 완료한다. trial은 같은 case cache만 쓰고 token=`<case>-trial-1`로
바꾼다. 두 번째 trial은 `...-trial-2`. 다른 case의 warm target을 물려받지 않는다. token이 바뀐 trial은 완료 artifact가 존재해도
producer 호출을 생략하지 않고 실제 RUN/Cargo 증거를 수집한다. 같은 내용의 export는 검증 후
no-clobber 재사용할 수 있지만 기존 receipt/로그를 덮어쓰지는 않는다.
01–03만 하나의 namespace를 공유한다. 단일 `--case exact-repeat` 또는 `forced-warm`도 01을
setup으로 먼저 수행한다. 04–27 단일 선택은 자기 W만 수행한다. setup 비용/검증은 별도
interval로 남기며 전체 경과에도 포함한다. 원본/완료 artifact, 이전 attempt source/log는
수정하지 않는다. 각 case의 namespace와 image tag에는 case/phase를 포함해 기존 tag를 덮어쓰지
않는다. exact-repeat는 동일 tag의 ID 불변을 검증하고, 다른 입력은 새 tag를 사용한다.

**R (주입 실패 후 재개):** 17–25는 지정한 phase와 정확한 오류/exit 원인이 확인되면
`EXPECTED_STOP` 이벤트를 기록하고 top-level exit **75**로 멈춘다. 이는 case PASS가 아니다.
이벤트에 주입을 consumed로 기록한다. 재개는 같은 run에 새 attempt 디렉터리를 만들고, 저장한
실패 source/receipt hash를 먼저 검증한 뒤 **새 source snapshot**에서 선언한 주입만 복원하여
같은 case를 완료하고 다음 case로 간다. 17의 b 수정은 package P를 바꾸므로 P1도 H를 다시
계산해 현재 입력과 맞을 때만 재사용한다. 실패 partial은 절대 공개/재사용하지 않는다.
24/25의 누락은 저장한 선언대로인지 확인하고 복원하며 임의의 외부 수정은 거부한다.
예상하지 않은 오류, gate 실패, H 불일치는 exit **1**, CLI 오류는 **2**, suite 성공은 **0**이다.
exit 1은 자동 재개 불가이며 coordinator 검토 대상이다. OS 중단으로 마지막 이벤트가 완결되지
않으면 실행 중 프로세스가 없고 입력이 동일함을 재검증한 뒤 그 phase를 pending/불신으로
처리하고 재실행한다. 실패 완료를 추정하지 않는다.

```text
새 실행: build-layout-probe.sh [--plan|--apply] [--layout baseline|common|grouped|all]
           [--case <위 ID>|all] --output-dir <새 canonical absolute run-dir>
재개:    build-layout-probe.sh [--plan|--apply] --resume-from <기존 canonical absolute run-dir>
오프라인: build-layout-probe.sh --self-test
```

mode 기본값은 plan, 새 실행의 layout/case 기본값은 all이다. 별도 `--inject`/임의 hook은 없다.
case ID가 주입과 위치를 결정한다. `all`도 위 순서로 실행하다 EXPECTED_STOP마다 75를 반환하며
아래 재개 명령을 반복해 이어간다. 재개에는 `--output-dir`, `--layout`, `--case`를 **함께 줄 수
없다**. 같은 run의 append-only `events.jsonl`과 새 `attempt-NNN/`에만 결과를 추가한다.
초기 `run.json`, 성공 artifacts/COMPLETE, 기존 logs/results를 덮어쓰지 않는다. cursor는 완결된
이벤트에서 복원하며 완성 suite를 resume하면 검사 후 already-complete를 출력하고 build하지 않는다.
초기 run.json에는 schema/tool/fixture/document SHA, origin HEAD, cases/layouts/순서,
source/compile settings, gate 입력 목록/threshold를 고정한다. 재개 전 현재 origin HEAD와 도구/
fixture bytes, 기대 synthetic commit/현재 source inventory, 완료 artifact/binary/OCI commit/hash,
게이트 입력 동일성을 검증한다. 하나라도 다르면 기존 성공을 바꿔 맞추지 않고 실패한다.
새 실행의 plan도 `--output-dir` 경로 인자는 필요하지만 디렉터리/파일은 만들지 않는다.
resume plan은 저장 상태 읽기만 하고 이벤트를 추가하지 않는다.

예: §5.7의 health ENV를 먼저 명시한 환경에서 다음처럼 한 중단 case만 수행/재개한다.

```sh
bash scripts/qa/build-layout-probe.sh --apply --layout common --case stop-after-p2 --output-dir /tmp/layout-stop-proof
# 정확한 EXPECTED_STOP일 때 exit 75. 동일 run에 새 attempt를 추가한다.
bash scripts/qa/build-layout-probe.sh --apply --resume-from /tmp/layout-stop-proof
```

실제 fixture binary는 기존 smoke의 `--network none`, read-only rootfs, cap-drop,
no-new-privileges, nonroot 방식으로 실행한다 (`docs/verification/production-build-cache.md:156–160`).
제품 binary는 실행하지 않는다. 이 명세 보완에서는 위 명령을 실행하지 않았다.

### 5.6 optional source/artifact 경로의 최소 시험

이미 허용한 `Dockerfile.consumer` 안에만 `source-builder`, `verified-artifacts`, 선택 alias
`builder`, final `runtime`을 둔다. global `ARG RUST_ARTIFACT_SOURCE=source-builder`와
`FROM ${RUST_ARTIFACT_SOURCE} AS builder`는 §6 초안과 같아야 한다. source-builder는 baseline의
단일 P1 compile/staging 명령을 복사한다. verified-artifacts는 named context
`release_artifacts`에서 receipt/bin을 읽고 기존 commit/hash 검사를 한다. P1 bin/runtime만
사용하고 추가 fixture service나 tracked 파일은 만들지 않는다.

`route-contract`는 각 layout에서 S/C1의 별도 namespace/tag로 아래 순서를 실행한다.
이것은 동일 routing mechanism 시험이며 L0가 artifact 제품 구조를 채택한다는 뜻이 아니다.

1. `source-fallback`: 기존 `probe-a-base` service의 fixture build override에서
   `dockerfile=Dockerfile.consumer`, `target=runtime`만 지정한다. `RUST_ARTIFACT_SOURCE` 및
   `additional_contexts` 키를 **완전히 생략**한다. build/정확 stdout/hash/OCI revision PASS,
   artifact stage 실행 및 `release_artifacts` remote image 조회 시도는 없어야 한다.
2. `prepare-route-artifact`: `Dockerfile.artifacts`로 P0/P1만 만들어 C1의 완료 artifact를 검증한다.
   이 producer 비용/결과도 route case 기록에 포함한다.
3. `artifact-present`: 동일 service/Dockerfile/target에 source=verified-artifacts,
   기대 bundle hash, `additional_contexts.release_artifacts=<검증한 directory>`를 지정한다.
   source-builder 앞부분에만 둔 fixture `ARG PROBE_SOURCE_TRAP=0` / `RUN test
   "$PROBE_SOURCE_TRAP" = 0`에 이 호출은 값 **1**을 전달한다. 이 trap은 Cargo/feature 설정과
   무관하며 source stage가 선택되면 반드시 실패한다. artifact route는 PASS이고 stdout/commit/
   binary hash는 검증된 P1과 같아야 한다. BuildKit log에 source compile/trap 실행이 없어야 한다.

세 단계는 한 번에 하나의 service/build만 제출한다. trap argv는 route case가 내부적으로만
설정하며 외부 CLI 옵션으로 노출하지 않는다. missing named context 때문에 source fallback이
실패하거나 trap이 실행되면 `FAILED_UNRESOLVED: optional-artifact-route`로 중단한다.
두 경로를 같은 Dockerfile/target에서 확인하지 않은 standalone consumer 성공은 G2 근거가 아니다.
지원 근거는 [BuildKit의 필요한 stage만 실행하는 계약](https://docs.docker.com/build/building/multi-stage/#differences-between-legacy-builder-and-buildkit)이지만,
실제 installed frontend 결과는 이 case로 확인하며 아직 실행한 것으로 간주하지 않는다.

### 5.7 prototype gate 입력/판정 (실행하지 않은 고정 인터페이스)

기존 WP-1 미완성 코드에 의존하거나 이를 source하지 않는다. probe의 `layout-helper.sh` 내부
`probe_gate <case> <phase> <previous-exit>`만 사용하고, 새 실행/P0 전, 각 producer/소비 종료,
2개 소비 batch 경계, 실패 주입 직후, resume 전후, 최종 검사 직전에 호출한다.
일반 previous-exit는 0만 허용한다. 주입의 정확한 예상 exit는 controller가 별도 이벤트로
검증한 뒤 gate에는 0을 전달하되 원 exit를 지우지 않는다. 예상 외 실패는 다음 build로 못 간다.

- 필수 ENV 이름은 **`BUILD_LAYOUT_HEALTH_UNITS`**, **`BUILD_LAYOUT_HEALTH_CONTAINERS`**다.
  둘 다 코디네이터가 제공하는 현재 운영 대상의 comma-separated **실제 이름 목록**이며,
  각 목록은 1–16개 이름이며 빈 값/원소, 중복, whitespace/glob/slash, placeholder는 거부한다. unit은 literal `.service`
  이름, container는 literal Docker name이다. unit 이름을 추정하거나 자동 발견해 선택하지 않는다.
  container 목록에는 `lagrange-station-research-worker-1`을 반드시 포함한다. 그 밖의 운영 health
  대상도 코디네이터의 목록 그대로 검사하며 실패한 대상을 빼거나 stopped로 바꿔 통과하지 않는다.
  실제 control-plane unit 목록은 외부 실행 입력이며 이 문서가 알 수 없는 이름을 만들어 넣지 않는다.
- 임계값은 `/proc/meminfo`의 **MemAvailable ≥ 2097152 KiB**, **SwapFree ≥ 524288 KiB**이다.
  값 누락/비정수/읽기 실패는 거부한다. threshold CLI/ENV override, fixture용 lowered threshold,
  fake 값으로 actual gate를 대체하는 모드는 없다. host `cargo/rustc/rustdoc`가 남아 있으면 거부한다.
- unit마다 `LC_ALL=C timeout 10s systemctl show <unit> --property=LoadState,ActiveState,SubState,ExecMainStatus,MainPID,NRestarts`
  의 정해진 필드만 읽는다. loaded/active/running, exit=0, MainPID>0이어야 한다. 첫 gate와 비교해
  MainPID 변경/NRestarts 증가/누락/경고/timeout/nonzero면 거부한다. 명령·환경·서비스 로그를 읽지 않는다.
- container마다 `timeout 10s docker inspect --type container --format <고정 template> <name>`으로
  `.Id`, `.State.Running`, `.State.Restarting`, `.State.OOMKilled`, `.State.Health.Status`
  (없으면 absent), `.RestartCount`, `.Config.Labels["com.docker.compose.project"]`만 읽는다.
  running=true/restarting=false/oom=false/health=healthy/project=lagrange-station을 요구한다.
  첫 gate와 비교해 Id 변경/RestartCount 증가 또는 필드 누락/nonzero/timeout이면 거부한다.
  Config.Env, health 로그, provider/DB 요청은 사용하지 않는다
  ([Docker inspect](https://docs.docker.com/reference/cli/docker/inspect/)).
- kernel 관측은 두 번의 제한된 읽기로 접근 가능성과 해당 시간 범위를 구분한다.
  먼저 `LC_ALL=C timeout 10s journalctl -k -b --no-pager -o json -n 1`로 현재 boot의
  실제 kernel entry 하나를 확보한다. nonzero/timeout, 비어 있지 않은 stderr, 빈 출력,
  malformed JSON 또는 필수 `__REALTIME_TIMESTAMP`, `__CURSOR`, `_BOOT_ID`,
  `_TRANSPORT=kernel`, 문자열 MESSAGE 누락은 `kernel-journal-unestablished`다.
  `/proc/sys/kernel/random/boot_id`와 entry의 boot ID도 일치해야 한다. 형식을 검증한 뒤
  UUID의 하이픈을 제거하고 소문자 32-hex로 정규화하여 비교한다. 이 읽기가
  실패하면 이후의 빈 결과를 OOM 없음으로 해석하지 않는다.
- 범위 조회는 `LC_ALL=C timeout 10s journalctl -k -b --no-pager -o json
  --since <고정 since> --until <gate UTC> -n 1000`이다. since는 첫 actual gate UTC의
  30분 전으로 run.json에 고정하고 resume에도 유지한다. stdout/stderr/exit를 private
  임시 파일로 구분하며, 공개 증거에는 상태·건수·범위·hash·OOM 판정만 남긴다. kernel
  MESSAGE 원문을 일반 로그에 출력하지 않는다. `--grep`, `--quiet`, `2>/dev/null`,
  `|| true` 또는 masked pipeline은 사용하지 않는다. nonzero/timeout, stderr 경고,
  잘못된 JSON/필수 필드/시간 범위/boot ID, 1000건 도달은 관측 미확립으로 거부한다.
  접근 probe가 유효한 현재 boot entry를 확보했고 범위 조회가 exit 0·stderr 없음으로
  정상 종료한 경우에만 빈 범위를 0건으로 기록할 수 있다. 정상 JSON의 MESSAGE에는
  case-insensitive `out of memory|oom[-_ ]?kill|killed process`를 검색하고 하나라도
  있으면 거부한다. gate 사이/재개 시 boot ID 변경도 거부한다. 실패를 숨기기 위해
  lookback/행 상한을 바꾸지 않는다. 옵션/접근권한 근거는
  [systemd journalctl 공식 매뉴얼 원문](https://github.com/systemd/systemd/blob/main/man/journalctl.xml)이다.
  이 두 단계 규칙은 G1 코디네이터가 조용한 시간 범위와 읽기 실패를 구분하도록 보완한
  계약이며, 현재 호스트의 자원/health 게이트가 해결됐다는 뜻은 아니다.

각 gate는 시간/phase/previous-exit, 실제 thresholds/관측값, unit/container의 위 필드,
journal exit/count/범위/원문 hash, PASS 또는 고정 실패 원인을 별도 새 record에 남긴다.
이 기록은 선택된 대상과 읽은 범위의 증거이며 host 전체 무장애 보장이 아니다.
`--self-test`만 fake command 결과를 주입하여 각 실패 분기를 확인할 수 있고 실제 실행 결과와
분리한다. 실제 prototype은 기존 승인 범위에서 **모든 gate가 통과할 때만** 실행 가능하다.
장시간 production systemd 실행 권한/설정은 별도 기존 게이트를 유지한다. 이 보완에서는
resource/health/journal을 재조회하지 않았고 host 복구를 주장하지 않는다.

`--self-test`/`bash -n`은 fake/오프라인 검사이고 실제 Cargo cache 합격이 아니다.
리포트는 `docs/verification/production-build-layout-probe.md`, 증거는 전용 `/tmp`에 둔다.
metadata에는 source/tool/fixture SHA, Docker/BuildKit/toolchain, cache namespace/초기상태,
순서/exit, 실제 artifact/output hashes, resource gate를 포함한다. WP-1 출력 필드와 정합화는
G1의 일이며 worker가 기존 QA 소유 파일을 수정하여 맞추지 않는다.

## 6. 제품 적용 초안과 정확한 파일/interface 집합

G2의 선택 전에는 다음 **초안**을 구현하지 않는다. 세 소형 결과와 출처 검증을 검토한
코디네이터가 이 절의 선택/파일 부분집합을 동결한다. 허용 집합은 계획 `:120–132`이다.

| 후보 | 기존 파일 | 신규 파일 |
|---|---|---|
| L0 B 유지 | 없음 | 없음 |
| L0 content guard 보완 | D7, `scripts/ops/build-production-images.sh` | `scripts/ops/lib/release-build-layout.sh`, `deploy/build/release-build-layout.json` |
| L1 또는 L2 artifact 경로 | D7, `scripts/ops/build-production-images.sh` | `scripts/ops/lib/release-build-layout.sh`, `deploy/build/Dockerfile.rust-artifacts`, `deploy/build/release-build-layout.json` |

이 초안은 `deploy/compose/compose.yml`과 `scripts/ops/compose-release.sh`를 수정하지 않는다.
D7의 기존 source build를 기본 경로로 남겨 infrastructure/backfill 및 profile 이미지의
직접 build 호환성을 보존한다 (`scripts/ops/compose-release.sh:365–381`). official image
builder만 선택된 artifact 경로를 연결한다. 실행/설치/rollback/manifest library는 변경하지 않는다.
추가 tracked 파일은 요구하지 않는다. G2에서 다른 경로가 필요하면 WP-4 착수 전에 별도 보고한다.

정확한 내부 인터페이스 초안:

- `release-build-layout.json`: `format=lagrange-build-layout-v1`, 선택 layout, builder pin,
  D7별 package/bin 순서·compile ENV 정책·runtime COPY inventory, package별 외부 입력,
  그룹 mapping, 4개 consumer batch/collectors sub-batch, guard version.
- `release-build-layout.sh`는 host에서 Bash로 source할 수 있는 helper와 builder에서 실행할
  `sh ... --guard-build <request-json> <output-dir>` 경로를 제공한다. builder 경로는 POSIX sh와
  표준 Python3만 사용하도록 분리하고 Cargo/JSON parser를 새 crate로 만들지 않는다.
  builder-only `python3` 설치는 artifact recipe에 명시하고 inventory/key/시간에 포함한다.
- host functions: `release_build_layout_plan <commit>`,
  `release_build_layout_prepare <service> <commit> <state-root>`,
  `release_build_layout_verify_bundle <service> <commit> <bundle-dir>`,
  `release_build_layout_write_override <service> <bundle-dir> <new-file>`.
  모든 인자는 canonical absolute path/정확한 commit/고정 service allowlist로 검증한다.
- `build-production-images.sh`의 기존 `--plan|--preflight|--apply`, 파일 옵션과 root/clean HEAD,
  manifest path 선행 거부를 유지한다. plan은 producer/group/기존 12 service/gate를 출력하고
  Docker/state write를 하지 않는다. preflight는 버전/Compose 입력/schema만 확인한다.
  apply는 한 실행 lock 아래 각 service 직전에 prepare/verify/override 후 한 service build,
  batch gate, 마지막 동일 12개 inspect/V2 write를 수행한다. 실패 시 즉시 중단한다.
- D7는 기존 builder를 `source-builder`로 이름만 바꾸고,
  `ARG RUST_ARTIFACT_SOURCE=source-builder` → 선택 alias `builder`를 둔다.
  `verified-artifacts` stage는 `release_artifacts` named context를 검증해 원래
  `/build/target/release/*`와 D6 payload 경로를 만든다. 기존 runtime stage는 이 alias를
  COPY한다. default source 경로는 외부 artifact/context가 없어도 빌드 가능해야 한다.
  BuildKit의 unused stage pruning은 공식 multi-stage 문서에 근거하되 실제 installed frontend
  검증은 §5.6 `route-contract`의 source-fallback/artifact-present 양쪽 실제 PASS를 G2
  prerequisite로 요구한다. 실패하면 standalone 호환성을 깨는 채 구현하지 않는다.
- helper가 `/tmp` 아래 root-owned mode-0700 run directory에 생성하는 **임시 Compose override**는
  해당 service의 `build.args` (`RUST_ARTIFACT_SOURCE=verified-artifacts`,
  `RUST_ARTIFACT_BUNDLE_SHA256=<digest>`)와 `build.additional_contexts.release_artifacts=<dir>`만
  갖는다. `LAGRANGE_CODE_COMMIT`은 기존 공급 경로 그대로다. runtime service/profile/network/
  volume/command 및 최종 image tag는 override하지 않는다. 검증용 선언 이외 다른 service나
  `service:`/URL context를 허용하지 않는다. `docker compose ... -f <override> build --pull=false
  <one-service>`이다.
- state root는 현재 invocation의 manifest parent 아래 `.lagrange-build-state/<commit>/`로
  derive하고 root ownership/symlink/no-clobber를 검사한다. clean source checkout 안에는 두지
  않는다. source snapshot, pending/artifact/receipt, measured timeline을 보존한다. publish
  bundle과 cache ownership을 구분하고 기존 manifest 덮어쓰기는 여전히 거부한다.

L0 보완은 동일 guard를 D7 내부 source RUN에 넣되 별도 export/Compose context를 만들지 않는다.
같은 source hash guard를 세 후보에 적용할 때의 이득과 export 분리 이득을 별도 집계한다.
L1/L2 common recipe 자체는 Cargo dependency/feature/profile을 추가/통합하지 않는다.

### 부작용 없는 제품 binary/출처 검증

현재 production 바이너리에 공통 `--version`/commit-print 모드가 있다고 가정하지 않는다.
`healthcheck`, `readiness`, `--help`, default entrypoint를 검증용으로 실행하지 않는다.
provider/DB/자격 증명 경로로 진입할 위험을 피하는 고정 방법은 **정적 bytes 검사**이다.

1. producer의 Cargo JSON에서 package/bin/executable을 확인하고 복사한 ELF/실행 mode/hash를
   검사한다. compile ENV와 source/input identity를 같은 receipt에 묶는다.
2. D7 verification stage가 기대 commit/bundle/hash를 확인하고 지정 경로로만 COPY한다.
   backtest는 ENV tracking에 의한 현재 commit compile record와 embedded string 존재도 확인한다.
   문자열 검색 하나만으로 함수 동작/출처가 증명되었다고 보고하지 않는다.
3. 최종 12개 image ID/OCI revision 검사는 기존 official builder를 유지한다. WP-5는 helper로
   `docker image save <exact-image-id>`의 bytes를 전용 증거 경로에 저장한 뒤 bin/wrapper/payload
   경로만 오프라인 검사하도록 연결한다. 최종 layer precedence/whiteout/정규경로/regular-file
   규칙을 적용하고 예상치 못한 archive 형식은 fail closed한다. 컨테이너 create/start/run은 없다
   ([docker image save](https://docs.docker.com/reference/cli/docker/image/save/)).
   동일 image ID의 export/검사는 한 번 재사용할 수 있으나 12 service 대응 검사는 전부 남긴다.
4. image-save 검사 범위는 D7 17 binary 경로와 D6 payload, D2/D5 NT 및 Paper wrapper,
   Web standalone/DB sqlx·wrapper의 기존 필수 경로이다. 실행 의미/DB health가 검증됐다고
   주장하지 않는다. fixture에서는 실제 stdout와 feature/commit failure로 메커니즘을 검사하고,
   제품에서는 기존 기능 회귀 검사와 bytes/출처 보존을 결합한다.

이 검사의 archive parsing/비용은 G2에서 WP-5에 동결하여 전달할 항목이다. 검증 우회 모드는
없다. 현재 builder의 metadata-only 검사와 새 bytes 검사를 혼동하지 않는다.
도표는 이번 문서 조사에서 바꾸지 않았다. 구현 시 기존 runtime 근거
`docs/diagrams/runtime_deployment.puml:148`의 Paper Dockerfile `:26` 인용이 이동하면 WP-5가
실제 코드 행으로 갱신한다. component graph는 `docs/diagrams/component_architecture.puml:128–190`
및 `:207–208`의 코드 계약을 보존한다. 구조가 바뀌는 경우에만 해당 PUML/PNG를 함께 갱신한다.

## 7. 계측과 결정 게이트

`T_total = 최종 strict V2 publish/검증 종료 - 최초 공통 준비 시작`이다.
준비/실행/검증은 서로 겹치지 않는 phase interval로 기록한다. source/hash/dependency 준비,
producer, cache guard/clean, Rust 및 DB sqlx install, Web/uv sync, export/COPY/image 생성,
이미지 inspect/bytes 검사/V2 발행을 포함한다. 공유 interval을 소비 서비스마다 중복 합산하지
않는다. 소비 서비스 elapsed와 producer elapsed를 중첩 집계했다면 표에서 합산 불가로 표시한다.
중단/재개는 active elapsed와 중단 포함 wall elapsed를 모두 남긴다.

compile/link 분리가 로그에서 불가능하면 `compile+link`로 보고한다. Cargo JSON fresh=false는
unit 재컴파일 근거지만 소요 시간 그 자체가 아니다. 중복은 같은 package/version/source,
feature set/target kind/target triple/profile/compiler 설정 unit의 반복만 센다. 정당한 feature
variant, commit 내장 갱신, 빌드 스크립트 재실행, 최종 링크를 별도 표기한다.
메모리/swap 최저치, OOM/health 관측 신뢰성, cache/완료 artifact/최종 image disk 증가도 포함한다.

G1: WP-1 수정 도구/fixture와 본 명세 고정, warm/삭제 미검증 표시 유지.
G2: 소형 L0/L1/L2의 동일 출력·출처·삭제/old-mtime·실패/재개·순차 정책 확인 후 코디네이터 선택.
부적합/이득 없음/실측 없음이면 선택을 강행하지 않는다. L0 유지가 합리적이어도 추가 단축
목표가 달성됐다는 뜻은 아니다.
G3/WP-6: 깨끗한 C에서 실제 12개/strict V2, A/B/C 동일 환경 측정.
대표 warm 변경 한 가지는 G2에서 WP-1 시나리오 ID로 고정하고 B/C 독립 warm-up 후 최소 3쌍,
순서를 교차한다. A/cold/기타 변경은 우선 같은 조건 1회로 시작한다
(계획 `docs/superpowers/plans/2026-09-14-production-build-architecture.md:160–161`).
전체 시간 중앙값/범위와 중복 unit 감소가 변동과 구분되어야 한다.
소형 fixture 시간이나 과거 Rust 일부 98분을 12개 이미지 성능 보장으로 환산하지 않는다.

## 8. 미확인 항목과 인계

- 최종 구조 선택 및 C 커밋: **없음/코디네이터 결정 필요**.
- WP-1 계측/fixture identity와 출력 계약은 §9에서 동결했다. 제품 B와 새 도구 SHA를
  구분한다. 현재 QA의 자체 검사 통과를 실제 B 캐시 인수로 간주하지 않는다.
- 실제 warm/삭제 smoke, installed builder의 feature-unit 재사용, selective clean/ENV tracking,
  local exporter/named context/unused stage 경로: **실제 미검증**. 공식 지원과 실행 성공은 다르다.
- pinned builder 내부 native toolchain/apk inventory, API missing migrations rerun의 실제 비용,
  target cache variant 전환, 링크 시간, 디스크 증가, cold 및 12개 전체 시간: **미측정**.
- §1의 마지막 관측에서는 research-worker health와 memory/swap gate가 실패했고 OOM 기준
  관측도 미확립이었다. G1 보완에서는 재조회하지 않았다. WP-2 범위에서 복구하거나 gate를
  낮추지 않았다. §5.7의 실제 health unit/container 목록은 coordinator 실행 입력으로 남는다.
- 제품의 모든 binary에 compile commit 출력이 있다는 주장은 근거가 없다. 확인된 backtest
  compile attestation과 나머지 OCI/ENV 계약을 명시적으로 구분했다. 모든 bin에 새 compile
  attestation 기능을 요구한다면 앱 기능 수정 금지와 충돌하므로 그 분기를 코디네이터에게 반환한다.
- 메타데이터 조사에서 Python 3.12 `tomllib`은 job manifest의 multiline inline table을
  파싱하지 못했다. 파일을 고치지 않고 Cargo 1.97.1 metadata/tree로 조사하여 성공했다.
  처음 요청한 fixture-app `src/lib.rs`는 B에 없다. bin-only 구조로 명세를 수정했다.
- 조사/문서화 외 제품 변경, prototype 구현, 테스트 compile, stage/commit: **없음**.

## 9. G1 코디네이터 인수 기록 (2026-09-14)

WP-1 및 WP-2의 Paseo `idle` 완료 보고와 실제 변경 범위를 확인했다. 제품 파일 변경은
없다. WP-1은 `scripts/qa/build-cache-benchmark.sh` 및 기준 검증 문서만 변경했고,
WP-2는 본 문서만 작성했다. 코디네이터가 §5.7의 정상적인 빈 journal 시간 범위와
접근 불가를 구분하는 계약을 보완했고, WP-1 도구가 같은 두 단계 판정을 구현했다.

WP-3의 입력은 이 문서와 `docs/verification/production-build-baseline.md`를 포함한
G1 커밋으로 고정한다. 실행자가 그 exact commit을 프롬프트에 전달한다. 제품 기준 B는
여전히 `f4eb4f83abb7c3f43ede072d0077a1a74edd0ee3`이며 G1 커밋은 제품 선택안 C가 아니다.
고정 입력 identity:

- benchmark SHA-256: `f1314dc4f41200295822e67b35f896e5a22c33124095e3de19db2c0118e79749`.
- smoke SHA-256: `d58156c9c328aa6504399794082d086ac1f0b65993b3117f3f6cadb0fa35feea`.
- B의 `tests/fixtures/build-cache` Git tree: `fbe9999abfc3873e1e07ca0d733b00c666cb2a9d`.
  원본 fixture 및 smoke는 이번 G1에서 변경하지 않았다.
- 계측은 V4의 source/tool/probe/instrumentation identity, image build/Cargo/verification
  시간, 순차 phase, sampled capacity, journal status/count/hash/range를 따른다. WP-3는
  독립 tool이며 기존 benchmark private function을 source하지 않는다. producer/unit/receipt
  상세 필드를 추가하되 공유 준비 시간과 실패를 숨기거나 중복 합산하지 않는다.

WP-1의 QA syntax 및 smoke/benchmark 자체 검사 로그와 SHA256SUMS는
`/tmp/lagrange-wp1-g1-self-test.xW1GjVkjcG`에 있다. 코디네이터가 현재 script SHA와
모든 로그 hash를 대조했다. 네 기존 검사도 별도 실행하여 모두 exit 0을 확인했다:
`build-production-images-static-check.sh`, `build-production-images-self-test.sh`,
`production-ops-static-check.sh`, `production-ops-self-test.sh`.
로그는 `/tmp/lagrange-layout-g1-root.74C2U4i4et`에 있다. `git diff --check`도 통과했다.
이들은 정적/Fake 검사이며 실제 Docker/Rust 실행이 아니다.

**G1은 오프라인 WP-3 구현 착수를 허용한다. 실제 시험 슬롯은 아직 인계하지 않는다.**
기존 corrected warm/삭제 fixture의 실제 재시험, 현재 자원·health·journal 게이트는
미충족/미검증이다. WP-3는 fixture/probe/자체 검사를 작성하고 실제 결과를 NOT_RUN으로
구분한다. §5.7의 health unit 목록은 실제 실행 전 외부 입력이며 임의로 만들지 않는다.
실제 구조 정확성을 확인하지 못하면 G2를 통과시키지 않으며 WP-4/5 제품 구현도 시작하지
않는다. 성능 개선이나 12개 이미지/V2 인수를 완료했다는 결론은 없다.
