# KIS KSD action-range Raw collector

이 runbook은 기존 일일 KIS agent, systemd unit, KIND capture, 금융위
provider, `STATUS.md`, Stage4B approved registry와 분리된 action-range 전용
운영 경계다. 이 collector는 실제 provider 호출을 이 문서 검증이나
`--plan`/`--check`에서 수행하지 않는다.

## 목적과 범위

`data-pipelines/collectors/src/bin/kis-action-range-raw.rs`는 KIS KSD의
기업행사 응답을 byte-for-byte immutable Raw로 수집한다. 기업행사 모드의 허용 surface는
다음 6 endpoint/TR channel뿐이며, paid-in capital endpoint의 `GB1=1`과
`GB1=2`를 별도 logical class로 세어 총 7 class다.

| logical class | endpoint | TR ID | extra query |
| --- | --- | --- | --- |
| paidin-subscription | `/uapi/domestic-stock/v1/ksdinfo/paidin-capin` | `HHKDB669100C0` | `GB1=1` |
| paidin-record | `/uapi/domestic-stock/v1/ksdinfo/paidin-capin` | `HHKDB669100C0` | `GB1=2` |
| bonus-issue | `/uapi/domestic-stock/v1/ksdinfo/bonus-issue` | `HHKDB669101C0` | 없음 |
| dividend | `/uapi/domestic-stock/v1/ksdinfo/dividend` | `HHKDB669102C0` | `GB1=0`, `HIGH_GB=` |
| merger-split | `/uapi/domestic-stock/v1/ksdinfo/merger-split` | `HHKDB669104C0` | 없음 |
| reverse-split | `/uapi/domestic-stock/v1/ksdinfo/rev-split` | `HHKDB669105C0` | `MARKET_GB=0` |
| capital-decrease | `/uapi/domestic-stock/v1/ksdinfo/cap-dcrs` | `HHKDB669106C0` | 없음 |

모든 GET은 공통으로 `CTS=`, `F_DT=YYYYMMDD`, `T_DT=YYYYMMDD`를 보내며,
whole-market 모드는 `SHT_CD=`를 보낸다. ETF11 모드는 fixed universe의 각
short code를 `SHT_CD`에 그대로 넣는다. 요청 순서는 symbol → logical class
이며 병렬화하지 않는다. 기존 KIS token manager와 endpoint/TR별 1 req/sec
rate limiter를 재사용한다. 운영(shared read coordination) 모드는 여기에 더해
모든 읽기 사이 1초 global spacing을 강제하므로, 77 calls는 지연시간을 빼고도 약
77초 이상 걸린다. 주문·계좌·잔고 surface와 DB/systemd/live
Compose profile은 이 collector에 없다.

## 두 scope

- `--scope whole-market`: Stage4B-v0가 요구한 blank `SHT_CD` request shape를
  유지하는 호환 수집이다. 초기 7 calls, class별 최대 10 pages다. 한 class라도
  2 pages 이상이면 complete Raw chain은 보존되지만 Stage4B-v0는 그 batch를
  직접 받지 않는다.
- `--scope etf11` (기본): `069500`, `102110`, `229200`, `143850`, `133690`,
  `195930`, `192090`, `148070`, `114260`, `153130`, `132030` 각각에 대해 같은
  7 classes를 요청한다. 초기 77 calls이며 class별 최대 10 pages다. request
  query와 파일명으로 대상 symbol lineage를 보존하고, 응답에 알려진
  `sht_cd`/short-code field가 있으면 exact symbol을 검사한다. 응답 class가
  symbol을 제공하지 않는 경우에는 검증 불가 상태를 그대로 Raw lineage로
  남기며 symbol을 추측하지 않는다.

Daily EOD bundle과 반복 action 공급(`r2-action-supply-v1`)도 같은 ETF11
symbol별 조회를 쓴다(Owner 승인 2026-11-17). EOD는 대상 날짜 하루(`F_DT = T_DT`)
77 calls, 공급은 요청 범위 77 calls이며 경로·TR ID·query key·header는 같고
`SHT_CD` 값만 다르다. whole-market 조회는 이 CLI의 명시적 `--scope whole-market`
선택으로만 남는다.

ETF11 symbol-scoped batch는 Stage4B-v0의 blank `SHT_CD`/single-page evidence
contract가 직접 받지 않는다. v0를 우회하지 않고 `bridge-v1` 입력을 위해
symbol·class·page 파일명, unchanged query, continuation header, hash와
batch identity를 보존한다.

## pagination과 원자성

