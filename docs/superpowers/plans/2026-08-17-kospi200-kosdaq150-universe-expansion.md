# KOSPI200·KOSDAQ150 복수 유니버스 확장 구현 계획

> Claude Code Fable 5 `xhigh`가 작성한 아키텍처·에이전트 계획을 현재 저장소의 실제
> SQL/Rust/API/Web 계약과 대조하여 교정한 실행 계획이다. 이 문서는 구현 계획이며,
> migrations `0042`~`0044` 또는 제품 코드를 수정하지 않는다.

**목표:** 현재 KOSPI200 전용 stock-research candidate vertical을
KOSPI200과 KOSDAQ150의 두 개 point-in-time 유니버스로 확장하고, 유니버스별
일일 Top 5와 하나/둘 모두를 조회할 수 있는 스크리너를 제공한다.

**핵심 구조:** `0045`가 유니버스 registry와 유니버스별 run/feed identity를
추가한다. 기존 generic `candidate_universe_snapshots.index_id`와
`candidate_universe_members`를 재사용하며 별도 KOSDAQ membership 테이블은 만들지
않는다. 하나의 immutable source Raw batch가 공통 source 4종과 유니버스별 membership
dataset 2종을 각각 sealed binding으로 보존한다. 단일 research-worker와 단일
candidate-runner가 활성 유니버스를 결정론적 순서로 처리한다.

**기술 스택:** Rust 1.97.1, SQLx 0.9, PostgreSQL 18, Axum 0.8, Next.js 16,
기존 immutable Raw/Curated 저장소, 기존 Docker Compose/CI.

---

## 1. 목표·비목표·최종 UX/API

### 1.1 확정 목표

- 1차 지원 유니버스는 `kospi200`, `kosdaq150` 두 개다.
- 매 거래일에 유니버스별로 독립된 분석 run과 Top 5 feed를 발행한다.
- `/candidates`는 유니버스 탭을 제공하며 한 번에 선택한 유니버스의 Top 5를
  보여준다.
- `/screener`는 한 유니버스 또는 둘 모두를 선택할 수 있다.
- 같은 종목이 두 유니버스에 속하면 스크리너에서 유니버스별 분석 맥락을 보존한
  두 행으로 표시한다. dedupe하지 않는다.
- 점수, winsorization, normalization, rank는 항상 한 run, 즉 한 유니버스 안에서만
  계산한다. 서로 다른 유니버스의 score/rank는 비교 가능한 값으로 취급하지 않는다.
- 60 거래세션 미만 종목은 기존 `INSUFFICIENT_PRICE_HISTORY` typed exclusion을 가진
  snapshot으로 남고 Top 5 및 eligible screener 결과에서는 제외한다.
- universe를 생략한 기존 요청은 `kospi200`으로 해석해 하위 호환을 유지한다.

### 1.2 비목표

- 두 유니버스를 합쳐 만든 전체 시장 Top 5 또는 전역 rank
- cross-universe score 재정규화, 가중합, dedupe 우선순위
- 확률 예측, 기대수익률, 목표가, 매수/매도 권고
- universe별 별도 runner/worker 서비스 또는 새 데이터베이스
- 실 KRX credentialed transport와 실 자격증명 연결
- migrations `0042`~`0044` 파일 수정 또는 checksum 변경
- 임의 N-universe 등록 UI, 동적 지수 생성기, intraday 후보 추천

### 1.3 최종 API 계약

- `GET /api/v1/candidates/feed/latest?universe=kospi200|kosdaq150`
  - query 생략 시 `kospi200`.
  - 기존 단일 feed 응답 shape를 유지하고 `universe`를 additive field로 추가한다.
  - 여러 universe를 한 응답에 합치지 않는다. Web은 선택한 탭을 별도 조회한다.
- `GET /api/v1/candidates/feed/{date}?universe=...`
  - 해당 날짜·유니버스의 최신 correction sequence를 선택한다.
- `GET /api/v1/stocks/{instrument_id}/analysis?date=...&universe=...`
  - universe 생략 시 `kospi200`; 다른 유니버스로 자동 fallback하지 않는다.
- `POST /api/v1/screener/query`
  - `ScreenCriteria.universes?: ["kospi200"|"kosdaq150"]` 추가.
  - 생략 시 `["kospi200"]`; 중복/빈 배열/알 수 없는 값은 `400`.
  - 하나 또는 둘 모두 허용. 결과는 flat list를 유지하되 각 item에 `universe`와
    해당 `run_id`를 포함한다.
  - 둘 모두일 때 안정 정렬은 `(universe_order ASC, total_score DESC,
    instrument_id ASC)`이다. `universe_order`는 registry의 고정 `sort_order`이며
    score로 두 유니버스를 interleave하지 않는다.
- saved screen v1은 universe 부재를 KOSPI200으로 해석한다. 새 create/update는
  `criteria_schema_version=2`와 명시적 `universes`를 저장한다.

---

## 2. 현재 하드코드와 일반화 전략

| 현재 위치 | 현재 가정 | 0045 이후 전략 |
|---|---|---|
| `migrations/0042_candidate_source_contracts.up.sql` | `index_membership -> krx_kospi200_membership`; universe publisher가 `kospi200` literal 사용 | 0042는 불변. 0045에서 함수 본문을 동적 registry lookup으로 교체하고 기존 signature는 유지한다. |
| `candidate_raw_batch_datasets` | 한 batch에 response kind당 한 row | `dataset_id` identity를 추가해 같은 `index_membership` kind 아래 두 membership dataset을 허용한다. |
| entitlement resolver | 공통 5 source + price 중 KOSPI membership만 요구 | 활성 registry의 membership dataset 전체와 공통 dataset을 한 exact contract가 덮는지 검사한다. |
| `candidate_sink.rs` | `UNIVERSE_DATASET=krx_kospi200_membership`, 문서의 index가 KOSPI200 하나여야 함 | `IndexMembershipDocument`를 canonical index별 partition으로 나누고 각 registry dataset에 publish한다. |
| `candidate_pipeline.rs` | response kind 하나에서 membership dataset 하나 생성 | binding key를 `(response_kind,dataset_id)`로 바꾸고 membership partition별 canonical hash/version을 생성한다. |
| `worker.rs` | KOSPI membership 하나의 completeness/health | 활성 유니버스별 source completeness와 current session health를 반환한다. |
| `schedule.rs` | `WHERE index_id='kospi200'` 최신 snapshot 하나 | registry `sort_order` 순서로 활성 유니버스 각각의 exact snapshot을 선택한다. |
| `schedule_candidate_run` | membership dataset id를 KOSPI로 고정 | snapshot의 `index_id`와 registry dataset을 결속하고 run identity에 universe를 포함한다. |
| `publish_candidate_analysis` | 같은 날짜의 기존 feed 전부 supersede | 같은 `(universe, as_of_date)` feed만 supersede한다. |
| `stock_analysis_runs` | `UNIQUE(as_of_date, computation_seq)` | `UNIQUE(universe_key, as_of_date, computation_seq)`. |
| `candidate_feed_snapshots` | date/sequence 및 active feed가 날짜 단독 identity | 두 unique/index 모두 `universe_key`를 포함한다. |
| API repository | 최신 단일 run/feed | 요청 universe별 최신 run/feed; screener는 frozen run-set. |
| `ScreenCursor` v1 | 단일 `run_id` | v2는 정렬된 `(universe,run_id)` run-set과 last universe를 서명한다. |
| Web | universe selector 없음 | candidate tab, screener multi-select, 행별 universe badge. |

