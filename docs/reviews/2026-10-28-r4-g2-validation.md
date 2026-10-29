# R4-06 격리 검증: 2026-10-28

검증 대상은 `a006e5ee683b4e36bdb7f89924a01def50e240d5`다. 이 문서는 해당 커밋의 hosted 실행 결과를 기록한다. 후속 문서 커밋은 검증된 소스 바이트를 변경하지 않는다.

| 실행 | 결과 | 확인한 증거 |
| --- | --- | --- |
| [전체 CI 40639806731](https://github.com/sihioov/Lagrange/actions/runs/40639806731) | SUCCESS, 2026-10-28 06:44:47 UTC 종료 | format·Python·Clippy·policy·Web·workspace·PostgreSQL·R4 전용 빌드 8개와 `required` 집계 모두 SUCCESS |
| [Research smoke 40639817540](https://github.com/sihioov/Lagrange/actions/runs/40639817540) | SUCCESS, 2026-10-28 06:44:17 UTC 종료 | 정적·기능 PASS, 원래 실행 exit0, 격리 컨테이너·볼륨·네트워크·임시 이미지와 디렉터리 정리 exit0 |

PostgreSQL job은 disposable DB에서 trigger metadata, ACL, routine metadata, RLS metadata, configured input rights, migration ledger 관찰자 여섯 개를 각각 `PG_DB_TEST_EXIT:...:0:SKIP_false`와 해당 `PG_DB_TEST_ROW:...:ok`로 기록했다. Migration 회귀는 실제 worker 연결에서 `public._sqlx_migrations`의 SELECT=true, INSERT=false, UPDATE=false를 검사한다. Policy job은 migration 65개 fixture의 DB self-test를 통과했다.

R4 전용 빌드는 verifier 4개, child 9개, provenance 14개, runner 7개, private process 289개, runner CLI 9개, process 4개, connection proof 6개를 PASS로 기록했다. Source inventory 921개와 connection proof receipt도 PASS다. 같은 SHA 이름의 R4 binary/provenance와 connection proof/provenance 산출물 두 개가 업로드됐다. `required` job은 모든 8개 작업의 success 값을 검사하고 성공했다.

이 증거는 합성·격리 환경의 컴파일, 테스트, PostgreSQL 쿼리와 스모크에 한정된다. 실제 승인 입력 및 개별 Owner 권리, reader·startup의 실제 운영 조건, 배포·API drain·rollback·백업/복원, Owner 사용은 확인하지 않았다. R4-06과 R4 전체는 **DOING**이다. G3는 실제 값과 권한을 별도 확인한 후 진행한다. 새 provider, 거래 또는 운영 활성화 권한을 이 검증에서 추론하지 않는다.
