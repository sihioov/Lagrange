# R2 G4 운영 상태 조회 실행안 — Wave110

상태: **PREPARED_NOT_AUTHORIZED_NOT_EXECUTED**. 이전 Wave108의 두 실행 승인은
이미 성공 종결되어 소진됐다. 이 실행안은 배포 전 복구 계획에 필요한 운영 상태를
한 번 읽는 범위다. 승인된 앱의 이미지 빌드나 통합 검증을 다시 실행하지 않는다.

앱 `e639169018519fd6636c5579d1957e323e2c7802`의 12개 이미지와 실제 격리 통합
6단계는 [전체 인수 기록](2026-10-19-r2-e639169-host-acceptance.md)에 고정돼 있다.
별도 복구 도구 `3de6420a17e6093bfe05656b4ff1406857b5404c`도
[CI 39195322025 attempt1](https://github.com/sihioov/Lagrange/actions/runs/39195322025)의
8개 작업이 03:53:57 UTC 성공했고, 04:00:50 UTC 전체 로그·7개 실제 checkout·
1447개 입력 pin·540개 blob·필수 결과 187개를 회수했다. 유지보수 회귀 41개
(기존 28개와 새 13개)가 모두 통과했다. 이 결과는 아래 조회 스크립트의 실행 검증이 아니다.

## 실행할 작업

새 명시적 Owner 결정을 최종 `prepared-survey.json` 해시에 결합하고,
`target/r2-daily-beta/wp-8/g4-admission-3de6420-a1/execute-once.py`가
exclusive intent를 남긴 뒤 아래 명령을 **한 번** 실행한다. 자동 재시도는 없다.

```text
/usr/bin/sudo -n /usr/bin/env -i PATH=/usr/sbin:/usr/bin:/sbin:/bin LANG=C LC_ALL=C PYTHONDONTWRITEBYTECODE=1 /usr/bin/timeout --signal=TERM --kill-after=5s 305s /usr/bin/nice -n 10 /usr/bin/ionice -c 3 /usr/bin/python3 -B /data/worktrees/3puw275b/excited-dolphin/target/r2-daily-beta/wp-8/g4-admission-3de6420-a1/read-only-admission.py
```

조회 자체는 300초, 외부 제한은 305초와 정리 5초다. 제한 신호는 이 읽기 전용
조회 프로세스에만 적용된다. CPU 우선순위 Nice 10, idle I/O를 사용한다.
소스는 직접 정적으로 검토했으며 실행·로컬 parser/test·새 독립 리뷰는 하지 않았다.

하위 명령은 로컬 Docker 소켓의 `ps`, 기존 ID에 대한 `inspect`, PostgreSQL에
이미 연결된 볼륨·네트워크의 `inspect`, 그리고 systemd의 `list-unit-files`,
`list-units`, 허용 속성만 지정한 `show`다. Compose, 컨테이너 생성/exec/시작/종료/
삭제, 서비스 동작, daemon reload, SQL, 프로젝트 helper, provider 요청,
lock 취득, 운영 파일 변경은 포함하지 않는다.

| 조회 대상 | 정확한 범위와 한도 |
| --- | --- |
| 현재 릴리스 | `releases/fce9b90542cf9b1a911d46e4c4d3530db7efba4c` 링크 일치 |
| systemd | `lagrange-*` 전체 두 inventory 합집합 최대 128개. 종류별 lifecycle/invocation/activation 속성, 정규 `/etc/systemd/system/<name>` 또는 `/run/systemd/transient/<name>` fragment의 메타데이터·해시. 외부 fragment는 열지 않고 UNKNOWN으로 남김 |
| 기존 컨테이너 | 최대 256개의 ID·상태·설정 해시·관련 source mount/DB network/host network 여부. env·자유 형식 오류는 출력하지 않음 |
| 고정 PostgreSQL | `b876fb12fa0c0bd0437bc3322c06d429f069e1b3c15bacc430829fa04cbc012e`에 이미 RW 연결된 정확한 local volume과 그 네트워크 |
| 고정 base | `1b8bec839b8592918a4f5785233e64bb73a6ab601ad0309f1f24b5941a3824a5`의 기존 `/data/owner-beta-artifacts` RO mount |
| 다섯 데이터 root | `/var/lib/lagrange/data/raw`, `/var/lib/lagrange/data/curated`, `/var/lib/lagrange/data/artifacts/historical-price-beta-root`, 위 기존 릴리스 root, 정확한 PG volume root. 파일 내용 없는 metadata walk, root별 최대 200,000 entries/깊이 64/겉보기 크기 1 TiB; symlink를 따라가지 않음 |
| 보호 파일 | 기존 릴리스 marker/manifest/env/compose, 기존 backup helper/config, 수용된 앱 manifest, 기존 두 lock의 메타데이터·해시. 부모 UID/GID/mode/ACL, 공간·메모리도 조회 |
| 백업 키 | `/etc/lagrange/secrets/backup_encryption_key`의 메타데이터만 조회. 키 바이트는 읽지 않음 |
| 기존 writer 후보 | 위 다섯 root에 이미 열린 writable FD/공유 writable mapping. 최대 8192 processes/프로세스별 4096 FD/전체 131072 FD/프로세스별 maps 8 MiB. PID·시작 tick·UID·root별 참조 개수만 출력; cmdline/env/파일·mapping 내용은 읽지 않음 |
| 보존 볼륨 | `159b052fed63a476fba2c51c8f15f532a837096e4bdd453bb753789672034f07`의 한정 metadata와 참조 ID만 이전 인수와 대조. mount/content/delete/prune 금지; 소유권 UNPROVEN·과거 0120 cleanup INCOMPLETE 유지 |

원시 명령 결과, 보호 파일 바이트와 Docker 설정은 RAM에서만 처리한다. 로컬 private
영수증에는 선택된 메타데이터, 유한 결과, 길이와 해시만 남긴다. 마지막에 컨테이너 ID 집합,
unit 집합, 현재 릴리스 링크를 다시 대조한다. 누락·드리프트·제한 초과·명령 실패는
실패 또는 UNKNOWN으로 종결하며 자동 보정·재실행하지 않는다.

## 필요한 이유와 인수 경계

이전 수용된 이미지 admission에서 disabled 상태의 persistent
`lagrange-kis-daily-66b2a8c.service`가 `failed/failed`, PID 0, cgroup process 0으로
관측됐다. 현재 복구 계약은 persistent unit의 `inactive/dead`와 별도 transient
terminal 기록만 허용한다. 이 과거 상태를 성공으로 간주하거나, 기록을 reset/delete/
filter/relaunch하여 통과시키지 않는다. 정확한 최신 fragment·activation·generation
정보가 있어야 필요한 처리 방식을 결정할 수 있다.

PG 볼륨의 실제 부모 보호 상태와 전체 관련 writer 목록도 아직 확인되지 않았다.
이번 결과로 구체적인 중단·백업·복구 순서를 완성한다. 살아 있는 파일시스템의
메타데이터 조회는 일관된 snapshot이나 미래 root 활동·외부 접속까지 막는 writer
fence가 아니다. 조회 성공도 G4 실행 가능 판정으로 올리지 않는다.

이후 실제 cold-cluster 백업은 writer fence, 기존 DB read-only audit, 같은 PG
generation의 정상 종료, 다섯 root 보존, 암호화·복호화와 격리 clone 검증, 기존 상태
복귀를 별도 구체적 운영 결정에 결합해야 한다. 이번 범위에는 그 중단·백업 실행,
migration, current 변경, 여섯 파일 backup 전환, timer 활성화, catch-up 수집,
Owner 인수 또는 연속 실제 3거래일 관찰이 없다. R2-04/R2-09와 최종 C_R2는 미완료다.

새 결정이 필요한 근거는 소진된 Wave108 이미지/통합 범위와 해당 종결 인계의
`successorHostExecutionAuthorized=false`다. 기존 KIS 읽기 전용 수집·Owner V2의
승인이나 권리를 다시 묻는 것이 아니다.

Private evidence: `wp-8/g4-admission-3de6420-a1/prepared-survey.json`에 정확한
제어·입력·이 문서의 해시를 고정한다. `wp-8/wave110/decision-question.json`에
질문 여부를 한 번 기록하며, 새 답변이 오기 전에는 실행하지 않는다.
