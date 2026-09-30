# KIS market stream 인계 — 2026-09-30

이 브랜치는 현재까지의 구현과 검증을 보존하는 **중간 인계 커밋**이다.
요청받은 original20–26 테스트는 root 최종 검증까지 완료했다.
전체 KIS 스트림 전환, 30종목 화면, 운영 배포가 완료된 상태는 아니다.

## 1. 새 담당자가 먼저 확인할 것

| 항목 | 기준 |
| --- | --- |
| 저장소 | `https://github.com/sihioov/Lagrange` |
| 인계 브랜치 | `handoff/kis-market-stream-20260930` |
| 분기 기준 커밋 | `7757245553ab8cbed5d6f706e46a87849e0cedc6` |
| 원래 작업 경로 | `/data/worktrees/3puw275b/hot-chipmunk` |
| 구현 계획 | [KIS 시장 시세 구독 전환](../superpowers/plans/2026-09-21-kis-market-stream-transition.md) |
| 타입·DB·wire·SSE 계약 | [KIS market stream contract](../superpowers/specs/2026-09-21-kis-market-stream-contract.md) |
| 지침 | [AGENTS.md](../../AGENTS.md) |
| 이 커밋의 의미 | 구현 보존 + 후속 개발 인계. 전체 통합 인수나 운영 활성화 승인이 아님 |

계획 문서에는 당시의 단계별 승인·중간 실패가 함께 남아 있다.
20–26의 최신 결과는 이 문서와 아래 root completion 증거를 기준으로 읽는다.
계약 요구사항과 실제 완료 여부는 구분한다. 이전 임시 실행 승인, 만료된 기한,
worker/heartbeat 식별자를 재사용하지 않는다.

현재 체크아웃의 커밋과 변경 여부는 다음 읽기 전용 명령으로 확인할 수 있다.

```sh
git branch --show-current
git log -1 --format='%H %s'
git status --short
```

이 문서가 포함된 최초 인계 커밋의 해시는 Basic Memory 인계 노트에도 기록한다.
브랜치를 다른 환경으로 옮길 때는 Git에 없는 로컬 증거의 가용성도 확인해야 한다.

## 2. 보존한 구현과 읽을 파일

| 영역 | 주요 파일 | 역할 |
| --- | --- | --- |
| 시세 도메인 | [market-data/market_stream.rs](../../crates/market-data/src/market_stream.rs) | REST 타입과 구분되는 스트림 quote/status/receipt 타입 |
| 소켓·명령·ACK | [kis-client/market_stream.rs](../../crates/kis-client/src/market_stream.rs) | 세션 소유권, prepare/send/ACK capability, bounded transport |
| 승인·영속 상태 | [market_stream_approval.rs](../../crates/kis-client/src/market_stream_approval.rs), [market_stream_state.rs](../../crates/kis-client/src/market_stream_state.rs) | 시장 시세 전용 approval 및 예약·소유권 상태 |
| wire 파서 | [market_stream_wire.rs](../../crates/kis-client/src/market_stream_wire.rs) | 고정 wire schema, ACK/data/heartbeat framing 검증 |
| 저장소 | [job-queue/market_stream.rs](../../crates/job-queue/src/owner_equity_v2/market_stream.rs) | grant/demand/lease, epoch/fence, subscription, 트랜잭션 게시·snapshot |
| producer | [market_stream_producer.rs](../../crates/job-queue/src/owner_equity_v2/market_stream_producer.rs) | 실제 transport와 저장소를 잇는 소유형 producer, 부분 진행·실패 경계 |
| DB 스키마 | [0055 up](../../migrations/0055_owner_market_stream.up.sql), [0055 down](../../migrations/0055_owner_market_stream.down.sql) | stream 전용 테이블·제약·역할/RLS/lock helper |
| transport 검증 | [domain tests](../../crates/kis-client/tests/market_stream_domain.rs), [transport tests](../../crates/kis-client/tests/market_stream_transport.rs) | synthetic fixture와 실제 local loopback 검증 |
| 저장소·producer 검증 | [C3A tests](../../crates/job-queue/src/owner_equity_v2/market_stream_c3a_tests.rs), [producer tests](../../crates/job-queue/src/owner_equity_v2/market_stream_producer_tests.rs) | 실제 역할 DB, command/ACK/commit/pacing/취소 경계 |
| 테스트 기반 | [producer support](../../crates/job-queue/src/owner_equity_v2/market_stream_producer_test_support.rs), [boundary support](../../crates/job-queue/tests/owner_market_stream_boundary_support/mod.rs), [boundary target](../../crates/job-queue/tests/owner_market_stream_boundary.rs) | task-owned PostgreSQL, 역할별 접근, fixture·정리 검증 |

