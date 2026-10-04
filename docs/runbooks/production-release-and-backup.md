# Production release installation and encrypted backups

These workflows remain repository artifacts until an operator explicitly
applies them as root. Do not apply them until the repository is committed and
clean, `/opt/lagrange` capacity is confirmed, and backup sizing is approved.

The separate [KIS market-stream procedure](kis-market-stream-operations.md) covers the
opt-in initializer artifact, persistent domain, reviewed grant and WS/off refresh. Installing
a release does not authorize or perform those operations.

## Install an exact release

`build-production-images.sh` and `deploy-production-release.sh` refuse every
tracked change and every untracked path except the exact operator-supplied
`docs/kis_openapi_entiredocs_20260818_030007.xlsx`. `--commit` must equal
`git rev-parse HEAD`. Deployment uses `git archive`, so it never copies the
mutable worktree, `.git`, host Raw/Curated, caches, secrets, or the untracked
KIS XLSX.

This is a single-host, single-platform owner-beta release. Its V2 manifest
records Docker's exact local immutable `.Id` as `image_id` (`sha256:` plus 64
lowercase hex), not a `RepoDigest`, registry digest, or multi-architecture
claim. The manifest scope is exactly these twelve locally built serving services:

- `db-role-bootstrap`, `db-migrate`, `api-server`, `web`, `research-worker`
- `recommendation-runner`, `candidate-runner`, `owner-beta-runner`,
  `owner-equity-v2-runner`, `nt-backtest-worker-1`,
  `nt-backtest-worker-2`, `paper-scheduler`

Postgres and reverse-proxy are independently content-pinned upstream images and
are outside this local-image manifest. `research-range-raw` is an
operator-confirmed historical-capture profile, not an owner-beta serving
release service; use its dedicated Stage5 runbook. `live-node-owner` and the
`live` profile remain forbidden and excluded.

The protected Compose `.env` and the V2 manifest are separate root-owned
mode-0600 regular files. The manifest must also live below a root-owned,
non-group/other-writable, non-symlink path. The deployer validates that external
file, copies it once into release staging, and validates the installed copy
again before atomically changing `current`.

The protected env also carries the temporary beta policy. The safe/default
values are `OWNER_BETA_ACCESS_MODE=disabled` and
`OWNER_BETA_PAPER_MODE=disabled`. An owner-only release derives its artifact
identity exclusively from the canonical registry embedded in the verified image;
the host env supplies no candidate hash. Do not set Paper to `enabled`: no
trusted three-unattended-session evidence checker exists,
so production validation rejects it with `owner_beta_paper_evidence_unavailable`.

The image builder invokes Compose once per service in the canonical order; it
never submits the twelve services as one parallel Bake graph. Every production
Rust builder also fixes `CARGO_BUILD_JOBS=2`. These limits are required on the
14 GiB production host so concurrent release compilation cannot starve the
running control plane or serving containers.

### G2 common-artifact preparation, checkpoints, and resume

The selected C layout is `common`: it prepares one verified Rust artifact
bundle per D7 recipe and gives the relevant final-image build that immutable
bundle as a named context. It is not a parallel Rust-build authorization. The
checked-in `build-production-images.sh` sources
`scripts/ops/lib/release-build-layout.sh`; its run state is bound to the exact
clean source commit, helper, layout recipe/schema, validated cache namespace,
and gate inputs before the first producer action. The common compile-cache
key excludes the source commit so compatible outputs can survive source
changes; the run state and final images still require their exact commit.

The official root-only entrypoint owns root state. A separately authorized
user-level prebuild or benchmark may use the same narrow helper API with its
own `0700` state root and current effective UID, but that never bypasses the
official root check, does not publish an official release manifest, and is not
release acceptance. Do not hand-write a Compose override or invoke an arbitrary
Docker build to substitute for the helper transport.

For the checked-in helper API, the fixed setup is:

