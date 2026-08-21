# KIS KSD action-range Raw collector

이 runbook은 기존 일일 KIS agent, systemd unit, KIND capture, 금융위
provider, `STATUS.md`, Stage4B approved registry와 분리된 action-range 전용
운영 경계다. 이 collector는 실제 provider 호출을 이 문서 검증이나
`--plan`/`--check`에서 수행하지 않는다.

## 목적과 범위

`data-pipelines/collectors/src/bin/kis-action-range-raw.rs`는 KIS KSD의
기업행사 응답을 byte-for-byte immutable Raw로 수집한다. 허용 surface는
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
rate limiter를 재사용한다. 주문·계좌·잔고 surface와 DB/systemd/live
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

ETF11 symbol-scoped batch는 Stage4B-v0의 blank `SHT_CD`/single-page evidence
contract가 직접 받지 않는다. v0를 우회하지 않고 `bridge-v1` 입력을 위해
symbol·class·page 파일명, unchanged query, continuation header, hash와
batch identity를 보존한다.

## pagination과 원자성

첫 request의 `tr_cont`는 blank다. 응답 header가 정확히 `M`일 때만 같은
query(빈 CTS 포함)로 다음 GET을 `tr_cont=N`으로 보낸다. `F`, blank, 그 밖의
값은 terminal이다. class별 10 pages를 넘으면 fail closed하며, 같은 response
bytes가 반복되면 fail closed한다. malformed JSON, nonzero `rt_cd`,
undocumented shape, request contract mismatch, 알려진 대상 symbol mismatch도
Raw visibility 전에 실패한다.

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