첫 request의 `tr_cont`는 blank다. 응답 header가 정확히 `M`일 때만 같은
query(빈 CTS 포함)로 다음 GET을 `tr_cont=N`으로 보낸다. `F`는 따라가지
않는다. KSD 응답은 `output1`을 최대 100행만 돌려주므로, chain의 마지막
page가 `F`이거나 `output1` 배열이 100행 이상이면(marker가 없거나 `E`여도
같다) 그 chain은 truncated이며 `BROKER_RESULT_TRUNCATED`로 Raw visibility 전에
fail closed한다. 100행 미만 page의 blank, `E`, 그 밖의 값은 terminal이다.
whole-market 범위가 한 class에서 100행에 닿으면 실행이 실패하므로 완전한 범위
증거는 ETF11 symbol-scoped 실행으로 얻는다. class별 10 pages를 넘으면 fail
closed하며, 같은 response bytes가 반복되면 fail closed한다. malformed JSON,
nonzero `rt_cd`, undocumented shape, request contract mismatch, 알려진 대상
symbol mismatch도 Raw visibility 전에 실패한다.

한 실행은 7 class의 모든 page chain(ETF11은 11×7 group)을 하나의
`provider=kis` RawStore batch로 커밋한다. `RawStore`의 pre-visible cleanup과
`batch.json`/manifest commit을 사용하므로 어느 symbol/class/page라도
실패하면 complete batch가 보이지 않는다. 따라서 실패 실행은 complete
coverage를 주장하지 않는다. 성공도 과거 데이터의 complete/PIT를 뜻하지
않으며, output은 vendor snapshot이고 `strict_pit=false`다.

## operator command

기본은 local-only plan이다.

```text
cargo run -p collectors --bin kis-action-range-raw -- \
  --start 2020-01-01 --end 2026-08-21 --scope etf11 --plan
cargo run -p collectors --bin kis-action-range-raw -- \
  --start 2020-01-01 --end 2026-08-21 --scope whole-market --check
```

`--check`는 `RESEARCH_RAW_ROOT`,
`RESEARCH_ENTITLEMENT_REFERENCE`, `KIS_APP_KEY_FILE`,
`KIS_APP_SECRET_FILE`의 local path/config만 확인하고 network/Raw write를
하지 않는다. credentialed run은 위 값과 함께 다음 acknowledgement가
정확히 필요하다.

```text
KIS_ACTION_RANGE_CONFIRM=I_UNDERSTAND_READ_ONLY_KIS_ACTION_RANGE_CALLS
```

execute output은 batch id, scope, page/file count와 typed error code만
출력한다. response body, broker prose, app key/secret, token, account 값과
entitlement reference는 stdout/stderr/metadata에 출력하지 않는다.

## downstream boundary

이 collector는 Raw acquisition만 담당한다. Stage4B-v0는 blank `SHT_CD`와
정확히 한 terminal page/class를 요구하므로 ETF11 symbol-scoped 또는
multi-page batch를 직접 입력으로 취급하지 않는다. 후속 bridge-v1이 batch
manifest와 file-level lineage를 검토한 뒤 별도 adapter로 연결해야 한다.
nonempty action class 중 현재 canonical mapping이 있는 것은 bonus issue뿐이며,
나머지는 Raw 수집 후 canonical stage에서 blocker로 남는다.

## 2026-10-22 한 번의 배당 충돌 진단 예외

Owner는 Sep15/153130의 원래 SOURCE_CONFLICT를 조사하기 위해 이번 한 번만
제한된 재조회를 명시적으로 허용했다. 기존 7/77-class capture와 달리
`--dividend-conflict-diagnostic`는 아래 고정 네 조건만 순서대로 조회한다.

| 순서 | SHT_CD | F_DT | T_DT |
| --- | --- | --- | --- |
| 1 | 153130 | 20260915 | 20260915 |
| 2 | 153130 | 20260829 | 20261018 |
| 3 | 빈 값 | 20260915 | 20260915 |
| 4 | 빈 값 | 20260829 | 20261018 |

경로는 dividend/HHKDB669102C0, GB1=0, CTS/HIGH_GB/tr_cont는 빈 값이다.
기존 shared_required token/quota 경로와 30초 timeout을 사용한다. 네 logical
요청 전체에서 기존 retry 포함 최대16 GET, 요청당 최대4 GET를 전송 직전에
강제한다. 순차 실행하며 첫 실패·불확실한 결과에서 전체를 중단한다.
M은 해당 첫 응답을 별도 진단 Raw로 보존한 뒤 불완전 종료하고 따라가지 않는다.
F도 따라가지 않는다. 성공은 완전성이나 원래 두 출처의 우선순위를 입증하지 않는다.

`raw/dividend-conflict-diagnostic-20261022-once`의 배타적 디렉터리 claim을
credential resolution/network보다 먼저 동기화한다. 실패·중단·불확실 상태도
claim을 보존하며 삭제하거나 자동 재실행하지 않는다. 각 검증된 원본은
`provider=kis-dividend-diagnostic-20261022`의 별도 immutable batch로 커밋한다.
기존 provider=kis와 거절된 outcomes는 변경하지 않고 DB/admission/rebind/enable을
호출하지 않는다. 출력은 건수·날짜·해시·정형 오류뿐이다. Raw 본문/credentials는
로그에 출력하지 않는다. 실제 GET 수와 새 파일 크기는 실행 후 증거로 확인한다.