Cargo manifest/lock과 각 crate의 export도 함께 보존했다.
DB library tests는 `market-stream-db-tests` feature로 opt-in한다.
외부 boundary target 하나의 PASS를 내부 실제 역할 DB tests 전체 PASS로 간주하면 안 된다.

[fixture manifest](../../crates/kis-client/tests/fixtures/market_stream_manifest.json)는
synthetic임을 명시한다. 실제 broker payload나 credential을 Git에 포함하는 용도가 아니다.
0055의 운영 적용은 이 인계에서 수행하지 않았다. 운영 migration 충돌·적용 상태는
배포 단계에서 별도로 확인한다.

## 3. original20–26의 최종 결과

공통 Rust 이름 접두사는
`owner_equity_v2::market_stream_producer_tests::database_cases::`이다.

| 번호 | 정확한 함수명 | 채택된 실행 | 결과 |
| --- | --- | --- | --- |
| 20 | `producer_unsubscribe_suppresses_target_and_retains_other_acked_wire_order` | execution4 | PASS/COMPLETE |
| 21 | `quote_commit_failure_rolls_back_and_returns_no_publication` | execution4 | PASS/COMPLETE |
| 22 | `resumed_pending_and_ack_db_phases_finish_before_wire_and_publication` | execution4 | PASS/COMPLETE |
| 23 | `same_epoch_resubscribe_is_terminal_and_new_epoch_requires_fresh_ack` | execution5 | PASS/COMPLETE |
| 24 | `start_epoch_actual_commit_failure_returns_no_facade_and_rolls_back` | execution5 | PASS/COMPLETE |
| 25 | `start_epoch_committed_but_lost_response_returns_no_facade` | execution5 | PASS/COMPLETE |
| 26 | `start_epoch_future_cancellation_drops_owned_socket_without_resume` | execution5 | PASS/COMPLETE |

각 채택 결과는 Cargo exit 0, 정확한 Rust test 1 passed / 0 failed / 216 filtered,
성공한 cleanup receipt, 별도의 DB catalog 부재 확인을 포함한다.
execution4는 20–22 통과 후 23의 잘못된 상태 기대값에서 중단했다.
그 뒤 original23의 네 assertion 영역만 수정했고, execution5에서는 23–26만 실행했다.
따라서 위 표는 두 실행의 검증된 결과를 합친 것이며, 한 번의 전체 suite 실행이 아니다.
기존에 채택된 original1–19와 pacing 4개도 이번 마지막 실행에서 반복하지 않았다.

execution5의 controller는 2026-09-30 09:58:28–09:58:39 UTC에 exit 0으로 끝났다.
root가 126개 소스 pin, Rust/raw/cleanup/catalog 자료, 종료 증거와 최종 보고서를 확인했다.
소유 PostgreSQL 정지 및 Cargo group 종료가 확인됐고 당시 미회수 child는 없었다.
이는 저장된 실행 시점의 사실이며, 나중 시점의 머신 상태를 대신하는 보장은 아니다.

### 확정하여 고친 실패 원인

1. **정상적인 pacing deferral을 테스트가 오류로 취급했다.**
   strict `apply_desired`는 `Deferred`를 `NotReady`로 바꾸는 계약이다.
   20/22/23의 성공 경로는 `reconcile_desired` 결과를 `Complete` 또는
   정확한 부분 진행을 가진 `Deferred`로 구분한다. bounded eligibility wait 뒤
   같은 slot의 demand를 새로 읽고 한 번만 이어서 실행한다. 이미 ACK된 명령을 재전송하지
   않으며 command 수, ACK gate와 순서를 보존했다.

2. **DB에서 읽은 가격 문자열의 scale 기대값이 틀렸다.**
   `numeric(20,8)`의 `price::text` / `Published` 결과는
   `70001.00000000`, `70004.00000000`이다.
   original20/23의 해당 assertion만 맞췄다. raw transport fixture의 숫자 문자열까지
   일괄 변환하지 않았다.

