# R5-03 Paper 원장 구현·격리 통합 검증

> 2026-11-05 최종 상태: R5-03 코드·격리 통합 DONE. [최종 인수와 정확한 범위](2026-11-05-r5-paper-acceptance.md). 아래 DOING·미실행·실패 표기는 각 시점의 기록이며 보존한다.

상태: **DOING**. 대상은 `a30edacff987d55b5be7cca25bed7faeec589629` 위의 기존 미커밋 작업공간이다. 최종 dirty 입력 핀과 기능 인수는 아직 확정하지 않았다. 이 문서는 진행 증거이며 배포·실제 운영 Origin·Owner 사용 인수의 완료 기록이 아니다.

R5-02의 [코드 인수](../../target/r5-paper/coordinator/r5-canonical-reader-integration-20261102/R5-02-code-acceptance-20261104.json) `f9141f76`는 보존한다. R5-03은 ADR-0010의 전략·금융 모형과 기존 권리 범위에서 내부 모의 원장을 구현한다. KIS 계좌·잔고·주문 API를 호출하지 않으며 production CLOSED/default UNBOUND를 유지한다. commit/push/deploy는 수행하지 않았다.

## 구현 경계

- 기존 fixed-point sizer, session-open plan, ledger, pending target, settlement outbox를 재사용한다. T 종가 preview와 E 시가 재계산·종가 평가를 분리하고, 같은 신호의 정상 무거래도 일별 정산·평가 기록을 남긴다.
- 실제 checked Daily view에서 F/W와 원본·admission 계보를 확인한 데이터만 계산에 전달한다. 계산 데이터 자체를 운영 권한으로 취급하지 않는다.
- API와 worker는 실제 실행 ELF와 고정된 보호 config, 같은 Current holder를 유지하는 action scope를 사용한다. 불확실한 commit은 readback이 필요한 오류로 남긴다.
- 신규 0067은 Owner 하나의 control/events/lineage/sessions, 별도 app/worker 함수, FORCE RLS와 일반 Paper 쓰기 경로의 beta 차단을 추가한다. ledger·target/session 상태·outbox를 한 트랜잭션으로 반영한다.
- 자동 적용도 동일한 계산 preview를 거친다. 이전 E 정산·평가가 완료된 잔고를 새 T preview가 사용해야 하며, 일반 Paper의 비어 있는 dataset pin을 beta 권한으로 사용하지 않는다.
- 저장된 ENABLED와 현재 자격을 구분한다. 재검증 실패는 기존 이력을 바꾸지 않고 BLOCKED 읽기 상태로 투영하도록 통합 중이다.

구현 파일과 정확한 인터페이스는 [작업 계획](../../target/r5-paper/coordinator/r5-03-20261104/package-plan.md), [SQL ABI](../../target/r5-paper/coordinator/r5-03-20261104/sql-abi.md), [API 인계](../../target/r5-paper/coordinator/r5-03-20261104/api-contract-handoff.md), [통합 결정](../../target/r5-paper/coordinator/r5-03-20261104/api-worker-integration-decisions.md)에 기록한다.

## 회수한 검증

아래 수치는 서로 겹치는 필터를 포함한다. 전체 고유 테스트 수로 합산하지 않는다.

| 범위 | 결과와 한계 |
| --- | --- |
| 순수 Paper 계산 | 집중 9 PASS. 실제 보호 런타임·DB 실행 증거는 아님 |
| checked 계산·달력 | factor 5, calendar 관련 24 PASS |
| matcher/action/financial/custody/runtime | 집중 31 PASS. 초기 fixture 오류와 zero-match 필터 기록 보존 |
| API state/HTTP/worker 및 달력 witness 보완 | state3·HTTP7·worker3 및 market S3 22 PASS. source 핀별 로그/receipt는 API 보고에 기록; 실제 DB/router 검증과 구분 |
| financial capture selector | 컴파일·결정적 8 PASS; genuine ignored selector 실제 실행은 별도 대기 |
| OpenAPI 전체 검사 | 105개 연산 메타데이터·JSON 동기화·타입 재생성·tsc PASS. 아래 기존 설치 의존성 재사용 |
| 0067 실제 DB Native06 | 생성·재적용·빈 원장 rollback 및 이전 카탈로그 일치 2 PASS |
| 0067 실제 DB Native11 | 기능6/source1 PASS, 0 ignored. 실제 역할/RLS, 수동·자동 적용, stale/lease/epoch/권리, 동시 retry, 거래/무거래 정산·평가·outbox 원자성. genuine Raw·보호 프로세스·process crash 증거는 아님 |

OpenAPI는 기본 작업공간의 기존 설치 도구를 읽기 전용으로 사용했다. 두 작업공간의 package-lock SHA256은 `1932c810e7165807f24c9aa3005063c9e8dde3292ac27d8ede9c3735da5e9e76`로 같고, 설치된 openapi-typescript 7.13.0과 TypeScript 5.9.3은 lock과 일치한다. 원래 검사기의 source/dependency 경로만 바꿨으며 새 설치·네트워크 호출은 없다. [전체 검사 receipt](../../target/r5-paper/coordinator/r5-03-20261104/root-openapi-full-check.json), [로그](../../target/r5-paper/coordinator/r5-03-20261104/root-openapi-full-check.log). 앞선 작업공간 의존성 탐색 실패는 [API 보고](../../target/r5-paper/coordinator/r5-03-20261104/api-report.md)에 보존한다.

## 실제 DB 실패·복구 이력

