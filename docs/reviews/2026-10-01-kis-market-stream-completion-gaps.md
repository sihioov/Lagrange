# KIS market stream R111 갭 및 runtime 계약 검토 — WP-1

작성일: 2026-10-01 KST. 기준 HEAD: `17d643e92bd4b5c257c9745cfc6ec188ef9f4d05`.
판정: **문서 검토 및 root 계약 채택 완료. WS3B 및 SOURCE_LOCAL_ACCEPTED는 미완료**.
코디네이터 채택(2026-10-01): 아래 WP-1 제안을 검토하고 C2 lock/idle 및 initializer 경계를
채택했다. 0055의 현재 배포 여부는 미확인이므로 편집하지 않는다. 아래 manifest의 0055
수정 행과 V12의 재개방 조건은 새 `0056_owner_market_stream_runtime.{up,down}.sql` 추가 및
전체56 migration 검증으로 대체한다. 나머지0001–0055는 보존하며 WP-3은 순차 실행 단위로 나눈다.
아래의 제안/채택 대기 표현은 작업자 원보고의 기록이며 이 채택과 갱신된 계획이 우선한다.
코드·runtime·DB·운영 실행 없이 소스와 인계를 대조했다. 이 문서의 제안은 구현 허가가 아니다.

## 증거의 구분과 보존

- **S — 현재 소스 사실:** 아래 `file:line`은 이 기준선의 구현 위치다. 존재하는 함수가
  있다는 사실과 실제 동작 인수는 다르다.
- **H — 채택된 과거 실행:** `docs/handoffs/2026-09-30-kis-market-stream.md:65`의
  original20–22/execution4,23–26/execution5 및 `:80`의 original1–19/pacing4.
  서로 다른 focused 실행이며 이번에 다시 실행하지 않았다. C1/C2/C3A의 채택 경계는
  `docs/superpowers/specs/2026-09-21-kis-market-stream-contract.md:697` 및 `:811`.
- **P — 제안 계약:** 같은 명세의 새 §13–14. root가 범위·회귀 의무를 채택해야 WP-3에 전달한다.
- **U — 미검증 runtime:** 새 runtime, 실제 인증 API/SSE/DOM, soak, 운영 설치·0055 적용,
  실제 KIS 프로토콜/30구독/당일 EOD는 이번 증거에 포함되지 않는다.

인계 `:126–127`의 producer/test SHA256은 각각
`9ebc67d81f8ea27a5d134cb454c00bfa7e56fc11f39af7a93c100f622ce13f62`,
`15db864f07de9c9f52a3868dd357f7cbc32e6cccd93f684d143743026a50d645`다.
이번 검토에서 두 소스 해시 일치를 다시 확인했다. private 보고서 해시 네 건을 root가 이미
확인했다는 배정 내용을 인계 근거로 받았으며, 이 작업자는 private 보고서/원장/실행기를
열거나 실행하지 않았다. 과거 DB cleanup·종료는 당시 사실이며 현재 상태로 소급하지 않는다
(`docs/handoffs/2026-09-30-kis-market-stream.md:82`).

## 유한 요구사항 → 소스 → 채택 증거 → 갭 → 담당 표

아래 표의 명세 절은 `docs/superpowers/specs/2026-09-21-kis-market-stream-contract.md`다.
총 16개 요구사항이며 범용 proof framework나 전체 로드맵 재계획을 추가하지 않는다.

