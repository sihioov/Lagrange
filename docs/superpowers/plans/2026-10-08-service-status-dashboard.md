Execution skill: $paseo-delegate (required)
Native subagents: prohibited for worker packages

# Lagrange 서비스 상태 대시보드 실행 계획

- 작성일: 2026-10-08 KST
- 상태 근거 (2026-10-09 KST): 구현 및 격리 QA 수용. Web 483개 단위 검증, API 관련 38개와 변경 없는 이전 29개, 호스트 회귀 41개가 통과했다. 기존 125개 판정과 실제 API→Web 응답 10개가 일치하며, 최신 브라우저 88개는 76개 통과·실패 0개·기존 REST 비활성 모드 12개 제외로 완료했다. 실제 격리 DB/API의 daily→daemon 전환과 별도 stream 검증도 통과했다. 근거는 [QA 기록](../../reviews/2026-10-08-service-status-qa.md), 최종 독립 판정은 [WP-6 리뷰](../../reviews/2026-10-08-service-status-review.md)에 기록한다. 운영 설치·활성화는 수행하지 않았다.
- 기준 commit: `c3295be6bb618483c6b2594de5232faea66bd8f3`
- 작업 디렉터리: `/data/worktrees/3puw275b/itchy-bulldog`
- 사용자가 `$paseo-delegate` 실행을 요청했다. 구현·격리 검증을 수행하며 운영 서비스는 변경하지 않는다.

## Goal and boundaries

### 목표와 완료 기준

Owner가 `/stock-beta`에서 서비스의 실행 여부와 실제 데이터·작업 처리 상태를 함께 확인한다.
차트 위쪽에 작은 운영 상태 요약을 두고, 펼치면 서비스별 상세 표를 보여준다.
기존 차트의 첫 번째 전체 너비 배치와 종목 선택·확대·축소 동작을 보존한다.

완료 조건:

1. 아래 서비스 목록을 누락 없이 표시하고, 상시 프로세스·예약 작업·데이터 기능을 구분한다.
2. 실행 중인 컨테이너에서도 시세 미수신, 달력 부재, 일봉 갱신 지연, 작업 실패를 표시한다.
3. 정상 대기, 의도적 비활성, 장애, 확인 불가를 구별한다. 관측 누락을 정상이나 비활성으로 추정하지 않는다.
4. 각 행에 마지막 확인 시각을 표시하고, 적용 가능한 마지막 성공·데이터 기준일·다음 실행·실패 이유를 제공한다.
5. Owner 권한을 API와 화면에서 검증한다. 비로그인·Member에는 상세 운영 정보를 반환하지 않는다.
6. 상태 조회는 기존 정보를 읽기만 한다. 조회 때문에 KIS 호출, 시세 구독, 수집·재시도·복구 작업이 발생하지 않는다.
7. API 계약·호스트 수집기·상태 판정·권한·브라우저 회귀를 검증하고, 근거가 없는 정상 판정을 막는다.
8. 호스트 수집기가 설치되지 않은 개발 환경에서도 차트와 API의 기존 기능이 동작한다.
   운영 상태의 해당 항목만 `unknown`으로 표시한다.
9. 새 API·호스트 관측 경로·read-only mount를 구조 다이어그램과 근거 주석에 반영하고 PNG를 로컬 렌더링한다.
10. 독립 리뷰의 blocking finding이 없고 coordinator가 전체 변경을 수용한다.

### 표시 대상

| 분류 | 고정 논리 ID / 대상 | 주요 관측 |
|---|---|---|
| 기본 서비스 | `postgres`, `reverse-proxy`, `web`, `api-server` | 프로세스·health/readiness 관측, 마지막 확인 |
| 기능 작업자 | `owner-equity-v2-runner`, `owner-beta-runner`, `candidate-runner`, `recommendation-runner`, `nt-backtest-worker-1`, `nt-backtest-worker-2` | 실행 상태, 실제 근거가 있는 작업 진행·대기·실패 |
| 일봉 수집 | `kis-eod` + 실행 주체 `research-worker` | 예약 활성, 실행 모드, 마지막 시도/성공, 게시된 데이터 기준일, 다음 실행 |
| 실시간 시세 | `market-stream` — Owner V2 프로세스 안의 기능 | 연결·현재 수요·세션 상태, 마지막 수신, typed 중단 이유 |
| 거래일 준비 | `intraday-calendar` | 당일 검증된 달력과 운영 시간창의 준비 여부를 각각 확인 |
| 운영 예약 작업 | `production-backup`, `production-backup-verify`, `tls-renewal` | timer 활성, 마지막 결과/성공, 다음 실행 |
| 호스트 기반 | `docker`, `tailscaled` | 고정 systemd unit 상태 |
| 선택 기능 | `paper-scheduler`, `metrics`, `live-node-owner` | 신뢰할 수 있는 실행 정책에 따른 비활성 표시; 활성화하지 않음 |

`research-worker` 데몬과 일회성 daily 실행은 같은 기능의 실행 방식으로 묶는다.
daily 방식이 정상이라면 오래된 정지 데몬 때문에 전체 장애로 판정하지 않는다.
`db-role-bootstrap`, `db-migrate`, `research-raw-init`, `research-schema-check` 및 수동 backfill 도구는
상시 정상 판정의 분모에 넣지 않는다. 필요한 설명은 상세 패널의 실행 방식 안내에 한정한다.
고정 서비스 목록은 현재 `docker ps` 결과에서 생성하지 않는다. 그래야 사라진 필수 서비스도 표시할 수 있다.

### 범위와 제외

포함: 읽기 전용 상태 수집기와 설치 코드, 운영 상태 API와 타입 계약, Owner 대시보드 표시,
필요한 최소 DB 읽기 권한/계측, provider-free fixture와 검증, 운영 runbook, 구조 다이어그램.

제외: 운영 설치·배포·재시작, 현재 멈춘 수집기 복구, 실제 KIS/타 외부 데이터 요청,
거래일 자료 신규 취득, 시세 구독 생성, 스케줄 활성화, 작업 재실행 버튼, 알림 발송,
Prometheus/Grafana 도입, 장기 상태 이력 저장소, 실거래·계좌 기능.
개발·검증용 관측기는 임시 경로와 fake 명령을 사용하며 운영 socket·DB·systemd를 변경하지 않는다.

서비스 상태 대시보드는 앱과 같은 장애 영역에 있다. 이미 열린 화면은 API 단절을 표시할 수 있지만,
웹/호스트 전체가 내려간 경우에도 새로 접속할 수 있는 외부 상태 페이지를 제공하는 범위는 아니다.

### 적용 지침과 확인한 근거

- `/home/l1nnx/.codex/AGENTS.md`: bounded delegation, 모델/effort 배정, 단일 단계 상향,
  검증 기반 수용, 정확한 Mem0 hook 사용. Basic Memory 자동 접근 금지.
