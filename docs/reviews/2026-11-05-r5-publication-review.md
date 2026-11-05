# R5 게시 검토와 수정 추적 — 2026-11-05

현재 대상은 `c2eb57b4b444fece2bf77b385645f0a3509a7c2f`다. Owner의 커밋·머지 지시에 따라 기존130파일을 보존했다.10개 독립 정적 lens와 별도 Claude 검토 후, 아래13개 수정 후보를 새 검증자가 각각 확인했다. **현재 머지 준비 미완료이며 R5-03을 재개한다.**

기존 R5-02 actual32/composite와 R5-03 Native27 60단계는 원래 핀·fixture 범위에서 통과한 이력이다. 이를 삭제하거나 새 경계의 검증으로 확장하지 않는다.905 입력 일치는 게시 검토 전의 소스 상태에 대한 것이며 후속 수정의 실행 증거는 별도로 기록한다.

| # | 등급 | 경계 | 수정·검증 상태 |
| --- | --- | --- | --- |
| 1 | P1 | Track compile-time fixtures in versioned paths (`crates/market-data/src/paper_source_evidence.rs:3185`) | 기계적 수정 반영; Root 해시/include/target 대조·Rust fast PASS; 컴파일·최종 CI 대기 |
| 2 | P1 | Revalidate signal admission before applying a READY preview (`migrations/0067_owner_beta_paper_ledger_v1.up.sql:751`) | 독립 정적 확인; 수정·영향 검증 대기 |
| 3 | P1 | Exclude the module helper from Cargo test autodiscovery (`tests/integration/migration-contract/tests/paper_0067.rs:1`) | 기계적 수정 반영; Root 해시/include/target 대조·Rust fast PASS; 컴파일·최종 CI 대기 |
| 4 | P1 | Decouple execution prices from same-day recommendation success (`crates/api-server/src/paper_session/owner_beta.rs:847`) | 독립 정적 확인; 수정·영향 검증 대기 |
| 5 | P1 | CWE-367: Keep checked inputs alive through ledger commits (`crates/job-queue/src/owner_beta/paper/evidence.rs:3255`) | 독립 정적 확인; 수정·영향 검증 대기 |
| 6 | P2 | Honor configured beta preview heartbeat intervals (`crates/api-server/src/paper_session/owner_beta.rs:1195`) | 독립 정적 확인; 수정·영향 검증 대기 |
| 7 | P2 | Update the ordering guard for the instrumented worker call (`crates/api-server/tests/owner_beta_paper_execution.rs:41`) | 기계적 수정 반영; Root 해시/include/target 대조·Rust fast PASS; 컴파일·최종 CI 대기 |
| 8 | P2 | Finalize beta previews when retry budgets exhaust (`crates/job-queue/src/queue.rs:1010`) | 독립 정적 확인; 수정·영향 검증 대기 |
| 9 | P2 | Correct stale R5 architecture evidence citations (`docs/diagrams/component_architecture.puml:499`) | 독립 정적 확인; 수정·영향 검증 대기 |
| 10 | P2 | Reuse the calendar-link validator in preview capture (`crates/api-server/src/paper_session/owner_beta.rs:1530`) | 독립 정적 확인; 수정·영향 검증 대기 |
| 11 | P2 | Bound financial checker SQL waits (`crates/job-queue/src/owner_beta/paper/read_transaction.sql:1`) | 독립 정적 확인; 수정·영향 검증 대기 |
| 12 | P2 | Reuse automatic lineage when submitting a manual preview (`migrations/0067_owner_beta_paper_ledger_v1.up.sql:555`) | 독립 정적 확인; 수정·영향 검증 대기 |
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
