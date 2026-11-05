# R5 게시 검토와 수정 추적 — 2026-11-05

게시 검토는 `c2eb57b4b444fece2bf77b385645f0a3509a7c2f`에서 시작했다. Owner의 커밋·머지 지시에 따라 기존130파일을 보존했다.10개 독립 정적 lens와 별도 Claude 검토 후, 아래13개 수정 후보를 새 검증자가 각각 확인했다. 후속 수정과 검증은 아래에 구분해 기록한다. **2026-11-06 코드·격리 검증 인수를 완료했다. 대상은 `ef0538fa7ff5125ec4dd6fc56fcffc31ce566d32`이며, 완료 기록 문서의 최종 CI·Git 통합은 별도로 확인한다.**

기존 R5-02 actual32/composite와 R5-03 Native27 60단계는 원래 핀·fixture 범위에서 통과한 이력이다. 이를 삭제하거나 새 경계의 검증으로 확장하지 않는다.905 입력 일치는 게시 검토 전의 소스 상태에 대한 것이며 후속 수정의 실행 증거는 별도로 기록한다.

| # | 등급 | 경계 | 수정·검증 상태 |
| --- | --- | --- | --- |
| 1 | P1 | Track compile-time fixtures in versioned paths (`crates/market-data/src/paper_source_evidence.rs:3185`) | versioned 입력·해시/include 및 fresh checkout 빌드·최종 required CI PASS |
| 2 | P1 | Revalidate signal admission before applying a READY preview (`migrations/0067_owner_beta_paper_ledger_v1.up.sql:751`) | 공통 apply admission 검사·독립 검토·철회/ancestor/READY retry를 포함한 실제 PG27 PASS |
| 3 | P1 | Exclude the module helper from Cargo test autodiscovery (`tests/integration/migration-contract/tests/paper_0067.rs:1`) | 명시적 단일 target·전체 workspace/Clippy·최종 required CI PASS |
| 4 | P1 | Decouple execution prices from same-day recommendation success (`crates/api-server/src/paper_session/owner_beta.rs:847`) | T/E 분리 및 NULL E finish 회귀·actual E 승인/추천 미실행38단계·최종 CI PASS |
| 5 | P1 | CWE-367: Keep checked inputs alive through ledger commits (`crates/job-queue/src/owner_beta/paper/evidence.rs:3255`) | historical descriptor 보존91/독립 closure, scoped 금융 소비자·actual entered callback/정리 및 최종 CI PASS; 모든 교체 타이밍의 전수 증명은 아님 |
| 6 | P2 | Honor configured beta preview heartbeat intervals (`crates/api-server/src/paper_session/owner_beta.rs:1195`) | 설정 heartbeat 전달·transaction handoff 수정, 실제 짧은 lease PG witness·금융 Native 및 최종 CI PASS |
| 7 | P2 | Update the ordering guard for the instrumented worker call (`crates/api-server/tests/owner_beta_paper_execution.rs:41`) | 실제 await에 맞춘 순서 guard, 기존 assertion 보존·전체 workspace/최종 CI PASS |
| 8 | P2 | Finalize beta previews when retry budgets exhaust (`crates/job-queue/src/queue.rs:1010`) | owner-scoped terminal projection·job→preview 잠금 순서 및 rollback을 포함한 실제 PG27 PASS |
| 9 | P2 | Correct stale R5 architecture evidence citations (`docs/diagrams/component_architecture.puml:499`) | 현재 코드 근거 대조·paired PNG 로컬 렌더 및 소유 정산 PASS |
| 10 | P2 | Reuse the calendar-link validator in preview capture (`crates/api-server/src/paper_session/owner_beta.rs:1530`) | 공통 calendar validator 연결·집중/최종 CI PASS; 선택된 실제 Source/금융 검증 범위는 아래 기록 |
| 11 | P2 | Bound financial checker SQL waits (`crates/job-queue/src/owner_beta/paper/read_transaction.sql:1`) | 기존15초 statement/5초 lock 연결·집중/최종 CI PASS; 실제 claim/Source lock 및 취소/기한 경계의 선택된 증거 보존 |
| 12 | P2 | Reuse automatic lineage when submitting a manual preview (`migrations/0067_owner_beta_paper_ledger_v1.up.sql:555`) | immutable AUTO/MANUAL lineage 양방향 재사용·실제 PG27 PASS |
| 13 | P2 | Execute ledger regressions in the required test lanes (`tests/integration/migration-contract/tests/paper_0067.rs:1470`) | 필수 hosted PG에서 정확한27개 DB 본문+ordinary source1 실제 PASS, container/PG/stage 정리 확인 |

전체9.9MB diff의 모든 줄을 검토했다고 주장하지 않는다. 리뷰는 금융·checked input/정리·SQL/RLS·API·이행·CI packaging의 선택된 의미 경계를 다뤘다. Claude 실제 model은 receipt의 `claude-opus-5`, 실제 effort는 미확인이다.4개 외부 제안은 보수적 상태 표시·실행 파일 재검증 비용·버전 rollback·cooperative cancellation의 채택 경계와 비교해 주 수정 목록에서 제외했고, 실제 운영 성능·drain은 기존 R5-04 인계로 유지했다.

후속 수정·도표 근거/로컬 렌더와 영향 runtime·hosted 결과를 아래에 기록한다. 코드 후보 `ef0538f`의 필수 CI는 모두 통과했다. 완료 기록 문서의 최종 head CI와 Git 통합은 별도로 확인한다. 운영 Origin/G4/WP7, 활성화·서비스·provider 권한은 별도이며 None4/default-unbound/production CLOSED를 유지한다.

## 첫 수정 — 재현 가능한 빌드 입력

기존 private fixture/ABI 두 파일의 정확한 bytes를 versioned 경로로 복사하고 include·인계 링크를 갱신했다. migration-contract의 자동 테스트 탐색을 끄고 원래 통합 target만 명시했다. due-call 순서 검사의 오래된 문자열을 현재 await 표현에 맞췄다. 원래 정렬 assertion과 fixture 값은 보존했다.0067 끝의 불필요한 빈 줄도 제거했다.

Root가 복사본 SHA 일치, synthetic=true/production_eligible=false 보존, include/link 실제 경로, Cargo metadata의 테스트 target 하나, 정확한 변경 경로를 대조했다. Rust fast `target/dev-check/20261105T020931Z-cf27228d9304`가 통과했다. Rust/PG 동작을 새로 실행한 결과는 아직 아니며 최종 hosted CI와 나머지 수정을 기다린다.

## 두 번째 수정 — 필수 SQL 회귀 실행 경로

기존 PostgreSQL job 뒤에 provider 없는 격리 fixture를 연결했다. 실제 libtest 목록에서 기존9개와 추가0067 사례를 찾아 각 이름을 한 번씩 실행하고, one-pass/zero-ignored 결과를 확인한다. 기존 required 집계·캐시·artifact 제한을 보존했다. Ubuntu24.04 job에서만 고정 libfaketime 패키지를 준비하고, 고정 PostgreSQL 이미지의 프로세스 realtime만 구성한다. 호스트 시계나 제품 SQL 시간을 바꾸지 않는다.

Root 순수12개 테스트와 CI 계약52개가 통과했다. 생성 실패 뒤 inspect 오류가 원래 오류를 덮거나, 정리 중 첫 인터럽트가 소유 자원/보고서 마무리를 중단하던 두 P2도 수정하고 독립 정적 검토로 닫았다. 새 세 회귀는 실제 run_fixture를 호출하되 외부 명령은 mock이며, SIGINT/SIGTERM은 자기 테스트 프로세스에 보낸다. 실제 Docker/PG/ELF/SQL 검증이나 모든 signal interleaving을 증명한 결과는 아니다. cleanup API는120초 잔여 예산을 사용하지만 파일시스템 I/O까지 엄격한 물리적 종료 시간을 보장하지 않는다. 실제 named SQL 결과와 소유 정리는 최종 hosted job에서 확인해야 한다.

기존 policy job에도 같은 제어기 순수 회귀를 연결했다. 실제 workflow의 discovery 명령으로12개, 갱신된 CI 계약52개가 통과했다. 이는 앞 문단의 local mocked 범위이며 hosted 결과는 아니다.

## 세 번째 수정 — historical 실제 reader 보존

승인 결과값만 보존해도 하위 파일 reader가 먼저 닫히던 경계를 분리했다. 새 retained checked 승인 경로는 실제 root/candidate/manifest/bars reader와 이름·inode·원래 bytes/hash를 유지하고, 같은 검증 규칙으로 offset0에서 다시 읽으며 명시적 완료까지 정리 결과를 보존한다. 기존 eager 경로는 유지하고, 보존 graph가 없는 재검증은 거절한다.

