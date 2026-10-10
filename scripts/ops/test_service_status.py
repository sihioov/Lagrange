from __future__ import annotations

import copy
import importlib.util
import json
import os
import stat
import tempfile
import unittest
from datetime import datetime, timedelta, timezone
from pathlib import Path
from types import SimpleNamespace


ROOT = Path(__file__).resolve().parents[2]
FIXTURES = ROOT / "tests/fixtures/service-status"


def load_module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError("test-module-unavailable")
    module = importlib.util.module_from_spec(spec)
    import sys
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


policy = load_module("test_service_status_policy", ROOT / "scripts/ops/lib/service-status-policy.py")
collector = load_module("test_collect_service_status", ROOT / "scripts/ops/collect-service-status.py")


def test_owner() -> tuple[int, int]:
    return os.geteuid(), os.getegid()


def policy_fixture(commit: str = "a" * 40, mode: str = "daily", project: str = "synthetic-service-status") -> dict:
    return {
        "schema_version": "service-status.policy.v1",
        "profile": "owner_daily" if mode == "daily" else "owner_daemon",
        "release_commit": commit,
        "compose_project": project,
        "owner_access": "owner_only",
        "quote_transport": "market_ws",
        "window_source": "operational_v1",
        "research_mode": mode,
        "policy_installed_at": "2032-02-10T00:00:00Z",
        "job_lease_seconds": {
            "owner_equity_v2": 60,
            "owner_beta_price_recommendation": 60,
            "candidate_compute": 60,
            "recommendation": 60,
            "backtest": 60,
        },
    }


def base_snapshot() -> dict:
    return json.loads((FIXTURES / "base-input.json").read_text())["host_read"]["snapshot"]


def wire_cases() -> dict[str, dict]:
    cases = json.loads((FIXTURES / "wire-cases.json").read_text())["cases"]
    return {case["id"]: case for case in cases}


def write_private_file(path: Path, data: bytes, mode: int = 0o600) -> None:
    path.write_bytes(data)
    path.chmod(mode)


class ParseBtimeTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="service-status-btime.")
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name) / "proc-stat"

    def parse_bytes(self, contents: bytes) -> str:
        self.path.write_bytes(contents)
        return collector.parse_btime(str(self.path))

    def test_ordinary_valid_btime_keeps_canonical_utc_timestamp(self) -> None:
        self.assertEqual(self.parse_bytes(b"cpu 1 2 3\nbtime 1960000000\n"), "2032-02-10T04:26:40Z")

    def test_long_unrelated_intr_line_before_valid_btime_is_accepted(self) -> None:
        contents = b"intr " + b"9" * 5000 + b"\nbtime 1960000000\n"
        self.assertGreater(len(contents.splitlines()[0]), 4096)
        self.assertEqual(self.parse_bytes(contents), "2032-02-10T04:26:40Z")

    def test_duplicate_btime_after_long_unrelated_line_is_rejected(self) -> None:
        contents = b"intr " + b"9" * 5000 + b"\nbtime 1960000000\nbtime 1960000001\n"
        with self.assertRaises(collector.CollectError) as caught:
            self.parse_bytes(contents)
        self.assertEqual(caught.exception.code, "invalid")

    def test_malformed_btime_records_are_rejected(self) -> None:
        for contents in (
            b"btime\t1960000000\n",
            b"btime 0\n",
            b"btime +1960000000\n",
            b"btime 1960000000x\n",
            b"btime 1960000000\nbtime\t1960000001\n",
        ):
            with self.subTest(contents=contents):
                with self.assertRaises(collector.CollectError) as caught:
                    self.parse_bytes(contents)
                self.assertEqual(caught.exception.code, "invalid")

    def test_missing_btime_is_rejected(self) -> None:
        with self.assertRaises(collector.CollectError) as caught:
            self.parse_bytes(b"cpu 1 2 3\nintr 4 5 6\n")
        self.assertEqual(caught.exception.code, "invalid")

    def test_oversized_proc_stat_is_rejected(self) -> None:
        contents = b"btime 1960000000\n" + b"x" * collector.MAX_PROC_STAT_BYTES
        self.assertGreater(len(contents), collector.MAX_PROC_STAT_BYTES)
        with self.assertRaises(collector.CollectError) as caught:
            self.parse_bytes(contents)
        self.assertEqual(caught.exception.code, "invalid")

    def test_exact_byte_limit_is_accepted(self) -> None:
        prefix = b"btime 1960000000\nintr "
        contents = prefix + b"9" * (collector.MAX_PROC_STAT_BYTES - len(prefix) - 1) + b"\n"
        self.assertEqual(len(contents), collector.MAX_PROC_STAT_BYTES)
        self.assertEqual(self.parse_bytes(contents), "2032-02-10T04:26:40Z")

    def test_proc_stat_oserror_fails_closed(self) -> None:
        with self.assertRaises(collector.CollectError) as caught:
            collector.parse_btime(str(self.path / "missing"))
        self.assertEqual(caught.exception.code, "invalid")


