# R2 WP-4 D2 — Owner-beta Web 접점 지도

- 기준선: `619634db32526032aae07ee2024a10f69fc560c4`.
- 범위: 기존 `apps/web` 전략 설정·Owner-beta 추천 경로, 직접 쓰는 DTO·route/state helper·테스트와 API DTO. 소스 수정, UI 정책/설계/구현, 실행 테스트·빌드는 하지 않았다.
- 기준 계약: [U2 채택 결정](2026-10-13-r2-daily-input-decision.md), Owner 채택 2026-10-14. 전략별 자동 갱신은 명시 opt-in이고 기존 설정은 OFF로 간주한다. 기대 거래일, 데이터 지원 종료, 추천 `as_of`, 발행 시각은 별도 의미로 다룬다.

## 현재 경로

### 전략 설정 화면

| 파일·심볼 | 현행 역할 / 관찰 |
|---|---|
| [`apps/web/app/(authenticated)/strategies/page.tsx`](<../../apps/web/app/(authenticated)/strategies/page.tsx#L22>) — `StrategiesPage` | `getProductApi()`로 catalog와 licensing을 병렬 조회(22–27행), `permitsUse`에 따라 설정 가능 여부 결정(35–40행). login-required는 `/login` redirect, entitlement/forbidden은 blocked, 나머지는 unavailable 화면(42–54행). |
| [`apps/web/components/strategies/strategy-catalog.tsx`](../../apps/web/components/strategies/strategy-catalog.tsx#L27) — `StrategyCatalog` | 전략별 catalog 카드와 `StrategyConfigForm` 표시(27–63행). retired 전략이나 권한이 없는 설정은 인라인 blocked 상태. 현재 구성 목록을 함께 읽거나 기존 구성의 자동 갱신 설정을 편집하는 UI는 확인되지 않았다. |
| [`apps/web/components/strategies/strategy-config-form.tsx`](../../apps/web/components/strategies/strategy-config-form.tsx#L103) — `strategyConfigPath`, `StrategyConfigForm` | 전략 parameter schema를 읽어 입력을 만들고, CSRF mutation으로 `POST /api/v1/strategies/{id}/configs`에 `{ config, is_active: true, strategy_version }`를 제출(103–147행). 이 폼은 기존 전략 설정 생성 지점 후보지만, owner-beta 자동 갱신 필드/기존 config 갱신은 현재 계약에 없다. |
| [`apps/web/lib/products/contracts.ts`](../../apps/web/lib/products/contracts.ts#L39) — `strategySchema`, `strategyConfigSchema` | catalog·strategy config Zod 계약(39–66행). config 계약은 API 생성형 `StrategyConfig`를 감싸며 추가 자동 갱신 속성은 이 타입에서 보이지 않는다. |
| [`apps/web/lib/api/product-client.ts`](../../apps/web/lib/api/product-client.ts#L291) — `getStrategyConfigs` | `GET /api/v1/strategy-configs`를 `strategyConfigPageSchema`로 파싱(291–292행). 전략 페이지는 이 method를 부르지 않지만 추천 페이지가 사용한다. |

### Owner-beta 추천 및 결과 화면

| 파일·심볼 | 현행 역할 / 관찰 |
|---|---|
| [`apps/web/app/(authenticated)/recommendations/page.tsx`](<../../apps/web/app/(authenticated)/recommendations/page.tsx#L95>) — `OwnerBetaRecommendationsProductPage` | Owner beta에서 licensing, supported-as-of discovery, strategy configs, history를 병렬 조회(103–123행); 활성 config만 수동 실행 폼에 전달(122–153행). `run_id`로 상세 조회, in-flight 상태·성공 보고서·history를 분기 렌더링(156–171행). entitlement/forbidden·login·일반 오류 분기(174–184행). 정상 Owner beta 여부는 session role과 access mode로 구분(197–207행). |
| [`apps/web/components/recommendations/owner-beta-run-form.tsx`](../../apps/web/components/recommendations/owner-beta-run-form.tsx#L19) — `OwnerBetaRunForm`, `submitOwnerBetaRun` | 전략 config와 지원 `as_of`를 선택해 CSRF POST 실행(32–43, 45–93행). 날짜는 `supported_as_of` membership을 재확인하고 schema parse 후 `{ as_of, strategy_config_id }`만 전송(52–90행). 자동 실행 예약/설정 제어는 없다. |
| [`apps/web/components/recommendations/owner-beta-run-status.tsx`](../../apps/web/components/recommendations/owner-beta-run-status.tsx#L18) — `pollOwnerBetaRun`, `OwnerBetaRunStatus` | no-store same-origin GET으로 상태를 polling하고 terminal 상태에 `router.refresh()`(36–49행); pending/running/failed/canceled 상태 패널·poll error 표시(51–80, 83–149행). 이는 수동 run 진행 상태이며 daily input admission pending/stale 상태가 아니다. |
| [`apps/web/components/recommendations/owner-beta-report.tsx`](../../apps/web/components/recommendations/owner-beta-report.tsx#L1) — `OwnerBetaReport` | 성공 추천 결과와 lineage/hash·전략·`as_of`·생성/처리 timestamps를 렌더링한다. `created_at`을 publication time으로 의미 변경하면 안 된다. 지원 종료·기대 세션 필드는 현 report DTO에 없다. |
| [`apps/web/components/recommendations/owner-beta-history.tsx`](../../apps/web/components/recommendations/owner-beta-history.tsx#L12) — `OwnerBetaHistory` | history table은 `as_of`, `created_at`, status, run id 표시(27–53행). 승인 입력 지원 종료/expected session은 표시하지 않는다. |
| [`apps/web/lib/products/owner-beta-contracts.ts`](../../apps/web/lib/products/owner-beta-contracts.ts#L115) — `ownerBetaSupportedAsOfSchema`, `ownerBetaRunSchema`, route constants | discovery는 `default_as_of`와 정렬/중복 없는 `supported_as_of`(115–140행); run은 historical input kind, `as_of`, `created_at`, `updated_at`, optional start/finish, 상태/lineage를 엄격 검증(190–222행). Request body는 `as_of`, `strategy_config_id`만(308–315행). 경로 상수와 상세 path helper는 327–336행. 신규 daily admission/readiness DTO는 없다. |
| [`apps/web/lib/api/product-client.ts`](../../apps/web/lib/api/product-client.ts#L236) — owner-beta client methods | `getOwnerBetaRecommendationRuns`, detail, supported-as-of를 각각 dedicated 경로 + Zod schema로 읽는다(236–241행). 일반 recommendation API와 분리돼 있다. |

### API DTO·접근 경계

| 파일·심볼 | 현행 역할 / 관찰 |
|---|---|
| [`crates/api-server/src/http/owner_beta.rs`](../../crates/api-server/src/http/owner_beta.rs#L28) — enqueue/read handlers | POST enqueue는 owner-only mode, enabled input, strict `as_of`, 날짜 상한, entitlement, 지원 세션, config ownership/active 및 queue 오류를 검사한다(28–188행). history와 supported-as-of GET은 owner-only + entitlement를 재검사(226–310행); detail 경로는 actor 범위 조회 후 해당 `as_of` entitlement 검사(313–363행). `owner_beta_read_allowed`는 access mode와 owner role을 결합(365–368행). |
| [`crates/api-server/src/http/dto.rs`](../../crates/api-server/src/http/dto.rs#L540) — `OwnerBetaPriceOnlyRunBody`, `OwnerBetaPriceOnlyRunDto`, `OwnerBetaPriceOnlySupportedAsOfDto` | request는 config ID와 `as_of`; enqueue response는 run/job/status; discovery는 `default_as_of`·`supported_as_of`(540–560행). |
| [`crates/api-server/src/http/dto.rs`](../../crates/api-server/src/http/dto.rs#L562) — read run/list/item DTOs | 상세/목록은 strategy/config ID·version, recommendation `as_of`, status, input contract, hashes, `created_at`/`updated_at` 및 선택적 started/finished(562–657행). 목록은 결과 items를 생략(605–642행). expected session, input `support_through`, 별도 `published_at`은 DTO에 없다. |
| [`apps/web/components/pages/owner-beta-product-route.tsx`](../../apps/web/components/pages/owner-beta-product-route.tsx#L17), [`apps/web/lib/api/contracts.ts`](../../apps/web/lib/api/contracts.ts#L11) — `OwnerBetaProductRoute`, `permitsOwnerBetaProduct` | server session을 확보해 product 접근을 먼저 차단하며 refusal 전에는 render callback을 실행하지 않는다. API middleware는 권위 경계로 남는다. 로그인 redirect와 Member 거부의 helper는 session/API 경로와 분리되어 있다. |

## 요구 표시·오류 상태와 현행 대응

| U2 항목 | 현재 표현/근거와 간극 |
|---|---|
| 전략별 자동 갱신 default OFF | 현재 전략 config 저장은 `is_active: true`만 POST하며 auto-update 개념 없음. U2 요구는 “기존 설정 opt-in 아님”; 현 UI는 자동 동의하지 않으므로 기본 OFF 근거는 될 수 있지만 persisted opt-in UI/DTO는 아직 없다. |
| 기대 세션 / 입력 지원 종료 / 추천 `as_of` / 발행 시각 | `as_of` 선택·latest supported label은 recommendations page와 form에서 표시한다. report/history는 run `as_of`와 `created_at`을 보여준다. 현재 API는 기대 세션 및 지원 종료를 보내지 않고 publish timestamp도 별도 제공하지 않는다. 미래 구현에서 `created_at`을 `published_at`으로 간주할 근거 없음. |
| pending | 현재 recommendation job `PENDING`/`RUNNING`은 `OwnerBetaRunStatus`로 표시·polling. U2의 daily admission/enqueue 대기 의미는 별도 DTO/status 미존재. |
| stale / unsupported / future date | stale indication 없음. unsupported 날짜는 form에서 supported list membership 검사 후 오류; API는 미래 서울 날짜 거부하고 정확한 지원 session 검사. daily input의 stale와 unsupported 사유 구분은 아직 UI 계약에 없음. |
| Member / owner isolation | Web `OwnerBetaProductRoute`에서 role/access policy 전에 callback/API read가 수행되지 않으며, API read handler도 owner-only mode와 actor-is-owner를 재검사. 현재 fixture/browser tests에 Member 차단 사례가 있음. |
| 인증/session 오류 | `StrategiesPage`와 recommendation page는 login-required면 `/login`; `getServerSession`도 로그인 필요 오류를 redirect한다. API browser polling 오류는 `parseBrowserApiResponse`의 오류 문자열을 status 영역에 표시. 명시적인 session-expired 복구 상태는 이 제한 탐색에서 확인되지 않음. |
| 권한/entitlement/일반 API 오류 | strategies는 `DATASET_BLOCKED`, `DATA_ENTITLEMENT_REQUIRED`, `FORBIDDEN`을 blocked, 기타 unavailable로 분리. owner-beta recommendation도 같은 세 코드를 blocked 처리, 다른 오류는 unavailable. enqueue 오류는 폼 alert, polling 오류는 status role로 표시. |

## 직접 관련 검증 harness 목록

- Vitest setup: `apps/web/package.json:10–16`, `apps/web/vitest.config.ts:1–10`. 단일 파일 예: `npm --prefix apps/web test -- tests/owner-beta-recommendation-surface.test.tsx`; 관련 설정 UI: `npm --prefix apps/web test -- tests/strategy-catalog-contract.test.tsx`; 접근 경계: `npm --prefix apps/web test -- tests/owner-beta-product-route.test.tsx`.
- Owner-beta surface fixture tests [`apps/web/tests/owner-beta-recommendation-surface.test.tsx`](../../apps/web/tests/owner-beta-recommendation-surface.test.tsx#L250): strict DTO/route, supported-as-of discovery, malformed/unavailable fail-closed, duplicate run query, exact submit route, polling refresh; spec examples 250–488행.
- Strategy form contract [`apps/web/tests/strategy-catalog-contract.test.tsx`](../../apps/web/tests/strategy-catalog-contract.test.tsx#L8): schema defaults and configuration form render; no daily opt-in expectation currently.
- Owner/member browser fixture [`apps/web/tests/e2e/owner-beta-recommendations.spec.ts`](../../apps/web/tests/e2e/owner-beta-recommendations.spec.ts#L14): owner report, Member cannot enumerate, entitlement blocked. Command shape: `npm --prefix apps/web run test:e2e -- tests/e2e/owner-beta-recommendations.spec.ts --project=chromium --workers=1`; browser requires the configured app and synthetic API fixture services per `playwright.config.ts` and `tests/e2e/support/*`.
- Relevant lower-level API helper tests live inline in `crates/api-server/src/http/owner_beta.rs` (e.g. supported-as-of and strict request DTO around 693–841); this D2 inventory did not run Rust tests. No command was executed; commands above are harness entry points, not claimed results.

## Next.js docs and local tooling availability

- `apps/web/CLAUDE.md` is `@AGENTS.md`; `apps/web/AGENTS.md` requires the installed version-specific guide under `apps/web/node_modules/next/dist/docs/` before Web code edits. Potential future guides for UI work are `01-app/02-guides/testing/vitest.md` and `.../playwright.md`; use the installed tree’s actual version and guide paths when implementation begins.
- At this baseline, neither `apps/web/node_modules/next/dist/docs/` nor root `node_modules/next/dist/docs/` exists. `apps/web/package.json` pins Next `16.3.0`, Vitest `4.1.10`, Playwright `1.61.0`; dependencies were not installed and no Next docs were read. The Next.js guide prerequisite therefore remains for the implementation worker.

## Not found / limitations

- No existing strategy-config auto-update flag, daily reservation endpoint/client, daily admission input status, or separate expected-session/support-through/published-at DTO field was found in the bounded strategy and Owner-beta recommendation paths above.
- `crates/api-server/src/http/owner_beta.rs` imports its DTOs from `http/dto.rs`; this is where the wire structs are declared. The current endpoint/DTO evidence is historical-price-only and does not define the adopted daily input contract.
- This is a path-scoped inventory, not a global Web search certification, runtime check, UI design, or test result. Other application behavior and real owner/session operation were not verified.
- Root `CLAUDE.md` is absent in this checkout. The explicitly requested `apps/web/CLAUDE.md` exists and delegates to `apps/web/AGENTS.md`.
