# R2 Wave73 immutable daily installation binding: source review

Roadmap R2-04/09. Baseline d462ea8 after complete CI38834438912 recovery.
Direct coordinator review, not an independent integrated review. Ten owned
paths connect the existing installer and protected-input validator to the
daily foundation. Actual Compose services and runtime/database operations
are unchanged; this package does not establish activation readiness.

The shared shell library accepts only disabled/owner_only, absent means OFF,
and requires exact protected-file/shell agreement. ON requires owner_only
access, sealed_v1 prices and disabled Paper. Internal container path/hash
and file-indirection inputs are rejected. The release validator permits ON
only in release scope. Mode values are never evaluated as shell code.

Apply consumes a separately trusted root0600 config only through an explicit
--daily-config-source; ON requires it, OFF may prepare it. Stage follows archive
ownership normalization. Installed root:10001/0440 bytes are verified against
the exact commit and canonical policy before and after publication and on
check/rollback. Wrong/unsafe inputs cannot change current. Check/rollback never
accept replacement external config. All existing archive, trusted manifest,
clean HEAD and atomic current-link predicates remain.

Apply/rollback share a root-owned, empty, single-link release lock. The opened
descriptor and named path must match before/after nonblocking flock. Release
existence is checked again after acquiring it. The lock remains held through
publication and current-link selection. Check creates neither lock nor floor.
Root-compromised replacement and real power-loss behavior are not established.

Reader-check validates without mutation; reader-guard reserves compatibility
immediately before current changes. Once present, the persistent floor rejects
old-reader targets even when OFF. Compatible OFF selection preserves the floor
and history. Failed later transitions retain the conservative floor. Current
selection alone does not stop an old process or modify the daily DB control.

The canonical twelve image/build records are byte-identical. Optional override
generation adds only two fixed aliases pointing to the same owner-beta-runner
image ID; invalid mode fails before the output file is written. Actual aliases,
command/UID/mount/network verification and runtime drain remain I6B2.

Twelve hosted real-root integration definitions use isolated temporary Git/
release roots. They cover OFF/ON/config staging, invalid commit/mode,
compatible/old-reader rollback, read-only check, actual held-lock rejection,
protected mode inputs and canonical alias IDs. The existing16 filesystem/UID
and3 Rust parity definitions remain unchanged. The legacy full production-ops
fixture is newly required in hosted CI; its fake Docker/system commands do
not prove runtime isolation. Copy-site inspection found only its real installer
fixture needs the new shell library; other copied validators are static inputs
or stubs, and direct validator tests use the repository root.

Local verification: source/diff/definition/pin inspection and whitespace only.
No Python project import/execution, shell checker/test, Cargo/compiler, root,
Docker/DB/provider or production action. No source/executable acceptance yet.
One exact-candidate hosted CI must pass before bounded source acceptance.

I6B2 still requires actual Compose transition, DB control and graceful drain,
coordinated daily filesystem/policy/DB backup and restore reconciliation and
evidence-bound architecture diagrams. Actual approved-base three-UID first/
continuation flow, integrated independent review, G4, Owner use, three actual
trading days and final C_R2 remain open. No new delegation is authorized.

| File | Lines | SHA256 |
| --- | ---: | --- |
| scripts/ops/lib/owner-beta-daily-release.sh | 44 | 84832b20c1a09f680aa2fa64825ac0e1501e9115753a66fd91ab3a1b400b536c |
| scripts/ops/lib/owner-beta-daily-release.py | 598 | 34872cc48c780974ab9ea8ebed3b6eae15b7c963c78837599eada465d7b523fc |
| scripts/ops/deploy-production-release.sh | 439 | 48f2d79b75642de4406ce8332b1a16158b209f310be50d86cd17c46089bbc911 |
| scripts/ops/validate-production-config.sh | 899 | 23774e5d98f6a839480e3266482f9c33aea125266c5cd0e627f4ae98fdea42f2 |
| scripts/ops/lib/release-image-manifest.sh | 374 | d4695660ec05df6bdef4e2dde08b0e8a3436104bafa8649c23361c076d613973 |
| scripts/ops/owner-beta-daily-integration-self-test.py | 304 | e0a9a8a9e2b5e1f4b225aea53e18d47da3e31a2385ffb1143f309d91899423af |
| scripts/ops/production-ops-self-test.sh | 1238 | 9af3577fea3565ff3dac282cf5a4b5f654db4743711888791a4f070fb2b3669f |
| scripts/ops/production-ops-static-check.sh | 394 | 663d92b6b43e792c638926ee819f10e89307f05d2500aa82d2e7ec46e90eeb80 |
| .github/workflows/ci.yml | 240 | 4428ff2eb31967d3f6b1eb44372fff79b55a207a4a29e1490c77dcaa9c0ead75 |
| docs/runbooks/owner-beta-daily-input.md | 127 | 89c2f91c2f3ed47e4095b486b822aaf6dbb72e120ca0d57db89028407ece0c3b |
