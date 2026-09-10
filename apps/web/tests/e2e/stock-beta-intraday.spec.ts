import { type APIRequestContext, expect, type Page, test } from "@playwright/test";

const appOrigin = process.env["PLAYWRIGHT_BASE_URL"] ?? "http://127.0.0.1:33041";
const syntheticOrigin = process.env["SYNTHETIC_API_ORIGIN"] ?? "http://127.0.0.1:38191";
const quotePathFragment = "/api/v1/research/owner-beta/equity-universe-v2/instruments/";
const membershipsPath = "/api/v1/research/owner-beta/equity-universe-v2/memberships";
const demandPath = "/api/v1/research/owner-beta/equity-universe-v2/quote-demands";
const statePath = "/__test/stock-beta/intraday-state";
const quoteHeading = "Intraday price · periodic refresh";
const quoteA = "101200.00";
const quoteB = "198400.00";
const membershipA = "00000000-0000-4000-8000-000000000601";
const forbiddenIntradayTraffic =
  /(?:kis|opendart|\/uapi\/|inquire-price|websocket|(?:^|\/)(?:account|accounts|balance|buying-power|sellable-quantity|execution-history|order|orders|live)(?:\/|$))/i;

type SyntheticState = {
  readonly active_consumer_count: number;
  readonly active_identity_count: number;
  readonly active_identity_keys: readonly string[];
  readonly created_demands: readonly {
    readonly consumer_id: string;
    readonly demand_id: string;
    readonly generation: number;
    readonly instrument_id: string;
    readonly membership_id: string;
  }[];
  readonly demand_posts: number;
  readonly demand_releases: number;
  readonly demand_renewals: number;
  readonly max_active_identity_count: number;
  readonly quote_gets: number;
  readonly quote_gets_by_instrument: Readonly<Record<string, number>>;
  readonly rejected_demand_membership_ids: readonly string[];
  readonly rejected_quote_identities: readonly {
    readonly generation: number;
    readonly instrument_id: string;
    readonly membership_id: string;
  }[];
  readonly released_demand_ids: readonly string[];
};

type SyntheticMembership = {
  readonly generation: number;
  readonly id: string;
  readonly instrument_id: string;
  readonly lifecycle: string;
};

const observedBrowserRequests = new WeakMap<Page, URL[]>();
const observedBrowserWebSocketAttempts = new WeakMap<Page, string[]>();
const observedBrowserWebSocketConnections = new WeakMap<Page, string[]>();

function ownerScenario(overrides: Record<string, unknown> = {}) {
  return {
    authSession: "valid",
    ownerBetaAccessMode: "owner_only",
    role: "owner",
    stockBetaRows: 2,
    stockBetaSeed: "ready",
    stockBetaIntradayState: "open",
    ...overrides,
  };
}

async function resetScenario(request: APIRequestContext, scenario: Record<string, unknown>) {
  const response = await request.post(`${syntheticOrigin}/__test/scenario`, { data: scenario });
  expect(response.status(), "synthetic scenario reset status").toBe(200);
}

async function syntheticState(request: APIRequestContext): Promise<SyntheticState> {
  const response = await request.get(`${syntheticOrigin}${statePath}`);
  expect(response.status(), "synthetic state status").toBe(200);
  return (await response.json()) as SyntheticState;
}

async function syntheticMemberships(
  request: APIRequestContext,
): Promise<readonly SyntheticMembership[]> {
  const response = await request.get(`${syntheticOrigin}${membershipsPath}`);
  expect(response.status(), "synthetic memberships status").toBe(200);
  const body = (await response.json()) as { readonly memberships: readonly SyntheticMembership[] };
  return body.memberships;
}

function observeBrowserRequests(page: Page): URL[] {
  const requests: URL[] = [];
  const websocketAttempts: string[] = [];
  const websocketConnections: string[] = [];
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (["http:", "https:", "ws:", "wss:"].includes(url.protocol)) requests.push(url);
  });
  page.on("websocket", (webSocket) => websocketConnections.push(webSocket.url()));
  observedBrowserRequests.set(page, requests);
  observedBrowserWebSocketAttempts.set(page, websocketAttempts);
  observedBrowserWebSocketConnections.set(page, websocketConnections);
  return requests;
}

function requestsFor(page: Page): URL[] {
  return observedBrowserRequests.get(page) ?? [];
}

function isLocalTestUrl(url: URL): boolean {
  const expectedPorts = new Set([new URL(appOrigin).port, new URL(syntheticOrigin).port]);
  return ["127.0.0.1", "localhost"].includes(url.hostname) && expectedPorts.has(url.port);
}

function kstDate(timestamp: number): string {
  const parts = new Intl.DateTimeFormat("en-CA", {
    day: "2-digit",
    month: "2-digit",
    timeZone: "Asia/Seoul",
    year: "numeric",
  }).formatToParts(new Date(timestamp));
  const values = new Map(parts.map((part) => [part.type, part.value]));
  return `${values.get("year") ?? ""}-${values.get("month") ?? ""}-${values.get("day") ?? ""}`;
}

