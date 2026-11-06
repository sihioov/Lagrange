# R5 carrier 로컬 컴파일·모델 검증 — 2026-11-02

R5-02의 Source 검토를 실제 Rust 검증으로 진행했다. 초기 Linux carrier 전체 타깃 컴파일과 수정 core 모델37개가 통과했고, carrier는170통과·6실패였다. 후속 격리 복제본에서는 실제 실행 로직을 공유하는 사업 규칙 모델과 공개 경로 거절 검사를 함께 유지해 대표1개·관련30개·전체176개가 모두 통과했다. 초기 실패 이력은 아래에 보존한다. 후속 두 파일의 독립 리뷰는 지원되는 P1/P2 없음으로 인수했다. R5-02·Native·운영 게이트 완료를 뜻하지 않는다.

## 검증 대상

- 채택된 부모는 S27 Source864의 한정 Source 기준 `4dfcebff07df013b09cbe5e363baa649b1552e85540b0e4153b92f33387ae5f8`이다. 기존 스냅샷과 그 인수 기록을 수정하지 않았다.
- 실제 빌드 후보는 `target/r5-paper/build-support/s27-b850bb1e-compilefix04-20261102/`다. 부모의 864개 payload 중 열 파일을 수정했고 원본·기존 테스트 본문을 보존했다. `carrier/Cargo.lock` 등 빌드 입력을 별도로 고정했다.
- 실제 toolchain은 Rust 1.97.1, 관측된 host는 `x86_64-unknown-linux-gnu`다. Carrier package는 `r5-paper-fixture-carrier`, binary는 `research-worker`다.
- 실제 오프라인 생성 carrier lock은 `8abdf1578b14c71e6ff266f316c1f73961517bd26dde8953fef86ea650e461fa`의 481개 package다. 기존 저장소 lock을 호환된 것으로 추정하지 않았다.

## 변경과 이유

실제 컴파일이 확인한 오류에 한정했다. 공용 `Owner<File/OwnedFd>`의 네 표준 trait 구현을 그 타입을 정의하는 crate로 옮겼다. 기존 close·settle·drop 코드는 바꾸지 않았다. Daily 후보는 실제 열린 파일에서 받은 `Attempt`를 보관해 기존 참조 반환 API를 유지한다. 모듈 경로·반환형 표기·정산 guard의 borrow 종료도 수정했다.

Checker의 SQLx import를 설치된 0.9 공개 경로에 맞췄다. 트랜잭션 시작 오류에서는 실제 실패 Result의 borrow를 끝낸 뒤 같은 connection을 닫는다. 세션 증거를 먼저 소비·정산한 다음 부모 정산 closure를 만들어 기존 최초 오류와 정산 순서를 보존했다. Native DB는 실행하지 않았다.

Carrier에서는 이미 캡처된 실제 mutable Context 참조를 BEFORE에서 이동해 FnOnce를 보장하고, 생성 거부 시 기존 오류를 유지하면서 반환형을 맞췄다. 합성 응답의 날짜 문자열 참조 타입과 이미 공급된 하위 모듈의 경로를 수정했다. 요청 값·whitelist·정책·운영 권한을 변경하지 않았다.

열 파일의 원본 대비 patch는 `target/r5-paper/coordinator/record-capture-build-context-s29/Source27-to-compilefix04.patch`다. SHA-256은 `78435f245f42168a1379ab895d96deac9f33ec8ba1183a2d0d401631b89d4250`이며 원래 지원본에 대한 `git apply --check`가 통과했다. 이 patch는 Source snapshot의 `carrier/`·`tree/` 배치용이며 현재 canonical 작업 트리에 적용하지 않았다.

## 실행과 결과

