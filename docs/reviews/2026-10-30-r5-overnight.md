# R5 야간 진행·검증·인계 — 2026-10-30

**08:30 KST 야간 실행 종료. 작업자·요청·소유 자원 최종 대조와 heartbeat 종료를 완료했다.**
R5-01/G1과 current producer·보호 reader·checker의 소스·집중 검증을 수용했다.
마지막 V3 로컬 격리 컴포넌트도 **테스트 1개에서 8개 전이 PASS**로 인수했다.
R5-02와 G2a 전체, Paper 활성화와 실제 무인 3거래일 인수는 미완료다.

대상은 `a30edacff987d55b5be7cca25bed7faeec589629` 위의 미커밋 R5 후보다.
[소스 수용 기록](../../target/r5-paper/coordinator/current-source-acceptance.json)의 기존 46개 입력은 보존했다.
검증용 새 ignored V3 테스트는 별도 핀 `61712fb0…808d1e`로 관리한다.
기존 금융 모형·지연 정산·보존/명시 재개 결정은 [ADR-0010](../decisions/0010-owner-beta-paper-v1.md)을 유지한다.
계획과 전체 인수 기준은 [로드맵](../ROADMAP.md)과 [R5 계획](../superpowers/plans/2026-10-29-r5-paper-implementation.md)을 따른다.

## 구현과 수리

- collector가 calendar 원본의 선행 claim 관측과 실제 batch ID를 전달한다.
- Paper 입력·연속 거래 세션 checker가 승인 입력, 현재 binding, DB·Raw·Daily 근거를 확인한다.
- 보호된 current publication과 reader가 세대·철회·OFD 잠금·시계 차이를 재검증한다.
- DB read-only/행 잠금 충돌, 달력 선택·날짜 범위, 시계 역행과 current binding 검증 누락을 수리했다.
- 자식 소유권 초기화 실패 때 종료·reap가 누락되던 경로를 수리하고 일반 UID에서 확인했다.
- 두 구조도의 코드 근거 32개를 대조하고 PNG의 오른쪽 잘림을 교정했다.

소스 인수의 세부 근거는 [checker 검토](2026-10-30-r5-checker-source.md)와
[source witness 검토](2026-10-29-r5-source-witness.md)에 있다.

## 검증 범위

| 범위 | 확인한 결과 | 남은 한계 |
| --- | --- | --- |
| collector | strict Clippy·fmt와 `raw_eod_` fixture 14개 PASS | 전체 focused의 최초 DB preflight 차단 이력과 전체 CI 미실행 보존 |
| producer·reader·checker 소스 | Python 98개, current 관련 Rust 31개, strict Clippy·fmt, 독립 검토 | 같은 후보의 전체 required CI 미실행 |
| 실제 로컬 PostgreSQL | 초기 named 4회와 FIX-A named 5회 실행 PASS, 전용 자원 정리 | 중복을 포함한 실행 횟수이며 synthetic origin fixture가 실제 root origin을 증명하지 않음 |
| 일반 UID 커널·자식 | timer/FD/pidfd/OFD component, 초기화 rollback 자식 3개, TERM/KILL·reap 확인 | 실제 root scheduler와 전체 자료 경로 연결 미검증 |
| 구조도 | component 8640×1645, runtime 16000×2160, 직접 확인과 renderer 정리 | 전체 G3·설치·운영 인수와 구분 |
| 검증용 V3 실행 파일 | fmt·정적 no-run compile·strict Clippy와 독립 static linkage 대조 PASS | 새 테스트 target의 컴파일이며 제품 이미지 빌드가 아님 |
| V3 실제 격리 컴포넌트 | live publication/OFD와 public reader의 테스트 1개·8개 전이·DONE·reader exit0 | current RO-directory mount 교체, full asset guard·실제 무인 root origin과 전체 G2a 미증명 |

V3 ELF는 24,944,048bytes이며 SHA-256은
`5b2c7d65b6bc61928f21921756d8b893f553a4351096ba767af2e5ff371d4b3f`다.
기존 동적 ELF의 GLIBC 2.39 요구와 캐시 이미지의 2.36 차이 때문에 설치된 도구만으로 정적 링크했다.
기존 제품 입력 46개·원 테스트·동적/정적 ELF 2개를 보존했다.
[컴파일 인수](../../target/r5-paper/coordinator/current-component-execution-v3/compile-acceptance.json)와
[실제 실행의 독립 대조](../../target/r5-paper/coordinator/current-component-execution-v3/coordinator-final-verification.json)에 정확한 핀을 연결한다.

07:50 KST에 [증거 목록](../../target/r5-paper/coordinator/overnight-evidence-index.json)을 작성했다.
기존 입력 46개·collector 3개·검증용 새 테스트 1개·구조도 4개를 합한 54개, ELF 3개와
증거 파일 23개의 경로·해시를 묶었다. 모두 예상 핀과 일치하며 이 검사는 파일 무결성 확인이다.
과거 runtime 관측을 새로 실행한 것으로 계산하거나 전체 G2a 인수로 확대하지 않았다.

