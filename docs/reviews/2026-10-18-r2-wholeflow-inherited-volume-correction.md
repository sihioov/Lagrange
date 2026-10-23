# R2 wholeflow inherited-volume contract correction

R2-04/R2-09, Wave101. Source written, unexecuted; coordinator static review only.
No new independent review or local project test/parser/helper/compiler execution.

## Observed outcome and limits

The [Wave100 approved sequence](2026-10-18-r2-0120b0e-host-sequence-packet.md)
completed its12-image build and restoration, then its single isolated wholeflow
run failed WHOLEFLOW_MOUNTS before bootstrap started. Both approvals are consumed.
Image closure SHA256 ff2471813c6ea56cac4d6ddfc77287e6346d56c0fd7318824fd0fbc40ecd7294;
wholeflow closure SHA256 861396afb7f86e11292c225ba18ed6a12fa3538e745682879c531f67ab7dd133.
No production identity/state or selected release changed.

Exact image metadata shows the fixed PostgreSQL base and both DB client images
inherit `Config.Volumes={"/var/lib/postgresql":{}}`. The real PostgreSQL container
explicitly bound this destination. Bootstrap, migrate and control bound secrets
but did not cover the inherited destination. Docker's anonymous persistent volume
conflicts with the existing bind/tmpfs-only guard. Runtime creation timing and
the failure code agree with this static contract gap; the deleted bootstrap's
complete rejected Mounts array was not retained, so it is not direct recovered
proof of that array or proof that later phases would pass.

Anonymous volume `159b052fed63a476fba2c51c8f15f532a837096e4bdd453bb753789672034f07`
remains, unreferenced and without the wholeflow nonce label. It was created next
to the bootstrap creation event, but exact ownership is not independently proven.
Do not delete/prune it or claim complete cleanup. Preserve failed unit/result and
private mount-contract-readback/anonymous-volume-disposition receipts. Current
labelled-container/network and staging absence does not prove volume absence.

## Coherent correction

The driver assigns `/var/lib/postgresql` a1MiB read-only/noexec/nodev/nosuid tmpfs
only for the three exact DB-client service/image/UID/entrypoint combinations.
PostgreSQL itself retains its exact private writable bind. Before recording an
intent or creating a container, every declared image volume must be covered by
an explicit bind or tmpfs. Unknown destinations fail before daemon mutation.
After creation, exact bind identities/modes, exact tmpfs options and the existing
ban on volume mounts remain mandatory, alongside all resource/security guards.

Cleanup refuses an unexpected persistent volume before deleting its associated
nonce-labelled container, preserving the intent/stage and ownership witness for
recovery instead of losing the association and claiming success. There is no
arbitrary-volume allowance, host permission change or data deletion.

Four mocked boundary regressions cover DB-client identity, pre-create rejection,
preserved cleanup witness and unchanged bind/tmpfs/no-volume rejection. Five new
hosted actual-Docker cases create and inspect each DB client, PostgreSQL's private
bind, and a derived image with an unexpected VOLUME. They never start containers,
read an approved base, run a DB process or contact providers. They require root
and the fixed cached PostgreSQL image; missing prerequisites fail, never skip.
They verify no new persistent volumes and exact owned-container cleanup.
A new workflow contract makes this actual-Docker suite a required policy step
with `!cancelled()` and no continue-on-error.

## Acceptance and next work

Carry all131 required definitions,15 image outcomes,23 wholeflow definitions,
Rust/strict Clippy/format/PostgreSQL/Web groups and functional publication/repeat/
10-cleanup outcomes. Add10 required definitions (4 mocked,5 actual Docker,
1 workflow contract), yielding141 required and32 wholeflow definitions. Freeze
full inherited1204 pins/523 committed blobs plus changed/new evidence, current
line references and one coherent candidate before exactly one CI and one
functional smoke after remote/exact-head absence checks. Prior passing results
are historical and cannot accept the successor. No rerun/cancel/duplicate.
Raw hosted logs stay in memory; only finite metadata/named outcomes/hashes persist.

After full terminal evidence, prepare the next exact image/wholeflow packet and
explicit retained-volume disposition for a new Owner decision. No new host probes,
SQL, lifecycle, local project execution or retry is authorized now. G4 consistent
pre-upgrade recovery/backup transition, deployment, Owner use,3 consecutive trading
days and final C_R2 remain open. Same-agent heartbeat04318894 is bounded tracking,
not a worker or outage protection; renew by20:30UTC and before47th tick.
