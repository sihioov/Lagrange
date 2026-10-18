# R2 image fixture feature receipt correction

R2-04/09, Wave94. Both Wave93 executions have terminal full recovery:
CI39084560317 attempt1 failed at the complete synthetic image fixture;
functional smoke39084562837 attempt1 succeeded, including publication,
idempotence and all ten cleanup outcomes. The [prior report](2026-10-18-r2-image-mode-ci.md)
preserves nine full log hashes, eight actual checkouts and the failed
candidate's 106 required PASS /12 NOT_EXECUTED and six image markers PASS /
three UNKNOWN. Old source, frozen inputs and terminal evidence are preserved.

## Static finding and bounded change

The fake producer in `scripts/ops/build-production-images-self-test.sh`
wrote `features: []` for every recipe. The committed D4 recipe requires
`market-stream-provisioning` for both `owner-equity-v2-runner` and
`kis-market-stream-state`. The real producer validator compares that exact
feature set and rejects an empty receipt. The next negative case after the
last printed Wave93 marker builds through D4 before reaching paper-scheduler.
This is a confirmed static fixture incompatibility. The old log does not
print the failed assertion, so it does not prove that this was the only or
first runtime failure.

Only the self-test changes. Its fake producer now models the D4 feature
independently and rejects inconsistent requests. Two positive receipt checks
and four negative cases cover both binaries with missing and unexpected
features. Negative copies have a recomputed COMPLETE digest, so rejection
must come from the real feature validator. Original producer/bundle bytes
are preserved. Each case emits its binary and mutation in a finite marker.
An ERR trap emits only the pinned source line and exit code when a fixture
assertion fails; it does not print commands or captured output.

The real image validator, production build paths, feature selection, byte/
mode/no-clobber/resource gates and runtime source are unchanged. The D4
provisioning feature is already required by the product contract; this
fixture correction grants no runtime or provider permission. No structural
change requires a diagram update.

## Verification and continuation

Coordinator reviewed the complete single-file delta against D4's recipe,
both producer bins, the real feature/producer validators and the prior
fixture's execution order. Whitespace inspection passed. Six new named
outcomes are written and unexecuted. No local project test, compiler,
parser, helper, database or Docker command was run; no independent review
is claimed. New exact-candidate hosted verification is required, retaining
all118 prior definitions, nine image markers, carried groups and exclusions.

Freeze the successor source and inventories, verify the remote SHA and
absence of all-event/all-workflow exact-head runs, then dispatch one new
CI and one separate functional smoke. Recover all terminal logs, actual
checkouts, named outcomes and cleanup before acceptance or another correction.
Never rerun or duplicate the old executions.

A third production interruption remains unauthorized. Actual twelve
verified images, approved-base host whole flow, G4 deployment, Owner use,
three consecutive trading days and final C_R2 remain open. Continue R2
tracking after this package; native Ralph remains blocked.
