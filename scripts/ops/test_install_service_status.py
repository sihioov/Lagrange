from __future__ import annotations

import importlib.util
import json
import os
import stat
import subprocess
import sys
import tempfile
import unittest
from datetime import datetime, timezone
from pathlib import Path
from types import SimpleNamespace


ROOT = Path(__file__).resolve().parents[2]
FIXTURES = ROOT / "tests/fixtures/service-status"


def wire_cases() -> dict[str, dict]:
    cases = json.loads((FIXTURES / "wire-cases.json").read_text())["cases"]
    return {case["id"]: case for case in cases}


def load_module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError("test-module-unavailable")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


policy = load_module("test_install_service_status_policy", ROOT / "scripts/ops/lib/service-status-policy.py")
collector = load_module("test_install_collect_service_status", ROOT / "scripts/ops/collect-service-status.py")
installer = load_module("test_install_service_status_module", ROOT / "scripts/ops/install-service-status.py")


def ids() -> tuple[int, int]:
    return os.geteuid(), os.getegid()


class FakeRunner:
    def __init__(self, *, research_mode: str = "daily"):
        self.research_mode = research_mode
        self.calls: list[tuple[str, ...]] = []
        self.fail_on: str | None = None
        self.fail_rollback = False
        self.activation_attempted = False
        self.systemd_outputs: dict[str, bytes] = {}
        self.overrides: dict[str, dict[str, str]] = {}
        self.observer_states = {
            "lagrange-service-status.timer": {"loaded": False, "active": "inactive", "enabled": "disabled"},
            "lagrange-service-status.service": {"loaded": False, "active": "inactive", "enabled": "disabled"},
        }

    def __call__(self, argv: tuple[str, ...], **kwargs):
        self.calls.append(argv)
        if argv[0] == "/bin/bash":
            return SimpleNamespace(returncode=0, stdout=None)
        if argv[0] == "/usr/bin/systemctl" and argv[1:2] == ("show",):
            unit = argv[-1]
            is_timer = unit.endswith(".timer")
            properties = collector.SYSTEMD_TIMER_PROPERTIES if is_timer else collector.SYSTEMD_SERVICE_PROPERTIES
            values = {key: "" for key in properties}
            if unit in self.observer_states:
                state = self.observer_states[unit]
                file_exists = Path(self.systemd_root, unit).is_file()
                loaded = bool(state["loaded"] and file_exists)
                values.update({
                    "LoadState": "loaded" if loaded else "not-found",
                    "ActiveState": state["active"],
                    "UnitFileState": state["enabled"] if loaded else "not-found",
                    "SubState": "waiting" if is_timer else "dead",
                    "Result": "success",
                    "FragmentPath": os.path.join(self.systemd_root, unit) if loaded else "",
                    "DropInPaths": "",
                })
                if is_timer:
                    values.update({"Unit": "lagrange-service-status.service" if loaded else "", "LastTriggerUSec": "", "NextElapseUSecRealtime": ""})
                else:
                    values.update({
                        "ExecMainCode": "0", "ExecMainStatus": "0",
                        "ExecMainStartTimestamp": "", "ExecMainExitTimestamp": "",
                    })
                raw = "".join(
                    f"{key}={values[key]}\n" for key in properties
                ).encode()
                self.systemd_outputs[unit] = raw
                return SimpleNamespace(returncode=0, stdout=raw)
            values.update({
                "LoadState": "loaded",
                "ActiveState": "active" if self.research_mode == "daily" else "inactive",
                "SubState": "waiting" if is_timer else "dead",
                "UnitFileState": "enabled" if self.research_mode == "daily" else "disabled",
                "Result": "success",
                "FragmentPath": os.path.join(self.systemd_root, unit),
                "DropInPaths": "",
            })
            if is_timer:
                values["Unit"] = unit[:-6] + ".service"
                values.update({
                    "LastTriggerUSec": "",
                    "NextElapseUSecRealtime": "2032-02-12T02:00:00Z" if self.research_mode == "daily" else "",
                })
            else:
                values.update({
                    "ExecMainCode": "1",
                    "ExecMainStatus": "0",
                    "ExecMainStartTimestamp": "",
                    "ExecMainExitTimestamp": "",
                })
            values.update(self.overrides.get(unit, {}))
            raw = "".join(
                f"{key}={values[key]}\n" for key in properties
            ).encode()
            self.systemd_outputs[unit] = raw
            return SimpleNamespace(returncode=0, stdout=raw)
        if argv[0] == "/usr/bin/systemctl" and argv[1:2] == ("daemon-reload",):
            if self.fail_rollback and self.activation_attempted:
                return SimpleNamespace(returncode=1, stdout=None)
            for unit, state in self.observer_states.items():
                state["loaded"] = Path(self.systemd_root, unit).is_file()
            return SimpleNamespace(returncode=1 if self.fail_on == "daemon-reload" else 0, stdout=None)
        if argv[0] == "/usr/bin/systemctl" and argv[1:2] == ("enable",):
            unit = argv[-1]
            if "--now" in argv:
                self.activation_attempted = True
            if self.fail_rollback and self.activation_attempted:
                return SimpleNamespace(returncode=1, stdout=None)
            if unit in self.observer_states:
                self.observer_states[unit]["enabled"] = "enabled"
                self.observer_states[unit]["loaded"] = True
            if "--now" in argv and unit == "lagrange-service-status.timer":
                self.observer_states[unit]["active"] = "active"
                if self.fail_on == "enable_partial":
                    self.observer_states["lagrange-service-status.service"]["active"] = "active"
                    return SimpleNamespace(returncode=1, stdout=None)
            return SimpleNamespace(returncode=1 if self.fail_on == "enable" else 0, stdout=None)
        if argv[0] == "/usr/bin/systemctl" and argv[1:2] in (("disable",), ("start",), ("stop",)):
            verb = argv[1]
            unit = argv[-1]
            if self.fail_rollback and self.activation_attempted:
                return SimpleNamespace(returncode=1, stdout=None)
            if unit in self.observer_states:
                state = self.observer_states[unit]
                if verb == "disable":
                    state["enabled"] = "disabled"
                    if "--now" in argv:
                        state["active"] = "inactive"
                elif verb == "start":
                    state["active"] = "active"
                else:
                    state["active"] = "inactive"
            return SimpleNamespace(returncode=0, stdout=None)
        return SimpleNamespace(returncode=1, stdout=b"")

    @property
    def systemd_root(self) -> str:
        return self._systemd_root

    @systemd_root.setter
    def systemd_root(self, value: str) -> None:
        self._systemd_root = value


