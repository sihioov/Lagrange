# Lagrange Station — 상태 종합

**최신 기준 시각: 2026-08-24 (Asia/Seoul), 기준 트리 `1150700` = `origin/main`,
작업 브랜치 `audit-project-status` = `fd10488`.**
§0.1~§0.12는 운영·Stage6 진행 당시의 날짜별 스냅샷이고, §0.13~§0.16은 remediation
당시의 기록이다. **현재 상태는 §0.29(이틀치 발행 달성), §0.30(출시 준비도 실측),
§0.31(독립 출시 준비도 분석)을 우선한다.** 출시까지의 작업 순서는 §0.15에 정의돼
있고 그 현재 위치는 §0.34에 있다 — 작업 2의 봉인 artifact 코드와 작업 4의 추천 코드
경로는 완료했지만, 작업 3의 실 승인·등록과 작업 4의 백테스트·운영 개방은 미완료다.
작업 5·6도 미수행이다. §4.2의 남은
소유자 결정 5건은 2026-08-23~24에 owner-only 베타 범위로 모두 해소됐고,
착수 가능한 코드 작업은 §4.3과 승인된 실행 계획에 있다. 이후 §1부터는 설계 목표와 08-17 이전 게이트·구현
이력을 보존한 기록이다. 과거의 "미설치", "KIS credential 없음", "초기 백필 미완료"
같은 문장은 당시에는 사실이었지만 현재 상태를 뜻하지 않는다. 반대로 §0.21·§0.23이
보여주듯 **"완료"·"처음" 같은 서사 문장도 나중에 철회된 전례가 있다** — 새 사실을
쓸 때는 반증이 될 수 있는 가장 가까운 곳을 먼저 열어볼 것(§0.23). 코드가 바뀌면
게이트와 판정은 다시 실행해 갱신해야 한다.

**2026-08-24 owner-beta execution seam.** The next immutable `research-worker`
image definition now includes historical price beta materialize/check and exposes
it only through the installed-current-release root-only wrapper
`scripts/ops/kis-historical-price-beta-artifact.sh`. It uses the V2 manifest's
exact research-worker image ID/revision, `network none`, UID/GID 10001, no
secrets/DB/provider environment, Raw read-only only for materialize, and the
dedicated artifact leaf read-write/read-only by operation. This closes the
runtime implementation seam only. The currently installed `66b2a8c` release has
not been rebuilt or switched, and this does not approve missing pins, Curated/DB
publication, READY, recommendation use, or live trading.

The separate root-only `--approval-check` mode invokes the compile-time embedded
checker with only the dedicated artifact leaf read-only; it does not chain from
`--check`, accept a registry path, or mount Raw/Curated. The checked-in embedded
approval registry is empty, so real approval-check execution remains blocked
until a separately reviewed registry commit is rebuilt into a new immutable
image.

**2026-08-24 historical action-evidence execution.** 소유자가
`2020-01-31..2026-08-19`, whole-market, KSD 7개 class, pagination 포함 최대
70 GET의 읽기 전용 운영 수집을 명시적으로 승인했다. 실행은 continuation 없이 정확히
7 GET/7파일로 끝났고 immutable Raw batch
`552c811a-d338-4cd1-96bd-fcd61e641bcb`, manifest
`sha256:6692f7e5dc215ddce145e63e647344f8264724497ef0d6f6c441b06dedd4f0bd`를
커밋했다. 기존 Stage5 manifest
`sha256:6f1414852fd50ccf35c7604c63af70fedc83020fc71685d8db5c2a5c431cbdc4`와
함께 network-none/read-only verifier로 본문을 재인증했다.

whole-market 응답을 고정 ETF11 후보에 투영할 때 타 종목의 정상 행까지 거부하던 결함은,
일일 KIS 정규화기와 같은 원칙(문서화된 모든 필드를 먼저 검증한 뒤 universe 필터)으로
수정했다. Raw 응답이 nonempty였다는 사실도 별도 attestation으로 보존해 이를 거짓
zero-result로 표시하지 않는다. 그 뒤 검증은 ETF11 관련 `dividend` class에서 의도대로
`unsupported_action_dividend`로 차단됐다. KIS dividend 응답만으로는 현재 계약이 요구하는
canonical ex-date/announcement-time을 증명할 수 없으므로 값을 추측하지 않았다. 따라서
sealed artifact 생성, 승인 레지스트리 수정, Curated/DB/READY/추천 연결은 모두 미실행이며,
다음 게이트는 별도 검토된 dividend evidence/mapping 결정이다.

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

**KIS 권리 판정 갱신(2026-08-21).** 소유자가 이 시스템을 개인 단독으로
사용하며 승인된 KIS read-only 데이터의 조회·보존·가공에 필요한 권리가 있음을
확정했다. 다중 사용자 기능이 코드에 남아 있어도 실제 승인 범위는 소유자 1명뿐이며,
그 기능의 존재를 이유로 KIS entitlement를 다시 질문하거나 외부 blocker로 되돌리지
않는다. 결정문과 해시 고정 metadata는 ADR-0005 및
`configs/data-rights/kis.entitlement.json`에 기록했다. 이 결정은 데이터 권리만
해소한다. KIS credential은 소유자가 앞서 등록한 기존 App Key/App Secret을 보호된
secret 경로에서 재사용하며 새 키 등록·발급을 다시 요구하지 않는다. 남은 운영 단계는
기존 secret의 경로·권한 확인과 runtime copy, 실제 endpoint 검증, PIT 교차검증,
backfill, DatasetManifest/five-pin과 운영 배포다.

실거래는 별도 후속 프로젝트다. 계좌·잔고·주문·정정·취소·체결·주문 WebSocket과 Compose `live` profile은 계속 금지한다.

### 0.4 Stage6 공식 데이터 통합 초기 계획 — §0.6 이후 정정됨

사용 목적은 개인 내부용으로 진행한다. 데이터는 API에서 DB로 직접 넣지 않고 다음 경계를 지킨다.

`공식 소스 → immutable Raw → 교차검증/종목 식별 → 승인된 Curated → DB`

소스 역할은 다음처럼 고정한다.

- **KIS**: 고정 ETF 11종목의 주 가격·거래량 소스. 과거 데이터는 strict PIT가 아닌 수집 시점 vendor snapshot으로 표시한다.
- **KRX Open API**: `ETF 일별매매정보` 표면은 deferred 상태이며 KIS 가격의 교차검증·fallback 경로로 자동 선택하지 않는다. 두 번째 price API는 아래 2026-08-21 결정의 명시적 승인이 있어야 한다.
- **KIND**: 신규/추가/변경상장, 상호변경, 시장조치와 게시·정정 관계를 보강한다. 반드시 API일 필요는 없고 공식 Excel/다운로드도 immutable Raw로 수집할 수 있다.
- **OpenDART**: 기업공시 결정 정보, 최초 공시시각, 정정·철회 체인을 제공한다.
- **KSD/SEIBro**: 배당·증자·감자·합병/분할·권리 일정 등 기업행사 세부사항을 제공한다.
- 소스가 충돌하거나 공개시각·효력일·정정 계보가 부족하면 자동 추정하지 않고 Raw만 보존한 채 Curated/READY/pin을 차단한다.

초기 범위는 고정 ETF 11종목, 시작일 `2020-01-31`이다. KIS는 주 가격·거래량 소스이며, 두 번째 price API를 쓰려면 독립적인 cross-check 또는 fallback 목적·우선순위·failure behavior에 대한 별도 소유자 승인이 필요하다. record date·지급일·상장일·권리락일을 `available_at`으로 소급하지 않는다.

### 0.5 Stage6 초기 작업 계획 — 현재 상태는 §0.13~§0.14

아래 목록은 Stage6 구현 전의 historical baseline이다. 완료 여부와 정정된
source 역할은 §0.6~§0.14를 따른다.

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

**Step 2 착수 — OpenDART fixture 어댑터 (2026-08-19).** 승인된 OpenDART 코어에 대해 Raw-only 어댑터를 구현했다. `ResponseKind`에 `DisclosureIndex`/`DisclosureEntityMaster`/`DisclosureEntityProfile`을 추가했고(`5309ffa`), 이들은 `EOD`/`CANDIDATE`/`CANDIDATE_MASTER`/`ALL_RESPONSE_KINDS` 어디에도 포함되지 않으며 `validate::validate_response`가 명시적으로 거부한다 — `CandidateMaster` 선례와 동일하게 Raw-only 증거를 기존 파이프라인에서 격리한다. 어댑터(`cc268d5`)는 reader trait + fixture 구조로, **HTTP 클라이언트와 네트워크 I/O가 존재하지 않고 의존성 추가도 없다.** live reader는 credential reference 없이 생성 불가한 stub이며 항상 fail-closed다. key는 query parameter이므로 `RequestMetadata` 생성 지점 단 한 곳에서 리댁션하고, sentinel key가 stored bytes·`batch.json`·manifest에 도달하지 않음을 테스트로 증명한다. 페이지네이션은 10페이지 상한, `total_count`/`total_page` 불변, 동일 bytes 거부, `013`은 batch를 만들지 않는 typed empty다. `corpCode.xml`은 ZIP magic 검사 후 바이트 그대로 저장하며 압축을 풀거나 파싱하지 않는다. 게이트는 fmt/clippy `-D warnings` clean, `cargo test -p market-data` 315 passed. **key가 발급되지 않았으므로 실제 요청은 한 건도 발생하지 않았다.**

전체 워크스페이스 실패 21건은 §0.9에서 진단·해소했다. 당시 기록: `cargo test --workspace --no-fail-fast` 1,692 passed / 21 failed, 실패 3개 타깃(`collectors research_worker` 6, `job-queue paper_preview` 3, `job-queue recommendation_compute` 12). 변경 이전 커밋 `603f9c7`에서 동일 재현되어 Stage6와 무관함을 확인했다.

### 0.7 Stage6 D4 판정 — OpenDART는 ETF11을 커버하지 않는다 (2026-08-20)

소유자가 OpenDART key를 제공해 live 경로를 승인했고, **`GET /api/corpCode.xml` 단 1회**를 호출했다. 결과는 결정적이다.

- 응답은 3,596,991바이트 ZIP, 내부 `CORPCODE.xml`은 28,585,431바이트, 엔트리 118,714개, 그중 `stock_code`가 비어 있지 않은 것은 3,984개다.
- **ETF11 종목코드 11개 중 `stock_code`로 등장하는 것은 0개다.**
- 같은 파일의 대조군 `005930`·`000660`·`035420`는 모두 확인되므로 방법론 실패가 아니라 실제 부재다.
- `자산운용`을 포함하는 법인은 458개 존재하지만 **전부 `stock_code`가 없다** — 공시 제출인은 운용사이고 ETF 자체는 공시 대상 법인이 아니다. `KODEX`·`상장지수`는 0건이다.

따라서 **`list.json`과 `company.json`은 `corp_code` 기반이므로 ETF11에 쓸 수 없다.** ADR-0004 D4가 확정됐고, 그 시점에는 ETF11의 공시일 근거로 **KIND만 남아** 게시 시각 granularity와 정정 연계 gap이 critical path로 올라왔다. 게시 시각은 바로 다음 §0.8에서 분 단위로 해소됐고, 정정 체인 추적은 §0.14의 다음 작업으로 남는다. `corpCode.xml` 자체는 이 부재의 증거로, 그리고 발행사가 공시 대상인 개별주식 범위에서는 identity join으로 계속 가치가 있다.

**live 경로 차단 — rustls 비호환 (ADR-0004 D10).** `opendart.fss.or.kr`은 TLS 1.2만 지원하고 순방향 비밀성이 없는 static RSA 암호군(`AES128-GCM-SHA256`)을 선택하며, ECDHE로 제한하면 handshake failure로 거부한다. rustls는 순방향 비밀성 없는 키교환을 구현하지 않으므로, 워크스페이스의 단일 TLS 스택 정책 아래서 in-process 전송 계층은 이 호스트에 도달할 수 없다. 인증서 체인 자체는 정상(GlobalSign Root CA - R3, SAN에 `opendart.fss.or.kr` 포함)이므로 신뢰 문제가 아니라 암호군 비호환이다. 위 `corpCode.xml` 취득은 그래서 **진단 목적의 외부 도구 조회**였고 immutable Raw 배치를 만들지 않았다. 두 번째 TLS 스택 도입은 명시된 단일 스택 규칙을 바꾸는 결정이라 소유자 판단으로 남긴다. D4 결과 때문에 이 표면의 ETF11 용도가 사라졌으므로 차단의 실질 비용은 거의 없다.

### 0.8 KIND 분 단위 공시시각 확인과 결정 3건 확정 (2026-08-20)

D4로 KIND가 ETF11 공시일의 유일한 근거가 된 뒤, 그 KIND가 시각을 제공하는지가 하류 전체의 critical path였다. **제공한다 — 분 단위다.**

KIND 검색은 순수 HTTP로 구동할 수 없다. 검색 타깃(`method=searchDetailsSub`, `forward=details_sub`)은 찾을 수 있지만 POST가 세션 쿠키를 붙여도 거부된다 — 엔드포인트가 페이지 JavaScript가 만드는 상태를 요구한다. 파라미터를 추측해 재구성하는 것은 이 프로젝트가 금지로 규정한 probing이므로, **사이트 자체의 `fnSearch()` 컨트롤을 브라우저 엔진에서 호출**해 확인했다(Playwright + headless Chromium, 저장소 밖 `~/tools/kind-probe`). export 다운로드나 데이터 수집 없이 렌더된 테이블 구조만 읽었고, 페이지 로드는 소량이다.

공시 목록 헤더는 `번호 | 시간 | 회사명 | 공시제목 | 제출인 | 차트/주가`이고 `시간`이 공시별로 `YYYY-MM-DD HH:MM`을 담는다. 부수 관측이 아니라 사용 가능한 값임을 3가지로 검증했다.

- **한 페이지 안에서 값이 다르다** (`2020-03-31 16:11`과 `16:09`) → 상수나 렌더 아티팩트가 아닌 레코드별 값
- **과거에도 존재한다** — 2020-03-31 창에서 15/15 행이 시각을 보유. 범위 시작 `2020-01-31`을 넘어선다
- **날짜 범위가 실제로 도달한다** — 입력 수락에 그치지 않고 2020년 행을 반환한다

따라서 **ADR-0004 D1이 KIND 출처 공시에 대해 분 단위로 조여진다.** D1이 "확인된 시각 소스는 좁힐 수만 있다"고 명시한 그 경우다. 변하지 않는 것: 기준일·지급일·상장일·권리락일 소급 금지, 피드 데이터는 여전히 `documented_cadence`. 남은 부정확성 2건은 가정으로 덮지 않고 기록했다 — **표시 시각의 timezone이 페이지에 명시되지 않았고**, 정정 버전은 날짜 단위로만 열거된다.

체크리스트 11번은 부분 해결이다. 공시 뷰어의 `mainDoc` select가 정정 체인 전체를 순서대로 노출한다(원본 `2025.11.05` → `11.14` → `11.25` → `12.04` → `12.17` → `2026.01.22`). 다만 시각이 없고 `관련공시` 참조도 없으며 `orgDiscls`는 샘플 문서에서 비어 있었다. 12번은 해결됐다.

**정정 사항.** `069500` 검색이 "KODEX 200"을 반환한 것을 ETF11 확인으로 오독한 초기 판독이 있었다. 실제로는 `searchCorpName`도 hidden `repIsuSrtCd`도 필터가 걸리지 않고 미필터 목록의 첫 행이 반환된 것이다(`102110`·`114260`·`132030` 모두 동일 결과가 증거). **ETF는 클래스로 커버되지만 종목 단위 확인은 미해결**이며 체크리스트 17번으로 남겼다.

**확정된 결정 3건 (2026-08-20).**

1. **TLS — 그대로 둔다** (ADR-0004 D10, option 3). 두 번째 TLS 백엔드를 도입하지 않으므로 단일 TLS 스택 규칙이 유지된다. OpenDART는 ETF11에 대해 documentation-only다. `opendart-client`·게이트 CLI·어댑터 계약은 fixture 검증 상태로 트리에 남기며 운영에서 도달 불가다 — 유지 비용이 없고, 발행사가 공시 대상인 개별주식 범위가 열리면 바로 쓸 수 있다.
2. **entitlement reference 확정** — `opendart:tou-art16-art23:personal-internal:2026-08-20`. 소유자가 개인 내부용으로 기록했다. KOGL 제2유형과 KRX 제11조③ 제약 사실 자체는 기록으로 유지한다.
3. **KIND 접근 방식 — 로컬 필터** (ADR-0004 D11). 사이트 팝업 상태를 재현하지 않고, ETF 전용 목록을 날짜 범위로 받아 `종목명`으로 로컬 필터한다. D6는 불변이므로 **KIND 대량·정기 수집은 계속 운영자 주도**이고 여기서 승인된 바 없다. 확정된 것은 수집이 승인될 때 쓸 *메커니즘*과, 항목 10·11·12를 답한 소량 브라우저 진단의 허용이다.

### 0.9 기존 실패 21건 진단과 해소 (2026-08-20)

Stage6와 무관한 별건이지만 저장소 자체 품질 기준(전체 스위트 통과)을 깨고 있었다. **원인은 3개이고 그중 둘이 같은 커밋에서 나왔다.**

커밋 `f815f63`("attest cumulative KIS datasets end to end", 2026-08-19)이 프로덕션을 두 곳에서 바꾸면서 대응 테스트 fixture를 갱신하지 않았다. `f815f63`은 `603f9c7`의 조상이므로 Stage6 이전 결함이 맞다.

1. **`job-queue paper_preview` 3건 — stale fixture, 수정 완료(`b18c982`).** `CurateStore::new(root)`는 root를 그대로 보관하고 `curated_dir()`가 `curated`를 붙인다. `f815f63`이 `load_recommendation_closes`·`attest_preview_dataset`을 bare root를 넘기도록 바꿨는데 테스트의 `write_preview_bars`가 계속 `root.join("curated")`를 넘겨서, fixture는 `root/curated/curated/...`에 쓰이고 프로덕션은 `root/curated/...`를 읽었다. `MissingPrice { instrument_id: "069500.KRX" }`는 실제 부재였다. 테스트 2줄 수정, 프로덕션·어서션 무변경.
2. **`job-queue recommendation_compute` 12건 — 환경 + stale fixture, 수정 완료(`fb6d12e`).** 먼저 Phase-0 fixture 생성기가 `pyarrow` 없는 python으로 죽어 12건이 났다. 저장소 안 `nt/.venv`(pyarrow 25.0.0)를 `PYTHON=`으로 지정하면 **12건이 8건으로 바뀌며 전부 다른 오류**가 된다 — `manifest has no exact curated artifact references`. 즉 pyarrow는 해결책이 아니라 더 깊은 결함을 가리고 있었다. 실제 원인은 테스트가 `artifacts: Vec::new()`로 manifest를 만든 것이다. fixture가 자기가 쓴 curated 파일을 실제 크기·SHA-256·스키마로 열거하도록 고쳤다. 해시는 하드코딩하지 않고 디스크에서 유도한다 — attestation의 존재 의미가 manifest가 실제와 일치하는 것이기 때문이다.
3. **`collectors research_worker` 6건 — 환경, 미해결.** `DATABASE_URL`이 없고 `127.0.0.1:55432`에 아무것도 없으며 docker 소켓 권한이 없다. 이 테스트들은 워크스페이스 다수가 쓰는 `ScratchDb::create() -> Option` 조용한 skip 관례를 따르지 않고 `.expect()`로 패닉한다. **QA PostgreSQL 제공이 필요하고 코드 변경 사항이 아니다.**

**커버리지 함정 2건을 함께 잡았다.** 수정 과정에서 두 테스트가 *엉뚱한 이유로* 통과하게 됐다. `semantically_invalid_parquet_value_is_integrity`는 크기·해시 불일치가 attestation 층에서 `Integrity`를 만족시켜 의미 검사에 도달하지 못했다 — 손상 파일이 여전히 유효 parquet이고 스키마가 그대로이므로 재attest해서 리더 경로를 복원하고, **오류가 attestation 계열이 아님을 어서션으로 고정**했다(향후 short-circuit이 다시 테스트를 공허하게 만들 수 없게). `malformed_parquet_is_integrity_not_a_retryable_store_error`는 임의 바이트가 크기·해시·스키마를 모두 실패하므로 `f815f63` 이후 리더 도달이 원리적으로 불가하다 — `Integrity`는 여전히 옳은 결과이고 더 이른 실패가 의도된 계약이므로, **리더 경로를 더 이상 커버하지 않는다는 사실을 주석으로 남겼다.**

**결과: 1,692 passed / 21 failed → 1,751 passed / 6 failed.** 남은 6건은 전부 `research_worker`의 DB 환경 의존이다. 어떤 테스트도 약화·삭제·`#[ignore]` 처리하지 않았고 프로덕션 코드는 변경하지 않았다.

**남긴 지뢰 1건.** `paper_valuation.rs:362`와 `paper_execution.rs:683`은 `dataset_root.join("curated")`를 쓰고 대응 테스트 fixture도 double-join이라 읽기·쓰기가 자체 정합이므로 현재 실패하지 않는다. 그러나 한 크레이트에 `CurateStore` 관례가 두 개 공존하는 상태다. DB 없이 검증할 수 없어 손대지 않았다. 또한 `ScratchDb::create()`의 조용한 skip 때문에 이 호스트의 "1,751 passed"는 실제 커버리지를 과대표시한다.

### 0.10 KIND 수집 파이프라인 완성과 Stage6 첫 Raw 배치 (2026-08-20)

소유자가 브라우저 구동 KIND 수집을 승인해(ADR-0004 D11) 파이프라인을 완성하고 **Stage6의 첫 immutable Raw 배치를 실제로 생성했다.**

**구조**는 두 단계로 나눴다. 캡처 단계는 필연적으로 브라우저이므로 신뢰하지 않는다 — `data-pipelines/kind-capture/`(Node/Playwright)가 사이트 자체 `fnSearch()`·`fnPageGo()`를 구동해 응답 바이트와 상호작용 기록만 staging에 쓰고, Rust ingest 경로가 **디스크 바이트에서 해시를 재계산**한다. `CapturedPage`에 해시 필드가 아예 없어 캡처 단계가 해시를 주장할 코드 경로가 없다. `kind-capture`는 **npm workspaces 멤버가 아니므로**(root는 `apps/*`만 나열) root install이 Playwright와 브라우저를 dev/CI로 끌어오지 않는다.

**라이브 실행으로 확정한 사실.** 페이지네이션은 페이지 자체 `fnPageGo`이고, **마지막 페이지를 넘기면 KIND가 `pageIndex`를 clamp해 최종 페이지를 반복 반환한다.** 첫 버전이 이 반복을 9개 잡았으므로 종료 조건을 "이전 응답과 바이트 동일"로 고쳐 40 → 정확히 32페이지가 됐다(해석 없는 순수 바이트 비교, ingest 쪽 중복 거부와 같은 기준). `2020-02-03..2020-02-07` 창에서 공시 473건 = 31페이지×15 + 마지막 8행이다. 하루 약 95건이므로 **약 6일이면 40페이지 상한**이라, 백필은 5일 단위 창을 권고한다.

**바이트 안정성이 세션을 넘어 확인됐다** — 독립 실행 2회에서 32/32 페이지 바이트 동일. 재캡처를 이전 배치와 대조할 수 있다.

**첫 배치**: `9dfacd13-1f07-4b5f-8004-cf6b11c1518b`, `provider=kind-disclosure/market=kr`, 32파일, manifest 1행, `ResponseKind::DisclosureIndex`, endpoint `kind.disclosure.etf.list.v1`, entitlement `kind:krx-legal-notice:personal-internal:2026-08-20`. 검증: 저장 파일 32개가 staging 바이트와 전부 동일, page-0001의 재계산 해시가 캡처 시점에 독립 관측한 값과 일치, manifest의 header는 빈 목록, credential 형태 문자열 검색 0건. Raw 루트는 저장소의 `data/`(gitignore 대상)이며 **운영 루트가 아니다** — sudo 제약 때문이고 운영 이관은 별도 판단 사항이다.