| ID / 요구사항 | S: 현재 코드 위치 | H: 채택 범위 | 남은 갭 / P | 담당 WP |
|---|---|---|---|---|
| R1 §9 off/rest/WS/once 배타 선택 | `crates/job-queue/src/bin/owner-equity-v2-runner.rs:116`, `:777`, `:796`; `crates/kis-client/src/read_coordination_config.rs:50` | 기존 REST 경로 존재; 새 WS runner 인수 없음 | WS enum/config/새 daemon 연결 없음. off/rest 기본, once 무접근, EOD 병존 | WP-3 |
| R2 §5 anchor→lease→socket | `crates/kis-client/src/market_stream.rs:299`; `crates/job-queue/src/owner_equity_v2/market_stream.rs:1392` | C2 lifetime ownership, C1 producer fencing | connect가 lock과 네트워크를 한 함수에서 실행. DB lease를 사이에 넣을 안전한 affine owner 필요 | WP-3, C2 최소 재개방 |
| R3 TTL20/renew5/소유권 상실 | `crates/job-queue/src/owner_equity_v2/market_stream.rs:1565`, `:3555`; `crates/job-queue/src/owner_equity_v2/market_stream_producer.rs:296` | C3B stale-fence·실패 focused 경계 | renewal 호출 루프·lease 갱신 전달·DB 지연 중 정지 감독 없음 | WP-3 |
| R4 30 union/1초 수요/명령 pacing | `crates/job-queue/src/owner_equity_v2/market_stream.rs:1292`; `crates/job-queue/src/owner_equity_v2/market_stream_producer.rs:415`, `:473`, `:514` | original1–26 및 pacing4 채택, Complete/Deferred 인계 `:89` | 최신 demand 재조회 scheduler,0수요 close/grace,1/10 consumer 실제 상류 count 검증 필요. Deferred는 완료/권리 proof가 아님 | WP-3 |
| R5 취소 및 command/read 공존 | `crates/job-queue/src/owner_equity_v2/market_stream_producer.rs:585`, `:851`, `:867`; `crates/kis-client/src/market_stream.rs:1230`, `:1318`, `:1822` | C2 capability·cancellation + original20/22/26 | quiet `next_event` 동안 command를 시작할 idle 반환이 없음. timer로 read future를 취소하면 facade 소실. 안전한 bounded read + 단일 소켓 command barrier | WP-3, C2 최소 재개방 |
| R6 §7 250ms batch/부하 | `crates/job-queue/src/owner_equity_v2/market_stream_producer.rs:597`, `:652`, `:657`; `crates/job-queue/src/owner_equity_v2/market_stream.rs:2222` | original21 commit 실패, 기존 receipt/ordinal 검증 | facade가 건별 즉시 publish. 저장소 batch를 private30 latest/2×30 writer로 연결하고 write·heap·coalesced 계측 필요 | WP-3; 종단 soak WP-7 |
| R7 ACK 전 상태/무체결 | `crates/job-queue/src/owner_equity_v2/market_stream_producer.rs:878`; `crates/job-queue/src/owner_equity_v2/market_stream.rs:2353`, `:2365`, `:3603` | original20은 test-only AwaitingFirstTrade bridge로 캐시 생성(인계 `:102`) | `require_ack=false`여도 최종 validator가 ACKED 요구. bridge만 production으로 공개해도 CONNECTING/PENDING/missing proof를 표현 못함. 별도 private control transition 필요 | WP-3 |
| R8 durable gap/reconnect/status | `crates/job-queue/src/owner_equity_v2/market_stream.rs:1652`, `:1669`; `migrations/0055_owner_market_stream.up.sql:173`; `crates/kis-client/src/market_stream_state.rs:873` | fresh epoch 및 same-epoch 재구독 거절 original23; 시작 실패24–26 | producer에 gap_since/reason/session_has_gap 없음. monotone counter만으로 다음날 플래그 reset 불가. clean-only 새 facade, ambiguity STOPPED | WP-3; schema gate root |
| R9 snapshot/notification/미수신 행 | `crates/job-queue/src/owner_equity_v2/market_stream.rs:2411`, `:2444`, `:3240`; `migrations/0055_owner_market_stream.up.sql:498`, `:521` | C3A snapshot/RLS, original20 캐시 없는 행 제한 | cache INNER JOIN은 never-quoted 행을 반환하지 않음. app은 producer/ACK state SELECT 불가. <=30 scope-bound delivery read helper와 안전한 snapshot projection 필요 | WP-3 seam; WP-4 SSE |
| R10 권리 pin·day/window lineage | `migrations/0055_owner_market_stream.up.sql:24`, `:525`; `crates/job-queue/src/owner_equity_v2/market_stream.rs:366`, `:1632`, `:3563`; `crates/job-queue/src/owner_equity_v2/intraday.rs:1505` | C1 rights/date, C3 receipt/session 일치 | worker grant SELECT에 network pin 없음. StreamSessionProof 형식검사는 실제 calendar 검증 아님. 기존 resolver/window로 private resolved day, commit 재검증 필요 | WP-3; proof 공급 WP-6/9 |
| R11 §9 최초 초기화/reapply | `crates/kis-client/src/market_stream_state.rs:504`, `:517`, `:660`, `:759`, `:837` | test 도메인 생성/재열기 경계; production root/mount 인수 아님 | production open은 validate-only; production initializer 없음. test 초기화 공개 금지. root-only 전용 feature/bin + no-replace 생성 계약 | WP-3 binary; WP-6 installer |
| R12 §8 인증 schema2/SSE | `crates/api-server/src/http/mod.rs:343`; 명세 `:876`, `:961` | 기존 REST API만 현재 실행 접점 | lease/SSE/인증 재검증/nginx/LISTEN loss/resync 실제 구현·검증 미인수 | WP-4; WP-7 통합 |
| R13 §8/9 실제30행/한 탭 controller | `apps/web/lib/products/intraday-quotes-contracts.ts:3`, `:10`; 명세 `:986`, `:1006` | focused DB PASS는 화면 증거 아님 | 정상 pilot→29 admission, schema2 controller, lifecycle, REST polling0, DB/SSE/DOM 새 수신 대조 | WP-5; WP-7/9 |
| R14 §7/11 shared collection/soak/회귀 | 명세 `:827`, `:1136`; 인계 `:165`, `:168` | original20–26는 둘로 나뉜 focused 실행(인계 `:75`) | 30종목1/10탭 동일 상류30, slow DB/consumer,notify loss,heap8MiB,p95≤2s,>=30분 통합/EOD 회귀 없음 | WP-3 local boundary; WP-7 종단 |
| R15 §9 immutable ops/grant/도면 | 명세 `:1025`, `:1059`; 인계 `:167`, `:169` | 과거 설치 기록은 현재 증거 아님 | initializer 포장·root/UID/mount·grant helper·rollback·exact image/env 및 evidence 도면 | WP-6; root 도면; WP-8 설치 |
| R16 §12 G1–G5 및 final acceptance | 명세 `:1158`; `docs/superpowers/plans/2026-10-01-kis-market-stream-completion.md:25`, `:98` | checkpoint/로컬 fixtures만 | 최신 허용면/한도/권리/proof/운영 입력, 실제30 ACK·pilot 두 새 수신·동일날 EOD는 별도. Wave1 권한으로 실행 금지 | WP-2 준비; root 결정; WP-9 인수 |