class InstallerFixture(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="service-status-install.")
        self.base = Path(self.temp.name)
        self.owner = ids()
        self.commit = "a" * 40
        self.install_root = self.base / "install"
        self.release_root = self.install_root / "releases" / self.commit
        self.release_root.mkdir(parents=True, mode=0o755)
        self.release_root.chmod(0o755)
        self.install_root.chmod(0o755)
        (self.install_root / "releases").chmod(0o755)
        self.current = self.install_root / "current"
        self.current.symlink_to(f"releases/{self.commit}")
        self.write_manifest()
        self.release_ops = self.release_root / "scripts/ops"
        self.release_ops.mkdir(parents=True)
        (self.release_root / "scripts").chmod(0o755)
        self.release_ops.chmod(0o755)
        kis_check = self.release_ops / "install-kis-daily.sh"
        kis_check.write_text("#!/usr/bin/env bash\nexit 0\n")
        kis_check.chmod(0o755)
        wrapper = self.release_ops / "kis-daily-production.sh"
        wrapper.write_text("#!/usr/bin/env bash\nexit 0\n")
        wrapper.chmod(0o755)
        self.systemd_root = self.base / "systemd"
        self.systemd_root.mkdir(mode=0o755)
        self.systemd_root.chmod(0o755)
        short = self.commit[:7]
        service_name = f"lagrange-kis-daily-{short}.service"
        timer_name = f"lagrange-kis-daily-{short}.timer"
        service = (
            "[Unit]\n"
            f"ConditionFileIsExecutable={wrapper}\n\n"
            "[Service]\n"
            f"Environment=LAGRANGE_CODE_COMMIT={self.commit}\n"
            f"ExecStart={wrapper} --execute\n"
        )
        timer = "[Timer]\n" + f"Unit={service_name}\n"
        for name, content in ((service_name, service), (timer_name, timer)):
            file = self.systemd_root / name
            file.write_text(content)
            file.chmod(0o644)
        deploy_systemd = self.release_root / "deploy/systemd"
        deploy_systemd.mkdir(parents=True)
        (self.release_root / "deploy").chmod(0o755)
        deploy_systemd.chmod(0o755)
        for name in ("lagrange-service-status.service", "lagrange-service-status.timer"):
            (deploy_systemd / name).write_bytes((ROOT / "deploy/systemd" / name).read_bytes())
            (deploy_systemd / name).chmod(0o644)
        compose = self.release_root / "deploy/compose"
        compose.mkdir(parents=True)
        compose.chmod(0o755)
        compose_text = (
            'services:\n'
            '  candidate-runner:\n    environment:\n      CANDIDATE_LEASE_MS: ${CANDIDATE_LEASE_MS:-60000}\n'
            '  owner-beta-runner:\n    environment:\n      PRICE_BETA_LEASE_SECS: ${PRICE_BETA_LEASE_SECS:-60}\n'
            '  owner-equity-v2-runner:\n    environment:\n      OWNER_EQUITY_V2_LEASE_SECS: "75"\n'
        )
        (compose / "compose.yml").write_text(compose_text)
        (compose / "compose.yml").chmod(0o644)
        queue_source = self.release_root / "crates/job-queue/src/queue.rs"
        queue_source.parent.mkdir(parents=True)
        for directory in (
            self.release_root / "crates", self.release_root / "crates/job-queue",
            self.release_root / "crates/job-queue/src",
        ):
            directory.chmod(0o755)
        queue_source.write_text(
            'impl Default for QueueConfig {\n    fn default() -> Self {\n'
            '        QueueConfig {\n            lease: Duration::from_secs(60),\n'
        )
        queue_source.chmod(0o644)
        recommendation_source = self.release_root / "crates/job-queue/src/bin/recommendation-runner.rs"
        recommendation_source.parent.mkdir(parents=True)
        (self.release_root / "crates/job-queue/src/bin").chmod(0o755)
        recommendation_source.write_text(
            'impl Default for Args {\n    fn default() -> Self {\n        Self {\n'
            '            lease: Duration::from_secs(60),\n'
        )
        recommendation_source.chmod(0o644)
        env_values = {
            "LAGRANGE_CODE_COMMIT": self.commit,
            "OWNER_INTRADAY_QUOTES_MODE": "owner_only",
            "OWNER_INTRADAY_QUOTE_TRANSPORT": "market_ws",
            "OWNER_INTRADAY_SESSION_WINDOWS_SOURCE": "operational_v1",
            "KIS_READ_COORDINATION_MODE": "shared_required",
            "OWNER_EQUITY_V2_RUNTIME_MODE": "owner_only",
            "OWNER_BETA_ACCESS_MODE": "owner_only",
            "OWNER_BETA_PAPER_MODE": "disabled",
            "CANDIDATE_LEASE_MS": "90000",
            "PRICE_BETA_LEASE_SECS": "45",
            "COMPOSE_PROFILES": "live",
        }
        self.env_file = compose / ".env"
        self.env_file.write_text("".join(f"{key}={value}\n" for key, value in env_values.items()))
        self.env_file.chmod(0o600)
        self.private_root = self.base / "var/lib/lagrange/service-status"
        self.public_root = self.private_root / "public"
        self.lock_file = self.base / "run/lagrange-service-status/collector.lock"
        self.paths = installer.InstallPaths(
            install_root=str(self.install_root), release_root=str(self.release_root),
            env_file=str(self.env_file), systemd_root=str(self.systemd_root),
            private_root=str(self.private_root), public_root=str(self.public_root),
            policy_file=str(self.private_root / "policy.json"), history_file=str(self.private_root / "history.json"),
            lock_file=str(self.lock_file), test_owner=self.owner, api_gid=self.owner[1],
        )
        self.runner = FakeRunner()
        self.runner.systemd_root = str(self.systemd_root)

    def tearDown(self) -> None:
        self.temp.cleanup()

    def write_manifest(self, commit: str | None = None) -> None:
        commit = commit or self.commit
        lines = [policy.MANIFEST_FORMAT, "commit|" + commit]
        for service in policy.MANIFEST_SERVICES:
            lines.append(f"image|{service}|lagrange-station-{service}:{commit}|sha256:{'1' * 64}|{commit}")
        path = self.release_root / ".lagrange-release-manifest"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("\n".join(lines) + "\n")
        path.chmod(0o600)

    def run_install(self, *, mode: str = "daily", apply: bool = False, runner=None):
        chosen = runner or self.runner
        chosen.systemd_root = str(self.systemd_root)
        return installer.install_service_status(
            self.paths,
            research_mode=mode,
            compose_project="lagrange-station",
            now=datetime(2032, 2, 10, tzinfo=timezone.utc),
            runner=chosen,
            apply=apply,
        )


