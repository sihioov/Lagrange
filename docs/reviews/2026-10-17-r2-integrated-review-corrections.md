# R2 integrated review: coordinator disposition and bounded corrections

R2-01/03/04/07/09 remain open. The integrated review found two P1 and two P2
conditions, and did not finish the full review scope. This change addresses
those four conditions and the separate D01 evidence-template discrepancy.
It is a source checkpoint, not independent closure or executable acceptance.

## Recovered independent result

The owned reviewer `a4b5c1b9-3354-400b-be82-b8dec4f87278` became actually idle at
2026-10-16T18:21:56.529Z, with no active turn or pending permission. The full
substantive final was recovered at18:22:18UTC; the later memory-only callback
was not substituted for it. The original [602-line report](2026-10-16-r2-integrated-release-review.md)
is preserved with SHA256
`1efc10bff851ab9bb7fbf3085cf5351dbd9a1961ab703b5d4287d102ffd5ef85`.
After durable recovery the old heartbeat57e90e28 was deleted successfully.

Before editing source, the coordinator independently matched all211 frozen
pins,202 candidate blobs and231 report links/line bounds. Coverage is exactly
F65/D56/P26/U55 across202 targets: F means full text, D changed hunks/context,
P specified partial ranges, U identity checks only. This is INCOMPLETE, and
neither hashing nor the earlier CI success fills unread source. Private recovery
and exact residual rows are in `target/r2-daily-beta/wp-7/f215665/`.
Two read-only receipt-inspection attempts failed on inventory/link text handling;
the corrected inspection completed at18:29:23UTC. These were not project tests.

## Findings and corrections

| Finding | Coordinator source disposition | Bounded change |
| --- | --- | --- |
| P1-01 | Confirmed: both official EOD suppliers call the backfill configuration scope, which rejected a valid daily ON environment. | Permit backfill alongside release for that mode. The closed mode parser, protected env, override rejection and actual release activation checks remain. No other scope or provider allowance is added. |
| P1-02 | Confirmed: ordinary users RLS is enabled but does not filter its direct table owner; testing the catalog flag blocked bootstrap. | Require effective identity visibility through `row_security_active`, while retaining strict caller, missing identities, actor bounds and orphan-history rejection. |
| P2-01 | Confirmed: the serving provisioner and validator both require10003:10003/0440; reconciliation required0400. | Align this one unmocked protected-file read to0440; preserve directory, owner, group, link and ACL checks. |
| P2-02 | Confirmed: create occurred outside cleanup protection and temporary staging was deleted even when cleanup was uncertain. | Persist a root0600 owned-name/nonce/image/staging intent before create. Recover only its exact resource, confirm removal before clearing the intent, retain staging on uncertainty and block another verification before operational quiescence. |
| D01 | Confirmed: the full-release evidence template still said60/60, while the full schema is62/62. | Update only the full-release counters. Earlier0060 staged evidence stays historical. |

The daily restore verifier remains networkless, nonroot, resource-bounded and
confined to its copied dump/data and fixed scripts. A lost reply followed by an
empty lookup is still ambiguous: the daemon can complete later. A retained
`daily-verifier-<nonce>.json` therefore blocks subsequent backup/verify/reconcile
execution. It is not an instruction to delete the record or force-remove a
container. Bounded operator recovery must first bind that exact name, labels,
image and copied staging, resolve any in-flight create, and confirm termination
and removal before clearing the record/copy. Unknown identity or daemon state
keeps them retained. This patch does not execute that recovery or introduce an
automatic stale-intent deletion path.

## Regression scope and limits

Ten new definitions are written in four already-wired hosted test modules.
All94 baseline names remain;104 definitions now appear in those four modules.
All new definitions are **WRITTEN_UNEXECUTED**. The private source inventory is
`target/r2-daily-beta/wp-7/wave87/test-inventory.json`.

- Three supplier definitions use the real configuration validator and real
  root-owned fixture metadata. Both official helper entry paths reach that
  validator with ON inputs. A test wrapper stops after validator success before
  DB/provider execution; calendar data is synthetic. OFF default and negative
  mode/override cases remain distinct.
- One actual-schema definition observes bootstrap status at migrations59 and62
  and executes the real rendered first-prepare SQL as direct migration_owner.
  The existing Docker fixture now seeds synthetic Owner/entitlement authority
  required by that transaction. This does not establish installed service startup.
- One actual-transport definition distinguishes effective FORCE RLS and missing
  identities. The existing orphan-run case now also enables ordinary users RLS.
- Two reconciliation definitions exercise the unmocked serving-secret reader
  using actual fixture metadata. Docker discovery is mocked; native execution
  stops at the next mount boundary. Wrong modes/owners/groups and links fail.
- Three disposable-Docker definitions cover an observed lost create reply, an
  accepted but temporarily unobservable create, and failed cleanup inspection.
  They require copied-state retention and duplicate rejection on uncertainty.
  Fixture cleanup targets only IDs returned by that fixture's own creation.

Only source/diff inspection, text inventories, hashes and whitespace checks are
performed locally. No project tests, compiler, SQL, production helper, provider,
image build or application lifecycle action is run. The six moved backup diagram
citations are refreshed; diagram topology is unchanged. PNGs are regenerated
locally from those documentation sources under the existing bounded renderer.

The final source checkpoint at 2026-10-16T18:45:45UTC identifies exactly seven
executable edits plus the separate D01 template edit among the original202
targets;194 target pins and all nine old draft/evidence pins are unchanged.
The original report hash is preserved. Source-text inventory confirms94 retained
and10 new names, and whitespace checks pass. The refreshed PNGs have identical
decompressed raster bytes to the prior inspected render; only embedded source
metadata changed. These are static checks, not compiler/test results. Receipts
are `target/r2-daily-beta/wp-7/wave87/source-checkpoint.json` and
`diagram-render/receipt.json` in that directory. The first checkpoint assertion
omitted D01 from its expected target list; correcting that classification did
not add an executable edit. The unrelated32 ROADMAP additions remain unstaged.

## Required continuation

Keep the original review and exact residual coverage. Continue the same owned
independent WP-7 review after freezing this coherent correction candidate;
no additional reviewer or worker is authorized. Reinspect all changed source
and regression definitions, then finish or explicitly bound the original
unreviewed wire/S1/adversarial, migration/helper and API/browser/Owner V2 scope.
No aggregate claim of full coverage is permitted while any required scope is
unreviewed. A coordinator's implementation review is not independent closure.

The earlier `f215665`/CI38901117928 success remains valid for its old hosted
scope. Do not run the missing functional smoke against that known-blocked old
candidate, or rerun/cancel old CI. After review closure, query existing runs and
remote SHA before one new exact-candidate CI and the separately required
research functional smoke. Actual R2 images, approved-base host flow, root
access, concrete G3/G4 operational packets, Owner use, three trading days and
final C_R2 remain open. Calendar waiting has not started merely because source
work or CI completed.