각 실행은 새 nonce, 고정 ELF/소스/이미지, network-none 임시 PostgreSQL 18, canonical HeavySlot을 사용했다. 실제 테스트 ELF에서 ignored DB 사례를 명시적으로 포함했다. 다음 실행의 성공으로 앞선 실패를 바꾸지 않는다.

| 실행 | 결과 |
| --- | --- |
| Native01 | inet 주소의 netmask 표현 때문에 identity 검사에서 실패. `host(inet_server_addr())`로 fixture 식별 수정 |
| Native02 | down에서 이미 제거된 predecessor constraint를 다시 삭제하여 실패. up migration 오류가 아님 |
| Native03 | down의 FK 의존성 제거 순서에서 실패 |
| Native04 | down 후 beta 추가 컬럼 두 개가 남아 최종 assertion 실패 |
| Native05 | 강화된 전체 catalog 비교에서 0041 함수 두 개의 search_path 복원이 달라 실패 |
| Native06 | 원래 `pg_catalog,public` 복원 후 catalog/down 전체 일치 PASS. ELF `4da2609dee22c1817afd59fcd0c2f3233fa90d764b94daa64e9d0ca7eb8895b0` |
| Native07 | calendar/catalog/source PASS; configuration fixture가 SQL NULL 대신 JSON null을 전달해 FAIL |
| Native08 | Native07 경계 수정 후 계정/동의/역할/RLS/이력 보존 down PASS. 새 ENABLE/AUTO에서 `cohort` 변수·컬럼 이름 충돌 42702로 FAIL |

| Native09 | AUTO의 run_id 변수 충돌로 4 PASS/1 FAIL. 신규 함수34개를 한정해 식별자 qualification 검토 |
| Native10 | 4 PASS/3 FAIL. 재사용 Daily claim fixture가 preview attempt.started_at을 누락. 실제 worker와 같은 새 0067 fixture claim으로 수정 |
| Native11 | 7 PASS/0 FAIL/0 ignored. ELF `6d3177174ed660b5972f4044a4768d2282a4bf6fcb1bbe0b920f4b6bc7371e6e`, manifest `3157f83ab5b17ccd9c86926ae1024cbae060be9a78fbed744838ab6808b1ec47` |

| Native12 | actor scope/late valuation 보완 후 source1 PASS, DB8 FAIL. 함수의 `SET app.actor_user_id` 선언 생성이 migration 역할에서 42501로 거절되어 기능 검증 전 실패. 권한 확장 없이 수정 중 |

| Native13 | 9 PASS(실제DB8/source1), 0ignored. 명시LOCALsave/set/restore로 역할 문맥 복원·late valuation recovery 확인. ELF `16e2dafbc5a6f0124e5ec7c1c642d0824b24d4f69356510b43ae2f9913b3a177`, manifest `d80c56164739b850da3780a7c418a556c710065abaa708119c7b1ea3aade0d67` |

Native01~13의 소유 컨테이너 종료·삭제와 Root 실행 자식의 terminal/reap을 확인했다. 다른 세션의 자원이나 과거 UNKNOWN 핸들까지 정리됐다는 주장은 하지 않는다. 정확한 실행 핸들, nonce, receipt와 현재 worker 책임은 [ledger](../../target/r5-paper/coordinator/r5-03-20261104/ledger.json)에 있다.

Native11 이후 실제 worker pool과 migration_owner tenant RLS를 대조하여 unset actor 연결 공백을 확인했다. 기존 fixture는 Owner actor를 설정해 이 경계를 검증하지 못했다. SQL 함수 내부에서 검증된 control Owner를 제한적으로 선택하고 모든 반환·예외에서 원래 actor를 복원하는 수정과 unset/foreign actor 회귀는 Native13에서 통과했다. Native11 통과 범위를 지우거나 실제 worker 인수로 확대하지 않는다. 이미 commit된 정확한 receipt의 내부 readback은 신규 경제적 쓰기와 구분하며, 철회 이후 새 체결·평가는 계속 차단한다.

## 통합 리뷰에서 확인한 결함과 보완

- 최초 E admission은 scan의 저장값 NULL에서 snapshot의 새 값으로 바뀌는 것이 정상이다. 이를 무조건 같은 값으로 요구해 첫 정산을 막던 worker 비교를 수정했다. 이미 저장된 admission 교체/소실과 잘못된 hash는 거절한다. 영향 worker 단위4 PASS; 실제 최초 정산 fixture는 별도 진행 중이다.
- 일반 Paper worker의 beta 제외 조건이 권한 없는 private control 테이블을 직접 읽었다. 기존 worker에 허용된 `paper_0067_is_beta_account` 함수로 두 scan을 바꿨다. 해당 source assertion을 갱신했으며 일반 Paper 실제 DB 회귀18건은 API-Native01에서 통과했다.
- E일 정산 후 평가 전 중단, 다음 날 재시작 시 기존 RECOVERY가 settled-but-unvalued 세션을 받지 않아 영구 대기했다. 기존 earliest-expired 복구 권한 경로에서 확정 원장/정산 identity를 보존하고 평가만 재개하도록 SQL 보완과 cross-midnight 검증은 Native13에서 거래/무거래 모두 통과했다.