로컬 계획: `scripts/ops/kis-action-range-raw-backfill.sh --dividend-conflict-diagnostic --plan`.
실행에는 깨끗한 exact commit, 보호된 대응 env, 원래 운영 경계 검사와 별도
`KIS_DIVIDEND_DIAGNOSTIC_CONFIRM=I_APPROVE_ONCE_DIVIDEND_4_QUERIES_MAX16_GET_NO_ADMISSION`
확인이 필요하다. 이 문서 변경은 실행·충돌 해소·운영 입력 인수를 뜻하지 않는다.

## Post-release ETF11 per-symbol recollection (Owner approval 2026-11-17)

Owner가 2026-11-17에 승인한 범위는 한 번의 종목별 재수집이다. 기간은 2026-08-19부터
최근 거래일까지, 실제 KIS 조회는 최대 77회 GET(7 class × 고정 ETF11 11종목, 각
chain 1 page 기준)이며, 릴리스를 설치한 뒤 Owner가 root로 실행한다. 이 CLI는 와이어
GET 수를 77회로 막지 않는다. `M` 표시나 제한·전송 오류 재시도가 생기면 더 나갈 수 있으므로
(아래 "확인되지 않은 항목"), 77회는 기대 횟수다. Owner는 2026-11-17에 코드 상한을 더하지 않고
이 조건으로 진행하기로 승인했다. 그래도 `pages`가 0이 아니거나 `file_count`가 77이 아니면
아래 중단 규칙대로 멈추고 보고한다.
2020-01-31~2026-08-19의 증거용 재수집은 승인 범위가 아니다. 승인된 V3 종목별 묶음
(2016-08-29~2026-08-28, 77개 파일, 모두 page 1 `E`)이 그 기간의 완전한 증거이기
때문이다. 이 절차는 Raw 수집만 하며 DB, admission, 공급기 예약, Daily 활성화를
호출하지 않는다.

### 선행 조건

- 설치된 릴리스가 PR46(`8e29240`)과 PR47(`bade376`)을 모두 포함한 커밋이다.
  checkout에서 `git merge-base --is-ancestor bade376 HEAD`가 성공해야 한다.
- 설치된 릴리스는 Raw 스냅숏 descriptor 예산 수정(2026-11-19, 브랜치
  `fix/r5-raw-snapshot-nofile`)도 포함해야 한다. 2026-11-19 운영 측정에서 모든 컨테이너의
  `Max open files`는 soft 1024 / hard 524288(Docker 기본값)이었다. 공급기가 함께 잡는 `kis`
  스냅숏은 D=47·B=48·F=1,441로 descriptor 1,594개, `kis-calendar`는 34개, 합계 1,628개를
  연다. 그래서 `677b0f3` 릴리스의 5단계 `--check`는 `ACTION_SUPPLY_SOURCE:IO`(EMFILE)로 실패한다.
  수정된 스냅숏은 batch 트리를 열기 전에 필요 수(`C + 9 + D + 2B + F`에 예비 256)를 계산하고,
  soft 한도가 hard 한도(최대 1,048,576)보다 낮으면 hard까지 올린다. 그래도 모자라면 batch
  트리를 열지 않고 `PROCESS_DESCRIPTOR_LIMIT`로 거부한다. 열기 중의 EMFILE/ENFILE도 같은 코드다
  (`crates/market-data/src/daily_price_only/source.rs`의 `ensure_process_descriptor_budget`).
  Raw는 거래일마다 파일이 약 100개 늘어나므로 soft 1024는 다시 충분해지지 않는다.
- Production은 CLOSED, ENABLE=false를 유지한다. Daily는 OFF이고 Daily admission은
  아직 하나도 없다. 첫 admission 뒤에는 봉인된 범위와 겹치는 새 action page가
  `SEALED_SOURCE_CHANGED`를 낳는다(`migrations/0060_owner_beta_daily_inputs.up.sql:1001`).
  그래서 순서는 릴리스, 재수집, 공급기 `--check`, Daily 활성화다.
- 재수집을 마치기 전에 KIS daily 타이머를 켜지 않는다. 공급기는 범위가 덮이지 않은
  날 하루치 span이 14일을 넘으면 예약 없이 `ACTION_SUPPLY_GAP_LIMIT`로 멈춘다
  (`crates/market-data/src/daily_price_only/supply.rs`의 `MAX_GAP_DAYS`).
