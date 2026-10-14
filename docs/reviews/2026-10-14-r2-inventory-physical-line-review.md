# R2-04/09 WP-4 I0b-F3-R — independent physical-line source review

2026-10-14, Asia/Seoul. Direct codex/gpt-6.1-sol/high reviewer; intermediate complexity,
high confidence. Workspace `/data/worktrees/3puw275b/excited-dolphin`,
`wks_10fae7d550ab1704`, assigned parent `ef6cdbb0-8409-4dba-86ca-c07eadccf377`.
Frozen candidate `1599e09dd6ef5e38006eeccaf4b0273ee5825d2f`; comparison source
`2593fbef358f5cd163e58c514e577793daa4f749`.

**No concrete findings in this bounded delta review. I0b-F2-R01 is resolved at
source-review level.** The real readers preserve decoded newline characters, all
three body loops split only on LF, and meaningful written regressions exercise
the actual source wrappers and reader entrypoints. The prior literal-boundary
finding's remaining physical-line issue is therefore closed at source-review level.
This is not source acceptance, an executed regression result or runtime readiness.

## Instructions, inputs and preserved scope

Read [AGENTS](../../AGENTS.md), current [ROADMAP](../ROADMAP.md), the plan's
[physical-line correction brief](../superpowers/plans/2026-10-14-r2-daily-input-implementation.md#L1675),
[recovery](../superpowers/plans/2026-10-14-r2-daily-input-implementation.md#L1758) and
[wave-15 review brief](../superpowers/plans/2026-10-14-r2-daily-input-implementation.md#L1777).
Applicable provider and direct/no-delegation instructions remain unchanged.
Read private `source-wave-15-inputs.json`, `wave14-I0b-F3-final.md` and
`wave14-I0b-F3-recovery.json`. Idle/pending0/substantive-final recovery is supplied
coordinator evidence; this reviewer made no agent lifecycle query.

| Reviewed input | Lines | SHA-256 |
| --- | ---: | --- |
| [runtime-static-check.sh](../../deploy/secrets/runtime-static-check.sh) | 1023 | `0bede130507a141cd281ded771b0bfb00e77334809f79aabade8eff1c591365f` |
| [Prior literal review](2026-10-14-r2-inventory-literal-review.md) | 197 | `34095bee08fc89b67967fff7b4dd8852aa79f0778b854dc173a00c84dce6c699` |

Target bytes match the frozen candidate blob. The scoped delta is one file,
261 additions and six deletions, entirely inside its embedded Python body. Shell
bytes outside that body match the comparison source. Reviewed all changed hunks
and the complete reader, source-wrapper, helper, regression invocation, production
parity and error paths. Prior reports were read but not edited.

All nineteen reference hashes/counts matched the freeze and comparison-source
blobs. These are preservation checks, not new reviews of their implementations:

| Unchanged reference | Lines | SHA-256 |
| --- | ---: | --- |
| deploy/db/bootstrap-roles.sh | 303 | `66b91261cdefbb0709234c5c3b2d3399ccf96428594c7efd0c74d81910151ddd` |
| scripts/ops/provision-db-secrets.sh | 613 | `2b2650c12fa3914f8258d40e034409dac3ce46cd03a8807646d0a898e4fa6125` |
| deploy/secrets/provision-runtime-secrets.sh | 343 | `f3529be0ff55dc0f1f3866a8cd43aa4a83ffa62da36e95796dd3600a25974f5c` |
| scripts/ops/validate-production-config.sh | 889 | `a472d2f8079985f124d66d7efddda2c11e2955858014b7325f563dd3b3ff9ba2` |
| deploy/compose/compose.yml | 1632 | `434643b6c749e7ceca8abcca1f29726f55bd9ef86ee42c08db7e53ed1d4c7931` |
| deploy/compose/.env.example | 149 | `e37029e2b47ef70360666e070eae591de4dc657186f430daef593bad19a9c330` |
| deploy/db/integration-validation/validate.sh | 1709 | `d2d4008249f0c79d64da7e00281861dfbc7a533e6e5793f585411e7693f6f6b3` |
| scripts/ops/self-test.sh | 1483 | `496702920d258e7252d7da96503adc22deca6609cb1866110c7b0abe67230d17` |
| crates/market-data/src/storage.rs | 3656 | `b1dca720210216d49d22858ac65b26a8f3cf230428629c9c4ec0ac64238e6fef` |
| crates/market-data/tests/daily_price_only_storage.rs | 644 | `1deec0dc3060fbbf880b3801b78cf640b70aae7a1d0f1c99807fe41476000d45` |
| crates/job-queue/tests/common/mod.rs | 401 | `242977cf561d54c88889feeb695c8de5b90a64910fd3fcbfde346a17a06e68fa` |
| crates/api-server/tests/common/mod.rs | 933 | `6735a723f3c874c129ed43685f8633c372a8384ba2d23da407bebb7d18ed7dcb` |
| tests/integration/migration-contract/role-bootstrap.sql | 94 | `9dfe008fdd08b49587940e14fc6f441b2a0053da5f48651fec98fdb53c31b85a` |
| tests/integration/migration-contract/bootstrap.sql | 33 | `24fcb7f582f927d9d94827597c74989e005c3378530a3b9c5677e6ac29bf3c77` |
| crates/market-data/src/daily_price_only/contract.rs | 4183 | `f086c065b903a00a022eeef3b37daf3b021691f0fa2fc525b17be1a7ed4ea523` |
| crates/market-data/tests/daily_price_only.rs | 2385 | `1596ecbe2b673264fdec2f41605e043f3d7fb79ea54463ac5075ad9da3cd0fdd` |
| nt/live-node/tests/test_runtime.py | 479 | `e2121f61d3be269b4b8b15313c6c0e8c4fe8ed34a218bac1c908cf72ae3dfb97` |
| D4 contract | 950 | `fa0529033945437e78eca5b0805f0524a4ae3c75fc4f0a36321ec56b052e717f` |
| D5 amendments | 740 | `8a8796d992da533389bac97fab6abf86c7e89e75f0772f6deceb3d4051d651a6` |

## Finding disposition and complete production path

**I0b-F2-R01: source-review resolved; no remaining corrective action identified in
this delta.** [read_inventory_source:272–275](../../deploy/secrets/runtime-static-check.sh#L272)
uses a context-managed UTF-8 text reader with newline="" and returns the unchanged
decoded contents. The context closes the file on read completion or exception.
The [provisioner entrypoint:296–297](../../deploy/secrets/runtime-static-check.sh#L296)
and [validator entrypoint:334–335](../../deploy/secrets/runtime-static-check.sh#L334)
both call that shared reader and delegate to their production source-text parsers.

The three body loops at
[288](../../deploy/secrets/runtime-static-check.sh#L288),
[310](../../deploy/secrets/runtime-static-check.sh#L310) and
[348](../../deploy/secrets/runtime-static-check.sh#L348) split only on literal LF.
Trimming/field splitting remains ASCII space/tab only. CR/U+2028 inside a physical
comment therefore remains within the same line skipped at its comment marker;
those characters in a non-comment mapping are retained and reach the existing
literal/arity/unknown-body checks. They are not silently removed or converted into
independent mappings. No splitlines call remains in the embedded body.

The exact component/source/numeric/mode/yes-no checks at
[217–262](../../deploy/secrets/runtime-static-check.sh#L217) and exact validator
service/target checks at [359–371](../../deploy/secrets/runtime-static-check.sh#L359)
retain their prior semantics. Source validation still precedes tuple construction.
The source wrappers preserve unique definitions, required selection counts, unknown
body rejection, selected-map duplicate rejection and nonempty map guards at
[278–356](../../deploy/secrets/runtime-static-check.sh#L278).

No shell interpretation, quote stripping, eval/exec/source, external fixture file,
new credential/service or dependency manifest was introduced. The two new imports
are standard-library io and unittest.mock.patch. Errors use fixed source labels,
categories or fixture positions/indices; submitted source tokens are not added to
diagnostics. The production handler at
[794–796](../../deploy/secrets/runtime-static-check.sh#L794) remains unchanged.

Ten existing function ASTs are identical: require_component, split_ascii_fields,
require_source_path, parse_metadata, parse_copy, add_unique, parse_validator_spec,
expect_inventory_error, run_parser_regressions and inventory_parity. The production
try body after the two new regression calls and its handlers are also identical.
Protected bootstrap presence and removal/UID+1/mode-XOR mutations remain at
[775–793](../../deploy/secrets/runtime-static-check.sh#L775).
Actual 33-row parity was neither executed nor recalculated here.

The prior I0b-F-R01 helper grammar closure can now include its physical-line boundary
at source-review level. Original DB3 and Raw5 source closures are preserved without
reopening their implementation reviews; role/log, wire/I0a, Raw, runtime maps and
Compose source are unchanged. All operational and later-package limits remain.

## Meaningful written coverage, not executed test results

| Requirement | Current written checks | Source assessment |
| --- | --- | --- |
| Complete valid LF/space/tab maps | [526–585](../../deploy/secrets/runtime-static-check.sh#L526) | Production wrappers compared with four explicit expected path/UID/GID/octal-mode tuples, including TLS and bootstrap; no copied parser supplies expected results |
| CR/U+2028 comments | [595–606](../../deploy/secrets/runtime-static-check.sh#L595) | Both separators placed in serving function, infrastructure function, release branch and validator array; each must retain the exact base map |
| Phantom parity prevention | [587–601](../../deploy/secrets/runtime-static-check.sh#L587) and [747–750](../../deploy/secrets/runtime-static-check.sh#L747) | Ordinary validator extra row has an explicit extra tuple; comment-only provisioner still must differ, through both pure and actual path entrypoints |
| Non-comment separators | [608–641](../../deploy/secrets/runtime-static-check.sh#L608) | Leading/trailing/joined-row cases plus all seven copy and five validator field positions; both separators across every relevant wrapper location |
| Duplicate/unknown/selection guards | [643–695](../../deploy/secrets/runtime-static-check.sh#L643) | Within-body/cross-selected duplicates, unknown statements, duplicate definitions/branches and repeated helper/source selection use actual wrappers |
| Real reader seam and exact preservation | [699–750](../../deploy/secrets/runtime-static-check.sh#L699) | Real TextIOWrapper over BytesIO, not a mock that forces preservation; exact probe includes CR, CRLF, U+2028 and LF |
| Real path-entrypoint rejection | [751–760](../../deploy/secrets/runtime-static-check.sh#L751) | CR/Unicode non-comment mapping rejection through both path readers |
| Scoped patch and handles | [732–765](../../deploy/secrets/runtime-static-check.sh#L732) | Fixed fake paths, forwarded newline argument, patch context restored, all handles asserted closed before finally cleanup |
| Correct rejection type and ordering | [374–379](../../deploy/secrets/runtime-static-check.sh#L374), [768–776](../../deploy/secrets/runtime-static-check.sh#L768) | Only InventoryError is expected rejection; both regression calls occur after every referenced parser/parity definition |

The explicit positive map is api_server/target-1=(10001,10001,0o440),
reverse-proxy/lagrange.crt=(101,101,0o440), bootstrap admitter=(999,999,0o400),
research-range-raw/kis_app_key=(10001,10001,0o440). Both fixtures exercise the shared
literal helpers through complete wrappers, with different numeric octal spellings
that must yield those exact tuples.

If a body loop is restored to splitlines, the matching location's CR/U+2028 comment
case would create an extra row and contradict its explicit expected map. If reader
newline="" is removed, fake_open's default newline=None is passed to the real
TextIOWrapper; the probe's CR/CRLF characters would normalize and fail exact equality
at 741–742. These are predicted regression sensitivities derived from source, not
executed mutation tests. The fake does not inspect or force a preservation outcome;
it forwards the reader's supplied newline argument.

The patch is limited to the in-memory reader regression context. No fixture uses
disk files, source evaluation or shell execution. Every constructed handle is
tracked, checked for closure before cleanup, and closed in finally even on failure.
Default-bound lambda arguments retain the current case rather than a final loop
value. The existing 17-token helper cases and prior parity mutations are preserved.
All these regressions remain unexecuted by this reviewer.

## Actual checks and remaining evidence

Performed bounded source/instruction/plan/recovery/prior-review reads and scoped
diff/numstat inspection. Exact target/candidate comparison, nineteen reference
hash/count and comparison-blob checks, prior-review hash/count, shell prefix/suffix
comparison and the ten-function/production-body AST comparisons passed.
bash -n deploy/secrets/runtime-static-check.sh returned exit 0. Scoped
git diff --check against 2593fbef returned exit 0. Complete embedded Python
ast.parse passed with 26 top-level nodes. No embedded statement, parser helper,
wrapper, regression, mocked reader or checker was invoked. Final frozen-input
stability passed for the target, nineteen references and prior review (21 files).
All 29 report links/line bounds/fences/whitespace checks passed.

The current exact memory context helper returned MCP_ERROR/no response. No result,
alternative envelope/scope or stored conclusion was inferred. Finish assessed this
unadopted review with no_durable_change using its current hook envelope and returned
SKIPPED, saved=false, incomplete=false. Its
administrative operational receipt is the allowed write exception, not durable
project progress. Basic Memory was not used.

Changed file: this new report only; final exact lines/hash are in the final response.
Deviations from task scope: none. Concrete unresolved source findings: none.
Required inputs missing/not found: none. Missing/conflicting requirements: none.
Further decomposition needed: none. Background work and pending permissions: none.

Follow-up: coordinator owns acceptance, integration and eventual one exact-candidate
hosted verification after checking existing runs. Unverified: compilation/Clippy,
parser/helper/wrapper/reader/regression execution, actual 33-row parity, checker,
validator/provisioner/self-tests/tests, new CI, SQL/DB/actual role/log/GUC/pools,
installed UID/mode/ACL/read-denial behavior, provider/production/runtime/deployment,
and Owner/source acceptance. Other unreported missing evidence: none. Earlier
929fcd6 CI success is baseline evidence only. I0/0060/full snapshot/diagrams before
main/runtime/Owner acceptance remain separate later scopes.

No source, prior/other report, plan, roadmap, ledger, private receipt or Git staging/
commit/push change was made, apart from the current memory hook's permitted receipt.
No package installation, root/sudo, Docker, network/workflow/provider/production,
lifecycle or native/Paseo delegation occurred. This completes the bounded source
review; no execution or source acceptance is claimed.
