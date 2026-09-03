import { expect, type Locator, type Page, test } from "@playwright/test";

const syntheticApiOrigin = process.env["SYNTHETIC_API_ORIGIN"] ?? "http://127.0.0.1:38180";
const appOrigin = process.env["PLAYWRIGHT_BASE_URL"] ?? "http://127.0.0.1:33000";

async function setScenario(
  request: import("@playwright/test").APIRequestContext,
  scenario: Record<string, string>,
): Promise<void> {
  const response = await request.post(`${syntheticApiOrigin}/__test/scenario`, { data: scenario });
  expect(response.ok()).toBe(true);
}

async function expectNoPageOverflow(page: Page): Promise<void> {
  await expect
    .poll(() => page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth))
    .toBe(true);
}

async function pageGeometry(page: Page) {
  return page.evaluate(() => ({
    clientHeight: document.documentElement.clientHeight,
    clientWidth: document.documentElement.clientWidth,
    scrollHeight: document.documentElement.scrollHeight,
    scrollWidth: document.documentElement.scrollWidth,
  }));
}

async function expectVisibleFocus(page: Page): Promise<void> {
  await expect
    .poll(() =>
      page.evaluate(() => {
        const element = document.activeElement;
        if (!(element instanceof HTMLElement)) return false;
        const style = getComputedStyle(element);
        return (
          (style.outlineStyle !== "none" && style.outlineWidth !== "0px") ||
          style.boxShadow !== "none"
        );
      }),
    )
    .toBe(true);
}

async function expectNoSignalData(page: Page): Promise<void> {
  await expect(page.getByRole("table")).toHaveCount(0);
  await expect(page.getByRole("row")).toHaveCount(0);
  await expect(page.getByTestId("stock-beta-factor-table")).toHaveCount(0);
  await expect(page.getByText("Configured instrument 1", { exact: true })).toHaveCount(0);
}

async function expectTouchTarget(control: Locator, description: string): Promise<void> {
  await control.scrollIntoViewIfNeeded();
  await expect(control, description).toBeVisible();
  const box = await control.boundingBox();
  expect(box, `${description} must have an effective touch hit area`).not.toBeNull();
  expect(
    box?.height,
    `${description} must be at least 44px tall for coarse pointers`,
  ).toBeGreaterThanOrEqual(44);
}

async function expectTouchTargets(
  controls: Locator,
  expectedCount: number,
  description: string,
): Promise<void> {
  await expect(controls, description).toHaveCount(expectedCount);
  for (let index = 0; index < expectedCount; index += 1) {
    await expectTouchTarget(controls.nth(index), `${description} ${index + 1}`);
  }
}

