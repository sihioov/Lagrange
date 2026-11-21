"""Opt-in installer checks with protected-read and service-manager test doubles."""
from __future__ import annotations

import contextlib
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import stat
import tempfile
import unittest
from unittest import mock

OPS = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("nightly_installer", OPS / "install-nightly-simulation-backup.py")
installer = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(installer)
ROOT = OPS.parent.parent
APP, TOOL = "a" * 40, "b" * 40
NEXT = "c" * 40
RELEASE = installer.INSTALL / "releases" / APP
PIN = installer.PINS / "nightly-inactive.conf"
CRED = installer.PINS / "nightly-simulation" / APP / installer.CREDENTIAL


class NightlyDeploymentTests(unittest.TestCase):
    def setUp(self):
        self.maintenance = {
            "BACKUP_ROOT": "/srv/backups/lagrange-nightly",
            "DATA_ROOT": "/var/lib/lagrange/data",
            "LOCK_FILE": "/var/lib/lagrange/backup-state/nightly-inactive.lock",
            "METRICS_FILE": "/srv/backups/lagrange-nightly/backup.prom",
            "DAILY_BACKUP_TOOL_COMMIT": TOOL,
            "DAILY_BACKUP_TOOL_MANIFEST": "sha256:" + "d" * 64,
            "LAGRANGE_CODE_COMMIT": APP,
        }
        self.config = {
            "schema": "lagrange-nightly-simulation-backup-v1",
            "application_commit": APP,
            "tool_commit": TOOL,
            "config_file": str(PIN),
            "config_sha256": "",
            "expected_db_first_session": "2026-10-21",
            "expected_config_first_session": "2026-10-22",
        }
        self.inputs = {Path(installer.LEGACY): b"BACKUP_ROOT=/srv/backups/lagrange\n",
                       RELEASE / installer.ENTRYPOINT: b"installed nightly",
                       RELEASE / "scripts/ops/preupgrade-simulation-backup.py": b"installed sequence",
                       RELEASE / "scripts/ops/compose-release.sh": b"installed narrow wrapper",
                       RELEASE / installer.TEMPLATE: (ROOT / installer.TEMPLATE).read_bytes()}
        for unit in (installer.SERVICE, installer.TIMER):
            source = (ROOT / "deploy/systemd" / unit).read_bytes()
            self.inputs[installer.SYSTEMD / unit] = self.inputs[RELEASE / "deploy/systemd" / unit] = source
        self.bind_config()
        self.start(mock.patch.object(installer, "__file__",
                                    str(RELEASE / "scripts/ops/install-nightly-simulation-backup.py")))
        self.start(mock.patch.object(installer, "read", side_effect=self.read))
        self.current = self.start(mock.patch.object(installer.policy, "validate_current_release"))
        self.manifest = self.start(mock.patch.object(installer.policy, "validate_manifest"))
        self.start(mock.patch.object(installer, "check_directory"))
        self.tool = self.start(mock.patch.object(installer.package, "read_manifest"))

    def start(self, patch):
        self.addCleanup(patch.stop)
        return patch.start()

    def bind_config(self):
        raw = "".join(f"{key}={value}\n" for key, value in self.maintenance.items()).encode()
        self.config["config_sha256"] = "sha256:" + hashlib.sha256(raw).hexdigest()
        self.inputs[PIN] = raw
        self.inputs[Path(installer.CONFIG)] = json.dumps(self.config).encode()

    def read(self, path, mode=None, maximum=None):
        try:
            return self.inputs[Path(path)]
        except KeyError:
            raise FileNotFoundError from None

    def test_template_keeps_canonical_timer_and_legacy_writes_out(self):
        release, credential, raw, _ = installer.deployment()
        self.assertEqual(release, RELEASE)
        self.tool.assert_called_once_with(installer.INSTALL / "maintenance" / TOOL, APP,
                                          install_root=installer.INSTALL, expected_hash="sha256:" + "d" * 64)
        self.assertIsNone(credential)
        unit = raw.decode()
        command = str(RELEASE / installer.ENTRYPOINT)
        self.assertIn(f"ExecStart=/usr/bin/python3 -I -S -B {command}", unit)
        self.assertIn("--config-file /etc/lagrange/nightly-simulation-backup.json --apply\n", unit)
        self.assertNotIn("ExecStartPre", unit)
        self.assertNotIn("LoadCredential", unit)
        self.assertIn("ReadWritePaths=/opt/lagrange /run /var/lib/lagrange/backup-state/nightly-simulation", unit)
        self.assertIn("ReadOnlyPaths=/opt/lagrange/releases /opt/lagrange/maintenance /opt/lagrange/bin", unit)
        self.assertIn("ReadWritePaths=/srv/backups/lagrange-nightly /var/lib/lagrange/backup-state", unit)
        self.assertNotIn("EnvironmentFile", unit)

    def test_credential_uses_exact_release_source_and_never_embeds_contents(self):
        self.inputs[CRED] = b"private fixture bytes"
        _, credential, unit, pin = installer.deployment()
        self.assertEqual(credential, CRED)
        self.assertIn(f"LoadCredential={installer.CREDENTIAL}:{CRED}".encode(), unit)
        self.assertNotIn(self.inputs[CRED], unit)
        self.inputs[CRED] = b"changed independent confirmation"
        self.assertNotEqual(installer.deployment()[3], pin)

    def test_fails_for_legacy_unit_drift_missing_release_components_and_config_hash(self):
        target = installer.SYSTEMD / installer.SERVICE
        original = self.inputs[target]
        self.inputs[target] += b"changed"
        with self.assertRaisesRegex(installer.Refused, "LEGACY_UNIT"):
            installer.deployment()
        self.inputs[target] = original
        backup = self.inputs.pop(RELEASE / installer.ENTRYPOINT)
        with self.assertRaises(FileNotFoundError):
            installer.deployment()
        self.inputs[RELEASE / installer.ENTRYPOINT] = backup
        self.inputs[PIN] += b"# modified config"
        with self.assertRaisesRegex(installer.Refused, "CONFIG_HASH"):
            installer.deployment()

    def test_refuses_overlapping_legacy_archive_and_ambiguous_unit_path(self):
        for target in ("/srv/backups/lagrange", "/srv/backups/lagrange/nightly"):
            self.maintenance["BACKUP_ROOT"] = target
            self.maintenance["METRICS_FILE"] = target + "/backup.prom"
            self.bind_config()
            with self.assertRaisesRegex(installer.Refused, "BACKUP_ROOT_OVERLAP"):
                installer.deployment()
        with self.assertRaisesRegex(installer.Refused, "CONFIG_PATH"):
            installer.unit_path("/srv/backups/%n")

    def test_retirement_reconstructs_old_unit_after_current_advanced(self):
        self.inputs[RELEASE / ".lagrange-release"] = (APP + "\n").encode()
        old_without = installer.deployment()[2]
        self.inputs[CRED] = b"old explicit confirmation"
        old_with = installer.deployment()[2]
        self.inputs.pop(CRED)  # Revoking a credential does not change the old unit bytes.
        self.inputs.pop(Path(installer.LEGACY))
        self.current.reset_mock()
        next_release = installer.INSTALL / "releases" / NEXT
        with mock.patch.object(installer, "__file__",
                               str(next_release / "scripts/ops/install-nightly-simulation-backup.py")):
            self.assertEqual(installer.deployment(retiring=True), (RELEASE, (old_without, old_with)))
        self.current.assert_called_once_with(str(installer.INSTALL), str(next_release), NEXT)
        self.manifest.assert_called_once_with(str(RELEASE / ".lagrange-release-manifest"), APP)
        self.current.reset_mock()
        installer.deployment(retiring=True)
        self.current.assert_not_called()


class NightlyUnitInstallationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.state = self.root / "state"
        self.target = self.root / (installer.SERVICE + ".d") / installer.DROP_IN
        self.data = b"protected drop-in\n"
        self.selected = (RELEASE, CRED, self.data, (b"config digest", b"credential digest"))
        self.calls = []
        self.reloaded = False
        self.active = "inactive"
        self.enabled = "disabled"
        self.other = ""
        self.jobs = ""
        self.reload_fails = False
        self.start(mock.patch.object(installer, "SYSTEMD", self.root))
        self.start(mock.patch.object(installer, "STATE", self.state))
        self.deploy = self.start(mock.patch.object(installer, "deployment", side_effect=self.deployment))
        self.start(mock.patch.object(installer, "release_lock", side_effect=contextlib.nullcontext))
        self.start(mock.patch.object(installer, "run", side_effect=self.run_command))
        self.start(mock.patch.object(installer, "show", side_effect=self.show))
        self.start(mock.patch.object(installer, "read", side_effect=lambda path, _mode: path.read_bytes()))
        self.start(mock.patch.object(installer.files, "root_directory", side_effect=self.directory))
        self.start(mock.patch.object(installer.files, "create_file", side_effect=self.create_file))
        self.start(mock.patch.object(installer.files, "directory_info", side_effect=self.directory_info))

    def start(self, patch):
        self.addCleanup(patch.stop)
        return patch.start()

    @contextlib.contextmanager
    def directory(self, path):
        fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
        try:
            yield fd
        finally:
            os.close(fd)

    def directory_info(self, fd, *, gid, mode):
        self.assertEqual(gid, 0)
        if mode is not None:
            self.assertEqual(stat.S_IMODE(os.fstat(fd).st_mode), mode)

    def create_file(self, parent, name, data, _uid, _gid, mode):
        fd = os.open(name, os.O_CREAT | os.O_EXCL | os.O_WRONLY, mode, dir_fd=parent)
        try:
            os.fchmod(fd, mode)
            os.write(fd, data)
        finally:
            os.close(fd)

    def show(self, unit, _properties):
        return {"LoadState": "loaded", "FragmentPath": str(self.root / unit),
                "DropInPaths": ((str(self.target) if self.target.exists() else "") if self.reloaded else self.other)
                    if unit == installer.SERVICE else "",
                "NeedDaemonReload": "no", "ActiveState": self.active if unit == installer.TIMER else "failed",
                "UnitFileState": self.enabled if unit == installer.TIMER else "disabled"}

    def deployment(self, *, retiring=False):
        if retiring:
            return RELEASE, (self.data,)
        return self.selected

    def run_command(self, argv, **kwargs):
        self.calls.append((argv, kwargs))
        if argv == ("/usr/bin/systemctl", "daemon-reload"):
            if self.reload_fails:
                raise installer.Refused("COMMAND")
            self.reloaded = True
        if argv[1] == "list-jobs":
            return self.jobs
        return ""

    def installed_old(self, data=None):
        self.state.mkdir(mode=0o700)
        self.target.parent.mkdir()
        self.target.write_bytes(self.data if data is None else data)
        self.other = str(self.target)
        return self.target.with_name(installer.DROP_IN + ".retired." + APP)

    def test_installs_without_touching_existing_state_or_activating_units(self):
        self.state.mkdir(mode=0o700)
        existing = self.state / "existing.json"
        existing.write_bytes(b"state owner retains")
        mask = os.umask(0o077)
        try:
            installer.install()
        finally:
            os.umask(mask)
        self.assertEqual(self.target.read_bytes(), self.data)
        self.assertEqual(stat.S_IMODE(self.target.parent.stat().st_mode), 0o755)
        self.assertEqual(existing.read_bytes(), b"state owner retains")
        self.assertEqual(self.calls, [
            ((*installer.PYTHON, str(RELEASE / installer.ENTRYPOINT), "--config-file", installer.CONFIG,
              "--preflight"), {"env": dict(installer.ENV, CREDENTIALS_DIRECTORY=str(CRED.parent)), "timeout": 26 * 60}),
            (("/usr/bin/systemctl", "daemon-reload"), {}),
        ])
        self.active, self.enabled = "active", "enabled"
        installer.check_destination(self.target, self.data)

    def test_refuses_live_timer_foreign_drop_in_and_existing_target_before_writes(self):
        self.active, self.enabled = "active", "enabled"
        with self.assertRaisesRegex(installer.Refused, "SYSTEMD_ACTIVE_OR_ENABLED"):
            installer.install()
        self.active, self.enabled, self.other = "inactive", "disabled", "/another/override.conf"
        with self.assertRaisesRegex(installer.Refused, "SYSTEMD_UNIT"):
            installer.install()
        self.target.parent.mkdir()
        self.target.write_bytes(b"foreign operator bytes")
        with self.assertRaisesRegex(installer.Refused, "DROP_IN_EXISTS"):
            installer.install()
        self.assertEqual(self.target.read_bytes(), b"foreign operator bytes")
        self.assertFalse(self.state.exists())
        self.assertEqual(self.calls, [])

    def test_install_preflight_failure_does_not_publish_and_does_not_clobber_state(self):
        with mock.patch.object(installer, "run", side_effect=installer.Refused("COMMAND")):
            with self.assertRaisesRegex(installer.Refused, "COMMAND"):
                installer.install()
        self.assertEqual(stat.S_IMODE(self.state.stat().st_mode), 0o700)
        self.assertFalse(self.target.parent.exists())

    def test_config_drift_during_preflight_blocks_publication(self):
        drift = (*self.selected[:3], (b"different config digest", b"credential digest"))
        with mock.patch.object(installer, "deployment", side_effect=(self.selected, drift)):
            with self.assertRaisesRegex(installer.Refused, "DEPLOYMENT_CHANGED"):
                installer.install()
        self.assertFalse(self.target.parent.exists())
        self.assertEqual(len(self.calls), 1)

    def test_old_to_new_preserves_retired_bytes_and_scheduler_history(self):
        retired = self.installed_old()
        history = self.state / "2026-10-21"
        history.mkdir()
        (history / "claim.json").write_bytes(b"protected claim")
        installer.retire()
        self.assertFalse(self.target.exists())
        self.assertEqual(retired.read_bytes(), self.data)
        self.assertEqual((history / "claim.json").read_bytes(), b"protected claim")
        self.assertEqual(self.deploy.call_args, mock.call(retiring=True))
        next_release = installer.INSTALL / "releases" / NEXT
        self.selected = next_release, None, b"new selected override\n", (b"new config", b"")
        installer.install()
        self.assertEqual(self.target.read_bytes(), b"new selected override\n")
        self.assertEqual(retired.read_bytes(), self.data)
        self.assertEqual((history / "claim.json").read_bytes(), b"protected claim")
        self.assertFalse(any("start" in argv or "enable" in argv for argv, _ in self.calls))

    def test_retire_refuses_tampered_source_foreign_override_or_existing_archive(self):
        retired = self.installed_old(b"changed")
        with self.assertRaisesRegex(installer.Refused, "DROP_IN_CHANGED"):
            installer.retire()
        self.assertEqual(self.target.read_bytes(), b"changed")
        self.assertFalse(retired.exists())
        self.target.write_bytes(self.data)
        self.other = str(self.target) + " /foreign.conf"
        with self.assertRaisesRegex(installer.Refused, "SYSTEMD_UNIT"):
            installer.retire()
        self.other = str(self.target)
        retired.write_bytes(b"foreign archive")
        with self.assertRaisesRegex(installer.Refused, "RETIRE_DESTINATION_EXISTS"):
            installer.retire()
        self.assertEqual(self.target.read_bytes(), self.data)
        self.assertEqual(retired.read_bytes(), b"foreign archive")

    def test_retire_refuses_active_timer_queued_job_or_held_scheduler_lock(self):
        retired = self.installed_old()
        self.active = "active"
        with self.assertRaisesRegex(installer.Refused, "SYSTEMD_ACTIVE_OR_ENABLED"):
            installer.retire()
        self.active = "inactive"
        self.jobs = f"17 {installer.SERVICE} start waiting\n"
        with self.assertRaisesRegex(installer.Refused, "SYSTEMD_ACTIVE_OR_ENABLED"):
            installer.retire()
        self.jobs = ""
        with self.directory(self.state) as fd:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with self.assertRaisesRegex(installer.Refused, "SCHEDULER_BUSY"):
                installer.retire()
        self.assertFalse(retired.exists())
        self.assertEqual(self.target.read_bytes(), self.data)

    def test_retire_reload_failure_keeps_durable_archive_for_explicit_retry(self):
        retired = self.installed_old()
        self.reload_fails = True
        with self.assertRaisesRegex(installer.Refused, "RETIRED_CHECK_REQUIRED"):
            installer.retire()
        self.assertFalse(self.target.exists())
        self.assertEqual(retired.read_bytes(), self.data)
        self.reload_fails = False
        retired.write_bytes(b"changed during recovery")
        with self.assertRaisesRegex(installer.Refused, "DROP_IN_CHANGED"):
            installer.retire()
        retired.write_bytes(self.data)
        installer.retire()
        self.assertFalse(self.target.exists())
        self.assertEqual(retired.read_bytes(), self.data)
        self.assertTrue(self.reloaded)


if __name__ == "__main__":
    unittest.main()
