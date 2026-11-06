# R5 WP-2A1 — calendar provenance 소스·집중 검증

검증일: 2026-10-29 (Asia/Seoul). 작업 트리: `/data/worktrees/3puw275b/modern-anaconda`.
기준 HEAD: `a30edacff987d55b5be7cca25bed7faeec589629` + 아래 세 파일의 미커밋 변경.
판정: **SOURCE / NAMED_FOCUSED_VERIFIED**. 전체 R5-02나 G1 완료가 아니다.

## 변경과 직접 검토

기존 EOD 결과는 가격 source 재사용 여부만 전달해 새 EOD 수집이 이미 존재한 calendar claim을
사용했는지 구분할 수 없었다. 이 사실을 보존하도록 다음 세 파일만 변경했다.

- [eod_raw.rs](../../data-pipelines/collectors/src/eod_raw.rs#L22):
  `calendar_claim_preexisting: bool`, `calendar_batch_id: Option<BatchId>` 추가.
  같은 CalendarDayLock에서 ensure/capture **전에** 이미 읽던 `has_claim`을 전달한다.
  반환 calendar의 실제 batch ID를 쓰며 historical 기존 source의 calendar 부재는 None으로 보존한다.
- [worker.rs](../../data-pipelines/collectors/src/worker.rs#L1688): 기존 `raw_committed`에 두 필드를
  UUID/null·boolean으로 전달한다. 기존 상태와 `ready/publication/curated/db=false`는 유지한다.
- [kis_recovery.rs](../../data-pipelines/collectors/tests/kis_recovery.rs#L319): 기존 fixture 세 개 보강,
  calendar-only 선행 claim 뒤 새 EOD 수집 회귀 한 개 추가.

코디네이터가 diff에서 관측 순서·원본 batch identity·두 반환 경로·worker 매핑과 기존
요청 순서/lock/재사용 조건 보존을 검토했다. 이 필드 자체는 무인 실행의 권위가 아니다.
source `reused=false`와 `calendar_claim_preexisting=true`가 동시에 성립할 수 있으며,
원래 claim을 삭제하거나 달력을 다시 호출해 false로 바꾸지 않는다.

## 실제 검증과 선행 차단

구현 담당은 Paseo `codex/gpt-6-luna`/max agent `e05e8905-6ffb-4e2c-8a42-b2e5a5e8cb75`다.
최초/후속 결과 모두 실제 idle/pending0에서 회수했다.

1. 작업자의 `cargo +1.97.1 fmt --all -- --check` 및 `git diff --check`는 PASS였다.
2. 전체 `dev-check.py ... rust/focused --package collectors` preflight는 disposable DB 미설정으로
   BLOCKED였다. 그 전체 프로필은 미실행이며 이 기록을 PASS로 재분류하지 않는다.
3. 코디네이터는 Raw fixture의 DB 비의존성을 확인하고
   `python3 scripts/qa/dev-check.py preflight --profile rust --stage focused --package collectors --check-id cargo-clippy-workspace`
   의 실제 READY를 확인했다. 도구·manifest pin·기존 private shared slot 전제가 충족됐다.
4. 작업자의 named 후속은 sandbox에서 기존 `/run/user/1000/lagrange-dev-check-heavy.lock`이
   read-only여서 child 전에 멈췄다. 실제 test/Clippy 실패가 아니며 실행 테스트 수는 0이었다.
5. 호스트의 기존 lock owner1000·directory0700/file0600·symlink 없음·쓰기 가능을 확인한 coordinator가
   기존 실행 문맥에서 아래 명령을 순차 실행했다. 작업자 모드나 공유 lock 경로·권한은 바꾸지 않았다.
   기존 `run_child_with_heavy_slot`/`command_environment`를 사용해 child 종료/reap까지 잠금을 유지했고
   `CARGO_BUILD_JOBS=2`, `RUSTUP_AUTO_INSTALL=0`이었다.

| 실제 명령 | 결과 |
|---|---|
| `cargo +1.97.1 clippy --package collectors --all-targets --all-features --locked -- -D warnings` | exit0 PASS |
| `cargo +1.97.1 test --package collectors --test kis_recovery raw_eod_ --locked` | exit0; **14 passed, 0 failed, 0 ignored, 19 filtered out** |

실행: 2026-10-29 10:30:24–10:32:49 UTC(19:30:24–19:32:49 KST), 전체 harness exit0.
14개에는 새/보강된 네 fixture와 closed/conflict/tamper/no-refetch/check-only 회귀가 포함된다.
calendar-only 선행 fixture는 source 재사용 false/claim 사전 존재 true를 검증한다.
fresh·당일 재사용은 총30회/달력1회, historical 두 날짜는 총59회/달력1회의 기존 **fake** 호출 assertion을
보존했다. 실제 broker 요청 횟수나 운영 증거가 아니다.

같은 test target의 19개는 필터에서 제외됐다. 전체 focused 프로필/workspace, DB/browser/hosted CI,
credentialed event 실제 직렬화, 운영·무인 거래일 인수는 NOT RUN이다.

## 검증 입력 identity

다음 SHA-256은 두 명령 실행 전후 일치했다. 검증한 Rust/lock 입력이 바뀌면 재검증해야 한다.

| 파일 | 실행 전후 SHA-256 |
|---|---|
| `data-pipelines/collectors/src/eod_raw.rs` | `385e6b33fe73fa1bfa0efbf893753a770c265a9339eb676bd10953c1ffc404af` |
| `data-pipelines/collectors/src/worker.rs` | `951fb449c06ed2694817018c8587ea329d4f05769be8600d9fcbb20c60d2c230` |
| `data-pipelines/collectors/tests/kis_recovery.rs` | `1eddb0b089f6f886df60195f9d4cc96767f21c91de0da8444b6e0168c8ad6ef7` |
| `Cargo.lock` (미변경) | `56cef9b11fed73359764978e6c8b2b6240d1dfba88684de10e3441c6159c2905` |

private argv·원본 stdout/stderr·영수증은 `target/r5-paper/coordinator/wp2a1-final-verification/receipt.json`
및 같은 경로의 log다. 작업자의 원래 차단은 `target/r5-paper/WP-2A1/report.md`,
`named-heavy-checks.json`에 보존했다. 이 문서가 변경·검증 범위·결과·입력 identity의 장기 요약 근거다.

## 남은 항목

WP-2A1의 소스와 named focused 결과만 수용한다. 전체 trusted origin producer/DB reader/checker,
G1 운용값 O1~O3 선택, 실제 입력·권리 runtime readback, 활성화 전후 실제3거래일,
Paper 실행·원장·복구·Owner 인수는 미완료다.
상태는 [R5 계획](../superpowers/plans/2026-10-29-r5-paper-implementation.md)과
[로드맵](../ROADMAP.md#9-r5--paper-운영)을 따른다. 이번 변경은 commit/push/배포하지 않았다.
