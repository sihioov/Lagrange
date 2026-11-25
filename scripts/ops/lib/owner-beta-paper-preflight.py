"""Check the fixed future Paper consumer mounts while the release stays OFF."""

from contextlib import ExitStack
import json
import subprocess


SERVICES = ("api-server", "paper-scheduler")
OVERLAY = "compose.owner-beta-paper-preflight.yml"
ARTIFACT_ROOT = "/historical-price-beta-root"
PAPER_EVIDENCE = "/owner-beta-paper-evidence"


def mounts(release, data, artifacts):
    return {
        "/run/owner-beta-paper/config.json": release + "/deploy/owner-beta-paper/config.json",
        "/data/owner-beta-paper-evidence/current": data + PAPER_EVIDENCE + "/current",
        "/data/owner-beta-paper-evidence/events": data + PAPER_EVIDENCE + "/events",
        "/run/owner-beta-daily/config.json": release + "/.owner-beta-daily/config.json",
        "/run/owner-beta-daily/policy.json": release + "/.owner-beta-daily/policy.json",
        "/data/raw": data + "/raw",
        "/data/owner-beta-daily/admitted": data + "/owner-beta-daily/admitted",
        "/data/owner-beta-artifacts": artifacts + ARTIFACT_ROOT,
    }


def _compose_config(arguments, environment, fail, *, model=b"", services=()):
    result = subprocess.run(
        ["/usr/bin/docker", "compose", *arguments, "config", "--no-env-resolution", "--format", "json", *services],
        env=environment, input=model, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
        check=False, timeout=30,
    )
    if result.returncode != 0 or len(result.stdout) > 512 * 1024:
        fail("PAPER_RELEASE_PREFLIGHT_RENDER")
    return result.stdout


def render(release, env_path, project, images, environment, fail):
    compose = release + "/deploy/compose/"
    return _compose_config(
        ["--project-name", project, "--env-file", env_path,
         "-f", compose + "compose.yml", "-f", compose + OVERLAY],
        {**environment, "LAGRANGE_PAPER_PREFLIGHT_RELEASE": release,
         "LAGRANGE_PAPER_PREFLIGHT_API_IMAGE": images["api-server"],
         "LAGRANGE_PAPER_PREFLIGHT_WORKER_IMAGE": images["paper-scheduler"]}, fail, services=SERVICES)


def no_create_options(environment, fail):
    # Compose releases can omit opposite default values from the rendered
    # bind object. Measure both encodings on this CLI using an inert model.
    mounts = [{"type": "bind", "source": "/paper-preflight-unused", "target": "/" + label,
               "read_only": True, "bind": {"create_host_path": create}}
              for label, create in (("no-create", False), ("create", True))]
    model = json.dumps({"services": {"probe": {"image": "scratch", "volumes": mounts}}}).encode()
    rendered = json.loads(_compose_config(["--project-name", "lagrange-paper-mount-probe",
                    "--env-file", "/dev/null", "-f", "-"], environment, fail, model=model))
    volumes = rendered["services"]["probe"]["volumes"]
    options = {mount["target"]: mount.get("bind") for mount in volumes}
    disabled, enabled = options.get("/no-create"), options.get("/create")
    if (len(volumes) != 2 or disabled not in ({}, {"create_host_path": False})
            or enabled not in ({}, {"create_host_path": True}) or disabled == enabled):
        fail("PAPER_RELEASE_PREFLIGHT_MOUNTS")
    return disabled


def validate_model(model, project, images, expected, data, artifacts, bind_options, fail):
    if type(model) is not dict or model.get("name") != project or type(model.get("services")) is not dict:
        fail("PAPER_RELEASE_PREFLIGHT_RENDER")
    # Other services (e.g. postgres) can appear as Compose dependencies. Only
    # these two consumers are covered by this prospective mount check.
    api_mounts = {"/data/artifacts": (artifacts, True),
                  "/data/stock-price-beta-artifacts": (data + "/stock-price-beta-artifacts", True),
                  "/data/owner-equity-v2-artifacts": (data + "/owner-equity-v2-artifacts", True)}
    worker_mounts = {"/data/artifacts/paper": (artifacts + "/paper", False)}
    for service, existing in zip(SERVICES, (api_mounts, worker_mounts), strict=True):
        selected = model["services"].get(service, {})
        if (selected.get("image") != images[service] or selected.get("build") is not None
                or selected.get("user") != "10001:10001" or selected.get("read_only") is not True
                or selected.get("environment", {}).get("OWNER_BETA_PAPER_MODE") != "disabled"
                or any(selected.get(key) for key in ("privileged", "cap_add", "devices",
                    "group_add", "volumes_from", "pid", "ipc", "network_mode",
                    "security_opt", "post_start", "pre_stop"))):
            fail("PAPER_RELEASE_PREFLIGHT_SERVICE")
        volumes = selected.get("volumes", ())
        destinations = {item.get("target"): item for item in volumes}
        if len(destinations) != len(volumes) or set(destinations) != expected.keys() | existing.keys():
            fail("PAPER_RELEASE_PREFLIGHT_MOUNTS")
        for target, source in expected.items():
            if destinations[target] != {"type": "bind", "source": source, "target": target,
                                        "read_only": True, "bind": bind_options}:
                fail("PAPER_RELEASE_PREFLIGHT_MOUNTS")
        for target, (source, readonly) in existing.items():
            volume = destinations[target]
            if (volume.get("type") != "bind" or volume.get("source") != source
                    or volume.get("read_only", False) is not readonly):
                fail("PAPER_RELEASE_PREFLIGHT_MOUNTS")


