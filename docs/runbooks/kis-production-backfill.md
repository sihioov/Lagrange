# KIS 운영 데이터 백필·승인 런북

이 런북은 한국투자증권(KIS) **읽기 전용** EOD 데이터를 운영 데이터셋으로
승인하는 절차다. 계좌번호·주문 토큰·주문 API는 이 절차에 필요하지 않으며,
Compose `live` profile은 끝까지 비활성 상태로 둔다. API 이용 권리 또는 앱
키가 있다는 사실만으로 데이터 재배포·추천·백테스트 권리가 증명되는 것은
아니다. 실제 entitlement 문서의 해시와 적용 범위가 없으면 게이트는
`BLOCKED_EXTERNAL`로 남는다.

## 현재 범위와 차단선

| 데이터 | 현재 실행 경로 | 상태 |
|---|---|---|
| 고정 11-ETF EOD | KIS wire `kis/kr` → `kis-normalized/kr` → DB logical `KRX/KR` | provider 구현 후 read-only 백필 대상 |
| 거래일·종목 마스터·기업행사 | KIS 응답의 Raw/normalized contract | entitlement·실응답 검증 필요 |
| KOSPI200 후보 | candidate source bridge | 현재 worker는 credentialed candidate를 거부하므로 `BLOCKED_EXTERNAL` |
| KOSDAQ150 후보 | candidate source bridge | 현재 worker는 credentialed candidate를 거부하므로 `BLOCKED_EXTERNAL` |
| 주문·계좌·실거래 | Live profile | 이번 출시 범위 밖, 항상 disabled |

KOSPI200/KOSDAQ150은 ETF 가격 백필에 섞지 않는다. 후보용 수급·재무·시장
상태·지수 편입·섹터 데이터는 별도 source set, 별도 entitlement, 별도
dataset version으로 수집·승인해야 한다. bridge가 준비되기 전에는 계획만
기록하고 API를 호출하지 않는다.

`candidate-runner` 컨테이너는 오늘도 기동할 수 있다. 기본 Docker healthcheck는
프로세스·DB heartbeat(liveness)만 확인하므로 source가 없을 때도 `healthy`일 수
있다. 실제 후보 사용 가능 여부는 `candidate-runner readiness`의
`all_feeds_current`/scheduler gate로 별도 확인하며, source bridge가 없으면
readiness와 후보 실행은 `BLOCKED_EXTERNAL`로 취급한다. 이를 ETF EOD health나
추천 dataset 승인으로 대체하지 않는다.

## 1. 호스트와 설정 preflight

실제 호스트에는 아래 순서로 실행한다. 첫 명령은 변경하지 않는 계획이다.

```bash
scripts/ops/provision-linux.sh --dry-run
sudo scripts/ops/provision-linux.sh --preflight
```

`--preflight`는 `/etc/lagrange`와 데이터 경로의 보호된 조상 디렉터리를
검사하므로 root가 필요하다. 실패하면 `--apply`를 실행하기 전에
경로·계정·권한을 검토한다.
`--apply`는 명시적으로 승인된 root 터미널에서만 실행하며, 계정·디렉터리
생성 외의 삭제·초기화·재귀 복사는 하지 않는다.

```bash
sudo scripts/ops/provision-linux.sh --apply
sudo scripts/ops/provision-db-secrets.sh --apply
sudo scripts/ops/provision-db-secrets.sh --check
sudo scripts/ops/provision-crypto-secrets.sh --apply
sudo scripts/ops/provision-crypto-secrets.sh --check
sudo deploy/secrets/provision-runtime-secrets.sh --scope infrastructure
```

`provision-db-secrets.sh --check` is a root-only, read-only verification of the
exact seven DB source files, accepting either canonical 64-hex or 44-character
standard Base64 values that decode to 32 bytes. It reports
`DB_SECRET_CHECK: PASS` or actionable filenames and metadata/shape reasons
only; it never prints secret values or hashes. If the files have one accidental
trailing LF/CRLF, the explicit root-only
`scripts/ops/provision-db-secrets.sh --strip-trailing-newline` command can
atomically repair a complete all-hex set; mixed or malformed sets are refused.

