# R2 Wave66 I5-C03 canonical result-name recovery

Roadmap R2-04/09; baseline abfca9c after full CI38815899362 recovery.
Direct coordinator source review; independent review and executable acceptance
are not claimed. This change touches one line in the PG validation harness.

The canonical replay regexp accepts a lowercase letter or underscore first,
followed by lowercase letters, digits or underscores. The previous [a-z_]+
discarded two actual passing test names containing503. Literal test prefix,
separator, ok/FAILED suffix and line anchors are unchanged.

All15 expected API names, unique successful-row checks, total row cardinality,
Cargo exit0 and noSKIP guards are byte-identical to the failed candidate.
Other target lists and test selections are unchanged. Rust and Web sources,
test definitions, assertions, compiler/lint flags, database roles and workflow
remain unchanged. The full baseline comparison proves exactly this replacement.

No additional low-value mirror test was added: the existing hosted PG target
executes the exact two digit-bearing names alongside the13 others and requires
all15 once. That existing guard must pass on a fresh exact candidate. All25Rust
and6Web definitions remain unexecuted for the new candidate until its own CI.
Whitespace check passes; no local shell helper, parser, compiler, tests, SQL,
DB, network provider or runtime execution occurred.

Previous failed run and actual passing test evidence remain immutable. I7/I6,
independent integrated review, real approved-base threeUID execution, G4,
Owner use, three trading days and final C_R2 remain open.

| Changed path | Lines | SHA256 |
| --- | ---: | --- |
| `deploy/db/integration-validation/validate.sh` | 2028 | `3455642dcae9873794646fabfda719d97c8b4914349bac2f895623dd590c2e5d` |
