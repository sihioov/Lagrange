# R2 Wave75 runtime boundaries: source review

Roadmap R2-04/09. Baseline570e277 after full CI38839505678 recovery.
Direct coordinator source review; not the independent integrated review.
Six owned paths; no existing source function or test definition was changed.

The inactive Compose overlay defines producer10002, admitter10003 and the
existing worker10001. Producer has network_mode none and no secret. Admission
receives only its dedicated restricted credential and the internal DB network.
Candidate/admitted/frontier/private-state read/write mounts follow the adopted
principal contract. Worker only receives read-only admitted inputs. Immutable
config/policy bind mounts reject host auto-creation. The two aliases share the
canonical worker image and add no image-build record.

The internal helper accepts an already verified manifest/config/project binding.
It does not load a manifest itself, start a service, open a DB or call a provider.
The future wrapper must validate the binding and hold the shared release lock.
Docker is fixed to /usr/bin/docker and the local Unix socket, with a clean
environment. Inspect output remains in memory and failures are fixed codes.
Ownership requires exact image/revision/project/service/oneoff identity. Strict
running acceptance additionally verifies UID/entrypoint/command, healthy state,
read-only root, capabilities, init/no-new-privileges, CPU/memory/log limits,
exact mounts, closed environment and only the permitted network identity.
Native host filesystem config/secret validation remains the caller's duty.

Capture binds container ID, creation time, start generation and restart count.
Drain rechecks the entire set before mutation, turns off restart policies before
signals, then sends SIGTERM before a single bounded wait. An update/generation/
signal failure still permits best-effort drain of other verified members but
never successful acceptance. Every captured and previously absent service is
relisted at the end. Paused/replaced/restarted/foreign/OOM/failed-exit or timed-out
members fail closed; there is no SIGKILL or restart reset in the runtime helper.
The overall ceiling is3670 seconds; individual control RPCs are capped at15.
Already absent/cleanly stopped states are distinguished from actual signals.

Nineteen injected-failure definitions cover binding, all three views, image and
project identity, command/privileges/mounts, environment/secrets, network identity,
health, signal ordering, changed generation, partial failure, idempotence,
paused/deadline failures and local-daemon/redacted/nonroot transport boundaries.
Five actual Docker definitions select the checked-in overlay and actual worker
stanza, substituting only a small fixture image and dependency launcher. They
check real UIDs, capabilities/no-new-privileges, no default routes, read-only and
cross-principal writes, real SIGTERM/exit/restart policy, foreign labels and
manual process restart invalidating a captured generation. Fixture teardown
targets only the unguessable project and verified fixture image.

The image is a trapped shell process based on the pinned Alpine runtime base.
It is not the Rust producer/admitter and does not prove approved-base computation
or DB/network connectivity to PostgreSQL. PyYAML is the already pinned CI
dependency. New test names must each be recovered once with ok; an aggregate
success or skipped step is insufficient. Existing root34, Rust/PG/Web gates stay.

Local checks were source, diff, hash/name inventories and whitespace only.
No local project Python import, parser/checker, test, compiler, Docker/DB,
provisioner, provider, production or workflow execution. All24 new definitions
are WRITTEN_UNEXECUTED. This package adds no supported activation CLI: I6B2c must
connect the sole release wrapper, explicit API capacity, fixed DB transport,
DB disable plus actual drain, preparation and rollback. Consistent backup/
reconciliation, diagrams, approved-base whole flow, independent integrated review,
G4, Owner/three actual trading days and final C_R2 remain open.

| File | Lines | SHA256 |
| --- | ---: | --- |
| scripts/ops/lib/owner-beta-daily-runtime.py | 364 | 29e50b22f696aafa7638cc97741b04b2f24c31d47547437296a4b4b66218a6b7 |
| scripts/ops/owner-beta-daily-runtime-self-test.py | 356 | d4b7a9c7af26ae8108a5164fc3defca3e5d8d8e5d167222d8d262c860fe76f33 |
| scripts/ops/owner-beta-daily-docker-self-test.py | 292 | ca66e005a7d34100b1cc31f20b6074e1519128d681efc5d777875ca601199b4d |
| deploy/compose/compose.owner-beta-daily.yml | 228 | cac8415d7ad5e65e9055bce6994c9ebab675d9dc18c922049c7601776da664b9 |
| .github/workflows/ci.yml | 246 | 253a18c1fc1990d72a15a7e712078fb8686369bf1e0bcb53bf230267630ccc75 |
| docs/runbooks/owner-beta-daily-input.md | 198 | d5199599cfa82dee0eb632b6401fd102acf1f3ad8630ef3c2a16e685f2152cb9 |
