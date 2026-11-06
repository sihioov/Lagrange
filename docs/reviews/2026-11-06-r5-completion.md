# R5 후속 구현·검증 기록 — 2026-11-06

Owner는 정기 백업 설정 복구보다 R5 마무리를 우선하고 delegator 실행을 지시했다.
기존 R5 계획의 WP-4~7과 G3~G5를 따른다. 별도 R2/R3 유예 조사와 정기 백업
설정 복구를 이번 구현의 선행 작업으로 확대하지 않는다. 실제 Paper 활성화에 필요한
입력·권리·무인 세션 증거와 복구 조건은 보존한다.

## 기준과 범위

- 작업 브랜치: `r5-completion-20261106`.
- 코드 기준: main `972fe37d9909626cb6dd0f665cbaf2723f3742d3`.
- R5-03은 PR #10 병합과 같은 main의 필수 CI·smoke 성공까지 확인했다.
  기존 완료·실패 근거는 [게시 검토](2026-11-05-r5-publication-review.md)에 보존한다.
- 기존 로컬 운영 빌드 수정 `d68480e`는 `review-recent-work-history` 브랜치에
  보존했다. 이번 R5 브랜치에 다른 작업을 섞지 않는다.
- 현재 단계는 R5-04 소스 구현이다. 배포·활성화·실제 거래일 관찰·Owner 인수는
  각각 별도 증거가 필요하며, production CLOSED를 유지한다.

## 위임과 소유 범위

`paseo-delegate`의 MCP 완료 알림을 켜고 같은 workspace
`wks_a246209eeeb36cce`에 작업자를 배치했다. 모델·실제 effort, 부모 연결과
작업 경로를 시작 직후 확인했다. 기본 권한을 우회하는 모드 변경은 하지 않았다.

| 작업 | 작업자 | 모델·effort | 소유 범위 | 상태 |
| --- | --- | --- | --- | --- |
| WP-4 Owner Paper 화면 | `df221e08-1382-4131-b1a6-5d99d302af11` | `codex/gpt-6-luna`, max | 계획의 Web Paper 파일·집중 회귀 | 구현 전달·정리 완료, coordinator 보완/Biome PASS; type/Vitest/browser·hosted 미완료 |
| WP-5 계약 대조 | `396fbd02-3b84-4c1d-89da-d7cedb7e7313` | `codex/gpt-6.1-sol`, high | 기존 ops/Rust/SQL 읽기, 구현 가능한 범위와 누락 접점 보고 | 회수·idle/대기0 확인, 소스 대조 수용 |
| WP-5 P0 운영 lifecycle 계약 | `93382b36-8fee-4596-9f78-c2d92fa05021` | `codex/gpt-6-astra`, xhigh | 새 운영 계약·closed schema, 후속 SQL/native/ops 접점 | 회수·idle/대기0, 소스 계약 채택 |
| WP-5 P3 독립 compile identity | `00b80246-e550-4075-bfd0-5552be6d35ea` | `codex/gpt-6-luna`, max | D1/D7 build revision과 두 source Dockerfile·집중 회귀 | 소스·집중 검증 수용; broad fake resource gate/actual ELF·hosted 미완료 |
| WP-5 P1 SQL lifecycle | `2f5984b0-4bff-4384-b88a-0b699f2c4bd1` | `codex/gpt-6-luna`, max | 신규0068·SQL 회귀·audit projection | 구현 중 |
| WP-5 P2 native bridge | `7a88f64c-6dc6-4483-9c10-e81ba6ffc1da` | `codex/gpt-6-luna`, max | job-queue consuming operations library·api-server native bin·집중 회귀 | 구현 중 |

코디네이터는 통합 판단, ROADMAP·계획·도표와 최종 검증을 맡는다. Web 작업자와
ops 작업자의 쓰기 범위를 겹치게 하지 않는다. 작업자별 직접 검증·미해결·미검증
보고를 회수하고 실제 idle 및 대기 요청 부재를 확인한 뒤 인수한다.

## 현재 gate와 다음 행동

1. WP-4는 확정 OpenAPI와 [API/runtime 인계](../handoffs/2026-11-04-r5-paper-api-runtime.md)에
   따라 비활성·차단·동의·preview·적용·정산·평가 표시를 구현한다.
2. WP-5는 기존 설치·drain·backup 패턴과 public SQL 전이를 대조해 누락된
   receipt/CLI/보존 경계를 구체화한 뒤, 범위가 확정된 구현으로 이어간다.
   병합된 0067을 수정하거나 직접 테이블 쓰기로 OFF 거절을 우회하지 않는다.
3. 통합 후 G3 후보를 고정하고 기존 hosted 필수 CI·smoke로 검증한다.
   구조 변경이 있으면 두 `.puml`의 실제 근거와 로컬 PNG를 함께 갱신한다.
