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

When the already deployed R2 daily service is OFF, its `owner-beta-runner` may
have completed normally with exit0 and `restart=no`. It must remain in the
eleven-container health inventory. The original two-reader attestation v1
continues to require a running, healthy beta container. The explicit
`lagrange-build-drained-readers-attestation-v2` adds only the exact stopped
`lagrange-station-owner-beta-runner-1` to the original two drained readers.
Both Owner runners require exit0; the historical research worker remains exit2.

Before publishing this root-owned mode0600 v2 attestation, observe the installed
source release's daily control OFF and zero running daily jobs, retain that
nonsecret observation in a protected receipt, and bind its SHA256. Its
`disabled_daily` object contains `enabled:false`, `running_jobs:0`,
`activation_epoch`, `owner_user_id`, `policy_sha256`, `source_release`, and
`control_observation_sha256` (hashes have no prefix). This is the operator's
bounded observation, not a DB query performed by the build helper. Preserve
the original maximum twelve-hour expiry and freeze the attestation before
initializing the build. Every checkpoint still inspects all eleven containers
and requires the exact stopped IDs, images, exit codes, health statuses,
restart counts and start/finish timestamps. Any changed generation, unhealthy
ordinary service, missing reader or changed observation binding fails.
Do not start a disabled beta service merely to satisfy a build check. This
image-only allowance neither enables daily recommendations nor accepts runtime
deployment; the running Owner V2 reader still needs the original normal drain
and independently owed restoration.

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

## R4-06 simulation release and recovery — validated candidate, not activated