async function expectTerminalUtilityBar(page: Page, hasSearch: boolean): Promise<void> {
  const bar = page.locator('[data-terminal-utility-bar="stock-beta"]');
  const main = page.getByRole("main");
  const asOf = bar.locator('[data-terminal-slot="as-of"]');
  const search = bar.getByRole("combobox", { name: "Search instruments" });

  await expect(bar).toHaveCount(1);
  await expect(main).toHaveCount(1);
  await expect(bar.locator('[data-terminal-utility-host="stock-beta"]')).toHaveCount(1);
  await expect(main.locator('[data-terminal-utility-content="stock-beta"]')).toHaveCount(0);
  await expect(page.locator('[data-terminal-utility-content="stock-beta"]')).toHaveCount(1);
  await expect(asOf).toHaveCount(1);
  await expect(asOf).toBeVisible();
  if (hasSearch) {
    await expect(search).toHaveCount(1);
    await expect(search).toBeVisible();
    await expect(bar.locator('[data-terminal-slot="search"]')).toHaveCount(1);
  } else {
    await expect(search).toHaveCount(0);
    await expect(bar.locator('[data-terminal-slot="search"]')).toHaveCount(0);
  }

  const geometry = await bar.evaluate((element) => {
    const rect = element.getBoundingClientRect();
    const children = Array.from(element.children).map((child) => {
      const childRect = child.getBoundingClientRect();
      return {
        bottom: childRect.bottom,
        left: childRect.left,
        right: childRect.right,
        top: childRect.top,
      };
    });
    const slots = Array.from(
      element.querySelectorAll<HTMLElement>(
        '[data-terminal-slot="search"], [data-terminal-slot="as-of"]',
      ),
    ).map((slot) => {
      const slotRect = slot.getBoundingClientRect();
      return {
        bottom: slotRect.bottom,
        left: slotRect.left,
        right: slotRect.right,
        top: slotRect.top,
      };
    });
    return { height: rect.height, rect, children, slots };
  });

  expect(geometry.height).toBeGreaterThanOrEqual(48);
  expect(geometry.height).toBeLessThanOrEqual(52);
  expect(geometry.children).toHaveLength(3);
  for (const child of geometry.children) {
    expect(child.left).toBeGreaterThanOrEqual(geometry.rect.left - 1);
    expect(child.right).toBeLessThanOrEqual(geometry.rect.right + 1);
    expect(child.top).toBeGreaterThanOrEqual(geometry.rect.top - 1);
    expect(child.bottom).toBeLessThanOrEqual(geometry.rect.bottom + 1);
  }
  for (let index = 1; index < geometry.children.length; index += 1) {
    expect(geometry.children[index - 1]?.right).toBeLessThanOrEqual(
      (geometry.children[index]?.left ?? 0) + 1,
    );
  }
  if (hasSearch) {
    expect(geometry.slots).toHaveLength(2);
    expect(geometry.slots[0]?.right).toBeLessThanOrEqual((geometry.slots[1]?.left ?? 0) + 1);
  } else {
    expect(geometry.slots).toHaveLength(1);
  }
}

async function expectTerminalUtilityHostEmpty(page: Page): Promise<void> {
  const bar = page.locator('[data-terminal-utility-bar="stock-beta"]');
  await expect(bar.locator('[data-terminal-utility-host="stock-beta"] > *')).toHaveCount(0);
  await expect(page.locator('[data-terminal-utility-content="stock-beta"]')).toHaveCount(0);
}