`krx_kospi200_membership`은 테이블 이름이 아니라 logical dataset ID다. 새 별도
membership 테이블을 만들지 않고, 기존 generic source tables에
`index_id='kosdaq150'` snapshot/member를 함께 저장한다.

---

## 3. Migration 0045 up/down 상세

### 3.1 파일

- Create: `migrations/0045_candidate_multi_universe.up.sql`
- Create: `migrations/0045_candidate_multi_universe.down.sql`
- Modify after DB agent handoff, coordinator only:
  `tests/integration/migration-contract/tests/migration_contract.rs`
- Modify after DB agent handoff, coordinator only:
  `deploy/compose/research-schema-check.sql`
- Modify after DB agent handoff, coordinator only:
  `deploy/compose/candidate-static-check.sh`

### 3.2 Registry

`candidate_universe_registry`를 생성한다.

```sql
CREATE TABLE public.candidate_universe_registry (
    universe_key          text PRIMARY KEY,
    membership_dataset_id text NOT NULL UNIQUE,
    display_name          text NOT NULL,
    market                text NOT NULL CHECK (market = 'kr'),
    sort_order            integer NOT NULL UNIQUE CHECK (sort_order > 0),
    enabled               boolean NOT NULL,
    created_at            timestamptz NOT NULL DEFAULT clock_timestamp(),
    CHECK (universe_key ~ '^[a-z0-9][a-z0-9._-]{0,63}$'),
    CHECK (membership_dataset_id ~ '^krx_[a-z0-9_]+_membership$')
);
```

고정 seed:

- `kospi200 / krx_kospi200_membership / KOSPI 200 / sort_order=10`
- `kosdaq150 / krx_kosdaq150_membership / KOSDAQ 150 / sort_order=20`

최종 release migration에서는 두 행을 `enabled=true`로 둔다. 배포 전환 중 임시
disable이 필요하면 serving role DML을 열지 말고 migration-owner 운영 절차로만 바꾼다.
테이블은 `ENABLE/FORCE RLS`; `research_writer`, `worker`, `app`, `admin`에 필요한
SELECT만 부여하고 INSERT/UPDATE/DELETE는 부여하지 않는다.

### 3.3 Raw binding identity 확장

현재 PK `(batch_id,surface,response_kind)`는 membership dataset 두 개를 표현하지
못한다. 0045 up은 다음 순서로 바꾼다.

1. `candidate_raw_batch_datasets.dataset_id text` nullable 추가.
2. `dataset_versions` join으로 기존 모든 row를 exact backfill.
3. `dataset_id SET NOT NULL` 및 문법 CHECK 추가.
4. 기존 PK를 drop하고 `(batch_id,surface,dataset_id)` PK 생성.
5. `(batch_id,surface,response_kind,dataset_id)` unique 추가.
6. trigger/SECURITY DEFINER에서 `dataset_version_id`의 실제 dataset id와
   denormalized `dataset_id`가 동일함을 검증한다.

공통 source batch의 expected bindings는 다음 6개다.

```text
investor_flow        -> krx_investor_flows
market_status        -> krx_market_status
fundamentals         -> krx_fundamentals
sector_classification-> krx_sector_classification
index_membership     -> krx_kospi200_membership
index_membership     -> krx_kosdaq150_membership
```

price surface의 `bars -> krx_eod_bars` 계약은 그대로다. 동일 response kind의 두
membership row가 허용돼도 다른 kind의 복수 dataset은 거부한다.

### 3.4 Run/feed universe identity

1. `candidate_universe_snapshots.index_id`에 registry FK를 추가하고 기존 KOSPI rows를
   검증한다.
2. `(id,index_id)` unique를 추가한다.
3. `stock_analysis_runs.universe_key text`를 추가하고 snapshot join으로 backfill한다.
4. `NOT NULL`, registry FK, `(id,universe_key)` unique를 추가한다.
5. 기존 `stock_analysis_run_date_seq_key`를
   `UNIQUE(universe_key,as_of_date,computation_seq)`로 교체한다.
6. `(universe_snapshot_id,universe_key)` composite FK로 run과 exact snapshot index를
   결속한다.
7. `candidate_feed_snapshots.universe_key text`를 run join으로 backfill하고 NOT NULL로
   만든다.
8. feed의 run FK를 `(run_id,universe_key)` composite FK로 강화한다.
9. `candidate_feed_date_seq_key`를
   `UNIQUE(universe_key,as_of_date,computation_seq)`로 교체한다.
10. `candidate_feed_active_date_uq`를
    `UNIQUE(universe_key,as_of_date) WHERE status='PUBLISHED'`로 교체한다.
11. latest indexes를 `(universe_key,as_of_date DESC,computation_seq DESC)`로 교체한다.

`universe_key DEFAULT 'kospi200'`는 migration/backfill 중에만 사용하고 최종 schema에서는
default를 제거한다. 신규 코드가 universe를 명시하지 않으면 DB가 조용히 KOSPI로
기록해서는 안 된다.

### 3.5 0045가 교체하는 기존 함수

0042~0044 파일은 건드리지 않되 0045 up에서 `CREATE OR REPLACE FUNCTION` 또는 같은
signature의 drop/recreate로 아래 body를 현재 정의에서 파생해 교체한다.

