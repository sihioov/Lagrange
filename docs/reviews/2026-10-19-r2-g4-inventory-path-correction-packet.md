# R2 G4 운영 조회 경로 보정 — Wave111

상태: **PREPARED_NOT_AUTHORIZED_NOT_EXECUTED**. 이 문서는 실패한 첫 조회를
보존하고, 보정된 읽기 전용 조회 1회를 제안한다. 운영 백업·서비스 중단·배포 실행안은 아니다.

## 첫 조회의 실제 결과

Owner의 “승인하니 이어서 작업 진행해”에 따라
[Wave110 실행안](2026-10-19-r2-g4-operating-inventory-packet.md)을
2026-10-19 04:10:17 UTC 한 번 실행했다. 약 0.5초 뒤 `G4_PG_PATH`, exit 1로
종료됐으며 stderr는 비어 있었다. 서비스 동작·운영 파일 변경·SQL·provider 호출은
없었다. 데이터 root walk와 키 metadata·writer process 조회에는 도달하지 않았다.

조회 코드가 Docker 볼륨 경로를 `/var/lib/docker/volumes/<name>/_data`로 가정했다.
정확한 기존 PG 컨테이너 mount와 local volume의 Mountpoint 일치 확인 뒤 이 경로
조건에서 거부됐다. 실제 거부된 경로는 출력하지 않아 **UNKNOWN**이다. 이전 로컬
영수증에서도 그 경로를 회수하지 못했다. 특정 실제 Docker data-root를 추측하지 않는다.
앞서 읽은 unit/container 정보 역시 원본 코드가 실패 시 출력하지 않아 미회수 상태다.

첫 승인 `babc9cb1a1b01541ecd79197beef42c776ef5697670ea2144a92910f98bc868a`는
1회 실행으로 소진됐다. 종료 영수증
`f6899e8d9c8f94062221ff4f99ad00dc364d0c8185b6deae68479d9ef2959dcd`,
종결 기록 `112ad9404a910c8af32ca2a210fb79f0fad68bacd54fa505a90734921976fc2b`와
원래 제어·실행안은 불변이다. 실행 실패를 운영 서비스 장애나 데이터 변경으로 해석하지 않는다.

## 보정된 단일 실행

실행할 명령은 다음과 같다. 새 Owner 결정은
`wp-8/g4-admission-3de6420-a2/prepared-survey.json` 해시에 결합한다.
`execute-once.py`가 별도 exclusive intent를 생성한다. 첫 intent나 승인을 재사용하지 않는다.

```text
/usr/bin/sudo -n /usr/bin/env -i PATH=/usr/sbin:/usr/bin:/sbin:/bin LANG=C LC_ALL=C PYTHONDONTWRITEBYTECODE=1 /usr/bin/timeout --signal=TERM --kill-after=5s 305s /usr/bin/nice -n 10 /usr/bin/ionice -c 3 /usr/bin/python3 -B /data/worktrees/3puw275b/excited-dolphin/target/r2-daily-beta/wp-8/g4-admission-3de6420-a2/read-only-admission.py
```

두 가지를 보정했다.

1. 고정 PG 컨테이너 `b876fb12fa0c0bd0437bc3322c06d429f069e1b3c15bacc430829fa04cbc012e`의
   기존 `/var/lib/postgresql` RW mount와 그 이름으로 조회한 local volume의
   **Name·Source·Mountpoint가 정확히 일치**해야 한다. 경로는 정규 절대경로이며
   마지막 세 부분이 `volumes/<정확한 실제 볼륨 이름>/_data`여야 한다. 다른 Docker
   root를 임의로 선택하거나 fallback하지 않는다. `..`·제어문자·보존 볼륨·secret
   root를 거부하며 실제 디렉터리 metadata 조회는 기존 held-FD/no-follow 방식이다.
2. 이미 필터링된 부분 관측과 고정된 진행 단계명을 실패 시에도 출력한다. 원시 Docker
   객체·파일 바이트·예외 문자열은 출력하지 않는다. 완료되지 않은 단계와 최종 identity
   재대조는 성공으로 표시하지 않는다. 부분 정보는 유지보수 실행 가능 판정이 아니다.

그 외 [Wave110의 조회 대상·명령·보존 경계](2026-10-19-r2-g4-operating-inventory-packet.md#실행할-작업)를
그대로 적용한다. 새 하위 명령이나 네트워크 surface를 추가하지 않았다. 조회 자체 300초,
외부 305초/정리 5초, Nice 10/idle I/O, unit 128개·container 256개,
root별 metadata 200,000 entries/깊이 64/1 TiB, process 8192개·FD 전체 131072개
한도를 유지한다. 모든 file content는 기존 고정 보호 파일의 RAM 해시 외에는 읽지 않으며,
키 바이트·데이터/PG/WAL 내용은 읽지 않는다. 보존 볼륨은 metadata/reference만 대조한다.

이 제어 소스는 직접 정적으로 검토했으며 로컬 parser/test/helper 실행이나 새 독립
리뷰는 하지 않았다. 수용된 앱 `e639169`와 복구 도구 `3de6420`, CI 및 이미지·통합
증거는 변경하지 않았다. hosted maintenance 41개 통과는 이 private 조회 코드의
실행 검증이 아니다.

## 종결과 다음 경계

새 승인이 있다면 위 조회를 **1회만** 실행하고 전체 또는 부분 terminal receipt를
회수한다. 실패·UNKNOWN·드리프트에 대한 자동 보정이나 재시도는 없다. 기존 failed
persistent KIS unit을 reset/delete/filter/relaunch하지 않는다. 실제 값에 따라 필요한
소스 또는 구체적 운영 처리를 별도로 검토한다.

기존 Wave110의 `executionLimit=1`, `automaticRetryAuthorized=false`와 종결 기록의
`successorHostExecutionAuthorized=false` 때문에 이 보정 실행에는 새 명시 결정이
필요하다. 기존 KIS/Owner V2 권리를 다시 묻는 것은 아니다. 서비스 중단, backup capture,
clone 실행, migration, current 변경, timer 활성화, 배포·Owner 인수·연속 실제 3거래일·
최종 C_R2는 여전히 이 실행안 밖에 있다. 과거0120 cleanup INCOMPLETE를 유지한다.

Private source correction: `wp-8/wave111/source-correction.json`.
Final controls and input hashes: `wp-8/g4-admission-3de6420-a2/prepared-survey.json`.
One decision is recorded in `wp-8/wave111/decision-question.json`; a pending question is not approval.
