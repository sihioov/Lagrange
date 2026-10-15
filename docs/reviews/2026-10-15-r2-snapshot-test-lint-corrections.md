# R2 snapshot C05 test-code lint corrections

Scope: WP-4/I1b-S2-C05, R2-04/R2-09, after complete recovery of C04
`d883c17c4217a29681a649fd29641172f53b6355` / run 38662224666 attempt 1.
The coordinator made two bounded edits directly; no new worker or independent
worker review was started. Hosted verification of this new candidate is pending.

| Finding | Final source | Disposition |
| --- | --- | --- |
| C05-01 needless_borrows_for_generic_args | daily_price_only_snapshot.rs:1326 | Pass the one-use metadata_path PathBuf directly to hard_link. Fixture, destination, valid control and exact HardLinkedFile assertion are unchanged. C01's stored-PathBuf reuse borrows are unchanged. |
| C05-02 type_complexity | source.rs:2726–2731 | Add cfg(test) private TestPhaseHook for the identical Box<dyn FnMut(CapturePhase) -> Result<(), DailyEvidenceError>>. RefCell/Option storage, default static callback lifetime, reset/drop behavior and callback invocation are unchanged. |

The alias is test-only; no production hook, exported API, capability trait,
dependency, guard or lint allowance changed. The source correction is exactly
one hunk per file against C04. All 49 S2 definition names and 60 other C04 pins
remain identical. No implementation-mirroring tests were added for these edits.

| Final file | Lines | SHA256 |
| --- | ---: | --- |
| crates/market-data/src/daily_price_only/source.rs | 4448 | 33af9bbafba7b2b0a3c85007a1dfd00e9a17adbef09c8e37de885b6745939a98 |
| crates/market-data/tests/daily_price_only_snapshot.rs | 1972 | 7dc0eb24dd4cad1a2ae5568682f7c2b42ff5e05f4de815ae39f68e81dc1bec87 |

Scoped standalone rustfmt edition 2024 and whitespace checks passed. Exact text
replacement, unchanged pins and definition-name comparisons passed; private
wave33-C05-source-checks.json has 379 lines, SHA256
`029a1e1fa5e9454e89f9f5556b8c781e41ebba4050221541d9d323a5ff777f40`.
No local compilation, Clippy, tests, project helpers, database or provider work
was run. The predecessor's 99 Rust and six chart PASS rows are evidence for C04,
not execution evidence for C05. Strict lint/source acceptance remains pending.

After committing and pinning this candidate, verify the remote SHA and query
all-event exact-head CI runs. Reuse an existing run or dispatch once when none
exists. Recover all jobs/steps, exact checkout/log hashes, named results and PG
evidence. Preserve previous failures and unknown nested helper rows/exits.
All later R2 gates listed in the plan remain open.
