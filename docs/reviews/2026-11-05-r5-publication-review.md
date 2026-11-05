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
| 9 | P2 | Correct stale R5 architecture evidence citations (`docs/diagrams/component_architecture.puml:499`) | 독립 정적 확인; 수정·영향 검증 대기 |
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