async function installProviderFreeNetworkGuard(page: Page): Promise<void> {
  await page.routeWebSocket("**/*", async (webSocket) => {
    observedBrowserWebSocketAttempts.get(page)?.push(webSocket.url());
    await webSocket.close({ code: 1000, reason: "synthetic QA blocks provider sockets" });
  });
  await page.route("**/*", async (route) => {
    const url = new URL(route.request().url());
    if (["http:", "https:"].includes(url.protocol) && isLocalTestUrl(url)) {
      await route.continue();
      return;
    }
    await route.abort("blockedbyclient");
  });
}

function assertProviderFree(page: Page): void {
  const requests = requestsFor(page);
  const external = requests.filter((url) => !isLocalTestUrl(url)).map((url) => url.href);
  expect(external, `non-loopback browser traffic: ${external.join(", ") || "none"}`).toEqual([]);
  const forbidden = requests
    .filter((url) => forbiddenIntradayTraffic.test(`${url.hostname}${url.pathname}${url.search}`))
    .map((url) => url.href);
  expect(
    forbidden,
    `forbidden intraday browser traffic: ${forbidden.join(", ") || "none"}`,
  ).toEqual([]);
  const browserQuoteProviderPaths = requests.filter((url) =>
    url.pathname.includes("equity-price-signals"),
  );
  expect(browserQuoteProviderPaths).toHaveLength(0);
  const websocketAttempts = observedBrowserWebSocketAttempts.get(page) ?? [];
  const websocketConnections = observedBrowserWebSocketConnections.get(page) ?? [];
  expect(
    [...websocketAttempts, ...websocketConnections],
    `browser WebSocket attempts: ${[...websocketAttempts, ...websocketConnections].join(", ") || "none"}`,
  ).toEqual([]);
}

function membershipCards(page: Page) {
  return page
    .getByRole("region", { name: "Membership status" })
    .getByTestId("stock-beta-membership-card");
}

function membershipCard(page: Page, instrumentId: string, lifecycle?: string) {
  const lifecycleSelector = lifecycle === undefined ? "" : `[data-lifecycle="${lifecycle}"]`;
  return page
    .getByRole("region", { name: "Membership status" })
    .locator(`li[data-testid="stock-beta-membership-card"]${lifecycleSelector}`)
    .filter({ hasText: instrumentId });
}

function rankedRow(page: Page, instrumentId: string) {
  return page
    .getByRole("region", { name: "Ranked signals" })
    .getByTestId(`stock-beta-row-${instrumentId}`);
}

function dashboardQuoteWidget(page: Page) {
  return page.getByTestId("stock-beta-widget-current-quote");
}

function detailQuoteWidget(page: Page) {
  return page.getByTestId("stock-beta-detail-widget-current-quote");
}

function quoteStatus(widget: ReturnType<typeof dashboardQuoteWidget>) {
  return widget.locator("[data-status-phase]");
}

async function expectReadableDetailQuote(page: Page): Promise<void> {
  const quote = detailQuoteWidget(page);
  await quote.scrollIntoViewIfNeeded();
  await expectReadyQuote(quote, quoteA);
  await expect(quote.locator("[data-last-success-at]")).toBeVisible();
  await expect
    .poll(async () =>
      quote.evaluate((element) => {
        const board = document.querySelector('[data-testid="stock-beta-detail-board"]');
        const policy = document.querySelector(
          '[data-testid="stock-beta-detail-widget-policy-boundary"]',
        );
        const price = element.querySelector("[data-quote-value]");
        const timestamp = element.querySelector("[data-last-success-at]");
        if (board === null || policy === null || price === null || timestamp === null) {
          return false;
        }
        const box = element.getBoundingClientRect();
        const boardBox = board.getBoundingClientRect();
        const policyBox = policy.getBoundingClientRect();
        const contained = (child: Element) => {
          const childBox = child.getBoundingClientRect();
          return (
            childBox.left >= box.left - 1 &&
            childBox.right <= box.right + 1 &&
            childBox.top >= box.top - 1 &&
            childBox.bottom <= box.bottom + 1
          );
        };
        return (
          boardBox.width > 0 &&
          box.width >= boardBox.width * 0.95 &&
          box.top >= policyBox.bottom - 1 &&
          contained(price) &&
          contained(timestamp)
        );
      }),
    )
    .toBe(true);
}

async function expectStockShell(page: Page): Promise<void> {
  await expect(page.locator('[data-shell="stock-beta-terminal"]')).toHaveCount(1);
  await expect(page.locator('[data-terminal-utility-bar="stock-beta"]')).toHaveCount(1);
}

async function openDashboard(
  page: Page,
  request: APIRequestContext,
  scenario: Record<string, unknown>,
) {
  await resetScenario(request, scenario);
  await page.goto("/stock-beta");
  await expectStockShell(page);
  const widget = dashboardQuoteWidget(page);
  await expect(widget).toHaveCount(1);
  await expect(widget.getByRole("heading", { name: quoteHeading })).toBeVisible();
  return widget;
}