```bash
export LAGRANGE_CODE_COMMIT="$(git rev-parse HEAD)"
export RELEASE_BUILD_SYSTEMD_UNIT='<approved-background-build.service>'
export RELEASE_BUILD_SYSTEMD_MANAGER=system
export RELEASE_BUILD_HEALTH_UNITS='<approved-unit-1.service>,<approved-unit-2.service>'
export RELEASE_BUILD_HEALTH_CONTAINERS='lagrange-station-postgres-1,lagrange-station-reverse-proxy-1,lagrange-station-api-server-1,lagrange-station-web-1,lagrange-station-research-worker-1,lagrange-station-recommendation-runner-1,lagrange-station-candidate-runner-1,lagrange-station-owner-beta-runner-1,lagrange-station-owner-equity-v2-runner-1,lagrange-station-nt-backtest-worker-1-1,lagrange-station-nt-backtest-worker-2-1'
# Set RELEASE_BUILD_RESEARCH_EXCEPTION only to the exact separately approved,
# currently valid exception file owned by the build's effective UID.
# Set RELEASE_BUILD_DRAINED_READERS_ATTESTATION only when the exact,
# separately authorized image-build-only record below exists at this path.
# This task did not create or authorize such a production record.
# export RELEASE_BUILD_DRAINED_READERS_ATTESTATION='/absolute/canonical/path/record.json'
```

The health list is the complete monitored project inventory, not only the
currently serving set. It must contain exactly these eleven names, including
the mandatory research-worker membership: `postgres-1`, `reverse-proxy-1`,
`api-server-1`, `web-1`, `research-worker-1`, `recommendation-runner-1`,
`candidate-runner-1`, `owner-beta-runner-1`, `owner-equity-v2-runner-1`,
`nt-backtest-worker-1-1`, and `nt-backtest-worker-2-1`, each with the
`lagrange-station-` prefix shown in the export above. In the drained route,
the nine containers other than research-worker and owner-equity-v2-runner must
still satisfy the ordinary running, nonrestarting, non-OOM, healthy, exit-zero
predicate. The two named readers are intentionally stopped and are not
classified as healthy; they require the exact attestation contract below.
Without that contract, stopped readers fail the ordinary predicate. On the
current host the control units are `docker.service` and `containerd.service`;
the Paseo process is monitored separately, without inventing a system unit.

The optional `RELEASE_BUILD_DRAINED_READERS_ATTESTATION` is a short-lived,
operator-approved observation for one image-build attempt. The canonical
absolute path is a nonsymlink regular file owned by the build's effective UID
(root for the official route), mode `0600`, and is read with the same bounded
race checks as other private gate files. Its canonical compact JSON has exactly
these top-level keys and values: `format`
`lagrange-build-drained-readers-attestation-v1`, `scope` `image-build-only`,
`project` `lagrange-station`, the exact target `target_commit`,
`observed_at_utc`, `expires_at_utc`, and `containers`. Observation must be no
later than the read, expiry must still be future, and the lifetime from
observation to expiry is at most twelve hours; expiry is never implicitly
renewed. `containers` contains exactly two records in canonical order:
`lagrange-station-owner-equity-v2-runner-1` followed by
`lagrange-station-research-worker-1`. Each record has only the pinned
`container_id`, `container_name`, `image_id`, `health_status`, `exit_code`,
`restart_count`, `status`, `running`, `restarting`, `paused`, `dead`,
`oom_killed`, `started_at_utc`, and `finished_at_utc` fields. The inspected
Compose project and every pinned identity, image, health value, exit code,
restart count, status, and lifecycle timestamp must match exactly. Both
readers must be `status=exited`, all five boolean state fields false, with
research exit `2` and Owner V2 exit `0`; lifecycle timestamps must be ordered
and finish no later than the observation. No credential, entitlement, provider,
or log content belongs in the record.

The path, SHA-256, and complete content binding are recorded in `run.json`
before the first producer action and in persistent gate state. Adding,
removing, replacing, renewing, expiring, or changing the record after
initialization fails the gate. The drained record and the legacy running
research exception are mutually exclusive; the legacy exception remains a
running `PRICE_CURATION_FAILED` contract and must never be reused for a
planned stopped-reader drain. A drained pass is recorded as
`image-build-only-drained-readers`, never `healthy` or
`image-build-only-known-incident`. This observation does not authorize
starting, stopping, recreating, removing, or changing either reader, and it
does not authorize deployment or rollout.

The helper obtains the whole-run lock before preparation. It records the first
gate time and reuses that immutable origin on resume.
`release_build_layout_init <source-root> <commit> <state-root> <cache-namespace>`
is called once in its own shell before any `release_build_layout_prepare`;
reinitializing that shell is an error. `prepare` prints only a verified bundle
path for a Rust service or the literal `NONE` for the explicit DB/Web records.
It is the checked-in build helper and benchmark, rather than an operator's
hand-built context, that call `verify_bundle`, write the one-service override,
and bind final image bytes.

