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
chain 1 page 기준)이며, 릴리스를 설치한 뒤 Owner가 root로 실행한다.
2020-01-31~2026-08-19의 증거용 재수집은 승인 범위가 아니다. 승인된 V3 종목별 묶음
(2016-08-29~2026-08-28, 77개 파일, 모두 page 1 `E`)이 그 기간의 완전한 증거이기
때문이다. 이 절차는 Raw 수집만 하며 DB, admission, 공급기 예약, Daily 활성화를
호출하지 않는다.

### 선행 조건

- 설치된 릴리스가 PR46(`8e29240`)과 PR47(`bade376`)을 모두 포함한 커밋이다.
  checkout에서 `git merge-base --is-ancestor bade376 HEAD`가 성공해야 한다.
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
  쓴다. 새 운영 env(`deploy/compose/.env`)를 설치하기 전에 이전 릴리스 env의 값을 다음처럼
  다시 만들어 그 한 줄을 바꾼다. 출력은 비밀이 아닌 설정의 hex 한 줄이다.

  ```sh
  prev_env=/opt/lagrange/releases/<PREVIOUS_COMMIT>/deploy/compose/.env
  new_commit=<NEW_40_HEX_COMMIT>
  sudo python3 -I - "$prev_env" "$new_commit" <<'PY'
  import json, sys
  path, commit = sys.argv[1], sys.argv[2]
  key = 'RESEARCH_R2_ACTION_SUPPLY_CONFIG_HEX='
  lines = [l for l in open(path, encoding='utf-8') if l.startswith(key)]
  if len(lines) != 1:
      sys.exit('SUPPLY_CONFIG_LINES=%d' % len(lines))
  cfg = json.loads(bytes.fromhex(lines[0][len(key):].strip()))
  if cfg.get('schema') != 'r2-action-supply-v1':
      sys.exit('SUPPLY_CONFIG_SCHEMA_MISMATCH')
  cfg['release_commit'] = commit
  print(key + json.dumps(cfg, separators=(',', ':')).encode().hex())
  PY
  ```

  이전 env에 이 줄이 없거나(`SUPPLY_CONFIG_LINES=0`) schema가 다르면 공급기는 그동안
  꺼져 있던 것이다. 새 설정 값 전체는 별도 Owner 결정이므로 여기서 멈추고 보고한다.
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
   시작 전 메모리·swap·OOM 확인).

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
   꺼져 있거나 활성 날짜 전이거나 시각이 이르다는 뜻이다.

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
  `ACTION_SUPPLY_SOURCE:*`(예: 칸 비교의 `SOURCE_CONFLICT`, 비현금 행사의 `NONCASH_ACTION`),
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
