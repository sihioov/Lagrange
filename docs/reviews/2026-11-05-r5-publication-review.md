# R5 게시 검토와 수정 추적 — 2026-11-05

게시 검토는 `c2eb57b4b444fece2bf77b385645f0a3509a7c2f`에서 시작했다. Owner의 커밋·머지 지시에 따라 기존130파일을 보존했다.10개 독립 정적 lens와 별도 Claude 검토 후, 아래13개 수정 후보를 새 검증자가 각각 확인했다. 후속 수정과 검증은 아래에 구분해 기록한다. **현재 머지 준비 미완료이며 R5-03은 재개 상태다.**

기존 R5-02 actual32/composite와 R5-03 Native27 60단계는 원래 핀·fixture 범위에서 통과한 이력이다. 이를 삭제하거나 새 경계의 검증으로 확장하지 않는다.905 입력 일치는 게시 검토 전의 소스 상태에 대한 것이며 후속 수정의 실행 증거는 별도로 기록한다.

| # | 등급 | 경계 | 수정·검증 상태 |
| --- | --- | --- | --- |
| 1 | P1 | Track compile-time fixtures in versioned paths (`crates/market-data/src/paper_source_evidence.rs:3185`) | versioned 입력·해시/include 대조 및 영향 패키지 컴파일·Clippy PASS; 최종 CI 대기 |
| 2 | P1 | Revalidate signal admission before applying a READY preview (`migrations/0067_owner_beta_paper_ledger_v1.up.sql:751`) | 공통 apply의 실제 admission 검사 수정·독립 정적 확인; 빌드/source1/Clippy PASS, PG27 실제 실행·최종 CI 대기 |
| 3 | P1 | Exclude the module helper from Cargo test autodiscovery (`tests/integration/migration-contract/tests/paper_0067.rs:1`) | 명시적 단일 target의 빌드·전체 target/feature Clippy PASS; 최종 CI 대기 |
| 4 | P1 | Decouple execution prices from same-day recommendation success (`crates/api-server/src/paper_session/owner_beta.rs:847`) | source: T 추천과 승인 가격 E 분리, E NULL finish ordinary red/green·집중164/Clippy 확인; genuine E 미실행·실패/복구 Native와 최종 CI 대기 |
| 5 | P1 | CWE-367: Keep checked inputs alive through ledger commits (`crates/job-queue/src/owner_beta/paper/evidence.rs:3255`) | historical 실제 descriptor 보존91/Clippy·독립 closure 및 네 금융 scoped 소비자 연결164/Clippy 확인; 실제 SQL 대기 변조/정리·최종 CI 대기 |
| 6 | P2 | Honor configured beta preview heartbeat intervals (`crates/api-server/src/paper_session/owner_beta.rs:1195`) | RunnerServices 설정 heartbeat 전달·유효 범위 검사와 집중 검사 확인; 실제 짧은 lease 동안 전달/상실 검증 대기 |
| 7 | P2 | Update the ordering guard for the instrumented worker call (`crates/api-server/tests/owner_beta_paper_execution.rs:41`) | 기계적 guard 수정·Root 입력 대조·Rust fast 및 API 전체target/feature Clippy 컴파일 PASS; 본문·최종 CI 대기 |
| 8 | P2 | Finalize beta previews when retry budgets exhaust (`crates/job-queue/src/queue.rs:1010`) | 제한된 terminal job→preview 원자적 projection 및 job→preview 잠금 순서 수정·독립 정적 확인; PG27 실제 실행·최종 CI 대기 |
| 9 | P2 | Correct stale R5 architecture evidence citations (`docs/diagrams/component_architecture.puml:499`) | 실제 호출·완료 위치로 근거 수정, R5 인용106개 경로/줄 대조 및 paired PNG 로컬 렌더 PASS |
| 10 | P2 | Reuse the calendar-link validator in preview capture (`crates/api-server/src/paper_session/owner_beta.rs:1530`) | preview/settlement/value 공통 calendar 검증 연결·집중164/Clippy 확인; 실제 별도 owner/policy/row/terminal 거절 검증 대기 |
| 11 | P2 | Bound financial checker SQL waits (`crates/job-queue/src/owner_beta/paper/read_transaction.sql:1`) | checker 두 트랜잭션 및 행동 트랜잭션에 기존15초 statement/5초 lock 설정 연결·집중 검사/Clippy 확인; 실제 잠금 대기와 scope/slot 완료 검증 대기 |
| 12 | P2 | Reuse automatic lineage when submitting a manual preview (`migrations/0067_owner_beta_paper_ledger_v1.up.sql:555`) | 자연 식별자의 immutable AUTO lineage 재검증·MANUAL 재사용 수정 및 독립 정적 확인; PG27 실제 실행·최종 CI 대기 |
| 13 | P2 | Execute ledger regressions in the required test lanes (`tests/integration/migration-contract/tests/paper_0067.rs:1470`) | 기존 필수 PG job에 고정 이미지·fixture clock·정확한 selector 실행 연결; 순수12/CI 계약52 PASS·독립 정리 검토 해소; actual hosted PG 대기 |

전체9.9MB diff의 모든 줄을 검토했다고 주장하지 않는다. 리뷰는 금융·checked input/정리·SQL/RLS·API·이행·CI packaging의 선택된 의미 경계를 다뤘다. Claude 실제 model은 receipt의 `claude-opus-5`, 실제 effort는 미확인이다.4개 외부 제안은 보수적 상태 표시·실행 파일 재검증 비용·버전 rollback·cooperative cancellation의 채택 경계와 비교해 주 수정 목록에서 제외했고, 실제 운영 성능·drain은 기존 R5-04 인계로 유지했다.

최종 변경 SHA의 hosted 필수 CI, 영향 runtime 검증, 도표 근거/로컬 렌더, 커밋·PR·merge 결과는 아직 없다. 수정 후 이 문서와 ROADMAP에 추가한다. 운영 Origin/G4/WP7, 활성화·서비스·provider 권한은 별도이며 None4/default-unbound/production CLOSED를 유지한다.

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
