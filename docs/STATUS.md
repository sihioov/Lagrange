# Lagrange Station — 상태 종합

**기준일: 2026년 8월 11일 (2026-08-11)** · 이 문서는 특정 시점의 스냅샷이다. 아래 수치와 판정은 각 표에 적힌 실행일의 코드에 대한 것이며, 코드가 바뀌면 게이트를 다시 돌려 갱신해야 한다 — 판정 파일은 자동으로 낡는다는 것이 이 프로젝트가 이미 한 번 배운 교훈이다. Paper 구현의 기준 커밋은 `cf8704a`와 `8da6548`이고, 연구 메타데이터 발행 구현은 `bf041f5`부터 `bb81837`까지다.

---

## 1. 목표 — 이 시스템은 무엇인가

> **Lagrange Station은 시장 데이터를 수집·검증하고, 규칙 기반 후보 종목과 목표 비중을 산출하며, 이를 재현 가능한 백테스트·가상투자·제한된 실거래로 연결하는 초대 기반 개인·소규모 퀀트 플랫폼이다.** — 요구사항서 §1.2

소유자와 지인 4~5명이 쓰는 시스템이다. "엔진이 알아서 종목을 골라주는 서비스"가 아니라, **명시된 규칙**(전략)이 팩터를 계산하고 순위를 매기며, NautilusTrader가 그 규칙을 과거 데이터(백테스트) → 가상 시장(Paper) → 실제 시장(Live)에서 동일하게 실행한다.

전 구간을 관통하는 설계 원칙 세 가지가 모든 게이트와 원장의 존재 이유다:

1. **미래정보 참조 금지** — 백테스트가 그 시점에 알 수 없던 정보를 쓰면 허위 성과가 나온다 (§14 위험표 1순위)
2. **재현성** — 같은 입력(데이터 버전·시드·엔진 버전·비용 프로파일)이면 같은 결과
3. **Fail-closed** — 확인할 수 없으면 거부한다. 판단 불가는 허가가 아니다 (§16)

### 단계 로드맵과 종료 기준 (요구사항서 §13)

| 단계 | 내용 | 종료 기준 |
|---|---|---|
| **Phase 0** 기술 검증 | NT 통합, 골든 테스트 | 미래정보 참조 없이 기준 전략이 반복 재현된다 |
| **Phase 1** MVP | 초대 인증, 데이터 파이프라인, 팩터·Selector, 백테스트 | 사용자 5명이 추천 조회와 백테스트를 안정적으로 사용한다 |
| **Phase 2** 가상투자 | Paper 계좌, 마감 후 추천, 다음 거래일 가상 체결 | 최소 한 전략이 백테스트와 Paper에서 동일한 신호 규칙으로 운영된다 |
| **Phase 3** 소유자 실거래 | KIS 어댑터, Risk Gateway, Kill Switch | 장애·타임아웃·재시작 테스트에서 중복 주문과 장부 불일치가 방지된다 |
| **Phase 4** 확장 | 개별주식 동적 유니버스, PIT 재무 팩터, 분봉·틱, LEAN 평가 | (후속 범위) |

---

## 2. 어디까지 왔나

**한 줄 요약: 기능·컴파일·rustfmt 검사는 통과한다. 릴리스는 외부 조달과 실제 KRX provider에 막혀 있고, 그 차단은 fail-closed 설계의 정상 상태다.**

### 2.1 게이트 판정 (2026-08-10 재실행, `--include-failure --include-restore` 포함)

| 게이트 | 코드 검사 | 판정 | 막고 있는 것 |
|---|---|---|---|
| Phase 1 | E3~E6 **전부 PASS** | `BLOCKED_EXTERNAL_DATA_RIGHTS` | E1 KRX 서면 데이터 권리, E2 Auth0 테넌트 (E7 Playwright는 이번 실행에서 스킵) |
| Phase 2 | P1~P5 9개 **전부 PASS** | `OWNER_ONLY_BLOCKED_EXTERNAL` | E1/E2 뿐 — **P6/P7 증거는 복원됨** (아래) |
| Phase 3 | L1~L11 15개 **전부 PASS** | `BLOCKED_EXTERNAL_CREDENTIALS` | X1/X2 실제 KIS 계좌 |
| 장애 주입 (failures) | 15개 시나리오(Phase 2: 11, Phase 3: 4 대표) | **`PASS`** | — (복원 완료) |
| PITR 복구 (restore) | 실제 백업 생성 → 격리 타깃 복구 → 검증 | **`PASS`** (`verdict: SUCCESS`) | — (복원 완료) |
| **종합 (F3)** | — | **`BLOCKED_EXTERNAL`** | E1/E2/X1/X2 (외부 조달만 남음) |