- 저장소 `AGENTS.md`: KIS/OpenDART 경계, 운영 자원 제한, 구조 다이어그램 동시 갱신.
- `apps/web/AGENTS.md`, `apps/web/CLAUDE.md`: Web 구현 전 설치된 Next의 관련 가이드 읽기.
- `/home/l1nnx/.agents/skills/paseo-delegate-plan/SKILL.md`: 계획 형식과 패키지 실행 계약.
- `/home/l1nnx/.agents/skills/paseo-delegate/SKILL.md`: 실행 시 다시 읽을 Paseo 선택·시작·감시·복구 계약.
- 현재 checkout의 추가 하위 `AGENTS.md`/`CLAUDE.md`는 Web의 두 파일뿐이다. 실행 시작 때 다시 확인한다.
- `apps/web/components/stock-beta/dashboard/widget-registry.ts:34`: 차트를 맨 앞 전체 너비로 유지하는 catalog.
- `apps/web/app/(authenticated)/stock-beta/page.tsx:145`: 기존 Owner 제품 진입 경로.
- `crates/api-server/src/repos/ops.rs:560`: 기존 `/admin/workers`의 생존 정보는 RUNNING job claim에서 파생된다.
  유휴 작업자의 프로세스 생존을 보장하는 전체 inventory가 아니다.
- `crates/api-server/src/runtime.rs:1345`: liveness와 DB/audit readiness의 기존 구분.
- `scripts/ops/kis-daily-production.sh:40`, `scripts/ops/install-kis-daily.sh:273`:
  daily는 16:30 KST의 bounded one-shot 실행이다.
- `scripts/ops/backfill-production.sh:578`: daily가 사용하는 bounded 경로는 실행 중인 research 데몬을 거부한다.
- `docs/runbooks/stock-beta-intraday-quotes.md:201`: calendar bootstrap은 명시적 일회성 경로다.
- `apps/api-server/openapi.json`, `apps/api-server/generated/openapi.ts`,
  `crates/api-server/tests/openapi_contract.rs`: API contract와 route inventory 일치 검증 경로.

2026-10-08 09:48 KST의 이전 읽기 전용 점검에서는 상시 컨테이너 10개가 healthy였지만,
daily timer들은 disabled였고 market-stream의 최근 저장 상태는
`DISCONNECTED / CALENDAR_UNAVAILABLE`였다. 이것은 테스트 사례의 동기이며 현재 상태나 고정 설정이 아니다.
운영 관측 시각·실제 ID·과거 commit·개인 정보는 fixture에 그대로 복사하지 않는다.

### 구현 방향과 WP-1에서 확정할 계약

아래 방향은 coordinator의 계획 결정이다. WP-1은 소스 근거와 구현 가능한 권한을 검증하고
명시된 미확정 항목을 닫는다. 다른 구조가 필요하면 임의 변경하지 않고 변경 제안을 반환한다.

**관측 경로**

1. 호스트의 전용 `lagrange-service-status.service`/`.timer`가 고정 목록의 Docker·systemd 상태를 읽는다.
   긴 상주 프로세스 대신 30초 간격의 bounded one-shot을 기본값으로 한다.
2. root가 소유한 전용 디렉터리에 제한된 schema의 `snapshot.json`을 원자적으로 교체한다.
   API에는 그 디렉터리만 read-only로 mount한다. 파일 하나의 bind mount로 inode 교체를 놓치지 않는다.
3. API는 snapshot과 기존 DB의 검증된 작업·publication·달력·stream 근거를 결합한다.
   브라우저가 status GET을 호출할 때 host 명령이나 provider 요청을 실행하지 않는다.
4. 신규 `GET /api/v1/admin/service-status`가 Owner에게 상태를 반환한다.
   기존 `/admin/workers`, `/healthz`, `/readyz`의 의미와 호출 계약은 유지한다.
5. 화면은 하나의 상태 조회 루프를 공유한다. 펼침/접힘 때문에 중복 poll을 만들지 않는다.

**권한과 데이터 경계**

- Web/API에 Docker socket, systemd bus, sudo 권한을 주지 않는다.
- 수집기는 허용된 서비스·unit과 속성만 읽는다. 임의 명령, 사용자 입력으로 만든 shell,
  전체 `docker inspect`, 전체 env/config, healthcheck 출력, journal 원문, provider 오류 본문을 수집하지 않는다.
- 호스트 snapshot에는 인증정보·계정/사용자/세션/lease ID·broker payload가 없다.
  명령 timeout/실패는 고정 코드로 바꾸고 원문 stderr를 응답이나 진단 파일에 넣지 않는다.
- 전용 state 경로의 symlink·소유자·권한·크기·schema·부분 쓰기를 검증한다.
  API UID가 읽을 수 있는 최소 권한과 root 전용 쓰기 권한을 WP-1에서 확정한다.
- DB는 기존 Owner 권한과 actor-scoped 읽기를 재사용한다. 부족한 권한은 필요한 safe aggregate만
  노출하는 additive migration으로 제한하며, 기존 RLS나 worker 쓰기 권한을 넓히지 않는다.
- migration이 필요하면 WP-1이 필요성과 대상 role을 기록하고 coordinator가 번호/소유권을 고정한다.
  기존 migration을 수정하지 않는다. 기존 protected broker state를 API에 mount하지 않는다.
- status GET의 감사 정책은 기존 admin API 규칙과 맞춘다. polling 때문에 무제한 audit 부하가 생기는지
  확인하고, 기존 감사 요구를 무단 생략하지 않는다.

**상태 계약**

- 공통 envelope: `schema_version`, `generated_at`, `overall_status`, `services`.
- 각 항목: 고정 `id`, `kind`, 명시적 기대 실행 정책, `process_status`, `work_status`,
  종합 `status`, typed `reason_codes`, `observed_at`, `valid_until`.
- 해당하는 항목에만 `last_success_at`, `last_failure_at`, `data_as_of`, `next_run_at`,
  현재 queued/running 집계와 bounded 실패 집계를 제공한다. 불명·비적용은 구분한다.
- 종합 상태는 `healthy`, `waiting`, `degraded`, `failed`, `unknown`, `disabled`의 닫힌 enum이다.
  문자열 번역은 Web이 담당하며 API는 임의 오류 문장을 전달하지 않는다.
- `generated_at`을 갱신해 오래된 원본 관측을 새 관측처럼 보이게 하지 않는다.
  부분 실패 시 살아 있는 다른 항목은 표시하되, 확인 실패한 항목은 개별 `unknown`이다.
- 호스트 수집은 30초, snapshot 유효기간은 90초로 둔다. 화면은 30초를 기본으로 하되
  행의 유효시간이 먼저 끝나면 1초 전에 재조회하고, 직전 요청 완료 후 최소 5초와 Retry-After를 지킨다.
  이는 신규 운영 화면의 전달 지연 정책이다. 실제 시세/worker lease의 기존 TTL을 대체하지 않는다.
- 미래 시각·잘못된 enum·schema·중복 ID·oversize·stale snapshot은 해당 근거를 거부한다.
  기준 시각과 임계값 경계는 테스트에서 고정 clock으로 재현한다.
- 실패가 확인된 필수 기능은 전체 상태에도 반영한다. 일부 필수 관측이 unknown이면 전체 healthy를 금지한다.
  비활성 선택 기능은 정상 개수에 섞지 않는다. 정확한 우선순위/집계표는 WP-1 산출물로 동결한다.

**업무 판정**

- 장 종료/휴장/정상적인 무수요는 검증된 달력·시간창·수요 근거가 있을 때 `waiting`이다.
  거래일 증거가 없는 것을 장 종료로 추정하지 않는다. 무수요가 달력 준비 실패를 숨기지 않는다.
- 일봉 최신성은 검증된 마지막 완료 세션과 실제 게시된 dataset 기준일을 비교한다.
  UTC 날짜, 단순 경과 24시간, 평일 계산만으로 stale을 판단하지 않는다.
