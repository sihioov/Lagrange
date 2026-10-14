# R2-04/09 WP-4 I0b-F-R — independent correction delta review

2026-10-14, Asia/Seoul. Direct reviewer, codex/gpt-6.1-sol/high under the repository
review rule; intermediate complexity, high confidence. Workspace
`/data/worktrees/3puw275b/excited-dolphin`. Reviewed the three-file delta against
`f1dfb91cdc0cad7d5ae96a310ff462f40cb738a3`, with hash-pinned current inputs rather
than relying on earlier source versions. Coordinator integration commit supplied
by the assignment is `de1889be8a0f3efa9bac2c1ab1a272c9c1609af0`.

**One new P2/MEDIUM finding:** the inert inventory parser accepts shell expansions
as literal mapping fields, contrary to the fixed unknown-syntax rejection rule.
The original R01/R02/R03 defects are corrected at source level, with this new parser
gap retained as follow-up. No other concrete delta finding was identified.
This report does not confer source acceptance, execution proof or full R2 readiness.

## Scope, instructions and frozen inputs

Read [AGENTS](../../AGENTS.md), applicable Codex provider instructions in
`/home/l1nnx/.codex/AGENTS.md`, [ROADMAP](../ROADMAP.md), the
[wave-10 correction brief](../superpowers/plans/2026-10-14-r2-daily-input-implementation.md#L1254)
and [wave-11 paired-review brief](../superpowers/plans/2026-10-14-r2-daily-input-implementation.md#L1406).
No additional applicable ancestor/subdirectory AGENTS or CLAUDE instructions were
found for these deployment/operations paths; web-only instructions do not apply.
Read the private `source-wave-11-inputs.json`, `wave10-I0b-F-final.md` and
`wave10-I0b-F-recovery.json`. Recovery receipt records implementer idle/pending0,
substantive final recovered and runtimeUpdatedAt 06:34:39.324 UTC. It is coordinator
recovery evidence, not an independent runtime check performed by this reviewer.

The [prior role review](2026-10-14-r2-daily-role-wiring-review.md) remained exactly
261 lines, SHA-256 `f83131c8cdbbf6211fe2b6f1f25426a73a4ecdcc6586eacedb6be29ba654db81`.
It was read but not edited. Raw corrections and their other reviewer are out of scope.

| Reviewed current source | Lines | SHA-256 |
| --- | ---: | --- |
| [Runtime static checker](../../deploy/secrets/runtime-static-check.sh) | 568 | `24ff74cce03ec8e9f899c2a14b6ae78f09f79a5b5e9e470a59417ddf3703dccd` |
| [Production validator](../../scripts/ops/validate-production-config.sh) | 889 | `a472d2f8079985f124d66d7efddda2c11e2955858014b7325f563dd3b3ff9ba2` |
| [Isolated integration validator](../../deploy/db/integration-validation/validate.sh) | 1709 | `d2d4008249f0c79d64da7e00281861dfbc7a533e6e5793f585411e7693f6f6b3` |

Reviewed every changed hunk and the relevant current parser, gate, self-test,
bootstrap caller, record/sanitizer and failure paths. The scoped diff is three files,
426 additions and 18 deletions. Fifteen other inputs matched the freeze and their
pinned-base blobs; the sibling Raw pair was not inspected or counted in these checks:

| Unchanged reference | Lines | SHA-256 |
| --- | ---: | --- |
| deploy/db/bootstrap-roles.sh | 303 | `66b91261cdefbb0709234c5c3b2d3399ccf96428594c7efd0c74d81910151ddd` |
| scripts/ops/provision-db-secrets.sh | 613 | `2b2650c12fa3914f8258d40e034409dac3ce46cd03a8807646d0a898e4fa6125` |
| deploy/secrets/provision-runtime-secrets.sh | 343 | `f3529be0ff55dc0f1f3866a8cd43aa4a83ffa62da36e95796dd3600a25974f5c` |
| deploy/compose/compose.yml | 1632 | `434643b6c749e7ceca8abcca1f29726f55bd9ef86ee42c08db7e53ed1d4c7931` |
| deploy/compose/.env.example | 149 | `e37029e2b47ef70360666e070eae591de4dc657186f430daef593bad19a9c330` |
| scripts/ops/self-test.sh | 1483 | `496702920d258e7252d7da96503adc22deca6609cb1866110c7b0abe67230d17` |
| crates/job-queue/tests/common/mod.rs | 401 | `242977cf561d54c88889feeb695c8de5b90a64910fd3fcbfde346a17a06e68fa` |
| crates/api-server/tests/common/mod.rs | 933 | `6735a723f3c874c129ed43685f8633c372a8384ba2d23da407bebb7d18ed7dcb` |
| tests/integration/migration-contract/role-bootstrap.sql | 94 | `9dfe008fdd08b49587940e14fc6f441b2a0053da5f48651fec98fdb53c31b85a` |
| tests/integration/migration-contract/bootstrap.sql | 33 | `24fcb7f582f927d9d94827597c74989e005c3378530a3b9c5677e6ac29bf3c77` |
| crates/market-data/src/daily_price_only/contract.rs | 4183 | `f086c065b903a00a022eeef3b37daf3b021691f0fa2fc525b17be1a7ed4ea523` |
| crates/market-data/tests/daily_price_only.rs | 2385 | `1596ecbe2b673264fdec2f41605e043f3d7fb79ea54463ac5075ad9da3cd0fdd` |
| nt/live-node/tests/test_runtime.py | 479 | `e2121f61d3be269b4b8b15313c6c0e8c4fe8ed34a218bac1c908cf72ae3dfb97` |
| D4 contract | 950 | `fa0529033945437e78eca5b0805f0524a4ae3c75fc4f0a36321ec56b052e717f` |
| D5 amendments | 740 | `8a8796d992da533389bac97fab6abf86c7e89e75f0772f6deceb3d4051d651a6` |

## I0b-F-R01 — inert parser accepts shell expressions as literal map fields

**Severity:** P2/MEDIUM. **Affected:** runtime-static-check.sh's new release parser.
Concrete source validation gap; current literal inventories still match.

The fixed brief requires unknown/ambiguous source syntax to fail explicitly.
However, [parse_copy:215–225](../../deploy/secrets/runtime-static-check.sh#L215)
accepts arbitrary non-whitespace service/target/source tokens using `\S+`. Its
additional nonempty checks cannot reject shell expansion or quoting syntax. The
validator path pattern at [299–309](../../deploy/secrets/runtime-static-check.sh#L299)
also accepts dollar signs, braces, quotes and other shell metacharacters. Both
parsers treat those bytes as literal identity rather than rejecting unrecognized
shell semantics. The source argument is accepted and then discarded from the map.

**Reachable source-input counterexample:** append the following mapping to the
selected serving function and the corresponding release runtime_specs list,
leaving all existing literal entries intact:

```text
add_copy ${unresolved_service} new_target cursor_secret 10001 10001 0440 yes
${unresolved_service}/new_target:10001:10001:440
```

Both patterns match and produce the same literal path and numeric metadata, so
[inventory_parity:315–323](../../deploy/secrets/runtime-static-check.sh#L315) accepts
them. The existing expected-literal checks still find their original entries;
bootstrap counts and the protected-path mutation checks are unaffected. Actual
shell execution would instead expand the variable, or abort under set -u if it
is unset. The inert checker therefore cannot establish a deterministic selected
map for this input, yet does not fail for the unknown syntax. This is a source
checker interface counterexample, not a claim about current production settings
or an executed modified checker. Current source has 33 literal rows and no such
expansion; only bounded in-memory regex/tuple reasoning was performed.

**Bounded correction:** constrain all parsed service/target/source/path fields to
the supported unquoted literal grammar of this inventory; explicitly reject
expansions, command substitution, quoting/escaping, wildcard and shell operator
syntax before constructing records. Keep source text inert and numeric mode
comparison unchanged. Add same-parser rejection cases for dynamic tokens and
ambiguous syntax, in addition to the existing dictionary mutation cases. This
fits runtime-static-check.sh alone and requires no new service or product policy.

## Disposition of the three prior findings

**I0b-R01: corrected at source-review level.** The eighth credential appears in
both [crypto:490–498](../../deploy/secrets/runtime-static-check.sh#L490) and
[numeric:526–532](../../deploy/secrets/runtime-static-check.sh#L526) fixture loops.
The deliberately malformed cursor, exact intended diagnostic, no runtime-tree
creation and no value-output assertions remain at
[488–515](../../deploy/secrets/runtime-static-check.sh#L488). The added positive
metadata assertions at [555–557](../../deploy/secrets/runtime-static-check.sh#L555)
check admitter file 10003:10003/0440, directory 10003:10003/0750 and bootstrap file
999:999/0400. Fake-id/fakeroot mechanics, repeated provisioning and availability
behavior are unchanged. These are written checks; no fakeroot or provisioner ran.

**I0b-R02: original omitted projection corrected at source-review level.** Release
now includes the bootstrap entry at
[validate-production-config.sh:842](../../scripts/ops/validate-production-config.sh#L842).
Independent inert source calculations found exactly 33 selected provisioner rows
and 33 release validator rows, identical by path/UID/GID/octally interpreted mode.
The protected tuple is (999,999,0o400). Other profile/source lists are unchanged.
The new parser reads only source files as data, requires unique function/branch
definitions, rejects unrecognized body statements and duplicate paths, and checks
each selected helper call once. Its shell-token rejection remains incomplete as
I0b-F-R01 specifies. The same parity function is used for written removal, UID+1
and mode-XOR variants at [330–338](../../deploy/secrets/runtime-static-check.sh#L330);
these exercise dictionary equality, not parser rejection of dynamic syntax.

**I0b-R03: original failure-path bypass corrected at source-review level.** All
three callers now capture status in a positive if/else and call the wrapper before
recording bootstrap evidence:
[full:1286–1292](../../deploy/db/integration-validation/validate.sh#L1286),
[staged:1461–1466](../../deploy/db/integration-validation/validate.sh#L1461), and
[test:1686–1692](../../deploy/db/integration-validation/validate.sh#L1686).
The unchanged [bootstrap function:842–858](../../deploy/db/integration-validation/validate.sh#L842)
returns its final Compose command's status. No negated-if status loss or direct
raw-bootstrap record_log call remains at these call sites.

The shared [gate:128–188](../../deploy/db/integration-validation/validate.sh#L128)
checks numeric status, regular/non-symlink/readable/nonempty raw and credential
files and zero LF count. Eight values are scanned with grep -Fq -f filenames;
credential contents never become argv/environment values or diagnostic prose.
Matching disclosure yields 20; grep status other than 1 after nonmatch yields 21.
Input/copy/metadata errors also yield 21. Only a clean scan copies the raw bytes;
then original zero/nonzero bootstrap status produces 0/10. Scanning can stop at
the first disclosure because that outcome already rejects evidence.

[record_bootstrap_result:1136–1172](../../deploy/db/integration-validation/validate.sh#L1136)
records gated clean output through the unchanged
[sanitizer/record_log:557–573](../../deploy/db/integration-validation/validate.sh#L557).
Category 10 records clean sanitized failure output and calls fail. Categories
20/21 record only verified fixed-marker output when available and call fail;
unknown gate categories call fail without recording raw output. Original numeric
bootstrap status is retained in safe detail. [fail:695–699](../../deploy/db/integration-validation/validate.sh#L695)
sets DENIED and exits 1; no bootstrap failure is promoted to success or external
BLOCKED. If the private marker destination cannot be written, the gate rejects
and the wrapper records no unchecked output. No actual credential leak or SQL
error output was observed or claimed.

## Written coverage and execution limits

| Check | Written source coverage | Limit |
| --- | --- | --- |
| Eighth fixture source / metadata | Runtime checker 490–498, 526–557 | Rootless/fakeroot and installed permissions unexecuted |
| Release parity / three variants | Runtime checker 201–342; production list 842 | Current 33 literal rows verified inertly; source functions and mutation suite not executed; dynamic grammar defect remains |
| Shared gate disclosure | [Self-test:337–406](../../deploy/db/integration-validation/validate.sh#L337): eight sentinels x URL/env/bare SQL forms x statuses 0/7 = 48 cases | Calls actual bootstrap_record_gate, not a copied mock; no shell test executed |
| Clean success/failure | [408–419](../../deploy/db/integration-validation/validate.sh#L408): returns 0/10 and copies clean output | Does not execute record_bootstrap_result, sanitizer, fail or three real callers |
| Scan inputs | [421–464](../../deploy/db/integration-validation/validate.sh#L421): missing, directory-valued and empty credential/raw input, six cases | Directory simulations prove intended nonregular rejection only; no permission-denied regular file or injected grep I/O-error case |
| No sentinel in diagnostics/evidence | [356–381](../../deploy/db/integration-validation/validate.sh#L356): empty diagnostics, exact marker or clean raw equality, fixed-string scans | Missing/empty/nonregular credentials are skipped in the helper's secondary sentinel scan; full sentinel matrix uses all eight intact files |

The gate definitions precede the existing --self-test branch; the self-test exits
at [466–467](../../deploy/db/integration-validation/validate.sh#L466), before runtime
setup. It does not activate DB/Docker for these gate cases. No test sources/evals
helper definitions. The unreadable-directory fixtures are not proof that -r or
grep error handling was exercised on a regular file, especially under root.
Wrapper/caller, real SQL diagnostics, actual role/GUC/pool and rollback behavior
remain execution gaps, not additional demonstrated source defects.

Bootstrap role SQL, eight-secret provisioning semantics, runtime copy selection,
Compose wiring and the four I0a harness files remain byte-identical to the pinned
base. No new service, credential projection, provider operation, schema0060 target,
skip weakening or role/pool readiness claim was introduced by this correction
delta. Non-bootstrap sanitizer behavior is preserved.

## Actual checks, remaining work and handoff

Actual local checks: current instructions/brief/private final/recovery and bounded
source reads; scoped diff/numstat review; exact hashes/line counts for the three
sources, fifteen references and prior report; Git blob comparison of the fifteen
references against f1dfb91. bash -n was run separately on each of the three named
scripts and returned exit 0. Scoped git diff --check returned exit 0.

AST-only parsing of the complete embedded Python body passed (ten top-level nodes).
Two initial local extraction attempts failed: one included trailing shell text;
one over-greedy selector yielded no expected regex constants. The corrected
line-bounded heredoc extraction parsed the full body and completed the inert
33-row and dynamic-token regex/tuple calculations. No embedded function/body,
source validator, provisioner, self-test or test was executed. All 30 report links,
line bounds, fences and whitespace checks passed. All 18 scoped source/reference
hashes and line counts, plus the prior report, remained exact at final stability.

The current exact memory context helper returned MCP_ERROR/no response. No context
was inferred or alternate scope/session used. Finish assessed this unadopted
source review with no_durable_change and returned SKIPPED, saved=false and
incomplete=false. The authorized
administrative receipt exception does not save project progress or another agent's
envelope. Basic Memory was not used.

Changed file: this report only; final exact lines/hash are supplied in the handoff.
Deviations from task scope: none. Missing required inputs: none. Additional missing
or contradictory requirements: none beyond the parser requirement gap above.
Other concrete findings: none. Coordinator follow-up: resolve I0b-F-R01, freeze the
new bytes, then recover independent/hosted verification for the exact candidate.
The original three dispositions are source-review closures only; neither old CI
nor this report accepts the corrected package or full R2.

Not verified: compilation, Clippy, tests/self-tests/validators/provisioners, hosted
CI, shell runtime/error propagation, grep I/O/regular-file permission denial,
marker-write failure, SQL/DB, actual role/password/grant rollback, actor GUC/pools,
installed UID/mode/ACL access and denials, production/provider/runtime state,
deployment and Owner/coordinator acceptance. Other unreported missing evidence:
none. I0a helper execution, first0060, I6 diagrams and activation remain later work.
Further decomposition needed: none; the correction is local to the owned parser.
No source, prior/other report, roadmap, plan, diagram, ledger, private receipt,
dependency or Git staging/commit/push change was made, apart from the authorized
memory hook's operational receipt. No root/sudo, Docker, network/workflow/provider,
production/environment/secret read, lifecycle, native/Paseo delegation or background
process occurred. Background work and pending permissions: none.