**이 08-10 게이트 범위에서 코드 때문에 막힌 항목은 하나도 없다.** P6/P7은 08-08에 이미 통과했던 것이 08-09 증거 갱신 때 누락 옵션으로 덮어써졌던 것뿐이며, 08-10에 **진짜 백업 생성 → 진짜 격리 복구 → 진짜 장애 주입 15개 시나리오**로 재검증해 복원했다(조작 없음, 외부 판정 불변). 08-11에 추가된 연구 발행 경로의 완료 범위와 실제 KRX provider 잔여 작업은 §2.4·§3.7에 별도로 기록한다.

### 2.2 테스트 (기준일 최종 실행)

| 스위트 | 결과 |
|---|---|
| Rust 워크스페이스 (08-10 기록) | **1,051개 통과** (Paper runner/valuation 이음매 테스트 11개 추가 포함) |
| Rust 워크스페이스 (08-11 재실행) | **1,192개 통과, 4개 의도적 ignore, 실패 binary 0개** — `--no-fail-fast`, QA PostgreSQL 포함 |
| Python (nt + 골든, 08-09 기준 실행) | **239개 통과**, 1 스킵 — 가격 보정 전 기준선의 역사적 실행 기록이며, 현재 v2 증거의 최신 전체 수치를 뜻하지 않음 |
| Web (vitest + tsc) | **48개 통과**, `openapi:check` 클린, `tsc --noEmit` 클린 |
| clippy (workspace, all-targets, all-features) | `-D warnings` 클린 (08-11 재실행) |
| rustfmt (workspace) | **PASS** — 08-11 GitHub Actions 도입 시 기존 drift를 pinned rustfmt로 기계 정규화 |

### 2.3 최종 판정 아티팩트 (`.omo/evidence/`)

| 아티팩트 | 판정 | 발행일 | 신선도 |
|---|---|---|---|
| F1 계획 준수 | `REQUEST_CHANGES_RESOLVED` (42/42 완료) | 08-08 | ⚠️ 이후 커밋 ~40개 — **사람의 재검토가 필요한 문서** |
| F2 코드 품질 | `APPROVE` | 08-08 | ⚠️ 동일 |
| F3 운영 E2E | `BLOCKED_EXTERNAL` | 08-10 | P6/P7 포함 재검증, 최신 |
| F4 범위 충실도 | `APPROVE` (LEAN·미국·분봉·파생 부재 확인) | 08-08 | ⚠️ F1과 동일 |

F1/F2/F4는 스크립트가 아니라 **사람이 코드를 읽고 내린 판단문**이므로, 갱신하려면 재검토를 해야 한다. 이번 세션의 Paper 실행 경로 구현이 Phase 2 관련 재검토의 실질적 근거가 됐다.

### 2.4 단계별 실질 상태

