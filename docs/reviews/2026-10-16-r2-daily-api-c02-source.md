# R2 Wave65 I5-C02 test verification correction

Roadmap R2-04/09; baseline3480afe after full failure recovery of CI38812634642.
Direct coordinator source review; independent review and executable acceptance
are not claimed. All25 I5 Rust and six Web definitions remain preserved.

- Five index-signature accesses in the Web contract fixture use brackets under
  existing noPropertyAccessFromIndexSignature. Four daily input reads and one
  list items deletion retain exactly the same object keys and assertions.
- Only two cross-table row-count observations in state_no_fallback switch from
  migration_owner to the harness's existing read-only admin pool. The existing
  expected delta of one fixture job, one historical run and zero audit writes
  remains unchanged. Both HTTP state reads and all response assertions remain.
- jobs uses forced actor RLS (0010:51-57); recommendation runs use forced actor
  RLS (0049:172-190); migration_owner has no audit SELECT policy (0010:130-138).
  The existing admin role has explicit SELECT grants/policies for all three.
  The product serving pools, actor transactions, privileges and RLS are unchanged.

Hosted failure evidence contains SQLSTATE22P02 for this test but does not print
the exact failing SQL. The correction is supported independently by the invalid
verification connection visible in source; no unobserved runtime statement is
claimed. This does not grant the API an admin daily-state read path.

Complete two-file diff, exact baseline comparisons, preserved prior pins and
all25Rust/6Web names were inspected. Scoped standalone Rust2024 and whitespace
checks pass. No local compiler/tests/lint/Web formatter/project helpers ran.
Fresh exact-candidate18-target PG, workspace, strict lint and Web acceptance
remain required. No new definitions, type/lint suppression, expectation removal,
workflow edits or gate weakening. Prior reports and raw-free receipts preserved.

| Changed path | Lines | SHA256 |
| --- | ---: | --- |
| `crates/api-server/tests/http_owner_beta_daily.rs` | 438 | `d8df8772046f48c6ec5d63a2c51472674aea0425fc261e2b72c1cb1633c860dd` |
| `apps/web/tests/r2-daily-api-contracts.test.ts` | 214 | `ea087c09f6d18bf219f2f0e1328a99bb5c4c6173613f2db2856d4084e75edd4d` |