class InstallerPlanAndPreflightTests(InstallerFixture):
    def test_default_cli_is_plan_only_and_provider_free(self) -> None:
        result = subprocess.run(
            [sys.executable, str(ROOT / "scripts/ops/install-service-status.py"),
             "--compose-project", "lagrange-station", "--research-mode", "daily"],
            cwd=ROOT, check=True, capture_output=True, text=True,
        )
        self.assertIn("SERVICE_STATUS_PLAN", result.stdout)
        self.assertIn("PLAN_ONLY", result.stdout)
        self.assertNotIn("/var/lib/lagrange/service-status/policy.json", result.stdout)

    def test_preflight_is_read_only_and_reads_full_commit_only(self) -> None:
        result = self.run_install()
        self.assertEqual(result["policy"]["release_commit"], self.commit)
        self.assertEqual(result["policy"]["profile"], "owner_daily")
        self.assertEqual(result["policy"]["job_lease_seconds"], {
            "owner_equity_v2": 75,
            "owner_beta_price_recommendation": 45,
            "candidate_compute": 90,
            "recommendation": 60,
            "backtest": 60,
        })
        self.assertFalse(self.private_root.exists())
        self.assertFalse((self.systemd_root / "lagrange-service-status.service").exists())
        self.assertEqual([c[0:2] for c in self.runner.calls], [
            ("/bin/bash", str(self.release_ops / "install-kis-daily.sh")),
            ("/usr/bin/systemctl", "show"),
            ("/usr/bin/systemctl", "show"),
        ])
        self.assertTrue(all("COMPOSE_PROFILES" not in " ".join(call) for call in self.runner.calls))

    def test_lease_receipt_uses_effective_dotenv_compose_and_immutable_defaults(self) -> None:
        env = self.env_file.read_text().replace("CANDIDATE_LEASE_MS=90000\n", "").replace("PRICE_BETA_LEASE_SECS=45\n", "")
        self.env_file.write_text(env)
        compose = self.release_root / "deploy/compose/compose.yml"
        compose.write_text(compose.read_text().replace(":-60000", ":-30000").replace(":-60}", ":-40}"))
        queue = self.release_root / "crates/job-queue/src/queue.rs"
        queue.write_text(queue.read_text().replace("from_secs(60)", "from_secs(74)"))
        recommendation = self.release_root / "crates/job-queue/src/bin/recommendation-runner.rs"
        recommendation.write_text(recommendation.read_text().replace("from_secs(60)", "from_secs(73)"))
        result = self.run_install()
        self.assertEqual(result["policy"]["job_lease_seconds"], {
            "owner_equity_v2": 75,
            "owner_beta_price_recommendation": 40,
            "candidate_compute": 30,
            "recommendation": 73,
            "backtest": 74,
        })

    def test_old_daily_unit_is_not_a_fallback(self) -> None:
        case = wire_cases()["old_daily_unit"]
        old_commit = "b" * 40
        old_service = f"lagrange-kis-daily-{old_commit[:7]}.service"
        old_timer = f"lagrange-kis-daily-{old_commit[:7]}.timer"
        (self.systemd_root / old_service).write_text(
            f"Environment=LAGRANGE_CODE_COMMIT={old_commit}\nExecStart=/old/release/kis-daily-production.sh --execute\n"
        )
        (self.systemd_root / old_timer).write_text(f"Unit={old_service}\n")
        for old_file in (old_service, old_timer):
            (self.systemd_root / old_file).chmod(0o644)
        current_timer = f"lagrange-kis-daily-{self.commit[:7]}.timer"
        self.runner.overrides[current_timer] = {
            "LoadState": "not-found", "ActiveState": "inactive", "UnitFileState": "not-found",
            "FragmentPath": "", "DropInPaths": "", "Unit": "",
        }
        with self.assertRaises(installer.InstallError) as caught:
            self.run_install()
        self.assertEqual(caught.exception.code, case["expected"]["reason_code"])
        self.assertFalse(case["expected"]["accept"])
        self.assertFalse(case["expected"]["raw_output_emitted"])
        self.assertFalse(self.private_root.exists())
        missing_timer = collector._parse_key_values(
            self.runner.systemd_outputs[current_timer], collector.SYSTEMD_TIMER_PROPERTIES,
        )
        self.assertEqual(missing_timer["LoadState"], "not-found")
        self.assertNotIn("ExecMainCode", missing_timer)

    def test_missing_daily_service_uses_service_shape_and_fails_closed(self) -> None:
        service_name = f"lagrange-kis-daily-{self.commit[:7]}.service"
        self.runner.overrides[service_name] = {
            "LoadState": "not-found", "ActiveState": "inactive", "UnitFileState": "not-found",
            "FragmentPath": "", "DropInPaths": "", "ExecMainCode": "0", "ExecMainStatus": "0",
        }
        with self.assertRaises(installer.InstallError) as caught:
            self.run_install()
        self.assertEqual(caught.exception.code, "POLICY_MISMATCH")
        missing_service = collector._parse_key_values(
            self.runner.systemd_outputs[service_name], collector.SYSTEMD_SERVICE_PROPERTIES,
        )
        self.assertEqual(missing_service["LoadState"], "not-found")
        self.assertNotIn("Unit", missing_service)
        self.assertNotIn("LastTriggerUSec", missing_service)

    def test_no_same_prefix_short_commit_collision(self) -> None:
        case = wire_cases()["short_commit_collision"]
        timer = f"lagrange-kis-daily-{self.commit[:7]}.timer"
        service = f"lagrange-kis-daily-{self.commit[:7]}.service"
        service_path = self.systemd_root / service
        collision_commit = self.commit[:7] + "b" * 33
        service_path.write_text(service_path.read_text().replace(self.commit, collision_commit))
        with self.assertRaises(installer.InstallError) as caught:
            self.run_install()
        self.assertEqual(caught.exception.code, case["expected"]["reason_code"])
        self.assertFalse(case["expected"]["accept"])
        self.assertFalse(case["expected"]["raw_output_emitted"])
        self.assertIn(timer, str(self.systemd_root / timer))
        self.assertFalse(self.private_root.exists())

    def test_daily_timer_health_is_observation_evidence_not_install_precondition(self) -> None:
        timer_name = f"lagrange-kis-daily-{self.commit[:7]}.timer"
        self.runner.overrides[timer_name] = {
            "ActiveState": "inactive", "UnitFileState": "disabled", "NextElapseUSecRealtime": "",
        }
        result = self.run_install(apply=True)
        self.assertEqual(result["policy"]["research_mode"], "daily")
        self.assertEqual(self.runner.observer_states["lagrange-service-status.timer"]["active"], "active")
        mutations = [call for call in self.runner.calls if call[0] == "/usr/bin/systemctl" and call[1] != "show"]
        self.assertTrue(all("lagrange-kis-daily-" not in " ".join(call) for call in mutations))

        self.runner.overrides[timer_name] = {"DropInPaths": "/etc/systemd/system/example.d/override.conf"}
        with self.assertRaises(installer.InstallError) as caught:
            self.run_install()
        self.assertEqual(caught.exception.code, "POLICY_MISMATCH")

    def test_daemon_observer_install_does_not_gate_on_kis_timer_state(self) -> None:
        self.runner.research_mode = "daily"
        result = self.run_install(mode="daemon", apply=True)
        self.assertEqual(result["policy"]["profile"], "owner_daemon")
        mutations = [call for call in self.runner.calls if call[0] == "/usr/bin/systemctl" and call[1] != "show"]
        self.assertTrue(all("lagrange-kis-daily-" not in " ".join(call) for call in mutations))

    def test_wrong_current_binding_manifest_and_project_fail_closed(self) -> None:
        original = self.current.readlink()
        self.current.unlink()
        other = "b" * 40
        other_release = self.install_root / "releases" / other
        other_release.mkdir()
        self.current.symlink_to(f"releases/{other}")
        with self.assertRaises(installer.InstallError) as caught:
            self.run_install()
        self.assertEqual(caught.exception.code, "POLICY_MISMATCH")
        self.current.unlink()
        self.current.symlink_to(original)
        self.write_manifest("b" * 40)
        with self.assertRaises(installer.InstallError) as caught:
            self.run_install()
        self.assertEqual(caught.exception.code, "POLICY_MISMATCH")

    def test_invalid_effective_modes_are_rejected_without_shell_or_profiles(self) -> None:
        self.env_file.write_text(self.env_file.read_text().replace("OWNER_INTRADAY_QUOTES_MODE=owner_only", "OWNER_INTRADAY_QUOTES_MODE=off"))
        self.env_file.chmod(0o600)
        with self.assertRaises(installer.InstallError) as caught:
            self.run_install()
        self.assertEqual(caught.exception.code, "CONFIG_INVALID")
        self.assertEqual(self.runner.calls, [])