3. **original20의 복원 snapshot에 필요한 cache row를 fixture가 만들지 않았다.**
   subscription ACK는 subscription 상태를 바꾸지만 latest-value cache를 생성하지 않는다.
   snapshot은 존재하는 cache row와 lease item을 join한다. target을 release하기 전에
   좁은 `#[cfg(test)]` producer bridge로 실제 검증된
   `StreamPublicationContext` / `record_stream_status` 경로를 통해
   `AwaitingFirstTrade`를 저장했다. `len == 1`, `quote == None` 검증과
   production ACK/cache semantics는 유지했다.

4. **original23이 상태 변경 시점을 앞당겨 기대했다.**
   같은 epoch의 재구독 거절은 `set_subscription_desired` 전에 발생한다.
   기존 `ABSENT` tuple은 그대로여야 한다. 새 epoch에서도 durable count가 0이면
   target command가 eligible해지기 전까지 `ABSENT`다.
   unrelated ACK와 pacing deferral 동안 target tuple이 바뀌지 않도록 네 assertion을
   고쳤고 최종 `PENDING_SUBSCRIBE → ACKED` 및 count/order 검증은 보존했다.

과거 R109에서는 wrapper가 `Err(_)`의 내부 값을 버려 최초 실패의 정확한 내부 오류를
복구할 수 없다. 이후 재현하여 고친 위 원인들을 R109의 확정 원인으로 소급하지 않는다.

## 4. 최종 소스 및 로컬 증거

검증된 핵심 소스 SHA-256:

| 파일 | SHA-256 |
| --- | --- |
| `crates/job-queue/src/owner_equity_v2/market_stream_producer.rs` | `9ebc67d81f8ea27a5d134cb454c00bfa7e56fc11f39af7a93c100f622ce13f62` |
| `crates/job-queue/src/owner_equity_v2/market_stream_producer_tests.rs` | `15db864f07de9c9f52a3868dd357f7cbc32e6cccd93f684d143743026a50d645` |

원본 증거 위치는 아래 절대경로다. **이 디렉터리는 Git에 포함하지 않는다.**
다른 host/checkout에서는 없을 수 있으므로 없는 증거를 검증했다고 주장하면 안 된다.

```text
/data/agent-tmp/lagrange-kis-stream-20260921
```

| 상대 파일명 | SHA-256 | 용도 |
| --- | --- | --- |
| `handoff-commit-review-20260930.md` | `daf7c0c9b8c045c3220ba552c46a0d12f8a995cd5eeca0c2d372842a1870a505` | 25개 원본 변경 파일의 checkpoint 범위·참조·후속 경계 독립 검토 |
| `ws-3a-c3b-original20-through26-root-completion-20260930.md` | `c4c47e0f81b7473bdaca1d34246735a2f9074cc65de820e150166d06f4979c07` | 최종 root 채택·범위·한계 |
| `ws-3a-c3b-original20-through26-runtime-report-4.md` | `83b13a5399b09625e8e7037f3e59e3b2f683ffabe11f448751db9fa79171132e` | 20–22 PASS, 23 FAIL 당시 기록 |
| `ws-3a-c3b-original20-through26-runtime-report-5.md` | `70a9c70a088309117fe5764cda5476f49c3230d9f915626daa4bfcb685d9e1a8` | 23–26 PASS, cleanup/종료 |
| `c3b-original23-state-boundary-review-20260930.md` | `291639f17e0b7db7f782eac8a7d89eb170a7cdd008933a4108e2ba3927f3e03c` | 상태 전이의 독립 원인 검토 |
| `c3b-original23-state-boundary-fix-20260930/change.diff` | `316f8e32173a2194e89ca55beb1c2010fdc26decb3d4f6c8c8a620e7ebb81e3b` | 마지막 네 영역의 한정 수정 |

execution5 원본 묶음:
`c3b-original20-through26-runtime-evidence-20260930-095556-turn-25`
(71 files / 3 directories / 18 child receipts / 36 raw streams).
manifest SHA-256:
`9114e6a8213de1b2c0ec8944a9fac21801ddcfcc97ca7c1a9478856775556183`.

