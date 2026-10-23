# R2 Wave70 I7-C03 configuration selection and bounded diagnostics

Roadmap R2-04/09. Baseline f9a64db after complete CI38826611025 recovery.
Direct coordinator review, not independent review. Three owned test/diagnostic
files; application/schema/backend and the corrected FAILED fixture unchanged.

The page label wraps the select and both nonempty options. Playwright1.61.0
getElementLabels/elementText recursively include option text; exact prompt-only
matching therefore cannot select this element. The installed package versions
were compared to package-lock.json. The corrected test matches the prompt
prefix and requires control id=daily-config-id/name=daily_config_id before
selecting the second UUID and checking its value. No synthetic navigation or
direct state replacement is used. The old CI blocking line remains UNKNOWN;
this is a concrete static mismatch, not a recovered stack.

Seven fixed test.step phases have five-second bounds; the overall30second
limit stays unchanged. The real old-config response is fetched then held;
actual selection/navigation precede its release. Successful late fulfillment
or observed request cancellation is required. Disabled status, no old date,
false preference and no previous-config result remain asserted. Finally always
releases the gate and removes the route. Full-document navigation evidence
does not claim all in-place React scheduling interleavings.

The existing sanitized projection adds only fixed failed-step identifiers for
this allowlisted file and failed/timedOut/interrupted attempts. No error prose,
stack, body, arbitrary step name, nested payload or path prefix is persisted.
The two existing privacy regressions cover fixed IDs without errorLocation,
duplicate IDs, absent/nonobject errors, malformed rows, hostile names, nested
data, wrong files and successful attempts. This does not weaken result,
selection, retry/skip/flaky, process ownership or cleanup validation.

All18UI names/markers,11Owner cases(8new),34supervisor definitions are retained.
Other10Owner bodies and the first32supervisor bodies are byte-identical.
Runner execution behavior is unchanged outside the evidence projection.
Exact baseline/diff/name/line/whitespace review passed; changed tests are
unexecuted for this candidate. No local project parser/test/compiler/helper
execution. One fresh exact-candidate hosted CI remains required.
I6/approved-base threeUID/integrated review/G4/Owner/3tradingdays/finalC_R2 open.

| File | Lines | SHA256 |
| --- | ---: | --- |
| apps/web/tests/e2e/owner-beta-recommendations.spec.ts | 424 | 85769f1d2a67e743937c59929877a3a62623ac52c13d3d2b92de96664980871b |
| scripts/qa/candidate-web-e2e-runner.py | 2252 | 0de0d417ec607333a6ed47e9881fe68171da4879e3a8adcf3d86db8306d3a0a6 |
| scripts/qa/test_candidate_web_e2e_runner.py | 1895 | b888ff0fb02c44206ea809a6404914799837ebe7c207f23ca0130bed6cb83414 |
