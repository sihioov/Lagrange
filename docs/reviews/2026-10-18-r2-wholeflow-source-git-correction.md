# R2 wholeflow source Git correction

R2-04/R2-09, Wave97. Source disposition only; hosted validation is pending.
This change follows complete recovery of the authorized first Wave96 integration
attempt. It does not replace the accepted candidate2a165fe source or image evidence.

## Actual first integration attempt

Owner said “승인하니 이어서 진행해”, authorizing the prepared bounded packet once.
The authorization receipt SHA is242321af9552f8d76696db50740dfe1cd488f170ca8d71196e1b45514c8709b9.
Fresh2132source/11critical pins,12images/manifest,localPGdigest,exact11production
states and resource gates were reconciled. The single read-only base mount bound
`/var/lib/lagrange/data/artifacts/historical-price-beta-root`; no guessed path.
One systemd dispatch occurred2026-10-18T04:39:37.138655UTC, launch exit0.
The driver exited1 at04:39:37.657390UTC with `WHOLEFLOW_UNCLASSIFIED` and no
functional phase. There was no retry or accepted host integration.

The actual unit is failed/exit-code, MainPID0/ControlPID0, no cgroup or driver
process. All11production identities, start/finish times, health/restart/OOM states
and old selected releasefce9b90 remained unchanged. Exact namespace inventories
show no private containers/networks/stages; the successful epoch-bounded Docker
events query has zero nonce events. The precise attempt kernel range returned
zero rows/OOM observations; this is not an all-history proof. The driver never
reached private-stage creation, so its receipt contains no cleanup-success field.

Closure: `target/r2-daily-beta/wp-8/wholeflow-2a165fe-a1/attempt-closure.json`,
SHAabac9cfad626d6aecf46fc11d99997fc69f342a47599b5bc7c67d99e1d49ac39,
14receipt pins. Raw log bytes stayed in memory. The original coordinator process
substring self-match and rejected/mis-zoned time queries remain recorded with
separate successful bounded corrections; they are not successful first readings.

## Reproduced boundary and scoped correction

The driver uses a minimal environment then invokes Git as root in an
operator-owned checkout. The exact root/ENV `git -C <checkout> rev-parse HEAD`
read returned128 and the finite “dubious ownership” diagnostic. The failed driver
hid its original exception as `WHOLEFLOW_UNCLASSIFIED`; this reproduction plus
source ordering identifies a concrete early blocker, not a claim that later
unexecuted phases would have passed.

The official image builder already uses command-scoped
`git -c safe.directory=<exact checkout> -C <exact checkout>`.
The driver now uses that same exact-path binding for revision and committed-byte
reads. It changes no system/global Git configuration, repository ownership,
approval registry, candidate match, six protected-byte comparisons, product
validation or runtime/cleanup guard. Git errors become only
`WHOLEFLOW_SOURCE_GIT_FAILED`, without stderr, paths or exception text.

Four real Git fixtures create private disposable repositories and change their
ownership to UID10001. Root must observe the ordinary Git ownership rejection,
succeed only through the exact invocation, still reject ordinary reads afterward,
and leave configuration and ownership unchanged. Additional cases reject the
wrong candidate, each modified protected file and malformed Git state with a
finite diagnostic. The existing19wholeflow fixture tests remain; the CI step
now invokes the suite as root, required even after another failure. Existing CI
contract assertions bind that command. No fixture may skip the ownership cases.

Changed executable/QA scope is four files: the driver, its fixture suite, the CI
workflow and existing CI contract assertion. Coordinator reviewed the source
only. No new independent review or local compiler/test/parser/helper execution
is claimed. Hosted validation must freshly recover122required definitions
(118carried+4ownership cases), all15image-suite outcomes, all prior named groups,
strict format/Clippy/PG/Web/workspace evidence and full functional smoke cleanup.

The failed unit/result and accepted2a165fe checkout/images remain intact. The
single host approval is consumed. After hosted recovery, prepare the new exact
candidate image/host dependencies and concrete next execution scope; no host
retry, production deployment/migration, service interruption, new provider or
trading follows automatically. Owner use/three trading days/finalC_R2 remain open.
