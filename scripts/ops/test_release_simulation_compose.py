#!/usr/bin/env python3
"""Strict stdlib source contract for the disabled D8 Compose declaration."""

from __future__ import annotations

import json
import re
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SIMULATION_SERVICE = "owner-beta-simulation-runner"
COMMIT_REQUIRED = "${LAGRANGE_CODE_COMMIT:?LAGRANGE_CODE_COMMIT must be supplied by CI as the exact 40-hex Git commit}"

SERVICES_V2 = (
    "db-role-bootstrap",
    "db-migrate",
    "api-server",
    "web",
    "research-worker",
    "recommendation-runner",
    "candidate-runner",
    "owner-beta-runner",
    "owner-equity-v2-runner",
    "nt-backtest-worker-1",
    "nt-backtest-worker-2",
    "paper-scheduler",
)
SERVICES_V3 = SERVICES_V2 + (SIMULATION_SERVICE,)
BATCHES = (
    ("B1", ("db-role-bootstrap", "db-migrate", "api-server")),
    ("B2", ("web", "research-worker", "recommendation-runner")),
    ("B3", ("candidate-runner", "owner-beta-runner", "owner-equity-v2-runner")),
    ("B4", ("nt-backtest-worker-1", "nt-backtest-worker-2")),
    ("B5", ("paper-scheduler", SIMULATION_SERVICE)),
)
RECIPE_BIN_COUNTS = {"D1": 1, "D2": 2, "D3": 1, "D4": 2, "D5": 1, "D6": 10, "D7": 1, "D8": 1}

EXPECTED_SIMULATION_BLOCK = (
    "  owner-beta-simulation-runner:",
    '    profiles: ["owner-beta-simulation-build"]',
    f"    image: lagrange-station-{SIMULATION_SERVICE}:{COMMIT_REQUIRED}",
    "    build:",
    "      context: ../..",
    "      dockerfile: crates/job-queue/Dockerfile.owner-beta-simulation-runner",
    "      args:",
    f"        LAGRANGE_CODE_COMMIT: {COMMIT_REQUIRED}",
    '    restart: "no"',
    "    network_mode: none",
    "    read_only: true",
    "    cap_drop:",
    "      - ALL",
    "    security_opt:",
    "      - no-new-privileges:true",
    "    init: true",
    "    logging: *default-logging",
    "    environment:",
    "      APP_ENV: production",
    "      OWNER_BETA_SIMULATION_MODE: disabled",
    "      OWNER_BETA_ACCESS_MODE: disabled",
    "      OWNER_BETA_PRICE_INPUT_MODE: disabled",
)

EXPECTED_COPIES = (
    "COPY --from=release_artifacts / /release-artifacts",
    "COPY --from=verified-artifacts --chown=0:0 --chmod=0755 /build/target/release/owner-beta-simulation-runner /usr/local/bin/owner-beta-simulation-runner",
    "COPY --from=verified-artifacts --chown=0:0 --chmod=0444 /build/provenance/owner-beta-simulation-runner/artifact.json /usr/share/lagrange/owner-beta-simulation/artifact.json",
    "COPY --from=verified-artifacts --chown=0:0 --chmod=0444 /build/provenance/owner-beta-simulation-runner/COMPLETE /usr/share/lagrange/owner-beta-simulation/COMPLETE",
    "COPY --from=verified-artifacts --chown=0:0 --chmod=0755 /build/deploy/runtime/owner-beta-simulation-entrypoint /usr/local/bin/owner-beta-simulation-entrypoint",
)


class ContractError(ValueError):
    """A closed source contract failed."""


def _need(condition: bool, message: str) -> None:
    if not condition:
        raise ContractError(message)