- `resolve_candidate_contract_entitlement(text,date,date)`
  - 공통 dataset 5개(price 포함)와 enabled registry membership dataset 전부를 exact
    candidate-use contract 하나가 덮어야 한다.
- `register_candidate_source_dataset(text,text,text,uuid,text,date)`
  - 고정 allowlist + registry membership dataset만 허용한다.
- `bind_candidate_raw_dataset(uuid,text,text,uuid,boolean)`
  - dataset id를 version row에서 파생하고 새 binding identity로 exact replay한다.
- `seal_candidate_raw_batch(uuid,text,text,text)`
  - enabled universe 각각에 non-empty, count-consistent snapshot/member set이 있어야
    source batch를 `PUBLISHED`로 전환한다.
- `block_candidate_raw_batch_for_inactive_rights(...)`
  - 현재 enabled universe dataset 전체의 rights를 검사한다.
- `candidate_source_dataset_write_matches(...)`
  - membership dataset을 registry로 검증한다.
- `insert_candidate_universe_snapshot(...)`
  - `dataset_version_id -> dataset_id -> universe_key`를 DB에서 파생한다. Rust가 보낸
    문자열만 신뢰하지 않는다.
- `candidate_source_validate_dataset_pin()`
  - universe source row의 expected dataset을 registry로 결정한다.
- `stock_analysis_validate_lineage()`
  - run `universe_key`와 snapshot `index_id`가 같아야 한다.
- `schedule_candidate_run(...)`
  - signature는 유지한다. snapshot에서 universe를 파생하고 enabled registry,
    membership dataset, entitlement, sealed fetch mode, 60-session viability를 재검증한다.
  - advisory key, `input_identity_sha256`, idempotency key, sequence allocation에 universe를
    포함한다.
- `publish_candidate_analysis(uuid,uuid,integer,text,jsonb,jsonb)`
  - run universe를 재검증하고 같은 universe/date의 feed만 supersede한다.
  - feed insert에 universe를 명시한다.

각 SECURITY DEFINER는 기존처럼 owner=`migration_owner`, 고정 search_path,
PUBLIC/serving-role revoke 후 필요한 role에만 EXECUTE를 부여한다. 기존 함수 signature를
유지해 롤링 호환은 확보하되, 새 semantics를 우회하는 legacy overload를 만들지 않는다.

### 3.6 Down migration과 rollback guard

Down은 먼저 다음 중 하나라도 있으면 `55000`으로 중단한다.

- `index_id='kosdaq150'` source snapshot/member
- `universe_key='kosdaq150'` run/feed/snapshot 참조
- KOSDAQ dataset binding 또는 published raw batch
- `criteria_schema_version >= 2`이면서 KOSDAQ을 포함한 saved screen
- 진행 중 candidate job/run 또는 0045 identity를 잃을 published correction history

guard를 통과한 경우에만 역순으로:

1. scheduler를 advisory lock으로 멈춘다.
2. 0042/0043/0044 당시 함수 body, owner, grants를 정확히 복원한다.
3. feed/run unique/index/FK를 원형으로 복구한다.
4. universe columns/keys를 제거한다.
5. raw binding이 response kind당 정확히 하나인지 확인하고 기존 PK를 복구한다.
6. registry 정책·테이블을 제거한다.
7. scheduler를 원래 상태로 복구한다.

Down SQL은 `0042`~`0044`의 원본 정의를 복사해 복구해야 하며 “비슷한” 축약 정의를
작성하지 않는다. live DB에서 up→no-op rerun→guarded down→up을 실행한다.

---

## 4. Raw ingest·typed publish·recovery·health·compute

### 4.1 Source/Raw

- `CandidateUniverseKey` enum을 `crates/market-data/src/candidate.rs`에 둔다.
  canonical string과 registry key는 `kospi200`, `kosdaq150` 두 개뿐이다.
- `IndexMembershipDocument`는 두 index의 row를 담을 수 있다. parsing 단계는 각 row의
  enum 유효성을 검사하고 natural-key duplicate를 fail closed한다.
- 한 source Raw batch에서 pagination을 기존처럼 파일명 순으로 병합한 뒤 membership
  문서를 universe별로 canonical partition한다.
- 공통 4 source는 한 번만 catalog/publish하고, membership partition은 서로 다른
  `dataset_id`, version, manifest hash로 catalog한다.
- partition manifest hash는 canonical typed bytes로 만들고 전체 Raw manifest hash 및
  exact file request/provenance를 raw ledger에 보존한다. 같은 bytes를 다른 universe로
  재해석할 수 없도록 partition의 `index_id`를 hash input에 포함한다.
- 현 단계의 synthetic fixture는 membership 두 종류를 제공한다. production
  `fetch_mode=credentialed`는 실제 transport/credential이 없으면 기존처럼 실패한다.

### 4.2 Typed publication과 recovery

- `CandidateDatasetBinding` identity를 `ResponseKind` 단독에서
  `(ResponseKind,dataset_id)`로 바꾼다.
- `candidate_sink.rs`의 단일 `UNIVERSE_DATASET` 상수를 제거한다.
- `catalog_candidate_batch`는 registry/entitlement를 조회해 공통 4 + membership 2의
  exact binding을 반환한다.
- `publish_candidate_batch`는 공통 observations를 한 번 publish하고 membership
  partition마다 `insert_candidate_universe_snapshot`과 member rows를 publish한다.
- exact replay는 batch id, Raw manifest hash, fetch mode, entitlement reference/date,
  response kind, dataset id, dataset version id가 모두 같을 때만 skip한다.
- 이미 PUBLISHED인 역사 batch는 현재 entitlement 재해석 전에 exact terminal identity로
  skip한다. CATALOGED/BLOCKED mismatch는 신규 ingest를 진행하지 않고 typed error로
  표면화한다.
- 기존 PIT 재사용, immutable flow snapshot membership, restatement, no-lookahead 보장을
  공통 source와 두 membership 모두 유지한다.

### 4.3 Health/readiness

- research-worker health는 enabled universe별 상태를 기록한다.
- 한 process의 liveness와 데이터 readiness를 분리한다.
  - process가 진행 중이면 liveness는 true.
  - 최신 confirmed KRX close에 대해 어느 enabled universe라도 source/price/rights가
    부족하면 전체 readiness는 false이며 `per_universe` 원인을 제공한다.
- pre-close에는 이전 confirmed session을, 16:30 Asia/Seoul 이후 오늘이 TRADING이면
  오늘 session을 기대한다. 이전 feed로 READY를 가장하지 않는다.
