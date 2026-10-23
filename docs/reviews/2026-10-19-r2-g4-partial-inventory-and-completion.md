# R2 G4 부분 조회 회수와 남은 조회 실행안 — Wave112

상태: **Wave111 실행 종결·부분 결과 회수. Wave112 PREPARED_NOT_AUTHORIZED_NOT_EXECUTED.**
앱 `e639169`의 이미지·실제 격리 통합과 도구 `3de6420`의 hosted CI 인수는 유지한다.
현재 운영 릴리스는 여전히 `fce9b90`이다. 운영 백업·배포·Owner 사용·연속 실제
3거래일·최종 C_R2는 완료되지 않았다.

## 실제 실행과 회수

Owner의 승인에 따라 Wave111 읽기 전용 조회를 2026-10-19 04:21:59 UTC
한 번 실행했다. 04:22:01 UTC에 exit 1, `G4_STRUCTURE_OR_READ_FAILURE` /
`FileNotFoundError`로 종료됐다. stderr는 0 bytes이며 필터링된 부분 결과
85,675 bytes를 회수했다. 서비스 동작, 운영 파일 쓰기, SQL, provider 호출은 없었다.

회수된 범위는 전체 목록에 나타난 유닛 56개, 컨테이너 13개, 기존 PostgreSQL
볼륨과 네트워크, 다섯 데이터·설정 root의 메타데이터, 보호 파일 7개다.
다섯 root의 파일 크기 합계는 972,201,939 bytes다. 이는 살아 있는 파일 트리의
메타데이터 합계이며 일관된 백업, 실제 사용 블록 수, 내용 해시 증명이 아니다.
프로젝트 컨테이너는 11개이며 연구 worker는 기존 exit 2 상태, 나머지 10개는
running으로 관측됐다. 이 조회는 health 인수나 최종 상태 재대조에 도달하지 않았다.

고정된 소스 실행 순서에서 다음으로 시도한 경로는
`/opt/lagrange/.lagrange-release.lock`이다. 어느 경로 구성 요소가 없었는지는
예외에 출력하지 않았으므로 **UNKNOWN**이다. 없는 잠금 파일을 조회 실패로 처리해
뒤의 독립적인 조회까지 중단한 것은 조회 제어의 한계다. 원래 두 번의 실패와 제어를
그대로 보존한다. 잠금 파일을 자동 생성하거나 승인된 조회를 재실행하지 않는다.

종료 receipt SHA256:
`a7695e492d693fae1156ab8d6402296852f45197727c3ac951f4d2cf73186c64`.
종결 기록 SHA256:
`f2efca1ed12ca3759bb73aea224ec11505834f2cc25f2c0d01451b8a2103be92`.
39개 봉인 입력의 바이트는 실행 후에도 일치했다. 승인
`e24fd5c21841bfa8289dfa02627135f44f71a64d3fa3092b2a0834104592415d`는 소진됐다.

## 관측으로 확인한 G4 준비 조건

| 항목 | 실제 관측과 필요한 처리 |
| --- | --- |
| DB 저장 경로 | 기존 정확한 PG container mount와 local named volume 모두 `/data/docker/volumes/lagrange-station-pgdata/_data`를 가리킨다. 기본 `/var/lib/docker` 가정은 실제 호스트와 달랐다. |
| 상위 경로 소유권 | `/data`는 1000:1000/0755다. 현 복구 도구의 root 소유 ancestor 조건을 만족하지 않는다. 공유 `/data`를 chown하거나 일반 디렉터리 검증을 느슨하게 하지 않는다. 별도 명시적인 신뢰·identity 결합 설계가 필요하다. |
| 볼륨 보호 조건 | volume parent는 root:root0701, `_data`는 999:999/1777이다. 현 도구는 이 sticky root의 부모를 root:root0700으로 요구한다. 이 조건은 유지하며, 향후 정확한 부모만 보호했다가 같은 inode에 0701을 복구하는 운영 절차는 별도 승인 대상이다. 지금 chmod하지 않았다. |
| 과거 실패 유닛 | `lagrange-kis-daily-66b2a8c.service`는 disabled, persistent, failed/failed, MainPID/ControlPID 0, Restart no, Job·hook·drop-in 없음으로 관측됐다. 현재 v2의 transient terminal 분류로는 인수할 수 없다. 기록을 reset/delete/filter/relaunch하지 않고 명시적인 소스 호환성 처리가 필요하다. 새 cgroup 검증은 아직 없다. |
| 유닛 연결 | 일부 inactive disabled timer의 Triggers에 비해 먼저 관측한 service의 TriggeredBy가 비어 있다. 이유를 추정하거나 역방향 값을 만들어 넣지 않는다. 전체 목록을 조회한 뒤 같은 metadata를 다시 관측해야 한다. 현 도구의 연결 검증은 변경하지 않았다. |
| 미회수 항목 | backup key의 metadata, 여유 공간·메모리, 이미 열린 writer FD/shared mapping, 최종 container/unit/current 재대조. 전역·외부 writer 차단과 secret 복구 보관도 이 조회만으로 입증되지 않는다. |