def verify(release, path, verified, source, operations, release_helpers):
    fail = release_helpers._fail
    if verified.process_sha256 is None:
        fail("PAPER_RELEASE_PREFLIGHT_PROCESS")
    commit = verified.commit
    env_path = release + "/deploy/compose/.env"
    b, original = source.binding(path)
    values, _ = operations.read_context(release, commit, "prepare", expected_env_sha256=b["production_env_sha256"])
    data = values["LAGRANGE_DATA_DIR"]
    artifacts = values["LAGRANGE_ARTIFACTS_DIR"]
    secrets = values["LAGRANGE_RUNTIME_SECRET_DIR"]
    cohort = b["cohort"]
    if (b["release_root"] != release or b["production_env_file"] != env_path
            or b["data_root"] != data or b["secret_root"] != secrets
            or b["database"] != (values["POSTGRES_DB"] or "lagrange")
            or cohort["verifier_revision"] != commit
            or cohort["release_manifest_sha256"] != verified.manifest_sha256
            or values["OWNER_BETA_ACCESS_MODE"] != "owner_only"
            or values["OWNER_BETA_PRICE_INPUT_MODE"] != "sealed_v1"
            or values["OWNER_BETA_PAPER_MODE"] not in ("", "disabled")):
        fail("PAPER_RELEASE_PREFLIGHT_BINDING")
    # Reject legacy/broad layouts which would expose the private witness,
    # credentials, or the entire installed release via the existing artifacts bind.
    for private in (release, path, secrets, b["calendar_dir"], b["evidence_root"]):
        if private == artifacts or private.startswith(artifacts + "/"):
            fail("PAPER_RELEASE_PREFLIGHT_BINDING")
    expected = mounts(release, data, artifacts)
    install = operations.install
    config, config_sha = install.verify_config(release, commit)
    if (config_sha != b["daily_config_sha256"] or config["policy_sha256"] != cohort["policy_sha256"]):
        fail("PAPER_RELEASE_PREFLIGHT_BINDING")
    process_path = expected["/run/owner-beta-paper/config.json"]
    process_bytes = install.read_root_file(process_path, gid=10001, mode=0o440, maximum=release_helpers.MAX_PROCESS)
    process = release_helpers._json(process_bytes, release_helpers.MAX_PROCESS)
    if (release_helpers._digest(process_bytes) != verified.process_sha256
            or process["owner_user_id"] != cohort["owner_user_id"]
            or process["strategy_config_id"] != cohort["strategy_config_id"]
            or process["daily_cohort_sha256"] != cohort["cohort_sha256"]):
        fail("PAPER_RELEASE_PREFLIGHT_BINDING")
    images = operations.read_manifest(release, commit)
    # Retain checked, no-symlink source chains through rendering. An absent
    # Current leaf is valid before the first ARM; its containing directory is mandatory.
    base = operations.base
    with ExitStack() as stack:
        for target, kind, uid in (("/data/owner-beta-paper-evidence/current", "paper_evidence", 0),
                                  ("/data/owner-beta-paper-evidence/events", "paper_evidence", 0),
                                  ("/data/raw", "raw", b["raw_owner_uid"]),
                                  ("/data/owner-beta-daily/admitted", "daily", 10003)):
            root = base.files.Root(kind, expected[target], uid, 10001, 0o750)
            stack.enter_context(base.files.open_root(root))
        stack.enter_context(base.source_artifact(expected["/data/owner-beta-artifacts"]))
        compose = release + "/deploy/compose/"
        compose_inputs = tuple((compose + name, install.read_root_file(compose + name,
                                gid=0, mode=0o644, maximum=256 * 1024))
                               for name in ("compose.yml", OVERLAY))
        model = release_helpers._json(render(release, env_path, b["project"], images,
                                             operations.ENVIRONMENT, fail), 512 * 1024)
        bind_options = no_create_options(operations.ENVIRONMENT, fail)
        validate_model(model, b["project"], images, expected, data, artifacts,
                       bind_options, fail)
        if (source.binding(path) != (b, original) or operations.read_manifest(release, commit) != images
                or any(install.read_root_file(file, gid=0, mode=0o644, maximum=256 * 1024) != before
                       for file, before in compose_inputs)):
            fail("PAPER_RELEASE_PREFLIGHT_CHANGED")
        verified.revalidate()
