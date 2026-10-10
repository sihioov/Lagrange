import { lstatSync, mkdirSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import {
  type APIRequestContext,
  expect,
  type Locator,
  type Page,
  type TestInfo,
  test,
} from "@playwright/test";
// @ts-expect-error Playwright loads this local Node fixture at runtime; it is not product TypeScript.
import { installServiceStatusFixture } from "./support/service-status-fixture.mjs";

const appOrigin = process.env["PLAYWRIGHT_BASE_URL"] ?? "http://127.0.0.1:33042";
const syntheticOrigin = process.env["SYNTHETIC_API_ORIGIN"] ?? "http://127.0.0.1:38192";
const statusPath = "/api/v1/admin/service-status";
const forbiddenApiPath =
  /(?:^|\/)(?:account|accounts|balance|buying-power|sellable-quantity|execution-history|order|orders|live)(?:\/|$)/i;
const productApiPath =
  /^\/api\/v1\/(?:research(?:\/|$)|strategies(?:\/|$)|recommendations(?:\/|$)|backtests(?:\/|$)|paper(?:\/|$)|screener(?:\/|$))/;
const forbiddenRequests = new Map<Page, string[]>();
const productRequests = new Map<Page, string[]>();
const guardedPages: Page[] = [];

type ActualSession = {
  readonly cookie_name: string;
  readonly cookie_value: string;
  readonly expires_at_secs: number;
};

type ActualProof = {
  readonly expected_mode: string;
  readonly host_observed_at: Readonly<Record<string, string | null>>;
  readonly policy_state: string;
  readonly status: number;
};

function screenshotPath(testInfo: TestInfo, name: string): string {
  const root = process.env["SERVICE_STATUS_SCREENSHOT_DIR"];
  if (root === undefined) return testInfo.outputPath(name);
  const prefix = resolve(process.cwd(), "../../test-results/service-status-qa");
  const relative = root.slice(prefix.length + 1);
  if (
    !root.startsWith(`${prefix}/`) ||
    !/^lagrange-service-status-wp5-[A-Za-z0-9._-]+\/(web-off|market-stream|actual)\/screenshots$/.test(
      relative,
    ) ||
    realpathSync(root) !== root ||
    !lstatSync(root).isDirectory() ||
    lstatSync(root).uid !== process.getuid?.()
  ) {
    throw new Error(
      "SERVICE_STATUS_SCREENSHOT_DIR must be the owned, disk-backed WP-5 artifact tree",
    );
  }
  return resolve(root, name);
}

async function resetScenario(request: APIRequestContext, scenario: Record<string, unknown>) {
  const response = await request.post(`${syntheticOrigin}/__test/scenario`, { data: scenario });
  expect(response.ok(), "synthetic scenario reset").toBeTruthy();
}

function isLoopback(url: URL): boolean {
  const allowed = new Set([new URL(appOrigin).port, new URL(syntheticOrigin).port]);
  return ["127.0.0.1", "localhost"].includes(url.hostname) && allowed.has(url.port);
}

async function guardProviderNetwork(page: Page) {
  const blocked: string[] = [];
  forbiddenRequests.set(page, blocked);
  const products: string[] = [];
  productRequests.set(page, products);
  guardedPages.push(page);
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.pathname.startsWith("/api/") && forbiddenApiPath.test(url.pathname))
      blocked.push(url.pathname);
    if (productApiPath.test(url.pathname)) products.push(url.pathname);
  });
  await page.routeWebSocket("**/*", async (socket) => {
    const url = new URL(socket.url());
    if (isLoopback(url) && url.port === new URL(appOrigin).port && url.pathname === "/_next/hmr") {
      socket.connectToServer();
    } else {
      await socket.close({ code: 1000, reason: "provider-free service-status QA" });
    }
  });
  await page.route("**/*", async (route) => {
    if (isLoopback(new URL(route.request().url()))) await route.continue();
    else await route.abort("blockedbyclient");
  });
}

async function ownerScenario(request: APIRequestContext) {
  await resetScenario(request, {
    authSession: "valid",
    ownerBetaAccessMode: "owner_only",
    role: "owner",
    stockBetaRows: 1,
    stockBetaSeed: "ready",
  });
}

async function openOwnerPage(page: Page, request: APIRequestContext, locale = "en") {
  await ownerScenario(request);
  await page.context().addCookies([{ name: "locale", value: locale, url: appOrigin }]);
  await page.goto("/");
  await expect(page.getByTestId("service-status-panel")).toBeVisible();
}

async function openIndependentStockBetaPage(page: Page) {
  const stockBetaPage = await page.context().newPage();
  await guardProviderNetwork(stockBetaPage);
  const statusFixture = await installServiceStatusFixture(stockBetaPage);
  await stockBetaPage.goto("/stock-beta");
  return { page: stockBetaPage, statusFixture };
}

async function expectNoStockBetaServiceStatus(page: Page, fixture: { calls(): number }) {
  await expect(page.getByTestId("service-status-panel")).toHaveCount(0);
  expect(fixture.calls()).toBe(0);
}

async function waitForStatusCall(fixture: { calls(): number }, expected: number) {
  await expect.poll(() => fixture.calls(), { timeout: 10_000 }).toBe(expected);
}

