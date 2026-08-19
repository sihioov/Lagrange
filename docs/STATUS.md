# Lagrange Station — 상태 종합

**최신 기준일: 2026년 8월 19일 (2026-08-19).** 현재 권위 있는 운영 스냅샷은 바로 아래 §0이다. 이후 §1부터는 설계 목표와 08-17 이전 게이트·구현 이력을 보존한 기록이다. 과거의 "미설치", "KIS credential 없음", "초기 백필 미완료" 같은 문장은 당시에는 사실이었지만 현재 상태를 뜻하지 않는다. 코드가 바뀌면 게이트와 판정은 다시 실행해 갱신해야 한다.

## 0. 2026-08-19 현재 운영 스냅샷

### 0.1 저장소와 실행 환경

- 운영 저장소는 `/data/workspace/lagrange`, 브랜치는 `main`, 기준 커밋은 `8da8527145505f7068499b6d703f8fada35d3e5f`다.
- `main`은 현재 `origin/main`보다 69개 커밋 앞서 있고 아직 push하지 않았다.
- 추적 파일은 깨끗하다. 공식 KIS 참고 문서 `docs/kis_openapi_entiredocs_20260818_030007.xlsx`만 의도적으로 untracked 상태이며 수정·삭제·커밋하지 않는다.
- Linux 서비스 계정/경로, PostgreSQL 역할과 migration 1~45, DB/KIS/Auth0/crypto secret, Tailscale TLS, 운영 이미지, immutable release, TLS 갱신 timer와 암호화 백업·복원 timer를 준비했다. 실제 secret 값은 이 문서나 Git에 기록하지 않는다.
- 확인 시점에 `lagrange-station-postgres-1`만 의도적으로 실행 중이며 31시간 동안 healthy다. API/Web/recommendation/backtest/Paper/reverse-proxy와 Live/order profile은 아직 기동하지 않았다.
- `/var/lib/lagrange/data`의 현재 실데이터 사용량은 약 411MiB다. Rust `target`, Docker image와 BuildKit cache는 운영 데이터 및 백업 범위가 아니다.

### 0.2 Stage5 KIS 과거 시세 상태

- immutable source Raw batch: `3d4f061f-8b8c-54f3-bb44-4d491b3ad256`
- Raw 파일 187개: 고정 ETF 11종목 × 17개 기간 창
- 정규화 결과: `2020-01-31`부터 `2026-08-19`까지 XKRX 거래일 1,608일, 매 거래일 정확히 11종목
- 승인 XKRX calendar artifact hash: `sha256:467f189584ceeeaa32858d3b3ae87b2cd1e93d049fb091ec623de050dc3bc6e9`
- provider-free recovery state는 `COMPLETED`이며 기존 Raw batch를 재사용했다. recovery 서비스는 `network_mode:none`, Raw read-only 범위, KIS secret/환경변수/DB/backend/egress 없음으로 격리된다.
- 이 데이터의 계약은 계속 `vendor_snapshot=true`, `strict_pit=false`, `ready=false`다. 아직 Production Curated, DB publication, recommendation/backtest/Paper five-pin에 연결하지 않는다.

### 0.3 출시 판정

현재는 운영 기반과 KIS 가격 수집·정규화까지 준비됐지만 **production READY 출시는 아니다**. KIS가 반환한 과거 가격은 수집 시점 vendor snapshot이며, 과거 시점의 종목 상태·기업행사 공개시각·정정/철회 계보를 단독으로 증명하지 못한다. 이 상태에서 five-pin을 만들거나 추천·백테스트·Paper에 연결하면 미래정보 참조 금지 원칙을 위반할 수 있으므로 fail-closed를 유지한다.

실거래는 별도 후속 프로젝트다. 계좌·잔고·주문·정정·취소·체결·주문 WebSocket과 Compose `live` profile은 계속 금지한다.

### 0.4 Stage6 공식 데이터 통합 결정

사용 목적은 개인 내부용으로 진행한다. 데이터는 API에서 DB로 직접 넣지 않고 다음 경계를 지킨다.

`공식 소스 → immutable Raw → 교차검증/종목 식별 → 승인된 Curated → DB`

소스 역할은 다음처럼 고정한다.

- **KIS**: 고정 ETF 11종목의 주 가격 소스. 과거 데이터는 strict PIT가 아닌 수집 시점 vendor snapshot으로 표시한다.
- **KRX Open API**: 종목·시장 상태·상장 효력일·시장조치의 최종 기준이며 KIS 가격의 교차검증 소스다.
- **KIND**: 신규/추가/변경상장, 상호변경, 시장조치와 게시·정정 관계를 보강한다. 반드시 API일 필요는 없고 공식 Excel/다운로드도 immutable Raw로 수집할 수 있다.
- **OpenDART**: 기업공시 결정 정보, 최초 공시시각, 정정·철회 체인을 제공한다.
- **KSD/SEIBro**: 배당·증자·감자·합병/분할·권리 일정 등 기업행사 세부사항을 제공한다.
- 소스가 충돌하거나 공개시각·효력일·정정 계보가 부족하면 자동 추정하지 않고 Raw만 보존한 채 Curated/READY/pin을 차단한다.

초기 범위는 고정 ETF 11종목, 시작일 `2020-01-31`이다. KIS는 주 가격, KRX는 가격 교차검증과 시장 효력일, KIND/OpenDART는 공개·정정 시각, KSD/SEIBro는 권리·행사 세부 기준으로 사용한다. record date·지급일·상장일·권리락일을 `available_at`으로 소급하지 않는다.

### 0.5 다음 작업 계획 — 아직 구현 시작 전

1. **(문서화 완료 — 승인 대기, §0.6 참조)** KSD/SEIBro·KRX Open API·OpenDART·KIND의 ETF11 read-only endpoint/파일, 이용 조건, 필드, 페이징, 수정·철회 의미를 공식 문서로 고정한다.
2. KSD/OpenDART부터 fixture 기반 immutable Raw 어댑터를 구현한다. exact request/source metadata, `retrieved_at`, content hash, pagination 완전성을 검증하고 실제 key가 필요한 호출은 별도 운영 gate로 둔다.
3. KRX/KIND 근거로 ETF11 종목 identity와 유효구간을 구성하고 KIS 가격↔KRX 가격, KSD 이벤트↔KIND/OpenDART 공시를 교차검증한다.
4. 효력일·공개시각·정정 계보가 충족된 이벤트만 canonical Curated로 변환한다. 누락·충돌·애매한 기업행사는 typed blocker로 중단한다.
5. `2020-01-31` 이후 ETF11 pilot 백필을 수행해 Raw/lineage/hash/count와 strict PIT 가능 범위를 검수한다.
6. 승인된 DatasetManifest와 five-pin을 확정하고 DB publication 및 recommendation/backtest/Paper의 동일 exact pin 사용을 검증한다.
7. release scope를 적용하고 Auth0/TLS/API/Web/worker/reverse-proxy health, 재부팅, 백업 복원을 최종 확인한다.

실제 구현은 coordinator agent가 총괄하고 Sonnet 5 하위 에이전트가 조사·구현을 수행한다. 조사 결과는 파일로 직접 쓰지 않고 coordinator가 조립하며, 커밋은 coordinator만 수행한다. 모든 편집과 커밋은 `/data/workspace/lagrange`의 `main`에서만 수행하며, 단계별 검토와 검증을 통과한 뒤 다음 단계로 이동한다.

### 0.6 Stage6 Step 1 결과 — 공식 소스 계약 문서화 (2026-08-19)

Step 1을 문서 수준에서 완료했다. 구현·어댑터·신규 API 호출·계정 등록·key 발급·allowlist 변경·DB 변경은 **하나도 수행하지 않았다**. 산출물은 두 개다.

- `docs/decisions/0004-stage6-official-source-contracts.md` — 결정 기록 (Status: Proposed, 운영자 승인 대기)
- `docs/runbooks/stage6-source-contracts.md` — 소스별 증거·인용·gap·운영자 검증 체크리스트 15항목

조사 방법은 공개 문서 페이지 조회로 한정했다. 계정 등록·key 발급·인증 호출·data endpoint 호출·form 제출·로그인은 전부 하지 않았고, 모든 주장에 fetch URL을 붙였으며 확인하지 못한 것은 추론 대신 typed gap으로 남겼다. 결정을 좌우하는 8개 주장은 별도 적대적 검증 패스로 재확인했다.

**§0.4의 전제 3건이 공식 문서와 어긋나므로 정정한다.**

