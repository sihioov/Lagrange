# R5-03 코드·격리 통합 검증 인수

2026-11-05 KST, **R5-03 DONE — 코드와 격리된 합성 workflow 범위**.
대상은 `a30edacff987d55b5be7cca25bed7faeec589629` 위의 미커밋 R5 후보다.
실행 종료 후 [최종 입력 대조](../../target/r5-paper/coordinator/r5-03-20261104/final-input-verification-20261105.json)에서 905개 입력 해시가 모두 일치했다.
커밋·push·배포·운영 활성화는 수행하지 않았다.

Owner 한 명의 승인된 추천을 T일 종가 preview, 명시 apply 또는 채택된 AUTO,
실제로 사용 가능한 다음 E 세션 시가 재산정, 원자 정산, E 종가 평가로 연결했다.
실제 실행 파일·보호 설정·Current 바인딩과 SQL 권한을 실행 전에 확인한다.
KIS 계좌·잔고·주문 API에는 연결하지 않았다.

## 실제 전체 workflow 결과

[SEALED Native27 실행 receipt](../../target/r5-paper/coordinator/r5-03-20261104/sealed-workflow/runtime/9da05863f3475ee3647a263efc509ae5/receipt.json)는
`FUNCTIONAL_PASS_NOT_ORIGIN`, `ALL_EXACT_PHASES_AND_CHECKERS_PASSED`로 종료했다.
**60개 정확한 phase가 모두 exit0**였고, 의도한 두 SIGKILL 경계는 별도 증거로 검증했다.
receipt SHA256은 `524f40f440190d4eb3d50a6c266c33d2b90d02e0e0ac49d9de6c3f5ae79ee71b`다.

- 실제 Raw/token/parser/writer, 역할별 V3 읽기 전용 view, Daily 승인·계산·게시,
  public Source SQL과 보호된 API/worker를 함께 실행했다. 다섯 합성 세션의
  달력·입력 계보·F/W hash·T/E 가격 가용성을 확인했다.
- Owner 동의 생성·같은 요청 replay·다른 내용 충돌, 다른 사용자 접근 거절,
  preview READY, 수동 apply/replay와 E 이전 대기·경제적 무변경이 통과했다.
- 금융 capture 4건과 최종 checker 4건이 통과했다. 정상 입력은 인정하고
  수동·변조 입력과 다른 cohort는 지정된 거절 결과를 반환했다.
- 실제 worker가 orders의 미획득 RowExclusiveLock에 대기할 때 SIGKILL137로
  종료했다. 커밋 전에는 주문·체결·비입금 원장·outbox·평가가 남지 않았다.
- 정산 후 daily_equity 쓰기 잠금에서 다시 SIGKILL137로 종료했다.
  정산 영수증·주문·체결·원장·outbox는 유지됐고 평가는 없었다.
  재시작에서는 정산 재실행0·평가1이며 기존 경제적 identity가 보존됐다.
- 다음 정상 회차의 AUTO preview 생성·적용과 기존 outbox 전달이 통과했다.
  다음 E에서는 동일 신호의 **NO_TRADE / SKIPPED / VALUED**를 확인했다.
  추가 주문·체결 없이 평가와 현금·보유 수량 정합성을 검증했다.

## 마지막 결함과 수정 검증

Native26은 무거래 정산·평가를 이미 commit한 뒤 같은 회차에서 새 추천 입력을
읽기 시작했고, 90초 cycle 한도에 도달했다. 고정된 단계명만 기록하는 진단과
종료 후 읽기 전용 상태 확인으로 이 순서를 관측했다. fixture는 오류가 나도
runtime이 살아 있는 동안 pool을 닫고 최초 오류를 보존하도록 수정했다.
Native25의 관측되지 않은 내부 원인은 UNKNOWN으로 남긴다.

[현재 worker](../../crates/api-server/src/paper_session/owner_beta.rs:253)는 성공한
정산 또는 평가가 있으면 그 회차의 결과를 반환한다. 새 추천은 다음 정상 회차에서
보호 설정·Current·권한과 갱신된 계정 상태를 다시 확인한다. 금융 정책, SQL,
원자성, 권한 재검증, 기한은 변경하지 않았다.

Native27에서 재시작 평가 phase는 55.554초, 다음 E의 무거래 정산·평가 phase는
94.413초에 종료했다. 후자의 bound worker 구간은 85.565초였다.
두 경우 모두 due 완료 직후 callback을 반환했고 새 capture가 없었다.
그 사이의 별도 AUTO 회차가 실제 preview 1건을 게시·적용해 후속 진행도 확인했다.
이는 **설정된 90초 디버그 fixture 검증**이며 제품 기본15초의 운영 성능 인수가 아니다.
실행 중인 blocking callback의 취소는 협조적이다. timeout의 즉시 강제 종료를
증명한 결과가 아니며 CommitUnknown readback과 WP-5의 실제 drain 확인이 필요하다.