**증거 보존.** 이 배치를 만든 staging을 저장소 밖 `~/lagrange-evidence/kind-staging-20260820/`(읽기 전용)에 보존했다. rollup `sha256:38aa2172f7e19a84de56f31410f47ff20d3843cc63edcd30c4b40a8f2e12ab87`(`capture.json` + 32 페이지의 해시 목록을 다시 해시한 값)로 캡처 전체를 한 줄로 고정하며, 보존본 32개가 커밋된 배치와 바이트 동일함을 확인했다. 결함이 있던 40페이지 시행분은 혼동을 막기 위해 삭제했다.

**배치 날짜 의미에 주의.** manifest row의 `date`는 **취득일**(`2026-08-19`)이고 데이터가 덮는 기간(`2020-02-03..2020-02-07`)이 아니다. 커버리지 구간은 기록된 form field(`fromDate`/`toDate`)에만 존재하며 일급 필드가 아니다. 즉 배치를 날짜로 열거하는 하류 단계는 취득일을 보게 된다 — Raw delivery 날짜로는 올바르지만, 백필 시 2020년 데이터 배치들이 모두 취득일을 갖는다는 뜻이므로 정규화 단계에서 커버리지를 명시적으로 다뤄야 한다.

**아직 하지 않은 것**: 정규화·Curated 승격·DB publication·five-pin. Raw는 필터 없이 공식 응답 전체를 보존하며, `종목명` 선택은 되돌릴 수 있는 정규화 단계의 일이다(D11).

### 0.11 KIND 정규화와 현재 상태 (2026-08-20)

**정규화가 실데이터로 동작한다.** `crates/market-data/src/kind_normalize.rs`가 KIND Raw 배치를 공시별 observation으로 파싱하고, `provider=kind-disclosure-normalized` 스코프에 원본 배치에서 결정론적으로 유도한 배치 id로 기록한다. 실측: Raw `9dfacd13-1f07-4b5f-8004-cf6b11c1518b`(32페이지) → 정규화 `576fa1b2-5ebb-5e0e-9a15-0ab35ca2287a`, **473 observations**, seq 473→1 누락 0.

**파서 계약은 아티팩트에서 나왔고, 그 구분이 재작성 비용을 치렀다.** 첫 버전은 표 헤더를 `<th>`에서 읽어 fixture 11개를 통과한 뒤 실제 첫 페이지에서 즉시 실패했다. 저장된 바이트에는 `<th>`가 없다 — 헤더 행이 비어 있고 컬럼명은 클라이언트 스크립트가 주입하므로, 앞서 관찰한 헤더는 브라우저 DOM에만 있었다. fixture가 렌더된 페이지를 흉내내고 코드가 그 fixture에 맞춰진 것이다. 계약을 `<table summary>` 속성으로 옮기고 fixture를 실제 마크업으로 재작성했다. **테스트 통과가 현실을 증명하지 않으며, 실배치 실행 전까지 완료가 아니다.**

**버려지던 식별자 두 개를 살렸다.** 각 행이 `openDisclsViewer('...')`로 **공시 접수번호(14자리)** 를 담는다 — **정정 체인으로 가는 조인 키**이며 D5 요구사항이자 체크리스트 11번의 대상이다. KIND 내부 종목 키도 보존하되 KRX 코드로 오인되지 않게 타입명·doc으로 못 박았다. **접수번호에서 날짜를 파생하지 않는다** — 구조가 문서화되지 않았으므로 파생하면 PIT 증거 조작이며 OpenDART `rcept_no`와 동일 규칙이다.

**timezone은 아는 것만 주장한다.** 지역 시각 원본을 그대로 보존하고 instant는 `Asia/Seoul` 가정을 명시적으로 기록한 채 유도한다(`2020-02-07 14:46` → `2020-02-07T05:46:00Z`, `AssumedAsiaSeoul`). 가정이 붙지 않은 instant를 낼 경로가 없어 확인 시 이력을 다시 쓰지 않고 유도만 고칠 수 있다.

**instrument identity는 차단이고 근거가 강해졌다.** 저장된 바이트에 6자리 KRX 코드가 없다(KIND 내부 키만). `seed_universe` 이름은 placeholder이고 universe 설정은 이름을 KRX에서 해석한다고 적어두며 **KRX는 DEFERRED**다. 실배치 202개 이름 중 `KODEX 200`·`KODEX 200ESG`·`KODEX 200TR`·`KODEX 200가치저변동`·`KODEX 200선물인버스2X`가 함께 있어 **접두어가 겹치므로 이름 매칭이 구조적으로 모호**하다. `InstrumentIdentity::Unresolved { reason }`가 유일한 변형이고 이유를 데이터로 담는다.

파싱은 모든 규칙에서 fail-closed다 — summary 불일치, 셀 개수, 시각 파싱, 빈 종목명·제목, 비정수 `번호`, 배치 전체의 `번호` 누락, 두 식별자 결측·형식오류, 행 0개 — 하나라도 실패하면 아무것도 쓰지 않는다.

### 0.12 2026-08-20 종료 시점 상태 — remediation 이전 스냅샷

- 커밋 30건(`99fb8e9`~`1f592a9`). **`main`에서 직접 작업했고 다른 브랜치에 이 커밋들이 없다.** `origin/main` 대비 **99 ahead / 0 behind**(분기 없음)이며 **push하지 않았다** — 이전 69개 커밋의 미push 상태도 그대로다.
- 당시 게이트: fmt·clippy `-D warnings` clean, `market-data` **347 통과**. 워크스페이스 **1,751 통과 / 6 실패**이고 남은 6건은 `research_worker`의 QA PostgreSQL 환경 의존이다(코드 결함 아님). 이 결과는 아래 remediation closure gate의 현재 상태를 뜻하지 않는다.
- 승인 범위: OpenDART 코어(documentation-only, D10으로 live 차단), KIND 브라우저 구동 수집. KRX 미러 vs 원본, KSD 포함 여부는 계속 DEFERRED.
- 다음에 승인 없이 가능한 것: **정정 체인 추적**(접수번호 조인 키 확보). 소유자 결정 필요: identity 해석(KRX), 백필 요청 예산, KSD. 환경 필요: QA PostgreSQL, push 여부.

### 0.13 Stage6 disclosure review 보완 완료 상태 (2026-08-20)

`f78d033`의 Stage6 disclosure review에 대한 코드·문서 보완을 반영했다.
이는 fixture 기반 및 로컬 검증이며, 이 보완 작업에서 live provider를 호출하지
않았다. 기존의 OpenDART ETF11 비식별 결론, KIS read-only 안전 경계, 권리·
entitlement의 미해결 공백은 변경하지 않는다.

| 항목 | 해소 내용 | 상태 |
|---|---|---|
| F1 | KIND `capture.json`에 필수 `termination` 4종을 기록한다. `clamped_duplicate`만 clean이고, 각 페이지는 두 bounded wait와 그 사이 한 번의 동일 control 재호출을 거친다. configured max(최대 40) 뒤 extra probe의 identical response만 clean이며 distinct response는 incomplete다. 실제 response는 exact HTTPS host/path와 decoded `method`/`forward`/date/page contract로 발행된 페이지에 귀속하고, 늦은 중복의 bytes/ordered fields가 다르거나 body task가 bounded wait 안에 끝나지 않으면 fail-closed한다. `kind-raw`와 `market-data`는 incomplete/missing/unknown termination을 Raw commit 전에 거부한다. termination 없는 옛 staging은 recapture한다. | 해소 |
| F2 | 첫 `<tr>`이 cell 없는 placeholder인지 raw opening `<td>`/`<th>` 기준으로 검사한다. placeholder 누락·정상/미종결 cell 모두 typed whole-batch failure이며 Raw HTML은 error에 담지 않는다. | 해소 |
| F3 | KIND Raw integrity test는 zip 전에 collection cardinality를 고정한다. credential-like field-name test는 각 이름의 exact typed rejection과 zero-write를 검증하며 value redaction 증거로 주장하지 않는다. | 해소 |
| F4 | Paper preview missing-close fixture의 bars deletion root를 writer와 같은 dataset root로 고치고, DB 없는 path regression test를 추가했다. 기존 manifest double-join 관례는 건드리지 않았다. | 코드·비DB 검증 해소; QA DB gate는 아래 환경 차단 |
| F5 | untrusted KIND staging에서 symlink·non-regular file·oversize page를 typed failure로 거부하고 page read를 1 MiB로 제한했다. | 해소 |
| F6 | OpenDART `list.json`의 response `page_no`를 requested page와 비교해 mismatch를 manifest 전에 fail-closed한다. 기존 duplicate-byte guard도 유지한다. | 해소 |
| F7 | OpenDART `--plan`은 실제 renderer가 만드는 text를 검사하며, query parameter value가 출력되지 않음을 sentinel으로 검증한다. | 해소 |
| F8 | arbitrary malformed Parquet bytes는 reader가 아니라 artifact attestation에서 Integrity가 되는 실제 도달 계약에 맞게 test명·assertion을 정렬했다. | 해소 |
| low: transport | connect/timeout/other send failure를 구분하되 원 `reqwest` error는 formatting·저장을 하지 않는 typed coarse classification으로 유지했다. | 해소 |
| low: credential | credential file의 앞뒤 whitespace를 canonicalize하고 empty-after-trim을 거부한다. | 해소 |
| low: OpenDART Debug | `OpenDartRead: Debug`와 맞지 않던 stale comment를 바로잡았고 reader details는 계속 출력하지 않는다. | 해소 |
| low: KIND provenance | `form_fields`를 duplicate name과 순서를 보존하는 ordered URL-decoded pairs로 명시하고 encoded POST body byte fidelity 주장을 제거했다. | 해소 |

**현재 검증.** Node capture logic **20 passed**; collectors `kind-raw`
**31 passed**, `opendart-raw` **10 passed**; market-data KIND normalization
**17 passed**, KIND Raw **14 passed**, OpenDART Raw **24 passed**;
opendart-client **32 passed**; F4의 DB 없는 deletion-path regression test가
passed했다. `pyarrow==25`가 pin된 환경에서 F8 malformed/semantic Parquet 두
focused test와 `recommendation_compute` 전체 **16 passed**, locked child 환경의
`recommendation_child` **17 passed**도 확인했다.
workspace `cargo fmt --check`, all-target/all-feature clippy `-D warnings`,
`git diff --check`가 모두 clean이고, `market-data` 전체 suite도 passed했다.

**아직 green으로 주장하지 않는 항목.** full workspace suite를 실행했으며,
대부분의 target은 passed했지만 QA PostgreSQL 의존 target이 남았다. collectors
`research_worker`는 62 passed / 6 failed이고 여섯 실패 모두
`DATABASE_URL` unset 및 Docker socket permission denied로 QA PostgreSQL을
준비할 수 없어 **BLOCKED_ENV**다. 따라서 F4의 DB 없는 regression과
`paper_preview` 전체 22 passed는 확인했어도, DB-gated preview-worker 경로는
early return되어 실제 QA DB 증거가 아니다. Python 환경은 `uv 0.12.1`, Python
3.12.13에서 `uv sync --project nt --locked`로 복구했고 `pyarrow==25.0.0`을
포함한 locked 의존성을 사용해 `recommendation_compute` 16/16과
`recommendation_child` 17/17을 통과했다. 이 호스트에는 PostgreSQL 실행 파일과
활성 service가 없고, `/var/run/docker.sock`은 root:docker `0660`인데 현재 사용자가
docker group이 아니어서 접근이 거부된다. 권한을 우회하지 않았으므로 QA DB gate는
계속 **BLOCKED_ENV**다. QA PostgreSQL을 갖춘 full workspace 재실행 전에는 전체
green으로 기록하지 않는다.

**2026-08-19 당시의 부분 승인 기록.** 소유자가 OpenDART 코어(`list.json`/`list.xml`, `corpCode.xml`, `company.json`)를 fixture 기반 Raw 어댑터 작업 범위로 승인했다. 나머지 allowlist 행, 모든 계정 등록, 라이선스 해석은 계속 보류였다. 당시에는 어떤 소스에도 key가 발급되지 않아 실제 요청이 없었다. 이후의 key·단일 live `corpCode.xml` 관측과 ETF11 결론은 §0.7 및 ADR-0004 D4를 따른다. KRX 미러 vs 원본 선택, KSD 포함 여부, KOGL 제2유형·KRX 제11조③ `entitlement_reference`의 미결정 기록은 유지한다.

### 0.14 Remediation 이후 현재 worktree와 다음 작업 (2026-08-21)

Remediation commit `90ac83d` and correction commit `460c942` were pushed on
`origin/unspecified-task`. Draft PR #1 was opened only to obtain CI evidence;
the owner subsequently selected reviewed direct-main delivery rather than a PR
merge. Python-focused gates are green. The local host still cannot provide QA
PostgreSQL. Direct-main correction `7966e72`의 GitHub PostgreSQL integration은
green으로 전환됐고, workspace test는 아래의 두 후속 원인 11건만 남겼다. The correction-viewer capture, separate Raw
ingest, and ordered-membership normalizer are implemented in the current change
set. Node tests are 37/37, the Rust correction normalizer tests are 5/5, the Raw
CLI tests are 16/16, workspace strict Clippy is clean, and all workspace test
binaries compile. This is not a READY, point-in-time, or full-workspace-green
claim: the DB-required runtime gate is red and the one observed viewer
does not prove a multi-version correction chain.

**현재 저장소 상태.** canonical repository는 Git remote 기준 `Lagrange`다.
reviewed direct-main merge는
`3ebc288e128b7a08a8434d6293a2b03a1347e1a9`로 완료됐고 첫 CI 보정
`7966e72585a27820b789220a5c85830771d5ac93`도 `main`에 직접 push했다. 기존
`/data/workspace/lagrange` worktree에서 직접 병합했으며 새 worktree나 새 브랜치를
만들지 않았다. `7966e72`의 CI run `32390587037`에서 policy, format, web,
Clippy, PostgreSQL integration은 통과했고 workspace test만 11건 실패했다.
별도 research-smoke run `32390587409`는 profile-gated range service의 필수
`RANGE_RAW_BATCH_ID`를 functional Compose config에 공급하지 않아 실패했다.
따라서 해당 시점에는 전체 workspace green이나 production READY를 주장하지 않는다. 공식 KIS
XLSX는 계속 의도적으로 untracked이며 수정·삭제·커밋하지 않는다.

**Historical remediation-branch comparison — 아래 direct-main CI 보정으로
superseded.** F1~F8/low 코드·문서 보완,
focused offline suites, fmt, strict clippy, diff check, 9개 mutation gate는 이미
완료했다. locked Python environment는 위와 같이 복구됐다. 로컬 QA PostgreSQL은
**BLOCKED_ENV**지만 원격 CI에서는 DB-backed gate가 실제 실행됐고 실패했으므로,
전체 workspace green을 주장하지 않는다. 원격 `main`의 기존 `5b12e0f` CI run
`32325584155`도 policy, PostgreSQL integration, workspace-tests job이 red였으며,
이번 branch run `32358404560` 역시 같은 세 job이 red였다. Sanitized log 대조에서
workspace의 실패 test-name 집합 31개와 PostgreSQL `paper_preview` 실패 test-name
6개가 baseline과 동일했다. F4 경로 수정은 `missing_close`의 기존 ENOENT를 제거했지만
그 뒤의 expected-outcome assertion도 실패했다. 당시 diff에는 관련 runtime source,
SQL migration, DB deployment 변경이 없었으므로 Stage6 회귀 근거는 없었지만,
entitlement와 fixture state의 근본 원인은 아직 확정하지 못했다. 이후 원인과 현재
보정은 아래 `Direct-main CI baseline 보정 진행 상태`에 기록한다.

환경 검증을 수행할 때는 CI와 같은 Python 3.12, `pyarrow==25.0.0`,
`uv==0.12.1`, `uv sync --project nt --locked`, Phase 0 fixture, disposable QA
PostgreSQL 순서를 따른다. 성공 조건은 `recommendation_child` 17/17,
`research_worker` DB-required 6개 통과, F4 DB test가 early return 없이
`PAPER_PREVIEW_CLOSE_MISSING`/permanent failure/zero-output assertion까지 도달하고
`cargo test --workspace --locked --no-fail-fast`가 0으로 끝나는 것이다.

Remediation은 commit으로 닫혔다. 현재 correction feature는 focused
offline gate와 실제 캡처의 read-only `--plan`, 새 `/tmp` Raw 루트를 사용한
one-shot ingest/parser 검증까지 통과했다. 최종 독립 감사도 HIGH/MEDIUM 0건으로
닫혔다. 실제 viewer bytes나 임시 Raw는 Git에 추가하지 않았다. Branch CI의 policy
failure는 기존 계약 불일치(`research-worker-smoke.sh`의 migration ledger 10 대 테스트
기대값 9)였고, 기대값을 실제 migration 수 10으로 맞춘 focused test는 6/6을
통과했다. DB-backed run은 `paper_preview`를 포함한 실패를 실제 노출했으므로 후속 QA
closure가 필요하지만, 이번 Stage6 focused review에서 새 HIGH/MEDIUM 회귀는 남지 않았다.

**KIND correction ordered-membership 구현.** 승인된 KIND correction-viewer 관찰은 list-anchor
acceptance `20200207000058`과 option raw value `20200207000081|Y`를 별도로
해소했을 뿐이다. 14자리 option
acceptance token은 `20200207000081`이다. 실제 다중 버전 correction chain을
증명하지 않는다. Rust ingest·normalizer·tests는
이 두 값을 별도로 보존하며, equality/join,
predecessor/supersedes/withdrawal, time/timezone 의미는 추론하지 않는다.
`|Y`는 opaque로 둔다. 번호에서 날짜를 파생하지 않고, 재구성 HTTP 요청이나
popup-state probing은 금지한다. 잘못된 후보 `20251204000324`는 ETF-list
provenance가 없어 거부됐다. 다음 증거 작업은 owner-gated 저빈도 범위에서 실제
복수 option을 가진 ETF viewer 표본을 확보할 수 있을 때만 진행하며, 그 전에는
“chain 추적 완료”나 predecessor/supersedes 관계를 주장하지 않는다.

**2026-08-20 소유자 결정과 bounded pilot 결과.** 소유자는 FSC mirror를 ETF11
authoritative identity 방향으로 선택하고, KIS event pilot은 `bonus-issue`에만
한정하며, KIND는 한 번의 5-calendar-day 저빈도 pilot만 승인했다. 이 선택은 FSC
endpoint 호출 허가가 아니다. 공식 활용가이드의 exact host/path/method/schema,
registration/key, entitlement reference/hash, 동일 ISIN의 두 과거 기준일 semantics가
확정되기 전에는 FSC adapter나 network call을 만들지 않는다. KIS bonus issue도
KIND availability와 deterministic relation이 없으면 기존 source evidence를 넘어
Curated/PIT로 승격하지 않는다.
이 historical identity 방향은 이후 D14에서 진단·control 소비 완료 및 ETF11 적용성
기각으로 대체됐다. 현재 상태는 아래 2026-08-21 기록을 따른다.

KIND pilot은 종료된 창 `2026-08-15..2026-08-19`를 own-controls capture로 정확히
한 번 실행했다. 40 stored pages 뒤 extra probe가 distinct response를 반환해
`page_bound_reached`로 종료했으므로 전체 staging은 incomplete이며 Raw ingest하지
않았다. 임시 staging의 captured portion을 오프라인 분류해 correction 표시와 exact
handler를 가진 opaque acceptance `20260819000134`, `20260819000124` 두 개를 찾았다.
첫 후보만 `2026-08-19` 단일 날짜 correction capture로 한 번 확인했지만 exact response
body에 target handler가 없어 `missing_target`으로 종료했고 viewer 파일은 생성되지
않았다. 두 번째 후보는 호출하지 않았다. 이 결과는 최근 구간에서 5일 창도 안전한
상한이 아니며 더 좁은 window 승인이 필요하다는 운영 근거일 뿐, correction chain
근거가 아니다. 모든 staging은 `/tmp`에만 있고 Git/Raw/DB에 들어가지 않았다.
이후 list capture CLI도 correction capture와 마찬가지로 exact operator confirmation을
browser launch 전에 요구하며, impossible calendar date를 양쪽 CLI에서 거절한다.

**KIS `bonus-issue` 공식 명세 보정.** 로컬 공식 XLSX의
`예탁원정보(무상증자일정)` sheet를 오프라인 검토했다. `fix_rate`는 소수 비율이
아니라 퍼센트이므로 canonical split factor는 정확한 십진수 연산
`1 + fix_rate * 0.01`로 계산한다. 기존 구현의 `1 + fix_rate`는 공식 예시
`100.00`을 factor `101`로 만들 수 있어 수정했고, `100.00 -> 2.0000` focused
회귀 테스트가 통과했다. 같은 명세는 이 데이터가 KSD 제공 일정임을 확인하지만
공표시각, revision/predecessor/supersedes, correction lineage, 기계 판독 ISIN code는
제공하지 않는다. 따라서 KIS retrieval time만 availability로 유지하며 deterministic
KIND relation 전에는 Curated/PIT 승격이 계속 차단된다. 이 검토에서 KIS/provider
요청은 하지 않았다.

**Direct-main CI baseline 보정 진행 상태.** run `32362557843`은 아래 저장소 내부
회귀를 구체적으로 노출했다. 현재 worktree에서 다음을 보정했다.

- backtest publication recheck의 `LEFT JOIN ... FOR UPDATE`는 nullable join 쪽까지
  잠그지 않도록 `FOR UPDATE OF backtest_runs`로 한정했다;
- Phase-0 생성기, runner path preflight, factor reader가 각각 다르게 사용하던
  `curated/curated` 이중 경로를 `data root/curated` 단일 계약으로 통일했다;
- cumulative artifact attestation 도입 뒤 빈 `artifacts`를 유지하던 candidate,
  recommendation, Paper QA manifest는 실제 Parquet bytes/path/size/schema/SHA-256에서
  exact artifact set을 만들도록 공용 test helper로 갱신했다;
- Paper fixture의 manifest 경로 이중 join을 제거했고, missing-close 테스트는
  attested 파일 삭제가 아니라 유효한 manifest 아래 요청일 행 부재를 구성해 실제
  close-reader 분기를 검증하도록 했다;
- `price_dataset_entitlement_is_valid(uuid,text,date,date)`를 5개 인자로 호출하던
  stale collector 호출을 4개 인자 계약으로 맞췄고, credentialed candidate fixture의
  required uses와 recovery fixture의 instrument FK seed를 보완했다.

이 변경 뒤 `scripts.ci.test_prepare_phase0` 3/3,
`scripts.ci.test_ci_contract` 6/6, `recommendation_compute` 16/16,
backtest path focused test 1/1, `market-data` 357/357, `cargo fmt --check`,
`git diff --check`, workspace all-target/all-feature `--no-run`이 통과했다.
이후 run `32390587037`에서 기존 31개 workspace 실패 중 20개와 PostgreSQL
integration 6개가 닫혔다. 남은 workspace 실패 11개는 (1) Python isolated
worker/golden 경로가 `data_root/curated/bars`에 `curated`를 다시 붙인 backtest 8개,
(2) candidate 실행 역할 `worker`에 읽기 전용
`price_dataset_entitlement_is_valid(uuid,text,date,date)` 실행 권한이 없어
`CANDIDATE_INPUT_UNAVAILABLE`로 닫힌 candidate/API 3개였다. Python 경로 계약을
`data_root` 기준으로 통일했고, 적용된 0046을 수정하지 않고 가역적 0047 migration으로
그 Boolean attestation 함수만 `worker`에 위임했다. `app`/`admin`/`audit_writer`와
직접 entitlement table 접근은 계속 거부한다. 로컬 focused 검증은 Python 35/35,
candidate runner 2/2, candidate HTTP 9/9, migration contract 28/28을 통과했다.

Research smoke의 별도 interpolation 결함도 production Compose의 필수값 계약을
완화하지 않고 Bash/PowerShell QA 스크립트가 동일한 deterministic non-secret UUID를
functional config에 공급하도록 수정했다. CI contract 6/6, Bash static/self-test,
문법 검사는 통과했다. 이 호스트는 Docker daemon과 PowerShell이 없어 functional
Compose/PowerShell 실행은 후속 CI 증거를 사용한다. 전체 green 최종 판정도 후속
`main` CI와 research-smoke가 이 변경을 실제 실행한 결과로만 내린다.

