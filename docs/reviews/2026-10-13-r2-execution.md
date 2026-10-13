# R2 실행 기록

- 실행 요청: Owner가 R2 계획 `cea7baa` 확인 후 `진행해`라고 지시했다.
- 시작 시각: 2026-10-13 13:47 UTC / 22:47 KST.
- 기준 workspace: `/data/worktrees/3puw275b/excited-dolphin`, branch `excited-dolphin`.
- 적용 계획: [R2 일일 연구 베타](../superpowers/plans/2026-10-13-r2-daily-research-beta.md).
- 상태: **R2 DOING; WP-1A/2 오프라인 진단 수용, G1 부분 결정. DB 진단과 U2는 미완료.** 실행 요청은 계획의 기존 승인·새 결정 구분을 유지한다.

## G0와 기준선

- 시작 HEAD `cea7baa6298402db79d655140ad4c199f57f8697`, worktree clean.
- `git fetch origin main` 후 main은 `b0a2579bef472ae6b604405d0ab6f50255786dd4`다.
  HEAD와 차이는 R1 인수·R2 계획을 포함한 문서 9개/로컬 커밋 7개다. merge/push는 수행하지 않았다.
- 검증 이미지 후보는 C3 `862fa954c16d28a617271fe6f613d34226e2d558`로 보존한다.
  [R1 완료 근거](2026-10-13-r1-c3-images.md)는 과거 시점의 인수이며 새 운영 상태를 대신하지 않는다.
- `$paseo-delegate`를 읽고 MCP 경로를 사용했다. profile에는 Astra/high 코디네이터만 있었고,
  model discovery에서 `gpt-6-luna/medium`, `gpt-6.1-sol/high`의 실제 지원을 확인했다.
- worker 두 개의 cwd/workspace와 parent label을 확인했다. workspace `wks_10fae7d550ab1704`,
  parent `ef6cdbb0-8409-4dba-86ca-c07eadccf377`다. mode를 변경하지 않았다.
- 두 launch 모두 `notifyOnFinish=true`. 보조 heartbeat `0b20083f`는 두 owned worker만 대상으로
  2분 간격/최대 60회/2026-10-13 15:47:56 UTC 만료로 등록했다. 새 운영 권한을 부여하지 않는다.
  실제 종료·대기 권한·보고 회수 후 정리하며, 만료 전 미완료 상태를 재조정한다.
- private 소유권 ledger: `target/r2-daily-beta/owned-workers.json`.

## 작업과 수용 상태

| 패키지 | worker / 모델·effort | 범위 | 상태 |
| --- | --- | --- | --- |
| WP-1A | `11a8f223-6f2d-4adb-89ab-86dbc917d2fd` / codex gpt-6-luna medium | 현재 호스트의 고정 metadata 수집, DB 조회문/runner 준비 | 부분 수용; 아래 G1과 각 보고서의 검증 한계 유지 |
| WP-2 | `4213016a-e26d-4e09-bbc3-e7764797795c` / codex gpt-6.1-sol high | 기존 입력·calendar·추천·upgrade/복구 계약의 오프라인 대조 | 부분 수용; 아래 G1과 각 보고서의 검증 한계 유지 |
| WP-1B | `4213016a-e26d-4e09-bbc3-e7764797795c` / codex gpt-6.1-sol high | operator READ ONLY SQL·runner 오프라인 구체화 | DOING; 실행 전 exact bytes 검토 |
| WP-3/F1 | `8a7c638a-bf04-4ea0-892e-ae51c158b089` / codex gpt-6-luna max | daily→backfill 선택 날짜 보존·fake-host 회귀 | DOING |
| WP-5/F2 | `7b793a02-47b0-4db5-bfa2-506185b2d67e` / codex gpt-6-luna max | Owner 동일 eligible day 재평가·격리 회귀 작성 | DOING; Rust/DB 실행은 자원 직렬화 후 |
| WP-4 | 미배정 | 새로운 ETF 입력·자동 추천 | BLOCKED; U2 결정안 대기 |
| WP-6~10 | 미배정 | 후보 검증·리뷰·이미지·운영·관찰 | TODO |

## WP-1A 읽기 범위

현재 root 접근, current link·release/manifest metadata, 설치 policy의 검증된 mode 값,
commit별 unit·timer의 고정 상태/시각, known container의 Id/Image/revision/State/health,
memory/swap/PSI/disk와 compiler 존재만 읽는다. env 전체·Docker 전체 inspect·원문 journal은 출력하지 않는다.
보호 JSON의 날짜/상태/count/hash를 선택하고 불명확한 shape는 추측하지 않는다.