1. **KRX Open API는 상장 효력일·시장조치의 API 기준이 될 수 없다.** 서비스 카탈로그는 7개 분류 31개 항목의 단일 표이며, 상장·상장폐지·상장변경·관리종목·매매거래정지 endpoint가 없고 ETF용 `종목기본정보`도 없다(`종목기본정보`는 유가증권/코스닥/코넥스 주식만). `ETF 일별매매정보`는 `2010-01-04`부터 존재한다. 해당 범주는 `data.krx.co.kr` 이슈통계와 KIND의 공식 다운로드로만 얻을 수 있다.
2. **OpenDART는 최초 공시 *시각*을 제공하지 않는다.** 공시검색 API 응답 15개 필드 중 `rcept_dt`는 `공시 접수일자(YYYYMMDD)`이며 시각 필드가 스키마 어디에도 없다. 정정→원공시를 잇는 구조화 필드도 없고 `rm` 필드의 `정`/`철` 자문 코드만 있다. 배당 *결정* 이벤트 API는 존재하지 않는다(DS002의 `배당에 관한 사항`은 정기보고서의 실현 금액 요약이다).
3. **KSD 직접 연결은 KIS `ksdinfo`에 대한 독립 교차검증이 아니다.** 둘 다 KSD 원천이므로 비교는 중계 충실도 검증이다. KIS 가격↔KRX 가격도 동일하게 KRX 원천이므로 독립 관측이 아니다.

**정정된 가용시점 원칙.** 어떤 공식 소스도 sub-day 공표시각을 문서화하지 않으므로 Stage6는 **일 단위**로 설계한다. 이는 상한이며 추정이 아니다 — 확인된 시각 소스가 나오면 `available_at`을 좁힐 수만 있고 넓힐 수는 없다. `금융위원회_KRX상장종목정보`가 명문화한 "기준일자 익영업일 13시 이후 갱신"은 **서비스 갱신 주기**이지 레코드별 knowledge-time이 아니므로, 이 근거로 admit된 데이터는 `strict_pit`이 아니라 `documented_cadence`로 표시한다. 기업행사는 KSD가 *무엇을*(일정·팩터), 공시 소스가 *언제 알 수 있었는지*를 제공하는 결합으로만 구성하고, 공시 근거 가용일이 없는 이벤트는 Raw에만 남긴다.

**권리 관계는 기록하되 결론내지 않는다.** KRX Open API 약관(2025-12-26 시행)은 비상업적 목적 한정(제6조②), 제3자 제공 금지(제11조②), 키당 1일 10,000회(제8조④)이며, **제11조③은 이용계약 종료 후 이미 제공받은 정보의 이용을 금지**해 immutable Raw 영구보존과 충돌한다. 같은 KRX 원천의 공공데이터포털 금융위 미러는 `이용허락범위: 제한 없음`, 무료이므로 해당 범주에서는 미러를 우선한다. KSD 계열 포털 데이터셋은 KOGL 제2유형(출처표시·상업적 이용금지)이다. 해석과 `entitlement_reference` 확정은 운영자 결정 사항이다.

Stage5 계약 flag는 그대로 `vendor_snapshot=true`, `strict_pit=false`, `ready=false`다. Curated/DB publication/five-pin/추천·백테스트·Paper 연결은 계속 `BLOCKED`다.

**부분 승인 (2026-08-19).** 소유자가 OpenDART 코어(`list.json`/`list.xml`, `corpCode.xml`, `company.json`)를 fixture 기반 Raw 어댑터 작업 범위로 승인했다. 나머지 allowlist 행, 모든 계정 등록, 라이선스 해석은 계속 보류다. 어떤 소스에도 key가 발급되지 않았으므로 실제 요청은 한 건도 발생하지 않았다. KRX 미러 vs 원본 선택, KSD 포함 여부, KOGL 제2유형·KRX 제11조③ `entitlement_reference`는 미결정 상태로 남는다.

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

**한 줄 요약: 2026-08-18 출시 후보 기준으로 Phase 1 E2~E7, Phase 2 P1~P7, Phase 3 L1~L11, 양 단계 장애 주입 30개 시나리오와 실제 PITR 복구가 모두 통과했다. 종합 F3는 코드 결함 없이 `BLOCKED_EXTERNAL`이며, 출시를 막는 것은 E1 KIS broker data-rights/실제 entitlement·credential·초기 dataset과 X1/X2 KIS 실계좌/소액 주문 증거다. KIS read-only provider wiring과 Linux 운영 preflight는 저장소에 있지만, 운영 secret·데이터 volume·systemd 설치·실제 endpoint 검증은 아직 수행하지 않았다(§2.12, `docs/runbooks/kis-production-backfill.md`).**

### 2.1 게이트 판정 (2026-08-17 재실행, `61af2bb`, `--include-failure --include-restore` 포함)

| 게이트 | 코드 검사 | 판정 | 막고 있는 것 |
|---|---|---|---|
| Phase 1 | E2~E7 **전부 PASS** | `BLOCKED_EXTERNAL_DATA_RIGHTS` | E1 KIS broker data-rights/entitlement 단독 |
| Phase 2 | P1~P7 **전부 PASS** | `OWNER_ONLY_BLOCKED_EXTERNAL` | E1 및 Phase 1이 외부 권리 때문에 `APPROVED`가 아닌 상태 |
| Phase 3 | L1~L11 15개 **전부 PASS** | `BLOCKED_EXTERNAL_CREDENTIALS` | X1/X2 실제 KIS 계좌 |
| 장애 주입 (failures) | Phase 2 15개 + Phase 3 15개, **30개 전부 PASS** | **`PASS`** | — |
| PITR 복구 (restore) | 실제 백업 생성 → 격리 타깃 복구 → 검증 | **`PASS`** (`verdict: SUCCESS`) | — (복원 완료) |
| **종합 (F3)** | — | **`BLOCKED_EXTERNAL`** | E1/X1/X2 (외부 조달만 남음; E2 Auth0는 해소) |

**이 08-17 게이트 범위에서 코드나 로컬 실행 환경 때문에 막힌 항목은 없다.** 실제 `pg_basebackup`, WAL 7개, 목표 LSN `0/5000028`을 사용한 격리 복구에서 목표 시점 80행, 이후 행 0, provenance 3행, secret marker 0건, 파일 해시 불일치 0건을 확인했다. Phase 2 복원 실패 드릴 6개를 포함한 장애 주입은 양 단계 모두 15/15다. 판정 아티팩트는 `.omo/evidence/`의 10:22~10:25 UTC 실행본이며 gitignore 대상이므로 이 호스트에만 존재한다.

### 2.2 테스트 (기준일 최종 실행)

