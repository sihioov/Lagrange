# R2 Wave71 I7-C04 field-specific date isolation

Roadmap R2-04/09. Baseline f1f55ac after full CI38829047490 recovery.
One owned browser-test file, direct coordinator review (not independent).
The current fixture gives the second config no session/coverage/result/
publication/due date but a valid shared observation timestamp. The existing
whole-panel negative date assertion therefore contradicts legitimate output.

The corrected assertion requires exactly Not reported in each of those five
definition-list values, and separately requires Oct15/17:01KST observation.
Each value is selected from its exact visible term and sibling dd. A missing,
duplicate or wrong value fails; this cannot pass through an empty locator.
The state must still be OFF, preference false, and no prior-config report.
Real delayed response, exact selected control/value, native navigation,
late fulfillment or observed cancellation, all seven five-second phases and
the overall30second limit are unchanged. Fixture/application/runner unchanged.

All18UI,8newbrowser/11Owner,34supervisor names are retained; other10Owner
bodies are byte-identical. All129OFF/14stream and carriedRust/PG gates remain.
Exact diff/name/line/whitespace inspection only; no local execution.
One exact-candidate hosted CI is required before I7 source/test acceptance.
Full-document navigation does not prove every in-place React interleaving.
I6/approved-base threeUID/integratedreview/G4/Owner/3tradingdays/C_R2 open.

| File | Lines | SHA256 |
| --- | ---: | --- |
| apps/web/tests/e2e/owner-beta-recommendations.spec.ts | 441 | 4801f481be2594d985248ac7a457df33db9dcdc3858c11d90ad72a9d40c64f31 |