- **Phase 0** — 완료. 가격 스케일을 바로잡은 v2 증거 기준선을 재승인했다. Phase 0의 6개와 robustness 5전략의 30개 비-provenance 경제 아티팩트는 이전 승인본과 byte-identical하여 전략 경제 결과가 유지됐고, provenance와 identity는 v2 계약으로 갱신됐다.
- **Phase 1** — Raw→PostgreSQL 연구 메타데이터 발행 경로 완료. 각 수집은 먼저 불변 Raw batch와 append-only manifest로 내구화되고, 검증한 **같은 batch**의 `data_batches` 4행과 캘린더를 한 트랜잭션으로 발행한다. `trading_calendar_versions`는 정정 이력을 append-only로 보존하고 `trading_calendars`는 더 최신 `retrieved_at`만 현재 projection으로 전진시킨다. 복구는 exclusive commit lock 아래 orphan evidence를 재동기화하고 실제 JSONL line으로 먼저 내구화한 뒤 append-order immutable high-water snapshot을 16건씩 재생하며, timeout 뒤 마지막 검증 cursor에서 재개한다. Linux에서는 UID 10001이 `0440` evidence/`batch.json`을 read-only handle로 `fsync`하고, `0640` manifest/lock만 변경한다. 따라서 orphan 뒤에 대기하던 정상 append도 실제 line 순서의 다음 suffix가 되고, snapshot 종료 후 high-water 불변을 재확인하므로 backdated concurrent append도 누락되지 않는다. daemon은 catch-up/매 scheduled cycle 직전에 다시 복구한다. `research-worker`는 16:30 KST 기본 일정과 지각 시작 즉시 catch-up, one-shot/daemon, 구조화 이벤트, batch-date-aware 4일 신선도 healthcheck, synthetic-production 선차단을 제공한다. Compose는 host `<data>/raw`↔container `/data/raw` 직접 Raw 경로, secret 파일, `cap_drop: ALL` 뒤 `CHOWN`/`FOWNER`/`DAC_OVERRIDE`만 복원하는 no-follow recursive UID 10001 Raw 초기화, exact constraint/column/index와 append-only function body까지 검사하는 migration/schema/role drift gate, 최소권한 `research_writer`를 연결했다. Risk Gateway와 worker health는 미래 batch를 제외하고 `min(retrieved_at, KST batch-date 종점)`을 동일하게 사용하므로 새로 발행한 역사 backfill도 stale이다. 단, 이는 synthetic fixture 기반 개발/QA 경로의 완료다. **라이선스·credential·entitlement를 실제 HTTP 요청에 적용하는 KRX provider는 아직 구현되지 않았고 실제 feed는 live가 아니다.** E1의 서면 권리와 E2뿐 아니라 실제 endpoint/credential 및 운영자 provisioning이 남아 있다.
- **Phase 2** — **실행 엔진·러너·종가 평가 경로 구현 완료** (`ecef4b2`, `cf8704a`, `8da6548`). `job_queue::paper_execution::execute_session`이 큐잉된 target을 실제로 체결해 `orders`/`fills`/`positions`/`cash_ledger`에 기록하고, `api_server::paper_session::run_and_settle`이 정산·패리티·통지를 수행한다. 새 `api_server::paper_runner::run_cycle`은 worker 역할의 전체 due target을 소유자 Actor로 재진입시켜 실행하고, 활성 PAPER 계좌를 스캔해 `job_queue::paper_valuation::value_account`를 호출한다. 종가 평가는 원장 현금 자기대조·보유 포지션별 curated close·미래 close 차단·cost profile 검증을 거쳐 `daily_equity`를 계좌/날짜별 불변·멱등으로 기록한다. `crates/api-server/src/bin/paper-runner.rs`가 `--once`/`--date`, 환경별 풀, 2초 polling/10초 backoff, Ctrl-C 종료를 제공한다. 실제 QA DB 이음매 테스트로 두 소유자 스캔, 실행·통지 중복 방지, 정확한 equity/cash/positions_value, missing/future/conflicting close, LIVE·교차 테넌트 거부를 검증했고, Python/Web/외부 데이터 권리 차단은 그대로다. 호스트 배포 단위와 운영 credential 주입은 `deploy/systemd/paper-runner.service` 및 `paper-runner.env.example`로 등록했고, `scripts/qa/paper-runner-smoke.ps1`가 해당 유닛 정적 계약·QA DB 테스트·CLI smoke를 묶는다.
- **Phase 3** — 안전 불변식 검증 완료(L1~L11) + 이번 감사로 치명 결함 수정. 게이트 입력 5개 모두 코드 수준에서 실제 원천에 연결됐다(`85f1902`: `strategy_promotion`/`instrument_allowed`, `d7d75c7`: KRX 세션·EOD batch freshness·actor-scoped intent conflict). 원천 행이 없거나 읽을 수 없으면 여전히 `Unknown`으로 닫히며, 운영 캘린더·데이터 수집 메타데이터가 준비되기 전까지 **라이브 주문은 승인되지 않는다** — 의도된 fail-closed 상태.
- **Phase 4** — PIT 재무 팩터의 **골격 완료** (`cda7182`): 이중 시간축(기간 + 공시일), 바 날짜별 as-of 해석, 정정 공시 처리. 실제 재무 데이터가 오면 채우기만 하면 된다. 나머지 항목은 미착수 (§4.4).

