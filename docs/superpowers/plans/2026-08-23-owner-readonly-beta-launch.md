# Owner-only Read-only Beta Launch Execution Plan

> 확정일: 2026-08-23<br>
> 상태: **Owner 승인 완료 / 2026-08-24 구현 착수**<br>
> 코드 기준선: `main@1150700`<br>
> 준비도 감사 기준선: `analysis/launch-readiness-audit@6ae6c50`

## 1. 목표

가장 짧은 안전 경로로 고정 11개 ETF에 대한 **소유자 전용 읽기 전용 베타**를 출시한다.
첫 출시는 기존 Stage5 공급자 스냅샷으로 추천과 백테스트를 제공하고, 연속 무인 운영을
검증한 뒤 Paper 기능을 두 번째 단계로 연다.

출시 성공은 일반 공개, 회원 제공, 실거래 준비 완료를 뜻하지 않는다. 이 계획에서 성공은
다음 문장으로만 정의한다.

> 고정 ETF11, 기존 공급자 스냅샷, 비엄격 PIT, 가격수익률 기준의 소유자 전용 베타가
> 재현 가능한 데이터 핀과 운영 증거 아래에서 추천·백테스트를 제공한다.

## 2. 확정된 출시 계약

| 항목 | 확정값 |
| --- | --- |
| 사용자 | 소유자 1명만 허용 |
| 기능 | 1차 추천·백테스트, 2차 Paper |
| 종목 범위 | 저장소에 고정된 ETF11만 허용 |
| 역사 데이터 | 기존 Stage5 스냅샷, 2020-01-31~2026-08-19 |
| PIT 주장 | `strict_pit=false`; 엄격 PIT 또는 당시 가용성 보장 주장 금지 |
| 수익률 능력 | `PRICE_RETURN_ONLY`; 배당 포함 총수익률 주장 금지 |
| 기업행위 | 무상증자만 자동 처리; 그 밖의 비어 있지 않은 유형은 fail-closed |
| READY 의미 | `owner-only vendor-snapshot beta fit` 범위에만 한정 |
| 안정성 문턱 | 연속 3개 거래일 무인 수집·게시 성공 |
| 이번 출시 제외 | 10년 SC-02, 수수료 golden 변경, Phase 4, 회원 KR, 실거래 |

기존 `ready` 필드가 위 범위를 함께 표현하지 못하면 전역 `ready=true`로 의미를 바꾸지
않는다. 범위가 명시된 별도 출시 판정 산출물을 추가하고 전역 READY는 거짓으로 유지한다.

### 2.1 소유자 부재 중 실행 위임 (2026-08-24)

소유자는 2026-08-24부터 다음 확인 시점까지 이 계획의 확정 범위 안에서 권장안대로
중단 없이 작업을 진행하도록 위임했다. 이 위임은 단순한 계획 작성 승인을 넘어 아래 작업의
착수와 완료를 허용한다.

- 저장소 내부 구현, 테스트, 문서 갱신과 단계별 커밋
- fixture/offline 검증, 코드 리뷰, 증거 보고서와 배포 리허설
- 각 선행 게이트가 모두 통과한 경우에 한한 기존 승인 read-only 운영 경로의
  plan/check/apply와 owner-only 베타 cutover
- 사전에 확정된 판정 기준에 정확히 맞는 경우의 manifest/5-pin 등록 절차 진행
- 사소한 구현 선택은 가장 좁고 되돌리기 쉬운 fail-closed 안으로 결정

소유자 응답을 기다리기 위해 병행 가능한 작업을 멈추지 않는다. 한 작업이 외부 환경,
거래일 경과 또는 증거 부족으로 막히면 그 상태를 기록하고 다른 안전한 작업을 계속한다.
단, 아래 항목은 이번 위임으로도 허용되지 않는다.