구조도는 코드 의존성과 0067 원장/API 경계를 반영했다. 첫 로컬 PNG는 default4096폭 잘림으로 인수하지 않았고, 새 network-none 렌더에서 component11091×1733/runtime16000×2174 전체 PNG를 시각 확인했다. 두 소유 컨테이너/자식은 모두 종료·삭제/회수했다. 후속 코드 변경의 line citation을 재대조한 최종 렌더도 통과했다([receipt](../../target/r5-paper/coordinator/r5-03-20261104/diagram-render-a09ebc2d7949fe09d099aca439a9ccb2.json)). 두 PNG 전체 경계와 입력 불변, 세 번째 소유 컨테이너 삭제·자식 회수를 확인했다.

## 남은 인수와 후속 경계

R5-03 완료 전에 실제 preview/apply/AUTO·TRADE/NO_TRADE 정산·평가, 재실행/동시 실행/lease/commit 전후 crash/outbox 재시작, 권리·epoch 재검증을 인수해야 한다. 채택 전략의 새 5세션 genuine Raw→Daily fixture와 실제 보호 API/worker 통합을 준비 중이며, synthetic 증거를 운영 성공으로 부르지 않는다. 최종 Rust fast/focused 및 소스 검토, 정확한 dirty 핀, 다이어그램의 로컬 PNG와 ROADMAP 완료 갱신도 남아 있다.

[WP-4/WP-5 인계](../handoffs/2026-11-04-r5-paper-api-runtime.md)에 구현 계약과 후속 책임을 고정했다. WP-4에는 새 HTTP DTO/오류·미래 session 및 관측시각 표시를 인계한다. Apply의 영속 재실행 identity는 preview/token/account/epoch이며, 필수 Idempotency-Key 헤더를 별도 key/body map으로 저장하지 않는다는 계약을 명시했다. WP-5에는 실제 보호 config 설치·CLI 상태·drain/restore·명시 재개를 인계한다. 운영 Origin/G4 선행 실제 세션과 WP7 운영 관측은 별도 gate다.

## API-Native01 실제 DB 회귀 — 2026-11-04

고정 API 실행 파일4개를 새 network-none PostgreSQL에서 실행해 총22개가 통과했다: Owner-beta Paper2, Owner 접근/비활성 HTTP2, 일반 Paper 리밸런스10, runner8. 실패·ignored·DB 미설정 skip은 모두0이다. [인수 JSON](../../target/r5-paper/coordinator/r5-03-20261104/api-native01-acceptance.json)과 [실행 receipt](../../target/r5-paper/coordinator/r5-03-20261104/api-db-fd209ba45ed955ea546d89d058dde577/receipt.json)에 정확한 manifest/ELF/입력 핀을 남겼다. 611개 입력 핀은 실행 전후 동일하며 PG와 테스트 종료0, 소유 컨테이너 삭제 및 Root 자식 회수를 확인했다. 실제 보호 Current/Source를 사용하는 경제적 경로와 강제 종료·재시작 통합은 별도 진행 중이다.

### 최종 집중 검사 진행

Rust fast(cargo-format)는 `target/dev-check/20261104T134212Z-a8a027f33b1d`에서 PASS다. API execution3·engine4·runtime10도 모두 PASS이며 테스트 실행 자식은 회수됐다. 첫 all-target Clippy는 HTTP Response 큰 오류형2, 명시 SQL action 인자 수2, 동등한 중첩 if1 지적으로 실패했다. 원본 로그를 보존하고 함수 한정 lint 설명과 동등 조건식으로 수정 중이며, 최종 Clippy 성공은 아직 주장하지 않는다.

후속 함수 한정 lint 설명과 동등 조건식 수정 후 집중 검사17개가 다시 통과했고, 4개 API 실행 파일을 새 핀으로 빌드했다. 이후 통합 테스트의 lint3개를 정리한 최종 `cargo clippy -p api-server --all-targets --locked --offline -- -D warnings`가 PASS했다([receipt](../../target/r5-paper/coordinator/r5-03-20261104/api-final3-clippy.json), 실행 전후 Rust 입력 핀 동일·자식 종료/회수). 기존 Clippy 두 차례 실패 로그를 보존한다.

## Sealed workflow 실행 이력 — 진행 중

모든 실행은 별도 nonce/admission과 정확한 입력 핀을 사용한다. 앞선 실패를 성공으로 바꾸지 않는다.

| 실행 | 관찰 | 정리/후속 |
| --- | --- | --- |
| Native01 `a39e084d` | 실행 전 manifest 약162KiB가 reader128KiB 제한을 넘어 INPUT_FILE 거절 | 자원0; 테스트 manifest reader만256KiB로 변경, 제품 config 상한 불변 |
| Native02 `6b97a10d` | 보호 실행 파일 staging actor PermissionError | 컨테이너1·볼륨2 삭제/자식 회수, 입력 핀 불변; 이미 해시 검증한0555 staged ELF 사용 및 runtime tmpfs holder 추가 |
| Native03 `caa1b930` | staging 성공 후 API test listing 전 환경변수 검증 API_ENV_KEY | 컨테이너3·볼륨2 삭제/자식 회수, 입력 핀 불변; 고정 이미지의 GOSU_VERSION 허용과 중복/미등록 변수 거절 확인 |
| Native04 `9a9f974f` | staging 완료, 초기 API test listing 시작 실패 | 컨테이너3·볼륨2 삭제/자식 회수; 당시 원인 UNKNOWN, 제한된 loader 진단 추가 |
| Native05 `f6eb0b0c` | OCI exec stat permission denied 진단 확보 | 소유 컨테이너·볼륨 삭제/자식 회수; actor umask077로 bin 디렉터리가0700인 경로 확인, 명시 chmod0755와 메타데이터 검사 추가 |
| Native06 `371bb4bb` | 실제 역할·migration67·prepare-config 성공 뒤 bootstrap_off 컨테이너 시작 실패 | 소유 자원 삭제/자식 회수; 첫 Raw 생성 전 Raw subpath 마운트 결함 확인·초기 OFF 단계만 제거 |
| 준비 r7 `654e4850` | Root 명령의 admission 경로 오타로 파일 읽기 전 실패 | 자원0; 재시도 전 fixture 추가 결함이 확인돼 준비 범위 철회, 실행 성공으로 세지 않음 |