- ETF 공통 EOD와 Owner 종목 V2 데이터를 구분한다. 한쪽 최신 결과가 다른 쪽의 지연을 숨기지 않는다.
- 활성화되어야 하는 timer의 부재/disabled는 수집 예약 문제다. one-shot service가 실행 사이에
  inactive인 것은 정상이며, 종료 코드 0만으로 데이터 게시 성공을 추정하지 않는다.
- 유휴 queue와 RUNNING claim 없음은 daemon 사망 증거가 아니다.
  복구된 과거 실패가 영구 장애로 남지 않도록 현재 상태와 최근 실패 집계를 분리한다.
- 읽을 수 있는 근거가 없는 last-success/heartbeat는 만들지 않는다. 필수 관측의 부재는 WP-1의
  gap으로 기록하고, 보완 계측 범위를 coordinator가 승인한 뒤 구현한다.

**화면 계약**

- `/stock-beta`의 Owner 경로에 작은 요약과 접이식 상세 표를 추가한다.
  status를 불러오는 동안 차트 SSR/초기 데이터 표시를 막지 않는다.
- 차트·signal snapshot이 없거나 오류인 상태에서도 기존 Owner 인증이 성립하면 운영 상태를 볼 수 있어야 한다.
  entitlement나 제품 데이터 오류가 상태 패널까지 숨기지 않도록 진입 위치를 검증한다.
- 문제 항목을 앞에 두되 안정된 ID/정렬과 keyboard focus를 유지한다. 매 poll 전체 DOM을 재생성하지 않는다.
- 정상/대기/주의/장애/확인 불가/비활성을 텍스트와 시각 표식으로 함께 표시한다.
  한국어·영어, KST 시각/상대 시간, 좁은 화면, forced-colors, 키보드를 지원한다.
- 마지막 응답 뒤 네트워크 단절/오류가 발생하면 마지막 확인 시각을 유지하고 확인 불가로 전환한다.
  보이지 않는 탭에서는 주기 요청을 멈추고, 복귀 시 재검증한다. 중복 요청·늦은 응답·unmount를 처리한다.
- 운영 상태 기능 때문에 기존 market-stream lease/SSE를 생성하거나 늘리지 않는다.

### 미확정 항목과 처리 지점

사용자에게 필요한 추가 제품 선택은 현재 없다. 상단 요약+펼침 상세, Owner 전용, 조회 전용을 기본으로 한다.
아래 기술 항목은 WP-1의 필수 산출물이며 해결 전 WP-2~4를 시작하지 않는다.

1. 각 서비스/업무의 실제 관측 source, role별 SELECT 가능 범위, idle 상태의 증거.
2. 현재 release에 대응하는 daily unit 선택, 기대 실행 모드/선택 기능의 신뢰 가능한 정책 출처.
3. host snapshot의 소유권/경로/상한, API 인증·audit·부분 실패 계약, 상태 우선순위.
4. 각 EOD source의 완료 세션·마감 후 허용 지연 판정과 job 실패 회복 규칙.
5. 필요한 최소 migration/계측, 실제 수정 파일 목록, 격리 DB 검증의 정확한 실행 명령.

## Initial classification

복잡도는 모델 선택보다 먼저 판정했다. 아래 분류는 최초 판단이며 실행 중 관측으로 재분류한다.

| Package | Complexity | Basis | Confidence | Reclassification or escalation signals |
|---|---|---|---|---|
| WP-1 | hard | host 권한·DB 역할·세션 판정·기대 실행 정책을 연결하는 계약 결정; 거짓 정상 판정의 비용이 큼 | high | 기존 안전한 읽기만으로 핵심 관측 불가, broad grant/원문 접근이 필요하다는 결론, 실행 모드 충돌이면 해당 분기 보류 후 coordinator 재설계 |
| WP-2 | intermediate | 고정 계약으로 제한된 host 관측기·설치 경로 구현; fake 명령/임시 FS로 검증 가능 | medium | 원자적 읽기·권한·명령 범위 테스트 반복 실패, 임의 root 명령/새 privileged daemon이 필요하면 hard로 재분류 |
| WP-3A | intermediate | WP-1에서 드러난 두 additive migration과 최소 실행 기록을 먼저 구현; 권한·fencing을 격리 DB로 검증 | medium | 실제 call graph가 중첩 기록을 만들거나 기존 수집 결과를 변경해야 하면 해당 분기 중단 후 재설계 |
| WP-3B | intermediate | 검증된 DB 관측 기반과 동결한 DTO·상태표를 HTTP API로 연결 | medium | RLS/권한 또는 publication 의미가 계약과 다름, 역할 확대/새 broker 호출이 필요하면 중단하고 WP-1 갱신 |
| WP-4 | intermediate | 기존 Owner 대시보드에 독립 상태 표시와 poll lifecycle 연결; DOM/브라우저 검증 가능 | high | status가 chart 로딩/선택에 결합, Owner 오류 경로에서 노출 불가, 동일 회귀 두 번 실패 |
| WP-5 | intermediate | 통합 실패 주입·브라우저 회귀·근거 다이어그램/운영 문서 완결 | medium | 격리 환경에서 현상 재현 불가, 필수 DB 테스트 skip, 실제 운영 변경 없이는 검증 불가 |
| WP-6 | intermediate | 구현과 검증 증거에 대한 독립 코드 리뷰; 변경량보다 권한/상태 의미의 일치 검토가 중심 | high | 계약과 구현 사이 구조적 충돌, 권한 누출, 근거 없는 healthy, QA 증거 누락이면 수용 거부 |

## Execution graph

| Package | Wave | Complexity | Objective | Owned scope | Depends on | Worker selection | Deliverable | Verification |
|---|---:|---|---|---|---|---|---|---|
| WP-1 | 1 | hard | 관측 source와 권한·schema·판정 계약 동결 | 계약 문서, status schema/catalog, 합성 fixture | 없음 | Codex `gpt-6-astra`, `xhigh` — 아키텍처 판단 규칙 적용 | source/권한 매핑, 정확한 수정 파일 지도, 기대 상태 fixture | 모든 대상의 file:line 근거와 경계값 표를 coordinator가 직접 확인 |
| WP-2 | 2 | intermediate | 호스트 관측과 immutable 설치 코드 | `scripts/ops/*service-status*`, 전용 systemd/Compose overlay, release 연결부 | WP-1 | Codex `gpt-6-luna`, `max` | bounded collector, atomic snapshot, installer, fake-host tests | allowlist·timeout·권한·원자성·release 일치 테스트 |
| WP-3A | 2 | intermediate | DB 관측 기반과 수집 실행 기록 | 지정 migrations, collectors 계측, SQL 권한/계측 테스트 | WP-1 | Codex `gpt-6-luna`, `max` | 제한된 stream read 함수, fenced research 실행 기록 | 격리 DB 권한·부작용 없음·실행 결과 보존·heartbeat/fencing |
| WP-3B | 3 | intermediate | Owner status API·업무 집계 | Rust API/repository, OpenAPI/생성 타입, API 테스트 | WP-1, WP-3A | Codex `gpt-6-luna`, `max` | 인증된 read model과 DTO, 상태 계산, API/DB 테스트 | 고정 clock 판정·Owner/Member·partial failure·OpenAPI 일치 |
| WP-4 | 3/4 | intermediate | 요약·서비스 상세 UI | Web 상태 컴포넌트/client/번역/route 연결과 unit tests | 단위 검증: 동결 계약·공식 생성 타입, 최종 수용: WP-3B | Codex `gpt-6-luna`, `max` | Owner 요약/상세, 재조회와 오류/접근성 처리 | Web unit/typecheck/lint 및 UI 상태 fixture |
| WP-5 | 5 | intermediate | 전체 경로 회귀와 증거 완결 | E2E/harness, QA 보고서, runbook, `.puml`/PNG | 코드 준비: WP-4 단위 검증, 실행·수용: WP-2/3A/3B/4 | Codex `gpt-6-luna`, `max` | 통합 QA, 스크린샷, 운영 검증/복구 절차, 다이어그램 | provider-free 실제 API/DB/Web 경로, 전체 요구 검증 결과 |
| WP-6 | 6 | intermediate | 독립 수용 리뷰 | 리뷰 문서만, 나머지 read-only | WP-5 | Codex `gpt-6.1-sol`, `high` | ACCEPT 또는 근거 있는 findings | frozen diff·계약·QA 증거 대조, coordinator 최종 수용 |