실제 secret 값은 이 저장소나 명령 인자에 넣지 않는다. 다음 검사는 값 자체를
출력하지 않고 파일 존재·regular/no-symlink·non-empty·소유자·mode와 env/dataset
pin의 shape만 확인한다.

```bash
export LAGRANGE_CODE_COMMIT="$(git rev-parse HEAD)"
sudo env LAGRANGE_CODE_COMMIT="$LAGRANGE_CODE_COMMIT" \
  scripts/ops/validate-production-config.sh \
  --scope infrastructure --env-file deploy/compose/.env
```

infrastructure scope는 KIS key/secret 없이 절대 운영 데이터·runtime 경로,
PostgreSQL 식별자, 정확한 code commit, DB source secret 7개와 runtime copy
10개의 shape/권한, 그리고 global live profile off만 확인한다. KIS entitlement,
production/credentialed fetch 및 candidate 설정은 backfill scope부터 요구한다.
아직 생성되지 않은 Curated manifest와 recommendation five-pin, Auth0/TLS
serving 값도 infrastructure 단계에서는 요구하지 않는다. 검사 결과가
`BLOCKED_EXTERNAL`이면 정상적인 대기 상태다. `INVALID_CONFIG`는 값을 기다릴
문제가 아니라 설정을 수정해야 하는 상태다.

먼저 DB/raw/schema 인프라만 적용한다. 이 단계는 PostgreSQL, role bootstrap,
migration, Raw ownership, schema check one-shot만 실행하며 research-worker,
API/Web 또는 어떤 provider/API call도 시작하지 않는다.

```bash
sudo env LAGRANGE_CODE_COMMIT="$LAGRANGE_CODE_COMMIT" \
  scripts/ops/compose-release.sh --scope infrastructure --plan
sudo env LAGRANGE_CODE_COMMIT="$LAGRANGE_CODE_COMMIT" \
  scripts/ops/compose-release.sh --scope infrastructure --preflight
sudo env LAGRANGE_CODE_COMMIT="$LAGRANGE_CODE_COMMIT" \
  scripts/ops/compose-release.sh --scope infrastructure --apply
```

KIS 없이 Auth0/TLS와 나머지 비-KIS serving runtime copy를 먼저 준비하려면
다음 copy/readiness-only scope를 선택적으로 실행한다. 이 scope는 Docker,
Compose service, provider/API call을 시작하지 않고, `RESEARCH_*`, entitlement,
KIS key/secret, recommendation five-pin도 요구하지 않는다. `compose-release.sh`
에는 이 scope를 추가하지 않는다.

```bash
sudo env \
  LAGRANGE_SECRET_SOURCE_DIR=/etc/lagrange/secrets \
  LAGRANGE_RUNTIME_SECRET_DIR=/etc/lagrange/secrets/runtime \
  deploy/secrets/provision-runtime-secrets.sh --scope serving-prereqs
sudo env LAGRANGE_CODE_COMMIT="$LAGRANGE_CODE_COMMIT" \
  scripts/ops/validate-production-config.sh \
  --scope serving-prereqs --env-file deploy/compose/.env
```

`serving-prereqs`는 DB7, session/CSRF/cursor/Auth0, backup source, TLS source를
전체 preflight한 뒤 reverse-proxy/API/infra DB/research DB/runner DB copy만
설치한다. backup key는 source shape만 확인하고 runtime에 복사하지 않는다.
KIS backfill과 immutable dataset five-pin이 준비되기 전에는 API/Web/edge를
기동할 수 없으므로 이 단계가 serving release를 의미하지 않는다.

이제 승인된 KIS 읽기 전용 자격증명이 준비된 경우에만 research-worker 런타임
copy를 추가하고 backfill scope로 worker image를 준비한다. 값은 저장소·명령
인자·환경변수에 넣지 말고 hidden terminal에서 입력한다.

```bash
scripts/ops/provision-kis-credentials.sh --dry-run
sudo scripts/ops/provision-kis-credentials.sh --apply
sudo scripts/ops/provision-kis-credentials.sh --check
```