[08:10 작업트리 목록](../../target/r5-paper/coordinator/0810-worktree-snapshot.json)은 현재
미커밋 파일 52개(기존 tracked 변경 16개, 새 untracked 파일 36개)의 경로·해시를 기록한다.
이는 전체 현재 변경 목록이며 선택된 검증 입력 54개와 같은 집합이 아니다. 모든 변경을 이번 turn의
작성물이나 전체 검증 완료 파일로 간주하지 않는다. 종료 문서는 08:30 최종 기록에서 다시 대조한다.

## 마지막 실제 컴포넌트 인수

UUID `34886fcd-5a73-46ce-8b81-4562eddaa082`의 컨테이너를 한 번 생성했다.
07:26 KST에 실행했고, 같은 컨테이너의 root 소유 tmpfs와 비권한 reader에서 아래 전이를 확인했다.

1. 유효 publication 열기.
2. 같은 세대의 refresh 허용.
3. invalid 발행 뒤 거절.
4. 철회된 reader 인스턴스의 부활 거절.
5. 세대 변경 뒤 거절.
6. writer close 뒤 거절.
7. 잠금 없는 복사본 거절.
8. publisher crash 뒤 거절.

실제 libtest 집계는 `1 passed / 0 failed / 0 ignored / 0 measured / 0 filtered out`이다.
DONE과 reader exit0을 확인했고 publisher exit73은 지정된 crash 관측이다.
namespace root는 이 전용 컨테이너 안의 역할이며 host root 권한을 추가하지 않았다.
CPU1·RAM256MiB·pids64·network none·read-only·capability0·NNP와 정해진 마운트만 사용했다.

완료 알림 `9f8c8111-df15-46e3-9783-dbe1ebf387a4` 뒤 실제 idle/pending0과 최종 보고를 회수했다.
coordinator도 기존 HeavySlot에서 읽기 전용 대조를 수행했다. 원 inode 15개의 journal과 선행 저장,
source context·hash·identity 일치와 stage 부재를 확인했다. 정확한 CID/name과 기록된 소유 PID/start
32개가 부재이며 기존 container/network/volume ID 집합 22/11/61개가 유지됐다.
이번 실행의 미해결 정리 항목은 없다. journal은 증거로 보존한다.

[전체 실행 보고서](../../target/r5-paper/WP-2A2/protected-current-static/execution-v3/report.md),
[실제 protocol·정리 기록](../../target/r5-paper/coordinator/current-component-execution-v3/execution-result.json),
[coordinator 독립 확인](../../target/r5-paper/coordinator/current-component-execution-v3/coordinator-final-verification.json).

## 이전 실패와 보존 증거

| 시도 | 결과 | 정리·한계 |
| --- | --- | --- |
| V1 `c404…` | UNKNOWN / SUMMARY_ORDER | 컨테이너·프로세스 부재와 기존 자원 복구 확인. 원 stage inode 증거 누락으로 파일 7개 보존 |
| V2 `a15b…` | FAILED_CLEAN, 0 passed / 1 failed | CLOSED 대기에서 실패. journal·컨테이너·stage·프로세스 정리를 직접 대조 |
| V3 `3488…` | COMPONENT_PASS, 1 passed / 0 failed | 별도 테스트 target과 별도 실행 묶음. 이전 실패를 소급해 성공으로 바꾸지 않음 |

원 테스트의 OPEN ACK 뒤 권위 재조회가 host의 다음 변경과 경쟁하는 구간을 소스에서 찾았다.
V3는 세대 관측·비교를 ACK 전에 끝내고 고정 실패 marker를 추가했다. 기존 거절 조건은 유지했다.
V3 성공만으로 V2의 정확한 panic 원인이나 제품 reader 버그를 확정하지 않는다.
private harness의 원 inode 선행 journal·FD close·오류 보존·출력 진단 수리는 모의 67개로 검증했다.

V1 stage 7파일, **25,121,326bytes**는 원 생성 inode를 입증할 기록이 없어 증거로 보존한다.
현재 hash·권한·nonce로 과거 소유권을 대신하지 않으며 coordinator가 보존·인계 책임을 맡았다.
활성 컨테이너나 작업 프로세스는 남아 있지 않다.

`target/r5-paper/WP-2A2/protected-current-static/run-c404b8b0-c4ac-4541-9d9e-3a1d5f947a02`

[V1 보고](../../target/r5-paper/WP-2A2/protected-current-static/execution/report.md),
[V2 보고](../../target/r5-paper/WP-2A2/protected-current-static/execution-v2/report.md),
[보존·인계 기록](../../target/r5-paper/coordinator/retained-stage-handoff.json)을 유지한다.
최초 timeout 전제 오류, 메모리 preflight 차단, dispatcher·검증 코드의 경로/스키마 해석 오류와 교정도 보존했다.
메모리 기준이나 권한을 완화하지 않았다.