WP-1 조사에서 기존 API만으로는 stream 관측과 research 실행 기록이 부족함이 드러났다.
2026-10-08 coordinator 결정으로 원 WP-3를 WP-3A와 WP-3B로 순차 분리했다.
전체 범위는 그대로이며, 검증되지 않은 DB 관측 기반 위에 API를 한 번에 구현하는 결합을 줄인다.
계약의 `WP-3` 표기는 이 두 하위 패키지를 함께 가리킨다.
Wave 2의 WP-2와 WP-3A는 병렬 가능하다. 파일 소유권은 아래와 WP-1의 accepted 지도에 따른다.
WP-2는 Compose/installer를, WP-3B는 API runtime config를 소유한다. 같은 설정 파일을 동시에 고치지 않는다.
2026-10-08 통합 조정: 공개 wire schema와 124/14/60 fixture가 고정되어 있으므로 WP-4의
화면·controller·테스트 코드 준비는 WP-3B와 병행한다. WP-4는 생성 타입을 수정하거나
임시 가짜 API를 제품에 넣지 않는다. 타입이 아직 없으면 동결 endpoint 경로의 타입 연결을
준비하고 생성 결과와의 일치는 후속 검증한다. 이 준비 단계에서는 test/build/typecheck/lint를
실행하지 않고 미검증 보고서를 회수한다. 2026-10-08 후속 조정으로 시간 경계 계약 수용과
공식 OpenAPI 생성·타입 검사 통과 후 WP-4의 단위·타입·lint 검증을 먼저 배정한다.
이 동안 API/G1 worker는 코드 준비만 수행하며 Rust 검증을 실행하지 않는다.
WP-4 최종 수용과 실제 연결 검증은 WP-3B 수용 이후다. 무거운 작업은 순차이며 최종
의존성과 수용 기준은 유지한다. WP-5 중 제품 결함이 나오면 QA를 멈추고 원 작성자에게
파일 소유권을 돌려 수정한 다음 영향받은 검증만 다시 실행한다.
WP-4 단위 검증 수용 후에는 WP-5의 독립 E2E/fixture/QA example/runbook 코드만 먼저
준비할 수 있다. 이 동안 기존 E2E·다이어그램은 읽기 전용이며 실행 검증은 하지 않는다.
API/G1 작성자가 idle이면 해당 명시적 API test target만 독점 슬롯에서 컴파일·검증할 수
있다. 병행 준비 중인 QA example은 그 명령에 포함하지 않는다. QA example 빌드·실제
연결 검증은 해당 작성자도 idle인 이후다. 전체 기능 수용은 기존 API·브라우저·독립 리뷰
완료 조건을 그대로 따른다.

모델은 실행 전 현재 Paseo 가용성·profile notes·provider 지침으로 확인한다. 지원되지 않으면 자동 대체하지 않는다.
WP-1만 아키텍처 작업에 지정된 모델 규칙을 적용한다. 명세가 고정된 구현은 Luna max에서 시작한다.
같은 수정 두 번 실패·반복 정체·맥락 이탈·검증 반복 실패 시 모델을 한 단계씩 올린다.
모델과 effort를 한 번에 모두 올리지 않는다. `max` effort가 다른 모델에서 지원되지 않으면
모델 전환과 effort 조정 여부를 coordinator가 별개로 결정하고 기록한다. `ultra`/자동 fan-out은 사용하지 않는다.

2026-10-08 잔여 fixture 대조: API 검증에서 남은 `process_start_age_30`,
`daily_next_run_missing`, `daily_active`, `etf_blocked`만 별도
`gpt-6.1-sol/high` 읽기 전용 검토에 배정한다. 기존 계약·호스트 mapper를 근거로
oracle와 구현 중 어느 쪽이 잘못됐는지 판단하며, API 작성자가 실행 중인 동안
동결 입력은 수정하지 않는다. 이 검토는 WP-6 최종 독립 수용을 대체하지 않는다.
독립 대조와 coordinator 확인에서 네 사례 모두 기존 계약과 다른 fixture로 판명됐다.
API worker를 실제 idle로 전환한 뒤 시작 유예 deadline·누락 next-run·차단 Curated 날짜와
진행 중 실행의 완료/exit 필드만 보정했다. 124개 ID, 상태·이유 코드, 공개 형태와 기존
명세는 유지됐고 두 validator가 통과했다. 11개 입력의 hash를 다시 동결한 뒤 API를
재개했으며, 이 fixture 검사는 실제 제품 테스트 통과로 계산하지 않는다.
WP-5는 단일 heavy slot에서 모드별 Next standalone을 먼저 빌드한 뒤 테스트 서버를
순차 실행하도록 준비했다. 실제 실행과 PNG 생성은 API 수용·slot 인계 이후다.

## Worker briefs

각 assignment는 이 절의 **공통 실행 문구 + 해당 WP brief + accepted 계약/fixture 경로**를 함께 전달한다.
다른 대화 이력을 전제로 하지 않는다.

### 공통 실행 문구

> Do not use native subagent, Task, Agent, team, or delegation features. Complete this assignment directly and report if it needs further decomposition.

- target cwd는 모든 패키지에서 `/data/worktrees/3puw275b/itchy-bulldog`이다.
- 이 문서와 적용 `AGENTS.md`를 읽고, 적힌 소유 범위 밖 변경은 coordinator에게 필요성만 보고한다.
- 운영 명령·credential·provider 요청·기존 서비스 재시작은 금지다. 실제 데이터/계정 ID를 fixture로 복사하지 않는다.
- 모순·필수 입력 누락은 추정으로 메우지 않는다. 해당 분기를 보류하고 정확한 필요 입력을 보고한다.
- 새 npm/Rust 의존성이나 workspace member 추가는 기본 범위가 아니다. 기존 수단으로 불가능하면 근거를 제시한다.
- 최종 보고에는 반드시 (1) 변경 파일/라인 (2) 명세 편차/이유 (3) 명령/결과와 skipped 여부
  (4) 미해결/후속 (5) 찾지 못함/미검증을 적고, 빈 항목도 `없음`이라고 명시한다.
- 테스트 전후 포트·프로세스·임시 컨테이너 ownership을 기록하고 자신이 만든 것만 정리한다.
- 저장소 지침에 따라 Mem0를 사용하되 자기 요청의 정확한 hook envelope만 사용한다. native agent를 만들지 않는다.

