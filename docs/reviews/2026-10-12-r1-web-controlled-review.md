# R1 WP-5 — CASE4 controlled lifecycle and CASE6 integration review

2026-10-12 · **SOURCE_REVIEW_COMPLETE — CONTROL1 blocks preparation readiness.**
HEAD `ac0a08364788e6c2f234cf85025a8d975f362d31`, shared dirty candidate.
Independent source review only; no execution or overall Web/R1 acceptance.

## Findings, ordered by severity

**CONTROL1 / P1 — missing schema import prevents the new fixture regressions from running.**
`apps/web/tests/stock-beta-fixture.test.ts:6–9` imports membership, status and
detail schemas, but omits `ownerEquityV2MembershipListSchema`. New calls at
203, 228, 339, 348 and 370 reference that undeclared identifier. It is exported
by `apps/web/lib/products/equity-signals-contracts.ts:543`, but has no import or
declaration in this test module. Type checking should reject these references;
execution of the affected cases would encounter an unbound identifier.
This is source-backed, not a claimed observed typecheck/test failure.

Minimal correction: add `ownerEquityV2MembershipListSchema` to the existing
contracts import. Keep all assertions. In the authorized heavy slot, verify
the fixture suite (including all four additions) and Web typecheck; this review
ran neither. No other actionable defect or optional correction identified.

## Snapshot and coverage

All three current source hashes matched the supplied frozen values at review
start and end. No source drift occurred.

| Source | SHA256 |
| --- | --- |
| `apps/web/tests/e2e/support/stock-beta-fixture.mjs` | `0aee2c8fd497ae1e041ce65b65a6114e2e5197b07c9d98e3c5e1a11f182a6822` |
| `apps/web/tests/stock-beta-fixture.test.ts` | `5697fbd3b0029117416f9b49e385783f138e89ab9d2bcc36bc8bf746f42979c1` |
| `apps/web/tests/e2e/stock-beta-intraday.spec.ts` | `e188a1d3ed23ebeaac8e2599a914a7b15c3f3156586f0b2b02f0232634f3616a` |

`C = target/r1-runtime-fxi6ltmv/web-controlled-lifecycle-20261012/`;
`P = target/r1-runtime-fxi6ltmv/web-timer-probe/`.
Independently reconciled all three retained preimages to `C/hashes-before.sha256`
and the worker's report: fixture `21d08554…560e4`, units `27f3cbf3…8a245`,
spec `f68a8acb…540fe`. CASE4 postimage `P/integrated-preimage.ts` independently
matched `8eeb373cdddbec4e0b1bce1086cc522f83257a274409dba82afd7799127776c4`.
`C/hashes-after.sha256` records that historical pre-probe spec; it correctly
differs from today's integrated spec. `P/integration-hashes.json` matches today.

Read all seven fixture changed hunks and four unit changed hunks, plus all three
CASE4 diff hunks and three probe integration hunks. Relevant current source reads:
fixture1–285/410–560; units1–90/183–375; spec695–780/1398–1625/1620–1775/
2140–2250. Read the complete controlled-lifecycle implementation report and
integration diff; reused the previously read full diagnostic with fresh72–143 reads.
Required changed hunks unread: **none**. Unrelated unchanged code was not broadly rereviewed.

Supporting contracts read: synthetic-api128–178/214–228; workspace250–272/
362–436; membership-status-widget36–68/133–148; equity-signals-contracts445–473/
524–571. These confirm reset/dispatcher reachability, real polling/prop updates,
DOM order/attributes and strict production membership/list/status shapes.

## Preserved CASE4 contracts

Fixture114–115 suppresses only read-driven advancement in controlled mode.
Reset150–174 initializes that mode from the exact opt-in value in every seed
branch; default behavior remains unchanged. Dispatcher473–476 exposes the
synthetic POST only in that mode and requires owner before body processing.
Validation199–234 accepts exactly three own string keys, recognized lifecycle
and safe nonnegative integer generation; malformed422, nonowner403, unknown404,
stale/out-of-order/terminal409 paths precede mutation. The next state comes from
the server map, and235–246 replace only the selected ID while retaining its
instrument/requested timestamp, existing coverage and pending0/READY1 semantics
for the two fresh CASE4 admissions. No production/provider route changed.