async function expectRowCell(row: Locator, column: 2 | 3 | 4, value: string) {
  await expect(
    row.locator(`td:nth-child(${column})`).getByText(value, { exact: true }),
  ).toBeVisible();
}

function serviceRow(page: Page, scope: Locator, name: string) {
  return scope.locator("tbody tr").filter({
    has: page.locator('th[scope="row"]').getByText(name, { exact: true }),
  });
}

async function viewportGeometry(locator: Locator) {
  return locator.evaluate((element) => {
    const rect = element.getBoundingClientRect();
    const clip = { left: 0, top: 0, right: innerWidth, bottom: innerHeight };
    const ancestors = [];
    for (let ancestor = element.parentElement; ancestor; ancestor = ancestor.parentElement) {
      const style = getComputedStyle(ancestor);
      const bounds = ancestor.getBoundingClientRect();
      const client = {
        left: bounds.left + ancestor.clientLeft,
        top: bounds.top + ancestor.clientTop,
        right: bounds.left + ancestor.clientLeft + ancestor.clientWidth,
        bottom: bounds.top + ancestor.clientTop + ancestor.clientHeight,
      };
      if (/^(auto|scroll|hidden|clip)$/.test(style.overflowX)) {
        clip.left = Math.max(clip.left, client.left);
        clip.right = Math.min(clip.right, client.right);
      }
      if (/^(auto|scroll|hidden|clip)$/.test(style.overflowY)) {
        clip.top = Math.max(clip.top, client.top);
        clip.bottom = Math.min(clip.bottom, client.bottom);
      }
      ancestors.push({
        tag: ancestor.tagName.toLowerCase(),
        id: ancestor.id,
        overflowX: style.overflowX,
        overflowY: style.overflowY,
        client,
        clientWidth: ancestor.clientWidth,
        clientHeight: ancestor.clientHeight,
        scrollWidth: ancestor.scrollWidth,
        scrollHeight: ancestor.scrollHeight,
        scrollTop: ancestor.scrollTop,
      });
    }
    const main = element.closest("main");
    return {
      viewport: { width: innerWidth, height: innerHeight },
      rect: {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
        width: rect.width,
        height: rect.height,
      },
      clip,
      ancestors,
      documentScrollTop: document.scrollingElement?.scrollTop ?? 0,
      mainScrollTop: main?.scrollTop ?? 0,
      scrollSurface:
        main !== null && getComputedStyle(main).overflowY === "auto" ? "main" : "document",
    };
  });
}

async function expectViewportReachable(locator: Locator, label: string) {
  await expect(locator, label).toBeVisible();
  const geometry = await viewportGeometry(locator);
  const evidence = `${label}: ${JSON.stringify(geometry)}`;
  // One CSS pixel tolerates border/subpixel rounding, not hidden content.
  expect(geometry.rect.left, evidence).toBeGreaterThanOrEqual(geometry.clip.left - 1);
  expect(geometry.rect.right, evidence).toBeLessThanOrEqual(geometry.clip.right + 1);
  expect(geometry.rect.top, evidence).toBeGreaterThanOrEqual(geometry.clip.top - 1);
  expect(geometry.rect.bottom, evidence).toBeLessThanOrEqual(geometry.clip.bottom + 1);
  return geometry;
}

async function setVisibility(page: Page, visibility: "hidden" | "visible") {
  await page.evaluate((next) => {
    Object.defineProperty(document, "visibilityState", { configurable: true, value: next });
    document.dispatchEvent(new Event("visibilitychange"));
  }, visibility);
}

async function setOnline(page: Page, online: boolean) {
  await page.evaluate((next) => {
    Object.defineProperty(Navigator.prototype, "onLine", { configurable: true, get: () => next });
    window.dispatchEvent(new Event(next ? "online" : "offline"));
  }, online);
}

async function expectNoHorizontalOverflow(page: Page) {
  const dimensions = await page.evaluate(() => ({
    clientWidth: document.documentElement.clientWidth,
    scrollWidth: document.documentElement.scrollWidth,
  }));
  expect(dimensions.scrollWidth, JSON.stringify(dimensions)).toBeLessThanOrEqual(
    dimensions.clientWidth,
  );
}

async function readableCaptionBounds(panel: Locator) {
  const caption = panel.locator("caption");
  await expect(caption).toHaveCount(1);
  await expect(caption).toBeVisible();
  const bounds = await caption.boundingBox();
  const panelBounds = await panel.boundingBox();
  expect(bounds).not.toBeNull();
  expect(panelBounds).not.toBeNull();
  expect(bounds?.width, "caption occupies the readable panel width").toBeGreaterThanOrEqual(
    (panelBounds?.width ?? 0) - 48,
  );
  expect(bounds?.height, "caption wraps within a small number of lines").toBeLessThanOrEqual(48);
  return bounds;
}