- 계좌·잔고·주문·체결·정정·취소 API, 주문 WebSocket 또는 Compose `live` 프로필
- Member KR, 외부 공개, 재배포 또는 소유자 이외 사용자 활성화
- 새 역사 KIS 수집, 새 데이터 소스, OpenDART/FSC 재개, KIND 대량·예약·역사 수집
- 엄격 PIT·총수익률·일반 READY 주장 또는 승인 범위를 넓히는 데이터 보정
- 테스트 실패, hash/count 불일치, 미지원 기업행위를 무시한 승격

manifest 생성기가 자기 출력을 자동 승인하는 것은 계속 금지한다. 다만 소유자는 이 문서의
정확한 ETF11·기간·hash·능력·PIT 기준을 모두 통과하고, 생성 경로와 분리된 검증 결과가
기록된 후보에 대해서는 운영자가 등록 절차를 계속 진행하도록 사전 승인했다. 조건 중 하나가
불명확하면 등록하지 않고 다른 작업을 진행한다.

프로덕션 반영 전에는 실행 revision, 이미지 digest, 데이터 5-pin, 권리 범위, rollback
대상을 기록한다. 반영 후에는 소유자가 다음 확인 시 한 번에 검토할 수 있도록 변경 커밋,
테스트 결과, 운영 적용 여부, 실패·보류 항목을 하나의 인계 보고서로 남긴다.

## 3. 변경할 수 없는 안전 경계

- KIS는 현재 허용된 읽기 전용 시세·기업행위 경로만 사용한다. 계좌, 잔고, 주문, 체결,
  정정·취소, 주문 WebSocket 및 Compose `live` 프로필은 사용하지 않는다.
- 새 역사 KIS 수집은 이 계획의 승인에 포함되지 않는다. 기존 Stage5 불변 스냅샷만
  사용하며, 일일 증분 수집은 기존 승인의 범위에서만 운영한다.
- OpenDART/FSC 경로를 다시 열지 않는다. FSC 추가 실호출·Raw 수집은 새 소유자 승인
  없이는 금지한다.
- KIND는 기존의 저빈도 수동 1일 브라우저 캡처 경계만 유지한다. 예약 실행, 대량 수집,
  전 기간 백필은 하지 않는다.
- 비밀, 토큰, 계정 식별자, 응답 본문, 자유 형식 공급자 메시지를 로그·진단·Git에 남기지
  않는다.
- 권리 범위는 개인 소유자 사용으로 제한한다. Member KR 또는 재배포를 암시하는 UI와
  API를 노출하지 않는다.
- 데이터가 불완전하거나 계약이 증명되지 않으면 일정을 맞추기 위해 날짜, 상장 이력,
  기업행위 또는 가용 시점을 만들어내지 않고 출시를 중단한다.

## 4. 핵심 제약과 일정 위험

Stage5 스냅샷이 존재한다고 해서 즉시 서비스할 수 있는 것은 아니다.
현재 Stage4B-0은 로컬 메모리상의 증거 게이트이며 Raw/Curated를 쓰지 않고,
`PublicationBundle`, worker, PostgreSQL, 추천, 백테스트 또는 Paper에 연결되어 있지 않다.

따라서 출시의 핵심 작업은 **기존 Stage5 스냅샷을 한정된 가격수익률 베타 데이터셋으로
재현 가능하게 승격하는 별도 브리지**다. 이 브리지는 기존 Stage4A/4B 증거의 의미를
변경하거나 빈 승인 레지스트리를 자체 승인하는 지름길이 되어서는 안 된다.

이 계약을 허위 주장 없이 구현할 수 없다는 결론이 나오면 작업을 멈추고 범위를 다시
결정한다. 이 조건이 전체 일정의 가장 큰 변수다.

## 5. 실행 순서

```text
0. 감사·계획 기준선 확정
   ├─ 1. 일일 타이머 실패 진단·릴리스 정렬
   └─ 2. 역사 가격 전용 베타 브리지 구현
          ↓
3. 증거 패키지 검토·승인·5-pin 등록
          ↓
4. 소유자 전용 추천·백테스트 출시
          ↓
5. 3거래일 무인 soak 및 Paper 개방
          ↓
6. 최종 게이트·출시 판정·운영 인계
```