후속 commit `6e8dfbf`의 CI run `32392991765`는 policy, format, web, strict
Clippy, workspace 전체 테스트, PostgreSQL migration/role boundary와 aggregate
required job까지 모두 통과했다. 같은 push의 research-smoke run `32392991751`는
앞선 Compose interpolation 오류를 지나 image build까지 진행했지만,
candidate-runner builder에서 `market-data`가 `include_bytes!`로 요구하는 tracked
XKRX calendar 3개와 승인 manifest 1개를 해당 Dockerfile이 복사하지 않아 실패했다.
`.dockerignore`는 이미 이 네 파일만 정확히 허용하고 있었으므로 build-context 범위를
넓히지 않고, `market-data`를 컴파일할 수 있는 모든 production Rust Dockerfile에 네
exact `COPY`를 추가했다. Research smoke와 production-image static/self-test도 이
계약을 검사한다. 로컬 static/self-test는 통과했고 실제 Docker release build의 최종
판정은 다음 `main` research-smoke 결과로만 내린다.

후속 commit `d409fcd`의 CI run `32396309662`도 required job을 포함해 전부
통과했다. 같은 push의 research-smoke run `32396309756`에서는 앞선 네 파일의
Docker build-context 결함이 닫혀 `research-worker`와 `candidate-runner` release
image가 모두 빌드됐다. 그 다음 실제 one-shot에서 candidate source publication이
`CANDIDATE_PIPELINE_FAILED`로 닫혔다. 원인은 synthetic entitlement가 주석과 달리
`candidate` use만 가지고 있어 가격 publication이 먼저 권리 부족으로 BLOCKED가
됐고, worker가 의도적으로 자동 수행하지 않는 Raw-reference instrument registration도
smoke가 생략해 source row의 instrument FK를 만족하지 못한 것이다. 운영 경계를
완화하지 않고, smoke entitlement에 가격의 네 exact downstream use를 추가하고,
immutable synthetic Raw의 reference hash·batch id·retrieval instant·calendar first
session·instrument master에서 인자를 유도해 `research_writer`의 좁은
`register_candidate_instrument` definer를 publication 전에 호출하도록 보완했다.
보완 commit `e1c3b4b`의 CI run `32400065805`는 policy, format, strict Clippy,
web, workspace 전체 테스트, PostgreSQL migration/role boundary와 required aggregate를
모두 통과했다. 같은 commit의 research-smoke run `32400065786`도 static PASS,
`research-worker`/`candidate-runner` release image build, 실제 synthetic price/source
publication과 두 universe candidate feed를 거쳐 최종 `RESEARCH_WORKER_SMOKE:
functional PASS`로 종료했다. 이전 `CANDIDATE_PIPELINE_FAILED`는 재현되지 않았다.
공식 KIS XLSX는 계속 untracked로 제외했고 provider/browser/live API 호출은 없었다.

**KIS entitlement 갱신(2026-08-21).** 개인 단독 사용 권리는 소유자 확정으로
해소됐고 앞으로 재질문하지 않는다. `kis:owner-attestation:personal-single-user:2026-08-21`
결정문을 SHA-256으로 고정한 ACTIVE metadata에는 `usr_owner` 한 명만 포함했다.
Member-visible KR-derived surface와 Live는 이 결정에 포함되지 않는다.

**FSC KRX상장종목정보 ETF11 적용성 종료(2026-08-21).** 소유자 승인 key와 공식
활용가이드로 exact `GET /1160100/service/GetKrxListedInfoService/getItemInfo`, JSON
`response.header/body`, `basDt`/`isinCd`, `resultType=json`, `A`-prefixed 단축코드
계약을 구현·검증했다. 069500/KR7069500007을 검증된 최근 XKRX 거래일 5개
(`08-20`, `08-19`, `08-18`, `08-14`, `08-13`)에서 비저장 조회했으나 모두
관측 0건이었다. 같은 서비스·`2026-08-19`에서 공식 예제 주식
000020/KR7000020008을 정확히 한 번 조회한 control은 관측됐다. 따라서 key·서비스·
날짜 문제가 아니라 최소 한 개의 필수 ETF가 빠진 것이며, 이 미러는 고정 ETF11의
완전한 identity 원천으로 사용할 수 없다. 응답 본문·provider prose·key는 출력/저장하지
않았고 FSC Raw batch/manifest도 생성되지 않았다(빈 commit lock만 존재). 임시 sudo
grant는 control 성공 직후 자동 회수됐다. 이는 모든 ETF의 영구 부재 주장이 아니라
ETF11 completeness 기각이며, fuzzy/bulk 확인으로 넓히지 않는다.

**FSC 진단·control 소비 완료.** 위 다섯 ETF probe와 정확히 한 번의 non-ETF control은
역사적이고 완전히 소비됐다. FSC `KRX상장종목정보`에 대한 추가 live query나 Raw 수집은
새 명시적 소유자 승인이 없으면 금지한다. 오프라인 fixture/contract 코드는 남을 수 있지만
그 존재가 live 권한을 만들지 않는다.

**가격·거래량 소스 결정(2026-08-21).** KIS read-only daily bars와 reference quotes를
고정 ETF11의 primary price/volume source로 사용한다. 다른 price API를 추가하는 것은
독립적인 cross-check 또는 fallback이라는 목적, 우선순위, failure behavior를 소유자가
명시적으로 승인하고 해당 공식 계약·focused test를 추가한 경우에만 가능하다. FSC의
겹치는 `증권상품시세정보`를 기본 후속 경로로 취급하지 않는다.

**운영 activation 결정.** KIS once-daily incremental EOD는 소유자 승인 범위다. runtime은
commit-suffixed immutable-release unit을 만드는 전용 installer로만 설치하며, 깨진 static
KIS unit/env 파일은 남기거나 대체 경로로 사용하지 않는다. KIND D11은 low-volume manual
operator-gated one-day capture만 허용하며 scheduled/timer/bulk/full-history 실행은 금지한다.

**결정 뒤에도 남은 외부 입력.** 다음 항목은 추측해서 진행하지 않는다.

- FSC `KRX상장종목정보`: registration/key/공식 guide/live control은 해소됐지만 ETF11
  completeness가 기각돼 identity 경로는 종료·소비됐다. 추가 live/Raw는 새 명시적 owner
  approval 없이는 금지한다. `증권상품시세정보`를 포함한 다른 price API는 independent
  cross-check/fallback 목적, priority, failure behavior를 먼저 승인받아야 한다. fuzzy
  name join은 계속 금지한다.
- KIS `bonus-issue`: 공식 field/unit과 lineage 부재는 로컬 KIS XLSX 검토로
  확정했다. 이제 남은 것은 KIND availability와의 deterministic relation이다.
  다른 `ksdinfo` event와 direct KSD/KSD portal은 이번 결정에 포함되지 않는다.
- KIND 추가 수집: 이번 1회 pilot 승인은 소비됐다. 전체 백필 추정 약 475
  captures/15,200 requests/20시간은 계속 승인 밖이며, 추가 window·요청 예산·보존
  위치는 다시 명시적으로 승인한다.

위 결정과 KIND 정정 관계가 확보된 뒤에만 identity/event cross-validation,
fail-closed Curated 후보, pilot backfill, DatasetManifest/DB publication/five-pin,
recommendation/backtest/Paper 연결 순서로 진행한다. 그 전까지 계약은 계속
`vendor_snapshot=true`, `strict_pit=false`, `ready=false`다. OpenDART TLS는
ETF11 critical path가 아니므로 개별주식 범위가 별도 승인되지 않는 한 다음
작업에 포함하지 않는다.

### 0.15 공식 소스 수집기 구현·검증과 main 반영 완료 (2026-08-21)

**Git 전달 상태.** canonical repository `/data/workspace/lagrange`의 `main`에
`3e349bc6e0fd351e1ca2f10856efa2f94a19641d` (`feat(stage6): add bounded
official-source collectors`)를 생성해 `origin/main`으로 push했고, 두 ref가 같은
커밋임을 확인했다. 개발용 `unspecified-task`의 세 커밋은 이미
`3ebc288` 병합을 통해 `main`의 조상이므로 중복 병합 커밋을 만들지 않았다.
작업 전부터 있던 공식 KIS XLSX
`docs/kis_openapi_entiredocs_20260818_030007.xlsx`는 의도한 untracked 파일로
그대로 보존했으며 이번 커밋에 포함하지 않았다.

**이번 커밋으로 완결한 저장소 내부 경계.** 비밀값은 코드·Git·로그·이 문서에
저장하지 않는다.

| 소스/경로 | 현재 구현과 허용 범위 | 아직 하지 않은 것 |
|---|---|---|
| KIS 일일 시세 | `kis-daily-production.sh`, commit-suffixed immutable-release installer와 XKRX calendar refresh를 추가했다. 하루 한 번 `16:30 Asia/Seoul`, `Persistent=true`, 단일 lock, 보호 state, 기존 KIS secret 재사용, read-only EOD 경로만 허용한다. `--plan`/`--check`는 provider를 호출하지 않고 installer `--apply`도 service/timer를 시작하지 않는다. | 실제 release 설치·timer 활성화와 credentialed read-only 첫 실행은 아직 수행하지 않았다. 주문·계좌·잔고·Live profile은 계속 금지한다. |
| KIS KSD 기업행사 범위 | 공식 6 endpoint를 paid-in 두 구분을 포함한 7 logical class로 고정했다. ETF11 기본 scope는 종목 11개 × class 7개 = 초기 77 GET을 순차 실행하며, class별 10-page bound, exact continuation, duplicate bytes·schema·symbol 검증과 전체 원자 Raw commit을 적용한다. | 이번 closure에서 live provider 호출이나 Raw backfill은 하지 않았다. symbol-scoped/multi-page batch는 아직 `bridge-v1` 입력이며, canonical mapping은 bonus issue만 존재한다. 나머지 nonempty class는 fail-closed blocker다. |
| KIND | 날짜별 명시된 opaque 14자리 후보만 받는 manual one-day wrapper, 별도 Raw 권한 경계, `kind-normalize` 실행 파일, 설치/check/self-test와 manual oneshot service를 추가했다. capture → immutable Raw → normalization을 기존 검증기와 연결한다. | timer·scheduled·bulk·full-history 수집은 없다. 추가 browser 실행은 D11의 저빈도 operator 승인 범위에서만 가능하다. |
| 금융위 `KRX상장종목정보` | 별도 `data-go-client`, FSC Raw adapter와 offline CLI를 추가했다. exact endpoint/query, 고정 `resultType=json`, private transport 단계의 `serviceKey`, bounded response/pagination, typed error, key 비노출과 `CandidateMaster` Raw 격리를 검증했다. | §0.14의 ETF11 적용성 기각이 최종 결정이다. 진단/control은 소비됐고 추가 live query·Raw 수집·가격 API 전환은 새 명시적 승인 없이는 금지한다. |

**최종 로컬 검증.** 외부 API·브라우저·운영 DB·systemd는 호출하지 않았다.

- shell syntax와 FSC/KIND/KIS daily provider-free self-test: 모두 PASS
- `data-go-client` 13/13, market-data FSC Raw 6/6, KIS action range 8/8
- collectors FSC CLI 3/3, KIND normalizer CLI 2/2, KIS action-range CLI 3/3
  — focused Rust 합계 35/35 PASS
- `cargo fmt --all -- --check`, workspace all-target/all-feature Clippy
  `-D warnings`, `cargo test --workspace --all-targets --all-features --no-run`,
  `scripts/ops/static-check.sh`, `git diff --check`: 모두 PASS
- `/tmp`의 user quota 때문에 첫 KIND self-test가 파일 쓰기 전에
  `Disk quota exceeded`로 중단됐다. 코드 실패가 아니며 `/data` 아래 새 임시
  디렉터리로 `TMPDIR`만 옮겨 동일 검증을 재실행해 PASS했고, 생성한 빈 임시
  디렉터리는 제거했다. 사용자 소유 `/tmp` 파일은 삭제하지 않았다.

**현재 출시 판정과 다음 순서.** 코드와 오프라인 운영 계약은 `main`에 있지만
실제 운영 activation과 새 데이터 증거는 아직 없다. 따라서 현재 판정은 계속
`vendor_snapshot=true`, `strict_pit=false`, `ready=false`다. 다음 순서는
(1) exact `3e349bc` immutable release 배포, (2) KIS daily installer preflight/check와
16:30 이전 timer 설치, (3) read-only 첫 credentialed daily 실행 및 Raw/정규화/DB
freshness 확인, (4) 별도 operator 확인 아래 ETF11 KIS action-range 최초 Raw 수집,
(5) `bridge-v1`과 KIND availability 증거가 확보된 유형만 canonical/Curated 후보로
승격, (6) DatasetManifest·DB publication·five-pin 후 recommendation/backtest/Paper
연결이다. FSC 상장종목 미러나 OpenDART를 ETF11 identity 경로로 다시 열지 않는다.

### 0.16 독립 read-only 검토 — 미해결 항목 3건 (2026-08-21)

§0.15 이후 상태(`06937a8`, `main` = `origin/main`)를 대상으로 한 독립 검토다.
**provider·브라우저·운영 DB·systemd·CI를 호출하지 않았고 코드도 변경하지 않았다.**
이 검토 환경에서는 shell이 동작하지 않아 확인 수단이 파일 직접 읽기뿐이었다.
따라서 어떤 테스트·게이트도 실행하지 않았으며 green·READY·PIT 주장을 하지
않는다. 아래 3건은 문서 교차검증과 해당 파일 직접 확인으로만 뒷받침된다. 전체
트리에 대한 `TODO`/`unimplemented!` 전수 조사는 검색 도구 부재로 수행하지
못했으므로 이 3건이 미해결 항목의 전부라고 주장하지 않는다.

**A. `CurateStore` 이중 경로가 Paper 런타임에 남아 있다 — §0.9의 지뢰 미해소.**
계약은 `crates/market-data/src/curate.rs:260-265`에 명시돼 있다 —
`CurateStore::new(root)`의 `root`는 `data/` 루트이고 `bars_path()`가 `curated`를
스스로 붙인다. 그런데 `crates/job-queue/src/paper_valuation.rs:362`
(`session_closes`)와 `crates/job-queue/src/paper_execution.rs:683`
(`session_opens`)는 계속 `CurateStore::new(dataset_root.join("curated"))`를 쓴다.
호출자가 넘기는 `dataset_root`가 `data/` 루트라면 두 경로는
`dataset_root/curated/curated/...`를 읽는다. §0.14의 경로 통일은 Phase-0 생성기·
runner path preflight·factor reader·Paper fixture manifest를 대상으로 했고 이 두
런타임 호출부는 포함하지 않았다. 대응 테스트 fixture도 같은 이중 join이라 읽기·
쓰기가 자체 정합이므로 **현재 CI green은 이 자리를 덮지 않는다.** 두 함수는 Paper
체결·종가 평가 경로이므로, 통일된 writer가 만든 실제 curated 트리를 처음 읽는
시점에 `MissingPrice`/`MissingMark`로 닫힐 수 있다. DB와 실데이터가 없어
재현하지 않았고 코드도 고치지 않았다. **§0.15 (3)에서 실데이터가 들어오면 재현
가능하며 (6)의 Paper 연결 전에 닫아야 한다.**

**B. 리밸런싱 미리보기 UI 부재를 확인했다 — §4 항목 7 유효.** 백엔드 표면은
존재한다(`POST /paper/accounts/{id}/recommendation-previews`,
`GET .../{preview_id}`, `POST .../apply`). 그러나
`apps/web/app/(authenticated)/paper/page.tsx`는 holdings·performance·parity·
lineage·bind·notifications만 조립하고 preview를 호출하는 코드가 없다. 따라서
소유자가 추천을 Paper에 적용하기 전 미리보기를 화면에서 확인할 경로가 아직 없다.
§2.5가 기록한 "UI와 Live 주문은 포함하지 않는다"는 `06937a8` 기준으로도 그대로다.

**C. CI가 Python 스위트를 실행하지 않는다 — 탐지 공백.**
`.github/workflows/ci.yml`의 `workspace-tests`는 `pyarrow==25.0.0`/`uv==0.12.1`
설치와 `uv sync --project nt --locked`로 환경만 구성한 뒤
`cargo test --workspace --locked --no-fail-fast`만 실행한다. `pytest` 스텝이 두
workflow 어디에도 없다. 즉 `nt/*/tests/`와 `tests/golden/`(Phase 0 게이트, 5전략
robustness 골든)은 CI 밖이다. 반면 `scripts/verify-all.sh`는 스스로를
"CI / clean containers" 용도로 선언하고 "Every gate is a hard gate"라 적으면서
10단계에 `uv run --project nt pytest -q`, 11단계에 백업 정책 테스트를 포함한다 —
그런데 **어느 workflow도 이 스크립트를 호출하지 않고 CI가 부분집합을 따로
재구현했다.** 미래정보 참조 금지가 §14 위험표 1순위인 저장소에서 골든 회귀가
자동 탐지되지 않는다는 뜻이다. 함께 관찰한 것: `research-smoke.yml`은 `push`/
`workflow_dispatch` 전용이고 `ci.yml`의 `required` 집계에 없으므로 PR을 막지
못한다. `scripts/qa/`의 `phase1|phase2|phase3-gate.sh`, `full-system-gate.sh`,
`failure-suite.sh`, `nginx-hardening.sh`와 `deploy/nginx/auth-route-static-check.sh`,
`deploy/systemd/*-static-check.sh`, `deploy/secrets/runtime-static-check.sh`는 두
workflow에서 직접 참조되지 않는다(다른 스크립트를 통한 간접 호출까지 배제하지는
않았다). `research-worker-smoke.sh`의 `wsl`/`wslpath` 분기는 §2.8이 지적한 뒤에도
남아 있다.

**출시 판정에 대한 영향.** A·B·C 어느 것도 §0.15의 순서나 계약 flag를 바꾸지
않는다. 판정은 계속 `vendor_snapshot=true`, `strict_pit=false`, `ready=false`다.
A는 (6) 이전에 닫을 항목, B는 사용자 가시 기능의 잔여 항목, C는 게이트가 아니라
탐지 수단의 공백이다. 또한 **§2.1 표는 2026-08-17 실행의 역사 증거이며, 08-21
ADR-0005의 ACTIVE KIS metadata로 E1을 재평가한 게이트 재실행은 아직 수행되지
않았다** — 이 재실행은 소유자 결정을 요구하지 않는 유일한 순수 실행 항목이다.

### 0.17 §0.16 A·C 종결과 잔여 코드 작업 4건 완료 (2026-08-22)

`assess-project-status` 브랜치에 네 커밋. provider·브라우저·운영 DB·systemd·CI를
호출하지 않았고 secret을 만들지 않았다. **판정은 그대로 `vendor_snapshot=true`,
`strict_pit=false`, `ready=false`** — 이 작업 중 어느 것도 계약 flag를 바꾸지 않는다.

| 커밋 | 내용 |
|---|---|
| `8e77c7a` | **§0.16 A 종결.** `session_closes`/`session_opens`의 `curated` 이중 join 제거. 픽스처 3곳도 실제 writer 레이아웃으로 정정 |
| `7b07522` | **§0.16 C 종결.** `python-tests` job 신설 + lockfile 검사 + verify-all 드리프트 가드 |
| `79da609` | **§4 항목 7 (=§0.16 B) 종결.** 리밸런싱 미리보기 UI |
| `6138075` | Stage4B evidence package 조립 CLI |

**A에서 실제로 산출한 것은 수정이 아니라 가드다.** 픽스처가 프로덕션과 같은 이중
join으로 *쓰고* 있었기 때문에 수정 전에도 후에도 스위트가 green이었다 — 즉 테스트가
정합성의 증거가 아니었다. 그래서 `session_closes`/`session_opens`를 실제로 호출하고
`curated/curated`가 없음을 단언하는 가드 테스트를 사이트별로 추가했고, **수정 전 트리에서
두 테스트가 실패하는 것을 먼저 확인한 뒤** 프로덕션을 뒤집었다. 이 순서를 지키지 않으면
그 테스트가 무엇을 지키는지 증명할 수 없다.

**C도 배선보다 드리프트 가드가 본체다.** `verify-all.sh`는 자신을 CI용 하드 게이트라고
선언하는데 어느 workflow도 부르지 않고 CI가 부분집합을 재구현했다. 부분집합을 또 늘리는
대신, `test_ci_contract.py`가 verify-all의 각 게이트에 CI 대응물이 있는지와 게이트 수가
매핑 표와 일치하는지를 단언한다. 가드가 실제로 실패하는지는 CI 스텝을 지워보고 확인했다.

**검증 (코디네이터 독립 실행).**

- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features
  --locked -- -D warnings`, `git diff --check`: 모두 PASS
- `uv run --project nt pytest -q`: **357 passed, 2 skipped, 0 failed**
- `python -m unittest scripts.ci.test_ci_contract`: 7/7 PASS
- web `lint`/`typecheck`/`test`/`build`: clean, **75/75 (16 files)**
- `configs/evidence/kis-range-canonical-approved-manifests.json` 무변경 확인
- `cargo test --workspace`: 2개 target 실패, **둘 다 환경 요인이며 이번 변경과 무관함을
  증명했다.** `job-queue::recommendation_compute`는 테스트가 시스템 `python`으로
  `prepare_phase0.py`를 부르는데 거기에 `pyarrow`가 없어서 실패했다 — `PYTHON`을
  `nt/.venv`로 지정하니 **16/16 PASS**. `collectors::research_worker`는 6건이
  `required QA DATABASE_URL ... NotPresent`이며 Docker QA DB가 이 호스트에 없다
  (62 passed / 6 blocked). 해당 파일은 이번에 건드리지 않았다.

**Stage4B CLI의 한계를 명시한다.** `write_evidence_package`는 로더가 역직렬화하는 바로
그 비공개 타입을 직렬화하고 로더 자신의 검증기(`load_action_evidence` 포함)를 먼저
돌리므로, 로드 불가능한 패키지는 애초에 써지지 않는다. 그러나 **승인 목록이 빈 배열인
한 이 CLI의 산출물은 fixture 밖에서 로드되지 않는다.** 이는 결함이 아니라 설계된
게이트다. 다음 단계는 소유자가 출력된 `manifest_sha256`을 검토해 승인 목록에 커밋하는
것이며, 그 행위는 이 작업 범위 밖이다.

**작업 환경에서 치른 비용 하나.** 병렬 워커의 Rust 빌드가 `/tmp`(tmpfs, 7.3G, `usrquota`)
사용자 쿼터를 소진시켜 모든 에이전트의 셸이 `EDQUOT`로 동시에 죽었다. 명령이 출력 없이
exit 1로만 실패해 원인 파악이 늦었다. §5의 함정 표와 같은 계열이며, `TMPDIR`을 `/data`로
옮기는 것만으로는 부족하다 — cargo/rustc가 일부 경로에서 `/tmp`을 직접 쓴다.

### 0.18 §0.17 독립 리뷰와 결함 8건 수정 (2026-08-22)

§0.17의 네 커밋을 커밋별 독립 적대적 리뷰에 걸었다. **세 리뷰 모두
PASS-WITH-FINDINGS였고 MAJOR 6건이 나왔다.** 리뷰를 돌리지 않았으면 전부 출시에
실려 갔을 것들이다. 수정 커밋 `733aca2`, `e16b9da`, `0b34b9f`, 그리고 CI 수정 `f1117fb`.

**가장 중요한 발견 — 미래정보 가드가 실행된 적이 없었다 (`733aca2`).**
`crates/job-queue/src/factor_series.rs:591`이 `data/phase0/curated`에 다시
`curated/bars/...`를 붙여 존재할 수 없는 경로를 검사했다. `phase0_root()`가 항상 `None`을
반환해 **테스트 4개가 아무것도 단언하지 않고 반환**했고, 그중 둘은 bare `return`이라 출력에서
통과와 구분조차 되지 않았다. 그 4개에 `a_factor_value_does_not_change_when_later_bars_exist`
— 주석 스스로 "설계 전체가 딛고 선 속성… 팩터가 trailing이 아니게 되면 하류 어떤 단언도
알아채지 못하는 방식으로 시리즈 전체가 미래정보에 오염된다"고 적은 가드 — 가 포함돼 있었다.
**§0.16 A와 정확히 같은 결함 계열이고, §0.14가 "factor reader를 커버했다"고 적은 그 파일이다**
(`:254/285/368/475`는 고쳤고 `:591`을 놓쳤다). 넷 다 이제 실행되고 전부 통과하므로 뒤에
숨은 결함은 없었다 — 가드가 꺼져 있었을 뿐이다. 증거는 실행 시간이다: 이전 `0.00s`에 SKIP
2줄, 이후 `0.08s`에 SKIP 0줄.

**Stage4B 조립기가 로드 불가능한 패키지를 쓸 수 있었다 (`0b34b9f`).** §0.17이 "로드 불가능한
패키지는 애초에 써지지 않는다"고 적었는데 **거짓이었다.** writer가 `load_action_evidence`는
부르고 짝인 `validate_actions`를 부르지 않았다. 둘은 의도적으로 갈라져 있다 — 앞은 nonempty
non-bonus 응답을 `RangeAction::Unsupported`로 **받아들이고**, 뒤가 그걸 거부한다. 그래서
writer가 해시를 출력하고, 소유자가 그 해시를 승인 목록에 커밋하고, 그제서야 로더가 거부하는
경로가 열려 있었다. 저장소에 단 하나뿐인 수작업 게이트 아티팩트에 **영구히 죽은 pin**을 남기는
것이며 원칙 5가 막으려는 바로 그 일이다. whole-market 범위에서 dividend가 nonempty인 것은
거의 확실하므로 예외 경로가 아니라 주 경로였다. 배치 매처에도 같은 불일치가 있었다 — 로더는
정확히 7파일을 요구하는데 매처는 "7종류 식별"만 봐서 8파일짜리 일일 EOD 번들이 후보가 됐고,
유효한 7파일 배치와 나란히 있으면 **둘 다 ambiguous로 거부**했다. 둘 다 수정 전 실패하는
테스트로 재현했다.

**Paper 페이지가 무관한 권한 하나로 전체가 닫혔다 (`e16b9da`).** §0.17이 추가한
`getRecommendationRuns()`가 페이지의 `Promise.all`에 들어갔는데, 이 엔드포인트만 `recommendation`
use이고 나머지는 전부 `paper_view`다. 즉 선택적 드롭다운 하나가 거부되면 보유·성과·parity·
lineage·알림이 **전부** blocked 셸로 대체됐다. 리뷰어가 probe 테스트로 실증했다. 또한 섹션이
`can_manage`(레코드 소유)로 게이트됐는데 엔드포인트 3개는 Owner **역할**을 요구한다 — member도
자기 Paper 계좌를 만들 수 있으므로 **5명 중 4명에게 전 기능이 403**이었을 것이다. 계좌 전환 시
이전 계좌의 READY 미리보기가 Apply 활성 상태로 남는 문제도 함께 닫았다.

**CI가 첫 실행에서 빨갛게 났고 즉시 고쳤다 (`f1117fb`).** `python-tests` job이 Phase 0 골든
테스트 3건에서 실패했다. 그 테스트들이 manifest에 pin된 code commit을 `git rev-parse`로
해석하는데 그 커밋(`9f319ca`)이 HEAD보다 236커밋 뒤라 `actions/checkout` 기본 depth-1
클론에는 없다. **드리프트가 아니라 클론 아티팩트를 검사하고 있었다.** `--depth 1` 클론에서
동일 실패를 재현하고 같은 클론을 `--unshallow`하니 통과하는 것으로 확정했다. `fetch-depth: 0`.

**리뷰가 확인해준 것도 기록한다.** 승인 목록 게이트는 뚫리지 않았다(`#[cfg(test)]` 핀 로더 유지,
매니페스트 구조체 비공개 유지, 승인 목록 무변경). 미리보기 UI의 zod 스키마 7개는 openapi.json과
필드 단위로 정확히 일치하고, 정밀 소수 문자열이 `number`로 변환되는 곳은 한 군데도 없다.
§0.17 A의 가드 테스트 2개는 **양방향 모두** load-bearing임이 변이 테스트로 증명됐다 —
특히 프로덕션과 픽스처를 함께 뒤집는 경우를 잡는 것은 `!doubled.exists()` 단언 하나뿐이다.

**최종 검증 (전체 재실행).** `cargo fmt`/`git diff --check`/workspace clippy `-D warnings`
PASS. `cargo test --workspace`는 환경 요인 6건(`research_worker`, QA `DATABASE_URL` 부재)만
실패하며 개수 불변. web `lint`/`typecheck`/`build` clean, **86/86**(75→86). pytest **357
passed / 2 skipped**. CI contract 7/7. **GitHub Actions `main` 전 job green.**

### 0.19 탐지 공백 일괄 폐쇄 — 재발 방지 가드와 CI 커버리지 (2026-08-22)

§0.18의 리뷰가 드러낸 것은 개별 결함 6건만이 아니었다. **"게이트라고 선언해놓고 아무도
실행하지 않는다"는 같은 패턴이 세 군데 더 있었다.** 커밋 `6c6e771`, `a6a9438`,
`ba5735b`, `6d54f59`. 전부 `main`에 반영됐고 CI 8 job 전부 green이다.

**1. curated 이중 경로에 저장소 전역 불변식을 걸었다 (`6c6e771`).** 이 결함은 네 번
나왔다 — Paper 리더 2곳, 픽스처 3곳, 그리고 `factor_series.rs:591`. 매번 그 자리만
고쳤고 재발 금지 규칙은 없었다. 그래서 두 겹짜리 가드를 넣었다. 소스 스캔은
`CurateStore::new(`의 **괄호 균형 인자**를 뜯어 마지막 경로 세그먼트가 `curated`인 문자열
리터럴을 거부한다 — 부분문자열이 아니라 리터럴만 보므로 이름만 curated인
`config.curated_root`(실제 값은 data 루트)를 오탐하지 않고, 전체 일치가 아니라 꼬리 일치라
`join("data/phase0/curated")` 같은 복합형도 잡는다. 파일시스템 절반은 생성된 curated 트리에
`curated/curated` 중첩이 없음을 단언하는데, **문자열 검색으로는 절대 못 찾는 두 홉짜리
형태를 잡는 것이 이쪽**이다. 허용 예외는 파일 단위가 아니라 **construction 단위**다 —
`let`으로 바인딩하고 15줄 안에서 `!binding.exists()`를 단언해야만 통과한다. 이중 경로로
읽으려는 코드는 그 단언을 할 수 없다(자기가 필요한 것의 부정이기 때문). 이미 허용된 파일
안에서 결함을 재도입해도 잡히는 것을 확인했다. 양쪽 절반 모두 재도입 시 실패하는 것을
증명하고 복원했다.

**2. e2e 7개 중 1개만 CI에서 돌고 있었다 (`a6a9438`, `ba5735b`).**
`scripts/qa/candidate-web-e2e.sh`가 `candidates.spec.ts` 하나만 실행했다. 돌지 않던 6개에
`no-member-live`와 `live-kill-switch` — **member가 Live 통제에 도달할 수 없음과 kill
switch 동작을 증명하는 스펙** — 이 들어 있었다. `playwright.config.ts`가 `workers: 1`,
`fullyParallel: false`이고 모든 스펙이 같은 synthetic API를 쓰므로 한 번에 직렬 실행이
구조적으로 안전하다. 배선 전에 7개 전부 로컬 실행해 **36 tests 전부 통과**를 확인했다
(하나라도 실패했으면 통과분만 배선하고 실패는 발견 사항으로 보고할 계획이었다). 함께
§0.18의 권한 격리 수정을 실제 브라우저 seam에 고정하는 스펙 2개를 추가했다 — 양방향
모두, 그리고 수정을 되돌리면 실패하는 것을 확인했다.

**3. static-check 22개 중 3개만 CI에 있었다 (`6d54f59`).** 배선되지 않은 것 중에 nginx
auth-route, systemd unit 2개, secret 파일 모드, migration 순서, 나머지 compose 검사가
있었다 — **출시가 딛고 서는 표면 그 자체**다. 10개를 추가했고, 이름이 아니라 **증거로**
골랐다: 깨끗한 체크아웃에서 각각 실행해 통과를 확인했고 Docker·DB·root·secret이 필요
없으며 수 초 안에 끝난다. 나머지 12개는 Docker/root/실 secret/live 호스트가 필요해
operator 실행으로 남긴다.

기록해둘 것 하나 — `scripts/ops/static-check.sh`와 `deploy/secrets/runtime-static-check.sh`는
**이 개발 호스트에서는 실패한다.** 둘 다 정확한 파일 모드를 요구하는데 이 호스트의
`umask 0002`가 체크아웃된 스크립트를 group-writable로 만들어 0755 대신 0775가 된다. git은
`100755`로 기록하고 러너는 umask 0022로 체크아웃하므로, 로컬에서 umask 0022로 클론하면
0755가 나오고 둘 다 통과한다 — 그리고 실제 CI에서 통과했다. **트리의 문제가 아니라 이
워크스테이션의 성질이다.** §5 함정 표에 추가할 만하다.

**검증.** 로컬: `cargo fmt`/`git diff --check` PASS, pytest **357 passed / 2 skipped**,
CI contract 7/7, web **86/86**, 새 가드 2/2. CI: **8 job 전부 green** — 특히 새로 배선한
`web`(e2e 7스펙)과 `policy`(static-check 10개)가 실제로 통과했다.

### 0.20 phase1 게이트 재실행 — VERDICT: APPROVED (2026-08-22)

§0.16이 "소유자 결정을 요구하지 않는 유일한 순수 실행 항목"이라 꼽고 계속 미뤄져 있던
게이트 재실행을 수행했다. 기준 트리 `3ddf1c1`.

| 검사 | 2026-08-17 (`61af2bb`) | 2026-08-22 (`3ddf1c1`) |
|---|---|---|
| **E1** written-rights | `BLOCKED_EXTERNAL` | **PASS** — `kis.entitlement.json` provider=kis ACTIVE, 문서 해시 `b888942df6b0...`, reference `repo://docs/decisions/0005-...` |
| E2 vendor-auth0 | PASS | **PASS** — 콜백 불일치를 고친 뒤(`2ec0460`) 통과 |
| E3 auth0-simulator | PASS | PASS (10) |
| E4 auth0-invite-mfa | PASS | PASS (52) |
| E5 phase1-five-user | PASS | PASS (5) |
| E6 restore-policy | PASS | PASS |
| E7 playwright-phase1 | PASS | PASS — chromium 4/4 |
| **VERDICT** | `BLOCKED_EXTERNAL_DATA_RIGHTS` | **`APPROVED`** |

**판정문 이름이 이제 오해를 부른다.** 게이트 NOTE가 적었듯 이 판정은 "written-rights가
ACTIVE가 아니거나 **vendor Auth0가 통과하지 못할 때**" 나온다. 이번 실행에서 막은 것은
**E2 하나뿐이고 data-rights가 아니다** — E1은 통과했다. 판정 문자열만 보고 권리가 여전히
막혀 있다고 읽으면 틀린다.

**E2도 이어서 닫았다 — 그리고 credential 문제가 아니었다 (`2ec0460`).** 소유자 승인 아래
`LAGRANGE_AUTH0_*`를 주입해 재실행하니 vendor 3개 중 2개는 통과하고
`vendor_authorize_endpoint_engages_oidc`만 실패했다 — `/authorize`가 302 대신 **403**.
**credential은 정상이었다**(confidential client 인증 테스트가 통과했다). 원인은 스위트가
보내던 `https://app.lagrange.local/auth/callback`이 테넌트에 등록돼 있지 않아서다.
`curl`로 직접 확인했다: placeholder는 403, 배포된 Tailscale 콜백은 **302**.

그 호스트는 저장소의 **명시된 placeholder**이고(`scripts/ops/validate-production-config.sh:153`이
production config에 남아 있으면 거부한다) §2.8에서 콜백을 Tailscale 주소로 옮길 때 목록에서
빠진 것으로 보인다. 즉 이 테스트는 "배포된 콜백이 동작한다"가 아니라 **"안 쓰는 placeholder가
아직 등록돼 있다"**를 검증하고 있었다. 콜백은 배포 종속 값이므로 domain·client_id와 똑같이
`LAGRANGE_AUTH0_REDIRECT_URI`로 받게 했다. Tailscale 호스트명을 저장소에 박으면 이 기계를
스위트에 고정시키고 주소가 바뀌면 또 깨진다. 미설정 시 fallback은 placeholder 그대로라
테넌트가 소리내어 거부한다 — 이 스위트는 조용히 skip하지 않는다.

**의도적으로 좁게 고쳤다.** `app.lagrange.local`은 저장소 약 40곳에 있지만 **실 테넌트에
접속하는 것은 이 파일 하나뿐**이다. 나머지는 오프라인 시뮬레이터라 값이 임의이고, compose
기본값은 production이 덮어쓰는 placeholder이며 `deploy/nginx/auth-route-static-check.sh`가
그 문자열을 정확히 고정하고 있다(=건드리면 방금 CI에 배선한 검사가 깨진다).

**결과: `VERDICT: APPROVED`.** E1~E7 전부 PASS. 이 저장소에서 phase1이 APPROVED를 낸 것은
처음이며, 08-17 실행은 `BLOCKED_EXTERNAL_DATA_RIGHTS`였다. 게이트 테스트 기대값 변경은
원칙 5의 명시적 행위이므로 커밋 메시지에 선언했다.

**부수 확인 — 밤새 "환경 요인"이라 분류한 것이 실제로 환경 요인이었다.** QA DB를 띄우고
`cargo test --workspace`를 재실행하니 **실패 0건**이다. `collectors::research_worker`의 6건은
`DATABASE_URL` 부재가 맞았고(68/68 통과), 다른 원인이 숨어 있지 않았다.

**환경 함정 둘을 새로 치렀다 (§5.1에 반영).**
1. docker 그룹이 `/etc/group`에는 있는데 에이전트 프로세스 트리가 물고 있지 않았다. UI
   세션을 리로드해도 부모인 Paseo 데몬이 낡은 그룹 집합을 유지해 반영되지 않는다.
   `sg docker -c '...'`로 획득하면 데몬 재시작 없이 해결된다.
2. 그런데 **`sg`는 setgid라 glibc가 `LD_LIBRARY_PATH`를 지운다.** E7의 chromium이
   `libasound.so.2` 부재로 죽는데 이 호스트에는 시스템 설치가 없고
   `/home/l1nnx/tools/pwlibs`의 사본을 `LD_LIBRARY_PATH`로 가리켜야 한다. 즉 게이트를
   `sg`로 감싸면 E7이 반드시 실패한다. phase1 게이트는 docker를 부르지 않으므로
   (QA DB가 이미 떠 있기를 요구할 뿐) **`sg` 없이 직접 실행해야 한다.** 이걸 모르면 E7
   실패를 코드 회귀로 오인한다 — 실제로 이번에 한 번 오인했다가 transcript를 읽고 정정했다.

### 0.21 첫 credentialed KIS 수집 — 실데이터가 들어왔고, 결함 5건과 벽 1건을 만났다 (2026-08-23)

§0.15가 미뤄둔 운영 activation (1)~(3)을 실제로 밟았다. **`--execute`가 KIS를 호출해
Raw를 커밋하고 정규화까지 성공했다.** 이 저장소에서 실데이터가 파이프라인을 통과한 것은
처음이다. DB publication 직전에서 멈췄고, 그 원인은 코드가 아니라 미등록 권리다.

**들어온 데이터 (2026-08-19분).**
`raw/provider=kis/market=kr/date=2026-08-19/batch=54421e32-...` 31파일 — calendar 1,
기업행사 7종, ETF11 시세와 reference. 정규화 결과는
`provider=kis-normalized/.../batch=4140c8d1-...`에 `bars/calendar/corporate-actions/reference`.

**§4.2-3 소유자 결정이 실측으로 해소 방향이 잡혔다.** 그 항목은 "KSD 비-bonus 기업행사가
ETF11에 실제로 발생하면 일일 실행이 닫힌다, 다만 발생 여부는 미확인"이었다. 이번 응답의
행 수는 dividend 0, merger-split 0, capital-decrease 0, reverse-split 0,
paidin-subscription 0, **bonus 1, paidin-record 1**이다. 즉 (a) **dividend는 실제로 비어
있었다** — 저장소가 추정한 "ETF는 배당이 아니라 분배금" 쪽이 맞는 방향이고, (b) non-bonus인
`paidin-record`가 1행 있었는데도 **정규화가 성공했다.** whole-market 응답의 타 발행사 행을
ETF11+대상일 필터가 정상적으로 걸러냈다는 뜻이다. 하루치 관측이므로 결정을 철회하지는
않되, "매일 터진다"는 전제는 성립하지 않는다.

**막힌 지점: `data_entitlements` 테이블이 비어 있다.** publication이
`PIPELINE_FAILED`(phase=publication)로 실패한다. §4.1이 "DB entitlement 등록은 별도 운영
provisioning"이라 적어둔 그 단계이며 결함이 아니다. **다만 여기서 멈췄다** —
`provision-entitlement.sh register`는 `lifecycle == "PENDING"` 레코드를 요구하는데
`configs/data-rights/kis.entitlement.json`은 이미 `ACTIVE`다. 통과시키려면 lifecycle을
PENDING으로 바꾼 사본을 만들어야 하는데 그것은 **검증기를 만족시키려 권리 상태를 위조하는
것**이고, `activate`의 activation-date도 소유자가 정할 값이다. 권리 증빙의 출처를 에이전트가
만들지 않는다. **소유자 작업으로 남긴다.**

**고친 결함 5건 — 전부 "작성됐으나 실행된 적 없음".**

| 커밋 | 결함 |
|---|---|
| `afa3e01` | `db_psql`이 컨테이너 안에 없는 `$POSTGRES_DB`를 읽어 `-d`가 비었고 libpq가 사용자명으로 폴백 → `database "migration_owner" does not exist`. 공유 헬퍼라 `register-dataset-version.sh`·`provision-entitlement.sh`도 같이 깨져 있었다 |
| `afa3e01` | snapshot SQL 9줄이 `$"..."`(로케일 번역 문법)이라 `\n`이 리터럴로 남아 psql 구문 오류 |
| `afa3e01` | `db_psql`에 `RANGE_RAW_BATCH_ID` 자리표시자 누락 — compose 전체 파싱이 무관한 Stage5 서비스에서 중단 |
| `3962f9d` | 상한을 `1_000_000`으로 써서 bash `[`가 `integer expected`로 죽고 **모든 실행이 "너무 크다"로 거부**됐다 |
| `e3d1739` | `backfill-production.sh`의 자체 compose 배열에도 같은 자리표시자 누락 |

**그리고 릴리스 배포로는 코드가 반영되지 않는다는 것을 늦게 알았다.**
`deploy-production-release.sh`는 소스를 `/opt/lagrange/releases/<commit>/`에 복사하고
`current`를 옮길 뿐 **이미지를 재빌드하지 않는다.** 컨테이너로 도는 Rust 워커는 2026-08-19
빌드 그대로였고, 그래서 `INVALID_CONFIG`의 원인을 최신 소스에서 찾느라 한참 헤맸다 —
재현 테스트가 통과한 것도 최신 소스로 돌렸기 때문이다. 오늘 고친 5건이 전부 셸 스크립트라
문제가 드러나지 않다가 컨테이너 안 Rust에 도달하자마자 튀어나왔다.
`build-production-images.sh --apply`로 재빌드하니 `INVALID_CONFIG`가 사라졌다. (**정정:** 빌드 목록은 11개지만 `reverse-proxy`는 digest-pin된 업스트림 이미지라 빌드 대상이 아니다 — 실제로 만들어진 것은 **10개**다.)

**정정:** 이 절의 초판은 "이미지에 소스 커밋 라벨이 없다"고 적었으나 **틀렸다.** 조회할 때
엉뚱한 이미지(nginx 베이스)를 지목한 실수였다. `data-pipelines/collectors/Dockerfile:31`이
`LABEL org.opencontainers.image.revision`과 `ENV LAGRANGE_CODE_COMMIT`을 모두 박고 있고,
실제로 `research-worker`·`api-server` 이미지에서 값이 확인된다. 따라서 진단 수단은 처음부터
있었고, 내가 그것을 쓰지 않아 낡은 바이너리를 못 알아본 것이다. 다음 세션은 컨테이너 동작이
소스와 어긋나 보이면 **먼저** 다음을 확인할 것:

    docker image inspect <image> --format '{{index .Config.Labels "org.opencontainers.image.revision"}}'

**두 번째 정정 (§0.23 리뷰):** 위에서 "`db-migrate`만 라벨이 없다"고 적은 것도 **틀렸다.**
라벨을 설정하는 Dockerfile은 세 개뿐이다(`crates/api-server/Dockerfile`,
`crates/job-queue/Dockerfile.backtest-runner`, `data-pipelines/collectors/Dockerfile`). 즉
빌드된 10개 중 **라벨이 있는 것은 4개**(api-server, research-worker, nt-backtest-worker-1/2)
뿐이고 **web·db-migrate·db-role-bootstrap·candidate-runner·recommendation-runner·
paper-scheduler 6개에는 없다.** 위에서 권한 진단 수단이라고 적은 그 명령이 함대의 다수에는
쓸 수 없다. 오늘 16:30 스냅샷 쿼리를 실행할 `db-migrate`도 그중 하나다.

그리고 릴리스를 배포해도 이미지는 자동으로 따라오지 않으므로 **배포 후 이미지 커밋과
릴리스 커밋이 어긋난 상태가 정상적으로 발생한다** — 이를 알려주는 검사는 없다.

**timer는 현재 릴리스로 재설치했다.** 처음 설치분이 옛 커밋(`b0576c8`)을 가리키고 있었고,
`--replace-existing`이 새 유닛을 넣으면서 **옛 유닛을 지우지 않아 둘 다 enabled**였다. 옛
유닛을 제거하고 `lagrange-kis-daily-e3d1739.timer`만 남겨 가동했다(다음 실행 16:30 KST).
entitlement가 등록되기 전까지 그 실행은 publication에서 fail-closed로 멈춘다.

---
### 0.22 내 수정이 main을 두 번 빨갛게 만들었고, 둘 다 어제 배선한 검사가 잡았다 (2026-08-23)

§0.21의 ops 수정 세 건(`afa3e01`, `3962f9d`, `e3d1739`)이 CI를 연속 실패시켰다. 두 원인 모두
**어제(§0.19) CI에 배선한 검사가 잡아낸 것**이다. 배선 첫 주에 실제 회귀를 두 건 잡았으니
그 작업의 값은 증명된 셈이다.

**1. `policy` 실패 — 고정된 SQL 술어가 깨졌다 (`cbb7357`).** `$"..."`를 `$'...'`로 바꾸면서
ANSI-C 인용이 SQL 작은따옴표를 전부 `\'`로 이스케이프하게 만들었고,
`scripts/ops/static-check.sh:613`이 고정 문자열로 찾는 `fetch_mode='credentialed'`가
매칭되지 않았다. 그 검사는 스냅샷이 credentialed 발행분만 세는지 지키는 것이라, 조용히
넓어지는 것을 막는 게 존재 이유다. **이스케이프를 다시 손보는 대신 quoted heredoc으로
바꿔 이 결함 계열 자체를 없앴다** — 개행과 작은따옴표가 모두 리터럴이라 로케일 번역 함정도
이스케이프 함정도 재현할 수 없다.

**2. `web` 실패 — e2e 하나가 CI에서만 깨졌다 (`be0922a`).** §0.19에서 e2e 7개를 전부
배선했는데, 배선 전 로컬 36/36을 확인했음에도 CI에서 `recommendations.spec.ts` 하나가
Playwright strict mode 위반으로 실패했다. `getByRole("status", …)`가 요소 2개에 매칭된다 —
`RecommendationRunForm`이 방금 제출한 run의 status를, `recommendations/page.tsx:109`가
`activeRun`의 status를 각각 렌더하기 때문이다. 둘 다 정상 동작이고, **둘이 동시에 pending인
순간에만** 모호해진다. 로컬은 빨라서 하나만 걸렸고 CI는 느려서 둘 다 걸렸다. 즉 이 테스트는
처음부터 모호했고 **CI에서 돌지 않아 드러나지 않았을 뿐**이다.