Root의 수정은 테스트 실행기·manifest 생성기·staging actor에 한정된다. 원본 ELF·실패 로그/receipt는 보존하며, 후속 실행의 인수 범위를 넓히거나 제품 권한을 완화하지 않는다.

실행기 가정의 반복 실패 후 동일 SEALED 작업자의 모델만 Luna→gpt-6.1-sol로 상향했다(effort max 유지). 좁은 재검토에서 web_sessions seed의 actor 누락, FORCE RLS 경제적 readback의 actor 누락, 날짜 이동 후 fixture 로그인 만료를 확인했다. 제품 RLS·권리·세션 만료를 바꾸지 않고 fixture 전용 transaction/readback와 HTTP 단계별 세션 갱신을 수정 중이다. 실제 worker와 migration-owner 실행 pool은 unset actor를 유지한다.

Native08 `67960c65`는 새 API ELF의 실제 bootstrap consent HTTP assertion에서 실패했다(예상200; 당시 실제 상태/오류 코드는 미보존). 기존 fixture 보완 후 session seed를 지나 HTTP 호출까지 도달했으며, 원인을 추측하지 않고 제한된 오류 진단을 추가한다. 컨테이너17개·역할 helper5개 및 소유 볼륨3개 삭제, 비밀 임시파일 제거, 입력 핀 불변과 Root 자식 종료/회수를 확인했다. [원본 receipt](../../target/r5-paper/coordinator/r5-03-20261104/sealed-workflow/runtime/67960c654ad0dca4c92b751b935ae6b1/receipt.json)를 보존한다.

API 함수 한정 lint 설명 추가로 이동한 세 citation까지 반영하여 PUML/PNG를 다시 맞췄다. [최종 대응 렌더](../../target/r5-paper/coordinator/r5-03-20261104/diagram-render-1214003472236be2fd1c4a8738b57981.json)는 PASS이며 입력 두 개 불변, 소유 컨테이너 삭제와 자식 회수를 확인했다.

Native08 후 actual app ACL과 consent repository를 대조해 추가 결함을 확인했다. 읽기 전용 immutable events의 prior-event 조회가 `FOR UPDATE`를 요청해 UPDATE 권한을 요구했다. 기존 consent definer가 control/advisory lock과 중복 검증을 담당하므로 repository의 불필요한 row lock 한 곳만 제거했다. SQL 권한은 유지한다. Native08의 실제 응답 미보존은 그대로 UNKNOWN이며, 새 실행에서 최초 동의·동일 재실행·같은 키 다른 요청 거절을 검증한다.

같은 repository의 apply readback도 SELECT/제한 INSERT만 허용된 preview 테이블에 `FOR SHARE`를 요청했다. apply definer가 이미 같은 외부 transaction 안에서 해당 preview의 `FOR UPDATE`를 유지하므로 중복 잠금만 제거했다. 두 수정의 코드 근거와 새 실행 대기 상태는 [Root 추가 리뷰](../../target/r5-paper/coordinator/r5-03-20261104/root-repository-lock-review.json)에 있다. 기존 bounded 리뷰 이후 발견된 두 항목이며, 당시 리뷰 범위의 성공으로 새 실행을 대신하지 않는다.

Native09 `d0ab6434`는 실제 app의 최초 동의200·동일 재실행200·같은 키 변경 요청409, Owner state200·다른 사용자403과 단일 동의 이벤트/epoch 보존을 통과했다. 뒤따른 bootstrap 파일 readback에서 cap-drop root가 UID10001의 비공개 파일을 읽지 못해 실패했다. 동일한 작성자 UID10001로 reader를 실행하도록 fixture 두 줄만 바꿨고 Rust/SQL/ELF는 불변이다. 원본 실패와 소유 자원 삭제/자식 회수를 보존하고 새 Native10에서 이어간다.

Native10 `640d7ec7`는 최초 세션 Raw→Source 예약→Daily produce/admit→실제 compute-publication→Source run 연결까지 통과했다. 두 번째 Raw/예약 후 producer가 `Err(Conflict)`를 반환해 중단했다. 정확한 충돌 분기는 조사 중이며 첫 세션 성공을 전체 경제적 통합 인수로 확대하지 않는다. 입력 핀 불변·소유 자원 정리·Root 자식 회수를 확인했다.

Native11 `b23d02b2`에서 최초3세션의 genuine Raw→Daily compute/publication→Source 연결과 SQL ENABLE(epoch1), 실제 보호 API/worker 설정·실행 파일 로딩/재검증을 통과했다. preview POST는409 PAPER_STALE로 거절됐다. fixture가 Current와 Paper config에 별개 release manifest 해시를 넣는 결정적 불일치를 확인해 동일한 초기 manifest의 해시와 API 실행 파일 핀을 함께 검증/재사용하도록 실행기만 수정했다. 원본 응답 이상의 세부 거절 원인은 추측하지 않는다. 기존 Rust/SQL/ELF는 불변이며 소유 자원 정리/자식 회수 후 Native12를 새 범위로 실행한다.