---

## 3. 최근에 고쳐진 것 (2026-08-08 ~ 08-11)

### 3.1 관통하는 패턴 — 결함은 이음매에 산다

이 기간에 찾은 결함 ~10건은 전부 같은 형태였다: **컴포넌트는 정확하고, 그 테스트는 통과하는데, 다음 컴포넌트로 잇는 코드가 없거나 틀렸다.** 신호도 항상 같았다 — **연결 함수를 부르는 테스트가 없다.** (예: 라우트의 실제 진입점 `submit_through_connection`은 테스트 0개, 그 아래 계층은 22개.)

### 3.2 백테스트 경로

| 결함 | 수정 |
|---|---|
| 큐에 넣은 백테스트를 **실행하는 프로세스 자체가 없음** | `943b1b3`, `d000485` — 러너 데몬 |
| 전략 어댑터가 매매하지 않음 | `ff33b5a` |
| 수수료가 한 번도 부과되지 않음 (+ 이를 잡아야 할 원장 검사가 날짜 비교 버그로 영원히 False) | `21a8fd3`, `f308226` — 버전 비용 프로파일 부과, 손계산 수수료(14,914원)를 Rust·Python 양쪽에 고정 |
| **요청한 기간이 통째로 무시됨** — 전 구간이 돌고 요청 기간인 척 보고 | `592f9fb` + `8c411d9` — 2단계였다: 행 필터만으론 부족했고(카탈로그가 세션 이벤트를 전 구간 리플레이), `BacktestRunConfig start/end`로 리플레이 자체를 제한. 물리적으로 자른 데이터셋과의 일치로 검증 |
| `cost_profile_id` 무검증 — 저장소 전체가 어디에도 해석되지 않는 철자를 사용 | `67982ce` — 해석기 단일화, 제출 시 400 |

### 3.3 검사 도구 자체의 결함

| 결함 | 수정 |
|---|---|
| phase3 게이트가 DB 없이 **DENIED**를 발행 (환경 문제를 코드 결함으로 보고) | `57997cb` — 판정 없이 exit 2로 거부 |
| `phase1-gate.sh`(WSL 전용)를 Windows에서 돌리면 조용히 오답 | 〃 — 거부 가드; Windows는 `.ps1` 쌍둥이 |
| 종합 게이트가 하위의 exit 코드를 삼켜 낡은 증거를 신선한 것처럼 읽음 | 〃 — exit 2 전파 |

### 3.4 라이브·페이퍼 감사 (발견 5건, 전부 처리)

| # | 심각도 | 결함 | 수정 |
|---|---|---|---|
| L-1 | **치명** | **리스크 게이트가 실제 주문이 아니라 테스트 픽스처(`snapshot_all_green`)를 심사.** 12개 검사 중 9개 무력화. 한도값도 픽스처 | `cab5c3a` + 회귀 잠금 `3f67e18` — 요청·저장소에서 실제 스냅샷 구성, `risk_limits` 테이블 최초 배선 |
| L-2 | 높음 | `"SELL"`이 아닌 모든 값(오타·공백·빈 문자열)이 조용히 **매수**가 됨 | 〃 — 라우트 400, 타입으로 운반 |
| L-3 | 중간 | 소수 수량을 문서화된 계약과 반대로 조용히 버림 | 〃 — 거부 |
| P-1 | 중간 | 게이트의 계좌 현금이 권위(`cash_ledger`)가 아닌, 아무도 쓰지 않는 저장 컬럼에서 옴 | `c1642a6` — 원장 파생 + **원장 자기 대조**(`balance` vs `SUM(amount)`, 불일치 시 거부) |
| P-2 | 높음 | **실행되지 않은 Paper 세션이 "완료"로 통지됨** — `Executed`는 호출자의 주장일 뿐 검증 없음 | `9e45091` — 원장에 주문이 없으면 `Failed`로 강등 + CRITICAL 통지 |
| 부수 | — | 일일 주문금액 집계가 존재하지 않는 상태명(`'CLAIMED'`)을 제외 → 게이트 미통과 인텐트가 한도를 잠식 | `8242028` |

