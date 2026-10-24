# R4-04 무결성 검증 기록

작성: 2026-10-24. 기준 `900057c259cd2397c4d41147029930c9be83750b`.
상태: **DOING**. 수량 회귀·실행 ID 변화 보완 및 G3 독립 소스 리뷰를 수용했다.
최종 후보의 테스트·CI·smoke·실제 실행은 아직 수행하지 않았다.

계획은 [R4-04 Paseo 계획](../superpowers/plans/2026-10-24-r4-integrity-verification.md),
기존 실제 실행은 [9a0b14f 수용 기록](2026-10-24-r4-actual-simulation.md)을 따른다.
고정 계약 SHA-256 `8c5c6bf7f07de09f14f1e5eaa2b6479be4b10e8b98ed5a89a280ac95568e87cd`는 유지한다.

## G1 감사 수용

WP-1의 실제 idle·pending0·보고서 회수 후 코디네이터가 manifest32개 해시와
27개 소스/harness의 검증된9a0b14f 바이트 일치를 확인했다. 계약204–209/231행,
결과 검증기의 최대 매수·공통 시가·q=0 경계, 기존 fixture와 검증 예제를 직접 대조했다.
관측된 새 생산 버그는 없으며 아래 두 직접 회귀가 빠져 있었다.

Private 감사 보고서 SHA-256은
`3a4352aab182ffe6c3904ff0fd428444630fb0fb810f77c7eb93bfc6de888a4f`,
입력 manifest는 `9acdbe1a753573ff0a576182a8d06c0d9b3b1627b119fe756cd9b17a55331a3b`다.
감사는 기존 실행 기록을 소스와 대응시킨 것이며 새 테스트 실행은 아니다.

## 검증 항목과 남은 작업

| 항목 | 현재 근거 | 이번 작업과 인수 경계 |
| --- | --- | --- |
| 동일 승인 입력 재현 | 9a0b14f 두 프로세스×각2회 canonical bytes/hash 일치 | 기존 증거 보존. 변경된 예제는 두 번째 run/config ID를 다르게 하여 최종 후보에서 직접 확인한다. |
| 미래정보 차단 | `factor-engine/bars.rs`의 as-of 뒤 행 제외, 실제 TrendFactor50/200 미래가격 변조 회귀와 target 미래행 거절 | 기존 named tests를 최종 CI에서 확인한다. vendor snapshot·strict_pit=false 한계는 유지한다. |
| 분할 중복 반영 | 승인 입력의 action_count=0 및 raw/adjusted OHLC 일치 검증, 변경 권리 label 거절 회귀 | v1에서 분할 입력을 거절하는 경계다. 분할 수량 조정 기능의 검증으로 표현하지 않는다. |
| 비용·최대 정수 수량·잔여 현금 | hand-calculated ledger, 비용 올림, positive underfill, 목표 불변 no-retry 회귀 | C1: 반대 원장에 시가 증거가 있는 거짓 q=0를 전략/벤치마크 양쪽에서 거절. C2: q+1 증가·다음 notional overflow 직접 거절. 구현·formatter 완료; 독립 리뷰 완료; hosted 실행 대기. |
| 지표·반올림 | 정확 분수 합산, 마지막 ties-to-even, 장기 회전율, 지표 변조 거절 회귀 | 최종 CI로 재확인. UI 표시 인수는 현재 범위가 아니다. |
| 누락·중복·가격 | 승인 달력×ETF11 전체 교차곱, 날짜·가격·거래량 및 원장 정렬 검증 | 기존 회귀를 재확인한다. 실제 입력의 전일값 대체는 허용하지 않는다. |
| pin·계보·내용 변조 | 5개 승인 pin, factor 정의/source digest, target hash, 폐쇄 wire와 원장/지표 일관성 검증 | binary attestation은 외부 provenance로 별도 확인한다. parser가 외부 원본 snapshot을 역산한다고 주장하지 않는다. |
| 두 비용 프로필 원자성 | 두 결과가 모두 검증된 후 pair 반환, 두 번째 profile 실패 회귀 | 순수 함수 반환 경계다. 저장/API의 부분 성공 비노출은 R4-05에 의존한다. |
| 사용자 격리 | 기존 Owner admission 및 추천 replay owner binding | **OPEN — R4-05 의존.** 현재 R4 결과의 저장·RLS·조회 경로가 없어 다른 사용자 간 결과 접근/재사용을 검증할 수 없다. |