class CollectorVectorTests(unittest.TestCase):
    def test_all_collector_vectors_reduce_to_expected_observations(self) -> None:
        vectors = json.loads((FIXTURES / "collector-vectors.json").read_text())
        self.assertEqual(vectors["fixture_version"], 1)
        count = 0
        for case in vectors["cases"]:
            service_id = case["service"]
            expected = case["expected_observation"]
            if case["result"] != "ok":
                actual = collector.failed_observation(service_id, case["result"])
            elif case["source"] == "docker" and "matching_containers" in case["safe_fields"]:
                actual = collector.missing_docker_observation(service_id, case["checked_at"])
            elif case["source"] == "docker":
                actual = collector.map_docker_fields(service_id, case["checked_at"], case["safe_fields"])
            else:
                timer = case["safe_fields"]["timer"]
                service = case["safe_fields"]["service"]
                expected_success = expected["timer"]["last_success_at"]
                actual = collector.map_systemd_fields(
                    service_id,
                    case["checked_at"],
                    timer_fields=timer,
                    service_fields=service,
                    history_record={"last_success_at": expected_success},
                    verified_unit_binding=case["safe_fields"]["verified_unit_binding"],
                )
            self.assertEqual(actual, expected, case["id"])
            count += 1
        self.assertEqual(count, 14)

    def test_command_failures_and_timeout_vectors_remain_explicit(self) -> None:
        cases = json.loads((FIXTURES / "collector-vectors.json").read_text())["cases"]
        for case_id, result in (
            ("collector_timeout", "timeout"),
            ("collector_command_unavailable", "command_unavailable"),
            ("collector_denied", "denied"),
            ("collector_deadline", "deadline"),
        ):
            case = next(item for item in cases if item["id"] == case_id)
            self.assertEqual(collector.failed_observation(case["service"], result), case["expected_observation"])

    def test_docker_project_service_labels_oneoff_and_duplicates(self) -> None:
        project = "synthetic-service-status"
        service = "web"
        first = "a" * 64
        second = "b" * 64
        raw = (
            f"{first}\t{project}\t{service}\tTrue\n"
            f"{second}\t{project}\t{service}\tFalse\n"
        ).encode()
        self.assertEqual(collector.parse_docker_ps(raw, project, service), [second])
        with self.assertRaises(collector.CollectError):
            collector.parse_docker_ps(raw + f"{'c' * 64}\t{project}\t{service}\tFalse\n".encode(), project, service)
        with self.assertRaises(collector.CollectError):
            collector.parse_docker_ps(f"{first}\t{project}\t{service}\tperhaps\n".encode(), project, service)

    def test_docker_host_timestamp_precision_and_observation_snapshot(self) -> None:
        base = "2032-02-12T00:59:03"
        self.assertEqual(collector.canonical_time(collector.parse_timestamp(base + "Z")), base + "Z")
        for width in range(1, 7):
            fraction = "123456"[:width]
            value = collector.parse_timestamp(base + "." + fraction + "Z")
            self.assertEqual(collector.canonical_time(value), base + "." + fraction + "Z")
        for fraction in ("1234567", "12345678", "123456999"):
            value = collector.parse_timestamp(base + "." + fraction + "Z")
            self.assertEqual(collector.canonical_time(value), base + ".123456Z")

        for value in (
            "2032-02-12T00:59:03.1234567890Z",
            "2032-02-12T00:59:03.12345xZ",
            "2032-02-12T00:59:03.１２３Z",
            "2032-02-30T00:59:03Z",
            "2032-02-12T00:59:03+00:00",
            "2032-02-12T00:59:03Z trailing",
        ):
            with self.subTest(timestamp=value), self.assertRaises(collector.CollectError):
                collector.parse_timestamp(value)
        for value in (None, "", "n/a", "0", "0001-01-01T00:00:00Z", "0001-01-01T00:00:00.000000000Z"):
            with self.subTest(timestamp=value):
                self.assertIsNone(collector.parse_timestamp(value))

        project = "synthetic-service-status"
        paths = collector.CollectorPaths(docker_bin="/usr/bin/docker")
        container_id = "a" * 64
        observed_start = "2026-10-08T13:37:03.467540542Z"
        responses = {
            collector.docker_ps_argv(paths, project, "web"): collector.CommandResult(
                "ok", f"{container_id}\t{project}\tweb\tFalse\n".encode("ascii")
            ),
            collector.docker_inspect_argv(paths, container_id): collector.CommandResult(
                "ok", (
                    f"running\thealthy\t{observed_start}\t0001-01-01T00:00:00Z\t"
                    f"\t{project}\tweb\tFalse"
                ).encode("ascii")
            ),
        }

        def runner(argv: tuple[str, ...], *, deadline: float) -> collector.CommandResult:
            return responses.get(argv, collector.CommandResult("command_unavailable"))

        observed = collector.docker_observation(
            "web", "2026-10-08T13:37:04Z", project, paths, runner, deadline=1.0,
        )
        self.assertEqual(observed["result"], "ok")
        self.assertEqual(observed["process"]["started_at"], "2026-10-08T13:37:03.46754Z")
        self.assertIsNone(observed["process"]["finished_at"])

        synthetic_start = "2032-02-12T00:59:03.123456789Z"
        synthetic_raw = (
            f"running\thealthy\t{synthetic_start}\t0001-01-01T00:00:00Z\t"
            f"\t{project}\tweb\tFalse"
        ).encode("ascii")
        fields = collector.parse_docker_inspect(synthetic_raw, "web", project)
        synthetic_observation = collector.map_docker_fields(
            "web", "2032-02-12T01:00:00Z", fields,
        )
        self.assertEqual(synthetic_observation["process"]["started_at"], "2032-02-12T00:59:03.123456Z")
        snapshot = base_snapshot()
        snapshot["observations"] = [
            synthetic_observation if item["id"] == "web" else item for item in snapshot["observations"]
        ]
        collector.validate_snapshot(snapshot, now=datetime(2032, 2, 12, 1, tzinfo=timezone.utc))

    def test_inspect_secret_sentinel_is_rejected_and_never_emitted(self) -> None:
        sentinel = "SERVICE_STATUS_SECRET_SENTINEL"
        raw = ("running\thealthy\t2032-02-12T00:00:00Z\t0001-01-01T00:00:00Z\t0\t"
               "synthetic-service-status\tweb\tFalse\t" + sentinel).encode()
        obs = collector.failed_observation("web", "invalid")
        with self.assertRaises(collector.CollectError):
            collector.parse_docker_inspect(raw, "web", "synthetic-service-status")
        self.assertNotIn(sentinel, json.dumps(obs))
        paths = collector.CollectorPaths()
        argv = collector.docker_inspect_argv(paths, "a" * 64)
        self.assertEqual(argv[0], "/usr/bin/docker")
        self.assertEqual(argv[1:4], ("inspect", "--type", "container"))
        self.assertNotIn("Config.Env", argv[5])
        self.assertNotIn("healthcheck", argv[5].lower())
        self.assertNotIn("logs", argv)

    def test_collector_argv_is_closed_and_arbitrary_binary_is_not_run(self) -> None:
        paths = collector.CollectorPaths()
        ps = collector.docker_ps_argv(paths, "synthetic-service-status", "web")
        self.assertEqual(ps[0:2], ("/usr/bin/docker", "ps"))
        self.assertIn("label=com.docker.compose.project=synthetic-service-status", ps)
        self.assertIn("label=com.docker.compose.service=web", ps)
        self.assertNotIn("environment", " ".join(ps).lower())
        systemd = collector.systemctl_argv(paths, "docker.service")
        self.assertEqual(systemd[0:3], ("/usr/bin/systemctl", "show", "--no-pager"))
        self.assertEqual(systemd[-1], "docker.service")
        self.assertEqual(set(systemd[3].split("=", 1)[1].split(",")), set(collector.SYSTEMD_PROPERTIES))
        with self.assertRaises(collector.CollectError):
            collector.systemctl_argv(paths, "unlisted.service")
        result = collector.run_fixed_command(("/bin/echo", "sentinel"), deadline=collector.time.monotonic() + 1)
        self.assertEqual(result.result, "invalid")

    def test_systemd_unknown_state_and_result_fail_closed(self) -> None:
        service = {
            "LoadState": "loaded", "ActiveState": "alien", "Result": "success",
            "ExecMainCode": "1", "ExecMainStatus": "0", "ExecMainStartTimestamp": "",
            "ExecMainExitTimestamp": "",
        }
        self.assertEqual(
            collector.map_systemd_fields("docker", "2032-02-12T01:00:00Z", timer_fields=None, service_fields=service)["result"],
            "invalid",
        )
        service["ActiveState"] = "inactive"
        service["Result"] = "secret provider prose"
        self.assertEqual(
            collector.map_systemd_fields("docker", "2032-02-12T01:00:00Z", timer_fields=None, service_fields=service)["result"],
            "invalid",
        )

    def test_systemd_property_sets_match_sparse_service_and_timer_shapes(self) -> None:
        common = {
            "LoadState": "loaded", "ActiveState": "inactive", "SubState": "dead",
            "UnitFileState": "disabled", "Result": "success", "FragmentPath": "/etc/systemd/system/fake",
            "DropInPaths": "",
        }
        service_values = {
            **common, "ExecMainCode": "1", "ExecMainStatus": "0",
            "ExecMainStartTimestamp": "", "ExecMainExitTimestamp": "",
        }
        timer_values = {
            **common, "LastTriggerUSec": "", "NextElapseUSecRealtime": "", "Unit": "fake.service",
        }

        def encoded(values: dict[str, str], fields: tuple[str, ...]) -> bytes:
            return "".join(f"{key}={values[key]}\n" for key in fields if key in values).encode()

        loaded_service = collector._parse_key_values(encoded(service_values, collector.SYSTEMD_SERVICE_PROPERTIES), collector.SYSTEMD_SERVICE_PROPERTIES)
        loaded_timer = collector._parse_key_values(encoded(timer_values, collector.SYSTEMD_TIMER_PROPERTIES), collector.SYSTEMD_TIMER_PROPERTIES)
        self.assertNotIn("Unit", loaded_service)
        self.assertNotIn("LastTriggerUSec", loaded_service)
        self.assertNotIn("ExecMainCode", loaded_timer)

        service_missing = dict(service_values, LoadState="not-found", UnitFileState="not-found")
        timer_missing = dict(timer_values, LoadState="not-found", UnitFileState="not-found", Unit="")
        parsed_service_missing = collector._parse_key_values(encoded(service_missing, collector.SYSTEMD_SERVICE_PROPERTIES), collector.SYSTEMD_SERVICE_PROPERTIES)
        parsed_timer_missing = collector._parse_key_values(encoded(timer_missing, collector.SYSTEMD_TIMER_PROPERTIES), collector.SYSTEMD_TIMER_PROPERTIES)
        self.assertEqual(parsed_service_missing["LoadState"], "not-found")
        self.assertEqual(parsed_timer_missing["LoadState"], "not-found")

        missing_common = dict(service_values)
        del missing_common["ActiveState"]
        with self.assertRaises(collector.CollectError):
            collector._parse_key_values(encoded(missing_common, collector.SYSTEMD_SERVICE_PROPERTIES), collector.SYSTEMD_SERVICE_PROPERTIES)
        with self.assertRaises(collector.CollectError):
            collector._parse_key_values(
                encoded(service_values, collector.SYSTEMD_SERVICE_PROPERTIES) + b"LoadState=loaded\n",
                collector.SYSTEMD_SERVICE_PROPERTIES,
            )
        with self.assertRaises(collector.CollectError):
            collector._parse_key_values(
                encoded(timer_values, collector.SYSTEMD_TIMER_PROPERTIES) + b"ExecMainPID=17\n",
                collector.SYSTEMD_TIMER_PROPERTIES,
            )

    def test_missing_disabled_timer_unset_next_and_dropins(self) -> None:
        temp = tempfile.TemporaryDirectory(prefix="service-status-units.")
        self.addCleanup(temp.cleanup)
        systemd_root = Path(temp.name)
        (systemd_root / "example.service").write_text("[Service]\n")
        (systemd_root / "example.timer").write_text("[Timer]\n")
        (systemd_root / "example.service").chmod(0o644)
        (systemd_root / "example.timer").chmod(0o644)
        service = {
            "LoadState": "loaded", "ActiveState": "inactive", "Result": "success",
            "ExecMainCode": "1", "ExecMainStatus": "0", "ExecMainStartTimestamp": "",
            "ExecMainExitTimestamp": "", "FragmentPath": str(systemd_root / "example.service"),
            "DropInPaths": "",
        }
        timer = {
            "LoadState": "loaded", "ActiveState": "inactive", "UnitFileState": "disabled",
            "LastTriggerUSec": "n/a", "NextElapseUSecRealtime": "", "Unit": "example.service",
            "FragmentPath": str(systemd_root / "example.timer"), "DropInPaths": "",
        }
        result = collector.map_systemd_fields(
            "kis-eod", "2032-02-12T01:00:00Z", timer_fields=timer, service_fields=service,
            expected_timer_name="example.timer", expected_service_name="example.service",
            systemd_root=str(systemd_root), test_owner=test_owner(),
        )
        self.assertEqual(result["timer"]["enabled"], "disabled")
        self.assertIsNone(result["timer"]["next_run_at"])
        self.assertEqual(result["timer"]["unit_binding"], "matched")
        timer["LoadState"] = "not-found"
        missing = collector.map_systemd_fields(
            "kis-eod", "2032-02-12T01:00:00Z", timer_fields=timer, service_fields=service,
            expected_timer_name="example.timer", expected_service_name="example.service",
            systemd_root=str(systemd_root), test_owner=test_owner(),
        )
        self.assertEqual(missing["timer"]["load"], "not-found")
        timer["LoadState"] = "loaded"
        timer["DropInPaths"] = "/etc/systemd/system/example.timer.d/override.conf"
        dropped = collector.map_systemd_fields(
            "kis-eod", "2032-02-12T01:00:00Z", timer_fields=timer, service_fields=service,
            expected_timer_name="example.timer", expected_service_name="example.service",
            systemd_root=str(systemd_root), test_owner=test_owner(),
        )
        self.assertEqual(dropped["timer"]["unit_binding"], "mismatch")

    def test_condition_skip_and_daily_exit_74_75_are_not_data_success(self) -> None:
        now = datetime(2032, 2, 12, 1, tzinfo=timezone.utc)
        cases = wire_cases()
        skip_case = cases["systemd_condition_skip"]
        base_service = {
            "LoadState": "loaded", "ActiveState": "inactive", "Result": "success",
            "ExecMainCode": "1", "ExecMainStatus": "0", "ExecMainStartTimestamp": "",
            "ExecMainExitTimestamp": "", "FragmentPath": "/etc/systemd/system/example.service",
            "DropInPaths": "",
        }
        self.assertEqual(skip_case["input"]["fault"], "exit_0_without_start_finish")
        timer = {
            "LoadState": "loaded", "ActiveState": "active", "UnitFileState": "enabled",
            "LastTriggerUSec": "", "NextElapseUSecRealtime": "2032-02-12T02:00:00Z",
            "Unit": "example.service", "FragmentPath": "/etc/systemd/system/example.timer", "DropInPaths": "",
        }
        skip = collector.map_systemd_fields(
            "kis-eod", collector.canonical_time(now), timer_fields=timer, service_fields=base_service,
            expected_timer_name="example.timer", expected_service_name="example.service",
        )
        self.assertEqual(skip["timer"]["last_result"], "unknown")
        self.assertEqual("RUN_RESULT_" + skip["timer"]["last_result"].upper(), skip_case["expected"]["reason_code"])
        exit_cases = [cases[f"daily_exit_{status}"] for status in (74, 75)]
        for case in exit_cases:
            exit_status = case["input"]["exit_status"]
            service = dict(base_service)
            service.update({
                "Result": case["input"]["systemd_result"],
                "ExecMainStatus": str(exit_status),
                "ExecMainStartTimestamp": "2032-02-12T00:59:00Z",
                "ExecMainExitTimestamp": "2032-02-12T00:59:30Z",
            })
            obs = collector.map_systemd_fields(
                "kis-eod", collector.canonical_time(now), timer_fields=timer, service_fields=service,
                expected_timer_name="example.timer", expected_service_name="example.service",
            )
            self.assertEqual(obs["timer"]["last_result"], "unknown")
            self.assertEqual("RUN_RESULT_" + obs["timer"]["last_result"].upper(), case["expected"]["reason_code"])
            self.assertFalse(collector.verified_completion(
                start=now - timedelta(minutes=1), end=now - timedelta(seconds=30), exec_code=1,
                exit_status=exit_status, result="success", expected_unit="/example.service",
                observed_unit="/example.service", installed_at=now - timedelta(days=1),
            ))

    def test_reboot_discards_prior_boot_process_samples_but_keeps_verified_success(self) -> None:
        case = wire_cases()["reboot_discards_process_cache"]
        boot_before = case["input"]["boot_before"]
        boot_after = case["input"]["boot_after"]
        observation = collector.map_docker_fields("web", boot_after, {
            "State.Status": case["input"]["prior_process_state"], "State.Health.Status": "none",
            "State.StartedAt": "2032-02-11T01:00:00Z",
            "State.FinishedAt": "2032-02-11T01:10:00Z", "State.ExitCode": 0,
        })
        self.assertLess(policy.timestamp(observation["process"]["started_at"]), policy.timestamp(boot_after))
        self.assertLess(policy.timestamp(boot_before), policy.timestamp(boot_after))
        observation["timer"] = {
            "load": "loaded", "active": "inactive", "enabled": "disabled",
            "last_trigger_at": "2032-02-11T01:00:00Z", "next_run_at": None,
            "unit_binding": "matched", "last_result": "success", "exit_status": 0,
            "last_started_at": "2032-02-11T01:00:00Z", "last_finished_at": "2032-02-11T01:10:00Z",
            "last_success_at": "2032-02-11T01:10:00Z",
        }
        observation["timer"]["last_success_at"] = case["input"]["last_success_at"]
        rebooted = collector._clear_preboot_observation(observation, boot_after)
        self.assertIsNone(rebooted["process"]["started_at"])
        self.assertIsNone(rebooted["process"]["finished_at"])
        self.assertIsNone(rebooted["process"]["exit_code"])
        self.assertEqual(rebooted["timer"]["last_result"], "unknown")
        self.assertIsNone(rebooted["timer"]["last_started_at"])
        self.assertIsNone(rebooted["timer"]["last_finished_at"])
        self.assertEqual(rebooted["timer"]["last_success_at"], case["expected"]["retain_last_success_at"])
        fresh = collector.failed_observation("web", case["input"]["fresh_query"])
        projected_process = "unknown" if fresh["process"] is None else fresh["process"]["state"]
        self.assertEqual(projected_process, case["expected"]["process_status"])

    def test_history_preserves_verified_success_across_restart_and_reboot(self) -> None:
        case = wire_cases()["reboot_discards_process_cache"]
        p = policy_fixture()
        history = collector.empty_history(p, "a" * 64, "b" * 64, "c" * 64)
        finish = policy.timestamp(case["input"]["last_success_at"])
        start = finish - timedelta(minutes=5)
        self.assertTrue(collector.verified_completion(
            start=start, end=finish, exec_code=1, exit_status=0, result="success",
            expected_unit="/etc/systemd/system/lagrange-kis-daily-aaaaaaa.service",
            observed_unit="/etc/systemd/system/lagrange-kis-daily-aaaaaaa.service",
            installed_at=policy.timestamp(p["policy_installed_at"]),
        ))
        collector.update_history_record(history, "kis-eod", start=start, end=finish,
                                        last_result="success", exit_status=0, successful=True)
        original = history["jobs"]["kis-eod"]["last_success_at"]
        failed_start = finish + timedelta(minutes=1)
        failed_finish = failed_start + timedelta(seconds=20)
        collector.update_history_record(history, "kis-eod", start=failed_start, end=failed_finish,
                                        last_result="failure", exit_status=1, successful=False)
        self.assertEqual(original, case["expected"]["retain_last_success_at"])
        self.assertEqual(history["jobs"]["kis-eod"]["last_success_at"], case["expected"]["retain_last_success_at"])
        self.assertEqual(history["jobs"]["kis-eod"]["last_result"], "failure")
        rebooted = collector.failed_observation("kis-eod", "timeout")
        self.assertIsNone(rebooted["process"])
        self.assertEqual(history["jobs"]["kis-eod"]["last_success_at"], "2032-02-11T07:35:00Z")


class SnapshotFilesystemTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="service-status.")
        self.root = Path(self.temp.name)
        self.owner = test_owner()
        self.public = self.root / "public"
        self.public.mkdir(mode=0o750)
        self.public.chmod(0o750)
        if (os.geteuid(), os.getegid()) != self.owner:
            os.chown(self.public, *self.owner)
        self.snapshot = base_snapshot()
        self.now = datetime(2032, 2, 12, 1, tzinfo=timezone.utc)

    def tearDown(self) -> None:
        self.temp.cleanup()

    def write(self, value: dict) -> Path:
        raw = json.dumps(value, separators=(",", ":"), sort_keys=True, allow_nan=False).encode() + b"\n"
        path = self.public / "snapshot.json"
        policy.atomic_write(
            str(path), raw, expected_uid=self.owner[0], expected_gid=self.owner[1], mode=0o640,
            max_bytes=collector.MAX_SNAPSHOT_BYTES, parent_mode=0o750, test_owner=self.owner,
        )
        return path

    def test_real_atomic_rename_new_inode_and_reader_reopens_by_name(self) -> None:
        case = wire_cases()["rename_refresh"]
        path = self.write(self.snapshot)
        first, inode1 = collector.read_public_snapshot(str(self.public), now=self.now, test_owner=self.owner)
        second_value = copy.deepcopy(self.snapshot)
        second_value["generated_at"] = "2032-02-12T01:00:01Z"
        second_value["collection_started_at"] = "2032-02-12T00:59:59Z"
        self.write(second_value)
        second, inode2 = collector.read_public_snapshot(
            str(self.public), now=self.now + timedelta(seconds=1), test_owner=self.owner
        )
        self.assertEqual(first["generated_at"], self.snapshot["generated_at"])
        self.assertEqual(second["generated_at"], second_value["generated_at"])
        self.assertNotEqual(inode1, inode2)
        self.assertEqual(case["expected"]["read"], "new_inode")
        self.assertTrue(case["input"]["reader_reopens_directory_child"])
        self.assertEqual(os.stat(path).st_ino, inode2[1])

    def test_production_snapshot_writer_accepts_public_mode_and_replaces_atomically(self) -> None:
        paths = collector.CollectorPaths(
            public_root=str(self.public), api_gid=self.owner[1], test_owner=self.owner,
        )
        first_snapshot = copy.deepcopy(self.snapshot)
        collector.validate_snapshot(first_snapshot, now=self.now)
        collector._write_snapshot(paths, first_snapshot)
        path = self.public / "snapshot.json"
        self.assertEqual(path.parent, Path(paths.public_root))
        first_stat = os.stat(path)
        self.assertEqual(stat.S_IMODE(first_stat.st_mode), 0o640)
        self.assertEqual((first_stat.st_uid, first_stat.st_gid), self.owner)
        first, first_identity = collector.read_public_snapshot(
            str(self.public), now=self.now, test_owner=self.owner,
        )
        self.assertEqual(first, first_snapshot)
        self.assertEqual(first_identity, (first_stat.st_dev, first_stat.st_ino))

        second_snapshot = copy.deepcopy(first_snapshot)
        second_snapshot["generated_at"] = "2032-02-12T01:00:01Z"
        second_snapshot["collection_started_at"] = "2032-02-12T00:59:59Z"
        second_now = self.now + timedelta(seconds=1)
        collector.validate_snapshot(second_snapshot, now=second_now)
        collector._write_snapshot(paths, second_snapshot)
        second_stat = os.stat(path)
        self.assertEqual(stat.S_IMODE(second_stat.st_mode), 0o640)
        self.assertNotEqual(first_stat.st_ino, second_stat.st_ino)
        self.assertEqual(sorted(item.name for item in self.public.iterdir()), ["snapshot.json"])
        second, second_identity = collector.read_public_snapshot(
            str(self.public), now=second_now, test_owner=self.owner,
        )
        self.assertEqual(second, second_snapshot)
        self.assertEqual(second_identity, (second_stat.st_dev, second_stat.st_ino))

        preserved_bytes = path.read_bytes()
        third_snapshot = copy.deepcopy(second_snapshot)
        third_snapshot["generated_at"] = "2032-02-12T01:00:02Z"
        collector.validate_snapshot(third_snapshot, now=self.now + timedelta(seconds=2))
        self.public.chmod(0o700)
        try:
            with self.assertRaises(collector.CollectError):
                collector._write_snapshot(paths, third_snapshot)
            self.assertEqual(os.stat(path).st_ino, second_stat.st_ino)
            self.assertEqual(path.read_bytes(), preserved_bytes)
        finally:
            self.public.chmod(0o750)

    def test_symlink_hardlink_wrong_owner_bad_mode_oversize_and_future_are_rejected(self) -> None:
        path = self.public / "snapshot.json"
        path.symlink_to("missing")
        with self.assertRaises(collector.CollectError):
            collector.read_public_snapshot(str(self.public), now=self.now, test_owner=self.owner)
        path.unlink()
        self.write(self.snapshot)
        other_owner = (self.owner[0] + 1, self.owner[1])
        with self.assertRaises(collector.CollectError):
            collector.read_public_snapshot(str(self.public), now=self.now, test_owner=other_owner)
        path.chmod(0o660)
        with self.assertRaises(collector.CollectError):
            collector.read_public_snapshot(str(self.public), now=self.now, test_owner=self.owner)
        path.chmod(0o640)
        alias = self.public / "alias"
        os.link(path, alias)
        with self.assertRaises(collector.CollectError):
            collector.read_public_snapshot(str(self.public), now=self.now, test_owner=self.owner)
        alias.unlink()
        path.write_bytes(b"{" + b" " * collector.MAX_SNAPSHOT_BYTES)
        path.chmod(0o640)
        with self.assertRaises(collector.CollectError):
            collector.read_public_snapshot(str(self.public), now=self.now, test_owner=self.owner)
        future = copy.deepcopy(self.snapshot)
        future["generated_at"] = "2032-02-12T01:00:01Z"
        future["collection_started_at"] = "2032-02-12T01:00:00Z"
        with self.assertRaises(collector.CollectError):
            collector.validate_snapshot(future, now=self.now)
        inconsistent = copy.deepcopy(self.snapshot)
        web = next(item for item in inconsistent["observations"] if item["id"] == "web")
        web["process"] = {
            "state": "running", "health": "healthy", "started_at": "2032-02-12T00:00:02Z",
            "finished_at": "2032-02-12T00:00:01Z", "exit_code": None,
        }
        with self.assertRaises(collector.CollectError):
            collector.validate_snapshot(inconsistent, now=self.now)

    def test_partial_write_keeps_old_inode_and_no_partial_file_visible(self) -> None:
        path = self.write(self.snapshot)
        old_inode = os.stat(path).st_ino
        new_data = b'{"schema_version":"service-status.host.v1"}\n'
        count = 0

        def partial_then_fail(fd: int, data: bytes) -> int:
            nonlocal count
            count += 1
            if count > 1:
                raise OSError("synthetic partial write")
            return os.write(fd, data[:4])

        with self.assertRaises(policy.PolicyError):
            policy.atomic_write(
                str(path), new_data, expected_uid=self.owner[0], expected_gid=self.owner[1],
                mode=0o640, max_bytes=collector.MAX_SNAPSHOT_BYTES, parent_mode=0o750,
                test_owner=self.owner, write_hook=partial_then_fail,
            )
        self.assertEqual(os.stat(path).st_ino, old_inode)
        self.assertEqual(json.loads(path.read_text())["schema_version"], "service-status.host.v1")
        self.assertEqual(sorted(p.name for p in self.public.iterdir()), ["snapshot.json"])

    def test_wire_reader_fault_recipes_construct_real_files_and_bytes(self) -> None:
        cases = [case for case in wire_cases().values() if case["layer"] == "host_reader" and
                 case["expected"]["reason_code"] in {"HOST_MISSING", "HOST_INVALID"}]
        for case in cases:
            with self.subTest(case=case["id"]):
                path = self.public / "snapshot.json"
                alias = self.public / "alias"
                target = self.public / "target"
                for old in (path, alias, target):
                    if old.exists() or old.is_symlink():
                        old.unlink()
                if case["id"] == "missing_file":
                    pass
                elif case["id"] == "symlink":
                    target.write_bytes(json.dumps(self.snapshot).encode())
                    target.chmod(0o640)
                    path.symlink_to(target.name)
                elif case["id"] == "hardlink":
                    self.write(self.snapshot)
                    os.link(path, alias)
                elif case["id"] == "wrong_owner":
                    self.write(self.snapshot)
                elif case["id"] == "oversize":
                    path.write_bytes(b" " * 262145)
                elif case["id"] == "partial_json":
                    path.write_bytes(case["input"]["fault"].encode())
                elif case["id"] == "duplicate_key":
                    path.write_bytes(case["input"]["fault"].encode())
                else:
                    bad = copy.deepcopy(self.snapshot)
                    fault = case["input"]["fault"]
                    if case["id"] == "unknown_field":
                        bad[fault] = "SENTINEL_DO_NOT_EMIT"
                    elif case["id"] == "wrong_version":
                        bad["schema_version"] = fault
                    elif case["id"] == "duplicate_id":
                        bad["observations"].append(copy.deepcopy(bad["observations"][0]))
                    elif case["id"] == "missing_inventory_id":
                        bad["observations"] = [x for x in bad["observations"] if x["id"] != "postgres"]
                    path.write_bytes(json.dumps(bad, separators=(",", ":")).encode())
                if case["id"] not in {"missing_file", "symlink", "hardlink", "wrong_owner"}:
                    path.chmod(0o660 if case["id"] == "writable_by_api" else 0o640)
                expected_owner = (self.owner[0] + 1, self.owner[1]) if case["id"] == "wrong_owner" else self.owner
                with self.assertRaises(collector.CollectError) as caught:
                    collector.read_public_snapshot(str(self.public), now=self.now, test_owner=expected_owner)
                self.assertFalse(case["expected"]["accept"])
                self.assertFalse(case["expected"]["raw_output_emitted"])
                expected_code = "missing" if case["expected"]["reason_code"] == "HOST_MISSING" else "invalid"
                self.assertEqual(caught.exception.code, expected_code)

    def test_lock_is_nonblocking_private_and_reboot_has_no_process_cache(self) -> None:
        run_dir = self.root / "run"
        run_dir.mkdir(mode=0o700)
        run_dir.chmod(0o700)
        lock = run_dir / "collector.lock"
        paths = collector.CollectorPaths(lock_file=str(lock), test_owner=self.owner)
        fd = collector._safe_lock(str(lock), paths)
        try:
            with self.assertRaises(collector.CollectError) as caught:
                collector._safe_lock(str(lock), paths)
            self.assertEqual(caught.exception.code, "locked")
        finally:
            import fcntl
            fcntl.flock(fd, fcntl.LOCK_UN)
            os.close(fd)
        self.assertEqual(stat.S_IMODE(lock.stat().st_mode), 0o600)


class HistoryCacheTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="service-status-history.")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.owner = test_owner()
        self.policy_value = policy_fixture()
        self.commit = self.policy_value["release_commit"]
        self.install_root = self.root / "install"
        self.release_root = self.install_root / "releases" / self.commit
        self.daily_ops = self.release_root / "scripts/ops"
        self.daily_ops.mkdir(parents=True)
        self.systemd = self.root / "systemd"
        self.systemd.mkdir(mode=0o755)
        self.systemd.chmod(0o755)
        self.private = self.root / "private"
        self.private.mkdir(mode=0o700)
        self.private.chmod(0o700)
        for directory in (
            self.install_root, self.install_root / "releases", self.release_root,
            self.release_root / "scripts", self.daily_ops,
        ):
            directory.chmod(0o755)
        service_name = f"lagrange-kis-daily-{self.commit[:7]}.service"
        timer_name = f"lagrange-kis-daily-{self.commit[:7]}.timer"
        wrapper = self.daily_ops / "kis-daily-production.sh"
        wrapper.write_text("#!/usr/bin/env bash\nexit 0\n")
        wrapper.chmod(0o755)
        checker = self.daily_ops / "install-kis-daily.sh"
        checker.write_text("#!/usr/bin/env bash\nexit 0\n")
        checker.chmod(0o755)
        (self.systemd / service_name).write_text(
            f"Environment=LAGRANGE_CODE_COMMIT={self.commit}\n"
            f"ExecStart={wrapper} --execute\nConditionFileIsExecutable={wrapper}\n"
        )
        (self.systemd / timer_name).write_text(f"Unit={service_name}\n")
        (self.systemd / service_name).chmod(0o644)
        (self.systemd / timer_name).chmod(0o644)
        self.history_path = self.private / "history.json"
        self.paths = collector.CollectorPaths(
            install_root=str(self.install_root), release_root=str(self.install_root / "current"),
            private_root=str(self.private), policy_file=str(self.private / "policy.json"),
            history_file=str(self.history_path),
            systemd_root=str(self.systemd), test_owner=self.owner,
        )

    def test_empty_history_never_fabricates_success_and_disk_cache_survives_restart_reboot(self) -> None:
        history = collector._load_history(self.paths, self.policy_value, "f" * 64)
        record = history["jobs"]["kis-eod"]
        self.assertEqual(record["last_result"], "unknown")
        self.assertIsNone(record["last_success_at"])

        finish = datetime(2032, 2, 11, 7, 35, tzinfo=timezone.utc)
        start = finish - timedelta(minutes=5)
        collector.update_history_record(
            history, "kis-eod", start=start, end=finish, last_result="success",
            exit_status=0, successful=True,
        )
        collector._write_history(self.paths, history)
        restarted = collector._load_history(self.paths, self.policy_value, "f" * 64)
        self.assertEqual(restarted["jobs"]["kis-eod"]["last_success_at"], "2032-02-11T07:35:00Z")

        reboot_stat = self.root / "proc-stat"
        reboot_stat.write_text("btime 1960000000\n")
        self.assertIsNotNone(collector.parse_btime(str(reboot_stat)))
        failed_start = finish + timedelta(hours=1)
        failed_finish = failed_start + timedelta(seconds=30)
        collector.update_history_record(
            restarted, "kis-eod", start=failed_start, end=failed_finish, last_result="failure",
            exit_status=1, successful=False,
        )
        collector._write_history(self.paths, restarted)
        after_reboot = collector._load_history(self.paths, self.policy_value, "f" * 64)
        self.assertEqual(after_reboot["jobs"]["kis-eod"]["last_success_at"], "2032-02-11T07:35:00Z")
        self.assertEqual(after_reboot["jobs"]["kis-eod"]["last_result"], "failure")

    def test_corrupt_history_is_rejected_instead_of_resetting_last_success(self) -> None:
        self.history_path.write_bytes(b'{"schema_version":"service-status.history.v1","schema_version":"bad"}')
        self.history_path.chmod(0o600)
        with self.assertRaises(collector.CollectError) as caught:
            collector._load_history(self.paths, self.policy_value, "f" * 64)
        self.assertEqual(caught.exception.code, "invalid")

    def test_wire_policy_receipt_missing_and_wrong_release_are_rejected(self) -> None:
        cases = wire_cases()
        missing = cases["not_installed"]
        with self.assertRaises(collector.CollectError) as caught:
            collector._read_policy(self.paths)
        self.assertEqual(caught.exception.code, "missing")
        self.assertFalse(missing["expected"]["accept"])
        self.assertEqual("POLICY_MISSING", missing["expected"]["reason_code"])

        wrong_release = cases["wrong_release"]
        policy_path = Path(self.paths.policy_file)
        write_private_file(policy_path, json.dumps(self.policy_value, separators=(",", ":")).encode() + b"\n")
        other_commit = "b" * 40
        (self.install_root / "current").symlink_to(f"releases/{other_commit}")
        with self.assertRaises(collector.CollectError) as caught:
            collector._read_policy(self.paths)
        self.assertEqual(caught.exception.code, "invalid")
        self.assertFalse(wrong_release["expected"]["accept"])
        self.assertEqual("POLICY_MISMATCH", wrong_release["expected"]["reason_code"])