08:01 KST [읽기 전용 자원 대조](../../target/r5-paper/coordinator/0800-resource-reconciliation.json)에서
앞선 PG·렌더·이미지 probe·컴포넌트의 기록된 CID 10개와 network ID 3개가 모두 부재임을 확인했다.
V2/V3의 원 stage 경로 30개도 부재이며 기존 자원 ID 집합 22/11/61개는 유지됐다.
조회 CLI와 검증 자식은 회수했다. 새 자원을 만들거나 V1 보존 stage를 읽고 변경하지 않았다.
이 대조는 정리 확인이며 생산 서비스 health나 G2a 인수를 추가하지 않는다.

## 남은 인수와 종료 절차

G2a에는 실제 무인 scheduler origin, 현재 정책·보호 자료, current RO-directory mount 교체,
pinned helper·sealed Raw·DB 전체 연결과 clock/suspend/systemd crash 검증이 남아 있다.
실제 거래일 관측·Owner 운용 인수도 별도다. WP-3와 활성화의 의존 gate를 유지한다.
이번 야간 범위에서는 host 시계·서비스·VM/KVM·provider·계정/주문·새 권리를 변경하지 않았다.
commit·push·배포도 수행하지 않았다.

### 후속 검증의 코드 접점

07:40 KST에 현재 소스를 읽어 다음 접점을 대조했다. 추가 실행이나 완료 판정은 하지 않았다.

| 미검증 항목 | 현재 코드와 재개 전제 |
| --- | --- |
| 전체 current 권위와 무인 origin | [witness](../../scripts/ops/lib/owner-beta-paper-evidence.py)의 `CurrentAuthority`(695행), `Scheduler.guard`(933행), `run_session`(1082행). V3는 core publication을 직접 사용했으며 private asset guard와 미래 slot dispatch 전체를 호출하지 않았다. |
| 보호 chain·실제 helper·Raw·DB | [helper 인자](../../scripts/ops/lib/owner-beta-daily-runtime.py)의 `paper_evidence_helper_args`(442행)와 [Rust helper](../../crates/job-queue/src/bin/owner-beta-runner/paper_evidence.rs)의 `verify_admission`(72행), `ProtectedSourceChain::open` 호출(296행). 같은 후보의 helper 산출물·승인 base·sealed Daily/Raw·DB 역할을 실제로 바인딩해야 한다. 이전 PG fixture와 V3를 연결 성공으로 합산하지 않는다. |
| current 디렉터리의 RO projection 교체 | [reader](../../crates/job-queue/src/owner_beta/paper/source_evidence.rs)의 `ProtectedCurrentBinding::open`(1137행). 같은 namespace tmpfs 검증과 실제 producer/consumer 사이의 directory mount 교체를 구분한다. |
| clock·suspend·systemd crash | [격리 harness](../../scripts/ops/owner-beta-paper-evidence-local-integration.py)의 `context`(158행)와 `main`(398행). 외부에서 바인딩한 폐기 가능한 Linux guest·root-owned marker·고정 자원이 선행하며 host에서 실행하지 않는다. |
| 실제 전체 producer 흐름 | 같은 harness의 `synthetic_producer_db`는 synthetic I/O 대체를 유지하고 모든 lane receipt도 `overall=INCOMPLETE`다. 기존 8개 lane을 모두 실행해도 실제 Rust helper/Raw·무인 origin 전체 증명으로 바뀌지 않는다. |

이 표는 현재 인수 기준의 코드 매핑이다. 새 VM·마커·이미지·권한을 만들거나 G2a 기준을 추가·완화한 계획이 아니다.
[초기 로컬 실행 조사와 단계별 보완](2026-10-30-r5-runtime-path.md)의 당시 기록도 보존한다.

07:25 시작한 V3가 마지막 장기 작업이며 07:30 이후 새 장기 작업은 시작하지 않았다.
08:15 정리 목표는 08:01 재대조까지 완료했다. 08:30에 새 작업을 중단하고 종료 대조만 수행했다.

소유 작업자 5개는 모두 실제 idle/activeTurn null이며 대기 승인·질문은 0건이다.
최종 결과와 오류 보고를 모두 회수했다. 읽기 전용 대조에서 기록된 CID 10개·network ID 3개와
V2/V3 원 stage 경로 30개는 부재였고 기존 자원 ID 집합도 유지됐다. 조회 CLI와 검증 자식은 회수했다.
선택 입력 54개·ELF 3개·증거 파일 23개의 최종 해시도 모두 일치했다.
V1의 25.1MB 증거 stage는 앞서 명시한 forensic 인계로 보존하며 활성 작업 자원이 아니다.

heartbeat `c522ae18`은 최종 책임 대조 후 삭제 성공을 확인했다. 감시와 야간 실행을 연장하지 않았다.
[최종 회수 기록](../../target/r5-paper/coordinator/0830-final-reconciliation.json)과
[최종 자원 대조](../../target/r5-paper/coordinator/0830-resource-reconciliation.json)를 연결한다.
[최종 작업트리 목록](../../target/r5-paper/coordinator/0830-worktree-snapshot.json)은 종료 문서를 포함한 현재 미커밋 후보를 고정한다.
G2a 전체·실제 무인 origin·운영 활성화는 OPEN/미인수로 남긴다.
