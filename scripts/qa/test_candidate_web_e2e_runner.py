"""Stdlib-only regressions for the Linux candidate E2E supervisor."""

from __future__ import annotations

import http.server
import hashlib
import importlib.util
import io
import json
import os
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
RUNNER_PATH = ROOT / "scripts/qa/candidate-web-e2e-runner.py"
SPEC = importlib.util.spec_from_file_location("candidate_web_e2e_runner", RUNNER_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("candidate E2E runner module could not be loaded")
runner = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = runner
SPEC.loader.exec_module(runner)

# The wrapper runs these tests immediately before the real candidate runner.
# Keep regression connections, including their TIME_WAIT sockets, outside all
# candidate API/Web port ranges. Production reservation behavior stays intact.
REGRESSION_API_PORT_RANGE = (38300, 38399)
REGRESSION_WEB_PORT_RANGE = (33100, 33199)
CONFIGURED_STREAM_WEB_PORT = runner.STREAM_WEB_PORT


def free_port(bounds: tuple[int, int], *, excluded: set[int] | None = None) -> int:
    excluded = excluded or set()
    for port in range(bounds[0], bounds[1] + 1):
        if port in excluded:
            continue
        sockets: list[socket.socket] = []
        try:
            v4 = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            sockets.append(v4)
            v4.bind(("127.0.0.1", port))
            v6 = socket.socket(socket.AF_INET6, socket.SOCK_STREAM)
            sockets.append(v6)
            v6.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 1)
            v6.bind(("::1", port))
            return port
        except OSError:
            continue
        finally:
            for sock in sockets:
                sock.close()
    raise AssertionError("no free task test port in bounded range")


class CountingServer:
    def __init__(self, port: int) -> None:
        owner = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self) -> None:
                owner.requests += 1
                self.send_response(200)
                self.send_header("Cache-Control", "no-store")
                self.end_headers()
                self.wfile.write(b"ok\n")

            def log_message(self, _format: str, *_args: object) -> None:
                return

        self.requests = 0
        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", port), Handler)
        self.server.daemon_threads = True
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def close(self) -> None:
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)


API_SERVER = r'''
import http.server, json, os
class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path != "/api/v1/__test/candidate-e2e-ready":
            self.send_response(404); self.end_headers(); return
        body = json.dumps({
            "schema_version": 1,
            "fixture": "candidate-synthetic-api",
            "run_id": os.environ["CANDIDATE_E2E_READINESS_TOKEN"],
            "lane": os.environ["CANDIDATE_E2E_READINESS_LANE"],
            "pid": os.getpid(),
        }).encode()
        self.send_response(200)
        self.send_header("Cache-Control", "no-store")
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers(); self.wfile.write(body)
    def log_message(self, *_args): pass
listen_port = os.environ.get("PORT") or os.environ.get("SYNTHETIC_API_PORT")
http.server.ThreadingHTTPServer(("127.0.0.1", int(listen_port)), Handler).serve_forever()
'''

WEB_RELAY = r'''
import http.client, http.server, json, os, sys
READY="/api/v1/__test/candidate-e2e-ready"
class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        mode = os.environ.get("RELAY_MODE", sys.argv[2] if len(sys.argv) > 2 else "relay")
        if self.path == "/healthz":
            body=b"ok\n"; status=200; content="text/plain"
        elif self.path == READY:
            if mode == "invalid": body=b"{"; status=200
            elif mode == "redirect": body=b""; status=302
            elif mode == "non200": body=b"{}"; status=503
            else:
                api_port = os.environ.get("API_PORT") or sys.argv[3]
                connection=http.client.HTTPConnection("127.0.0.1", int(api_port), timeout=1)
                connection.request("GET", READY)
                response=connection.getresponse(); body=response.read(4097); status=response.status
                connection.close()
                if mode == "stale":
                    try:
                        value=json.loads(body); value["run_id"]="0"*32; body=json.dumps(value).encode()
                    except Exception: pass
            content="application/json"
        else:
            body=b""; status=404; content="text/plain"
        self.send_response(status)
        self.send_header("Cache-Control", "no-store")
        self.send_header("Content-Type", content)
        if status == 302: self.send_header("Location", "http://127.0.0.1/foreign")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers(); self.wfile.write(body)
    def log_message(self, *_args): pass
listen_port = os.environ.get("PORT") or sys.argv[1]
http.server.ThreadingHTTPServer(("127.0.0.1", int(listen_port)), Handler).serve_forever()
'''


def child_env(evidence: Path, extra: dict[str, str] | None = None) -> dict[str, str]:
    env = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "HOME": str(evidence),
        "LANG": "C.UTF-8",
        "LC_ALL": "C.UTF-8",
        "TMPDIR": str(evidence),
    }
    if extra:
        env.update(extra)
    return env


RAW_SENTINEL = "SYNTHETIC_RAW_REPORT_SENTINEL"


def late_reporter(evidence: Path, *, document: dict[str, object], interrupt: int = 0, orphan: bool = False) -> Path:
    """A real reporter writes after cancellation and again from its TERM handler."""
    script = evidence / "late-reporter.py"
    code = r'''
import json, os, pathlib, signal, time
report = pathlib.Path(os.environ["PLAYWRIGHT_JSON_OUTPUT_FILE"])
marker = pathlib.Path(__file__).with_suffix(".writes")
ready = pathlib.Path(__file__).with_suffix(".ready")
def write(kind):
    report.write_text(json.dumps({"errors": [{"message": SENTINEL}]}))
    with marker.open("a") as stream:
        stream.write(kind + "\n")
def term(_signum, _frame):
    write("term")
    os._exit(7)
def serve():
    signal.signal(signal.SIGUSR1, lambda _signum, _frame: write("late"))
    signal.signal(signal.SIGTERM, term)
    ready.write_text(str(os.getpid()))
report.write_text(json.dumps(DOCUMENT))
if ORPHAN:
    read_fd, write_fd = os.pipe()
    if os.fork() == 0:
        os.close(read_fd)
        os.setsid()
        serve()
        os.write(write_fd, b"1")
        os.close(write_fd)
    else:
        os.close(write_fd)
        os.read(read_fd, 1)
        os.close(read_fd)
        os._exit(0)
else:
    serve()
    if INTERRUPT:
        time.sleep(0.1)
        os.kill(os.getppid(), INTERRUPT)
while True:
    time.sleep(0.05)
'''
    script.write_text(
        f"DOCUMENT = {document!r}\nSENTINEL = {RAW_SENTINEL!r}\nINTERRUPT = {interrupt}\nORPHAN = {orphan!r}\n" + code
    )
    return script


def make_supervisor(testcase: unittest.TestCase) -> tuple[tempfile.TemporaryDirectory[str], Path, runner.Supervisor]:
    temp = tempfile.TemporaryDirectory(prefix="candidate-e2e-test-")
    evidence = Path(temp.name)
    for child in ("logs", "tmp", "inventory", "off", "market-stream"):
        (evidence / child).mkdir(mode=0o700)
    return temp, evidence, runner.Supervisor(evidence_dir=evidence)


def fake_harness(
    evidence: Path,
    supervisor: runner.Supervisor,
    reservations: runner.PortReservations,
    *,
    api_port: int,
    web_port: int,
    lane_specs: dict[str, dict[str, object] | list[dict[str, object]]],
    browser_marker: Path,
    api_script: str = API_SERVER,
    relay_mode: str = "relay",
    browser_exit: int = 0,
    reporter_errors: bool = False,
) -> runner.Harness:
    settings = runner.RunnerSettings(ROOT, api_port, web_port, None)
    lane_documents: dict[str, dict[str, object]] = {}
    for lane, value in lane_specs.items():
        specs = value if isinstance(value, list) else [value]
        lane_documents[lane] = inventory_document(specs)
        if lane == "market-stream":
            owner = [spec for spec in specs if spec["file"] == runner.STOCK_BETA_FILE]
            lane_documents["market-stream-results"] = inventory_document([spec for spec in specs if spec not in owner])
            lane_documents["owner-empty-results"] = inventory_document(owner)
    if reporter_errors:
        for document in lane_documents.values():
            document["errors"] = [{"code": "SYNTHETIC_REPORTER_FAILURE"}]
    code = (
        "import json,os,pathlib,sys; "
        f"documents=json.loads({json.dumps(lane_documents)!r}); "
        f"pathlib.Path({str(browser_marker)!r}).write_text('started'); "
        "pathlib.Path(os.environ['PLAYWRIGHT_JSON_OUTPUT_FILE']).write_text("
        "json.dumps(documents.get(os.environ['CANDIDATE_E2E_PHASE'], documents[os.environ['CANDIDATE_E2E_LANE']]))); "
        f"raise SystemExit({browser_exit})"
    )
    return runner.Harness(
        settings=settings,
        evidence_dir=evidence,
        supervisor=supervisor,
        reservations=reservations,
        node=sys.executable,
        playwright_cli=Path("fake-playwright-cli"),
        api_command_factory=lambda _lane, _port, _token: (sys.executable, "-u", "-c", api_script),
        web_command_factory=lambda _lane, port, api: (
            sys.executable,
            "-u",
            "-c",
            WEB_RELAY,
            str(port),
            relay_mode,
            str(api),
        ),
        playwright_prefix=(sys.executable, "-u", "-c", code),
    )


def inventory_document(specs: list[dict[str, object]]) -> dict[str, object]:
    suites: list[dict[str, object]] = []
    for spec in specs:
        file = str(spec["file"])
        describe = {
            runner.ACTUAL_FILE: "Owner service-status browser acceptance",
            runner.STOCK_BETA_FILE: "provider-free Stock Beta V2",
            runner.INTRADAY_FILE: runner.INTRADAY_DESCRIBE,
        }.get(file)
        if describe is not None:
            suites.append(
                {
                    "title": Path(file).name,
                    "file": file,
                    "line": 0,
                    "column": 0,
                    "specs": [],
                    "suites": [
                        {
                            "title": describe,
                            "line": 340,
                            "column": 1,
                            "specs": [spec],
                        }
                    ],
                }
            )
        else:
            suites.append(
                {
                    "title": Path(file).name,
                    "file": file,
                    "line": 0,
                    "column": 0,
                    "specs": [spec],
                }
            )
    return {"suites": suites}


def inventory_spec(
    file: str,
    line: int,
    title: str,
    test_id: str,
    *,
    tags: list[str] | None = None,
    status: str = "expected",
    expected_status: str = "passed",
    attempts: list[dict[str, object]] | None = None,
    annotations: list[dict[str, str]] | None = None,
) -> dict[str, object]:
    return {
        "file": file,
        "line": line,
        "column": 1,
        "title": title,
        "id": test_id,
        "tags": tags or [],
        "tests": [
            {
                "projectName": "chromium",
                "status": status,
                "expectedStatus": expected_status,
                "results": [{"status": "passed", "retry": 0}] if attempts is None else attempts,
                "annotations": annotations or [],
            }
        ],
    }


def parsed_case(spec: dict[str, object]) -> runner.Case:
    result = runner.parse_playwright_inventory(inventory_document([spec]))
    if len(result) != 1:
        raise AssertionError("one synthetic inventory case expected")
    return result[0]


def final_inventory_specs() -> tuple[list[dict[str, object]], list[dict[str, object]], list[dict[str, object]]]:
    """Synthetic JSON only; these counters never stand in for real collection."""
    off: list[dict[str, object]] = []
    for file in runner.OFF_FILES:
        count = runner.FULL_FILE_COUNTS[file] - int(file == runner.ACTUAL_FILE)
        for index in range(count):
            title = f"unchanged-off-{Path(file).stem}-{index}"
            if file == runner.STOCK_BETA_FILE and index == 0:
                title = "renders an empty owner capacity with no signal rows"
            off.append(inventory_spec(file, 40 + index, title, f"{file}:{index}"))
    intraday = [
        inventory_spec(runner.INTRADAY_FILE, 2000 + index, title, f"ws:{index}")
        for index, title in enumerate(runner.INTRADAY_TITLE_MAP.values())
    ]
    stream = inventory_spec(runner.STREAM_FILE, 112, runner.STREAM_TITLE, "ws-status")
    owner = next(spec for spec in off if spec["file"] == runner.STOCK_BETA_FILE and spec["title"] == "renders an empty owner capacity with no signal rows")
    actual = inventory_spec(
        runner.ACTUAL_FILE, 1500,
        "actual DB-backed API router rereads the mounted snapshot and updates policy after atomic replacement @actual",
        "actual-id", tags=["actual"], attempts=[], status="skipped",
    )
    return [*off, *intraday, stream, actual], off, [*intraday, stream, owner]


def snapshot_root(supervisor: runner.Supervisor, info: runner.ProcessInfo) -> runner.ManagedProcess:
    process = mock.Mock(pid=info.pid, returncode=None)
    process.poll.return_value = None
    capture = mock.Mock()
    managed = runner.ManagedProcess("snapshot-root", (), "off", process, info, capture)
    supervisor.roots.append(managed)
    supervisor.owned[info.pid] = runner.ProcessRef(info.pid, info.start_ticks, info.uid, managed.root_key, info.pgid, info.sid)
    return managed


