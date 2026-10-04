#!/usr/bin/env node
// Actual browser -> cached nginx -> Unix Axum API -> owned runtime fixture.
// Inert without a separately reviewed gate and its SHA. This never starts PG,
// creates demand itself, or fabricates an API response or a market-stream proof.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { constants } from "node:fs";
import { chmod, lstat, mkdir, open, readFile, readlink, rename, rm, unlink, writeFile } from "node:fs/promises";
import { createServer as httpServer } from "node:http";
import { connect, createServer as tcpServer } from "node:net";
import { dirname, extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, expect } from "@playwright/test";
import { auditPublication } from "./publication-audit.mjs";
import { auditResources } from "./resource-audit.mjs";
import { PassiveStreamDiagnostics, domMetadata } from "./passive-stream-diagnostics.mjs";

const REPOSITORY = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const PRIVATE = "/tmp/lagrange-kis-stream-completion-20261001-653ff5d84407";
const BINDING = `${PRIVATE}/wp3c2-runtime-binding.json`;
const BINDING_SHA = "f173e4eed80a6543535f574fa003d6d77d521bfb41d6c532f6c44442a2f4f268";
const CLUSTER = "/tmp/lagrange-kis-c2-4b4fed7166084d97a6b67ae3ba294be0";
const NGINX_IMAGE = "sha256:7377697a821c131a924a7105fafbe7414db4e9fcc77a6f08f776f33f141ec3f8";
const UID = 1000;
const CASE = "integrated_runtime_api_fixture";
const sha = (bytes) => createHash("sha256").update(bytes).digest("hex");
const delay = (ms) => new Promise((yes) => setTimeout(yes, ms));
const exactKeys = (object, keys) => assert.deepEqual(Object.keys(object).sort(), [...keys].sort(), "gate fields differ");
const summaryError = (error) => ({ name: error?.name ?? "Error", message: String(error?.message ?? "unknown harness error").slice(0, 1200) });
const baseEnv = { PATH: "/usr/bin:/bin", LANG: "C", LC_ALL: "C" };

assert.equal(Number(process.versions.node.split(".")[0]), 24);
assert.equal(process.getuid(), UID);
process.umask(0o077);
assert.equal(process.argv.length, 4, "Pass one reviewed gate and its exact SHA256");
const gatePath = resolve(process.argv[2]);
assert.match(gatePath, new RegExp(`^${PRIVATE}/wp7e[0-9a-z-]*-gate\\.json$`));
assert.match(process.argv[3], /^[0-9a-f]{64}$/);

async function regular(path, max, { privateMode = false, appendSnapshot = false } = {}) {
  const before = await lstat(path);
  assert(before.isFile() && !before.isSymbolicLink() && before.nlink === 1 && before.uid === UID, "unexpected file identity");
  if (privateMode) assert.equal(before.mode & 0o777, 0o600, "private file mode");
  assert(before.size <= max, "bounded file size exceeded");
  const fd = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
  try {
    const own = await fd.stat();
    assert(own.isFile() && own.dev === before.dev && own.ino === before.ino && (appendSnapshot ? own.size >= before.size && own.size <= max : own.size === before.size), "file changed before read");
    // Observations are append-only while the fixture runs. Read the bounded
    // prefix visible at fstat; a concurrent append must not look like pin drift.
    let bytes;
    if (appendSnapshot) {
      bytes = Buffer.alloc(own.size);
      let offset = 0;
      while (offset < bytes.length) {
        const result = await fd.read(bytes, offset, bytes.length - offset, offset);
        assert(result.bytesRead > 0, "observation file truncated"); offset += result.bytesRead;
      }
    } else bytes = await fd.readFile();
    const after = await fd.stat();
    assert(bytes.length <= max && (appendSnapshot ? after.size >= own.size && after.size <= max : after.size === own.size && after.mtimeMs === own.mtimeMs), "file changed during read");
    return bytes;
  } finally { await fd.close(); }
}
const gateBytes = await regular(gatePath, 1024 * 1024, { privateMode: true });
assert.equal(sha(gateBytes), process.argv[3], "gate SHA mismatch");
const gate = JSON.parse(gateBytes);
exactKeys(gate, ["schema_version", "package", "operation", "evidence_dir", "scenario", "fixture_binary", "fixture_binary_sha256", "controller_sha256", "max_work_seconds", "nginx_sha256", "chromium_executable", "chromium_sha256", "assets", "source_pins"]);
assert.equal(gate.schema_version, 1);
assert.equal(gate.package, "WP-7-E19");
assert.equal(gate.scenario, "fault_api_restart_logout_cleanup");
assert.match(gate.operation, /^[0-9a-f]{32}$/);
assert.equal(gate.evidence_dir, `/tmp/lagrange-kis-integrated-${gate.operation}`);
assert.equal(gate.max_work_seconds, 240);
assert.match(gate.fixture_binary, new RegExp(`^${REPOSITORY}/target/debug/deps/owner_market_stream_integrated-[0-9a-f]+$`));
for (const digest of [gate.fixture_binary_sha256, gate.controller_sha256, gate.nginx_sha256, gate.chromium_sha256]) assert.match(digest, /^[0-9a-f]{64}$/);
assert.equal(gate.chromium_executable, "/home/l1nnx/.cache/ms-playwright/chromium-1228/chrome-linux64/chrome");
assert.equal(sha(await regular(gate.chromium_executable, 512 * 1024 * 1024)), gate.chromium_sha256, "Chromium binary drift");
assert.equal(sha(await regular(fileURLToPath(import.meta.url), 128 * 1024)), gate.controller_sha256, "controller SHA mismatch");
assert.equal(sha(await regular(gate.fixture_binary, 256 * 1024 * 1024)), gate.fixture_binary_sha256, "fixture executable SHA mismatch");
assert.equal(sha(await regular(BINDING, 4096, { privateMode: true })), BINDING_SHA, "binding SHA mismatch");

