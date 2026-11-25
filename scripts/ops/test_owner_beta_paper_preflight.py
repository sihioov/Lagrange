"""Render the OFF-only Paper model with Compose and exercise its trust checks."""

import contextlib
import copy
import importlib.util
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import types
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
MODULE = ROOT / "scripts/ops/lib/owner-beta-paper-release.py"
SPEC = importlib.util.spec_from_file_location("owner_beta_paper_preflight_release_test", MODULE)
assert SPEC is not None and SPEC.loader is not None
release = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = release
SPEC.loader.exec_module(release)
ops = release._daily_operations()
preflight = ops.load_local("paper_preflight_test", "owner-beta-paper-preflight.py")
COMMIT = "a1" * 20
DATA = "/var/lib/lagrange-paper-fixture/data"
ARTIFACTS = "/var/lib/lagrange-paper-fixture/artifacts"
SECRETS = "/var/lib/lagrange-paper-fixture/secrets"
WITNESS = "/var/lib/lagrange-paper-fixture/witness.json"
PROJECT = "paper-preflight-fixture"
OWNER = "78341cf5-2e80-4b99-a928-def2a77b0e45"
COHORT = "sha256:" + "a" * 64
STRATEGY = "2888ad66-f43c-4ac1-a068-c9aa336579dc"
POLICY = "sha256:" + "c" * 64
DAILY = "sha256:" + "d" * 64