새 운영 SQL은 아직 허용하지 않았다. 기존 R1 probe의 local socket·READ ONLY transport를 참고해
bounded query/runner 초안을 만들면 코디네이터가 exact bytes와 scope를 검토한다.
`scripts/ops/lib/db.sh`의 공식 DB helper는 one-shot container를 생성하므로, 단순 `--check` 이름만으로
WP-1A의 무변경 읽기 범위에 포함하지 않는다. 승인된 후속 조회는 별도 기록한다.

## 금지·후속

이 wave에서는 코드/보호 파일 수정, lifecycle·build·migration·timer 활성화·provider 호출·거래·WS 수요를
시작하지 않는다. WP-2는 운영 조회도 하지 않는다. 새 발견은 현재 시각의 관측과 source evidence로 구분한다.
WP-1/2의 실제 결과를 회수·대조한 뒤 U1/U2/U3와 원인·수정 allowlist를 G1에서 판단한다.
R2-01/02가 진행 중이라는 표시가 완료·운영 적용·실제 Owner 인수를 뜻하지 않는다.

## G1 부분 결정 — 2026-10-13

WP-1A와 WP-2의 실제 idle·pending permission 없음·보고 회수를 확인했다.
[운영 기준선](2026-10-13-r2-runtime-baseline.md)에서 current `fce9b90`,
`owner_daily`, 비활성 daily timer, 유효한 release-local env와 manifest를 확인했다.
실행 중 manifest 대상 8개 중 3개가 일치하고 5개는 `74b39fa7` 이미지다.
원인을 추정하거나 자동 교체하지 않는다. DB 발행·누락·migration 증거는 아직 없다.
[계약 분석](2026-10-13-r2-contract-and-recovery.md)과
[운영 runbook 초안](../runbooks/r2-daily-beta-operations.md)을 소스 근거로 수용한다.

- **WP-3/F1:** 기존 daily의 pre-close 필터가 backfill의 날짜 재구성에서 사라지는
  제어 흐름을 직접 확인했다. 실제 backfill 선택을 통과하는 fake-host 재현 후 수정한다.
  정확한 수정 범위는 `scripts/ops/kis-daily-production.sh`,
  `scripts/ops/backfill-production.sh`, `scripts/ops/kis-daily-production-self-test.sh`,
  필요 시 `scripts/ops/backfill-production-self-test.sh` 및 지정 보고서다.
  WP-3 최대 범위를 backfill 두 파일까지 확장한다. 선택 날짜 전달은 validated calendar의
  정렬·중복 없는 부분집합으로만 좁히고, full-range V4 identity와 상태/실패 보존을 유지한다.
  `--session-dates` 선택 필터를 추가하며 기본 backfill 동작은 보존한다.
  16:29:59/16:30:00, 오늘만 누락, 내부 누락, 재실행, FAILED/74/75를 검증한다.
- **WP-5/F2 독립 분기:** U2 대기 중 기존 Owner scheduler만 수정한다.
  성공을 civil-day 영구 완료로 취급하지 않고 기존 60초 retry 간격으로 재평가한다.
  DB의 admitted generation/active-job/idempotency fence를 그대로 사용한다.
  CLOSED/no-work에서도 새 작업을 만들지 않고, 늦은 calendar와 이전 작업 정산 뒤 진전을 확인한다.
  수정 범위는 `crates/job-queue/src/bin/owner-equity-v2-runner.rs`,
  필요 시 `crates/job-queue/src/owner_equity_v2/schedule.rs`,
  `crates/api-server/tests/owner_equity_v2_db.rs`와 지정 보고서다.
  API/DTO/Web/schema/provider는 동결한다. WP-4 재개 후 통합 회귀는 별도 필수다.
- **DB 진단 준비:** WP-2의 SQL 초안을 operator-wide 집계로 구체화한다.
  기존 postgres container의 local socket, 강제 READ ONLY/timeout/RLS 우회 가시성 guard,
  날짜·count·enum·hash만 출력하고 actor를 가장하지 않는다. exact SQL/runner 검토 전 실행하지 않는다.
- **U2:** active v3의 끝은 2026-08-28이며 daily owner-beta producer는 없다.
  과거 승인 artifact를 늘리거나 generic PAPER recommendation으로 대체하지 않는다.
  새 입력의 승인·불변 lineage·예약·중복 방지 설계가 정해지기 전 WP-4는 BLOCKED다.