The four unit additions assert repeated list/status stability, all four exact
state/coverage/generation transitions, disabled-old versus fresh replacement
identity, malformed/nonowner/unknown/stale/terminal no-mutation behavior, and
reset/default read-driven advancement. CONTROL1 prevents treating them as usable
verification yet. Their assertions are otherwise meaningful and remain intact.

Spec1413–1419 is the only E2E controlled-mode opt-in. Both actual UI POST202
receipts retain REQUESTED/gen0/nonduplicate assertions, and replacement UUID
inequality remains1688/1696. Each server advance1511–1531 validates exact identity,
instrument, requested timestamp, held lifecycle/generation and coverage.
1536–1575 consumes real typed browser GET responses, discards only earlier
admission states after identity/instrument checks, rejects foreign/future/terminal
states, and binds list order to exact DOM instrument/generation/lifecycle.
All four DOM phases and the POST-plus-DOM five-state sequence remain asserted.
Listener removal remains in finally1606–1608; no extra GET drives the lifecycle.

The helper advances the paused clock while waiting for matching responses;
once it finds one, DOM assertions await without additional clock advancement.
Source cannot prove that React will commit during those awaits, or that an old
in-flight RSC payload cannot temporarily overwrite the DOM. These remain focused
browser scheduling checks, not observed defects. Exact held-state assertions
fail on stale DOM; they do not accept a later lifecycle as equivalent coverage.
Existing disable, stale POST/GET404, new lease, EventSource snapshot, invalid old
identity/generation, reconnect/recovery and cleanup tail are unchanged by the diff.

## CASE6 integration equivalence

The helper is immediately before `advanceReconnectBoundary` at703–765; only
CASE6 calls it. Added captures follow the original before-return/response-complete
attachments at2149–2150/2182–2183. Boundary2186–2229 follows the original two-frame
attachment and precedes unchanged board1/third-lease checks2230–2233.
The original Link, RSC response-finished wait and remaining tail are preserved.

Stdlib token comparison of the integrated helper/boundary against the accepted
private snippets passed, preserving strings/operators while normalizing whitespace,
comments and trailing commas. Removing exactly that helper, two discarded-result
calls and boundary restores the entire pre-probe spec token stream.
The pre-probe spec is byte-identical to its initial preimage outside CASE4;
all12 titles remain identical. No unrelated testcase or old assertion was removed.

Private driver/helper/boundary hashes remain `84e8c85c…68fe`, `49765f06…1024`,
`e95330b0…4435`; official runner remains `c8c7e326…afb4`, all independently checked.
Reuse the accepted private-probe review for clock semantics and supervisor safety.
The same64-entry metadata/whitelist, one observed deadline, bounded two advances,
no raw secrets and fail-closed missing/ambiguous/continued-blocker behavior remain.
The React cause is still unconfirmed. Remove the private API probe before commit.

## Checks, remaining work and ownership

Performed Git HEAD read, `rg`, numbered/ranged reads, preimage/current diffs,
stdlib SHA256/JSON parsing and source token/boundary/title comparisons only.
Expected diff exit1 denotes source differences. One combined read was truncated;
bounded subsequent reads recovered all required CASE4/unit/fixture hunks.
Worker formatter/syntax claims are historical static evidence, not unit/typecheck
or browser success, and do not resolve CONTROL1.

Missing required inputs: **none**. Review deviations: **none**. Further decomposition:
**none**. Unverified: compilation/typecheck, four unit regressions, actual controlled
poll/RSC/DOM timing, CASE6 timer presence/consumption, focused browser bodies/cleanup,
full Web and R1 acceptance. Focused diagnostic readiness is **blocked by CONTROL1**
and pending the separately owned unit/typecheck results; integrated probe source
otherwise conforms to the accepted preparation.

Owned writes: this new report and `P/wp5-integrated-source-review.json`, SHA
`871d73d82251d516066178c8f33ad8557dd81bca2765fa7142ecbb2e5d2578cc`.
No source or previous report edited, no tests/modules executed, no runtime
resources/processes created. Actual idle at completion.
