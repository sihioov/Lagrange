# KIS ETF 백필 일일 재개 계약

이 문서는 KIS 읽기 전용 ETF 백필의 일일 재개 동작을 설명한다. 기업행사·계좌·주문
API를 호출하지 않으며, `--auto-resume`는 운영 systemd timer에서만 사용한다.

## 왜 일일 실행인가

`chk-holiday (CTCA0903R)`는 한 번의 응답으로 제한된 날짜 창만 반환한다. 현재
provider는 첫 응답의 Raw bytes를 immutable snapshot으로 보존하고, 창 밖 날짜에서
`KIS_CALENDAR_SNAPSHOT_MISS`를 반환한다. 이때 두 번째 휴장일 호출을 같은 실행에서
시도하지 않는다. 따라서 수년 범위는 같은 append-only state를 다음 일일 실행에서
재사용하여 창을 순차적으로 전진시킨다.

일일 timer는 다음 계약을 사용한다.

- `OnCalendar=*-*-* 03:15:00 Asia/Seoul`, `Persistent=true`
- 같은 immutable release 경로, code commit, civil date range, universe, state 파일
- 범위는 scheduler-only XKRX artifact의 materialized range 안에 있어야 하며, 각
  실행은 검증된 비연속 session list만 worker에 전달한다(주말/휴장일은 호출 없음)
- `After=docker.service network-online.target` 및 `Wants=network-online.target`
- `--auto-resume --execute`와 읽기 전용 guard만 unit에 포함
- `SuccessExitStatus=74 75`; 영구 실패 exit 1은 systemd 실패로 남겨 자동 재개를 막음
- KIS app key/secret, token, 계좌번호, 주문 설정은 unit에 기록하지 않음
- timer가 완료 state를 발견하면 worker/Docker/KIS 호출 없이 즉시 종료

설치 계획과 unit 생성은 다음 스크립트가 담당한다. 기본은 plan이며, 기존 unit을
교체할 때는 명시적으로 `--replace-existing`를 추가해야 한다.

```bash
scripts/ops/install-kis-backfill-timer.sh --dry-run \
  --release-root /opt/lagrange/releases/<40-hex-commit> \
  --code-commit <40-hex-commit> \
  --state-file /var/lib/lagrange/state/backfill/state.tsv \
  --start 2020-01-31 --end 2026-08-18
```

`--apply`는 현재 한국 시간 03:15 이전에만 허용된다. `Persistent=true` timer를
03:15 이후 설치하고 나중에 시작하면 놓친 실행을 즉시 catch-up할 수 있기
때문이다. installer는 timer와 service를 시작하지 않고 timer만 enable한다.
설치 후 unit을 검토하고 다음 03:15 전에 의도적으로 timer를 시작한다.

## 오류별 재개

worker의 오류 JSON은 body-free stable fields만 progress parser에 전달된다.

| 상태 | 자동 재개 | 의미 |
| --- | --- | --- |
| `DEFERRED` + `KIS_CALENDAR_SNAPSHOT_MISS` | 허용 | 다음 daily run이 다음 calendar window를 요청 |
| `RETRYABLE` | 허용 | bounded retryable infrastructure/provider failure |
| `FAILED` | 금지 | 영구 오류 또는 progress protocol 오류; operator review 필요 |

deferred failure에서는 오류 날짜 하나만 state에 추가된다. 아직 시도하지 않은
미래 날짜 전체를 `FAILED`로 기록하지 않는다. 영구 오류의 재시도는 operator가
state와 Raw/Curated evidence를 검토한 뒤 `--auto-resume` 없이 별도로 실행해야 한다.

상태 파일은 worker가 쓰는 Raw/Curated data tree와 분리된
`/var/lib/lagrange/state/backfill/` 아래에 생성된다. 디렉터리는 root:root 0700,
state/lock 파일은 root:root 0600이어야 한다. 과거
`/var/lib/lagrange/data/backfill/` 파일은 자동 이동·삭제하지 않으며, 해당 경로를
계속 쓰려면 `LAGRANGE_BACKFILL_STATE`로 명시해야 한다. 다만 worker-owned
`data` 트리 아래의 기존 경로는 조상 신뢰 경계를 만족하지 않아 정상적으로
거부될 수 있으므로 새 root-owned 경로를 지정한다. 상태 파일은 scheduler-only XKRX
calendar id/hash/range까지 포함하는 V4 identity를 유지하지만 오류 행은 다음처럼
error code를 추가한다.

```text
2026-01-20\tDEFERRED\t<run-identity>\tKIS_CALENDAR_SNAPSHOT_MISS
2026-02-04\tFAILED\t<run-identity>\tUNSUPPORTED_ACTION
```

`PUBLISHED` 이후 다른 상태가 붙은 conflicting state, 범위 밖 날짜, foreign identity,
잘못된 error code는 fail-closed로 거부된다. state는 root:root 0600이어야 한다.

## 검증

로컬 계약 테스트는 다음 명령으로 실행한다. production env, Docker, PostgreSQL,
KIS, secret에는 접근하지 않는다.

```bash
scripts/ops/backfill-resume-self-test.sh
```