async function checkPins() {
  assert(Array.isArray(gate.source_pins) && gate.source_pins.length >= 20, "complete source pins required");
  const seen = new Set();
  for (const pin of gate.source_pins) {
    exactKeys(pin, ["path", "sha256"]);
    assert(pin.path.startsWith(`${REPOSITORY}/`) || pin.path.startsWith(`${PRIVATE}/`), "pin outside source scope");
    assert.equal(resolve(pin.path), pin.path);
    assert(!seen.has(pin.path), "duplicate source pin"); seen.add(pin.path);
    assert.equal(sha(await regular(pin.path, 32 * 1024 * 1024)), pin.sha256, "source pin drift");
  }
}
await checkPins();
const nginxSource = await regular(join(REPOSITORY, "deploy/nginx/nginx.conf"), 64 * 1024);
assert.equal(sha(nginxSource), gate.nginx_sha256, "nginx source drift");
assert(Array.isArray(gate.assets) && gate.assets.length > 0 && gate.assets.length <= 30, "bounded built assets required");
const assets = new Map();
for (const item of gate.assets) {
  exactKeys(item, ["url", "path", "sha256"]);
  assert(/^\/(?:index\.html|assets\/[a-zA-Z0-9._-]+)$/.test(item.url), "invalid asset URL");
  assert(item.path.startsWith(`${PRIVATE}/wp7-web-csrf-build/client/`));
  assert.equal(resolve(item.path), item.path);
  const data = await regular(item.path, 4 * 1024 * 1024);
  assert.equal(sha(data), item.sha256, "asset drift");
  assert(!assets.has(item.url), "duplicate asset"); assets.set(item.url, data);
}
assert(assets.has("/index.html"));

// mkdir is the one-attempt marker. An existing directory is never reused.
const evidence = gate.evidence_dir;
await mkdir(evidence, { mode: 0o700 });
await writeFile(join(evidence, "controller-attempt.json"), JSON.stringify({ gate_sha256: process.argv[3], started_at: new Date().toISOString(), pid: process.pid }) + "\n", { flag: "wx", mode: 0o600 });
await mkdir(join(evidence, "home"), { mode: 0o700 });
// Chromium adds its own Unix socket suffix; keep this owned prefix short.
const shortTemp = `/tmp/k7-${gate.operation}`;
await mkdir(shortTemp, { mode: 0o700 });
const shortTempIdentity = await lstat(shortTemp);
assert(shortTempIdentity.isDirectory() && shortTempIdentity.uid === UID && (shortTempIdentity.mode & 0o777) === 0o700);
const env = { ...baseEnv, HOME: join(evidence, "home"), TMPDIR: shortTemp };
const report = { schema_version: 1, package: gate.package, scenario: gate.scenario, started_at: new Date().toISOString(), gate_sha256: process.argv[3], status: "INCOMPLETE", checks: [], commands: [], errors: [], cleanup_errors: [], browser: {}, lifecycle: {} };
const children = new Set();
const streams = new Set();
const requestCounts = new Map();
const responseCounts = new Map();
const pageErrors = [];
const foreignRequests = [];
let fixture;
let browser;
let context;
let mainPage;
const passiveSse = new PassiveStreamDiagnostics();
let passiveRequestLimit = false;
let passiveCallbackErrors = 0;
let browserProcess;
let edge;
let staticServer;
let identifier;
let ready;
let origin;
let interrupt = null;
let unexpectedSocketErrors = 0;
const name = `lagrange-wp7e-${gate.operation.slice(0, 16)}`;
const lifecycle = report.lifecycle;
lifecycle.temporary_directory = { path: shortTemp, device: shortTempIdentity.dev, inode: shortTempIdentity.ino };

async function captureOwnedBrowser() {
  // Inspect only this controller's direct children, never unrelated commands.
  const children = (await readFile(`/proc/${process.pid}/task/${process.pid}/children`, "utf8")).trim().split(/\s+/).filter(Boolean);
  assert(children.length <= 32, "unexpected direct child count");
  const matches = [];
  for (const value of children) {
    assert.match(value, /^[1-9][0-9]*$/);
    try {
      if (await readlink(`/proc/${value}/exe`) !== gate.chromium_executable) continue;
      const stat = await readFile(`/proc/${value}/stat`, "utf8");
      const fields = stat.slice(stat.lastIndexOf(") ") + 2).split(" ");
      assert.equal(Number(fields[1]), process.pid, "browser parent differs");
      assert.equal(Number(fields[2]), Number(value), "browser process group differs");
      matches.push({ pid: Number(value), pgid: Number(fields[2]), start_ticks: fields[19] });
    } catch (error) { if (error.code !== "ENOENT") throw error; }
  }
  assert.equal(matches.length, 1, "owned Chromium identity is ambiguous");
  return matches[0];
}

function groupAbsent(pgid) {
  try { process.kill(-pgid, 0); return false; }
  catch (error) { if (error.code === "ESRCH") return true; throw error; }
}