수정 과정에서 수정 코드 자체의 버그 2건(FORCE RLS 하에서 bare pool 읽기, 존재하지 않는 상태명)을 새로 쓴 이음매 테스트가 잡았다 — **이음매 회귀 테스트를 먼저 쓰는 것이 검증 수단으로 실증됐다.**

### 3.5 Paper 실행 경로 + 게이트 배선 + 증거 복원 (2026-08-10)

| 항목 | 내용 | 커밋 |
|---|---|---|
| **Paper 실행·러너·종가 평가** | `execute_session`/`run_and_settle` 경로에 worker-wide `run_cycle`과 `paper-runner` 바이너리를 연결했다. `value_account`가 `close_valuation_event`와 원장 자기대조를 사용해 불변·멱등 `daily_equity`를 기록한다. 11개 새 이음매 테스트가 정확한 금액, 미래·누락 close, 충돌, LIVE·교차 테넌트 거부, 대상/통지 중복 방지를 검증 | `cf8704a`, `8da6548` |
| **게이트 입력 5개 배선** | `strategy_promotion`/`instrument_allowed`에 이어 `market_session` ← `trading_calendars`, `data_freshness` ← 최신 KRX EOD `data_batches.retrieved_at`, `IntentConflict` ← actor-scoped 미종결 `order_intents`. 누락·오류는 `Unknown`으로 유지 | `85f1902`, `d7d75c7` |
| **P6/P7 증거 복원** | 실제 백업 생성(`scripts/backup/create.sh`) → 격리 타깃 복구(`scripts/backup/restore-and-verify.sh`, `verdict: SUCCESS`) → 장애 주입 15개 시나리오 재검증. 조작 없이 실제 인프라로 재현 | (증거 재실행, 코드 변경 없음) |
| **Paper 표시 현금 대조** | `PaperRepo::equity`가 `cash_ledger`와 as-of 대조(`LATERAL JOIN`)해 `cash_reconciled` 플래그를 반환. 불일치해도 화면은 보여주되(FR-PAPER-003) 사실대로 표시 | `e99385c` |

**US-006 해결:** Phase 0 v1은 논리 Decimal을 미리 스케일링해 10,150 KRW를 `101,500,000.0000`으로 읽었다. 승인된 v2 기준선은 `10150.0000` 논리 Decimal을 저장하고 catalog/simulation 경계에서만 raw scale-4로 변환하며, 불변 `version=2` 파티션을 사용하고 Phase 0 및 robustness provenance를 재생성했다. v1은 역사 기록으로만 남으며 활성 데이터셋으로 사용해서는 안 된다.

### 3.6 빌드 인프라

- 타깃 디렉터리 95GB → **13GB** (`9e79028`): MSVC는 디버그 정보가 켜져 있으면 PDB를 통째로 쓰므로, 의존성만 `debug = 0`. polars 링크 바이너리 하나의 심볼이 790MB → 173MB.
- 단, **카고는 옛 산출물을 지우지 않는다** — 긴 세션 뒤 `cargo clean` 필요.

### 3.7 연구 메타데이터 발행 경로 (2026-08-11)

| 항목 | 내용 | 커밋 |
|---|---|---|
| **Raw→PostgreSQL 원자 발행** | 검증된 동일 source batch의 4개 파일 lineage를 `data_batches`에 기록하고, 캘린더 이력과 현재 projection을 같은 트랜잭션으로 발행한다. 충돌·부분 상태는 영구 오류로 닫힌다 | `bf041f5`~`96f4212` |
| **정정·복구 계약** | 캘린더 version 이력은 append-only이고 최신 retrieval만 projection을 전진시킨다. Recovery는 exclusive commit lock 아래 orphan을 evidence 재동기화 후 실제 JSONL line으로 append·sync하고, 그 durable append order의 high-water snapshot과 immutable batch-ID cursor를 16건씩 재생한다. timeout/failure 뒤 마지막 검증 event 다음부터 재개하며 종료 후 suffix/high-water 불변을 확인해 orphan/정상/backdated append 누락을 막고 complete batch는 exact replay로 검증한다 | `d5fcc38`~현재 |
| **worker·관측성** | `--once --date`, 16:30 KST daemon과 at/after-schedule 즉시 catch-up, startup 및 매 daemon cycle 직전 paged recovery, 10초~600초 retry, stable JSON events/errors, batch-date-aware 345600초 healthcheck, synthetic-production 선차단을 구현했다 | `125eac1`~현재 |
| **Compose·Risk 이음매** | direct host `<data>/raw` 계약, `research_writer` 최소권한, secret 파일, no-follow recursive Raw UID init, exact constraint 정의/column/index/normalized append-only function body까지 닫는 non-root schema gate와 mutation smoke를 연결했다. Risk Gateway와 health는 적용 가능한 최신 `EOD` 및 역사 backfill을 stale로 유지하는 동일 effective instant를 사용하며 `EOD_UNAVAILABLE`은 제외한다 | `30e2679`~현재 |