작업 1과 2는 병렬 진행할 수 있다. 서비스 리허설은 미리 준비할 수 있지만, 승인된
데이터 버전과 5-pin이 없으면 추천·백테스트 결과를 출시하지 않는다.

## 6. 작업 패키지

### 작업 0 — 문서 기준선과 출시 범위 고정

**대상**

- `docs/STATUS.md`
- `docs/reviews/2026-08-23-launch-readiness-analysis.md`
- 이 계획서

**수행**

1. 준비도 감사 커밋과 이 계획을 `main`에 반영한다.
2. 위 2절의 출시 계약과 제외 범위를 `STATUS.md`의 현재 사실로 연결한다.
3. 감사 브랜치가 `main` 이후에 만든 두 문서 변경만 포함하는지 확인한다.
4. 저장소 밖의 공식 XLSX와 기존 사용자 파일은 이동·삭제·커밋하지 않는다.

**수용 기준**

- `main`에서 감사 결과, 승인된 계약, 실행 계획을 한 경로로 찾을 수 있다.
- 문서가 “출시됨”이나 “엄격 PIT”를 주장하지 않는다.
- 미병합 감사 커밋이나 동일 문서의 중복 버전이 남지 않는다.

**권장 커밋**

`docs(launch): approve owner-only beta execution plan`

### 작업 1 — 일일 타이머 실패 원인 제거와 실행물 정렬

**주요 대상**

- `scripts/ops/kis-daily-production.sh`
- `scripts/ops/kis-daily-production-self-test.sh`
- `scripts/ops/install-kis-daily.sh`
- `scripts/ops/build-production-images.sh`
- `scripts/ops/deploy-production-release.sh`
- `scripts/ops/production-ops-static-check.sh`
- 관련 systemd 단위와 운영 런북

**수행**

1. 2026-08-23 16:30 KST 실패 지점의 원인을 재현 가능한 로컬 테스트로 먼저 고정한다.
2. 네 가지 daily-state 실패 분류가 안전한 typed detail을 stderr에 보존하도록 회귀 테스트를
   추가한다. 공급자 본문·메시지·비밀은 출력하지 않는다.
3. 설치된 systemd 단위, 이미지, 배포 manifest와 실행 중 revision이 동일 커밋을 가리키는
   정적 검사와 운영 검사를 추가한다.
4. 모든 프로덕션 이미지에 revision/digest 식별자를 넣고 배포 전후 비교가 가능하게 한다.
5. 로컬 self-test와 정적 검사가 통과한 뒤에만 운영자가 plan/check/apply 절차로 immutable
   release를 설치한다.
6. 누락된 세션이 있으면 기존 읽기 전용 일일 경로로 정확한 날짜만 catch-up한다. 이 수동
   catch-up은 무인 성공 횟수에 포함하지 않는다.

**수용 기준**

- 실패가 같은 위치에서 재현되지 않고, 실패 시 원인은 비밀 없는 typed 상태로 구분된다.
- 배포된 단위·이미지·manifest·Git revision의 불일치가 자동으로 차단된다.
- 새 설치 경로가 계좌·주문 API와 `live` 프로필을 참조하지 않는다.
- 이후 작업 5에서 연속 3개 거래일 무인 성공을 증명할 수 있는 로그와 상태가 남는다.

**권장 커밋**

`fix(ops): align the daily collector with its pinned release`

### 작업 2 — Stage5 역사 스냅샷의 가격 전용 베타 브리지

**주요 대상**

- `crates/market-data/src/range_to_canonical.rs`
- `crates/market-data/src/range_to_canonical_tests.rs`
- `data-pipelines/collectors/src/bin/kis-range-evidence-package.rs`
- `data-pipelines/collectors/src/bin/research-worker.rs`
- `configs/evidence/kis-range-canonical-approved-manifests.json`
- 새 버전 계약/스키마와 해당 런북

**설계 원칙**

- 기존 Stage4A/4B를 일반 게시 경로로 넓히지 않고, 별도 버전
  `kis-historical-price-only-beta-v1` 계약을 둔다.