Native12 `739280cd` ended independently on second candidate `Err(InputBlocked)`, distinct from Native10 Conflict. Its same Rust ELFs had reached three sessions in Native11. Root's subsequent exact-controller stop attempt found no matching process and sent no signal. All owned resources/children were already cleaned/reaped; inputs remained unchanged. Exact internal stage is UNKNOWN. A read-only remaining-phase review also found API fixture omissions: required Daily config path/hash environment and wrong producer-UID historical mount for UID10001. Private driver now passes the existing pinned config and uses the already approved reader's read-only source mount, preserving producer/admitter DAC. Finite cfg(test)-only producer diagnostics are being added to observe the new error without changing validation rules.

Native13 `c595c404` reproduces the second candidate failure with finite diagnostic **DISCOVER / INVALID_EVIDENCE**. Root traced the isolated new-session commitment to calendar coverage validation that incorrectly starts at the historical base. Selection of an older valid lookahead calendar can then fail because its request date belongs to the already sealed prior interval. A focused regression and represented-interval correction are underway; complete manifests still require the historical base and contiguous chain. Native12's erased internal stage remains UNKNOWN. Native13 inputs were unchanged and all owned resources/children were cleaned/reaped.

### 2026-11-05 KST — 달력 continuation 범위 회귀

SEALED Native13의 실제 진단은 `DISCOVER / INVALID_EVIDENCE`였다. 새 하루만 commitment하는 경로가 전체 역사 기준일을 달력 요청일 검사 하한으로 사용해, 직전 frontier에서 이미 관측한 lookahead 달력을 잘못 거절했다. 수정 전 deterministic wire/Raw 회귀는 4 FAIL·1 PASS로 이를 재현했다. 첫 supplied session의 `previous_frontier`를 해당 부분 구간의 하한으로 쓰도록 수정한 뒤 wire 31·Raw discovery 27, 합계 58 PASS다. Root가 실제 diff와 테스트를 검토했으며 전체 manifest의 BASE 시작·연속성 및 새 구간의 미주장 요청일 거절은 유지된다. 기존 wire bytes/hash golden도 유지된다. 정상 market-data 라이브러리 변경이므로 세 native 실행 파일을 새로 빌드 중이며, 전체 보호 Paper workflow는 아직 미인수다. Native12의 상세 실패 단계는 UNKNOWN으로 보존한다.

### SEALED Native14 — 새 달력 범위 통과, Source 원본 연결 미인수

새 세 실행 파일과 904개 입력 핀에서 두 번째 날짜의 produce/admit/source-read까지 통과했다. 다음 source-link-admission은 `R5_INTEG_SOURCE_OBSERVATION`으로 거절됐다. 세부 observation 단계는 UNKNOWN이다. Root 실행은 exit 1/reap이며 모든 소유 컨테이너·helper·세 volume 제거, 비밀 파일 제거와 입력 불변을 확인했다. 계약 §7의 current-day calendar 원본과 Daily의 agreeing witness 선택 관계를 좁혀 검토 중이다. 실제 Source 원본 일치 guard는 유지하며, 이 실행은 전체 Paper 인수가 아니다.

Native14 이후 Root는 계약의 당일 calendar 원본 요구와 기존 Daily의 최소 증거 hash 선택 사이의 충돌을 확인했다. 새 trading-session discovery만 검증된 당일 terminal original을 우선하고, 기존 봉인 frontier에서는 성공한 새 재구성이 저장된 세션과 다를 때 기존 hash 규칙으로 전체 세션을 재구성해 정확히 일치하는 경우만 호환하는 방안을 채택했다. 새 candidate admission에는 legacy 우회를 넣지 않는다. Raw 변경·추가·충돌 검사와 전체 observation 비교는 보존한다. 봉인 replay reader는 실제로 Raw를 재선택하지 않으므로 변경하지 않는다. 구현·회귀·새 runtime 결과는 아직 대기 중이다.

당일 calendar 선택·봉인된 legacy 선택 호환 구현 후 집중 회귀는 wire31, discovery27, frontier17, 실제 Raw/shared compatibility1, terminal original/copy-date1로 77 PASS다. 새 거래일은 당일 terminal original을 우선하되 CLOSED 날짜와 모든 witness 충돌 검사를 보존한다. 봉인된 세션은 성공한 새 재구성 불일치 시에만 기존 선택을 재구성하고 저장된 전체 세션과 일치해야 한다. 추가 재구성도 기존 작업량 한도에 포함한다. 과거 prior-selected protected 체인 전체의 runtime 검증은 미실행이며, 실제 Raw·저장 바이트·같은 내부 분기를 검사한 범위와 구별한다. Root의 Rust fast·diff 검사 및 market all-target Clippy도 통과했다. 새 세 실행 파일의 통합 실행은 대기 중이다.

### SEALED Native15 — 세 Daily 연결 통과, 보호 API 단계 timeout

당일 calendar 선택 수정 후 세 날짜의 실제 produce/admit/compute/Source 연결과 epoch1 모의 ENABLE이 통과했다. 다음 `protected_process`의 attach CLI가 180초 제한을 넘었다. 기록은 timed_out=true, terminal=true, CLI returncode0, stdout64B/stderr608B이며, CLI의0은 native 테스트 성공을 증명하지 않는다. 기존 backend는 timeout 시 결과를 반환하기 전에 예외를 내므로 내부 HTTP/단계 진단은 보존되지 않았다. 정확한 native 원인은 UNKNOWN이다. Root session96076 exit1/reap, 전체 소유 컨테이너·helper·3volume 제거, 입력 불변/비밀 파일 제거를 확인했다. 제한을 늘리거나 성공 처리하지 않고, 종료 전 정확한 소유 컨테이너의 유한 진단을 회수하는 test-only/private-driver 수정에 착수했다.