- price/flow 60-session 최소 5 eligible rule은 각 universe member 집합에 독립 적용한다.

### 4.4 Scheduler/input/runner/publication

- `schedule_latest_candidate_run`을 내부 단일-universe helper로 남기고 새
  `schedule_latest_candidate_runs`가 registry `sort_order` 순으로 호출한다.
- candidate-runner의 한 schedule tick이 두 run을 enqueue할 수 있지만 queue drain과
  compute는 기존 단일 worker/lease 흐름을 사용한다.
- 한 universe의 SourceUnavailable/Blocked는 다른 universe의 이미 성공한 feed를
  supersede하지 않는다. readiness에는 실패 universe를 명시한다.
- `CandidatePayload`와 `CandidateRunInput`에 `universe_key`를 추가하고 DB run/snapshot과
  exact 일치시킨다.
- factor-engine는 이미 한 run의 frozen input만 정규화하므로 수식은 바꾸지 않는다.
  회귀 테스트로 두 universe의 score distribution이 섞이지 않는지만 증명한다.
- 기존 `CandidateExclusion::InsufficientPriceHistory`를 재사용한다. 새 의미가 같은 enum을
  추가하지 않는다.
- publication replay, correction, lease settlement는 universe-scoped identity를 사용한다.

---

## 5. API·OpenAPI·Web·cursor·saved screen

### 5.1 API repository와 handlers

- `CandidateRunRow`, `CandidateFeedRow`에 `universe_key` 추가.
- `latest_feed`, `instrument_analysis`, `screen` SQL은 universe를 명시해야 한다.
- feed/stock API의 omitted universe만 KOSPI200 default다. 명시한 KOSDAQ run이 없으면
  KOSPI로 fallback하지 않고 기존 typed `RESOURCE_NOT_FOUND`/empty-state 계약을 따른다.
- screener에서 legacy `run_id`를 보내면 정확히 한 universe만 허용한다. 둘 모두 선택한
  요청과 `run_id`를 함께 보내면 `INVALID_PARAMETER`다.

### 5.2 ScreenCursor v2

```text
cursor_version = 2
run_set = [(universe_key, run_id)]  // registry sort order의 고정 배열
criteria_sha256                     // universes 정규화 결과 포함
after_universe
after_score                         // PostgreSQL numeric canonical text
after_instrument
```

- HMAC payload에 run-set 전체를 포함한다.
- 다음 페이지는 새 correction feed가 나와도 cursor의 frozen runs만 조회한다.
- 요청 universe 집합, criteria hash, run-set, last key 중 하나라도 다르면 거부한다.
- v1 cursor는 universe가 생략된 legacy KOSPI200 요청에서만 허용한다.
- 새 cursor는 항상 v2로 발급한다.
- 둘 모두 결과는 universe block 순서를 먼저 사용해 cross-universe score 비교를 암시하지
  않는다.

### 5.3 Saved screen

- `ScreenCriteria`에 serde default 함수로 `[Kospi200]`을 제공한다.
- DB v1 row는 읽을 때만 KOSPI default로 해석하고 자동 mutation하지 않는다.
- 새 create/update는 version 2와 canonical explicit `universes`를 저장한다.
- v2 criteria hash에는 중복 제거 후 registry sort order로 정렬한 universe 배열을 넣는다.
- v1 row update 시 v2로 승격한다.

### 5.4 OpenAPI

- Authored source는 `apps/api-server/scripts/openapi-spec.mjs`다.
- Rust route inventory는 `crates/api-server/src/contract.rs`와 일치시킨다.
- `UniverseKey`, universe query parameter, `ScreenCriteria.universes`, response item universe를
  schema에 추가한다.
- coordinator만 `npm run openapi:check --workspace @lagrange/api-server`를 실행해
  `apps/api-server/openapi.json`과 `apps/api-server/generated/openapi.ts`를 재생성한다.

### 5.5 Web

- `/candidates`: query/search param 기반 universe tab; 기본 KOSPI200.
- `/screener`: 두 checkbox/multi-select, universe별 그룹, 행별 badge. 통합 rank label 금지.
- `/stocks/[instrument]`: 선택 universe를 query에 보존하고 badge/해당 rank 표시.
- saved screens: v1 기본값과 v2 명시 선택 모두 처리.
- API BLOCKED/STALE/NOT_FOUND를 빈 READY 결과로 바꾸지 않는다.
- Web 카피에는 기존 disclaimer를 유지하고 확률·목표가·buy/sell 문구를 추가하지 않는다.

---

## 6. Normalization·identity·idempotency·correction

- instrument identity는 전역으로 하나이며 membership만 universe별 관계다.
- analysis snapshot identity는 기존 `(run_id,instrument_id)`를 유지한다. run이 정확한
  universe를 내포한다.
- run correction sequence는 `(universe_key,as_of_date)` 범위다.
- active feed는 `(universe_key,as_of_date)`당 하나다.
- schedule identity/hash에는 universe key, universe snapshot/entitlement, 기존 5 source pin,
  cutoff/config를 모두 포함한다.
- 동일 source pins와 universe의 동시 schedule은 같은 run/job을 반환한다.
- 같은 날짜의 KOSDAQ correction은 KOSPI feed/sequence를 변경하지 않는다.
- publication은 모든 member snapshot, eligible count, Top 5, queue settlement를 기존처럼
  한 transaction에서 처리한다.
- sealed source 또는 SUCCEEDED analysis를 UPDATE해 correction하지 않는다. 새 dataset/run
  sequence만 허용한다.

---

## 7. 호환성·배포·롤백·단일 서버 자원

### 7.1 호환성

- API universe 생략, saved screen v1, cursor v1은 KOSPI200 의미를 유지한다.
- DB 함수 public signatures는 유지하되 0045 body가 universe를 DB에서 파생한다.
- 0042~0044 file hashes/checksums는 변하지 않는다.
- 기존 KOSPI rows는 FK/constraint 전환 전에 정확히 backfill하고 live test에서 row/hash를
  비교한다.

### 7.2 배포 순서

1. 현재 0042~0044 snapshot/checksum 및 candidate test baseline 저장.
2. 새 binary/images를 build하되 시작하지 않는다.
3. candidate scheduler/worker를 짧게 drain한다.
4. 0045 migration을 적용한다.
5. research schema gate를 통과시킨다.
6. research-worker와 candidate-runner를 기동한다.
7. KOSPI200 exact replay/READY가 유지되는지 먼저 확인한다.
8. 두 membership이 포함된 synthetic/QA source batch와 두 run을 검증한다.
9. API/Web smoke를 실행한다.