- 설치된 환경 파일의 `RESEARCH_R2_ACTION_SUPPLY_CONFIG_HEX`가 가리키는 설정의
  `release_commit`은 같은 환경 파일의 `LAGRANGE_CODE_COMMIT`과 같아야 한다.
  공급기가 `ACTION_SUPPLY_CONFIG`로 거부한다(`data-pipelines/collectors/src/action_supply.rs`
  `Config::validate`). 설정 계약은 [owner-beta-daily-input.md](owner-beta-daily-input.md)의
  "Bounded recurring civil-day action acquisition"에 있다. `Config::validate`에서
  릴리스에 따라 달라지는 필드는 `release_commit` 하나뿐이다. 나머지 값(owner, policy·base·
  entitlement hash, 활성일, 초기 frontier, raw UID, timeout)은 이전 릴리스 설정을 그대로
  쓴다. 새 운영 env는 설치된 현재 릴리스의 env에서 `scripts/ops/release-env-rebind.py`로
  만든다. 이 도구는 `LAGRANGE_CODE_COMMIT` 줄과 공급기 설정의 `release_commit`만 바꾸고
  나머지 바이트는 그대로 복사한다. 값은 출력하지 않는다. 새 릴리스 commit의 깨끗한
  checkout에서 실행한다.

  ```sh
  prev_env=/opt/lagrange/releases/<PREVIOUS_COMMIT>/deploy/compose/.env
  new_commit=<NEW_40_HEX_COMMIT>
  pending_env=<root 소유 보호 디렉터리 안의 새 파일 경로>
  sudo /usr/bin/python3 -I -S -B scripts/ops/release-env-rebind.py --preflight \
    --previous-env "$prev_env" --commit "$new_commit" --output "$pending_env"
  sudo /usr/bin/python3 -I -S -B scripts/ops/release-env-rebind.py --apply \
    --previous-env "$prev_env" --commit "$new_commit" --output "$pending_env"
  ```

  성공하면 `RELEASE_ENV_REBIND: PASS commit=<NEW_40_HEX_COMMIT> changed_keys=2`가 나오고,
  기존 파일을 덮어쓰지 않고 root 0600 새 파일을 만든다. 이 파일을 설치기의 `--env-source`로
  쓴다. `<PREVIOUS_COMMIT>`은 설치된 `current` 릴리스여야 하고, 아니면
  `ENV_REBIND_PREVIOUS_NOT_CURRENT`로 멈춘다. 이전 env에 공급기 줄이 없으면
  (`ENV_REBIND_SUPPLY_ABSENT`) 공급기는 그동안 꺼져 있던 것이다. 새 설정 값 전체는 별도
  Owner 결정이므로 여기서 멈추고 보고한다. 그 밖의 `RELEASE_ENV_REBIND: FAIL code=...`도
  멈추고 보고한다.
- 같은 KST 날짜에 이전 릴리스의 공급기가 실제로 실행되어 남긴 claim 파일이 없어야 한다.
  그런 claim은 새 코드에서 설정 해시와 예약값(280 대 3,080)이 모두 달라
  `ACTION_SUPPLY_STATE`로 거부된다(`action_supply.rs`의 `DayStore::read`). 이 오류는 `--check`에서도
  난다. 코드는 claim을 지우거나 덮어쓰지 않으며 날짜별 파일 이름이 다르다.
- 실행 위치는 설치된 릴리스 디렉터리가 아니라 설치 commit에 정확히 맞춘 깨끗한 git
  checkout이다. 설치된 릴리스는 `.git`이 없는 archive라서
  (`scripts/ops/deploy-production-release.sh`의 사용 설명) wrapper의 HEAD 검사를 통과할 수 없다.
  wrapper는 checkout의 HEAD가 환경 파일의 `LAGRANGE_CODE_COMMIT`과 같고 추적되지 않는
  파일을 포함해 작업 트리가 깨끗하지 않으면 `BLOCKED_EXTERNAL`로 멈춘다
  (`scripts/ops/kis-action-range-raw-backfill.sh`).

### 재수집 전 달력 공백 조사

2026-11-20 공급기 사전검사는 `ACTION_SUPPLY_SOURCE:CALENDAR_COVERAGE_GAP`에서 멈췄다.
기업행사 재수집은 달력 행을 추가하지 않는다. 먼저 설치된 Raw를 읽을 수 있는 운영 계정에서
검토한 [달력 조사 도구](../../scripts/ops/kis-calendar-coverage.py)를 실행한다.
이 조사는 릴리스 설치·이미지 빌드·KIS 호출·DB 연결 없이 실행할 수 있다.
`ENTITLEMENT_HASH`에는 해당 릴리스 공급기 설정의 `entitlement_reference_sha256`을 사용한다.

```bash
python3 -I -S -B scripts/ops/kis-calendar-coverage.py \
  --data-root /var/lib/lagrange/data --entitlement-sha256 "$ENTITLEMENT_HASH" --through 2026-11-20
```

`--through`는 재현할 KST 기준일이다. 생략하면 공급기의 16:30 기준을 따른다.
`missing_intervals`는 시작일(2026-08-29)부터 기준일까지 **응답 행에서 빠진 달력일**이다.
`response_windows`의 요청일·첫날·끝날·날짜 수를 함께 읽는다. 한 응답이 요청일 이후
여러 날을 덮을 수 있고, 첫날과 끝날 사이에도 결손이 있을 수 있다.
`status=blocked`이면 원인을 먼저 확인한다. 빈 구간 배열만으로 입력을 승인하지 않는다.
실제 공급기 판정은 기존 설치본의 읽기 전용 `--check`로 확인한다.
달력 보충이 필요하면 날짜·호출 상한과 사용할 수집 경로를 Owner가 결정한 뒤 진행한다.

