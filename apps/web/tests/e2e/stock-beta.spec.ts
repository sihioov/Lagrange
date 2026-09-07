import { type APIRequestContext, expect, type Locator, type Page, test } from "@playwright/test";

const appOrigin = process.env["PLAYWRIGHT_BASE_URL"] ?? "http://127.0.0.1:33000";
const syntheticOrigin = process.env["SYNTHETIC_API_ORIGIN"] ?? "http://127.0.0.1:38180";
const membershipsPath = "/api/v1/research/owner-beta/equity-universe-v2/memberships";
const chartPathFragment = "/api/v1/research/owner-beta/equity-universe-v2/signals/instruments/";
const chartAsOf = "2026-08-28";
const chartFirstSession = "2025-08-29";
const chartLatestClose = "12120";
const chartLatestVolume = "106000";
const chartLatestChange = "+75";
const chartLatestRate = "0.62%";
const uuidPattern = /[0-9a-f]{8}-[0-9a-f]{4}-[4-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}/i;
const forbiddenRoutePattern =
  /(?:^|\/)(?:account|accounts|balance|buying-power|sellable-quantity|execution-history|order|orders|live)(?:\/|$)/i;

const observedBrowserRequests = new WeakMap<Page, URL[]>();

async function resetScenario(request: APIRequestContext, scenario: Record<string, unknown>) {
  const response = await request.post(`${syntheticOrigin}/__test/scenario`, { data: scenario });
  expect(response.ok()).toBeTruthy();
}

function observeBrowserRequests(page: Page) {
  const requests: URL[] = [];
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.protocol === "http:" || url.protocol === "https:") requests.push(url);
  });
  observedBrowserRequests.set(page, requests);
  return requests;
}

function isLocalTestUrl(url: URL) {
  const expectedPorts = new Set([new URL(appOrigin).port, new URL(syntheticOrigin).port]);
  return ["127.0.0.1", "localhost"].includes(url.hostname) && expectedPorts.has(url.port);
}

async function installProviderFreeNetworkGuard(page: Page) {
  await page.route("**/*", async (route) => {
    const url = new URL(route.request().url());
    if (isLocalTestUrl(url)) {
      await route.continue();
    } else {
      await route.abort("blockedbyclient");
    }
  });
}

function expectProviderFree(requests: URL[]) {
  const externalUrls = requests.filter((url) => !isLocalTestUrl(url)).map((url) => url.href);
  expect(externalUrls, `External browser requests: ${externalUrls.join(", ") || "none"}`).toEqual(
    [],
  );
  expect(
    requests.filter((url) => /kis|opendart/i.test(`${url.hostname}${url.pathname}`)),
  ).toHaveLength(0);
  expect(requests.filter((url) => url.pathname.includes("equity-price-signals"))).toHaveLength(0);
  const forbiddenApiUrls = requests
    .filter((url) => url.pathname.startsWith("/api/"))
    .filter((url) => forbiddenRoutePattern.test(url.pathname))
    .map((url) => url.href);
  expect(
    forbiddenApiUrls,
    `Forbidden API requests: ${forbiddenApiUrls.join(", ") || "none"}`,
  ).toEqual([]);
}

function requestsFor(page: Page) {
  return observedBrowserRequests.get(page) ?? [];
}

function chartRequests(requests: URL[], instrumentId?: string, range?: string) {
  return requests.filter((url) => {
    if (!url.pathname.includes(chartPathFragment) || !url.pathname.endsWith("/chart")) return false;
    if (instrumentId && !url.pathname.includes(encodeURIComponent(instrumentId))) return false;
    if (range && url.searchParams.get("range") !== range) return false;
    return true;
  });
}

function snapshotIdFromText(text: string | null) {
  const snapshotId = text?.match(uuidPattern)?.[0];
  if (!snapshotId) throw new Error("Expected the snapshot strip to contain a snapshot id");
  return snapshotId;
}

function observeMembershipPostCount(page: Page) {
  let count = 0;
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (request.method() === "POST" && url.pathname === membershipsPath) count += 1;
  });
  return () => count;
}

function membershipRegion(page: Page) {
  return page.getByRole("region", { name: "Membership status" });
}

function universeManagementRegion(page: Page) {
  return page.getByRole("region", { name: "V2 universe management" });
}

function membershipCards(page: Page) {
  return membershipRegion(page).getByTestId("stock-beta-membership-card");
}

function membershipCard(page: Page, instrumentId: string) {
  return membershipCards(page).filter({ hasText: instrumentId });
}

function stockBetaRegion(page: Page) {
  return page.getByRole("region", { name: "Stock signal beta" });
}

function snapshotStrip(page: Page) {
  return stockBetaRegion(page).getByTestId("stock-beta-snapshot-strip");
}

function rankedSignalsRegion(page: Page) {
  return page.getByRole("region", { name: "Ranked signals" });
}