class FakeRunner:
    def __init__(self, responses: dict[tuple[str, ...], collector.CommandResult]):
        self.responses = responses
        self.calls: list[tuple[str, ...]] = []

    def __call__(self, argv: tuple[str, ...], *, deadline: float) -> collector.CommandResult:
        self.calls.append(argv)
        if argv not in self.responses:
            return collector.CommandResult("command_unavailable")
        return self.responses[argv]


class FakeHostCollectionTests(unittest.TestCase):
    def test_full_fixed_inventory_collects_with_fake_host_only(self) -> None:
        temp = tempfile.TemporaryDirectory(prefix="service-status-host.")
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        owner = test_owner()
        systemd = root / "systemd"
        systemd.mkdir(mode=0o755)
        systemd.chmod(0o755)
        project = "synthetic-service-status"
        commit = "a" * 40
        policy_value = policy_fixture(commit=commit, project=project)
        for name in (f"lagrange-kis-daily-{commit[:7]}.service", f"lagrange-kis-daily-{commit[:7]}.timer",
                     "lagrange-production-backup.timer", "lagrange-production-backup.service",
                     "lagrange-production-backup-verify.timer", "lagrange-production-backup-verify.service",
                     "lagrange-tailscale-tls-renewal.timer", "lagrange-tailscale-tls-renewal.service"):
            path = systemd / name
            path.write_text("synthetic unit\n")
            path.chmod(0o644)
        import hashlib
        service_hash = hashlib.sha256((systemd / f"lagrange-kis-daily-{commit[:7]}.service").read_bytes()).hexdigest()
        timer_hash = hashlib.sha256((systemd / f"lagrange-kis-daily-{commit[:7]}.timer").read_bytes()).hexdigest()
        history = collector.empty_history(policy_value, "f" * 64, service_hash, timer_hash)
        proc_stat = root / "stat"
        boot_epoch = int(datetime(2032, 2, 1, tzinfo=timezone.utc).timestamp())
        proc_stat.write_text(f"cpu 1 2 3\nbtime {boot_epoch}\n")
        paths = collector.CollectorPaths(
            install_root=str(root / "install"), release_root=str(root / "install" / "current"),
            systemd_root=str(systemd), proc_stat=str(proc_stat), docker_bin="/usr/bin/docker",
            systemctl_bin="/usr/bin/systemctl", api_gid=owner[1], test_owner=owner,
        )
        now = datetime(2032, 2, 12, 1, tzinfo=timezone.utc)
        responses: dict[tuple[str, ...], collector.CommandResult] = {}
        for service_id in collector.DOCKER_IDS:
            argv = collector.docker_ps_argv(paths, project, service_id)
            responses[argv] = collector.CommandResult("ok", b"")
        all_units = set()
        for service_id in collector.SYSTEMD_HOST_IDS:
            unit = "docker.service" if service_id == "docker" else "tailscaled.service"
            all_units.add(unit)
        all_units.update(n for pair in collector.SCHEDULES.values() for n in pair)
        all_units.add(f"lagrange-kis-daily-{commit[:7]}.timer")
        all_units.add(f"lagrange-kis-daily-{commit[:7]}.service")
        for unit in all_units:
            properties = collector.SYSTEMD_TIMER_PROPERTIES if unit.endswith(".timer") else collector.SYSTEMD_SERVICE_PROPERTIES
            props = {key: "" for key in properties}
            props.update({
                "LoadState": "loaded", "ActiveState": "active" if unit.endswith(".timer") or unit == "docker.service" else "inactive",
                "SubState": "running", "UnitFileState": "enabled", "Result": "success",
                "FragmentPath": str(systemd / unit),
                "DropInPaths": "",
            })
            if unit.endswith(".timer"):
                props["Unit"] = unit[:-6] + ".service"
                props.update({"LastTriggerUSec": "", "NextElapseUSecRealtime": "2032-02-12T02:00:00Z"})
            elif unit.endswith(".service"):
                props.update({
                    "ExecMainCode": "0", "ExecMainStatus": "0",
                    "ExecMainStartTimestamp": "", "ExecMainExitTimestamp": "",
                })
            if unit.endswith(".service") and "kis-daily" in unit:
                props["ActiveState"] = "inactive"
            if unit.endswith(".timer") and "kis-daily" not in unit:
                props["UnitFileState"] = "enabled"
            argv = collector.systemctl_argv(paths, unit)
            raw = "".join(
                f"{key}={props[key]}\n" for key in properties
            ).encode()
            responses[argv] = collector.CommandResult("ok", raw)
        runner = FakeRunner(responses)
        mono = [100.0]
        def monotonic() -> float:
            mono[0] += 0.001
            return mono[0]
        snapshot, returned_history = collector.collect_snapshot(
            paths, runner=runner, clock=lambda: now, monotonic=monotonic,
            supplied_policy=policy_value, supplied_history=history, manifest_sha256="f" * 64,
        )
        self.assertEqual([x["id"] for x in snapshot["observations"]], list(collector.HOST_IDS))
        self.assertEqual(len(snapshot["observations"]), 20)
        self.assertTrue(all(x["result"] == "ok" for x in snapshot["observations"]))
        self.assertEqual(len(runner.calls), len(collector.DOCKER_IDS) + 2 * (len(collector.SCHEDULES) + 1) + 2)
        self.assertIsNotNone(returned_history)
        collector.validate_snapshot(snapshot, now=now)

    def test_absolute_deadline_fills_all_rows_without_invoking_commands(self) -> None:
        temp = tempfile.TemporaryDirectory(prefix="service-status-deadline.")
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        proc = root / "stat"
        boot_epoch = int(datetime(2032, 2, 1, tzinfo=timezone.utc).timestamp())
        proc.write_text(f"btime {boot_epoch}\n")
        paths = collector.CollectorPaths(proc_stat=str(proc), test_owner=test_owner())
        runner = FakeRunner({})
        now = datetime(2032, 2, 12, 1, tzinfo=timezone.utc)
        snapshot, _ = collector.collect_snapshot(
            paths, runner=runner, clock=lambda: now, monotonic=lambda: 100.0,
            total_deadline_seconds=0, supplied_policy=policy_fixture(),
            supplied_history=None, manifest_sha256="f" * 64,
        )
        self.assertEqual(len(snapshot["observations"]), 20)
        self.assertTrue(all(x["result"] == "deadline" for x in snapshot["observations"]))
        self.assertEqual(runner.calls, [])


if __name__ == "__main__":
    unittest.main()