- 입력은 기존 Stage5 불변 배치의 정확한 hash와 고정 ETF11,
  `2020-01-31..2026-08-19`에만 묶는다.
- 출력에는 `vendor_snapshot=true`, `strict_pit=false`,
  `Capability::PriceReturnOnly`가 반드시 포함된다.
- 상장일, 역사적 장중 스케줄, `available_at`을 추정하거나 과거로 소급하지 않는다.
- 배당 포함 성과물이나 `TOTAL_RETURN_CAPABLE` 판정을 만들지 않는다.
- 무상증자 이외의 비어 있지 않은 기업행위가 나타나면 전체 승격을 중단한다.

**수행**

1. 먼저 입력 hash, 허용 기간, ETF11, 산출물 schema, 실패 조건을 문서와 RED 테스트로
   고정한다.
2. Stage5 배치에서 가격 전용 canonical dataset을 결정적으로 생성하는 브리지를 구현한다.
3. ETF별 정확한 관측 수와 전체 기간 연속성을 검증한다. 현재 스냅샷 기대치는 ETF당
   1,608개 관측이며, 실제 manifest와 다르면 자동 보정하지 않고 중단한다.
4. 동일 입력이 byte-identical manifest와 hash를 만드는지 검증한다.
5. 다음 음성 테스트를 추가한다.
   - 종목·기간·입력 hash 범위 확장
   - 누락/중복 세션 또는 종목
   - 지원하지 않는 기업행위
   - `strict_pit=true` 또는 총수익률 능력 오표기
   - manifest 변조와 승인 전 자체 등록
   - Member 또는 재배포 범위 사용
6. 독립 코드 리뷰를 통과한 뒤에만 운영자 승인 후보 패키지를 만든다.

**수용 기준**

- 기존 Stage5 bytes와 승인된 계약만으로 결과를 재현할 수 있다.
- 가격수익률·비엄격 PIT·공급자 스냅샷 표지가 DB/API/UI까지 손실되지 않는다.
- 미지원 기업행위나 증거 누락은 Raw/Curated/추천에 부분 노출되기 전에 fail-closed 한다.
- 승인 레지스트리 수정과 데이터 생성은 같은 자동 작업이 아니다.

**권장 커밋**

`feat(data): publish the bounded price-only vendor snapshot`

### 작업 3 — 증거 패키지 검토, 승인, 데이터셋 5-pin 등록

**주요 대상**

- `scripts/ops/backfill-review-report.sh`
- `scripts/ops/register-dataset-version.sh`
- `scripts/ops/provision-entitlement.sh`
- `configs/evidence/kis-range-canonical-approved-manifests.json`
- `docs/runbooks/kis-production-backfill.md`

**수행**

1. 작업 2의 패키지를 plan/check 모드로 생성하고 데이터·manifest·보고서 hash를 기록한다.
2. 운영자가 코드가 생성한 hash와 별개로 범위, 11종목, 기간, 능력, PIT 표지를 검토한다.
3. 검토한 manifest SHA만 별도 커밋으로 승인 레지스트리에 추가한다. 생성기가 자기 결과를
   자동 승인하지 못하게 한다.
4. 승인 후 `register-dataset-version.sh`를 plan → check → apply 순서로 실행한다.
5. source version, raw batch, curated generation, strategy version, policy version의 정확한 5-pin을
   기록하고 재조회한다.
6. 개인 사용 entitlement의 reference/hash는 유지하되 본문이나 비밀을 Git에 넣지 않는다.

**수용 기준**

- 동일 입력에서 동일한 증거와 5-pin이 생성된다.
- 등록 전 추천/백테스트가 해당 데이터셋을 선택할 수 없다.
- `PRICE_RETURN_ONLY`, `strict_pit=false`, owner-only 범위가 등록과 조회에서 보존된다.
- 전역 READY를 켜는 경우에는 반드시 이 범위가 판정 산출물에 포함된다. 포함할 수 없으면
  전역 READY는 거짓으로 둔다.

