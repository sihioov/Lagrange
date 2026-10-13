# R2 U2 — ETF 일일 입력·자동 추천 결정안

- 상태: **제안; Owner 채택 전.** 구현·실데이터 승인·운영 활성화 근거가 아니다.
- 대상: R2-04/08/09, WP-4. 작성일 2026-10-13.
- 배경: [계약 분석](2026-10-13-r2-contract-and-recovery.md)의 v3는 2026-08-28까지다.
  현재 요청 기반 추천은 그 입력을 사용하고, 일일 owner-beta enqueue 경로는 없다.
- 기존 권리: [ADR-0005](../decisions/0005-kis-personal-use-entitlement.md)의 개인·단일 Owner
  이용과 보관 승인을 유지한다. 권리·KIS endpoint 승인을 다시 요청하지 않는다.

## 채택할 제품 동작

**권고안:** 기존 v3를 보존하면서, 이후 거래일의 검증된 ETF11 EOD를 별도의 불변 일일 입력으로
누적한다. Owner가 자동 갱신을 켠 전략에 대해서만, 해당 거래일 입력이 독립 검증을 통과한 뒤
하루 한 번 추천을 생성한다. 실패·누락·승인 대기 중에는 이전 추천과 그 실제 기준일을 표시한다.

이 안을 채택하면 매일 사람이 데이터마다 승인 버튼을 누르는 대신 아래 고정 규칙의 검증기가
일일 입력을 승인할 수 있다. 검증 정책 자체와 최초 기반 입력은 코드 리뷰·Owner 승인으로 고정한다.
데이터 생산자가 자신이 쓴 파일을 곧바로 승인하는 구조는 허용하지 않는다.
[ADR-0006](../decisions/0006-etf11-ten-year-historical-coverage.md)의 개별 v3 승인과 구분되는
새 승격 정책이므로 이 부분에 Owner 결정이 필요하다.

## 입력과 승격 규칙

| 항목 | 권고 규칙 |
| --- | --- |
| 범위 | 고정 ETF11, Owner 전용, PRICE_RETURN_ONLY, vendor_snapshot=true, strict_pit=false 유지 |
| 과거 기반 | v3의 정확한 5-pin 및 2016-08-29..2026-08-28 입력 보존. v2/v3 재현 경로도 보존 |
| 누적 범위 | 2016-08-29 기반에 이후 검증된 완료 세션을 추가. 베타 중 rolling 삭제·새 lookback 정책 없음 |
| 새 입력 | 별도 version/tag. 기반 hash, 날짜별 price/reference/action/calendar 출처·파일 hash·원래 retrieval 시각, 코드·정책 version, 정확한 공통 세션 목록을 결합 |
| 데이터 의미 | 원가격 semantics 일치가 필수. 단일 과거 capture로 표시하거나 여러 날의 수집 시각을 오늘로 바꾸지 않음 |
| 완전성 | 각 추가 세션 ETF11 전체 관측·기존 Raw/정규화 검증·calendar/source binding·연속 거래일 coverage가 모두 맞아야 승인. 한 종목/한 날짜 누락도 차단 |
| Corporate actions | 베타의 자동 승격은 검증된 현금배당 제외 정책만 수용. 비현금 조정 필요·불명확한 action은 중단하고 별도 검토. 조정계수·날짜 추정 금지 |
| 독립 검증 | producer와 분리된 검증 경로가 기존 Raw/manifest/정규화 근거를 다시 읽어 hash·count·coverage·정책을 확인. producer가 쓸 수 없는 admission 영역에 append-only 증거를 남김 |
| 재현성 | 내용 주소를 가진 새 artifact/admission. 승인 정책 hash와 모든 lineage를 추천 입력에 봉인. 기존 artifact·추천을 덮어쓰지 않음 |
| 정정/충돌 | 같은 세션의 다른 데이터나 과거 수정 발견 시 자동 덮어쓰기·재추천 금지. 새 revision의 별도 검토/승격 필요 |
| 재시도 | 같은 bytes/정책의 재실행은 같은 입력으로 수렴. 불완전·상충·취소된 승인으로 발행 금지 |

일일 Raw 발행이나 generic dataset READY만으로 위 승격이 성립하지 않는다.
처음 누적 입력을 만들려면 8월 28일 이후의 실제 누락 구간을 확인해야 한다. 필요한 과거 수집 날짜·
시도 예산은 WP-1 DB 관측 후 구체화하며 이 계약 채택이 무제한 catch-up 수집을 승인하지 않는다.