## 기존 불변식 안에서 가능한 것과 재개방이 필요한 것

**유지:** 저장소의 bounded batch, exact ordinal no-op, unknown commit 한 번 재조회,
C1 producer→rights→owner mutex→admission/session/demand→subscription→cache 잠금,
실제 Prepared→DB pending→consuming send→실제 ACK→DB ACK 순서, positive refcount ACK
보존, private proofs 및 cancellation terminal은 그대로 사용한다. 출처는
`crates/job-queue/src/owner_equity_v2/market_stream.rs:2251`, `:2284`, `:2334`, `:3261` 및
`docs/superpowers/specs/2026-09-21-kis-market-stream-contract.md:765`다.

**단순 연결로 해결 불가:** `read_and_publish`를 timer/select로 취소했다가 다시 부르면
`take_session`/terminal 규칙을 깨뜨린다. read만 별도 소켓 task로 떼면 command와 소유권이
겹친다. read 반환 후 매번 sleep250ms 하면 받지 못한 틱을 병합한 것이 아니며 hot symbol
에서 stale backlog를 만든다. ACK 없이 arbitrary status context를 만드는 public factory도
허용하지 않는다. 이 이유로 §13의 private latest writer와 소켓 단일 owner를 선택했다.

**C2 최소 재개방안:** `crates/kis-client/src/market_stream.rs:299` 연결부를 affine
`reserve_connection`/consuming async `connect`로 나누고, `:1318`, `:1562`, `:2089`,
`:2202`, `:2278`에 내부 deadline idle 반환을 추가한다. source의 frame-buffer 구현을
재사용한다. `send_prepared`/capability/getter/private nonce/wire/approval/예산/상태 schema는
그대로다. 외부 취소는 재개 가능 idle이 아니다. ACK 대기 중 C2가 보류한 receipt를 공개
callback으로 내보내지 않는다. 정상 부하 p95 및 ACK 지연/overflow/PIPELINE_LAG를 실제로
측정해야 하며, 기준에 못 미치면 WP-3을 반환한다. 큐 확대·C2 완화는 후속 자동 권한이 아니다.