| 스위트 | 결과 |
|---|---|
| Rust 워크스페이스 (08-10 기록) | **1,051개 통과** (Paper runner/valuation 이음매 테스트 11개 추가 포함) |
| Rust 워크스페이스 (08-11 재실행) | **1,192개 통과, 4개 의도적 ignore, 실패 binary 0개** — `--no-fail-fast`, QA PostgreSQL 포함 |
| Python (nt + 골든, 08-09 기준 실행) | **239개 통과**, 1 스킵 — 가격 보정 전 기준선의 역사적 실행 기록이며, 현재 v2 증거의 최신 전체 수치를 뜻하지 않음 |
| Web (vitest + tsc) | **48개 통과**, `openapi:check` 클린, `tsc --noEmit` 클린 — 08-13 Windows 호스트 기록. 08-14 Linux 호스트 재실행 결과는 §2.6 |
| clippy (workspace, all-targets, all-features) | `-D warnings` 클린 (08-11 재실행) |
| rustfmt (workspace) | **PASS** — 08-11 GitHub Actions 도입 시 기존 drift를 pinned rustfmt로 기계 정규화 |
| Multi-universe vertical (08-17) | live PostgreSQL migration contract **25/25**, full workspace fmt/check/strict Clippy/tests, OpenAPI drift, Web lint/typecheck/**69 unit**/build/**Playwright 3**, CI contract **6/6**, Docker functional smoke **PASS** (§2.9) |

### 2.3 최종 판정 아티팩트 (`.omo/evidence/`)

| 아티팩트 | 판정 | 발행일 | 신선도 |
|---|---|---|---|
| F1 계획 준수 | `REQUEST_CHANGES_RESOLVED` (42/42 완료) | 08-08 | ⚠️ 이후 커밋 ~100개(Auth0·추천 파이프라인 포함) — **사람의 재검토가 필요한 문서** |
| F2 코드 품질 | `APPROVE` | 08-08 | ⚠️ 동일 |
| F3 운영 E2E | `BLOCKED_EXTERNAL` | 08-17 | `61af2bb`, Phase 1/2/3 + failures + restore 재검증, 최신 |
| F4 범위 충실도 | `APPROVE` (LEAN·미국·분봉·파생 부재 확인) | 08-08 | ⚠️ F1과 동일 |

F1/F2/F4는 스크립트가 아니라 **사람이 코드를 읽고 내린 판단문**이므로, 갱신하려면 재검토를 해야 한다. 이번 세션의 Paper 실행 경로 구현이 Phase 2 관련 재검토의 실질적 근거가 됐다.

### 2.4 단계별 실질 상태

- **Phase 0** — 완료. 가격 스케일을 바로잡은 v2 증거 기준선을 재승인했다. Phase 0의 6개와 robustness 5전략의 30개 비-provenance 경제 아티팩트는 이전 승인본과 byte-identical하여 전략 경제 결과가 유지됐고, provenance와 identity는 v2 계약으로 갱신됐다.
- **Phase 1** — Raw→PostgreSQL 연구 메타데이터 발행 경로와 KIS read-only provider wiring을 연결했다. 각 수집은 먼저 불변 Raw batch와 append-only manifest로 내구화되고, 검증한 **같은 batch**의 `data_batches` 4행과 캘린더를 한 트랜잭션으로 발행한다. `trading_calendar_versions`는 정정 이력을 append-only로 보존하고 `trading_calendars`는 더 최신 `retrieved_at`만 현재 projection으로 전진시킨다. 복구는 exclusive commit lock 아래 orphan evidence를 재동기화하고 실제 JSONL line으로 먼저 내구화한 뒤 append-order immutable high-water snapshot을 16건씩 재생하며, timeout 뒤 마지막 검증 cursor에서 재개한다. Linux에서는 UID 10001이 `0440` evidence/`batch.json`을 read-only handle로 `fsync`하고, `0640` manifest/lock만 변경한다. 따라서 orphan 뒤에 대기하던 정상 append도 실제 line 순서의 다음 suffix가 되고, snapshot 종료 후 high-water 불변을 재확인하므로 backdated concurrent append도 누락되지 않는다. daemon은 catch-up/매 scheduled cycle 직전에 다시 복구한다. `research-worker`는 16:30 KST 기본 일정과 지각 시작 즉시 catch-up, one-shot/daemon, 구조화 이벤트, batch-date-aware 4일 신선도 healthcheck, synthetic-production 선차단을 제공한다. Compose는 host `<data>/raw`↔container `/data/raw` 직접 Raw 경로, secret 파일, `cap_drop: ALL` 뒤 `CHOWN`/`FOWNER`/`DAC_OVERRIDE`만 복원하는 no-follow recursive UID 10001 Raw 초기화, exact constraint/column/index와 append-only function body까지 검사하는 migration/schema/role drift gate, 최소권한 `research_writer`를 연결했다. Risk Gateway와 worker health는 미래 batch를 제외하고 `min(retrieved_at, KST batch-date 종점)`을 동일하게 사용하므로 새로 발행한 역사 backfill도 stale이다. 단, 이는 synthetic fixture와 KIS wiring 기반 개발/QA 경로의 완료다. **실제 KIS endpoint/credential, broker data-rights/entitlement를 적용한 production feed는 아직 live가 아니다.** 초기 백필·dataset pin과 운영자 provisioning도 남아 있다. 한편 Phase 1의 사용자 기능 축인 **추천 조회는 08-12~13에 고정 11-ETF 파이프라인으로 구현 완료**됐다(제출→계산→발행→최신/이력 화면, §3.10).
- **Phase 2** — **실행 엔진·러너·종가 평가 경로 구현 완료** (`ecef4b2`, `cf8704a`, `8da6548`). `job_queue::paper_execution::execute_session`이 큐잉된 target을 실제로 체결해 `orders`/`fills`/`positions`/`cash_ledger`에 기록하고, `api_server::paper_session::run_and_settle`이 정산·패리티·통지를 수행한다. 새 `api_server::paper_runner::run_cycle`은 worker 역할의 전체 due target을 소유자 Actor로 재진입시켜 실행하고, 활성 PAPER 계좌를 스캔해 `job_queue::paper_valuation::value_account`를 호출한다. 종가 평가는 원장 현금 자기대조·보유 포지션별 curated close·미래 close 차단·cost profile 검증을 거쳐 `daily_equity`를 계좌/날짜별 불변·멱등으로 기록한다. `crates/api-server/src/bin/paper-runner.rs`가 `--once`/`--date`, 환경별 풀, 2초 polling/10초 backoff, Ctrl-C 종료를 제공한다. 실제 QA DB 이음매 테스트로 두 소유자 스캔, 실행·통지 중복 방지, 정확한 equity/cash/positions_value, missing/future/conflicting close, LIVE·교차 테넌트 거부를 검증했고, Python/Web/외부 데이터 권리 차단은 그대로다. 호스트 배포 단위와 운영 credential 주입은 `deploy/systemd/paper-runner.service` 및 `paper-runner.env.example`로 등록했고, `scripts/qa/paper-runner-smoke.ps1`가 해당 유닛 정적 계약·QA DB 테스트·CLI smoke를 묶는다. 08-12~13에 **추천→Paper 연계**가 추가됐다: 16:30 KST 스케줄러가 `auto_apply_recommendations=true`로 opt-in한 활성 바인딩에만 scheduled run의 target을 자동 발행하고(§3.10), 수동 run은 리밸런싱 미리보기 + 명시적 적용 경로(§2.5)로만 pending target이 된다 — 두 lineage는 섞이지 않는다.
- **Phase 3** — 안전 불변식 검증 완료(L1~L11) + 이번 감사로 치명 결함 수정. 게이트 입력 5개 모두 코드 수준에서 실제 원천에 연결됐다(`85f1902`: `strategy_promotion`/`instrument_allowed`, `d7d75c7`: KRX 세션·EOD batch freshness·actor-scoped intent conflict). 원천 행이 없거나 읽을 수 없으면 여전히 `Unknown`으로 닫히며, 운영 캘린더·데이터 수집 메타데이터가 준비되기 전까지 **라이브 주문은 승인되지 않는다** — 의도된 fail-closed 상태.
- **Phase 4** — PIT 재무 팩터의 **골격 완료** (`cda7182`): 이중 시간축(기간 + 공시일), 바 날짜별 as-of 해석, 정정 공시 처리. 실제 재무 데이터가 오면 채우기만 하면 된다. 나머지 항목은 미착수 (§4.4).

---

### 2.5 추천 기반 Paper 리밸런싱 미리보기 (2026-08-13)

추천 결과를 활성 Paper 계좌에 적용하기 전에 주문 후보·예상 수수료·가용 현금·잔여 현금·종목별 판단 사유를 고정소수점으로 계산하는 백엔드 경로를 추가했다. 미리보기는 추천 종가와 정확한 데이터셋/포트폴리오/계좌 상태 해시에 고정되며, 상태가 달라지면 적용을 거부한다. 명시적 적용은 주문이나 체결을 즉시 만들지 않고 `MANUAL_RECOMMENDATION` pending target만 원자적으로 생성한다. 실제 Paper 실행 시에는 다음 거래일 raw open으로 다시 계획하므로 미리보기와 체결 모델을 혼동하지 않는다. 이번 범위는 API·큐·DB·실행 경계까지이며 **UI와 Live 주문은 포함하지 않는다.**

최종 무결성 감사에서는 worker가 계산 전에 on-disk manifest의 canonical hash를 DB pin과 다시 대조하고, snapshot 전·후 목표 포트폴리오 변경을 READY 발행 전에 영구 거부하도록 보강했다. `cash_ledger`/`positions`는 이제 `(account_id, owner_user_id)` 복합 FK로 실제 계정 소유자와 일치해야 하므로 교차 테넌트 행을 계정에 연결해 상태 버전을 우회할 수 없다. 적용 시에는 계산 당시 Seoul 날짜 이후의 첫 attested KRX 거래일이 여전히 같은지도 재검증한다.

### 2.6 Linux 호스트 이관 (2026-08-14)

개발 호스트가 Windows/WSL(31.5GB, 12 logical CPU)에서 native Ubuntu(14GB, 14 logical CPU)로 이관됐다. 관련 커밋은 `a66c46a`(Linux 대응 코드 수정 — `paper.rs`, `state.rs`, `paper-runner.rs`, `nt/isolation.py`, `nt/uv.lock`, 백업 정책 테스트)와 `3cdf785`(web 픽스처 정렬)이며, 720줄짜리 절차서는 `docs/LINUX_MIGRATION_AND_OPERATIONS.md`다.

**이관이 증거에 미친 영향이 이 문서에서 가장 중요한 부분이다.** `.omo/`는 gitignore 대상이라 저장소에 따라오지 않는다. 따라서 §2.1의 게이트 판정, §2.3의 F1~F4 아티팩트는 **현재 호스트에 파일로 존재하지 않는다.** 이는 판정이 낡았다는 뜻을 넘어, 이 머신에서는 아직 어떤 판정도 발행된 적이 없다는 뜻이다. 마찬가지로 `data/`(raw·curated·phase0·catalog)도 이관되지 않았다.

| 항목 | 2026-08-14 Linux 호스트 실측 |
|---|---|
| `scripts/check-pins.sh` | **PASS** — `ALL PINS OK (rustc/python/node/NT)` |
| `scripts/validate-foundation.sh` | **PASS** |
| toolchain | rustc 1.97.1, Python 3.12.13, Node 24.13.1, uv 0.12.1, Docker Server 29.7.2 |
| Web (vitest) | **55개 통과 / 14개 파일**, `tsc --noEmit` 클린, `biome check` 클린 (08-13 기록의 48개에서 증가) |
| Web e2e 하네스 | `synthetic-api.mjs` + `next dev` 조합이 Linux에서 동작함을 확인 (7개 라우트 렌더링). **E7 Playwright는 이 호스트에서 실행 가능하다** |
| Phase 0 데이터 | `scripts/ci/prepare_phase0.py`로 재생성 성공 — 3종목 × 260세션 = **780 bar**, `dataset_version: kr-etf-daily-phase0-v2` |
| QA PostgreSQL | `deploy/qa/qa-db.compose.yml`로 기동 성공 (127.0.0.1:55432, healthy) |
| Rust 워크스페이스 | **1,371개 통과, 실패 0, 의도적 ignore 5**, 151개 test binary — 아래 §2.7 |

**이관이 만든 잔여 작업 3건:**

1. **`scripts/qa/phase1-gate.sh`는 이 호스트에서 실행할 수 없다.** 46행이 `/root/.cargo/bin/cargo`의 존재를 요구하며 거부하는 WSL 전용 가드다. 런북 §8.2가 남긴 선택지는 두 가지다 — GitHub Actions Ubuntu runner의 결과를 증거로 삼거나, 가드를 native Linux용으로 수정·검증하는 것. 결정 전까지 이 스크립트의 환경 오류를 외부 blocker나 코드 합격으로 오해해서는 안 된다. `scripts/qa/research-worker-smoke.sh`도 동일하게 `wsl` 참조를 갖고 있다.
2. **`.cargo/config.toml`의 `jobs`는 옛 머신 기준이었다.** `6`은 31.5GB 기준으로 산정된 값이라 14GB 호스트에서는 peak 12~18GB를 요구한다 — 그 파일의 주석이 예고한 실패 모드 그대로다. 08-14에 `3`으로 낮췄다.
3. **git identity, `data/`, secret, systemd 유닛, Docker 컨테이너가 모두 미설정 상태다.** 런북 §12 체크리스트 기준으로 이관은 "코드와 toolchain은 준비됐고 나머지는 미복원" 단계에 있다. 런북 자체가 지시하듯 이 체크리스트가 끝나기 전에는 Windows 원본을 삭제하지 않는다.

### 2.7 Linux 호스트 최초 워크스페이스 테스트 (2026-08-14)

CI(`.github/workflows/ci.yml` `workspace-tests`)의 순서를 그대로 재현했다 — Phase 0 데이터 생성 → disposable QA DB 기동 → `cargo test --workspace --locked --no-fail-fast`.

| 항목 | 값 |
|---|---|
| 커밋 | `3cdf785` (+ 미커밋 변경: `.cargo/config.toml` jobs 6→3) |
| 호스트 | Ubuntu, 14 logical CPU, 14GB RAM, 4GB swap |
| 실행 시각 | 2026-08-14 (KST 오후) |
| 결과 | **1,371개 통과, 실패 0, 의도적 ignore 5** — 151개 test binary. 단일 실행 수치가 아니라 합산이다: 전체 실행에서 1,357개 통과·14개 실패, 그 14개를 `PYTHON` 지정 후 재실행해 전부 통과(아래 참조) |
| DB | `deploy/qa/qa-db.compose.yml`, `postgres://…@127.0.0.1:55432` (disposable) |

**따라서 `a66c46a`의 Linux 대응 코드 변경은 이 실행으로 처음 검증됐다.** 빌드 중 OOM이나 스래싱은 발생하지 않았다(jobs=3, peak 여유 5~6GB 유지).

첫 실행에서는 14개가 실패했는데, 원인은 하나였고 코드 회귀가 아니었다. 실패는 test binary 3개에 걸쳐 있었다 — `api-server/tests/http_recommendations.rs`(1), `job-queue/tests/recommendation_compute.rs`(12), `job-queue/tests/recommendation_runner.rs`(1). 셋 다 Phase 0 픽스처를 만들기 위해 `scripts/ci/prepare_phase0.py`를 자식 프로세스로 부르며, 인터프리터를 `PYTHON` 환경변수 또는 기본값 `python`에서 찾는다. GitHub Actions에서는 `setup-python`이 `python`에 pyarrow를 함께 제공하지만, **이 호스트의 `python`(`~/.local/bin/python`)에는 pyarrow가 없어서** 생성기가 `ModuleNotFoundError`로 죽었다. pyarrow 25.0.0이 있는 인터프리터를 `PYTHON`으로 지정해 재실행하니 세 바이너리 모두 통과했다(11/11, 16/16, 10/10). 여기에는 `uv`로 5개 배포 전략을 실제 worker로 발행하는 `real_worker_and_uv_publish_all_five_shipped_strategies`도 포함된다.

**Linux 작업자를 위한 운영 메모:** 워크스페이스 테스트 전에 pyarrow 25.0.0을 `python`에 설치하거나, `PYTHON=<pyarrow가 있는 인터프리터>`를 export한다. 이것을 하지 않으면 추천 계산 경로의 테스트 14개가 실패하는데, **실패 메시지가 Python 쪽에 있어 Rust 회귀로 오독하기 쉽다.**

### 2.8 실행 호스트 확정과 Auth0 실 테넌트 검증 (2026-08-17)

**서비스가 실행되는 호스트는 native Ubuntu Linux 하나다.** Windows/WSL은 개발 이력이며 운영 경로가 아니다. 이 확정이 만드는 실질적 결과가 하나 있다 — WSL 전용 스크립트(`scripts/qa/phase1-gate.sh`의 `/root/.cargo/bin/cargo` 가드, `research-worker-smoke.sh`의 `wsl`/`wslpath` 분기)에 대해 §3.3과 §5가 남겨둔 "Windows에서는 `.ps1` 쌍둥이로 우회한다"는 선택지는 **더 이상 대체 경로가 아니다.** 해당 스크립트는 native Linux용으로 이식하거나, GitHub Actions 결과를 증거로 삼는 결정을 해야 한다(§2.6-1, 런북 §8.2).

**`phase1-gate.sh`는 native Linux로 이식됐다(같은 날, 아래 §2.10).** WSL 잔재는 46행 가드 하나가 아니라 넷이었다 — PATH 교체, `CARGO_TARGET_DIR=/root/lagrange-target` 강제, `WSL_DATABASE_URL` 기본값 `:5432`(이 호스트 QA DB는 `:55432`). 이제 phase2/phase3 게이트와 같은 관례(호출자의 cargo, `$root/target`, `LAGRANGE_QA_DB_PORT`)를 쓴다. `research-worker-smoke.sh`의 `wsl`/`wslpath` 분기는 아직 남아 있다.

**Auth0 vendor 스위트가 실 테넌트를 상대로 통과했다.** `lagrange-station.jp.auth0.com` / client `YZ4T7g575IohtS1HsltlFAiU7AlyUUuI` 조합으로 `cargo test -p auth --test vendor_auth0 -- --include-ignored` 실행, commit `41e8005`에서 **5개 전부 통과**(vendor 3개 + 비-vendor 2개, 실패 0). 개별 확인은 JWKS가 RS256 키 2개를 게시하고, `/authorize`가 등록된 콜백으로 302를 반환하며, 설정된 credential이 `403 invalid_grant`(클라이언트 인증 통과 후 의도적 무효 code만 거부)를, 음성 대조군이 `401 access_denied`를 받는 것이다.

`auth0_client_secret` 실파일은 이 Linux 호스트의 `deploy/secrets/`에 배치됐다(mode 0600, 개행 없는 단일 행, symlink 아님 — `ClientSecret::from_file`의 거부 조건 전부 회피). §3.9가 기록한 "secret 파일은 이 호스트에 provisioning되어 있음"은 Windows 호스트 기준 서술이었고, 이번 배치로 Linux에서 처음 충족됐다.

**E2 게이트 증거는 §2.10의 게이트 실행으로 발행됐다.** 스위트 통과와 게이트 판정 발행은 다른 사건이며, 이 저장소에서 체크가 존재한다는 것은 후자를 뜻한다. 게이트 밖에서 증거 파일을 손으로 만들지 않는다는 원칙(5·6)은 그대로다.

게이트 실행 시 잊기 쉬운 전제: `LAGRANGE_AUTH0_DOMAIN`, `LAGRANGE_AUTH0_CLIENT_ID`, `LAGRANGE_AUTH0_CLIENT_SECRET` 세 환경변수를 미리 export해야 한다. 게이트는 부모 환경을 상속하지만 이 값들을 설정해 주는 코드는 저장소 어디에도 없고, 게이트가 secret 파일을 직접 읽지도 않는다 — 자격증명 주입은 운영자의 명시적 행위로 남긴다. secret은 `"$(cat deploy/secrets/auth0_client_secret)"`로 주입하고 인자나 로그에 남기지 않는다.

배포 대상 앱은 JP 테넌트의 `YZ4T7g575IohtS1HsltlFAiU7AlyUUuI` **하나**다. 검증 과정에서 다른 테넌트의 앱(`kwaoPWGvfvRQWlAIwUQd3LtiYr67UlAI`)이 후보로 등장했으나 이 저장소와 무관하며, 그 앱의 secret으로는 pin된 client가 인증되지 않는다.

### 2.9 KOSPI200/KOSDAQ150 후보 연구 vertical (2026-08-17)

개별주식 후보 연구 경로가 KOSPI200 단일 기본값에서 **KOSPI200 + KOSDAQ150 두 universe**로 확장됐다. Migration `0045`가 universe registry와 source snapshot/run/feed/sequence의 universe-scoped identity를 추가하고, 동일 종목이 두 지수에 속해도 두 ranking context를 보존한다. 기존 `0042`~`0044`는 수정하지 않았고 up/down checksum이 기준선과 일치한다. `0045` down은 KOSDAQ row·binding뿐 아니라 registry 생성 이후 발행된 publication-only source 이력도 감지해 데이터 손실 가능성이 있으면 거부한다.

수집은 fundamentals, investor flows, market status, sector classification 공통 4종과 KOSPI200/KOSDAQ150 membership 2종을 하나의 immutable source batch로 seal한다. entitlement·dataset binding·canonical partition·PIT effective/available/cutoff 조건을 검증하며, credential/transport가 없는 production 실행은 READY를 가장하지 않는다. 계산기는 registry 순서대로 universe를 독립 스케줄링하고 universe별 score normalization, run, sequence, feed, Top 5를 발행한다. 한 universe가 source 부족으로 막혀도 다른 universe의 성공 feed는 유지되고 replay는 멱등이다.

API는 universe 생략 시 기존 KOSPI200 동작을 유지하면서 명시적 KOSDAQ150과 screener one/both 조회를 지원한다. signed cursor v2는 전체 immutable run-set과 universe 위치를 고정하고, v1은 universe를 생략한 KOSPI200 호환 요청에서만 수용한다. Web candidate feed, screener, 종목 분석에는 universe 선택과 badge가 추가됐고 양쪽 지수의 동일 종목을 dedupe하지 않는다. OpenAPI JSON/TypeScript 생성물도 함께 갱신됐다.

검증은 migration contract 25/25, source/compute/API live tests, full workspace fmt/check/strict Clippy/tests, Web 69 unit + Playwright 3, CI contract 6/6을 통과했다. Docker smoke는 실제 research-worker와 candidate-runner를 사용해 공통 4 + membership 2 sealing, 두 universe별 Top 5, runner 2회 replay idempotency를 no-SKIP으로 확인했다. 독립 reviewer 최종 판정은 `P0=0`, `P1=0`, `core P2=0`, `P3=1`, `OK`다. 남은 P3는 source-missing readiness 메시지의 universe별 세분화로, correctness/release gate를 막지 않는 observability 항목이다.

이 완료 판정은 synthetic fixture를 사용한 코드·QA vertical과 KIS wiring에 대한 것이다. **실제 KIS production endpoint/credential, broker data-rights/entitlement, 운영 backfill과 dataset pin은 여전히 미완료**이며 실제 feed가 live라는 뜻이 아니다.

### 2.10 phase1 게이트 Linux 이식과 이 호스트 최초 증거 (2026-08-17, `5b3f832`)

게이트를 Linux에서 실행 가능하게 만들자 **이 게이트가 실행되지 않은 테스트에 PASS를 줄 수 있는 경로 3개**가 드러났다. §3.3("검사 도구 자체의 결함")의 연장이며, 게이트가 검사하는 내용을 바꾸는 일이므로 전부 커밋 메시지에 선언했다.

| # | 결함 | 실제로 실행된 테스트 | 수정 |
|---|---|---|---|
| E2 | `run_cargo`가 이미 `--`가 있는 호출에 `-- --nocapture`를 덧붙여 `cargo test … -- --ignored -- --nocapture`가 됐다. libtest가 두 번째 `--`와 `--nocapture`를 이름 필터로 읽어 5개 중 **0개** 실행 후 exit 0 → 게이트는 "real Auth0 tenant suite green" 기록 | 0 / 5 | cargo 인자와 libtest 인자를 분리해 구분자 하나만 삽입 |
| E4 | `cargo test -p auth protocol`은 테스트 **이름** 필터라 아무것도 매치하지 않는다. `invites`도 0개, `stepup`만 5개 중 4개 | 0 / 0 / 4 → **32 / 15 / 5** | `--test <target>` 사용. 세 스위트가 transcript 한 경로를 공유해 서로 덮어쓰던 것도 분리 |
| E7 | 준비 판정이 TCP 프로브뿐이고 자식이 아닌 subshell PID를 기록했다. 이 호스트에서는 mock이 `EADDRINUSE`, next dev가 `Cannot find module`로 죽었는데도 **다른 워크트리가 그 포트를 서빙 중이라 두 포트 모두 응답** → 게이트가 남의 앱을 상대로 Playwright를 돌렸다 | — | `exec`로 자식 PID를 잡고, 포트 전후로 생존 확인, 포트 override 허용, 의존성 부재 시 조기 차단 |

일반화 가드 2개를 함께 넣었다. 검사는 **cargo exit 0 + 최소 1개 테스트 실행**일 때만 PASS다(phase2 게이트가 Todo 35부터 쓰던 계수 방식). 그리고 **QA DB 도달성을 선검사해 안 되면 exit 2**, 판정 없음이다 — E5는 DB-gated인데 `DATABASE_URL`이 없으면 스스로 skip하고 skip은 passed로 집계되며, 반대로 DB가 죽어 있으면 모든 DB 검사가 suite failure가 된다(phase3가 그래서 DENIED를 발행했던 사고, §3.3). 게이트는 DB를 띄우지 않는다 — 공유 인스턴스는 `full-system-gate.sh`가 소유한다.

**이 호스트 최초의 phase1 판정** (2026-08-17T08:32:21Z, disposable QA DB, `LAGRANGE_AUTH0_*` export):

| 검사 | 결과 | 비고 |
|---|---|---|
| E1 written-rights | `BLOCKED_EXTERNAL` | `krx.entitlement.example.json`이 placeholder — E1 조달 대기, 정상 상태 |
| E2 vendor-auth0 | **PASS** (3) | §2.8의 실 테넌트 검증이 게이트 증거가 됐다 |
| E3 auth0-simulator | **PASS** (10) | |
| E4 auth0-invite-mfa | **PASS** (52) | 이전 게이트에서는 4개만 돌았다 |
| E5 phase1-five-user | **PASS** (5) | DB-gated, 실제 실행 확인 |
| E6 restore-policy | **PASS** | |
| E7 playwright-phase1 | `BLOCKED_EXTERNAL` | `apps/web/node_modules` 미설치 — 외부 차단이 아니라 **환경 미비**, 저장소 루트에서 `npm ci`(apps/* 는 npm workspaces라 루트에만 lockfile이 있다) 후 `npx playwright install`로 해소 가능 |
| **VERDICT** | **`BLOCKED_EXTERNAL_DATA_RIGHTS`** | E1 때문이며 이것이 Phase 1의 정상 결과다 |

증거는 `.omo/evidence/task-28-lagrange-station-implementation.json`에 있다. **`.omo/`는 gitignore 대상이므로 이 판정도 이 호스트에만 존재한다** — §2.6이 지적한 성질은 변하지 않았다.

E7은 §2.11에서 닫혔다. 남은 것은 phase2/phase3/종합 게이트 재실행이다(6b는 아직 부분 완료다).

### 2.11 E7 해소 — 게이트의 npm workspaces 가정 오류 2건 (2026-08-17)

web 의존성을 설치하자(`npm ci` 저장소 루트 → `npx playwright install`) E7이 여전히 열리지 않았다. 원인은 환경이 아니라 게이트였다. **`apps/*`는 npm workspaces라 의존성이 전부 루트 `node_modules`로 hoist되고 `apps/web/node_modules`는 아예 생성되지 않는데, 게이트가 그 경로를 두 곳에서 직접 참조하고 있었다.**

| 위치 | 증상 | 수정 |
|---|---|---|
| 사전조건 | `[ -d apps/web/node_modules/@playwright ]`가 거짓 → "dependencies not installed"를 기록하고, **그 해결책으로 방금 실행한 바로 그 `npm ci`를 안내**했다. 검사가 영원히 못 풀린다 | `require.resolve("@playwright/test", {paths:[apps/web]})`로 자식과 같은 방식으로 해석 |
| next 기동 | `node node_modules/next/dist/bin/next`를 상대 경로로 실행 → `MODULE_NOT_FOUND`. 게이트는 이를 "next dev exited immediately; **port may be taken by another worktree**"로 보고해 원인을 포트 충돌로 오인하게 했다 | `require.resolve("next/dist/bin/next", …)`로 해석하고, 해석 실패 시 조기에 의존성 부재로 보고 |

§2.10의 세 결함이 **실행 안 된 것을 PASS**로 만든 반면 이 둘은 반대 방향, 즉 **설치된 것을 미설치로** 보는 오탐이다. 닫히는 쪽이라 위험도는 낮지만 E7을 도달 불가 상태로 고정시킨다는 점에서 결과는 같다.

포트는 override가 실제로 필요했다 — 기본값 38180/33000을 이 호스트의 다른 워크트리가 이미 점유하고 있어 `PHASE1_E7_MOCK_PORT=38191`, `PHASE1_E7_APP_PORT=33011`로 실행했다. §2.10의 이식이 추가한 override와 자식 PID 확인이 없었다면 또다시 남의 서버를 테스트했을 상황이다.

`npx playwright install`이 출력한 호스트 라이브러리 누락 경고(libwebp, libmanette, libenchant 등)는 WebKit/Firefox용이며 `apps/web/playwright.config.ts`는 chromium 단일 project라 무관하다. chromium 실제 기동으로 확인했다.

**재실행 판정** (E1 외 전 항목 PASS):

| 검사 | 결과 |
|---|---|
| E1 written-rights | `BLOCKED_EXTERNAL` — placeholder entitlement, 조달 대기 |
| E2 vendor-auth0 | **PASS** (3) |
| E3 auth0-simulator | **PASS** (10) |
| E4 auth0-invite-mfa | **PASS** (52) |
| E5 phase1-five-user | **PASS** (5) |
| E6 restore-policy | **PASS** |
| E7 playwright-phase1 | **PASS** — chromium 4개 전부 통과(16.1초): 5인 추천 조회, 5인 백테스트, member의 Owner admin 차단, entitlement 차단 시 KR 파생 표면 전면 거부 |
| **VERDICT** | **`BLOCKED_EXTERNAL_DATA_RIGHTS`** — 이제 **E1 단독**이 유일한 차단 사유다 |

이 실행의 트리는 `c362216`, 즉 **multi-universe 후보 연구가 병합된 현재 main**이다(§2.10의 08:32 실행은 병합 이전 트리였다). 따라서 이것이 현재 main 기준의 phase1 증거다.

**이 호스트에서 phase1의 코드·환경 요인이 전부 소진됐다.** 남은 `BLOCKED_EXTERNAL_DATA_RIGHTS`는 KIS broker data-rights/entitlement라는 외부 조달 항목 하나이며, 그것이 없을 때 닫히는 것이 이 게이트의 존재 이유다.

### 2.12 오늘 출시 감사 — 전체 게이트와 Linux 배포 preflight (2026-08-17, `61af2bb`)

`review-work-progress`에서 Linux 운영 배포 경계를 다시 점검하고 `61af2bb`(`fix(release): harden Linux deployment preflight`)를 로컬 커밋했다. 이 커밋은 아직 원격에 push하지 않았다. 변경 범위는 다음과 같다.

- recommendation runner의 production env 예제를 추가하고 DB password file, dataset pin 5종, health state, 로그 설정을 명시했다.
- Paper Rust 바이너리와 shell entrypoint가 같은 경로를 덮어쓰던 런북을 고쳐 바이너리는 `paper-runner-bin`, wrapper는 `/opt/lagrange/bin/paper-runner`로 분리했다.
- Linux runbook의 오래된 WSL·Compose·migration·secret provisioning 설명을 현재 구현과 migration `0045`에 맞췄다.
- Phase 2/3/F3와 Paper/recommendation smoke가 Docker socket 권한 부족을 QA DB 장애로 오진하지 않고 exit 2 환경 오류로 거부하도록 했다.
- recommendation smoke에 `--static-only`를 추가했고 systemd env 예제와 Paper wrapper 설치 경로를 정적 회귀 검사로 잠갔다.

정적 배포·DB·Compose·secret·systemd 검사, CI contract 6/6, release binary 두 개 빌드와 `--help`, GitHub required CI, research-worker smoke를 확인했다. Phase 1은 E2~E7 통과/E1 단독 차단, Phase 2는 P1~P7 통과/외부 차단, Phase 3는 L1~L11 통과/X1~X2 차단, F3는 failures·restore `PASS`를 포함해 `BLOCKED_EXTERNAL`을 발행했다. Playwright Chromium은 호스트에서 `libasound.so.2` 하나가 빠져 있어 Ubuntu `libasound2t64` 패키지를 `/tmp`에 비권한 추출하고 이번 프로세스의 `LD_LIBRARY_PATH`로만 주입해 E7 4개를 실행했다. 시스템 패키지는 변경하지 않았다.

아직 `/etc/lagrange`, `/opt/lagrange`, `/var/lib/lagrange/data`와 production systemd 유닛은 설치되지 않았고, runtime DB/session/CSRF/cursor/TLS/KRX/KIS secret 및 실제 production dataset도 없다. 따라서 오늘 가능한 결과는 **코드·게이트 기준 release candidate + Owner-only 준비**까지다. Member KR-derived surface와 Live trading은 외부 증거가 생길 때까지 계속 비활성화해야 한다.

### 2.13 초대 그룹 계좌·주문·리포트 공유 조회 (2026-08-17, 작업 트리)

초대된 사용자는 서로의 Paper 계좌 목록·계좌 상세·주문·포지션·성과·lineage·parity와 백테스트 실행·지표·요약 리포트를 조회할 수 있다. 응답은 `owner_user_id`와 현재 사용자의 변경 가능 여부인 `can_manage`를 함께 반환하며, Web은 계좌 선택기를 제공하고 공유 리소스에서 바인딩·취소·robustness 실행 같은 소유자 전용 조작을 숨긴다.

DB의 FORCE RLS 소유권 정책은 완화하지 않았다. 공유 GET/조회성 compare만 기존 SELECT 전용 admin role을 사용하고, 생성·바인딩·취소·robustness 등 mutation은 계속 actor-scoped app role을 사용한다. 따라서 타인 계좌/리포트 조회는 200이지만 타인 리소스 변경은 404로 닫힌다. 원본 백테스트 artifact 다운로드, 추천 run, 알림, Paper 리밸런싱 미리보기, Live 경로는 이번 공유 범위에 포함하지 않았다.

검증 결과: API Paper 5/5, Backtest 8/8, Phase 1 5/5, Web unit 71/71, 관련 Playwright 14/14, TypeScript, production build, Biome, rustfmt, strict Clippy, OpenAPI drift/type 생성이 통과했다. 이 변경은 아직 원격 push·배포하지 않았다.

---

## 3. 최근에 고쳐진 것 (2026-08-08 ~ 08-17)

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

이 완료 판정은 저장·발행·복구·배포 **이음매**에 대한 것이다. KIS HTTP/token/provider wiring은 저장소에 있으나 production credential/endpoint, broker entitlement-aware evidence, 외부 role/secret/data-volume provisioning과 초기 backfill은 구현·조달·검증되지 않았다. 따라서 실제 KIS feed가 운영 중이라는 뜻이 아니다.

### 3.8 GitHub Actions CI (2026-08-11)

- Pull request와 `main` push는 policy, rustfmt, workspace 전체 strict Clippy, deterministic Phase 0 생성, disposable PostgreSQL, Rust workspace 전체 테스트를 각각 독립 GitHub-hosted runner에서 수행한다.
- `main` push는 별도 runner에서 기존 research-worker Docker/Compose functional smoke를 한 번 더 수행한다. 수동 `workflow_dispatch`는 지원하지만 **`schedule`/nightly 트리거는 없다.**
- 260-session Phase 0 데이터는 tracked generator에서 780개 bar로 runner 내부에 생성되고 테스트 종료와 함께 폐기된다. `data/phase0`, Rust `target/`, 테스트 결과는 artifact나 cache로 업로드하지 않는다.
- 로컬 사전 검증에서 생성 데이터로 기존 clean-checkout `job-queue --test backtest_runner` 누락을 복구했고 12/12 통과했다. QA PostgreSQL을 포함한 `cargo test --workspace --locked --no-fail-fast`는 310.4초에 실패 binary 없이 종료했고, workspace all-target/all-feature Clippy `-D warnings`도 통과했다. GitHub-hosted Linux의 실제 디스크·시간 증거는 첫 push 실행에서 확정한다.

### 3.9 Auth0 confidential client 배선 (2026-08-12, main)

| 항목 | 내용 | 커밋 |
|---|---|---|
| **문제** | `HttpOidcTransport`가 Auth0 Regular Web Application(confidential client)인데 token exchange에 `client_secret`을 보내지 않았다. ADR-0002의 "PKCE가 confidential client 인증을 대체한다"는 서술 자체가 오류였다 — PKCE는 authorization code를 보호할 뿐, 어느 애플리케이션이 code를 상환하는지는 client 인증이 증명한다 | (설계 `a164eb5`) |
| **배선** | 배포 테넌트로 일본 리전 `lagrange-station.jp.auth0.com` 선택. `AUTH0_CLIENT_SECRET_FILE`로 mount된 secret 파일을 로딩(`zeroize`, 누락·빈 파일 fail-closed)하고, PKCE S256을 유지한 채 Client Secret Post로 token exchange를 인증 | `8c22f79`, `2d4aa94` |
| **비노출 경계** | secret은 provider-neutral `TokenRequest`·browser·URL·DB·로그·Debug 출력 어디에도 들어가지 않는다. token endpoint 실패 body redaction, vendor 진단 sanitize, 잔여 secret 경로 봉쇄 | `a7c4a15`, `81c60f9`, `e39438e`, `a03d8fc` |
| **배포 계약** | Compose가 gitignored `deploy/secrets/auth0_client_secret`을 read-only mount, 비밀 아닌 설정(`AUTH0_DOMAIN`/`AUTH0_CLIENT_ID`/`AUTH0_CLIENT_SECRET_FILE`)은 env로. secret 파일 provisioning은 **Windows 호스트 기준 서술이었고, Linux 호스트에는 2026-08-17에 배치됐다**(§2.8) | `afcbb77` |

**남은 것 없음 (2026-08-17 갱신):** vendor 스위트는 실 테넌트 상대 5/5 통과했고 secret은 Linux 호스트에 배치됐으며(§2.8), phase1 게이트가 **E2 = PASS**를 발행했다(§2.10~2.11). 자격증명을 위조해 통과시키는 것은 여전히 금지다.

### 3.10 고정 ETF 추천 파이프라인 (2026-08-12 ~ 08-13, `feat/recommendation-pipeline`)

큐에만 쌓이고 소비자가 없던 추천 표면(설계 문서의 "queue-only shell")을 실제 제품 흐름으로 만들었다. 이 경로는 의도적으로 `kr-etf-core-v1.yaml`의 **고정 11개 한국 ETF**로 한정된다. 별도 후보 연구 경로의 KOSPI200/KOSDAQ150 확장은 §2.9에서 완료됐지만, 임의로 구성 가능한 범용 동적 universe는 후속 범위다. 설계·계획은 `docs/superpowers/{specs,plans}/2026-08-12-recommendation-pipeline*`.

| 항목 | 내용 | 커밋 |
|---|---|---|
| **타입별 큐 claim** | 러너가 자기 job type만 claim(`claim_next_for`) — backtest 러너가 recommendation job을 훔치지 못함 | `f752b24`, `f75c8f4` |
| **DB lineage (0026)** | `recommendation_runs`에 job/dataset lineage와 manifest sha256, items/target 유니크 제약, `auto_apply_recommendations` opt-in, worker는 좁은 `schedule_recommendation_run` 함수만 실행 가능한 최소권한. 롤백 순서·스케줄러 활성화 fencing까지 hardening | `7c4fb29`, `682155f`, `942bdad`, `e09e618` |
| **입력 attestation** | pinned immutable dataset·universe를 계산 전에 attest, curated partition identity 검증, bounded factor window, 고정 유니버스 팩터 계산은 `factor-engine` 재사용 | `46d037a`, `d5020b6`, `a74ea85`, `a67fbff`, `c2797e8` |
| **격리 Python target generator** | 기존 버전드 전략 패키지를 allow-list된 자식 프로세스로 호출(strict JSON 계약) — 전략 의미론을 Rust에 중복 구현하지 않음. 자식 lifecycle/spawn·output 경계·플랫폼 race를 6커밋에 걸쳐 봉쇄 | `cbb47b6`, `810f448`~`51f1b02` |
| **원자 발행** | items + target portfolio + provenance + terminal run 상태를 한 트랜잭션으로. 발행 idempotency 검증, attestation lock, lock 후 replay snapshot 갱신 | `0506d84`, `fd3255b`, `4210b3e`, `836330a`, `01c2a48` |
| **러너** | queued run 실행, enqueue 후 revocation 처리, production lifecycle hardening, `recommendation-runner` 바이너리 | `de26f5b`, `9078f3d`, `6f4198d` |
| **API 제출** | 원자 비동기 제출, durable idempotency replay(단, mismatch는 비권위 — 저장된 응답이 진실을 대체하지 않음), 원자적 job 용량 제한 | `b236c85`, `531f871`, `2bad7c5`, `37f994c` |
| **scheduled→Paper** | 16:30 KST 스케줄 run만, opt-in 바인딩만 자동 발행 — 수동 run과 lineage 분리 | `fd3fc5b` |
| **Web 워크플로** | 첫 실행 제출(latest가 404여도), polling, 최신 성공 결과 contract 렌더링, 실패 이력 보존, active config로 실행 제한 | `e08a30f`, `217b578`, `a8c4e6b`, `fcc812a` |

이 위에 §2.5의 리밸런싱 미리보기(08-13, `58150dd`~`dc4dd0e`)가 얹혔다. 실제 KIS 데이터·broker entitlement 없이는 production 추천이 fail-closed로 차단되는 원칙은 이 경로에도 동일하게 적용된다.

### 3.11 추천 러너 운영 (2026-08-13)

- `recommendation-runner`는 16:30 KST 기본 스케줄과 시작 시 최신 적격 종가 catch-up을 사용한다. 활성 Paper 계좌 바인딩 중 `auto_apply_recommendations=true`인 경우만 자동 요청하며, 수동 요청과 lineage를 섞지 않는다.
- Compose/systemd는 curated 데이터와 고정 11-ETF universe를 읽기 전용으로 마운트하고 worker DB password를 `_FILE`로만 받는다. broker credential은 이 서비스에 주입하지 않는다.
- healthcheck는 non-secret runtime state(재시작 시 초기화), process heartbeat, read-only DB reachability, 마지막 schedule 결과(빈 cycle 포함), queue age, BLOCKED run 수를 보고한다. synthetic 11-ETF QA smoke는 실제 배포/큐 경로 검증용일 뿐 production data가 아니다.
- 실 KIS endpoint/credential, broker data-rights/entitlement 증거, 초기 backfill/dataset pin 및 운영 provisioning은 여전히 외부 blocker다. 이들이 없으면 production recommendation은 fail-closed로 차단되어야 한다.

---

## 4. 앞으로 해야 할 일

### 한눈에 보기 — 완료된 코드 작업과 남은 항목

| # | 항목 | 누가 | 지금 시작 가능? |
|---|---|---|---|
| 1 | Paper 러너 데몬 + `daily_equity` 종가 평가 | **코드 작업** | ✅ **완료** (`8da6548`, `cf8704a`) |
| 2 | 리스크 게이트 입력 5개 배선 | **코드 작업** | ✅ **완료** (`d7d75c7`, 연구 발행 이음매 `30e2679`) |
| 3 | 고정 11-ETF 추천 파이프라인 (제출→계산→발행→화면) | **코드 작업** | ✅ **완료** (§3.10, 브랜치) |
| 4 | 추천→Paper 자동화(scheduled opt-in) + 리밸런싱 미리보기 백엔드 | **코드 작업** | ✅ **완료** (`fd3fc5b`, §2.5) |
| 5 | Auth0 confidential client 배선 | **코드 작업** | ✅ **완료** (§3.9, main) |
| 6 | `feat/recommendation-pipeline` → main 병합 | **코드 작업** | ✅ **완료** (08-13 fast-forward, §2.6) |
| 6b | 전체 게이트 재실행(E7 포함) → Linux 호스트 최초 증거 발행 | **코드/운영** | ✅ **완료** — Phase 1/2/3, failures 30개, 실제 PITR, 종합 F3 재실행(§2.12) |
| 7 | 리밸런싱 미리보기 UI (백엔드만 완료, UI 미포함) | **코드 작업** | ▶️ 착수 가능 |
| 8 | paper-runner·recommendation-runner 배포 서비스 활성화 | **운영자** | ◐ 배포 계약/preflight 완료, 운영 secret·volume·systemd 설치 대기(§2.12) |
| 9 | Auth0 vendor 스위트 실제 실행 → E2 증거 갱신 | **운영자** | ✅ **완료** — 스위트 5/5 통과(§2.8), phase1 게이트가 **E2 PASS**를 발행(§2.10) |
| 10 | phase-0 골든에 수수료 필드 추가 재승인 | **사장님 결정** | ⛔ 동일 |
| 11 | KIS broker entitlement·실제 provider endpoint/credential / KIS 실계좌 | **외부 조달·운영자 provisioning** | ⛔ 현재 저장소만으로 완료 불가 |
| 12 | KOSPI200/KOSDAQ150 개별주식 후보 연구 vertical | **코드 작업** | ✅ **완료·독립 리뷰 OK** (§2.9, `ac97970`~`8c5ef9d`) |

Paper 엔진·추천 파이프라인·Paper 연계와 multi-universe 후보 연구의 저장소 내부 이음매는 완료됐다. KIS provider wiring은 완료됐지만 production credential/endpoint/권리/운영 backfill/dataset pin은 외부 잔여 작업이다. **전체 게이트 재실행도 완료됐으므로(§2.12), 다음 순서는 외부 조달과 운영 호스트 provisioning이다. 권리·자격증명 없이 Member/Live를 활성화하지 않는다.**

### 4.1 소유자만 할 수 있는 것 — 외부 조달

| 항목 | 구체적으로 |
|---|---|
| **E1** KIS broker data-rights/entitlement + 실제 공급자 | 초대 사용자 5명 + 파생 분석물을 포괄하는 실제 계약/사용허가 metadata, KIS HTTP/token/provider wiring, 실제 endpoint/credential, `research_writer` role·secret·Raw volume 운영자 provisioning과 초기 backfill/dataset pin. 현재 저장소에는 synthetic fixture와 wiring/발행 이음매만 있고 real feed는 live가 아님. `configs/data-rights/`는 여전히 placeholder뿐 |
| **E2** Auth0 테넌트 | **해소(08-17):** 테넌트 선택·confidential client 배선(08-12, §3.9), Linux 호스트 secret 배치와 실 테넌트 vendor 스위트 5/5 통과(§2.8)에 이어, phase1 게이트가 **E2 = PASS**를 발행했다(§2.10). 이 항목은 더 이상 외부 조달 대기가 아니다 |
| **X1/X2** KIS 실계좌 | 실거래 자격증명 + 소액 실주문 증거 (변동 없음) |

이 3건이 없는 동안 게이트는 `BLOCKED_EXTERNAL`이며 **그것이 합격 조건이다. 위조해서 APPROVED에 도달하는 것은 금지** — 닫히는 쪽으로 실패하는 것 자체가 게이트의 존재 이유다. 한국 데이터 권리가 끝내 안 오면 계획의 답은 Member 접근 연기다, 시장 변경이 아니라.

### 4.2 소유자 결정 대기 2건

1. **phase-0 골든에 수수료 필드를 넣을지** — 넣는 것은 승인된 기준값을 바꾸는 명시적 재승인 행위라 보류 중
2. **Phase 4 우선순위** — §4.4 참조

### 4.3 코드 작업 — 착수 가능, 권장 순서

1. ~~**`phase1-gate.sh` native Linux 이식.**~~ **완료 (2026-08-17, `5b3f832`, §2.10)** — WSL 가드·PATH·`CARGO_TARGET_DIR`·DB 포트를 정리했고, 이식 과정에서 드러난 거짓 PASS 3건도 함께 닫았다. pyarrow 전제는 이 게이트에는 해당하지 않는다(추천 계산 경로의 문제이며 phase1 검사는 `prepare_phase0.py`를 부르지 않는다).
2. ~~**전체 게이트 재실행.**~~ **완료 (2026-08-17, `61af2bb`, §2.12)** — Phase 1/2/3, 양 단계 failures, 실제 PITR, 종합 F3를 재실행했다. 내부 검사는 모두 통과했고 최종 판정은 외부 권리·실계좌 때문에 `BLOCKED_EXTERNAL`이다. F1/F2/F4 판정문은 여전히 사람 재검토가 필요하다.
3. **리밸런싱 미리보기 UI.** §2.5의 백엔드 계약은 완료됐지만 화면과 Live 주문은 범위 밖이다.
4. **배포 서비스 활성화.** Paper/recommendation/candidate runner에 실제 role-scoped DB URL과 curated/raw volume을 호스트 Secret Manager에서 주입한다. 저장소에는 비밀값을 넣지 않는다.
5. **실제 KIS provider와 운영 원천 활성화.** KIS credential/token/endpoint를 운영 secret으로 provisioning하고 broker entitlement metadata, `research_writer`, migration, Raw volume을 검증한 뒤 KIS calendar/EOD/instrument/corporate-action 원천을 공급한다. 고정 ETF 백필 후 후보 bridge와 KOSPI200/KOSDAQ150 source set은 별도 승인한다. 원천이 없거나 오래되면 게이트는 계속 닫힌다.

**작지만 기록해 둘 잔여 항목** (아키텍트 검토에서 발견, 차단 아님): `strategy_promotion`(§3.5)이 계좌 단위라 그 계좌에 묶인 주문 전부를 승격된 것으로 본다 — 운영 원천이 채워져 결정적 검사가 되기 전에 재검토할 것. 이전에 기록한 `positions` 소유자 재확인 gap은 0038의 account-owner 복합 FK로 닫혔다.

### 4.4 Phase 4 잔여

| 항목 | 상태 | 선행 조건 |
|---|---|---|
| PIT 재무 팩터 | **골격 완료** — 채우기만 남음 | 한국 재무 데이터 공급원 + 계약 |
| KOSPI200/KOSDAQ150 후보 universe | **코드·QA vertical 완료** (§2.9), 실제 feed 미활성 | KRX 개별종목 데이터 권리 + 실제 provider/credential |
| 임의 구성 범용 동적 universe | 미착수 | universe registry 운영 정책 + 데이터 권리 |
| 분봉·틱·호가 | 미착수 (`Cadence::parse("intraday")`는 현재 타입 오류로 거부 — 의도) | 시세 권리 + 신규 수집 파이프라인 |
| LEAN 연구 워커 | 평가 대상, **트리거 미충족** | 설계 §1263: 미국 Fundamental·LEAN Universe 확보 시에만. 현재 조건 0/2 |
| 순수 Rust NT 경로 확대 | 미착수 | 없음 — 지금 가능하나 사용자 가시 기능 아님 |

**데이터 권리 없이 앞 셋의 코드를 쓰면 검증 불가능한 코드가 된다** — 착수하지 않은 이유.

---

## 5. 개발 환경 주의사항 (반복 비용을 치른 것들)

**이 절은 2026-08-14 Linux 이관 이전의 Windows/WSL 환경 기록이다.** 함정 자체는 그 환경에서 실제로 치른 비용이므로 남겨두지만, 현재 Linux 호스트의 절차는 `docs/LINUX_MIGRATION_AND_OPERATIONS.md`를 따른다. Windows 원본은 런북 §12 체크리스트가 끝날 때까지 읽기 전용으로 보존한다.

**서비스가 실행되는 호스트는 native Linux 하나다(§2.8).** 따라서 아래 표에서 "Windows에서는 `.ps1` 쌍둥이를 쓴다"로 적힌 대응은 **역사 기록이지 현재 선택지가 아니다.** WSL 전용 스크립트는 이식 대상이다.

| 함정 | 대응 |
|---|---|
| WSL이 유휴 시 VM을 종료 → Postgres가 테스트 중 내려감 (`PoolTimedOut`) | 긴 스위트 전 `wsl -d Ubuntu -- sh -c 'sleep 3000'` 백그라운드 실행 |
| `command -v docker`는 **엔진이 죽어도 성공** | `docker version --format '{{.Server.Version}}'`으로 확인. phase2/3 게이트는 Docker QA DB(127.0.0.1:55432) 필수 |
| `phase1-gate.sh`는 WSL 전용 | 당시 대응은 Windows의 `phase1-gate.ps1` (이제 .sh가 스스로 거부한다). **현재는 Linux 단일 호스트라 이 우회가 없다 — 이식 필요(§2.8)** |
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