4. G4/G5는 실제 운영 Origin, 현재 권리·입력, 활성화 전 EOD/추천 3거래일과
   활성화 후 Paper 3거래일, 복원·Owner 사용 확인을 각각 요구한다.
   합성 fixture 또는 90초 debug 실행을 실제 운영·기본15초 성능 인수로 확대하지 않는다.

## 검증과 한계

착수 시 working tree가 깨끗하고 기준 브랜치가 main을 가리키는 것을 확인했다.
위임 시작은 구현·집중 검사 통과의 증거가 아니다. 현재 새 소스·CI·배포·복원·실제
운용 인수 결과는 미회수다. 실제 세션이 없으면 R5-05를 완료 처리하지 않는다.

## WP-5 계약 대조 회수와 영향 분기

읽기 전용 대조의 private 보고서는
`target/r5-completion-20261106/ops-audit/report.md`다. 원본 source의 중요한 접점을
코디네이터가 직접 대조했다. 이번 대조는 테스트나 실제 운영 인수가 아니다.

- 0067의 OFF/RESTORED_OFF 거절을 해소할 protected receipt와 maintenance-safe
  readback이 필요하다. 기존 파일 bytes를 보존하고 후속 0068을 사용한다.
- 기존 checker 공개 결과는 설명용 verdict이며 SQL ENABLE의 전체 입력이나
  native lifecycle 소비자가 아니다. worker/admitter helper에 admin 권한을 추가하지 않는다.
- D1/API와 D7/Paper의 official recipe는 `compile_commit=unset`이고 두 source
  Dockerfile도 컴파일 시 revision을 전달하지 않는다. 현재 Rust runtime은 그 revision을
  요구한다. 이 독립적인 source 불일치는 P3의 첫 구현 범위로 확정했다.
- Paper-only 중지와 combined backup은 writer 목록·동결 범위가 다르다. 실제 작업
  종료와 commit unknown 해소, old-lineage 명시 복구, Paper-aware backup이 필요하다.
  현재 계획의 ops 파일 목록만으로는 이 경계를 구현할 수 없어 해당 WP-5 분기에
  한해 후속 Rust/SQL 접점을 추가한다. 구체적인 쓰기 소유권은 P0 인수 후 확정한다.
- 기존 API/runtime 인계의 heartbeat 경고를 현재 코드에 맞게 정정했다.
  기본10초/lease60초와 configured heartbeat 전달, operation15초/cycle90초 및
  내부 action90초를 구분한다. 실제 갱신·성능 인수는 아직 없다.

P0는 열린 프로토콜 문제라 hard/Astra xhigh로 배정했다. 확정된 P3의 revision
전달 수정은 기계적 회귀와 직접 소스 검증이 가능해 Luna max부터 실행한다.
나머지 WP-5 구현은 P0 인수 후 SQL(P1), native consumer(P2), 설치 통합(P3),
lifecycle controller(P4), backup(P5)의 의존 순서로 이어간다.

## P0 인수와 P1 착수

[운영 프로토콜](../contracts/owner-beta-paper-operations-v1.md)과 operations/archive
closed schema를 소스 계약으로 채택했다. 작업자 stdlib 구조/canonical124·request-key10
검사와 별도로 코디네이터가 두 schema bytes/hash와 기존 여섯 계약·0067 up/down 불변을
대조했다. 이 결과는 실제 SQL·프로세스 종료·복원·운영 인수가 아니다.

P0의 native 진입점을 D7의 기존 단일 `api-server` Cargo package와 맞추도록
코디네이터가 조정했다. 검증 로직은 `job-queue` operations library에 두고,
`crates/api-server/src/bin/owner-beta-paper-ops-native.rs`에서 좁은 consuming bridge만
호출한다. 이 작은 통합 수정으로 heterogeneous-package recipe를 새로 만들지 않는다.
작업자 원본 보고서 해시는 이 조정 전 bytes이며 현재 프로토콜의 상태와 구분한다.

0068은 P1 작업자에게 독점 예약했다. 새 migration·maintenance readback·epoch/receipt
재실행 안전성과 old-lineage recovery grant를 구현하고, P5와 공유할 정확한 audit
projection을 실제 schema에서 고정한다. 기존0067·경제적 이력·public ABI는 보존한다.
현재 새 구현의 실제 DB·native·hosted CI 결과는 아직 없다.

## P3 compile identity 소스 인수와 자원 정산