고정 Rust1.97.1의 market-data historical 라이브러리90개(실패0/ignored0)와 전체 target/feature Clippy가 통과했다. Root는 두 최종 영수증의 동일391개 입력과 현재 파일 해시, 테스트 실행 파일 SHA `aaab56676f4bb3c1fa87ce1e0230638e4f3891ec1259ac99b80296abbff90120`을 대조했다. 초기 실행 권한·컴파일·Clippy 오류와 수정 기록도 보존했다. 표준 전체 패키지 preflight는 로컬 Phase0/pyarrow 부재로 여전히 blocked이며, 좁힌 라이브러리 검사로 이를 통과했다고 부르지 않는다. 실제 금융 소비자와 blocked SQL 중 변경·정리 검증, 전체 hosted CI는 후속이다.

## historical 후속 검토 — stream 뒤 leaf 재검증

독립 검토가 bars 검증 중 같은 이름의 manifest inode가 교체될 수 있는 마지막 검사 누락을 찾았다. Root는 실제 graph와 descriptor를 쓰는 회귀를 먼저 실행해 기존90PASS/새1FAIL을 관측한 뒤, 원래 reader와 동일하게 마지막 manifest·bars identity 검사를 추가했다. 수정 후91PASS/0ignored와 전체target/feature Clippy가 통과했고, 독립 정적 closure에서 해당 P2 해소·새 P1/P2 없음이 확인됐다. 이전90개 통과는 더 강한 이 경계를 증명한 결과로 소급하지 않는다.

회귀는 실제 bars cursor 끝에서 교체를 주입하며, 후보 디렉터리에 정확히 두 항목만 남긴다. 모든 graph/enumeration reader의 단일 정리 콜백과 기존 정리 순서를 확인하고 진단용 cursor clone도 닫는다. 이는 실제 임시 파일의 ordinary graph/정리 회귀이며 checked native close errno·blocked SQL·모든 동시 파일 변경의 원자성을 입증하는 결과는 아니다. 금융 통합 및 최종CI는 계속 대기한다.

## 네 번째 수정 — 적용 권한과 terminal preview

MANUAL과 AUTO의 새 target 생성 전에 실제 signal admission 사용 가능성을 공통 경로에서 검사한다. source 철회와의 공유/배타 fence를 유지하고, 이미 커밋된 결과의 동일 재조회는 보존한다. MANUAL은 자연 식별자가 같은 AUTO lineage를 immutable 필드 대조 후 재사용한다. 실패·취소된 job은 같은 트랜잭션에서 정확히 연결된 미완료 preview만 종료하며 caller actor와 이미 발행된 결과를 보존한다. 후속 독립 검토가 찾은 job/preview 잠금 역전도 기존 권한·오류 의미를 유지하면서 job→preview 순서로 수정했다. down은 영속 이력 거절 뒤 원래 callback을 복원한다.

대상은 이 기록을 추가한 커밋의 migration/ABI/test4파일이다. Rust1.97.1 no-run 빌드, 실제 source 테스트1개, 전체 target/feature Clippy와 rustfmt가 통과했다. Root는 각 영수증의360개 입력과 현재 파일, 최종 ELF `3d27cdbb64304664ade1701804b27d8f5a8f384dd614a57a9c859207620e9a4b`를 대조했다. 독립 정적 검토는 원래 세 SQL 수정과 잠금 역전의 해소를 확인했다. 새 persisted READY AUTO 사례는 앞선 세션의 실제 정산·평가, 유효한 적용의 trial/rollback, signal 철회 후 무변경 거절을 요구한다. 이 사례의 잘못된 `SETTLED` 기대값은 실제 SQL 상태 `SETTLED_TRADE`로만 고치고 영향 검사를 다시 통과했다. 이전 실패·검토 기록은 보존한다.

컴파일된 DB 사례27개는 모두 ignored이며 이번 집중 검사에서 본문을 실행하지 않았다. 최종 required PostgreSQL job이 각 이름을 실제 실행해야 한다. Source/단위 검증을 실제 DB나 새 금융 통합의 통과로 확대하지 않으며 R5-03 DOING과 production CLOSED를 유지한다.

## 다섯 번째 수정 — 계산·원장 commit까지 금융 입력 보존

네 실제 소비자(AUTO queue, preview, settlement, valuation)가 local 비동기 checked action 안에서 실행된다. 실제 Current/config/Source/Raw/historical/sealed Daily reader를 유지하고, 경제 트랜잭션 안에서 SQL 식별자·파일·시계를 다시 확인한 뒤 commit한다. 소유 transaction은 한 번만 소비하고, commit 시작·성공과 후속 검사/정리 오류를 함께 보존한다. 호출자 소유 pool은 닫지 않는다. 행동 입력은 정확한 성공 T와 승인된 E 가격을 검증하며 E 추천 성공이나 최신3세션 진단 조건을 추가로 요구하지 않는다. 활성화/진단의 기존 eligibility 조건은 유지한다.

후속 정적 검토가 E의 NULL finished_at을 여전히 거절하던 경계와 하위 오류/정리 결과를 코드로 축약하며 원본을 버리던 경계를 찾았다. 첫 ordinary red는116PASS/3FAIL/17ignored였고, 두 새 결함과 기존 정리 순서 기대값 하나를 구분했다. 수정 후 complete E 무실행·pending wire와 T finish 거절, 실제 driver 실패 상태·비복사 오류의 heap identity/drop·실제 close callback/completion·최종 시각 보존 회귀가 통과했다. 정리 helper는 바꾸지 않고 저장된 session 순서에 맞춘 기존 assertion도 유지했다. 기존 unwind의 Abandoned 처리를 확인해 중복 guard는 추가하지 않았다.

집중 코호트에서 job 단위120·문서31(checked-action compile-fail5 포함)·Owner-beta API7·runner6, 총164개가 통과했다. job-queue/API 전체target·feature Clippy와 Rust1.97.1 포맷도 통과했다. Root는 최종7영수증의 before/after와 현재1059개 입력, job ELF `5cb9d1f356a1c425a3c3285bd8b0c00c87968e90a3fea05627b0702d4e1586b8`, API ELF `edc926b3147201ed80a2721445ccb9b00c095e1d0ee752195c226059d6329d4e`, 빌드 전용 Native ELF `593f4d4ab79453310d4e8b930a85e9d118133f9a4e5c2fcc77b8d90ec9a58fc1`을 대조했다. 최초 sandbox EROFS는 Cargo 미실행으로 보존했고, 동일 shared slot/명령의 제한된 승인 뒤 나온 컴파일·lint 오류와 수정 기록도 보존했다. 전체 접근 모드나 다른 lock으로 우회하지 않았다.

이 검증은 이후 테스트 보조 경로를 추가하기 전의 고정 코호트다. ignored job17개와 실제 Native/PG 본문은 실행하지 않았다. genuine E 무예약·실패, 실제 SQL 대기 중 입력 교체와 전체 경제 행 불변, 취소/마감/commit 불명 상태의 완료·정리, 오래된 E의 공개 RECOVERY, 실제 짧은 heartbeat/lease, calendar 대조 및 최종 full workflow 검증은 별도다. 추가한 private cfg(test) 소비자 진입 경로와 실행기 연결의 최종 타입/런타임 검증까지 마친 뒤 Git 통합 상태를 갱신한다.

## 인증 factory를 사용하는 확장 Raw fixture

기존 3·5거래일 exporter의 모든 bytes를 보존하고 테스트 전용18거래일 경로를 추가했다. 실제 credential/token/client/provider/parser/RawStore 코드를 호출하지만 transport와 credential은 합성이다. 외부 인증·공급자 요청·Root Origin을 주장하지 않는다. 한 번에 선택된 날짜의 EOD만 쓰고, 명시적 합성 calendar와 누적 Source metadata의 날짜·순서를 확인한다.

기존 authenticated factory와 새 날짜/calendar 및 닫힌 plan/prefix 검사3개가 통과했고 exporter3개는 ignored로 유지했다. market-data 전체target·feature Clippy, 최종525개 입력과 ELF `f42eaecd4c2205131448a08633393e69e6225865d41135948faf24bf18a30cab` 대조를 통과했다. active job/API 파일은 이 패키지 의존 경로에 포함되지 않았다. 실제18일 Raw/DB 구축이나 금융 Native를 실행한 결과로 확대하지 않는다.


## 실제 통합 재검증 — preview 경계 진단 중

후속 helper를 포함한 고정 코호트에서 ordinary·문서165개와 두 패키지 Clippy가 통과했다. 실제 Rust 컴파일 기록에서 adopted fixture revision `Some(a30edacff987d55b5be7cca25bed7faeec589629)`를 확인했다. 이 값은 격리 fixture의 검증 기준이며 운영 immutable release provenance를 대신하지 않는다. 이전 compiled-None 실패와 수정 전후 영수증은 보존했다.