async function expectReadyQuote(
  widget: ReturnType<typeof dashboardQuoteWidget> | ReturnType<typeof detailQuoteWidget>,
  rawPrice: string,
): Promise<void> {
  const quote = widget.getByTestId("stock-beta-current-quote");
  await expect(quote).toBeVisible();
  await expect(quote.locator(`[data-quote-value="${rawPrice}"]`)).toHaveCount(1);
  await expect(widget.locator('[data-status-phase="ready"]')).toBeVisible();
}

async function setVisibility(page: Page, state: "hidden" | "visible"): Promise<void> {
  await page.evaluate((nextState) => {
    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      value: nextState,
    });
    document.dispatchEvent(new Event("visibilitychange"));
  }, state);
}

async function stopSyntheticDemand(page: Page, request: APIRequestContext): Promise<void> {
  await setVisibility(page, "hidden");
  await waitForNoActiveSyntheticDemand(request);
}

async function setOnline(page: Page, online: boolean): Promise<void> {
  await page.evaluate((nextOnline) => {
    Object.defineProperty(Navigator.prototype, "onLine", {
      configurable: true,
      get: () => nextOnline,
    });
    window.dispatchEvent(new Event(nextOnline ? "online" : "offline"));
  }, online);
}

async function waitForNoActiveSyntheticDemand(request: APIRequestContext): Promise<void> {
  await expect
    .poll(async () => (await syntheticState(request)).active_consumer_count, {
      timeout: 5_000,
      intervals: [50, 100, 250, 500],
    })
    .toBe(0);
}

async function waitForActiveSyntheticDemand(
  request: APIRequestContext,
  expected: number,
): Promise<void> {
  await expect
    .poll(async () => (await syntheticState(request)).active_consumer_count, {
      timeout: 5_000,
      intervals: [50, 100, 250, 500],
    })
    .toBe(expected);
}