실 KRX credentialed transport가 없으므로 실제 production 데이터 READY는 이 범위에서
주장하지 않는다. contract와 QA path만 완성하며 production은 credential/rights/data가
없으면 fail closed한다.

### 7.3 rollback

- 1차: migration_owner 운영 절차로 KOSDAQ registry를 disable하고 KOSPI processing 유지.
- 2차: 새 코드 rollback. KOSDAQ data가 존재하면 schema는 유지한다.
- 3차: 0045 down. guard가 모두 통과할 때만 수행한다.
- published KOSDAQ data를 자동 delete하거나 KOSPI로 재분류하지 않는다.

### 7.4 자원 정책

- 새 Compose service, queue type, database를 추가하지 않는다.
- source batch 공통 4종은 한 번만 publish한다.
- 두 compute run은 순차 실행해 peak memory를 기존 한 universe 수준에 가깝게 유지한다.
- scheduler loop에서 universe별 bounded timeout/backoff를 사용한다.
- p95 query/runner duration과 DB row 증가를 측정한 뒤에만 parallel runner나 cache를
  검토한다.

---

## 8. 서브에이전트별 작업 명세

공통 규칙:

- coordinator를 포함해 동시 슬롯 4개, 즉 worker는 최대 3개다.
- coordinator 모델은 `gpt-5.6-sol`, reasoning effort는 `xhigh`로 고정한다.
- DB/Contracts, Source/Ingest, Compute, API, Web, Verification/Reviewer를 포함한 모든
  worker 모델은 `gpt-5.6-luna`, reasoning effort는 `max`로 고정한다.
- agent 생성 시 위 model/effort override를 명시하며, coordinator 승인 없이 모델을
  변경하거나 낮은 reasoning effort로 재실행하지 않는다.
- 같은 파일은 같은 wave에서 두 agent가 수정하지 않는다.
- `Cargo.lock`, `.github/workflows/ci.yml`, `deploy/compose/compose.yml`,
  `deploy/compose/.env.example`, `apps/api-server/openapi.json`,
  `apps/api-server/generated/openapi.ts`, migration-contract의 최종 편집은 coordinator만 한다.
- 기존 테스트를 약화하거나 skip으로 바꾸는 수정은 금지한다.
- 모든 agent는 RED test → 최소 구현 → GREEN → 소유 파일 diff 보고 순서로 작업한다.

### Agent DB/Contracts — “Atlas”

**소유 파일**

- `migrations/0045_candidate_multi_universe.up.sql`
- `migrations/0045_candidate_multi_universe.down.sql`
- Wave 1 동안만
  `tests/integration/migration-contract/tests/migration_contract.rs`; 완료 후 coordinator에게
  소유권 반환

**금지 파일**

- `migrations/0042_*`, `0043_*`, `0044_*`
- Rust feature code, Web, Compose, OpenAPI generated, Cargo.lock, CI

**선행조건**

- coordinator가 baseline checksum/constraint/function signature를 기록한다.
- disposable PostgreSQL 18 `DATABASE_URL`이 준비된다.

**RED**

- 0045 부재, KOSDAQ binding 불가, 같은 날짜 두 universe feed 불가를 각각 실패로 확인.
- direct serving-role registry DML, wrong dataset/universe pairing, cross-universe supersede,
  concurrent schedule collision, down with KOSDAQ rows가 거부되는 live test를 먼저 추가.

**구현 산출물**

- §3의 schema/function/grant/rollback guard 전체.
- exact function owner/search_path/execute matrix.
- KOSPI backfill hash/identity 보존 증거.

**GREEN**

```bash
DATABASE_URL="$DATABASE_URL" cargo test -p migration-contract \
  --test migration_contract candidate_multi_universe -- --nocapture --test-threads=1
DATABASE_URL="$DATABASE_URL" cargo test -p migration-contract \
  --test migration_contract -- --nocapture --test-threads=1
cargo fmt --all -- --check
cargo clippy -p migration-contract --all-targets --all-features --locked -- -D warnings
```

**fail-closed 기대**

- unknown/disabled universe, wrong dataset binding, inactive entitlement, unsealed batch,
  cross-universe feed mutation은 SQLSTATE `23514`, `42501`, 또는 `55000`의 기존 분류로
  중단하고 어떤 queue/feed row도 남기지 않는다.

**checkpoint**

- `feat(db): add multi-universe candidate boundary`

### Agent Source/Ingest — “Kepler”

**소유 파일**

- `crates/market-data/src/candidate.rs`
- `crates/market-data/tests/candidate_ingestion.rs`
- `data-pipelines/collectors/src/candidate_pipeline.rs`
- `data-pipelines/collectors/src/candidate_sink.rs`
- `data-pipelines/collectors/src/worker.rs`
- `data-pipelines/collectors/src/bin/research-worker.rs`
- `data-pipelines/collectors/tests/research_worker.rs`
- `data-pipelines/collectors/tests/candidate_catalog.rs`
- `tests/fixtures/kr-candidates/**`
- `configs/data-rights/krx.schema.json`
- `configs/data-rights/krx.entitlement.example.json`

**금지 파일**

- migrations, job-queue candidate, API/Web, Compose, OpenAPI, Cargo.lock, CI

**선행조건**

- 0045 SQL contract과 registry/binding column names가 frozen 상태다.

**RED**

- 두 membership을 가진 paginated Raw fixture가 현재 단일-binding assertion으로 실패.
- one membership missing, duplicate index natural key, wrong dataset, mixed entitlement,
  synthetic-as-production, future effective membership, exact replay mismatch를 각각 실패로
  고정한다.

**구현 산출물**

- §4.1~4.3의 partition/catalog/publish/recovery/health.
- KOSPI-only regression과 two-universe source batch live test.

**GREEN**

```bash
cargo test -p market-data --test candidate_ingestion -- --nocapture
DATABASE_URL="$DATABASE_URL" cargo test -p collectors --test candidate_catalog \
  -- --nocapture --test-threads=1
DATABASE_URL="$DATABASE_URL" cargo test -p collectors --test research_worker \
  -- --nocapture --test-threads=1
cargo clippy -p market-data -p collectors --all-targets --all-features --locked -- -D warnings
```

**fail-closed 기대**