이 helper는 두 값을 `root:root`/`0600` source file로 원자적으로 설치·검사할
뿐이며 KIS network/API 호출이나 vendor 검증을 하지 않는다. worker/client에
provider 고유 길이 계약이 없으므로 4096바이트 상한은 accidental paste 방지용
로컬 guard일 뿐이다. KIS data-use/redistribution rights, read-only entitlement와
적용 범위를 실제 문서로 판단·기록하는 책임은 operator에게 있다. 앱 키가
있다는 사실만으로 해당 권리가 증명되지는 않는다.

그 다음 runtime copy와 config 검사를 실행한다.

```bash
sudo deploy/secrets/provision-runtime-secrets.sh --scope backfill
sudo env LAGRANGE_CODE_COMMIT="$LAGRANGE_CODE_COMMIT" \
  scripts/ops/validate-production-config.sh \
  --scope backfill --env-file deploy/compose/.env
```

## 2. 고정 ETF 백필

날짜 범위는 먼저 scheduler-only XKRX artifact의 검증된 session 목록으로 제한한다.
주말과 휴일을 임의로 데이터로 채우지 않는다. 계획 명령은 API·Docker·파일 쓰기를
하지 않는다.

```bash
scripts/ops/backfill-production.sh \
  --start 2020-01-31 --end 2026-08-17 --universe etf --plan
```

운영자가 범위와 읽기 전용 호출을 확인한 뒤에만 실행 guard를 설정한다.

```bash
sudo env LAGRANGE_CODE_COMMIT="$LAGRANGE_CODE_COMMIT" \
  BACKFILL_CONFIRM_EXTERNAL=I_UNDERSTAND_READ_ONLY_KIS_CALLS \
  scripts/ops/backfill-production.sh \
  --start 2020-01-31 --end 2026-08-17 --universe etf --execute
```

실행기는 먼저 검증된 scheduler-only XKRX 날짜 artifact에서 요청 범위의 session
날짜만 읽는다. artifact는 `data/calendars/xkrx/calendar.json`과 manifest의
SHA-256을 검증하고, 범위 밖·symlink·변조·manifest mismatch에서는 fail-closed한다.
토요일/일요일/closure는 worker·Docker·KIS 호출을 만들지 않는다. 하나의
`research-worker --backfill-session-dates DATE[,DATE...]` 프로세스가 정확히
검증된 sorted session list만 순회하며, civil range로 다시 넓히지 않는다. 이
프로세스는 provider와 `TokenManager` 하나를 공유하여 공식 24시간
토큰을 만료 전까지 메모리에서 재사용하고, 토큰 값 자체를 파일에 저장하지 않는다.
발급 실패를 포함한 시도 간격은 최소 1분이며, wrapper는 빠른 프로세스 재실행도
막기 위해 root:root 0600의 비밀이 아닌 마지막 시도 시각만 별도 기록한다. 별도
worker daemon이 실행 중이면 서로 다른 token cache가 생기므로 실행을 거부한다.
같은 provider는 첫 번째 필요한 session day에 `chk-holiday (CTCA0903R)`를 한 번
호출하고, 성공한 응답을 범위 프로세스의 불변 calendar snapshot으로 재사용한다.
이후 session day는 이 캐시된 snapshot으로만 검증한다. snapshot의 날짜 창을
벗어나면 두 번째 휴장일 호출을 하지 않고 `KIS_CALENDAR_SNAPSHOT_MISS`로
fail-closed 중단한다. 그때까지 성공한 날짜는 state에 보존되므로 같은 state를
검토 후 재실행하여 다음 snapshot 창을 순차적으로 진행한다. scheduler artifact가
허용한 session list 밖의 날짜에는 휴장일 API를 호출하지 않는다. artifact의
dates-only/session schedule을 KIS 응답으로 위조하거나 Raw/publication/curation에
주입하지 않는다.
이 계약의 근거는 제공된 공식 XLSX sheet 4 `접근토큰발급(P)`와 KIS 공식
`examples_user/kis_auth.py`의 만료 기반 캐시 예제다. worker는 한 날짜의 durable
canonical EOD DB publication이 끝날 때마다 값/본문 없는 JSON event를 flush하고, wrapper는 exact
date 순서와 batch UUID를 검증한 뒤 해당 날짜의 `PUBLISHED`를 즉시 fsync한다.
따라서 뒤 날짜가 실패해도 앞서 성공한 progress는 재실행에 보존된다. 상태 파일은 현재 실행의 date range,
universe, code commit, entitlement reference와 source scope, XKRX calendar id,
artifact SHA-256, full artifact range를 해시한 V4 pre-run identity에 바인딩된다.
아직 생성되지 않은 curated manifest/five-pin은
identity에 들어가지 않으므로, 백필 후 승인 pin을 `.env`에 입력해도 재개 state가
무효화되지 않는다. 다른 실행의 상태나 구버전 형식은 fail-closed로 거부한다.
실행 중 동시 백필은 `flock`으로 직렬화한다. 실패한 날짜에서 중지하며, 재실행
시 같은 identity에서 이미 `PUBLISHED`인 날짜만 건너뛴다. worker 자체의
deterministic normalized ID와 exact manifest/evidence 비교가 재시도·crash
recovery의 기준이다.