선택자를 폼이 속한 region으로 좁혔다. `.first()`로도 해소되지만 그건 제출이 아무 일도 하지
않았을 때 기존 `activeRun` status에 매칭돼 통과할 수 있어 **더 느슨해진다** — 이 단언의
목적이 "제출이 run을 만들었다"이므로 region 범위가 맞다. 로컬 재확인 36/36, CI green.

**기록해 둘 습관 하나.** 두 사고 모두 "로컬에서 통과했으니 됐다"가 틀린 경우였다. §0.19에서
"배선 전에 먼저 실행한다"는 규칙을 세웠는데, 그것만으로는 부족하고 **로컬 통과는 CI 통과의
증거가 아니다**. 특히 타이밍에 의존하는 e2e는 느린 러너에서만 드러나는 모호성을 갖는다.

---
### 0.23 §0.21 정정 — "첫 수집"도 "entitlement 차단"도 사실이 아니었다 (2026-08-23)

§0.20~§0.22를 독립 사실검증에 걸었다. 대부분의 수치·SHA·file:line·가드 동작은 재현됐지만
**§0.21의 서사 부분 두 건이 거짓**이었고, 그중 하나는 소유자에게 잘못된 다음 행동을
지시하고 있었다. 둘 다 직접 재확인했다.

**① "실데이터가 파이프라인을 통과한 것은 처음이다" — 거짓.** 나흘 앞선 2026-08-18에
credentialed 수집이 **DB publication까지 전부 성공**했다. 직접 조회한 `data_batches`:

    KRX|KR|EOD|2026-08-18|credentialed|2026-08-18 16:08:15+00
    KRX|KR|REFERENCE|2026-08-18|credentialed|2026-08-18 16:08:15+00
    KRX|KR|CALENDAR|2026-08-18|credentialed|2026-08-18 16:08:15+00
    KRX|KR|CORPORATE_ACTIONS|2026-08-18|credentialed|2026-08-18 16:08:15+00

Raw manifest의 첫 항목도 `date=2026-08-18`, `mode=credentialed`다. 08-19 종가가 08-18
종가에서 `prdy_vrss`만큼 정확히 이어지는 것도 확인됐다. §0.21의 제목과 "처음" 주장, 그리고
같은 내용을 반복한 BM 노트를 철회한다. **§0.15(:496,:519)의 "첫 credentialed 실행은 아직
안 했다"도 같은 근거로 틀렸다** — 내가 만든 오류가 아니라 물려받아 증폭한 오류다.

**② "막힌 지점은 빈 `data_entitlements`다" — 인과가 틀렸다.** 그 테이블은 지금도 비어
있지만 **08-18 publication이 성공하던 시점에도 똑같이 비어 있었다.** 비어 있다는 사실은
맞고 원인 지목이 틀렸다. 실제 원인을 재실행으로 확인했다:

    {"error_code":"PRICE_CURATION_FAILED","phase":"publication","class":"permanent",
     "message":"price curation failed"}

`WorkerError::Curation(CurateError)` — **Curated 생성 단계**이지 권리 검사가 아니다.
(앞서 3일치를 한 번에 넘겼을 때는 `PIPELINE_FAILED`로 보였는데, 1일치로 좁히니 더 정확한
코드가 드러났다.)

`data_entitlements`가 무의미한 것은 아니다. DB 함수 `public.resolve_price_dataset_entitlement`
(`candidate_sink.rs:211`)와 `crates/auth`의 API 권한 게이트가 읽는다. 즉 **candidate/Curated
승격 경로와 사용자 화면에는 필요하고, 오늘의 EOD publication을 막은 것은 아니다.** §4.2-4는
유지하되 "지금 이것 때문에 막혀 있다"는 표현을 철회한다.

**어느 CurateError인지는 아직 모른다.** 임시 sudo를 이미 회수해 `/var/lib/lagrange/data/curated`를
읽을 수 없다. 추측하지 않고 미확인으로 남긴다.

**③ 그 외 정정.**
- §0.19 "나머지 12개는 operator 실행": `6d54f59`가 static-check 12개를 전부 배선했으므로
  남는 것은 **10개**이고, 그중 3개는 `scripts/ops/static-check.sh`가 직접 실행하므로 실제
  operator 전용은 **7개**다. "22개 중 3개 배선"도 정확히는 2개다(세 번째 CI 스텝
  `validate.sh --self-test`는 그 22개에 속하지 않는다).
- §0.20 "기준 트리 `3ddf1c1`": 게이트 아티팩트는 `2ec0460`(콜백 수정) 커밋 **39초 전**에
  발행됐다. `APPROVED`는 `3ddf1c1` + 미커밋 수정 + 주입된 `LAGRANGE_AUTH0_*`의 결과이며
  **트리 `3ddf1c1`만으로는 재현되지 않는다.** 판정 자체는 진짜다.
- §0.18 제목 "결함 8건" vs 본문 6 MAJOR + CI 1 = 7. 8번째는 특정되지 않는다.
- §0.21의 timer 이름 `e3d1739`는 이후 `cbb7357`로 교체됐다. 더 중요한 것은 **같은 중복
  유닛 문제가 `lagrange-kis-backfill-*` 3세대에 그대로 남아 전부 enabled**라는 점이다.
- §0.18 "236커밋 뒤"는 237이다.

**④ 과잉주장 하나를 특히 철회한다.** §0.21은 08-19 dividend가 0행인 것을 "ETF는 배당이
아니라 분배금" 추정의 근거로 삼았다. 그 응답은 **whole-market 질의**라 "ETF가 배당을 안
준다"와 "그날 아무 발행사도 배당 기준일이 아니었다"를 구분하지 못한다. 결정적 반증이 바로
옆 디렉터리에 있었고 열어보지 않았다 — **08-18 dividend 응답에는 5행이 있다**(덕양에너젠, SK,
SK1우, 포스코인터내셔널, 케이비발해인프라, 전부 `record_date 20260818`, ETF11 코드는 없음).
즉 dividend는 비는 날도 있고 안 비는 날도 있으며, ETF11이 걸러지는 이유는 **필터가 동작하기
때문**이지 응답이 항상 비어서가 아니다. §4.2-3의 소유자 결정은 이 정정으로 오히려 **더**
필요해진다.

**이 절이 남기는 규칙.** 두 밤 동안 내가 STATUS에 쓴 거짓 주장이 이것으로 네 건째다
(§0.17 writer 주장, §0.21 이미지 라벨, §0.21 "처음", §0.21 entitlement 인과). 넷 다 **한
디렉터리 옆이나 한 쿼리 거리에 반증이 있었는데 확인하지 않고 단정한** 경우다. 새 사실을
쓸 때는 반증이 될 수 있는 가장 가까운 곳을 먼저 열어볼 것.

### 0.24 `PRICE_CURATION_FAILED` 규명 — 이 파이프라인은 이틀치를 발행할 수 없다 (2026-08-23)

§0.23이 "어느 CurateError인지는 모른다"로 남긴 것을 재현으로 확정했다. **추측이 아니라
실제 코드를 실제 데이터에 돌려서 얻은 변종이다.**

    NonCanonicalNormalizedBatch {
      reason: "cumulative curation inputs contain conflicting instrument masters" }

실패 지점은 `curation_inputs_from_raw_entries`(`curate.rs:1208`)이며 **`curate_generation`에
도달하기 전**이다. Curated 트리에 version=3이 없는 것과 일치한다.

**재현 방법.** sudo를 회수한 상태라 `/var/lib/lagrange/data`를 직접 못 읽는다. `/data`를
마운트한 컨테이너로 raw 2배치(36파일)와 curated 트리(68파일)를 읽기 가능한 위치로 복사한
뒤, `market-data`에 일회용 통합 테스트를 두어 worker의 누적 큐레이션 블록
(`worker.rs:2626~2740`)을 그대로 재현했다. 배포된 worker 이미지의 revision은 `e3d1739`이고
`git diff e3d1739..85e5774 -- crates/market-data data-pipelines/collectors`가 **비어 있으므로**
큐레이션 코드는 이미지와 워크트리가 동일하다 — 재현이 프로덕션에 대해 유효하다.
(테스트는 하드코딩 경로를 담고 있어 커밋하지 않았다.)

**인과 사슬 — 네 단계 전부 확인했다.**

1. KIS normalized `reference.json`의 instrument에는 `listed_at`이 **없다**(11개 중 0개).
   프로덕션 두 배치 모두, 그리고 저장소 픽스처
   `tests/fixtures/kr-etf/contract/reference-response.json`도 2개 중 0개다.
2. 그래서 `curate.rs:1127`이 `listed_at`을 `calendar_first_session`으로 폴백한다.
3. `calendar_first_session`은 `sessions.min().unwrap_or(entry.date)`(`curate.rs:1073`)인데,
   KIS `chk-holiday`는 요청한 날짜로 정규화되므로 **각 배치의 calendar에는 자기 날짜
   세션 하나뿐**이다(08-18 배치 → `[2026-08-18]`, 08-19 배치 → `[2026-08-19]`, 확인함).
4. 결과적으로 배치마다 `listed_at`이 자기 날짜가 된다. `curation_inputs_from_raw_entries`는
   모든 배치의 instrument master가 **완전히 같을 것**을 요구하므로 fail-closed 된다.

두 Instrument 레코드를 직접 출력해 대조했다. **`listed_at` 한 필드만 다르고 나머지는 전부
같다:**

    A = Instrument { instrument_id: 069500.KRX, name: "KODEX 200", asset_class: Etf,
        ..., listed_at: TradingDate(2026-08-18), ... }
    B = Instrument { instrument_id: 069500.KRX, name: "KODEX 200", asset_class: Etf,
        ..., listed_at: TradingDate(2026-08-19), ... }

**영향 범위 — 이것이 출시를 막는 벽이다.** 날짜가 2개 이상인 누적 큐레이션은 **항상**
실패한다. 즉 이 파이프라인은 **첫날만 발행할 수 있고 둘째 날부터 영구히 실패한다.**
관측된 이력과 정확히 일치한다 — 08-18은 version=2로 발행됐고(`source_batches` 1개),
08-19 이후는 전부 `PRICE_CURATION_FAILED`. `class:"permanent"`라 재시도로는 절대 풀리지
않는다.

**왜 테스트가 못 잡았나 (§0.19와 같은 종류의 공백).** 다중 배치 누적 큐레이션 테스트는
**이미 있다** — `crates/market-data/tests/price_publication_evidence.rs:247`이 2개 배치를
넘겨 성공을 단언한다. 통과하는 이유는 `fixture_entry`(:11)가 **두 배치에 완전히 같은
calendar 바이트**(`kr-etf/2020-01-31/calendar.json`, 세션 여러 개)를 넣기 때문이다. 그래서
`calendar_first_session`이 양쪽 동일 → master 동일 → 검사 통과. 픽스처는 프로덕션의
"`listed_at` 없음"은 재현하지만 **"배치마다 자기 날짜 세션 하나뿐"은 재현하지 않는다.**
정작 `curation_inputs_from_raw_entries`의 doc 주석(`curate.rs:1203`)은 "KIS는 chk-holiday를
요청 날짜로 정규화하므로 단일 배치의 calendar로는 부족하다"고 **이미 적고 있다.** 코드가
아는 사실을 픽스처가 반영하지 않아, 세션은 배치 간 병합하면서 그 세션에서 파생된
`listed_at`은 배치별 동일성을 요구하는 자기모순이 검출되지 않았다.

**아직 하지 않은 것 — 수정.** 원인은 확정이지만 고치는 방법은 설계 판단이 섞인다.
`listed_at`은 `NotListed { instrument, date }` 게이트의 입력이므로, 값을 바꾸면 어떤 바가
거부되는지가 바뀐다(원칙 6: PIT 증거를 지어내지 않는다). 이 절은 **원인 기록까지만**이고
수정은 별도로 판단한다. 같은 영역에서 다섯 번째 조사이고, §0.23이 남긴 규칙대로
"반증이 될 수 있는 가장 가까운 곳을 먼저 연다"를 지켰다 — entitlement 참조 불일치, curated
트리 손상, calendar provenance 불일치, instrument master 원본 불일치, `BatchAlreadyCurated`
다섯 후보를 **각각 데이터나 코드로 제거한 뒤** 재현에 도달했다.

**수정 (`b67ae1b`).** 폴백을 **병합된 세션 집합**에서 유도하도록 바꿨다 — 배치가 아니라
**세대(generation)의 속성**이 된다. `curation_inputs_from_raw`는 `None`을 넘기는 얇은
래퍼가 되어 **단일 배치 동작은 바이트 단위로 그대로**이고, 따라서 이미 발행된
version=1/2의 의미도 움직이지 않는다.

의도적으로 하지 **않은** 선택: instrument별 `min(listed_at)` 병합. 그렇게 하면 한 배치는
진짜 상장일을 주고 다른 배치는 폴백을 주는 불일치를 **조용히 받아들이게** 되는데, 지금은
그것이 fail-closed 된다. 진짜 `listed_at`을 가진 종목은 계속 엄격하게 비교된다.

**이 수정이 PIT 증거를 새로 지어내지 않는 근거.** `listed_at`은 **curated 산출물에 전혀
저장되지 않는다** — `manifest.json` 스키마에도, 어떤 parquet에도 없다(실제 version=2
manifest로 확인). 큐레이션 시점 `NotListed` sanity 게이트의 입력일 뿐이므로, 이 변경은 그
경계를 "한 배치"에서 "지금 큐레이션하는 세대"로 넓힌 것이고 그것이 올바른 스코프다.

**회귀 테스트** `crates/market-data/tests/cumulative_curation_single_session_calendars.rs`는
프로덕션의 **두 조건을 모두** 재현한다(배치당 세션 1개 + `listed_at` 0개). 수정 전 트리에서
`NonCanonicalNormalizedBatch`와 **동일한 reason 문자열로 실패**하는 것을 먼저 확인한 뒤
수정했다. 기존 다중 배치 테스트는 "calendar가 일치할 때"를 지키므로 건드리지 않았다.

**검증.** 실제 08-18/08-19 배치로 하네스를 다시 돌려 `curate_generation`이 **version=3**을
두 source batch로 생성하고 `last_session == anchor.date`(2026-08-19)임을 확인했다. 공통
게이트: fmt/clippy 통과, `cargo test --workspace --all-targets --all-features --no-fail-fast`
**1904건 통과 / 실패 0**(QA DB + `PYTHON=nt/.venv/bin/python` 필요), CI `policy` job의 정적
검사 전부 통과.

**아직 프로덕션 미검증.** 이미지 재빌드와 08-19 재실행이 남았다. version=3이 실제로 생기고
DB publication이 성공하기 전까지 이 벽은 "원인 수정, 운영 미확인"이다.

**부수 발견 — 진단 가능성 결함.** `CurateError`는 변종이 약 30개인데 worker가 전부
`PRICE_CURATION_FAILED` + `"price curation failed"` 한 줄로 접는다
(`research-worker.rs:709`). 변종 이름조차 남지 않아 이번 규명에 컨테이너 복사와 일회용
하네스가 필요했다. 변종 이름과 구조화된 필드(모두 우리가 만든 문자열이며 provider 응답
본문이 아니다)는 노출해도 KIS 안전 경계를 침범하지 않는다. §4.3에 등재한다.


### 0.25 §0.24 수정의 운영 검증 — 큐레이션 벽은 뚫렸고, 그 뒤에 두 개가 더 있었다 (2026-08-23)

소유자가 3시간 한정 sudo를 발급해 §0.24 수정을 운영 호스트에 반영했다. 지난번 발급의 결함
두 개(회수 유닛이 `/run`에 있어 재부팅하면 사라짐, `OnActiveSec`을 `daemon-reload`가 재기준화)를
고쳐 유닛을 `/etc`에 두고 **절대 시각** `OnCalendar`를 썼다.

**세 가지가 서로 어긋나 있었다.** 타이머가 실행하는 릴리스는 `cbb7357`, worker 이미지는
`e3d1739`, main은 `9b4a175`였다. `cbb7357..9b4a175` 사이에 큐레이션뿐 아니라 **운영 스크립트도
바뀌었으므로**(마감 가드 `7f72d39`, `lib/db.sh` `ccea9c2`) 셋 다 갱신했다. 이것이 §0.23이
"릴리스 커밋·이미지 커밋·타이머 대상이 일치하는지 자동 검사가 없다"로 남긴 항목의 실제 비용이다.

**① 큐레이션 수정은 운영에서 동작한다 — 확인.** `/var/lib/lagrange/data/curated/datasets/krx_eod_bars/`에
**`version=3`이 생성됐다.** `source_batches`가 08-18(`655f5f30`)과 08-19(`4140c8d1`) **두 개**이고
`bar_count=22`, artifacts 33개다. 수정 전에는 이 지점에서 영구 실패했다. 이미지 라벨
`org.opencontainers.image.revision`이 `9b4a1750ef...`임을 실행 전에 확인했다(§0.21의 낡은
바이너리 함정을 이번에는 밟지 않았다).

**② 다음 벽 — entitlement. 이번에는 DB가 직접 말했다.** PostgreSQL 로그에:

    ERROR:  price dataset requires one exact active entitlement
    CONTEXT: PL/pgSQL function public.resolve_price_dataset_entitlement(text,date,date) line 26

원장 `candidate_raw_batch_publications`에 08-19 배치가 **내 실행 시각(06:07:08Z)에** 기록됐다:
`state=BLOCKED reason_code=ENTITLEMENT_INACTIVE rights_first_date=2026-08-18
rights_last_date=2026-08-19`. **누적 창이 정확히 계산됐다는 증거이며**, 수정 전에는 큐레이션이
먼저 죽어 이 지점에 도달조차 못 했다. §4.2-4는 이제 추정이 아니라 **DB가 명시적으로 요구하는
차단 항목**이다. (§0.23이 인과를 잘못 지목했던 그 테이블이 맞지만, 이번엔 근거가 다르다 —
그때는 "비어 있다"는 관찰뿐이었고 지금은 함수가 거부하는 로그가 있다.)

**③ 아직 규명 못 한 것.** 그 뒤 08-19에서 `PIPELINE_FAILED`(phase=publication,
batch_id=`4140c8d1`, class=permanent)가 남는다. **어느 변종인지 모른다.** 배제한 것:
`PublicationBundle::from_raw`(두 배치 모두 로컬 통과), `validate_bundle`(두 배치 모두 통과),
`PublicationState::Partial`(`data_batches`·`trading_calendars`·`trading_calendar_versions`
전부 08-19 행 0건), SQL 거부(해당 실행 구간 PostgreSQL 로그에 entitlement 외 에러 없음),
신규 KIS 호출(raw 디렉터리에 오늘 생성된 배치 없음). **추측하지 않고 미확인으로 남긴다.**

**④ 그래서 진단 가능성을 먼저 고쳤다 (`485c937`, §4.3-7).** 코드 역추적이 여기서 막힌 것이
§0.24에 이어 **두 번째**다. `PIPELINE_FAILED`와 `PRICE_CURATION_FAILED`가 각각 서른 개 남짓
변종을 고정 문장 하나로 접는 것이 원인이므로, 변종 이름을 담는 `detail` 필드를 추가했다.
provider 전송 텍스트를 인용할 수 있는 `IngestError`와 sqlx 계열은 payload 없이 이름만,
우리가 만든 문자열인 `SinkError::Conflict/Invariant`는 그대로 노출한다. `PublicationError`는
exhaustive match라 변종이 늘면 컴파일 에러가 난다. KIS 경계는 넓히지 않는다.

**⑤ 부수 확인 — 중복 유닛 결함은 kis-daily에도 있다.** `install-kis-daily.sh --apply`는 새
세대를 설치하면서 **이전 세대를 비활성화하지 않는다.** 설치 직후 `cbb7357`과 `9b4a175`
타이머가 **동시에 16:30 발사 예정**이었고, 직접 `disable --now`로 정리했다. `--replace-existing`은
같은 이름의 기존 유닛 파일에만 관여하므로 커밋 접미사가 바뀌면 도움이 되지 않는다.

**⑥ §0.23의 한 문장을 정정한다.** "`lagrange-kis-backfill-*` 타이머 3세대가 전부 enabled"는
**지금 사실이 아니다** — `systemctl list-unit-files`에서 세 개 모두 `disabled`다. 언제 그렇게
됐는지는 특정할 수 없으므로(내가 이번 세션에 끈 것은 `lagrange-kis-daily-cbb7357.timer`뿐이다)
"현재 disabled"라는 관찰만 기록한다.

**⑦ 운영 지식 하나.** `deploy-production-release.sh --apply`의 `--env-source` 기본값은
저장소 체크아웃 안의 `.env`인데, 스크립트는 그 **부모 디렉터리가 root 소유일 것**을 요구한다.
사용자 소유 체크아웃에서는 항상 거부되며 메시지는 `install-root ancestor must be root-owned`라
install-root 문제처럼 읽힌다(검사 함수가 두 경로에 공유되고 메시지가 하드코딩이다).
root 소유 보호 사본 `/etc/lagrange/compose.env.pending`을 명시해야 한다 — 내용은 sha256으로
동일함을 확인했다.


### 0.26 세 번째 벽 — 같은 원인이 DB 계층에서 한 번 더 (2026-08-23)

§0.25-③이 "어느 변종인지 모른다"로 남긴 것을, 바로 앞 커밋에서 넣은 `detail` 필드가
**재실행 한 번에** 알려줬다. 계측이 즉시 값을 했다.

    "detail": "Sink(stage=Publish, SinkError::Conflict:
               calendar source version differs for KRX kis-chk-holiday-v1:schema-1)"

**검사 지점.** `lock_and_verify_source_versions`(`sink.rs:570`)가 강제하는 것은
**`(exchange, source_version)` 하나에 `content_sha256`이 정확히 하나**라는 불변식이다:

    SELECT EXISTS(SELECT 1 FROM trading_calendar_versions
     WHERE exchange=$1 AND source_version=$2
       AND (source <> $3 OR timezone <> $4 OR content_sha256 <> $5))

**증거.**

| | `calendar.json` sha256 | source_version |
|---|---|---|
| 08-18 (발행 성공) | `96624dabde9e1dd7…` — `trading_calendar_versions`에 저장된 값과 일치 | `kis-chk-holiday-v1:schema-1` |
| 08-19 | `eaf6e6be10f54aca…` **다름** | `kis-chk-holiday-v1:schema-1` **동일** |

**§0.24와 같은 원인 계열이다.** KIS `chk-holiday`는 요청한 날짜로 정규화되므로 매일의
calendar 문서가 다르다. 그런데 `source_version`은 `calendar_id`(`kis-chk-holiday-v1`)와
`schema_version`(1)에서만 나오고 둘 다 상수다. DB는 "source version 하나 = 불변 문서 하나"로
모델링하고 있는데 — 이는 KRX가 발행하는 연간 달력처럼 **버전이 찍힌 문서**를 전제한 설계이며,
벤더가 발행된 달력을 몰래 바꾸는 것을 잡아내는 좋은 불변식이다 — **날짜별로 생성되는 KIS
소스는 이 전제를 원리적으로 만족할 수 없다.** 그래서 둘째 날에 반드시 충돌한다.

**즉 파이프라인은 여전히 이틀치를 발행하지 못한다.** §0.24를 고쳐 Curated 생성(version=3)까지는
갔지만, canonical EOD publication이 DB에서 막힌다.

**두 벽은 독립이다.** 실행 순서상 `recover_price_publications`의 entitlement 거부(§0.25-②)가
먼저 나오고 그 다음 날짜별 publish에서 이 충돌이 난다. 전자는 **candidate/price 승격** 경로를,
후자는 **canonical EOD publication**(`data_batches`)을 막는다. **둘 다 풀려야 둘째 날이 발행된다.**

**고치지 않았다 — 설계 판단이다.** 후보는 최소 셋이고 셋 다 PIT 증거의 의미를 건드린다:
(a) `source_version`을 세션 날짜까지 포함하도록 유도해 날짜마다 다른 버전으로 취급,
(b) KIS 달력을 "버전이 찍힌 문서"가 아닌 별도 모델로 분리,
(c) 불변식을 `(exchange, source_version, session_date)` 단위로 완화.
(c)는 벤더가 같은 날짜의 달력을 바꿔치기하는 것을 못 잡게 되므로 이 불변식이 원래 지키려던
것을 잃는다. (a)는 "source version"이 무엇을 뜻하는지를 재정의하는 것이다. 원칙 1·4·6에
걸리므로 **소유자 결정으로 등재하고 여기서는 원인 기록까지만 한다.**

