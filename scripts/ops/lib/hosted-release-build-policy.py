"""Closed contracts for the official GitHub-hosted image build adapter.

This module has no import-time I/O.  The caller passes only the small set of
non-secret GitHub runner identity fields needed to bind the hosted exception
to the repository's release-delivery job.  It never accepts a general
environment mapping, provider credential, or user-selected build profile.
"""

import re


GITHUB_REPOSITORY = "sihioov/Lagrange"
GITHUB_WORKFLOW_REF = (
    "sihioov/Lagrange/.github/workflows/release-delivery.yml@refs/heads/main"
)
WORKSPACE = "/home/runner/work/Lagrange/Lagrange"
REMOTE_URLS = frozenset(
    {
        "https://github.com/sihioov/Lagrange",
        "https://github.com/sihioov/Lagrange.git",
    }
)
def _require(condition, reason):
    if not condition:
        raise ValueError(reason)


def _positive_decimal(value):
    return isinstance(value, str) and re.fullmatch(r"[1-9][0-9]{0,19}", value)


def validate_runner_identity(
    *,
    fields,
    os_release,
    machine,
    commit,
    source_root,
    source_owner_uid,
    runner_uid,
    remote_url,
):
    """Validate the fixed release workflow, host, sudo caller, and checkout."""

    _require(isinstance(fields, dict), "hosted-runner-fields-invalid")
    expected = {
        "GITHUB_ACTIONS": "true",
        "GITHUB_REPOSITORY": GITHUB_REPOSITORY,
        "GITHUB_REF": "refs/heads/main",
        "GITHUB_EVENT_NAME": "workflow_dispatch",
        "GITHUB_WORKFLOW_REF": GITHUB_WORKFLOW_REF,
        "GITHUB_JOB": "delivery",
        "RUNNER_OS": "Linux",
        "RUNNER_ARCH": "X64",
        "RUNNER_ENVIRONMENT": "github-hosted",
        "GITHUB_SHA": commit,
        "GITHUB_WORKSPACE": WORKSPACE,
        "SUDO_USER": "runner",
        "SUDO_UID": str(runner_uid),
    }
    _require(set(fields) == set(expected) | {"GITHUB_RUN_ID", "GITHUB_RUN_ATTEMPT"},
             "hosted-runner-field-set-invalid")
    _require(all(fields.get(key) == value for key, value in expected.items()),
             "hosted-runner-identity-invalid")
    _require(_positive_decimal(fields.get("GITHUB_RUN_ID")), "hosted-runner-run-id-invalid")
    _require(_positive_decimal(fields.get("GITHUB_RUN_ATTEMPT")), "hosted-runner-attempt-invalid")
    _require(
        isinstance(commit, str) and re.fullmatch(r"[0-9a-f]{40}", commit)
        and commit != "0" * 40,
        "hosted-runner-commit-invalid",
    )
    _require(isinstance(os_release, dict), "hosted-runner-os-release-invalid")
    _require(os_release.get("ID") == "ubuntu" and os_release.get("VERSION_ID") == "24.04",
             "hosted-runner-os-invalid")
    _require(machine == "x86_64", "hosted-runner-architecture-invalid")
    _require(source_root == WORKSPACE and source_owner_uid == runner_uid,
             "hosted-runner-checkout-invalid")
    _require(runner_uid > 0 and remote_url in REMOTE_URLS, "hosted-runner-source-invalid")
    return {
        "repository": GITHUB_REPOSITORY,
        "workflow_ref": GITHUB_WORKFLOW_REF,
        "job": "delivery",
        "run_id": fields["GITHUB_RUN_ID"],
        "run_attempt": fields["GITHUB_RUN_ATTEMPT"],
        "runner_image": "ubuntu-24.04",
        "runner_architecture": "linux/amd64",
        "source_commit": commit,
    }


def parse_os_release(raw):
    """Read only ID and VERSION_ID from the simple os-release assignment form."""

    if not isinstance(raw, str):
        raise ValueError("hosted-runner-os-release-invalid")
    wanted = {"ID", "VERSION_ID"}
    result = {}
    for line in raw.splitlines():
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        if key not in wanted:
            continue
        if key in result or not re.fullmatch(r"[A-Za-z0-9._-]+", value.strip('"')):
            raise ValueError("hosted-runner-os-release-invalid")
        result[key] = value.strip('"')
    if set(result) != wanted:
        raise ValueError("hosted-runner-os-release-invalid")
    return result


def reject_production_cohorts(*, systemd_units, containers, production_root_present):
    """Fail closed if a hosted runner has any installed production cohort."""

    _require(not production_root_present, "production-install-root-present")
    _require(isinstance(systemd_units, (list, tuple)) and
             all(isinstance(item, str) for item in systemd_units),
             "hosted-systemd-inventory-invalid")
    _require(isinstance(containers, (list, tuple)) and
             all(isinstance(item, str) for item in containers),
             "hosted-container-inventory-invalid")
    _require(not any(item.startswith("lagrange-") for item in systemd_units),
             "production-systemd-cohort-present")
    _require(not any(item.startswith("lagrange-station-") for item in containers),
             "production-container-cohort-present")


def parse_systemd_inventory(raw):
    """Parse the first column from systemctl's no-legend unit listings."""

    try:
        text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
    except UnicodeError as error:
        raise ValueError("hosted-systemd-inventory-invalid") from error
    if not isinstance(text, str):
        raise ValueError("hosted-systemd-inventory-invalid")
    names = []
    for line in text.splitlines():
        if not line.strip():
            continue
        name = line.split()[0]
        if not re.fullmatch(r"[A-Za-z0-9_.:@-]+\.service", name):
            raise ValueError("hosted-systemd-inventory-invalid")
        names.append(name)
    return names


def parse_docker_container_inventory(raw):
    """Parse `docker ps --all --format {{.Names}}` without retaining IDs."""

    try:
        text = raw.decode("utf-8") if isinstance(raw, bytes) else raw
    except UnicodeError as error:
        raise ValueError("hosted-container-inventory-invalid") from error
    if not isinstance(text, str):
        raise ValueError("hosted-container-inventory-invalid")
    names = [line for line in text.splitlines() if line]
    if any(not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]*", name) for name in names):
        raise ValueError("hosted-container-inventory-invalid")
    return names