Run every long action in the approved background systemd service with
`Nice=10` and `IOSchedulingClass=idle`; `CPUWeight=20` and `IOWeight=20`
also lower the unit's scheduling weights. The service continues after a
terminal or Paseo disconnect. Keep `CARGO_BUILD_JOBS=2` and
`COMPOSE_PARALLEL_LIMIT=1`; never begin a service while an earlier build or
compiler is alive.

The producer checks are separate from consumer progress. The native identity
gate runs first; Rust producers run one requested bin at a time, with the
collector producer checkpoints fixed as `3 + 3 + 2 + 2`. A pending or malformed
guard/receipt recreates only that compatibility target; no successful reuse
may run `cargo clean` for every artifact call, edit a fingerprint, or prune a
global cache. A producer failure leaves no usable success receipt or bundle;
resume revalidates the current source/receipt/bundle before reusing it.

Use these four progress batches, issuing exactly one Compose build call for
each service in the listed order:

1. `db-role-bootstrap`, `db-migrate`, `api-server`
2. `web`, `research-worker`, `recommendation-runner`
3. `candidate-runner`, `owner-beta-runner`, `owner-equity-v2-runner`
4. `nt-backtest-worker-1`, `nt-backtest-worker-2`, `paper-scheduler`

The consumer checkpoints are likewise `3 + 3 + 3 + 3`. Between every producer
or consumer checkpoint, require the preceding one-service build to have
returned successfully, then verify available memory, swap state, the complete
bounded kernel-journal capture for OOM/killed-compiler events, no live
compiler, the approved production health inputs, and that the already-running
background build unit still has its required service/cgroup/priority state. A
failed build, OOM event, compiler/control-plane termination, invalid
receipt/bundle, or unhealthy production service stops progression. Do not
submit the next batch or retry an all-image parallel command; after correction,
resume only the same one-service route so the helper can revalidate and reuse
safe cache/bundle state.

The seven Dockerfile source fallbacks remain for existing callers that build
without a named context. They retain B's source-compilation, OCI-revision, and
compile-ENV provenance contracts. The stronger content-addressed transport and
old-mtime guarantees are specific to the official/helper/benchmark route; do
not infer them from a legacy source-fallback build. Do not treat a
source-fallback test as C artifact-route or performance acceptance.

For A/B/C measurement, `scripts/qa/build-cache-benchmark.sh` records actual
product kind independently of baseline/candidate position. Every canonical
twelve-service release includes preparation or producers, all consumers,
image-save and strict byte checks, and final private V2 publication/revalidation
in `release-totals.tsv`; `release-comparison.tsv` compares only complete warm
releases. Source A/B retain actual Cargo/native evidence without invented
producer receipts, while C additionally binds its producer receipts. The
benchmark manifests under `source-manifests/` and `common-manifests/` are
private evidence, never official release manifests.

The official root-only `build-production-images.sh --apply` invocation itself
performs preparation, the producer checkpoints, and the four consumer batches
before it may finish the release. Run it with the exact commit and a new V2
manifest output path. It holds its lock through preparation, all twelve
sequential builds, strict saved-image byte/OCI checks, exact image-ID/revision
checks, and V2 serialization plus revalidation before an atomic no-clobber
publish. This final validation is not a production rollout; later installation
and Compose activation remain separately authorized actions. The command below
starts the approved unit and runs the builder inside its required cgroup.
`--wait` returns the build's exit status; the service runs independently of
that waiting terminal. `--collect` releases the transient unit after exit, so
a failed attempt can reuse the same unit name and unchanged gate inputs.
Retain its journal and helper attempt logs before retrying, and require a
successful build exit and completed V2 verification before installation.
Prepare the credential-free file below outside the clean source checkout. It
contains only the approved inactive interpolation sentinel and must never be
copied from, or replaced by, the operational `deploy/compose/.env`.