**이 절이 확인해 주는 것.** §0.24의 부수 발견(진단 가능성)을 먼저 고친 판단이 옳았다.
같은 벽에 두 번 부딪힌 뒤 계측에 30분을 썼고, 그 다음 원인 규명은 **재실행 한 번**으로 끝났다.
앞선 두 번은 각각 컨테이너로 Raw를 복사하고 일회용 하네스를 빌드해야 했다.


### 0.27 entitlement 등록 실행 — 결함 2건을 넘었고, 참조 불일치 1건에서 멈췄다 (2026-08-23)

소유자가 §4.2-4를 **(a)안, 활성화 날짜 2020-01-31**로 확정해 실행했다. 등록·활성화는
성공했다(`data_entitlements` 1행, `status=ACTIVE`, `2020-01-31..9999-12-31`,
`managed_by=00000000-0000-4000-8000-000000000042`). 다만 **경로에서 결함 2건을 만났고 둘 다
고쳤으며, 세 번째 불일치에서 소유자 결정이 한 번 더 필요해졌다.**

**결함 ① 승인된 권리 문서의 sentinel을 검증기가 거부했다 (`86e6577`).**

    entitlement: effective_until is not a real date

`configs/data-rights/kis.entitlement.json`의 `effective_until`은 무기한 sentinel
`9999-12-31`인데, `valid_date`가 `date -u -d`로 라운드트립했다. GNU date는 오프셋이 적용되면
그 날을 표현하지 못한다 — `date -u -d 9999-12-31`은 실패하고, `-u`를 빼면 **양수 오프셋 존에서만**
통과한다(Asia/Seoul 성공, UTC·America/Sao_Paulo 실패, 셋 다 실측). `9999-12-30`과 `5000-06-15`는
멀쩡히 통과하므로 **sentinel이 정확히 경계값**이고, 그것이 승인 레코드가 담고 있는 값이다.
달력을 산술로 검증하도록 바꿨다. `register-dataset-version.sh`에 **동일한 헬퍼와 동일한 결함**이
있어 함께 고쳤다. **레코드를 고치는 선택지는 없다** — 검증기를 만족시키려 승인된 권리 문서를
편집하는 것은 이 저장소가 다른 곳에서 이미 거부하는 실패 양식이다. 틀린 쪽은 검증기였다.
self-test가 못 잡은 이유: 픽스처가 `effective_until: "2026-12-31"`을 써서 **실제 레코드가
쓰는 바로 그 필드에서 프로덕션과 어긋나 있었다.** 이제 같은 sentinel을 쓰고, UTC와 그 양쪽
오프셋 존에서 달력 규칙을 고정한다.

**결함 ② 커밋에 성공하고도 실패를 보고했다 (`4bf67a2`).**

    ENTITLEMENT_APPLY 대신 → BLOCKED_EXTERNAL: database returned no entitlement row
    (그런데 data_entitlements 에는 정확한 행이 이미 커밋돼 있었다)

**동시에 참인 이 두 상태가 운영자 실패 중 최악의 모양이다** — 이미 써진 DB에 재시도를
유도한다. psql은 파일 전체에 대해 결과 행마다 한 줄을 내는데, 각 트랜잭션을 지키는
`SELECT pg_advisory_xact_lock(...)`이 void를 반환해 **빈 줄을 먼저** 찍는다. 파서가 `NR==1`을
읽어 그 빈 줄을 집었다. 원시 출력으로 확인했다:

    $
    2ac51ec7-78d9-4180-91c5-f37f2fb6ed68^IPENDING$

즉 두 헬퍼의 `--apply`는 **한 번도 성공한 적이 없다.** `--check`만 되는 이유는 그 쿼리가
advisory lock 없이 돌기 때문이다. 기대하는 열 개수를 가진 첫 줄을 집도록 공유 헬퍼
`db_row_field`로 바꿨다(두 스크립트 10곳, `register-dataset-version.sh`의 4열 readback 포함).
self-test는 DB를 쓰지 않아 끝단을 못 덮지만 **출력 모양은 고정할 수 있고** 이제 고정한다.

**③ 멈춘 지점 — 참조 불일치. 소유자 결정이 한 번 더 필요하다.** entitlement가 ACTIVE인데도
함수는 여전히 거부한다. `resolve_price_dataset_entitlement`는
`entitlement.contract_reference = p_contract_reference` **정확 일치**로 찾는데:

| | 값 |
|---|---|
| Raw 배치가 citing (`RESEARCH_ENTITLEMENT_REFERENCE`) | `operator-attestation://l1nnx/kis-readonly/2026-08-18` |
| 등록된 DB `contract_reference` (승인 레코드의 `document_reference`) | `repo://docs/decisions/0005-kis-personal-use-entitlement.md` |
| self-test 픽스처 | `operator-attestation://self-test/kis-readonly` |

**근본 원인은 한 필드가 두 역할을 겸하는 것이다.** `document_reference`는 "계약 문서가 어디
있는가"(해시도 그 ADR의 해시다)인데, 스크립트가 그것을 그대로 `contract_reference` —
"Raw 배치가 citing하는 키" — 로 쓴다. 둘은 다른 것이다. self-test 픽스처가
operator-attestation URI를 쓰는 것은 **설계된 형태가 후자**임을 시사한다.

08-18·08-19 Raw는 불변이고 이미 operator-attestation URI를 citing하므로, 설정만 바꾸는 것으로는
그 이틀을 살릴 수 없다. **어느 쪽이 권위인지는 권리 주장에 관한 결정이라 §4.2-6으로 등재하고
여기서 멈춘다** — 승인 레코드에 없는 참조로 권리를 등록하는 것을 에이전트가 지어내지 않는다.


### 0.28 네 번째 벽 — 코드 계약 테스트와 DB 스키마가 서로 모순한다 (2026-08-23)

소유자가 §4.2-6을 **(a)안 — operator-attestation URI로 등록**으로 확정해 실행했다.
`data_entitlements`에 두 번째 행을 만들고 활성화했다(`9e3c78cb-…`, ACTIVE,
`contract_reference = operator-attestation://l1nnx/kis-readonly/2026-08-18`, 문서 해시는
ADR-0005 그대로 유지해 문서 결속을 지켰다). **함수가 이제 해결된다:**

    select public.resolve_price_dataset_entitlement(
      'operator-attestation://l1nnx/kis-readonly/2026-08-18','2026-08-18','2026-08-19');
    -> 9e3c78cb-029b-41d2-89b1-c5641ac63ec1

행이 둘이 되어도 안전한 것을 먼저 확인했다 — `repos/entitlements.rs::load()`는 전체를
읽어 Vec에 담고, `recommendation/runner.rs:790`은 `EXISTS`이며, 함수는 참조로 필터한다.

**entitlement 벽은 뚫렸고 재검증도 통과해 훨씬 안쪽에서 멈췄다.**

    ERROR: insert or update on table "candidate_price_instrument_coverage"
           violates foreign key constraint
           "candidate_price_instrument_coverage_instrument_id_fkey"
    CONTEXT: PL/pgSQL function public.publish_candidate_price_publication(...) line 161

`candidate_price_instrument_coverage.instrument_id` → `instruments(id)` FK인데
**`instruments` 테이블이 0행이다.**

**그리고 그 테이블에는 프로덕션 writer가 없다.** `INSERT INTO instruments`는 저장소 전체에서
**테스트에만** 있다. 유일한 프로덕션 함수 `register_candidate_instruments`
(`candidate_sink.rs:280`)는 **호출처가 하나도 없다** — 테스트와, 아래 부정 단언뿐이다.

**모순이 명시적으로 코드에 박혀 있다.** `worker.rs:2927`의 계약 테스트 이름이
`cumulative_recovery_revalidates_blocked_price_and_does_not_require_candidate_catalog`이고
본문이 단언한다:

    assert!(!function.contains("register_candidate_instruments("));

즉 코드 쪽은 "고정 ETF price dataset은 candidate 카탈로그를 요구하지 않는다"를 **계약으로
고정**하고 있는데, DB 함수는 `v_inserted = 1`일 때 coverage 행을 넣고 그 FK가 `instruments`를
요구한다. **테스트 이름이 프로덕션에서 거짓이다.** 이 경로는 출하된 상태 그대로는 완료될 수
없다 — backfill 경로는 `candidate_sources_enabled`를 명시적으로 금지하므로
(`run_credentialed_backfill_session_dates_stream`), 카탈로그를 채울 유일한 경로와 상호
배타적이다.

**고치지 않았다 — 어느 쪽이 권위인지가 결정이다.** (a) price publication이 coverage를 쓰지
않도록 한다(코드 계약이 맞고 DB 함수가 과하다) (b) 고정 ETF 11종을 `instruments`에 등록하는
프로덕션 경로를 만든다(DB가 맞고 계약 테스트가 틀렸다) (c) FK를 완화한다 — 이는 coverage가
지키려던 참조 무결성을 잃는다. §4.2-7로 등재한다.

**오늘 벽 넷을 통과했고 넷 다 같은 성질이었다** — 단일 배치·단일 날짜를 전제한 설계가 둘째
날에 무너진다(§0.24 listing 폴백, §0.26 calendar source_version), 그리고 한쪽이 참으로
가정한 것을 다른 쪽이 강제한다(§0.27 참조 불일치, 이 절의 FK). 매번 **가장 가까운 반증을
먼저 열었고**, 매번 원인을 확정한 뒤에야 다음으로 갔다.


### 0.29 파이프라인이 처음으로 이틀치를 발행했다 — 그리고 벽 여섯 개는 전부 같은 모양이었다 (2026-08-23)

    data_batches
      2026-08-18 | CALENDAR, CORPORATE_ACTIONS, EOD, REFERENCE | credentialed
      2026-08-19 | CALENDAR, CORPORATE_ACTIONS, EOD, REFERENCE | credentialed   ← 신규

`exit=0`, `{"event":"published","phase":"canonical_publication","target_date":"2026-08-19"}`,
`outcome: backfilled`. 아침에 `PRICE_CURATION_FAILED` 한 줄로 시작한 문제가 닫혔다.

**각 수정이 의도한 자리에서 확인된다.**

| 수정 | 증거 |
|---|---|
| §0.24 큐레이션 폴백 | Curated `version=3` (source batch 2개, bar 22개) |
| §0.26/(나)안 달력 | `trading_calendar_versions`에 **같은 `source_version`으로 두 날짜**가 각자 다른 해시(`96624dab…`/`eaf6e6be…`)로 공존 |
| §4.2-7 instrument 등록 | `instruments` 11행, `candidate_price_instrument_coverage` 11행 |
| `0048` 재사용 바인딩 | `candidate_raw_batch_datasets`에 `reused_existing=false`(anchor)와 `true`(이전 날짜)가 **둘 다** |
| §0.27/§0.28 entitlement | 원장 두 날짜 모두 `PUBLISHED`, `ENTITLEMENT_INACTIVE` 소멸 |

**이 절이 남기는 진짜 교훈 — "둘째 날" 패턴.** 오늘 넘은 벽이 여섯인데 **여섯 다 같은 성질**이었다:
**하루치에는 맞고 이틀치를 표현할 수 없는 설계.** 우연이 아니라 이 시스템이 단일 배치·단일
날짜로 개발되고 그 상태로만 검증돼 온 결과다.

| # | 벽 | 하루치에서 왜 안 보였나 |
|---|---|---|
| 1 | 상장일 폴백이 배치별 (`curate.rs`) | 배치가 하나면 폴백도 하나 |
| 2 | 달력 `source_version`이 상수인데 문서는 날짜별 (`sink.rs`) | 문서가 하나면 충돌 상대가 없다 |
| 3 | `instruments` FK를 요구하면서 등록 경로가 제거됨 (`84e6ce1`) | — 이건 날짜와 무관한 자기모순 |
| 4 | 카탈로그 `listed_at`이 master에서 오면 윈도우 따라 드리프트 | 윈도우가 안 넓어지면 안 움직인다 |
| 5 | 재사용 바인딩 경로에 DB 전제조건 미갱신 (`f815f63`) | 하루치는 originate만 하고 reuse를 안 한다 |
| 6 | entitlement 참조가 두 역할을 겸함 | 발행이 한 번뿐이면 불일치가 드러날 기회가 없다 |

**다음에 이 시스템에서 "왜 어제는 됐는데 오늘 안 되지"를 만나면, 먼저 의심할 것은
"이 값이 배치 하나를 전제하는가"이다.**

**계측이 실제로 값을 했다.** §0.24와 §0.25에서 같은 벽에 두 번 부딪힌 뒤 `detail` 필드에
30분을 썼다. 그 다음 규명 세 건(§0.26 달력, FK, `0048` 바인딩)은 **재실행 한 번씩**으로
끝났다. 앞선 두 건은 각각 컨테이너로 Raw를 복사하고 일회용 하네스를 빌드해야 했다.

**내가 오늘 반복한 실수 둘.**
- **머지되어 닫힌 PR에 계속 푸시했다.** 그러면 CI가 돌지 않는다. PR #3에서 겪고 PR #4에서
  또 했다. 그 사이 커밋 두 개가 검증 없이 브랜치에 앉아 있었다.
- **`research-smoke.yml`이 별도 워크플로이고 `push: [main]`에서만 돈다는 것을 몰랐다.**
  `ci.yml` 8개 job만 보고 "green"이라고 여러 번 보고했고, 실제로는 머지 뒤 main이 빨개졌다
  (`6521db9`). 30분 넘게 지나서야 발견했다. **머지 후에는 `gh run list`로 워크플로 전체를
  확인한다** — `gh pr checks`는 PR에 붙는 것만 보여준다.

**정렬 상태 (2026-08-23 20:40 기준).**

| | |
|---|---|
| main / 이미지 | `6c2d18b` |
| 릴리스 / 타이머 | `66b2a8c` |

일일 경로 스크립트(`kis-daily-production.sh`, `backfill-production.sh`, `lib/`, compose)는
`66b2a8c..6c2d18b` 사이에 **변경이 없으므로** 내일 16:30 실행은 새 이미지로 정상 동작한다.
다만 셋을 일치시키려면 **내일 16:30 이전에** 릴리스·타이머를 `6c2d18b`로 옮겨야 한다
(`install-kis-daily.sh --apply`는 `Persistent=true` 때문에 16:30 이후 설치가 금지된다).

**적용된 마이그레이션.** 47(worker price entitlement attestation)과 48(price generation
reuse binding)이 함께 적용됐다 — 47도 미적용 상태였다.

**남은 것.** 08-20·08-21 미수집(다음 실행이 집어간다). §4.2의 소유자 결정 3건(수수료 필드,
Phase 4 우선순위, 기업행사 6개 클래스)은 그대로다. 기업행사 건은 이제 **실제로 터지는지
확인할 수 있다** — 발행이 되므로 다음 실행에서 드러난다.

**정정 (§0.31):** 바로 위 "다음 실행이 집어간다"는 **그날 저녁에 이미 깨졌다.** 16:30 타이머
실행이 실패해 08-20·08-21은 여전히 미수집이다.


### 0.30 출시 준비도 실측 — 데이터 경로는 열렸고, 그 위층은 아직 한 번도 돈 적이 없다 (2026-08-23)

§0.29 직후 "이제 출시를 앞둔 것이냐"는 물음에 답하기 위해 운영 상태를 **기억이 아니라 실측**했다.
답은 **아니다**. 오늘 연 것은 필요조건이지 출시가 아니다.

**지금 실제로 떠 있는 것은 데이터베이스 하나뿐이다.**

    docker ps → lagrange-station-postgres-1 | Up (healthy)

compose에 정의된 서비스는 14개다(`api-server`, `web`, `reverse-proxy`, `paper-scheduler`,
`recommendation-runner`, `candidate-runner`, `research-worker`, `nt-backtest-worker-1/2`,
`postgres`, `db-migrate`, `db-role-bootstrap`, `research-raw-init`, `research-schema-check`).
**사용자가 닿는 것은 하나도 돌지 않는다.** systemd 유닛도 없다 — `systemctl list-unit-files
'lagrange*'`에 있는 것은 kis-daily, kis-backfill 3세대, production-backup, tailscale-tls뿐이고
api/web/paper/recommendation 유닛은 **존재하지 않는다.** 설치된 적이 없다는 뜻이다.

**데이터와 상태.**

| | |
|---|---|
| EOD 발행일 | `2026-08-18`, `2026-08-19` — **2일** |
| `recommendation_runs` | **0** — 추천이 실데이터로 한 번도 계산된 적 없다 |
| `accounts` | **0** |
| `instruments` | 11 (오늘 최초 등록) |

**오늘 한 일의 정확한 위치.** 어제까지 데이터는 하루치에서 멈춰 있었고, 그래서 위층(추천·
Paper·화면)을 실데이터로 시험할 **수단 자체가 없었다.** 오늘 그 수단이 생겼다. 그것이
전부이며, 작지 않지만 출시는 아니다.

**남은 것, 순서대로.**

1. **운영 서비스 활성화** (§4.3-4, §4 항목 8) — role-scoped DB URL과 curated/raw volume을
   주입해 `api-server`/`web`/`paper-scheduler`/`recommendation-runner`를 띄운다.
   이것 없이는 사용자가 접속할 대상이 없다.
2. **추천 파이프라인 최초 실행** — 코드는 완성이고 테스트도 통과하지만 `recommendation_runs=0`
   이다. **실데이터로 한 번도 돌지 않았다.**
3. **데이터 축적** — 팩터·추천에 의미 있는 이력이 필요하다. 지금 2일이고 백필 승인은 §4.3-5다.
4. **게이트 재실행** — phase1 APPROVED는 08-22 기준이고 그 뒤 오늘 여덟 커밋이 들어갔다.
5. ~~**소유자 결정 5건**~~ **해소 (2026-08-23~24, §0.32·§4.2).**

**§0.29의 패턴이 여기에 주는 예보.** 오늘 무너진 여섯 개는 **전부 "두 번째로 해볼 때"**
드러났다. 위 1·2번은 각각 **"처음 해보는 것"**이다. 같은 밀도로 결함이 나올 것을 전제하고
계획하는 편이 맞다. "이제 거의 다 됐다"는 판단은 오늘 배운 것을 무시하는 것이다.

**권장 다음 순서: ①서비스 활성화 → ②추천 최초 실행.** 둘 다 현재 데이터로 착수 가능하고,
막히는 지점이 곧 다음에 알아야 할 것이다.


### 0.31 독립 출시 준비도 분석 — 타이머 첫 발사가 실패했고, 출시 스위치에 주인이 없다 (2026-08-23)

`3b24957` 기준 read-only 분석이다 — 그 뒤 `origin/main`은 문서 전용 커밋 `1150700`(§0.30)으로
앞서갔고 코드·동작 변경이 없으므로 아래 사실관계는 그대로 유효하다. provider·브라우저·운영 DB
쓰기·systemd·CI를 호출하지 않았고 코드·운영 설정도 변경하지 않았다. 확인 수단은 문서 정독, 운영 호스트의
journal·docker·systemd 조회, 운영 PostgreSQL **read-only SELECT**, 코드 grep이다. 판정은 그대로
`vendor_snapshot=true`, `strict_pit=false`, `ready=false`.

**① 오늘 16:30 일일 수집 타이머 실행이 실패했다 — 이 문서에 없던 사실이다.**

    BLOCKED_EXTERNAL: daily state is missing, stale, malformed, or not appendable;
                      no worker or KIS call was made
    Main process exited, code=exited, status=2/INVALIDARGUMENT

`lagrange-kis-daily-66b2a8c.service`, 08-23 16:30:03~04. 그 직전 `PRODUCTION_CONFIG: PASS`는
났으므로 설정이 아니라 **당일 상태 파일 검증**(`scripts/ops/kis-daily-production.sh:616`)에서
닫혔다. 상태 파일 헤더에는 schema·기간·universe·`code_commit`·`entitlement_reference`·calendar
해시로 만든 run identity가 박히고 불일치하면 stale로 거부한다. **유력 가설은 같은 날 낮의 수동
디버깅 실행들(§0.25~§0.29의 sudo 구간, 그때 릴리스는 `cbb7357`→`9b4a175`)이 당일 파일을 자기
identity로 만들어 두고, 16:30 타이머는 릴리스 `66b2a8c`로 다른 identity를 계산한 것**이며, 이는
§0.29의 "둘째 날" 패턴의 변형 — 같은 날의 **두 번째 실행 주체**를 상태 파일 설계가 표현하지
못한다. 상태 파일이 root 보호라 헤더를 직접 읽지 못했으므로 **원인은 미확정으로 남긴다.**

파급은 실측으로 확인했다. `data_batches`에는 여전히 08-18·08-19만 있고 **08-20·08-21은 없다.**
또한 **타이머 단독으로 발행까지 성공한 실적은 0회**다 — 08-19는 수동 실행 결과이고(§0.25~0.29),
08-18 실행의 트리거는 기록상 불명이며(§0.23이 사후 발견), 첫 타이머 설치는 그 이후다. 상태
파일이 일자별이라 08-24 실행은 새 파일로 자가 회복될 **가능성이 높지만 검증 전에는 가정하지
않는다.** 조치는 (1) 릴리스·타이머를 `6c2d18b`로 정렬하고 구세대 유닛을 직접 `disable --now`
(§0.29-9, §0.25-⑤), (2) sudo로 상태 파일 헤더를 확인해 원인 확정, (3) 08-24 16:30 journal로
캐치업 확인이다.

부수 결함: 이 검증 블록은 python stderr를 `2>/dev/null`로 버리고 네 원인(missing/stale/
malformed/not appendable)을 한 문장으로 접는다 — §0.24-⑥·§0.25-④에서 두 번 비용을 치른
진단 가능성 결함과 **같은 계열**이다. §4.3-11로 등재한다.

**② 정렬·기동 상태 실측 — §0.30을 독립적으로 재확인했고, 세 가지를 덧붙인다.** 서빙 스택이
운영에서 한 번도 기동된 적 없다는 §0.30의 실측에는 독립적으로 같은 결론에 도달했다(실행 중
컨테이너는 `lagrange-station-postgres-1` 하나). 덧붙일 것은 세 개다. ① 이미지
`research-worker`·`api-server`의 `org.opencontainers.image.revision`은 둘 다 `6c2d18b`인데
릴리스·타이머는 `66b2a8c`이므로 **§0.29가 남긴 3자 어긋남이 그대로다.** ② `data_entitlements`
2행으로 §0.27~§0.28 등록분과 일치한다(§0.30 표의 `instruments` 11행과 같은 계열의 확인).
③ 가동 중인 타이머는 kis-daily(`66b2a8c`, 다음 발사 `2026-08-24` 16:30)·백업(일일)·
백업복원검증(주간)·TLS갱신(일일) 4개이고, 구세대 kis-daily 2종과 backfill 3종은 disabled다
— §0.25-⑥의 "현재 disabled" 관찰이 유지된다.

**③ §0.16이 남긴 TODO 전수 조사 구멍을 종결했다.** §0.16이 "검색 도구 부재로 수행하지 못했다"고
적은 그 조사다. **프로덕션 코드의 미완성 마커는 사실상 0건**이다 — `unimplemented!`/`todo!`
0건, 프로덕션 `TODO`/`FIXME` 0건. `unreachable!` 15건은 전부 불가능 분기의 문서화이며 마커가
아니다. 유일한 스텁 2개는 `crates/api-server/src/error.rs:28`의 `TenancyError::NotImplemented`
(정의와 HTTP 501 매핑만 있고 **생성처 0 = 죽은 변형**)와 `nt/live-node` entrypoint의
"non-dry-run live execution is not implemented"(설계된 fail-closed)다.

**④ 요구사항 대비 갭 6건 — 코드로 확인했다.** STATUS가 지금까지 다루지 않은 항목들이다.

