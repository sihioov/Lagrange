# R2 실행 기록

- 실행 요청: Owner가 R2 계획 `cea7baa` 확인 후 `진행해`라고 지시했다.
- 시작 시각: 2026-10-13 13:47 UTC / 22:47 KST.
- 기준 workspace: `/data/worktrees/3puw275b/excited-dolphin`, branch `excited-dolphin`.
- 적용 계획: [R2 일일 연구 베타](../superpowers/plans/2026-10-13-r2-daily-research-beta.md).
- 상태: **R2 DOING; DB 진단과 F1/F2 보정본 `f8b9296` CI·독립 리뷰 수용. 2026-10-14 Owner가 U2를 채택해 WP-4 설계·구현 진행.** 운영 적용·실제 데이터·Owner·3거래일 인수는 후속이다.

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
| WP-1B~F | `4213016a-e26d-4e09-bbc3-e7764797795c` / codex gpt-6.1-sol high; 운영 실행은 코디네이터·WP-1 worker | operator READ ONLY SQL·runner, 현재 집계 및 복구 날짜 검토 | 1C~E 실패 보존; 1F 성공·오프라인 검토 수용, 실제 복구는 미실행 |
| WP-3/F1 | `8a7c638a-bf04-4ea0-892e-ae51c158b089` / codex gpt-6-luna max | daily→backfill 선택 날짜 보존·fake-host 회귀 | 보정본 CI 성공, P2 격리 보정·hostile 환경 self-test·독립 리뷰 수용; 운영 복구는 별도 |
| WP-5/F2 | `7b793a02-47b0-4db5-bfa2-506185b2d67e` / codex gpt-6-luna max | Owner 동일 eligible day 재평가·격리 회귀 작성 | cadence·동시 DB 회귀와 연도 경계 fixture 보정본 CI 성공·독립 리뷰 종료; 운영·WP-4 통합은 별도 |
| WP-4 | 후속 계획에서 분해 | 새로운 ETF 입력·독립 검증·자동 추천 | DOING; 2026-10-14 U2 채택, 설계·구현 접점 확정 |
| WP-6/7 부분 | WP-1 worker / Luna medium; WP-2 worker / Sol high | `3ee453f`·`f8b9296` 기존 CI 회수와 F1/F2 독립 리뷰 | 두 CI terminal success, 보정본 P2 리뷰 종료; 최종 C_R2 인수는 별도 |
| WP-8~10 | 미배정 | 이미지·운영·관찰 | TODO |

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

### 부분 후보 푸시·CI와 리뷰 후속