새 baseline 통합은 실제 3거래일의 Raw 생성, Source 연결, Daily 생산·admission, 추천 계산·publication과 SQL run 연결을 통과했고, Paper ENABLE 뒤 인증된 preview POST에 도달했다. 이 요청이 HTTP500/INTERNAL로 끝나 전체 workflow는 실패했다. 제출과 후속 조회 중 어느 경계에서 실패했는지 아직 입증되지 않았으므로, 제한 시간을 늘리거나 성공으로 처리하지 않았다. 실패 시도의 소유 컨테이너·볼륨·비밀 파일 정리와 부모 실행 종료를 확인했다. 과거 60단계 성공은 이 후보의 통과 증거로 대체하지 않는다.

원인 분석은 고정79개 입력의 정적 경로를 확인했으나 원인을 확정하지 못했다. Root는 debug fixture에서만 닫힌 단계·SQLx 종류·5자리 SQLSTATE를 기록하는 진단을 추가했고 영향 API 빌드·Clippy 검증 중이다. 오류 본문·SQL·URL·비밀·금융 행은 수집하지 않는다. 재실행과 최종 hosted CI가 남아 있다.


실제 진단 재실행은 `SubmitSql`에서 Database/SQLSTATE `42702`를 확인했다. 새 MANUAL lineage 재사용 조회의 `account_id`가 함수 인수와 충돌하는 지점을 Root가 테이블 별칭으로 명확히 지정했다. 경제 정책·잠금 순서·권한·제한 시간은 바꾸지 않았다. 임시 진단용 Rust 두 파일은 원본으로 정확히 복원했고, 실제 오류 코드와 진단 diff는 private 영수증에 보존했다. 수정 후 영향 컴파일 코호트와 실제 Native 통과는 확인 중이다. 이 실패 시도도 소유112개 컨테이너·역할 helper5개·볼륨3개·비밀 파일 정리 및 부모 실행 정산을 확인했다.


최종 SQL 별칭 수정 코호트에서 job120·문서31·Owner-beta API8·runner6, 총165개와 두 패키지 전체 target/feature Clippy가 통과했다. Root는 1060개 현재 입력, 원본7영수증과 세 실행 파일·컴파일 리비전·selector 목록을 대조했다. SQL 수정 자체의 실행 성공은 별도 Native 결과로 판정한다.

아키텍처 근거는 실제 retained reader 획득·금융 callback/commit·완료 호출 위치로 갱신했다. R5 관련 인용106개의 경로/줄 대조와 선택된 변경 의미 검토를 마쳤다. 두 PNG를 고정된 로컬 이미지와 네트워크 없는 컨테이너에서 다시 렌더링했고 컨테이너 종료·부재를 확인했다. 출력은 component11107×1733, runtime16000×2174이며, runtime의16000은 원본의 명시적 축척 제한이다. Source/PNG의 일치이며 설치된 운영 토폴로지 인수는 아니다.


## 실제 preview 수정 확인과 claim 경계 분리 — 2026-11-05

`3313024`와 `b737f67`에 금융 입력 보존 구현·SQL 이름 충돌 수정 및 로컬 도표 증거를 저장했다. 후속 격리 실행에서 실제 authenticated preview POST202와 조회200을 관측해 이전 SQL42702 거절은 닫혔다. 그러나 preview worker는 commit 직전 `PRECOMMIT_CLAIM_LOST`와 runtime 거절을 남겼으며 READY에 도달하지 못했다. 이 실행을 전체 Native 통과로 표현하지 않는다. 소유 컨테이너112개·역할 helper5개·볼륨3개와 직접 CLI의 종료·정리를 회수했다.

현재 Native fixture는 heartbeat1초·lease5초를 사용한다. 경제 트랜잭션이 job/attempt 행 잠금을 보유한 동안 별도 heartbeat가 같은 job을 갱신하는 경로가 소스에 있지만, 해당 실행의 정확한 heartbeat SQL 오류·SQLSTATE·대기 backend는 아직 관측되지 않았다. 실행 시간만으로 lock timeout을 단정하지 않는다. 기존 timeout이나 lease를 늘리는 대신, 기존 일회용 DB fixture와 실제 `JobQueue::heartbeat`·`lock_claim_in` 및 heartbeat의 결과 매핑을 연결하는 한 번의 작은 PG witness를 준비한다. 정상 갱신·실제 blocker·typed 결과·자원 정리를 확인한 뒤 필요한 프로토콜 수정을 결정한다. 운영 Origin/G4/WP7 및 production CLOSED 경계는 유지한다.


## 실제 job 잠금·heartbeat witness — 2026-11-05

`b737f67` 위 진단·test-only observer와 기존 queue/common fixture를 연결한 별도 PG 실행에서 실제 `JobQueue::heartbeat`와 `lock_claim_in`의 충돌을 확인했다. 대상 API 소스 해시는 `3557ad71…`와 `08441b5b…`, 최종 실행 파일은 `a7653b2fc69236ac1c22fd5d371daa7e62c84cf30e8df81bc5dbbc8e52f1d8fe`였다. 해당 코호트의 빌드·일반9개/ignored2개·Clippy와 동일 최종 ELF의 selector 목록을 확인한 뒤, nonce `1f6824dd693386b857018fc53c33ba7f`에서 ignored 본문1개가 실제 통과했다.

정상 heartbeat 갱신을 먼저 확인했고, 별도 owning worker 트랜잭션이 잡은 정확한 job/attempt 잠금에 heartbeat backend가 실제 대기했다. 기존5초 lock timeout의 `Database/55P03`이 실제 heartbeat 결과로 돌아왔으며 sticky loss가 true가 됐다. job과 attempt는 RUNNING 상태를 유지했고, 마지막 committed lease anchor는 변하지 않았다. owning 트랜잭션의 `transaction_timestamp()`는 약6초 동안 고정된 반면 실제 DB 시각은 진행했다. 기간1초·lease5초·lock5초·statement15초를 변경하지 않았다.

이 결과는 안전하지 않은 동일 행 갱신 조합을 직접 입증한다. 이전 Native07의 정확한 오류를 이 관측으로 소급 확정하지 않는다. 경제·terminal-success 행을 만들지 않았고 owning rollback, heartbeat join, pool close, scratch DB 부재를 확인했다. Root parent42058은 exit0으로 회수했으며 소유 컨테이너2개·CLI26개와 staging 정리가 완료됐다. 전체 금융 Native·최종 CI·merge는 계속 대기다. 다음 수정은 기존 입력 검사를 보존하면서 heartbeat 종료·회수 뒤 실제 트랜잭션 lease와 최종 commit fence를 연결하는 범위다.


## claim transaction 인계 — Source와 실제 PG를 구분한 검증

`b737f67` 위 후속 Source6 동결 코호트(API `24d1a179`, queue `67531b32`, checker `77bd5dca`)에서 실제 heartbeat를 경제 transaction 잠금 직전에 quiesce하고 join하도록 연결했다. 전용 opaque lease는 실제 backend/TX·owner/job/attempt와 DB의 fresh clock을 대조하고, live claim만 원래 기간으로 갱신한다. 원래 queue 성공 정산은 마지막 같은 transaction hook으로 넘기며, 정산 뒤에는 terminal lease를 갱신하지 않고 마지막 live anchor를 확인한다. 하위 오류·정산 결과와 commit 불명 상태를 그대로 보존한다. generic heartbeat/정산 API와 기존1초 heartbeat/5초 lease·SQL15초/잠금5초는 변경하지 않았다.

이 동결 코호트의 집중 검사는 job 단위123·문서33·API15·runner6, 합계177개와 두 패키지 Clippy가 통과했다. 집중 실행에서 ignored21개 본문은 제외됐다. Root는 최종1061개 입력·원본 영수증·동일 실행 파일의 selector 목록과 여섯 파일의 완전한 역변환을 대조했다. 이전 실패와 수정 이력은 보존했다.

별도로 nonce `dfc1abefc7102230caadd15fe3a6c51f`의 실제 PG에서 정상 join 뒤 live lease를 약3초·6초에 갱신하고, 전체6.018초 동안 DB anchor가 전진함을 확인했다. 다른 backend와 같은 backend의 새 transaction은 mutation 전에 거절됐고, 만료된 claim은 anchor를 바꾸지 않고 거절됐다. 원래 transaction 시각은 고정된 반면 실제 DB 시계는 전진했다. 금융·terminal-success 행을 만들지 않았으며 실제 rollback, heartbeat join, pool close와 scratch DB 제거를 확인했다. Root parent98580 exit0/reap, 두 컨테이너·26 CLI 자식·stage 정리를 대조했다. 이는 transaction 인계 primitive의 증거이며 전체 금융 commit 통과가 아니다.