```bash
export LAGRANGE_CODE_COMMIT="$(git rev-parse HEAD)"
release_source_root=$(git rev-parse --show-toplevel)
# Output must be empty or exactly the one allowlisted workbook above.
git status --porcelain=v1 --untracked-files=all

image_build_env_dir=$(mktemp -d "${TMPDIR:-/tmp}/lagrange-image-only-env.XXXXXXXXXX")
chmod 0700 "$image_build_env_dir"
image_build_env=$image_build_env_dir/compose.env
(umask 077; printf '%s\n' 'RESEARCH_ENTITLEMENT_SHA256=0000000000000000000000000000000000000000000000000000000000000000' >"$image_build_env")
chmod 0600 "$image_build_env"
image_build_env_sha256=$(sha256sum "$image_build_env" | awk '{print $1}')
[ "$image_build_env_sha256" = df9d4d1ceb45d0ddb79b98b1fc12c5a2925424c46b79b5d9959f0a1640b27bf6 ]
printf 'image_build_env=%s sha256=%s\n' "$image_build_env" "$image_build_env_sha256"

sudo install -d -o root -g root -m 0755 /etc/lagrange/release-manifests

sudo systemd-run --unit="$RELEASE_BUILD_SYSTEMD_UNIT" \
  --service-type=exec --wait --collect \
  --working-directory="$release_source_root" \
  --property=Nice=10 --property=IOSchedulingClass=idle \
  --property=CPUWeight=20 --property=IOWeight=20 \
  /usr/bin/env CARGO_BUILD_JOBS=2 COMPOSE_PARALLEL_LIMIT=1 \
  LAGRANGE_CODE_COMMIT="$LAGRANGE_CODE_COMMIT" \
  RELEASE_BUILD_SYSTEMD_UNIT="$RELEASE_BUILD_SYSTEMD_UNIT" \
  RELEASE_BUILD_SYSTEMD_MANAGER="$RELEASE_BUILD_SYSTEMD_MANAGER" \
  RELEASE_BUILD_HEALTH_UNITS="$RELEASE_BUILD_HEALTH_UNITS" \
  RELEASE_BUILD_HEALTH_CONTAINERS="$RELEASE_BUILD_HEALTH_CONTAINERS" \
  RELEASE_BUILD_RESEARCH_EXCEPTION="${RELEASE_BUILD_RESEARCH_EXCEPTION:-}" \
  RELEASE_BUILD_DRAINED_READERS_ATTESTATION="${RELEASE_BUILD_DRAINED_READERS_ATTESTATION:-}" \
  /bin/bash "$release_source_root/scripts/ops/build-production-images.sh" --apply \
  --env-file "$image_build_env" \
  --manifest-file "/etc/lagrange/release-manifests/$LAGRANGE_CODE_COMMIT.manifest"
```

The printed path/hash is part of the build attempt evidence. Keep that exact
mode-0600 input at the recorded external path for same-unit retries; it is not
the protected operational Compose environment and contains no credential or
active provider value. The gate samples the two readers' lifecycle state at
each checkpoint, but sampled checks cannot prove that an unrelated one-off
container or external timer did not run between samples. Keep the existing
operator prelaunch and postcheck requirement for zero KIS timers/one-offs and
make no concurrent Docker or runtime mutation during an authorized build.

Using the drained route requires a new source commit after the failed `c8a32ee`
attempt, a fresh commit-specific state root and manifest, and a separately
approved transient unit with the exact environment above. Do not resume the
failed state under changed helper bytes, create an attestation, or retry a
build from this runbook entry; source implementation, attestation creation,
build authorization, installation, and activation remain separate decisions.

After that build succeeds, the separately authorized installation is:

```bash
sudo install -o root -g root -m 0600 deploy/compose/.env \
  /etc/lagrange/compose.env.pending
scripts/ops/deploy-production-release.sh --dry-run \
  --commit "$LAGRANGE_CODE_COMMIT" \
  --env-source /etc/lagrange/compose.env.pending
sudo scripts/ops/deploy-production-release.sh --apply \
  --commit "$LAGRANGE_CODE_COMMIT" \
  --env-source /etc/lagrange/compose.env.pending \
  --release-manifest "/etc/lagrange/release-manifests/$LAGRANGE_CODE_COMMIT.manifest"
sudo scripts/ops/deploy-production-release.sh --check \
  --commit "$LAGRANGE_CODE_COMMIT"
```

Do not call this a product-performance result. Actual twelve-image correctness,
OCI/byte verification under production resources, and whole-release timing are
reserved for WP6.

Each immutable directory is `/opt/lagrange/releases/<commit>`. `current` is an
atomically replaced relative symlink. Compatibility links such as
`/opt/lagrange/deploy -> current/deploy` provide an operator convenience view.
Protected TLS and backup configs keep their no-symlink fence and must pin
`/opt/lagrange/releases/<commit>/deploy/...`; after switching a release,
customize/check the TLS config for that exact release before renewal activation.
`/opt/lagrange/bin` stays installer-owned for stable helpers.
Applying a second commit never overwrites or deletes the first. `--check` and
`--rollback` use only the installed trusted manifest; they reject an optional
external manifest and explicitly block a legacy manifest-less release. Roll
back with:

```bash
sudo scripts/ops/deploy-production-release.sh --rollback \
  --commit <previous-exact-40-hex>
```

