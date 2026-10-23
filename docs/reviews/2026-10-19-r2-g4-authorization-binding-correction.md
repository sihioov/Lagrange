# R2 G4 조회 승인 경로 오류 회수와 보정 — Wave113

상태: **Wave112 승인 조회 1회 실패 종결. Wave113 보정 사본 준비, 미실행.**
담당 작업은 R2-04/R2-09다. 앱 `e639169`의 hosted·12개 이미지·실제 격리 통합과
도구 `3de6420`의 hosted CI 인수는 유지한다. 운영 배포와 G4 백업은 아직 실행하지 않았다.

## 확인된 실패 원인

2026-10-19 04:44:27 UTC에 승인된 a3 실행기를 한 번 호출했다. 실행기는 Wave112
승인과 50개 봉인 입력을 확인했지만, 내부 root 조회기의 승인 파일 경로가 Wave111로
남아 있었다. 조회기는 V3 범위를 요구하면서 V2 승인을 읽어
`G4_NEW_AUTHORIZATION_REQUIRED`, exit 1로 종료했다. 이전 a2 BASE 경로도
사용되지 않는 상수로 남아 있었다. 이는 준비 과정에서 경로 치환을 빠뜨린 코디네이터
오류다. 승인이 부족하거나 운영 서버의 접근 권한이 거부된 문제가 아니다.

실패는 첫 운영 관측 전에 발생했다. 새 systemd/Docker 조회, 운영 파일 조회,
SQL·provider 호출·서비스 동작·파일 변경은 없었다. root 조회 프로세스는 실제로
실행됐으므로 이를 미실행으로 바꾸지 않는다. 종료 출력 224 bytes와 stderr 0 bytes의
길이·해시 및 유한한 실패 필드를 회수했다. 50개 입력은 종료 후에도 바이트가 일치한다.

| 증거 | SHA256 |
| --- | --- |
| Wave112 승인 | `8b43cd7684ea9df62df704982e0b9e6b0c71234ed3e334186e8f7ca993db44d1` |
| a3 종료 결과 | `b065beb9893f901761dd6d572cdb14c02efef3196b5eb3e33af9cc6cd07b6e2a` |
| a3 종결 기록 | `9e538ad5ff83568d0be81dbadb06515e2e76562a0297afb5323a323279c1f91b` |

기존 a1/a2/a3 제어·실패·승인·준비 증거는 수정하지 않는다. Wave111의 실제 부분
관측은 그대로 유효한 과거 증거이며, 새 키 metadata·자원·writer·최종 재대조 결과는
여전히 미확보다. 이전 정적 역변환 검사는 파일 간 경로 일치까지 증명하지 못했다.

## 보정한 사본과 정적 검증 범위

새 사본은 `wp-8/g4-admission-3de6420-a4`다. 조회기와 실행기 각각에서 다음
세 종류의 리터럴만 바꾼다.

- BASE는 모두 `g4-admission-3de6420-a4`.
- 승인 파일은 모두 `wave113/owner-authorization.json`.
- 승인 범위는 모두 `G4_READ_ONLY_SURVEY_V4_ONCE`.

원본과 역변환한 전체 바이트가 일치하는지 확인하고, 별도로 조회기·실행기·ledger의
BASE·승인 경로·scope·앱/도구 SHA·실행 argv를 같은 기대값과 대조한다. 실행 제어의
이전 a2/a3, Wave111/112, V2/V3 참조가 남지 않았는지도 확인한다. 증거에 대한
과거 경로 참조는 ledger에 계속 보존한다.

조회 동작·대상·한도·실패 처리에는 변경이 없다. [Wave112의 고정 대상과 보완 동작](2026-10-19-r2-g4-partial-inventory-and-completion.md)을
유지한다. 특히 키는 metadata만 읽고, 없는 잠금 파일은 부재로 기록하며 생성하지 않는다.
누락이나 unit 변화가 있으면 수집 완료와 인수 성공을 구별한다. 운영 적용 허용은 항상 false다.

```text
/usr/bin/sudo -n /usr/bin/env -i PATH=/usr/sbin:/usr/bin:/sbin:/bin LANG=C LC_ALL=C PYTHONDONTWRITEBYTECODE=1 /usr/bin/timeout --signal=TERM --kill-after=5s 305s /usr/bin/nice -n 10 /usr/bin/ionice -c 3 /usr/bin/python3 -B /data/worktrees/3puw275b/excited-dolphin/target/r2-daily-beta/wp-8/g4-admission-3de6420-a4/read-only-admission.py
```

이 검증은 정적 바이트·문자열 대조다. 사본 실행, project parser/helper/compiler/test,
새 독립 리뷰는 하지 않았다. 실제 운영 조회 성공이나 전체 제어의 실행 검증으로
표현하지 않는다. 실행 시에도 다른 원인으로 실패할 수 있으며 부분 결과까지 회수한다.

## 결정 범위와 다음 단계

새 결정은 최대 300초 읽기 전용 조회 **1회와 종료 결과 회수**만 포함한다.
서비스 중단, 권한 변경, 키·데이터 내용 읽기, SQL, 백업·clone·배포·timer 활성화,
provider 호출과 자동 재시도는 포함하지 않는다.

이미 승인된 [Wave112 실행안의 승인 범위](2026-10-19-r2-g4-partial-inventory-and-completion.md#승인-범위)는
`executionLimit=1`, `automaticRetryAuthorized=false`였다. 실제 root 프로세스를
한 번 실행했으므로, 코디네이터 오류라도 그 승인을 새 사본에 옮기지 않는다.
그 제한 때문에 보정 사본 실행에 새 결정이 필요하다. 이전 운영·EOD 권리를
다시 확인하려는 요청은 아니다.

조회 결과 이후의 G4 준비에는 `/data` 소유권과 cold PG 경로의 명시적인 신뢰 결합,
기존 볼륨 부모 보호·원상 복구, disabled persistent failed 유닛의 보존 및 빈 cgroup
검증, 전체 writer 차단, 독립 복구 절차가 필요하다. 기존 보안 검증을 임의로 낮추거나
과거 실패 유닛을 삭제하지 않는다. G4·배포·Owner 사용·연속 실제 3거래일·C_R2는 미완료다.