Owner의 추가 지시에 따라 R5-03 작업 종료 시한은 2026-11-05 08:30 Asia/Seoul이다. 새 장기 작업은08:00 전에 제한하고,08:25까지 소유 정리를 목표로 하며08:30까지 실제 결과와 미해결 사항을 기록한다. provider·운영·배포 권한은 확대하지 않는다.

### SEALED Native16 — preview timeout의 실제 유한 진단

`1c2fc32f56f871398cf04f839c67127c`에서 세 Daily/Source 연결·ENABLE 이후 보호 API 로딩/재검증10013ms, worker10008ms를 관측했다. preview POST는30069ms 후503 `PAPER_COMMIT_UNKNOWN`을 반환했다. 이후 native 단계는 종료되지 않았고180초 attach timeout 당시 컨테이너는 running=true/OOM=false였다. 실행 중 inspected ExitCode0은 성공이 아니다. 원래 예외와 유한 progress/HTTP 진단을 삭제 전에 회수했고 모든 소유 자원 제거·자식 회수·입력 불변을 확인했다. Native15 내부 원인은 여전히 UNKNOWN이다.

Root는 반복 executable hash 비용과 CPU0.45 제한이30초 action budget에 미치는 영향을 가설로 분리했다. Native17은 API CPU만1로 변경하고 세 ELF,30초 제품 제한,180초 phase 제한, 권한과 모든 검증을 유지한다. 첫 준비본은 실행 전 FRESH_DEADLINE으로 거절되어 자원이 없었으며, 신선한 nonce `0895c70f58ca8109e1bf083a8a7735b1`로 다시 검토·실행했다. 동시에 action/취소 경로를 읽기 전용으로 검토한다. 비교 결과 전까지 원인을 확정하거나 timeout을 늘리지 않는다.

Native17 비교 결과: 실제 API 로딩/재검증4325ms·worker4354ms, preview POST15458ms 후422 `PAPER_SOURCE_BLOCKED`, native101 종료였다. 제품30초 제한과 동일 ELF를 유지한 상태에서 timeout 구간을 넘겼다. CPU 할당이 검증 시간에 영향을 준 것은 관측됐으나 Native16의 종료 대기 내부 원인은 확정하지 않는다. Root session84130을 회수했고 소유112컨테이너·helper5·3volume 제거와 입력 불변을 확인했다. 다음 실행에는 실패한 실제 app submit SQL의 거절을 동일 fixture에서 rollback-only 진단으로 분류한다. 이 별도 진단은 보호 API 성공이나 영속 경제적 쓰기의 인수가 아니다.

### SEALED Native18 — Daily 제외 종목 표현 불일치 확인

실제 API422 이후 동일 app 역할·동일 입력의 rollback-only 진단이 `23514 / ALLOCATION_UNSUPPORTED`를 반환했다. 정상 Daily publisher는 제외 종목을 `excluded=true, target_weight=0`으로 기록하며0060도 이를 요구한다. 새0067의 수동·자동 적용 검사는 `target_weight IS NULL`과excluded가 같아야 한다고 요구해 genuine Daily를 거절했다. 기존 SQL fixture가 모든 종목을excluded=false로 생성해 이 경계를 놓쳤다. Root가 두 검사와 실제 형식의 fixture/음성 회귀 보완을 지시했다. 금융 비중·비용·전략이나 기존0060/R4 의미는 바꾸지 않는다. Native18 Root42644 exit1/reap, 전체 소유 자원 제거·입력 불변을 확인했으며 SQL 진단은 경제적 성공 인수가 아니다.

SQL Native14 `f4db351a04eaf4967cf2b969c8766829`에서 수정된0067와 canonical 제외 fixture를 실제 PostgreSQL로 검증했다. 기능9·source1 총10PASS/0FAIL/0ignored이며 manual/AUTO·투자/현금 신호와6종류 잘못된 배분의 정확한 거절, 무변경을 확인했다. 이전 RLS·stale·동시성·원자 정산/평가·자정 복구·rollback 경계도 통과했다. 새 ELF `6d270ea0`, manifest `b1696f2b`; 실행8.236초, 입력196핀 불변, PGexit0·소유 컨테이너 제거·Root28997회수. [인수 JSON](../../target/r5-paper/coordinator/r5-03-20261104/sql-native14-acceptance.json). 실제보호 workflow Native19는 별도로 진행한다. native 세 ELF는0067이나migration-contract를 내장하지 않음을 소스/depfile로 확인하고 정확히 같은 바이트를 재사용하며, 변경된SQL은 새905핀으로 외부 적용한다.

### SEALED Native19 — 실제 preview 접수, worker 실행기 경계

수정된0067에서 보호 API preview가202 Accepted로 통과했다(POST17582ms). 이어진 worker cycle에서 Polars0.54.4의`block_in_place_on`(pinned source81행) panic과 native test의 물리PG backend PID 동일성 assertion이 관측됐다. 정확한 자유문 panic 내용은 보존하지 않았다. action은현재`new_current_thread` Tokio 실행기를 사용하며 Polars의`block_in_place`는 이 flavor를 지원하지 않으므로, 같은 executor seam의 집중 회귀와 callback의!Send 수명을 보존하는 좁은 수정에 착수했다. PgPool 별도 borrow 사이의 PID 교체도 실제 역할/actor·이전 backend 종료 증거로 구분하도록 fixture를 검토한다. Root24469 exit1/reap, 소유 자원 제거·입력 불변 확인. 제품 deadline이나hash/권한guard는 그대로이며 전체 경제적 인수는 아직 미완료다.