The installer binds the release to the protected Compose env as well. On
`--apply`, it parses the root-owned mode-0600 env with the non-evaluating dotenv
parser and requires its `LAGRANGE_CODE_COMMIT` to be one exact nonzero lowercase
40-hex value equal to `--commit`; the shell variable cannot override the file.
Missing, empty, duplicate, malformed, or mismatched input fails before staging,
and the copied staged and installed env are revalidated before publication or a
current-link switch. `--check` and `--rollback` apply the same installed-env
check before switching. Prepare the correct protected input; the installer does
not rewrite or repair it.

The release installer never runs Compose, rebuilds/restarts services, migrates
a DB, or contacts a provider. Rollout is a separate installed-release action:

```bash
sudo /opt/lagrange/current/scripts/ops/compose-release.sh --scope release --apply
```

That command requires the installed V2 manifest, rechecks every local image ID
and OCI revision immediately before startup, creates a mode-0600 temporary
Compose override mapping the twelve services to `image: sha256:<image_id>` with
build reset, passes `--no-build` to every `up`, and omits the opt-in `--build`
from every one-shot `run` (Compose v5 has no `run --no-build`). It checks the
actual `.Image` and OCI revision of each persistent local service after startup. The
one-shot services consume the same override but are intentionally not claimed as
post-start inspected after `--rm`. A mismatch returns failure without printing
container/environment configuration and without automatically stopping or
rolling back services.

### Pre-amendment Docker image and daemon gate

The entitlement amendment guard proves the physically resolved installed files,
protected env, strict manifest, Compose project, and commit tag. It deliberately
does not call Docker and therefore does not, by itself, prove which image a later
`db-migrate` `run` will execute. Before either amendment `--check` or `--apply`, run
the following read-only gate in the same shell that will invoke the amendment.
`--check` itself invokes `db-migrate`, so the full gate must precede `--check` and
be rerun immediately before `--apply`. The gate uses bare `docker` exactly as
`db.sh` does; it must not select a separate daemon with a context-qualified Docker
invocation.

If both `DOCKER_HOST` and `DOCKER_CONTEXT` are set, stop as ambiguous. If only
`DOCKER_CONTEXT` is set, preserve and export that exact context. If neither is
set, resolve `docker context show` once and export the result so later bare
`docker` calls inherit it. If only `DOCKER_HOST` is set, preserve it unchanged.
Do not print either endpoint variable or any endpoint credentials. The same-shell
revalidation below checks the physical current release, selected endpoint,
daemon, local tag/image ID/revision, and both `db-migrate` Compose resolutions.
Do not retag, pull, or mutate images, and do not allow concurrent Docker
operations between a successful revalidation and either action.