현재 coordinator 원장은 같은 디렉터리의 `owned-workers.json`이다.
예전 계획의 `/tmp/lagrange-kis-stream-20260921/owned-workers.json` 대신 실제
`/data/agent-tmp/...` 원장을 먼저 읽는다. 원장은 가변 상태이므로 위 immutable 보고서와
구분한다. 종료된 callback이나 heartbeat 문구만 보고 작업을 재전송하지 않는다.

## 5. 남은 작업과 권장 순서

다음 표는 **전체 프로젝트에서 아직 인수되지 않은 범위**다.
모든 관련 코드가 없다는 뜻은 아니다. 현재 소스와 기존 증거를 대조해 남은 부분만
작은 작업 단위로 확정한다.

| 순서 | 범위 | 다음 산출물 / 필요한 완료 증거 |
| --- | --- | --- |
| 1 | R111 / C3B·WS3B 전체 gap 확인 | 이미 통과한 26개와 기존 C1/C2/C3A 증거를 보존하고, 요구사항별 구현 위치·검증 자료·미완료 이유·다음 작업을 한 표로 작성. 기존 G1–G5 전체 인수 여부를 따로 판정 |
| 2 | WS3B producer 실행 연결 | runner/config와 실제 producer 연결, reconnect/demand lifecycle, epoch/fence/date rollover. 30종목, consumer 1개/10개의 상류 구독 수 동일, 정상 stream 중 REST quote 호출 0, 느린 consumer 격리, write/메모리/명령 budget 검증 |
| 3 | WS4 인증 API/SSE/화면 | snapshot/delta/status version 및 resync, logout/권한 철회, 30종목 자동 admission, dashboard/detail, 탭·offline·숨김 lifecycle, nginx buffering/cache 설정. 실제 인증 API/nginx/Chromium 증거 |
| 4 | WS5 운영 구성 | off-default 모드, immutable release/env/image 일치, 안전한 approval/state 경로·mount/소유권, grant 설치·검증·rollback, 운영 runbook. disposable/fake helper 검증과 운영 입력 준비를 구분 |
| 5 | WS6 독립 로컬 통합 | fake market WS → 실제 producer/writer-role DB → 인증 API/SSE → nginx → 실제 Chromium 연결. bounded soak, queue/메모리/latency/write budget, 다중 탭·보안·기존 EOD 회귀 |
| 6 | main 통합 전 구조 문서 | 아래 두 PlantUML 파일의 실제 `file:line` evidence 갱신 및 PNG 로컬 렌더를 같은 통합 변경에 포함 |
| 7 | WS7 운영/live 인수 | 별도 명확한 실행 범위·권리·network allowlist·현재 calendar·exact immutable release가 준비된 뒤 실제 30 ACK, 공유 수집, SSE/DOM 업데이트, lifecycle/security, 동일 날짜 EOD 증거 |

R111은 새 범용 proof framework를 만드는 작업이 아니다.
우선 [계획 WS3–WS7](../superpowers/plans/2026-09-21-kis-market-stream-transition.md)의
각 요구사항을 실제 파일과 기존 결과에 연결하고, 가장 작은 미완료 leaf를 구현한다.
이번 인계 시점에는 `ws-3a-market-stream-completion-gap-review.md`가 작성되지 않았다.
후임은 20번을 다시 시작하는 대신 여기서 이어간다.

인계 전 독립 검토는 현재 source inventory에 runner/config의 market-stream 연결이
없음을 확인했다. 첫 R111 작업은 다음 세 경계만 읽기 전용으로 대조하면 된다.

1. `crates/job-queue/src/bin/owner-equity-v2-runner.rs`의 실행·스케줄링 진입점.
2. `crates/kis-client/src/read_coordination_config.rs`의 설정 경계.
3. `market_stream_producer.rs`와 저장소 `market_stream.rs`의 공개 facade.

계획 WS3의 구현·검증 항목과 계약 C3B 절을 기준으로 정확한 누락 연결·설정·스케줄링·
인수 checklist를 작성하고 다음 구현 범위를 정한다. 이 첫 검토 자체에는 새 runtime이
필요하지 않다.

다음 구현을 찾을 주요 위치:

- runner: `crates/job-queue/src/bin/owner-equity-v2-runner.rs`, 기존 intraday/config 경계.
- API: `crates/api-server/src/http/`, `crates/api-server/src/repos/`의
  `owner_intraday_quotes.rs` 및 router/state/session.
