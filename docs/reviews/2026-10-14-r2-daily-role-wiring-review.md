# R2-04/09 I0b-R — independent role and secret wiring source review

2026-10-14, Asia/Seoul. Assigned codex/gpt-6.1-sol/high, intermediate complexity,
high confidence, reused direct reviewer. This is a new review of wave-8 bytes;
earlier wire/I0a reviews were not redone or edited. Coordinator owns acceptance,
corrections, Git/docs integration and hosted verification.

**Three concrete findings: one P1/HIGH and two P2/MEDIUM.** The static-check fixtures
omit the eighth credential, the release validator omits its bootstrap projection,
and failed bootstrap logs bypass the new credential-disclosure gate. No source
acceptance, executed SQL, new CI result or full R2 readiness is asserted here.

## Inputs, instructions and comparison

Read repository [AGENTS](../../AGENTS.md), applicable provider instructions in
`/home/l1nnx/.codex/AGENTS.md`, [ROADMAP](../ROADMAP.md), the final
[wave-8 fixed brief](../superpowers/plans/2026-10-14-r2-daily-input-implementation.md#L1027)
and [wave-9 review brief](../superpowers/plans/2026-10-14-r2-daily-input-implementation.md#L1146).
Applied the explicitly invoked paseo-delegate skill to this direct assignment;
its delegation mechanics did not override the user's prohibition on delegation.
Read the [source handoff](2026-10-14-r2-role-and-raw-source-handoff.md) and private
`target/r2-daily-beta/wp-4-integration/wave8-I0b-final.md` implementer final.

Accepted comparison baseline: `929fcd68eb99be5ec18bbbfbb585e6cc17655fee`.
Package brief `0a98b9f`, startup docs `da0499e`. The baseline's accepted hosted
result is background supplied by the assignment; it does not validate these new
bytes, and no hosted job/status/log was queried. HEAD may advance while the
following bytes remain frozen.

All nine owned review inputs matched the actual-byte inventory before review.
The malformed production-validator digest in the private implementer final is an
administrative report error; only the inventory's actual digest below is used.
No source tampering is inferred from that report error.

| Source reviewed against baseline | Lines | SHA-256 |
| --- | ---: | --- |
| [Role bootstrap](../../deploy/db/bootstrap-roles.sh) | 303 | `66b91261cdefbb0709234c5c3b2d3399ccf96428594c7efd0c74d81910151ddd` |
| [DB source provisioner](../../scripts/ops/provision-db-secrets.sh) | 613 | `2b2650c12fa3914f8258d40e034409dac3ce46cd03a8807646d0a898e4fa6125` |
| [Runtime provisioner](../../deploy/secrets/provision-runtime-secrets.sh) | 343 | `f3529be0ff55dc0f1f3866a8cd43aa4a83ffa62da36e95796dd3600a25974f5c` |
| [Runtime static checker](../../deploy/secrets/runtime-static-check.sh) | 418 | `fa2a9ea7559a09ce0ea53763008836776b017eb1a297702a7509b1fc176f923f` |
| [Production validator](../../scripts/ops/validate-production-config.sh) | 887 | `5ee7223a8012ce480ad3493878fe3494bab87fa90021000e8a7ac72827165510` |
| [Compose](../../deploy/compose/compose.yml) | 1632 | `434643b6c749e7ceca8abcca1f29726f55bd9ef86ee42c08db7e53ed1d4c7931` |
| [Example environment](../../deploy/compose/.env.example) | 149 | `e37029e2b47ef70360666e070eae591de4dc657186f430daef593bad19a9c330` |
| [Official isolated validator](../../deploy/db/integration-validation/validate.sh) | 1453 | `0676d8839a6fcde7f280371aff84868ec83591311832db5c1ce7a9b9e993838a` |
| [Operations fixtures](../../scripts/ops/self-test.sh) | 1483 | `496702920d258e7252d7da96503adc22deca6609cb1866110c7b0abe67230d17` |

The scoped baseline diff has exactly these nine paths, 432 added and 34 removed
lines. Reviewed every changed hunk and the adjacent bootstrap, provisioner,
inventory, fixture and validator call paths used below. Read-only adjacent CI,
service-login and static-check source established the callers; those files were
not modified or executed. No review of the sibling Raw implementation is claimed.

Nine reference files matched the inventory and their accepted-baseline blobs:

| Unchanged reference | Lines | SHA-256 |
| --- | ---: | --- |
| job-queue tests/common/mod.rs | 401 | `242977cf561d54c88889feeb695c8de5b90a64910fd3fcbfde346a17a06e68fa` |
| api-server tests/common/mod.rs | 933 | `6735a723f3c874c129ed43685f8633c372a8384ba2d23da407bebb7d18ed7dcb` |
| migration-contract role-bootstrap.sql | 94 | `9dfe008fdd08b49587940e14fc6f441b2a0053da5f48651fec98fdb53c31b85a` |
| migration-contract bootstrap.sql | 33 | `24fcb7f582f927d9d94827597c74989e005c3378530a3b9c5677e6ac29bf3c77` |
| market-data daily_price_only/contract.rs | 4183 | `f086c065b903a00a022eeef3b37daf3b021691f0fa2fc525b17be1a7ed4ea523` |
| market-data tests/daily_price_only.rs | 2385 | `1596ecbe2b673264fdec2f41605e043f3d7fb79ea54463ac5075ad9da3cd0fdd` |
| nt/live-node/tests/test_runtime.py | 479 | `e2121f61d3be269b4b8b15313c6c0e8c4fe8ed34a218bac1c908cf72ae3dfb97` |
| D4 contract | 950 | `fa0529033945437e78eca5b0805f0524a4ae3c75fc4f0a36321ec56b052e717f` |
| D5 amendments | 740 | `8a8796d992da533389bac97fab6abf86c7e89e75f0772f6deceb3d4051d651a6` |

## I0b-R01 — static checker fixtures cannot satisfy the expanded inventory

**Severity:** P1/HIGH. **Affected package:** I0b static checker/fixture integration.
Verified source contradiction; no shell checker or fixture was executed.

The runtime provisioner now requires the new admitter source as its third serving
copy at [provision-runtime-secrets.sh:151](../../deploy/secrets/provision-runtime-secrets.sh#L151).
Selected sources are preflighted in copy order at
[269–273](../../deploy/secrets/provision-runtime-secrets.sh#L269); a missing source
exits at [232](../../deploy/secrets/provision-runtime-secrets.sh#L232).
The static checker's crypto fixture still creates only the old DB sources at
[345–347](../../deploy/secrets/runtime-static-check.sh#L345), then runs the
serving-prereqs provisioner and requires a cursor-shape diagnostic at
[356–365](../../deploy/secrets/runtime-static-check.sh#L356).

**Trigger and consequence:** on the existing crypto fixture, TLS files exist but
db_daily_input_admitter_password does not. Preflight exits for that missing source
before reaching the deliberately malformed cursor. The expected cursor diagnostic
is absent, so the checker calls die. This happens independently of fakeroot.
The policy CI step calls this checker at
[ci.yml:55](../../.github/workflows/ci.yml#L55); operations self-test also calls it at
[self-test.sh:225](../../scripts/ops/self-test.sh#L225). Their written paths cannot
complete with these bytes, although no actual job failure was observed here.

The separate fakeroot numeric fixture has the same omission at
[380–382](../../deploy/secrets/runtime-static-check.sh#L380), so after repairing the
first fixture it would still reject its intended positive provisioning path when
fakeroot is available. It also has no assertion for the new 10003 projection.

**Bounded correction:** include a valid, nonsecret eighth source in both fixtures;
keep the cursor fixture's intended error/no-write/redaction assertions. Extend the
positive numeric fixture with the admitter file/directory ownership and mode checks
(10003:10003/0440, directory 0750), plus the new bootstrap file's 999:999/0400 check.
Do not skip the fixture or relax provisioning's source inventory. All changes fit
the assigned runtime-static-check.sh scope.

## I0b-R02 — release preflight omits the bootstrap admitter runtime file

**Severity:** P2/MEDIUM. **Affected package:** I0b production runtime inventory.
Verified source inventory mismatch; no production configuration was read.

The release provisioner combines infrastructure and serving copies at
[193–198](../../deploy/secrets/provision-runtime-secrets.sh#L193). Its new bootstrap
copy is [142](../../deploy/secrets/provision-runtime-secrets.sh#L142), and Compose
binds it at [352–356](../../deploy/compose/compose.yml#L352), resolving the source at
[1555–1556](../../deploy/compose/compose.yml#L1555). However, the production validator's
release runtime_specs at
[830–850](../../scripts/ops/validate-production-config.sh#L830) adds the 10003 serving
copy but omits `db-role-bootstrap/db_daily_input_admitter_password:999:999:400`.
The other applicable profiles include that entry.

**Trigger and consequence:** with otherwise valid release inputs and runtime files,
delete the new bootstrap copy, or give only that copy the wrong UID/mode. The loop
at [852–865](../../scripts/ops/validate-production-config.sh#L852) never examines it;
the new source and 10003 copy can still pass. The release preflight can therefore
report PASS while Compose/bootstrap cannot consume the required credential, or its
host permissions violate the fixed contract. No claim is made that such a host
currently exists.

Inert text inventory comparison found 33 release copies in the provisioner and 32
in the validator, with this one omission and no extras. Infrastructure 11/11,
serving-prereqs 31/31, backfill 14/14, range-raw 2/2 and recovery 0/0 match.

**Bounded correction:** add the missing bootstrap specification to release's list.
Add an inert inventory parity assertion or a written release fixture that rejects
its absence and wrong metadata. Preserve DB-free scopes and the separate staged
admitter copy. No new service or production operation is required.

## I0b-R03 — failed bootstrap logs bypass the credential-disclosure guard

**Severity:** P2/MEDIUM. **Affected package:** I0b isolated validator log safety.
Verified control-flow and redaction-coverage gap; no actual leak was observed.

The new [credential scan:907–918](../../deploy/db/integration-validation/validate.sh#L907)
rejects any exact credential found in bootstrap output before recording successful
logs. All three failure branches instead record first and exit, bypassing the scan:
[full:1032–1037](../../deploy/db/integration-validation/validate.sh#L1032),
[staged:1205–1211](../../deploy/db/integration-validation/validate.sh#L1205), and
[test:1430–1436](../../deploy/db/integration-validation/validate.sh#L1430).

record_log delegates to the existing sanitizer at
[348–364](../../deploy/db/integration-validation/validate.sh#L348). Its only redaction
patterns are PostgreSQL URLs, DATABASE_URL= and PGPASSWORD=. A failure diagnostic
containing a bare `PASSWORD '<credential>'` SQL literal matches none of them.
The password-bearing DO body and psql diagnostics route exist at
[bootstrap-roles.sh:192–202](../../deploy/db/bootstrap-roles.sh#L192) and
[300–302](../../deploy/db/bootstrap-roles.sh#L300). SQL error context during a
password-bearing statement, for example a lock-timeout failure after preflight,
is a relevant failure case to validate; the SQL/psql output for it was not executed
or observed here.

**Concrete interface counterexample:** a nonzero bootstrap result whose captured
raw log contains `CONTEXT: SQL statement "ALTER ROLE daily_input_admitter PASSWORD
'<64-hex synthetic sentinel>'"` takes the failure branch, survives all three
sanitizer prefixes, and is retained as a purported sanitized evidence log. The
workflow then prints retained failure logs at
[ci.yml:168–177](../../.github/workflows/ci.yml#L168). This text/control-flow proof
does not assume that every bootstrap failure contains a secret, nor assert that
the current constant preflight exceptions leak. The sanitizer/failure branches
are baseline behavior; the introduced scan promises a credential gate but leaves
these paths outside it, including the newly added credential.

**Bounded correction:** make the credential scan gate recording for every bootstrap
exit outcome. On disclosure, retain only a redacted reason rather than copying the
raw log; preserve the original bootstrap failure verdict. Add synthetic success
and failure log cases with URL/env and bare SQL PASSWORD forms, plus a clean failure
case. Inspect the captured SQL-error behavior in the later authorized isolated
lane. Do not infer credential non-disclosure solely from successful bootstrap.

## Guard-to-written-check mapping and closure limits

| Guard / interface | Source implementation | Written checks and limits |
| --- | --- | --- |
| Atomic bootstrap / preflight before password changes | [BEGIN/preflight:90](../../deploy/db/bootstrap-roles.sh#L90), mutation loop 187, COMMIT 291; psql ON_ERROR_STOP 301 | [Static markers/order:51–78](../../deploy/secrets/runtime-static-check.sh#L51); no behavioral wrong-attribute/membership/CREATE or password/ACL rollback fixture added |
| Existing admitter attributes and no repair | [100–134](../../deploy/db/bootstrap-roles.sh#L100) checks LOGIN, NOINHERIT and all five negative privilege attributes before ALTER | [Catalog positive assertions:821–894](../../deploy/db/integration-validation/validate.sh#L821); negative existing-role cases remain unwritten/unexecuted |
| Incoming/outgoing and transitive SET-role paths | [116–122](../../deploy/db/bootstrap-roles.sh#L116), UNION closure 138–156, postflight 243–285 | Static membership/UNION checks and positive isolated catalog check; no seeded direct/transitive/cycle negative fixtures |
| Effective DB/all-schema CREATE and preserved grants | [124–182](../../deploy/db/bootstrap-roles.sh#L124), legacy PUBLIC revoke / CONNECT / USAGE / migration_owner CREATE 219–226, effective postflight 251–287 | Catalog assertions repeat effective checks; no hostile PUBLIC/direct/owned-schema privilege fixture or actual ACL proof |
| Eight source secrets, nonoverwrite and rollback | [Inventory:18–27](../../scripts/ops/provision-db-secrets.sh#L18); complete destination fence 521–525, private staging/distinctness 555–586, hard-link install/rollback 532–552 and 588–609 | [Eight-file apply/check fixtures:809–855](../../scripts/ops/self-test.sh#L809); existing format, newline, partial-set, duplicate and no-overwrite checks use the expanded array; all unexecuted |
| Scoped projection / UID / mode | [Infrastructure and serving inventories:132–199](../../deploy/secrets/provision-runtime-secrets.sh#L132); selected-source preflight before writes 266–277; per-file staged rename/metadata 295–335 | Static inventory/count checks and fixture updates in operations self-test; R01 breaks static checker fixtures, R02 leaves release parity incomplete; no real 10003 read/denial or ACL evidence |
| File path, newline, symlink, format and distinctness | [DB source checks:179–265](../../scripts/ops/provision-db-secrets.sh#L179), normalization 338–457; runtime no-symlink/source checks 84–98 and 228–251 | Expanded source array preserves existing checks; runtime copies inherit baseline per-file atomic rename, not a newly claimed whole-runtime-tree transaction |
| Official bootstrap env / two role maps / generated credentials | [Env:633–648](../../deploy/db/integration-validation/validate.sh#L633), role maps 659–675 and 800–818, inventories 1142–1155 | New admitter entry appears in both maps and generated/test inventories; isolated compatibility passwords remain synthetic and intentionally distinct from production rules |
| Direct login, catalog and role setting | [Full direct-login list:1039](../../deploy/db/integration-validation/validate.sh#L1039), staged list 1213; shared [login SQL](../../deploy/db/integration-validation/service-login.sql); catalog and role observation 821–905 | Current/session identity and role=none assertions written for full/staged lanes; current_setting('role') proves neither actor-GUC behavior nor execution of the unchanged max-2 Rust pool helper |
| Credential output gate | [907–918](../../deploy/db/integration-validation/validate.sh#L907) and success calls | Success-output scanning written; R03 identifies failure-path omission; no synthetic disclosure/SQL-error regression added |
| Existing migration/no-SKIP boundary | Latest migration derived at 328–333; [test loop and guard:1122–1133](../../deploy/db/integration-validation/validate.sh#L1122), unchanged targets 1439–1450 | No 0060 target, new service or skip relaxation introduced by the reviewed diff; first-0060 role/denial/rollback/race and no-SKIP integration remain later work |

The preflight checks the existing admitter's effective CREATE before the legacy
PUBLIC/public cleanup, so an existing admitter inheriting unexpected CREATE is
rejected, not repaired. When the role is absent, the baseline public-schema revoke
is preserved and postflight verifies no effective CREATE. PUBLIC database and
non-public-schema CREATE are rejected before mutation. The all-schema catalog
queries include every pg_namespace row, and membership closure deduplicates
(start_role, reachable_role), so cycles terminate by source inspection.

There is no new supervisor schema mutation beyond the baseline public grant/revoke
rules. migration_owner's public CREATE is preserved and postflight asserted.
The new role receives only target CONNECT/public USAGE. This describes intended
SQL source behavior; it does not prove actual PostgreSQL execution or concurrent
catalog behavior.

Compose changes are confined to bootstrap env, its eighth 999:999/0400 mount and
top-level secret declaration, with comments only otherwise. No producer/admitter
service, network, profile or timer was introduced. New 10003:10003/0440 files are
prepared only by serving-prereqs/release; infrastructure/backfill receive only the
bootstrap copy. API/worker/producer10002 receive no new credential. Range-raw and
recovery remain DB-free. .env.example adds path documentation without a secret
value or unused new environment input. These conclusions are from source maps,
not installed permissions, Docker Compose execution or service activation.

## Actual checks and handoff

Performed local source/document reads, hash/line assertions for nine source files
and nine references, accepted-baseline blob comparison for all nine references,
scoped nine-file diff/numstat review, and inert parsing of copy/runtime inventories.
The inventory check found only R02's release omission; inert fixture-loop inspection
confirmed both R01 omissions. Synthetic log-prefix inspection established R03's
sanitizer coverage limit without reading a real credential or log.

Ran bash -n separately on each of the seven assigned shell scripts: all returned
exit 0. Scoped git diff --check against 929fcd6 returned exit 0. No shell validator,
self-test, provisioner or SQL parser was invoked. All 51 report local links/line
bounds, fences and whitespace checks passed; all 18 reviewed source/reference
hashes and line counts remained exact at final stability check. No Rust source
changed in this package, so rustfmt is not applicable. YAML semantics were inspected
as source; no Compose or YAML parser execution is claimed by this reviewer.

The exact current memory context helper returned MCP_ERROR/no response. No context
was inferred and no alternative envelope, session, scope or Basic Memory was used.
Finish assessed this unadopted review with no_durable_change and returned SKIPPED,
saved=false, incomplete=false. Administrative hook receipts are the authorized exception to report-only
writes and are not source progress or durable saved conclusions.

Changed file: this new report only; exact final lines/hash are in the final handoff.
Deviations from the assigned source-review scope: none. Required inputs not found:
none. Additional missing/contradictory product requirements: none beyond the concrete
interface mismatches above. Other concrete source findings: none.

Unresolved follow-up: coordinator resolves R01–R03 within I0b, freezes the correction
bytes, and arranges independent/hosted verification of the new exact candidate.
Behavioral wrong-attribute, bidirectional/transitive membership, cycle, effective
CREATE, rollback and credential-disclosure cases remain unwritten or unexecuted as
mapped above; their execution is not supplied by prior accepted CI. No requirement
for production role inventory, future 0060 or new provider operation is invented.
I0a helper execution and actor-GUC/pool behavior, first-0060 readiness, I6 diagram
source/PNG integration before main, later runtime activation and Owner acceptance
remain assigned later-package boundaries, not claimed closures.

Not verified: compilation, Clippy, new hosted CI, behavioral shell/test/SQL/DB execution,
actual role/password/privilege rollback, direct login, actor GUC, pools, installed
ownership/mode/ACL/read-denial behavior, secret provisioning, production/provider
state, runtime behavior and acceptance. Unreported missing evidence: none.
Further decomposition needed for this bounded review: none; reported corrections
fit its existing files. No source, contract, roadmap, plan, ledger, other report,
Git staging/commit/push or dependency change was made. No root/sudo, Docker, network,
provider, production, secret read/provisioning, lifecycle, native/Paseo delegation
or background operation occurred. Background work and pending permissions: none.