### D-06 단일 달력 보충 후보

2026-11-23 운영 원본 집계에서 11월 20일 기준 11월 15~20일, 11월 23일
16:30 KST 전 기준 11월 15~22일이 빠졌다. 검증된 원본 응답 9개는 모두 요청일부터
24일을 연속으로 포함했고, 마지막은 11월 14일까지 덮었다. 보충 후보의 요청일은
2026-11-15다. 24일 응답이면 12월 8일까지 덮을 수 있으므로 실제 응답을 다시 검증한다.

로컬 계획은 `scripts/ops/kis-action-range-raw-backfill.sh --calendar-gap-repair --plan`이다.
실제 실행은 이 경로의 Rust 검사와 후보 이미지 검증이 끝나고 Owner가 11월 15일의
실제 GET 최대 1회를 별도로 승인한 뒤에만 진행한다. 기존 배포를 유지한 채 별도의
정확한 소스 checkout과, 그 revision에 맞춰 설치된 현재 env에서
`release-env-rebind.py`로 만든 root 0600 후보 env를 사용한다. 후보의
`research-action-range-raw` 이미지 한 개를 승인된 빌드 절차로 미리 준비한다.
실행기는 로컬 Docker 소켓과 `lagrange-station` 프로젝트에 고정한다. OCI revision과
commit 환경을 확인한 이미지 ID로 Compose 구성의 실행 계정·Raw·공유 토큰 마운트·전용
네트워크를 대조한다. 사전검사는 같은 이미지에서 Raw를 읽기 전용으로 마운트해
네트워크와 자격 증명 없이 날짜·권리·기존 증거·claim을 확인한다.

같은 운영 Compose 프로젝트에서 연구 daemon과 KIS 수집 타이머가 정지·비활성이며,
다른 일회성 수집이 없음을 확인한다. 이 경로와 잠금을 공유하지 않는 과거 범위
수집기와 수동 호출도 실행 전체에서 정지시킨다. 이 요청 이후에는 같은 날 별도의
휴장일 수집을 실행하지 않는다. 후보 checkout에서 root 운영 계정으로 다음 두 명령을 차례로 실행한다.

```bash
scripts/ops/kis-action-range-raw-backfill.sh --calendar-gap-repair \
  --env-file "$pending_env" --preflight
KIS_R2_CALENDAR_REPAIR_CONFIRM=I_APPROVE_20261115_CALENDAR_ONE_GET_RAW_ONLY \
  scripts/ops/kis-action-range-raw-backfill.sh --calendar-gap-repair \
  --env-file "$pending_env" --execute
```

버전과 권리 해시가 맞는 공급기 설정 및 Raw 소유 UID를 검사한다. 기존 공백을
검증한 뒤 날짜별 claim을 영속화한다. GET은 `chk-holiday / CTCA0903R`에
`BASS_DT=20261115`와 빈 연속 조회 필드만 보낸다. 기존 공유 토큰과 호출 간격을
사용하고 클라이언트와 HTTP 계층의 재시도를 끈다. 첫 응답에 정확한 요청일이
포함되고 현재 실행일 컷오프까지 날짜가 연속돼야 Raw를 커밋한다. 중복 JSON 키,
잘못된 UTF-8, 잘못된 행이나 범위, 없는 날짜를 커밋 전에 거부한다. 격리된 Raw
범위에서 예약한 단일 원본과 연속 달력 증거를 다시 검사한다.

실행 직전에 Raw 밖의 보호된 운영 상태 디렉터리에도 `r2-d06-calendar-20261115.claim`을
영속화한다. Raw 복원에 포함되지 않는 날짜별 claim과 별개로 운영 시도 증거를 보존한다.
실패·중단·불확실한 결과는 두 기록을 보존하고 Owner에게 보고한다. 운영 상태까지
교체하거나 복원했을 때는 기존 시도 증거를 확인할 때까지 실행을 보류한다. 검증은 위 읽기 전용 집계 도구로
반복할 수 있다. 승인된 ETF11 행사 재수집과 공급기 확인은 그다음 단계다.

### Raw 누적량 점검

운영 Raw를 읽을 수 있는 계정에서 다음 명령을 사용한다. 같은 도구의 달력 조사와 독립적으로 실행할 수 있다.

```bash
python3 -I -S -B scripts/ops/kis-calendar-coverage.py \
  --capacity-only --data-root /var/lib/lagrange/data
```

`kis`와 `kis-calendar`를 각각 본다. 일일 공급기는 범위당 manifest 8 MiB·레코드 128개·고유 묶음 128개·묶음당 파일 256개·전체 파일 8,192개·열린 서술자 16,384개까지 허용한다. Paper 복원의 Raw 스캔은 범위당 서술자 5,632개까지 허용한다. 다른 권리의 묶음도 전체 스캔 자원을 차지한다. [공급기 한도](../../data-pipelines/collectors/src/action_supply.rs) · [Paper 복원 한도와 경로](../../crates/job-queue/src/owner_beta/paper/operations/protected/archive.rs)

