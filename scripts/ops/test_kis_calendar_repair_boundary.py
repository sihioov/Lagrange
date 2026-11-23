import copy
from importlib.util import module_from_spec, spec_from_file_location
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).with_name("lib") / "kis-calendar-repair-boundary.py"
SPEC = spec_from_file_location("calendar_boundary", SCRIPT)
BOUNDARY = module_from_spec(SPEC)
SPEC.loader.exec_module(BOUNDARY)
REVISION = "a" * 40
IMAGE = "sha256:" + "b" * 64
STATE = "/fixture/state"
SECRETS = "/fixture/secrets"


def compose_config():
    secret_refs = [
        {"source": f"research_range_raw_{name}", "target": name,
         "uid": "10001", "gid": "10001", "mode": "0440"}
        for name in ("kis_app_key", "kis_app_secret")
    ]
    return {
        "name": BOUNDARY.PROJECT,
        "services": {
            BOUNDARY.SERVICE: {
                "image": IMAGE,
                "command": None,
                "user": "10001:10001",
                "read_only": True,
                "entrypoint": ["/usr/local/bin/kis-action-range-raw"],
                "profiles": ["action-range-raw"],
                "healthcheck": {"disable": True},
                "cap_drop": ["ALL"],
                "security_opt": ["no-new-privileges:true"],
                "networks": {BOUNDARY.NETWORK: None},
                "environment": {
                    "LAGRANGE_CODE_COMMIT": REVISION,
                    "RESEARCH_RAW_ROOT": "/data",
                    "RESEARCH_ENTITLEMENT_REFERENCE": "fixture:licensed",
                    "KIS_APP_KEY_FILE": "/run/secrets/kis_app_key",
                    "KIS_APP_SECRET_FILE": "/run/secrets/kis_app_secret",
                    "KIS_READ_COORDINATION_MODE": "shared_required",
                    "KIS_READ_CREDENTIAL_GENERATION": "1",
                    "OWNER_INTRADAY_QUOTES_MODE": "off",
                },
                "volumes": [
                    {"type": "bind", "source": BOUNDARY.RAW, "target": "/data/raw",
                     "bind": {"create_host_path": False}},
                    {"type": "bind", "source": f"{STATE}/kis-read-coordination",
                     "target": "/run/lagrange/kis-read-coordination",
                     "bind": {"create_host_path": False}},
                ],
                "secrets": secret_refs,
            },
        },
        "networks": {
            BOUNDARY.NETWORK: {
                "name": f"{BOUNDARY.PROJECT}_{BOUNDARY.NETWORK}", "driver": "bridge",
            },
        },
        "secrets": {
            ref["source"]: {
                "file": f"{SECRETS}/research-range-raw/{ref['target']}",
            } for ref in secret_refs
        },
    }


class BoundaryTests(unittest.TestCase):
    def verify(self, config):
        with patch.dict(os.environ, {"RESEARCH_ENTITLEMENT_REFERENCE": "fixture:licensed"}):
            BOUNDARY.verify_compose(config, IMAGE, REVISION, STATE, SECRETS, "1", "off")

    def test_normalized_service_contract_accepts_only_the_expected_isolation(self):
        good = compose_config()
        self.verify(good)
        changes = {
            "image": "mutable:latest",
            "build": {"context": "/fixture"},
            "user": "0:0",
            "networks": {"backend": None},
            "read_only": False,
            "privileged": True,
            "healthcheck": {"test": ["CMD", "another-command"]},
        }
        for key, value in changes.items():
            with self.subTest(key=key):
                bad = copy.deepcopy(good)
                bad["services"][BOUNDARY.SERVICE][key] = value
                with self.assertRaises(ValueError):
                    self.verify(bad)

    def test_rejects_foreign_raw_or_coordination_mounts_and_other_services(self):
        for index in (0, 1):
            bad = compose_config()
            bad["services"][BOUNDARY.SERVICE]["volumes"][index]["source"] = "/foreign"
            with self.assertRaises(ValueError):
                self.verify(bad)
        bad = compose_config()
        bad["services"]["database"] = {}
        with self.assertRaises(ValueError):
            self.verify(bad)

    def test_image_must_be_id_pinned_to_revision_and_worker_identity(self):
        inspection = [{"Id": IMAGE, "Config": {
            "Labels": {"org.opencontainers.image.revision": REVISION},
            "Env": [f"LAGRANGE_CODE_COMMIT={REVISION}"],
            "User": "10001:10001",
        }}]
        self.assertEqual(BOUNDARY.inspect_image(inspection, REVISION), IMAGE)
        config = inspection[0]["Config"]
        config["Env"].append("LAGRANGE_CODE_COMMIT=" + "c" * 40)
        with self.assertRaises(ValueError):
            BOUNDARY.inspect_image(inspection, REVISION)

    def test_host_witness_cannot_be_consumed_again_after_raw_is_absent(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            state = root / "state"
            state.mkdir()
            with patch.object(BOUNDARY, "open_state", side_effect=lambda _: os.open(state, os.O_RDONLY)):
                self.assertEqual(BOUNDARY.witness(str(state), REVISION, IMAGE, False), "unused")
                self.assertEqual(BOUNDARY.witness(str(state), REVISION, IMAGE, True), "consumed")
                self.assertTrue((state / BOUNDARY.MARKER).is_file())
                with self.assertRaises(FileExistsError):
                    BOUNDARY.witness(str(state), "c" * 40, IMAGE, True)

    def test_rejects_unexpected_or_non_raw_native_status(self):
        valid = {"target_date": BOUNDARY.TARGET, "raw_only": True,
                 "outcome": "WOULD_CAPTURE_NO_RESERVATION"}
        self.assertEqual(BOUNDARY.check_report(valid), valid["outcome"])
        with self.assertRaises(ValueError):
            BOUNDARY.check_report({**valid, "outcome": "CAPTURED"})
        with self.assertRaises(ValueError):
            BOUNDARY.check_report({**valid, "raw_only": False})


if __name__ == "__main__":
    unittest.main()