test.describe("provider-free Stock Beta intraday integration", () => {
  test.beforeEach(async ({ page }) => {
    observeBrowserRequests(page);
    await installProviderFreeNetworkGuard(page);
  });

  test.afterEach(async ({ page }) => {
    assertProviderFree(page);
  });

  test("renders the signed current quote beside an unchanged EOD chart on dashboard and detail", async ({
    page,
    request,
  }, testInfo) => {
    const widget = await openDashboard(page, request, ownerScenario({ stockBetaRows: 1 }));
    await expectReadyQuote(widget, quoteA);
    await expect(widget).toContainText("101,200.00");
    await expect(widget).toContainText("100,000");
    await expect(widget).toContainText("+1,200");
    await expect(widget).toContainText("+1.20%");
    await expect(widget.getByText("Up", { exact: true })).toBeVisible();
    await expect(widget).toContainText("Not previous close");
    const timestamp = await widget
      .locator("[data-last-success-at]")
      .getAttribute("data-last-success-at");
    expect(timestamp).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z$/);
    expect(Number.isFinite(Date.parse(timestamp ?? ""))).toBe(true);

    const chart = page.getByTestId("stock-beta-price-chart");
    await expect(chart).toBeVisible();
    await expect(chart.locator("g[data-candle-index]")).toHaveCount(261);
    await expect(page.getByRole("tabpanel", { name: "Price" })).toContainText("12120");
    const browserChartRequestsBefore = requestsFor(page).filter((url) =>
      url.pathname.endsWith("/chart"),
    );
    await widget.screenshot({
      animations: "disabled",
      path: testInfo.outputPath("stock-beta-intraday-dashboard.png"),
    });

    await expect
      .poll(async () => (await syntheticState(request)).quote_gets, {
        timeout: 7_000,
        intervals: [100, 250, 500, 1_000],
      })
      .toBeGreaterThan(1);
    expect(requestsFor(page).filter((url) => url.pathname.endsWith("/chart"))).toHaveLength(
      browserChartRequestsBefore.length,
    );
    await expect(widget.locator("[data-quote-value]")).toHaveAttribute("data-quote-value", quoteA);

    await rankedRow(page, "000001.KRX").getByRole("link", { name: "Open detail" }).click();
    await expect(page).toHaveURL(/\/stock-beta\/000001\.KRX$/);
    const detailWidget = detailQuoteWidget(page);
    await expect(detailWidget).toHaveCount(1);
    await expectReadyQuote(detailWidget, quoteA);
    await expect(page.getByTestId("stock-beta-detail-returns")).toContainText("Returns");
    await expectReadableDetailQuote(page);
    await detailWidget.screenshot({
      animations: "disabled",
      path: testInfo.outputPath("stock-beta-intraday-detail.png"),
    });
    for (const width of [900, 390]) {
      await page.setViewportSize({ height: 900, width });
      await expectReadableDetailQuote(page);
      const receipt = await detailWidget
        .locator("[data-last-success-at]")
        .getAttribute("data-last-success-at");
      expect(Number.isFinite(Date.parse(receipt ?? ""))).toBe(true);
      await detailWidget.screenshot({
        animations: "disabled",
        path: testInfo.outputPath(`stock-beta-intraday-detail-${width}.png`),
      });
    }
    await page.setViewportSize({ height: 900, width: 1280 });
  });

  test("switches identity and discards a late A response after B is selected", async ({
    page,
    request,
  }, testInfo) => {
    const observed = requestsFor(page);
    const lateARequest = page.waitForRequest(
      (candidate) =>
        candidate.method() === "GET" &&
        candidate.url().includes(`${quotePathFragment}000001.KRX/quote`),
      { timeout: 15_000 },
    );
    const widget = await openDashboard(
      page,
      request,
      ownerScenario({ stockBetaIntradayDelays: { "000001.KRX": 1_200 } }),
    );
    await lateARequest;
    await rankedRow(page, "000002.KRX")
      .getByRole("button", { name: "Select signal: 000002.KRX" })
      .click();
    const rowB = rankedRow(page, "000002.KRX");
    await expect(rowB).toHaveAttribute("data-selected", "true");
    await expectReadyQuote(widget, quoteB);
    await expect(widget).toContainText("-1,600");
    await expect(widget.getByText("Down", { exact: true })).toBeVisible();
    await page.waitForTimeout(1_400);
    await expect(widget.locator("[data-quote-value]")).toHaveAttribute("data-quote-value", quoteB);
    await expect(widget.locator(`[data-quote-value="${quoteA}"]`)).toHaveCount(0);
    await expect
      .poll(async () => (await syntheticState(request)).active_identity_count, {
        timeout: 5_000,
        intervals: [50, 100, 250, 500],
      })
      .toBe(1);
    expect(
      observed.some((url) => url.pathname.includes(`${quotePathFragment}000001.KRX/quote`)),
    ).toBe(true);
    await widget.screenshot({
      animations: "disabled",
      path: testInfo.outputPath("stock-beta-intraday-switch-b.png"),
    });
  });

  test("adds 005930 to READY, disables it, and re-registers the same instrument with a new identity", async ({
    page,
    request,
  }, testInfo) => {
    await resetScenario(request, ownerScenario({ stockBetaRows: 0, stockBetaSeed: "empty" }));
    await page.goto("/stock-beta");
    await expectStockShell(page);
    await expect(dashboardQuoteWidget(page)).toHaveCount(0);
    const widget = dashboardQuoteWidget(page);
    await page.getByLabel("KRX code").fill("005930");
    await page.getByRole("button", { name: "Add instrument" }).click();
    const firstCard = membershipCard(page, "005930.KRX");
    await expect(firstCard).toHaveAttribute("data-lifecycle", "READY", { timeout: 20_000 });
    await expect(rankedRow(page, "005930.KRX")).toBeVisible({ timeout: 20_000 });
    await expectReadyQuote(widget, "106030");
    const firstMembership = (await syntheticMemberships(request)).find(
      (membership) => membership.instrument_id === "005930.KRX" && membership.lifecycle === "READY",
    );
    if (firstMembership === undefined) throw new Error("initial 005930 membership was not READY");
    const firstState = await syntheticState(request);
    const firstDemand = firstState.created_demands.find(
      (demand) => demand.membership_id === firstMembership.id,
    );
    if (firstDemand === undefined) throw new Error("initial 005930 demand was not created");

    await firstCard.getByRole("button", { name: "Disable" }).click();
    await firstCard.getByRole("button", { name: "Confirm disable" }).click();
    await expect(firstCard).toHaveAttribute("data-lifecycle", "DISABLED", { timeout: 20_000 });
    await expect(widget.getByTestId("stock-beta-current-quote")).toHaveCount(0, {
      timeout: 20_000,
    });
    await waitForNoActiveSyntheticDemand(request);

    const oldDemandResponse = await request.post(`${syntheticOrigin}${demandPath}`, {
      data: {
        consumer_id: "00000000-0000-4000-8000-000000000702",
        generation: firstMembership.generation,
        membership_id: firstMembership.id,
        renewal_sequence: 0,
        schema_version: 1,
      },
      headers: { "x-csrf-token": "synthetic", "idempotency-key": "old-demand-rejected" },
    });
    expect(oldDemandResponse.status()).toBe(404);
    const oldQuoteResponse = await request.get(
      `${syntheticOrigin}${quotePathFragment}005930.KRX/quote?membership_id=${firstMembership.id}&generation=${firstMembership.generation}`,
    );
    expect(oldQuoteResponse.status()).toBe(404);

    await page.getByLabel("KRX code").fill("005930");
    await page.getByRole("button", { name: "Add instrument" }).click();
    const duplicateCards = membershipCards(page).filter({ hasText: "005930.KRX" });
    await expect(duplicateCards).toHaveCount(2, { timeout: 20_000 });
    const secondCard = membershipCard(page, "005930.KRX", "READY");
    await expect(secondCard).toHaveAttribute("data-lifecycle", "READY", { timeout: 20_000 });
    await expect(rankedRow(page, "005930.KRX")).toBeVisible({ timeout: 20_000 });
    await expectReadyQuote(widget, "106030");
    const membershipsAfterReadd = await syntheticMemberships(request);
    const disabledMembership = membershipsAfterReadd.find(
      (membership) => membership.id === firstMembership.id,
    );
    const replacementMembership = membershipsAfterReadd.find(
      (membership) =>
        membership.instrument_id === "005930.KRX" && membership.id !== firstMembership.id,
    );
    if (disabledMembership === undefined || replacementMembership === undefined) {
      throw new Error("re-registration did not retain the old membership and create a replacement");
    }
    expect(disabledMembership.lifecycle).toBe("DISABLED");
    expect(replacementMembership.lifecycle).toBe("READY");
    expect(replacementMembership.id).not.toBe(firstMembership.id);
    expect(replacementMembership.generation).toBe(firstMembership.generation);
    const readdState = await syntheticState(request);
    const replacementDemand = readdState.created_demands.find(
      (demand) => demand.membership_id === replacementMembership.id,
    );
    if (replacementDemand === undefined)
      throw new Error("replacement 005930 demand was not created");
    expect(replacementDemand.consumer_id).not.toBe(firstDemand.consumer_id);
    expect(replacementDemand.demand_id).not.toBe(firstDemand.demand_id);
    expect(readdState.released_demand_ids).toContain(firstDemand.demand_id);
    expect(readdState.rejected_demand_membership_ids).toContain(firstMembership.id);
    expect(readdState.rejected_quote_identities).toContainEqual({
      generation: firstMembership.generation,
      instrument_id: "005930.KRX",
      membership_id: firstMembership.id,
    });
    await expect
      .poll(async () => (await syntheticState(request)).demand_posts, {
        timeout: 5_000,
        intervals: [50, 100, 250, 500],
      })
      .toBeGreaterThanOrEqual(2);
    await widget.screenshot({
      animations: "disabled",
      path: testInfo.outputPath("stock-beta-intraday-membership-ready.png"),
    });
  });

  test("keeps multi-tab consumers and identity demand isolated with typed capacity state", async ({
    browser,
    request,
  }) => {
    await resetScenario(request, ownerScenario());
    const firstContext = await browser.newContext({ baseURL: appOrigin });
    const secondContext = await browser.newContext({ baseURL: appOrigin });
    const firstPage = await firstContext.newPage();
    const secondPage = await secondContext.newPage();
    observeBrowserRequests(firstPage);
    observeBrowserRequests(secondPage);
    let state: SyntheticState;
    try {
      await installProviderFreeNetworkGuard(firstPage);
      await installProviderFreeNetworkGuard(secondPage);
      await firstPage.goto("/stock-beta");
      await secondPage.goto("/stock-beta");
      await expectReadyQuote(dashboardQuoteWidget(firstPage), quoteA);
      await expectReadyQuote(dashboardQuoteWidget(secondPage), quoteA);
      state = await syntheticState(request);
      expect(state.active_consumer_count).toBe(2);
      expect(state.active_identity_count).toBe(1);
      expect(state.quote_gets_by_instrument["000001.KRX"]).toBeGreaterThanOrEqual(2);

      await rankedRow(secondPage, "000002.KRX")
        .getByRole("button", { name: "Select signal: 000002.KRX" })
        .click();
      await expectReadyQuote(dashboardQuoteWidget(secondPage), quoteB);
      await expect
        .poll(async () => (await syntheticState(request)).active_identity_count, {
          timeout: 5_000,
          intervals: [50, 100, 250, 500],
        })
        .toBe(2);
      state = await syntheticState(request);
      expect(state.quote_gets_by_instrument["000001.KRX"]).toBeGreaterThanOrEqual(1);
      expect(state.quote_gets_by_instrument["000002.KRX"]).toBeGreaterThanOrEqual(1);
      expect(state.max_active_identity_count).toBe(2);
      await setVisibility(firstPage, "hidden");
      await waitForActiveSyntheticDemand(request, 1);
      await stopSyntheticDemand(secondPage, request);
    } finally {
      await firstContext.close();
      await secondContext.close();
      assertProviderFree(firstPage);
      assertProviderFree(secondPage);
    }

    await waitForNoActiveSyntheticDemand(request);
    await resetScenario(request, ownerScenario({ stockBetaIntradayFailure: "capacity-after-two" }));
    const capContexts = await Promise.all(
      Array.from({ length: 3 }, () => browser.newContext({ baseURL: appOrigin })),
    );
    const capPages = await Promise.all(capContexts.map((context) => context.newPage()));
    for (const page of capPages) observeBrowserRequests(page);
    try {
      for (const page of capPages) await installProviderFreeNetworkGuard(page);
      for (const page of capPages) await page.goto("/stock-beta");
      for (const page of capPages) {
        await expect(page.getByTestId("stock-beta-widget-current-quote")).toHaveCount(1);
      }
      const capThirdPage = capPages[2];
      if (capThirdPage === undefined)
        throw new Error("capacity scenario did not create three pages");
      await expect(capThirdPage.getByTestId("stock-beta-current-quote")).toHaveCount(0, {
        timeout: 5_000,
      });
      await expect(
        capThirdPage.getByTestId("stock-beta-widget-current-quote").locator("[data-status-phase]"),
      ).toHaveAttribute("data-status-phase", "unavailable");
      state = await syntheticState(request);
      expect(state.demand_posts).toBeGreaterThanOrEqual(3);
      expect(state.active_consumer_count).toBe(2);
      expect(state.active_identity_count).toBe(1);
      for (const page of capPages) await setVisibility(page, "hidden");
      await waitForNoActiveSyntheticDemand(request);
    } finally {
      for (const context of capContexts) await context.close();
      for (const page of capPages) assertProviderFree(page);
    }
  });

  test("stops on hidden/offline state, reconnects with a new demand, and stops on logout notification", async ({
    page,
    request,
  }) => {
    test.setTimeout(60_000);
    const widget = await openDashboard(page, request, ownerScenario({ stockBetaRows: 1 }));
    await expectReadyQuote(widget, quoteA);

    await setVisibility(page, "hidden");
    await expect(widget.getByTestId("stock-beta-current-quote")).toHaveCount(0);
    await waitForNoActiveSyntheticDemand(request);
    const hiddenState = await syntheticState(request);
    await page.waitForTimeout(6_200);
    expect((await syntheticState(request)).quote_gets).toBe(hiddenState.quote_gets);

    // Hidden already released the old session. Unmount a new ACTIVE session instead.
    await setVisibility(page, "visible");
    await expectReadyQuote(widget, quoteA);
    await waitForActiveSyntheticDemand(request, 1);
    const beforeUnmount = await syntheticState(request);
    // Next Link navigation exercises React cleanup; a document unload need not run effects.
    await page
      .getByRole("navigation", { name: "Primary" })
      .getByRole("link", { name: "Strategies", exact: true })
      .click();
    await expect(page).toHaveURL(/\/strategies$/);
    await expect(widget).toHaveCount(0);
    await waitForNoActiveSyntheticDemand(request);
    const afterUnmount = await syntheticState(request);
    expect(afterUnmount.demand_releases).toBeGreaterThan(beforeUnmount.demand_releases);
    await page.waitForTimeout(6_200);
    expect((await syntheticState(request)).quote_gets).toBe(afterUnmount.quote_gets);

    await page.goto("/stock-beta");
    await expectStockShell(page);
    const remountedWidget = dashboardQuoteWidget(page);
    await expect(remountedWidget).toHaveCount(1);
    await expectReadyQuote(remountedWidget, quoteA);
    await setVisibility(page, "visible");
    await expectReadyQuote(remountedWidget, quoteA);
    await setOnline(page, false);
    await expect(remountedWidget.getByTestId("stock-beta-current-quote")).toHaveCount(0);
    await expect(quoteStatus(remountedWidget)).toHaveAttribute("data-status-phase", "offline");
    await waitForNoActiveSyntheticDemand(request);
    const offlineState = await syntheticState(request);
    await page.waitForTimeout(6_200);
    expect((await syntheticState(request)).quote_gets).toBe(offlineState.quote_gets);

    await setOnline(page, true);
    await expectReadyQuote(remountedWidget, quoteA);
    await page.getByRole("button", { name: "Sign out" }).click();
    await expect(remountedWidget.getByTestId("stock-beta-current-quote")).toHaveCount(0);
    await expect
      .poll(async () => (await syntheticState(request)).active_consumer_count, {
        timeout: 5_000,
        intervals: [50, 100, 250, 500],
      })
      .toBe(0);
    await expect(page.getByRole("button", { name: "Sign out" })).toBeVisible();
    expect(requestsFor(page).some((url) => url.pathname === "/api/v1/auth/logout")).toBe(true);
  });

  test("advances one instrument's price, signed change, version, and receipt time", async ({
    page,
    request,
  }, testInfo) => {
    test.setTimeout(25_000);
    const widget = await openDashboard(
      page,
      request,
      ownerScenario({ stockBetaRows: 1, stockBetaIntradayState: "advancing" }),
    );
    await expectReadyQuote(widget, quoteA);
    const quote = widget.getByTestId("stock-beta-current-quote");
    const price = quote.locator("[data-quote-value]");
    const change = quote.locator("dd[data-direction]").first();
    const firstPrice = await price.getAttribute("data-quote-value");
    const firstChange = await change.textContent();
    const firstTimestamp = await quote
      .locator("[data-last-success-at]")
      .getAttribute("data-last-success-at");
    const firstState = await syntheticState(request);
    const nextQuoteResponse = page.waitForResponse(
      (response) =>
        response.request().method() === "GET" && response.url().includes(quotePathFragment),
      { timeout: 15_000 },
    );

    await expect
      .poll(async () => (await syntheticState(request)).quote_gets, {
        timeout: 15_000,
        intervals: [100, 250, 500, 1_000],
      })
      .toBeGreaterThan(firstState.quote_gets);
    const response = await nextQuoteResponse;
    const body = (await response.json()) as {
      readonly quote: {
        readonly change_from_previous_day: string;
        readonly price: string;
        readonly quote_version: string;
        readonly last_success_at: string;
      } | null;
    };
    expect(body.quote).toMatchObject({
      change_from_previous_day: "1300",
      price: "101300.00",
      quote_version: "2",
    });
    if (body.quote === null) throw new Error("advancing fixture omitted its second quote");
    expect(Date.parse(body.quote.last_success_at)).toBeGreaterThan(
      Date.parse(firstTimestamp ?? ""),
    );
    await expect
      .poll(async () => price.getAttribute("data-quote-value"), {
        timeout: 5_000,
        intervals: [50, 100, 250, 500],
      })
      .toBe("101300.00");
    await expect
      .poll(async () => change.textContent(), {
        timeout: 5_000,
        intervals: [50, 100, 250, 500],
      })
      .toBe("+1,300");
    const secondTimestamp = await quote
      .locator("[data-last-success-at]")
      .getAttribute("data-last-success-at");
    expect(firstPrice).toBe(quoteA);
    expect(firstChange).toBe("+1,200");
    expect(secondTimestamp).not.toBe(firstTimestamp);
    await widget.screenshot({
      animations: "disabled",
      path: testInfo.outputPath("stock-beta-intraday-advancing.png"),
    });
  });

  test("retains the last good quote across bounded zero and invalid updates", async ({
    page,
    request,
  }) => {
    test.setTimeout(30_000);
    const widget = await openDashboard(
      page,
      request,
      ownerScenario({
        stockBetaRows: 1,
        stockBetaIntradayQuoteSequence: ["open", "zero", "invalid"],
      }),
    );
    await expectReadyQuote(widget, quoteA);
    const quote = widget.getByTestId("stock-beta-current-quote");
    const lastGoodTimestamp = await quote
      .locator("[data-last-success-at]")
      .getAttribute("data-last-success-at");
    expect(lastGoodTimestamp).not.toBeNull();
    const firstState = await syntheticState(request);
    for (let attempt = 0; attempt < 2; attempt += 1) {
      await expect
        .poll(async () => (await syntheticState(request)).quote_gets, {
          timeout: 15_000,
          intervals: [100, 250, 500, 1_000],
        })
        .toBeGreaterThan(firstState.quote_gets + attempt);
      await expect(quote.locator('[data-quote-value="101200.00"]')).toHaveCount(1);
      await expect(quote.locator("[data-last-success-at]")).toHaveAttribute(
        "data-last-success-at",
        lastGoodTimestamp ?? "",
      );
      await expect(quote.locator('[data-quote-value="101300.00"]')).toHaveCount(0);
    }
    // WidgetFrame renders status beside quoteContent, not inside its data-testid node.
    await expect(quoteStatus(widget)).toHaveAttribute("data-status-phase", "stale");
  });

  test("renders closed, halted, stale, unknown-calendar, and typed failure states", async ({
    page,
    request,
  }) => {
    test.setTimeout(90_000);
    const cases = [
      { state: "closed", marketState: "CLOSED", text: "Market closed", hasQuote: false },
      { state: "halted", marketState: "HALTED", text: "Instrument halted", hasQuote: true },
      { state: "stale", marketState: "OPEN", text: "Quote is stale", hasQuote: true },
      {
        state: "unknown-calendar",
        marketState: "UNKNOWN",
        text: "Market state unknown",
        hasQuote: false,
      },
      {
        state: "session-window-unknown",
        marketState: "UNKNOWN",
        text: "Market state unknown",
        hasQuote: false,
      },
      {
        state: "rate-limited",
        marketState: "OPEN",
        text: "Current quote is unavailable",
        hasQuote: false,
      },
      {
        state: "provider-timeout",
        marketState: "OPEN",
        text: "Current quote is unavailable",
        hasQuote: false,
      },
      { state: "zero", marketState: null, text: "Current quote is unavailable", hasQuote: false },
      {
        state: "invalid",
        marketState: null,
        text: "Current quote is unavailable",
        hasQuote: false,
      },
      {
        state: "future-timestamp",
        marketState: null,
        text: "Current quote is unavailable",
        hasQuote: false,
      },
      {
        state: "prior-session-after-rollover",
        marketState: null,
        text: "Current quote is unavailable",
        hasQuote: false,
      },
    ] as const;

    for (const [index, scenarioCase] of cases.entries()) {
      if (index > 0) {
        await stopSyntheticDemand(page, request);
        await page.goto("about:blank");
      }
      const rawQuoteResponse =
        scenarioCase.state === "provider-timeout" ||
        scenarioCase.state === "future-timestamp" ||
        scenarioCase.state === "prior-session-after-rollover"
          ? page.waitForResponse(
              (response) =>
                response.request().method() === "GET" && response.url().includes(quotePathFragment),
              { timeout: 15_000 },
            )
          : null;
      const widget = await openDashboard(
        page,
        request,
        ownerScenario({
          stockBetaIntradayState: scenarioCase.state,
          stockBetaIntradayFailure: undefined,
        }),
      );
      if (rawQuoteResponse !== null) {
        const response = await rawQuoteResponse;
        expect(response.status()).toBe(200);
        const body = (await response.json()) as {
          readonly quote: {
            readonly last_success_at: string;
          } | null;
          readonly reason_code: string | null;
          readonly session: { readonly date: string } | null;
        };
        if (scenarioCase.state === "provider-timeout") {
          expect(body.reason_code).toBe("PROVIDER_TIMEOUT");
          expect(body.quote).toBeNull();
        } else {
          if (body.quote === null || body.session === null) {
            throw new Error(`${scenarioCase.state} fixture omitted its invalid quote evidence`);
          }
          const receivedAt = Date.parse(body.quote.last_success_at);
          expect(Number.isFinite(receivedAt)).toBe(true);
          if (scenarioCase.state === "future-timestamp") {
            expect(receivedAt).toBeGreaterThan(Date.now());
          } else {
            expect(receivedAt).toBeLessThanOrEqual(Date.now());
            expect(body.session.date).toBe(kstDate(receivedAt));
            expect(body.session.date).not.toBe(kstDate(Date.now()));
          }
        }
      }
      await expect(quoteStatus(widget)).toContainText(scenarioCase.text);
      if (scenarioCase.marketState === null) {
        await expect(quoteStatus(widget)).not.toHaveAttribute("data-market-state", /.+/);
      } else {
        await expect(quoteStatus(widget)).toHaveAttribute(
          "data-market-state",
          scenarioCase.marketState,
        );
      }
      if (scenarioCase.hasQuote) {
        await expect(widget.getByTestId("stock-beta-current-quote")).toBeVisible();
      } else {
        await expect(widget.getByTestId("stock-beta-current-quote")).toHaveCount(0);
      }
    }

    await stopSyntheticDemand(page, request);
    await page.goto("about:blank");
    for (const failure of ["503", "capacity"] as const) {
      await resetScenario(request, ownerScenario({ stockBetaIntradayFailure: failure }));
      await page.goto("/stock-beta");
      const widget = dashboardQuoteWidget(page);
      await expect(widget.getByTestId("stock-beta-current-quote")).toHaveCount(0);
      await expect(quoteStatus(widget)).toContainText("Current quote is unavailable");
      await stopSyntheticDemand(page, request);
      await page.goto("about:blank");
    }
  });

  test("keeps quote catalog placement independent across dashboard and detail", async ({
    page,
    request,
  }) => {
    await openDashboard(page, request, ownerScenario());
    const dashboardIds = await page
      .locator('[data-testid^="stock-beta-widget-"][data-widget-id]')
      .evaluateAll((elements) => elements.map((element) => element.getAttribute("data-widget-id")));
    expect(dashboardIds.at(-1)).toBe("current-quote");
    const dashboardQuote = dashboardQuoteWidget(page);
    for (const breakpoint of ["desktop", "tablet", "mobile"] as const) {
      await expect(dashboardQuote).toHaveAttribute(`data-${breakpoint}-visible`, "true");
    }
    await page.setViewportSize({ width: 375, height: 800 });
    await expect(dashboardQuote).toBeVisible();

    await rankedRow(page, "000001.KRX").getByRole("link", { name: "Open detail" }).click();
    await expect(page).toHaveURL(/\/stock-beta\/000001\.KRX$/, { timeout: 15_000 });
    await expect(page.getByTestId("stock-beta-detail-board")).toBeVisible();
    await expect(detailQuoteWidget(page)).toBeVisible();
    const detailIds = await page
      .locator('[data-testid^="stock-beta-detail-widget-"][data-widget-id]')
      .evaluateAll((elements) => elements.map((element) => element.getAttribute("data-widget-id")));
    expect(detailIds.at(-1)).toBe("current-quote");
    await expect(page.getByTestId("stock-beta-detail-returns")).toBeVisible();
  });

  test("repeats cache-only app GETs 1, 10, and 100 times without demand mutations", async ({
    request,
  }) => {
    await resetScenario(request, ownerScenario({ stockBetaRows: 1 }));
    const path = `${appOrigin}${quotePathFragment}000001.KRX/quote?membership_id=${membershipA}&generation=1`;
    let expectedGets = 0;
    for (const count of [1, 10, 100]) {
      const before = await syntheticState(request);
      for (let index = 0; index < count; index += 1) {
        const response = await request.get(path);
        expect(response.status()).toBe(200);
      }
      expectedGets += count;
      const after = await syntheticState(request);
      expect(after.quote_gets).toBe(expectedGets);
      expect(after.demand_posts).toBe(before.demand_posts);
      expect(after.demand_renewals).toBe(before.demand_renewals);
      expect(after.demand_releases).toBe(before.demand_releases);
    }
    expect((await syntheticState(request)).active_consumer_count).toBe(0);
  });
});