11월 19일 측정한 `kis`는 날짜 47개, 묶음 48개, 파일 1,441개였다. 이 구성의 예상 서술자는 공급기에서 1,594개, archive에서 1,601개다. archive에는 원본과 잠금 디렉터리의 경로가 더 있다. 기존 가정인 주 5회 EOD·행사 1묶음이면 한 주에 파일 577개와 서술자 약 594개가 늘어난다. 이 조건에서 Paper 복원은 약 21거래일 전후, 공급기 파일 수는 약 55~60거래일 전후에 한도에 닿는다. 예정된 77파일 재수집은 추가 묶음 1개로 반영한다. 실제 페이지 수, 일별 묶음 수, 바이트와 실행 시간에 따라 더 일찍 멈출 수 있다.

2026-11-24 capacity-only 보정은 Paper Raw snapshot 한도를 4,096에서 5,632로 올렸다. 공급기 한도와 Raw 보존 계약은 그대로 두고, archive aggregate 131,072도 유지한다. archive의 검증식 `11 × Raw snapshot cap + 65,536 members + 280 = 127,768`은 aggregate보다 3,304 낮다. `source.rs`는 각 snapshot의 `retained + 256`을 현재 프로세스 `RLIMIT_NOFILE`과 비교하고, soft를 실제 hard 한도까지만 올린다. 2026-11-19 운영 측정의 컨테이너 hard 한도는 524,288이었고, 이번 로컬 측정도 soft/hard 524,288/524,288이었다. 다른 호스트·릴리스의 hard 한도는 별도로 확인한다.