## 검증 근거와 정확한 범위

| 검증 | 결과와 범위 |
| --- | --- |
| [SQL Native14](../../target/r5-paper/coordinator/r5-03-20261104/sql-native14-acceptance.json) | 10 PASS, 0 FAIL, 0 ignored: 실제 격리 PG 기능9 + source1. 실제 역할/FORCE RLS, unset·empty·foreign actor 복원, stale/중복/동시 실행, 권리·epoch, 정산 후 자정 경과 평가 복구, migration 재적용·빈 rollback·이력 보존 거절을 포함한다. 선행 데이터는 synthetic fixture다. |
| [API-Native01](../../target/r5-paper/coordinator/r5-03-20261104/api-native01-acceptance.json) | DB 기반22 PASS: Owner-beta HTTP/접근4, 일반 Paper 리밸런스10, runner8. 해당 receipt의 원래 핀에 대한 결과이며 최신 소스에서 22건을 다시 실행했다고 주장하지 않는다. |
| [r27 집중 검증](../../target/r5-paper/coordinator/r5-03-20261104/due-progress-r27-report.md) | API lib4 PASS, native 실행 파일 빌드, API all-target Clippy PASS. 899개 build 입력의 전후 해시가 동일했다. |
| [r26 진단 검증](../../target/r5-paper/coordinator/r5-03-20261104/next-session-diagnostic-r26-report.md) | 정상 API lib check, DB 없는 진단·cleanup 테스트2, driver pure30, native 빌드·Clippy PASS. r27은 진단·driver를 변경하지 않았다. |
| [달력 호환성](../../target/r5-paper/coordinator/r5-03-20261104/sealed-current-calendar-compat-report.md) | focused77 PASS와 현재 선택 방식의 실제 보호된 5세션 workflow PASS. 과거 prior-selected 보호 prefix 전체의 runtime 재생은 미실행이며, 실제 Raw·공유 호환 helper·정확한 이전 세션 바이트 검증과 구분한다. |
| 최종 빠른 검사·문서 | Rust fast PASS: `target/dev-check/20261104T213243Z-48935874ba20`. Root 실제 diff·입력 핀 검토 및 공백 검사. 두 PUML/PNG 로컬 렌더 PASS; 마지막 변경은 근거 줄 번호이며 decoded pixels 동일. |

이전 계산·capture·runtime·factor·OpenAPI 검증과 실패 기록은
[진행 기록](2026-11-04-r5-paper-ledger-integration.md)에 보존한다.
중복 실행의 PASS 수를 합산하지 않는다. 전체 hosted CI 또는 운영 관측을 수행했다고
주장하지 않는다.

최종 [r27 build-ready](../../target/r5-paper/coordinator/r5-03-20261104/sealed-workflow/build-ready-20261104-final-r27.json)
SHA256은 `ee89d1756de9367c6badf367908017078e3896020f5349a0916bbfd68a96d25b`,
[Root admission](../../target/r5-paper/coordinator/r5-03-20261104/sealed-workflow/root-ready-9da05863f3475ee3647a263efc509ae5.json)
SHA256은 `79e678be548a9359e1272c2e3e47e5426d7ed0f7fe5b17501ca9dff7d732f4d3`다.
이 핀들이 HEAD 위의 정확한 미커밋 소스·migrations·도구·실행 파일을 특정한다.

## 정리와 다음 책임

[최종 실행 정리](../../target/r5-paper/coordinator/r5-03-20261104/sealed-native27-cleanup-review.json)에서
소유 컨테이너215·보조5·볼륨3 제거, 자식 종료·회수, 입력 불변을 확인했다.
[작업자 대조](../../target/r5-paper/coordinator/r5-03-20261104/final-worker-reconciliation-20261105.json)는
소유 작업자9 idle·active turn 없음·승인 대기0을 확인했다. 소유 heartbeat `4fc557dd`도 삭제했다.
이전에 소유하지 않은 자원이나 호스트 전체의 정지 상태를 주장하지 않는다.

[WP-4/WP-5 인계](../handoffs/2026-11-04-r5-paper-api-runtime.md)에 화면/API와
설치·설정·drain/restore·회차 실행 책임을 남겼다. 공개 수동/AUTO 전환 API를
검증했다고 주장하지 않는다. workflow의 mode 변경은 기존 동의·epoch·설정을
유지하는 fixture 전용 설정이었다.

**운영 Origin, G4 선행 실제 세션, WP-5 활성화·복원, WP7 무인 관측과 Owner 운영 인수는 OPEN**이다.
None4/default-unbound/production CLOSED를 유지한다. 다음 로드맵 작업은
R5-04 운영 전환과 복구이며, 이 인수는 운영 데이터·계좌·주문 권한을 확장하지 않는다.
