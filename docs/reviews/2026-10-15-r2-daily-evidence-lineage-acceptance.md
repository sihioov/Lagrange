# R2 I1b-S3 — lineage and calendar acceptance

Completed 2026-10-15 (Asia/Seoul), bounded to WP-4/I1b-S3 under R2-04/R2-09.
Accepted candidate: `eb0e839b5e808fb1353451b47baa1850d248ebb9`.
[CI 38711965495, attempt 1](https://github.com/sihioov/Lagrange/actions/runs/38711965495)
completed SUCCESS at 09:40:29 UTC. **Original/copy lineage and offline
calendar reconstruction are implemented and verified on this hosted candidate.
The complete I1b package and R2 remain open.**

## Accepted scope

The [bounded contract](2026-10-15-r2-daily-evidence-lineage-contract.md) follows
the adopted D4/D5 interfaces. Borrowed S2 snapshots resolve exact committed
references across the kis and kis-calendar scopes to the uncopied original.
Every hop checks mode, entitlement, byte and metadata equality, reference
identity and retrieval order, with explicit future/cycle/depth rejection.
Original batch precision and all recorded commitments are retained.

Calendar projection validates the original first-page request and every
response row. Duplicate decoded JSON keys, malformed surrounding rows, duplicate
dates and missing target evidence fail closed. Row/source hashes and one-day
reconstructed documents are deterministic. Continuous proofs require every civil
date, retain equivalent witnesses and reject conflicts throughout the supplied
set. Expected-session selection uses an explicit clock and the 16:30 KST cutoff.

The existing normalizer reuses its extracted pure date/flag mapper with the
previous filtering, outputs and errors preserved. Accepted snapshot/storage,
migration0060, workflow and dependency files are unchanged. This slice adds no
provider request, Raw write, SQL projection write or admission authority.

## Verification

| Evidence | Result |
| --- | --- |
| Candidate identity | 262 input pins, 215 committed blobs, seven actual checkout SHAs match |
| Complete hosted CI | Seven execution jobs plus required SUCCESS; 83 successful steps and one expected failure-log-only skip |
| New behavior | All 22 named Linux S3 fixtures passed |
| Existing normalization | All 37 affected normalizer/calendar names passed |
| Previous R2 regressions | 99 Rust, 32 daily SQL, 28 affected legacy DB, 3 transport and 6 separate chart names passed |
| Dedicated DB | 32 named tests, 73 exact subcases, 12 outer DB exits 0/no-SKIP, 83 bootstrap names passed |
| Migration/authority/cleanup | 60 SHA384 checksums match twice; NO_OP, direct roles, 0039/catalog/0060 rollback and cleanup gates passed |
| Whole-candidate checks | Strict Clippy, workspace format, policy, Python and Web including E2E passed |

The [CI record](2026-10-15-r2-daily-evidence-lineage-ci.md) preserves exact counts,
hashes, platform exclusions and unknowns. Inventory groups are reconciled
separately; their counts are not a unique-test aggregate. The coordinator directly
reviewed source and reconciled results. No new independent review is claimed;
final integrated independent review remains open.

## Limits and next boundary

- These tests use synthetic committed storage. They establish the read-only
  fixture behavior and held-lock lifetime, not installed UID/ACL or live data.
- Entitlement, Owner/policy and observation time are explicit caller context.
  The later trusted pipeline must bind them to installer and database facts.
- Proofs cover the supplied calendar source set. They do not seal a complete
  source frontier, reconstruct bars/reference/actions, or admit an artifact.
- Provider JSON has byte/depth limits; this is not a claim against every possible
  allocator failure. Original timing remains batch precision.
- Non-Linux execution is unverified; two carried platform cases remain excluded.
  Python's ten skips and unprinted nested helper rows/inner exits retain their
  existing status.
- No operational release, collection activation, Owner interaction or
  three-trading-day observation was performed.

Next is the remaining **I1b price/reference/corporate-action reconstruction,
complete source frontier and artifact/admission/supply** work against D4/D5.
Then I3–I7, final integrated review, runtime UID/ACL and installer filesystem
rollback must close. Diagrams must track structural changes before main.
Operational activation and actual Owner use precede the three-consecutive-
trading-day observation; that period has not begun.

R2-04/R2-09 remain DOING. This acceptance does not advance to R3 or complete
final C_R2. The [I2 acceptance](2026-10-15-r2-daily-sql-acceptance.md), historical
snapshot reviews and every previous CI failure remain preserved.