test.describe("Owner stock signal beta", () => {
  test.describe.configure({ mode: "serial" });

  test("renders the Top 5, complete ranked table, and policy boundary", async ({
    page,
    request,
  }) => {
    await setScenario(request, { role: "owner" });

    await page.goto("/stock-beta");

    await expect(page.getByRole("heading", { level: 1, name: "Stock signal beta" })).toBeVisible();
    await expectTerminalUtilityBar(page, true);
    await expect(page.getByTestId("stock-beta-top-five")).toBeVisible();
    const table = page.getByTestId("stock-beta-rank-table");
    await expect(table.locator("tbody").getByRole("row")).toHaveCount(30);
    await expect(table.locator('[data-top-five="true"]')).toHaveCount(5);
    await expect(table.locator('[data-current-result-leader="true"]')).toHaveCount(0);
    await expect(table.locator("caption")).toContainText(
      "Ranked price-and-volume signal table · Result count: 30 · Configured results",
    );
    await expect(
      page.getByRole("note", { name: "Stock signal beta policy boundary" }),
    ).toContainText("not current or historical index membership");
    await expect(
      page.getByRole("note", { name: "Stock signal beta policy boundary" }),
    ).toContainText("not execution liquidity");
  });

  test("submits URL filters and renders a server-ranked screen", async ({ page, request }) => {
    await setScenario(request, { role: "owner" });

    await page.goto("/stock-beta?condition=BULLISH&condition=BEARISH&return_20_min=0.05&trend=up");

    await expect(page).toHaveURL(
      /\/stock-beta\?condition=BULLISH&condition=BEARISH&return_20_min=0\.05&trend=up/,
    );
    const table = page.getByTestId("stock-beta-rank-table");
    await expect(table.locator("tbody").getByRole("row")).toHaveCount(20);
    await expect(page.locator('input[name="condition"][value="BULLISH"]')).toBeChecked();
    await expect(page.locator('select[name="trend"]')).toHaveValue("up");
    await expect(page.getByTestId("stock-beta-current-result-leaders")).toBeVisible();
    await expect(page.getByTestId("stock-beta-current-result-leaders")).toContainText(
      "Current-result leaders",
    );
    await expect(
      page.getByText(
        "Up to five rows from these current results, in the order returned by the server.",
        { exact: true },
      ),
    ).toBeVisible();
    await expect(table.locator('[data-current-result-leader="true"]')).toHaveCount(5);
    await expect(page.getByTestId("stock-beta-top-five")).toHaveCount(0);
    await expect(table.locator('[data-top-five="true"]')).toHaveCount(0);
    await expect(page.getByText("Top 5", { exact: true })).toHaveCount(0);
    await expect(table.locator("caption")).toContainText(
      "Ranked price-and-volume signal table · Result count: 20 · Current results",
    );
  });

  test("submits, preserves, and clears the compact GET filters", async ({ page, request }) => {
    await setScenario(request, { role: "owner" });
    await page.goto("/stock-beta");

    await page.getByText("Show filters", { exact: true }).click();
    await page.locator('input[name="condition"][value="BULLISH"]').check();
    await page.locator('input[name="return_20_min"]').fill("0.095");
    await page.locator('select[name="trend"]').selectOption("up");
    await page.getByRole("button", { name: "Apply filters" }).click();

    await expect.poll(() => new URL(page.url()).searchParams.get("condition")).toBe("BULLISH");
    await expect.poll(() => new URL(page.url()).searchParams.get("return_20_min")).toBe("0.095");
    await expect.poll(() => new URL(page.url()).searchParams.get("trend")).toBe("up");
    await expect(page.getByTestId("stock-beta-active-filters")).toContainText("BULLISH");
    await expect(page.getByTestId("stock-beta-rank-table").locator("tbody tr")).toHaveCount(2);

    const selected = page.getByTestId("stock-beta-row-000001.KRX");
    await selected.getByRole("link", { name: "Open instrument detail" }).click();
    await page.getByRole("link", { name: "Back to stock signal beta" }).click();
    await expect(page).toHaveURL(/\/stock-beta\?condition=BULLISH&return_20_min=0\.095&trend=up$/);

    await page.getByRole("link", { name: "Clear filters" }).click();
    await expect(page).toHaveURL(/\/stock-beta$/);
    await expect(page.getByTestId("stock-beta-rank-table").locator("tbody tr")).toHaveCount(30);
  });

  test("renders an explicit invalid-filter alert without stale signal data", async ({
    page,
    request,
  }) => {
    await setScenario(request, { role: "owner" });

    await page.goto("/stock-beta?trend=sideways");

    const alert = page.getByRole("alert", { name: "Stock signal filters are invalid" });
    await expect(alert).toBeVisible();
    await expect(alert).toContainText(
      "Check the scenario, trend, and numeric range values, then submit the GET filter form again.",
    );
    await expectNoSignalData(page);
  });

  test("renders an explicit filtered-empty status without stale ranked rows", async ({
    page,
    request,
  }) => {
    await setScenario(request, { role: "owner", stockBeta: "empty" });

    await page.goto("/stock-beta?condition=BULLISH");

    const emptyState = page
      .getByTestId("stock-beta-widget-ranked-signals")
      .getByRole("status")
      .filter({ hasText: "No configured instruments match these filters." });
    await expect(emptyState).toBeVisible();
    await expect(emptyState).toContainText("No configured instruments match these filters.");
    await expect(page.getByRole("heading", { level: 2, name: "Full ranked table" })).toBeVisible();
    await expect(page.getByTestId("stock-beta-current-result-leaders")).toHaveCount(0);
    await expect(page.getByTestId("stock-beta-top-five")).toHaveCount(0);
    await expectNoSignalData(page);
  });

  test("renders the unavailable snapshot alert without stale ranked rows or factor data", async ({
    page,
    request,
  }) => {
    await setScenario(request, { role: "owner", stockBeta: "unavailable" });

    await page.goto("/stock-beta");

    const alert = page.getByRole("alert", { name: "Signal data unavailable" });
    await expect(alert).toBeVisible();
    await expect(alert).toContainText(
      "The approved signal snapshot is unavailable. No signal rows are shown and no fallback data is substituted.",
    );
    await expectNoSignalData(page);
  });

  test("renders the integrity-failure alert without stale ranked rows or factor data", async ({
    page,
    request,
  }) => {
    await setScenario(request, { role: "owner", stockBeta: "integrity" });

    await page.goto("/stock-beta");

    const alert = page.getByRole("alert", { name: "Signal snapshot integrity failed" });
    await expect(alert).toBeVisible();
    await expect(alert).toContainText(
      "The approved signal snapshot failed its integrity check. No signal rows are shown and no fallback data is substituted.",
    );
    await expectNoSignalData(page);
  });

  test("renders generic dashboard and detail failures without stale numeric or detail data", async ({
    page,
    request,
  }) => {
    await setScenario(request, { role: "owner", stockBeta: "generic" });

    for (const route of ["/stock-beta", "/stock-beta/000001.KRX"]) {
      await page.goto(route);
      await expect(
        page.getByRole("alert", { name: "Stock signal beta unavailable" }),
      ).toBeVisible();
      await expectNoSignalData(page);
      await expect(page.getByText("20-session price return", { exact: true })).toHaveCount(0);
      await expectTerminalUtilityHostEmpty(page);
    }
  });

  test("renders the detail not-found status instead of generic unavailable", async ({
    page,
    request,
  }) => {
    await setScenario(request, { role: "owner" });

    await page.goto("/stock-beta/999999.KRX");

    const emptyState = page
      .getByRole("status")
      .filter({ hasText: "No approved signal row matches this configured instrument." });
    await expect(emptyState).toBeVisible();
    await expect(emptyState).toContainText(
      "No approved signal row matches this configured instrument.",
    );
    await expect(
      page.getByRole("heading", { level: 2, name: "Instrument signal not found" }),
    ).toBeVisible();
    await expect(
      page.getByRole("heading", { level: 2, name: "Stock signal beta unavailable" }),
    ).toHaveCount(0);
    await expectNoSignalData(page);
  });

  test("renders every detail evidence section and provenance", async ({ page, request }) => {
    await setScenario(request, { role: "owner" });

    await page.goto("/stock-beta/000001.KRX");

    await expectTerminalUtilityBar(page, false);
    await expect(page.getByTestId("stock-beta-instrument-header")).toContainText(
      "Configured instrument 1",
    );
    await expect(page.getByTestId("stock-beta-factor-table")).toContainText("return_20");
    await expect(page.getByTestId("stock-beta-factor-table")).toContainText(
      "20-session price return",
    );
    await expect(
      page.getByRole("heading", { level: 3, name: "Exact condition reasons" }),
    ).toBeVisible();
    await expect(page.getByText("trend_up is true", { exact: true })).toBeVisible();
    await expect(page.getByTestId("stock-beta-provenance")).toContainText(
      "batch-stock-beta-synthetic",
    );
    await expect(page.getByTestId("stock-beta-provenance")).toContainText("Approval registry hash");
  });

  test("refuses a Member direct visit without rendering signal rows", async ({ page, request }) => {
    await setScenario(request, { role: "member" });

    for (const route of ["/stock-beta", "/stock-beta/000001.KRX"]) {
      await page.goto(route);
      await expect(page.getByRole("alert", { name: "Owner access required" })).toContainText(
        "Owner access required",
      );
      await expect(page.getByTestId("stock-beta-rank-table")).toHaveCount(0);
      await expect(page.getByText("Configured instrument 1")).toHaveCount(0);
      await expect(page.getByTestId("stock-beta-factor-table")).toHaveCount(0);
    }
  });

  test("supports slash search, Escape clearing, and matrix-to-ranking keyboard selection", async ({
    page,
    request,
  }) => {
    await setScenario(request, { role: "owner" });
    await page.goto("/stock-beta");

    const search = page.getByRole("combobox", { name: "Search instruments" });
    await expect(search).toBeVisible();
    // Confirm the client tree is interactive before exercising the window-level slash shortcut.
    await page
      .getByTestId("stock-beta-row-000001.KRX")
      .getByRole("button", { name: "Select for preview: Configured instrument 1", exact: true })
      .click();
    await page.keyboard.press("/");
    await expect(search).toBeFocused();
    await search.fill("000004");
    await expect(page.getByTestId("stock-beta-rank-table").locator("tbody tr")).toHaveCount(1);
    await search.press("Escape");
    await expect(search).toHaveValue("");
    await expect(search).not.toBeFocused();
    await expect(page.getByTestId("stock-beta-rank-table").locator("tbody tr")).toHaveCount(30);

    const matrixSelection = page.getByTestId("stock-beta-matrix-000004.KRX");
    await matrixSelection.focus();
    await expectVisibleFocus(page);
    await page.keyboard.press("Space");
    await expect(matrixSelection).toHaveAttribute("aria-pressed", "true");
    await expect(page.getByTestId("stock-beta-row-000004.KRX")).toHaveAttribute(
      "data-selected",
      "true",
    );
    await expect(page.getByTestId("stock-beta-signal-preview")).toHaveAttribute(
      "data-selected-instrument",
      "000004.KRX",
    );
  });

  test("uses the named instrument button for keyboard preview selection and preserves filter context", async ({
    page,
    request,
  }) => {
    await setScenario(request, { role: "owner" });
    await page.goto("/stock-beta?condition=BULLISH&trend=up");

    const selectedRow = page.getByTestId("stock-beta-row-000001.KRX");
    const nextRow = page.getByTestId("stock-beta-row-000004.KRX");
    const nextInstrument = page.getByRole("button", {
      name: "Select for preview: Configured instrument 4",
      exact: true,
    });

    await expect(selectedRow).not.toHaveAttribute("tabindex");
    await expect(selectedRow).not.toHaveAttribute("aria-selected");
    await expect(nextRow).not.toHaveAttribute("tabindex");
    await nextInstrument.focus();
    await expect(nextInstrument).toBeFocused();
    await expect(nextInstrument).toHaveAttribute("aria-pressed", "false");
    await page.keyboard.press("Enter");
    await expect(nextInstrument).toHaveAttribute("aria-pressed", "true");
    await expect(nextRow).toHaveAttribute("data-selected", "true");

    const detailLink = nextRow.getByRole("link", { name: "Open instrument detail" });
    await expect(detailLink).toHaveAttribute(
      "href",
      "/stock-beta/000004.KRX?condition=BULLISH&trend=up",
    );
    await page.goto((await detailLink.getAttribute("href")) ?? "/stock-beta");
    await page.getByRole("link", { name: "Back to stock signal beta" }).click();
    await expect(page).toHaveURL(/\/stock-beta\?condition=BULLISH&trend=up$/);
  });

  test("keeps dashboard and detail content within the page at required breakpoints and 200% reflow", async ({
    page,
    request,
  }) => {
    await setScenario(request, { role: "owner" });

    const viewports = [
      { height: 812, label: "375x812", width: 375 },
      { height: 1024, label: "768x1024", width: 768 },
      { height: 720, label: "1280x720", width: 1280 },
      { height: 900, label: "1440x900", width: 1440 },
      // A 640 CSS-pixel layout viewport is the reflow equivalent of 200% zoom at 1280px.
      { height: 360, label: "200% at 1280px", width: 640 },
    ];
    for (const viewport of viewports) {
      await page.setViewportSize(viewport);
      await page.goto("/stock-beta");
      await expect(page.getByTestId("stock-beta-rank-table")).toBeVisible();
      await expectTerminalUtilityBar(page, true);
      await expectNoPageOverflow(page);
      expect((await pageGeometry(page)).scrollWidth, viewport.label).toBeLessThanOrEqual(
        viewport.width,
      );

      const search = page.getByRole("combobox", { name: "Search instruments" });
      // Confirm hydration before relying on the page-level slash shortcut at each responsive size.
      await page
        .getByTestId("stock-beta-row-000001.KRX")
        .getByRole("button", { name: "Select for preview: Configured instrument 1", exact: true })
        .click();
      await page.keyboard.press("/");
      await expect(search).toBeFocused();
      await search.fill("000004");
      await search.press("Escape");
      await expect(search).toHaveValue("");
      await expect(search).not.toBeFocused();

      await page.goto("/stock-beta/000001.KRX");
      await expect(page.getByTestId("stock-beta-detail-board")).toBeVisible();
      await expectTerminalUtilityBar(page, false);
      await expectNoPageOverflow(page);
      expect((await pageGeometry(page)).scrollWidth, viewport.label).toBeLessThanOrEqual(
        viewport.width,
      );
    }
  });

  test("renders Korean light and English dark themes without losing semantic content", async ({
    page,
    request,
  }) => {
    await setScenario(request, { role: "owner" });

    for (const appearance of [
      { locale: "ko", theme: "light" },
      { locale: "en", theme: "dark" },
    ] as const) {
      await page.context().addCookies([
        { name: "locale", value: appearance.locale, domain: "127.0.0.1", path: "/" },
        { name: "theme", value: appearance.theme, domain: "127.0.0.1", path: "/" },
      ]);
      await page.goto("/stock-beta");
      await expect(page.locator("html")).toHaveAttribute("lang", appearance.locale);
      await expect(page.locator("html")).toHaveAttribute("data-theme", appearance.theme);
      await expect(page.getByRole("heading", { level: 1 })).toBeVisible();
      await expect(page.getByTestId("stock-beta-rank-table")).toBeVisible();
      await expectNoPageOverflow(page);
    }
  });

  test("retains keyboard access and essential content under reduced motion and forced colors", async ({
    page,
    request,
  }) => {
    await setScenario(request, { role: "owner" });
    await page.emulateMedia({ forcedColors: "active", reducedMotion: "reduce" });
    await page.goto("/stock-beta");

    await expect
      .poll(() => page.evaluate(() => matchMedia("(prefers-reduced-motion: reduce)").matches))
      .toBe(true);
    await expect
      .poll(() => page.evaluate(() => matchMedia("(forced-colors: active)").matches))
      .toBe(true);
    const firstInstrument = page.getByRole("button", {
      exact: true,
      name: "Select for preview: Configured instrument 1",
    });
    await firstInstrument.focus();
    await expect(firstInstrument).toBeFocused();
    await expect(page.getByTestId("stock-beta-rank-table").getByRole("columnheader")).toHaveCount(
      7,
    );
    await expectNoPageOverflow(page);
  });

  test("keeps navigation inside one terminal look and feel across authenticated routes", async ({
    page,
    request,
  }) => {
    await setScenario(request, { role: "owner" });

    await page.goto("/");
    await expect(page.locator("[data-shell=research-terminal]")).toHaveCount(1);
    await expect(page.locator('[data-terminal-utility-bar="research"]')).toHaveCount(1);
    await expect(page.locator("[data-shell=stock-beta-terminal]")).toHaveCount(0);
    await page
      .getByRole("navigation", { name: "Primary" })
      .getByRole("link", { exact: true, name: "Strategies" })
      .click();
    await expect(page).toHaveURL(/\/strategies$/);
    await expect(page.locator("[data-shell=research-terminal]")).toHaveCount(1);
    await expect(page.locator("[data-shell=general]")).toHaveCount(0);
  });

  test("keeps Stock Beta semantic landmarks, table structure, and touch targets accessible", async ({
    browser,
    request,
  }) => {
    await setScenario(request, { role: "owner" });
    const context = await browser.newContext({
      baseURL: appOrigin,
      hasTouch: true,
      viewport: { height: 812, width: 375 },
    });
    const page = await context.newPage();
    try {
      await page.goto("/stock-beta");
      await expect(page.getByRole("main")).toHaveCount(1);
      await expect(page.getByRole("navigation", { name: "Primary" })).toHaveCount(1);
      await expect(page.getByTestId("stock-beta-rank-table").locator("caption")).toBeVisible();
      await expect(page.getByTestId("stock-beta-rank-table").getByRole("columnheader")).toHaveCount(
        7,
      );

      const filterSummary = page.getByText("Show filters", { exact: true });
      const columnSummary = page.getByText("Columns", { exact: true });
      const search = page.getByRole("combobox", { name: "Search instruments" });
      const profileTabs = page.getByRole("tab");
      const instrument = page.getByRole("button", {
        name: "Select for preview: Configured instrument 1",
        exact: true,
      });
      const matrixTile = page.getByTestId("stock-beta-matrix-000001.KRX");
      const detailLink = page
        .getByTestId("stock-beta-row-000001.KRX")
        .getByRole("link", { name: "Open instrument detail" });

      await expectTouchTarget(search, "instrument search");
      await expectTouchTarget(filterSummary, "filter summary");
      await expectTouchTarget(columnSummary, "column summary");
      await expectTouchTargets(profileTabs, 3, "profile tab");
      await expectTouchTarget(instrument, "ranked instrument selection");
      await expectTouchTarget(matrixTile, "condition-matrix selection");
      await expectTouchTarget(detailLink, "ranked detail navigation");

      await filterSummary.click();
      const filterForm = page.locator('form[action="/stock-beta"][method="get"]');
      const conditionInputs = filterForm.locator('input[name="condition"]');
      const conditionLabels = conditionInputs.locator("xpath=ancestor::label");
      const numericInputs = filterForm.locator('input[type="number"]');
      const trendSelect = filterForm.locator('select[name="trend"]');
      const applyFilters = filterForm.getByRole("button", { name: "Apply filters" });
      const clearFilters = filterForm.getByRole("link", { name: "Clear filters" });

      await expect(conditionInputs).toHaveCount(3);
      await expectTouchTargets(conditionLabels, 3, "condition checkbox label");
      await expectTouchTargets(numericInputs, 18, "numeric range input");
      await expectTouchTarget(trendSelect, "trend select");
      await expectTouchTarget(applyFilters, "apply filters control");
      await expectTouchTarget(clearFilters, "clear filters control");

      await filterSummary.click();
      await columnSummary.click();
      const columnGroup = page.getByRole("group", { name: "Visible ranking columns" });
      const columnLabels = columnGroup.locator("label");
      const columnInputs = columnGroup.getByRole("checkbox");
      await expect(columnInputs).toHaveCount(13);
      await expect(columnGroup.locator('input[type="checkbox"]:disabled')).toHaveCount(1);
      await expectTouchTargets(columnLabels, 13, "column checkbox label");
      await columnSummary.click();

      await filterSummary.click();
      await conditionInputs.nth(0).check();
      await applyFilters.click();
      const activeFilterRemoval = page.getByRole("link", { name: /Remove filter:/ });
      await expect(activeFilterRemoval).toHaveCount(1);
      await expectTouchTarget(activeFilterRemoval, "active-filter removal control");

      const filteredDetailLink = page
        .getByTestId("stock-beta-row-000001.KRX")
        .getByRole("link", { name: "Open instrument detail" });
      await expectTouchTarget(filteredDetailLink, "filtered ranked detail navigation");
      await filteredDetailLink.click();
      await expectTerminalUtilityBar(page, false);
      await expectTouchTarget(
        page.getByRole("link", { name: "Back to stock signal beta" }),
        "detail back navigation",
      );
    } finally {
      await context.close();
    }
  });
});