**권장 커밋**

`chore(data): approve the owner beta dataset manifest`

### 작업 4 — 소유자 전용 추천·백테스트 1차 출시

**주요 대상**

- `deploy/compose/compose.yml` 및 release 배포 스크립트
- recommendation/candidate/backtest 서비스와 UI
- `scripts/qa/recommendation-runner-smoke.sh`
- `scripts/qa/full-system-gate.sh`
- `docs/runbooks/production-release-and-backup.md`

**수행**

1. Auth0 접근 정책을 소유자 1명으로 제한하고 Member KR surface가 노출되지 않는지 확인한다.
2. `live` 프로필 없이 reverse proxy, web, API, recommendation, candidate, backtest만 먼저
   commit-pinned immutable release로 올린다. Paper runner는 이 단계에서 비활성화한다.
3. 승인된 5-pin으로 첫 추천과 백테스트를 생성하고 동일 핀으로 재현되는지 확인한다.
4. API와 UI에 다음 표지를 항상 노출한다.
   - 공급자 스냅샷
   - 비엄격 PIT
   - 가격수익률 전용
   - 소유자 전용 베타
5. 백업을 수행하고 별도 검증 위치에서 restore drill을 완료한다.

**수용 기준**

- 소유자 외 인증 주체는 접근할 수 없고 공개 엔드포인트에서 결과가 새지 않는다.
- 추천과 백테스트가 승인된 동일 5-pin을 사용한다.
- UI/API가 총수익률·엄격 PIT·일반 공개 준비 완료를 암시하지 않는다.
- 재시작 및 restore 후에도 핀과 결과 계보가 유지된다.

**권장 커밋**

`ops(beta): open owner-only recommendation and backtest`

### 작업 5 — 연속 3거래일 soak와 Paper 2차 개방

**주요 대상**

- `deploy/runtime/Dockerfile.paper-runner`
- `deploy/systemd/paper-runner.service`
- `scripts/qa/paper-runner-smoke.sh`
- 일일 수집·게시 상태와 운영 런북

**수행**

1. 수동 개입이나 catch-up 없이 연속 3개 거래일의 수집, 검증, Raw commit, Curated,
   publication 성공을 기록한다.
2. 각 세션에서 exact target observation, ETF11 coverage, 기업행위 fail-closed 상태,
   revision alignment와 게시 원자성을 확인한다.
3. 첫 번째 다음 세션을 대상으로 Paper preview → apply → settle을 소유자 1명 범위에서
   검증한다.
4. Paper가 내부 모의 원장만 사용하고 KIS 계좌·잔고·주문·체결 API에 연결되지 않는지
   정적·동적 smoke로 확인한다.
5. 장애 재시작과 백업/복원 후 중복 주문 또는 중복 결제가 생기지 않는지 확인한다.

**수용 기준**

- 세 거래일 모두 예약된 실행이 무인으로 끝나며 수동 성공은 카운트하지 않는다.
- 한 거래일이라도 실패하면 연속 카운트를 0으로 되돌리고 원인 수정 후 다시 시작한다.
- Paper는 승인된 추천과 5-pin만 소비하며 실거래 surface를 갖지 않는다.
- preview/apply/settle의 재실행 안전성과 계보가 증명된다.

**권장 커밋**

`ops(beta): enable paper after the unattended soak`

### 작업 6 — 최종 게이트와 출시 판정

**자동 게이트**

- Rust format, clippy, workspace tests
- Python tests와 research-worker smoke
- web lint, typecheck, unit test, production build, 필요한 E2E
- 정책/정적 검사와 production ops self-test
- `scripts/qa/phase1-gate.sh`
- `scripts/qa/full-system-gate.sh`
- recommendation/paper runner smoke
- 백업·복원 검증

네트워크가 필요한 단계는 fixture 기반 자동 게이트와 분리한다. 운영 실호출은 기존 허용
경계와 운영자 절차 안에서만 수행하며, 테스트가 공급자 실호출을 암묵적으로 만들지 않는다.

**사람 검토**

