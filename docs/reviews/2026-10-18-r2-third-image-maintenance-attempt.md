# R2 third image-maintenance execution

R2-04/09, Wave95. Candidate `2a165fe577e833529134627e980a129b9717ae6f`.
Status at2026-10-18T03:21:43UTC: **BUILD_ACTIVE; RECOVERY_ACTIVE**.
Owner answered “진행해” to the [prepared single-attempt packet](2026-10-18-r2-third-image-maintenance-packet.md).
Exactly one dispatch occurred at03:21:22.119955UTC; systemd-run returned0.
The existing Owner V2 container exited normally with0 and work settled before
the official builder started. The first two DB images completed; API was in
progress. Full image acceptance and same-container restoration are pending.

## Admission and identity

All2132 detached source file/symlink bytes, Git modes/blobs, eleven pinned
controls and inert0600env matched before execution and after plan/preflight.
The clean source is `/data/worktrees/3puw275b/r2-release-2a165fe`.
The oldab48 source, previous failed units/controls/image and hosted evidence
are preserved. No product source, guard, CI run or operational release changed.

Fresh bounded admission at03:19:50UTC observed rootUID0, safe protected parents,
absent new output/state/root-control paths and new units, exact eleven project
containers, ten healthy/restart0 and research exited2/restart7935. Docker and
containerd were active. There were no compiler processes, running Compose
one-offs or active producer execution. Historical active/exited build services
had no main/control/cgroup processes; they were preserved without reset/stop.
MemAvailable7582580KiB, SwapFree4005040KiB, memory PSI some/full avg10 zero.
The kernel journal was readable; the preceding30-minute range had zero rows
and zero OOM observations. This is bounded evidence, not all-history proof.

The reviewed runtime and READ ONLY/repeatable-read aggregate probe passed.
Jobs/attempts were settled with operator-wide visibility. Official plan
reported exactly twelve services; official preflight returned its PASS marker.
The freshly observed source-input SHA256 was
`454cad1f9cb8f12ddafca6729897acc9ef2eccd5947adf40942020b1e9a1cf76`.
Its equality with an earlier value was observed, not assumed or substituted.

## Bounded execution and restoration

New root controls are under
`/etc/lagrange/release-manifests/.r2-maintenance-2a165fe-a3-20261018`.
Build unit: `lagrange-r2-build-2a165fe-a3.service`.
Independent unit: `lagrange-r2-recovery-2a165fe-a3.service`.
The independent unit was actually active/running with a startup receipt and
no stop intent before the build dispatch. The shared window was created once.

| Boundary | UTC deadline |
| --- | --- |
| Shared start |2026-10-18 03:21:04.258525 |
| Latest permitted stop start |2026-10-18 04:21:04.258525 |
| Build runtime |10hours, enforced by systemd |
| Independent watchdog build stop |2026-10-18 13:51:04.258525 |
| Drained-reader attestation expiry |2026-10-18 14:21:04.258525 |
| Hard maintenance/recovery end |2026-10-18 15:21:04.258525 |

Normal SIGTERM/exit0 and post-stop work/stream settlement passed. Candidate-bound
attestation was created; research remained stopped and nine other serving
containers retained their required state. No forced-kill fallback was used.
Build policy is Nice10/idle I/O/CPU and I/O weights20, Compose parallel1/Cargo
jobs2, one service per invocation and four batches of three. Official resource,
health, OOM and prior-process termination gates remain enabled.

The original Owner identity required for restoration is container
`f293c7be42fb02f5b20f0bbcf1d7b6568518fdcb52b6c47c20deaba3f73a6af4`, image
`sha256:32bc37e1bffda89baefea466c0ba127c89b0cca69401c10ccf4f984c76cf6c50`.
ExecStopPost and the independent supervisor must prove its healthy restoration
after build/compiler termination. A missing unit alone is not terminal proof.
Recovery includes the independent completion receipt, actual successful exit,
eleven required container states and post-restoration READ ONLY settlement.

At the initial observation, actual saved-byte verification receipts existed for
`db-role-bootstrap` and `db-migrate`, each bound to the exact candidate and
linux/amd64. Their image IDs were respectively
`sha256:ae362c8a9c43fab3bc16f37811005f08a8fbc84e9f9571dff4792411b62a217c`
and `sha256:cbf02569cc2a657636b10b1908712fb5ad6da97f2c2938bb3da39bae5fb6b8af`.
The full twelve-image set, manifest, final gates and restoration are not yet
accepted. No retry or fourth interruption follows automatically from this approval.

## Durable evidence and continuation

Private receipt directory:
`target/r2-daily-beta/wp-8/maintenance-2a165fe-a3/`.

| Receipt | SHA256 |
| --- | --- |
| owner-authorization.json |83b09762c4f5a93c83e3c366d6824922266e103a5b673c58dcbbef19744c9147 |
| authorized-static-reconciliation.json |0b68d99a85615554eab5a83450af9f4d7b7ac8b7d55e7769ea5ca3e22a432f7b |
| authorized-host-admission.json |b01b00f3e569c2566c9ba9ed49ca6890e150233937b0f82924d8fb6cea4215ff |
| authorized-prelaunch-settlement.json |64ecf8e27890e05ee703fd0004a24f5e2e06a6cdf906febfcb0f58c9533056a6 |
| control-installation.json |974e580e5e5ef74c1caa8d1ce42a85f62ffda5d28e8e4e3c72269958dc6204f3 |
| recovery-launch.json |039814a8c4cb67f59fa4d77f6636b8f060bd5758a962c38a99b7623edb0136f6 |

