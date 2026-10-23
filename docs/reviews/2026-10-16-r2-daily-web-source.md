# R2 Wave67 daily recommendation Web source review

Roadmap R2-04/09. Baseline7704749 after full I5 CI38818261699 recovery.
Direct coordinator review, not independent review. No compiler, browser or
runtime acceptance yet. I5 API/generated contracts and backend remain unchanged.

The strategy form exposes an unchecked daily preference only to an Owner in
Owner-only mode with recommendation entitlement. Creation sends the explicit
boolean alongside unchanged strategy/config fields. Existing configuration
PATCH sends only owner_beta_daily_enabled through existing CSRF/idempotency/
mutation serialization. No epoch or evidence pin is client-controlled.
Controls commit only an exact matching response identity and boolean. Errors
retain confirmed state; legacy absent flags stay unreported and disabled.

Recommendations select one validated configuration. Duplicate selectors/cursors
or conflicting run/config identities fail closed. Daily GET has a10second abort,
30second cadence, no overlap, hidden-tab downgrade and unmount/identity fencing.
SSR dates remain last observations until a fresh read; failed reads preserve
dates but cannot retain a CURRENT badge. Publication/reservation identity changes
refresh the server report once. All ten state codes and24 finite reasons have
English/Korean labels. Expected session, support end, daily result as-of and
published time remain distinct; due/observed times use existing KST formatting.

Historical discovery failure disables the manual form without erasing daily
state or previous results. Session and entitlement denials still propagate.
History searches at most six100-row pages for a same-config successful result,
rejecting duplicate runs, inconsistent pages and repeated cursors. A bounded
incomplete window says so and offers its next cursor. Manual history never
becomes daily CURRENT. Reports display daily four commitments plus nested base
five, or unchanged legacy historical pins; creation is not publication time.

Nine new files and17 existing files form the26-path allowlist. No dependency,
backend, migration, provider, production, workflow or permission change.
Synthetic fixture changes serve this test contract only and are not real-data
or Owner acceptance. Installed Next16.3.0 Vitest/Playwright and relevant
server/client guidance was read before edits.

Written verification:18 new Vitest cases cover contract corruption, identity,
CSRF/idempotency/auth, cancellation, bounded history and initial-render evidence.
Eight new real-browser cases cover defaultOFF creation, persisted toggles,
denial, missing historical discovery, stale/blocked/unavailable retention,
paged prior success, late foreign-config responses and session expiry.
Original three Owner browser names remain byte-identical. Existing ambiguous
query and historical-discovery tests retain their original checks and extend
the new contract. Browser inventory changes only3->11 in that file and121->129
in the OFF lane;14 stream and all noSKIP/retry/uniqueness/cleanup guards remain.
The four matching supervisor count assertions change only their expected totals.

Static source/diff/name/line/whitespace checks do not execute these tests.
No local compiler, TypeScript, Biome, project helper, fixture, parser, browser,
SQL, Docker or provider execution occurred. All new cases are WRITTEN_UNEXECUTED.
One exact-candidate hosted CI is required after freeze and existing-run/remote
checks. I6 installer/diagrams, independent integrated review, real approved-base
three-UID execution, G4, Owner, three trading days and C_R2 remain open.