### WP-1 — 관측과 상태 계약 확정

- **cwd / 분류:** `/data/worktrees/3puw275b/itchy-bulldog`; `hard`, confidence `high`.
  근거는 host/DB/권한/시간 의미의 결합이며, 접근 권한 확대나 자료 없는 판정이 필요하면 재설계 신호다.
- **목표/known facts:** 상시 서비스만 정상이어도 수집이 멈출 수 있다. 기존 admin worker 조회는
  RUNNING job에서만 파생된다. research daemon과 daily one-shot은 동시에 돌리는 경로가 아니다.
  Owner V2는 EOD와 WS를 같은 프로세스에서 수행하지만 한 기능의 실패가 다른 기능의 생존을 증명하지 않는다.
- **소유:** 신규 `docs/contracts/service-status-v1.md`, `configs/operations/service-status.schema.json`,
  `configs/operations/service-status-catalog.json`, `tests/fixtures/service-status/`.
  제품 코드·운영 상태 변경 금지. 계약 fixture에는 raw host observation과 API expected projection을 구분한다.
- **입력:** 본 계획, Compose/overlays, API runtime/ops repositories, role migrations, queue/stream/calendar
  구현, daily/backup/TLS installer, Web Owner 진입점. 코드 전수 탐색은 이 WP가 수행한다.
- **산출물:** 모든 대상의 안전한 source와 file:line/role/TTL 지도, 부족한 관측 목록과 최소 보완,
  source 접근 실패별 결과, host→API→UI schema, expected mode 출처, 상태 우선순위/판정표,
  정확한 WP-2/3/4 수정 파일 지도와 테스트 명령. migration이 필요하면 목적/권한/검증을 먼저 제안한다.
- **검증:** 제공된 service matrix의 전 항목에 source 또는 명시적 gap이 있어야 한다.
  active/idle/closed/no-demand/failed/stale/missing/disabled 사례를 고정 시각 fixture로 작성한다.
  source 미확정인 필수 관측을 그냥 unknown으로 영구 제외해 완료 처리하지 않는다.
- **보고:** 공통 5항목과 coordinator 결정이 필요한 항목을 명시한다. 이 계약이 accepted되기 전 후속 WP 금지.

### WP-2 — 호스트 수집기와 설치 경로

- **cwd / 분류:** `/data/worktrees/3puw275b/itchy-bulldog`; `intermediate`, confidence `medium`.
  고정 계약의 구현이고 fake-host 검증이 가능하다. root 명령 범위/FS 권한/원자성의 반복 실패는 상향 신호다.
- **목표/known facts:** Web/API에는 Docker/systemd 권한이 없다. 수집기는 원문 로그나 provider 응답을 읽지 않는다.
  고정 service inventory와 기대 실행 정책이 필요하며 commit별 daily unit을 임의로 가장 최근 이름에서 고르지 않는다.
- **소유:** 신규 `scripts/ops/collect-service-status.py`, `scripts/ops/install-service-status.py`,
  `scripts/ops/test_service_status.py`, `scripts/ops/test_install_service_status.py`,
  `deploy/systemd/lagrange-service-status.service`, `.timer`, `deploy/compose/compose.service-status.yml`.
  WP-1 지도에서 확정한 `scripts/ops/compose-release.sh`, `scripts/ops/validate-production-config.sh`,
  `scripts/ops/lib/`의 필요한 release 연결부와 대응 ops tests만 추가 허용한다.
  Rust/Web/schema/공유 fixture/다이어그램은 수정하지 않는다.
- **입력/의존:** accepted WP-1 계약·fixture·파일 지도.
- **수용 보완 (2026-10-08):** private receipt는 `policy.json`으로 확정한다.
  timer의 `Unit` 속성을 service에 요구하지 않는다. 두 API refresh 경로에서도 설치된 mount와
  project binding을 보존한다. daily timer의 비활성/다음 실행 부재는 관측 결과로 처리하고,
  설치·refresh의 무결성 검사와 분리한다. daemon 시작 시 중복 daily 방지 검사는 유지한다.
  설치 실패 시 observer 자신의 unit 파일과 이전 실행/enable 상태만 복원하고 rollback 실패를 구분한다.
  실제 systemd처럼 service/timer별 속성 부재를 검증한다. `kis-read-compose-self-test.sh`의
  synthetic release fixture도 WP-2 소유로 추가한다. WS refresh는 실제 mode parser로 검사하고,
  기존 REST intraday flag의 거부를 보존한다. 테스트를 위해 REST 허용이나 parser 우회를 추가하지 않는다.
- **산출물:** 고정 safe-field 수집, 명령별 timeout과 전체 deadline/중복 실행 방지,
  root-controlled atomic snapshot, 최소 권한 디렉터리, immutable release 기반 installer와 optional RO mount.
  installer는 기본 plan, 명시적 preflight/check/apply를 구분한다. 기능 코드 작성은 실제 apply 권한이 아니다.
  미설치 상태에서도 기존 release/app 동작을 보존하고, 설치 실패를 정상 관측으로 쓰지 않는다.
- **검증:** `python3 -m unittest discover -s scripts/ops -p 'test*service_status*.py'`;
  수정한 shell은 `bash -n`; 대응 기존 ops static/self-test를 WP-1 확정 명령으로 수행한다.
  timeout/명령 부재/호스트 재부팅/컨테이너 부재/old unit/disabled/oversize/미래 시각/symlink/부분 쓰기와
  새 파일 rename 이후 API reader가 새 snapshot을 읽는 실제 임시 디렉터리 테스트를 포함한다.
  금지 명령·raw env/log와 secret sentinel이 출력/파일에 없음을 검증한다.
- **보고:** 공통 5항목. 운영 설치·실제 root/systemd 실행은 미검증으로 명시하고 source/fixture 검증과 구분한다.

### WP-3A — DB 관측 기반과 최소 실행 기록

- **cwd / 분류:** `/data/worktrees/3puw275b/itchy-bulldog`; `intermediate`, confidence `medium`.
  accepted WP-1의 고정 SQL/권한/계측 계약을 구현한다. 실제 call graph가 다르거나 관측 실패가
  수집 결과를 바꾸게 되면 추정으로 진행하지 않고 해당 지점을 보고한다.
- **목표/known facts:** 기존 stream delivery 함수는 페이지 lease가 있어야 하므로 status 조회에
  재사용하면 안 된다. research 로그와 daily 상태 파일은 신뢰할 수 있는 실행 시각 API가 아니다.
  읽기 전용 stream aggregate와 실제 EOD 시도 경계의 최소 기록을 먼저 구현한다.
- **소유:** `migrations/0058_service_status_reads.{up,down}.sql`,
  `migrations/0059_research_execution_status.{up,down}.sql` (번호는 실행 직전 재확인);
  `data-pipelines/collectors/src/service_status.rs`, `src/worker.rs`, `src/lib.rs`,
  `data-pipelines/collectors/tests/service_status.rs`;
  신규 `crates/api-server/tests/service_status_reads.rs`,
  `crates/api-server/tests/service_status_reads_support/mod.rs`.
  HTTP/API 제품 소스, OpenAPI, ops/Compose/Web, 공유 fixture와 기존 migration은 읽기 전용이다.