이 완료 판정은 저장·발행·복구·배포 **이음매**에 대한 것이다. 실제 라이선스 KRX HTTP transport, production credential/endpoint, entitlement-aware provider 동작, 외부 role/secret/data-volume provisioning은 구현·조달되지 않았다. 따라서 실제 KRX feed가 운영 중이라는 뜻이 아니다.

### 3.8 GitHub Actions CI (2026-08-11)

- Pull request와 `main` push는 policy, rustfmt, workspace 전체 strict Clippy, deterministic Phase 0 생성, disposable PostgreSQL, Rust workspace 전체 테스트를 각각 독립 GitHub-hosted runner에서 수행한다.
- `main` push는 별도 runner에서 기존 research-worker Docker/Compose functional smoke를 한 번 더 수행한다. 수동 `workflow_dispatch`는 지원하지만 **`schedule`/nightly 트리거는 없다.**
- 260-session Phase 0 데이터는 tracked generator에서 780개 bar로 runner 내부에 생성되고 테스트 종료와 함께 폐기된다. `data/phase0`, Rust `target/`, 테스트 결과는 artifact나 cache로 업로드하지 않는다.
- 로컬 사전 검증에서 생성 데이터로 기존 clean-checkout `job-queue --test backtest_runner` 누락을 복구했고 12/12 통과했다. QA PostgreSQL을 포함한 `cargo test --workspace --locked --no-fail-fast`는 310.4초에 실패 binary 없이 종료했고, workspace all-target/all-feature Clippy `-D warnings`도 통과했다. GitHub-hosted Linux의 실제 디스크·시간 증거는 첫 push 실행에서 확정한다.

### 3.9 Recommendation runner operations (2026-08-13)

- `recommendation-runner`는 16:30 KST 기본 스케줄과 시작 시 최신 적격 종가 catch-up을 사용한다. 활성 Paper 계좌 바인딩 중 `auto_apply_recommendations=true`인 경우만 자동 요청하며, 수동 요청과 lineage를 섞지 않는다.
- Compose/systemd는 curated 데이터와 고정 11-ETF universe를 읽기 전용으로 마운트하고 worker DB password를 `_FILE`로만 받는다. broker credential은 이 서비스에 주입하지 않는다.
- healthcheck는 non-secret runtime state(재시작 시 초기화), process heartbeat, read-only DB reachability, 마지막 schedule 결과(빈 cycle 포함), queue age, BLOCKED run 수를 보고한다. synthetic 11-ETF QA smoke는 실제 배포/큐 경로 검증용일 뿐 production data가 아니다.
- 실 KRX provider, 라이선스/credential/entitlement 증거 및 운영 provisioning은 여전히 외부 blocker다. 이들이 없으면 production recommendation은 fail-closed로 차단되어야 한다.

---

## 4. 앞으로 해야 할 일

### 한눈에 보기 — 완료된 코드 작업과 남은 항목

| # | 항목 | 누가 | 지금 시작 가능? |
|---|---|---|---|
| 1 | Paper 러너 데몬 | **코드 작업** | ✅ **완료** (`8da6548`) |
| 2 | `daily_equity`(종가 평가) 쓰기 | **코드 작업** | ✅ **완료** (`cf8704a`) |
| 3 | 리스크 게이트 입력 3개 배선 (장 캘린더·데이터 신선도·주문 충돌) | **코드 작업** | ✅ **완료** (`d7d75c7`, 연구 발행 이음매 `30e2679`) |
| 4 | phase-0 골든에 수수료 필드 추가 재승인 | **사장님 결정** | ⛔ 동일 |
| 5 | KRX 계약·실제 provider/credential/endpoint / Auth0 / KIS 실계좌 | **외부 구현·사장님 조달·운영자 provisioning** | ⛔ 현재 저장소만으로 완료 불가 |