검증용 `3ee453fe38a54ae92cc098daf61b0eaf56f833cb`를 `origin/excited-dolphin`에
푸시했다. 같은 SHA의 CI 목록은 비어 있었고 `workflow_dispatch`를 한 번 수행했다.
[CI 38415230019](https://github.com/sihioov/Lagrange/actions/runs/38415230019)는
14:39:26 UTC에 시작했고 14:56:44 UTC terminal success를 회수했다. main은 변경하지 않았다.
기존 WP-1 worker에 이 실행만 읽고 최종 checkout·job/step·회귀 결과를 회수하도록 위임했다.

[독립 리뷰](2026-10-13-r2-f1-f2-review.md)의 실제 idle/pending0·exact hashes를 확인했다.
생산 코드의 구체적 finding은 없었으나 테스트에 P2 두 건이 있었다. fake-host 환경 상속을
차단하고, 동적 DB 날짜와 일치하는 전용 QA entitlement를 넣도록 기존 두 구현 worker에 위임했다.
공통 production rights나 실행 guard는 수정하지 않는다. F1 mock은 임시 디렉터리의 정확한
ancestor **directory**만 합성 ownership/mode를 허용하고 file metadata는 fixture 내부로 제한한다.
이는 실제 `/tmp` mode `1777`과 root-only guard 사이의 격리 테스트 전제를 명시한 것이다.
두 후속 worker의 실제 idle/pending0과 전체 diff·hash를 확인했다. F1의 hostile 환경·
baseline 재현·daily self-test는 통과했다. F2 전용 entitlement의 소스 보정은 검토했지만
Rust·실제 역할 DB 실행은 여전히 별도 검증 대상이다. [P2 수용 판단](2026-10-13-r2-f1-f2-review.md#coordinator-disposition--fixture-corrections).

CI worker의 실제 idle/pending0과 [최종 보고서](2026-10-13-r2-f1-f2-ci.md)를 회수하고
run metadata를 재확인했다. 실행 job 7개와 required가 모두 성공했고, workspace 결과 행 합계는
2,891 pass/0 fail/기존 ignored6, 별도 chart6 pass다. F2 late-calendar actual-role 회귀와
cadence 5개도 `ok`였다. PostgreSQL은 APPROVED/59 migrations/skip guard ENFORCED다.
이 성공은 `3ee453f`에 한정하며 이후 테스트 fixture 보정 bytes는 별도 커밋·CI로 검증한다.

### DB 진단 실패 보존과 현재 기준일

WP-1C는 typed rejection, WP-1D는 `DATABASE_PROCESS`/exit3에서 실패했다.
현재 link와 postgres ID·image·이름 검사는 통과했다. 코디네이터는 원문 stderr를 저장하지 않고
SQLSTATE/줄 번호만 추출하도록 진단을 보완했으며, bare SQLSTATE 형식의 누락은 합성 child로 재현했다.
WP-1E에서 `42601`/line402를 확인하고 SQL의 indexes JSON 집계 닫는 괄호 하나 누락을 찾았다.
SQL·contract·runner의 hash를 함께 고친 WP-1F는 14:51:08.639884 UTC에 exit0으로 끝났다.
과거 실패·검토본은 각각 별도 private 경로에 보존했다. 상세 근거는
[DB 기록](2026-10-13-r2-db-baseline.md)과 `target/r2-daily-beta/wp-1f/activation-review.json`이다.

- 161개 schema 전제, 59개 migration 성공·source checksum 일치, pending/mismatch 0 확인.
  7개 index의 존재/valid/ready/table/unique만 확인하며 full predicate 동등성을 주장하지 않는다.
- daily가 쓰는 credentialed EOD 기록은 9개 날짜, 마지막 8월 31일이다. 최신 정확한 ETF11
  발행은 8월 28일까지 8개 날짜이며 두 날짜 기준을 동일하게 취급하지 않는다.
- 역사 추천 성공 2건의 최대 `as_of`는 8월 19일, 현재 v3 pin 일치 건수는 0이다.
  이 결과만으로 과거 입력이 부정하거나 손상됐다고 판단하지 않는다.
- Owner V2 active/READY 1개와 admission·신호 snapshot의 DB 연결 기준일은 10월 6일이다.
  가장 최근 검증된 eligible calendar는 10월 7일; 10월 13일 calendar는 부재다.
  조회 시 queued/running job·running attempt는 0이며 미래 정지/정산 보장은 아니다.
- 공식 `--emit-sessions`를 읽기 전용으로 실행해 기존 artifact/manifest hash를 대조했다.
  8월 18일~10월 13일의 37세션과 daily EOD 9개 날짜의 차이는 **28세션**이다.
  provider 요청·state 갱신 없이 선택 목록만 검증했다. 실제 복구·요청 예산은 후속 검토 대상이다.

전체 결과는 fixed-field JSON 검증·READ ONLY/rollback marker·전후 runtime identity 검사를 통과했다.
물리 Raw/Curated bytes, Owner 로그인 화면, 실제 복구·배포 증거를 대신하지 않는다.
U2는 계속 Owner 채택 대기이며 누락 수집·새 migration·운영 적용은 실행하지 않았다.

### 알림 감시 갱신

기존 heartbeat `0b20083f`를 삭제하고 네 owned worker ID를 포함하는 `a136717b`로 교체했다.
2분 간격/최대60회/2026-10-13 16:50:32 UTC 만료이며 모든 follow-up은 `notifyOnFinish=true`다.
운영 권한이나 sudo 만료를 연장하지 않았다. 결과 회수 후 실제 idle/pending0을 확인해 정리한다.

### DB 집계·복구 범위 검토 수용 — 2026-10-14 KST

Sol/high worker의 실제 idle·pending0과 [DB 기록](2026-10-13-r2-db-baseline.md),
[복구 준비안](2026-10-13-r2-catch-up-readiness.md)을 회수했다. 코디네이터가 원본 집계의
고정 필드·날짜 차이·source 근거를 대조하고 offline `verify-evidence.py`를 재실행해 PASS를 확인했다.
수용 범위는 10월 13일의 READ ONLY snapshot과 그에 대한 오프라인 분석이다.
28개 수집 후보와 8월 31일의 기존 EOD 발행 복구 후보를 분리하고, 기존 range recovery가
전체 eligible manifest와 누적 generation에 미치는 파일·DB 효과를 운영 검토 대상으로 남겼다.

조건부 2,577 logical GET / 10,308 dispatch 계산은 한 process의 소스 상한일 뿐 실행 예산이 아니다.
특히 calendar의 shared retry 상한 4는 저장소 정책의 daily당 `chk-holiday` 최대 1회 호출을
늘리는 승인이 아니다. 실제 실행은 그 정책과 token·process·전체 시간 상한을 함께 만족해야 한다.
새 10월 14일 범위, 보호 bytes·복구 지점·정산·설치 전제는 실행 직전에 별도로 확정한다.
U2를 임의 채택하거나 운영 실행으로 확대하지 않았다. 테스트 보정 두 파일과 관련 문서를
하나의 후속 커밋으로 고정한 뒤 같은 SHA의 CI를 확인한다.

### 후속 커밋 전 remote main 차이 — 2026-10-14 KST

`git ls-remote`와 `git fetch origin main`으로 remote main이
`1286e732c098eff307c9537c3cedffc58ec9f930`임을 확인했다. `b0a2579`와의 차이는 별도
개발 속도 개선 작업 15개 파일이며 CI/workflow·검증 harness·collector 테스트·지침을 포함한다.
`excited-dolphin`과의 merge-base는 여전히 `b0a2579`다. 이번 작업에서 제외된 개발 속도
개선 5가지를 수행하거나 병합하지 않고 R2 수정과 fixture를 현재 브랜치의 workflow로 검증한다.
최종 main 통합 시에는 이 차이와 검증 대상이 달라지는 점을 별도 처리해야 한다.

### 보정된 부분 후보 — 2026-10-14 KST

fixture 두 파일과 진행·진단 문서를 `f8b9296c14cb48dacdf8cacefab6c2abff92bf24`로
커밋하고 `origin/excited-dolphin`에 푸시했다. 소스 hash는 F1
`ccbd3762dfa5d2c0aa1503b01180f498f909e96d3578f48d916b99738448d5b0`, F2
`0bcdabfb1bb7462e216e2fa2f6f29dfeeec881bc7726e40a6233703c4c7d16db`로 worker 검증본과 같다.
기존 exact-SHA run 부재를 확인하고 CI를 한 번 dispatch했다.
[CI 38420816275](https://github.com/sihioov/Lagrange/actions/runs/38420816275)는
2026-10-13 15:17:43 UTC 시작이며 기존 Luna/medium worker가 terminal 근거를 회수한다.
기존 Sol/high 리뷰어는 같은 두 파일의 P2 보정만 독립 재검토한다. 두 follow-up 모두
실제 running/pending0, `notifyOnFinish=true`와 기존 bounded heartbeat 감시를 확인했다.
후속 문서 커밋은 이 검증 대상과 구분하며 U2 구현 전의 부분 후보를 최종 C_R2로 부르지 않는다.

Sol/high의 [정확한 보정본 독립 리뷰](2026-10-14-r2-fixture-review.md)를 실제 idle/pending0
후 회수하고 소스 hash·HEAD blob·보고서의 근거를 직접 대조했다. P2-F1/P2-F2는 소스 수준에서
모두 닫혔고 새 구체적 finding은 없다. 별도 CI의 실행 성공을 이 리뷰로 대신하지 않는다.
private `review/` 부모를 0700으로 제한한 경미한 경로 범위 이탈은 보고서에 보존했고,
이전 mode를 추측해 복원하지 않았다. product·운영 파일은 변경하지 않았다.

### 보정본 CI 인수·이번 실행 정산 — 2026-10-14 KST

`f8b9296`의 [CI 38420816275](2026-10-14-r2-fixture-ci.md)는 2026-10-13
15:38:41 UTC terminal success다. 7개 실행 job의 checkout SHA와 required 성공을 회수했다.
코디네이터가 GitHub terminal metadata와 private JSON·hash·mode를 별도로 대조했다.
workspace 235개 결과 행 합계는 2,892 pass/0 fail/기존 ignored6이며 고유 테스트 수가 아니다.
별도 chart6 pass, 새 year-boundary와 late-calendar 회귀 및 cadence5 모두 `ok`다.
PostgreSQL APPROVED/59 migrations/skip guard ENFORCED이며 Web·Python 단계도 성공했다.
CI policy의 개별 daily/backfill 결과 marker는 노출되지 않아 기존 로컬 실행 증거와 구분했다.

F1/F2 소스 수정·보정·독립 리뷰·CI 검증을 수용한다. 최종 C_R2 고정, U2 새 입력 구현,
main 통합, 이미지·운영 적용·실제 Owner 사용·연속 3거래일 인수는 완료하지 않았다.
다음 의존 작업은 [U2 결정안](2026-10-13-r2-daily-input-decision.md) 채택 후 WP-4 설계·구현
분해다. 그 사이 만든 DB·복구 준비안은 실제 운영 명령의 승인이나 실행 근거를 대신하지 않는다.

네 owned worker의 실제 idle·pending0과 최종 보고 회수를 확인하고 15:51:04 UTC에
heartbeat `a136717b`를 삭제했다. 남은 worker/background/permission wait는 없다.
검증된 scheduler 재평가와 EOD/ETF 발행 복구 구분 교훈은 hook helper의 Mem0 `SAVED`와
memory ID로 저장을 확인했다. 제안 상태인 U2는 채택된 결정으로 저장하지 않았다.

### U2 채택·WP-4 재개 — 2026-10-14 KST

Owner가 다음 작업으로 안내한 일일 입력·자동 추천 계약에 **“그걸로 진행해”**라고 답해
[U2 채택](2026-10-13-r2-daily-input-decision.md)을 기록했다. 계약 승인 대기를 해소하고
R2-04를 DOING으로 전환했다. 시작 HEAD는 clean `619634db32526032aae07ee2024a10f69fc560c4`다.
기존 v3·F1/F2 검증을 보존하며 새 입력·독립 admission·예약·화면의 접점을
[WP-4 후속 계획](../superpowers/plans/2026-10-14-r2-daily-input-implementation.md)에 따라 분해한다.

현재 profiles는 코디네이터용 Astra/high 하나뿐이다. worker용 profile은 없으므로 모델 지원을
조회하고 repository 배정 규칙으로 D1 hard/Astra xhigh와 D2 simple/Luna medium을 선택했다.
각 worker는 보고서 하나만 소유하며 제품 파일·다른 보고서·운영 상태를 변경하지 않는다.

| 패키지 | worker | 실제 runtime | 범위 |
| --- | --- | --- | --- |
| D1 | `1b2bd1a5-94ce-4e5c-9691-26e770617ee0` | codex gpt-6-astra / xhigh | 일일 입력·독립 권한·DB/queue/API 구조 설계 |
| D2 | `e8ae2870-0240-4826-b1dc-0c37d5b03eea` | codex gpt-6-luna / medium | 기존 Web 설정·추천 DTO와 검증 접점 목록 |

두 worker의 cwd·workspace `wks_10fae7d550ab1704`·parent
`ef6cdbb0-8409-4dba-86ca-c07eadccf377`, 실제 running/pending0을 확인했다.
별도 mode 설정은 하지 않았다. `notifyOnFinish=true`와 두 worker만 대상으로 하는 heartbeat
`1aaa7e97`(2분, 최대 60회, 2026-10-14 02:25:55 UTC 만료)을 등록했다.
private ledger에 소유 범위와 회수 상태를 보존한다. 같은 daemon의 감시는 daemon 중단을 보호하지 않는다.
설계 결과를 통합·리뷰한 뒤 exact 파일 소유권과 인터페이스를 확정해 구현을 위임한다.
운영 배포·migration·timer·provider 호출과 과거 추천 backfill은 이번 채택으로 실행하지 않는다.

### WP-4 Web 접점·준비 수용 — 2026-10-14 KST

D2의 [Web 접점 목록](2026-10-14-r2-daily-web-map.md)을 idle/pending0 후 회수했다.
21개 링크·라인 경계와 실제 form/page/API DTO를 직접 대조했다. 기존 화면은 수동 추천 경로이며
auto-update·일일 admission·expected session·support-through·별도 published_at 계약은 없다.
이 목록은 제한된 경로의 소스 근거이며 실행·전역 완전성 증거가 아니다.

설치본 Next 가이드가 없어 D2a를 동일 Luna/medium worker에 별도 범위로 배정했다.
코디네이터 관측은 MemAvailable 2,844,548 KiB, swap free 92 KiB, workspace free 약 588 GB,
compiler process 0이었다. Node heap 512 MiB·10분 timeout·install scripts 비활성으로 준비했다.
최초 app-prefix 명령은 root workspace lockfile을 놓친 명세 오류로 EUSAGE/exit1이었다.
root package/lock과 npm help를 확인해 명령을 보정했고, 한 번의 root workspace install이 exit0였다.
이전 실패와 보정은 [prerequisite 기록](2026-10-14-r2-web-prerequisites.md)에 보존했다.

코디네이터가 세 manifest/lock hash 불변, 설치된 Next 16.3.0·Vitest 4.1.10·Playwright 1.61.0의
Web 해석 경로, 가이드 다섯 개의 존재를 확인했다. D2a도 실제 idle/pending0 후 수용했다.
의존성 설치는 ignored root node_modules와 npm cache만 바꿨으며 제품·lockfile은 그대로다.
테스트·빌드·서버·브라우저 실행·운영 행위는 없었다. D1 구조 설계는 아직 진행 중이다.

### D1 회수·D3 독립 검토 시작 — 2026-10-14 KST

D1은 실제 idle/pending0 후 [926행 설계](2026-10-14-r2-daily-input-design.md)를 회수했다.
SHA-256은 `e74934b7e5909dc69713bac16118a5d816d25e38498ebbe69dffa45304e9c670`이다.
별도 daily 타입·기존 역사 replay 보존·0060 additive migration·독립 파일/DB principal·영구 예약
방향과 I1~I7의 단계적 source allowlist를 제안한다. 소스 구현·테스트·운영 성공 근거는 아니다.
코디네이터가 U2 및 기존 raw-price 요청·일일 action 조회·0052 발행 fence와 핵심 접점을 대조하고,
휴장일 action/calendar 증거의 구조적 충족 가능성 등 확인 질문을 후속 계획에 고정했다.

D3는 `e183dbb1-4e18-4278-a109-7bd9b0855530`, codex gpt-6.1-sol/high이며 이 고정 설계의
독립 리뷰 보고서만 쓴다. 실제 runtime·동일 workspace/parent·running/pending0를 확인했다.
profile은 계속 코디네이터용 하나여서 repository 리뷰 배정 규칙을 적용했고 mode는 지정하지 않았다.
완료된 D1/D2 감시 `1aaa7e97`을 삭제하고, D3 전용 `6d616899`를 2분/최대 60회/
2026-10-14 02:44:42 UTC 만료로 등록했다. create에 `notifyOnFinish=true`를 사용했다.

다음 작업은 D3의 구체적 source finding을 해소하고 첫 구현 묶음의 타입·파일·회귀를 확정해
Luna/max에 위임하는 것이다. 운영 근거 부재만으로 독립 소스 작업 전체를 다시 차단하지 않는다.
R2-04는 DOING이며 제품 코드·0060·실제 입력·자동 추천 실행은 아직 구현/검증하지 않았다.

### D3 인수·D4/D5 분리 — 2026-10-14 KST

[D3 리뷰](2026-10-14-r2-daily-input-design-review.md) 423행·SHA-256
`1ea75c9e1f8612475923775c5b2b49c2f3b7a4a2aa395e482d09cda1c4c9a487`를 실제 idle/pending0 후
회수했다. 코디네이터가 0009의 app table-level 권한, 0049 worker output UPDATE 권한,
generic calendar의 target-date 필터 등 구체적인 소스와 대조해 6개 finding을 수용했다.
설계상의 빈틈이며 아직 실행하지 않은 0060의 런타임 실패로 표현하지 않는다.

- D3-01/02: trading-date-only 공급이 휴장일 action/calendar의 연속 증거를 자동 제공하지 않는다.
  단발 catch-up으로 반복되는 다음 주말의 빈틈이 해소되지 않으며, 별도 offline 소비/projection과
  구체적인 반복 공급안의 준비가 필요하다. 시장 API 호출 권한을 확대하지 않는다.
- D3-03/04: 기존 table 권한을 정확히 교체하고 DB 소유 epoch·ID/owner 경계를 고정해야 한다.
  최초 success gate뿐 아니라 발행 후 output/item/time 불변성·현재 attempt 결합도 명시해야 한다.
- D3-05/06: 새 role·actual-role 검증을 기존 official harness에 연결하고, 완전한 byte schema와
  typed admission conflict 결과·deterministic 무쓰기 normalization 경계를 고정한다.

기존 Sol/high worker `e183dbb1-4e18-4278-a109-7bd9b0855530`는 D4의 정확한 pure wire 계약
한 문서만, Astra/xhigh `1b2bd1a5-94ce-4e5c-9691-26e770617ee0`는 D5 설계 보완 한 문서만
소유하도록 follow-up했다. 둘 다 notifyOnFinish=true·실제 running/pending0·같은 workspace/parent
및 예상 runtime을 확인했다. 고정 D1/D3는 수정하지 않는다.

D3 감시 `6d616899`를 삭제하고 D4/D5 전용 `f840501c`(2분·최대 60회·2026-10-14
02:57:15 UTC 만료)를 등록했다. ledger는 전 단계 결과를 보존하고 새 범위로 갱신한다.
D4가 고정되면 승인 타입·DB·파일 신뢰·provider와 분리된 I1a 계약/encoding/hash 코드·회귀를
Luna/max에 위임한다. D5의 세부 교정이 필요한 경로만 그 결과와 리뷰를 기다린다.