이 결정은 소스 수정과 격리 검증만 진행시킨다. 새 운영 적용·서비스 교체·migration·
provider 호출·timer 활성화는 아직 실행하지 않았다. 실제 사용과 연속 3거래일도 미검증이다.

### U2 결정 요청과 과거 daily 원인 구분

[일일 입력·자동 추천 결정안](2026-10-13-r2-daily-input-decision.md)을 작성해 Owner에게
채택 여부를 비동기로 요청했다. 아직 답변을 승인으로 간주하지 않는다. ADR-0005의 기존
개인 이용 권리는 재확인하지 않으며, ADR-0006과 구분되는 자동 승격 정책이 결정 대상이다.
WP-3/F1·WP-5/F2·DB 진단 준비는 이 답변에 의존하지 않는다.

[2026-09-07 사건 기록](../runbooks/kis-daily-stale-release-20260907.md)은 구 daily의 실패와
Owner의 obsolete timer 비활성화, 교체·재수집 미완료를 기록한다. `db7de37` ancestry와
현재 `pipeline.rs`의 all-action Raw 제외 코드를 확인했다. 그 원인을 별개의 research 종료나
현재 DB 상태에 재사용하지 않는다. 당일 누락/발행은 여전히 미검증이다.

새 두 worker의 parent label·workspace와 running/pending0을 launch 후 확인했다.
`notifyOnFinish=true`가 등록됐고 코디네이터는 활성 상태에서 결과를 수용한다.
기존 heartbeat는 최초 두 ID만 감시하므로 새 worker 전체 감시를 대체한다고 주장하지 않는다.
문서 8개의 로컬 링크 130개와 WP-1B가 아닌 WP-1A binding receipt hash를 직접 검증했다.

### WP-1C — 검토한 운영 READ ONLY 집계 승인

WP-1B의 SQL 전체와 runner를 직접 읽고 exact hash를 대조했다. 쿼리는 고정 catalog·
metadata SELECT, READ ONLY/timeout/전역 가시성 guard 및 rollback만 포함한다.
actor 설정·provider·새 container·운영 변경은 없다. local Docker socket 접근 가능도 확인했다.
초안은 보존하고 `target/r2-daily-beta/wp-1c/`에 실행 검토본을 별도 만들었다.
SQL 본문은 바꾸지 않고 inert gate와 연동 hash만 일치하도록 변경했으며,
정의만 로드해 lexical scope·contract/source hash 검사를 통과했다. 이 시점 SQL은 미실행이다.

- runner SHA256: `8ac22ff72999cd00e6b1d1502eb4d51868594033b66df24182c2b670dced1e94`
- SQL SHA256: `e685570d8a5c17eb6d713388f7347d76b5eb8b38f5fb039d2ebcd578b55765df`
- contract SHA256: `577a1d3526e2d9c7a29326ba02d7dec3654b4578aa3ae06b49db1defc7ab829c`
- local migration manifest SHA256: `5793c369f863b0965dc8bb65936f2e4e75287360be4444d1351a4c53c2d67300`

기존 WP-1 worker가 이 exact runner를 한 번 실행해 sanitised 집계만 회수하도록 승인한다.
current·postgres ID/image·시작 시각·restart count가 전후 일치해야 하며 drift/실패는 중단한다.
1초 lock·10초 statement·전체 180초 및 512KiB 출력 상한을 유지한다. AVAILABLE은 건강 판정이 아니다.

### F1/F2 부분 검증 커밋 준비

F1 worker의 실제 idle/pending0, baseline 재현 및 변경 self-test PASS를 회수하고 패치를
직접 읽었다. F2는 monotonic cadence와 5개 unit case/actual-role DB case를 작성했으며
실행은 아직 하지 않았다. 코디네이터는 동시 예약 assertion을 추가하고 fixture의 권리 검사를
분리했다. 구조·API·schema·Compose 의존성은 바꾸지 않아 architecture diagrams는 유지한다.

두 수정의 검증용 커밋을 별도로 고정해 기존 CI workflow를 feature branch에서 실행한다.
먼저 같은 SHA의 기존 실행을 조회하고 중복 dispatch하지 않는다. 이 CI는 hosted disposable DB를
사용한다. U2 구현 전의 부분 후보이므로 C_R2 최종 고정·출시 인수나 C3 대체를 뜻하지 않는다.
Sol/high의 독립 소스 리뷰도 병행하며 finding이 있으면 수정·영향 검증을 갱신한다.