1·2는 Paper 세션에 완료했고, 3도 발행된 연구 메타데이터까지 이음매가 연결됐다. Paper **엔진**(체결 로직)은 커밋 `ec81d73`에, 러너와 종가 평가는 각각 `8da6548`·`cf8704a`에 있다. 저장소 안의 synthetic Raw→PostgreSQL 경로와 리스크 소비 이음매는 완료됐지만, 실제 KRX provider 구현과 production credential/endpoint/운영자 provisioning은 외부 잔여 작업이다. 외부 조달·소유자 결정 항목도 그대로다.

### 4.1 소유자만 할 수 있는 것 — 외부 조달 3건

| 항목 | 구체적으로 |
|---|---|
| **E1** KRX 서면 데이터 권리 + 실제 공급자 | 초대 사용자 5명 + 파생 분석물을 포괄하는 계약 아티팩트, 라이선스·entitlement-aware KRX HTTP transport 구현, 실제 endpoint/credential, `research_writer` role·secret·Raw volume 운영자 provisioning. 현재 저장소에는 synthetic fixture와 발행 이음매만 있고 real feed는 live가 아님 |
| **E2** Auth0 테넌트 | 실제 테넌트 + 자격증명 (vendor 스위트 실행용) |
| **X1/X2** KIS 실계좌 | 실거래 자격증명 + 소액 실주문 증거 |

이 3건이 없는 동안 게이트는 `BLOCKED_EXTERNAL`이며 **그것이 합격 조건이다. 위조해서 APPROVED에 도달하는 것은 금지** — 닫히는 쪽으로 실패하는 것 자체가 게이트의 존재 이유다. 한국 데이터 권리가 끝내 안 오면 계획의 답은 Member 접근 연기다, 시장 변경이 아니라.

### 4.2 소유자 결정 대기 2건

1. **phase-0 골든에 수수료 필드를 넣을지** — 넣는 것은 승인된 기준값을 바꾸는 명시적 재승인 행위라 보류 중
2. **Phase 4 우선순위** — §4.4 참조

### 4.3 코드 작업 — 착수 가능, 권장 순서

1. **배포 서비스 활성화.** `deploy/systemd/paper-runner.service`와 운영별 `/etc/lagrange/paper-runner.env`를 설치·시작하고, 실제 role-scoped DB URL과 curated dataset 마운트를 호스트 Secret Manager에서 주입해야 한다. 저장소에는 비밀값을 넣지 않는다.
2. **실제 KRX provider와 운영 원천 활성화.** 코드의 synthetic 수집·발행·복구 배선은 완료됐지만, 라이선스·credential·entitlement-aware HTTP transport와 실제 endpoint를 구현하고 운영 secret, `research_writer`, migration, Raw volume을 provisioning해야 한다. 그 뒤 운영 PostgreSQL에 실제 KRX 캘린더와 EOD batch 메타데이터를 공급하고 라이브 계정의 intent 상태를 정상적으로 유지해야 한다. 원천이 없거나 오래되면 게이트는 계속 닫힌다.
3. **E7 Playwright 포함 전체 게이트 재실행** — 증거 신선화(이번 재실행도 E7은 스킵).

**작지만 기록해 둘 잔여 항목** (아키텍트 검토에서 발견, 차단 아님): `strategy_promotion`(§3.5)이 계좌 단위라 그 계좌에 묶인 주문 전부를 승격된 것으로 본다 — 운영 원천이 채워져 결정적 검사가 되기 전에 재검토할 것. `positions` upsert의 `ON CONFLICT ... DO UPDATE`가 갱신 절에서 소유자를 재확인하지 않는다 — 지금은 계좌-소유자가 1:1이라 안전하지만, 스키마 차원의 보강(`UNIQUE (account_id, owner_user_id, instrument_id)`)이 더 견고한 해법이다.

### 4.4 Phase 4 잔여

