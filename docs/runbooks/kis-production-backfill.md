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
scripts/ops/provision-linux.sh --preflight
```

`--preflight`가 실패하면 `--apply`를 실행하기 전에 경로·계정·권한을 검토한다.
`--apply`는 명시적으로 승인된 root 터미널에서만 실행하며, 계정·디렉터리
생성 외의 삭제·초기화·재귀 복사는 하지 않는다.

```bash
sudo scripts/ops/provision-linux.sh --apply
sudo deploy/secrets/provision-runtime-secrets.sh
```

실제 secret 값은 이 저장소나 명령 인자에 넣지 않는다. 다음 검사는 값 자체를
출력하지 않고 파일 존재·regular/no-symlink·non-empty·소유자·mode와 env/dataset
pin의 shape만 확인한다.

```bash
export LAGRANGE_CODE_COMMIT="$(git rev-parse HEAD)"
scripts/ops/validate-production-config.sh --env-file deploy/compose/.env
```

검사 결과가 `BLOCKED_EXTERNAL`이면 정상적인 대기 상태다. 누락된 KIS key/secret,
TLS/Auth0/DB secret, 운영 DB, entitlement reference, 또는 dataset pin을 전용
운영 절차로 주입한 뒤 다시 실행한다. `INVALID_CONFIG`는 값을 기다릴 문제가
아니라 설정을 수정해야 하는 상태다.

## 2. 고정 ETF 백필

날짜 범위는 KIS provider의 trading-day/holiday 검증에 맡긴다. 주말과 휴일을
임의로 데이터로 채우지 않는다. 계획 명령은 API·Docker·파일 쓰기를 하지 않는다.

```bash
scripts/ops/backfill-production.sh \
  --start 2020-01-01 --end 2026-08-17 --universe etf --plan
```

운영자가 범위와 읽기 전용 호출을 확인한 뒤에만 실행 guard를 설정한다.

```bash
BACKFILL_CONFIRM_EXTERNAL=I_UNDERSTAND_READ_ONLY_KIS_CALLS \
  scripts/ops/backfill-production.sh \
  --start 2020-01-01 --end 2026-08-17 --universe etf --execute
```

실행기는 날짜별로 `research-worker --once --date`를 호출하고, 성공한 날짜만
state file에 `PUBLISHED`로 append한다. 실패한 날짜에서 중지하며, 재실행 시
이미 `PUBLISHED`인 날짜는 건너뛴다. worker 자체의 deterministic normalized
ID와 exact manifest/evidence 비교가 재시도·crash recovery·동시 실행의 기준이다.

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
  --start 2020-01-01 --end 2026-08-17 --universe all --plan
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

## 5. Compose 기동과 사후 확인

기동은 계획/검증을 먼저 수행한다.

```bash
scripts/ops/compose-release.sh --plan
scripts/ops/compose-release.sh --preflight
```

운영자가 외부 blocker를 해소한 뒤 `--apply`를 실행하면 다음 순서를 보장한다.

`postgres` → `db-role-bootstrap` → `db-migrate` → `research-raw-init` →
`research-schema-check` → API/Web/KIS research/recommendation/candidate/backtest/
Paper/reverse-proxy. One-shot 실패는 후속 서비스 기동을 막는다. `report-worker`는
아직 producer/settlement contract가 없어 fake healthy service로 만들지 않는다.

```bash
scripts/ops/compose-release.sh --apply
docker compose --env-file deploy/compose/.env \
  -f deploy/compose/compose.yml ps
docker compose --env-file deploy/compose/.env \
  -f deploy/compose/compose.yml logs --tail=200 research-worker recommendation-runner
```

헬스체크는 DB readiness, migration/schema gate, Raw/Curated root, dataset pin,
worker heartbeat와 latest EOD freshness를 모두 확인해야 한다. 재시작은 health
원인을 확인한 뒤에만 수행하며, `down -v`는 disposable QA project 밖에서 실행하지
않는다. `live` profile은 명령에 포함하지 않는다.

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