async function actualApiRead(page: Page, testInfo: TestInfo) {
  const sessionFile = process.env["SERVICE_STATUS_SESSION_FILE"];
  if (sessionFile === undefined || !sessionFile.startsWith("/tmp/lagrange-service-status-wp5-")) {
    throw new Error(
      "SERVICE_STATUS_SESSION_FILE must be a private handoff in the named WP-5 /tmp tree",
    );
  }
  const session = JSON.parse(readFileSync(sessionFile, "utf8")) as ActualSession;
  expect(session.cookie_name).toBe("__Host-lagrange_session");
  expect(session.cookie_value.length).toBeGreaterThan(20);
  // Playwright rewrites the Secure flag from url.protocol. Keep the Host cookie
  // secure using its HTTPS setting URL; no request is sent to this setting URL.
  const cookieUrl = new URL(appOrigin);
  cookieUrl.protocol = "https:";
  await page.context().addCookies([
    {
      name: session.cookie_name,
      value: session.cookie_value,
      url: cookieUrl.href,
      httpOnly: true,
      secure: true,
      sameSite: "Lax",
      expires: session.expires_at_secs,
    },
  ]);

  const installedSession = (await page.context().cookies(cookieUrl.href)).find(
    (cookie) => cookie.name === session.cookie_name,
  );
  expect(installedSession?.secure).toBe(true);
  expect(installedSession?.httpOnly).toBe(true);
  expect(installedSession?.path).toBe("/");

  const responsePromise = page.waitForResponse((response) => {
    const url = new URL(response.url());
    return url.pathname === statusPath && response.request().method() === "GET";
  });
  await page.goto("/");
  const response = await responsePromise;
  await expect(page.getByTestId("service-status-panel")).toBeVisible();
  expect(response.status()).toBe(200);
  expect(response.headers()["cache-control"]).toContain("private");
  expect(response.headers()["cache-control"]).toContain("no-store");
  expect(response.headers()["vary"]).toContain("Cookie");
  const body = (await response.json()) as {
    readonly policy_state: string;
    readonly services: readonly {
      readonly id: string;
      readonly expected_mode: string;
      readonly sources: { readonly host: { readonly observed_at: string | null } | null };
    }[];
  };
  expect(body.policy_state).toBe("verified");
  expect(body.services).toHaveLength(22);
  const dailyMarketData = body.services.find((service) => service.id === "kis-eod");
  expect(dailyMarketData).toBeDefined();
  const phase = process.env["SERVICE_STATUS_REAL_PHASE"];
  if (phase !== "initial" && phase !== "updated") {
    throw new Error("SERVICE_STATUS_REAL_PHASE must be initial or updated");
  }
  expect(dailyMarketData?.expected_mode).toBe(phase === "initial" ? "daily" : "daemon");

  const hostObservedAt = Object.fromEntries(
    body.services.map((service) => [service.id, service.sources.host?.observed_at ?? null]),
  );
  const proof: ActualProof = {
    expected_mode: dailyMarketData?.expected_mode ?? "unknown",
    host_observed_at: hostObservedAt,
    policy_state: body.policy_state,
    status: response.status(),
  };
  const proofFile = process.env["SERVICE_STATUS_ACTUAL_PROOF_FILE"];
  if (proofFile === undefined || !proofFile.startsWith("/tmp/lagrange-service-status-wp5-")) {
    throw new Error("SERVICE_STATUS_ACTUAL_PROOF_FILE must be in the named WP-5 /tmp tree");
  }
  mkdirSync(dirname(proofFile), { recursive: true, mode: 0o700 });
  if (phase === "initial") {
    writeFileSync(proofFile, `${JSON.stringify(proof, null, 2)}\n`, { mode: 0o600 });
  } else {
    const previous = JSON.parse(readFileSync(proofFile, "utf8")) as ActualProof;
    expect(previous.expected_mode).toBe("daily");
    expect(proof.expected_mode).toBe("daemon");
    expect(proof.host_observed_at).toEqual(previous.host_observed_at);
    expect(previous.status).toBe(200);
    writeFileSync(proofFile, `${JSON.stringify(proof, null, 2)}\n`, { mode: 0o600 });
  }
  await testInfo.attach(`actual-api-${phase}-safe-proof`, {
    body: JSON.stringify(proof, null, 2),
    contentType: "application/json",
  });
}