class InstallerApplyTests(InstallerFixture):
    def test_apply_uses_fake_runners_and_writes_receipt_private_paths(self) -> None:
        result = self.run_install(apply=True)
        policy_path = self.private_root / "policy.json"
        history_path = self.private_root / "history.json"
        snapshot_path = self.public_root / "snapshot.json"
        self.assertTrue(policy_path.is_file())
        self.assertTrue(history_path.is_file())
        self.assertFalse(snapshot_path.exists())
        self.assertEqual(stat.S_IMODE(policy_path.stat().st_mode), 0o600)
        self.assertEqual((policy_path.stat().st_uid, policy_path.stat().st_gid), self.owner)
        self.assertEqual(stat.S_IMODE(history_path.stat().st_mode), 0o600)
        self.assertEqual(stat.S_IMODE(self.private_root.stat().st_mode), 0o700)
        self.assertEqual(stat.S_IMODE(self.public_root.stat().st_mode), 0o750)
        self.assertEqual(stat.S_IMODE((self.systemd_root / "lagrange-service-status.service").stat().st_mode), 0o644)
        receipt = policy.validate_policy(json.loads(policy_path.read_text()))
        history = collector.validate_history(json.loads(history_path.read_text()), receipt, result["manifest_sha256"])
        self.assertTrue(all(job["last_success_at"] is None for job in history["jobs"].values()))
        self.assertEqual(history["binding"]["release_commit"], self.commit)
        mutations = [call for call in self.runner.calls if call[0] == "/usr/bin/systemctl" and call[1] != "show"]
        self.assertEqual(mutations, [
            ("/usr/bin/systemctl", "daemon-reload"),
            ("/usr/bin/systemctl", "enable", "--now", "lagrange-service-status.timer"),
        ])
        self.assertTrue(result["policy"]["policy_installed_at"].endswith("Z"))

    def test_failed_systemd_activation_rolls_back_receipt_units_and_history(self) -> None:
        runner = FakeRunner()
        runner.systemd_root = str(self.systemd_root)
        runner.fail_on = "enable_partial"
        with self.assertRaises(installer.InstallError) as caught:
            self.run_install(apply=True, runner=runner)
        self.assertEqual(caught.exception.code, "APPLY_FAILED")
        self.assertFalse((self.private_root / "policy.json").exists())
        self.assertFalse((self.private_root / "history.json").exists())
        self.assertFalse((self.systemd_root / "lagrange-service-status.service").exists())
        self.assertFalse((self.systemd_root / "lagrange-service-status.timer").exists())
        self.assertEqual(runner.observer_states["lagrange-service-status.timer"]["active"], "inactive")
        self.assertEqual(runner.observer_states["lagrange-service-status.timer"]["enabled"], "disabled")
        mutations = [call for call in runner.calls if call[0] == "/usr/bin/systemctl" and call[1] != "show"]
        self.assertIn(("/usr/bin/systemctl", "disable", "--now", "lagrange-service-status.timer"), mutations)
        self.assertIn(("/usr/bin/systemctl", "stop", "lagrange-service-status.service"), mutations)
        self.assertEqual(mutations.count(("/usr/bin/systemctl", "daemon-reload")), 2)

    def test_partial_activation_failure_restores_existing_observer_files_and_state(self) -> None:
        self.run_install(apply=True)
        service_path = self.systemd_root / "lagrange-service-status.service"
        timer_path = self.systemd_root / "lagrange-service-status.timer"
        policy_path = self.private_root / "policy.json"
        history_path = self.private_root / "history.json"
        preserved = {path: path.read_bytes() for path in (service_path, timer_path, policy_path, history_path)}
        self.runner.observer_states["lagrange-service-status.timer"].update(
            {"loaded": True, "active": "inactive", "enabled": "disabled"}
        )
        self.runner.observer_states["lagrange-service-status.service"].update(
            {"loaded": True, "active": "active", "enabled": "enabled"}
        )
        self.runner.fail_on = "enable_partial"
        with self.assertRaises(installer.InstallError) as caught:
            self.run_install(apply=True)
        self.assertEqual(caught.exception.code, "APPLY_FAILED")
        self.assertEqual({path: path.read_bytes() for path in preserved}, preserved)
        self.assertEqual(self.runner.observer_states["lagrange-service-status.timer"]["active"], "inactive")
        self.assertEqual(self.runner.observer_states["lagrange-service-status.timer"]["enabled"], "disabled")
        self.assertEqual(self.runner.observer_states["lagrange-service-status.service"]["active"], "active")
        self.assertEqual(self.runner.observer_states["lagrange-service-status.service"]["enabled"], "enabled")

    def test_rollback_failure_has_typed_incomplete_result(self) -> None:
        runner = FakeRunner()
        runner.systemd_root = str(self.systemd_root)
        runner.fail_on = "enable_partial"
        runner.fail_rollback = True
        with self.assertRaises(installer.InstallError) as caught:
            self.run_install(apply=True, runner=runner)
        self.assertEqual(caught.exception.code, "ROLLBACK_INCOMPLETE")
        self.assertFalse((self.private_root / "policy.json").exists())

    def test_startup_receipt_missing_daily_daemon_and_wrong_commit(self) -> None:
        self.assertEqual(policy.validate_startup_binding(
            str(self.private_root / "policy.json"), str(self.install_root), str(self.release_root), self.commit,
            test_owner=self.owner,
        ), "absent")
        result = self.run_install(apply=True)
        self.assertEqual(policy.validate_startup_binding(
            str(self.private_root / "policy.json"), str(self.install_root), str(self.release_root), self.commit,
            test_owner=self.owner,
        ), "daily")
        with self.assertRaises(policy.PolicyError):
            policy.validate_startup_binding(
                str(self.private_root / "policy.json"), str(self.install_root), str(self.release_root), "b" * 40,
                test_owner=self.owner,
            )
        self.assertEqual(result["policy"]["research_mode"], "daily")

    def test_missing_receipt_with_symlinked_private_parent_is_not_legacy_absence(self) -> None:
        self.private_root.parent.mkdir(parents=True)
        alternate = self.base / "alternate-private"
        alternate.mkdir(mode=0o700)
        alternate.chmod(0o700)
        self.private_root.symlink_to(alternate, target_is_directory=True)
        with self.assertRaises(policy.PolicyError):
            policy.validate_startup_binding(
                str(self.paths.policy_file), str(self.install_root), str(self.release_root), self.commit,
                test_owner=self.owner,
            )

    def test_daily_startup_accepts_inactive_timer_but_requires_unit_integrity(self) -> None:
        self.run_install(apply=True)
        timer_name = f"lagrange-kis-daily-{self.commit[:7]}.timer"
        service_name = timer_name[:-6] + ".service"

        def fake_systemctl(argv, *, deadline):
            import sys
            unit = argv[-1]
            startup_collector = sys.modules["service_status_startup_collector"]
            properties = (
                startup_collector.SYSTEMD_TIMER_PROPERTIES
                if unit.endswith(".timer")
                else startup_collector.SYSTEMD_SERVICE_PROPERTIES
            )
            fields = {key: "" for key in properties}
            fields.update({
                "LoadState": "loaded",
                "ActiveState": "inactive",
                "UnitFileState": "disabled" if unit == timer_name else "static",
                "FragmentPath": str(self.systemd_root / unit),
                "DropInPaths": "",
                "NextElapseUSecRealtime": "",
            })
            if unit == timer_name:
                fields["Unit"] = service_name
            if unit.endswith(".service"):
                fields.pop("Unit", None)
            raw = "".join(f"{key}={fields[key]}\n" for key in properties).encode()
            return startup_collector.CommandResult("ok", raw)

        result = policy.validate_startup_policy(
            str(self.paths.policy_file), str(self.paths.history_file), str(self.install_root),
            str(self.release_root), str(self.systemd_root), self.commit,
            test_owner=self.owner, runner=fake_systemctl,
        )
        self.assertEqual(result, ("daily", "lagrange-station"))

        def dropped_timer(argv, *, deadline):
            import sys
            result = fake_systemctl(argv, deadline=deadline)
            if argv[-1] == timer_name:
                fields = result.stdout.decode().replace("DropInPaths=\n", "DropInPaths=/etc/systemd/system/override.conf\n")
                return sys.modules["service_status_startup_collector"].CommandResult("ok", fields.encode())
            return result

        with self.assertRaises(policy.PolicyError):
            policy.validate_startup_policy(
                str(self.paths.policy_file), str(self.paths.history_file), str(self.install_root),
                str(self.release_root), str(self.systemd_root), self.commit,
                test_owner=self.owner, runner=dropped_timer,
            )

    def test_daemon_full_startup_gate_rejects_active_daily_timer(self) -> None:
        self.run_install(mode="daemon", apply=True)
        timer_name = f"lagrange-kis-daily-{self.commit[:7]}.timer"
        service_name = timer_name[:-6] + ".service"
        active_daily = True

        def fake_systemctl(argv, *, deadline):
            import sys
            unit = argv[-1]
            startup_collector = sys.modules["service_status_startup_collector"]
            properties = (
                startup_collector.SYSTEMD_TIMER_PROPERTIES
                if unit.endswith(".timer")
                else startup_collector.SYSTEMD_SERVICE_PROPERTIES
            )
            fields = {key: "" for key in properties}
            fields.update({
                "LoadState": "loaded",
                "ActiveState": "active" if unit == timer_name and active_daily else "inactive",
                "UnitFileState": "enabled" if unit == timer_name and active_daily else "disabled",
                "FragmentPath": str(self.systemd_root / unit),
                "DropInPaths": "",
                "NextElapseUSecRealtime": "2032-02-10T00:30:00Z" if unit == timer_name else "",
            })
            if unit == timer_name:
                fields["Unit"] = service_name
            if unit.endswith(".service"):
                fields.pop("Unit", None)
            raw = "".join(f"{key}={fields[key]}\n" for key in properties).encode()
            gate_collector = sys.modules.get("service_status_daemon_gate_collector", startup_collector)
            return gate_collector.CommandResult("ok", raw)

        mode = policy.validate_startup_policy(
            str(self.paths.policy_file), str(self.paths.history_file), str(self.install_root),
            str(self.release_root), str(self.systemd_root), self.commit,
            test_owner=self.owner, runner=fake_systemctl,
        )
        self.assertEqual(mode, ("daemon", "lagrange-station"))
        with self.assertRaises(policy.PolicyError) as caught:
            policy.validate_daemon_timer_gate(
                str(self.systemd_root), self.commit, test_owner=self.owner, runner=fake_systemctl,
            )
        self.assertEqual(caught.exception.code, "DAEMON_MODE_CONFLICT")
        active_daily = False
        policy.validate_daemon_timer_gate(
            str(self.systemd_root), self.commit, test_owner=self.owner, runner=fake_systemctl,
        )


if __name__ == "__main__":
    unittest.main()