def _parse_bash_array(source: str, name: str) -> tuple[str, ...]:
    matches = list(re.finditer(rf"(?m)^{re.escape(name)}=\(\s*$", source))
    _need(len(matches) == 1, f"manifest-array-count-invalid:{name}")
    match = matches[0]
    values: list[str] = []
    for line in source[match.end() :].splitlines():
        if line.strip() == ")":
            return tuple(values)
        value = line.strip()
        if not value or value.startswith("#"):
            continue
        _need(re.fullmatch(r"[a-z0-9][a-z0-9-]*", value) is not None, f"manifest-array-shape:{name}")
        values.append(value)
    raise ContractError(f"manifest-array-unclosed:{name}")


def _unique_json_object(items: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in items:
        _need(key not in result, f"layout-duplicate-key:{key}")
        result[key] = value
    return result


def _reject_json_constant(value: str) -> None:
    raise ContractError(f"layout-nonfinite-number:{value}")


def _extract_service_block(compose: str, service: str) -> tuple[str, ...]:
    lines = compose.splitlines()
    service_roots = [index for index, line in enumerate(lines) if line == "services:"]
    _need(len(service_roots) == 1, "compose-services-root-invalid")
    start = [index for index, line in enumerate(lines) if line == f"  {service}:"]
    _need(len(start) == 1 and start[0] > service_roots[0], "compose-simulation-service-count-invalid")
    service_start = start[0]
    end = len(lines)
    for index in range(service_start + 1, len(lines)):
        line = lines[index]
        if re.fullmatch(r"  [A-Za-z0-9_-]+:", line) or (line and not line.startswith((" ", "\t"))):
            end = index
            break
    block = list(lines[service_start:end])
    while block and not block[-1].strip():
        block.pop()
    return tuple(block)


def _validate_compose(compose: str) -> None:
    comments = " ".join(line.lstrip()[1:].strip() for line in compose.splitlines()
                         if line.lstrip().startswith("#"))
    _need("All thirteen local release images embed" in comments, "compose-local-image-count-comment-missing")
    _need("pending the later simulation lifecycle integration gate" in comments,
          "compose-simulation-lifecycle-pending-comment-missing")
    _need("does not activate it." in comments, "compose-simulation-nonactivation-comment-missing")

    tagged_images = re.findall(
        r"(?m)^    image: lagrange-station-([a-z0-9-]+):\$\{LAGRANGE_CODE_COMMIT:\?",
        compose,
    )
    release_images = [service for service in tagged_images if service in SERVICES_V3]
    _need(len(release_images) == 13 and set(release_images) == set(SERVICES_V3),
          "compose-local-image-service-set-not-exactly-thirteen")
    _need(_extract_service_block(compose, SIMULATION_SERVICE) == EXPECTED_SIMULATION_BLOCK,
          "compose-simulation-block-not-exact-disabled-build-only-shape")


def _validate_manifest(manifest: str) -> None:
    lines = manifest.splitlines()
    _need(lines.count("RELEASE_IMAGE_MANIFEST_V2_FORMAT='LAGRANGE_RELEASE_MANIFEST_V2'") == 1,
          "manifest-v2-format-constant-missing")
    _need(lines.count("RELEASE_IMAGE_MANIFEST_V3_FORMAT='LAGRANGE_RELEASE_MANIFEST_V3'") == 1,
          "manifest-v3-format-constant-missing")
    _need(lines.count("RELEASE_IMAGE_MANIFEST_FORMAT=$RELEASE_IMAGE_MANIFEST_V3_FORMAT") == 1,
          "manifest-default-writer-is-not-v3")
    _need(lines.count('RELEASE_IMAGE_SERVICES=("${RELEASE_IMAGE_SERVICES_V3[@]}")') == 1,
          "manifest-default-service-writer-is-not-v3")
    _need(lines.count('      expected_services=("${RELEASE_IMAGE_SERVICES_V2[@]}")') == 2 and
          lines.count('      expected_services=("${RELEASE_IMAGE_SERVICES_V3[@]}")') == 2,
          "manifest-v2-v3-reader-sets-not-both-retained")
    _need(_parse_bash_array(manifest, "RELEASE_IMAGE_SERVICES_V2") == SERVICES_V2,
          "manifest-v2-reader-service-order-changed")
    _need(_parse_bash_array(manifest, "RELEASE_IMAGE_SERVICES_V3") == SERVICES_V3,
          "manifest-v3-writer-service-order-changed")


def _validate_layout(layout_text: str, build_source: str) -> None:
    try:
        layout = json.loads(
            layout_text,
            object_pairs_hook=_unique_json_object,
            parse_constant=_reject_json_constant,
        )
    except (json.JSONDecodeError, UnicodeDecodeError) as exc:
        raise ContractError("layout-json-invalid") from exc
    _need(isinstance(layout, dict), "layout-root-not-object")

    recipes = layout.get("recipes")
    _need(isinstance(recipes, dict) and tuple(recipes) == tuple(RECIPE_BIN_COUNTS),
          "layout-recipe-set-not-d1-through-d8")
    total_bins = 0
    for recipe_id, expected_count in RECIPE_BIN_COUNTS.items():
        recipe = recipes.get(recipe_id)
        _need(isinstance(recipe, dict), f"layout-recipe-invalid:{recipe_id}")
        bins = recipe.get("bins")
        _need(isinstance(bins, list) and all(isinstance(item, str) for item in bins),
              f"layout-recipe-bins-invalid:{recipe_id}")
        _need(len(bins) == expected_count, f"layout-recipe-bin-count-invalid:{recipe_id}")
        total_bins += len(bins)
    _need(total_bins == 19, "layout-bin-count-not-nineteen")
    d8 = recipes["D8"]
    _need(d8.get("dockerfile") == "crates/job-queue/Dockerfile.owner-beta-simulation-runner" and
          d8.get("package") == "job-queue" and d8.get("bins") == [SIMULATION_SERVICE] and
          d8.get("features") == [] and d8.get("compile_commit") == "present",
          "layout-d8-guarded-recipe-contract-changed")

    services = layout.get("services")
    _need(isinstance(services, dict) and tuple(services) == SERVICES_V3,
          "layout-service-order-not-exactly-v3-thirteen")
    _need(services.get(SIMULATION_SERVICE) == {"kind": "rust", "recipe": "D8"},
          "layout-d8-service-binding-invalid")

    batches = layout.get("batches")
    _need(isinstance(batches, list), "layout-batches-invalid")
    actual_batches: list[tuple[str, tuple[str, ...]]] = []
    for batch in batches:
        _need(isinstance(batch, dict) and isinstance(batch.get("id"), str) and
              isinstance(batch.get("services"), list) and
              all(isinstance(item, str) for item in batch["services"]),
              "layout-batch-shape-invalid")
        actual_batches.append((batch["id"], tuple(batch["services"])))
    _need(tuple(actual_batches) == BATCHES, "layout-five-batch-order-changed")
    gates = layout.get("gates")
    _need(isinstance(gates, dict) and gates.get("consumer_batches") == [item[0] for item in BATCHES],
          "layout-consumer-batch-gate-changed")

    batch_calls = []
    for line in build_source.splitlines():
        if re.match(r"^\s*run_batch(?:\s|$)", line):
            match = re.fullmatch(r"run_batch\s+(B\d+)\s+(.+?)\s*", line)
            _need(match is not None, "build-controller-batch-call-shape-invalid")
            batch_calls.append((match.group(1), tuple(match.group(2).split())))
    _need(tuple(batch_calls) == BATCHES, "build-controller-five-batch-order-changed")
    _need('local_image_services=("${RELEASE_IMAGE_SERVICES[@]}")' in build_source,
          "build-controller-not-bound-to-canonical-service-writer")
    _need("COMPOSE_PARALLEL_LIMIT=1" in build_source, "build-controller-compose-parallelism-not-one")


def _validate_d8_image(dockerfile: str, entrypoint: str) -> None:
    copies = tuple(line.strip() for line in dockerfile.splitlines() if line.lstrip().startswith("COPY "))
    _need(copies == EXPECTED_COPIES, "d8-copy-sources-modes-or-order-changed")
    for literal in (
        "--guard-build /release-artifacts/.release-build/request.json /release-artifacts /build",
        'ARG LAGRANGE_CODE_COMMIT',
        '&& test "${#LAGRANGE_CODE_COMMIT}" -eq 40',
        "&& printf '%s' \"$LAGRANGE_CODE_COMMIT\" | grep -Eq '^[0-9a-f]{40}$'",
        '&& test "$LAGRANGE_CODE_COMMIT" != 0000000000000000000000000000000000000000',
        'LABEL org.opencontainers.image.revision="$LAGRANGE_CODE_COMMIT"',
        "ENV APP_ENV=production",
        "ENV OWNER_BETA_SIMULATION_MODE=disabled",
        "ENV OWNER_BETA_ACCESS_MODE=disabled",
        "ENV OWNER_BETA_PRICE_INPUT_MODE=disabled",
        "USER 10001:10001",
        "STOPSIGNAL SIGTERM",
        'ENTRYPOINT ["/usr/local/bin/owner-beta-simulation-entrypoint"]',
    ):
        _need(literal in dockerfile, f"d8-image-contract-missing:{literal}")
    _need(not re.search(r"(?i)\bcargo\b|\bHEALTHCHECK\b", dockerfile),
          "d8-image-has-source-build-or-health-success-path")

    for literal in (
        "RUNNER=/usr/local/bin/owner-beta-simulation-runner",
        '[ "${APP_ENV:-}" = production ] || fail',
        '[ "${OWNER_BETA_SIMULATION_MODE:-}" = owner_only ] || fail',
        '[ "${OWNER_BETA_ACCESS_MODE:-}" = owner_only ] || fail',
        '[ "${OWNER_BETA_PRICE_INPUT_MODE:-}" = sealed_v1 ] || fail',
        'exec "$RUNNER" \\',
        '--official-daemon',
        '--expected-commit "$commit"',
        '--expected-config-sha256 "$config_pin"',
        '--expected-receipt-sha256 "$expected_receipt"',
    ):
        _need(literal in entrypoint, f"d8-entrypoint-contract-missing:{literal}")
    _need(not re.search(r"(?i)healthcheck|\bHEALTHCHECK\b|\btrue\b", entrypoint),
          "d8-entrypoint-contains-health-success-stub")


def validate_sources(
    compose: str,
    manifest: str,
    layout_text: str,
    build_source: str,
    dockerfile: str,
    entrypoint: str,
) -> None:
    """Validate the current closed V3 source shape without parsing general YAML."""
    _validate_compose(compose)
    _validate_manifest(manifest)
    _validate_layout(layout_text, build_source)
    _validate_d8_image(dockerfile, entrypoint)


def validate_paths(paths: list[str]) -> None:
    _need(len(paths) == 6, "source-path-count-invalid")
    contents = []
    for raw_path in paths:
        with open(raw_path, "r", encoding="utf-8", newline="") as handle:
            contents.append(handle.read())
    validate_sources(*contents)


def _fixture_manifest() -> str:
    v2 = "\n".join(f"  {service}" for service in SERVICES_V2)
    v3 = "\n".join(f"  {service}" for service in SERVICES_V3)
    return (
        "RELEASE_IMAGE_MANIFEST_V2_FORMAT='LAGRANGE_RELEASE_MANIFEST_V2'\n"
        "RELEASE_IMAGE_MANIFEST_V3_FORMAT='LAGRANGE_RELEASE_MANIFEST_V3'\n"
        "RELEASE_IMAGE_MANIFEST_FORMAT=$RELEASE_IMAGE_MANIFEST_V3_FORMAT\n"
        "RELEASE_IMAGE_SERVICES_V2=(\n" + v2 + "\n)\n"
        "RELEASE_IMAGE_SERVICES_V3=(\n" + v3 + "\n)\n"
        'RELEASE_IMAGE_SERVICES=("${RELEASE_IMAGE_SERVICES_V3[@]}")\n'
        'case "$header" in\n'
        '    "$RELEASE_IMAGE_MANIFEST_V2_FORMAT")\n'
        '      expected_services=("${RELEASE_IMAGE_SERVICES_V2[@]}")\n'
        "      ;;\n"
        '    "$RELEASE_IMAGE_MANIFEST_V3_FORMAT")\n'
        '      expected_services=("${RELEASE_IMAGE_SERVICES_V3[@]}")\n'
        "      ;;\n"
        "esac\n"
        'case "$header" in\n'
        '    "$RELEASE_IMAGE_MANIFEST_V2_FORMAT")\n'
        '      expected_services=("${RELEASE_IMAGE_SERVICES_V2[@]}")\n'
        "      ;;\n"
        '    "$RELEASE_IMAGE_MANIFEST_V3_FORMAT")\n'
        '      expected_services=("${RELEASE_IMAGE_SERVICES_V3[@]}")\n'
        "      ;;\n"
        "esac\n"
    )


def _fixture_layout() -> str:
    recipes = {}
    for recipe_id, count in RECIPE_BIN_COUNTS.items():
        recipe = {"bins": [f"{recipe_id.lower()}-bin-{index}" for index in range(count)]}
        recipes[recipe_id] = recipe
    recipes["D8"].update(
        {
            "dockerfile": "crates/job-queue/Dockerfile.owner-beta-simulation-runner",
            "package": "job-queue",
            "bins": [SIMULATION_SERVICE],
            "features": [],
            "compile_commit": "present",
        }
    )
    services = {
        service: {"kind": "rust", "recipe": "D8" if service == SIMULATION_SERVICE else "D1"}
        for service in SERVICES_V3
    }
    batches = [{"id": batch_id, "services": list(batch_services)} for batch_id, batch_services in BATCHES]
    return json.dumps(
        {"recipes": recipes, "services": services, "batches": batches,
         "gates": {"consumer_batches": [item[0] for item in BATCHES]}},
        separators=(",", ":"),
    )


def _fixture_compose() -> str:
    lines = [
        "# All thirteen local release images embed their exact commit revision.",
        "# The simulation service remains pending the later simulation lifecycle integration gate; this does not activate it.",
        "services:",
    ]
    for service in SERVICES_V3[:-1]:
        lines.extend(
            [
                f"  {service}:",
                f"    image: lagrange-station-{service}:{COMMIT_REQUIRED}",
                "    build:",
                "      context: ../..",
                "      args:",
                f"        LAGRANGE_CODE_COMMIT: {COMMIT_REQUIRED}",
            ]
        )
    lines.extend(EXPECTED_SIMULATION_BLOCK)
    lines.extend(["networks:", "  default:", "    internal: true"])
    return "\n".join(lines) + "\n"


def _fixture_dockerfile() -> str:
    return "\n".join(
        [
            "FROM alpine@sha256:fixture AS verified-artifacts",
            "COPY --from=release_artifacts / /release-artifacts",
            "RUN set -e \\",
            "    && /release-artifacts/.release-build/release-build-layout.sh --guard-build /release-artifacts/.release-build/request.json /release-artifacts /build",
            "FROM alpine@sha256:fixture",
            "ARG LAGRANGE_CODE_COMMIT",
            "RUN test -n \"$LAGRANGE_CODE_COMMIT\" \\",
            '    && test "${#LAGRANGE_CODE_COMMIT}" -eq 40 \\',
            "    && printf '%s' \"$LAGRANGE_CODE_COMMIT\" | grep -Eq '^[0-9a-f]{40}$' \\",
            '    && test "$LAGRANGE_CODE_COMMIT" != 0000000000000000000000000000000000000000 \\',
            "LABEL org.opencontainers.image.revision=\"$LAGRANGE_CODE_COMMIT\"",
            EXPECTED_COPIES[1],
            EXPECTED_COPIES[2],
            EXPECTED_COPIES[3],
            EXPECTED_COPIES[4],
            "ENV APP_ENV=production",
            "ENV OWNER_BETA_SIMULATION_MODE=disabled",
            "ENV OWNER_BETA_ACCESS_MODE=disabled",
            "ENV OWNER_BETA_PRICE_INPUT_MODE=disabled",
            "USER 10001:10001",
            "STOPSIGNAL SIGTERM",
            'ENTRYPOINT ["/usr/local/bin/owner-beta-simulation-entrypoint"]',
        ]
    ) + "\n"


def _fixture_entrypoint() -> str:
    return "\n".join(
        [
            "#!/bin/sh",
            "RUNNER=/usr/local/bin/owner-beta-simulation-runner",
            '[ "${APP_ENV:-}" = production ] || fail',
            '[ "${OWNER_BETA_SIMULATION_MODE:-}" = owner_only ] || fail',
            '[ "${OWNER_BETA_ACCESS_MODE:-}" = owner_only ] || fail',
            '[ "${OWNER_BETA_PRICE_INPUT_MODE:-}" = sealed_v1 ] || fail',
            'exec "$RUNNER" \\',
            '    --official-daemon \\',
            '    --expected-commit "$commit" \\',
            '    --expected-config-sha256 "$config_pin" \\',
            '    --expected-receipt-sha256 "$expected_receipt"',
        ]
    ) + "\n"


def _fixture_build() -> str:
    return "\n".join(
        [
            'local_image_services=("${RELEASE_IMAGE_SERVICES[@]}")',
            "CARGO_BUILD_JOBS=2",
            "COMPOSE_PARALLEL_LIMIT=1",
            "run_batch() {",
            "    :",
            "}",
            *(f"run_batch {batch_id} {' '.join(batch_services)}" for batch_id, batch_services in BATCHES),
        ]
    ) + "\n"


class SimulationComposeContractTests(unittest.TestCase):
    def setUp(self) -> None:
        self.sources = [
            _fixture_compose(),
            _fixture_manifest(),
            _fixture_layout(),
            _fixture_build(),
            _fixture_dockerfile(),
            _fixture_entrypoint(),
        ]

    def assert_rejected(self, index: int, old: str, new: str) -> None:
        changed = list(self.sources)
        if index == 0:
            block = "\n".join(EXPECTED_SIMULATION_BLOCK)
            self.assertIn(old, block)
            changed[0] = changed[0].replace(block, block.replace(old, new, 1), 1)
            with self.assertRaises(ContractError):
                validate_sources(*changed)
            return
        self.assertIn(old, changed[index])
        changed[index] = changed[index].replace(old, new, 1)
        with self.assertRaises(ContractError):
            validate_sources(*changed)

    def test_source_shaped_fixture_passes_through_static_checker_cli(self) -> None:
        with tempfile.TemporaryDirectory(prefix="simulation-compose-contract-") as directory:
            paths = []
            for index, contents in enumerate(self.sources):
                path = Path(directory) / f"source-{index}.txt"
                path.write_text(contents, encoding="utf-8")
                paths.append(str(path))
            result = subprocess.run(
                [sys.executable, str(Path(__file__).resolve()), "--validate-source", *paths],
                check=False,
                capture_output=True,
                text=True,
                timeout=10,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("D8_COMPOSE_STATIC: PASS", result.stdout)

    def test_disabled_modes_profiles_and_restart_are_closed(self) -> None:
        for old, new in (
            ("OWNER_BETA_SIMULATION_MODE: disabled", "OWNER_BETA_SIMULATION_MODE: owner_only"),
            ("OWNER_BETA_PRICE_INPUT_MODE: disabled", "OWNER_BETA_PRICE_INPUT_MODE: sealed_v1"),
            ("OWNER_BETA_ACCESS_MODE: disabled", "OWNER_BETA_ACCESS_MODE: ${ACCESS_MODE:-disabled}"),
            ('profiles: ["owner-beta-simulation-build"]', 'profiles: ["live"]'),
            ('restart: "no"', "restart: unless-stopped"),
        ):
            with self.subTest(change=old):
                self.assert_rejected(0, old, new)
        self.assert_rejected(0, 'profiles: ["owner-beta-simulation-build"]', "")
        self.assert_rejected(0, "read_only: true", "read_only: ${READ_ONLY:-true}")
        self.assert_rejected(
            0,
            f"image: lagrange-station-{SIMULATION_SERVICE}:{COMMIT_REQUIRED}",
            f"image: lagrange-station-{SIMULATION_SERVICE}:latest",
        )
        self.assert_rejected(0, f"LAGRANGE_CODE_COMMIT: {COMMIT_REQUIRED}", "LAGRANGE_CODE_COMMIT: missing")

    def test_forbidden_compose_capabilities_fail_closed(self) -> None:
        forbidden_lines = (
            "    volumes: []",
            "    networks: [backend]",
            "    secrets: [api_db_app_password]",
            '    ports: ["5432:5432"]',
            "    depends_on: [postgres]",
            '    entrypoint: ["/bin/sh"]',
            '    command: ["true"]',
            "    privileged: true",
            "    environment:\n      DATABASE_URL: postgresql://fixture",
        )
        for addition in forbidden_lines:
            with self.subTest(addition=addition.splitlines()[0]):
                changed = list(self.sources)
                changed[0] = changed[0].replace(
                    "      OWNER_BETA_PRICE_INPUT_MODE: disabled\n",
                    "      OWNER_BETA_PRICE_INPUT_MODE: disabled\n" + addition + "\n",
                    1,
                )
                with self.assertRaises(ContractError):
                    validate_sources(*changed)
        self.assert_rejected(0, "network_mode: none", "network_mode: bridge")
        self.assert_rejected(0, "context: ../..", "context: ../../outside")
        self.assert_rejected(0, "dockerfile: crates/job-queue/Dockerfile.owner-beta-simulation-runner",
                             "dockerfile: crates/job-queue/Dockerfile")

    def test_image_receipt_copy_modes_revision_and_entrypoint_are_pinned(self) -> None:
        self.assert_rejected(4, "/build/provenance/owner-beta-simulation-runner/artifact.json",
                             "/build/output/provenance/owner-beta-simulation-runner/artifact.json")
        self.assert_rejected(4, "--chmod=0444 /build/provenance/owner-beta-simulation-runner/COMPLETE",
                             "--chmod=0644 /build/provenance/owner-beta-simulation-runner/COMPLETE")
        self.assert_rejected(4, "/build/deploy/runtime/owner-beta-simulation-entrypoint",
                             "/release-artifacts/deploy/runtime/owner-beta-simulation-entrypoint")
        self.assert_rejected(4, '    && test "$LAGRANGE_CODE_COMMIT" != 0000000000000000000000000000000000000000 \\',
                             "    && test \"$LAGRANGE_CODE_COMMIT\" != \"\" \\")
        self.assert_rejected(4, 'LABEL org.opencontainers.image.revision="$LAGRANGE_CODE_COMMIT"', "")
        self.assert_rejected(4, 'ENTRYPOINT ["/usr/local/bin/owner-beta-simulation-entrypoint"]',
                             'ENTRYPOINT ["/bin/true"]')
        self.assert_rejected(4, "FROM alpine@sha256:fixture AS verified-artifacts",
                             "FROM alpine@sha256:fixture AS verified-artifacts\nRUN cargo build --locked --release")
        self.assert_rejected(4, "STOPSIGNAL SIGTERM", "HEALTHCHECK CMD true\nSTOPSIGNAL SIGTERM")
        self.assert_rejected(5, "RUNNER=/usr/local/bin/owner-beta-simulation-runner", "RUNNER=/usr/local/bin/unverified-runner")

    def test_manifest_layout_bin_and_batch_drift_is_rejected(self) -> None:
        self.assert_rejected(
            1,
            "RELEASE_IMAGE_MANIFEST_FORMAT=$RELEASE_IMAGE_MANIFEST_V3_FORMAT",
            "RELEASE_IMAGE_MANIFEST_FORMAT=$RELEASE_IMAGE_MANIFEST_V2_FORMAT",
        )
        self.assert_rejected(
            1,
            "  owner-beta-simulation-runner\n)",
            ")",
        )
        layout = json.loads(self.sources[2])
        layout["recipes"]["D6"]["bins"].pop()
        self.assert_layout_rejected(layout)
        layout = json.loads(self.sources[2])
        layout["services"].pop(SIMULATION_SERVICE)
        self.assert_layout_rejected(layout)
        layout = json.loads(self.sources[2])
        layout["batches"][4]["services"].reverse()
        self.assert_layout_rejected(layout)
        self.assert_rejected(
            3,
            f"run_batch B5 paper-scheduler {SIMULATION_SERVICE}",
            "run_batch B5 paper-scheduler",
        )

    def assert_layout_rejected(self, layout: dict[str, object]) -> None:
        changed = list(self.sources)
        changed[2] = json.dumps(layout, separators=(",", ":"))
        with self.assertRaises(ContractError):
            validate_sources(*changed)


class SimulationOverlayContractTests(unittest.TestCase):
    def test_overlay_uses_fixed_health_arguments_and_read_only_mounts(self) -> None:
        # The production verification environment provides real PyYAML; the
        # Compose reset tag is represented as None for structural inspection.
        import yaml

        class Loader(yaml.SafeLoader):
            pass

        Loader.add_constructor("!reset", lambda loader, node: None)
        root = Path(__file__).resolve().parents[2]
        overlay = yaml.load((root / "deploy/compose/compose.owner-beta-simulation.yml").read_text(), Loader=Loader)
        service = overlay["services"][SIMULATION_SERVICE]
        self.assertIsNone(service["network_mode"])
        self.assertEqual(service["networks"], ["owner-beta-db"])
        self.assertEqual(service["user"], "10001:10001")
        self.assertEqual(service["restart"], "no")
        self.assertTrue(service["read_only"])
        self.assertEqual(service["cap_drop"], ["ALL"])
        self.assertEqual(service["environment"]["DB_USER"], "worker")
        self.assertEqual(service["environment"]["DB_PASSWORD_FILE"], "/run/secrets/db_worker_password")
        self.assertNotIn("DATABASE_URL", service["environment"])
        mounts = {item["target"]: item for item in service["volumes"]}
        self.assertEqual(set(mounts), {"/run/owner-beta-simulation/config.json", "/data/owner-beta-artifacts"})
        self.assertTrue(all(item["read_only"] and not item["bind"]["create_host_path"] for item in mounts.values()))
        self.assertEqual(service["healthcheck"]["test"], [
            "CMD", "/usr/local/bin/owner-beta-simulation-runner", "--health", "--expected-commit",
            "${LAGRANGE_CODE_COMMIT:?exact release commit required}", "--expected-config-sha256",
            "${LAGRANGE_SIM_CONFIG_SHA256:?protected simulation config required}", "--expected-receipt-sha256",
            "${LAGRANGE_SIM_RECEIPT_SHA256:?protected simulation config required}"])
        self.assertEqual(service["secrets"][0]["target"], "db_worker_password")
        for field in ("mem_limit", "cpus", "pids_limit", "stop_grace_period"):
            self.assertIn(":?", str(service[field]))


def main(argv: list[str]) -> int:
    if argv and argv[0] == "--validate-source":
        try:
            validate_paths(argv[1:])
        except (OSError, ContractError) as exc:
            print(f"D8_COMPOSE_STATIC: FAIL: {exc}", file=sys.stderr)
            return 1
        print("D8_COMPOSE_STATIC: PASS")
        return 0
    unittest.main(argv=[sys.argv[0], *argv], verbosity=2)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