## 추천 예약과 화면 규칙

1. 전략마다 `자동 갱신`을 명시적으로 선택한다. 기존 설정은 자동으로 opt-in시키지 않는다.
   최초 실제 Owner 인수에서 정상 화면으로 선택하며, 서비스가 Owner 세션을 가장하지 않는다.
2. 16:30 KST 이후 완료 세션만 대상으로 한다. 수집 완료와 독립 입력 승격을 확인한 뒤 예약한다.
   자원·입력 상태를 최대 분당 한 번 확인하며, 시장 API를 polling 용도로 호출하지 않는다.
3. 일일 예약의 영구 중복 키는 Owner·전략 설정 식별자·거래일·예약 계약 version을 묶는다.
   처음 예약할 때 정확한 전략 snapshot/hash와 승인 입력 revision/hash를 원자적으로 봉인한다.
   설정이나 입력이 당일 바뀌어도 두 번째 자동 실행을 만들지 않는다. 새 정책이 필요한 정정은 중단한다.
4. 기존 owner capacity·entitlement·전략 활성 상태·소유자 격리·원자 발행·stale claim 검증을 유지한다.
   비활성화·권한 철회 후에는 새 예약/발행을 허용하지 않는다. 수정된 설정과 과거 snapshot을 혼합하지 않는다.
5. 재시작·동시 producer·실패 후 재시도는 같은 예약/실행에 수렴한다. 수동 요청은 별도 기존 경로로
   구분하고 일일 무인 성공의 대체 근거로 세지 않는다.
6. 기대 거래일, 데이터 지원 끝, 추천 as_of, 발행 시각을 구분한다. 기존 18:30 freshness 마감은
   유지하되 지연·실패를 성공으로 보정하지 않는다. 해당 일 자동 실행이 누락되면 3거래일 관찰을 재시작한다.
7. 과거 backlog를 이유로 모든 과거 추천을 자동 생성하지 않는다. 최초 활성화의 기대 세션과 실행
   예산을 G4/G5에서 고정한다. 과거 일별 추천 backfill은 이 계약 범위에 없다.

## 구현 준비와 검증 범위

채택 후 WP-4는 기존 intermediate brief를 그대로 실행하지 않고 설계·구현 묶음으로 다시 분해한다.
새 입력은 historical-price-only의 기존 pin 의미를 재해석하지 않는 별도 tagged 계약이어야 한다.
현재 factor builder와 queue input/SQL guard가 역사 artifact 타입에 결합돼 있어 단순 range_end 수정으로
구현할 수 없다. `crates/factor-engine/src/price_only.rs`,
`crates/job-queue/src/owner_beta/{input,compute}.rs`,
`migrations/0050_owner_beta_strategy_snapshots.up.sql`이 이 결합의 근거다.

- 설계 묶음: immutable daily input/admission 구조, 독립 권한·저장 경계, 역사/일일 replay 구분,
  additive schema/RLS/queue와 API/화면 계약. exact 파일 소유권·migration·diagram 영향 확정 후 구현 위임.
- 입력 묶음: provider 없는 ETF11 fixture에서 정상/빈틈/중복/원가격 불일치/action/lineage/hash 정정 검증.
- 예약 묶음: 실제 역할의 격리 DB에서 opt-in·원자적 중복 방지·설정 변경·권한 철회·재시도·동시 실행 검증.
- 표시 묶음: 구/신 입력, pending/stale/unsupported·미래 날짜·다른 Owner 접근을 HTTP/화면으로 검증.
- 인수 묶음: exact candidate의 CI·이미지·새 migration/복구 검토, 승인된 운영 적용 후 실제 Owner와
  연속 3거래일 관찰. 코드 검증만으로 R2 완료 처리하지 않음.

새 운영 배포·DB migration·서비스 중단·timer 활성화·누락 수집의 명령과 영향은 G4에서 별도로 확정한다.
대안으로 기존 수동 역사 추천을 유지할 수 있지만, 그 경우 R2-04/08/09는 계속 BLOCKED다.

## 검증 및 결정 기록

코디네이터가 ADR-0005/0006, 현재 historical approval reader, Owner input/compute,
API enqueue의 snapshot·capacity·idempotency 결합을 읽고 작성했다. 코드나 운영 동작은 바꾸지 않았다.
새 규칙은 제안이며 구현·테스트·독립 설계 리뷰·Owner 채택은 아직 미완료다.