- Web: `apps/web/lib/products/intraday-*`, `apps/web/components/stock-beta/quote/`.
  Web 작업 전 `apps/web/AGENTS.md`도 읽는다.
- nginx/QA: `deploy/nginx/nginx.conf`,
  `scripts/qa/owner-equity-v2-live-acceptance.*`.
- Ops: `scripts/ops/validate-production-config.sh`, `compose-release.sh`,
  `lib/kis-read-compose.sh`, `provision-linux.sh` 및 관련 runbook.
- 구조 문서: `docs/diagrams/component_architecture.puml`,
  `docs/diagrams/runtime_deployment.puml`와 해당 PNG.

전체 인수는 Source/local, Deployment, Onboarding, Provider subscriptions,
Shared collection, Live screen, Lifecycle/security, EOD를 각각 확인한다.
DB fixture PASS를 브라우저·실제 provider·운영 인수 증거로 대체하지 않는다.

## 6. 실행 및 보존 규칙

- 이미 채택한 original1–26/pacing 결과를 인계나 증거 파일 이름 변경 때문에 반복하지 않는다.
  새 코드 변경이 특정 검증을 필요로 할 때만 영향 범위와 실행 예산을 새로 정한다.
- 과거 Q15, D15/D17/D19/D19R proof/runner, execution1–5의 one-use
  authority/context/entry는 모두 닫힌 이력이다. 재실행·기한 연장·fallback 재사용하지 않는다.
  실행 controller도 Git에 포함하지 않았으며 그 존재가 실행 허가는 아니다.
- 새 로컬 DB 실행이 필요한 경우 소유 cluster/binary/system identity, stopped 상태,
  private Unix socket, 권한·환경, source pins, 단일 실행, cleanup/catalog 및 정지 증거를
  현재 기준으로 갖춘다. 이전 성공 보고서는 현재 머신 상태 확인을 대신하지 않는다.
- 테스트 fixture는 `LAGRANGE_WS3A_SUPERVISOR_URL`을 명시적으로 요구하고
  `DATABASE_URL` fallback을 거부한다. 비밀값을 문서·Git·진단 출력에 기록하지 않는다.
  외부 DB/PG/SQLX override가 task-owned DB를 바꾸지 않도록 환경을 제한한다.
- Cargo는 공통 lock으로 직렬화하고 `CARGO_BUILD_JOBS=2`,
  `--locked --offline`을 유지한다. 이 인계에서는 새 테스트·빌드를 실행하지 않았다.
- 기존 실행에서 쓰던 임시 저장소는
  `TMPDIR=/data/agent-tmp/lagrange-kis-stream-20260921/_runtime-tmp`,
  `PYTHONDONTWRITEBYTECODE=1`이었다. 다른 환경에서는 가용 경로를 먼저 확인한다.
- 과거 실패 DB `lagrange_ws3a_490544_0`, `lagrange_ws3a_524825_0`,
  `lagrange_ws3a_606843_0`의 현재 부재는 입증되지 않았다. 새 범위 없이
  scan/query retry/DROP/수동 repair나 cleanup을 하지 않는다. 이것을 성공한 네 fixture의
  catalog closure와 혼동하지 않는다.
- KIS 계좌·잔고·주문·체결/주문 WebSocket은 계속 금지한다. 시장 데이터 권리와
  정확한 method/path/TR/host 승인은 별도이며, 이 checkpoint가 새로운 live 권한을 주지 않는다.
  운영 key가 있다는 이유로 테스트나 새 endpoint를 실행하지 않는다.
- 기존 운영 DB·서비스·immutable release를 직접 고치지 않는다. 운영 빌드는
  AGENTS.md의 서비스 하나씩 직렬 실행, jobs2, 메모리/OOM 확인 규칙을 따른다.
- private raw evidence/DB/credential/state/authority 파일은 Git에 추가하지 않는다.
  원본은 보존하고 인계에는 사실·해시·위치만 기록한다. 과거 procedural finding은
  현재 소스가 통과했다는 이유로 삭제하거나 소급 면제하지 않는다.

이 인계 커밋의 검증은 Git diff 무결성, 의도한 파일 집합, 기존 소스 hash 보존,
문서 링크 및 저장된 완료 증거 확인이다. 새로운 전체 suite, 운영 배포 또는 live 수신
성공을 주장하지 않는다.