- **입력/의존:** accepted WP-1 §7의 G1/G2, schema/fixture/source map과 정확한 top-level 실행 경계.
- **수용 보완 (2026-10-08):** 실제 새 시도는 90초를 초과해 오래된 RUNNING 기록만
  새 fence로 인수할 수 있다. 이전 완료/실패 시각은 만들지 않고 기존 성공 근거는 보존한다.
  0059 down은 exclusive lock 뒤에 빈 테이블인지 확인하여 concurrent 첫 INSERT를 보호한다.
  기존 작업자의 동일 SQL/권한 검증 실패가 반복되어, 인계 후 보완은 `gpt-6.1-sol`, `max`로
  모델만 한 단계 올린다. 0058의 정확한 session proof ID 일치와 실제 DB 부정 사례도 보완한다.
- **산출물:** session/actor/Owner/권리 검증을 유지하는 safe stream function;
  app의 private table SELECT/DML 금지 보존; research_writer만 쓰는 singleton 상태;
  heartbeat/조건부 완료를 갖는 attempt fence와 기존 pipeline 결과를 보존하는 계측.
  calendar-only/healthcheck/Raw-only/자식 recovery 경로에 DB 계측을 추가하지 않는다.
- **검증:** 격리 DB를 실제 설정하고 `CARGO_BUILD_JOBS=2 cargo test -p api-server --test service_status_reads -- --test-threads=1`,
  `CARGO_BUILD_JOBS=2 cargo test -p collectors --test service_status -- --test-threads=1`.
  app/admin/worker/research_writer 권한, 위조 actor/session, 다른 Owner의 데이터, READ ONLY 실행,
  stream lineage/TTL 경계, concurrent fence, start/heartbeat/finish 실패, Raw-only/NO_WORK/성공/실패,
  기존 수집 결과와 호출 수 보존을 검증한다. 필수 DB 테스트의 skip은 통과가 아니다.
- **보고:** 공통 5항목과 계측 실패의 관측 가능 한계, 실제 DB role/테스트 실행 수.
  coordinator가 이 기반을 수용한 뒤 WP-3B를 시작한다.

### WP-3B — Owner 상태 API와 업무 집계

- **cwd / 분류:** `/data/worktrees/3puw275b/itchy-bulldog`; `intermediate`, confidence `medium`.
  동결된 source/권한 매핑을 구현한다. 접근할 수 없는 table이나 다른 freshness 의미 발견 시 해당 분기를 멈춘다.
- **목표/known facts:** `/api/v1/admin/service-status`를 추가한다. 기존 worker 목록만으로 모든 worker의
  생존을 판정할 수 없으며 process와 work 상태는 각각의 근거로 계산해야 한다.
- **소유:** 신규 `crates/api-server/src/http/service_status.rs`, `crates/api-server/src/repos/service_status.rs`,
  필요한 새 내부 status 모듈; 해당 module 선언과 route/state/runtime/config 연결부;
  `crates/api-server/src/contract.rs`; `apps/api-server/scripts/openapi-spec.mjs`와 필요한 contract source,
  `apps/api-server/openapi.json`, `apps/api-server/generated/openapi.ts`;
  신규 `crates/api-server/tests/service_status.rs` 및 기존 HTTP/admin/OpenAPI 회귀의 필요한 추가.
  migration과 producer 계측은 WP-3A 소유이며 이 패키지에서는 수정하지 않는다.
  ops scripts/Compose/Web/공유 fixture/다이어그램은 수정하지 않는다.
- **입력/의존:** accepted WP-1의 schema, expected projection, SQL/source map, 권한과 audit 정책,
  accepted WP-3A의 migration/관측 기반과 role 테스트 결과.
- **산출물:** Owner 인증, 크기 제한 snapshot parser, bounded DB aggregate와 시간 판정,
  개별 unknown을 보존하는 부분 응답, no-store 계약, WP-3A의 제한된 읽기 함수 연결.
  요청 경로에서 subprocess·provider 호출·구독/업무 mutation을 만들지 않는다.
  사용자가 여는 status 페이지 수에 따라 무제한 쿼리/파일 읽기/감사 쓰기가 증가하지 않도록 계약의 부하 상한을 지킨다.
- **검증:** `CARGO_BUILD_JOBS=2 cargo test -p api-server --test service_status`,
  `CARGO_BUILD_JOBS=2 cargo test -p api-server --test openapi_contract`,
  `npm run openapi:check --workspace=@lagrange/api-server` 및 관련 HTTP/admin 회귀.
  DB 검증은 WP-1이 확인한 격리 Postgres harness로 실제 수행한다.
  `DATABASE_URL` 부재로 조용히 skip된 테스트는 통과로 인정하지 않는다.
  Owner/Member/비로그인, 타 사용자 데이터 배제, query timeout, schema drift, stale/future observation,
  idle worker와 failed/recovered job, missing calendar, daily mode를 각각 검증한다.
- **보고:** 공통 5항목과 DB fixture 적용 migration/role, 실제 실행 수·skip 수.

### WP-4 — 대시보드 요약과 상세 표시

- **cwd / 분류:** `/data/worktrees/3puw275b/itchy-bulldog`; `intermediate`, confidence `high`.
  기존 Owner 화면의 독립 표시 기능이다. chart state와 결합되거나 동일 회귀가 반복되면 상향 신호다.
- **목표/known facts:** 현재 chart는 catalog의 첫 번째 full-width widget이다.
  운영 패널의 조회 실패가 chart나 snapshot 로딩을 중단시키지 않아야 한다.
- **소유:** 신규 `apps/web/components/service-status/`, `apps/web/lib/operations/service-status-*.ts`,
  `apps/web/lib/i18n/dictionaries/service-status.ts`, `apps/web/tests/service-status*.test.ts*`;
  신규 `apps/web/tests/support/render-async-page.tsx`; 기존 product `stock-beta/page.tsx`와
  `stock-beta-intraday-page-seam.test.tsx`, `owner-beta-equity-signals-surface.test.tsx`만 수정한다.
  두 기존 테스트는 async SSR 완료를 기다리도록 바꾸되 모든 기존 검증을 유지한다.
  정확한 파일은 계약 §8에 고정한다. dashboard/terminal/차트/market-stream 소스는 읽기 전용이다.
  Rust/OpenAPI 생성 파일/ops/E2E/다이어그램은 수정하지 않는다.
- **입력/의존:** 준비는 WP-1 동결 계약/fixture와 accepted WP-2의 missing/stale host 동작으로 시작한다.
  단위·타입·lint 검증은 동결 계약과 공식 생성 타입 확인 후 독점 검증 슬롯에서 수행한다.
  최종 수용은 accepted WP-3B를 기다린다. 준비 보고서는 완료 수용이 아니다.
- **산출물:** 작은 요약/펼침 상세, 서비스 역할/상태/관측 시각/업무 근거, ko/en copy,
  accessible 상태·오류·포커스, 단일 기본 30초 조회와 유효시간에 맞춘 조기 재조회,
  최소 5초 간격·Retry-After, hidden/offline/unmount/late response 처리.
  마지막 값의 stale 표시와 Owner 인증 오류를 일반 네트워크 오류와 구분한다.
- **검증:** 먼저 `apps/web/node_modules/next/dist/docs/`에서 실제 package 해석 경로의
  server/client·data fetching·route 가이드를 확인한다.
  `npm test --workspace=@lagrange/web -- service-status`,
  `npm run typecheck --workspace=@lagrange/web`, `npm run lint --workspace=@lagrange/web`;
  Owner route/stock-beta dashboard 기존 unit 회귀도 실행한다.
  패널 접힘과 펼침의 요청 수 동일, background pause/resume, 늦은 응답 무시,
  logout/권한 거부 시 정보 제거, 기존 시세 구독 수 증가 없음, unavailable chart에서도 패널 표시를 검증한다.
