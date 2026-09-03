import { type APIRequestContext, expect, type Locator, type Page, test } from "@playwright/test";

const appOrigin = process.env["PLAYWRIGHT_BASE_URL"] ?? "http://127.0.0.1:33000";
const syntheticOrigin = process.env["SYNTHETIC_API_ORIGIN"] ?? "http://127.0.0.1:38180";
const membershipsPath = "/api/v1/research/owner-beta/equity-universe-v2/memberships";

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
  const localRequests = requests.filter(isLocalTestUrl);
  expect(localRequests.some((url) => url.pathname.includes("equity-price-signals"))).toBeFalsy();
}

function observeMembershipPostCount(page: Page) {
  let count = 0;
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (request.method() === "POST" && url.pathname === membershipsPath) count += 1;
  });
  return () => count;
}

function membershipCard(page: Page, instrumentId: string) {
  return page.getByTestId("stock-beta-membership-card").filter({ hasText: instrumentId });
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

test.describe("provider-free Stock Beta V2", () => {
  test.beforeEach(async ({ page }) => {
    await installProviderFreeNetworkGuard(page);
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
    await expect(page.getByText(/No research instruments are configured\./)).toBeVisible();
    await expect(page.getByTestId("stock-beta-policy-capacity")).toContainText("100");
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
    await expect(page.getByTestId("stock-beta-snapshot-strip")).toBeVisible();
    await expect(page.getByTestId("stock-beta-snapshot-universe")).toContainText("0");
    await expect(
      page
        .getByTestId("stock-beta-widget-ranked-signals")
        .getByText("The current V2 snapshot has no signal rows."),
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
    await expect(page.getByTestId("stock-beta-membership-card")).toHaveCount(31);
    await expect(page.getByTestId("stock-beta-rank-table")).toBeVisible();
    await expect(page.getByTestId("stock-beta-rank-table").locator("tbody tr")).toHaveCount(31);
    await expect(page.getByTestId("stock-beta-row-000031.KRX")).toBeVisible();
    await expect(page.getByTestId("stock-beta-row-000001.KRX")).toHaveAttribute(
      "data-top-five",
      "true",
    );
    await expect(page.locator('[data-top-five="true"]')).toHaveCount(5);
    await expect(page.getByTestId("stock-beta-snapshot-universe")).toContainText("31");
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
    await expect(page.getByTestId("stock-beta-membership-card")).toHaveCount(100);
    await expect(page.getByTestId("stock-beta-rank-table").locator("tbody tr")).toHaveCount(100);
    await expect(page.getByTestId("stock-beta-row-000100.KRX")).toBeVisible();
    await expect(page.getByTestId("stock-beta-snapshot-universe")).toContainText("100");
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
    const row = page.getByTestId("stock-beta-row-005930.KRX");
    await expect(row).toBeVisible({ timeout: 20_000 });
    await expect(page.getByTestId("stock-beta-snapshot-universe")).toContainText("1", {
      timeout: 20_000,
    });
    await expect(row.getByRole("link", { name: "Open detail" })).toBeVisible();
    await expect(observed.some((url) => url.pathname.includes("equity-universe-v2"))).toBeTruthy();
    expectProviderFree(observed);

    await row.getByRole("link", { name: "Open detail" }).click();
    await expect(page).toHaveURL(/\/stock-beta\/005930\.KRX$/);
    await expect(page.getByTestId("stock-beta-detail-board")).toBeVisible();
    await expect(page.getByTestId("stock-beta-detail-widget-instrument-header")).toContainText(
      "005930.KRX",
    );
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
    await expect(page.getByTestId("stock-beta-row-000001.KRX")).toBeVisible({ timeout: 20_000 });
    await expect(page.getByTestId("stock-beta-snapshot-strip")).toBeVisible({ timeout: 20_000 });
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
    const oldRow = page.getByTestId("stock-beta-row-000001.KRX");
    await expect(oldRow).toBeVisible();
    await card.getByRole("button", { name: "Disable" }).click();
    const confirm = card.getByRole("button", { name: "Confirm disable" });
    await expect(confirm).toBeFocused();
    await confirm.click();

    await expect(oldRow).toHaveCount(0);
    await expect(page.getByTestId("stock-beta-snapshot-strip")).toHaveCount(0);
    await expect(card).toHaveAttribute("data-lifecycle", "DISABLED", { timeout: 20_000 });
    await expect(page.getByTestId("stock-beta-row-000001.KRX")).toHaveCount(0);
    await expect(page.getByTestId("stock-beta-snapshot-universe")).toContainText("30", {
      timeout: 20_000,
    });
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
    await expect(page.getByTestId("stock-beta-membership-card")).toHaveCount(31);

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
    await page
      .getByTestId("stock-beta-row-000001.KRX")
      .getByRole("link", { name: "Open detail" })
      .click();
    await expect(page.getByTestId("stock-beta-detail-board")).toBeVisible();
    await expectStockShell(page);
    await page.getByRole("link", { name: "Back to stock signal beta" }).click();
    await expect(page.getByTestId("stock-beta-rank-table")).toBeVisible();
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
    await expect(page.getByTestId("stock-beta-row-000004.KRX")).toBeVisible();
    await expect(page.getByTestId("stock-beta-rank-table").locator("tbody tr")).toHaveCount(1);
    await search.press("Escape");
    await expect(search).toHaveValue("");
    await expect(page.getByTestId("stock-beta-rank-table").locator("tbody tr")).toHaveCount(31);

    const matrixTile = page.getByTestId("stock-beta-matrix-000004.KRX");
    await matrixTile.focus();
    await expect(matrixTile).toBeFocused();
    await matrixTile.press("Space");
    await expect(matrixTile).toHaveAttribute("aria-pressed", "true");
    await expect(page.getByTestId("stock-beta-row-000004.KRX")).toHaveAttribute(
      "data-selected",
      "true",
    );

    const secondRow = page.getByTestId("stock-beta-row-000002.KRX");
    const secondSelect = secondRow.getByRole("button", {
      name: "Select signal: 000002.KRX",
    });
    await secondSelect.focus();
    await secondSelect.press("Enter");
    await expect(secondRow).toHaveAttribute("data-selected", "true");
    await expect(page.getByTestId("stock-beta-signal-preview")).toHaveAttribute(
      "data-selected-instrument",
      "000002.KRX",
    );
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
    const ranked = await box(page.getByTestId("stock-beta-widget-ranked-signals"));
    const profile = await box(page.getByTestId("stock-beta-widget-signal-profile"));
    const decomposition = await box(page.getByTestId("stock-beta-widget-signal-decomposition"));
    const matrix = await box(page.getByTestId("stock-beta-widget-condition-matrix"));
    const tape = await box(page.getByTestId("stock-beta-widget-snapshot-tape"));
    const management = await box(page.getByTestId("stock-beta-widget-universe-management"));

    expect(ranked.x).toBeLessThan(profile.x);
    expect(profile.x).toBeLessThan(decomposition.x);
    expect(Math.abs(ranked.y - profile.y)).toBeLessThan(2);
    expect(Math.abs(profile.y - decomposition.y)).toBeLessThan(2);
    expect(matrix.x).toBeLessThan(tape.x);
    expect(Math.abs(matrix.y - tape.y)).toBeLessThan(2);
    expect(management.y).toBeGreaterThan(matrix.y);
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
      { width: 375, height: 812 },
      { width: 768, height: 1024 },
      { width: 1280, height: 720 },
      { width: 1440, height: 900 },
      { width: 640, height: 360 },
    ]) {
      await page.setViewportSize(viewport);
      await page.goto("/stock-beta");
      await expect(page.getByTestId("stock-beta-rank-table")).toBeVisible();
      await expect(page.getByTestId("stock-beta-signal-preview")).toBeVisible();
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
    await expect(page.getByTestId("stock-beta-rank-table")).toBeVisible();

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
    await expect(page.getByTestId("stock-beta-rank-table")).toBeVisible();
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
      viewport: { width: 375, height: 812 },
    });
    const page = await context.newPage();
    try {
      await installProviderFreeNetworkGuard(page);
      await page.goto("/stock-beta");
      const selectTarget = page
        .getByTestId("stock-beta-row-000001.KRX")
        .getByRole("button", { name: "Select signal: 000001.KRX" });
      const targets = [
        page.getByLabel("Search signals"),
        selectTarget,
        page.getByTestId("stock-beta-matrix-000001.KRX"),
      ];
      for (const target of targets) {
        const targetBox = await box(target);
        expect(targetBox.height).toBeGreaterThanOrEqual(44);
      }
      await selectTarget.tap();
      await expect(page.getByTestId("stock-beta-row-000001.KRX")).toHaveAttribute(
        "data-selected",
        "true",
      );
      await expectNoHorizontalOverflow(page);
    } finally {
      await context.close();
    }
  });
});