대상은 baseline `972fe37` 위 P3 미커밋 파일이다. D1/D7의 selected revision을
실제 Cargo 환경으로 전달하고 두 source-builder의 전체40ASCIIhex 검사를 연결했다.
집중 producer13·guard17·static/syntax가 통과했다. 코디네이터는 두 실제 Dockerfile
조건식을 `/bin/sh`로 실행해 각 정상1·잘못된6 입력을 독립 대조했다. 최초의 multiline
허용 실패는 보존하고 byte-length 검사 후 거절되는 것을 확인했다. 기존 pre-D8 해시는
유지하며 의도한 D1/D7 두 compile policy만 역사 대조 시 정규화한다.

넓은 fake 자체검사의 private-umask Git clone0700/예상0755 충돌과 fixture copy의
ownership 보존 실패를 fixture 범위에서 보정했다. 실제 제품 권한 검사는 유지한다.
후속 runtime payload/native identity 단계는 통과했으나 fake apply는 실제
`mem-available-below-floor`에서 중단됐다(702276KiB, 기존 하한2097152KiB).
이 suite를 성공으로 기록하거나 resource gate를 우회하지 않았다. 실제 ELF·이미지와
동일 후보 hosted 검증도 미완료다.

코디네이터 재관측 당시 available803MiB/swap 가득 참, 운영11개 healthy와 최근30분
kernel journal의 OOM 일치0을 확인했다. 결과 회수가 끝나고 실제 idle/대기0인 소유
audit/P0/P3 작업자만 archive해 available2302MiB로 회복했다. 다른 작업자·서비스·잠금은
종료하거나 변경하지 않았다. 현재 Web/P1 두 작업은 유지하며, 새 로컬 heavy 시작은
그 시점의 전제와 공통 slot을 다시 확인한다. 무거운 통합은 기존 hosted job을 따른다.

## WP-4 전달 회수와 hosted 검사 준비

Web 작업자의 실제 idle/대기0와 private 보고서
`target/r5-completion-20261106/web/report.md`를 회수했다. 동의·AUTO·preview/apply·
T/E/자료 가용/정산/평가·정확 decimal·비교 불가 표시와 계약/surface/E2E 사례를
구현했다. 작업자 검증은 fixture `node --check`와 whitespace까지이며, 로컬 필수
패키지가 없어 fast/focused preflight가 차단됐다. 제한된 npm ci도 실패했다.
TypeScript/Vitest/OpenAPI/build/browser 통과로 확대하지 않는다.

코디네이터는 lockfile SHA512와 일치하는 Next16.3.0 공식 tarball에서 server/client와
useRouter 가이드를 읽었다. 전체 설치 없이 같은 방식으로 Biome2.5.7 실행 파일을
확인해 Web workspace에서 사용했다. 회수본의 JSX 구문·hook 의존성·array index key를
작은 통합 수정으로 보완하고, 첫 pending preview GET 실패 뒤 명시 재조회가 가능하도록
버튼과 E2E 사례를 추가했다. 오류·수정 이력은 private check 로그에 보존한다.

- 변경10파일의 workspace Biome check/format: PASS.
- `npm run lint --workspace @lagrange/web`: exit0,309파일 검사,6 warning/69 info.
  변경10파일 자체는 별도 검사에서 오류/경고0이다. 기존 무관한 경고는 수정하지 않았다.
- 새 pending-read 회귀는 작성했으며 실제 browser 실행은 아직 없다.
- TypeScript/OpenAPI/Vitest/build/browser 및 최종 동일 후보 CI/smoke는 미완료다.

완료분의 hosted 검사를 위한 체크포인트에는 Web/P0/compile identity와 진행 기록만
넣는다. P1 SQL/audit projection과 P2 native의 진행 중인 파일은 해당 체크포인트의
검증 대상이 아니다. R5-04/G3 전체 완료를 뜻하지 않는다.

## 첫 checkpoint CI와 Web 타입 보완

완료분 checkpoint는 `6a4ce2a33a47af5924394bf5ed30333d66b94b3b`이며
[CI42170740484](https://github.com/sihioov/Lagrange/actions/runs/42170740484)를 요청했다.
첫 Web job은 lint 다음 TypeScript에서 누락된 apply body schema import와
Owner-beta 번역 사전의 타입 선언 오류를 확인해 실패했다. OpenAPI/Vitest/build/browser는
그 뒤 단계라 미실행이다. 다른 job 결과는 별도로 회수한다.

코디네이터가 두 Web 파일만 보완했다. apply body schema를 import하고, beta 전용
label/error 함수 타입을 generic 사전에서 OwnerBeta 사전으로 옮겼다. 고정된 문자열
label 표는 readonly string map으로 조회해 알려지지 않은 코드의 fallback을 유지한다.
이 변경으로 TypeScript 검사나 기존 assertion을 완화하지 않는다. 해당 두 파일의
workspace Biome check/format은 exit0이다. 실제 TypeScript와 후속 검사는 수정 커밋의
hosted CI로 다시 확인하며, 최초 실패·건너뜀 이력을 보존한다.