test.describe("Owner service-status browser acceptance", () => {
  test.beforeEach(async ({ page }) => {
    guardedPages.length = 0;
    await guardProviderNetwork(page);
  });

  test.afterEach(async ({ page }) => {
    for (const guardedPage of guardedPages) {
      expect(forbiddenRequests.get(guardedPage) ?? []).toEqual([]);
    }
    expect(productRequests.get(page) ?? []).toEqual([]);
    guardedPages.length = 0;
    forbiddenRequests.clear();
    productRequests.clear();
  });

  test("keeps a compact collapsed summary and reveals all 22 stable rows with separate process/work state", async ({
    page,
    request,
  }) => {
    const fixture = await installServiceStatusFixture(page);
    await page.clock.install({ time: new Date("2026-10-08T00:00:00Z") });
    await openOwnerPage(page, request);
    await waitForStatusCall(fixture, 1);

    const panel = page.getByTestId("service-status-panel");
    const disclosure = panel.locator("details");
    const summary = disclosure.locator("summary");
    await expect(disclosure).not.toHaveAttribute("open", "");
    await expect(disclosure.locator("tbody tr")).toHaveCount(22);
    await expect(disclosure.locator("tbody tr").first()).toBeHidden();
    await expect(panel).toHaveText(/Service status/);
    expect(await panel.evaluate((element) => element.getBoundingClientRect().height)).toBeLessThan(
      190,
    );

    await summary.focus();
    await expect(summary).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(disclosure).toHaveAttribute("open", "");
    await expect(disclosure.locator("tbody tr")).toHaveCount(22);
    await expect(summary).toBeFocused();

    const recommendation = serviceRow(page, disclosure, "Recommendations");
    await expect(recommendation.getByText("Running", { exact: true })).toBeVisible();
    await expect(recommendation.getByText("Waiting for work", { exact: true })).toBeVisible();
    fixture.setCase("running_claim");
    await page.clock.fastForward(30_000);
    await waitForStatusCall(fixture, 2);
    await expect(summary).toBeFocused();
  });

  test("keeps main Dashboard status independent while the Stock Beta chart loads", async ({
    page,
    request,
  }) => {
    const fixture = await installServiceStatusFixture(page);
    fixture.setFault({ kind: "delayed", status: 200, body: fixture.currentBody() });
    await openOwnerPage(page, request);
    const panel = page.getByTestId("service-status-panel");
    await expect(panel).toBeVisible();
    await expect(panel.locator("summary")).toContainText("Loading service status");

    const stockBeta = await openIndependentStockBetaPage(page);
    await expect(stockBeta.page.getByTestId("stock-beta-price-chart")).toBeVisible();
    await expect(stockBeta.page.getByTestId("stock-beta-current-quote")).toHaveCount(0);
    await expectNoStockBetaServiceStatus(stockBeta.page, stockBeta.statusFixture);

    await fixture.releaseDelayed();
    await waitForStatusCall(fixture, 1);
    await expect(panel.locator("tbody tr")).toHaveCount(22);
  });

  test("does not request Owner details for Member or anonymous pages", async ({
    page,
    request,
  }) => {
    const fixture = await installServiceStatusFixture(page);
    await resetScenario(request, {
      authSession: "valid",
      ownerBetaAccessMode: "disabled",
      role: "member",
      stockBetaRows: 1,
      stockBetaSeed: "ready",
    });
    await page.goto("/");
    await expect(page.getByTestId("service-status-panel")).toHaveCount(0);
    expect(fixture.calls()).toBe(0);
    const workspaceOverview = page.locator(".workspace-overview");
    for (const href of ["/strategies", "/recommendations", "/backtests", "/paper"]) {
      await expect(workspaceOverview.locator(`a[href="${href}"]`)).toBeVisible();
    }
    await expect(workspaceOverview.locator('a[href="/stock-beta"]')).toHaveCount(0);

    await resetScenario(request, { authSession: "unknown", role: "member" });
    await page.goto("/");
    await expect(page).toHaveURL(/\/login(?:\?|$)/);
    await expect(page.getByTestId("service-status-panel")).toHaveCount(0);
    expect(fixture.calls()).toBe(0);
  });

  for (const status of [401, 403]) {
    test(`${status} clears previously visible details`, async ({ page, request }) => {
      const fixture = await installServiceStatusFixture(page);
      await page.clock.install({ time: new Date("2026-10-08T00:00:00Z") });
      await openOwnerPage(page, request);
      await waitForStatusCall(fixture, 1);
      const panel = page.getByTestId("service-status-panel");
      await panel.locator("summary").click();
      await expect(panel.locator("details")).toHaveAttribute("open", "");
      await expect(panel.locator("tbody tr")).toHaveCount(22);

      fixture.setFault({ kind: "json", status, body: { error: { code: "FORBIDDEN" } } });
      await setVisibility(page, "hidden");
      await setVisibility(page, "visible");
      await page.clock.fastForward(5_000);
      await waitForStatusCall(fixture, 2);
      await expect(panel.locator("tbody tr")).toHaveCount(0);
      await expect(panel).toContainText("Service details are no longer available");
      fixture.setFault(null);
      await page.reload();
      await waitForStatusCall(fixture, 3);
      await expect(panel.locator("tbody tr")).toHaveCount(22);
    });
  }

  for (const fault of [
    { name: "network failure", kind: "network" as const },
    { name: "503 outage", kind: "json" as const, status: 503 },
  ]) {
    test(`marks the last reading stale after a ${fault.name}`, async ({ page, request }) => {
      const fixture = await installServiceStatusFixture(page);
      await page.clock.install({ time: new Date("2026-10-08T00:00:00Z") });
      await openOwnerPage(page, request);
      await waitForStatusCall(fixture, 1);
      fixture.setFault({ kind: fault.kind, status: fault.status ?? 0, body: {} });
      await setVisibility(page, "hidden");
      await setVisibility(page, "visible");
      await page.clock.fastForward(5_000);
      await waitForStatusCall(fixture, 2);
      const panel = page.getByTestId("service-status-panel");
      await panel.locator("summary").click();
      await expect(panel.locator("tbody tr")).toHaveCount(22);
      await expect(panel.getByText("Needs a new check", { exact: true }).first()).toBeVisible();
      await expect(panel).toContainText("Previous readings are shown as stale");
    });
  }

  test("honors 429 Retry-After and the minimum request gap", async ({ page, request }) => {
    const fixture = await installServiceStatusFixture(page);
    await page.clock.install({ time: new Date("2026-10-08T00:00:00Z") });
    fixture.setFault({ kind: "json", status: 429, headers: { "Retry-After": "2" }, body: {} });
    await openOwnerPage(page, request);
    await waitForStatusCall(fixture, 1);
    await expect(page.getByTestId("service-status-panel")).toContainText("rate limited");
    fixture.setFault(null);
    await page.clock.fastForward(4_000);
    expect(fixture.calls()).toBe(1);
    await page.clock.fastForward(1_000);
    await waitForStatusCall(fixture, 2);
    await expect(page.getByTestId("service-status-panel").locator("tbody tr")).toHaveCount(22);
  });

  test("keeps refreshes serial across delay, hidden, offline, and recovery transitions", async ({
    page,
    request,
  }) => {
    const fixture = await installServiceStatusFixture(page);
    await page.clock.install({ time: new Date("2026-10-08T00:00:00Z") });
    fixture.setFault({ kind: "delayed", status: 200, body: fixture.currentBody() });
    await openOwnerPage(page, request);
    await expect(page.getByTestId("service-status-panel")).toBeVisible();
    await expect.poll(() => fixture.isPending()).toBe(true);
    await page.clock.fastForward(60_000);
    expect(fixture.calls()).toBe(1);
    await fixture.releaseDelayed();
    await waitForStatusCall(fixture, 1);

    await setVisibility(page, "hidden");
    await setOnline(page, false);
    await page.clock.fastForward(60_000);
    expect(fixture.calls()).toBe(1);
    await setOnline(page, true);
    await setVisibility(page, "visible");
    await page.clock.fastForward(5_000);
    await waitForStatusCall(fixture, 2);
    await expect(page.getByTestId("service-status-panel").locator("tbody tr")).toHaveCount(22);
  });

  test("does not extend healthy evidence past monotonic expiry when polling stops or wall time jumps", async ({
    page,
    request,
  }) => {
    const fixture = await installServiceStatusFixture(page);
    await page.clock.install({ time: new Date("2026-10-08T00:00:00Z") });
    const body = fixture.currentBody();
    const gateway = body.services.find((service: { id: string }) => service.id === "reverse-proxy");
    expect(gateway).toBeDefined();
    const deadline = new Date(Date.parse(body.generated_at) + 30_000).toISOString();
    gateway.valid_until = deadline;
    gateway.sources.host.valid_until = deadline;
    fixture.setBody(body);
    await openOwnerPage(page, request);
    await waitForStatusCall(fixture, 1);
    const panel = page.getByTestId("service-status-panel");
    await panel.locator("summary").click();
    await expect(panel.locator("tbody tr")).toHaveCount(22);
    await setVisibility(page, "hidden");
    await page.clock.fastForward(20_000);
    const gatewayRow = serviceRow(page, panel, "Web gateway");
    await expectRowCell(gatewayRow, 2, "Healthy");
    await expectRowCell(gatewayRow, 3, "Running");
    await expectRowCell(gatewayRow, 4, "Not applicable");
    await page.clock.setFixedTime(new Date("2099-01-01T00:00:00Z"));
    await page.clock.fastForward(10_000);
    await expectRowCell(gatewayRow, 2, "Unknown");
    await expectRowCell(gatewayRow, 3, "Unknown");
    await expectRowCell(gatewayRow, 4, "Unknown");
    await expect(gatewayRow.getByText("Needs a new check", { exact: true })).toBeVisible();
    expect(fixture.calls()).toBe(1);
  });

  const projectionCases: Array<{
    readonly id: string;
    readonly serviceId: string;
    readonly name: string;
    readonly status: string;
    readonly process?: string;
    readonly work: string;
    readonly evidence?: readonly string[];
    readonly expiredProjection?: {
      readonly process: "running" | "unknown";
      readonly work: "not_applicable" | "unknown";
      readonly reason: string;
    };
  }> = [
    {
      id: "baseline_idle_and_fresh",
      serviceId: "recommendation-runner",
      name: "Recommendations",
      status: "Waiting",
      work: "Waiting for work",
    },
    {
      id: "same_job_recovered",
      serviceId: "owner-equity-v2-runner",
      name: "Owner research",
      status: "Healthy",
      work: "Up to date",
    },
    {
      id: "eod_after_grace",
      serviceId: "kis-eod",
      name: "Daily market data",
      status: "Needs attention",
      work: "Out of date",
    },
    {
      id: "daily_timer_disabled",
      serviceId: "kis-eod",
      name: "Daily market data",
      status: "Failed",
      work: "Failed",
    },
    {
      id: "daily_timer_missing",
      serviceId: "kis-eod",
      name: "Daily market data",
      status: "Failed",
      work: "Failed",
    },
    {
      id: "market_no_demand",
      serviceId: "market-stream",
      name: "Live quotes",
      status: "Waiting",
      work: "No active work",
    },
    {
      id: "market_closed",
      serviceId: "market-stream",
      name: "Live quotes",
      status: "Waiting",
      work: "Closed",
    },
    {
      id: "stream_gap",
      serviceId: "market-stream",
      name: "Live quotes",
      status: "Needs attention",
      work: "Limited",
    },
    {
      id: "market_disconnected",
      serviceId: "market-stream",
      name: "Live quotes",
      status: "Failed",
      process: "Running",
      work: "Failed",
    },
    {
      id: "calendar_unavailable_no_demand",
      serviceId: "intraday-calendar",
      name: "Trading calendar",
      status: "Failed",
      work: "Failed",
      evidence: ["Missing", "Open"],
    },
    {
      id: "window_missing",
      serviceId: "intraday-calendar",
      name: "Trading calendar",
      status: "Failed",
      work: "Failed",
      evidence: ["Ready", "Missing"],
    },
    {
      id: "etf_partial_coverage",
      serviceId: "kis-eod",
      name: "Daily market data",
      status: "Needs attention",
      work: "Out of date",
    },
    {
      id: "host_timeout",
      expiredProjection: { process: "unknown", work: "not_applicable", reason: "HOST_TIMEOUT" },
      serviceId: "web",
      name: "Web application",
      status: "Unknown",
      work: "Unknown",
    },
    {
      id: "host_ttl_90_000001",
      expiredProjection: { process: "unknown", work: "not_applicable", reason: "HOST_STALE" },
      serviceId: "postgres",
      name: "Database",
      status: "Unknown",
      work: "Unknown",
    },
    {
      id: "queue_db_timeout",
      expiredProjection: { process: "running", work: "unknown", reason: "DB_TIMEOUT" },
      serviceId: "recommendation-runner",
      name: "Recommendations",
      status: "Unknown",
      work: "Unknown",
    },
    {
      id: "host_future",
      expiredProjection: { process: "unknown", work: "not_applicable", reason: "HOST_FUTURE" },
      serviceId: "reverse-proxy",
      name: "Web gateway",
      status: "Unknown",
      work: "Unknown",
    },
  ];

  // Playwright disposes the default page/context (including trace/video) after every case.
  for (const item of projectionCases) {
    test(`shows the frozen ${item.id} browser projection`, async ({ page, request }, testInfo) => {
      const fixture = await installServiceStatusFixture(page, { caseId: item.id });
      const service = fixture
        .currentBody()
        .services.find((candidate: { id: string }) => candidate.id === item.serviceId);
      expect(service, "the selected frozen API row").toBeDefined();
      if (item.expiredProjection !== undefined) {
        // The API preserves the diagnosis; the controller expires all three displayed states.
        expect(service.valid_until).toBeNull();
        expect(service.status).toBe("unknown");
        expect(service.process_status).toBe(item.expiredProjection.process);
        expect(service.work_status).toBe(item.expiredProjection.work);
        expect(service.reason_codes).toContain(item.expiredProjection.reason);
        await testInfo.attach("frozen-api-projection-diagnosis", {
          body: JSON.stringify(
            {
              id: service.id,
              status: service.status,
              process_status: service.process_status,
              work_status: service.work_status,
              valid_until: service.valid_until,
              reason_codes: service.reason_codes,
            },
            null,
            2,
          ),
          contentType: "application/json",
        });
      }
      await page.clock.install({ time: new Date("2026-10-08T00:00:00Z") });
      await openOwnerPage(page, request);
      await waitForStatusCall(fixture, 1);
      const panel = page.getByTestId("service-status-panel");
      await panel.locator("summary").click();
      await expect(panel.locator("tbody tr")).toHaveCount(22);
      const row = serviceRow(page, panel, item.name);
      await expect(row).toHaveCount(1);
      await expectRowCell(row, 2, item.status);
      if (item.expiredProjection !== undefined) {
        await expectRowCell(row, 3, "Unknown");
        await expectRowCell(row, 4, "Unknown");
        await expect(row.getByText("Needs a new check", { exact: true })).toBeVisible();
      } else {
        if (item.process !== undefined) await expectRowCell(row, 3, item.process);
        await expectRowCell(row, 4, item.work);
      }
      for (const evidence of item.evidence ?? []) {
        await expect(row.getByText(evidence, { exact: true })).toBeVisible();
      }
      if (item.id === "host_timeout") {
        const unaffectedHost = serviceRow(page, panel, "Web gateway");
        await expectRowCell(unaffectedHost, 2, "Healthy");
        await expectRowCell(unaffectedHost, 3, "Running");
        await expectRowCell(unaffectedHost, 4, "Not applicable");
      }
      expect(fixture.calls()).toBe(1);
    });
  }

  const layoutSizes = [
    { width: 375, height: 800, label: "375x800" },
    { width: 1024, height: 900, label: "1024x900" },
    { width: 1280, height: 720, label: "1280x720" },
    { width: 1440, height: 900, label: "1440x900" },
  ];

  // Independent locale/viewport captures retain screenshots, traces and videos per test.
  for (const locale of ["ko", "en"]) {
    for (const size of layoutSizes) {
      test(`captures the ${locale} ${size.label} Dashboard layout and separate Stock Beta chart`, async ({
        page,
        request,
      }, testInfo) => {
        const fixture = await installServiceStatusFixture(page);
        const layoutMeasurements: Array<Record<string, unknown>> = [];
        await page.setViewportSize({ width: size.width, height: size.height });
        fixture.setCase("baseline_idle_and_fresh");
        await openOwnerPage(page, request, locale);
        await waitForStatusCall(fixture, 1);

        const stockBeta = await openIndependentStockBetaPage(page);
        await stockBeta.page.setViewportSize({ width: size.width, height: size.height });
        const chart = stockBeta.page.getByTestId("stock-beta-price-chart");
        await expect(chart).toBeVisible();
        await expectNoStockBetaServiceStatus(stockBeta.page, stockBeta.statusFixture);

        const panel = page.getByTestId("service-status-panel");
        const workspace = page.locator(".workspace-overview");
        await expect(panel.locator("details")).not.toHaveAttribute("open", "");
        const panelBox = await panel.boundingBox();
        const workspaceBox = await workspace.boundingBox();
        const dashboardBox = await stockBeta.page.getByTestId("stock-beta-dashboard").boundingBox();
        const profileBox = await stockBeta.page
          .getByTestId("stock-beta-signal-profile")
          .boundingBox();
        const chartBox = await chart.boundingBox();
        expect(panelBox).not.toBeNull();
        expect(workspaceBox).not.toBeNull();
        expect(dashboardBox).not.toBeNull();
        expect(profileBox).not.toBeNull();
        expect(chartBox).not.toBeNull();
        expect(panelBox?.width).toBeGreaterThan(0);
        expect((panelBox?.y ?? 0) + (panelBox?.height ?? 0)).toBeLessThanOrEqual(
          workspaceBox?.y ?? 0,
        );
        expect(chartBox?.width).toBeGreaterThan(300);
        expect(profileBox?.width).toBeGreaterThanOrEqual((dashboardBox?.width ?? 0) - 32);
        expect(chartBox?.x).toBeGreaterThanOrEqual(profileBox?.x ?? 0);
        expect((chartBox?.x ?? 0) + (chartBox?.width ?? 0)).toBeLessThanOrEqual(
          (profileBox?.x ?? 0) + (profileBox?.width ?? 0),
        );
        expect(panelBox?.height).toBeLessThan(190);
        await expectNoHorizontalOverflow(page);
        await expectNoHorizontalOverflow(stockBeta.page);
        const dimensions = await page.evaluate(() => ({
          clientHeight: document.documentElement.clientHeight,
          clientWidth: document.documentElement.clientWidth,
          scrollHeight: document.documentElement.scrollHeight,
          scrollWidth: document.documentElement.scrollWidth,
        }));
        let verticalScrollY = 0;
        if (dimensions.scrollHeight > dimensions.clientHeight) {
          await page.evaluate(() => window.scrollTo(0, document.documentElement.scrollHeight));
          verticalScrollY = await page.evaluate(() => window.scrollY);
          expect(verticalScrollY).toBeGreaterThan(0);
          await page.evaluate(() => window.scrollTo(0, 0));
        }
        layoutMeasurements.push({
          locale,
          viewport: size.label,
          panel: panelBox,
          workspace: workspaceBox,
          chart: chartBox,
          verticalScrollY,
          ...dimensions,
        });
        await page.screenshot({
          animations: "disabled",
          fullPage: true,
          path: screenshotPath(testInfo, `service-status-${locale}-${size.label}.png`),
        });
        const summary = panel.locator("summary");
        await summary.focus();
        await page.keyboard.press("Enter");
        await expect(summary).toBeFocused();
        await expect(panel.locator("details")).toHaveAttribute("open", "");
        await expect(panel.locator("tbody tr")).toHaveCount(22);
        const caption = await readableCaptionBounds(panel);
        layoutMeasurements.push({ caption });
        await expect(panel.locator("tbody tr").first()).toBeVisible();
        await expectNoHorizontalOverflow(page);
        await page.screenshot({
          animations: "disabled",
          fullPage: true,
          path: screenshotPath(testInfo, `service-status-${locale}-${size.label}-expanded.png`),
        });

        // The desktop shell scrolls its main element; fullPage cannot expand it.
        // Keep that capture, then prove every row is reachable through real scrolling
        // and save honest viewport frames at the summary and the table end.
        await summary.scrollIntoViewIfNeeded();
        const expandedTop = await expectViewportReachable(summary, "expanded summary");
        await page.screenshot({
          animations: "disabled",
          fullPage: false,
          path: screenshotPath(testInfo, `service-status-${locale}-${size.label}-expanded-top.png`),
        });
        await page.keyboard.press("PageDown");
        await expect
          .poll(async () => {
            const geometry = await viewportGeometry(summary);
            return geometry.scrollSurface === "main"
              ? geometry.mainScrollTop
              : geometry.documentScrollTop;
          })
          .toBeGreaterThan(
            expandedTop.scrollSurface === "main"
              ? expandedTop.mainScrollTop
              : expandedTop.documentScrollTop,
          );
        const keyboardScroll = await viewportGeometry(summary);
        await expect(summary).toBeFocused();
        await summary.scrollIntoViewIfNeeded();
        const rows = panel.locator("tbody tr");
        const reachableRows = [];
        for (let index = 0; index < 22; index += 1) {
          const row = rows.nth(index);
          const identity = await row.locator('th[scope="row"] > span:first-child').textContent();
          await row.scrollIntoViewIfNeeded();
          reachableRows.push({
            index,
            identity,
            ...(await expectViewportReachable(row, `service row ${index}: ${identity}`)),
          });
        }
        const tableEnd = reachableRows[21];
        expect(tableEnd).toBeDefined();
        if (expandedTop.scrollSurface === "main") {
          expect(tableEnd?.mainScrollTop).toBeGreaterThan(expandedTop.mainScrollTop);
        } else {
          expect(tableEnd?.documentScrollTop).toBeGreaterThan(expandedTop.documentScrollTop);
        }
        await expectNoHorizontalOverflow(page);
        await page.screenshot({
          animations: "disabled",
          fullPage: false,
          path: screenshotPath(testInfo, `service-status-${locale}-${size.label}-table-end.png`),
        });
        await summary.scrollIntoViewIfNeeded();
        const returnedSummary = await expectViewportReachable(summary, "returned summary");
        await expect(summary).toBeFocused();
        layoutMeasurements.push({ expandedTop, keyboardScroll, reachableRows, returnedSummary });
        await page.keyboard.press("Enter");
        await expect(panel.locator("details")).not.toHaveAttribute("open", "");

        await testInfo.attach("service-status-layout-measurements", {
          body: JSON.stringify(layoutMeasurements, null, 2),
          contentType: "application/json",
        });
      });
    }
  }

  test("captures the English 200% viewport-equivalent layout", async ({
    page,
    request,
  }, testInfo) => {
    const fixture = await installServiceStatusFixture(page);
    const layoutMeasurements: Array<Record<string, unknown>> = [];
    await page.setViewportSize({ width: 640, height: 360 });
    await openOwnerPage(page, request, "en");
    await waitForStatusCall(fixture, 1);
    const stockBeta = await openIndependentStockBetaPage(page);
    await stockBeta.page.setViewportSize({ width: 640, height: 360 });
    await expect(stockBeta.page.getByTestId("stock-beta-price-chart")).toBeVisible();
    await expectNoStockBetaServiceStatus(stockBeta.page, stockBeta.statusFixture);
    await expectNoHorizontalOverflow(page);
    await expectNoHorizontalOverflow(stockBeta.page);
    layoutMeasurements.push({
      locale: "en",
      viewport: "640x360 (200% viewport equivalent)",
      ...(await page.evaluate(() => ({
        clientHeight: document.documentElement.clientHeight,
        clientWidth: document.documentElement.clientWidth,
        scrollHeight: document.documentElement.scrollHeight,
        scrollWidth: document.documentElement.scrollWidth,
      }))),
    });
    await page.screenshot({
      animations: "disabled",
      fullPage: true,
      path: screenshotPath(testInfo, "service-status-en-640x360-zoom-equivalent.png"),
    });

    await testInfo.attach("browser-zoom-limit", {
      body: "The 640x360 screenshot is a viewport-equivalent for 200% zoom. Playwright does not control browser UI zoom in this harness.",
      contentType: "text/plain",
    });

    await testInfo.attach("service-status-layout-measurements", {
      body: JSON.stringify(layoutMeasurements, null, 2),
      contentType: "application/json",
    });
  });

  test("captures keyboard disclosure and forced colors at 375x800", async ({
    page,
    request,
  }, testInfo) => {
    const fixture = await installServiceStatusFixture(page);
    const layoutMeasurements: Array<Record<string, unknown>> = [];
    await page.emulateMedia({ forcedColors: "active" });
    await page.setViewportSize({ width: 375, height: 800 });
    await openOwnerPage(page, request, "en");
    await waitForStatusCall(fixture, 1);
    const panel = page.getByTestId("service-status-panel");
    await expect(panel).toBeVisible();
    await panel.locator("summary").focus();
    await page.keyboard.press("Enter");
    await expect(panel.locator("summary")).toBeFocused();
    await expect(panel.locator("tbody tr")).toHaveCount(22);
    const caption = await readableCaptionBounds(panel);
    await expectNoHorizontalOverflow(page);
    layoutMeasurements.push({
      locale: "en",
      viewport: "375x800 forced colors",
      caption,
      ...(await page.evaluate(() => ({
        clientHeight: document.documentElement.clientHeight,
        clientWidth: document.documentElement.clientWidth,
        scrollHeight: document.documentElement.scrollHeight,
        scrollWidth: document.documentElement.scrollWidth,
      }))),
    });
    await page.screenshot({
      animations: "disabled",
      fullPage: true,
      path: screenshotPath(testInfo, "service-status-en-375x800-forced-colors.png"),
    });
    await page.screenshot({
      animations: "disabled",
      fullPage: false,
      path: screenshotPath(testInfo, "service-status-en-375x800-forced-colors-top.png"),
    });

    await testInfo.attach("service-status-layout-measurements", {
      body: JSON.stringify(layoutMeasurements, null, 2),
      contentType: "application/json",
    });
  });

  test("keeps Dashboard status available when the Stock Beta chart read fails", async ({
    page,
    request,
  }) => {
    const fixture = await installServiceStatusFixture(page);
    await resetScenario(request, {
      authSession: "valid",
      ownerBetaAccessMode: "owner_only",
      role: "owner",
      stockBetaChartState: "unavailable",
      stockBetaRows: 1,
      stockBetaSeed: "ready",
    });
    await page.goto("/");
    await expect(page.getByTestId("service-status-panel")).toBeVisible();
    await waitForStatusCall(fixture, 1);
    await expect(page.getByTestId("service-status-panel").locator("tbody tr")).toHaveCount(22);

    const stockBeta = await openIndependentStockBetaPage(page);
    await expect(stockBeta.page.getByTestId("stock-beta-price-chart")).toHaveCount(0);
    await expect(
      stockBeta.page
        .getByRole("tabpanel", { name: "Price", exact: true })
        .getByText("The EOD chart is being prepared or is unavailable.", { exact: true }),
    ).toBeVisible();
    await expectNoStockBetaServiceStatus(stockBeta.page, stockBeta.statusFixture);
  });

  test("actual DB-backed API router rereads the mounted snapshot and updates policy after atomic replacement @actual", async ({
    page,
  }, testInfo) => {
    await actualApiRead(page, testInfo);
  });
});