- expected membership 하나라도 없으면 raw source batch는 PUBLISHED가 되지 않는다.
- old PUBLISHED exact identity는 rights revoke 뒤에도 읽기/skip 가능하지만 새 ingest는
  current entitlement 없이는 진행하지 않는다.

**checkpoint**

- `feat(research): publish sealed multi-universe sources`

### Agent Compute — “Noether”

**소유 파일**

- `crates/job-queue/src/candidate/mod.rs`
- `crates/job-queue/src/candidate/schedule.rs`
- `crates/job-queue/src/candidate/input.rs`
- `crates/job-queue/src/candidate/runner.rs`
- `crates/job-queue/src/bin/candidate-runner.rs`
- `crates/job-queue/tests/candidate_runner.rs`

**금지 파일**

- migrations, market-data/collectors, API/Web, Compose, Cargo.lock, CI

**선행조건**

- 0045 run/feed identity와 Source agent의 `CandidateUniverseKey` API가 frozen 상태다.

**RED**

- 같은 date 두 universe schedule, universe-scoped correction, one-universe failure isolation,
  separate normalization, recent listing exclusion, exact replay를 먼저 추가한다.

**구현 산출물**

- `schedule_latest_candidate_runs`, payload/input universe binding, single-service sequential run,
  per-universe health/readiness.
- factor-engine 계산식 변경 없음.

**GREEN**

```bash
DATABASE_URL="$DATABASE_URL" cargo test -p job-queue --test candidate_runner \
  -- --nocapture --test-threads=1
cargo test -p job-queue --lib candidate -- --nocapture
cargo clippy -p job-queue --all-targets --all-features --locked -- -D warnings
```

**fail-closed 기대**

- missing/unlicensed/unsealed universe는 해당 run을 만들지 않는다.
- 한 universe failure가 다른 universe의 기존 PUBLISHED feed를 supersede하지 않는다.
- partial snapshots/job success 조합은 deferred settlement guard가 거부한다.

**checkpoint**

- `feat(candidate): schedule and publish per universe`

### Agent API — “Turing”

**소유 파일**

- `crates/api-server/src/http/candidates.rs`
- `crates/api-server/src/http/screener.rs`
- `crates/api-server/src/repos/candidates.rs`
- `crates/api-server/src/contract.rs`
- `crates/api-server/tests/http_candidates.rs`
- `apps/api-server/scripts/openapi-spec.mjs`

**금지 파일**

- generated `openapi.json`/`openapi.ts`, migrations, worker/runner, Web, Compose, CI

**선행조건**

- DB/Compute universe fields와 response semantics가 frozen 상태다.

**RED**

- omitted universe compatibility, KOSDAQ feed, no fallback, both-universe screener,
  duplicate instrument two-row behavior, cursor v1/v2, run-set freeze, tamper, rights revoke를
  live HTTP test로 먼저 추가한다.

**구현 산출물**

- §5.1~5.4의 repository/handler/cursor/saved-screen/authored OpenAPI.

**GREEN**

```bash
DATABASE_URL="$DATABASE_URL" cargo test -p api-server --test http_candidates \
  -- --nocapture --test-threads=1
cargo test -p api-server --test openapi_contract -- --nocapture
cargo clippy -p api-server --all-targets --all-features --locked -- -D warnings
```

**fail-closed 기대**

- invalid universe, mismatched run, cursor/run-set splice, inactive pinned entitlement는 typed
  error이며 다른 universe data를 대신 반환하지 않는다.

**checkpoint**

- `feat(api): expose universe-scoped candidate research`

### Agent Web — “Ada”

**소유 파일**

- `apps/web/app/(authenticated)/candidates/**`
- `apps/web/app/(authenticated)/screener/**`
- `apps/web/app/(authenticated)/stocks/**`
- `apps/web/components/candidates/**`
- `apps/web/components/screener/**`
- `apps/web/lib/products/candidate-contracts.ts`
- `apps/web/lib/api/product-client.ts`
- `apps/web/tests/candidate-research-surface.test.tsx`
- `apps/web/tests/e2e/candidates.spec.ts`
- `apps/web/tests/e2e/support/candidate-fixture.mjs`
- candidate 관련 `apps/web/app/product.css` 구간

**금지 파일**

- backend, migrations, `apps/api-server/generated/openapi.ts`, Compose, CI

**선행조건**

- coordinator가 OpenAPI generated types를 재생성한 뒤 시작한다.

**RED**

- default KOSPI tab, KOSDAQ tab, both screener grouping, duplicate instrument badges,
  saved-screen universe restore, blocked/stale/no-run UI를 먼저 추가한다.

**구현 산출물**

- §5.5 UI와 accessibility/mobile behavior.

**GREEN**

```bash
npm run lint --workspace @lagrange/web
npm run typecheck --workspace @lagrange/web
npm test --workspace @lagrange/web
npm run build --workspace @lagrange/web
bash scripts/qa/candidate-web-e2e.sh
```

**fail-closed 기대**

- typed API error를 빈 READY list로 렌더하지 않는다.
- cross-universe global rank/Top5 문구를 만들지 않는다.

**checkpoint**

- `feat(web): add candidate universe controls`

### Agent Verification/Reviewer — “Raman”

**소유 파일**

- read-only가 원칙. 최종 evidence 문서만 coordinator 승인 하에 생성 가능.

**금지 파일**

- 모든 product/test/migration 파일 수정 금지.

**선행조건**

- 최초 baseline 또는 각 wave의 writer가 편집을 중단한 상태.

**산출물**

- §10 전체 evidence, 0042~0044 무변경, P0/P1/core-P2 판정.
- 발견은 정확한 file/line/reproduction으로 owner agent에 반환한다.

**checkpoint**

- 코드 commit 없음. 최종 판정은 `OK` 또는 `NOT OK` 하나다.

---

## 9. 4슬롯 Wave/DAG

| Wave | Coordinator | Worker 1 | Worker 2 | Worker 3 | 종료 gate |
|---|---|---|---|---|---|
| 0 | baseline/checksum, ownership freeze | Reviewer baseline | 비움 | 비움 | 현재 전 테스트 GREEN |
| 1 | schema questions 결정 | Atlas: 0045+DB tests | Kepler: source RED tests만 | Noether: compute RED tests만 | live 0045 up/down/up GREEN |
| 2 | migration-contract 최종 통합 | Kepler: source 구현 | Noether: compute 구현 | Turing: API RED+구현 | source/compute/API focused GREEN |
| 3 | OpenAPI json/TS, Cargo.lock, compose/static/smoke 통합 | Ada: Web | Turing: API 회귀/지원 | Reviewer: read-only 통합 검사 | Web+OpenAPI+Docker GREEN |
| 4 | CI 보완 | Reviewer 최종 | 필요 owner 1명만 수정 | 다른 writer 금지 | P0=0, P1=0, core-P2=0 |