function child(binary, args, label, childEnv = env) {
  const startedAt = new Date().toISOString();
  const stdout = [];
  const stderr = [];
  let size = 0;
  const process = spawn(binary, args, { cwd: REPOSITORY, env: childEnv, stdio: ["ignore", "pipe", "pipe"], detached: false });
  const record = { label, binary, argv: args, started_at: startedAt, pid: process.pid ?? null, exit: null, signal: null, forced: false };
  children.add(process);
  const done = new Promise((yes) => {
    process.once("error", (error) => { record.spawn_error = error.code ?? error.name; });
    process.once("close", (code, signal) => {
      record.exit = code; record.signal = signal; record.ended_at = new Date().toISOString(); children.delete(process);
      yes({ record, stdout: Buffer.concat(stdout), stderr: Buffer.concat(stderr) });
    });
  });
  for (const [pipe, chunks] of [[process.stdout, stdout], [process.stderr, stderr]]) pipe.on("data", (chunk) => {
    size += chunk.length;
    if (size <= 8 * 1024 * 1024) chunks.push(chunk);
    else { record.output_overflow = true; record.forced = true; process.kill("SIGTERM"); }
  });
  return { process, done, record };
}
async function awaitChild(owned, milliseconds, label) {
  let timer;
  let expired = false;
  let killTimer;
  try {
    timer = setTimeout(() => {
      expired = true; owned.record.forced = true; owned.process.kill("SIGTERM");
      killTimer = setTimeout(() => owned.process.kill("SIGKILL"), 3000);
    }, milliseconds);
    const result = await owned.done; // Always reap the exact child, including after abort.
    report.commands.push(result.record);
    for (const kind of ["stdout", "stderr"]) {
      const path = join(evidence, `${label}.${kind}.log`);
      await writeFile(path, result[kind], { flag: "wx", mode: 0o600 });
      result.record[`${kind}_sha256`] = sha(result[kind]);
    }
    assert(!expired && !result.record.forced && !result.record.spawn_error && result.record.exit === 0 && result.record.signal === null, `${label} did not exit cleanly`);
    return result;
  } finally { clearTimeout(timer); clearTimeout(killTimer); }
}
async function docker(args, label, ms = 10000) {
  const owned = child("/usr/bin/docker", ["-H", "unix:///var/run/docker.sock", ...args], label);
  return (await awaitChild(owned, ms, label)).stdout.toString("utf8");
}
async function until(predicate, ms, label) {
  const deadline = Date.now() + ms;
  while (Date.now() < deadline) {
    if (interrupt) throw interrupt;
    if (fixture && fixture.record.ended_at) throw new Error("fixture exited before requested stop");
    if (await predicate()) return;
    await delay(100);
  }
  throw new Error(`${label} timed out`);
}
async function check(label, action) {
  const started = new Date().toISOString();
  await action();
  assert.equal(pageErrors.length, 0, "browser page exception");
  assert.equal(foreignRequests.length, 0, "foreign browser request");
  if (interrupt) throw interrupt;
  report.checks.push({ label, started_at: started, ended_at: new Date().toISOString(), status: "PASS" });
}
function replacement(text, from, to, count = 1) {
  assert.equal(text.split(from).length - 1, count, "nginx substitution count drift");
  return text.split(from).join(to);
}
async function latestObservation() {
  const path = join(evidence, "observations.jsonl");
  try {
    const bytes = await regular(path, 4 * 1024 * 1024, { privateMode: true, appendSnapshot: true });
    const complete = bytes.toString("utf8").split("\n"); complete.pop();
    return complete.length ? JSON.parse(complete.at(-1)) : null;
  } catch (error) { if (error.code === "ENOENT") return null; throw error; }
}
async function publishStop() {
  // This process is the sole writer in a fresh private directory. Publish the
  // completed file by rename, so the Rust poll cannot observe a zero-byte open.
  const temporary = join(evidence, "stop.pending");
  const target = join(evidence, "stop");
  const fd = await open(temporary, "wx", 0o600);
  try { await fd.writeFile("STOP\n"); await fd.sync(); } finally { await fd.close(); }
  try { await lstat(target); throw new Error("stop evidence already exists"); }
  catch (error) { if (error.code !== "ENOENT") throw error; }
  await rename(temporary, target);
  const directory = await open(evidence, constants.O_RDONLY | constants.O_DIRECTORY);
  try { await directory.sync(); } finally { await directory.close(); }
}
const safeRows = (page) => page.locator("[data-stream-instrument]").evaluateAll((rows) => rows.map((row) => ({ instrument: row.getAttribute("data-stream-instrument"), availability: row.getAttribute("data-availability"), epoch: row.getAttribute("data-epoch"), version: row.getAttribute("data-quote-version"), ordinal: row.getAttribute("data-receive-ordinal"), received_at: row.getAttribute("data-received-at"), price: row.querySelector("[data-quote-value]")?.getAttribute("data-quote-value") ?? null })));