`official-prelaunch-evidence.json` records exact plan services/preflight marker;
the earlier generic output extraction did not match their actual grammar.
`recovery-ready.json`, `build-dispatch-intent.json`, `build-launch.json` and
`initial-build-observation.json` preserve the exact ordering and observations.
Raw retrieved command/journal bytes stayed in memory; coordinator receipts
contain sanitized metadata, aggregate results and complete byte hashes.

Heartbeat276c42d8 was confirmed active03:22:24UTC; its handoff was persisted
before cad91dfe deletion. Renew by14:00UTC, before expiry15:22:24.009UTC and
the47th tick. It continues this same coordinator; no additional worker exists.
The root recovery service is independent of this same-daemon heartbeat.
Native Ralph remains blocked; no active/completed goal claim is made.

Hosted acceptance remains limited to the complete CI/smoke evidence recorded
in [Wave94](2026-10-18-r2-d4-fixture-ci.md). After image/recovery closure, continue
approved-base host wholeflow, G4 deployment, Owner use, three consecutive trading
days and finalC_R2. Image permission does not authorize those production changes.


Follow-up03:25:25UTC: both units remain active/running; two image verification
receipts are present and the latest official gate is native:D1 PASS. API
compilation has one Cargo/two rustc processes, matching Cargo jobs2. Nine other
serving containers are healthy; Owner remains stopped0/research stopped2.
MemAvailable8065152KiB, SwapFree2253580KiB, PSI some/full avg10 zero. This is
in-progress evidence, not terminal or restoration acceptance. The bounded
continuation owns recovery of all remaining images and same-container health.


## B1 complete and B2 in progress — 03:45:36UTC

Four exact-candidate saved-byte verification receipts now exist: the two DB
images, API and web. API completed in947seconds; web in26seconds. Seven official
gates passed with previous_exit0, including producer:D1:api-server and batch:B1.
The current service is research-worker and its native:D6 gate passed before
compilation. This is image construction; the production research container
remains stopped2. The remaining eight image verifications and final manifest
are pending, with no terminal or restoration acceptance yet.

New verified image identities: API
`sha256:d0777f5f96bebdc5cc45e0a24361449fe256cb99ba621cf2e8bdbaae0e6bb626`,
web `sha256:f550e332e684f2e2ebc5bf5a61065d037125f6a0bb17ec9fa981fef7b9cb3fb8`.
Their saved archive SHA256 values are respectively
`81a4fe4b3cfc5c65deb87e7485fc6aebada8dd8c225644969282001458c56d4d`
and `402fda315fd049699a662c40311746045a5f5bbf569ef6df1b8a4bbed39189d0`.

Both build/recovery units are actually active/running with their expected
main PIDs and nonempty process groups. Nine other serving containers remain
healthy; original Owner is stopped0. One Cargo/two rustc processes are active.
MemAvailable10021800KiB, SwapFree1433672KiB and both PSI avg10 values0.
Kernel scan03:19:50–03:45:36UTC recovered182rows and zero OOM observations.

Private observation `build-observation-20261018T034536Z.json` SHA256
`a238acf1841173fd560ed72c1dc56fba555053eac2f87e448a57434919af29d8`.
Complete retrieved build journal SHA256
`dbd56f9e581ffaa096a22968cfbbfc6828cd768b16cc0664b3f220e853f79fc8`,
31581bytes/396rows; raw bytes were not persisted by the coordinator.
Same heartbeat276c42d8 and original12-hour bounds continue. No new launch,
stop, retry, source change, test or production action was performed by this tick.


## Research image complete; recommendation image in progress — 04:00:49UTC

Five exact-candidate image verification receipts now exist. Research-worker
completed in699seconds with23 file checks; its actual image ID is
`sha256:180192ae2ad795e6c770bf55bac469b1ac1971d6df4756a5cff34c716b9ae18f`,
archive SHA256`6ab612bc7454befa942643c65b32e4eec474d3f74a530c5f4bc420ae2e0451fc`,
verification receipt SHA256`61dfdaa909f235bccf21a7e215a139b8f58be70799d03e52ec8c056056ad3d83`.
All24 official gates passed with previous_exit0, including ten D6 producer
checks and collector checkpoints3/6/8/10. Recommendation-runner image is in
progress; its D2 producer gate passed, but image completion is not yet recorded.
These are build results; no research container or provider operation was started.

Build and independent recovery remain active/running. Nine serving containers
are healthy; original Owner is stopped0/research stopped2. One Cargo/one rustc
process is active. MemAvailable10020728KiB, SwapFree1437748KiB, PSI avg10 both0;
kernel scan since03:19:50UTC returned364rows and zero OOM observations.
Seven image verifications, final manifest and actual restoration/independent
terminal acceptance remain open under the original bounds and heartbeat.

Observation`build-observation-20261018T040049Z.json` SHA256
`d6ee311ac58bb48746bf43bb37d596a409685e1763b14b531a5bbd6e13c57035`.
Complete retrieved build journal39506bytes/500rows SHA256
`0fc10fb4f785cd7a8e0d26a99bed838abb77ce982e5beff9a3345c42bfb24147`.
Raw bytes stayed in memory; no additional dispatch, lifecycle or source change.