The official simulation path is authored and source-reviewed. Candidate
`a006e5ee683b4e36bdb7f89924a01def50e240d5` passed the same-SHA hosted
[CI run](https://github.com/sihioov/Lagrange/actions/runs/40639806731) and
[Research smoke](https://github.com/sihioov/Lagrange/actions/runs/40639817540).
CI includes Rust compilation, the 289 Linux private process regressions,
simulation DB tests, six real PostgreSQL observer SQLx regressions, and
repository-only operations and backup fixtures. The exact evidence and limits
are in the [G2 review](../reviews/2026-10-28-r4-g2-validation.md).
Actual installation, Compose transition,
approved-input observation, backup/restore and Owner acceptance remain **NOT RUN**.
The operations below require the separately scoped candidate and execution plan in the
[integration verification sheet](../superpowers/plans/2026-10-27-r4-integration-interface.md#wp-6b-early-verification-sheet--commands-prepared-not-executed).
This section does not authorize activation or replace the established build
resource limits. The V2 twelve-service manifest described above remains the
legacy contract; use the simulation-aware installer and manifest checks for a
candidate containing the optional simulation image.

Installation accepts `--simulation-config-source`, `--simulation-receipt-source`
and `--simulation-complete-source` together. The protected configuration and
receipt bind the exact release/image, approved input and limits. The launcher
passes independent commit, prefixed config SHA-256 and bare receipt SHA-256 pins
to `--official-daemon`; `--health` uses the same three pins. Do not manufacture
these values from placeholders or treat a configured entitlement as a legal
grant. The base declaration stays OFF. The optional
`compose.owner-beta-simulation.yml` uses the internal `owner-beta-db` network,
a dedicated worker secret, read-only input/config mounts and a separate health
records tmpfs. Its resource values come from validated protected config.

Before first official startup, verify that `worker` is `NOINHERIT` and has no
role memberships. The trigger observer rejects `INHERIT` even when membership
is empty. The normal bootstrap currently preserves this existing worker flag;
its explicit `NOINHERIT` branch covers only `daily_input_admitter`. On the
2026-10-29 installation, the coordinator used a finite guarded reduction to
`worker NOINHERIT` under a membership lock and verified an unchanged digest of
all table, routine, schema and database ACLs and role memberships. This is no
new grant. Do not weaken the observer, alter a role with memberships without
disposition, or relabel the original failed process as a clean drain. The
[G3 record](../reviews/2026-10-28-r4-g3-readonly-preflight.md) preserves the actual
rejection and recovery evidence.

The release lock covers inspection, API fencing, worker drain, DB settlement
and the atomic `current` switch. The API must carry the image-bound
`org.lagrange.api.http-drain-contract=v1` capability and the expected executable
identity. The controller retains the exact process generation, disables restart,
sends TERM and confirms successful exit of that same generation. Health, absence,
a previously stopped container or a saved handoff record alone cannot prove
handler completion. DB settlement is a separate check.

| Explicit installed-wrapper operation | Behavior |
| --- | --- |
| `--simulation-drain` | Fence the source API, drain the exact worker, check settlement, retain a protected handoff and leave services stopped. |
| Installer activation / compatible rollback | Validate source and target identities and reader floor before switching `current`; retain the source generations across the switch. No automatic resume. |
| `--simulation-release-start` | Recheck the stopped source and target under lock, then use ordinary Compose replacement/start for the target API and release. Simulation worker start remains separate. |
| `--simulation-start` | Record ownership before creation/start, check exact target policy, then start only the captured worker ID and verify health. |
| `--simulation-reconcile-start` | Reobserve a recorded attempt. Preserve an unchanged stopped source or fence and settle the proven target worker; never start a service. |
| `--simulation-reconcile-release` | Observe an interrupted API handoff; accept the exact target or make retry available only when the original stopped source is still proven. Never starts services. |

Use these flags with the existing installed wrapper's release/apply arguments.
Do not delete `.owner-beta-simulation-start-pending` or
`.owner-beta-simulation-release-handoff` to bypass a rejection. An unknown or
foreign generation, unconfirmed API completion, partial journal write, or missing
target simulation config remains pending. Reconciliation preserves primary,
start-response, interruption and cleanup uncertainty independently. Forced
recreation/removal, `down`, KILL and automatic resume are not recovery steps.

Simulation-aware backup uses the existing protected backup controller and
encrypted dump/archive/verifier pipeline, with a distinct simulation format.
It captures simulation-only or combined Daily/simulation evidence, the input
and release/receipt bindings, reader floor and history, and checks an isolated
restore before publication. It leaves services OFF. Legacy backup is available
only when simulation is positively inapplicable; unknown history is not empty.
The current fixed approval registry describes one candidate. Additional input
history is rejected rather than silently omitted and needs an explicit retention
contract. Neither a source review nor a `CONFIG_ONLY` check proves backup safety.

## Repository-only verification

These use namespace root and fake Docker; they do not touch host `/opt`,
systemd, PostgreSQL, production data, or credentials.

The hosted CI policy job ran the repository-only operations self-test on
`a006e5e` successfully, including 23 simulation operations and 10 simulation
backup Python tests. These fixtures do not prove real container transition,
backup safety or restoration. The prior SOURCE-stage two full fake-Docker
runs remain consumed; this documentation grants no additional production run.

```bash
bash scripts/ops/production-ops-static-check.sh
bash scripts/ops/production-ops-self-test.sh
git diff --check
```

## Preactivation checkpoint with independently preserved dates

The Owner adopted this separate pre-upgrade route on 2026-10-29 for the existing
Daily DB first session2026-10-21 and installed configuration2026-10-22. Neither
date is repaired. Normal Daily backup still requires exact date equality.

The immutable maintenance package exposes `--operation preactivation-backup`
and `--operation preactivation-verify` directly through
`scripts/ops/lib/owner-beta-daily-backup.py`. Both require `--mode apply`,
`--install-root`, exact `--application-release` and `--config-file` bindings,
`--expected-db-first-session`, `--expected-config-first-session`, and
`--stopped-worker-from-commit`. The latter selects an independently trusted
installed manifest for an already stopped legacy worker; mutable container
labels alone cannot supply its identity. Plan and preflight remain non-mutating.

Execution requires schema62 with no R4 tables/history/config/floor/worker, Daily
OFF and activation_epoch0, no Daily runs/admissions/verified frontiers or other
committed Daily history, matching Owner/policy/trigger, and the exact two date
pins. All relevant workers must already be stopped with successful exits and
restart=no, and their generations are rechecked through publication. The
controller never calls disable, repair, enable or resume. Any changed source
or unconfirmed verifier cleanup rejects success. This is not a route for an
activated Daily cohort or a later simulation backup.

Use a separate private backup root and metrics path within it, the existing
shared backup/release locks, and the existing minimum free-space and ancestry
checks. Existing checkpoints are never automatically pruned. A checkpoint uses
`backup-preactivation-<timestamp>-<application-commit>`,
`LAGRANGE_PREACTIVATION_BACKUP_V1`, and encrypted state
`lagrange-preactivation-backup-state-v1`. Its `date_binding` records the DB and
configuration dates independently. Normal Daily verification rejects this
format. Full PostgreSQL dump equality, exact archived file bytes/metadata and
the restored config's hash/date are verified in the existing disposable,
networkless PostgreSQL restore path before publication. A later explicit
preactivation-verify repeats the isolated restore while the original source
bindings still hold; it never restores over the live database.

This capture preserves the existing discrepancy as recovery evidence. It does
not authorize Daily activation, choose a new first session, or establish
post-upgrade simulation backup/rollback or Owner acceptance.


### R4 capture while Daily remains unactivated with preserved dates

When R4 is configured but the separately adopted inactive Daily contract still
has two preserved dates, use `inactive-simulation-backup` and
`inactive-simulation-verify` on the exact pinned maintenance entrypoint. Supply
both `--expected-db-first-session` and `--expected-config-first-session` and the
exact `--stopped-worker-from-commit`; never change DB/config dates to pass the
normal Daily guard. These explicit operations require the normal release and
backup locks, fully bound simulation config/receipt/manifest, a separate backup
root and metrics path, Daily OFF/epoch0/no history, and an already stopped,
owned Daily cohort. Every Python controller dependency except the distinct
backup entrypoint must match the immutable application. The maintenance package
itself remains fully hash-verified against its exact source commit.

The capture fences the actual v1 API and drains the exact simulation worker,
checks settlement, and keeps source Daily untouched. It archives the full
Daily/simulation profile, full PG dump, reader floor/history and actual approved
input. The distinct `LAGRANGE_INACTIVE_DAILY_SIMULATION_BACKUP_V1` format binds
both dates and original config hash and checks both Daily and simulation audits
on a networkless restored DB before publication. Explicit verify repeats that
restore while services remain stopped. Normal Daily/simulation verifiers reject
this state/format; it cannot enable or reconcile Daily. Failure never resumes
services, repairs source control, prunes old checkpoints or publishes an
unverified set. Source and isolated tests do not constitute an actual completed
production backup; recover its real operation and cleanup evidence separately.


### Initial OFF bridge observer receipt

A first OFF bridge can be current before its configured successor exists. Use
the hash-pinned maintenance installer's explicit `install-service-status.py
--maintenance-off` only with the actual installed bridge env: quotes OFF and
Owner-beta access, price input, Daily and equity signals all `disabled`. The
existing owner-only observer visibility, transport, project, exact current
manifest, lease, unit integrity and Paper guards remain. Default installation
still rejects disabled Owner-beta access. This flag changes observer binding,
not application admission; it never edits env, enables the KIS timer or starts
Research/Daily. Prepare the bridge's exact KIS units through
`install-kis-daily.sh --install-disabled`, preserving reviewed runner flags.
Only the status observation timer may be enabled by the observer installer.
After configured activation use the normal installer without this flag.


For the separately bound inactive-Daily maintenance verifier, the complete
historical input tree may contain both the fixed approved V3 candidate and the
already approved V2 subtree under `kis-historical-price-only-beta/v2/`.
Retain both in the `base` and `simulation_input` aliases and require identical
metadata and bytes in those aliases. The finite extension accepts only the
existing embedded V2 registry hash, its exact candidate and canonical manifest,
and every bars byte/hash/size and protected mode. Unknown siblings still fail
closed; never delete or omit historical input to satisfy the V3-only verifier.
Normal simulation verification and the normal Daily date guard are unchanged.
A maintenance source check does not prove an actual capture or isolated restore.


### R4 inactive Daily snapshot: retained stopped handoff (2026-10-29)

현행 application `312f4c8401e7922063561381045f2568641008bf`용 별도 maintenance `43958cd94487f1ed3bc0665c0ade6c316e5af744`는 전체 CI40781769180·설치·preflight를 통과했다. 도구 manifest는 `sha256:c28a7c97913661f80228c75c8fcdafc6c9da53b8e66b3f95261cfd5f03a8b624`, 보호된 config는 `/etc/lagrange/release-manifests/r4-312f4c8/inactive-simulation-backup-43958cd.conf`다. DB2026-10-21/config2026-10-22를 독립 보존하며 정상 Daily guard를 대체하지 않는다. 기존 승인 V2 sibling과 V3 전체 입력·같은 두 alias를 누락 없이 검증한다. 미승인 sibling은 계속 거절한다.

`inactive-simulation-verify`는 같은 current commit의 protected `switched` handoff를 읽어 기존 C4의 전체 source/target manifest·config·receipt·API image pin과 START_PENDING 부재를 검증한다. 실제 API/worker stopped generation과 DB settlement를 검증 전후 다시 확인하며 journal은 변경하거나 소비하지 않는다. 일반 no-pending guard를 끄거나 record를 지워 진입하지 않는다. 정상 release/simulation 시작만 명시적 복귀를 담당한다.

2026-10-29 실제 encrypted post-R4 capture와 첫 isolated PG restore는 PASS다. 보정 전 독립 verify는 pending 충돌로 거절됐고, 보정 후 독립 verify는 Owner 야간 시한 때문에 NOT RUN이다. 기존 완료 집합과 두 실패 기록을 보존한다. 다음 Owner 실행 지시에서 새 시간 한도·private 출력·fresh 세대 증거로 기존 집합의 독립 복원을 실제 검증한 뒤 정상 복귀한다. 과거03/04 감독 스크립트의 만료 시각을 수정해 재사용하지 않는다. 이 별도 snapshot은 자동 Daily 백업 활성화나 fstab 영속 storage 설정을 뜻하지 않는다.


2026-10-29 후속 실제 검증: Owner의 재개 지시로 새05 control/새 generation에서439의 정상 `inactive-simulation-verify`를 실행해 PASS했다. actual network-none PG verifier의 읽기 전용 root·UID999:999·권한 제거·자원 제한을 직접 관측했고, 기존 capture의 첫 restore와 합쳐 실제 복원2회다. 이후 별도 정상 release/simulation 시작·원래 manual reader 정책·fresh observer·healthy/API200·7개 이력/두 날짜 동일·pending/verifier 부재를 확인했다. 원03/04 helper는 미실행 상태로 보존하며, 이전 NOT RUN은 야간 cutoff 시점의 역사다. 실제05 근거는 G3 검토 기록과 root `owner-resume-20261029-01` 출력이다. human Owner 사용 확인은 여전히 별도다.