### 2026-11-05 KST — action 실행기 집중 수정 검증

같은 private executor factory를 사용하는 회귀에서 수정 전 `block_in_place`의 current-thread 거절을 재현했다. 전용 worker 한 개의 multi-thread runtime으로 바꾸고 callback은 원래 blocking caller의 `block_on`에서 실행해 `Rc`를 await 너머 보유하는 !Send 성질을 유지한다. 수정 후 action5 PASS, job 라이브러리 Clippy, job/API native 빌드, API all-target Clippy가 통과했고 899개 소스 입력은 전후 동일했다. 기존 deadline·Current/실행 파일 재검증·권한 경계는 변경하지 않았다. fixture만 기존 기본 lease60초와 실제 worker 역할/actor 및 교체 전 backend 종료 확인에 맞췄으며 crash 식별 marker는 고정했다. 이동한 PUML 코드 근거와 PNG 두 개를 로컬 재생성했고 소유 render 자원을 제거했다. 새 실행 파일의 전체 workflow 검증은 별도 대기이며 DOING을 유지한다.


SEALED Native20에서는 앞선 Polars panic 없이 preview202와 조회200까지 진행했다. 실제 worker 단계 시작26310ms→WORKER_STAGE_TIMED_OUT41324ms로 외부15초 제한이 확인됐고 READY assertion에서 종료했다. Root99487 exit1/reap, 소유112컨테이너·5helper·3volume 제거와 입력불변을 확인했다. 기존 `RunnerServices::with_deadlines`로 이 큰 디버그 ELF fixture의 두 실행 경로만 operation/cycle90초를 사용한다. 제품 기본15초·내부 action90초·API30초·driver180초는 바꾸지 않는다. 이는 설정된 fixture 검증이며 기본값의 운영 성능 인수가 아니다. 전체 경제적 실행은 여전히 미완료다.


SEALED Native21 진행 중 보호 workflow의 `protected_process`와 `missing_e_wait`가 각각 실제1PASS로 종료했다. 실제 worker68.626초 후 READY preview를 조회했고 manual apply200/동일target replay200, E 이전 PENDING·정산/평가 없음·경제적 원장0을 확인했다. 보호 phase145.078초 뒤 모든 pool.close가 완료됐다. 다음 E 입력·중단/복구·평가/AUTO는 아직 실행 중이며 전체 성공으로 표시하지 않는다. 기존90초 fixture 예산과 제품 기본15초 성능의 구분을 유지한다.


Native21은 E 세션 Source 연결까지40개 phase를 통과한 뒤 독립 financial-normal selector에서 Polars81 panic으로 종료했다. 해당4개 금융 capture selector가 current-thread 테스트 런타임을 사용함을 확인하고 테스트 annotation만 multi-thread/worker1로 맞췄다. 제품 action의 실제 성공과 이 독립 테스트 실행기 실패를 구분한다. Root82161exit1/reap,149개 소유컨테이너·5helper·3volume 정리와905입력 불변을 확인했다. 제품/API/SQL/금융검사·기한은 바꾸지 않으며 job 테스트 실행 파일만 재빌드한다.


SEALED Native22은 실제 금융 capture4개까지44phase를 통과했다. 정상 T/E snapshot·SQL identity/F/W hash·가용성·달력·preview/실행/평가 계산과 수동/변조/다른입력의 기대 거절을 확인했다. 이후 precommit 잠금 관찰45초가 만료됐으며 실제 내부worker단계는 UNKNOWN이다. Root37794exit1/reap,161소유컨테이너·5helper·3volume 제거·입력불변을 확인했다. 앞서 실제worker68.626초가 관측된90초 fixture 작업을45초 관찰로 제한하던 harness만120초 관찰로 맞추고, 기존180초phase·90초action·전체작업/정리 한도는 유지한다. 목표잠금을 관측해야만 중단할 수 있는 조건은 유지하며 실패시 유한단계·잠금상태 진단을 보완한다. 금융정책/제품/SQL/ELF는 바꾸지 않는다.


SEALED Native23은44phase와 실제precommit중단을 통과했다. orders의미획득RowExclusiveLock을 exactworker/PID에서 확인하고SIGKILL137·backend종료 뒤에도 WAITING_ADMISSION/PENDING 및주문·체결·경제적원장·outbox0이 유지됐다. 이후prevaluation worker는97191ms에WORKER_ITEM_ERROR를 기록하고모든pool을닫아exit0했으나목표잠금을관측하지못해전체실행은FAIL이다. 구체오류·정산commit여부는UNKNOWN이다. Root76953exit1/reap,163컨테이너·5helper·3volume정리와핀불변확인. native는기존8종유한오류분류를기록하고driver는종료된정확한소유API에한해읽기전용경제상태를기록하도록보완한다. 또한prevaluation의ACCESS EXCLUSIVE는읽기까지막아의도한INSERT이전에서막힐수있으므로기존orders와같은SHARE쓰기장벽으로좁힌다. 실제RowExclusive대기와정산완료/평가미완료상태확인은그대로필수다. 이설계수정으로Native23의실제원인을소급확정하지않는다.