```bash
set -euo pipefail

# `LAGRANGE_RELEASE_ROOT` is a synthetic-fixture seam for the checked-in
# self-test. Production operators leave it unset; it is not an activation path.
release_root=${LAGRANGE_RELEASE_ROOT:-/opt/lagrange}
current_link=$release_root/current
[ -L "$current_link" ]
release_dir=$(readlink -f -- "$current_link")
case "$release_dir" in
  "$release_root"/releases/*) ;;
  *) echo 'pre-amendment image gate: current link is outside releases' >&2; exit 1 ;;
esac
release_commit=${release_dir##*/}
compose_file=$release_dir/deploy/compose/compose.yml
env_file=$release_dir/deploy/compose/.env
manifest=$release_dir/.lagrange-release-manifest

source "$release_dir/scripts/ops/lib/entitlement-amend-installed-release.sh"

pre_amendment_gate_fail() {
  printf '%s\n' "pre-amendment image gate: $1" >&2
  return 1
}

# This repeats the installed-release guard at each action boundary. The guard
# rereads the protected env and strict manifest, checks the physical current
# link and entrypoint, and binds db.sh to that same release's Compose inputs.
pre_amendment_release_revalidate() {
  local current_release_dir
  [ -L "$current_link" ] || {
    pre_amendment_gate_fail 'current link is absent'
    return 1
  }
  current_release_dir=$(readlink -f -- "$current_link") || {
    pre_amendment_gate_fail 'current link cannot be resolved'
    return 1
  }
  case "$current_release_dir" in
    "$release_root"/releases/*) ;;
    *)
      pre_amendment_gate_fail 'current link is outside releases'
      return 1
      ;;
  esac
  [ "$current_release_dir" = "$release_dir" ] || {
    pre_amendment_gate_fail 'current release changed'
    return 1
  }
  entitlement_amend_verify_installed_release \
    "$release_commit" "$current_link/scripts/ops/provision-entitlement.sh" || {
    pre_amendment_gate_fail 'installed release identity changed'
    return 1
  }
  [ "${ENTITLEMENT_AMEND_VERIFIED_RELEASE_REVISION:-}" = "$release_commit" ] || {
    pre_amendment_gate_fail 'installed release revision changed'
    return 1
  }
}

pre_amendment_release_revalidate || exit 1

if [ -n "${DOCKER_HOST:-}" ] && [ -n "${DOCKER_CONTEXT:-}" ]; then
  echo 'pre-amendment image gate: DOCKER_HOST and DOCKER_CONTEXT are both set' >&2
  exit 1
fi
if [ -n "${DOCKER_HOST:-}" ]; then
  docker_selection=host
  docker_host=$DOCKER_HOST
  export DOCKER_HOST="$docker_host"
else
  docker_selection=context
  docker_context=${DOCKER_CONTEXT:-}
  if [ -z "$docker_context" ]; then
    docker_context=$(docker context show) || {
      echo 'pre-amendment image gate: Docker context cannot be resolved' >&2
      exit 1
    }
  fi
  [ -n "$docker_context" ] || {
    echo 'pre-amendment image gate: Docker context is empty' >&2
    exit 1
  }
  export DOCKER_CONTEXT="$docker_context"
fi

daemon_before=$(docker info --format '{{.ID}}') || {
  echo 'pre-amendment image gate: Docker daemon cannot be resolved' >&2
  exit 1
}
[ -n "$daemon_before" ] || {
  echo 'pre-amendment image gate: Docker daemon ID is empty' >&2
  exit 1
}

pre_amendment_docker_revalidate() {
  if [ -n "${DOCKER_HOST:-}" ] && [ -n "${DOCKER_CONTEXT:-}" ]; then
    echo 'pre-amendment image gate: Docker endpoint overrides became ambiguous' >&2
    return 1
  fi
  if [ "$docker_selection" = host ]; then
    [ "${DOCKER_HOST:-}" = "$docker_host" ] || {
      echo 'pre-amendment image gate: Docker host changed' >&2
      return 1
    }
    [ -z "${DOCKER_CONTEXT:-}" ] || {
      echo 'pre-amendment image gate: Docker context changed' >&2
      return 1
    }
  else
    [ -z "${DOCKER_HOST:-}" ] || {
      echo 'pre-amendment image gate: Docker host changed' >&2
      return 1
    }
    [ "${DOCKER_CONTEXT:-}" = "$docker_context" ] || {
      echo 'pre-amendment image gate: Docker context changed' >&2
      return 1
    }
    current_context=$(docker context show) || {
      echo 'pre-amendment image gate: Docker context cannot be resolved' >&2
      return 1
    }
    [ "$current_context" = "$docker_context" ] || {
      echo 'pre-amendment image gate: Docker context changed' >&2
      return 1
    }
  fi
  daemon_now=$(docker info --format '{{.ID}}') || {
    echo 'pre-amendment image gate: Docker daemon cannot be resolved' >&2
    return 1
  }
  [ "$daemon_now" = "$daemon_before" ] || {
    echo 'pre-amendment image gate: Docker daemon changed' >&2
    return 1
  }
}

image_override=$(mktemp /tmp/lagrange-entitlement-image-override.XXXXXX)
chmod 0600 "$image_override"
trap 'rm -f -- "$image_override"' EXIT

pre_amendment_image_daemon_gate_revalidate() {
  local base_db_ref expected_id expected_revision inspected override_db_id ref service
  pre_amendment_release_revalidate || return 1
  release_image_manifest_write_compose_override "$image_override" || return 1

  for service in "${RELEASE_IMAGE_SERVICES[@]}"; do
    ref=${RELEASE_IMAGE_MANIFEST_REFS[$service]}
    expected_id=${RELEASE_IMAGE_MANIFEST_IDS[$service]}
    expected_revision=${RELEASE_IMAGE_MANIFEST_REVISIONS[$service]}
    pre_amendment_docker_revalidate || return 1
    inspected=$(docker image inspect \
      --format '{{.Id}}|{{index .Config.Labels "org.opencontainers.image.revision"}}' \
      "$ref") || return 1
    pre_amendment_docker_revalidate || return 1
    [ "${inspected%%|*}" = "$expected_id" ] || {
      pre_amendment_gate_fail 'local image ID changed'
      return 1
    }
    [ "${inspected#*|}" = "$expected_revision" ] || {
      pre_amendment_gate_fail 'local image revision changed'
      return 1
    }
  done

  # This uses the same bare docker-compose selection and guard-exported project
  # as db.sh. The base configuration must retain the mutable manifest tag.
  pre_amendment_docker_revalidate || return 1
  base_db_ref=$(COMPOSE_PROFILES= docker compose \
    --env-file "$env_file" -f "$compose_file" config --format json |
    jq -r '.services["db-migrate"].image // empty') || return 1
  pre_amendment_docker_revalidate || return 1
  [ "$base_db_ref" = "${RELEASE_IMAGE_MANIFEST_REFS[db-migrate]}" ] || {
    pre_amendment_gate_fail 'db-migrate base tag changed'
    return 1
  }

  # The official override is a separate exact-ID assertion. It does not turn
  # the mutable-tag check above into immutable image-pin enforcement.
  pre_amendment_docker_revalidate || return 1
  override_db_id=$(COMPOSE_PROFILES= docker compose \
    --env-file "$env_file" -f "$compose_file" -f "$image_override" \
    config --format json | jq -r '.services["db-migrate"].image // empty') || return 1
  pre_amendment_docker_revalidate || return 1
  [ "$override_db_id" = "${RELEASE_IMAGE_MANIFEST_IDS[db-migrate]}" ] || {
    pre_amendment_gate_fail 'db-migrate exact-ID override changed'
    return 1
  }
}

pre_amendment_image_daemon_gate_revalidate || exit 1
printf '%s\n' 'PRE_AMENDMENT_IMAGE_DAEMON_GATE: PASS (release identity, db-migrate tag/ID/revision, release override, and daemon unchanged)'
```