후속 독립 정적 검토는 새 P1 없이 P2 두 건을 확인했다. 마지막 lease/정산 SQL 대기 뒤 취소·마감·Seoul 날짜 veto가 없던 경계와, callback panic 시 heartbeat join remainder보다 전용 실행기가 먼저 종료될 수 있는 경계다. 전자는 Source3의 값싼 최종 veto로, 후자는 실제 async owner 안에서 원래 panic을 잡고 join 후 다시 전파하는 한 파일 수정으로 처리 중이다. 해당 두 수정의 최종 컴파일·실행과 영향 금융 Native, required SQL27 및 전체 current-head CI는 아직 완료되지 않았다. R5-03 DOING·production CLOSED와 운영 Origin/G4/WP7의 별도 OPEN 경계는 유지한다.


## 최종 commit 경계와 전체 실행의 남은 실패

마지막 awaited claim 검사·settlement/seal 뒤 실제 commit을 시작하기 직전에 취소·기한·서울 날짜를 동기식으로 다시 확인한다. 거절하면 기존 rollback 경로와 원래 오류/후속 정리 결과를 유지한다. panic 경로도 heartbeat 작업의 실제 종료와 join 뒤 원래 panic을 전달하도록 보완했다. 독립 정적 검토에서 두 P2 해소와 새 P1/P2 없음이 확인됐다. 고정된 최종 코호트는184개이며, 새로 실행한 API22개와 완전한850개 입력이 일치해 재사용한 job/문서162개를 합친 수다.184개를 모두 새로 실행한 결과로 세지 않는다. 두 패키지 Clippy와 최종1061개 입력·실행 파일 대조도 통과했다.

실제 Native08(`4192cdc2087c66ed8a2db7c52a3708f5`)은3거래일의 Raw→Daily 생산/admission→추천 계산/publication→Source SQL 연결과 bootstrap을 통과했다. 보호 프로세스의 preview POST202와 실제 발행 응답 뒤 worker cycle이 실패했다. `BETA_BOUND_BEGIN`26711ms부터 `CYCLE_RETURN_ERR`116709ms까지 약90초였지만 기존 테스트가 실제 RunnerError를 버렸으므로 정확한 오류 종류는 UNKNOWN이다. 시간 제한·lease를 늘리거나 검증을 생략하지 않았고, 이 후보의 금융 전체 실행을 통과로 판정하지 않는다. 앞서 성공한 queue transaction primitive를 전체 금융 통과로 확대하지 않는다.

실패 부모35534의 exit1/reap, 정확한 소유112개 컨테이너·역할 helper5개·볼륨3개·시작한 CLI1227개의 종료/회수, 비밀 파일 제거와1099개 입력 해시 불변을 확인했다. 제어기가 남긴 코드 stage는 nonce·소유자·정확한7개 파일 목록을 대조한 뒤 제거했다. 이는 해당 시도의 소유 정리 증거이며 다른 프로세스의 전역 정지나 과거 UNKNOWN 해소를 뜻하지 않는다. 원래 실패와 Root 정리 검사기의 schema 읽기 오류/정정 기록도 보존했다.

테스트가 원래 오류를 opaque typed payload로 정리 뒤까지 보존하고 닫힌 오류 종류만 기록하도록 진단을 보강했다. 오류 본문·SQL·URL·사용자 입력을 출력하지 않는다. 진단 순수 parser 검사를 통과했고, 후속으로 앞 beta 단계와 마지막 오류를 함께 보존하도록 보강했다. R5-03은 DOING이며 실제 영향 Native, 필수 SQL27/전체 hosted CI와 최종 머지는 아직 완료되지 않았다.


2026-11-06 KST 진단 코호트에서 API16·runner7·순수 Native 진단6, 총29개와 Clippy가 통과했다. 독립 고정 분석이 확인한 beta 결과→다음 generic preview의 오류 가림 경로를 구분하도록 닫힌 두 단계 종류만 기록한다. 앞서 나온 엔진 progress가 결과를 지우지 않도록 기존24개 상한과 별도로8개 결과 상한을 사용하며, 원래 결과 처리·제한 시간·lease·금융 계산은 유지했다. 실제 subscriber/비밀 필드 거절/원래 typed 오류의 정리 후 보존 회귀와 실행기 순수20개가 통과했다. Root는5개 최종 명령,1061개 현재 입력과 실행 파일을 대조했다. job/문서162개는 완전한850개 입력이 일치하는 기존 결과를 재사용한다. 합계191개를 모두 새 실행으로 표현하지 않는다.

초기 shared-slot EROFS는 Cargo 미시작으로 보존했고 동일 명령의 제한된 재실행이 통과했다. 실제 금융 Native와27개 DB 회귀 본문은 이 집중 검사에서 실행하지 않았다. 먼저 이 수정 코호트를 커밋한 초안 PR에서 필수 hosted CI를 병행하고, 별도 새 nonce·자원·마감에 고정한 한 번의 금융 진단 실행으로 실제 원인을 확인한다. 이전 실패를 통과로 바꾸거나 Native 마감을 연장하지 않는다.


## 초안 PR와 현재 CI/통합 경계 (2026-11-06 KST)

