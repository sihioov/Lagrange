#!/usr/bin/env python3
"""Provider-free tests of dotenv selection and fixed market-stream mounts."""

import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
OPS = ROOT / "scripts/ops"
VALID = {
    "OWNER_INTRADAY_QUOTES_MODE": "owner_only",
    "OWNER_INTRADAY_QUOTE_TRANSPORT": "market_ws",
    "KIS_READ_COORDINATION_MODE": "shared_required",
    "OWNER_INTRADAY_SESSION_WINDOWS_SOURCE": "operational_v1",
    "KIS_MARKET_STREAM_CREDENTIAL_SLOT_ID": "00000000-0000-4000-8000-000000000001",
    "KIS_MARKET_STREAM_GRANT_ID": "00000000-0000-4000-8000-000000000002",
    "KIS_MARKET_STREAM_CONTRACT_SHA256": "a" * 64,
    "OWNER_MARKET_STREAM_ORIGIN": "https://quotes.example",
}


class MarketStreamComposeTests(unittest.TestCase):
    def configure(self, values, shell=None, overlays=True):
        with tempfile.TemporaryDirectory(prefix="lagrange-ws-compose-") as tmp:
            root = Path(tmp) / "source with spaces"
            compose = root / "deploy/compose"
            compose.mkdir(parents=True)
            if overlays:
                for name in ("intraday", "intraday-operational", "market-stream"):
                    (compose / f"compose.{name}.yml").write_text('{"services":{}}\n')
            envfile = Path(tmp) / "fixture.env"
            envfile.write_text("".join(f"{key}={value}\n" for key, value in values.items()))
            env = {"PATH": "/usr/bin:/bin", "LANG": "C", "LC_ALL": "C"}
            env.update(shell or {})
            result = subprocess.run(
                ["bash", "-c", '''
set -euo pipefail
source "$1/lib/dotenv.sh"
source "$1/lib/kis-read-compose.sh"
dotenv_load "$2"
if ! kis_read_compose_configure "$3"; then
  printf 'ERROR:%s\n' "$KIS_READ_COMPOSE_ERROR"
  exit 1
fi
printf 'TRANSPORT:%s\n' "$KIS_READ_COMPOSE_QUOTE_TRANSPORT"
if [ "${#KIS_READ_COMPOSE_FILE_ARGS[@]}" -gt 0 ]; then
  printf '%s\n' "${KIS_READ_COMPOSE_FILE_ARGS[@]}"
fi
''', "market-stream-config-test", str(OPS), str(envfile), str(root)],
                env=env, capture_output=True, text=True, timeout=5, check=False,
            )
            return result.returncode, result.stdout.splitlines(), result.stderr

    def test_default_and_explicit_off_need_no_market_state_or_pins(self):
        for values in ({}, {"OWNER_INTRADAY_QUOTE_TRANSPORT": "rest"},
                       {"OWNER_INTRADAY_QUOTE_TRANSPORT": "market_ws"}):
            with self.subTest(values=values):
                code, lines, error = self.configure(values, overlays=False)
                self.assertEqual((code, error), (0, ""))
                self.assertEqual(len(lines), 1)

    def test_active_mode_selects_fixed_overlay_last(self):
        for source, expected in (("operational_v1", 3), ("release_v1", 2)):
            code, lines, error = self.configure({**VALID, "OWNER_INTRADAY_SESSION_WINDOWS_SOURCE": source})
            self.assertEqual((code, error), (0, ""))
            self.assertEqual(len(lines), 1 + 2 * expected)
            self.assertEqual(lines[0], "TRANSPORT:market_ws")
            self.assertTrue(lines[2].endswith("/compose.intraday.yml"))
            self.assertTrue(lines[-1].endswith("/compose.market-stream.yml"))
            self.assertEqual(lines[1::2], ["-f"] * expected)

    def test_missing_or_noncanonical_metadata_fails(self):
        bad = {
            "OWNER_INTRADAY_QUOTE_TRANSPORT": ["", "ws", "REST"],
            "KIS_MARKET_STREAM_CREDENTIAL_SLOT_ID": ["", "00000000-0000-0000-0000-000000000000", "A" * 36],
            "KIS_MARKET_STREAM_GRANT_ID": ["", "00000000-0000-0000-0000-000000000000"],
            "KIS_MARKET_STREAM_CONTRACT_SHA256": ["", "A" * 64, "sha256:" + "a" * 64],
            "OWNER_MARKET_STREAM_ORIGIN": ["", "http://quotes.example", "https://QUOTES.example",
                "https://quotes.example/", "https://quotes.example?q=x", "https://a@quotes.example",
                "https://*.example", "https://quotes.example:65536"],
            "DB_APP_MAX_CONNECTIONS": ["", "0", "23", "024", "-1", "4294967296", "x"],
        }
        for key, values in bad.items():
            for value in values:
                with self.subTest(key=key, value=value):
                    code, lines, _ = self.configure({**VALID, key: value})
                    self.assertEqual(code, 1)
                    self.assertTrue(lines[0].startswith("ERROR:"))

    def test_origin_port_and_pool_boundaries(self):
        for origin in ("https://quotes.example:8443", "https://[::1]:443"):
            for size in ("24", "32", "4294967295"):
                code, _, error = self.configure({**VALID, "OWNER_MARKET_STREAM_ORIGIN": origin,
                                                  "DB_APP_MAX_CONNECTIONS": size})
                self.assertEqual((code, error), (0, ""))

    def test_shell_overrides_and_file_aliases_fail_closed(self):
        keys = [key for key in VALID if key.startswith("KIS_MARKET_STREAM") or
                key in ("OWNER_INTRADAY_QUOTE_TRANSPORT", "OWNER_MARKET_STREAM_ORIGIN")]
        keys.append("DB_APP_MAX_CONNECTIONS")
        for key in keys:
            with self.subTest(key=key):
                self.assertEqual(self.configure(VALID, {key: "changed"})[0], 1)
                self.assertEqual(self.configure(VALID, {key + "_FILE": ""})[0], 1)
                self.assertEqual(self.configure({**VALID, key + "_FILE": ""})[0], 1)
        self.assertEqual(self.configure({}, {"OWNER_INTRADAY_QUOTE_TRANSPORT": "market_ws"})[0], 1)

    def test_pins_do_not_replace_shared_coordination_or_overlay(self):
        code, lines, _ = self.configure({**VALID, "KIS_READ_COORDINATION_MODE": "legacy"})
        self.assertEqual(code, 1)
        self.assertEqual(lines, ["ERROR:owner_intraday_quotes_requires_shared"])
        self.assertEqual(self.configure(VALID, overlays=False)[0], 1)

    def test_overlay_has_no_provider_credentials_and_only_runner_state(self):
        services = json.loads((ROOT / "deploy/compose/compose.market-stream.yml").read_text())["services"]
        self.assertEqual(set(services), {"web", "api-server", "owner-equity-v2-runner"})
        for name, service in services.items():
            self.assertNotIn("secrets", service)
            self.assertNotIn("networks", service)
            self.assertNotIn("ports", service)
            self.assertEqual(service["environment"]["OWNER_INTRADAY_QUOTE_TRANSPORT"], "market_ws")
            self.assertFalse(any("APP_KEY" in key or "APP_SECRET" in key or "URL" in key
                                 for key in service["environment"]))
            if name != "owner-equity-v2-runner":
                self.assertNotIn("volumes", service)
        api = services["api-server"]["environment"]
        runner = services["owner-equity-v2-runner"]["environment"]
        for key in VALID:
            if key.startswith("KIS_MARKET_STREAM"):
                self.assertEqual(api[key], runner[key])
        self.assertIn("OWNER_MARKET_STREAM_ORIGIN", api)
        self.assertNotIn("OWNER_MARKET_STREAM_ORIGIN", runner)
        self.assertEqual(api["DB_APP_MAX_CONNECTIONS"], "${DB_APP_MAX_CONNECTIONS:-32}")
        volumes = services["owner-equity-v2-runner"]["volumes"]
        self.assertEqual(len(volumes), 2)
        for volume, leaf, readonly in zip(volumes, ("kis-market-stream", "kis-market-stream-locks"), (False, True)):
            self.assertEqual(volume["type"], "bind")
            self.assertEqual(volume["target"], "/run/lagrange/" + leaf)
            self.assertTrue(volume["source"].startswith("${LAGRANGE_RUNTIME_STATE_DIR:?"))
            self.assertTrue(volume["source"].endswith("/" + leaf))
            self.assertIs(volume["read_only"], readonly)
            self.assertIs(volume["bind"]["create_host_path"], False)


if __name__ == "__main__":
    unittest.main()