- F1: 권리/라이선스와 소유자 전용 접근
- F2: 비밀·로그·출력 redaction
- F4: 결과 해석, 비엄격 PIT 및 가격수익률 표지
- 실행 revision, 이미지 digest, 데이터 5-pin, 백업 위치, rollback 명령 확인

**출시 판정**

- 작업 4 완료 시: `OWNER_RECOMMENDATION_BETA_AVAILABLE`
- 작업 5와 모든 게이트 완료 시: `OWNER_PAPER_BETA_AVAILABLE`
- 일반 `READY`, Member KR, 실거래, 엄격 PIT, 총수익률 준비 완료로 해석하지 않는다.

**권장 커밋**

`docs(status): record the owner beta launch evidence`

## 7. 출시 중단 조건

다음 중 하나라도 발생하면 진행 중인 승격 또는 배포를 중단하고 마지막 승인 release로
rollback한다.

- Stage5 입력 hash, ETF11 또는 기간이 승인 계약과 다름
- 11종목 중 하나라도 기대 관측 수·세션 연속성 검증 실패
- 무상증자 이외의 비어 있지 않은 기업행위 발견
- `strict_pit=false` 또는 `PRICE_RETURN_ONLY` 표지가 계층 사이에서 소실됨
- 승인되지 않은 manifest, 데이터 버전 또는 5-pin 사용
- 실행 revision과 설치/이미지/manifest revision 불일치
- 비밀, 계정 식별자, 공급자 본문/자유 형식 메시지 노출
- Member/공개 접근 또는 KIS 계좌·주문 surface 발견
- 게시 원자성, 백업 복원 또는 Paper 재실행 안전성 실패
- 역사 스냅샷을 허위 날짜나 가용성 정보 없이 게시할 수 없다는 설계 결론

## 8. 일정과 완료 예상

| 기간 | 목표 |
| --- | --- |
| Day 0 | 감사·계획 반영, 출시 계약 고정 |
| Day 1~2 | 일일 타이머 수정과 역사 브리지 RED 테스트/구현 |
| Day 3~4 | 증거 검토·5-pin 등록, 서비스 배포 리허설 |
| Day 4~7 | 소유자 추천·백테스트 1차 출시 |
| 이후 3거래일 | 무인 soak, Paper 검증·2차 개방 |
| Day 7~10 수준 | 전체 게이트와 운영 인계 완료 |

현실적인 예상은 **추천·백테스트 베타까지 4~7 영업일**, **Paper 포함 안정 베타까지
7~10 영업일이며 최소 3개 실제 거래 세션**이다. 역사 가격 전용 브리지의 계약 검토에서
새 증거 공백이 나오면 이 일정은 중단되고 재산정한다.

## 9. 이번 출시 이후 백로그

- 10년 SC-02 역사 범위와 엄격 PIT 증거
- 배당 및 기타 기업행위의 자동 정규화와 총수익률 능력
- phase-0 fee golden 변경
- Phase 4
- Member KR와 외부 배포 권리
- 관리자 UI와 이메일 운영 완성
- 별도 명시적 프로젝트로서의 계좌/주문/실거래
- OpenDART/FSC 범위 재검토 또는 KIND 대량/역사 수집

이 항목들은 소유자 베타의 차단 조건이 아니며, 이번 계획 중 편의상 끌어오지 않는다.

## 10. 완료 증거 묶음

최종 STATUS 갱신에는 최소한 다음 식별자를 남긴다. 비밀이나 원문 응답은 포함하지 않는다.

- 코드 commit과 release manifest SHA
- 모든 실행 이미지 digest/revision
- Stage5 입력 batch/hash와 승인 manifest SHA
- source/raw/curated/strategy/policy 5-pin
- 3개 연속 거래일의 무인 실행 ID와 typed terminal state
- recommendation/backtest/Paper smoke 결과
- 자동 게이트 결과와 F1/F2/F4 검토자·시각
- 백업 및 별도 위치 restore drill 식별자
- rollback 대상 release와 실행 확인 결과
