# R2-04/09 WP-4 I0b-F2-R — independent literal parser delta review

2026-10-14, Asia/Seoul. Direct codex/gpt-6.1-sol/high reviewer; intermediate complexity,
high confidence. Workspace `/data/worktrees/3puw275b/excited-dolphin`,
`wks_10fae7d550ab1704`, assigned parent `ef6cdbb0-8409-4dba-86ca-c07eadccf377`.
Frozen candidate `2593fbef358f5cd163e58c514e577793daa4f749`; comparison source
`de1889be8a0f3efa9bac2c1ab1a272c9c1609af0`.

**One P2/MEDIUM finding remains in the complete wrapper path.** Shared token helpers
implement the fixed literal grammar, but line splitting/newline normalization can
erase unsupported source characters before those helpers see them. I0b-F-R01 is
partially resolved and remains open at source-review level for this boundary.
No other concrete delta finding was identified. No source acceptance, executed
parser/test/CI result or runtime readiness is conferred by this report.

## Instructions, scope and exact inputs

Read [AGENTS](../../AGENTS.md), current [ROADMAP](../ROADMAP.md), the plan's
[review disposition](../superpowers/plans/2026-10-14-r2-daily-input-implementation.md#L1492),
[I0b-F2 brief](../superpowers/plans/2026-10-14-r2-daily-input-implementation.md#L1521),
[recovery](../superpowers/plans/2026-10-14-r2-daily-input-implementation.md#L1579) and
[wave-13 review brief](../superpowers/plans/2026-10-14-r2-daily-input-implementation.md#L1604).
Applicable provider instructions and the direct/no-delegation boundary are unchanged.
Read private `source-wave-13-inputs.json`, `wave12-I0b-F2-final.md` and
`wave12-I0b-F2-recovery.json`. Their idle/pending0/final recovery is coordinator
evidence, not an independently queried agent or runtime result.

| Reviewed input | Lines | SHA-256 |
| --- | ---: | --- |
| [runtime-static-check.sh](../../deploy/secrets/runtime-static-check.sh) | 768 | `332324fba66d9c9c5a8f8518d470bc1cf9a9df88b004743d8b23c5ee06069e06` |
| [Prior correction review](2026-10-14-r2-daily-role-corrections-review.md) | 229 | `52d7400dc60c14e744178e59bcf2e31c43e5ca9a5bf299bfe8d4c24aa7f3ab03` |

The current target matched its frozen candidate blob. The delta is one source file,
219 additions and 19 deletions, confined to the embedded Python body. Shell bytes
before and after that body match the comparison source exactly. Reviewed all changed
hunks and the complete directive, readers/body loops, parsing helpers, regression
calls, parity construction and error exit paths. No unrelated product review was done.

All nineteen read-only reference hashes/counts matched the private inventory and
their comparison-source blobs. These are preservation checks, not repeated reviews:

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

## I0b-F2-R01 — wrapper normalizes unsupported line separators before validation

**Severity:** P2/MEDIUM. **Affected:** the inert source-map wrapper in this file.
Predicted source defect, not an executed reproduction or current inventory mismatch.

The provisioner reader at [289](../../deploy/secrets/runtime-static-check.sh#L289)
and validator reader at [324](../../deploy/secrets/runtime-static-check.sh#L324)
use text open with default universal-newline translation. The function body,
release branch and validator body loops use splitlines() at
[280](../../deploy/secrets/runtime-static-check.sh#L280),
[299](../../deploy/secrets/runtime-static-check.sh#L299) and
[334](../../deploy/secrets/runtime-static-check.sh#L334). Python splitlines recognizes
separators beyond LF, including U+2028, while ordinary Bash comment/command boundaries
do not treat U+2028 as a newline. Default text reading additionally converts CR to LF.
The new ASCII field split/strip occurs only after these transformations.

**Concrete source-input trigger:** inside add_serving_copies, use this single LF-ended
physical comment line; `<U+2028>` denotes the actual Unicode LINE SEPARATOR character,
not the printable placeholder text:

```text
  # note<U+2028>add_copy extra_service extra_target cursor_secret 10001 10001 0440 yes
```

The body loop splits it into `# note` and a syntactically valid ASCII add_copy record.
It skips the first logical line and constructs an extra mapping from the second.
Bash instead treats the whole physical line as a comment. If the validator release
list contains the ordinary literal row
`extra_service/extra_target:10001:10001:440`, both inferred dictionaries match with
that phantom provisioner row. Existing expected entries, bootstrap count, protected
path and duplicate checks remain intact, so this is not detected by those checks.
The checker can therefore report equal inventories that the actual provisioner
does not select. Without the paired row, it can instead reject genuinely equal
shell inventories because it invented an extra record.

A CR placed within the same physical comment has the analogous problem at the
file-reader layer: universal-newline conversion manufactures a logical newline
even if splitlines is replaced later. The affected operations predate this delta;
the correction nevertheless promises a strict literal boundary through their
complete call path. This is incomplete closure of that requirement, not an assertion
that the patch newly introduced Unicode splitting or that current frozen sources
contain this counterexample. No altered source, parser helper or checker was run.

**Bounded correction:** preserve source newline bytes when decoding, and split
source bodies only at physical LF boundaries. Unsupported separators in mapping
tokens must reach literal validation or fail explicitly; comment text should stay
within its actual physical comment line. Add in-memory wrapper-level cases for
U+2028/CR comment and mapping boundaries using the same production line handling.
Preserve valid LF/space/tab rows, duplicate/unknown-body guards and safe diagnostics.
No general shell parser, new product policy or external fixture is required.

## Disposition, helper semantics and written checks

**I0b-F-R01: partially resolved, source-review closure withheld for I0b-F2-R01.**
The original `${unresolved_service}` token counterexample is rejected by the shared
component grammar before tuple construction. The source field is also validated
despite being omitted from parity. The wrapper boundary above prevents full closure.
The original DB I0b-R01–03 remain source-review closed and were not reopened or edited.

The [shell directive:201–206](../../deploy/secrets/runtime-static-check.sh#L201)
passes only two source-file paths to the embedded Python process. Parser routines
read those files as data; no eval, exec, sourcing, quote stripping or shell expansion
was added. [215–260](../../deploy/secrets/runtime-static-check.sh#L215) applies the
exact ASCII component, decimal UID/GID, three/four-digit octal and yes/no grammar.
Source splitting on slash rejects empty, absolute, repeated-slash, dot and dotdot
components. [345–357](../../deploy/secrets/runtime-static-check.sh#L345) requires
exactly four colon fields and exactly two validated service/target path components.
Both production map builders call these helpers. Errors name field/category or
fixed source label, not submitted tokens; the outer error handler remains at
[539–541](../../deploy/secrets/runtime-static-check.sh#L539).

Unique definitions/branches, one call to each selected inventory function, the
required extra source, unknown-body rejection and duplicate-path rejection are
preserved at [263–342](../../deploy/secrets/runtime-static-check.sh#L263).
The dictionary parity, protected bootstrap presence, removal, UID+1 and mode-XOR
mutations remain at [515–538](../../deploy/secrets/runtime-static-check.sh#L515).
The selected source inventories themselves are unchanged. Actual 33-row parity
was not executed or recalculated in this review.

| Guard | Written regression source | Review limit |
| --- | --- | --- |
| Shell syntax in three copy/two validator positions | [369–406](../../deploy/secrets/runtime-static-check.sh#L369): 17 tokens, 51 copy and 34 validator cases | Actual parse_copy/parse_validator_spec helpers; no regex-copy mock; no execution |
| Relative source paths | [408–422](../../deploy/secrets/runtime-static-check.sh#L408): seven absolute/parent/repeated/trailing/dot cases | Empty source also fails arity/path by source reasoning; explicit empty case is absent |
| Arity and non-ASCII separator | [424–436](../../deploy/secrets/runtime-static-check.sh#L424), validator 458–466 | NBSP is tested directly at helper level; no full body/read normalization case detects R01 |
| Numeric/mode/single_line | [437–480](../../deploy/secrets/runtime-static-check.sh#L437): expansion/non-ASCII decimal, unsupported octal widths/digits and yes/no spelling | Shared metadata helper checks before int conversion; assertions unexecuted |
| Valid ordinary/TLS paths and exact tuples | [482–509](../../deploy/secrets/runtime-static-check.sh#L482): three copy and two validator positives | Includes TLS cert/key, 0440/0400 and 440 numeric modes; no helper execution |
| Expected rejection type | [360–365](../../deploy/secrets/runtime-static-check.sh#L360) | Only InventoryError is caught; unrelated exceptions cannot count as rejection |
| Invocation and prior parity mutations | [512–538](../../deploy/secrets/runtime-static-check.sh#L512) | Written regressions run before real map construction in normal checker; neither phase executed here |

The lambdas capture each constructed input through default arguments, so loop cases
do not accidentally reuse a final token. Rejection labels use positions/indices;
positive errors use indices, without token dumps. The meaningful helper cases do
not exercise parse_copy_function, file reading or body line splitting, which is
why they do not establish the required end-to-end syntax boundary.

## Actual checks, retained limits and handoff

Actual checks: bounded instruction/plan/recovery/prior-report/target source reads;
one-file diff and numstat review; exact target/candidate blob comparison; all nineteen
reference hash/count and comparison-blob checks; prior report hash/count; shell
prefix/suffix comparison outside the embedded body. All passed.
bash -n deploy/secrets/runtime-static-check.sh returned exit 0. Scoped
git diff --check against de1889be returned exit 0. Complete embedded Python
ast.parse passed with 21 top-level nodes; AST inspection confirmed 17 invalid-token
literals. No AST code was compiled for execution, evaluated or invoked. All 26 report
links/line bounds/fences/whitespace checks passed. Final stability passed for the
target, nineteen references and prior review (21 files).

The current exact memory context helper returned MCP_ERROR/no response; no context,
alternative scope or stored conclusion was inferred. Finish assessed this unadopted
review with no_durable_change using its current hook envelope and returned SKIPPED,
saved=false, incomplete=false. The operational
receipt is the authorized write exception; no Basic Memory or speculative durable
progress was written.

Changed file: this new report only; exact final range/hash are in the final response.
Task-scope deviations: none. Required inputs not found: none. Additional missing
or conflicting requirements: none beyond the wrapper/literal boundary mismatch.
Other concrete delta findings: none. Further decomposition needed: none; correction
and wrapper regression coverage fit the existing embedded parser.

Unresolved follow-up: coordinator resolves I0b-F2-R01 and freezes the next candidate,
then arranges independent review and the authorized exact-candidate hosted check.
This review neither queries nor dispatches CI. Compilation/Clippy, parser helpers,
regression loops, actual 33-row parity, checker/validator/provisioner/self-tests,
SQL/DB, actual role/log/GUC/pool behavior, installed UID/mode/ACL access, provider,
production/runtime/deployment and Owner/source acceptance remain unverified.
Other unreported missing evidence: none. First0060, full pipeline and diagrams are
retained later-package work, not acceptance prerequisites invented by this review.
No source, prior/other report, plan, roadmap, ledger, private receipt or Git write
was performed, except the authorized request-scoped memory operational receipt.
No dependency installation, root/sudo, Docker, network/workflow/provider/production,
lifecycle or native/Paseo delegation occurred. Background work and pending permissions:
none. This is a complete bounded source review, not a passing execution report.
