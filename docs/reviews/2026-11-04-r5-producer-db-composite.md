# R5-02 producer→PG 연결 검증 — 2026-11-04

**R5-02 코드·격리 통합 검증을 완료했다.** 기존 실제 checker 32단계와 이번 실제 future timer→producer→PG→LINKED 검증을 합쳐 [최종 인수 JSON](../../target/r5-paper/coordinator/r5-canonical-reader-integration-20261102/R5-02-code-acceptance-20261104.json) `f9141f76`에 기록했다. 실제 운영 Origin·활성화·운영 3세션은 이 완료 범위에 포함하지 않는다.

대상 revision은 `a30edacff987d55b5be7cca25bed7faeec589629` 위의 고정 미커밋 후보다. checker Source3는 기존 기능 인수 `bad8c259`, 격리 QA Source6는 [인수06](../../target/r5-paper/coordinator/r5-canonical-reader-integration-20261102/post-functional-g2a-20261104/composite/Root-source-acceptance06.json) `c265d3ad`와 묶음 `be079833`로 고정했다. commit/push/deploy는 하지 않았다.

- [기존 기능 통합32](2026-11-04-r5-checker-functional-integration.md): 실제 Raw→Daily 승인·sealed replay→계산·발행→Source SQL→public checker를 연결했다. 정상 fixture 3세션 Eligible, 수동·변조 ORIGIN_UNPROVEN, 다른 입력 COHORT_CHANGED를 확인했다. 이번에 반복 실행하지 않았다.
- [실제 composite 인수](../../target/r5-paper/coordinator/r5-canonical-reader-integration-20261102/post-functional-g2a-20261104/composite/Root-composite-component-acceptance.json) `aafbe196`: VM `4c733ee7-d57b-44e6-87ac-475c1474a8b4`에서 같은 원래 harness와 Rust bridge를 실행했다. 실제 timer 대기278.018초 후16:30 예약은 마감 기준7.781ms 뒤였다. 23개 원본·20개 journal event가 LINKED까지 이어졌다.
- 실제 SQL bridge의 reserve→read→link-admission→read→link-run→stop 6요청/응답과 canonical event/원본 hash 연결을 확인했다. 고정 ELF `68eda548`의 지정 ignored test는 실제1PASS/실패·ignored0이며, 같은 rig의 중복 실행이 새 source launch를 만들지 않는 원래 assertion도 통과했다.
- 정상 종료·scratch DB의 COMPLETED closed receipt, PG0/terminal/reap·log EOF/pump join, guest 자식19 terminal/reap·7 mount 해제, host 자식3 reap/QEMU0,30입력 불변과 overlay/ISO 제거를 확인했다. Parent12820의 최종 영수증은 `b773ca`/exit0, runtime receipt는 `8dec871d`다.
- 준비·검증 회귀는 **94개 PASS** (`f692b4`/exit0). 기존92 테스트 본문은 그대로다. Root가 기존 host validator와 Source6·checker Source3·기존32·runtime30핀을 다시 대조했다. 이 대조는 Native 재실행이 아니다.

원래 계획299–306은 같은 producer/relay/helper와 실제 SQL의 연결 검증에 명시적 SYNTHETIC 입력을 허용한다. G2a의 코드 검증과 G4 선행 실제3세션·WP7 활성화 후 운영3세션을 구분한다. 따라서 R5-02를 완료하고 현재 코드 검증 범위를 종료하며, WP-3는 이 실행에서 시작하지 않았다. 전체 R5·운영 인수는 미완료다.

## 실패·수정 이력

| 실행 | 결과 | 후속 및 보존 |
| --- | --- | --- |
| Native01 `95a467cf` | PAYLOAD_FILE_INVALID, PG/timer 전 중단 | ISO UID/GID0·파일0444·디렉터리0555로 수정하고 실제 xorriso metadata를 확인했다. |
| Native02 `0034c047` | ARCHIVE_PATH_INVALID, PG/timer 전 중단 | 정상 Linux multiarch 파일명의 `:`를 허용했다. 절대경로·상위탈출·제어문자·역슬래시 거절은 유지했고10,010개 archive 경로/링크를 읽기 전용 검사했다. |
| Native03 `50a2cfcf` | same-filesystem bind를 ismount가 관측하지 못함 | setup·cleanup 모두 bounded self mountinfo로 바꿨다. 이 실행의 guest rootfs unmount API는 미관측이며 전체 VM 종료·disk 제거와 구분한다. |
| Native04 `79b3f3d6` | initdb exit1 | 고정 이미지에 없는 PG18 부모를 준비기가 umask077 아래0700으로 만드는 문제를 확인했다. 새 부모0755·데이터0700을 명시했다. 당시 오류 본문은 미수출이므로 정확한 문구는 UNKNOWN이다. |
| Native05 `e5b0067c` | PG 준비·접속 성공, harness390초 CHILD_TIMEOUT | 구체적인 timeout 원인은 여전히 UNKNOWN이다. 이후 성공을 이 실행에 소급하지 않는다. |
| Native06 `4c733ee7` | PRODUCER_DB_COMPONENT_PASS | 원래 CLI/알고리즘·390초 한도를 유지하고 bounded stack/실패 산출물 export를 추가한 진단 실행이 정상 완료했다. 진단 추가가 이전 timeout의 원인을 고쳤다고 주장하지 않는다. |

[정확한 소유6실행 정산](../../target/r5-paper/coordinator/r5-canonical-reader-integration-20261102/post-functional-g2a-20261104/composite/Root-all-runtime-attempts-cleanup.json) `678ae27c`에 모든 host 자식 terminal/reap·overlay/ISO 부재·정리 오류0을 기록했다. 이는 global process quiescence 주장이 아니다. 작업자3명도 actual idle/activeTurn null/pending0으로 회수했다.

Unix socket URL은 실제 Rust parser와 기존 DB/role 재작성에 맞춰 percent-encoded host로 표현했다. TCP를 추가하지 않았다. bind 감지 변경은 [Python이 명시한 ismount 한계](https://docs.python.org/3.12/library/os.path.html#os.path.ismount)에 대응한다. 새 검사 코드의 이름/인코딩 비교 오류와 수정 이력도 private 인수·영수증에 보존했다.

Rootless TCG/no-NIC, guest CPU1·RAM1GiB·disk4GiB, host AS4GiB·polled RSS2GiB, active900초·cleanup20초를 유지했다. 시각 설정은 guest 내부에서만 한 번16:25로 했으며 host 시계를 변경하지 않았다. provider/container transport는 SYNTHETIC, production_eligible=false다. None4/default-unbound/production CLOSED와 과거 UNKNOWN을 유지한다. 실제 무인 Origin·권리·불변 release·설치·운영 세션과 전체 hosted CI는 별도 후속이다.

변경은 격리 QA harness와 연결 문자열·검증 metadata에 한정된다. 제품 서비스·Compose·DB role/table 구조를 바꾸지 않아 도표는 수정하지 않았다. Disposable harness/fixture는 기존 component 도표의 folded 경계다. 위임·회수는 [paseo-delegate](/home/l1nnx/.agents/skills/paseo-delegate/SKILL.md)를 따랐다.