**0055 최소 재개방안:** producer의 reason/status_at/gap_since/session_has_gap4필드,
그 제약과 worker 컬럼 grant, worker grant SELECT3컬럼, app-only delivery read helper1개만
제안한다. app에 producer/구독 테이블을 직접 공개하지 않는다. 기존 status setter를
quote 권한 우회로 바꾸지 않는다. 명세 §13.3의 상태 transition과 snapshot read가 이 확장을
사용한다. source tree의 0055는 `migrations/0055_owner_market_stream.up.sql`과
`migrations/0055_owner_market_stream.down.sql` **한 쌍뿐**이다. 인계
`docs/handoffs/2026-09-30-kis-market-stream.md:57`은 당시 미적용만 말한다. 현재 운영 DB는
보지 않았다. root가 unpublished를 입증하지 못하면0055 편집을 허가하지 말고 append-only
migration 번호/범위를 먼저 채택해야 한다. 기존0001–0054 또는 철회한 audit0055는 변경 금지다.

**initializer 최소 재개방안:** state validation/runtime replacement 경로를 바꾸지 않고
feature-gated production provisioning module/bin만 추가한다. fresh layout을 확실히 하기 위해
root initializer가 두 leaf와 final anchors를 같은 호출에서 새로 만든다. WP-6 shell은
이를 미리 만들거나 state JSON을 작성하지 않는다. exact fixed-path parent mount,root-only
생성,0600 UID10001 state,no-replace/fsync,reapply validate-only를 §14에서 확정했다.
이는 source/도구 계약 제안이며 현재 보호 경로를 읽거나 초기화하지 않았다.

## WP-3 제안 수정 manifest

경로는 저장소 root 기준이며 다음 표 밖 수정은 root에 반환한다. 아래 허용은 **채택 조건부**다.
기존 test 파일은 assertion 삭제/완화 없이 영향 회귀·새 fixture 지원에만 수정한다.

| 파일 | 정확한 변경 목적 |
|---|---|
| `crates/job-queue/src/owner_equity_v2/market_stream_runtime.rs` (신규) | 공개 safe runtime/config/exit,grant/day resolver,수요·lease·shutdown supervisor,새 epoch orchestration. 일반 proof framework 금지 |
| `crates/job-queue/src/owner_equity_v2/market_stream_runtime_tests.rs` (신규) | 아래 V1–V10 actual-role/loopback runtime 검증; library opt-in 연결 |
| `crates/job-queue/src/owner_equity_v2/market_stream_producer.rs` | 기존 public focused facade 보존; private run_owned/latest/serial writer·safe status 연결; terminal 재개 금지 |
| `crates/job-queue/src/owner_equity_v2/market_stream.rs` | private runtime grant/status/retire; real lineage/window commit guard; 안전한 delivery snapshot·DTO; 기존 publication/ACK proof 공개 금지 |
| `crates/job-queue/src/owner_equity_v2/intraday.rs` | 기존 `validate_session_lineage`의 `pub(super)` 재사용에 필요한 최소 visibility만. REST proof/ledger semantics 변경 없음 |
| `crates/job-queue/src/owner_equity_v2.rs` | 명시적 safe runtime/DTO exports와 private test 모듈. wildcard proof export 금지 |
| `crates/job-queue/src/bin/owner-equity-v2-runner.rs` | enum 선택,daemon WS 연결,once/off에서 WS factory0,종료 join 및 기존 EOD branch 보존; 자체 focused tests |
| `crates/job-queue/src/owner_equity_v2/market_stream_producer_tests.rs` | 새 경로 영향이 있는 original1–26/pacing 회귀. 기존 기대값 느슨화 금지 |
| `crates/job-queue/src/owner_equity_v2/market_stream_producer_test_support.rs` | task-owned synthetic runtime fixtures/counters,기존 DB authority/fallback 규칙 유지 |
| `crates/job-queue/src/owner_equity_v2/market_stream_c3a_tests.rs` | status/read helper/lineage/role·RLS·locking 회귀 |
| `crates/job-queue/tests/owner_market_stream_boundary_support/mod.rs` | 전체 migration/실제 역할 fixture의 새 schema 지원만 |
| `crates/job-queue/tests/owner_market_stream_boundary.rs` | default public API closure/새 safe DTO 검증만; 이것만 PASS로 내부 DB suite 대체 금지 |
| `crates/kis-client/src/read_coordination_config.rs` | 기존 parser signature 보존; 별도 Rest/MarketWs transport enum/parser |
| `crates/kis-client/src/market_stream.rs` | 위 C2 lock-before-connect/next_event_until hunks와 private read deadline plumbing만 |
| `crates/kis-client/src/market_stream_state.rs` | private fresh-production-layout 생성/검증/no-replace 설치 지원; 기존 schema·runtime repair·REST 상태 변경 없음 |
| `crates/kis-client/src/market_stream_provisioning.rs` (신규) | §14 feature-gated 두 library entry; fresh-layout/state helpers는 private |
| `crates/kis-client/src/bin/kis-market-stream-state.rs` (신규) | root-only initialize-new/read-only validate-existing CLI; 고정 경로·정형 출력 |
| `crates/kis-client/src/lib.rs` | transport enum/opaque connection owner 및 feature-gated provisioning module export |
| `crates/kis-client/tests/market_stream_transport.rs` | 기존 C2+bounded idle/lock sequence/취소 영향 회귀 |
| `crates/kis-client/tests/market_stream_domain.rs` | 기존 state/anchor/uncertainty 회귀 |
| `crates/kis-client/tests/market_stream_provisioning.rs` (신규) | 실제 library/bin fixture; default feature 차단; root/mount 인수는 WP-6로 분리 |
| `crates/kis-client/Cargo.toml` | provisioning feature 및 required-features bin 명시; test-support runtime 의존 금지 |
| `crates/job-queue/Cargo.toml`, `Cargo.lock` | 기존 의존으로 우선 구현; 필요 manifest/feature 연결만. 신규 dependency는 이유를 root에 보고 |
| `migrations/0055_owner_market_stream.up.sql`, `.down.sql` | unpublished 확인 후 위 좁은 schema/read helper/grants; 그렇지 않으면 이 행 보류 |