DAG:

```text
baseline
  -> 0045 contract
     -> source/ingest ----\
     -> compute ----------+-> coordinator integration -> Web -> full validation
     -> API --------------/                               -> independent review
```

동일 파일 충돌 방지:

- `migration_contract.rs`: Wave 1 Atlas 종료 후 coordinator만.
- `candidate.rs`: Kepler만. Noether는 공개 type을 소비만 한다.
- `openapi-spec.mjs`: Turing, generated artifacts: coordinator.
- `compose.yml`, `.env.example`, static/schema checks, QA smoke, CI: coordinator.
- `Cargo.lock`: agent가 수정하지 않고 coordinator가 전체 dependency 수렴 후 한 번 갱신한다.

---

## 10. 명령 수준 검증 매트릭스

### 10.1 Hygiene/static

```bash
git diff --check
git diff --name-only -- migrations/0042_* migrations/0043_* migrations/0044_*
rg -n '<<<<<<<|=======|>>>>>>>' --glob '!target/**' .
bash -n deploy/compose/candidate-static-check.sh scripts/qa/research-worker-smoke.sh \
  scripts/qa/candidate-web-e2e.sh
bash deploy/compose/candidate-static-check.sh
```

두 번째 명령은 작업 시작 baseline과 비교해 0042~0044가 이번 확장으로 변경되지 않았음을
증명해야 한다. 현재 branch의 기존 변경을 origin과 단순 비교하지 말고 coordinator가
기록한 pre-task tree object/checksum과 비교한다.

### 10.2 PostgreSQL 18 live

```bash
DATABASE_URL="$DATABASE_URL" cargo test -p migration-contract \
  --test migration_contract -- --nocapture --test-threads=1
DATABASE_URL="$DATABASE_URL" cargo test -p collectors --test candidate_catalog \
  -- --nocapture --test-threads=1
DATABASE_URL="$DATABASE_URL" cargo test -p collectors --test research_worker \
  -- --nocapture --test-threads=1
DATABASE_URL="$DATABASE_URL" cargo test -p job-queue --test candidate_runner \
  -- --nocapture --test-threads=1
DATABASE_URL="$DATABASE_URL" cargo test -p api-server --test http_candidates \
  -- --nocapture --test-threads=1
```

필수 live cases:

- fresh apply, no-op rerun, guarded down, clean down/up
- existing KOSPI row backfill의 id/hash/entitlement 불변
- direct research_writer/worker/app DML denial
- SECURITY DEFINER owner/search_path/EXECUTE matrix
- same universe concurrent schedule exact replay
- two universe concurrent schedule creates two independent identities
- cross-universe supersede/sequence corruption denial
- KOSDAQ entitlement missing/inactive/fetch-mode mismatch denial
- source batch 5/6 binding, wrong dataset, mixed license, future membership denial
- KOSDAQ data/saved screen 존재 시 down guard

### 10.3 Rust workspace

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked --offline
cargo clippy --workspace --all-targets --all-features --locked --offline -- -D warnings
cargo test --workspace --locked --offline
```

환경 의존 test는 DB/Docker/loopback이 준비된 별도 job에서 no-SKIP으로 재실행한다.
skip을 pass evidence로 집계하지 않는다.

### 10.4 2일 rolling source-to-HTTP E2E

fixture 구성:

- KOSPI eligible 5+, KOSDAQ eligible 5+
- 두 universe 중복 종목 1개
- KOSDAQ 신규상장/60세션 미만 종목 1개
- 60 KRX sessions의 price와 FOREIGN/INSTITUTION flow
- day 2에서 겹치는 59 session immutable reuse + 새 session 1개

검증:

1. Raw ingest → typed partition → six source bindings seal.
2. 두 universe schedule/run/PUBLISHED feed.
3. 각 feed Top 5와 normalization 분리.
4. 신규상장 typed exclusion.
5. screener both에서 중복 종목 두 행 및 stable cursor.
6. pinned entitlement revoke 뒤 unrelated active entitlement가 있어도 403/BLOCKED.
7. day 2 exact replay와 correction isolation.

### 10.5 OpenAPI/Web

```bash
npm run openapi:check --workspace @lagrange/api-server
npm run lint --workspace @lagrange/web
npm run typecheck --workspace @lagrange/web
npm test --workspace @lagrange/web
npm run build --workspace @lagrange/web
npx playwright install --with-deps chromium
bash scripts/qa/candidate-web-e2e.sh
```

OpenAPI check는 첫 실행이 drift를 고치고 실패할 수 있으므로 coordinator가 생성 결과를
검토한 후 다시 실행해 clean PASS를 확인한다.

### 10.6 Compose/Docker/CI

```bash
LAGRANGE_CODE_COMMIT=0123456789abcdef0123456789abcdef01234567 \
  docker compose --env-file deploy/compose/.env.example \
  -f deploy/compose/compose.yml config --quiet