## 사용자 격리 의존성

채택 계약은 Owner ID를 계산 해시에서 제외하면서 권한은 별도 owner-scoped job/result
저장·RLS로 강제하도록 정한다. 현재 `SimulationRunRequest`는 실행/config UUID와 기간을 받고
메모리 안의 계산 결과를 반환한다. OWNER_ONLY 메타데이터, UUID 변화 불변성,
기존 추천 API의 접근 거절을 R4 시뮬레이션 결과의 사용자 격리 증거로 대신하지 않는다.

R4-05 연결 후 인증 actor에 따른 소유권, 다른 사용자 간 목록·ID/hash 기반 조회/재사용 거절,
동일 계산 hash의 별도 소유권 유지, 실패·부분 pair 비노출을 검증해야 한다.
이번 범위에서 새 queue/DB/API/Web 연결을 만들지 않으며 R4-04 전체 체크박스도 유지한다.

## 최종 검증 계획

독립 리뷰 후 정확한 후보를 고정해 기존 CI와 required, Research smoke를 각각1회 실행한다.
검증 예제 변경 때문에 source-only optional build를 함께 실행하고 해당 후보·run·attempt·binary
hash/provenance를 확인한다. 기존 승인 root를 read-only로 사용해 자원 제한된 격리 프로세스1회에서
서로 다른 실행 ID의 내부2회 결과를 비교한다. 제공자 데이터는 업로드·복사·재수집하지 않는다.
새 실행 증거는 별도 private 경로에 기록하고 이전9a0b14f 결과와 섞지 않는다.

현재 형식 검사 성공은 컴파일·회귀 실행·실제 반복 성공을 대신하지 않는다.

## G2 소스 대조

2026-10-24 WP-2가 실제 idle/pending0으로 종료했다. 두 회귀의 diff와 helper 인수·fixture를
대조했고 생산 영역 바이트 불변과 source/report/receipt 해시를 확인했다. q=0 사례는 개별
원장·목표 경로가 유효함을 먼저 확인하고 공통 시가 검증 및 세 결과 진입 경로의 InvalidLedger를
요구한다. q+1 사례는 현재 체결 유효성과 정상 unaffordable 대조를 포함한다.
Rust fast formatter PASS를 수용했으며 compile/test/Clippy와 실행 ID 변경의 실제 결과는 NOT_RUN이다.

## G3 독립 리뷰 수용

2026-10-24 리뷰 작업자 종료·대기 요청 없음·보고 회수와 전후 소스 해시 일치를 확인했다.
필수 수정 및 선택 권고는 없었다. C1은 의도한 shared-price 거절 경계를, C2는 현재 체결이
정상인 뒤의 q+1/다음 notional overflow를 검사한다는 소스 판단을 수용했다. 검증 예제는
동일 provenance·입력에서 다른 UUID 요청의 두 결과를 비교한다. 컴파일·실행 증거는 아직 없다.
보고서 SHA-256: `8916bc6e595146ce377a589ea8ae0db03e35f90bc4c001fa1de274dbaf6e30de`.

새 private 실행 준비는 기존 컨테이너 소유권·정리·HeavySlot 함수 바이트를 유지하고
프로세스 수만1회로 줄였다. 정리 관련 모의6개가 통과했으며 실제 Docker는 실행하지 않았다.
후속 기록은 최종 후보의 CI/required·smoke·새 binary provenance·서로 다른 ID의 실제 반복
결과를 확인한 뒤 추가한다. R4-05 사용자 격리 의존성은 그대로 OPEN이다.