SEALED Native24 진행 중 두 crash 경계를 실제로 통과했다. 두 테이블의SHARE장벽에서 정확한worker의미획득RowExclusiveLock을확인하고SIGKILL137/backend종료·회수를확인했다. precommit은WAITING_ADMISSION/PENDING·경제적기록0을보존했고, prevaluation은SETTLED_TRADE/EXECUTED·주문/체결/비입금원장/outbox/terminalevent각1·평가0을보존했다. 재시작평가/outbox·AUTO·최종checker/정리는계속실행중이다. 이성공으로Native23의관측되지않은구체오류를소급확정하지않는다.


Native24는 45개 phase와 두 실제 SIGKILL 경계, 재시작 후 정산 재실행0·평가1·원장/outbox 동일성까지 통과했다. 이후 AUTO preview에서 WORKER_LEASE_LOST로 실패했다. Root18797 exit1/reap, 소유166컨테이너·5helper·3volume 제거와905입력 불변을 확인했다. 정확한 queue/heartbeat 실패 지점은 아직 UNKNOWN이며 lease나 제한 시간을 늘리지 않고 실제 처리 경계를 검토한다. 전체 R5-03은 DOING이다. [정리 증거](../../target/r5-paper/coordinator/r5-03-20261104/sealed-native24-cleanup-review.json) receipt SHA256 `54393d4a3fb80c186fe81df4bb8b5bf7bee860ccd82d45909565ec0c4650ebb0`.


Native24 후속 소스 검토에서 단일 연결 fixture의 금융 검증 트랜잭션과 heartbeat가 같은 풀을 경쟁하고, 연결 획득 오류도 LeaseLost로 분류됨을 확인했다. 실제 Native24의 실패 지점은 UNKNOWN으로 남긴다. 경제 처리 연결/PID를 고정한 채 기존 RunnerServices 인터페이스로 동일 worker 역할의 큐 연결만 분리했다. 실제 role/actor/application/PID와 점유 중 연결 독립성 assertion, 명시 close를 추가했고 제품·SQL·lease·기한·crash guard는 변경하지 않았다. API 빌드·Clippy·Rust fast PASS, 905개 입력을 고정해 새 Native25를 시작했다. 연결 독립성 assertion과 전체 workflow 결과는 실행 후 따로 인수한다.


Native25는 AUTO 적용과 다음 세션 Source까지55phase, 두 SIGKILL 경계 및 재시작 평가를 통과했다. 다음 E의 worker가180초 CHILD_TIMEOUT에 도달했고 진단 당시 실제 컨테이너는 running이었다. 숫자 exit_code0 필드를 성공으로 해석하지 않는다. 마지막 유한 marker는 API_STATE_END8803ms이며 내부 정산/평가 단계와 영속 결과는 UNKNOWN이다. Root68626exit1/reap,200컨테이너·5helper·3volume 정리와905입력 불변을 확인했다. 시간 한도는 유지하고 내부 단계 진단을 보완한다. R5-03 DOING 유지.


Native25 진단 후속으로 API 실행 경계에 고정된 DEBUG 단계 24종을 추가했다. native 검증기는 정확한 target/level/단일 문자열 필드만 받아 phase당 총64개 한도를 유지하며 입력·식별자·금액·자유 형식 오류를 기록하지 않는다. 테스트 본문이 panic하더라도 runtime이 살아 있는 동안 pool을 먼저 닫고 최초 panic을 보존한다. 다음 세션 경제 상태는 정확한 소유 프로세스가 종료된 경우에만 읽기 전용으로 확인한다. 기존 tracing 의존성을 같은 버전 그대로 일반 dependency로 옮겼고 lock은 변경하지 않았다. Root 소스 검토, driver pure30, Rust fast 및 diff 공백 검사 PASS; API 집중 검증과 새 전체 실행은 별도 대기다. Native25의 실제 내부 원인은 아직 UNKNOWN이며 제품·fixture 제한 시간은 그대로다.


Native26은 다음 E의 무거래 정산과 평가를 실제로 commit했고 종료 후 읽기 전용 상태는 SKIPPED/VALUED·equity1이었다. 유한 단계 기록상 bound8716ms→정산 commit56349→평가 commit92734→due 반환92745 뒤, 새 추천 capture97130에서 cycle90초가 만료돼98711ms에 오류가 반환됐다. pool 정리는121019ms에 완료했고 최초exit101을 보존했다. 이전180초 hang의 원인을 소급 확정하지 않는다. Root63386exit1/reap,200컨테이너·5helper·3volume 제거·905입력 불변을 확인했다. 다음 수정은 경제적 진전을 완료한 회차에서 성공 결과를 반환하고 새 추천은 다음 정상 회차의 새로운 Current/계정/권한 검증으로 넘기는 경계다. 기한·원장 원자성·금융 정책은 변경하지 않는다. [정리·관측 증거](../../target/r5-paper/coordinator/r5-03-20261104/sealed-native26-cleanup-review.json).


Native27 전체60phase와 최종checker4·실제두SIGKILL·재시작평가·다음회차AUTO·NO_TRADE가 모두 통과했다. 재시작평가55.554초, 마지막경제적phase94.413초(bound worker85.565초)에서 due뒤추가capture없이정상반환했다. 소유215컨테이너·5helper·3volume·Root13187자식 정리,9작업자idle/대기0,905입력일치와heartbeat삭제확인. [최종 인수](2026-11-05-r5-paper-acceptance.md)에서 코드·격리완료와별도운영gate를구분한다.