class PaperProcessPreflightTests(unittest.TestCase):
    def setUp(self):
        tmp = self.enterContext(tempfile.TemporaryDirectory(prefix="paper-preflight-model-"))
        self.environment = {**ops.ENVIRONMENT, "HOME": tmp}
        self.release = str(Path(tmp) / "releases" / COMMIT)
        self.compose_dir = Path(self.release) / "deploy/compose"
        self.compose_dir.mkdir(parents=True)
        base = ROOT / "deploy/compose/compose.yml"
        shutil.copyfile(base, self.compose_dir / base.name)
        overlay = ROOT / "deploy/compose" / preflight.OVERLAY
        shutil.copyfile(overlay, self.compose_dir / overlay.name)
        self.env_path = self.compose_dir / ".env"
        # Config renders the whole base even when only two services are selected.
        # Fill unrelated required interpolation slots with inert fixture strings.
        values = dict.fromkeys(re.findall(r"\$\{([A-Z0-9_]+):\?", base.read_text()), "fixture")
        values.update(LAGRANGE_CODE_COMMIT=COMMIT, LAGRANGE_DATA_DIR=DATA,
                      LAGRANGE_ARTIFACTS_DIR=ARTIFACTS, LAGRANGE_RUNTIME_SECRET_DIR=SECRETS,
                      OWNER_BETA_PAPER_MODE="disabled")
        self.env_path.write_text("".join(f"{k}={v}\n" for k, v in values.items()))
        self.images = {"api-server": "sha256:" + "2" * 64, "paper-scheduler": "sha256:" + "3" * 64}
        self.manifest_sha = release._digest(b"fixture-manifest")
        self.mounts = preflight.mounts(self.release, DATA, ARTIFACTS)
        self.bind_options = preflight.no_create_options(self.environment, release._fail)

    def render(self):
        raw = preflight.render(self.release, str(self.env_path), PROJECT, self.images,
                               self.environment, release._fail)
        return json.loads(raw)

    def assert_model(self, model, *, options=None):
        preflight.validate_model(model, PROJECT, self.images, self.mounts, DATA, ARTIFACTS,
                                 self.bind_options if options is None else options, release._fail)

    def test_witness_flag_is_restricted_to_the_official_check(self):
        command = ["/bin/bash", str(ROOT / "scripts/ops/deploy-production-release.sh"),
                   "--commit", COMMIT, "--paper-witness-binding", WITNESS]
        for mode in ("--dry-run", "--apply", "--rollback"):
            result = subprocess.run([*command, mode], capture_output=True, check=False, timeout=5)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(b"--paper-witness-binding requires --check", result.stderr)

    def test_real_render_keeps_both_services_off_and_pins_all_aliases(self):
        self.assert_model(self.render())

    def test_compose_v2_zero_value_retains_no_create_bind_semantics(self):
        model = self.render()
        for name in preflight.SERVICES:
            for volume in model["services"][name]["volumes"]:
                if volume["target"] in self.mounts:
                    volume["bind"] = {}
        self.assert_model(model, options={})
        required = next(volume for volume in model["services"]["api-server"]["volumes"]
                        if volume["target"] in self.mounts)
        del required["bind"]
        with self.assertRaisesRegex(release.PaperReleaseError, "PAPER_RELEASE_PREFLIGHT_MOUNTS"):
            self.assert_model(model, options={})

    def test_real_render_rejects_writeable_swapped_and_missing_mounts(self):
        overlay = self.compose_dir / preflight.OVERLAY
        original = overlay.read_text()
        edits = (
            ("source: ${LAGRANGE_DATA_DIR:?}/owner-beta-paper-evidence/current",
             "source: ${LAGRANGE_DATA_DIR:?}/owner-beta-paper-evidence/current/current.json"),
            ("target: /run/owner-beta-daily/policy.json\n  read_only: true",
             "target: /run/owner-beta-daily/policy.json\n  read_only: false"),
            ("      - *paper-admitted\n", ""),
            ("source: ${LAGRANGE_PAPER_PREFLIGHT_RELEASE:?}/deploy/owner-beta-paper/config.json",
             "source: /foreign/release/deploy/owner-beta-paper/config.json"),
            ("bind: {create_host_path: false}", "bind: {create_host_path: true}"),
        )
        for before, after in edits:
            with self.subTest(before=before):
                self.assertIn(before, original)
                overlay.write_text(original.replace(before, after, 1))
                with self.assertRaisesRegex(release.PaperReleaseError, "PAPER_RELEASE_PREFLIGHT_MOUNTS"):
                    self.assert_model(self.render())

    def test_real_render_rejects_enabled_and_wrong_image(self):
        self.env_path.write_text(self.env_path.read_text().replace("PAPER_MODE=disabled", "PAPER_MODE=enabled"))
        with self.assertRaisesRegex(release.PaperReleaseError, "PAPER_RELEASE_PREFLIGHT_SERVICE"):
            self.assert_model(self.render())
        self.env_path.write_text(self.env_path.read_text().replace("PAPER_MODE=enabled", "PAPER_MODE=disabled"))
        overlay = self.compose_dir / preflight.OVERLAY
        overlay.write_text(overlay.read_text().replace("${LAGRANGE_PAPER_PREFLIGHT_WORKER_IMAGE:?}",
                                                       "sha256:" + "1" * 64))
        with self.assertRaisesRegex(release.PaperReleaseError, "PAPER_RELEASE_PREFLIGHT_SERVICE"):
            self.assert_model(self.render())

    def binding_fixture(self):
        record = {"data_root": DATA, "secret_root": SECRETS, "project": PROJECT,
                  "release_root": self.release, "production_env_file": str(self.env_path),
                  "calendar_dir": "/var/lib/lagrange-paper-fixture/calendar", "database": "lagrange",
                  "evidence_root": DATA + preflight.PAPER_EVIDENCE, "raw_owner_uid": 10002,
                  "production_env_sha256": release._digest(self.env_path.read_bytes()),
                  "daily_config_sha256": DAILY,
                  "cohort": {"verifier_revision": COMMIT, "release_manifest_sha256": self.manifest_sha,
                             "owner_user_id": OWNER, "cohort_sha256": COHORT,
                             "strategy_config_id": STRATEGY, "policy_sha256": POLICY}}
        values = {"LAGRANGE_DATA_DIR": DATA, "LAGRANGE_ARTIFACTS_DIR": ARTIFACTS,
                  "LAGRANGE_RUNTIME_SECRET_DIR": SECRETS, "POSTGRES_DB": "lagrange",
                  "OWNER_BETA_ACCESS_MODE": "owner_only", "OWNER_BETA_PRICE_INPUT_MODE": "sealed_v1",
                  "OWNER_BETA_PAPER_MODE": "disabled"}
        process = release._canonical({"owner_user_id": OWNER, "daily_cohort_sha256": COHORT,
                                      "strategy_config_id": STRATEGY})
        handle = types.SimpleNamespace(commit=COMMIT, manifest_sha256=self.manifest_sha,
                                       process_sha256=release._digest(process), revalidate=mock.Mock())
        source = types.SimpleNamespace(binding=mock.Mock(side_effect=lambda _: (copy.deepcopy(record), b"source")))
        operations = types.SimpleNamespace(read_context=mock.Mock(return_value=(values, [])),
            read_manifest=mock.Mock(return_value=self.images), ENVIRONMENT=self.environment,
            install=types.SimpleNamespace(verify_config=mock.Mock(return_value=({"policy_sha256": POLICY}, DAILY)),
                read_root_file=mock.Mock(side_effect=lambda p, **kw:
                process if p == self.mounts["/run/owner-beta-paper/config.json"] else Path(p).read_bytes())),
            base=types.SimpleNamespace(source_artifact=mock.Mock(return_value=contextlib.nullcontext()),
                files=types.SimpleNamespace(Root=ops.base.files.Root,
                    open_root=mock.Mock(side_effect=lambda root: contextlib.nullcontext()))))
        return record, handle, source, operations

    def test_connected_renderer_rejects_foreign_source_or_missing_original(self):
        record, handle, source, operations = self.binding_fixture()
        with self.assertRaisesRegex(release.PaperReleaseError, "PAPER_RELEASE_PREFLIGHT_PROCESS"):
            preflight.verify(self.release, WITNESS, types.SimpleNamespace(process_sha256=None), source, operations, release)
        for key, foreign in (("release_root", "/foreign/release"),
                             ("cohort_sha256", "sha256:" + "b" * 64),
                             ("strategy_config_id", OWNER),
                             ("policy_sha256", "sha256:" + "e" * 64)):
            with self.subTest(key=key):
                mapping = record if key == "release_root" else record["cohort"]
                original = mapping[key]
                mapping[key] = foreign
                with self.assertRaisesRegex(release.PaperReleaseError, "PAPER_RELEASE_PREFLIGHT_BINDING"):
                    preflight.verify(self.release, WITNESS, handle, source, operations, release)
                mapping[key] = original
        operations.base.files.open_root.side_effect = FileNotFoundError
        with self.assertRaises(FileNotFoundError):
            preflight.verify(self.release, WITNESS, handle, source, operations, release)
        operations.base.files.open_root.side_effect = lambda root: contextlib.nullcontext()
        preflight.verify(self.release, WITNESS, handle, source, operations, release)
        handle.revalidate.assert_called_once_with()

    def test_connected_renderer_rejects_source_changed_during_compose(self):
        record, handle, source, operations = self.binding_fixture()
        source.binding.side_effect = [(record, b"original"), (record, b"other")]
        with self.assertRaisesRegex(release.PaperReleaseError, "PAPER_RELEASE_PREFLIGHT_CHANGED"):
            preflight.verify(self.release, WITNESS, handle, source, operations, release)




    def test_actual_p4_candidate_render_keeps_daily_and_guards_precise_generation(self):
        serving = ops.load_local("paper_serving_preflight_test", "owner-beta-paper-serving.py")
        self.env_path.write_text(self.env_path.read_text()+"OWNER_BETA_ACCESS_MODE=owner_only\n")
        shutil.copyfile(ROOT/"deploy/compose/compose.owner-beta-daily.yml", self.compose_dir/"compose.owner-beta-daily.yml")
        args=["--project-name",PROJECT,"--env-file",str(self.env_path),"-f",str(self.compose_dir/"compose.yml"),
              "-f",str(self.compose_dir/"compose.owner-beta-daily.yml")]
        environment={**self.environment,"LAGRANGE_PAPER_PREFLIGHT_RELEASE":self.release,
                     "LAGRANGE_PAPER_PREFLIGHT_API_IMAGE":self.images["api-server"],
                     "LAGRANGE_PAPER_PREFLIGHT_WORKER_IMAGE":self.images["paper-scheduler"],
                     "LAGRANGE_DAILY_VERIFIED_CONFIG_SHA256":DAILY}
        plan=object.__new__(serving.Plan)
        def require(value,code):
            if not value:
                raise release.PaperReleaseError(code)
        plan.l=types.SimpleNamespace(require=require)
        plan.context=types.SimpleNamespace(project=PROJECT,images=self.images,release_path=self.release,
             install_root="/opt/lagrange",values={"LAGRANGE_DATA_DIR":DATA,"LAGRANGE_ARTIFACTS_DIR":ARTIFACTS})
        plan.env=environment
        plan.original=json.loads(preflight._compose_config(args,environment,release._fail,services=serving.SERVICES))
        plan.files=args+["-f",str(self.compose_dir/preflight.OVERLAY)]
        plan.preflight=preflight
        activation=serving.Activation(OWNER,OWNER,1,STRATEGY,self.manifest_sha,COHORT)
        model,payload=plan.render_enabled(activation)
        self.assertEqual(model["services"]["api-server"]["environment"]["OWNER_BETA_DAILY_MODE"],"owner_only")
        self.assertEqual(model["services"]["api-server"]["environment"]["OWNER_BETA_ACCESS_MODE"],"owner_only")
        self.assertEqual(model["services"]["paper-scheduler"]["labels"][serving.SERVING_LABEL],OWNER)
        self.assertEqual(self.env_path.read_text().split("OWNER_BETA_PAPER_MODE=")[1].splitlines()[0],"disabled")
        original_model=preflight._compose_config
        def foreign(arguments, env, fail, **kwargs):
            rendering=json.loads(original_model(arguments, env, fail, **kwargs))
            rendering["services"]["paper-scheduler"]["labels"][serving.SERVING_LABEL]=STRATEGY
            return json.dumps(rendering).encode()
        with mock.patch.object(preflight,"_compose_config",side_effect=foreign),\
             mock.patch.object(preflight,"no_create_options",return_value=self.bind_options),\
             self.assertRaisesRegex(release.PaperReleaseError,"PAPER_SERVING_MODEL"):
            plan.render_enabled(activation)
        self.assertIn(OWNER.encode(),payload)


if __name__ == "__main__":
    unittest.main()
