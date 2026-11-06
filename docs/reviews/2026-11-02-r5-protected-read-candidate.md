# R5-02 고정 소스 후보와 보호-read 실행 대상

2026-11-02 Owner 재개 지시에 따라 `$paseo-delegate`로 진행했다. 기존 carrier176 통과를 반복하지 않고, 검증 전용 소스 묶음과 별도 읽기 전용 executable을 준비했다. 이 결과로 실제 보호 읽기·Origin/G2a·Paper 운영을 완료 처리하지 않는다.

## 통합 경계

검증 대상 package는 `r5-paper-fixture-carrier`이며 synthetic provider를 포함한다. 기존 bin 이름이 같아도 canonical package `collectors`의 운영 `research-worker`와 다르다. Canonical storage3667행과 derivative30921행의 API·legacy 동작도 다르므로 overlay 전체를 제품에 복사하거나 검증 이미지를 운영으로 승격하지 않았다.

다음 단위로 독립된 `r5-protected-read-proof`를 채택했다. 기존 contract/parser/Context/strict selector/cleanup을 그대로 재사용한다. Plan은 gate 전에 반환하며, 다른 모든 parsed 실행 요청은 실제 gate를 먼저 거친다. Gate 뒤에도 `InspectStrictEvidence`만 runtime에 도달하고 수집·action·legacy 실행은 거절한다. 기존 worker·dependency·feature·바인딩은 변경하지 않았다.

## 고정 산출물

- 소스 후보: `target/r5-paper/candidates/r5-02-e589a759`. 원본 소스864개와 carrier lock1개가 원본과 바이트 단위로 일치한다. Manifest SHA `31286db5866888f4dc36812d9947f464d5336e48d37e690edcb5412047b191e2`.
- 실행 후보: `target/r5-paper/candidates/r5-protected-read-proof-79237868`. ELF와 source delta, 설명을 고정했다. Manifest SHA `b646d4e50b1edbdfb8dcf7515e724efc2314ca23984466252977bae978be9b1a`.
- ELF SHA `792378687ca6f03d7888a492d86b9a5ae5a0001df5baa227ddbb7720ed778555`, 26,799,520 bytes. 선언된 Linux GNU host/Rust1.97.1의 비운영 후보이며 Docker/Alpine/musl 실행을 입증하지 않는다.
- 변경 Source: 새 `carrier/src/protected_read_proof.rs` SHA `629597ab0134ac94cd56ca742b81f9c3a2b80ca5d70ccc6faeff4f5582e284b2`, bin stanza만 추가한 `carrier/Cargo.toml` SHA `9837d223ef84d48c5589ef557b5a3337ae55b85ed3da2ff7baf2a329a9d4bf86`. 새 working copy의 기존866개 입력 중 manifest1개만 바뀌었고 나머지865개는 동일하다. Source base·lock·기존 Source 후보를 덮어쓰지 않았다.

## 실제 검증

| 검사 | 결과·범위 |
| --- | --- |
| Source freeze와 readonly verify | 865개 declared payload PASS. 잘못된 첫 map/manifest 핀은 mapped member 읽기 전에 거절. 미선언 파일의 디렉터리 완전성은 주장하지 않음 |
| 새 target 테스트 | 6 PASS / 0 FAIL / 0 ignored / 175 filtered, 원래 exit0. 기존176개는 재실행하지 않음 |
| 실제 새 binary `--plan` | exit0; distinct target·provider_calls0·production_eligible=false·bindings=NONE 출력 |
| 실제 inspection 요청 | 예상한 exit2, typed `Unbound`, capture_count0·sources빈값·close_reportNone. 실제 gate 거절이며 보호 I/O 성공이 아님 |
| 형식 | 실제 새 ROOT items 전체·inline6cases check0/출력0·emit 동일. 파일 링크 전체 check는 변경하지 않은 runtime/sidecar/synthetic의 기존 reflow로 exit1; PASS로 처리하거나 baseline을 쓰지 않음 |
| 다른 제작자의 Source2 리뷰 | 지원되는 새 P1/P2 없음. [private report](../../target/r5-paper/coordinator/r5-execution-candidate-20261102/proof-review/report.md) |
| 격리 도구 사전 확인 | 로컬 Docker29.7.2와 기존 고정 linux/amd64 이미지의 읽기 전용 metadata 확인. Container/VM create/start/exec·pull/build 없음 |

세 Cargo 명령은 같은 H1 helper·공유 private slot·jobs2·offline/locked를 사용했다. DEV/TEST debug 정보만0이며 debug assertions·overflow·features·target은 바꾸지 않았다. 전후 입력8021개가 동일했고 모든 소유 child/tool handle은 실제 종료까지 기다렸다. 실제 영수증은 `target/r5-paper/build-support/r5-protected-read-proof-20261102/receipts/proof-{test,plan,deny}-01/`이다.

Source 후보 생성기의 최초 parent 부재, 중복 `receipts/` 경로와 metadata alias 검증 보정, worker 조회·보고 오류는 private 기록에 보존했다. 실행기·통제 source를 성공으로 소급 변경하지 않았다. 기존84001/S10/이전 writer lifecycle UNKNOWN도 그대로이며 프로세스 탐색·재실행·전역 quiescence 추론은 없다.

## 다음 실행 조건

실제 보호 읽기 성공은 아직 미검증이다. Contract의 `SCOPE/TARGET/IMAGE_PROVENANCE/CARRIER_REVISION/DEPENDENCY_HASH/PROTECTED_BINDING_HASH` 여섯 값과 storage의 `SCOPE/TARGET/IMAGE/REVISION` 네 값은 모두 None이다. 새 실행 후보의 정확한 target/image/revision/dependency 및 실제 protected binding·원본/event chain·UID·명시적 borrowed limits를 한 실행 계약으로 먼저 선정·봉인·검토해야 한다. 캐시된 일반 이미지나 요청 batch UUID가 이 권한을 대신하지 않는다.

Namespace current-reader component의 과거 인수와 일반 관측 모델은 실제 Root source origin을 증명하지 않는다. 현재 selector는 checked Vec reader를 사용하므로 observed prefix 복구도 주장하지 않는다. Reconcile은 여전히 partial/unbounded/mutating, 한도는 read별이며 `fullstrictRW=false`다. Native/Origin/전체 G2a·실제3세션·R5-02·운영 활성화는 OPEN이다.