- **보고:** 공통 5항목과 UI 연결 위치/상태별 캡처 준비 사항.

### WP-5 — 통합 QA와 운영 문서·다이어그램

- **cwd / 분류:** `/data/worktrees/3puw275b/itchy-bulldog`; `intermediate`, confidence `medium`.
  검증 환경·실패 주입·문서 근거를 연결한다. 필수 테스트 skip이나 재현 불가를 발견하면 완료 처리하지 않는다.
- **목표/known facts:** Docker healthy는 데이터 정상의 충분조건이 아니다.
  daily는 예약 사이에 종료되어 있을 수 있고, 현재 chart-first 배치를 보존해야 한다.
- **소유:** 신규 `apps/web/tests/e2e/service-status.spec.ts`,
  `apps/web/tests/e2e/service-status-market-stream.spec.ts`, 전용 E2E support;
  필요한 `stock-beta.spec.ts`의 관련 회귀; `scripts/qa/service-status-*`;
  `docs/reviews/2026-10-08-service-status-qa.md`, `docs/runbooks/service-status.md`;
  `docs/diagrams/component_architecture.puml`, `runtime_deployment.puml`과 대응 PNG.
  제품 소스는 수정하지 않는다. 결함은 재현 증거와 함께 원 작성자에게 반환한다.
- **입력/의존:** WP-1~4 결과, frozen 구현 diff, 명령·테스트·환경 manifest.
- **산출물:** API fixture만 보는 UI 테스트와 실제 격리 DB/API + 임시 host snapshot 통합을 구분한 QA;
  설치/확인/업데이트/제거/원래 상태 복구 runbook; 설치 전후 검증에 필요한 safe-field 목록;
  새 실제 edge마다 file:line 근거를 갖춘 두 구조 그림.
- **검증:** 전체 Web unit, typecheck, lint, production Web build;
  새 E2E와 기존 `stock-beta.spec.ts` 및 `stock-beta-intraday.spec.ts`의 적용 가능한 회귀.
  Owner 권한 fixture로 핵심 새 경로를 실제 실행하며 off-mode skip을 핵심 기능 검증으로 대체하지 않는다.
  `npm run test:e2e --workspace=@lagrange/web -- service-status.spec.ts stock-beta.spec.ts stock-beta-intraday.spec.ts`
  의 BASE_URL/API fixture/mode/정확한 시작 명령을 보고서에 기록한다.
  2026-10-08 소스 대조: `intraday-quotes-mode.ts:11`의 현재 화면 경로는 `off | market_ws`다.
  기존 `stock-beta-intraday.spec.ts:18`은 REST `quote-demands`용이며 현재 활성 경로를 검증하지 않는다.
  기존 두 파일의 chart 회귀는 off 구성으로 수행하고 REST suite의 기존 skip은 그대로 공개한다.
  새 상태 기능의 Owner 검증과 별도로 `owner_only + market_ws` 구성에서 신규
  `service-status-market-stream.spec.ts`를 실제 실행하여 추가 lease/SSE 연결 없음과
  기존 시세/차트 동작을 확인한다. REST 제품 경로를 재활성화하거나 기존 테스트를 억지로 바꾸지 않는다.
  API/DB/ops 집중 검증을 통합 변경 후 한 번 확인하고, 성공 후 무의미하게 전체 suite를 반복하지 않는다.
- **필수 실패 주입:** healthy 프로세스+시세 단절, calendar와 window 각각 부재,
  disabled/missing daily timer, 유휴 worker, 장 종료/무수요, 오래된 EOD, 복구된 job 실패,
  host 수집기/DB/API 각각 timeout, partial/malformed/future/stale snapshot,
  권한 거부, API 단절 뒤 마지막 정상 badge 만료. 실제 provider 호출은 0이어야 한다.
- **브라우저:** 375×800, 1024×900, 1280×720, 1440×900; ko/en, 키보드, forced-colors,
  200% 실제 zoom 또는 대체 검증의 한계 명시. 차트 폭·선택/제스처·페이지 스크롤 회귀 없음.
- **다이어그램:** `docker run --rm -v "$PWD/docs/diagrams:/data" plantuml/plantuml -tpng /data/component_architecture.puml /data/runtime_deployment.puml`.
  로컬 이미지 사용 여부를 먼저 확인하고 content를 외부 렌더링 서비스로 보내지 않는다.
  source와 PNG를 같은 결과물로 제출한다.
- **보고:** 공통 5항목, 실제 passed/failed/skipped 수, screenshot/log 경로,
  시작한 프로세스/포트 정리, 실제 운영 설치·장애 탐지 시간은 미검증임을 명시한다.

### WP-6 — 독립 리뷰

- **cwd / 분류:** `/data/worktrees/3puw275b/itchy-bulldog`; `intermediate`, confidence `high`.
  계약/구현/증거를 독립적으로 판정한다. 거짓 정상·권한 누출·구조적 계약 충돌은 blocking 신호다.
- **목표/known facts:** Owner 전용 조회이며 프로세스와 실제 업무 상태를 분리해야 한다.
  UI가 정상으로 보이더라도 원본 관측 시각/권한/세션 기준이 틀리면 수용하지 않는다.
- **소유:** `docs/reviews/2026-10-08-service-status-review.md`만 작성한다.
  코드·테스트·fixture·plan 수정 및 전체 suite 재실행은 기본 범위가 아니다.
- **입력/의존:** accepted WP-1 계약, WP-2~5 산출물, coordinator가 고정한 commit/diff/hash 목록.
- **검토:** Owner 권한과 정보 범위, host root 경계, read-only 효과, query/audit/poll 부하,
  stale/부분 실패/세션 판정, 스케줄 실행 방식, 새 프로세스/포트 cleanup, 다이어그램 근거,
  tests가 구현 결함을 가리거나 skip으로 대체하지 않았는지 확인한다.
- **산출물/검증:** ACCEPT 또는 severity/file:line/reproduction/영향을 포함한 findings.
  시작/종료 diff identity를 비교하고 필요한 범위의 추가 검증만 수행한다.
  이미 관측된 결함 해결은 원 WP가 담당하며 수정 후 해당 finding을 다시 리뷰한다.
- **보고:** 공통 5항목과 blocking finding 수, 잔여 제한, coordinator 수용 가능 여부.

## Coordinator gates

2026-10-08 WP-1 수용 기록: 22개 항목과 안전한 source/권한 지도, root 정책 receipt와
선택적 디렉터리 RO mount, 0058/0059의 최소 DB 관측·계측, 2시간 EOD 표시 유예,
요청별 감사와 유한한 query/cache/poll 한도, §8의 파일 소유권을 수용했다.
원 WP-3는 3A/3B로 순차 분리했다. 기록되지 않은 시도를 탐지할 수 없다는 계측 한계를
명시했으며 마지막 기록과 현재 publication 최신성을 독립 판정한다.
Coordinator의 소스 대조에서 V2 EOD 상한은 100, stream 수요 상한은 30임을 확인하여
계약을 바로잡고 31/100개 집계와 101개 초과 검증 벡터를 추가했다.
기본·baseline migration 검사, 엄격한 JSON 파싱을 통과했다. 이는 제품 동작 검증이 아니며
각 후속 WP의 실제 검증과 독립 리뷰가 아직 필요하다. 운영 활성화 권한은 추가하지 않는다.

