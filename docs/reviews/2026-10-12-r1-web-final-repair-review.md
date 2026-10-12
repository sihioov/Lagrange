# R1 WP-5 — final CASE4/CASE6 source repair review

2026-10-12 · **SOURCE_REVIEW_COMPLETE — no new actionable findings; CONTROL1 resolved.**
HEAD `ac0a08364788e6c2f234cf85025a8d975f362d31`, shared dirty candidate.
Source acceptable for separately authorized focused validation, pending current
typecheck results. No dynamic browser, full Web or overall R1 acceptance.

## Findings and prior finding resolution

New blocking/optional findings: **none** in the bounded patch.
Prior **CONTROL1/P1 is resolved** at `apps/web/tests/stock-beta-fixture.test.ts:7`:
`ownerEquityV2MembershipListSchema` is now in the existing contracts import.
Removing exactly that one line reconstructs the previously reviewed file's
SHA256 `5697fbd3…79c1`; all five schema uses and other assertions are unchanged.
Historical finding evidence in the controlled review remains untouched.

Independently matched the retained `01-vitest.log` SHA256
`0462fccccafa8234d715ded234aee082ab35b935ab995c6ce08cf460f409492b` and exit record0:
the official command ran the fixture and intraday-catalog suites, respectively
8+4 tests, two files, 12 passed/0 skipped, 685ms. This is coordinator/worker-provided
dynamic evidence, not reviewer execution. The historical typecheck failure in
web-validation201–211 concerned the removed private helper's dot access; it does
not establish the current patch's typecheck outcome. Current typecheck logs were
not read as completed evidence.

## Frozen snapshot and exact diff coverage

All three supplied current hashes matched at start and end:

| Source | SHA256 |
| --- | --- |
| `apps/web/tests/e2e/stock-beta-intraday.spec.ts` | `b168826afcfd40f01b6e6fdd94e3a421e0a62b3f175547ec495fb3316fb58de4` |
| `apps/web/tests/stock-beta-fixture.test.ts` | `ea3e9ddbdc6517c5458fffdf33d06c096c92c3a05b1ece579f883fc99cad3d46` |
| `apps/web/tests/e2e/support/stock-beta-fixture.mjs` | `0aee2c8fd497ae1e041ce65b65a6114e2e5197b07c9d98e3c5e1a11f182a6822` |

`D = target/r1-runtime-fxi6ltmv/web-final-source-20261012/`.
Preimage `D/preimage.ts` independently matched
`952eacf484595f2120fb4132e4e7ea3f3a3b02cd9c6443660e25095d939bc906`.
It reconstructs prior accepted `e188a1d3…3616a` when only the two bracket accesses
in the now-deleted private timer helper are restored to dot access.
All seven hunks in `D/implementation.diff` were read and independently regenerated
byte-for-byte from preimage/current source. Diff SHA256:
`8e15f8bf3d621a20ef197cc4ef598e4575d58f52567ec40af8fdf112e94602e6`.

Exact new/supporting spec reads:1390–1468/1702–1800/2092–2168/2200–2275;
unchanged helpers185–229/424–486/560–645/665–673/831–843, plus the full retained diff.
All12 titles remain identical; the other10 complete testcase bodies are
byte-identical to the preimage. CASE4 lifecycle and both CASE4/CASE6 later tails
have no changed diff hunks. All private clock/timer/callback-probe identifiers
are absent from the final spec. No skip/only/retry or dependency change was added.

Official runner remains `c8c7e326…afb4`, private driver `84e8c85c…68fe`, and
market-stream fixture `ecf788b3…826fa`; independently matched their accepted pins.
Retrospectively hashed all30 safe inputs in the completed unit-run manifest:
29 remain identical, and only the authorized intraday spec differs. Manifest SHA
`aba72cbee5ba4501f0cb5f04eaa790520aaa97b12ac5e596d17f81b92934b612` matched.
This corroborates unchanged covered production/fixture/runner inputs; it is not
a fabricated contemporaneous capture of the final patch or a new whole-branch audit.

## Contract A — fresh invalid events and preserved rejection/recovery

Spec1400–1434 reacquires the exact captured connection ID from the current ledger,
requires it still open and retains the captured lease/stream. It requires prior
maximum1 or2, candidate2 or3 and candidate strictly newer than every sent sequence.
The finite attachment carries phase/sequences/booleans, without UUIDs or bodies.
Read fixture245–279/700–738/774–790: ledger entries are copied; `emitInvalid`
records the specified event sequence and rejects closed/unknown owned connections.
This confirms why rereading the ledger fixes the stale-copy construction defect.

Spec1711–1722/1750–1761 uses the fresh connection consistently for target ID,
serialized envelope and event ID. The body sequence and event-ID suffix share
the same checked string. Old disabled membership/price9 and replacement+1
generation/price8 rows are unchanged. No extra frame or clock advancement precedes
the existing immediate quote/time purge assertions1723/1762.
New1436–1455 additionally requires that exact source closed, with the5000ms budget,
unchanged creates2/releases1/renewals0 and ledger/open counts2 then3 before retry.
Existing1000/2000ms bounds, reset/full snapshots, recoveries, final exact counters
and owned cleanup remain unchanged. No production duplicate/identity behavior weakened.

## Contract B — public, conditional and finite readiness

CASE6-local helper2105–2162 atomically reads real board count and exact Loading
heading count in one evaluation. The markup matches authenticated/loading.tsx1–9,
state-panel.tsx27–45 (data-kind loading/direct h2), shell.ts67 English title and
intraday-stream-board.tsx49 board marker. Already-ready state performs zero steps.
Otherwise at most16 steps of16ms plus12ms add268ms; unchanged two frame pumps add32ms,
making total300ms. Each public runFor uses the existing synchronized context/fixture
helper; no resume/private timer API, increased test timeout or open-ended loop.
The entire readiness step has a5000ms wall limit and asserts exact readiness at
exhaustion. Entry/ready/exhausted/failure attachments contain finite metadata/counters.

Spec2205–2245 retains the real Link, exact local RSC predicate, successful response
completion, URL, ordinary receipts and original two16ms frames. The helper sits
only after those frames, before explicit Loading0/board1 and exact third creates/
opens, active lease/source and fresh snapshot. Offline/online, retained-price,
1/2/4-second retry, terminal/logout and cleanup tails remain byte-preserved.
The observed timer did not imply the board was blocked; this implementation no
longer asserts that premise. The300ms budget remains a candidate runtime contract.

## Methods, remaining gates and ownership

Read both adopted contract and native diagnostic reports in full; read the retained
unit evidence and web-validation201–211. Methods: Git HEAD, bounded source reads,
rg, stdlib SHA256/JSON/diff/boundary comparisons only. One supporting read truncated;
bounded subsequent reads recovered decisive helper/metrics spans. No project code
imported/executed and no tests/build/runtime/resource/network probes performed.

Missing required inputs: **none**. Unread required hunks: **none**. Deviations: **none**.
Further decomposition: **none**. Not verified: current typecheck, actual fresh-frame
delivery/purge/close/recovery, zero-step/slow-navigation readiness, all focused tails,
owner-empty3/3 outcome, complete135 Web gate and overall R1/hosted acceptance.
Do not enlarge the readiness budget if focused validation fails.

Owned writes: this new report and `D/wp5-source-review.json`, SHA256
`6a9434373b88c1cd3a00df832613ea7e435201960d70ecfd1293c035e4c6ab26`.
No product/source or previous report edited; no runtime resources/processes created.
Actual idle at completion.