`market_stream_wire.rs`, `market_stream_approval.rs`, 기존 REST coordination/ledger, API/Web,
ops/compose/nginx/runbooks 및 도면은 WP-3 수정 범위가 아니다. initializer 포장·호출·host/mount
계약은 WP-6, SSE mapping은 WP-4, 구조 도면/PNG 갱신은 root가 맡는다. source 구조가 아직
바뀌지 않았으므로 WP-1에서 도면은 수정하지 않는다.

## 채택 후 검증 matrix — 이번에는 실행하지 않음

새 library filter는 `owner_equity_v2::market_stream_runtime_tests::database_cases::`로
고정하고 WP-3이 실행 전 실제 함수명/정확한 개수를 root에 제출한다. 아래는 의무 시나리오
묶음이며 존재하지 않는 test PASS가 아니다. DB는 명시적 task-owned supervisor 입력과
실제 bootstrap/전체 migrations/app·worker·research_writer·admin direct login을 요구한다.

| ID | 경계와 필수 관측 | 기존 회귀 의무 / 소유 |
|---|---|---|
| V1 | off/rest/WS×daemon/once: WS state/approval/socket factory 및 recurring REST quote count. once/off는WS0,WS 모드 REST quote0,invalid window에서도EOD 진행 | runner startup tests; WP-3 |
| V2 | 서로 다른 process가 동일 anchor 경쟁, DB claim 실패 시approval/socket0; lease expired여도 옛 socket lock 보유면 competitor0; connect 취소 후socket-before-unlock | C2 domain/process/cancellation + C1 fence; WP-3 |
| V3 | quiet/부분 frame/fragment/control write/hot traffic에서 bounded Idle, 명령과 read 동시 실행0; timer가 취소/복구하지 않음 | C2 wrong/duplicate/late ACK,poison-before-proof,immutable capture,default compile-fail 전체 영향 검사 |
| V4 | 실제30종목1/10소비자 상류30공유,120/10min·1000/day·1000ms command spacing,Deferred 후fresh demand,positive count ACK 보존,zero→positive 새 epoch | pacing4,original19/20/22/23 및 같은 함수 변경 영향 전부 |
| V5 | receipt30 latest,두 batch 한도,hot-symbol overwrite,>=250ms quote tx spacing/≤4tx·120rows/s,상태1Hz,heap≤8MiB,RSS slope/commands/coalesced/drop count | immediate/packed/ordinal 및 C3A batch atomicity; private 미commit fanout0 |
| V6 | 느린 DB/unknown commit/renew timeout/lease margin5s/권리 철회 중 owned task 정지,유일한 exact reread,no blind replay; producer 재사용0 | original21,24–26 + producer 모든 await 취소/실패; C1 locks/RLS 재검증 |
| V7 | clean-close→durable backoff→fresh facade/ACK;TCP error·pending·cancelled write→STOPPED;grace/no-demand≤5s;budget고갈새tab으로reset0 | C2 reconnect/domain ambiguity + original23; counters원본보존 |
| V8 | CONNECTING/PENDING/ACK무체결/STALE/gap/STOPPED 저장;quote_version 불변;stale fence update0;동일gap counter중복증가0,다음 proven day flag reset | C3A actual-role constraint/update grant tests; 상태 setter가 quote 권한을 우회하지 않음 |
| V9 | never-quoted30행,LISTEN 알림 commit후 only,unknown commit→공개값0,expiry/currentepoch가cachedLIVE를덮음;read helper wrong owner/session/GUC/generation/revoked grant 거부 | C1/C3A role matrix+SECURITY DEFINER ownership/search_path/PUBLIC deny; WP-4 통합 입력 |
| V10 | 정확한 calendar/window lineage 및 close boundary,날짜전환·해시교체·missing proof에서quote/approval/WS/calendar GET0;새proof가 독립 검증된 경우만재개 | 기존 resolver/EOD source reuse 회귀; 당일실데이터는WP-9 |
| V11 | initializer actual bin/library의nonnil slot/generation,UID gate,완전 absent layout만 생성,partial/zero/corrupt/symlink/hardlink/anchor교체거절,no-replace/fsync failure,재적용bytes/inode/history보존 | kis-client state/domain/default-doc tests; production소유권+부모/leaf mounts 검증 WP-6 |
| V12 | 전체55 migration source에 좁은 변경을 적용한 freshDB에서up/down 의존순서/함수권한/C1교착·generation·release/revoke 회귀 | 0055 unpublished root gate 후WP-3. 실제운영 DB적용은WP-8 별도 |