| 요구사항 | 우선순위 | 판정 | 근거 |
|---|---|---|---|
| FR-ADM-001 관리자 운영 화면 | **Must** | 부분 | 백엔드 완비(`/admin/jobs`·`retry`·`workers`·`audit-logs`·`datasets approve/block`, `http/admin.rs`). 그러나 `apps/web/app/(authenticated)/admin/page.tsx:11-18`은 **빈 `StatePanel` 한 장**이고 하위 화면·재시도 UI가 없다 |
| FR-RPT-002 이메일/메신저 알림 | Should | 부분 | 구독·전달기록 인프라는 있으나 `notify.rs:89-93`의 `EmailTransport`는 **항상 `Err` 반환 스텁**이고 SMTP/메신저 코드는 저장소 전체에 없다. 트리거도 Paper 정산뿐 — 추천 완료·오류·리스크 차단은 알림을 만들지 않는다 |
| FR-BT-009 취소·진행률·실패사유 | Should | 부분 | 취소는 full-stack 완비. **진행률은 UI가 읽는 `progress_percent`를 쓰는 producer가 crates 전체에 0건**이라 항상 "not reported"다. 실패 사유는 `summary_json`에 있는데 `backtests/page.tsx:89-91`이 고정 문구만 렌더한다 |
| FR-BT-010 두 실행 비교 | Should | 부분 | 요약지표 3개(total_return/cagr/mdd) 델타만이고 equity curve·거래·설정 비교가 없다. UI는 그중 1개만 렌더 |
| FR-SEL-006 파라미터 세트 비교 | Should | **미구현** | 관련 라우트·화면 없음. `selector`의 turnover는 전략 카드 서술 텍스트이지 계산·비교값이 아니다 |
| 미국 ETF (MVP 범위 §4.1) | — | 사실상 이연 | 도메인 타입(USD, ARCA/NYSE/NASDAQ)만 존재. `validation.rs:41-43`이 `SUPPORTED_CURRENCIES=["KRW"]`이고 KIS provider가 비-KRX를 fail-closed 거부한다 |

이들은 게이트를 막지 않으므로 판정에 영향이 없다. 다만 **소유자 단독 운영에서도 ①"장 마감
추천이 왔는지"를 알 수단이 화면 확인뿐이고 ②Must 항목인 관리자 화면이 비어 있다**는 사실은
출시 범위 문서에 명시해야 한다.

**⑤ 출시의 정의를 좁혀야 한다.** 요구사항은 "사용자 5명"(SC-01, Phase 1 종료 기준)이지만
ADR-0005의 승인 범위는 **소유자 1인 단독**이고 Member KR-파생 표면은 승인 밖이며 Live는 별도
프로젝트다. 따라서 현 권리로 가능한 출시는 **소유자 단독 read-only 운영** 하나뿐이다. "5인
개방"은 코드 결함이 아니라 **재배포 권리 결정**이며(요구사항 §11, §14 위험표 1행), 그렇게
읽어야 §2의 "5명 사용" 종료 기준이 미충족인 이유가 정확해진다.

**⑥ 출시 스위치에 주인이 없다 — 이 문서 자체의 공백이다.** §0.15가 정의한 순서의 종점은
`ready=false`를 뒤집고 DatasetManifest·five-pin을 승인하는 것인데, **§4.2의 소유자 결정
목록에 그 항목이 없다.** 플래그 전환의 기준(무개입 연속 발행 며칠? 어떤 증거로?)과 승인
주체·절차가 어디에도 적혀 있지 않다. §4.2-8로 등재한다.

**⑦ 과거 데이터 정책도 미등재였다.** 현재 발행분은 이틀이고 Stage5의 1,608거래일은 Raw로만
있다. SC-02는 "10년 이상 일봉 백테스트"를 요구하는데 Stage6 범위 시작일은 `2020-01-31`(약
6.5년)이다. (a) KIS read-only 재백필(요청 예산 승인 필요), (b) Stage5 vendor snapshot을
`vendor_snapshot` 플래그로 발행, (c) 당분간 일일 증분만 — 어느 쪽이든 SC-02 기준 수정 여부가
따라온다. §4.2-9로 등재한다.

**확인하지 못한 것.** ① 16:30 실패의 정확한 원인(상태 파일 root 보호), ②
`/var/lib/lagrange` 내부 실물(Raw·Curated 트리), ③ 게이트·테스트 재실행 — read-only 원칙과
§5.1의 `/tmp` 쿼터 함정 때문에 의도적으로 생략했고 `6c2d18b`의 CI green 기록으로 갈음한다.
따라서 이 절은 어떤 green·READY·PIT 주장도 하지 않는다.

**전체 분석 문서:** `docs/reviews/2026-08-23-launch-readiness-analysis.md`.

### 0.32 소유자 전용 베타 계약과 실행 계획 확정 (2026-08-24)

§0.31이 등재한 출시 스위치와 과거 데이터 정책을 포함해 §4.2의 남은 결정 5건을
소유자가 확정했다. 출시는 **소유자 1명, 기존 Stage5 공급자 스냅샷,
`vendor_snapshot=true`, `strict_pit=false`, `PRICE_RETURN_ONLY`**로 한정한다. 역사 범위는
`2020-01-31..2026-08-19` 약 6.5년으로 시작하고 10년 SC-02는 이번 베타에서 이연한다.
자동 기업행위는 무상증자만 유지하고 나머지는 fail-closed, phase-0 수수료 golden 변경과
Phase 4도 이연한다. 연속 3개 거래일 무인 성공 전에는 Paper를 열지 않으며, 전역 READY가
이 좁은 범위를 표현하지 못하면 전역 `ready=false`를 유지한다.

실행 계획은 `docs/superpowers/plans/2026-08-23-owner-readonly-beta-launch.md`다. 소유자는
계획 범위의 세부 선택과 승인 게이트를 권장안대로 진행하는 원칙을 승인했고, 2026-08-24
최신 지시로 `paseo-delegate` 워커를 포함한 구현 시작을 명시했다. 소유자만 직접 할 수 있는
확인은 제거할 수 없을 때만 다음 검토 목록에 남긴다. 이 위임도 계좌·주문·Live,
Member KR, 새 역사 수집·소스, 엄격 PIT·총수익률 주장 또는 실패한 증거의 승격을 허용하지
않는다. manifest 생성 경로의 자동 자기승인도 계속 금지하며, 분리된 검증이 계약 전부를
확인한 경우에만 사전 승인된 등록 절차를 진행한다.

### 0.33 소유자 베타 구현 진행과 현재 외부 차단 (2026-08-24)

승인된 실행 계획의 선행 구현은 `4a975d3`(일일 상태 실패 분류), `3e77a3f`(역사 입력
인증), `8e276ec`(가격 전용 메모리 후보), `07003be`(release 이미지 identity),
`cd06b09`(읽기 전용 manifest snapshot), `daa47d5`(안전한 pin discovery)까지 진행됐다.
프로덕션 읽기 전용 점검에서는 기존 `66b2a8c` release의 일일 timer 하나가 활성 상태이고
정적 self-test와 현재 HEAD의 이미지 build plan/preflight가 통과했지만, 새 release 설치나
timer 전환은 아직 수행하지 않았다.

운영 Raw에 대한 body-free pin discovery 결과는 Stage5 고정 입력을 식별했지만, 계약이
요구하는 정확한 7파일 KSD action batch를 단일 후보로 증명하지 못해
`reason=action_evidence`로 차단됐다. 이 공백은 임의 hash, 빈 action 추정 또는 새 역사 KIS
호출로 메우지 않는다. 따라서 실제 역사 artifact 생성, 데이터셋 등록, 5-pin, 추천·백테스트
활성화와 3거래일 soak는 아직 시작할 수 없다.

병행 가능한 오프라인 작업으로는 `OWNER_ONLY`, `MATERIALIZED`, `UNREGISTERED`,
`NOT_PUBLISHED` 봉인 artifact의 canonical manifest/NDJSON 투영 코어와 Unix
descriptor-relative reader/writer를 구현했다. 후보 hash는 제외된 Raw request metadata까지
복원하는 artifact-content hash가 아니라 불투명 생산자 계보 pin으로 취급한다. reader와
writer는 symlink·hardlink·교체 race·tamper·크기 상한·atomic no-replace·부분 실패 cleanup을
포함한 테스트를 통과했고, 최초 writer 리뷰의 staging/cleanup 지적을 수정한 뒤 동일 보안
리뷰어의 `ACCEPT`를 받았다. Raw/Curated와 canonical path·Unix filesystem identity가
겹치지 않는 전용 artifact root gate와 제한형 `materialize`/`check` CLI도 구현하고 focused
8개, 전체 collectors binary 86개 테스트와 두 crate의 warning-zero clippy를 통과했다.
`materialize`는 같은 프로세스에서 pinned Raw verifier → opaque candidate → no-replace writer만
호출하며, `check`는 artifact 자기 무결성만 확인하고 Raw를 재인증했다고 주장하지 않는다.
공개 API·root 격리·정적 출력까지 독립 최종 리뷰에서 `ACCEPT`를 받았다. downstream
consumer, 등록, publication은 여전히 없다. 현재 상태는 구현 완료·실데이터 외부 차단이지
데이터 등록 또는 베타 출시 완료가 아니다.

최종 로컬 재검증에서 `cargo test -p market-data`는 단위 141개와 모든 통합 테스트를 포함해
전체 PASS했고, artifact focused 34개·artifact CLI 8개·collectors binary 86개도 PASS했다.
`cargo test -p collectors` 전체 실행에서만 기존 `research_worker` 6개가 QA
`DATABASE_URL` 부재(`NotPresent`)로 실패했다. 이는 §0.6·§0.11에 이미 기록된 동일한
호스트 환경 제한이며 이번 artifact 변경의 회귀로 판정하지 않는다. 실제 QA PostgreSQL이나
운영 DB를 임의로 대체해 green을 만들지 않았다.

### 0.34 소유자 베타 추천 경로 구현 완료, 출시는 아직 차단 (2026-08-24)

`d02d0d9..fd10488`에서 작업 2의 봉인 artifact를 소비하는 **추천 전용 오프라인 코드
경로**를 끝까지 연결했다. 이 범위는 데이터 승인·운영 배포가 아니라, 승인된 입력이 나중에
주어졌을 때만 움직이는 fail-closed 구현이다.

- `299296f`는 일반 추천 원장과 분리된 owner-beta run/item 테이블, 강제 RLS, 불변 필드와
  상태 전이 제약을 추가했다. `a81e951`~`68ed735`는 다섯 승인 pin과 전략 snapshot을 봉인한
  큐 입력, 가격 전용 factor/target 계산, 원자적 결과 발행을 구현했다.
- `4d564c4`·`1912a0e`·`3f6bbfb`는 전용 worker 실행, lease 상실·취소·stale claim 복구,
  commit-pinned release 배포 경계를 추가했다. 기존 recommendation worker나 일반 추천
  테이블이 이 작업을 대신 claim하지 않는다.
- `49b1ff5`와 `969bde4`는 소유자 전용 POST enqueue 및 GET history/detail을 제공한다. GET은
  역사 조회이므로 현재 입력 materialization mode와 독립적이지만, owner role·추천
  entitlement·actor-scoped RLS를 계속 요구한다. 상세 응답은 실행 당시 `as_of` entitlement를
  재검사하고, ETF11 정확 집합·고정 6자리 비중·reason/factor 상한·고정 flag·hash·상태별
  nullability·수명주기 시각을 검증한 뒤에만 반환한다. OpenAPI 72개 operation과 생성
  TypeScript가 동기화됐다.
- `fd10488`은 일반 추천 화면과 분리된 소유자 전용 생성·polling·이력·상세 화면을 추가했다.
  모든 상태 화면에서 `Owner-only`, `Vendor snapshot`, `Non-strict PIT`,
  `Price-return only`를 유지하고, 클라이언트도 정확한 ETF11과 결과 의미를 엄격히 검증한다.
  Member는 product client 생성 전에 차단되며 일반 모드는 기존 endpoint를 유지한다.

독립 API/Web 적대 리뷰는 최초 `NEEDS_REMEDIATION` 뒤 수정된 경계를 각각 `ACCEPT`했다.
최종 로컬 검증은 `cargo test -p api-server` 전체 PASS, owner-beta 단위 17개와 OpenAPI 계약
11개 PASS, API all-target clippy warning-zero, Web 18파일 104개 PASS, TypeScript PASS,
OpenAPI check PASS다. Web lint의 기존 `document.cookie` 경고 2개와 Biome 설정 deprecation은
이번 변경과 무관하게 남아 있다.

후속 출시 게이트에서는 `npm run build --workspace @lagrange/web` 프로덕션 빌드,
`deploy/compose/owner-beta-static-check.sh`, historical artifact self-test,
production image·production ops 정적 검사, 전체 `scripts/ops/static-check.sh`, 격리된 임시
release apply/rollback을 수행하는 `production-ops-self-test.sh`가 모두 PASS했다. 실제 이미지를
빌드·설치하거나 현재 설치 release를 바꾸지는 않았으며, 이 결과는 QA DB/RLS·backup/restore
rehearsal이나 운영 기동 증거를 대신하지 않는다.

추가로 fake-Docker 기반 `build-production-images-self-test.sh`, DB·Docker·KIS·network를 쓰지
않는 operator attestation self-test, disposable PostgreSQL 검증 workflow의 정적 검사도 PASS했다.
상위 `scripts/ops/self-test.sh`는 제품 assertion에 도달하기 전, 이 호스트 `fakeroot`가 fixture의
numeric ownership `10001:10001`을 설정하는 단계에서 `EINVAL`로 중단됐다. 이는 앞서 확인된
실행환경 제한이며 PASS로 처리하지 않았고, 실제 root/CI 환경에서의 전체 self-test 증거는 남아
있다.

합성 API와 로컬 Next 서버만 사용하는 전체 Playwright suite도 시도했다. localhost bind는
샌드박스 밖 재실행으로 통과했지만 Chromium headless shell이 호스트의 `libasound.so.2` 부재로
기동하지 못해 36건 모두 test body 이전 launch 단계에서 중단됐다. 따라서 브라우저 E2E를
PASS했다고 보지 않았다. 당시 suite에는 owner-beta 전용 브라우저 시나리오도 없으므로, 필요한
시스템 라이브러리가 있는 CI/QA에서 기존 suite와 새 owner-beta owner/member·성공/차단 시나리오를
실행하는 증거가 남아 있다.

`857e37f`는 그 누락됐던 owner-beta Web E2E fixture와 3개 시나리오를 추가했다. Owner 성공
화면은 `OWNER_ONLY`·`PRICE_RETURN_ONLY`·vendor snapshot·non-strict PIT와 ETF11 결과를
확인하고, Member는 제품 정보와 payload를 열거하지 못하며, entitlement 차단 시 Owner 화면은
계약 라벨만 유지하고 종목 payload를 숨긴다. 합성 list/detail 응답은 실제 Zod 계약 파싱 PASS,
전체 Playwright discovery는 8파일 39건, Web 단위 18파일 104건·TypeScript·lint·프로덕션
빌드는 PASS했다. 이후 호스트의 격리된 Chromium 라이브러리 경로를 적용하고 합성 API·Next 서버를
직접 수명주기 관리하는 공식 `candidate-web-e2e.sh`로 test body를 실행했다. 첫 실행은 기존 36건과
새 2건이 통과하고 새 성공 시나리오 1건만 접근성 이름 부분 일치로 두 region을 선택해 실패했다.
locator에 `exact: true`를 추가한 뒤 전체 **39/39 PASS**했다. 이로써 로컬 브라우저 계약 증거는
닫혔지만, 합성 API 증거이므로 실제 PostgreSQL/RLS나 운영 API·데이터를 검증한 것은 아니다.

처음에는 이 호스트에 `DATABASE_URL`이 없어 새 DB 통합 테스트와 job-queue
publish/recovery 테스트가 clean skip했고, 전체 suite의 `research_worker` 6건도 명시적인
`required QA DATABASE_URL ... NotPresent`에서 중단됐다. 이후 기존 컨테이너와 분리된
루프백 전용 일회용 PostgreSQL 18.4를 고정 digest로 기동했다. 여기서 owner-beta
publish/recovery 5/5, 가격 전용 API 2/2, 중앙 owner 접근 경계 1/1, `research_worker`
68/68이 실제 실행 PASS했다. 이 증거는 RLS·cancel/claim-loss/recovery 경로를 포함하지만
합성 fixture와 일회용 DB에 대한 QA 증거이며 운영 DB·실데이터 증거는 아니다.

CI 전제도 그대로 재현했다. `uv sync --locked`로 ignored `nt/.venv`를 복구하고,
`PYTHON=nt/.venv/bin/python`, 결정론적 `data/phase0`, 위 QA `DATABASE_URL`을 제공했다.
소켓이 필요한 test는 샌드박스 밖 로컬 격리 환경에서 실행했다. 이 조건에서
`cargo test --workspace --locked --no-fail-fast`가 최종 **exit 0**으로 끝났고,
workspace all-target/all-feature strict Clippy `-D warnings`와 rustfmt도 PASS했다.
같은 생성 데이터와 잠금 기반 환경에서 `uv run --project nt pytest -q`는 357 passed / 2
skipped였다. 경고 6개는 NautilusTrader 내부의 `Timestamp.utcnow` deprecation뿐이며 실패는
없다. backup policy의 완전성·hash·tamper·secret-exclusion·결정론 검사도 6/6 PASS했다.

이 전체 재실행은 기존 계약 테스트가 놓친 새 migration 결함 하나를 실제로 잡았다.
`0050/0051` guard가 tenant RLS를 순회하며 `app.actor_user_id` custom GUC를 설정한 뒤 빈
문자열로 복원했는데, PostgreSQL 연결에는 빈 custom GUC가 남아 구버전 strict RLS의 직접
`::uuid` 변환을 깨뜨렸다. `3f9ba1d`는 이미 `ACCESS EXCLUSIVE`로 잠긴 run table에서만
guard 동안 `NO FORCE RLS`로 전체 행을 검사하고 즉시 `FORCE RLS`를 복원하도록 바꿨다.
tenant GUC를 전혀 쓰지 않는 순서 자체를 정적 계약으로 고정했고 migration 통합 31/31이
통과했다.

독립 PostgreSQL validator는 첫 실실행에서 `deploy/db/Dockerfile`의 필수
`LAGRANGE_CODE_COMMIT` build arg를 Compose override가 전달하지 않는 별도 하네스 결함도
찾았다. `4485d23`은 정확한 nonzero 40-hex HEAD를 db-tool build와 sanitized evidence에
전달하고 self-test로 배선을 고정했다. 재실행은 commit
`4485d23fffcc23caa4cb96cf0804b8d93f6c94e2`에서 **APPROVED**였다. 고정 PostgreSQL digest,
직접 service-role 6개 로그인, 0038→0039→0040→0041과 rerun/rollback guard, DB-gated suite가
skip 없이 통과했다. 검증용 컨테이너·네트워크·임시 볼륨은 모두 제거했다. production Web
build와 browser 39/39는 위와 같이 PASS했지만 새 production image의 실제 build/install과
현재 설치 release 변경은 하지 않았다.

**출시 차단은 그대로다.** embedded approval registry는 비어 있고 정확한 7파일 KSD action
pin도 아직 독립적으로 확정되지 않았다. 실 artifact·DatasetManifest·5-pin을 만들거나
등록하지 않았고, 현재 설치 release `66b2a8c`를 교체하거나 systemd/Compose를 활성화하지
않았다. owner-beta 백테스트 입력·실행·조회/UI도 아직 없다. 다음 안전 순서는 아래 Phase A
분석 승인을 얻어 simulation 의미를 확정하고, 백테스트 전용 경로를 별도 구현하는 것이다.
실 승인 pin 없이는 materialize/register/추천·백테스트 실실행으로 넘어가지 않는다.

백테스트 Phase A의 두 read-only `$paseo-delegate` 분석은 계획과 프롬프트까지 확정했지만 아직
실행되지 않았다. 외부 Codex 워커에 저장소 소스가 전달될 수 있어 명시적 소유자 승인이 필요하다는
보안 게이트에서 Paseo 실행이 차단됐고, 생성된 워커는 없다. native subagent로 우회하지 않았다.
소유자가 해당 소스 전달을 승인하면 두 분석만 시작하며, 그 승인은 라이브 데이터·DB 쓰기·배포·
계정/주문 API 권한으로 확대되지 않는다.

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

위 표는 2026-08-17 실행의 역사 증거다. 2026-08-21 소유자 결정으로 KIS
개인 단독 사용 entitlement는 해소됐으며, 추가 계약·5인 사용 권리 확인을 다시
요청하지 않는다. 다음 게이트 재실행에서는 새 ACTIVE KIS metadata로 E1을
재평가한다. X1/X2는 별도 Live 프로젝트의 조건이지 read-only 출시에 필요한 조건이
아니다.

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
| 7 | 리밸런싱 미리보기 UI | **코드 작업** | ✅ **완료·리뷰 반영** (`79da609`+`e16b9da`, e2e 고정 `a6a9438`, §0.17~§0.19) |
| 8 | paper-runner·recommendation-runner 배포 서비스 활성화 | **운영자** | ◐ 배포 계약/preflight 완료, 운영 secret·volume·systemd 설치 대기(§2.12) |
| 9 | Auth0 vendor 스위트 실제 실행 → E2 증거 갱신 | **운영자** | ✅ **완료** — 스위트 5/5 통과(§2.8), phase1 게이트가 **E2 PASS**를 발행(§2.10) |
| 10 | phase-0 골든에 수수료 필드 추가 재승인 | **사장님 결정** | ⛔ 동일 |
| 11 | KIS 실제 provider endpoint / 기존 credential 재사용 / KIS 실계좌 | **운영자 provisioning / 별도 Live 범위** | ◐ 개인 단독 사용 entitlement는 08-21 해소. 기존 등록 KIS 키를 보호된 secret 경로에서 재사용하며 새 키를 요구하지 않는다. read-only endpoint 검증은 운영 작업이고 실계좌는 별도 Live 프로젝트다 |
| 12 | KOSPI200/KOSDAQ150 개별주식 후보 연구 vertical | **코드 작업** | ✅ **완료·독립 리뷰 OK** (§2.9, `ac97970`~`8c5ef9d`) |

Paper 엔진·추천 파이프라인·Paper 연계와 multi-universe 후보 연구의 저장소 내부 이음매는 완료됐다. KIS 개인 단독 사용 권리는 해소됐고 provider wiring과 기존 credential도 있다. 남은 것은 보호된 secret 경로·runtime copy 확인, production endpoint 검증, 운영 backfill과 dataset pin이다. **새 KIS 키를 요구하지 않는다. 다음 순서는 운영 호스트 provisioning이며, 실제 승인 범위는 계속 소유자 한 명이다. Member/Live를 활성화하지 않는다.**

### 4.1 소유자만 할 수 있는 것 — 외부 조달

| 항목 | 구체적으로 |
|---|---|
| **E1** KIS data-rights/entitlement | **해소(08-21):** 개인 단독 사용 권리를 소유자가 확정했다. ADR-0005와 해시 고정 `configs/data-rights/kis.entitlement.json`이 ACTIVE이며 `usr_owner`만 포함한다. 다중 사용자 권리를 다시 묻지 않는다. 기존 등록 KIS credential을 재사용하고 새 키를 요구하지 않는다. 실제 endpoint 검증, DB entitlement 등록, Raw volume, 초기 backfill/dataset pin은 별도 운영 provisioning이다 |
| **E2** Auth0 테넌트 | **해소(08-17):** 테넌트 선택·confidential client 배선(08-12, §3.9), Linux 호스트 secret 배치와 실 테넌트 vendor 스위트 5/5 통과(§2.8)에 이어, phase1 게이트가 **E2 = PASS**를 발행했다(§2.10). 이 항목은 더 이상 외부 조달 대기가 아니다 |
| **X1/X2** KIS 실계좌 | 별도 Live 프로젝트의 조건. read-only 개인용 출시에 필요하지 않으며 현재 범위에서 계속 비활성 |

KIS 개인 단독 사용 권리는 더 이상 외부 조달 항목이 아니다. 과거 E1 판정은 역사
증거로 유지하고 새 metadata를 사용해 게이트를 재실행한다. Member 접근과 Live는
권리 추정으로 넓히지 않고 명시적으로 비활성 상태를 유지한다.