sg docker -c 'bash scripts/qa/research-worker-smoke.sh'
python -m unittest scripts.ci.test_ci_contract -v
```

Docker smoke는 migration_owner migration, research_writer least privilege, common+two
membership binding, two feed, second idempotent run, cleanup까지 확인한다. Compose service
목록은 전후 동일해야 한다.

---

## 11. 보안·권리·성능·데이터 품질 위험

| 위험 | 방어 |
|---|---|
| KOSPI contract로 KOSDAQ membership 사용 | registry dataset id + exact entitlement UUID/reference/date + covered_datasets 검증 |
| synthetic candidate rows가 production READY로 승격 | raw ledger fetch_mode를 두 membership binding까지 결속; scheduler가 required_fetch_mode를 재검증 |
| 한 Raw membership 문서를 두 index로 재해석 | index별 canonical partition hash와 dataset binding; DB publisher가 dataset에서 index를 파생 |
| 같은 response kind의 binding overwrite | PK를 dataset id 축으로 변경하고 seal expected-set 검증 |
| 미래 편입 정보 누출 | announced/effective/available/cutoff PIT 조건과 live negative test |
| cross-universe score 비교 | per-run normalization, block ordering, universe badge, global rank 필드 금지 |
| cursor 페이지 중 correction | signed frozen run-set v2 |
| duplicate instrument 손실 | `(universe,run,instrument)` 맥락 유지, screener dedupe 금지 |
| 한 universe 장애가 다른 feed 삭제 | publication supersede WHERE universe+date; concurrent live test |
| serving role가 registry/source 수정 | FORCE RLS, no direct DML, narrow definer only |
| 350종목으로 runtime 증가 | 순차 run, 기존 bounded lease/timeout, p95 측정 후에만 최적화 |
| 0045 down 데이터 손실 | KOSDAQ rows/bindings/saved screens/active jobs guard |
| rolling deploy 구/신 binary 혼용 | 짧은 scheduler drain, migration/function gate, exact code commit provenance |

---

## 12. Definition of Done

- [ ] `0045`만 추가되고 0042~0044 pre-task checksums가 동일하다.
- [ ] live PostgreSQL에서 apply/rerun/down guard/clean down-up, RLS, grants, concurrency가
  no-SKIP PASS다.
- [ ] source batch가 공통 4 + KOSPI/KOSDAQ membership 2를 immutable하게 seal한다.
- [ ] KOSPI-only 기존 경로의 응답/identity가 universe default 아래 유지된다.
- [ ] 두 universe가 같은 session에 각각 독립 run/feed/Top 5를 발행한다.
- [ ] scores/ranks/normalization/correction이 universe를 넘지 않는다.
- [ ] 60-session 미만 종목이 typed exclusion snapshot으로 남는다.
- [ ] feed/stock API universe default·explicit behavior가 통과한다.
- [ ] screener one/both, duplicate row, saved v1/v2, cursor v1/v2/tamper가 통과한다.
- [ ] Web lint/typecheck/65+ regression tests/build/Playwright가 통과한다.
- [ ] Docker functional smoke가 두 universe와 idempotent replay를 no-SKIP 통과한다.
- [ ] 실제 credential/transport 부재 production이 READY를 가장하지 않는다.
- [ ] 새 Compose service가 없다.
- [ ] full workspace fmt/check/strict Clippy/tests와 CI contract가 통과한다.
- [ ] 독립 reviewer가 `P0=0`, `P1=0`, `core-flow P2=0`으로 `OK`를 준다.

하나라도 미충족이면 구현 완료 또는 production-ready로 보고하지 않는다.

---

## 13. 규모와 이번에 미룰 과설계

### 규모

- **Small:** UI label/query parameter만 추가 — 불충분. DB identity와 source rights가 깨진다.
- **Medium:** DB run/feed axis + API/Web — 불충분. KOSDAQ source가 sealed provenance를 갖지
  못한다.
- **Large(권장/본 계획):** 0045 source binding, run/feed identity, source/compute/API/Web/E2E를
  하나의 vertical slice로 완성한다.

예상 작업량은 6개 역할 기준 약 8~12 agent-days, 3개 worker 병렬 시 통합/재리뷰 포함
약 3~5 작업일 수준이다. 실 credential 계약 협의와 production 데이터 backfill 시간은
포함하지 않는다.

### 미룰 항목

- 임의 N-universe self-service registry CRUD
- KOSPI/KOSDAQ 통합 ranking·추천
- universe별 별도 worker/sharding
- materialized view/cache/search engine
- 별도 entitlement 여러 개를 한 Raw batch에 조합하는 multi-contract contract
- 실 KRX client 구현
- intraday refresh, alert, 확률 calibration

초기 범위는 하나의 active candidate-use entitlement가 공통 dataset과 두 membership
dataset 모두를 덮는 것으로 고정한다. 실제 라이선스가 분리돼 있다면 구현 시작 전에
multi-contract batch 설계로 계획을 개정해야 한다.

---

## 14. 구현 전 확인 항목과 권장 기본값

이미 확정된 제품 결정은 다시 묻지 않는다. 다음 기술 사실만 Wave 0에서 확인한다.

| 확인 | 권장 기본값 |
|---|---|
| KRX 계약이 두 membership dataset을 하나의 candidate-use entitlement로 덮는가 | 예. 아니면 구현 중 임의로 합치지 말고 계획 재승인 |
| provider-neutral membership 응답이 두 index를 한 batch에 포함 가능한가 | 한 `IndexMembershipDocument`, index별 partition |
| registry 활성화 방식 | 두 row enabled, serving DML 없음, migration_owner 운영 절차 |
| API feed 복수조회 | 같은 endpoint의 단일 `universe` param; Web tabs가 별도 호출 |
| both screener ordering | registry order → score desc → instrument asc |
| duplicate instrument | universe별 별도 row |
| cursor | v1 KOSPI legacy 수용, 신규 v2만 발급 |
| saved screen | v1 read default KOSPI, 새 write v2 |
| Top 5 미달 | padding/다른 universe 보충 금지; fail/block existing publication rule 유지 |
| sequence | `(universe,as_of_date)` 범위 |
| runtime | single service, deterministic sequential universe order |

Wave 0에서 실제 constraint/function names를 다시 캡처하고 0045 down에 원형 복구 SQL을
포함한다. 추측한 이름으로 migration을 작성하지 않는다.

---

## 15. 커밋·체크포인트 순서

현재 대규모 미커밋 worktree와 섞지 않도록 구현 시작 전에 candidate vertical baseline을
먼저 독립 commit/branch checkpoint로 고정한다. 이후 각 커밋은 해당 focused GREEN 뒤에만
만든다.

1. `docs(plan): add multi-universe candidate rollout plan`
2. `test(candidate): add multi-universe red contracts`
3. `feat(db): add multi-universe candidate boundary`
4. `feat(research): publish sealed multi-universe sources`
5. `feat(candidate): schedule and publish per universe`
6. `feat(api): expose universe-scoped candidate research`
7. coordinator integration: OpenAPI JSON/TS, Cargo.lock, Compose/static/schema/smoke
8. `feat(web): add candidate universe controls`
9. `test(candidate): cover rolling multi-universe e2e`
10. `ci(candidate): enforce multi-universe release gates`
11. `docs(verify): record independent multi-universe review`

각 checkpoint에서:

```bash
git diff --check
cargo fmt --all -- --check
```

를 기본 실행하고, 소유 agent의 focused command를 추가한다. 최종 commit 이후에만 push/CI를
실행하며 CI가 실패하면 해당 owner의 새 fix commit으로 고친다. 이미 검증된 commit을
amend/rewrite하지 않는다.