작업 간 Cargo 직렬 lock, `CARGO_BUILD_JOBS=2`, `--locked --offline`와 새 실행 범위를 유지한다.
원래26건을 인계명 변경만으로 반복하지 않는다. 다만 `producer.rs`, C2,0055가 바뀌므로
실제 영향 함수에 걸친 original회귀는 생략할 수 없다. 정확한 테스트목록은 구현 diff에 따라
확정한다. 외부boundary target 하나나 산술 unit test로 actual-role/pacing 검증을 대체하지 않는다.
WP-7의 fake provider→producer→PG→인증API→nginx→React/Chromium >=30분/p95≤2s는
V1–V12와 별도의 통합 acceptance다. live KIS/운영 scope는 여전히 없다.

## 실제 수행 검토와 root disposition 입력

수행: 적용 AGENTS/명시된 skill과 네 primary 문서 읽기, `git rev-parse HEAD`/`git status`,
`rg`/`nl`/`sed`로 위 source seam과 migration source pair 확인, 두 보존 source SHA256 대조,
소유 두 문서의 diff/공백/참조 위치 검사. 관련 memory context는 이 작업자의 fresh hook
envelope로만 읽었으며 최신 인계보다 오래된 실패 메모리를 우선하지 않았다.
빌드/Cargo/테스트/DB/PG/서비스/Docker/provider·WebSocket/운영 파일 접근은 수행하지 않았다.
Basic Memory·추가 위임·commit/push·다른 worker 수정은 없었다.

root는 다음 세 설계 선택을 채택/반려하면 된다: (1) C2 lock/idle 최소 seam과 command barrier
보존, (2) unpublished 확인을 조건으로 한0055 status/delivery 최소 확장 또는 append-only
분기, (3) root-only initializer가 두 새 leaf를 함께 소유하는 WP-3↔WP-6 인터페이스.
채택 후 WP-3 manifest와 V1–V12를 고정할 수 있다. 지금은 runtime readiness가 아니라
**구현 계약 검토 readiness**다. 추가 native/Paseo worker 분해는 수행하지 않았다.
WP-3이 이 manifest를 한 번에 안전하게 처리할 수 없으면 root가 C2/initializer→storage→
runtime 순서의 내부 실행 단위를 따로 배정해야 하며 이 문서는 다음 패키지를 시작하지 않는다.

미해결/후속: 위 세 채택,0055 현재 unpublished 여부,새 runtime 및 전체 integration 검증,
WP-2 G1–G5 및 실제 next-day proof 공급/운영 인수. 확인하지 못한 것: 실제운영DB/설치상태,
private 보고서 원본 자체,live broker 응답/한도,전체 WS3/SSE/browser/soak 결과. 이 범위 안에서
필요한 source seam을 찾지 못한 항목은 **없음**. 명세를 완화하거나 허가를 만들어 메운 항목도
**없음**; 모든 변경은 제안으로 표시했다.