| 검사 | 결과 | 근거와 범위 |
| --- | --- | --- |
| Carrier 전체 타깃 | PASS, 원래 exit 0 | `cargo +1.97.1 check --offline --locked --package r5-paper-fixture-carrier --all-targets`; 실제 입력 동일, 진단 전체. `carrier-repair04-compile-pass.json` SHA `dac761cc841a2933e7cef515d3a23d18bf36a035d5165dff61c97afa1eb5a5b3` |
| 수정 core 실제 모델 | 37 PASS, 0 FAIL/ignored | `r5-close-custody` Source `ebd4b88b376a4cab3492c3c9d29898a66ba2fefd9a3f35ad6cfb74c498074fc1`; 실제 `--lib` 테스트, 입력 동일, 진단 전체 |
| Carrier 실제 모델 | FAIL, 170 PASS / 6 FAIL | 총176, ignored/filtered0, 원래 exit101, 모든 입력 동일·진단 전체. 테스트/Source 변경·skip 없음 |
| 독립 열 파일 통합 검토 | P1/P2 NONE | 다른 작업자가 열 파일 재구성·전체866핀·최초 오류·정산 순서를 검토. 실제 compiler/type 검증과 구분 |
| Linux host metadata | PASS | 실제 host filter의 413개 resolve node/로컬 13개 package. 전체 JSON SHA `f65225aa24209d85f3ade9cae89741e05bcd3ddafa4b6843aa5c4f2e4bc9409d` |
| Tree lock | PASS | 누락된 실제 auth library 입력을 공급한 별도 지원본에서 507개 package 생성. 전체 module·cfg 폐쇄성 인수는 아님 |

모든 Cargo 실행은 기존 QA helper와 같은 외부 private 공유 advisory slot, Cargo jobs 2를 사용했다. 모델 테스트는 단일 스레드이며 codegen 크기를 줄이려고 테스트 debug 정보만 0으로 설정했다. Debug assertions·overflow·features·target은 바꾸지 않았다. 최종 CI·hosted integration을 대체하지 않는다.

## 실패·미확인 이력

초기 컴파일 오류와 각 수정 전 진단은 별도 지원본에 보존했다. 최초 Root carrier 테스트는 heartbeat로 turn이 중단된 뒤 session 도구가 원래 handle을 찾지 못했고 완료 receipt가 없어 원래 exit·테스트 결과를 UNKNOWN으로 유지한다. 이번 명령에 한정한 process 관측은 그 시점의 비일치 사실일 뿐 종료·전역 quiescence의 증거로 삼지 않았다.

작업자 실행은 잘못 전달된 핀, 체크포인트 전후의 임시 PATH와 기대 환경 지문 혼용, 이후 상세 원인이 없는 OSError에서 거절됐다. 이를 성공으로 바꾸지 않았다. 현재 실행은 최소한의 비밀 없는 환경을 고정하고 실제 Popen PID와 원래 종료를 기록하는 지속 터미널을 사용한다. 구 84001·S10 및 이전 writer의 UNKNOWN 이력은 별개로 유지하며 조회·재실행하지 않았다.

## 남은 테스트 실패

첫 지속 실행은 168 PASS / 8 FAIL이었다. 셸의 실제 umask0002와 `tempfile`의 기본 DirBuilder가 private 디렉터리 검사를 거부하게 하는 점을 확인했다. 테스트 프로세스에만 umask0077을 적용한 재실행에서 두 DayStore 상태/namespace 검사는 통과했다. Source·UID·검증 조건을 느슨하게 바꾸지 않았다.

초기 검증본에서 실패한 기존 테스트 6개는 다음과 같다.

- `action_supply::tests::failed_supplier_burns_day_allowance_and_does_not_retry`
- `action_supply::tests::next_day_uses_verified_frontier_without_reusing_previous_reservation`
- `action_supply::tests::supplier_rejects_large_gap_without_chopping_or_reserving`
- `action_supply::tests::supplier_source_check_does_not_reserve_and_capture_revalidates_on_restart`
- `action_supply::tests::supplier_timeout_consumes_attempt_and_never_replays`
- `action_supply::tests::unrelated_complete_batch_cannot_satisfy_consumed_identity`

실제 Source의 `CommittedRawReader::snapshot`은 dormant overlay에서 checked ownership 우회를 막기 위해 항상 `UnsupportedPlatform`을 반환한다(`source.rs:197`). Legacy action-supply는 `source_plan`에서 이 경로를 사용한다. 이 기존 경로의 성공·timeout·재시도 소진 기대값을 현재 스냅샷에서 인수하지 않았다. 닫힌 reader를 다시 열거나 테스트를 제거/ignore/skip하지 않았다. 실제 checked 경로의 Native fixture·호출자 인수가 다음 조건이다.