class CandidateE2ERunnerTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        import ctypes

        libc = ctypes.CDLL(None, use_errno=True)
        previous = ctypes.c_int()
        if libc.prctl(37, ctypes.byref(previous), 0, 0, 0) != 0:  # PR_GET_CHILD_SUBREAPER
            raise OSError(ctypes.get_errno(), "prctl")

        def restore_subreaper() -> None:
            if libc.prctl(36, previous.value, 0, 0, 0) != 0:  # PR_SET_CHILD_SUBREAPER
                raise OSError(ctypes.get_errno(), "prctl")

        cls.addClassCleanup(restore_subreaper)
        cls.baseline_children = frozenset(
            (info.pid, info.start_ticks, info.uid)
            for info in runner.proc_snapshot().values()
            if info.ppid == os.getpid()
        )
        runner.enable_child_subreaper()
        stream_port = free_port(REGRESSION_WEB_PORT_RANGE)
        stream_patch = mock.patch.object(runner, "STREAM_WEB_PORT", stream_port)
        stream_patch.start()
        cls.addClassCleanup(stream_patch.stop)

    def tearDown(self) -> None:
        # Full pytest can already own unrelated multiprocessing helpers. Keep
        # their generations intact and still reject every new surviving child.
        children = frozenset(
            (info.pid, info.start_ticks, info.uid)
            for info in runner.proc_snapshot().values()
            if info.ppid == os.getpid()
        )
        self.assertEqual(
            children, self.baseline_children,
            "baseline child changed or task-owned child remains after regression",
        )

    def test_canonical_port_overrides_and_ranges(self) -> None:
        candidate_ports = set(range(runner.API_PORT_RANGE[0], runner.API_PORT_RANGE[1] + 1))
        candidate_ports.update(range(runner.WEB_PORT_RANGE[0], runner.WEB_PORT_RANGE[1] + 1))
        candidate_ports.add(CONFIGURED_STREAM_WEB_PORT)
        for bounds in (REGRESSION_API_PORT_RANGE, REGRESSION_WEB_PORT_RANGE):
            self.assertTrue(candidate_ports.isdisjoint(range(bounds[0], bounds[1] + 1)))
        self.assertNotIn(runner.STREAM_WEB_PORT, candidate_ports)
        self.assertEqual(CONFIGURED_STREAM_WEB_PORT, 33043)
        self.assertEqual(
            runner.parse_port_override("CANDIDATE_E2E_API_PORT", "38200", 38182, runner.API_PORT_RANGE),
            38200,
        )
        for raw in ("038200", "+38200", " 38200", "38200 ", "38179", "38280", "abc"):
            with self.subTest(raw=raw), self.assertRaises(runner.RunnerError):
                runner.parse_port_override("CANDIDATE_E2E_API_PORT", raw, 38182, runner.API_PORT_RANGE)

    def test_busy_api_web_and_pinned_stream_ports_fail_without_http(self) -> None:
        api_port = free_port(REGRESSION_API_PORT_RANGE, excluded={runner.BASE_API_PORT})
        web_port = free_port(REGRESSION_WEB_PORT_RANGE, excluded={runner.BASE_WEB_PORT, runner.STREAM_WEB_PORT})
        for label, port in (("api", api_port), ("base-web-override", web_port), ("pinned-stream", runner.STREAM_WEB_PORT)):
            with self.subTest(port_kind=label):
                try:
                    foreign = CountingServer(port)
                except OSError as exc:
                    self.fail(f"reserved regression port {label} was not available: {exc}")
                reservations = runner.PortReservations()
                try:
                    with self.assertRaises(runner.RunnerError) as raised:
                        reservations.reserve(port)
                    self.assertEqual(raised.exception.code, "PORT_BUSY")
                    self.assertEqual(foreign.requests, 0)
                    self.assertTrue(foreign.thread.is_alive())
                finally:
                    reservations.close_all()
                    foreign.close()

        v6_port = free_port(REGRESSION_WEB_PORT_RANGE, excluded={runner.BASE_WEB_PORT, runner.STREAM_WEB_PORT})
        wildcard = socket.socket(socket.AF_INET6, socket.SOCK_STREAM)
        try:
            wildcard.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 1)
            wildcard.bind(("::", v6_port))
            wildcard.listen(1)
            with self.assertRaises(runner.RunnerError) as raised:
                runner.PortReservations().reserve(v6_port)
            self.assertEqual(raised.exception.code, "PORT_BUSY")
        finally:
            wildcard.close()

    def test_foreign_same_user_listener_after_reservation_release_is_never_probed(self) -> None:
        port = free_port(REGRESSION_WEB_PORT_RANGE, excluded={runner.BASE_WEB_PORT, runner.STREAM_WEB_PORT})
        with tempfile.TemporaryDirectory(prefix="candidate-race-") as temp:
            evidence = Path(temp)
            (evidence / "logs").mkdir(mode=0o700)
            driver = r'''
import importlib.util, json, pathlib, sys
spec=importlib.util.spec_from_file_location("candidate_web_e2e_runner", sys.argv[1])
r=importlib.util.module_from_spec(spec); sys.modules[spec.name]=r; spec.loader.exec_module(r)
r.enable_child_subreaper()
port=int(sys.argv[2]); evidence=pathlib.Path(sys.argv[3])
reservations=r.PortReservations(); reservations.reserve(port); reservations.release(port)
print("RESERVATION_RELEASED", flush=True)
sys.stdin.readline()
supervisor=r.Supervisor(evidence_dir=evidence)
process=supervisor.spawn(label="api-race", scope="off", argv=(sys.executable,"-c","import time; time.sleep(5)"), cwd=evidence, env={"PATH":"/usr/bin:/bin"})
try:
    supervisor.wait_listener(process, port, timeout=0.5)
    state="unexpected_ready"
except r.RunnerError as exc:
    state=exc.code
clean=supervisor.cleanup_all()
reservations.close_all()
print(json.dumps({"state":state,"cleanup_ok":clean}), flush=True)
'''
            process = subprocess.Popen(
                [sys.executable, "-B", "-c", driver, str(RUNNER_PATH), str(port), str(evidence)],
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                env={"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "HOME": temp, "LANG": "C.UTF-8"},
            )
            try:
                self.assertEqual(process.stdout.readline().strip(), "RESERVATION_RELEASED")
                foreign = CountingServer(port)
                assert process.stdin is not None
                process.stdin.write("GO\n")
                process.stdin.flush()
                outcome_line = process.stdout.readline()
                process.wait(timeout=4)
                self.assertEqual(process.returncode, 0, process.stderr.read() if process.stderr else "")
                outcome = json.loads(outcome_line)
                self.assertEqual(outcome, {"state": "FOREIGN_LISTENER", "cleanup_ok": True})
                self.assertEqual(foreign.requests, 0)
                self.assertTrue(foreign.thread.is_alive())
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=2)
                for stream in (process.stdin, process.stdout, process.stderr):
                    if stream is not None:
                        stream.close()
                if "foreign" in locals():
                    foreign.close()

    def test_startup_exit_preserves_exit_code_without_contacting_foreign_listener(self) -> None:
        port = free_port(REGRESSION_API_PORT_RANGE, excluded={runner.BASE_API_PORT})
        foreign = CountingServer(port)
        temp, evidence, supervisor = make_supervisor(self)
        try:
            process = supervisor.spawn(
                label="api-startup-exit",
                scope="off",
                argv=(
                    sys.executable,
                    "-c",
                    "import errno,socket,sys; s=socket.socket();\n"
                    "try:\n s.bind(('127.0.0.1', int(sys.argv[1])))\n"
                    "except OSError as exc:\n raise SystemExit(98 if exc.errno == errno.EADDRINUSE else 99)\n"
                    "raise SystemExit(0)",
                    str(port),
                ),
                cwd=evidence,
                env=child_env(evidence),
            )
            self.assertEqual(process.popen.wait(timeout=2), 98)
            with self.assertRaises(runner.RunnerError) as raised:
                supervisor.wait_listener(process, port, timeout=0.5)
            self.assertEqual(raised.exception.code, "SERVER_EXITED")
            self.assertEqual(raised.exception.exit_code, 98)
            self.assertEqual(foreign.requests, 0)
            self.assertTrue(supervisor.cleanup_all())
            self.assertTrue(foreign.thread.is_alive())
        finally:
            temp.cleanup()
            foreign.close()

    def test_bad_api_and_web_handshakes_block_browser_launch(self) -> None:
        cases = (("api-token", "api"), ("stale", "web"), ("invalid", "web"), ("redirect", "web"), ("non200", "web"))
        for index, (mode, failure_stage) in enumerate(cases):
            with self.subTest(mode=mode):
                api_port = free_port(REGRESSION_API_PORT_RANGE, excluded={runner.BASE_API_PORT + index})
                web_port = free_port(REGRESSION_WEB_PORT_RANGE, excluded={runner.BASE_WEB_PORT, runner.STREAM_WEB_PORT})
                temp, evidence, supervisor = make_supervisor(self)
                reservations = runner.PortReservations()
                reservations.reserve_all((api_port, web_port, runner.STREAM_WEB_PORT))
                browser_marker = evidence / "browser-ran"
                try:
                    spec = inventory_spec(
                        "tests/e2e/backtests.spec.ts", 24, "backtest", f"bad-{index}"
                    )
                    api_script = API_SERVER
                    if failure_stage == "api":
                        api_script = API_SERVER.replace(
                            'os.environ["CANDIDATE_E2E_READINESS_TOKEN"]', '"0" * 32'
                        )
                    harness = fake_harness(
                        evidence,
                        supervisor,
                        reservations,
                        api_port=api_port,
                        web_port=web_port,
                        lane_specs={"off": spec},
                        browser_marker=browser_marker,
                        api_script=api_script,
                        relay_mode=mode,
                    )
                    outcome = harness._run_lane("off", [runner.parse_playwright_inventory(inventory_document([spec]))[0]])
                    expected_error = {
                        "api-token": "API_IDENTITY_MISMATCH",
                        "stale": "WEB_RELAY_IDENTITY_MISMATCH",
                        "invalid": "WEB_RELAY_IDENTITY_INVALID",
                        "redirect": "WEB_RELAY_IDENTITY_INVALID",
                        "non200": "WEB_RELAY_IDENTITY_MISMATCH",
                    }[mode]
                    self.assertEqual(outcome.error, expected_error)
                    self.assertFalse(browser_marker.exists())
                    self.assertTrue(outcome.cleanup_ok)
                    self.assertTrue(outcome.reservation_ok)
                finally:
                    supervisor.cleanup_all()
                    reservations.close_all()
                    temp.cleanup()

    def test_two_serial_lanes_use_fresh_identity_and_clean_owned_children(self) -> None:
        api_port = free_port(REGRESSION_API_PORT_RANGE, excluded={runner.BASE_API_PORT})
        off_web = free_port(REGRESSION_WEB_PORT_RANGE, excluded={runner.BASE_WEB_PORT, runner.STREAM_WEB_PORT})
        temp, evidence, supervisor = make_supervisor(self)
        reservations = runner.PortReservations()
        browser_marker = evidence / "browser-started"
        try:
            reservations.reserve_all((api_port, off_web, runner.STREAM_WEB_PORT))
            _, off_specs, stream_specs = final_inventory_specs()
            harness = fake_harness(
                evidence,
                supervisor,
                reservations,
                api_port=api_port,
                web_port=off_web,
                lane_specs={"off": off_specs, "market-stream": stream_specs},
                browser_marker=browser_marker,
            )
            off_cases = runner.parse_playwright_inventory(inventory_document(off_specs))
            stream_cases = runner.parse_playwright_inventory(inventory_document(stream_specs))
            off = harness._run_lane("off", off_cases)
            stream = harness._run_lane("market-stream", stream_cases)
            self.assertEqual((off.state, off.selected, off.passed, off.skipped), ("passed", 151, 151, 0))
            self.assertEqual((stream.state, stream.selected, stream.passed, stream.skipped), ("passed", 14, 14, 0))
            self.assertEqual(len({case.key for case in off.cases + stream.cases}), 164)
            self.assertNotEqual(off.token_sha256, stream.token_sha256)
            api_roots = [item.root_key for item in supervisor.roots if item.label.startswith("api-")]
            self.assertEqual(len(api_roots), 2)
            self.assertNotEqual(api_roots[0], api_roots[1])
            ws_browsers = [item for item in supervisor.roots if item.label.startswith("playwright-market-stream-")]
            self.assertEqual(len(ws_browsers), 2)
            self.assertTrue(all("--test-list" not in item.argv for item in ws_browsers[:1]))
            self.assertIn("--test-list", ws_browsers[1].argv)
            owner_list = evidence / "inventory/owner-empty.test-list"
            self.assertEqual(owner_list.read_text(), f"[chromium] › stock-beta.spec.ts › {runner.OWNER_EMPTY_TITLE}\n")
            self.assertEqual(owner_list.stat().st_mode & 0o777, 0o600)
            self.assertTrue(browser_marker.exists())
            self.assertTrue(off.cleanup_ok and stream.cleanup_ok)
            self.assertTrue(off.reservation_ok and stream.reservation_ok)
            self.assertTrue(reservations.held(api_port))
            self.assertTrue(reservations.held(off_web))
            self.assertTrue(reservations.held(runner.STREAM_WEB_PORT))
        finally:
            supervisor.cleanup_all()
            reservations.close_all()
            temp.cleanup()

    def test_lane_startup_timeout_cleans_api_and_does_not_launch_browser(self) -> None:
        api_port = free_port(REGRESSION_API_PORT_RANGE, excluded={runner.BASE_API_PORT})
        web_port = free_port(REGRESSION_WEB_PORT_RANGE, excluded={runner.BASE_WEB_PORT, runner.STREAM_WEB_PORT})
        temp, evidence, supervisor = make_supervisor(self)
        reservations = runner.PortReservations()
        browser_marker = evidence / "browser-started"
        try:
            reservations.reserve_all((api_port, web_port, runner.STREAM_WEB_PORT))
            harness = fake_harness(
                evidence,
                supervisor,
                reservations,
                api_port=api_port,
                web_port=web_port,
                lane_specs={
                    "off": inventory_spec("tests/e2e/backtests.spec.ts", 24, "timeout", "timeout-case")
                },
                browser_marker=browser_marker,
                api_script="import time; time.sleep(10)",
            )
            with mock.patch.object(runner, "API_START_TIMEOUT_SECONDS", 0.05):
                outcome = harness._run_lane(
                    "off",
                    [parsed_case(inventory_spec("tests/e2e/backtests.spec.ts", 24, "timeout", "timeout-case"))],
                )
            self.assertEqual(outcome.error, "LISTENER_START_TIMEOUT")
            self.assertTrue(outcome.cleanup_ok and outcome.reservation_ok)
            self.assertFalse(browser_marker.exists())
            self.assertFalse(any(harness._port_has_listener(port) for port in (api_port, web_port)))
        finally:
            supervisor.cleanup_all()
            reservations.close_all()
            temp.cleanup()

    def test_browser_failure_preserves_exit_and_cleans_api_web_and_browser(self) -> None:
        api_port = free_port(REGRESSION_API_PORT_RANGE, excluded={runner.BASE_API_PORT})
        web_port = free_port(REGRESSION_WEB_PORT_RANGE, excluded={runner.BASE_WEB_PORT, runner.STREAM_WEB_PORT})
        temp, evidence, supervisor = make_supervisor(self)
        reservations = runner.PortReservations()
        marker = evidence / "browser-started"
        spec = inventory_spec("tests/e2e/backtests.spec.ts", 24, "browser fails", "browser-failure")
        try:
            reservations.reserve_all((api_port, web_port, runner.STREAM_WEB_PORT))
            harness = fake_harness(
                evidence,
                supervisor,
                reservations,
                api_port=api_port,
                web_port=web_port,
                lane_specs={"off": spec},
                browser_marker=marker,
                browser_exit=7,
                reporter_errors=True,
            )
            outcome = harness._run_lane("off", [parsed_case(spec)])
            self.assertEqual((outcome.error, outcome.exit_code), ("COMMAND_FAILED", 7))
            self.assertTrue(outcome.cleanup_ok and outcome.reservation_ok)
            self.assertTrue(marker.exists())
            self.assertFalse(any(harness._port_has_listener(port) for port in (api_port, web_port)))
        finally:
            supervisor.cleanup_all()
            reservations.close_all()
            temp.cleanup()

    def test_reporter_failure_and_cleanup_failure_preserve_nonzero_browser_precedence(self) -> None:
        for code in (0, 7):
            with self.subTest(browser_exit=code):
                api_port = free_port(REGRESSION_API_PORT_RANGE, excluded={runner.BASE_API_PORT})
                web_port = free_port(REGRESSION_WEB_PORT_RANGE, excluded={runner.BASE_WEB_PORT, runner.STREAM_WEB_PORT})
                temp, evidence, supervisor = make_supervisor(self)
                reservations = runner.PortReservations()
                try:
                    reservations.reserve_all((api_port, web_port, runner.STREAM_WEB_PORT))
                    spec = inventory_spec("tests/e2e/backtests.spec.ts", 24, "reporter failure", f"report-error-{code}")
                    harness = fake_harness(evidence, supervisor, reservations, api_port=api_port, web_port=web_port, lane_specs={"off": spec}, browser_marker=evidence / "started", browser_exit=code, reporter_errors=True)
                    if code:
                        with mock.patch.object(supervisor, "cleanup_all", return_value=False):
                            outcome = harness._run_lane("off", [parsed_case(spec)])
                        self.assertEqual((outcome.error, outcome.exit_code), ("COMMAND_FAILED", 7))
                        self.assertFalse(outcome.cleanup_ok)
                    else:
                        outcome = harness._run_lane("off", [parsed_case(spec)])
                        self.assertEqual(outcome.error, "PLAYWRIGHT_REPORT_ERRORS")
                        self.assertNotEqual(outcome.exit_code, 0)
                        self.assertTrue(outcome.cleanup_ok)
                    self.assertTrue(supervisor.cleanup_all())
                finally:
                    supervisor.cleanup_all()
                    reservations.close_all()
                    temp.cleanup()

    def test_detached_orphan_is_reaped_and_unrelated_control_listener_survives(self) -> None:
        port = free_port(REGRESSION_WEB_PORT_RANGE, excluded={runner.BASE_WEB_PORT, runner.STREAM_WEB_PORT})
        unrelated = CountingServer(port)
        temp, evidence, supervisor = make_supervisor(self)
        pid_file = evidence / "orphan.pid"
        code = r'''
import os, signal, sys, time
pid_file=sys.argv[1]
child=os.fork()
if child == 0:
    os.setsid()
    with open(pid_file,"w") as stream: stream.write(str(os.getpid()))
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    while True: time.sleep(0.05)
os._exit(0)
'''
        try:
            parent = supervisor.spawn(
                label="orphan-maker",
                scope="off",
                argv=(sys.executable, "-c", code, str(pid_file)),
                cwd=evidence,
                env=child_env(evidence),
            )
            self.assertEqual(supervisor.wait(parent, 2), 0)
            deadline = time.monotonic() + 2
            while not pid_file.exists() and time.monotonic() < deadline:
                supervisor.refresh_owned()
                time.sleep(0.02)
            orphan_pid = int(pid_file.read_text())
            self.assertIn(orphan_pid, supervisor.owned)
            with mock.patch.object(runner, "TERM_GRACE_SECONDS", 0.05), mock.patch.object(
                runner, "REAP_GRACE_SECONDS", 0.5
            ):
                self.assertTrue(supervisor.cleanup_all())
            self.assertIsNone(runner.proc_info(orphan_pid))
            self.assertTrue(unrelated.thread.is_alive())
            self.assertEqual(unrelated.requests, 0)
        finally:
            if "orphan_pid" in locals() and (ref := supervisor.owned.get(orphan_pid)) is not None and supervisor.is_current(ref):
                supervisor.safe_signal(ref, signal.SIGKILL)
            supervisor.cleanup_all()
            temp.cleanup()
            unrelated.close()

    def test_descendant_adopted_during_term_is_killed_and_reaped_without_foreign_contact(self) -> None:
        control_port = free_port(REGRESSION_WEB_PORT_RANGE, excluded={runner.BASE_WEB_PORT, runner.STREAM_WEB_PORT})
        unrelated = CountingServer(control_port)
        owned_port = free_port(REGRESSION_WEB_PORT_RANGE, excluded={runner.BASE_WEB_PORT, runner.STREAM_WEB_PORT, control_port})
        temp, evidence, supervisor = make_supervisor(self)
        pid_file = evidence / "shutdown-orphan.pid"
        code = r'''
import os, signal, socket, sys, time
def term(_signum, _frame):
    read_fd, write_fd = os.pipe()
    if os.fork() == 0:
        os.close(read_fd)
        os.setsid()
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        listener = socket.socket()
        listener.bind(("127.0.0.1", int(sys.argv[2])))
        listener.listen(1)
        with open(sys.argv[1], "w") as stream:
            stream.write(str(os.getpid()))
        os.write(write_fd, b"1")
        os.close(write_fd)
        while True: time.sleep(0.05)
    os.close(write_fd)
    os.read(read_fd, 1)
    os.close(read_fd)
    os._exit(0)
signal.signal(signal.SIGTERM, term)
print("ready", flush=True)
while True: time.sleep(0.05)
'''
        try:
            root = supervisor.spawn(label="shutdown-fork", scope="off", argv=(sys.executable, "-c", code, str(pid_file), str(owned_port)), cwd=evidence, env=child_env(evidence))
            deadline = time.monotonic() + 2
            while root.capture.byte_count == 0 and time.monotonic() < deadline:
                supervisor.refresh_owned()
                time.sleep(0.01)
            self.assertGreater(root.capture.byte_count, 0)
            self.assertFalse(pid_file.exists())
            self.assertEqual(set(supervisor.owned), {root.popen.pid})
            original_session = supervisor._signal_session
            original_waitpid = os.waitpid
            reaped: list[tuple[int, int]] = []

            def root_shutdown(process: runner.ManagedProcess, signum: int) -> set[int]:
                members = original_session(process, signum)
                if process is root and signum == signal.SIGTERM:
                    # Let the real root exit before the next discovery snapshot,
                    # making the detached child a kernel adoption with no origin.
                    self.assertEqual(root.popen.wait(timeout=2), 0)
                return members

            def record_reap(pid: int, options: int) -> tuple[int, int]:
                result = original_waitpid(pid, options)
                if result[0]:
                    reaped.append((result[0], os.waitstatus_to_exitcode(result[1])))
                return result

            started = time.monotonic()
            with (
                mock.patch.object(supervisor, "_signal_session", side_effect=root_shutdown),
                mock.patch.object(supervisor, "safe_signal", wraps=supervisor.safe_signal) as individual_signals,
                mock.patch.object(runner.os, "kill", wraps=os.kill) as kills,
                mock.patch.object(runner.os, "killpg", wraps=os.killpg) as groups,
                mock.patch.object(runner.os, "waitpid", side_effect=record_reap),
                mock.patch.object(runner, "TERM_GRACE_SECONDS", 0.05),
                mock.patch.object(runner, "REAP_GRACE_SECONDS", 0.5),
            ):
                self.assertTrue(supervisor.cleanup_all())
            self.assertLess(time.monotonic() - started, 1.0)
            orphan_pid = int(pid_file.read_text())
            orphan = supervisor.owned[orphan_pid]
            self.assertIsNone(orphan.origin_root)
            self.assertIn((orphan_pid, -signal.SIGKILL), reaped)
            self.assertTrue(any(call.args == (orphan, signal.SIGKILL) for call in individual_signals.call_args_list))
            self.assertTrue(all(call.args[0] in (root.popen.pid, orphan_pid) for call in kills.call_args_list))
            self.assertTrue(all(call.args[0] == root.popen.pid for call in groups.call_args_list))
            self.assertIsNone(runner.proc_info(root.popen.pid))
            self.assertIsNone(runner.proc_info(orphan_pid))
            self.assertFalse(any(item.port == owned_port for item in runner.all_listeners()))
            self.assertTrue(unrelated.thread.is_alive())
            self.assertEqual(unrelated.requests, 0)
            self.assertTrue(any(item.port == control_port for item in runner.all_listeners()))
            self.assertTrue(all(item.capture.stream.closed and item.pidfd is None for item in supervisor.roots))
        finally:
            supervisor.cleanup_all()
            temp.cleanup()
            unrelated.close()

    def test_browser_timeout_removes_raw_report_and_reaps_owned_process(self) -> None:
        for collection in (True, False):
            for signum, expected in ((0, 1), (signal.SIGINT, 130), (signal.SIGTERM, 143)):
                with self.subTest(collection=collection, interruption=signum):
                    temp, evidence, supervisor = make_supervisor(self)
                    try:
                        script = late_reporter(evidence, document=inventory_document([]), interrupt=signum)
                        harness = runner.Harness(settings=runner.RunnerSettings(ROOT, 38230, 33090, None), evidence_dir=evidence, supervisor=supervisor, reservations=runner.PortReservations(), node=sys.executable, playwright_cli=script)
                        result_path = evidence / ("inventory" if collection else "off") / "timeout-probe-raw.json"
                        with supervisor.signals, mock.patch.object(runner, "INVENTORY_TIMEOUT_SECONDS", 0.2):
                            with self.assertRaises(runner.RunnerError) as raised:
                                if collection:
                                    harness._playwright_json(phase="timeout-probe", lane="inventory", files=("tests/e2e/",), mode="market-stream", list_only=True)
                                else:
                                    harness._run_browser_phase("off", "timeout-probe", runner.OFF_FILES, 0.2)
                            self.assertEqual(raised.exception.code, "INTERRUPTED" if signum else "COMMAND_TIMEOUT")
                            self.assertEqual(raised.exception.exit_code, expected)
                            self.assertEqual(harness.command_records[-1]["exit_code"], expected)
                            writer_pid = int(script.with_suffix(".ready").read_text())
                            writer = supervisor.owned[writer_pid]
                            self.assertTrue(supervisor.is_current(writer))
                            # Reproduce an early unlink, then require an actual
                            # later write, not a timing that suppresses the race.
                            result_path.unlink(missing_ok=True)
                            self.assertEqual(supervisor.safe_signal(writer, signal.SIGUSR1), "signaled")
                            deadline = time.monotonic() + 2
                            while not script.with_suffix(".writes").exists() and time.monotonic() < deadline:
                                supervisor.refresh_owned()
                                time.sleep(0.01)
                            self.assertEqual(script.with_suffix(".writes").read_text(), "late\n")
                            self.assertTrue(RAW_SENTINEL in result_path.read_text())
                            original_unlink = Path.unlink

                            def remove_after_reap(path: Path, *args: object, **kwargs: object) -> None:
                                if path == result_path:
                                    self.assertIsNone(runner.proc_info(writer_pid))
                                    self.assertTrue(all(root.popen.poll() is not None for root in supervisor.roots))
                                    self.assertTrue(RAW_SENTINEL in path.read_text())
                                original_unlink(path, *args, **kwargs)

                            with mock.patch.object(Path, "unlink", autospec=True, side_effect=remove_after_reap):
                                self.assertTrue(supervisor.cleanup_all())
                        self.assertEqual(script.with_suffix(".writes").read_text(), "late\nterm\n")
                        self.assertFalse(result_path.exists())
                        self.assertFalse(supervisor.raw_reports)
                        self.assertFalse(any(RAW_SENTINEL in path.read_text() for path in evidence.rglob("*.json")))
                        self.assertTrue(all(root.popen.poll() is not None and root.capture.stream.closed and root.pidfd is None for root in supervisor.roots))
                    finally:
                        supervisor.cleanup_all()
                        temp.cleanup()

    def test_exited_reporter_root_retains_descendant_until_post_reap_raw_removal(self) -> None:
        for collection in (True, False):
            with self.subTest(collection=collection):
                temp, evidence, supervisor = make_supervisor(self)
                try:
                    spec = inventory_spec("tests/e2e/backtests.spec.ts", 24, "reporter writer", "lingering-writer")
                    script = late_reporter(evidence, document=inventory_document([spec]), orphan=True)
                    harness = runner.Harness(settings=runner.RunnerSettings(ROOT, 38230, 33090, None), evidence_dir=evidence, supervisor=supervisor, reservations=runner.PortReservations(), node=sys.executable, playwright_cli=script)
                    phase = "exited-writer"
                    if collection:
                        cases = harness._playwright_json(phase=phase, lane="inventory", files=("tests/e2e/",), mode="market-stream", list_only=True)
                    else:
                        cases, code = harness._run_browser_phase("off", phase, runner.OFF_FILES, 2)
                        self.assertEqual(code, 0)
                    self.assertEqual([case.key for case in cases], [parsed_case(spec).key])
                    self.assertEqual(supervisor.roots[0].popen.returncode, 0)
                    writer_pid = int(script.with_suffix(".ready").read_text())
                    supervisor.refresh_owned()
                    writer = supervisor.owned[writer_pid]
                    self.assertTrue(supervisor.is_current(writer))
                    result_path = evidence / ("inventory" if collection else "off") / f"{phase}-raw.json"
                    result_path.unlink(missing_ok=True)
                    self.assertEqual(supervisor.safe_signal(writer, signal.SIGUSR1), "signaled")
                    deadline = time.monotonic() + 2
                    marker = script.with_suffix(".writes")
                    while time.monotonic() < deadline:
                        if marker.exists() and marker.read_text() == "late\n":
                            break
                        supervisor.refresh_owned()
                        time.sleep(0.01)
                    self.assertEqual(marker.read_text(), "late\n")
                    self.assertTrue(RAW_SENTINEL in result_path.read_text())
                    original_unlink = Path.unlink

                    def remove_after_reap(path: Path, *args: object, **kwargs: object) -> None:
                        if path == result_path:
                            self.assertIsNone(runner.proc_info(writer_pid))
                            self.assertTrue(all(root.popen.poll() is not None for root in supervisor.roots))
                            self.assertEqual(marker.read_text(), "late\nterm\n")
                            self.assertTrue(RAW_SENTINEL in path.read_text())
                        original_unlink(path, *args, **kwargs)

                    started = time.monotonic()
                    with mock.patch.object(Path, "unlink", autospec=True, side_effect=remove_after_reap):
                        self.assertTrue(supervisor.cleanup_all())
                    self.assertLess(
                        time.monotonic() - started,
                        runner.TERM_GRACE_SECONDS + runner.REAP_GRACE_SECONDS + 5.0,
                    )
                    self.assertEqual(script.with_suffix(".writes").read_text(), "late\nterm\n")
                    self.assertEqual(supervisor.roots[0].popen.returncode, 0)
                    self.assertIsNone(runner.proc_info(writer_pid))
                    self.assertFalse(result_path.exists())
                    self.assertFalse(supervisor.raw_reports)
                    self.assertFalse(any(RAW_SENTINEL in path.read_text() for path in evidence.rglob("*.json")))
                    self.assertTrue(all(root.capture.stream.closed and root.pidfd is None for root in supervisor.roots))
                finally:
                    supervisor.cleanup_all()
                    temp.cleanup()

    def test_raw_removal_failure_is_explicit_and_preserves_original_browser_exit(self) -> None:
        for browser_exit in (0, 7):
            with self.subTest(browser_exit=browser_exit):
                api_port = free_port(REGRESSION_API_PORT_RANGE, excluded={runner.BASE_API_PORT})
                web_port = free_port(REGRESSION_WEB_PORT_RANGE, excluded={runner.BASE_WEB_PORT, runner.STREAM_WEB_PORT})
                temp, evidence, supervisor = make_supervisor(self)
                reservations = runner.PortReservations()
                try:
                    reservations.reserve_all((api_port, web_port, runner.STREAM_WEB_PORT))
                    spec = inventory_spec("tests/e2e/backtests.spec.ts", 24, "raw removal", "raw-removal-failure")
                    harness = fake_harness(evidence, supervisor, reservations, api_port=api_port, web_port=web_port, lane_specs={"off": spec}, browser_marker=evidence / "started", browser_exit=browser_exit)
                    result_path = evidence / "off/off-results-raw.json"
                    original_unlink = Path.unlink

                    def denied_removal(path: Path, *args: object, **kwargs: object) -> None:
                        if path == result_path:
                            self.assertTrue(all(root.popen.poll() is not None for root in supervisor.roots))
                            self.assertFalse(any(supervisor.is_current(ref) for ref in supervisor.owned.values()))
                            raise PermissionError("synthetic removal denied")
                        original_unlink(path, *args, **kwargs)

                    with mock.patch.object(Path, "unlink", autospec=True, side_effect=denied_removal):
                        outcome = harness._run_lane("off", [parsed_case(spec)])
                    self.assertEqual((outcome.error, outcome.exit_code), ("COMMAND_FAILED", 7) if browser_exit else ("CLEANUP_FAILED", 1))
                    self.assertFalse(outcome.cleanup_ok)
                    self.assertIn("RAW_REPORT_REMOVAL_FAILED", supervisor.cleanup_errors)
                    self.assertTrue(result_path.exists())
                    self.assertIn(result_path, supervisor.raw_reports)
                    self.assertTrue(supervisor.cleanup_all())
                    self.assertFalse(result_path.exists())
                    self.assertFalse(supervisor.raw_reports)
                    self.assertTrue(all(root.capture.stream.closed and root.pidfd is None for root in supervisor.roots))
                finally:
                    supervisor.cleanup_all()
                    reservations.close_all()
                    temp.cleanup()

    def test_generation_mismatch_never_signals_reused_pid(self) -> None:
        temp, evidence, supervisor = make_supervisor(self)
        try:
            process = supervisor.spawn(
                label="generation-check",
                scope="off",
                argv=(sys.executable, "-c", "import time; time.sleep(10)"),
                cwd=evidence,
                env=child_env(evidence),
            )
            assert process.root_info is not None
            wrong = runner.ProcessRef(
                process.popen.pid,
                process.root_info.start_ticks + 1,
                process.root_info.uid,
                process.root_key,
                process.root_info.pgid,
                process.root_info.sid,
            )
            self.assertEqual(supervisor.safe_signal(wrong, signal.SIGTERM), "generation_mismatch")
            self.assertIsNone(process.popen.poll())
            self.assertTrue(supervisor.cleanup_all())
        finally:
            temp.cleanup()

    def test_timeout_signal_and_browser_exit_preserve_original_status(self) -> None:
        temp, evidence, supervisor = make_supervisor(self)
        try:
            slow = supervisor.spawn(
                label="timeout-child",
                scope="off",
                argv=(sys.executable, "-c", "import time; time.sleep(10)"),
                cwd=evidence,
                env=child_env(evidence),
            )
            with self.assertRaises(runner.RunnerError) as raised:
                supervisor.wait(slow, 0.1)
            self.assertEqual(raised.exception.code, "COMMAND_TIMEOUT")
            self.assertTrue(supervisor.cleanup_all())

            next_process = supervisor.spawn(
                label="browser-failure",
                scope="off",
                argv=(sys.executable, "-c", "raise SystemExit(7)"),
                cwd=evidence,
                env=child_env(evidence),
            )
            settings = runner.RunnerSettings(ROOT, 38230, 33090, None)
            harness = runner.Harness(
                settings=settings,
                evidence_dir=evidence,
                supervisor=supervisor,
                reservations=runner.PortReservations(),
                node=sys.executable,
                playwright_cli=Path("playwright"),
            )
            with self.assertRaises(runner.RunnerError) as raised:
                harness._record_command(next_process, next_process.argv, 2, "browser", raise_on_failure=True)
            self.assertEqual(raised.exception.code, "COMMAND_FAILED")
            self.assertEqual(raised.exception.exit_code, 7)
            self.assertTrue(supervisor.cleanup_all())
        finally:
            temp.cleanup()

    def test_term_and_startup_interrupts_return_130_143_and_reap(self) -> None:
        for signum, expected in ((signal.SIGINT, 130), (signal.SIGTERM, 143)):
            with self.subTest(signal=signum):
                temp, evidence, supervisor = make_supervisor(self)
                controller = runner.SignalController()
                supervisor.signals = controller
                signal_code = f"import os,signal,time; time.sleep(0.1); os.kill(os.getppid(), {signum})"
                try:
                    with controller:
                        process = supervisor.spawn(
                            label=f"signal-{signum}",
                            scope="off",
                            argv=(sys.executable, "-c", signal_code),
                            cwd=evidence,
                            env=child_env(evidence),
                        )
                        with self.assertRaises(runner.RunnerError) as raised:
                            supervisor.wait(process, 2)
                        self.assertEqual(raised.exception.code, "INTERRUPTED")
                        self.assertEqual(raised.exception.exit_code, expected)
                        self.assertTrue(supervisor.cleanup_all())
                finally:
                    temp.cleanup()

        temp, evidence, supervisor = make_supervisor(self)
        controller = runner.SignalController()
        supervisor.signals = controller
        port = free_port(REGRESSION_API_PORT_RANGE, excluded={runner.BASE_API_PORT})
        reservations = runner.PortReservations()
        try:
            reservations.reserve(port)
            reservations.release(port)
            api = supervisor.spawn(
                label="startup-interrupt",
                scope="off",
                argv=(sys.executable, "-c", "import time; time.sleep(10)"),
                cwd=evidence,
                env=child_env(evidence),
            )
            interrupter = supervisor.spawn(
                label="startup-interrupter",
                scope="off",
                argv=(sys.executable, "-c", f"import os,signal,time; time.sleep(0.1); os.kill(os.getppid(), {signal.SIGINT})"),
                cwd=evidence,
                env=child_env(evidence),
            )
            with controller:
                with self.assertRaises(runner.RunnerError) as raised:
                    supervisor.wait_listener(api, port, timeout=5)
            self.assertEqual(raised.exception.exit_code, 130)
            self.assertTrue(supervisor.cleanup_all())
            reservations.close_all()
            self.assertIsNotNone(interrupter.popen.returncode)
        finally:
            reservations.close_all()
            temp.cleanup()

    def test_term_ignoring_child_uses_bounded_kill_fallback(self) -> None:
        temp, evidence, supervisor = make_supervisor(self)
        code = "import signal,time; signal.signal(signal.SIGTERM, signal.SIG_IGN); print('ready', flush=True); time.sleep(10)"
        try:
            process = supervisor.spawn(
                label="term-ignorer",
                scope="off",
                argv=(sys.executable, "-c", code),
                cwd=evidence,
                env=child_env(evidence),
            )
            deadline = time.monotonic() + 2
            while process.capture.byte_count == 0 and time.monotonic() < deadline:
                supervisor.refresh_owned()
                time.sleep(0.01)
            self.assertGreater(process.capture.byte_count, 0)
            with mock.patch.object(runner, "TERM_GRACE_SECONDS", 0.05), mock.patch.object(
                runner, "REAP_GRACE_SECONDS", 0.5
            ):
                self.assertTrue(supervisor.cleanup_all())
            self.assertEqual(process.popen.returncode, -signal.SIGKILL)
            self.assertIsNone(runner.proc_info(process.popen.pid))
        finally:
            temp.cleanup()

    def test_cleanup_failure_is_nonzero_until_owned_child_is_manually_reaped(self) -> None:
        temp, evidence, supervisor = make_supervisor(self)
        process = supervisor.spawn(
            label="cleanup-failure",
            scope="off",
            argv=(sys.executable, "-c", "import time; print('capture-ready', flush=True); time.sleep(10)"),
            cwd=evidence,
            env=child_env(evidence),
        )
        try:
            deadline = time.monotonic() + 2
            while process.capture.byte_count == 0 and time.monotonic() < deadline:
                supervisor.refresh_owned()
                time.sleep(0.01)
            self.assertGreater(process.capture.byte_count, 0)
            cleanup_started = time.monotonic()
            with (
                mock.patch.object(supervisor, "_signal_session", return_value=set()),
                mock.patch.object(supervisor, "safe_signal", return_value="permission_denied"),
                mock.patch.object(runner, "TERM_GRACE_SECONDS", 0.02),
                mock.patch.object(runner, "REAP_GRACE_SECONDS", 0.02),
            ):
                self.assertFalse(supervisor.cleanup_all())
            self.assertLess(time.monotonic() - cleanup_started, 1.0)
            self.assertTrue(supervisor.cleanup_errors)
            self.assertIsNone(process.popen.poll())
            self.assertTrue(process.capture.closed)
            self.assertFalse(process.capture.complete)
            self.assertTrue(process.capture.stream.closed)
            capture = process.capture.finish()
            self.assertEqual(capture["output_sha256"], hashlib.sha256(b"capture-ready\n").hexdigest())
            self.assertEqual(capture["capture_reason"], "pipe_open")
        finally:
            if process.popen.poll() is None:
                ref = supervisor.owned[process.popen.pid]
                self.assertEqual(supervisor.safe_signal(ref, signal.SIGKILL), "signaled")
                process.popen.wait(timeout=2)
            supervisor.cleanup_all()
            temp.cleanup()

    def test_buffered_pipe_close_lock_mechanism_is_bounded_and_reproduced(self) -> None:
        read_fd, write_fd = os.pipe()
        stream = io.BufferedReader(io.FileIO(read_fd, closefd=True))
        entered = threading.Event()
        closed = threading.Event()

        def read() -> None:
            entered.set()
            stream.read(65536)

        def close() -> None:
            stream.close()
            closed.set()

        reader = threading.Thread(target=read)
        closer = threading.Thread(target=close)
        try:
            os.write(write_fd, b"synthetic")
            reader.start()
            self.assertTrue(entered.wait(0.5))
            time.sleep(0.02)
            closer.start()
            # The historical synchronous close contends with read's lock while
            # the writer remains open. No child sleep or suite hang is needed.
            self.assertFalse(closed.wait(0.05))
        finally:
            os.close(write_fd)
            reader.join(timeout=1)
            if closer.ident is not None:
                closer.join(timeout=1)
            else:
                stream.close()
        self.assertFalse(reader.is_alive())
        self.assertFalse(closer.is_alive())
        self.assertTrue(closed.is_set())

    def test_nonblocking_capture_complete_hash_and_finite_cancel(self) -> None:
        temp, evidence, supervisor = make_supervisor(self)
        payload = b"synthetic finite output\n"
        try:
            process = supervisor.spawn(
                label="capture-complete", scope="off", argv=(sys.executable, "-c", f"import os; os.write(1,{payload!r})"),
                cwd=evidence, env=child_env(evidence),
            )
            self.assertEqual(supervisor.wait(process, 2), 0)
            self.assertTrue(supervisor.cleanup_all())
            capture = process.capture.finish()
            self.assertTrue(capture["capture_complete"])
            self.assertEqual(capture["output_bytes"], len(payload))
            self.assertEqual(capture["output_sha256"], hashlib.sha256(payload).hexdigest())
            self.assertTrue(process.capture.stream.closed)
            self.assertIsNone(process.pidfd)
        finally:
            supervisor.cleanup_all()
            temp.cleanup()

    def test_caller_session_and_preexisting_children_never_authorize_cleanup(self) -> None:
        uid = os.geteuid()
        caller = runner.proc_info(os.getpid())
        self.assertIsNotNone(caller)
        assert caller is not None
        ancestor = runner.ProcessInfo(caller.ppid, 1, caller.pgid, caller.sid, 100, uid, "S")
        preexisting = runner.ProcessInfo(940001, caller.pid, 940001, 940001, 200, uid, "S")
        tracker = runner.ProcessInfo(940002, caller.pid, caller.pgid, caller.sid, 201, uid, "S")
        peer = runner.ProcessInfo(940003, 1, caller.pgid, caller.sid, 202, uid, "S")
        unrelated_child = runner.ProcessInfo(940004, preexisting.pid, preexisting.pgid, preexisting.sid, 203, uid, "S")
        original = runner.ProcessInfo(940010, caller.pid, 940010, 940010, 210, uid, "S")
        detached = runner.ProcessInfo(940011, original.pid, 940011, 940011, 211, uid, "S")
        snapshot = {info.pid: info for info in (caller, ancestor, preexisting)}
        with mock.patch.object(runner, "proc_snapshot", side_effect=lambda: snapshot):
            temp, _evidence, supervisor = make_supervisor(self)
            root = snapshot_root(supervisor, original)
            try:
                snapshot.update({info.pid: info for info in (tracker, original, detached)})
                with (
                    mock.patch.object(supervisor, "_identity", side_effect=lambda pid: snapshot.get(pid)),
                    mock.patch.object(runner.os, "kill") as kill,
                    mock.patch.object(runner.os, "killpg") as killpg,
                    mock.patch.object(runner.signal, "pidfd_send_signal") as pidfd_signal,
                ):
                    supervisor.refresh_owned()
                    supervisor.refresh_owned()
                    forbidden = (caller, ancestor, preexisting, tracker)
                    for info in forbidden:
                        self.assertNotIn(info.pid, supervisor.owned)
                    self.assertIn(original.pid, supervisor.owned)
                    self.assertEqual(supervisor.owned[detached.pid].origin_root, root.root_key)

                    # Even a contaminated registry cannot turn these identities
                    # into session/parent authority or permit final delivery.
                    for info in forbidden:
                        supervisor.owned[info.pid] = runner.ProcessRef(
                            info.pid, info.start_ticks, info.uid, root.root_key, info.pgid, info.sid,
                        )
                    snapshot.update({info.pid: info for info in (peer, unrelated_child)})
                    supervisor.refresh_owned()
                    self.assertNotIn(peer.pid, supervisor.owned)
                    self.assertNotIn(unrelated_child.pid, supervisor.owned)
                    for info in forbidden:
                        self.assertEqual(
                            supervisor.safe_signal(supervisor.owned[info.pid], signal.SIGTERM),
                            "ownership_unverified",
                        )

                    # Test the group gate with otherwise matching ownership so
                    # generation/origin mismatches cannot mask an unsafe send.
                    for info in (caller, preexisting):
                        foreign_root = runner.ManagedProcess(
                            "foreign-root", (), "off", mock.Mock(pid=info.pid), info, mock.Mock(),
                        )
                        supervisor.owned[info.pid] = runner.ProcessRef(
                            info.pid, info.start_ticks, info.uid, foreign_root.root_key, info.pgid, info.sid,
                        )
                        with mock.patch.object(supervisor, "refresh_owned", return_value={info.pid: info}):
                            self.assertEqual(supervisor._signal_session(foreign_root, signal.SIGTERM), set())
                    kill.assert_not_called()
                    killpg.assert_not_called()
                    pidfd_signal.assert_not_called()

                    self.assertEqual(supervisor.safe_signal(supervisor.owned[detached.pid], signal.SIGTERM), "signaled")
                    kill.assert_called_once_with(detached.pid, signal.SIGTERM)
                    self.assertEqual(supervisor._signal_session(root, signal.SIGTERM), {original.pid})
                    killpg.assert_called_once_with(original.pgid, signal.SIGTERM)
                    pidfd_signal.assert_not_called()
            finally:
                temp.cleanup()

    def test_reused_root_session_and_descendant_generation_never_authorize_foreign_listener_or_signal(self) -> None:
        temp, evidence, supervisor = make_supervisor(self)
        uid = os.geteuid()
        original = runner.ProcessInfo(910001, os.getpid(), 910001, 910001, 100, uid, "S")
        child = runner.ProcessInfo(910002, original.pid, original.pgid, original.sid, 101, uid, "S")
        root = snapshot_root(supervisor, original)
        snapshot = {original.pid: original, child.pid: child}
        try:
            with mock.patch.object(runner, "proc_snapshot", side_effect=lambda: snapshot):
                supervisor.refresh_owned()
                child_ref = supervisor.owned[child.pid]
                snapshot = {
                    original.pid: runner.dataclasses.replace(original, start_ticks=900, ppid=1),
                    child.pid: runner.dataclasses.replace(child, start_ticks=901, ppid=1),
                    910003: runner.ProcessInfo(910003, 1, original.pgid, original.sid, 902, uid, "S"),
                }
                with (
                    mock.patch.object(supervisor, "_identity", side_effect=lambda pid: snapshot.get(pid)),
                    mock.patch.object(runner.os, "kill") as kill,
                    mock.patch.object(runner.os, "killpg") as killpg,
                    mock.patch.object(runner, "all_listeners", return_value=[runner.Listener(socket.AF_INET, "127.0.0.1", 38181, 44)]),
                    mock.patch.object(runner, "socket_owners", return_value={44: {child.pid}}),
                    mock.patch.object(runner.http.client, "HTTPConnection") as http,
                ):
                    supervisor.refresh_owned()
                    self.assertEqual(supervisor.owned[original.pid].start_ticks, 100)
                    self.assertEqual(supervisor.owned[child.pid].start_ticks, 101)
                    self.assertNotIn(910003, supervisor.owned)
                    self.assertEqual(supervisor.safe_signal(child_ref, signal.SIGTERM), "generation_mismatch")
                    self.assertEqual(supervisor._signal_session(root, signal.SIGTERM), set())
                    with self.assertRaises(runner.RunnerError) as raised:
                        supervisor.request(root, 38181, runner.READY_PATH)
                    self.assertEqual(raised.exception.code, "ROOT_GENERATION_MISMATCH")
                    kill.assert_not_called()
                    killpg.assert_not_called()
                    http.assert_not_called()
        finally:
            temp.cleanup()

    def test_verified_member_and_parent_chains_survive_root_exit_and_detachment_with_uid_gate(self) -> None:
        temp, evidence, supervisor = make_supervisor(self)
        uid = os.geteuid()
        original = runner.ProcessInfo(920001, os.getpid(), 920001, 920001, 110, uid, "S")
        member = runner.ProcessInfo(920002, original.pid, original.pgid, original.sid, 111, uid, "S")
        root = snapshot_root(supervisor, original)
        snapshot = {original.pid: original, member.pid: member}
        try:
            with mock.patch.object(runner, "proc_snapshot", side_effect=lambda: snapshot):
                supervisor.refresh_owned()
                snapshot = {
                    member.pid: runner.dataclasses.replace(member, ppid=os.getpid(), pgid=member.pid, sid=member.pid),
                    920003: runner.ProcessInfo(920003, member.pid, member.pid, member.pid, 112, uid, "S"),
                    920004: runner.ProcessInfo(920004, os.getpid(), 920004, 920004, 113, uid, "Z"),
                    920005: runner.ProcessInfo(920005, member.pid, member.pid, member.pid, 114, uid + 1, "S"),
                }
                supervisor.refresh_owned()
                self.assertEqual(supervisor.owned[920003].origin_root, root.root_key)
                self.assertIsNone(supervisor.owned[920004].origin_root)
                self.assertNotIn(920005, supervisor.owned)
                with mock.patch.object(supervisor, "_identity", side_effect=lambda pid: snapshot.get(pid)), mock.patch.object(runner.os, "kill") as kill:
                    self.assertEqual(supervisor.safe_signal(supervisor.owned[920003], signal.SIGTERM), "signaled")
                    kill.assert_called_once_with(920003, signal.SIGTERM)
                with mock.patch.object(runner.os, "waitpid", return_value=(920004, 0)) as waitpid:
                    supervisor._reap_adopted(snapshot)
                    self.assertIn(mock.call(920004, os.WNOHANG), waitpid.call_args_list)
                    self.assertNotIn(mock.call(920005, os.WNOHANG), waitpid.call_args_list)
        finally:
            temp.cleanup()

    def test_live_root_does_not_authorize_reused_descendant_uid_or_parent_generation(self) -> None:
        temp, evidence, supervisor = make_supervisor(self)
        uid = os.geteuid()
        original = runner.ProcessInfo(930001, os.getpid(), 930001, 930001, 120, uid, "S")
        child = runner.ProcessInfo(930002, original.pid, original.pgid, original.sid, 121, uid, "S")
        detached = runner.ProcessInfo(930003, child.pid, 930003, 930003, 122, uid, "S")
        root = snapshot_root(supervisor, original)
        snapshot = {info.pid: info for info in (original, child, detached)}
        try:
            with mock.patch.object(runner, "proc_snapshot", side_effect=lambda: snapshot):
                supervisor.refresh_owned()
                child_ref, detached_ref = supervisor.owned[child.pid], supervisor.owned[detached.pid]
                snapshot = {
                    original.pid: original,
                    child.pid: runner.dataclasses.replace(child, start_ticks=999),
                    detached.pid: runner.dataclasses.replace(detached, uid=uid + 1),
                    930004: runner.ProcessInfo(930004, detached.pid, 930004, 930004, 123, uid, "S"),
                    930005: runner.ProcessInfo(930005, original.pid, original.pgid, original.sid, 124, uid + 1, "S"),
                    930006: runner.ProcessInfo(930006, child.pid, 930006, 930006, 125, uid, "S"),
                }
                with mock.patch.object(supervisor, "_identity", side_effect=lambda pid: snapshot.get(pid)), mock.patch.object(runner.os, "kill") as kill, mock.patch.object(runner.os, "killpg") as killpg:
                    supervisor.refresh_owned()
                    self.assertEqual(supervisor.owned[child.pid], child_ref)
                    self.assertEqual(supervisor.owned[detached.pid], detached_ref)
                    self.assertNotIn(930004, supervisor.owned)
                    self.assertNotIn(930005, supervisor.owned)
                    self.assertNotIn(930006, supervisor.owned)
                    self.assertEqual(supervisor.safe_signal(child_ref, signal.SIGKILL), "generation_mismatch")
                    self.assertEqual(supervisor.safe_signal(detached_ref, signal.SIGKILL), "uid_mismatch")
                    self.assertEqual(supervisor._signal_session(root, signal.SIGKILL), set())
                    with mock.patch.object(runner, "all_listeners", return_value=[runner.Listener(socket.AF_INET, "127.0.0.1", 38181, 55)]), mock.patch.object(runner, "socket_owners", return_value={55: {child.pid}}), mock.patch.object(runner.http.client, "HTTPConnection") as http:
                        with self.assertRaises(runner.RunnerError) as raised:
                            supervisor.request(root, 38181, runner.READY_PATH)
                        self.assertEqual(raised.exception.code, "FOREIGN_LISTENER")
                        http.assert_not_called()
                    kill.assert_not_called()
                    killpg.assert_not_called()
        finally:
            temp.cleanup()

    def test_unverified_original_root_is_not_recovered_from_reused_pid_and_pidfd_unwinds_owned_launch(self) -> None:
        temp, evidence, supervisor = make_supervisor(self)
        try:
            with mock.patch.object(supervisor, "_identity", return_value=None):
                with self.assertRaises(runner.RunnerError) as raised:
                    supervisor.spawn(label="unverified-root", scope="off", argv=(sys.executable, "-c", "import time; time.sleep(10)"), cwd=evidence, env=child_env(evidence))
            self.assertEqual(raised.exception.code, "CHILD_IDENTITY_UNVERIFIED")
            root = supervisor.roots[0]
            reused = runner.ProcessInfo(root.popen.pid, os.getpid(), root.popen.pid, root.popen.pid, 999999999, os.geteuid(), "S")
            with mock.patch.object(runner, "proc_snapshot", return_value={reused.pid: reused}), mock.patch.object(runner.os, "kill") as kill, mock.patch.object(runner.os, "killpg") as killpg:
                supervisor.refresh_owned()
                self.assertIsNone(root.root_info)
                self.assertNotIn(reused.pid, supervisor.owned)
                with mock.patch.object(runner.http.client, "HTTPConnection") as http:
                    with self.assertRaises(runner.RunnerError) as readiness:
                        supervisor.request(root, 38181, runner.READY_PATH)
                    self.assertEqual(readiness.exception.code, "CHILD_IDENTITY_UNVERIFIED")
                    http.assert_not_called()
                self.assertTrue(supervisor.cleanup_all())
                kill.assert_not_called()
                killpg.assert_not_called()
            self.assertIsNotNone(root.popen.returncode)
            self.assertIsNone(root.pidfd)
        finally:
            supervisor.cleanup_all()
            temp.cleanup()

    def test_inventory_requires_final_title_map_counts_and_only_named_overlap(self) -> None:
        full_specs, off_specs, stream_specs = final_inventory_specs()
        full = runner.parse_playwright_inventory(inventory_document(full_specs))
        off = runner.parse_playwright_inventory(inventory_document(off_specs))
        stream = runner.parse_playwright_inventory(inventory_document(stream_specs))
        receipt = runner.compare_lane_inventories(full, {"off": off, "market-stream": stream})
        self.assertEqual(receipt["state"], "complete")
        self.assertEqual((receipt["full_count"], receipt["required_synthetic_count"], receipt["selected_count"], receipt["selected_execution_count"]), (165, 164, 164, 165))
        self.assertEqual(len(receipt["approved_overlap"]), 1)
        self.assertIn("renders an empty owner capacity", receipt["approved_overlap"][0])
        self.assertEqual(receipt["actual_case"]["state"], "dedicated_not_executed_in_synthetic_CI")
        self.assertEqual(receipt["actual_case"]["line"], 1500)
        self.assertEqual(receipt["contract_errors"], [])
        self.assertEqual(len(receipt["title_map"]), 12)

        retired = next(old for old, new in runner.INTRADAY_TITLE_MAP.items() if old != new)
        changed = [dict(spec) for spec in full_specs]
        changed_intraday = next(spec for spec in changed if spec["file"] == runner.INTRADAY_FILE and spec["title"] == runner.INTRADAY_TITLE_MAP[retired])
        changed_intraday["title"] = retired
        wrong = runner.compare_lane_inventories(runner.parse_playwright_inventory(inventory_document(changed)), {"off": off, "market-stream": stream})
        self.assertEqual(wrong["state"], "coverage_incomplete")
        self.assertIn("INTRADAY_TITLE_MAP_MISMATCH", wrong["contract_errors"])

    def test_inventory_rejects_missing_extra_duplicates_and_unapproved_cross_lane_overlap(self) -> None:
        full_specs, off_specs, stream_specs = final_inventory_specs()
        parse = lambda specs: runner.parse_playwright_inventory(inventory_document(specs))
        full, off, stream = parse(full_specs), parse(off_specs), parse(stream_specs)
        for name, lanes in (
            ("missing", {"off": off[:-1], "market-stream": stream}),
            ("missing-owner-branch", {"off": off, "market-stream": [case for case in stream if case.key.title != runner.OWNER_EMPTY_TITLE]}),
            ("duplicate-within-lane", {"off": off + off[:1], "market-stream": stream}),
            ("unapproved-overlap", {"off": off, "market-stream": stream + off[:1]}),
            ("unknown-lane", {"off": off, "market-stream": stream, "other": []}),
            ("actual-selected", {"off": off + [case for case in full if runner.is_actual_case(case)], "market-stream": stream}),
        ):
            with self.subTest(name=name):
                result = runner.compare_lane_inventories(full, lanes)
                self.assertEqual(result["state"], "coverage_incomplete")
        unknown = inventory_spec("tests/e2e/future.spec.ts", 1, "future unknown", "future")
        result = runner.compare_lane_inventories(parse(full_specs + [unknown]), {"off": off, "market-stream": stream})
        self.assertIn("FULL_FILE_COUNTS_MISMATCH", result["contract_errors"])
        self.assertTrue(any("future.spec.ts" in key for key in result["missing"]))
        result = runner.compare_lane_inventories(full, {"off": off + parse([unknown]), "market-stream": stream})
        self.assertTrue(any("future.spec.ts" in key for key in result["extra"]))
        duplicate = dict(off_specs[0], id="different-id", line=9999)
        with self.assertRaises(runner.RunnerError) as raised:
            parse([off_specs[0], duplicate])
        self.assertEqual(raised.exception.code, "INVENTORY_DUPLICATE_KEY")
        with self.assertRaises(runner.RunnerError) as raised:
            runner.parse_playwright_inventory({"suites": [{"file": "tests/e2e/../../outside.ts", "line": 0, "column": 0, "specs": []}]})
        self.assertEqual(raised.exception.code, "INVENTORY_FILE_OUTSIDE_E2E")
        second_actual = inventory_spec("tests/e2e/backtests.spec.ts", 999, "unexpected @actual", "actual-other", tags=["actual"])
        with self.assertRaises(runner.RunnerError) as raised:
            runner.compare_lane_inventories(parse(full_specs + [second_actual]), {"off": off, "market-stream": stream})
        self.assertEqual(raised.exception.code, "ACTUAL_INVENTORY_CARDINALITY")

    def test_inventory_rejects_omitted_owner_beta_simulation_inventory(self) -> None:
        full_specs, off_specs, stream_specs = final_inventory_specs()
        simulation_file = runner.OWNER_BETA_SIMULATIONS_FILE
        self.assertEqual(sum(spec["file"] == simulation_file for spec in full_specs), 22)
        self.assertEqual(sum(spec["file"] == simulation_file for spec in off_specs), 22)
        parse = lambda specs: runner.parse_playwright_inventory(inventory_document(specs))
        full, off, stream = parse(full_specs), parse(off_specs), parse(stream_specs)
        omitted = [case for case in off if case.key.file != simulation_file]

        result = runner.compare_lane_inventories(full, {"off": omitted, "market-stream": stream})

        self.assertEqual(result["state"], "coverage_incomplete")
        self.assertIn("OFF_COUNT_MISMATCH", result["contract_errors"])
        self.assertIn("OFF_SELECTION_MISMATCH", result["contract_errors"])
        self.assertEqual(len(result["missing"]), 22)
        self.assertTrue(all(simulation_file in missing for missing in result["missing"]))

    def test_collection_derives_exact_owner_test_list_and_rejects_old_source(self) -> None:
        full_specs, off_specs, stream_specs = final_inventory_specs()
        parse = lambda specs: runner.parse_playwright_inventory(inventory_document(specs))
        full, off, stream = parse(full_specs), parse(off_specs), parse(stream_specs)
        stock = [case for case in full if case.key.file == runner.STOCK_BETA_FILE]
        owner = [case for case in stock if case.key.title == runner.OWNER_EMPTY_TITLE]
        main = [case for case in stream if case.key.file != runner.STOCK_BETA_FILE]
        temp, evidence, supervisor = make_supervisor(self)
        harness = runner.Harness(settings=runner.RunnerSettings(ROOT, 38230, 33090, None), evidence_dir=evidence, supervisor=supervisor, reservations=runner.PortReservations(), node=sys.executable, playwright_cli=Path("fake-cli"))
        responses = {"full-inventory": full, "off-inventory": off, "market-stream-inventory": main, "owner-stock-inventory": stock, "owner-empty-inventory": owner}
        calls = []
        def collect(**kwargs):
            calls.append(kwargs)
            self.assertTrue(kwargs["list_only"])
            if kwargs["phase"] == "owner-empty-inventory":
                self.assertEqual(kwargs["test_list"].read_text(), f"[chromium] › stock-beta.spec.ts › {runner.OWNER_EMPTY_TITLE}\n")
            return responses[kwargs["phase"]]
        try:
            with mock.patch.object(harness, "_playwright_json", side_effect=collect):
                collected = harness.collect_inventories()
            self.assertEqual(tuple(map(len, collected)), (165, 151, 14))
            self.assertEqual(calls[0]["mode"], "market-stream")
            self.assertEqual(calls[1]["files"], runner.OFF_FILES)
            self.assertEqual(calls[2]["files"], (runner.INTRADAY_FILE, runner.STREAM_FILE))
            self.assertEqual(calls[-1]["files"], (runner.STOCK_BETA_FILE,))
            self.assertEqual(supervisor.roots, [])
        finally:
            temp.cleanup()
        # The final contract fails before server launch if a successor is absent.
        temp, evidence, supervisor = make_supervisor(self)
        harness.evidence_dir, harness.supervisor = evidence, supervisor
        responses["market-stream-inventory"] = main[:-1]
        try:
            with mock.patch.object(harness, "_playwright_json", side_effect=collect):
                with self.assertRaises(runner.RunnerError) as raised:
                    harness.collect_inventories()
            self.assertEqual(raised.exception.code, "FINAL_INVENTORY_CONTRACT_MISMATCH")
            self.assertEqual(supervisor.roots, [])
        finally:
            temp.cleanup()

    def test_reporter_relative_paths_errors_and_inventory_skip_are_rejected_explicitly(self) -> None:
        spec = inventory_spec("backtests.spec.ts", 24, "root-relative", "root-relative")
        doc = inventory_document([spec])
        doc["config"] = {"rootDir": str(runner.WEB_DIR / "tests/e2e")}
        self.assertEqual(runner.parse_playwright_inventory(doc)[0].key.file, "tests/e2e/backtests.spec.ts")
        doc["errors"] = [{"finite_test_error": True}]
        with self.assertRaises(runner.RunnerError) as raised:
            runner.parse_playwright_inventory(doc)
        self.assertEqual(raised.exception.code, "PLAYWRIGHT_REPORT_ERRORS")
        full_specs, off_specs, stream_specs = final_inventory_specs()
        full, off, stream = [runner.parse_playwright_inventory(inventory_document(specs)) for specs in (full_specs, off_specs, stream_specs)]
        skipped = runner.dataclasses.replace(off[0], expected_status="skipped", annotations=({"type": "skip"},))
        result = runner.compare_lane_inventories(full, {"off": [skipped, *off[1:]], "market-stream": stream})
        self.assertIn("INVENTORY_SKIPPED_OR_IGNORED", result["contract_errors"])

    def test_lane_result_gate_rejects_skipped_flaky_and_actual_with_reasons(self) -> None:
        skip = parsed_case(
            inventory_spec(
                "tests/e2e/backtests.spec.ts",
                24,
                "skipped with reason",
                "s1",
                status="skipped",
                expected_status="skipped",
                attempts=[{"status": "skipped"}],
                annotations=[{"type": "skip", "description": "current mode boundary"}],
            )
        )
        self.assertEqual(runner.summarize_lane_results([skip])["skipped"], 1)
        self.assertIn("current mode boundary", runner.safe_case(skip)["annotations"][0]["description"])
        with self.assertRaises(runner.RunnerError) as raised:
            runner.validate_lane_results([skip], [skip])
        self.assertEqual(raised.exception.code, "RESULT_SKIPPED")

        flaky = parsed_case(
            inventory_spec(
                "tests/e2e/backtests.spec.ts",
                24,
                "flaky test",
                "f1",
                status="flaky",
                attempts=[{"status": "failed"}, {"status": "passed"}],
            )
        )
        with self.assertRaises(runner.RunnerError) as raised:
            runner.validate_lane_results([flaky], [flaky])
        self.assertEqual(raised.exception.code, "RESULT_FLAKY")

        retry = parsed_case(inventory_spec("tests/e2e/backtests.spec.ts", 24, "single later retry", "retry", attempts=[{"status": "passed", "retry": 1}]))
        with self.assertRaises(runner.RunnerError) as raised:
            runner.validate_lane_results([retry], [retry])
        self.assertEqual(raised.exception.code, "RESULT_RETRY")
        self.assertEqual(runner.summarize_lane_results([retry])["passed"], 0)
        self.assertEqual(runner.summarize_lane_results([retry])["retry_attempts"], 1)
        self.assertEqual(runner.safe_case(retry)["attempts"], [{"status": "passed", "retry": 1}])

        normal = parsed_case(inventory_spec("tests/e2e/backtests.spec.ts", 24, "normal", "normal"))
        moved = runner.dataclasses.replace(normal, key=runner.dataclasses.replace(normal.key, line=25))
        for actual_results in ([], [normal, normal], [moved]):
            with self.subTest(results=len(actual_results)), self.assertRaises(runner.RunnerError):
                runner.validate_lane_results([normal], actual_results)
        interrupted = runner.dataclasses.replace(normal, results=({"status": "interrupted", "retry": 0},), status="unexpected")
        with self.assertRaises(runner.RunnerError) as raised:
            runner.validate_lane_results([normal], [interrupted])
        self.assertEqual(raised.exception.code, "RESULT_UNEXPECTED")

        actual = parsed_case(
            inventory_spec(
                runner.ACTUAL_FILE,
                1038,
                "actual DB-backed API router rereads the mounted snapshot and updates policy after atomic replacement @actual",
                "actual-id",
                tags=["actual"],
                attempts=[],
            )
        )
        with self.assertRaises(runner.RunnerError) as raised:
            runner.validate_lane_results([actual], [actual])
        self.assertEqual(raised.exception.code, "ACTUAL_CASE_SELECTED")

    def test_environment_dotenv_and_private_evidence_contract(self) -> None:
        api_port = free_port(REGRESSION_API_PORT_RANGE, excluded={runner.BASE_API_PORT})
        web_port = free_port(REGRESSION_WEB_PORT_RANGE, excluded={runner.BASE_WEB_PORT, runner.STREAM_WEB_PORT})
        temp, evidence, supervisor = make_supervisor(self)
        reservations = runner.PortReservations()
        try:
            harness = fake_harness(
                evidence,
                supervisor,
                reservations,
                api_port=api_port,
                web_port=web_port,
                lane_specs={},
                browser_marker=evidence / "unused",
            )
            api_env = harness.minimal_env(lane="off", mode="off", token="a" * 32)
            playwright_env = harness.minimal_env(lane="off", mode="off")
            forbidden = {
                "HTTP_PROXY",
                "HTTPS_PROXY",
                "ALL_PROXY",
                "SERVICE_STATUS_SESSION_FILE",
                "SERVICE_STATUS_REAL_PHASE",
                "SERVICE_STATUS_ACTUAL_PROOF_FILE",
            }
            self.assertFalse(forbidden & api_env.keys())
            self.assertFalse(forbidden & playwright_env.keys())
            self.assertNotIn(runner.READY_ENV_TOKEN, playwright_env)
            self.assertEqual(api_env[runner.READY_ENV_TOKEN], "a" * 32)

            # Development servers must not attempt CDN instrumentation requests.
            # The runner constructs every child environment, including inventory.
            for ambient_value in (None, "0", "1", "unexpected"):
                with mock.patch.dict(os.environ, {}, clear=True):
                    if ambient_value is not None:
                        os.environ["NEXT_PUBLIC_DISABLE_REACT_DEVTOOLS"] = ambient_value
                    os.environ["UNAPPROVED_INHERITED_VALUE"] = "sentinel"
                    for lane, mode in ((None, None), ("off", "off"), ("market-stream", "owner_only")):
                        with self.subTest(ambient_value=ambient_value, lane=lane):
                            child_env = harness.minimal_env(lane=lane, mode=mode)
                            self.assertEqual(child_env["NEXT_PUBLIC_DISABLE_REACT_DEVTOOLS"], "1")
                            self.assertNotIn("UNAPPROVED_INHERITED_VALUE", child_env)

            safe_root = Path(temp.name) / "safe-root"
            safe_root.mkdir(mode=0o700)
            target = safe_root / "target"
            self.assertFalse(target.exists())
            private_evidence, lock_fd = runner.create_evidence(safe_root)
            try:
                self.assertEqual(target.stat().st_mode & 0o777, 0o700)
                self.assertEqual(private_evidence.stat().st_mode & 0o777, 0o700)
                self.assertEqual((private_evidence / "logs").stat().st_mode & 0o777, 0o700)
                receipt = private_evidence / "receipt.json"
                runner.write_private(receipt, b"{}\n")
                self.assertEqual(receipt.stat().st_mode & 0o777, 0o600)
            finally:
                os.close(lock_fd)

            target_mode = target.stat().st_mode & 0o777
            second_evidence, second_lock_fd = runner.create_evidence(safe_root)
            try:
                self.assertNotEqual(second_evidence, private_evidence)
                self.assertEqual(target.stat().st_mode & 0o777, target_mode)
                self.assertEqual(second_evidence.stat().st_mode & 0o777, 0o700)
            finally:
                os.close(second_lock_fd)

            victim = Path(temp.name) / "victim"
            victim.mkdir(mode=0o700)
            victim_marker = victim / "marker"
            victim_marker.write_bytes(b"unchanged\n")
            victim_mode = victim.stat().st_mode & 0o777
            symlink_root = Path(temp.name) / "symlink-root"
            symlink_root.mkdir(mode=0o700)
            symlink_target = symlink_root / "target"
            symlink_target.symlink_to(victim, target_is_directory=True)
            with self.subTest(unsafe_root=symlink_root.name):
                with self.assertRaises(runner.RunnerError) as raised:
                    runner.create_evidence(symlink_root)
                self.assertEqual(raised.exception.code, "TASK_TARGET_NOT_REAL_DIRECTORY")
            self.assertTrue(symlink_target.is_symlink())
            self.assertEqual(symlink_target.readlink(), victim)
            self.assertFalse((symlink_root / ".candidate-web-e2e.lock").exists())
            self.assertEqual(victim_marker.read_bytes(), b"unchanged\n")
            self.assertEqual(victim.stat().st_mode & 0o777, victim_mode)
            self.assertFalse((victim / ".candidate-web-e2e.lock").exists())

            dangling_root = Path(temp.name) / "dangling-root"
            dangling_root.mkdir(mode=0o700)
            dangling_target = dangling_root / "target"
            dangling_victim = Path(temp.name) / "dangling-victim"
            dangling_target.symlink_to(dangling_victim, target_is_directory=True)
            with self.assertRaises(runner.RunnerError) as raised:
                runner.create_evidence(dangling_root)
            self.assertEqual(raised.exception.code, "TASK_TARGET_NOT_REAL_DIRECTORY")
            self.assertTrue(dangling_target.is_symlink())
            self.assertEqual(dangling_target.readlink(), dangling_victim)
            self.assertFalse(dangling_victim.exists())
            self.assertFalse((dangling_root / ".candidate-web-e2e.lock").exists())

            file_root = Path(temp.name) / "file-root"
            file_root.mkdir(mode=0o700)
            file_target = file_root / "target"
            file_target.write_bytes(b"do not replace\n")
            file_mode = file_target.stat().st_mode & 0o777
            with self.assertRaises(runner.RunnerError) as raised:
                runner.create_evidence(file_root)
            self.assertEqual(raised.exception.code, "TASK_TARGET_NOT_REAL_DIRECTORY")
            self.assertEqual(file_target.read_bytes(), b"do not replace\n")
            self.assertEqual(file_target.stat().st_mode & 0o777, file_mode)
            self.assertFalse((file_root / ".candidate-web-e2e.lock").exists())

            web_dir = Path(temp.name) / "web"
            web_dir.mkdir()
            runner.verify_dotenv_absent(web_dir)
            (web_dir / ".env.local").write_text("do not read")
            with self.assertRaises(runner.RunnerError) as raised:
                runner.verify_dotenv_absent(web_dir)
            self.assertEqual(raised.exception.code, "DOTENV_PRESENT")
        finally:
            reservations.close_all()
            supervisor.cleanup_all()
            temp.cleanup()


    def test_hosted_lane_evidence_keeps_named_locations_and_each_result(self) -> None:
        good = runner.Case(runner.CaseKey("chromium", runner.STOCK_BETA_FILE, 42, "good fixture", "id1"),
                           (), "expected", "passed", ({"status": "passed", "retry": 0},), ())
        failed = runner.Case(runner.CaseKey("chromium", runner.STOCK_BETA_FILE, 56, "failed fixture", "id2"),
                             (), "unexpected", "passed",
                             ({"status": "timedOut", "retry": 0,
                               "errorLocation": {"file": "/private/checkout/apps/web/" + runner.STOCK_BETA_FILE,
                                                 "line": 61, "column": 9},
                               "error": {"message": "PRIVATE_ERROR", "stack": "PRIVATE_STACK"}},), ())
        outcome = runner.LaneOutcome("off", "command_failed", 1, 2, 1, 0, 0,
                                     "COMMAND_FAILED", True, cases=[good, failed])
        row = runner.hosted_lane_evidence(outcome)
        self.assertEqual((row["lane"], row["state"], row["exit_code"]), ("off", "command_failed", 1))
        self.assertEqual((row["selected"], row["passed"], row["case_count"]), (2, 1, 2))
        self.assertTrue(row["cleanup_ok"])
        self.assertTrue(row["reservation_ok"])
        self.assertEqual([case["line"] for case in row["cases"]], [42, 56])
        self.assertEqual([case["status"] for case in row["cases"]], ["expected", "unexpected"])
        self.assertEqual(row["cases"][1]["attempts"], [{
            "status": "timedOut", "retry": 0,
            "error_location": {"file": runner.STOCK_BETA_FILE, "line": 61, "column": 9},
        }])
        self.assertNotIn("/private/checkout", json.dumps(row))
        self.assertNotIn("PRIVATE_ERROR", json.dumps(row))
        self.assertNotIn("PRIVATE_STACK", json.dumps(row))
        self.assertNotEqual(row["cases"][0]["identity_sha256"], row["cases"][1]["identity_sha256"])
        output = io.StringIO()
        with mock.patch("sys.stdout", output):
            runner.emit_hosted_lane_evidence(outcome)
        self.assertEqual(json.loads(output.getvalue().split(":", 1)[1]), row)

        # A step timeout can lack a stack/errorLocation in Playwright's JSON.
        # Only the fixed phase identifier may leave the private report.
        switched = runner.Case(
            runner.CaseKey("chromium", "tests/e2e/owner-beta-recommendations.spec.ts", 273,
                           "fixture", "id3"), (), "unexpected", "passed",
            ({"status": "failed", "retry": 0, "steps": [
                {"title": "R2_DAILY_SWITCH_SETUP"},
                {"title": "R2_DAILY_SWITCH_SELECT", "error": {"message": "PRIVATE_ERROR"}},
                {"title": "R2_DAILY_SWITCH_SELECT", "error": {"stack": "PRIVATE_STACK"}},
                {"title": "R2_DAILY_SWITCH_CLEANUP"},
            ]},), ())
        projected = runner.hosted_lane_evidence(runner.LaneOutcome(
            "off", "command_failed", 1, 1, 0, 0, 0, "COMMAND_FAILED", True, cases=[switched]))
        self.assertEqual(projected["cases"][0]["attempts"], [{
            "status": "failed", "retry": 0, "failed_steps": ["R2_DAILY_SWITCH_SELECT"],
        }])
        self.assertNotIn("PRIVATE_ERROR", json.dumps(projected))
        self.assertNotIn("PRIVATE_STACK", json.dumps(projected))

        for message, expected in [
            ("Error: apiRequestContext.post: read ECONNRESET", "CONNECTION_RESET"),
            ("apiRequestContext.get: connect ECONNREFUSED", "CONNECTION_REFUSED"),
            ("apiRequestContext.post: write EPIPE", "BROKEN_PIPE"),
            ("apiRequestContext.fetch: connect ETIMEDOUT", "NETWORK_TIMEOUT"),
            ("apiRequestContext.post: socket hang up", "SOCKET_CLOSED"),
            ("apiRequestContext.post: Timeout 30000ms exceeded.", "REQUEST_TIMEOUT"),
            ("apiRequestContext.post: PRIVATE_SENTINEL", "OTHER_REQUEST_FAILURE"),
        ]:
            with self.subTest(category=expected):
                case = runner.Case(
                    runner.CaseKey("chromium", "tests/e2e/candidates.spec.ts", 17, "fixture", "id"),
                    (), "unexpected", "passed", ({"status": "failed", "retry": 0,
                    "error": {"message": message + "\nPRIVATE_SENTINEL", "stack": "PRIVATE_SENTINEL"}},), ())
                projected = runner.hosted_lane_evidence(runner.LaneOutcome(
                    "off", "command_failed", 1, 1, 0, 0, 0, "COMMAND_FAILED", True, cases=[case]))
                self.assertEqual(projected["cases"][0]["attempts"], [{
                    "status": "failed", "retry": 0, "request_error": expected,
                }])
                self.assertEqual((projected["state"], projected["exit_code"], projected["passed"]),
                                 ("command_failed", 1, 0))
                self.assertEqual(projected["retry_attempts"], 0)
                self.assertNotIn("PRIVATE_SENTINEL", json.dumps(projected))

    def test_hosted_lane_evidence_localizes_nested_owner_assertions_with_bounds(self) -> None:
        owner_file = "tests/e2e/owner-beta-recommendations.spec.ts"
        sensitive = {
            "message": "MESSAGE_SENTINEL https://private.invalid/?token=TOKEN_SENTINEL",
            "stack": "STACK_SENTINEL cookie=COOKIE_SENTINEL",
            "expected": "EXPECTED_SENTINEL",
            "actual": "ACTUAL_SENTINEL",
        }
        unknown_title = "UNKNOWN_TITLE_SENTINEL R2_DAILY_SWITCH_VERIFY_COVERAGE"
        steps = [{
            "title": "R2_DAILY_SWITCH_VERIFY",
            "error": sensitive,
            "steps": [
                {"title": "R2_DAILY_SWITCH_VERIFY_COVERAGE", "error": sensitive},
                {"title": unknown_title, "error": sensitive},
                {"title": "R2_DAILY_SWITCH_VERIFY_STATUS", "error": sensitive},
                {"title": "R2_DAILY_SWITCH_VERIFY_COVERAGE", "error": sensitive},
            ],
        }]
        case = runner.Case(
            runner.CaseKey("chromium", owner_file, 273, "fixture", "id"),
            (), "unexpected", "passed",
            ({"status": "failed", "retry": 0, "steps": steps},), (),
        )
        outcome = runner.LaneOutcome(
            "off", "command_failed", 1, 1, 0, 0, 0, "COMMAND_FAILED", True, cases=[case],
        )
        projected = runner.hosted_lane_evidence(outcome)
        failed_steps = projected["cases"][0]["attempts"][0]["failed_steps"]
        self.assertEqual(failed_steps, [
            "R2_DAILY_SWITCH_VERIFY",
            "R2_DAILY_SWITCH_VERIFY_COVERAGE",
            "R2_DAILY_SWITCH_VERIFY_STATUS",
        ])
        self.assertEqual(projected, runner.hosted_lane_evidence(outcome))
        encoded = json.dumps(projected)
        for value in [*sensitive.values(), unknown_title, "https://private.invalid/", "COOKIE_SENTINEL"]:
            self.assertNotIn(value, encoded)

        for source, status in ((owner_file, "passed"), (runner.STOCK_BETA_FILE, "failed")):
            ineligible = runner.Case(
                runner.CaseKey("chromium", source, 273, "fixture", "id"),
                (), "unexpected", "passed",
                (({"status": status, "retry": 0, "steps": steps}),), (),
            )
            result = runner.hosted_lane_evidence(runner.LaneOutcome(
                "off", "command_failed", 1, 1, 0, 0, 0, "COMMAND_FAILED", True,
                cases=[ineligible],
            ))
            self.assertNotIn("failed_steps", result["cases"][0]["attempts"][0])

        def nested_result(depth: int) -> dict[str, object]:
            nested: dict[str, object] = {
                "title": "R2_DAILY_SWITCH_VERIFY_DUE_AT",
                "error": {},
            }
            for _ in range(depth):
                nested = {"title": "unlisted wrapper", "steps": [nested]}
            deeply_nested = runner.Case(
                runner.CaseKey("chromium", owner_file, 273, "fixture", "id"),
                (), "unexpected", "passed",
                (({"status": "failed", "retry": 0, "steps": [nested]}),), (),
            )
            return runner.hosted_lane_evidence(runner.LaneOutcome(
                "off", "command_failed", 1, 1, 0, 0, 0, "COMMAND_FAILED", True,
                cases=[deeply_nested],
            ))

        depth_four = nested_result(runner.HOSTED_LANE_STEP_MAX_DEPTH)
        self.assertEqual(
            depth_four["cases"][0]["attempts"][0]["failed_steps"],
            ["R2_DAILY_SWITCH_VERIFY_DUE_AT"],
        )
        depth_five = nested_result(runner.HOSTED_LANE_STEP_MAX_DEPTH + 1)
        self.assertNotIn("failed_steps", depth_five["cases"][0]["attempts"][0])

        phase = {
            "title": "R2_DAILY_SWITCH_VERIFY",
            "error": {},
            "steps": [
                *({"title": "unlisted"} for _ in range(runner.HOSTED_LANE_STEP_MAX_NODES - 2)),
                {"title": "R2_DAILY_SWITCH_VERIFY_COVERAGE", "error": {}},
                {"title": "R2_DAILY_SWITCH_VERIFY_STATUS", "error": {}},
            ],
        }
        capped = runner.Case(
            runner.CaseKey("chromium", owner_file, 273, "fixture", "id"),
            (), "unexpected", "passed",
            (({"status": "interrupted", "retry": 0, "steps": [phase]}),), (),
        )
        capped_result = runner.hosted_lane_evidence(runner.LaneOutcome(
            "off", "command_failed", 1, 1, 0, 0, 0, "COMMAND_FAILED", True,
            cases=[capped],
        ))
        self.assertEqual(
            capped_result["cases"][0]["attempts"][0]["failed_steps"],
            ["R2_DAILY_SWITCH_VERIFY", "R2_DAILY_SWITCH_VERIFY_COVERAGE"],
        )

        beyond_legacy_top_level_limit = runner.Case(
            runner.CaseKey("chromium", owner_file, 273, "fixture", "id"),
            (), "unexpected", "passed",
            (({"status": "failed", "retry": 0, "steps": [
                *({"title": "unlisted", "error": {}} for _ in range(32)),
                {"title": "R2_DAILY_SWITCH_VERIFY", "error": {}},
            ]}),), (),
        )
        legacy_limit_result = runner.hosted_lane_evidence(runner.LaneOutcome(
            "off", "command_failed", 1, 1, 0, 0, 0, "COMMAND_FAILED", True,
            cases=[beyond_legacy_top_level_limit],
        ))
        self.assertNotIn("failed_steps", legacy_limit_result["cases"][0]["attempts"][0])

    def test_hosted_lane_evidence_excludes_untrusted_text_and_token_fields(self) -> None:
        sentinel = "PRIVATE_SENTINEL_DO_NOT_EMIT"
        case = runner.Case(runner.CaseKey(sentinel, sentinel, -1, sentinel, sentinel),
                           (sentinel,), sentinel, sentinel,
                           ({"status": sentinel, "retry": sentinel, "error": sentinel, "stdout": sentinel},),
                           ({"type": sentinel, "description": sentinel},))
        outcome = runner.LaneOutcome(sentinel, sentinel, 1, 1, 0, 0, 0,
                                     sentinel, False, token_sha256=sentinel, cases=[case])
        row = runner.hosted_lane_evidence(outcome)
        self.assertNotIn(sentinel, json.dumps(row))
        self.assertEqual((row["lane"], row["state"], row["error"]), ("UNKNOWN", "other_failure", "OTHER_FAILURE"))
        self.assertEqual(row["cases"][0]["file"], "UNLISTED")
        self.assertIsNone(row["cases"][0]["line"])
        self.assertIsNone(row["cases"][0]["attempts"][0]["retry"])
        self.assertNotIn("token_sha256", row)
        self.assertNotIn("annotations", row["cases"][0])
        self.assertNotIn("title", row["cases"][0])
        self.assertFalse(row["cleanup_ok"])
        for error, status, source in [
            (sentinel, "failed", runner.STOCK_BETA_FILE),
            ({"message": [sentinel]}, "failed", runner.STOCK_BETA_FILE),
            ({"message": "PRIVATE_ECONNRESET " + sentinel}, "failed", runner.STOCK_BETA_FILE),
            ({"message": "apiRequestContext.post: socket hang up"}, "passed", runner.STOCK_BETA_FILE),
            ({"message": "apiRequestContext.post: socket hang up"}, "failed", sentinel),
        ]:
            case = runner.Case(runner.CaseKey("chromium", source, 42, "fixture", "id"),
                               (), "unexpected", "passed", ({"status": status, "retry": 0,
                               "error": error},), ())
            projected = runner.hosted_lane_evidence(runner.LaneOutcome(
                "off", "command_failed", 1, 1, 0, 0, 0, "COMMAND_FAILED", True, cases=[case]))
            self.assertNotIn("request_error", projected["cases"][0]["attempts"][0])
            self.assertNotIn(sentinel, json.dumps(projected))
        for location in [
            {"file": sentinel, "line": 1, "column": 1},
            {"file": runner.STOCK_BETA_FILE, "line": True, "column": 1},
            {"file": runner.STOCK_BETA_FILE, "line": 0, "column": 1},
            {"file": runner.STOCK_BETA_FILE, "line": 1, "column": -1},
            {"file": runner.STOCK_BETA_FILE, "line": 1, "column": 1_000_001},
            {"file": runner.STOCK_BETA_FILE, "line": sentinel, "column": 1},
            {"file": runner.STOCK_BETA_FILE, "line": 1},
            sentinel,
        ]:
            failed = runner.Case(runner.CaseKey("chromium", runner.STOCK_BETA_FILE, 42, "fixture", "id"),
                                 (), "unexpected", "passed",
                                 ({"status": "failed", "retry": 0, "errorLocation": location},), ())
            rejected = runner.hosted_lane_evidence(runner.LaneOutcome(
                "off", "command_failed", 1, 1, 0, 0, 0, "COMMAND_FAILED", True, cases=[failed]))
            self.assertNotIn("error_location", rejected["cases"][0]["attempts"][0])
            self.assertNotIn(sentinel, json.dumps(rejected))

        for steps in [
            sentinel, None, {},
            [sentinel, None, [], {"title": sentinel, "error": {"message": sentinel}}],
            [{"title": "R2_DAILY_SWITCH_SELECT", "error": sentinel}],
            [{"title": "R2_DAILY_SWITCH_SELECT"}],
            [{"title": ["R2_DAILY_SWITCH_SELECT"], "error": {}}],
            [{"title": "R2_DAILY_SWITCH_SELECT", "steps": [
                {"title": sentinel, "error": {"message": sentinel}},
            ]}],
        ]:
            failed = runner.Case(
                runner.CaseKey("chromium", "tests/e2e/owner-beta-recommendations.spec.ts", 273,
                               "fixture", "id"), (), "unexpected", "passed",
                ({"status": "timedOut", "retry": 0, "steps": steps},), ())
            projected = runner.hosted_lane_evidence(runner.LaneOutcome(
                "off", "command_failed", 1, 1, 0, 0, 0, "COMMAND_FAILED", True, cases=[failed]))
            self.assertNotIn("failed_steps", projected["cases"][0]["attempts"][0])
            self.assertNotIn(sentinel, json.dumps(projected))
        for source, status in [
            (runner.STOCK_BETA_FILE, "failed"),
            ("tests/e2e/owner-beta-recommendations.spec.ts", "passed"),
        ]:
            case = runner.Case(runner.CaseKey("chromium", source, 273, "fixture", "id"),
                               (), "unexpected", "passed", ({"status": status, "retry": 0,
                               "steps": [{"title": "R2_DAILY_SWITCH_SELECT", "error": {}}]},), ())
            projected = runner.hosted_lane_evidence(runner.LaneOutcome(
                "off", "command_failed", 1, 1, 0, 0, 0, "COMMAND_FAILED", True, cases=[case]))
            self.assertNotIn("failed_steps", projected["cases"][0]["attempts"][0])


if __name__ == "__main__":
    unittest.main()
