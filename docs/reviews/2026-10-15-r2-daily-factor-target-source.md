# Wave50 — I3 daily loader, factor and target source disposition

Roadmap R2-04/09; adopted D1 section7. Baseline
`edeaa5db456bc4c62e6eef6e7a561e6a49accff4` is a fully recovered failed candidate,
not an accepted source baseline. Its two cfg(test) visibility errors are corrected
alongside I3, with the [failure evidence](2026-10-15-r2-daily-continuation-frontier-ci.md)
retained. Direct coordinator review only; no new worker or independent-review claim.

## Scope and trust boundary

- market-data daily_price_only/input.rs adds AdmittedDailyPriceInputV1 as a borrowed
  view of existing opaque reopened first/continued sealed inputs. It has no wire,
  caller-bars, path, policy or fabricated approval constructor. This proves local
  sealed input, **not current DB admission/revocation/Owner authorization**; I4 must
  apply those gates before computation and inside settlement.
- factor-engine adds daily_price_only.rs and PriceOnlyBars::from_admitted_daily.
  Only the sealed capability and its exact frontier date enter the builder. It
  revalidates files before/after input resolution and after computation. ETF11,
  adjusted-close-only, point-in-time and no-forward-fill rules share the existing
  input layer. Factor arithmetic alone is extracted unchanged; historical preimage
  and historical builder/approval path remain unchanged.
- job-queue adds daily_input.rs. The exact daily shape has schema/run/config/date,
  input kind plus four pins, existing sealed strategy and DAILY trigger epochs.
  Exact literal enums, nonnegative i64 epochs and strategy rehash are checked.
  OwnerBetaExecutionInput dispatches only on absent schema or the exact daily tag;
  unknown/null/mixed shapes fail. A recursive visitor rejects duplicate keys before
  Value conversion, including a duplicate whose last value would otherwise be valid.
  Historical serialization stays the original untagged five-key object. The byte
  decoder caps input at1MiB; serde retains its normal recursion bound.
- Daily factor and target canonical preimages have distinct versioned schemas and
  bind content/manifest/admission/policy hashes. No historical pins substitute for
  daily provenance. Daily factor serialization rejects nonfinite values rather than
  allowing JSON null to hide them. A serialized snapshot is computation evidence,
  never an approval or publication token.
- daily_target.rs uses only shared pure row validation, factor-version/close-only
  validation, existing five strategy equations, rounding, canonical items and reasons.
  Historical target preimage and constructors remain intact. Daily target has private
  fields, exact factor hash/pins/date validation and a recomputed canonical hash.
- daily_compute.rs requires the reopened capability, validates the sealed job pins,
  derives exactly the strategy-declared close-only factor set, builds both outputs
  and revalidates the input before returning. It performs no DB, provider, process,
  environment or latest-pointer operation. I4's existing runner dispatch is not yet
  connected; this change supplies its typed computation seam.

One scope addition to D1's I3 file list is the small market-data input adapter
and its module declaration. It reconciles the adopted AdmittedDailyPriceInputV1 name
with the already implemented sealed first/continued handle types without weakening
their constructors. Existing input.rs and compute.rs are entirely untouched.
The other is the isolated hosted PostgreSQL harness: its existing jobqueue lane
previously selected only the admitter-pool test from owner_beta_publish. The eight
publication/recovery fixtures can return early without DATABASE_URL, so workspace
`ok` rows alone cannot validate them. A separate full-target invocation now requires
all eight exact names once, numeric exit0 and noSKIP; it prints only finite named
result markers. Existing twelve DB invocations remain, making thirteen outer labels.
This change is source-only locally; no harness, shell checker or DB was executed.

## Written verification and preservation

20new definitions, all WRITTEN_UNEXECUTED at source handoff:
3private shared-factor regression tests,5daily factor canonical/hash cases,
12daily loader/target integration-contract cases. They cover exact domain bytes,
four-pin sensitivity, nonfinite versus null, future filtering, missing sessions,
liquidity rejection, legacy untagged roundtrip, strict schema/duplicates/epochs,
strategy mutation, all five baseline outputs, all-cash null handling, concrete
rounding residue, future/missing/duplicate/unknown rows, bad factor versions and
stale factor hashes. Test wire values do not mint an admitted filesystem capability.

Existing factor/target/input/compute tests and owner_beta_publish replay fixtures
remain required in addition to every prior S5def/S5c/S5ab/S4/S3/I2/R2/PG group.
The preservation receipt `wave50-I3-preserved-regions.json` under the private
wp-4-integration directory records9 direct comparisons: historical factor definitions,
canonical bytes/builder and assembly/tests; exact factor arithmetic; historical target
public/preimage/wire shapes; all five strategy/rounding/reason bodies and old tests;
entire historical input/compute files; continuation production bytes. Only named
private visibility substitutions were removed for the target wire/helper comparison.
These are static comparisons, not executable verification.

Scoped standalone edition2024 rustfmt and whitespace checks pass. No local Cargo,
compiler, tests, project helper, database, provider or production execution occurred.
Freeze exact pins and named definitions before one new exact-candidate hosted run,
only after all-event existing-run query0 and remote SHA equality. Earlier runs must
not be rerun, canceled or duplicated.

## Remaining limits

No compiler/test/source acceptance before full successor CI recovery. Real approved-v3
factory through complete distinct-UID multi-day admission/frontier/computation remains
untested. Pure wire tests are not that flow. Full crash/ACL/installer rollback evidence,
I4 runner gates/scheduler/publication/recovery, I5 API, I6 release/diagrams before main,
I7 web and final integrated independent review remain. G4 concrete supplier/activation
delta, operational application, actual Owner use, three trading days and final C_R2
remain separate. Ralph continues toward those conditions; R3 is not started here.