## 여섯 사업 규칙의 공통 실행 검증 — 후속 복제본

Owner의 재개 지시와 `$paseo-delegate`에 따라 독립 진단 후 새 `s27-b850bb1e-legacyfix01-20261102` 지원본을 만들었다. 원본866개 입력을 복사했고 supplier·range 정책 두 파일만 수정했다. Legacy와 checked runner는 같은 business engine과 snapshot 정책을 호출한다. 기존 여섯 이름·사업 규칙은 ordinary 관측 모델로 검증하고, 실제 legacy 거절/provider0 및 checked scope·미바인딩 거절은 그대로 검증한다. 실제 mock 수집의7/1회 호출·timeout을 사용하지만 모델 journal을 Native·durability 증거로 주장하지 않는다.

Root는 helper의 성공값 반환형 오류1개를 바로 보정했다. 수집이 성공해도 후속 독립 관측이 없으면 `ConsumedIncomplete`로 실패하는 검사를 보강했고, 미사용 provider closure가 실제 DayStore 정산 뒤 해제되는 기존 순서를 유지했다. 두 파일 외864개 입력은 그대로다. 구 지원본과 Source27 원본·봉인 metadata는 수정하지 않았다.

| 실행 | 실제 결과 |
| --- | --- |
| 수정 전 대표 | 0 PASS / 1 FAIL / 175 filtered, 원래 exit101 |
| 수정 후 대표 | 1 PASS / 0 FAIL / 175 filtered, exit0 |
| 관련 supplier 전체 | 30 PASS / 0 FAIL / 146 filtered, exit0; 여섯 사례와 checked·DayStore 영향 범위 포함 |
| 최종 carrier 전체 | 176 PASS / 0 FAIL / 0 ignored·filtered, exit0 |
| 실제 두 파일 rustfmt | exit0·stdout/stderr0, 하위 모듈을 건너뛰지 않음 |

모든 실행은 같은 채택 H1 lifecycle·공유 private slot·Cargo2·offline/locked·단일 테스트 스레드·umask0077을 사용했고 입력이 전후 동일했다. 대표 실행기 최초 오류와 작업자 셸의 현재 slot errno30도 실패로 남겼다. Root의 같은 슬롯 진입과 실제 Cargo 종료는 별도 관측이며 과거 오류 원인을 소급 확정하지 않는다.

후속 증거는 `target/r5-paper/coordinator/r5-six-legacy-tests-resume-20261102/ledger.json`, 실제 영수증은 새 지원본 `receipts/representative-after-02`, `affected-after-01`, `full-after-01`이다. 새 controller snapshot을 만든 것으로 취급하지 않는다. Source27 대비11개 파일의 통합 patch는 `Source27-to-legacyfix01.patch`이며 원본에 대한 적용 검사도 통과했다. 다른 제작자의 두 파일 리뷰는 지원되는 P1/P2 없음으로 인수했다(`review/report.md` SHA `7bc0bdb80f500960fe0404cc65c146b7192e42bdae53424dad73fffbb4d99521`). 최종 로컬 derivative map은 `local-verified-derivative.json`, SHA `e589a759af36768c941210367f56a01971ad4767ad818deafc2849d028e296d9`다. 실제 Native calendar/reconstruction과 모델의 일부 interior-gap/mixed-witness 분기는 이 여섯 사례로 검증하지 않았다. 새 수정본의 별도 all-targets/Clippy·hosted CI·Native 실행을 이 모델 테스트로 대체하지 않는다.

## 남은 인수 조건

현재 결과는 선언된 Linux host/default-feature 범위다. Native 보호 reader·실제 UID/FD/guard·DB·root origin·전체 G2a·세 번의 실제 무인 거래 세션·운영 활성화는 인수하지 않았다. 기존 reconcile은 partial/unbounded/mutating이며 한도는 strict read별이다. `fullstrictRW=false`, `None/PLAN`, runtime CLOSED, production/whole-engine false와 전체 게이트 OPEN을 유지한다.

선택된 입력·정의·산출물 누락은 없다. 전체 물리 resolver/cfg/macro/generated 및 원래 native reader 내부의 미반환 prefix, 현재 거부 row, panic/abort/OOM·durability·Origin은 이 결과로 증명하지 않는다.