This gate performs no Compose `run`, `up`, migration, amendment, or provider
call. If the daemon/context, base tag, exact local image ID/revision, or override
resolution changes, stop and re-run the installed-release preflight. A verified
mutable tag immediately before an action is not an immutable image pin, and these
checks cannot make a Docker/image change atomic. The official immutable-release
workflow and the operational no-mutation boundary remain required.

Keep the same shell and Docker-selection variables alive for the actions. The
following is a runnable wrapper only after its named shell variables have been
populated from the approved protected inputs; it deliberately hardcodes no owner
ID, target ID, document path, or approval revision. The amendment command itself
validates the exact pins and input-file modes.

```bash
: "${AMEND_CURRENT_METADATA_FILE:?set the approved current metadata path}"
: "${AMEND_CURRENT_DOCUMENT_FILE:?set the approved current document path}"
: "${AMEND_ORIGINAL_METADATA_FILE:?set the approved original metadata path}"
: "${AMEND_ORIGINAL_DOCUMENT_FILE:?set the approved original document path}"
: "${AMEND_TARGET_ID:?set the approved entitlement UUID}"
: "${AMEND_EXPECTED_OLD_MANAGER:?set the approved previous manager UUID}"
: "${AMEND_CURRENT_OWNER:?set the approved current owner UUID}"
: "${AMENDMENT_APPROVED_REVISION:?set the approved amendment revision}"

amend_args=(
  --current-metadata-file "$AMEND_CURRENT_METADATA_FILE"
  --current-document-file "$AMEND_CURRENT_DOCUMENT_FILE"
  --original-metadata-file "$AMEND_ORIGINAL_METADATA_FILE"
  --original-document-file "$AMEND_ORIGINAL_DOCUMENT_FILE"
  --target-id "$AMEND_TARGET_ID"
  --expected-old-manager "$AMEND_EXPECTED_OLD_MANAGER"
  --current-owner "$AMEND_CURRENT_OWNER"
  --amendment-revision "$AMENDMENT_APPROVED_REVISION"
  --executing-release-revision "$release_commit"
  --env-file "$env_file"
)

pre_amendment_image_daemon_gate_revalidate || exit 1
"$current_link/scripts/ops/provision-entitlement.sh" amend --check "${amend_args[@]}"
pre_amendment_image_daemon_gate_revalidate || exit 1
"$current_link/scripts/ops/provision-entitlement.sh" amend --apply "${amend_args[@]}" \
  --confirm I_UNDERSTAND_AMEND_ACTIVE_ENTITLEMENT
```

The checked-in `scripts/qa/pre-amendment-image-gate-self-test.sh` exercises the
exact fenced gate above against controlled synthetic release inputs and a fake
bare-Docker command. It covers default/explicit context and lone-host positives;
ambiguous overrides; host/context/daemon drift; mutable tag ID/revision drift;
base-tag and exact-override mismatch; and retagging between check and apply
revalidation. It does not prove a deployed candidate. Real installed-release/
image-ID/daemon verification remains a coordinator/WP6 prerequisite because this
candidate is not installed and production access is outside QA scope.