// This observes DOM commits only. It does not replace the browser visibility,
// clock, fetch, EventSource or any production controller method.
function installDomObserver() {
  const counts = new Map();
  const histogram = new Uint32Array(10002);
  let samples = 0;
  let negativeClock = 0;
  let regressingVersion = 0;
  let versionGaps = 0;
  let callbacks = 0;
  const observe = () => {
    callbacks++;
    for (const row of document.querySelectorAll("[data-stream-instrument]")) {
      const instrument = row.getAttribute("data-stream-instrument");
      const epoch = row.getAttribute("data-epoch");
      const version = row.getAttribute("data-quote-version");
      const received = row.getAttribute("data-received-at");
      if (!instrument || !epoch || !version || !received || !/^\d+$/.test(version)) continue;
      const prior = counts.get(instrument);
      if (prior?.epoch === epoch && prior?.version === version) continue;
      if (prior?.epoch === epoch) {
        const distance = BigInt(version) - BigInt(prior.version);
        if (distance <= 0n) regressingVersion++;
        if (distance > 1n) versionGaps += Number(distance - 1n);
      }
      const age = Date.now() - Date.parse(received);
      if (!Number.isFinite(age) || age < -5) negativeClock++;
      else { histogram[Math.min(10001, Math.max(0, Math.round(age)))]++; samples++; }
      counts.set(instrument, { epoch, version, observations: (prior?.observations ?? 0) + 1 });
    }
  };
  new MutationObserver(observe).observe(document, { subtree: true, childList: true, attributes: true, attributeFilter: ["data-quote-version", "data-epoch", "data-received-at"] });
  Object.defineProperty(window, "__ownedMarketStreamDomAudit", { value: () => ({ samples, negative_clock: negativeClock, regressing_version: regressingVersion, dom_version_gaps: versionGaps, callbacks, histogram: [...histogram], instruments: [...counts].map(([instrument, value]) => ({ instrument, ...value })) }), configurable: false });
}
const stopSignal = () => { interrupt ??= new Error("owned controller interrupted"); };
process.on("SIGINT", stopSignal);
process.on("SIGTERM", stopSignal);
const watchdog = setTimeout(stopSignal, 300000);
try {
  edge = tcpServer((downstream) => {
    const upstream = connect(join(evidence, "edge.sock"));
    for (const socket of [downstream, upstream]) {
      streams.add(socket); socket.once("close", () => streams.delete(socket));
      socket.on("error", (error) => { if (!["ECONNRESET", "EPIPE"].includes(error.code)) unexpectedSocketErrors++; downstream.destroy(); upstream.destroy(); });
    }
    downstream.once("close", () => upstream.destroy()); upstream.once("close", () => downstream.destroy());
    downstream.pipe(upstream); upstream.pipe(downstream);
  });
  await new Promise((yes, no) => { edge.once("error", no); edge.listen(0, "127.0.0.1", yes); });
  origin = `https://127.0.0.1:${edge.address().port}`;
  const config = JSON.stringify({ schema_version: 1, origin, max_work_seconds: gate.max_work_seconds, fault_scenario: "api_restart" });
  await writeFile(join(evidence, "config.json"), config, { flag: "wx", mode: 0o600 });
  fixture = child(gate.fixture_binary, [CASE, "--exact", "--ignored", "--nocapture", "--test-threads=1"], "fixture", {
    ...env,
    LAGRANGE_MARKET_STREAM_E2E_DIR: evidence,
    LAGRANGE_MARKET_STREAM_E2E_CONFIG_SHA256: sha(config),
    LAGRANGE_KIS_C2_BINDING_FILE: BINDING,
    LAGRANGE_KIS_C2_BINDING_SHA256: BINDING_SHA,
    LAGRANGE_WS3A_SUPERVISOR_URL: `postgresql:///postgres?host=${CLUSTER}/socket&port=55471&user=c2_supervisor&sslmode=disable`,
  });
  await until(async () => {
    try { ready = JSON.parse(await regular(join(evidence, "ready.json"), 32 * 1024, { privateMode: true })); return true; }
    catch (error) { if (error.code === "ENOENT") return false; throw error; }
  }, 65000, "fixture readiness");
  assert.equal(ready.schema_version, 1); assert.equal(ready.status, "READY");
  assert.equal(ready.origin, origin); assert.equal(ready.api_socket, join(evidence, "api.sock"));
  assert.equal(ready.identities.length, 30); assert.match(ready.generated_database, /^lagrange_ws3a_[0-9]+_[0-9]+$/);
  assert(typeof ready.cookie_name === "string" && typeof ready.cookie_value === "string" && ready.cookie_value.length <= 4096);
  lifecycle.ready_at = new Date().toISOString(); lifecycle.generated_database = ready.generated_database;

  staticServer = httpServer((req, res) => {
    const url = new URL(req.url, origin);
    const key = url.pathname === "/" ? "/index.html" : url.pathname;
    const bytes = assets.get(key);
    if (req.method !== "GET" || !bytes) { res.writeHead(404); res.end(); return; }
    const type = { ".html": "text/html; charset=utf-8", ".js": "text/javascript; charset=utf-8", ".css": "text/css" }[extname(key)] ?? "application/octet-stream";
    res.writeHead(200, { "content-type": type, "cache-control": "no-store", "content-security-policy": "default-src 'self'; connect-src 'self'; script-src 'self'; style-src 'self'" }); res.end(bytes);
  });
  staticServer.on("connection", (socket) => { streams.add(socket); socket.once("close", () => streams.delete(socket)); });
  await new Promise((yes, no) => { staticServer.once("error", no); staticServer.listen(join(evidence, "web.sock"), yes); });
  await chmod(join(evidence, "web.sock"), 0o600);
  const cert = child("/usr/bin/openssl", ["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", join(evidence, "key.pem"), "-out", join(evidence, "cert.pem"), "-days", "1", "-subj", "/CN=127.0.0.1", "-addext", "subjectAltName=IP:127.0.0.1"], "synthetic-tls");
  await awaitChild(cert, 10000, "synthetic-tls");
  let configText = replacement(nginxSource.toString("utf8"), "worker_processes auto;", "worker_processes 1;");
  configText = replacement(configText, "listen 8443 ssl;", "listen unix:/qa/edge.sock ssl;");
  configText = replacement(configText, "/run/secrets/lagrange_tls_cert", "/qa/cert.pem");
  configText = replacement(configText, "/run/secrets/lagrange_tls_key", "/qa/key.pem");
  configText = replacement(configText, "http://api-server:8080", "http://unix:/qa/api.sock:", 5);
  configText = replacement(configText, "http://web:3000", "http://unix:/qa/web.sock:");
  configText = replacement(configText, "http {", "http {\n    access_log off;\n    client_body_temp_path /tmp/client_temp;\n    proxy_temp_path /tmp/proxy_temp;\n    fastcgi_temp_path /tmp/fastcgi_temp;\n    uwsgi_temp_path /tmp/uwsgi_temp;\n    scgi_temp_path /tmp/scgi_temp;");
  await writeFile(join(evidence, "nginx.conf"), configText, { flag: "wx", mode: 0o600 });
  lifecycle.nginx_config_sha256 = sha(configText);
  identifier = (await docker(["run", "-d", "--pull=never", "--name", name, "--label", `io.lagrange.wp7e.operation=${gate.operation}`, "--network=none", "--read-only", "--user", "1000:1000", "--cap-drop=ALL", "--security-opt", "no-new-privileges", "--memory", "128m", "--cpus", "1", "--pids-limit", "32", "--tmpfs", "/tmp:rw,nosuid,nodev,size=16777216,mode=1777", "--mount", `type=bind,src=${evidence},dst=/qa`, "--entrypoint", "/usr/sbin/nginx", NGINX_IMAGE, "-c", "/qa/nginx.conf", "-g", "daemon off;"], "nginx-start")).trim();
  assert.match(identifier, /^[0-9a-f]{64}$/);
  await until(async () => { try { return (await lstat(join(evidence, "edge.sock"))).isSocket(); } catch (error) { if (error.code === "ENOENT") return false; throw error; } }, 5000, "nginx readiness");
  await check("ready without browser demand", async () => {
    await until(async () => { const value = await latestObservation(); if (!value) return false; lifecycle.before_browser = value; return value.active_leases === 0 && value.desired_count === 0 && value.desired_reference_sum === 0 && value.transport.websocket_accepts === 0 && value.transport.approval_accepts === 0; }, 5000, "zero demand");
  });
  lifecycle.browser_launch_started = true;
  try {
    browser = await chromium.launch({ executablePath: gate.chromium_executable, headless: true, timeout: 15000, env });
  } catch (error) {
    // Launch precedes cookie injection; this bounded log contains no session data.
    const detail = String(error?.message ?? error);
    await writeFile(join(evidence, "browser-launch-error.log"), detail.slice(-32768), { flag: "wx", mode: 0o600 });
    lifecycle.browser_launch_failed = true;
    const launched = [...new Set([...detail.matchAll(/<launched> pid=([1-9][0-9]*)/g)].map((match) => match[1]))];
    if (launched.length === 1) {
      const pid = Number(launched[0]);
      assert(Number.isSafeInteger(pid) && pid > 1);
      // Playwright launches its Linux browser in a new process group.
      // Record and check absence only; never signal a PID parsed from a log.
      browserProcess = { pid, pgid: pid, launch_failed: true };
      lifecycle.browser_process = browserProcess;
    }
    throw error;
  }
  browserProcess = await captureOwnedBrowser(); lifecycle.browser_process = browserProcess;
  context = await browser.newContext({ ignoreHTTPSErrors: true, serviceWorkers: "block", viewport: { width: 1280, height: 900 } });
  await context.addCookies([{ name: ready.cookie_name, value: ready.cookie_value, url: origin, secure: true, httpOnly: true, sameSite: "Strict" }]);
  await context.addInitScript(installDomObserver);
  await context.route("**/*", (route) => {
    const request = route.request(); const url = new URL(request.url());
    if (url.origin !== origin) { foreignRequests.push({ method: request.method(), protocol: url.protocol }); return route.abort(); }
    const key = `${request.method()} ${url.pathname}`;
    requestCounts.set(key, (requestCounts.get(key) ?? 0) + 1); return route.continue();
  });
  context.on("response", (response) => {
    const path = new URL(response.url()).pathname;
    const key = `${response.request().method()} ${path} ${response.status()}`;
    responseCounts.set(key, (responseCounts.get(key) ?? 0) + 1);
  });
  context.on("page", (page) => page.on("pageerror", (error) => pageErrors.push({ name: error.name })));
  const pages = [await context.newPage()];
  const page = pages[0]; mainPage = page;
  // Observe only the owned first page through Chromium's passive network events.
  // No interception, replay, response-body storage, or application callback patch.
  const cdp = await context.newCDPSession(page);
  const ownedSseRequests = new Set();
  cdp.on("Network.requestWillBeSent", (event) => {
    try {
      if (event.type !== "EventSource") return;
      const url = new URL(event.request.url);
      if (url.origin !== origin || url.pathname !== "/api/v1/research/owner-beta/equity-universe-v2/market-stream") return;
      if (ownedSseRequests.size >= 8) { passiveRequestLimit = true; return; }
      ownedSseRequests.add(event.requestId);
    } catch { passiveCallbackErrors = Math.min(30000, passiveCallbackErrors + 1); }
  });
  cdp.on("Network.loadingFinished", (event) => ownedSseRequests.delete(event.requestId));
  cdp.on("Network.loadingFailed", (event) => ownedSseRequests.delete(event.requestId));
  cdp.on("Network.eventSourceMessageReceived", (event) => {
    if (ownedSseRequests.has(event.requestId)) passiveSse.observe(event.eventName, event.data,
      Math.floor(performance.now()), Math.floor(event.timestamp * 1000));
  });
  await cdp.send("Network.enable", { maxTotalBufferSize: 131072, maxResourceBufferSize: 65536 });
  await check("one browser shows thirty actual authenticated quotes", async () => {
    await page.goto(origin, { timeout: 20000, waitUntil: "domcontentloaded" });
    await expect(page.locator("[data-stream-instrument]")).toHaveCount(30, { timeout: 20000 });
    // Thirty commands need at least 29 seconds at the production 1-second spacing.
    await expect(page.locator("[data-stream-instrument][data-availability=LIVE] [data-quote-value]")).toHaveCount(30, { timeout: 60000 });
    const first = await safeRows(page); report.browser.first_rows = first;
    assert.equal(new Set(first.map((row) => row.epoch)).size, 1, "initial epoch differs across rows");
    await until(async () => (await safeRows(page)).every((row, index) => row.epoch === first[index].epoch && BigInt(row.version ?? 0) > BigInt(first[index].version ?? 0)), 10000, "second fresh receipt for all thirty");
    await until(async () => { const value = await latestObservation(); if (!value) return false; lifecycle.one_tab = value; return value.active_leases === 1 && value.desired_count === 30 && value.desired_reference_sum === 30 && value.transport.websocket_accepts === 1 && value.transport.active_websockets === 1 && value.transport.subscribe_commands === 30; }, 10000, "one-tab shared upstream");
    assert(passiveSse.snapshot().by_kind.snapshot > 0, "passive SSE diagnostics received no snapshot");
    await page.screenshot({ path: join(evidence, "integrated-board.png"), fullPage: true });
  });
  const fault = async (sequence, kind) => {
    const request = JSON.stringify({ schema_version: 1, sequence, kind });
    const temporary = join(evidence, `fault-${String(sequence).padStart(2, "0")}.pending`);
    const path = join(evidence, `fault-${String(sequence).padStart(2, "0")}.json`);
    await writeFile(temporary, request, { flag: "wx", mode: 0o600 });
    await rename(temporary, path);
    let result;
    await until(async () => {
      try {
        result = JSON.parse(await regular(join(evidence, `fault-${String(sequence).padStart(2, "0")}-result.json`), 2048, { privateMode: true }));
        return true;
      } catch (error) { if (error.code === "ENOENT") return false; throw error; }
    }, 5000, `owned fault ${sequence}`);
    assert.equal(result.schema_version, 1); assert.equal(result.sequence, sequence);
    assert.equal(result.kind, kind); assert.equal(result.status, "PASS");
    return result;
  };
  await check("real browser offline withdraws quotes and fresh API server reconnects", async () => {
    const before = await safeRows(page);
    const oldIds = [...ownedSseRequests];
    assert.equal(oldIds.length, 1);
    const beforeReset = passiveSse.snapshot().by_kind.reset;
    const beforeSnapshot = passiveSse.snapshot().by_kind.snapshot;
    await context.setOffline(true);
    await until(async () => !(await page.evaluate(() => navigator.onLine)), 3000, "native browser offline");
    await expect(page.locator("[data-quote-value]")).toHaveCount(0, { timeout: 3000 });
    await until(async () => ownedSseRequests.size === 0, 3000, "offline closes original SSE");
    const result = await fault(1, "api_restart");
    assert.deepEqual(result.detail, { old_server_joined: true, old_listener_gone: true,
      fresh_api_state: true, same_owned_endpoint: true, new_server_started: true });
    await context.setOffline(false);
    await until(async () => await page.evaluate(() => navigator.onLine), 3000, "native browser online");
    await until(async () => {
      const rows = await safeRows(page);
      return rows.length === 30 && rows.every((row, index) => row.availability === "LIVE"
        && row.epoch === before[index].epoch && BigInt(row.version) > BigInt(before[index].version)
        && BigInt(row.ordinal) > BigInt(before[index].ordinal));
    }, 15000, "new authenticated stream after API restart");
    assert.equal(ownedSseRequests.size, 1);
    assert(!ownedSseRequests.has(oldIds[0]), "restarted server did not create a new SSE response");
    assert(passiveSse.snapshot().by_kind.reset > beforeReset);
    assert(passiveSse.snapshot().by_kind.snapshot > beforeSnapshot);
    const resumed = await safeRows(page);
    await until(async () => (await safeRows(page)).every((row, index) => row.availability === "LIVE"
      && row.epoch === resumed[index].epoch && BigInt(row.version) > BigInt(resumed[index].version)
      && BigInt(row.ordinal) > BigInt(resumed[index].ordinal)), 5000, "continued fresh quotes after reconnect");
    report.browser.api_restart = { ...result.detail, offline_quotes_purged: true,
      new_sse_response: true, reset_and_snapshot_observed: true, thirty_authentic_rows_progressed: true };
  });
  await check("authenticated logout purges quotes and closes demand", async () => {
    await page.getByRole("button", { name: "Logout", exact: true }).click();
    await expect(page.locator("[data-quote-value]")).toHaveCount(0, { timeout: 3000 });
    await until(async () => (responseCounts.get("POST /api/v1/auth/logout 204") ?? 0) === 1,
      10000, "normal authenticated logout response");
    await until(async () => ownedSseRequests.size === 0, 3000, "logout closes SSE");
    await until(async () => {
      const value = await latestObservation();
      return value && value.active_leases === 0 && value.desired_count === 0
        && value.desired_reference_sum === 0 && value.transport.active_websockets === 0;
    }, 35000, "logout releases demand including prior offline TTL");
    assert.equal(await page.locator("[data-testid=integrated-fixture-error]").count(), 0);
    report.browser.logout = { authenticated_status: 204, quotes_purged: true, stream_closed: true,
      active_leases: 0, desired_count: 0, active_upstream: 0 };
  });
  await check("last mounted page releases demand and upstream closes", async () => {
    report.browser.dom_audit = await page.evaluate(() => window.__ownedMarketStreamDomAudit());
    await page.getByRole("button", { name: "Toggle page", exact: true }).click();
    await until(async () => { const value = await latestObservation(); if (!value) return false; lifecycle.after_last_unmount = value; return value.active_leases === 0 && value.desired_count === 0 && value.desired_reference_sum === 0 && value.transport.active_websockets === 0; }, 35000, "last demand cleanup");
    assert.equal([...requestCounts].filter(([key]) => key.includes("/quote")).length, 0, "REST quote request in WS mode");
    const audit = report.browser.dom_audit;
    assert.equal(audit.negative_clock, 0); assert.equal(audit.regressing_version, 0);
    assert.equal(audit.instruments.length, 30); assert(audit.instruments.every((row) => row.observations >= 2));
    let cumulative = 0;
    const target = Math.ceil(audit.samples * 0.95);
    audit.p95_received_to_dom_ms = audit.histogram.findIndex((count) => { cumulative += count; return cumulative >= target; });
    assert(audit.samples > 0 && audit.p95_received_to_dom_ms >= 0 && audit.p95_received_to_dom_ms <= 2000, "positive-case received-to-DOM p95 exceeds two seconds");
  });
} catch (error) {
  report.errors.push(summaryError(error));
  report.browser.passive_sse_at_failure = passiveSse.snapshot();
  // Capture finite page lifecycle/DOM-counter evidence only after the assertion.
  if (mainPage && !mainPage.isClosed()) {
    try {
      let timer;
      try {
        report.browser.failure_page = await Promise.race([
          mainPage.evaluate(() => ({
            visibility: document.visibilityState === "visible" ? "visible" : "hidden",
            hidden: document.hidden, focused: document.hasFocus(),
            dom_audit: window.__ownedMarketStreamDomAudit(),
          })),
          new Promise((yes) => { timer = setTimeout(() => yes({ capture_timed_out: true }), 2000); }),
        ]);
      } finally { clearTimeout(timer); }
    } catch { report.browser.failure_page_capture_failed = true; }
  }
}
finally {
  clearTimeout(watchdog);
  const clean = async (label, action) => { try { await action(); } catch (error) { report.cleanup_errors.push({ label, ...summaryError(error) }); } };
  await clean("browser-close", async () => {
    if (browser) await browser.close();
    lifecycle.browser_closed = true;
    if (browserProcess) {
      const deadline = Date.now() + 5000;
      while (!groupAbsent(browserProcess.pgid) && Date.now() < deadline) await delay(25);
      lifecycle.browser_group_absent = groupAbsent(browserProcess.pgid);
      assert(lifecycle.browser_group_absent, "owned Chromium group remains");
    }
  });
  await clean("fixture-stop-and-join", async () => {
    if (!fixture) return;
    let stopFailure;
    try { if (!fixture.record.ended_at) await publishStop(); }
    catch (error) { stopFailure = error; lifecycle.stop_failure = summaryError(error); }
    const value = await awaitChild(fixture, 55000, "fixture");
    if (stopFailure) throw stopFailure;
    const raw = value.stdout.toString("utf8");
    assert.match(raw, /running 1 test/);
    assert.match(raw, /test result: ok\. 1 passed; 0 failed; 0 ignored;/);
    const result = JSON.parse(await regular(join(evidence, "result.json"), 64 * 1024, { privateMode: true }));
    lifecycle.fixture_result = result;
    assert.equal(result.status, "PASS", "fixture lifecycle result incomplete");
    assert.equal(result.primary_failure, null); assert.equal(result.cleanup_failure, null);
    assert.equal(result.forced_abort, false); assert.equal(result.active_fixture_tasks, 0);
    assert.equal(result.transport.event_overflow, false);
    assert.equal(result.transport.unexpected_commands_or_requests, 0);
    assert.equal(result.transport.server_errors, 0);
    assert.equal(result.transport.peak_websockets, 1);
    assert.equal(result.transport.client_close_seen, true);
    assert.equal(result.publication_evidence_written, true, "publication evidence missing");
    const publicationBytes = await regular(join(evidence, "publication.json"), 4 * 1024 * 1024, { privateMode: true });
    const publication = JSON.parse(publicationBytes);
    assert.deepEqual(result.publication_summary, publication.summary, "final publication summary drift");
    const publicationAudit = auditPublication(publication);
    lifecycle.publication_evidence = { sha256: sha(publicationBytes), bytes: publicationBytes.length, summary: publication.summary, audit: publicationAudit };
    assert(publicationAudit.records > 0 && publicationAudit.known_committed_changed_rows > 0, "no measured committed publication");
    assert.equal(publicationAudit.commit_outcomes_all_known, true, "indeterminate positive-case commit outcome");
    assert.equal(publicationAudit.observed_rate_bounds_pass, true, "observed rolling publication rate exceeds contract");
    const observationBytes = await regular(join(evidence, "observations.jsonl"), 4 * 1024 * 1024, { privateMode: true });
    const observationLines = observationBytes.toString("utf8").split("\n");
    assert.equal(observationLines.pop(), "", "final observation file has a partial record");
    const resourceAudit = auditResources({
      buffer_summary: result.buffer_summary, heap_summary: result.heap_summary,
      process_rss_bytes: result.process_rss_bytes, resource_failure: result.resource_failure,
      observations: observationLines.map((line) => JSON.parse(line)),
    });
    lifecycle.resource_evidence = { sha256: sha(observationBytes), bytes: observationBytes.length, audit: resourceAudit };
    assert(resourceAudit.adapter_calls > 0 && resourceAudit.coalescing_observed, "actual receipt coalescing was not observed");
    assert.equal(resourceAudit.tracked_global_superset_within_8_mib, true, "tracked global allocation superset exceeds 8 MiB; stream-owned bound remains unproven");
  });
  await clean("nginx-close", async () => {
    if (!identifier) {
      const found = (await docker(["ps", "-aq", "--no-trunc", "--filter", `name=^/${name}$`], "nginx-recover-id")).trim().split("\n").filter(Boolean);
      assert(found.length <= 1); identifier = found[0];
    }
    if (identifier) {
      const found = JSON.parse(await docker(["container", "inspect", identifier], "nginx-cleanup-identity"));
      assert.equal(found.length, 1);
      const value = found[0]; assert.equal(value.Id, identifier); assert.equal(value.Image, NGINX_IMAGE); assert.equal(value.Name, `/${name}`); assert.equal(value.Config.Labels["io.lagrange.wp7e.operation"], gate.operation);
      if (value.State.Running) await docker(["container", "kill", "--signal", "SIGTERM", identifier], "nginx-term");
      const exit = (await docker(["container", "wait", identifier], "nginx-wait")).trim(); lifecycle.nginx_exit = Number(exit); assert.equal(exit, "0");
      await docker(["container", "rm", identifier], "nginx-remove");
    }
    lifecycle.nginx_absent = (await docker(["ps", "-aq", "--no-trunc", "--filter", `name=^/${name}$`], "nginx-absence")).trim() === "";
    assert(lifecycle.nginx_absent);
  });
  await clean("owned-network-close", async () => {
    for (const socket of streams) socket.destroy();
    for (const server of [edge, staticServer]) if (server?.listening) await new Promise((yes, no) => server.close((error) => error ? no(error) : yes()));
    await delay(10); assert.equal(streams.size, 0); assert.equal(children.size, 0);
    for (const path of ["edge.sock", "web.sock"]) {
      try { const own = await lstat(join(evidence, path)); assert(own.isSocket() && own.uid === UID); await unlink(join(evidence, path)); }
      catch (error) { if (error.code !== "ENOENT") throw error; }
    }
    lifecycle.owned_sockets_absent = true;
  });
  await clean("owned-temp-close", async () => {
    assert.equal(children.size, 0, "owned children remain before temporary cleanup");
    assert(lifecycle.browser_closed);
    assert(!lifecycle.browser_launch_started || lifecycle.browser_group_absent === true, "browser group absence is unproven");
    const identity = await lstat(shortTemp);
    assert(identity.isDirectory() && !identity.isSymbolicLink() && identity.uid === UID && (identity.mode & 0o777) === 0o700);
    assert.equal(identity.dev, shortTempIdentity.dev); assert.equal(identity.ino, shortTempIdentity.ino);
    await rm(shortTemp, { recursive: true, force: false });
    try { await lstat(shortTemp); assert.fail("owned temporary directory remains"); }
    catch (error) { if (error.code !== "ENOENT") throw error; }
    lifecycle.temporary_directory_absent = true;
  });
  await clean("source-final-pins", checkPins);
  report.browser.passive_sse = passiveSse.snapshot();
  report.browser.passive_sse_request_limit = passiveRequestLimit;
  report.browser.passive_sse_callback_errors = passiveCallbackErrors;
  report.browser.requests = Object.fromEntries(requestCounts);
  report.browser.responses = Object.fromEntries(responseCounts);
  report.browser.page_errors = pageErrors;
  report.browser.foreign_requests = foreignRequests;
  report.browser.unexpected_socket_errors = unexpectedSocketErrors;
  report.ended_at = new Date().toISOString();
  report.status = report.errors.length === 0 && report.cleanup_errors.length === 0 && report.checks.length === 5 && report.checks.every((check) => check.status === "PASS") && unexpectedSocketErrors === 0 && !interrupt ? "PASS" : "INCOMPLETE";
  report.not_verified = ["PG identity/final catalog/normal PG shutdown (outer runner owns these)", "independent acceptance", "extended soak/heap/cadence coverage and untracked/native heap", "fault matrix", "installed or provider/live behavior"];
  await writeFile(join(evidence, "controller-result.json"), JSON.stringify(report, null, 2) + "\n", { flag: "wx", mode: 0o600 });
  console.log(JSON.stringify({ status: report.status, passed: report.checks.length, errors: report.errors, cleanup_errors: report.cleanup_errors, evidence }));
  process.exitCode = report.status === "PASS" ? 0 : 1;
}