### 4.2 소유자 결정 — 9건 모두 해소 (결정 당시 기록 유지)

1. ~~**phase-0 골든에 수수료 필드를 넣을지**~~ **이번 베타에서 이연 (2026-08-23).** 넣는 것은 승인된 기준값을 바꾸는 명시적 재승인 행위라 보류한다.
2. ~~**Phase 4 우선순위**~~ **이번 베타 이후로 이연 (2026-08-23).** §4.4 참조.
3. ~~**기업행사 7개 클래스 중 6개의 canonical 매핑**~~ **무상증자만 자동 처리하고 나머지는 fail-closed 유지 (2026-08-23).** 일일 EOD 경로
   (`crates/market-data/src/normalize.rs:1045`)는 ETF11 종목이면서 이벤트 날짜가 대상일과
   같은 non-bonus 기업행사를 만나면 그날 실행 전체를 닫는다. 매핑된 것은 bonus-issue
   하나뿐이다. **이건 매핑 코드를 안 짜서가 아니다** — 차단 사유가 코드에 적혀 있듯이
   KSD가 canonical action 계약이 요구하는 필드를 주지 않는다(dividend는 record/pay date만
   있고 문서화된 ex-date도 공시 시각도 없다). 매핑하려면 canonical 계약을 확장하고 어떤
   PIT 주장을 할지 정해야 하므로 원칙 1·4에 걸리는 **소유자 결정**이다.
   실제로 터지는지는 미확인 — 저장소 자체 증거(`docs/runbooks/stage6-source-contracts.md:550`)
   는 "ETF는 배당이 아니라 분배금을 지급한다"고 적고 있어 KSD `dividend`가 ETF11에 대해
   빈 응답일 가능성이 있으나, 그 관찰은 KIND 유형 체계에서 나온 것이고 KSD 응답을 확인한
   것이 아니다. §0.15 (3)/(4)의 첫 credentialed 실행에서 read-only GET 한 번이면 결판난다.

4. ~~**KIS entitlement의 DB 등록**~~ **등록·활성화 완료 (2026-08-23, §0.27) — 다만 §4.2-6이 남았다.** (아래는 결정 당시 기록) — `data_entitlements`가
   비어 있다. **오늘의 EOD publication을 막는 것은 이것이 아니다**(§0.23 — 08-18에는 같은 상태로
   성공했고, 실제 실패는 `PRICE_CURATION_FAILED`다). 다만 DB 함수
   `resolve_price_dataset_entitlement`와 `crates/auth`의 API 권한 게이트가 이 테이블을 읽으므로
   **candidate/Curated 승격과 사용자 화면에는 필요하다.** `provision-entitlement.sh register`는 `PENDING` 레코드를
   요구하는데 `configs/data-rights/kis.entitlement.json`은 `ACTIVE`다. **에이전트가 lifecycle을
   PENDING으로 바꾼 사본을 만들어 통과시키지 않았다** — 검증기를 만족시키려 권리 상태를
   위조하는 것이고, `activate --activation-date`도 소유자가 정할 값이기 때문이다. 소유자가
   (a) PENDING 레코드를 작성해 register → activate 2단계를 밟거나, (b) ACTIVE 레코드를 직접
   받아들이도록 흐름을 조정할지 결정해야 한다. 필요한 나머지 입력은 확인해뒀다:
   owner UUID `00000000-0000-4000-8000-000000000042`(users 테이블의 유일한 행),
   문서 `docs/decisions/0005-kis-personal-use-entitlement.md`, `jq` 설치됨.
   메타데이터/문서 파일은 0400/0600을 요구하므로 `/etc/lagrange/`에 보호 사본을 만들어 뒀다.

   **2026-08-23 추가 조사로 결정이 한 필드까지 좁혀졌다 (§0.25-②).** 이제 이것은 추정이
   아니라 **DB가 명시적으로 거부하는 차단 항목**이다 — PostgreSQL 로그에
   `ERROR: price dataset requires one exact active entitlement`
   (`public.resolve_price_dataset_entitlement` line 26). `register`의 jq 검증기를
   실제로 돌려 보니 **통과하지 못하는 조건은 `.lifecycle == "PENDING"` 하나뿐**이고
   provider·covered_datasets(9개)·covered_uses(10개)·covered_users·effective_from·
   effective_until·contract_document 해시는 전부 통과한다. 즉 (a)안은 **`lifecycle`만
   `PENDING`으로 둔 임시 입력 파일 하나**를 만들어 register → activate 2단계를 밟는 것이고,
   이는 권리 상태를 위조하는 것이 아니라 **설계된 lifecycle을 그대로 걷는 것**이다
   (저장소의 `ACTIVE` 레코드는 그 흐름의 최종 상태 문서다).
   `--activation-date`에 넣을 값도 레코드에 이미 있다: `effective_from = 2020-01-31`
   (`effective_until = 9999-12-31`). **소유자가 정할 것은 "(a)로 간다"와 활성화 날짜뿐이며,
   실행은 에이전트가 할 수 있다.**

5. ~~**KIS 달력의 `source_version` 모델**~~ **(나)안으로 해소 (2026-08-23, §0.29)** — 비교를 세션 날짜 범위로 좁혔다. 마이그레이션 불필요 — 테이블 UNIQUE 제약은 `0022`부터 이미 날짜 단위였고 교차 날짜 규칙은 `sink.rs`에만 있었다. 아래는 결정 당시 기록 — DB는
   `(exchange, source_version)` 하나에 `content_sha256`이 하나여야 한다고 강제하는데
   (`sink.rs:570`), KIS `chk-holiday`는 날짜마다 다른 문서를 주면서 `source_version`은
   `kis-chk-holiday-v1:schema-1`로 상수다. **둘째 날 publication이 반드시 충돌한다.**
   불변식 자체는 벤더가 발행된 달력을 몰래 바꾸는 것을 잡는 좋은 장치라 그냥 풀 수 없다.
   후보 (a) `source_version`에 세션 날짜를 포함 (b) KIS 달력을 별도 모델로 분리
   (c) 불변식을 세션 날짜 단위로 완화 — (c)는 원래 지키려던 것을 잃는다. 어느 쪽이든
   "source version"이 무엇을 뜻하는지에 대한 결정이라 원칙 1·4·6에 걸린다.

6. ~~**entitlement `contract_reference`의 권위**~~ **(a)안으로 해소 (2026-08-23, §0.28)** — operator-attestation URI로 등록해 함수가 해결된다. 아래는 결정 당시 기록 —
   `resolve_price_dataset_entitlement`는 Raw가 citing하는 참조와 DB `contract_reference`의
   정확 일치를 요구하는데, 전자는 `operator-attestation://l1nnx/kis-readonly/2026-08-18`,
   후자는 승인 레코드의 `document_reference`인 `repo://docs/decisions/0005-...md`다.
   한 필드가 "계약 문서 위치"와 "Raw가 citing하는 키" 두 역할을 겸하는 것이 원인이다.
   후보 (a) operator-attestation URI로 등록 — self-test 픽스처가 그 형태이고 기존 불변 Raw
   이틀을 살린다 (b) `RESEARCH_ENTITLEMENT_REFERENCE`를 repo:// 로 변경 — 앞으로만 유효하고
   08-18·08-19는 영구 미발행 (c) 두 역할을 별도 필드로 분리. **에이전트가 승인 레코드에 없는
   참조로 권리를 등록하지 않는다.**

7. ~~**`instruments` 카탈로그와 price coverage FK**~~ **(b)안으로 해소 (2026-08-23, §0.29)** — price 복구 경로가 카탈로그를 등록한다. 커버리지 하한은 master가 아니라 승인된 `APPROVED_EFFECTIVE_FROM`을 쓴다. 아래는 결정 당시 기록 —
   `publish_candidate_price_publication`이 `candidate_price_instrument_coverage`에 쓰고 그
   FK가 `instruments(id)`를 요구하는데 그 테이블은 0행이고 **프로덕션 writer가 없다**
   (`register_candidate_instruments`는 호출처 0). 반대로 `worker.rs:2927`의 계약 테스트는
   price 경로가 카탈로그를 요구하지 **않는다**고 단언한다. 코드 계약과 DB 스키마 중
   어느 쪽이 권위인지가 결정이다. 후보 (a) price publication이 coverage를 쓰지 않게
   (b) 고정 ETF 11종 등록 경로 신설 (c) FK 완화 — (c)는 coverage의 참조 무결성을 잃는다.

8. ~~**출시 스위치 — `ready` 전환과 five-pin 승인의 절차**~~ **owner-only vendor-snapshot beta로 해소 (2026-08-23~24, §0.32).**
   §0.15의 순서는 DatasetManifest·five-pin 확정과 계약 flag 전환에서 끝나는데, **그 행위의
   기준·주체·증거 요건이 이 문서 어디에도 없다.** 원칙 5(게이트 증거를 조용히 바꾸지 않는다)에
   따르면 `vendor_snapshot`/`strict_pit`/`ready` 전환은 명시적 승인 행위여야 한다. 정할 것:
   (a) 전환의 선행 조건 — 무개입 연속 발행 며칠을 요구하는지, 어떤 증거를 채택하는지,
   (b) DatasetManifest·five-pin 승인 절차(Stage4B CLI가 출력하는 `manifest_sha256`를 승인
   목록에 커밋하는 그 행위와 동일 계열, §0.17), (c) `strict_pit`은 과거 이력에 대해 영구
   불가이므로(§0.6, §0.14) 어떤 범위에서 무엇을 주장하며 출시하는지.
9. ~~**과거 데이터 정책과 SC-02 기준**~~ **Stage5 비엄격 PIT 가격 전용 6.5년 베타로 해소하고 10년 SC-02는 이연 (2026-08-23, §0.32).** 현재 발행분은 08-18·08-19
   이틀이다. 반면 Stage5의 1,608거래일(2020-01-31~2026-08-19, 매 거래일 정확히 11종목)은
   **Raw와 정규화까지 완료**돼 있고 Curated/DB publication/five-pin 앞에서 멈춰 있다(§0.2).
   즉 (b)안은 새 수집이 아니라 이미 가진 것을 발행하는 선택이다. 요구사항 SC-02는
   "10년 이상 일봉 백테스트"인데 Stage6 범위 시작일은 `2020-01-31`(약 6.5년)이다. 후보
   (a) KIS read-only 재백필 — 요청 예산 승인 필요, (b) Stage5 vendor snapshot을
   `vendor_snapshot=true`로 발행 — PIT 주장 없이 백테스트 입력으로만, (c) 당분간 일일 증분만.
   어느 쪽이든 **SC-02 기준을 유지할지 수정할지가 함께 결정된다.** 원칙 1·4에 걸린다.

### 4.3 코드 작업 — 착수 가능, 권장 순서

1. ~~**`phase1-gate.sh` native Linux 이식.**~~ **완료 (2026-08-17, `5b3f832`, §2.10)** — WSL 가드·PATH·`CARGO_TARGET_DIR`·DB 포트를 정리했고, 이식 과정에서 드러난 거짓 PASS 3건도 함께 닫았다. pyarrow 전제는 이 게이트에는 해당하지 않는다(추천 계산 경로의 문제이며 phase1 검사는 `prepare_phase0.py`를 부르지 않는다).
2. ~~**전체 게이트 재실행.**~~ **완료 (2026-08-17, `61af2bb`, §2.12)** — Phase 1/2/3, 양 단계 failures, 실제 PITR, 종합 F3를 재실행했다. 당시 판정은 외부 권리·실계좌 때문에 `BLOCKED_EXTERNAL`이었다. 권리 판정은 2026-08-21에 해소됐고 실계좌는 read-only 범위 밖이다. F1/F2/F4 판정문은 여전히 사람 재검토가 필요하다.
3. ~~**리밸런싱 미리보기 UI.**~~ **완료 (2026-08-22, `79da609`+`e16b9da`, §0.17~§0.19)** — 생성→폴링→적용 전체와 `INDICATIVE_NEXT_OPEN_REPLAN_REQUIRED` 경고 노출. 리뷰에서 나온 MAJOR 3건(권한 격리, 계좌 전환 리셋, Owner 역할 게이트)을 반영했고 e2e로 seam에 고정했다. Live 주문은 계속 범위 밖이다.
4. **배포 서비스 활성화.** Paper/recommendation/candidate runner에 실제 role-scoped DB URL과 curated/raw volume을 호스트 Secret Manager에서 주입한다. 저장소에는 비밀값을 넣지 않는다.
5. **실제 KIS provider와 운영 원천 활성화.** 새 credential을 발급하지 않고 기존 등록 KIS App Key/App Secret의 보호된 source/runtime secret을 검증한다. 이후 token/endpoint를 확인하고 확정된 entitlement metadata를 운영 DB에 등록한 뒤 `research_writer`, migration, Raw volume을 검증해 KIS calendar/EOD/instrument/corporate-action 원천을 공급한다. 고정 ETF 백필 후 후보 bridge와 KOSPI200/KOSDAQ150 source set은 별도 승인한다. 원천이 없거나 오래되면 게이트는 계속 닫힌다.

6. ~~**누적 큐레이션이 이틀치를 발행하지 못하는 결함.**~~ **코드 수정 완료 (2026-08-23,
   `b67ae1b`, §0.24) — 운영 검증은 남음.** 폴백을 병합 세션 집합에서 유도하도록 바꿨고,
   배치당 단일 세션 calendar를 주는 회귀 테스트를 세웠다. **남은 것: worker 이미지 재빌드 →
   `org.opencontainers.image.revision`이 새 커밋과 일치하는지 확인 → 08-19 재실행 →
   version=3과 DB publication 확인.**
7. ~~**`CurateError` 진단 가능성.**~~ **완료 (2026-08-23, `485c937`, §0.25-④)** —
   `PIPELINE_FAILED`와 `PRICE_CURATION_FAILED` 양쪽에 변종 이름을 담는 `detail` 필드를
   추가했다. 같은 벽에 두 번 부딪힌 뒤에 고쳤다.
8. ~~**08-19 `PIPELINE_FAILED`의 변종 규명.**~~ **완료 (2026-08-23, §0.26)** —
   `Sink(stage=Publish, SinkError::Conflict: calendar source version differs for KRX
   kis-chk-holiday-v1:schema-1)`. 원인은 §4.2-5로 등재했다.

**작지만 기록해 둘 잔여 항목** (아키텍트 검토에서 발견, 차단 아님): `strategy_promotion`(§3.5)이 계좌 단위라 그 계좌에 묶인 주문 전부를 승격된 것으로 본다 — 운영 원천이 채워져 결정적 검사가 되기 전에 재검토할 것. 이전에 기록한 `positions` 소유자 재확인 gap은 0038의 account-owner 복합 FK로 닫혔다. §0.16의 A(Paper 런타임 `CurateStore` 이중 경로)와 C(CI가 Python 스위트를 실행하지 않음)는 **2026-08-22에 종결됐다(§0.17)**.

9. **릴리스·타이머를 `6c2d18b`로 정렬 (2026-08-23 미완, `2026-08-24` 16:30 이전).** 이미지와 main은
   `6c2d18b`이고 릴리스·타이머는 `66b2a8c`다. 일일 경로 스크립트는 동일하므로 동작에는
   문제가 없지만 셋이 어긋난 상태다. `install-kis-daily.sh --apply`는 `Persistent=true`
   때문에 16:30 이후 설치가 금지되고, 설치 스크립트가 **이전 세대 타이머를 끄지 않으므로**
   교체 후 옛 유닛을 직접 `disable --now` 해야 한다(§0.25-⑤).
10. **`fsc-krx-listed` 수집과 실제 상장일 승격 (§0.29).** `instruments.listed_at`은 현재
   플랫폼 커버리지 하한(`2020-01-31`)이지 거래소 상장일이 아니다 — 우리가 수집하는 어떤
   소스도 실제 상장일을 주지 않는다. `register_candidate_instrument`는
   `ON CONFLICT (id) DO NOTHING`이라 **이 경로로는 영원히 못 고친다.** 실제 상장일을 넣으려면
   fsc-krx-listed 수집 + 별도 갱신 마이그레이션이 필요하다.
11. **daily-state 검증의 진단 가능성 (신규, 2026-08-23, §0.31-①).**
   `scripts/ops/kis-daily-production.sh`의 상태 파일 검증이 python stderr를 `2>/dev/null`로
   버리고 네 원인(missing/stale/malformed/not appendable)을 한 문장으로 접는다. 오늘 16:30
   실패의 원인 규명이 여기서 막혔다 — `CurateError`(§0.24-⑥)와 `PIPELINE_FAILED`(§0.25-④)에서
   이미 두 번 값을 증명한 조치와 같다. identity 불일치일 때 **어느 필드가 다른지**까지 내면
   같은 실패를 재조사 없이 읽을 수 있다(값은 모두 우리가 만든 문자열이며 provider 응답이 아니다).
12. **릴리스·이미지·타이머 3자 정렬 자동 검사 (신규).** §0.23이 "일치 검사가 없다"로 남긴
   항목이고 §0.25에서 셋이 어긋난 실제 비용을 치렀으며 **지금도 어긋나 있다**(main·이미지
   `6c2d18b` 대 릴리스·타이머 `66b2a8c`, §0.31-②). 배포 후 어긋남이 정상적으로 발생하는
   구조이므로 검사가 없으면 매번 사람이 기억해야 한다. 함께: 이미지 revision 라벨이 빌드된
   10개 중 4개에만 있다(§0.21 두 번째 정정) — 진단 수단이 함대의 다수에 없다.
13. **요구사항 갭 6건 (신규, §0.31-④).** 출시를 막지는 않으나 소유자 단독 운영에 실제로
   영향이 있는 둘을 먼저 볼 것 — 관리자 화면(Must인데 웹이 빈 화면)과 알림(`EmailTransport`가
   항상 실패하는 스텁이라 장 마감 추천 도착을 화면 확인 외에 알 방법이 없다).
14. ~~**owner-beta 가격 전용 추천 코드 경로.**~~ **완료 (2026-08-24,
    `299296f..fd10488`, §0.34).** 전용 DB/RLS → 봉인 queue 입력 → 계산·원자 발행·복구 →
    owner-only POST/GET API → 엄격한 Web 화면까지 연결했다. 독립 API/Web 재리뷰 `ACCEPT`,
    API 전체 테스트와 Web 104개 테스트를 통과했다. 단, DB 통합 테스트는 이 호스트의
    `DATABASE_URL` 부재로 clean skip이므로 운영 증거가 아니다.
15. **owner-beta 백테스트와 운영 출시 증거.** 추천 경로와 같은 승인 5-pin·능력 flag를
    사용하는 전용 백테스트 제출/worker/result/API/UI를 구현하고, QA PostgreSQL에서 RLS·복구
    통합 테스트를 실제 실행한다. 그 뒤 production Web/image build, 정적 운영 게이트,
    backup/restore rehearsal을 수행한다. 승인 레지스트리와 정확한 KSD action pin이 비어 있는
    동안 실 artifact 생성·등록·추천/백테스트 실행은 계속 차단한다.

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

### 5.1 현재 Linux 호스트의 함정 (위 표와 달리 지금 유효하다)

| 함정 | 대응 |
|---|---|
| **`/tmp` 사용자 쿼터 소진 → 모든 에이전트 셸이 동시에 죽는다** | `/tmp`은 디스크가 아니라 **tmpfs 7.3G**(램의 절반)이고 `usrquota`가 걸려 있다. `df`가 여유를 보여줘도 사용자 쿼터를 넘기면 `EDQUOT`다. 에이전트 툴이 명령 출력을 `/tmp`에 캡처하므로, 쿼터가 차면 `echo`조차 **출력 없이 exit 1**로 죽어 원인 파악이 매우 어렵다. 병렬 Rust 빌드가 주범이다. 긴 작업 전 `du -sh /tmp/* \| sort -rh \| head`로 확인하고, 모든 워커에 `TMPDIR`을 `/data` 아래로 지정할 것 — 다만 cargo/rustc가 일부 경로에서 `/tmp`을 직접 쓰므로 `TMPDIR`만으로는 부족하다 (2026-08-22, §0.18) |
| **`TMPDIR`을 `/data` 아래로 옮기면 PostgreSQL 검증 게이트 2개가 로컬에서만 실패한다** | 바로 위 칸의 `/tmp` 쿼터 대응과 **정면으로 충돌한다.** `deploy/db/integration-validation/validate.sh:101`은 `mktemp -d "${TMPDIR:-/tmp}/..."` 결과가 **리터럴 `/tmp/lagrange-pg-validation-self.*`와 일치하는지 검사**하고, 아니면 `unsafe self-test temp path`로 죽는다. `static-check.sh`는 이것을 호출하므로 같이 실패한다. 메시지가 권한 문제를 가리키는 것처럼 읽히지만 **경로 고정 가드**이며 디렉터리 모드와 무관하다(0777→0700으로 바꿔도 동일하게 실패). 이 두 게이트만 기본 `TMPDIR`로 돌릴 것. CI는 `TMPDIR`을 건드리지 않아 green이다 (2026-08-23, §0.24) |
| **`umask 0002` 때문에 정확한 파일 모드를 요구하는 static-check가 로컬에서만 실패한다** | 이 호스트의 umask는 `0002`라 체크아웃된 스크립트가 `0775`가 된다. `scripts/ops/static-check.sh`와 `deploy/secrets/runtime-static-check.sh`는 정확히 `0755`를 요구하므로 로컬에서 실패한다. **git은 `100755`로 기록하고 CI 러너는 umask 0022라 CI에서는 통과한다.** `( umask 0022; git clone --depth 1 ... )` 후 실행하면 확인된다. 이 실패를 트리의 결함으로 오인하지 말 것 (2026-08-22, §0.19) |
| **docker 그룹이 `/etc/group`엔 있는데 `id`엔 안 보인다** | 계정은 이미 `docker`(gid 983) 멤버다. 그런데 그룹 멤버십은 로그인 시 프로세스에 박히므로, 그보다 먼저 뜬 프로세스의 자손은 갖지 못한다. **UI 세션을 리로드해도 부모인 Paseo 데몬이 낡은 그룹 집합을 유지하면 소용없다** — `/proc/<pid>/status`의 `Groups:`로 조상 체인을 확인하면 어디서 끊기는지 보인다. 데몬을 재시작하는 대신 **`sg docker -c '<명령>'`**으로 획득하면 된다. `sudo`는 비밀번호를 요구해 비대화식으로 못 쓴다 (2026-08-22, §0.20) |
| **`sg`로 감싸면 `LD_LIBRARY_PATH`가 지워진다 → E7이 반드시 실패한다** | `sg`는 setgid 바이너리라 glibc가 보안상 `LD_LIBRARY_PATH`를 제거한다. E7의 chromium은 `libasound.so.2`를 필요로 하는데 이 호스트엔 시스템 설치가 없고 `/home/l1nnx/tools/pwlibs`의 사본을 `LD_LIBRARY_PATH`로 가리켜야만 뜬다. **phase1 게이트는 docker를 호출하지 않는다**(QA DB가 이미 떠 있기를 요구할 뿐)이므로 `sg` 없이 직접 실행할 것. 모르면 E7 실패를 코드 회귀로 오인한다 (2026-08-22, §0.20) |

---

## 6. 이 저장소가 지키는 원칙 (작업자를 위한 요약)

1. **Fail-closed.** 읽을 수 없으면 거부. `Unknown`은 허가가 아니다.
2. **권위는 하나.** 현금은 원장 재생으로만; 세율·수수료는 버전 있는 설정으로만; 파생 수치의 저장 사본은 두 번째 진실이 되므로 대조 없이 믿지 않는다.
3. **검증은 이음매에서.** 컴포넌트 테스트와 게이트 통과는 경로가 동작한다는 증거가 아니다. 실서비스 진입점을 부르는 테스트가 있는지 먼저 확인하라.
4. **없는 값은 없는 채로.** 0 대입·이월·기본값 대체는 하류에서 구분 불가능한 거짓 신호를 만든다.
5. **게이트 증거를 조용히 바꾸지 않는다.** 골든 재승인·게이트 입력 변경은 명시적 행위이며 커밋 메시지에 선언한다.
6. **차단을 위조하지 않는다.** BLOCKED_EXTERNAL은 실패가 아니라 이 시스템이 설계대로 멈춰 있다는 증거다.
