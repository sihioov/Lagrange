# R2 I1b-S5c C01 — fixture ancestry correction

2026-10-15; R2-04/09. Wave46 candidate718d99d/run38737242162 attempt1 is fully
recovered before this change. All16 new filesystem behavior tests stop in their
common fixture Root::open with UnsafePath at artifact.rs:969; their later assertions
did not run. Ten other new cases and all carried named groups pass. Strict Clippy,
format, web, Python and PostgreSQL pass; workspace/required fail. The failure and
same-hash diagnostic supplement are preserved in the [CI record](2026-10-15-r2-daily-artifact-admission-ci.md).

The fixture previously created its root underneath current_dir(), assuming the
checkout's full ancestor chain met protected-storage requirements. That assumption
failed. The log does not identify the precise ancestor ownership/mode/ACL condition;
no more specific environmental root cause is asserted.

C01 changes only the fixture parent to the test user's HOME. HOME must exist;
there is no /tmp fallback, guard bypass, skip, chmod of shared ancestors, UID
substitution or suite serialization. Production Root::open still checks the entire
new path and can reject it. This is a concrete correction of the fixture's ancestry
assumption, awaiting hosted execution, not a claim that the new anchor passed.

All33088 production bytes before the private test module are identical to718d99d,
SHA2567c2820e4cfe4a621b7410bb7e608fb6d637f9ff777bc9b35a8eacf96e6ab1892.
All27 new names and prior groups remain unchanged; line anchors are regenerated.
Scoped standalone2024rustfmt and whitespace pass. No local compiler/test/helper,
provider, DB, Docker, runtime action or delegation. No source acceptance yet.

Freeze one successor and query all-event exact-head runs/verify remote SHA before
one new hosted dispatch. Never rerun/cancel/duplicate the failed run. The unregistered
continuation/price-extension/frontier preparation is excluded from this candidate.
Real approved-base/distinct-UID integration, fault/crash durability and all remaining
R2 gates remain open. Ralph continues; R3 is not substituted for unfinished R2.