function rankedSignalsTable(page: Page) {
  return rankedSignalsRegion(page).getByRole("table", {
    name: /V2 ranked price-and-volume signal table/,
  });
}

function signalProfileRegion(page: Page) {
  return page.getByRole("region", { name: "Selected signal profile" });
}

function signalProfilePanel(page: Page, name = "Price") {
  return signalProfileRegion(page).getByRole("tabpanel", { name });
}

function detailInstrumentRegion(page: Page, instrumentId: string) {
  return page.getByRole("region", { name: instrumentId, exact: true });
}

function rankedRow(page: Page, instrumentId: string) {
  return rankedSignalsTable(page).getByTestId(`stock-beta-row-${instrumentId}`);
}

function signalPreview(page: Page) {
  return signalProfileRegion(page).getByTestId("stock-beta-signal-preview");
}

function priceChart(page: Page) {
  return signalProfilePanel(page).getByTestId("stock-beta-price-chart");
}

function priceChartPart(page: Page, testId: string) {
  return priceChart(page).getByTestId(testId);
}

function dashboardRegion(page: Page, name: string) {
  return page.getByRole("region", { name });
}

async function expectNoSignalWidgets(page: Page) {
  for (const testId of [
    "stock-beta-snapshot-strip",
    "stock-beta-rank-table",
    "stock-beta-signal-preview",
    "stock-beta-signal-decomposition",
    "stock-beta-condition-matrix",
    "stock-beta-snapshot-tape",
  ]) {
    await expect(page.getByTestId(testId)).toHaveCount(0);
  }
}

async function expectStockShell(page: Page) {
  await expect(page.locator('[data-shell="stock-beta-terminal"]')).toHaveCount(1);
  await expect(page.locator('[data-terminal-utility-bar="stock-beta"]')).toHaveCount(1);
  await expect(page.locator("main")).toHaveCount(1);
}

async function expectNoHorizontalOverflow(page: Page) {
  await expect
    .poll(() => page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth))
    .toBeTruthy();
}

async function box(locator: Locator) {
  const result = await locator.boundingBox();
  if (result === null) {
    throw new Error("Expected the locator to have a bounding box");
  }
  return result;
}

function expectBoxInside(
  inner: { x: number; y: number; width: number; height: number },
  outer: {
    x: number;
    y: number;
    width: number;
    height: number;
  },
) {
  expect(inner.x).toBeGreaterThanOrEqual(outer.x);
  expect(inner.y).toBeGreaterThanOrEqual(outer.y);
  expect(inner.x + inner.width).toBeLessThanOrEqual(outer.x + outer.width);
  expect(inner.y + inner.height).toBeLessThanOrEqual(outer.y + outer.height);
}