| Owned file | Lines | SHA256 |
| --- | ---: | --- |
| `apps/web/app/(authenticated)/recommendations/page.tsx` | 438 | `03b20054ad69379b50373479632af07cb4ceb0e1de6e0e542b5fd76de3031e21` |
| `apps/web/app/(authenticated)/strategies/page.tsx` | 98 | `06c16827a07ae95e52574e44c402ed050a0299444fbf96febd797405e7dc7e11` |
| `apps/web/components/recommendations/owner-beta-history.tsx` | 72 | `b9f38068382e34ae0a38683f6605cc560f87c6600511c1ab6fada2ffef4e4a8e` |
| `apps/web/components/recommendations/owner-beta-report.tsx` | 374 | `4a45333997619c5bb5481ea5f233001323d0ce3f1a56d3fa58ba65481d9412d9` |
| `apps/web/components/recommendations/owner-beta-daily-panel.tsx` | 171 | `d08884ec47a68e7e17ee1b97b177aa1707e0650fbd5f8f8749ee50be48a26acf` |
| `apps/web/components/strategies/strategy-catalog.tsx` | 70 | `6065c257babe96f8cd681415a7a69761bc84343de8d8948d3b49123a3719b0b2` |
| `apps/web/components/strategies/strategy-config-form.tsx` | 252 | `e6d767af5db6dfd78a958f9e633a861ac229915d29d0ed1a024f8355f3788e74` |
| `apps/web/components/strategies/strategy-daily-control.tsx` | 83 | `4365903bc097504541734b6d758ff0541f79e4e18978084fbea83fb623a1e036` |
| `apps/web/lib/api/product-client.ts` | 370 | `46c89ecc1a7e2aef70230537ec18dbf5be3c7bbd4c75a765209a220f015b7f35` |
| `apps/web/lib/api/contracts.ts` | 130 | `1b1f0f1512193152ec77227b9460d40502666a2b21fd6fc1ff1d7cd312c501bd` |
| `apps/web/lib/api/owner-beta-daily.ts` | 40 | `198f50c1698a130812eb77832743230e5a3eae8fdcee8cfba14b5534b331d077` |
| `apps/web/lib/products/owner-beta-daily-contracts.ts` | 110 | `6f14af771df923e51540a20a98a806e8844cbb4ff2a4ec4316e8adc4270b2142` |
| `apps/web/lib/products/owner-beta-history.ts` | 89 | `6175a283d6469d0fb27e4ca09a45393d3bab5173a64d867ee1b26f333648c9df` |
| `apps/web/lib/i18n/dictionaries/recommendations.ts` | 604 | `b8cc93d3b40dc9fa7019e641bf7d89c21c3d704fe42558f4218c51d95be7c581` |
| `apps/web/lib/i18n/dictionaries/strategies.ts` | 145 | `fd29b295aa61b4ec6f95b259ad27c0233dc417160c20bfe2da685460df939c2d` |
| `apps/web/tests/r2-daily-ui-contracts.test.ts` | 122 | `7cda5217e08545b735a75dc12f5fe4707d55a4df3e506ff7c1f4de30dd19a123` |
| `apps/web/tests/r2-daily-ui-browser.test.ts` | 128 | `30505ca7919af769afc5442ec8a478d7232a842f72aa52ddb9d13840bbd294ea` |
| `apps/web/tests/r2-daily-ui-history.test.ts` | 141 | `e2b821c3ff80e623669b9f205a1d3daf8b4d1b9296acd439fcd33cc824f02489` |
| `apps/web/tests/r2-daily-ui-surface.test.tsx` | 94 | `a27a2fb608a56ace2e8c7de35a8fc2cf8478e37deb2bd6e35f2744b56348944e` |
| `apps/web/tests/strategy-catalog-contract.test.tsx` | 63 | `82a2e8e4a51b908162bf0efd3a13a8c7eaf88c5c83024464b3e5e3898e12d6db` |
| `apps/web/tests/owner-beta-recommendation-surface.test.tsx` | 490 | `2be288f4500f5ef26061cc5f6f721e78c1c96a2e77c1e75b7f047cca0a1b1142` |
| `apps/web/tests/e2e/owner-beta-recommendations.spec.ts` | 329 | `8c457707622d00e23f08060b5f0be8689678a1dc3a8a3e13ae7868641fe03a39` |
| `apps/web/tests/e2e/support/recommendation-fixture.mjs` | 547 | `b509e757efb2400b5aeb1807f3d77e6f4b2c5d04d8ee3b0cf35742079b67a659` |
| `apps/web/tests/e2e/support/synthetic-api.mjs` | 324 | `f18605e2f20ec3da09259a07d0b4b89c702670d1c16f4f84541b0e8da9b113b0` |
| `scripts/qa/candidate-web-e2e-runner.py` | 2215 | `b06e7eea143583685a73c3535e6c3d7ac62484a1dd1568d02fb382e0767aab96` |
| `scripts/qa/test_candidate_web_e2e_runner.py` | 1820 | `0d427afefee95b53dc854d56b8128da947b8a6c3ccd6211caf39b22adaba5a35` |