오프라인 manifest projection은 현재 기준 `D=47, B=49, F=1,518`에서 출발해, 23개 복구일에 날짜당 batch 1개·100 files를 넣으면 4,049 descriptors, 그 뒤 100-file 다음 batch를 넣으면 4,152를 산출했다. 이전 4,096 cap은 복구+다음 batch를 거부하고 새 5,632 cap은 받아들인다. 회복 shape 이후 여섯 번의 비교 가능한 EOD/supplier 실행을 각각 약 182 descriptors로 보는 전망은 `4,049 + 6 × 182 = 5,141`, 새 cap까지 491의 여유다. 이는 미래 증가량 가정이지 운영 측정이나 복원 성공 보증이 아니다. 실제 날짜별 batch 수, page 수, Raw 크기, 실행시간과 현재 OS hard limit을 매 회차 다시 확인하고, projection이 5,632에 닿기 전에 멈춰 재평가한다. 2026-11-24 Owner가 승인한 과거 23개 세션의 원본 조회 범위와 10/22 선행 검증은 [복구 절차](owner-beta-daily-input.md#historical-eod-recovery-with-existing-calendar-originals)를 따른다.

두 manifest를 함께 잠가 읽은 시점의 집계다. 마지막 줄을 해석할 수 없으면 `INDETERMINATE_MANIFEST_TAIL`, 같은 batch ID의 메타데이터가 서로 다르면 `INDETERMINATE_DUPLICATE_METADATA`로 집계를 중단한다. 후자는 같은 시각의 표기 차이처럼 공급기가 허용하는 경우도 포함한다. 이때는 기존 공급기의 읽기 전용 `--check`로 원본 판정을 확인한다.

타이머를 재개하기 전에 최신 집계와 다음 수집분을 대조한다. 남은 용량이 다음 묶음을 수용하지 못하면 공급기·Daily·격리 복원에서 같은 규모를 시험한다. 운영 한도 변경에는 바이트·메모리·서술자·소요 시간 실측과 기존 이력의 재검증이 필요하다. 본문·권리와 복원 성공 여부는 각각 해당 검사로 확인한다. [전체 원본 스캔 계약](../reviews/2026-10-14-r2-raw-snapshot-implementation-contract.md)

### 종료일 `<END>` 고르기

공급기는 KST 날짜 `D`의 16:30 이후 실행에서 2026-08-29부터 `D` 이하의 마지막 거래일 `T`까지
모든 달력일이 완전한 종목별 chain으로 덮여 있어야 `ALREADY_COVERED`를 낸다. 하루
단위로 검사하며 한 chain이 범위 전체를 덮어도 된다
(`action_supply.rs` `source_plan`, `crates/market-data/src/daily_price_only/reconstruct.rs`의
칸 선택). 따라서 `<END>`는 `T` 이상이어야 한다.

- 거래일 `D`의 16:30 KST 이후에 재수집과 확인을 한 번에 하면 `<END>`는 `D`다.
  공급기의 자체 마감 시각과 같은 시점에 받은 자료여서 같은 날 EOD 묶음과 칸 비교가
  어긋날 가능성이 가장 낮다. 어긋나면 `SOURCE_CONFLICT`다.
- `<END>`가 `T`보다 1~14일 앞서면 `--check`는 남은 구간을 `start`/`end`로 담아
  `WOULD_CAPTURE_NO_RESERVATION`을 낸다. 그 span이 14일을 넘으면
  `ACTION_SUPPLY_GAP_LIMIT`다. `WOULD_CAPTURE_NO_RESERVATION`은 재수집이 부족하다는
  뜻이므로 이 절차의 성공이 아니다.
- `D` 이후 날짜는 쓰지 않는다. 코드는 거부하지 않지만 필요하지 않다.

### 절차

모든 명령은 위 checkout 안에서 실행한다. 변수는 이후 단계가 공유한다.

```sh
commit=$(git rev-parse HEAD)
release_root=/opt/lagrange/releases/$commit
env_file=$release_root/deploy/compose/.env
end=<END>   # YYYY-MM-DD, 위 규칙
```

1. 계획(로컬 전용, root 불필요). Docker, secret, 운영 env, 네트워크를 쓰지 않는다.

   ```sh
   scripts/ops/kis-action-range-raw-backfill.sh \
     --start 2026-08-19 --end "$end" --scope etf11 --plan
   ```

   wrapper는 텍스트를 출력한다. `range=2026-08-19..<END> scope=etf11`,
   `initial_requests=77 max_pages_per_class=10`, 마지막 줄
   `PLAN_ONLY: no Docker, secret, production-env, data-root write, or network action made`가
   나와야 한다.

2. 사전검사(root). 설치된 환경 파일이 `root:root` mode 0600인지, 환경 파일의
   `LAGRANGE_CODE_COMMIT`이 checkout HEAD와 같은지, 작업 트리가 깨끗한지, range-raw
   운영 설정과 Compose 확장이 맞는지 확인한다. 빌드, KIS 호출, 컨테이너 생명주기는
   없다.

   ```sh
   sudo scripts/ops/kis-action-range-raw-backfill.sh \
     --start 2026-08-19 --end "$end" --scope etf11 \
     --env-file "$env_file" --preflight
   ```

   기대 출력은 `KIS_ACTION_RANGE_RAW_PREFLIGHT: PASS (no build, KIS call, or container lifecycle)`다.
   다른 결과면 중단하고 보고한다. 이 단계를 실행 전에 하는 이유는 다음 단계가
   research-worker를 먼저 멈추기 때문이다.

3. 실행(root). 일반 `research-worker` 데몬이 돌고 있으면 실행 wrapper가 거부하므로
   worker 보호 wrapper를 쓴다. 이 wrapper는 정확한 research-worker 컨테이너를
   멈추고(`docker stop --time 300`), 같은 wrapper를 `--scope etf11`로 실행한 뒤 같은
   컨테이너를 다시 시작한다. 확인 환경값
   `KIS_ACTION_RANGE_CONFIRM=I_UNDERSTAND_READ_ONLY_KIS_ACTION_RANGE_CALLS`는 이
   wrapper가 직접 설정한다. 터미널이 끊겨도 이어지도록
   [kis-range-raw-stage5.md](kis-range-raw-stage5.md)와 같은 transient unit으로 실행한다.
   실행 wrapper는 전용 action 이미지를 `compose build`로 만든다. 이 이미지는 릴리스 이미지
   manifest 밖이다(`scripts/ops/lib/release-image-manifest.sh`에 없다). 미확인: research-worker와
   같은 Dockerfile·빌드 인자를 쓰므로 릴리스 빌드의 layer가 호스트에 남아 있으면 재사용되겠지만,
   남아 있는지는 확인하지 않았다. 없으면 collectors Rust 전체 빌드이며, 이 경우 AGENTS.md의
   production build resource policy를 따른다(한 번에 한 서비스, 낮은 CPU/I/O 우선순위,
   시작 전 메모리·swap·OOM 확인). wrapper는 `compose build --pull=false`로 서비스 하나만
   빌드하고, `data-pipelines/collectors/Dockerfile`이 `CARGO_BUILD_JOBS=2`를 고정한다.

   ```sh
   sudo systemd-run --unit="lagrange-etf11-ksd-recollect-${commit:0:8}" \
     --collect --property=Type=exec --property=KillMode=control-group \
     --property=Nice=10 --property=IOSchedulingClass=idle \
     --property=CPUWeight=20 --property=IOWeight=20 \
     "$PWD/scripts/ops/kis-action-range-raw-with-worker-pause.sh" \
     --start 2026-08-19 --end "$end" --commit "$commit" --env-file "$env_file"
   ```

   진행은 `systemctl status`와 `journalctl -u UNIT`로 본다. 두 명령 모두 credential이나
   응답 본문을 출력하지 않는다. unit이 끝날 때까지 checkout을 수정하지 않는다.

4. 결과 확인. 성공 출력에는 다음이 모두 있어야 한다. 모든 줄이 개수와 상태뿐이다.

   - 컨테이너가 낸 JSON 한 줄: `"status":"success"`, `"provider":"kis"`,
     `"scope":"etf11"`, `"start":"2026-08-19"`, `"end":"<END>"`, `"initial_calls":77`,
     `"pages":0`, `"file_count":77`, `"raw_capture_complete":true`,
     `"coverage_claim":"not_asserted"`, `"strict_pit":false`, `"batch_id"`.
     `pages`는 `file_count`에서 초기 호출 77을 뺀 값이다.
   - wrapper 줄: `KIS_ACTION_RANGE_RAW: PASS (Raw-only action capture completed; no publication/catalog action)`,
     그리고 worker가 돌고 있었다면 `KIS_ACTION_RANGE_RAW_WORKER_PAUSE: PASS`와
     `KIS_ACTION_RANGE_RAW_WORKER_RESTORE: PASS`, 마지막으로
     `KIS_ACTION_RANGE_RAW_WITH_WORKER_PAUSE: PASS`.
   - 요청은 7 class × 11종목을 순서대로 보내고 모든 읽기 사이에 1초 간격이 있으므로
     지연시간을 빼도 77초 이상 걸린다.

5. 공급기 읽기 전용 확인. 2026-08-29부터 `T`까지의 칸을 검사하므로 KST 16:30 이후,
   KRX 거래일에 한다. 16:30 이전은 `DEFERRED_TODAY`, 거래일이 아니면 `NO_SESSION`을 내고
   공급기에 닿지 않는다. 이 확인은 provider 호출과 예약이 없다. 설치된 릴리스의 wrapper를 쓴다.

   ```sh
   sudo "$release_root/scripts/ops/kis-daily-production.sh" \
     --raw-source --action-supply --check
   ```

   wrapper가 `KIS_DAILY_RAW_CHECK: date=<D> source_present=... publication=false ready=false`를
   낸 뒤 worker가 JSON 한 줄을 낸다. 기대 결과는 `"outcome":"ALREADY_COVERED"`다.
   `DISABLED`, `BEFORE_ACTIVATION`, `BEFORE_CUTOFF`는 커버리지 증거가 아니다. 설정이
   꺼져 있거나 활성 날짜 전이거나 시각이 이르다는 뜻이다. check 모드는 그날 EOD Raw가
   없어도(`source_present=no`) 공급기를 실행한다. 환경 파일에 설정 줄이 없으면 wrapper가
   `BLOCKED_EXTERNAL: explicit action-supply configuration is missing or oversized`로
   멈춘다. 위 세 결과와 이 `BLOCKED_EXTERNAL`은 모두 중단하고 보고한다.

6. Daily 활성화와 공급기·KIS daily 타이머의 시작은 이 절차의 일부가 아니다. 별도 Owner 결정이다.

### 중단 규칙

위에 적힌 기대 결과 외의 모든 결과는 중단하고 보고한다. 승인을 넘는 재시도는 하지 않는다.
특히 다음은 모두 중단 사유다.

- 실행이 `"status":"failed"`와 `error_code`를 낸 경우(`BROKER_RESULT_TRUNCATED`,
  `BROKER_PAGINATION_*` 등). 완전한 batch는 보이지 않고 실제 GET은 이미 나갔을 수 있다.
  `network_calls`가 `possibly_attempted`다.
- `pages`가 0이 아니거나 `file_count`가 77이 아닌 경우. 이는 `M` 표시를 따라 승인된 77회를 넘는 요청이
  나갔다는 뜻이다.
- 확인 결과가 `ACTION_SUPPLY_STATE`, `ACTION_SUPPLY_CONFIG`, `ACTION_SUPPLY_GAP_LIMIT`,
  `ACTION_SUPPLY_SOURCE:*`(예: 칸 비교의 `SOURCE_CONFLICT`, 비현금 행사의 `NONCASH_ACTION`,
  컨테이너 hard 한도가 스냅숏 descriptor 수보다 작은 `PROCESS_DESCRIPTOR_LIMIT`),
  `ACTION_SUPPLY_CONSUMED_INCOMPLETE`, `WOULD_CAPTURE_NO_RESERVATION`인 경우.

### 확인되지 않은 항목

- 이 CLI는 와이어 GET 수에 상한이 없다. 한 chain은 `M` 표시를 따라 최대 10 page이고, 한
  요청은 제한 대기나 전송 오류에서 재시도될 수 있다(공급기는 요청당 최대 4회 시도로 계산한다).
  성공 JSON의 `file_count`와 `pages`는 재시도를 세지 않는다. 따라서 "최대 77회"는 기대 횟수이며
  코드가 강제하는 한도가 아니다.
- 공급기의 300초 시한(`timeout_seconds` 최대값) 안에 77회 조회와 소스 검사 두 번이 들어가는지는
  운영 첫 실행에서 재야 한다.
- `<END>`를 `D` 이전 시각(16:30 이전)에 수집해도 되는지는 코드로 판정하지 않았다. 위 권고는
  공급기의 마감 시각에 맞춘 보수적 선택이다.