test.describe("provider-free Stock Beta V2", () => {
  test.beforeEach(async ({ page }) => {
    observeBrowserRequests(page);
    await installProviderFreeNetworkGuard(page);
  });

  test.afterEach(async ({ page }) => {
    expectProviderFree(requestsFor(page));
  });

  test("renders an empty owner capacity with no signal rows", async ({ page, request }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 0,
      stockBetaSeed: "empty",
    });

    await page.goto("/stock-beta");
    await expectStockShell(page);
    const membershipEmpty = page
      .getByRole("region", { name: "Membership status" })
      .getByText(/No research instruments are configured\./);
    await expect(membershipEmpty).toHaveCount(1);
    await expect(membershipEmpty).toBeVisible();
    await expect(
      universeManagementRegion(page).getByTestId("stock-beta-policy-capacity"),
    ).toContainText("100");
    await expect(page.getByRole("heading", { name: "Signal snapshot unavailable" })).toBeVisible();
    await expectNoSignalWidgets(page);
    await expect(page.getByLabel("KRX code")).toBeVisible();
    await expect(page.locator('[data-terminal-utility-content="stock-beta"]')).toHaveCount(0);
  });

  test("renders a typed V2 snapshot with zero rows", async ({ page, request }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 0,
      stockBetaSeed: "empty",
      stockBetaSnapshot: "empty",
    });

    await page.goto("/stock-beta");
    await expectStockShell(page);
    const terminalSnapshot = snapshotStrip(page);
    await expect(terminalSnapshot).toBeVisible();
    await expect(terminalSnapshot.getByTestId("stock-beta-snapshot-universe")).toContainText("0");
    await expect(
      rankedSignalsRegion(page).getByText("The current V2 snapshot has no signal rows."),
    ).toBeVisible();
    await expect(page.getByTestId("stock-beta-rank-table")).toHaveCount(0);
    await expectNoHorizontalOverflow(page);
  });

  test("renders exactly 31 V2 memberships and ranked rows", async ({ page, request }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 31,
      stockBetaSeed: "ready",
    });

    await page.goto("/stock-beta");
    await expect(membershipCards(page)).toHaveCount(31);
    await expect(
      page.getByRole("table", { name: /V2 ranked price-and-volume signal table/ }),
    ).toBeVisible();
    await expect(rankedSignalsTable(page).locator("tbody tr")).toHaveCount(31);
    await expect(rankedRow(page, "000031.KRX")).toBeVisible();
    await expect(rankedRow(page, "000001.KRX")).toHaveAttribute("data-top-five", "true");
    await expect(page.locator('[data-top-five="true"]')).toHaveCount(5);
    await expect(snapshotStrip(page).getByTestId("stock-beta-snapshot-universe")).toContainText(
      "31",
    );
    await expectNoHorizontalOverflow(page);
  });

  test("renders exactly 100 rows and closes the capacity boundary", async ({ page, request }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 100,
      stockBetaSeed: "ready",
    });

    await page.goto("/stock-beta");
    await expect(membershipCards(page)).toHaveCount(100);
    await expect(rankedSignalsTable(page).locator("tbody tr")).toHaveCount(100);
    await expect(rankedRow(page, "000100.KRX")).toBeVisible();
    await expect(snapshotStrip(page).getByTestId("stock-beta-snapshot-universe")).toContainText(
      "100",
    );
    await expect(page.getByRole("button", { name: "Add instrument" })).toBeDisabled();
  });

  test("rejects every invalid six-digit shape without a membership POST", async ({
    page,
    request,
  }) => {
    const getMembershipPostCount = observeMembershipPostCount(page);
    const invalidCodes = [
      { name: "short five-digit input", value: "12345" },
      { name: "long seven-digit input", value: "1234567" },
      { name: "six digits plus a suffix", value: "123456X" },
      { name: "non-digit input", value: "ABCDEF" },
    ];
    const locales = [
      {
        addButton: "Add instrument",
        codeLabel: "KRX code",
        cookie: "en",
        message: "Enter exactly six ASCII digits.",
      },
      {
        addButton: "종목 추가",
        codeLabel: "KRX 코드",
        cookie: "ko",
        message: "ASCII 숫자 6자리를 정확히 입력하세요.",
      },
    ];

    for (const locale of locales) {
      await resetScenario(request, {
        authSession: "valid",
        role: "owner",
        stockBetaRows: 0,
        stockBetaSeed: "empty",
      });
      await page.context().addCookies([{ name: "locale", value: locale.cookie, url: appOrigin }]);
      await page.goto("/stock-beta");
      const input = page.getByLabel(locale.codeLabel);
      const addButton = page.getByRole("button", { name: locale.addButton });
      const validationMessage = page.locator('[role="alert"]').filter({ hasText: locale.message });

      for (const invalidCode of invalidCodes) {
        await input.fill(invalidCode.value);
        await expect(input).toHaveValue(invalidCode.value);
        await addButton.click();
        await expect(validationMessage).toBeVisible();
        await expect(input).toHaveValue(invalidCode.value);
        expect(getMembershipPostCount(), invalidCode.name).toBe(0);
        await input.fill("");
      }
    }
  });

  test("adds, polls to READY, refreshes the rank, and opens V2 detail", async ({
    page,
    request,
  }) => {
    const observed = observeBrowserRequests(page);
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 0,
      stockBetaSeed: "empty",
    });

    await page.goto("/stock-beta");
    await page.getByLabel("KRX code").fill("005930");
    await page.getByRole("button", { name: "Add instrument" }).click();

    const card = membershipCard(page, "005930.KRX");
    await expect(card).toHaveAttribute("data-lifecycle", "READY", { timeout: 20_000 });
    const row = rankedRow(page, "005930.KRX");
    await expect(row).toBeVisible({ timeout: 20_000 });
    await expect(snapshotStrip(page).getByTestId("stock-beta-snapshot-universe")).toContainText(
      "1",
      {
        timeout: 20_000,
      },
    );
    await expect(row.getByRole("link", { name: "Open detail" })).toBeVisible();
    await expect(observed.some((url) => url.pathname.includes("equity-universe-v2"))).toBeTruthy();
    expectProviderFree(observed);

    await row.getByRole("link", { name: "Open detail" }).click();
    await expect(page).toHaveURL(/\/stock-beta\/005930\.KRX$/);
    await expect(detailInstrumentRegion(page, "005930.KRX")).toBeVisible();
    await expectStockShell(page);
  });

  test("retries a typed failure and polls the membership to READY", async ({ page, request }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 31,
      stockBetaSeed: "failed",
    });

    await page.goto("/stock-beta");
    const card = membershipCard(page, "000001.KRX");
    await expect(card).toHaveAttribute("data-lifecycle", "FAILED");
    await expect(card).toContainText("OWNER_EQUITY_BACKFILL_RETRYABLE");
    await card.getByRole("button", { name: "Retry" }).click();
    await expect(card).toHaveAttribute("data-lifecycle", "READY", { timeout: 20_000 });
    await expect(card).not.toContainText("OWNER_EQUITY_BACKFILL_RETRYABLE");
    await expect(rankedRow(page, "000001.KRX")).toBeVisible({ timeout: 20_000 });
    await expect(snapshotStrip(page)).toBeVisible({ timeout: 20_000 });
  });

  test("removes a disabled row and stale snapshot signal immediately", async ({
    page,
    request,
  }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 31,
      stockBetaSeed: "ready",
    });

    await page.goto("/stock-beta");
    const card = membershipCard(page, "000001.KRX");
    const oldRow = rankedRow(page, "000001.KRX");
    await expect(oldRow).toBeVisible();
    await card.getByRole("button", { name: "Disable" }).click();
    const confirm = card.getByRole("button", { name: "Confirm disable" });
    await expect(confirm).toBeFocused();
    await confirm.click();

    await expect(oldRow).toHaveCount(0);
    await expect(page.getByTestId("stock-beta-snapshot-strip")).toHaveCount(0);
    await expect(card).toHaveAttribute("data-lifecycle", "DISABLED", { timeout: 20_000 });
    await expect(rankedRow(page, "000001.KRX")).toHaveCount(0);
    await expect(snapshotStrip(page).getByTestId("stock-beta-snapshot-universe")).toContainText(
      "30",
      {
        timeout: 20_000,
      },
    );
  });

  test("keeps typed latest-unavailable and integrity failures free of stale signals", async ({
    page,
    request,
  }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 31,
      stockBetaSeed: "ready",
      stockBeta: "unavailable",
    });
    await page.goto("/stock-beta");
    await expect(page.getByRole("heading", { name: "Signal snapshot unavailable" })).toBeVisible();
    await expectNoSignalWidgets(page);
    await expect(membershipCards(page)).toHaveCount(31);

    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 31,
      stockBetaSeed: "ready",
      stockBeta: "integrity",
    });
    await page.goto("/stock-beta");
    await expect(
      page.getByRole("heading", { name: "Signal snapshot integrity failed" }),
    ).toBeVisible();
    await expectNoSignalWidgets(page);
    await expect(page.getByText("000001.KRX")).toHaveCount(0);
  });

  test("renders a typed detail not-found state without signal data", async ({ page, request }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 31,
      stockBetaSeed: "ready",
    });

    await page.goto("/stock-beta/999999.KRX");
    await expectStockShell(page);
    await expect(page.getByRole("heading", { name: "Instrument signal not found" })).toBeVisible();
    await expect(page.getByTestId("stock-beta-detail-board")).toHaveCount(0);
    await expect(page.locator('[data-terminal-utility-content="stock-beta"]')).toHaveCount(0);
  });

  test("enforces owner-only redirect and forbidden boundaries", async ({ page, request }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "member",
      stockBetaRows: 31,
      stockBetaSeed: "ready",
    });

    await page.goto("/stock-beta");
    await expect(page.getByRole("alert", { name: "Owner access required" })).toBeVisible();
    await expect(page.getByTestId("stock-beta-rank-table")).toHaveCount(0);
    await page.goto("/stock-beta/000001.KRX");
    await expect(page.getByRole("alert", { name: "Owner access required" })).toBeVisible();
    await expect(page.getByTestId("stock-beta-detail-board")).toHaveCount(0);

    await resetScenario(request, {
      authSession: "expired",
      role: "owner",
      stockBetaRows: 31,
      stockBetaSeed: "ready",
    });
    await page.goto("/stock-beta");
    await expect.poll(() => new URL(page.url()).pathname, { timeout: 10_000 }).toBe("/auth/login");
  });

  test("keeps the terminal shell continuous through dashboard, detail, and navigation", async ({
    page,
    request,
  }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 31,
      stockBetaSeed: "ready",
    });

    await page.goto("/stock-beta");
    await expectStockShell(page);
    await rankedRow(page, "000001.KRX").getByRole("link", { name: "Open detail" }).click();
    await expect(detailInstrumentRegion(page, "000001.KRX")).toBeVisible();
    await expectStockShell(page);
    await page.getByRole("link", { name: "Back to stock signal beta" }).click();
    await expect(rankedSignalsTable(page)).toBeVisible();
    await expectStockShell(page);

    await page.goto("/strategies");
    await expect(page.locator('[data-shell="research-terminal"]')).toHaveCount(1);
    await expect(page.locator('[data-shell="stock-beta-terminal"]')).toHaveCount(0);
    await expect(page.locator('[data-terminal-utility-bar="research"]')).toHaveCount(1);
  });

  test("supports instrument search, slash/Escape, matrix selection, and keyboard activation", async ({
    page,
    request,
  }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 31,
      stockBetaSeed: "ready",
    });

    await page.goto("/stock-beta");
    const search = page.locator("#stock-beta-instrument-search-input");
    await expect(search).toHaveAccessibleName("Search signals");
    await page.locator("body").press("/");
    await expect(search).toBeFocused();
    await search.fill("000004");
    await expect(rankedRow(page, "000004.KRX")).toBeVisible();
    await expect(rankedSignalsTable(page).locator("tbody tr")).toHaveCount(1);
    await search.press("Escape");
    await expect(search).toHaveValue("");
    await expect(rankedSignalsTable(page).locator("tbody tr")).toHaveCount(31);

    const matrixTile = dashboardRegion(page, "Condition matrix").getByTestId(
      "stock-beta-matrix-000004.KRX",
    );
    await matrixTile.focus();
    await expect(matrixTile).toBeFocused();
    await matrixTile.press("Space");
    await expect(matrixTile).toHaveAttribute("aria-pressed", "true");
    await expect(rankedRow(page, "000004.KRX")).toHaveAttribute("data-selected", "true");

    const secondRow = rankedRow(page, "000002.KRX");
    const secondSelect = secondRow.getByRole("button", {
      name: "Select signal: 000002.KRX",
    });
    await secondSelect.focus();
    await secondSelect.press("Enter");
    await expect(secondRow).toHaveAttribute("data-selected", "true");
    await expect(signalPreview(page)).toHaveAttribute("data-selected-instrument", "000002.KRX");
  });

  test("renders the deterministic 1Y OHLCV chart, latest price, and every range", async ({
    page,
    request,
  }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 1,
      stockBetaSeed: "ready",
      stockBetaChartFreshness: "CURRENT",
    });

    await page.goto("/stock-beta");
    const chart = priceChart(page);
    const candles = chart.locator("g[data-candle-index]");
    await expect(chart).toBeVisible();
    await expect(priceChartPart(page, "stock-beta-price-chart-surface")).toBeVisible();
    await expect(candles).toHaveCount(261);
    const legend = chart.getByRole("list", { name: "EOD price chart" });
    await expect(legend).toBeVisible();
    await expect(legend.locator("li").filter({ hasText: "20-session moving average" })).toHaveCount(
      1,
    );
    await expect(legend.locator("li").filter({ hasText: "60-session moving average" })).toHaveCount(
      1,
    );

    const profile = signalProfilePanel(page);
    const latestStrip = profile.locator("dl").first();
    await expect(latestStrip).toContainText("Latest EOD close");
    await expect(profile.locator("dl")).toContainText(chartLatestClose);
    await expect(profile.locator("dl")).toContainText(chartLatestChange);
    await expect(profile.locator("dl")).toContainText(chartLatestRate);
    await expect(profile.locator("dl")).toContainText(chartLatestVolume);
    await expect(profile.locator("dl")).toContainText(chartAsOf);

    const rangeCounts: number[] = [];
    for (const range of ["1M", "3M", "6M", "1Y"]) {
      const rangeButton = page.getByRole("button", { name: range, exact: true });
      await rangeButton.click();
      await expect(rangeButton).toHaveAttribute("aria-pressed", "true");
      await expect.poll(() => candles.count()).toBeGreaterThan(0);
      rangeCounts.push(await candles.count());
    }
    expect(rangeCounts).toEqual([24, 67, 130, 261]);
  });

  test("selects a chart from both a ranked row and filtered search result", async ({
    page,
    request,
  }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 4,
      stockBetaSeed: "ready",
    });

    await page.goto("/stock-beta");
    const rowTwo = rankedRow(page, "000002.KRX");
    await rowTwo.getByRole("button", { name: "Select signal: 000002.KRX" }).click();
    await expect(rowTwo).toHaveAttribute("data-selected", "true");
    await expect(signalPreview(page)).toHaveAttribute("data-selected-instrument", "000002.KRX");

    const search = page.locator("#stock-beta-instrument-search-input");
    await search.fill("000004");
    const rowFour = rankedRow(page, "000004.KRX");
    await expect(rowFour).toBeVisible();
    await rowFour.getByRole("button", { name: "Select signal: 000004.KRX" }).click();
    await expect(rowFour).toHaveAttribute("data-selected", "true");
    await expect(signalPreview(page)).toHaveAttribute("data-selected-instrument", "000004.KRX");
    await expect(priceChart(page)).toBeVisible();
  });

  test("discards a late A chart response after selecting B", async ({ page, request }) => {
    const observed = observeBrowserRequests(page);
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 2,
      stockBetaSeed: "ready",
      stockBetaSeedCode: "000002",
      stockBetaChartDelays: { "000001.KRX:1y": 350 },
    });

    await page.goto("/stock-beta");
    const rowA = rankedRow(page, "000001.KRX");
    const rowB = rankedRow(page, "000002.KRX");
    const lateA = page.waitForRequest(
      (request) =>
        request.url().includes("000001.KRX/chart") &&
        new URL(request.url()).searchParams.get("range") === "1y",
    );
    await rowA.getByRole("button", { name: "Select signal: 000001.KRX" }).click();
    await lateA;
    await expect(rowA).toHaveAttribute("data-selected", "true");

    await rowB.getByRole("button", { name: "Select signal: 000002.KRX" }).click();
    await expect(rowB).toHaveAttribute("data-selected", "true");
    await expect(signalPreview(page)).toHaveAttribute("data-selected-instrument", "000002.KRX");
    const chart = priceChart(page);
    const surface = priceChartPart(page, "stock-beta-price-chart-surface");
    await expect(chart).toBeVisible();
    await surface.press("End");
    await expect(priceChartPart(page, "stock-beta-price-chart-summary")).toContainText("12,220");
    expect(observed.some((url) => url.pathname.includes("000001.KRX/chart"))).toBeTruthy();
  });

  test("keeps the previous range visible during delayed loading and rejects a stale response after a rapid change", async ({
    page,
    request,
  }) => {
    const observed = observeBrowserRequests(page);
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 1,
      stockBetaSeed: "ready",
      stockBetaChartDelays: { "000001.KRX:1m": 2_000 },
    });

    await page.goto("/stock-beta");
    const profile = signalProfilePanel(page);
    const chart = priceChart(page);
    const candles = chart.locator("g[data-candle-index]");
    const oneMonthRangeButton = profile.getByRole("button", { name: "1M", exact: true });
    const threeMonthRangeButton = profile.getByRole("button", { name: "3M", exact: true });
    const oneYearRangeButton = profile.getByRole("button", { name: "1Y", exact: true });
    await expect(chart).toBeVisible();
    await expect(candles).toHaveCount(261);
    await expect(oneYearRangeButton).toHaveAttribute("aria-pressed", "true");
    const lateOneMonth = page.waitForRequest(
      (request) =>
        request.url().endsWith("/chart?range=1m") ||
        (request.url().includes("/chart?") &&
          new URL(request.url()).searchParams.get("range") === "1m"),
    );
    await oneMonthRangeButton.click();
    await lateOneMonth;

    await expect(chart).toBeVisible();
    await expect(candles).toHaveCount(261);
    await expect(oneYearRangeButton).toHaveAttribute("aria-pressed", "true");
    await expect(oneMonthRangeButton).toHaveAttribute("aria-pressed", "false");
    await expect(profile.getByText("Updating EOD chart…", { exact: true })).toBeVisible();

    const threeMonth = page.waitForRequest(
      (request) =>
        request.url().includes("/chart?") &&
        new URL(request.url()).searchParams.get("range") === "3m",
    );
    await threeMonthRangeButton.click();
    await threeMonth;

    await expect(threeMonthRangeButton).toHaveAttribute("aria-pressed", "true");
    await expect(oneMonthRangeButton).toHaveAttribute("aria-pressed", "false");
    await expect(oneYearRangeButton).toHaveAttribute("aria-pressed", "false");
    await expect(candles).toHaveCount(67);
    expect(chartRequests(observed, "000001.KRX", "1m").length).toBeGreaterThan(0);
    expect(chartRequests(observed, "000001.KRX", "3m").length).toBeGreaterThan(0);
  });

  test("pins each chart request to the newly displayed snapshot after refresh", async ({
    page,
    request,
  }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 1,
      stockBetaSeed: "ready",
    });
    await page.goto("/stock-beta");
    const oldSnapshotId = snapshotIdFromText(await snapshotStrip(page).textContent());

    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 2,
      stockBetaSeed: "ready",
    });
    await page.getByLabel("KRX code").fill("005930");
    await page.getByRole("button", { name: "Add instrument" }).click();
    await expect(membershipCard(page, "005930.KRX")).toHaveAttribute("data-lifecycle", "READY", {
      timeout: 20_000,
    });
    const newSnapshotId = snapshotIdFromText(await snapshotStrip(page).textContent());
    expect(newSnapshotId).not.toBe(oldSnapshotId);
    await expect
      .poll(() =>
        chartRequests(requestsFor(page), "000001.KRX", "1y").some(
          (url) => url.searchParams.get("snapshot_id") === newSnapshotId,
        ),
      )
      .toBeTruthy();
  });

  test("updates the tooltip and live summary for pointer and keyboard selection", async ({
    page,
    request,
  }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 1,
      stockBetaSeed: "ready",
    });
    await page.goto("/stock-beta");
    const chart = priceChart(page);
    const surface = priceChartPart(page, "stock-beta-price-chart-surface");
    const summary = priceChartPart(page, "stock-beta-price-chart-summary");
    const surfaceBox = await box(surface);

    await surface.dispatchEvent("pointermove", {
      clientX: surfaceBox.x + surfaceBox.width / 2,
      clientY: surfaceBox.y + surfaceBox.height / 2,
      pointerType: "mouse",
    });
    await expect(chart).toHaveAttribute("data-interaction-source", "pointer");
    await expect(priceChartPart(page, "stock-beta-price-chart-tooltip")).toBeVisible();
    const pointerDate = await summary.getAttribute("data-selected-date");
    expect(pointerDate).not.toBeNull();

    await surface.focus();
    await surface.press("Home");
    const firstDate = await summary.getAttribute("data-selected-date");
    expect(firstDate).toBe(chartFirstSession);
    await surface.press("ArrowRight");
    const secondDate = await summary.getAttribute("data-selected-date");
    expect(secondDate).not.toBe(firstDate);
    await surface.press("ArrowLeft");
    await expect(summary).toHaveAttribute("data-selected-date", chartFirstSession);
    await surface.press("End");
    await expect(summary).toHaveAttribute("data-selected-date", chartAsOf);
    await expect(chart).toHaveAttribute("data-interaction-source", "keyboard");
  });

  test("switches Returns, Volatility, and Activity profile tabs", async ({ page, request }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 1,
      stockBetaSeed: "ready",
    });
    await page.goto("/stock-beta");
    for (const [name, metric] of [
      ["Returns", "20-session return"],
      ["Volatility", "20-session volatility"],
      ["Activity", "20-session average volume"],
    ] as const) {
      const tab = page.getByRole("tab", { name, exact: true });
      await tab.click();
      await expect(tab).toHaveAttribute("aria-selected", "true");
      await expect(signalProfilePanel(page, name)).toContainText(metric);
    }
  });

  test("renders current, stale, and unverifiable chart freshness without inventing data", async ({
    page,
    request,
  }) => {
    for (const [freshness, message] of [
      ["CURRENT", null],
      ["STALE", "EOD data is stale: as of 2026-08-28; expected 2026-08-31."],
      ["UNVERIFIABLE", "The latest EOD close cannot be verified against the expected market date."],
    ] as const) {
      await resetScenario(request, {
        authSession: "valid",
        role: "owner",
        stockBetaRows: 1,
        stockBetaSeed: "ready",
        stockBetaChartFreshness: freshness,
      });
      await page.goto("/stock-beta");
      const profile = signalProfilePanel(page);
      await expect(profile).toContainText(chartAsOf);
      if (message === null) await expect(profile).not.toContainText("cannot be verified");
      else await expect(profile).toContainText(message);
    }
  });

  test("fails chart closed for unavailable, integrity, missing, and forbidden chart responses", async ({
    page,
    request,
  }) => {
    for (const [state, expected] of [
      [
        "unavailable",
        { kind: "profile", text: "The EOD chart is being prepared or is unavailable." },
      ],
      [
        "integrity",
        { kind: "profile", text: "Chart integrity could not be verified. No price data is shown." },
      ],
      ["not_found", { kind: "boundary", text: "RESOURCE_NOT_FOUND" }],
      ["forbidden", { kind: "boundary", text: "FORBIDDEN" }],
    ] as const) {
      await resetScenario(request, {
        authSession: "valid",
        role: "owner",
        stockBetaRows: 1,
        stockBetaSeed: "ready",
        stockBetaChartState: state,
      });
      await page.goto("/stock-beta");
      await expect(page.getByTestId("stock-beta-price-chart")).toHaveCount(0);
      if (expected.kind === "profile") {
        await expect(
          signalProfilePanel(page)
            .filter({ hasText: expected.text })
            .getByText(expected.text, { exact: true }),
        ).toBeVisible();
      } else {
        await expect(page.getByRole("tabpanel", { name: "Price" })).toHaveCount(0);
        await expect(
          page.getByRole("alert", { name: "Stock signal beta unavailable" }),
        ).toContainText(expected.text);
      }
    }
  });

  test("uses the requested desktop geometry at 1280 and captures evidence", async ({
    page,
    request,
  }, testInfo) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 31,
      stockBetaSeed: "ready",
    });

    await page.setViewportSize({ width: 1280, height: 720 });
    await page.goto("/stock-beta");
    const ranked = await box(rankedSignalsRegion(page));
    const profile = await box(signalProfileRegion(page));
    const decomposition = await box(dashboardRegion(page, "Signal decomposition"));
    const matrix = await box(dashboardRegion(page, "Condition matrix"));
    const tape = await box(dashboardRegion(page, "Current snapshot tape"));
    const management = await box(universeManagementRegion(page));

    expect(ranked.x).toBeLessThan(profile.x);
    expect(profile.x).toBeLessThan(decomposition.x);
    expect(Math.abs(ranked.y - profile.y)).toBeLessThan(2);
    expect(Math.abs(profile.y - decomposition.y)).toBeLessThan(2);
    expect(matrix.x).toBeLessThan(tape.x);
    expect(Math.abs(matrix.y - tape.y)).toBeLessThan(2);
    expect(management.y).toBeGreaterThan(matrix.y);
    const membership = membershipRegion(page);
    const firstCard = membershipCard(page, "000001.KRX");
    const detailAction = firstCard.getByRole("link", { name: "Open detail" });
    const disableAction = firstCard.getByRole("button", { name: "Disable" });
    await detailAction.focus();
    await page.keyboard.press("Tab");
    await expect(disableAction).toBeFocused();
    const disableBounds = await box(disableAction);
    const disableFocusOutline = {
      height: disableBounds.height + 6,
      width: disableBounds.width + 6,
      x: disableBounds.x - 3,
      y: disableBounds.y - 3,
    };
    expectBoxInside(disableFocusOutline, await box(firstCard));
    expectBoxInside(disableFocusOutline, { x: 0, y: 0, width: 1280, height: 720 });
    expectBoxInside(disableFocusOutline, await box(membership));
    await expectNoHorizontalOverflow(page);
    await page.screenshot({
      animations: "disabled",
      fullPage: true,
      path: testInfo.outputPath("stock-beta-1280x720.png"),
    });
  });

  test("reflows at mobile, tablet, desktop, and 200% zoom-equivalent viewports", async ({
    page,
    request,
  }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 31,
      stockBetaSeed: "ready",
    });

    for (const viewport of [
      { width: 375, height: 800 },
      { width: 768, height: 1024 },
      { width: 1280, height: 720 },
      { width: 1440, height: 900 },
      { width: 640, height: 360 },
    ]) {
      await page.setViewportSize(viewport);
      await page.goto("/stock-beta");
      await expect(rankedSignalsTable(page)).toBeVisible();
      await expect(signalPreview(page)).toBeVisible();
      const activeNavigationLink = page
        .getByRole("navigation", { name: "Primary" })
        .getByRole("link", { name: "Stock signal beta" });
      const navigation = page.getByRole("navigation", { name: "Primary" });
      const bodyScrollY = await page.evaluate(() => window.scrollY);
      expectBoxInside(await box(activeNavigationLink), await box(navigation));
      expect(await page.evaluate(() => window.scrollY)).toBe(bodyScrollY);
      await expect(page.locator("body")).toBeFocused();
      await activeNavigationLink.focus();
      await expect(activeNavigationLink).toBeFocused();
      expectBoxInside(await box(activeNavigationLink), await box(navigation));
      await expectNoHorizontalOverflow(page);
    }
  });

  test("supports Korean and English locale plus forced colors and reduced motion", async ({
    page,
    request,
  }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 31,
      stockBetaSeed: "ready",
    });

    await page.context().addCookies([{ name: "locale", value: "ko", url: appOrigin }]);
    await page.goto("/stock-beta");
    await expect(page.locator("html")).toHaveAttribute("lang", /^ko/);
    await expect(page.getByRole("heading", { name: "종목 신호 베타" })).toBeVisible();
    await expect(page.getByRole("table", { name: /V2 가격·거래량 신호 순위 표/ })).toBeVisible();

    await page.context().addCookies([{ name: "locale", value: "en", url: appOrigin }]);
    await page.goto("/stock-beta");
    await expect(page.locator("html")).toHaveAttribute("lang", /^en/);
    await expect(page.getByRole("heading", { name: "Stock signal beta" })).toBeVisible();

    await page.emulateMedia({ forcedColors: "active", reducedMotion: "reduce" });
    await page.goto("/stock-beta");
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            window.matchMedia("(forced-colors: active)").matches &&
            window.matchMedia("(prefers-reduced-motion: reduce)").matches,
        ),
      )
      .toBeTruthy();
    await expect(
      page.getByRole("table", { name: /V2 ranked price-and-volume signal table/ }),
    ).toBeVisible();
    await expectNoHorizontalOverflow(page);
  });

  test("keeps touch targets usable with coarse input", async ({ browser, request }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 31,
      stockBetaSeed: "ready",
    });

    const context = await browser.newContext({
      baseURL: appOrigin,
      hasTouch: true,
      viewport: { width: 375, height: 800 },
    });
    const page = await context.newPage();
    try {
      const requests = observeBrowserRequests(page);
      await installProviderFreeNetworkGuard(page);
      await page.goto("/stock-beta");
      const selectTarget = rankedRow(page, "000001.KRX").getByRole("button", {
        name: "Select signal: 000001.KRX",
      });
      const targets = [
        page.getByLabel("Search signals"),
        selectTarget,
        dashboardRegion(page, "Condition matrix").getByTestId("stock-beta-matrix-000001.KRX"),
      ];
      for (const target of targets) {
        const targetBox = await box(target);
        expect(targetBox.height).toBeGreaterThanOrEqual(44);
      }
      for (const range of ["1M", "3M", "6M", "1Y"]) {
        expect(
          (await box(page.getByRole("button", { name: range, exact: true }))).height,
        ).toBeGreaterThanOrEqual(44);
      }
      await selectTarget.tap();
      await expect(rankedRow(page, "000001.KRX")).toHaveAttribute("data-selected", "true");
      const chartSurface = priceChartPart(page, "stock-beta-price-chart-surface");
      const chartBox = await box(chartSurface);
      await chartSurface.tap({
        position: { x: chartBox.width / 2, y: chartBox.height / 2 },
      });
      await expect(priceChart(page)).toHaveAttribute("data-interaction-source", "touch");
      await expect(priceChartPart(page, "stock-beta-price-chart-tooltip")).toBeVisible();
      await expectNoHorizontalOverflow(page);
      expectProviderFree(requests);
    } finally {
      await context.close();
    }
  });
});