`653067008b008a52144b8353ff68faa093d3538d`까지11개 커밋을 push하고 [초안 PR10](https://github.com/sihioov/Lagrange/pull/10)을 만들었다. 금융 통합과 필수 CI가 미완료이므로 초안이며 머지하지 않았다. 이 상태에서는 자동 PR 감시를 시작하지 않았고, 준비 완료 뒤 동일 head의 필수 CI/리뷰를 ce-babysit으로 확인할 계획이다.

Native09은 새로 추가한 진단 테스트3개가 private 실행기의 정확한 목록에 없어서 금융 본문 시작 전에 거절됐다. 부모6617 exit1/reap,3컨테이너/2볼륨/41CLI와 정확한7개 stage 파일을 정산했다. 목록을 정확히6개 순수 테스트+1개 workflow로 갱신했으며 추가·중복·누락 거절을 유지했다. 실제 메서드로 기존 오류를 재현했고 순수22개·자체검사30개가 통과했다. 제품 Source와 기한을 바꾼 수정은 아니다.

새 Native10(`8c5f77fb9832799849a3acf0c49cbac1`)은3일 계산/Source SQL 연결 뒤 worker cycle에서 `BETA_STAGE_TIMED_OUT`와 `CYCLE_DEADLINE_PREVIEW`를 같은117097ms에 관측했다. cycle 시작은27096ms였다. 앞 beta 단계가90초를 소모하고 다음 preview의 마감 오류로 나타나는 경로를 구분했으며, 읽기·해시·SQL 중 무엇이 시간을 소모했는지는 아직 별도다. PublishReply는 SQL 응답 직후의 marker이므로 실제 성공 판정/COMMIT 증거로 세지 않는다. 부모34165 exit1/reap·112컨테이너/5helper/3볼륨/1227CLI·7개 stage 정리와1099입력 불변을 확인했다. 새 금융 재시도 대신 실제158101280바이트 코드 파일로 제한된 읽기/해시 진단을 마쳤다. SHA-2 0.10.9 기본 디버그 해시3회의 중앙값은2.114323초, 해당 패키지만 opt-level3인3회의 중앙값은38.115793ms였으며6개 digest가 모두 원래 핀과 일치했다. 읽기는 각 설정에서 약71ms의 한 번 관측이며 캐시가 데워진 값이다. 단독 패키지는 전체 workspace의 feature-unified subtle 의존성이 빠져 있으므로 완전한 환경 동등성이나 전체 시간 원인으로 확대하지 않는다. 제품 manifest·파일 검사·90초 제한은 유지하며 private dev/test 실행 파일만 버전 한정 SHA-2 opt3로 빌드하고 실제 root lock/기능 구성·회귀·새 통합 실행으로 확인할 예정이다.

[첫 CI 실행](https://github.com/sihioov/Lagrange/actions/runs/42053973691)은 head653에서 Web·Clippy·format을 통과했지만 Python·policy·PostgreSQL·workspace-tests 및 required는 실패했다. QA 테스트의 기대 workspace 목록에 새 `r5-close-custody`가 빠져 있었고, 두 systemd 단위 테스트의 동적 Mock이 존재하지 않는 `_custody_intent`를 만들어 잘못된 opt-in 경로로 진입했다. 각각 로컬 red/green으로 재현·수정했고, 기존 package 거절과 unit 소유권/실패/정리 assertion을 보존했다. 두 격리 full-schema fixture도 여전히65개 migration만 허용해, 현재의 정확한67개 연속 목록으로 갱신했다. 해당 Docker/PG 검사는 로컬에서 실행하지 않았으며 새 hosted 결과를 기다린다.

PostgreSQL 작업의 R4 metadata 관측기는 R5가 추가한 `paper_0067_project_terminal_preview` jobs 훅 때문에 기존 닫힌21개 forward 목록을 초과한다. 상한만 늘리면 기존 정확한 attachment/root 검사도 실패한다. 추가한 훅과 root·15초 설정을 정확히 검증 목록에 포함하고, 모르는/이동한/변조된 훅 거절과 기존 timeout profile을 보존하는 Source 수정을 마쳤고 일반35개·all-target/all-feature Clippy가 통과했다. Root는 두 전체 diff,864개 일치 입력과 같은 최종 ELF를 확인했다. 실제 실패 subquery/카탈로그 rowset은 CI가 기록하지 않아 Source의 충분한 호환성 원인과 구분한다. 기존 policy ID·fingerprint·Owner 문자열, 금융·권한·운영 비활성 경계는 유지한다.

워크스페이스 검사에서는 두 추가 fixture 불일치를 확인했다. 실행 순서 source guard가 이전 `run_targets_once(context, &selector)` 인자와 이동 전 SQL 위치를 찾고 있었다. 새 checked AUTO 소비자의 진입·완료 뒤 preview claim 순서를 검사하도록 수정안을 준비했다. 별도 R4 rollback 테스트는 전체67 schema의 콜백을 과거0063 기준선과 비교했다. 새 scratch DB에서 이력이 없는0067의 원래 guarded down을 먼저 실행해 정확한 R4 body를 복원한 뒤 기존 모든0063/0064 rollback·권한·이력 거절 assertion을 유지하는 수정안이다. 카탈로그 작업의 실제 final/idle/pending0와 입력 검증 뒤 두 테스트 수정안을 적용했다. 기존 R4 rollback assertion은 모두 보존했고,0067 guarded down의 복원 body가 기존 정확한 R4 기대 body와 같음을 정적으로 대조했다. DB 본문 결과는 새 hosted CI에서 확인한다.

새 head `5cf3447`의 hosted CI에서 추가한 R4 회귀 테스트를 기존 검증 빌드의 실행 명단에도 등록해야 함을 확인했다. Python은856PASS/10SKIP/해당1FAIL이며 policy에도 같은 전체 명단 비교가 있다. Root는 그1개를 기존 순서에 추가했고, 기존 system Python/PyYAML로 CI 계약·cache59개와 shell 문법을 통과했다. 첫 기본 Python은 PyYAML 부재로 테스트 전에 멈췄으며 설치 없이 기존 도구로 정정했다. 제품 Rust의 compiler 입력은 바꾸지 않았고 현재 compiler 완료 후 커밋한다.

후속 private artifact 코호트는 실제 root lock과 기능 구성을 사용해8개 집중 명령을 모두 통과했다. 일반·문서·순수 진단191개를 새로 실행했고 두 package Clippy도 통과했다. Root는8개 원본 영수증·현재850/1061 입력·471개 Cargo profile 그룹·같은 최종 ELF의 목록·실제 Some(a30edac) env-dependency를 확인했다. SHA-2 0.10.9만 opt3/default,std이며 parent와 다른 의존성은 opt0, debug assertion/overflow 검사는 유지한다. 이는 제품 manifest 변경이나 실제 Native 통과가 아니며 실제 금융 검증은 별도다. 보고서 Markdown의 unowned 변경 목록에서 workflow가 빠진 작은 표기 편차는 JSON/최종 통지의4개 목록으로 대조했고 봉인본을 수정하지 않았다.

CI5cf의 기존 PostgreSQL/R4 검증 단계는 통과했지만 새 Paper 명단28개(ignored DB27+일반 source1)는 모두 FAIL로만 요약됐다. 해당 요약만으로 실제 DB assertion 실패, 실행 준비 실패, 결과 판독 오류를 구분할 수 없다. 기존 결과의 숫자 exit_code·닫힌 fixture error_code와 소유 container/PG/stage 정산 상태만 공개 요약에 추가했다. raw 로그·SQL·환경 값은 노출하지 않는다. 이 관측 전용 workflow 수정 후에도 CI 계약/cache59개와 YAML/Python AST가 통과했다. 실제 오류 분류와 최종 hosted 전체 검증은 후속이다.


## 실제 금융 baseline 통과와 추가 경계 (2026-11-06 KST)

head `b8a0bfcd00dd50adab946d648705be2d85750e15`의 고정 Source와 private root-lock artifact에서 Native11(`48f93e5459073cce4fd31a55d6c3e091`) 전체60단계가 통과했다. 실제 Raw→Daily/admission→추천/publication→Source 연결, 금융 preview/apply/settlement/valuation, 두 crash/복구와 최종 checker 네 경계를 포함한다. 보호 프로세스는22956ms에 정상 종료했다. 파일 재검사·90초 fixture 제한·1초 heartbeat/5초 lease·권한과 경제 규칙은 유지했다. 이 결과는 합성 fixture의 기능 인수이며 기본15초 운영 성능이나 실제 Source Origin 인수가 아니다.

부모69579 exit0/reap, 정확한 소유215컨테이너·5역할 helper·3볼륨·시작 CLI2753개의 종료/회수와 비밀 파일·7개 코드 stage 제거를 확인했다.1102입력도 정리 시점까지 일치했다. 이전 Native10 실패는 그대로 보존한다.

추가 E 승인/추천 미실행 Native12(`e055d5050a236a67d2dad0e912c98f29`)는37개 선행 단계를 통과했지만 `api-admitted_e_no_run_settle`의 worker 진입 전 fixture assertion에서 실패했다. 같은 연결의 실제 admission readback/rollback 뒤 actor를 반드시 None으로 가정한 검사였으며 기존 복원 검사는 None과 빈 문자열을 모두 허용한다. 실패 실행의 실제 actor 값은 기록되지 않았다. 이 불일치를 고쳐 외부 actor 값은 계속 거절하고 ordinary 회귀를 추가했다. 제품 SQL·권한·actor 설정은 바꾸지 않았다. 새 Native 검증은 별도다. 부모97536 exit1/reap과140컨테이너/5helper/3볼륨/1482CLI·비밀·7stage 제거,1102입력 일치를 확인했다. 해당 admission은 소모됐으며 재사용하지 않는다.

## Hosted Paper fixture의 파일 크기 제한 범위

[head b8의 CI](https://github.com/sihioov/Lagrange/actions/runs/42072184279)는 Python·policy·Clippy·workspace-tests·format·Web에 성공했고 PostgreSQL과 required는 실패했다. 새로 닫힌 숫자 witness를 확인하니 Paper28개 모두 `NAMED_TEST_RECEIPT_MISSING`이며 개별 libtest 본문 영수증이 없었다. PG exit125/reap UNKNOWN, 소유 컨테이너 terminal/제거와 input/output stage 정산은 확인됐다.27개 SQL assertion 실패로 세지 않는다. 실제 initdb stderr는 공개 witness에 없어 해당 실행의 정확한 오류 원인은 미확정이다.

독립적으로 guest의 전역 `ulimit -f 4096`이 initdb와 PostgreSQL에도 상속되는 충돌을 확인했다. 로컬 같은 POSIX 셸은 RLIMIT_FSIZE=2097152와 실제 EFBIG를 보였고, [PostgreSQL18 initdb 문서](https://www.postgresql.org/docs/18/app-initdb.html)의 기본 WAL segment는16MiB다. 제한을 named test를 exec하는 private subshell로 옮겼다. 테스트 명령·로그·종료값·나머지 시간/메모리/소유 정리 계약은 유지했다. 기존12개 테스트 AST를 모두 보존했고 새 구조·실제 kernel 상속 회귀2개를 포함한14PASS, sh/bash 문법 검사를 통과했다. 부모의 전후16MiB 쓰기와 한도 불변, 제한된 자식의 EFBIG/종료·회수를 확인했다. 이것은 수정의 순수·커널 검증이며 실제 PostgreSQL/hosted 통과는 새 head에서 확인해야 한다.


Native13(`81cbf51ea2bdbcca9a39b726fd80a4f1`)에서는 E일 admission·candidate·admitted_at과 추천/run/job/attempt/finished_at NULL·reservation0을 실제 SQL로 확인한 뒤 실제 정산·평가 commit 반환과 `BETA_STAGE_OK`를 관측했다. 다만 같은 phase의 두 번째 idempotence cycle이 PID 증거 파일을 다시 `create_new`로 열면서 실패해 전체 사례는 FAIL로 보존한다. 실제 첫 cycle 이후 `VALUED`·settled/valued readback assertion까지 통과한 경로다. phase 증거 파일은 처음 한 번 발행하고 두 기존 replay 호출은 실제 현재 DB 연결·actor와 전후 복원 검사, 원래 금융·outbox·중복 거절 assertion을 유지하는 fixture 한 파일 수정으로 처리했다. 제품 Rust·SQL은 바꾸지 않았다. Root의 같은 pinned launcher/HeavySlot/jobs2 집중 빌드·순수7·all-target/all-feature Clippy와1061입력·최종 ELF 목록8개(순수7+workflow1)를 확인했다. 원래 지시의 “workflow selector3개”는 실제 하나의 selector로 세 group을 실행한다는 뜻으로 정정했고 없는 이름을 만들지 않았다.

Native13 부모48343 exit1/reap·140컨테이너/5helper/3볼륨/1482CLI·7stage 제거와1102입력 전후 일치를 확인했다. 이전 admission은 소모됐으며 새 artifact와 별도 새 패킷으로 E idempotence를 다시 확인한다. Native11 baseline은 제품 변경이 없어 기존 인수를 유지한다.


Native14(`c145ff7427631c422bd67c0fc69741ca`)의 E 승인/추천 미실행38단계가 전부 통과했다. E run/job/schedule/compute/run-link 없음과 admission 연결을 실제로 확인하고, 정산·평가 한 번 및 재실행의 경제 상태/outbox 중복 없음 assertion을 완료했다. 부모76615 exit0/reap,143컨테이너/5helper/3볼륨/1510CLI·7stage 제거와1102입력 일치를 확인했다. Root 정리 검사기가 성공 outcome을 None으로 가정해 첫 시도에 멈춘 오류도 보존했다. 실제 성공 dict를 대조한 뒤 정리했다.

Native15(`a2467620fa4c82c55fe928e447c1b04a`)는 보호 프로세스와 첫 local non-Send SQL probe 본문을 통과했지만, 실행기가 직접 checked-action 호출에도 worker cycle의 PID 파일을 요구해 후처리에서 실패했다. 첫 판독에서 본문 미실행으로 추정한 Root 보고는 exact one-pass guard 뒤 actor 호출이라는 코드 경로로 정정했다. 나머지 세 본문은 미실행이다.115컨테이너/5helper/3볼륨/1252CLI·7stage 제거,1102입력 일치를 확인했다. worker cycle을 호출하지 않는 정확한 네 phase만 PID 관측을 해당 없음으로 기록하고, 실제 worker cycle의 필수 PID 검사는 유지하는 private 실행기 보정을 적용했다. 실제 api_phase 메서드로 네 phase 실패를 재현한 뒤 원래22개 AST를 보존한24개·자체30개를 통과했다. Rust·SQL·ELF 변경은 없으며 새 입력/nonce로 해당 네 경계를 다시 검증한다.


## SQL 대기 관측의 역할과 마지막 fixture 검토

Native16(`92d3a7abe12fbcfaed80a5252ed144ff`)은 non-Send SQL probe를 정상 기록했고 실제 취소 consumer의 backend PID까지 전달받았다. 이후 `wait_for_action_sleep`의 관측 대기가 마감돼 취소/기한/마지막 commit 사례 전체는 미완료다. 실제 pg_stat_activity 행은 수출되지 않았다. 소스를 대조하니 관측 pool은 NOSUPERUSER `migration_owner`, 실제 consumer는 `worker`이고 통계 역할·멤버십 grant가 없다. [PostgreSQL18 문서](https://www.postgresql.org/docs/18/monitoring-stats.html)는 일반 역할에서 다른 역할 세션의 상세 필드가 제한됨을 명시한다. 이 조합에서 state/query를 요구하는 관측 조건은 성립할 수 없다. 이미 있는 별도 `queue_worker` 연결을 사용하고 exact PID/current_user 조건을 추가하는 test-only 한 함수 수정으로 처리했다. 새 역할·권한, 제품 SQL·consumer·90/20초 기한·12초 sleep은 바꾸지 않았다. 같은 pinned launcher/HeavySlot/jobs2의 빌드·순수7·all-target/all-feature Clippy,1061개 입력과 같은 최종 ELF를 대조했다.

부모39968 exit1/reap,115컨테이너/5helper/3볼륨/1254CLI·7stage 제거와1103입력 일치를 확인했다. 별도 신규 Native는 아직 시작하지 않았다. 남은 마지막 fixture의 취소 시점이 강화된 final commit veto와 일치하는지도 기존 Sol/high의 동결9핀 분석으로 확인 중이다. 실제 성공을 만들기 위해 guard를 늦추거나 commit/outbox 상태를 꾸미지 않는다. 새 hosted PG 확인을 위해 검증된 CI 파일 제한 및 현재 fixture 보정을 먼저 게시하며 PR은 초안·R5-03 DOING이다.


## 실제 COMMIT 중단 경계 fixture 보정

동결9핀 독립 검토로 기존 fixture의 barrier가 마지막 취소 veto보다 앞에 있음을 확인했다. 그 지점에서 취소하고 commit 성공을 기대하는 검사를 유지할 수 없어, 제품 veto를 그대로 둔 채 테스트의 관측 지점을 실제 서버 COMMIT 대기로 옮겼다. 새 nonce 전용 schema의 deferred FK와 두 작은 테이블만 사용하며 기존 worker에는 USAGE·parent SELECT·marker INSERT만 부여한다. 실제 COMMIT/Lock을 같은 역할의 별도 연결로 관측한 뒤 취소하고, 소유 blocker를 반드시 rollback한 뒤 결과와 정리를 확인한다. 이것은 비금융 marker 쓰기의 commit/취소 경계이며 입력 파일 교체 시험으로 확대하지 않는다.

Root의 영향 빌드·순수7개·Clippy와 현재1061개 입력, 같은 최종 ELF 목록을 확인했다. 원래 Runtime first와 secondary 오류 개수, commit 시작/성공 표시를 보존한다. 실제 Native·PG 결과는 아직 대기 중이다. 첫 동적 SQL typing 오류와 수정·재빌드 이력도 private 영수증에 남겼다. head `ce340657cb58ac46ea8cfa3bc7392f9bbd87d3c5`의 새 hosted CI는 다섯 job이 성공했지만 PostgreSQL job은 실패했고 workspace 검사는 진행 중이다. 닫힌 개별 결과를 확인하기 전에는 실패 원인이나 SQL 본문 실행을 추정하지 않는다.


Native17(`5c81b4a634c0ec6bb902f8fdebc633b7`)은 실제 SQL 대기 중 취소와 rollback·scope 완료·경제 상태 불변 assertion을 통과했다. 다음 기한 사례는17.7초에 outer Ok로 반환해90초 마감 거절을 기대한 assertion에서 실패했고 마지막 COMMIT 사례는 실행하지 않았다. 하위 오류 종류는 이 시도에 수출되지 않아 원인을 추정하지 않는다. 부모3048 exit1/reap,116컨테이너/5helper/3볼륨/1263CLI·7stage 제거와1104입력 일치를 확인했다. 첫 Runtime·원래 SQL 오류·secondary를 보존하는 제품 코드는 바꾸지 않고, 테스트 경로에 닫힌 오류 enum·SQLSTATE 진단만 추가했다. 해당 빌드·순수7·Clippy·1061입력과 같은 최종 ELF 목록이 통과했다. 서로 독립적인 마지막 COMMIT 사례가 기한 사례의 실패 때문에 계속 미실행되지 않도록 private 실행 순서만 바꾸었고 기존24·자체30 순수 검사를 통과했다. 사례나 assertion·기한은 삭제·완화하지 않았다. 새 실제 Native는 별도 admission으로 진행한다.

[head ce34065 CI](https://github.com/sihioov/Lagrange/actions/runs/42089908187)의 Python·policy·Clippy·workspace-tests·format·Web 여섯 job은 모두 성공했다. Paper named28개에서는23개(DB22+source1)가 통과하고5개가 exit101로 실패했다. PG exit0/reaped·소유 container/stage 제거를 확인했다. 동결19개 정의를 보충한 독립 검토는 foreign actor의 migration_owner UPDATE가 RLS로 필터되는 fixture와 foreign job seed가 actor-local WITH CHECK를 위반하는 fixture 두 충돌을 찾았다. 실제 최초 오류 위치는 아직 수출되지 않았으며 다른 세 사례는 원인 미확정이다. 원래 권한·잠금·경제 assertion을 유지하며, 실패 위치와 SQLSTATE만 수출하는 닫힌 witness를 보강 중이다. R5-03·merge는 아직 미완료다.


## 실제 COMMIT 통과·기한 fixture와 SQL 역할 보정

Native18(`9fe77c4ce8fd1b022ad23878d65f9ac8`)에서 실제 SQL 대기 취소와 실제 COMMIT 대기 뒤 취소가 통과했다. 기존 worker의 실제 backend에서 COMMIT/Lock 상태를 확인하고 취소한 뒤, 별도 nonce schema의 marker 한 행이 실제 commit됐음과 commit 시작·성공 표시, Runtime first·secondary 오류 개수 및 Paper 경제 상태 불변을 확인했다. 입력 파일 교체 검증을 수행한 것으로 확대하지 않는다. 마지막 기한 사례에서는90초 outer 마감 전에 SQLSTATE57014를 관측했다. [PostgreSQL18 오류 표](https://www.postgresql.org/docs/18/errcodes-appendix.html)의 query_canceled 코드이며 정확한 취소 발생원은 이 기록만으로 확정하지 않는다. 부모21747 exit1/reap,117컨테이너/5helper/3볼륨/1272CLI·7stage 제거와1105입력 일치를 확인했다.

해당 fixture는 반복 SQL 대기를12초에서1초 단위로 나누어 기존15초 statement 제한 안에서 충분한 여유를 두고, 실제90초 outer 기한을 검증하도록 보정했다. SQL 오류가 생기면 원래처럼 보존·rollback·거절하며 숨겨진 재시도는 추가하지 않았다. 원래90초 outer·20초 완료 대기·15초 statement/5초 lock·reader 보존과 모든 assertion을 유지했다. Root의 정확한 한 값 변경 뒤 API 빌드·순수7·Clippy·1061입력과 같은 ELF 목록을 확인했다. 독립 컴파일 보고서가 대기값 위치를1437로 전사한 편차는 Root가 실제965줄로 정정해 별도 인계에 기록했고 봉인 보고서는 보존했다. 실제 기한 통과는 새 admission에서 확인한다.

SQL fixture에서는 foreign actor를 설정한 queued job 취소를 기존 worker 연결로 실행하고 실제 변경1행을 요구한다. foreign job seed는 기존 migration_owner 연결의 별도 트랜잭션마다 해당 owner의 LOCAL actor를 설정해 FORCE RLS를 만족시킨다. caller actor·tenant/mismatch·원래 queued 상태와 경제·projection assertion은 유지한다. 제품 migration·RLS·grants는 바꾸지 않았다. Source 계약 테스트1·빌드·Clippy·포맷과 현재360입력·같은 ELF를 확인했으며 DB27은 컴파일/목록만 확인한 상태다. 최초 OSError/null child와 새 이름의 성공 영수증을 함께 보존했다.

CI 실행기는 실패 로그에서 허용된 Rust 경로/줄/열, SQLSTATE와 닫힌 오류 분류만 수출한다. 원래14개 테스트 본문을 보존한18개 및 CI 계약/cache59개가 통과했다. Root는 전체 workflow에서 기존 Paper summary 본문 외 모든 설정이 같고, driver의 원래 함수 중 결과 metadata 부착 외 경로가 유지됨을 대조했다. raw 오류·SQL·행·환경 값은 공개하지 않는다. 기존 필수28개와 정리·원래 실패 결과도 유지하며, 다음 hosted 실행이 남은 세 SQL 실패의 실제 위치를 확인한다. R5-03과 최종 Git 통합은 계속 미완료다.


## 기한 사례의 반복 SQL 취소 — 2026-11-06 KST

Native19(`dd97520439102b9a47e71239d1fccd25`)에서도 실제 취소·COMMIT 사례는 통과했지만 기한 사례는 약17.6초에 동일한57014를 기록하고90초 outer 기한 전에 반환했다.1초 SQL 대기로 나누는 변경만으로 해결된다는 가설은 확인되지 않았으므로 추가 전체 재시도를 반복하지 않는다. 부모61720 exit1/reap,117컨테이너/5helper/3볼륨/1272CLI·7stage 제거와1114입력 불변을 확인했다.

기한 분기만 원래 요구인 실제 자원을 보유한 콜백 진입 후 기한 거절·rollback·경제 상태 불변으로 대조한다.15초로 제한된 인공 서버 SQL을90초 기한의 유일한 대기 장치로 삼는 대신, 실제 SQL 트랜잭션과 입력 reader를 유지한 콜백의 local await를 쓰는 방식을 검토한다. 원래90/20초와 모든 거절·정리 assertion을 유지하고, SQL sleep 관측은 취소 사례로 구분해 정직하게 기록한다. 이는 아직 제안이며 새 코드/실행 인수가 아니다.

게시 후보의 두 RLS fixture는 빌드·정확한 source1·Clippy·포맷과360개 현재 입력 대조를 통과했다.27개 DB 본문은 이 집중 검사에서 실행하지 않았다. 닫힌 CI witness는18개·계약/cache59개와 Root의 기존14개 본문/나머지 workflow 불변 대조를 통과했다. 안정된 수정·진단을 초안 PR에 먼저 게시해 남은 SQL 실패의 실제 첫 위치를 확인하며, 기한 사례와 최종 required CI·merge가 완료되기 전까지 R5-03 DOING을 유지한다.

## 실제 자원을 보유한 기한 콜백 fixture 구현

동결 계약 검토는 원래 요구가 콜백 진입 뒤 기한 거절과 자원 보존·롤백이며 기한 때 활성 SQL sleep을 요구하지 않음을 확인했다. Root는 기존 실제 트랜잭션과 checked reader를 보유한 local await로 기한 분기만 구현했다. 같은 worker 역할의 별도 연결에서 exact backend가 idle in transaction이고 xact_start가 있음을 확인하며, 취소 분기는 실제 활성 SQL 대기를 유지한다. 짧은 SELECT1과 local await는 경제 쓰기를 수행하지 않는다. 원래90초 outer·20초 완료 대기, CommitUnknown/DeadlineExceeded·롤백·경제 상태 불변 검사는 유지했다.

대상은 이 기록과 함께 수정된 Native 테스트 소스다. 정확한 집중 빌드·순수7·Clippy와 현재1061개 입력, 동일 최종 ELF의7진단+workflow 목록을 대조했다. 원래 ELF에서 새 strip 사본을 파생하고 기존 실행 파일과 interpreter·NEEDED·버전 ABI가 같음을 확인했다. 실제 기한 결과와 소유 자원 정리는 새 admission의 실행 결과로 별도 기록한다. Native19 조기57014의 정확한 발생원은 여전히 미확정이며 제품 SQL 제한은 바꾸지 않았다.

## 마지막 실제 기한·취소 경계 통과 — 2026-11-06 KST

Native20(`920e31769c0b1871eab40d5f3db97227`)은 non-Send SQL probe, 실제 SQL 대기 중 취소, 실제 COMMIT 뒤 취소, 실제 트랜잭션과 checked reader를 보유한 콜백의90초 기한 거절 네 사례를 모두 통과했다. 원래 CommitUnknown/DeadlineExceeded·20초 scope 완료·롤백·경제 상태 불변·worker 연결 반환 assertion을 실행했다. 기한 사례를 활성 SQL 대기였다고 확대하지 않는다. 기존 baseline60·E 승인/추천 미실행38단계 인수는 재실행하지 않고 유지한다.

부모93642 exit0/reap,119컨테이너/5helper/3볼륨/1290CLI의 종료·제거 기록, 정확한7stage 제거와1115입력 해시 전후 일치를 확인했다. 생성 당시 running 필드는 최종 정리의 증거로 쓰지 않고 실제 removed_confirmed/명령 terminal 기록을 대조했다. 운영 Origin·성능·생산 활성화 인수는 아니다.

[head7ee4ed3 CI](https://github.com/sihioov/Lagrange/actions/runs/42103469918)는 Paper25/28(DB24+source1)이 통과해 두 RLS fixture 수정의 실제 통과를 확인했다. 남은 실패는 source-lock 대기의8초 outer timeout, READY AUTO 재시도의 ROW_NOT_FOUND, terminal projection의55P03 기대 assertion이다. 새 닫힌 witness로 첫째2726:10·셋째3665:9의 실패 위치를 확인했으며 실제 반환 SQLSTATE가 없으면 추정하지 않는다. Web167사례 중 로그인 전환 한 사례도 실패해 원인을 확인한다. 해당 세 DB 사례와 Web·최종 required CI·merge가 남아 R5-03은 DOING이다.

## READY trial 가시성과 닫힌 실패 단계 보고

같은 worker 트랜잭션이 unrevoked READY AUTO를 실제 적용한 뒤, beta preview/target을 직접 SELECT하던 fixture는 기존 restrictive RLS와 충돌한다. 해당 disposable DB에만 정확한 owner/account/preview와 참조 target으로 묶인 읽기 전용 SECURITY DEFINER helper를 두었다. 검색 경로와 caller actor 복구를 고정하고 PUBLIC 권한을 회수한 뒤 기존 worker의 helper 실행만 허용한다. 제품 테이블의 권한·RLS·migration은 바꾸지 않았다. 같은 미커밋 트랜잭션에서 APPLIED·E2·target/session 수를 검증한 뒤 원래 rollback·revocation42501·전체 상태/이력 불변 검사를 유지한다. Root는 helper 뒤에도 worker의 직접 preview/target 조회가 각각0행임을 요구하는 assertion을 추가했다.

APPLY는 원래8초 안에서 접속 확보·실제 backend PID·원래 apply query를 추적하고, 관측된 소유 advisory blocker 여부와 실제 오류 분류만 기록한다. TERMINAL은 원래7초/해제/4초 복구 순서와55P03 기대값을 유지하면서 해제 전 완료 여부와 실제 반환 분류를 기록한다. READY의 주요 단계도 닫힌 enum으로 남긴다. 기존2MiB·16개 한도 안에서 허용된 stage/outcome/SQLSTATE/PID/boolean만 파싱하며 workflow에서 다시 검증한다. 원래 네 필드의 보고는 그대로 호환되고 새 test_witnesses는 선택 항목이다.

기존 Python18개 본문 AST를 보존한22개, CI 계약/cache59개를 Root가 확인했다. Root의 직접 가시성 assertion을 포함한 최종 SQL 빌드·정확한 source1·Clippy·Rustfmt와360개 입력·동일 ELF/27 ignored+source1 목록을 확인했다. DB 본문은 이번 로컬 검사에서 실행하지 않았다. 초기 OSError·E0277과 중간 source 세대의 영수증은 보존했다. 원래 APPLY/TERMINAL의 동적 첫 원인은 아직 미확정이며 제품 시간 제한을 보정한 것으로 주장하지 않는다. 전체 nt pytest는 이 집중 패키지 밖이며 로컬 설치는 하지 않았다. 최종 hosted required lane을 계속 사용한다.

패키지가 직접 실행 지시와 달리 한 차례 추가 Paseo 작업자에게 재위임된 편차를 확인했다. 두 세션의 실제 final/idle·대기0과 허용된 네 파일 범위를 복구했고, Root가 소스·영수증·집중 검증을 별도로 대조했다. 해당 가지에는 추가 작업을 맡기지 않는다. 상세 실행 이력과 일시적 참조 보고서 메타데이터 변경/원본 복구 이력은 private 원장에 보존했다.

CI summary가 추가되며 이동한 R4 proof artifact 근거 두 곳을 현재 단계로 갱신하고, 로컬 고정 PlantUML 이미지로 paired PNG를 렌더했다. 부모51383 exit0/reap·소유 renderer 부재·모든 CLI 종료와 입력/PNG 핀을 확인했다. 최종 후보의 DB27·브라우저 로그인 사례와 required CI·merge는 대기 중이다.


## CI18 출력 수집과 signal-handler 시계 경계

`23cafe2`의 [CI42115087153](https://github.com/sihioov/Lagrange/actions/runs/42115087153)에서 Web·format·policy·Clippy·Python은 성공했다. PostgreSQL은25/28이며 READY는 본문 exit0 뒤 정확한 libtest 성공 행을 찾지 못했다. `--nocapture`의 첫 진단이 libtest 진행 행과 섞일 수 있으므로 `--show-output`으로 각 테스트의 결과와 수집 출력을 분리한다. strict 결과 검사는 유지한다. 기존22개 Python 본문을 보존한24개와 CI 계약/cache59가 통과했다.

APPLY는 실제 advisory blocker가 관측된 query에서8초 client 제한에 도달했다. TERMINAL의 첫 닫힌 반환 표시는 출력 행에 섞여 실제 SQLSTATE가 아직 수출되지 않았다. 동결8입력 분석은 이 증거만으로 제품 수정을 지지하지 않았고 그 결론을 보존한다. 이후 별도 Root 자식 실험에서 +36000초 fixture의 `SIGALRM` 처리기만 실제 시각을 읽는 것을 재현했다. 정상 경로의 시각 차이는0초, 처리기 차이는-36000초였다. Linux `epoll_wait`를 syscall로 통과시키면 같은 처리기의 차이는0~1초로 돌아오고 monotonic 대기는 약200ms를 유지했다. 두 실험의 모든 컴파일/실행 자식은 exit0·회수 완료다. 호스트 시계·서비스·제품 SQL은 변경하지 않았다.

[libfaketime0.9.10 소스](https://raw.githubusercontent.com/wolfcw/libfaketime/v0.9.10/src/libfaketime.c)는 epoll 대기 동안 재진입 방지 상태를 유지하고 그 상태에서 시간 변환을 생략한다. [PG18 timeout 처리기](https://raw.githubusercontent.com/postgres/postgres/REL_18_STABLE/src/backend/utils/misc/timeout.c)는 신호 처리 중 같은 wall clock으로 만료를 비교한다. 두 시간 영역이 달라지면 상수 offset이 상쇄된다는 가정이 성립하지 않는다. 기존 로컬0.9.13 라이브러리의 실험은 이 원시 경계를 검증한 것이며 hosted0.9.10의 실제 SQL 테스트 통과를 대신하지 않는다.

보정은 격리 PostgreSQL의 대기 호출에만 연결한다. 원래5초 lock/15초 statement·8/7초 fixture 제한과55P03·rollback·경제 상태 assertion을 보존한다. 최종 실제DB27과 현재 head의 전체 required CI·merge는 대기 중이다. 운영 Origin/G4/WP7과 activation·기본15초 성능 인수는 별도 OPEN이다.

최종 fixture29·CI 계약/cache59와 기존24개 본문 AST 보존을 Root가 확인했다. 실제 controller helper로 C를 컴파일해 원본/결과 source hash,0444 ELF15408B와 GLIBC2.2.5 요구를 대조했다. 원래 Rust 빌드/source1/Clippy의 전체360입력은 현재와 일치해 결정적 영수증을 재사용한다. 제품/원장 SQL·Rust·기존27 DB 본문은 변경하지 않았다. 실제DB와 현재 head 전체 CI는 새 게시 실행에서 확인한다.


## R5-03 코드·격리 검증 인수 — 2026-11-06

**코드·격리 검증 DONE.** 대상은 `ef0538fa7ff5125ec4dd6fc56fcffc31ce566d32`다. [CI42120579006 attempt2](https://github.com/sihioov/Lagrange/actions/runs/42120579006/attempts/2)에서 Web·format·policy·Clippy·Python·workspace와 required 집계가 모두 성공했다. 첫 workspace 실행은 Docker daemon 사전 점검에서 테스트 시작 전에 멈췄다. 전체 workflow 종료 뒤 실패한 job만 재시도했고, 사전 점검과 실제 workspace 실행이 성공했다. 같은 SHA의 성공한 검사를 임의로 재실행하거나 사전 점검을 건너뛰지 않았다.

[실제 PostgreSQL job](https://github.com/sihioov/Lagrange/actions/runs/42120579006/job/127571564122)의 이름별28개 결과는 모두 PASS/exit0다. 그중27개는 ignored DB 본문을 정확한 이름으로 실행한 것이고1개는 ordinary source 검사다. READY trial/revoke, APPLY Source lock55P03·무변경·replay, TERMINAL projection55P03·동반 rollback을 원래 assertion 그대로 통과했다. PostgreSQL exit0/reap, 소유 컨테이너 terminal/removed, 입력·출력 staging 제거도 확인했다.

실제 금융 인수는 Native11의60단계·두 강제 종료/복구·checker4, Native14의 E 승인/추천 미실행38단계와 중복 없음, Native20의 entered callback 네 경계/34단계다. 실제 SQL 대기 취소, 실제 server COMMIT 뒤 취소, 실제 transaction·checked reader를 보유한 원래90초 기한 거절과 원장 상태·정리를 확인했다. Root의 집중 컴파일·단위/문서·Clippy·입력 해시 대조, 독립 Source 검토 및 이전 실패 이력도 함께 보존한다. 모든 파일 교체 타이밍·전체9.9MB의 모든 줄·운영 설치 성능을 전수 검증했다는 뜻은 아니다.

이 기록은 source/합성 격리 인수다. SYNTHETIC 인증과 UNKNOWN Source를 실제 Root Origin으로 승격하지 않는다. production CLOSED·None4/default-unbound를 유지한다. 현재 권리·실제 운영 Origin/G4/WP7·활성화·기본15초 성능·drain/restore는 R5-04 및 기존 후속 범위로 남는다. 완료 기록 문서만 추가한 마지막 head의 required CI와 승인된 정상 Git merge까지 확인하며 운영 배포는 수행하지 않는다.