When `OWNER_BETA_ACCESS_MODE=owner_only`, the command runs the installed
networkless artifact `--approval-check` after validating all twelve manifest image
IDs and before the first Compose `up`. It accepts only the fixed sanitized
success contract, including the compile-time embedded approval-registry hash;
failure starts no Compose service and prints no candidate, path, or internal
error. The installed immutable `research-worker` embeds the reviewed v2 and v3
approval records. The default v3 approval-check passed on 2026-08-29 with
ETF11/2,452 sessions/26,972 bars; persisted five-pin v2 work remains replayable.
Owner-only startup also omits `paper-scheduler`; the twelve-image manifest is
unchanged so a future reviewed
Paper gate can activate that exact image without changing release provenance.

Because installation switches `current` without starting services, an approval
failure is recovered by leaving the old containers untouched and switching the
link back to the previously installed V2 release with the rollback command
above. Re-run that previous release's `--check` before any later Compose action.
This sequence is preflight hardening, not evidence that the beta has launched.

No real registry, multi-architecture, or remote-Docker verification is claimed
or required for this host-local beta. A future registry promotion needs separate
owner approval and a new provenance contract.

## Size and configure backups

Backups contain exactly three encrypted classes: a consistent PostgreSQL
custom-format dump, Raw, and Curated. They exclude `/etc/lagrange`, Compose
`.env`, TLS/Auth0/KIS credentials, the encryption key, checkout, and untracked
files.

Choose a dedicated filesystem and size `MAX_TOTAL_BYTES`, `MIN_FREE_BYTES`,
`RETENTION_DAYS`, and `MIN_KEEP` from current DB + Raw + Curated size and growth.
`MIN_KEEP` is a hard floor. If the byte cap cannot be met without crossing it,
the run fails visibly instead of deleting additional sets.

```bash
sudo install -o root -g root -m 0600 \
  deploy/systemd/production-backup.conf.example \
  /etc/lagrange/production-backup.conf.pending
sudoedit /etc/lagrange/production-backup.conf.pending
```

Replace the commit placeholder and verify every path. `KEY_FILE` points to the
existing exact 64-lowercase-hex, no-newline
`/etc/lagrange/secrets/backup_encryption_key`; its value never enters argv,
environment, logs, manifests, or metrics.
Pin `COMPOSE_FILE` and `COMPOSE_ENV_FILE` to the same immutable
`/opt/lagrange/releases/<commit>` directory rather than the mutable `current`
link; `LAGRANGE_CODE_COMMIT` must be that identical commit.

```bash
sudo scripts/ops/run-production-backup.sh --check \
  --config-file /etc/lagrange/production-backup.conf.pending
scripts/ops/install-production-backup.sh --dry-run \
  --config-source /etc/lagrange/production-backup.conf.pending
sudo scripts/ops/install-production-backup.sh --apply \
  --config-source /etc/lagrange/production-backup.conf.pending
sudo scripts/ops/install-production-backup.sh --check \
  --config-source /etc/lagrange/production-backup.conf.pending
```

Apply enables both timers but deliberately does not start either and never
calls Docker. After the configured immutable release paths pass check, the
PostgreSQL image is present, and disk headroom is confirmed,
activate scheduling separately:

```bash
sudo systemctl start lagrange-production-backup.timer
sudo systemctl start lagrange-production-backup-verify.timer
```

The daily timer runs at 02:15 KST with a randomized 30-minute delay. Each set is
staged on the backup filesystem, encrypted with AES-256-CBC/PBKDF2, hashed,
decrypted into a private temporary directory, and restored into a networkless
disposable PostgreSQL instance. Raw and Curated member paths are validated and
extracted. Only then are `VERIFIED` and `COMPLETE` written and the set atomically
published. The weekly timer repeats the isolated restore of `latest`.

Retention runs only after a new verified set exists. It deletes only strictly
named, complete, verified directories under configured `BACKUP_ROOT`, oldest
first, and reports each deletion as unrecoverable. Releases are never pruned.

## Repository-only verification

These use namespace root and fake Docker; they do not touch host `/opt`,
systemd, PostgreSQL, production data, or credentials.

```bash
bash scripts/ops/production-ops-static-check.sh
bash scripts/ops/production-ops-self-test.sh
git diff --check
```