| 항목 | 상태 | 선행 조건 |
|---|---|---|
| PIT 재무 팩터 | **골격 완료** — 채우기만 남음 | 한국 재무 데이터 공급원 + 계약 |
| 개별주식 동적 유니버스 | 미착수 | KRX 개별종목 데이터 권리 (E1보다 큰 요구) |
| 분봉·틱·호가 | 미착수 (`Cadence::parse("intraday")`는 현재 타입 오류로 거부 — 의도) | 시세 권리 + 신규 수집 파이프라인 |
| LEAN 연구 워커 | 평가 대상, **트리거 미충족** | 설계 §1263: 미국 Fundamental·LEAN Universe 확보 시에만. 현재 조건 0/2 |
| 순수 Rust NT 경로 확대 | 미착수 | 없음 — 지금 가능하나 사용자 가시 기능 아님 |

**데이터 권리 없이 앞 셋의 코드를 쓰면 검증 불가능한 코드가 된다** — 착수하지 않은 이유.

---

## 5. 개발 환경 주의사항 (반복 비용을 치른 것들)

| 함정 | 대응 |
|---|---|
| WSL이 유휴 시 VM을 종료 → Postgres가 테스트 중 내려감 (`PoolTimedOut`) | 긴 스위트 전 `wsl -d Ubuntu -- sh -c 'sleep 3000'` 백그라운드 실행 |
| `command -v docker`는 **엔진이 죽어도 성공** | `docker version --format '{{.Server.Version}}'`으로 확인. phase2/3 게이트는 Docker QA DB(127.0.0.1:55432) 필수 |
| `phase1-gate.sh`는 WSL 전용 | Windows에서는 `phase1-gate.ps1` (이제 .sh가 스스로 거부한다) |
| 카고가 옛 산출물을 안 지움 — 하루 이터레이션으로 수 GB 누적 | 긴 세션 뒤 `cargo clean`; `CARGO_TARGET_DIR=C:/cargo-target/lagrange`는 셸마다 재설정 |
| 판정 아티팩트는 스냅샷 | 유의미한 변경 후 게이트 재실행 — 파일은 자동으로 낡는다 |
| 게이트를 백그라운드로 돌리면서 그 게이트가 빌드할 크레이트를 동시에 편집 | 편집 중간 상태(예: DTO 필드는 추가했는데 호출부는 아직)를 빌드해 가짜 `DENIED`/컴파일 실패가 남는다. 코드를 만지는 동안엔 게이트를 돌리지 말고, 편집 완료 + `cargo check` 클린 확인 후 돌릴 것. 원인 불명의 실패는 먼저 "지금 트리가 안정 상태였나"부터 의심 |
| PITR 복구 드릴은 `&`로 백그라운드에 넣지 말 것 | `run_in_background: true` 도구 자체가 백그라운드 처리를 하므로 셸 레벨 `&`를 더 얹으면 실제 스크립트가 끝나기 전에 "완료" 알림이 옴(도커 컨테이너만 뜨고 백업은 미완성). 스크립트를 그대로 포그라운드처럼 넘기고 도구의 백그라운드 기능만 쓸 것 |

---

## 6. 이 저장소가 지키는 원칙 (작업자를 위한 요약)

1. **Fail-closed.** 읽을 수 없으면 거부. `Unknown`은 허가가 아니다.
2. **권위는 하나.** 현금은 원장 재생으로만; 세율·수수료는 버전 있는 설정으로만; 파생 수치의 저장 사본은 두 번째 진실이 되므로 대조 없이 믿지 않는다.
3. **검증은 이음매에서.** 컴포넌트 테스트와 게이트 통과는 경로가 동작한다는 증거가 아니다. 실서비스 진입점을 부르는 테스트가 있는지 먼저 확인하라.
4. **없는 값은 없는 채로.** 0 대입·이월·기본값 대체는 하류에서 구분 불가능한 거짓 신호를 만든다.
5. **게이트 증거를 조용히 바꾸지 않는다.** 골든 재승인·게이트 입력 변경은 명시적 행위이며 커밋 메시지에 선언한다.
6. **차단을 위조하지 않는다.** BLOCKED_EXTERNAL은 실패가 아니라 이 시스템이 설계대로 멈춰 있다는 증거다.