### 2.1 KSD 기업행사 페이지네이션 계약

KSD 여섯 endpoint path의 페이지네이션은 사용자가 승인한 현재 공식
[`koreainvestment/open-trading-api` 예제](https://github.com/koreainvestment/open-trading-api/blob/main/examples_user/domestic_stock/domestic_stock_functions.py)의
endpoint-specific 동작을 따른다. 첫 요청은 `CTS`와 `tr_cont`를 빈 값으로 보내고,
응답 헤더가 정확히 `M`일 때만 같은 query(빈 `CTS`)로 다음 요청을 하며 다음 요청의
`tr_cont`는 `N`으로 보낸다. `F`, 빈 값, 그 밖의 값은 종료로 처리한다. 페이지 수는
최대 10개이며, 같은 응답 bytes가 다시 오면 중복 페이지로 간주해 Raw 저장 전에
fail-closed 중단한다. 페이지는 `*-page-01.json`, `*-page-02.json`처럼 별도 불변
envelope으로 유지하고, 모든 페이지의 응답 검증이 끝나기 전에는 Raw manifest를
공개하지 않는다.

제공된 `docs/kis_openapi_entiredocs_20260818_030007.xlsx`의 KSD 설명은
페이지네이션 불가/`CTS` 공백으로 적혀 있어 위 GitHub 예제와 충돌한다. 이번 운영
범위에서는 사용자가 지정한 최신 공식 GitHub 예제를 KSD의 좁은 `M → N` 규칙에
우선 적용하되, XLSX의 충돌 사실을 이 문서와 테스트에 남긴다. 이 예외는 KSD 여섯
path에만 적용하며 `chk-holiday`, 일봉, 현재가 조회에는 전파하지 않는다.

전체 Raw/Curated manifest recovery는 range 시작에 한 번 수행한다. 신규 날짜마다
전체 manifest를 다시 훑지 않으며, range가 끝날 때 cumulative Curated generation을
한 번 생성한다. 마지막 날짜의 canonical event는 이 최종 Curated 단계까지 성공한
뒤에만 출력하므로, 최종 단계 실패도 마지막 날짜가 pending인 재실행으로 반드시
수렴한다. 중간 실패 뒤의 재실행은 시작 recovery가 앞서 완료된 날짜의 cumulative
상태를 복구한다.

각 날짜의 승인 조건은 다음 네 단계가 모두 확인되는 것이다.

1. Raw manifest의 KIS request/response shape, endpoint/query lineage, 30 wire 응답,
   file hash/size와 target date를 검증한다.
2. `kis-normalized/kr`의 canonical four-file 문서(bars, instruments, calendar,
   corporate actions)를 확인한다. wire `kis` batch를 publication sink에 직접
   넣지 않는다.
3. DB의 canonical `data_batches`/calendar/instrument/publication row가 하나의
   normalized batch ID와 동일 lineage를 가리키는지 확인한다. DB logical mapping은
   현재 호환 계약상 `KRX/KR`이며, KIS provenance는 Raw/lineage/fetch mode에 남는다.
4. Raw/normalized manifest와 DB row count/hash를 별도 승인 기록에 남긴 뒤,
   immutable curated generation을 만들고 그 generation의 manifest SHA-256을
   pin한다.

## 3. 후보 데이터 백필 (현재는 계획만)

다음 명령은 현재 명시적으로 차단된다. ETF EOD 성공을 후보 source 권리나
후보 dataset 승인으로 해석하지 않기 위한 안전장치다.

```bash
scripts/ops/backfill-production.sh \
  --start 2020-01-31 --end 2026-08-17 --universe all --plan
# bridge가 준비되기 전 --execute는 BLOCKED_EXTERNAL이어야 한다.
```

bridge가 준비되면 별도 실행 계획에서 다음 source를 각각 seal한다.

- KOSPI200 membership snapshot과 KOSDAQ150 membership snapshot
- investor flows, fundamentals, market status, sector classification
- 각 source의 available/effective/cutoff time과 KIS request lineage
- source별 entitlement/use 범위와 canonical dataset version

한 source가 부족하면 전체 후보 dataset을 `READY`로 표시하지 않는다. 두 universe
중 하나만 성공한 경우에도 성공한 universe의 pin과 실패한 universe의 blocker를
분리 기록한다.

## 4. dataset 승인·동일 pin attestation

운영자가 승인하는 pin은 dataset ID, immutable version ID, curated generation,
manifest SHA-256, upstream normalized batch lineage의 다섯 값이다. 값을
환경변수에 복사해 서로 다르게 관리하지 말고, 한 개의 승인된 pin 파일을
Compose API/recommendation/backtest/Paper 설정에 주입한다.

승인 전 체크리스트:

- Raw 및 normalized manifest가 immutable이고 hash 재계산이 일치한다.
- 거래일 달력·종목 마스터·기업행사 문서가 같은 normalized lineage다.
- 0-session holiday가 데이터 손상으로 오판되지 않는다.
- 추천, 백테스트, Paper가 모두 동일한 dataset version/curated generation/
  manifest SHA-256을 로그·health/readiness에서 보고한다.
- dataset pin을 바꿀 때 기존 추천·백테스트·Paper artifact의 lineage는 변하지
  않고 새 generation만 새 pin을 사용한다.
- approval evidence에는 실제 manifest hash와 operator/time만 기록하고 secret,
  API token, 계좌번호는 기록하지 않는다.

### 4.1 기업행사 Parquet schema rollout

기업행사 Curated Parquet은 현재 `CORPORATE_ACTIONS_SCHEMA_VERSION=2`다. v2는
`announced_at`을 nullable로 바꾸고, 원천 관측이 실제로 사용 가능해진 시각을
non-null `available_at`으로 별도 저장한다. 이 버전은
`lagrange.corporate_actions.schema_version=2` Parquet schema metadata와
`QualityGate`의 exact-schema 검사로 검증된다.

기존 `version={v}` 디렉터리와 그 안의 Parquet은 절대 덮어쓰거나 in-place
마이그레이션하지 않는다. v2 기업행사를 포함한 산출물은 curation이 할당한
새 `dataset version`으로 생성하고, 새 per-version manifest의 SHA-256을
재계산해 `RECOMMENDATION_DATASET_MANIFEST_SHA256`를 새 값으로 pin한다. 기존
manifest/pin을 유지한 채 파일만 교체하거나 schema metadata를 삭제하면
quality gate가 `SCHEMA_MISMATCH`/`BLOCKED`로 거부해야 한다.

구버전 파일은 직접 `read_corporate_actions`할 때만 legacy 호환으로 읽을 수
있다. `available_at`이 없는 legacy row에서는 당시 유일한 시각인
`announced_at`을 읽기 호환 fallback으로 사용할 뿐이며, 이것은 v2 dataset
승인이나 새 KIS 관측의 announcement를 의미하지 않는다. 새 KIS KSD 응답은
공시 시각을 제공하지 않으므로 `announced_at`을 추정하지 않고
`available_at=verified retrieval time`만 기록한다.

현재 KSD 기업행사 매핑에서 자동 지원하는 것은 `bonus-issue`의 확정배정율과
권리락일을 split factor로 변환하는 경우뿐이다. 유상증자 권리청약, 배당,
합병/분할, 액면병합, 감자 응답은 필드를 검증한 뒤 typed blocker로 중단한다.
이 blocker를 우회하거나 빈 값으로 Curated에 기록해 dataset을 승인하지 않는다.

## 5. Compose 기동과 사후 확인

기동은 계획/검증을 먼저 수행한다.

```bash
sudo env LAGRANGE_CODE_COMMIT="$LAGRANGE_CODE_COMMIT" \
  scripts/ops/compose-release.sh --scope backfill --plan
sudo env LAGRANGE_CODE_COMMIT="$LAGRANGE_CODE_COMMIT" \
  scripts/ops/compose-release.sh --scope backfill --preflight
```

먼저 backfill scope를 적용한다. 이 단계는 Auth0/TLS와 아직 생성되지 않은
recommendation five-pin을 요구하지 않으며 다음 순서를 보장한다.

`postgres` → `db-role-bootstrap` → `db-migrate` → `research-raw-init` →
`research-schema-check`, 그리고 `research-worker` image build만 수행한다.
앞선 infrastructure scope에서는 이 순서의 one-shot만 수행하며 KIS 자격증명을
읽지 않는다. backfill scope에서는 research-worker daemon을 시작하지 않는다.
One-shot 실패는
후속 단계와 백필 실행을 막고, 완료된 one-shot을 `--rm`으로 제거한다. API/Web/
recommendation/candidate/backtest/Paper/reverse-proxy는 이 단계에서 시작하지
않는다.

```bash
sudo env LAGRANGE_CODE_COMMIT="$LAGRANGE_CODE_COMMIT" \
  scripts/ops/compose-release.sh --scope backfill --apply
```

bootstrap이 성공한 뒤에만 검증된 세션 날짜 목록으로 백필을 실행한다. daemon을 먼저
기동하면 기본 16:30 스케줄러가 승인된 범위 밖의 날짜를 가져오고 별도 token
cache에서 재발급할 수 있으므로, wrapper는 실행 중 daemon을 fail-closed로
거부하고 검증된 XKRX 세션 날짜만 포함한 하나의
`docker compose run --rm --no-deps research-worker
--backfill-session-dates YYYY-MM-DD[,YYYY-MM-DD...]`만 실행한다. 주말·휴장일은
이 목록에 포함되지 않아 worker/KIS/Docker 호출이 발생하지 않는다.

신규 DB에는 백필 전 EOD 행이 없으므로 backfill bootstrap의
`COMPOSE_BACKFILL_BOOTSTRAP: PASS`는 PostgreSQL/one-shot 인프라 gate만 의미하며,
KIS 데이터가 준비됐거나 worker health가 healthy라는 뜻이 아니다.

```bash
sudo env LAGRANGE_CODE_COMMIT="$LAGRANGE_CODE_COMMIT" \
  scripts/ops/post-backfill-health.sh --scope backfill --check
```

이 검사는 실행 중 worker daemon을 요구하지 않고 동일한 worker image의
`healthcheck` subcommand를 `run --rm --no-deps`로 실행한다. 따라서 백필 날짜
범위 밖의 추가 수집 없이 publication freshness만 확인한다. 검사를 통과한 뒤
Raw/normalized/DB/Curated 증거를 검토하고 immutable
dataset version과 five-pin을 승인한다. 이어서 Auth0/TLS와 serving runtime
secret을 provision하고 full release validator/Compose를 실행한다.

백필 상태와 Raw/Curated 산출물을 한 번에 검토할 때는 별도의 로컬 보고서를
사용한다. 이 보고서는 승인이나 DB 쓰기를 수행하지 않고, V4 상태의 검증된 XKRX
session 날짜가
`PUBLISHED`인지, `kis`/`kis-normalized` Raw manifest가 요청 범위를 덮는지,
Curated manifest가 선언한 파일의 크기·SHA-256·Parquet magic이 맞는지만 확인한다.
DB `READY`, entitlement 승인, 다섯 pin은 의도적으로 `NOT_CHECKED`로 남긴다.

```sh
scripts/ops/backfill-review-report.sh --start 2020-01-31 --end 2026-08-18 \
  --state-file /var/lib/lagrange/state/backfill/etf-2020-01-31-to-2026-08-18-<commit>.tsv \
  --data-root /var/lib/lagrange/data --check
```

기본 progress state는 worker-writable data tree와 분리된
`/var/lib/lagrange/state/backfill/` 아래 root-owned 0700/0600 계약으로 생성된다.
기존 `/var/lib/lagrange/data/backfill/` state는 자동 이동·삭제하지 않으며,
`LAGRANGE_BACKFILL_STATE`로 강제해도 worker-owned 조상 경로 때문에 fail-closed될
수 있다.
`CURATED_CANDIDATE_FOUND_UNAPPROVED`가 출력되어도 출시 승인이 아니다. 이후
operator-attestation 런북의 `register-dataset-version.sh --plan/--check/--apply`
절차로 정확한 artifact set, DB lineage, ACTIVE entitlement를 다시 확인하고,
성공한 apply의 다섯 pin만 release 환경에 입력한다. 상태/Raw coverage가 부족하거나
Curated artifact가 불일치하면 보고서는 non-zero로 중단한다.

```bash
sudo env LAGRANGE_CODE_COMMIT="$LAGRANGE_CODE_COMMIT" \
  scripts/ops/validate-production-config.sh \
  --scope release --env-file deploy/compose/.env
sudo env LAGRANGE_CODE_COMMIT="$LAGRANGE_CODE_COMMIT" \
  scripts/ops/compose-release.sh --scope release --apply
docker compose --env-file deploy/compose/.env \
  -f deploy/compose/compose.yml ps
docker compose --env-file deploy/compose/.env \
  -f deploy/compose/compose.yml logs --tail=200 research-worker recommendation-runner
```

full serving 기동 뒤에는 release scope readiness gate를 다시 실행한다.

```bash
sudo env LAGRANGE_CODE_COMMIT="$LAGRANGE_CODE_COMMIT" \
  scripts/ops/post-backfill-health.sh --scope release --check
```

이 gate는 worker daemon의 running 여부를 확인하지 않는다. Compose에 정의된
동일 image/binary의 `research-worker healthcheck` subcommand를
`run --rm --no-deps` one-shot으로 직접 실행한다. 따라서
`data_batches`의 credentialed `KRX/KR` EOD 행과 batch-date-aware
freshness가 실제로 충족되어야 PASS한다. 이 healthcheck의 실제 범위는 DB
round-trip, logical `KRX/KR` EOD publication, fetch mode, 미래 날짜 차단,
freshness이다(`data-pipelines/collectors/src/worker.rs:1388-1429`). Raw/Curated
root·manifest hash·dataset five-pin·migration/schema는 각각
`validate-production-config.sh`, publication/curation, API/runner startup 및
one-shot schema gate에서 검증되며 worker healthcheck가 모두 대리한다고
해석하지 않는다. 재시작은 health 원인을 확인한 뒤에만 수행하며, `down -v`는
disposable QA project 밖에서 실행하지 않는다. `live` profile은 명령에
포함하지 않는다.

## 6. 권리 증거와 release gate

`configs/data-rights/kis.entitlement.json`은 실제 계약/사용허가 문서의 redacted
metadata만 담아야 한다. `document_hash`는 실제 문서 SHA-256이어야 하며, example
파일을 복사해 `ACTIVE`로 바꾸거나 zero hash를 채워 gate를 통과시키면 안 된다.
KIS API portal의 provider access는 entitlement evidence의 대체물이 아니다.

```bash
bash scripts/ops/self-test.sh
bash scripts/qa/phase1-gate.sh
bash scripts/qa/phase2-gate.sh
bash scripts/qa/phase3-gate.sh
```

실제 권리·운영 secret·dataset이 없으면 gate는 `BLOCKED_EXTERNAL_*`를 내고,
Member-facing KR-derived surface와 Live order는 계속 닫혀 있어야 한다. 이
런북은 외부 증거를 생성하거나 위조하지 않는다.