2026-10-08 WP-2/3A 수용 기록: 호스트 관측기 fake-host 39개와 ops 회귀, 실제 격리 DB의
권한/stream 2개·수집 기록 3개·기존 worker 회귀 37개가 통과했다. 90초 초과 시 새 실제
시도의 fence 인계, down과 첫 INSERT의 동시성 보호, 현재 stream proof ID 일치와 SQL
시간 경계를 대조했다. 현 corpus 124/14/60 검증과 whitespace가 통과했고 소유 자원은
정리됐다. 실제 운영 설치는 미수행이며, stream 정확한 TTL equality는 후속 고정 clock
API 판정 검증 대상이다. 이 기반을 읽기 전용으로 고정하고 WP-3B를 진행한다.

2026-10-08 추가 통합 게이트: API 대조 중 G1의 CLOSED 입력·포함/배타 집계 의미와 일부
fixture의 시간 경계가 명세에 맞지 않는 것을 확인했다. WP-3B 구현 반복을 중단하고 소유
자원 정리·초기 인계를 회수한다. 독립 `gpt-6.1-sol/high` 검토의 제안을 바탕으로 계약·독립
fixture만 먼저 보완한다. 실제 DB의 새 관측과 살아 있는 시세·거래정지 근거의 유효기간을
구분하며, 유효한 근거의 경계를 늘려 테스트에 맞추지 않는다. 124개 case ID와 공개 wire
형태는 유지한다. 변경 case와 이유를 검토·동결한 뒤 G1 SQL/실제 DB 증거, API 집계·cache와
시점별 판정, Web 검증 순서로 재개한다. 0058의 제한된 계약 보완과 해당 테스트만 새로
소유권을 배정하며, 기존 0059 fence/down 및 collector 구현의 수용은 유지한다. 운영 설치나
provider 권한은 추가되지 않는다.
API의 격리된 코드·테스트 보완 준비는 계약 보완과 병행하되 파일 소유권을 분리한다.
반복된 기본 projection 수정과 명세의 queue 경계를 fixture에 맞춰 지운 미수용 변경을
근거로 API 보완 모델만 `gpt-6.1-sol`로 한 단계 높이고 effort `max`는 유지한다.
새 API worker는 contract/SQL/OpenAPI/Web을 수정하지 않으며, 수용된 corpus와 G1 SQL
후속 인계를 받기 전에는 compile/test/runtime을 실행하지 않는다.

1. **실행 전**
   - 사용자 요청이 계획 실행까지 포함하는지 확인한다. 2026-10-08에 실행 요청을 받았다.
   - 실제 `$paseo-delegate` skill과 현재 profile/model 가용성·작업공간·parent/notification 경로를 확인한다.
   - git baseline/사용자 변경을 보존한다. instructions와 WP-1 입력의 경로/줄 번호 drift를 확인한다.
   - service scope·Owner 전용·읽기 전용·기존 차트 배치 유지에는 추가 사용자 결정이 필요 없다.
     새로운 제품 동작/권한 확대/운영 변경이 필요하면 구체적 변경안을 만든 뒤 해당 지점만 확인한다.
2. **WP-1 계약 수용**
   - 서비스 전 항목의 source·권한·expected policy·상태표와 필수 gap 해소를 직접 확인한다.
   - `research-worker` 실행 방식, idle worker, calendar/window 구분, ETF/V2 최신성 판정을 우선 검토한다.
   - schema/fixture/version과 정확한 수정 파일 소유권, migration 번호, 격리 DB 명령을 동결한다.
   - 해결되지 않은 필수 입력이 있으면 해당 후속 WP를 시작하지 않고 계약/graph를 갱신한다.
3. **Wave 2 통합**
   - host snapshot을 API parser에 넣는 contract 검증을 수행한다. 파일 rename/권한/TTL/disabled 정책을 확인한다.
   - 운영 Docker/systemd/DB/provider를 건드리지 않았는지 확인한다. API/ops가 공유 파일을 수정하면 순차로 재배정한다.
   - Rust와 Web의 무거운 build/test를 동시에 돌리지 않는다. 2026-10-08 실행 전 관측에서
     available RAM 약 2.6 GiB, swap 여유 0이었다. 이번 격리 검증은 `CARGO_BUILD_JOBS=1`,
     `CARGO_INCREMENTAL=0`, dev/test debug=0으로 낮춰 실행한다. 이는 운영 release build 정책과 별개다.
     사용자 systemd 임시 작업의 `MemoryMax=2G`, `MemorySwapMax=0`, `CPUWeight=10` 적용을
     실제 확인했다. 이 제한으로 Rust 검증을 감싸고, 동시에 다른 무거운 작업을 시작하지 않는다.
     I/O cgroup controller는 없어 적용을 주장하지 않으며 `ionice -c 3`를 사용할 수 있다.
     각 단계 직전에 자원을 다시 확인하고 부족하거나 제한에 걸리면 원인을 보고하고 재조정한다.
     운영 서비스나 다른 개발 프로세스를 중단해 공간을 확보하지 않는다.
4. **Wave 3~4 통합**
   - 화면 로딩과 status 조회가 독립적인지, 전체 inventory와 unknown/disabled 의미가 보존되는지 확인한다.
   - 화면 계약/Owner 부정 사례/실제 DB 경로/실제 Web 브라우저 검증과 다이어그램을 함께 수용한다.
   - required test의 skip을 통과로 계산하지 않는다. production 변경이나 provider 호출 없이 검증한다.
5. **최종 end-to-end 수용**
   - 고정 snapshot + 격리 DB + API + Web 전체 경로에서 정상/대기/장애/unknown을 확인한다.
     필요하면 WP-5 소유의 `crates/api-server/examples/service-status-browser-harness.rs`가
     기존 읽기 전용 `tests/common` Harness와 실제 API router를 재사용한다. Cargo 의존성이나
     공통 Harness를 변경하지 않고, UID 10001의 격리 컨테이너에 실제 고정 snapshot 경로를
     mount해 제품 파일 권한 검증을 유지한다. 이는 router 통합 증거이며 운영 main 기동 증거로
     표현하지 않는다. 기존 API/G1 작성자가 idle이고 컴파일 슬롯이 배정된 후에만 빌드한다.
   - API 단절·stale snapshot이 마지막 정상 표시를 무기한 유지하지 않는지 확인한다.
   - `git diff --check`, 적용 lint/typecheck/build/API/ops/브라우저 결과, 독립 리뷰를 검토한다.
   - 모든 worker의 실제 idle/terminal, pending permission 없음, 보고서 회수와 owned 프로세스 정리를 확인한다.
   - 완료 보고는 구현/QA 완료와 운영 설치 여부를 분리한다. main merge·운영 배포는 수행하지 않는다.
     추후 배포 요청 시 immutable release와 기존 자원 정책에 따라 별도 preflight/적용/관측을 진행한다.

모든 worker 시작·후속 요청·권한 대기·결과 회수·실패 복구는 실행 시점의 `$paseo-delegate`에 따른다.
이 문서는 MCP 명령을 재정의하지 않으며 native delegation으로 대체할 수 없다.
새 필수 요구나 구조 변경이 발견되면 coordinator가 이 계획의 소유권/graph/brief를 먼저 수정한 뒤 실행한다.
