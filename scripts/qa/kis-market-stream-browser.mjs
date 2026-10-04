#!/usr/bin/env node
// Actual React + Chromium, with an owned synthetic same-origin API. Never a live acceptance tool.
import assert from "node:assert/strict";
import { createSecureServer } from "node:http2";
import { execFileSync } from "node:child_process";
import { readFile, mkdir, writeFile, unlink } from "node:fs/promises";
import { dirname, resolve, join, extname, relative, isAbsolute } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { build } from "vite";
import { chromium, expect } from "@playwright/test";

const repository = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
assert.equal(Number(process.versions.node.split(".")[0]), 24, "Use the repository's Node 24 runtime");
const requested = process.argv[2];
assert(requested?.startsWith("/tmp/"), "Pass a NEW absolute /tmp evidence directory");
const evidence = resolve(requested);
assert(evidence.startsWith("/tmp/") && evidence !== "/tmp", "Evidence must stay under /tmp");
await mkdir(evidence, { recursive: false, mode: 0o700 });
const web = join(repository, "apps/web");
const fixtureRoot = join(web, "tests/browser/market-stream");
const emptyEnv = join(evidence, "empty-env");
await mkdir(emptyEnv);
const buildOptions = {
  configFile: false, envDir: emptyEnv, root: fixtureRoot, logLevel: "warn",
  cacheDir: join(evidence, "vite-cache"),
  resolve: { alias: { "@": web } },
};
await build({ ...buildOptions, build: { outDir: join(evidence, "client"), emptyOutDir: false } });
await build({ ...buildOptions, ssr: { noExternal: true }, build: {
  ssr: join(fixtureRoot, "server-fixture.ts"), outDir: join(evidence, "server"), emptyOutDir: false,
  rolldownOptions: { output: { entryFileNames: "fixture.mjs" } },
} });
const { streamRow, streamFixtureUuid, INTRADAY_STREAM_LEASE_PATH, intradayStreamLeaseRequestSchema, intradayStreamReleaseRequestSchema } = await import(pathToFileURL(join(evidence, "server/fixture.mjs")).href);
// Ephemeral synthetic TLS material only; never read an installed certificate or key.
const tlsKey = join(evidence, "fixture.key");
const tlsCertificate = join(evidence, "fixture.crt");
execFileSync("/usr/bin/openssl", ["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", tlsKey, "-out", tlsCertificate, "-days", "1", "-subj", "/CN=localhost", "-addext", "subjectAltName=IP:127.0.0.1,DNS:localhost"], { stdio: "ignore", timeout: 10000 });

