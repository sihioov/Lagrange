# R4 approved artifact location and hash verification

2026-10-24, R4-02/03, source baseline `2c4cad2cbaac223d8ed2bf7d83bc3a96b188784c`.
The tested simulation source remains `6ada50c8bf62250dc8f3c0be98f9735c944fec75`.

The Owner directed the coordinator to recover the existing artifact location rather
than request it from the Owner. This removes the path-information wait. It authorizes
bounded discovery of existing records and local files; it does not create a provider,
data-copy, mount, deployment, or expired privilege-grant allowance.

## Recovered location

The recorded artifact root is
`/var/lib/lagrange/data/artifacts/historical-price-beta-root`.

- `docs/superpowers/plans/2026-10-14-r2-daily-input-implementation.md:8206`
  records resolution from the unchanged protected environment, the approved V3
  entry, and manifest/bars sizes of 2,189 and 7,672,914 bytes.
- `docs/reviews/2026-10-18-r2-wholeflow-source-git-correction.md:12`
  independently records this exact root as the historical read-only base mount.
- `deploy/compose/compose.yml:1091` maps the configured artifact directory's
  `historical-price-beta-root` to `/data/owner-beta-artifacts:ro`.
- `crates/market-data/src/historical_price_only_v3_artifact.rs:1247`
  derives the child directory from the approved candidate hash without `sha256:`.

The expected child is therefore
`b0fb82f6a580f3def13a1e1e34bea68e30d95dc720306e7ee54b6c3199cf402d`,
containing `manifest.json` and `bars.ndjson`. The API receives the root above,
not the child directory. Current existence and hashes were subsequently verified
through the existing read-only mount described below.

## Current access evidence

The discovery worker's bounded local `stat` and directory listing received
permission denied. Coordinator inspection confirms `/var/lib/lagrange/data` is
`root:lagrange`, mode `0750`; the current UID/GID is `1000:1000` and its group list
does not contain `lagrange`.

A single `sudo -n /usr/bin/python3` invocation, whose body only opens the fixed
manifest/bars read-only and prints metadata/hash comparisons, exited 1 with
`sudo: interactive authentication is required`. The Python body did not execute.
No artifact bytes were read, changed, copied, or printed by this attempt.

`sudo -n -l` succeeded. The readable general gateways have expiry times
2026-10-12T03:50:47Z and 2026-09-14T01:36:06Z, both before this observation.
They were inspected, not invoked or extended. Historical KIS maintenance
allowances were not used for this R4 task. No file permissions, group memberships,
containers, mounts, services, or production configuration were changed.

## Existing read-only mount verification

The existing API container exposes `/var/lib/lagrange/data/artifacts` at
`/data/artifacts`, with `RW=false`, running as its existing UID/GID `10001:10001`.
The coordinator used that established read path to run bounded BusyBox metadata,
hash, row-count and manifest reads as the existing service user. No root override,
new mount, new container, installation or artifact copy was used. The manifest was
parsed in memory and was not persisted; bars were hashed and counted inside the
container, without printing or transferring their contents.

At 2026-10-24T08:14:49Z the following checks passed:

- Candidate, artifact-manifest, Stage5 and action pins match the checked-in V3
  registry. Re-encoding the unsigned manifest reproduces its approved manifest
  hash, and re-encoding the complete manifest reproduces its exact bytes.
- Registry SHA-256 is `5d3aa2b354d8c0c51d0d7d029e9fd3f92e0570fe074262bfc900829a5a2bb707`.
- Manifest file SHA-256 is `254a9ec22783d5955bbe69034144bb7a82d1278ff7e0d579453981aae0530cc7`,
  length 2,189 bytes. This file hash differs from the embedded unsigned-manifest pin.
- Bars SHA-256 is `23354c54708d7a458aadc4f2cb2fca77469969e91256b6d3b119101ebae98ffe`,
  length 7,672,914 bytes, 26,972 rows. The manifest records 2,452 sessions and the
  required OWNER_ONLY/PRICE_RETURN_ONLY/vendor_snapshot/strict_pit labels.
- Root mode is `0750`, candidate child `0700`, both files `0600`; ownership is
  `10001:10001` and files are regular with one link. The observed metadata was
  unchanged before/after the reads.
- Container identity, image, start time, restart count and mount were unchanged.
  Its health was already `unhealthy` before the reads and remained so; this is
  not a new health acceptance claim or a diagnosis. No service repair was attempted.

Private evidence:
`target/r4-first-simulation/u2-discovery/coordinator-readonly-verification.json`.
The host-user access failure above remains part of the history, but it no longer
blocks location or hash verification. The full sealed Rust loader has not run in
this check.

## Gate disposition

- Artifact path information, current file presence/metadata, approved manifest
  pins and bars hash: **VERIFIED** against records, registry and existing files.
- Full sealed Rust approval and bar semantics: **NOT_RUN** in this check.
- U2 actual artifact execution, U3 executable provenance, actual result hashes
  and their repeatability: **NOT_RUN**.
- Existing same-SHA CI/smoke acceptance remains valid for its source/isolated
  scope; R4-02/03 final actual-input acceptance remains open.

Next action is an isolated executable built from the final source, with its actual
binary/toolchain/target/profile provenance, followed by the existing sealed approval
function and simulation repeatability verification. The running API image predates
the simulation source and is not that executable. The path does not need to be
supplied again. No new workflow dispatch was made to locate or hash it.

The notified Paseo discovery worker `1fbd3c51-b74b-44f2-9467-70a177c6bafa`
finished idle with no pending permissions. Its private report is
`target/r4-first-simulation/u2-discovery/report.md`; the coordinator separately
checked the cited records and access outcomes. That worker did not perform U2/U3
runtime validation.
