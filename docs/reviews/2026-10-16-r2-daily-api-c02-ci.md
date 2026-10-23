# R2 Wave65 I5-C02 full CI recovery

Roadmap R2-04/09; candidate abfca9cdf186e559092209e8c7340ecad02728fb.
[CI38815899362 attempt1](https://github.com/sihioov/Lagrange/actions/runs/38815899362)
finished FAILURE. All eight terminal jobs/full-log hashes and seven actual
checkouts recovered; 722pins/375committed blobs matched. No rerun/cancel/duplicate.
Later documentation HEAD does not alter the tested candidate.

Workspace tests, strict Clippy, format, Web, Python and policy passed.
I5 Rust25/25 and every listed carried regression group passed; chart6 separately.
Web six new contract markers, supervisor34, OFF121 and market-stream14 all passed
without skipped/flaky/retried browser cases. Python487pass/10existing skips/7warnings.
Three platform exclusions and unprinted nested helper rows/inner exits remain
explicitly excluded and UNKNOWN respectively.

All18 dedicated PG command targets exited0/noSKIP. The final target's actual
three binaries printed all15 required API names as ok, including all10 daily
HTTP cases and the previously failing read-only/no-generic-ready case.
Other PG evidence passed: publish8, schedule32, dailyPublish6, recovery6, runner24,
daily32/subcase73/bootstrap83, 62migration SHA384 checksum sets twice, no-op rerun,
direct roles, 0039 guard, catalog and cleanup.

The PG validator still failed its canonical result-name guard. Its replay regexp
at deploy/db/integration-validation/validate.sh:1542 allows [a-z_]+ only.
Two required names contain the digits503; their actual ok rows at full-log
lines1935/1937 were discarded. It emitted13/15 canonical rows and correctly
refused acceptance. The selected binaries start at1748/1847/1931; the separate
earlier admittance-pool binary is not counted a second time.

The original PG log SHA256 is
2c6f28ba46c4d1c88b2bb8a9f2767df346c82c6c6c556ad12c3b19f09d42d1d9.
Its refetch matched that full hash. Sanitized fixed rows/binary anchors are in
target/r2-daily-beta/ci/abfca9c/pg-diagnostic-supplement.json,
211lines/24ba4eeb2173a3c08f8c94661cbe19db13852769b4d54590f1ddd92b6d8f7083.
Raw logs were not persisted.

Full receipt: target/r2-daily-beta/ci/abfca9c/ci-evidence.json,
8693lines/159718d2c23eef3c25b6039825e9b2f750fe276adef54ec7f9c8ace34716c857.
Actual test success does not override the failed validator/required job.
No I5 package acceptance is claimed.

Wave66 I5-C03 will change only the replay name character class to allow digits
after an initial lowercase letter or underscore. Preserve all15 exact names,
uniqueness, cardinality, Cargo exit and noSKIP guards. No product source, test
assertions, DB privileges or workflow changes. A new exact-candidate CI is
required after review/freeze and all-event run query/remote verification.
I7 drafts remain RAM-only. I7/I6, real approved-base/threeUID integration,
independent final review, G4, Owner use, three trading days and C_R2 remain open.