보존 볼륨 `159b052f…2034f07`의 허용된 metadata와 참조는 이전 인수와 같고
참조 컨테이너는 없었다. 내용 접근·mount·delete·prune는 하지 않았다. 소유권
UNPROVEN과 과거0120 cleanup INCOMPLETE는 그대로다.

이 결과는 새 소스 보정과 실제 중단·복구 절차를 한 묶음으로 검토하기 위한 입력이다.
서로 다른 보안 가정을 즉시 바꾸어 통과시키지 않는다. 신뢰 경계가 정해진 뒤에만
새 도구 후보를 만들고 기존 검증과 추가 회귀를 hosted에서 한 번 검증한다.
현재 인수된 앱·도구 소스는 수정하지 않았다.

## 보완한 단일 읽기 전용 조회

새 결정은 `wp-8/g4-admission-3de6420-a3/prepared-survey.json`에 결합한다.
고정 실행 명령은 다음과 같다.

```text
/usr/bin/sudo -n /usr/bin/env -i PATH=/usr/sbin:/usr/bin:/sbin:/bin LANG=C LC_ALL=C PYTHONDONTWRITEBYTECODE=1 /usr/bin/timeout --signal=TERM --kill-after=5s 305s /usr/bin/nice -n 10 /usr/bin/ionice -c 3 /usr/bin/python3 -B /data/worktrees/3puw275b/excited-dolphin/target/r2-daily-beta/wp-8/g4-admission-3de6420-a3/read-only-admission.py
```

기존 [Wave110 조회 대상](2026-10-19-r2-g4-operating-inventory-packet.md)과
[Wave111 경로 결합·부분 회수](2026-10-19-r2-g4-inventory-path-correction-packet.md)를
유지하며 다음을 보완했다.

1. 고정 보호 파일과 키의 경로 부재를 `MISSING_PARENT_COMPONENT` 또는 열린 부모에서
   재확인한 `ABSENT_LEAF_OBSERVED`로 기록한다. 파일을 만들지 않는다. 빠진 경로를
   성공 조건으로 바꾸지 않고 다른 고정 metadata 조회를 계속한다. 키 바이트는 읽지 않는다.
2. 마지막으로 시도한 고정 파일 경로를 기록하고, 존재하는 파일은 nonblocking/no-follow
   open 후 기존 regular/single-link/크기/identity 검증을 그대로 적용한다.
3. 같은 56개 등 실제 목록의 모든 unit metadata와 허용된 fragment hash를 마지막에
   다시 읽어 첫 관측과 비교한다. 다른 값은 이름 목록과 두 실제 관측을 보존한다.
   역방향 연결을 보정하거나 systemd 상태를 바꾸지 않는다.
4. 모든 조회가 끝나도 누락 경로나 unit 변화가 있으면
   `METADATA_RECOVERED_WITH_GAPS`, exit 2다. `collectionCompleted`와
   `acceptedObservation`을 구분하며 **operatingAdmission은 항상 false**다.
   나머지 guard 실패·시간 초과는 실패와 부분 결과로 종결한다.

새 파일 경로나 Docker 하위 명령은 추가하지 않았다. 두 번째 unit 관측은 기존과 같은
systemctl show·고정 fragment metadata 조회다. 300초 내부/305초 외부/5초 자체 정리,
Nice10/idle I/O, unit128·container256, root별200,000 entries/깊이64/1TiB,
process8192/FD합계131072, 출력2MiB 한도를 유지한다. 자동 재시도는 없다.
원시 출력은 RAM에서만 취급하고 필터링된 metadata와 hash/길이만 보존한다.

변경은 바이트 차이와 역변환으로 직접 정적 검토했다. private 제어의 로컬 실행,
project parser/helper/compiler/test, 새 독립 리뷰는 하지 않았다. 인수된 maintenance
41개 hosted PASS는 이 조회 제어의 실행 검증이 아니다.

## 승인 범위

새 승인은 이 읽기 전용 조회 **1회와 종료·부분 결과 회수**만 포함한다. 서비스 중단,
DB/데이터 내용 읽기, 키 내용, 권한 변경, 백업·clone·SQL·migration·배포·timer 활성화,
provider 호출은 포함하지 않는다. 결과를 토대로 G4의 구체적인 명령·원상 복구 절차와
소스 검증을 완성한 뒤 실제 운영 변경을 제안한다.

이전 실행안의 `executionLimit=1`, `automaticRetryAuthorized=false`와 종결 기록의
`successorHostExecutionAuthorized=false` 때문에 새 실행 결정이 필요하다. 기존
KIS 읽기 전용/EOD 및 Owner V2 승인을 다시 요구하는 것은 아니다.