const requests = [];
const leases = new Map();
const streams = new Map();
const sockets = new Set();
const pageErrors = [];
const foreignRequests = [];
let nextLease = 1000;
let nextStream = 2000;
let origin = null;
let maximumStreamsPerLease = 0;
const checks = [];
let browser;
let fatal;
let watchdog;
let pauseDelivery = false;
const incomplete = ["actual auth/API/database/provider/runtime"];
let activeCheck = null;
const expectedChecks = [
  "actual React board/detail, one lease and one native SSE, no REST quote",
  "membership removal clears selected price and replaces the full set",
  "transport disconnect requires a fresh reset and replacement stream",
  "real offline event closes SSE; online creates a new consumer",
  "logout clears quotes and closes SSE before a new session can restart",
  "React unmount closes the owned connection and bounded release",
  "off makes no price requests and REST mode never starts stream demand",
  "ten real pages keep ten distinct consumers; closing nine preserves the tenth",
  "unchanged receipts age after thirty seconds while the same lease renews",
  "five-second delivery silence removes LIVE before bounded reconnect",
  "native hidden window releases demand and restore starts a new consumer",
];
const json = (response, status, value) => {
  response.writeHead(status, { "content-type": "application/json", "cache-control": "no-store" });
  response.end(JSON.stringify(value));
};
async function body(request) {
  const parts = [];
  let size = 0;
  for await (const part of request) {
    size += part.length;
    assert(size <= 16384, "Fixture input exceeded the contract limit");
    parts.push(part);
  }
  return JSON.parse(Buffer.concat(parts).toString("utf8"));
}
function send(stream, kind, value) {
  stream.sequence++;
  stream.response.write(`event: ${kind}\nid: ${stream.id}:${stream.sequence}\ndata: ${JSON.stringify({
    schema_version: 2, stream_id: stream.id, event_sequence: String(stream.sequence),
    server_time: new Date().toISOString(), body: value,
  })}\n\n`);
}
function closeStreams(leaseId) {
  for (const stream of streams.values()) if (stream.leaseId === leaseId) stream.response.end();
}
const server = createSecureServer({ key: await readFile(tlsKey), cert: await readFile(tlsCertificate), allowHTTP1: false }, (request, response) => {
  void handle(request, response).catch((error) => {
    fatal ??= error;
    if (!response.headersSent) json(response, 500, { error: { code: "FIXTURE_FAILED" } });
    else response.end();
  });
});
server.on("connection", (socket) => {
  sockets.add(socket);
  socket.on("close", () => sockets.delete(socket));
});
async function handle(request, response) {
  assert(origin && (request.headers[":authority"] ?? request.headers.host) === new URL(origin).host, "Foreign Host header");
  assert.equal(request.httpVersionMajor, 2, "Ten same-origin event streams require multiplexed HTTP/2 in this fixture");
  const url = new URL(request.url, origin);
  const path = url.pathname;
  requests.push({ method: request.method, path, at: Date.now() });
  if (path === "/api/v1/auth/csrf" && request.method === "GET") return json(response, 200, { csrf_token: "synthetic-csrf-only" });
  if (path === INTRADAY_STREAM_LEASE_PATH && request.method === "POST") {
    assert.equal(request.headers["x-csrf-token"], "synthetic-csrf-only");
    const input = intradayStreamLeaseRequestSchema.parse(await body(request));
    let lease = [...leases.values()].find((item) => item.consumer_id === input.consumer_id);
    if (lease) assert.equal(input.renewal_sequence, lease.renewal_sequence + 1);
    else assert.equal(input.renewal_sequence, 0);
    lease = { ...input, lease_id: lease?.lease_id ?? streamFixtureUuid(++nextLease), lease_expires_at: new Date(Date.now() + 30000).toISOString(), renew_after_ms: 15000 };
    leases.set(lease.lease_id, lease);
    return json(response, 200, lease);
  }
  if (path.startsWith(`${INTRADAY_STREAM_LEASE_PATH}/`) && request.method === "DELETE") {
    const input = intradayStreamReleaseRequestSchema.parse(await body(request));
    const id = path.slice(INTRADAY_STREAM_LEASE_PATH.length + 1);
    const lease = leases.get(id);
    assert(lease);
    assert.equal(input.consumer_id, lease.consumer_id);
    assert.equal(input.renewal_sequence, lease.renewal_sequence);
    leases.delete(id);
    closeStreams(id);
    return json(response, 200, { schema_version: 2, lease_id: id, released: true });
  }
  if (path === "/api/v1/research/owner-beta/equity-universe-v2/market-stream" && request.method === "GET") {
    const lease = leases.get(url.searchParams.get("lease_id"));
    assert(lease, "SSE must reference an accepted lease");
    const prior = [...streams.values()].filter((item) => item.leaseId === lease.lease_id).length;
    maximumStreamsPerLease = Math.max(maximumStreamsPerLease, prior + 1);
    assert.equal(prior, 0, "A page opened overlapping event streams");
    response.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-store" });
    const stream = { id: streamFixtureUuid(++nextStream), sequence: 0, leaseId: lease.lease_id, response };
    streams.set(stream.id, stream);
    const snapshotTime = new Date().toISOString();
    if (!pauseDelivery) {
      send(stream, "reset", { reason_code: "RESYNC_REQUIRED" });
      send(stream, "snapshot", {
      lease_id: lease.lease_id, lease_expires_at: lease.lease_expires_at,
      rows: lease.identities.map((identity) => {
        const index = Number(identity.membership_id.slice(-12)) - 1;
        return streamRow(index, snapshotTime);
      }),
      });
    }
    const timer = setInterval(() => { if (!pauseDelivery) send(stream, "status", { connection: "CONNECTED", reason_code: null, gap_open: false, session_has_gap: false, gap_generation: "0" }); }, 1000);
    response.on("close", () => { clearInterval(timer); streams.delete(stream.id); });
    return;
  }
  if (path.startsWith("/api/")) return json(response, 503, { error: { code: "SYNTHETIC_REST_UNAVAILABLE", message: "Synthetic fixture only", request_id: "fixture" } });
  const file = path === "/" ? "index.html" : decodeURIComponent(path.slice(1));
  const candidate = resolve(evidence, "client", file);
  const within = relative(join(evidence, "client"), candidate);
  assert(!within.startsWith("..") && !isAbsolute(within), "Static path escaped owned build");
  try {
    const bytes = await readFile(candidate);
    const types = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css" };
    response.writeHead(200, { "content-type": types[extname(candidate)] ?? "application/octet-stream", "cache-control": "no-store", "content-security-policy": "default-src 'self'; connect-src 'self'; script-src 'self'; style-src 'self'" });
    response.end(bytes);
  } catch { response.writeHead(404); response.end(); }
}
async function check(name, action) {
  activeCheck = name;
  await action();
  if (fatal) throw fatal;
  checks.push(name);
  activeCheck = null;
  console.log(`PASS ${name}`);
}
const waitFor = async (predicate) => expect.poll(predicate, { timeout: 5000 }).toBe(true);
const interrupt = () => {
  fatal ??= new Error("Owned browser fixture interrupted");
  if (browser) void browser.close().catch(() => undefined);
  for (const stream of streams.values()) stream.response.end();
  for (const socket of sockets) socket.destroy();
};
process.once("SIGTERM", interrupt);
process.once("SIGINT", interrupt);
try {
  watchdog = setTimeout(interrupt, 100000);
  await new Promise((yes, no) => { server.once("error", no); server.listen(0, "127.0.0.1", yes); });
  origin = `https://127.0.0.1:${server.address().port}`;
  browser = await chromium.launch({ headless: true, timeout: 15000 });
  const context = await browser.newContext({ ignoreHTTPSErrors: true, serviceWorkers: "block", viewport: { width: 1280, height: 900 } });
  await context.route("**/*", (route) => {
    if (new URL(route.request().url()).origin === origin) return route.continue();
    foreignRequests.push(new URL(route.request().url()).origin);
    return route.abort();
  });
  context.on("page", (page) => page.on("pageerror", (error) => pageErrors.push({ name: error.name, message: error.message, stack: error.stack })));
  const page = await context.newPage();
  await check("actual React board/detail, one lease and one native SSE, no REST quote", async () => {
    await page.goto(origin);
    await expect(page.locator("[data-stream-instrument]")).toHaveCount(30);
    await expect(page.locator("[data-stream-instrument] [data-quote-value]")).toHaveCount(30);
    await expect(page.getByTestId("stock-beta-stream-selected").locator("strong")).toHaveAttribute("data-quote-value", "100123456789.12345678");
    assert.equal(leases.size, 1); assert.equal(streams.size, 1);
    assert.equal(requests.filter((item) => item.method === "POST").length, 1);
    assert.equal(requests.filter((item) => item.path.includes("/quote")).length, 0);
    const selected = page.locator("[data-stream-instrument]").nth(10);
    const instrument = await selected.getAttribute("data-stream-instrument");
    await selected.getByRole("button").click();
    await expect(page.getByTestId("stock-beta-stream-selected")).toContainText(instrument);
    assert.equal(leases.size, 1); assert.equal(streams.size, 1);
    await page.screenshot({ path: join(evidence, "board.png"), fullPage: true });
  });
  await check("membership removal clears selected price and replaces the full set", async () => {
    await page.getByRole("button", { name: "Remove selected membership" }).click();
    await expect(page.getByTestId("stock-beta-stream-selected").locator("strong")).not.toHaveAttribute("data-quote-value");
    await waitFor(() => [...leases.values()].every((lease) => lease.identities.length === 29));
    await expect(page.locator("[data-stream-instrument] [data-quote-value]")).toHaveCount(29);
  });
  await check("transport disconnect requires a fresh reset and replacement stream", async () => {
    const before = nextStream;
    for (const stream of streams.values()) stream.response.end();
    await waitFor(() => nextStream > before && streams.size === 1);
    await expect(page.locator("[data-stream-instrument] [data-quote-value]")).toHaveCount(29);
    assert.equal(leases.size, 1);
  });
  await check("real offline event closes SSE; online creates a new consumer", async () => {
    const before = new Set([...leases.values()].map((lease) => lease.consumer_id));
    await context.setOffline(true);
    await waitFor(() => streams.size === 0);
    await expect(page.locator("[data-stream-instrument] [data-quote-value]")).toHaveCount(0);
    await context.setOffline(false);
    await waitFor(() => streams.size === 1 && [...leases.values()].some((lease) => !before.has(lease.consumer_id)));
    await expect(page.locator("[data-stream-instrument] [data-quote-value]")).toHaveCount(29);
  });
  await check("logout clears quotes and closes SSE before a new session can restart", async () => {
    await page.getByRole("button", { name: "Logout", exact: true }).click();
    await waitFor(() => streams.size === 0);
    await expect(page.locator("[data-stream-instrument] [data-quote-value]")).toHaveCount(0);
    await page.getByRole("button", { name: "New session", exact: true }).click();
    await waitFor(() => streams.size === 1);
    await expect(page.locator("[data-stream-instrument] [data-quote-value]")).toHaveCount(29);
  });
  await check("React unmount closes the owned connection and bounded release", async () => {
    const before = requests.filter((item) => item.method === "DELETE").length;
    await page.getByRole("button", { name: "Toggle page" }).click();
    await waitFor(() => streams.size === 0 && requests.filter((item) => item.method === "DELETE").length > before);
    await expect(page.getByTestId("stock-beta-stream-board")).toHaveCount(0);
  });
  await check("off makes no price requests and REST mode never starts stream demand", async () => {
    await page.goto(`${origin}/?mode=off`);
    await expect(page.getByTestId("stock-beta-stream-board")).toHaveCount(0);
    const before = requests.filter((item) => item.path.startsWith("/api/")).length;
    await page.waitForTimeout(200);
    assert.equal(requests.filter((item) => item.path.startsWith("/api/")).length, before);
    const leasesBefore = nextLease;
    await page.goto(`${origin}/?mode=rest`);
    await expect(page.getByTestId("stock-beta-stream-board")).toHaveCount(0);
    await expect(page.getByText("Intraday price · periodic refresh")).toBeVisible();
    await waitFor(() => requests.some((item) => item.path.includes("/quote")));
    assert.equal(nextLease, leasesBefore); assert.equal(streams.size, 0);
  });
  await page.goto(`${origin}/?mode=off`);
  await check("ten real pages keep ten distinct consumers; closing nine preserves the tenth", async () => {
    const pages = [];
    const before = nextLease;
    try {
      for (let index = 0; index < 10; index++) {
        const owned = await context.newPage();
        pages.push(owned);
        await owned.goto(origin);
        await expect(owned.locator("[data-stream-instrument] [data-quote-value]")).toHaveCount(30);
        assert.equal(await owned.evaluate(() => document.visibilityState), "visible");
      }
      assert.equal(nextLease - before, 10); assert.equal(streams.size, 10);
      const active = [...streams.values()].map((stream) => leases.get(stream.leaseId));
      assert.equal(new Set(active.map((lease) => lease.consumer_id)).size, 10);
      assert(active.every((lease) => lease.identities.length === 30));
      for (const owned of pages.slice(0, 9)) { await owned.goto("about:blank"); await owned.close(); }
      await waitFor(() => streams.size === 1);
      await expect(pages[9].locator("[data-stream-instrument] [data-quote-value]")).toHaveCount(30);
    } finally { for (const owned of pages) if (!owned.isClosed()) { await owned.goto("about:blank"); await owned.close(); } }
    await waitFor(() => streams.size === 0);
  });
  await check("unchanged receipts age after thirty seconds while the same lease renews", async () => {
    await page.goto(origin);
    await expect(page.locator('[data-stream-instrument][data-availability="LIVE"]')).toHaveCount(30);
    const leaseId = [...streams.values()][0].leaseId;
    const row = page.locator("[data-stream-instrument]").first();
    const received = await row.getAttribute("data-received-at");
    await expect(page.locator('[data-stream-instrument][data-availability="LAST_KNOWN"]')).toHaveCount(30, { timeout: 35000 });
    assert.equal(await row.getAttribute("data-received-at"), received);
    assert.equal(await row.getAttribute("data-quote-version"), "1");
    assert.equal([...streams.values()][0].leaseId, leaseId);
    assert(leases.get(leaseId).renewal_sequence >= 1);
    await expect(row).toContainText("Stale");
  });
  await check("five-second delivery silence removes LIVE before bounded reconnect", async () => {
    // A new page supplies a new receipt; the subsequent outage sends neither status nor prices.
    await page.goto("about:blank");
    await waitFor(() => streams.size === 0);
    await page.goto(origin);
    await expect(page.locator('[data-stream-instrument][data-availability="LIVE"]')).toHaveCount(30);
    pauseDelivery = true;
    await expect(page.locator('[data-stream-instrument][data-availability="LIVE"]')).toHaveCount(0, { timeout: 6500 });
    assert.equal(await page.locator("[data-stream-instrument] [data-quote-value]").count(), 30);
    pauseDelivery = false;
    for (const stream of streams.values()) stream.response.end();
    await expect(page.locator('[data-stream-instrument][data-availability="LIVE"]')).toHaveCount(30, { timeout: 6500 });
  });
  // Use a real Chromium window state, never override document.visibilityState in JavaScript.
  const native = await context.newCDPSession(page);
  let windowId;
  try {
    // Playwright enables focus emulation at page creation; turn it off for the native hide test.
    await native.send("Emulation.setFocusEmulationEnabled", { enabled: false });
    const { targetInfo } = await native.send("Target.getTargetInfo");
    ({ windowId } = await native.send("Browser.getWindowForTarget", { targetId: targetInfo.targetId }));
    await native.send("Browser.setWindowBounds", { windowId, bounds: { windowState: "minimized" } });
    await page.waitForTimeout(200);
    if (await page.evaluate(() => document.visibilityState) === "hidden") {
      await check("native hidden window releases demand and restore starts a new consumer", async () => {
        const before = nextLease;
        await waitFor(() => streams.size === 0);
        await native.send("Browser.setWindowBounds", { windowId, bounds: { windowState: "normal" } });
        await expect(page.locator('[data-stream-instrument][data-availability="LIVE"]')).toHaveCount(30);
        assert(nextLease > before);
      });
    } else incomplete.push("native window minimization does not hide this Chromium build; hidden lifecycle unverified");
  } catch (error) {
    // A failed behavioral assertion is a failure, not an unsupported-environment skip.
    if (error instanceof Error && /not supported|wasn't found|Method not found/i.test(error.message)) incomplete.push("native window minimization unavailable; hidden lifecycle unverified");
    else throw error;
  } finally {
    if (windowId !== undefined) await native.send("Browser.setWindowBounds", { windowId, bounds: { windowState: "normal" } });
    await native.send("Emulation.setFocusEmulationEnabled", { enabled: true });
    await native.detach();
  }
  assert.deepEqual(pageErrors, []); assert.deepEqual(foreignRequests, []);
  assert.equal(maximumStreamsPerLease, 1);
} catch (error) {
  fatal ??= error;
  console.error(error);
} finally {
  clearTimeout(watchdog);
  process.off("SIGTERM", interrupt);
  process.off("SIGINT", interrupt);
  if (browser) await browser.close();
  for (const stream of streams.values()) stream.response.end();
  for (const socket of sockets) socket.destroy();
  await new Promise((done) => server.close(done));
  await unlink(tlsKey);
  await unlink(tlsCertificate);
  const statuses = expectedChecks.map((name) => ({ name, status: checks.includes(name) ? "PASS" : activeCheck === name ? "FAIL" : "NOT_RUN" }));
  const report = { synthetic_only: true, real_react_chromium: browser !== undefined, node_version: process.versions.node, browser_version: browser?.version(), protocol: "HTTP/2", synthetic_tls_files_removed: true, origin, checks, statuses, passed: !fatal && statuses.every((item) => item.status === "PASS"), browser_closed: browser !== undefined && !browser.isConnected(), server_listening: server.listening, remaining_streams: streams.size, maximum_streams_per_lease: maximumStreamsPerLease, request_counts: Object.fromEntries([...new Set(requests.map((item) => `${item.method} ${item.path}`))].map((key) => [key, requests.filter((item) => `${item.method} ${item.path}` === key).length])), page_errors: pageErrors, foreign_requests: foreignRequests, incomplete };
  await writeFile(join(evidence, "result.json"), `${JSON.stringify(report, null, 2)}\n`, { flag: "wx", mode: 0o600 });
}
if (fatal || expectedChecks.some((name) => !checks.includes(name))) process.exitCode = 1;
