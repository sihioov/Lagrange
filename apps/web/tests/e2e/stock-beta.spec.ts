import { expect, test } from "@playwright/test";

const syntheticApiOrigin = process.env["SYNTHETIC_API_ORIGIN"] ?? "http://127.0.0.1:38180";

async function setScenario(
  request: import("@playwright/test").APIRequestContext,
  scenario: Record<string, unknown>,
): Promise<void> {
  const response = await request.post(`${syntheticApiOrigin}/__test/scenario`, { data: scenario });
  expect(response.ok()).toBe(true);
}

test.describe("Owner stock signal beta V2", () => {
  test("renders the Owner-managed empty state and server policy capacity", async ({
    page,
    request,
  }) => {
    await setScenario(request, { role: "owner", stockBetaSeed: "empty", stockBetaRows: "31" });

    await page.goto("/stock-beta");

    await expect(page.getByRole("heading", { level: 1, name: "Stock signal beta" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "No configured instruments" })).toBeVisible();
    await expect(page.getByTestId("stock-beta-policy-capacity")).toContainText("100");
    await expect(page.getByText("Signals are not ready")).toBeVisible();
    await expect(page.getByTestId("stock-beta-rank-table")).toHaveCount(0);
  });

  test("renders all 31 configured signal rows", async ({ page, request }) => {
    await setScenario(request, { role: "owner", stockBetaRows: "31" });

    await page.goto("/stock-beta");

    await expect(page.locator("[data-testid=stock-beta-rank-table] tbody tr")).toHaveCount(31);
    await expect(page.getByText("000031.KRX", { exact: true })).toBeVisible();
    await expect(
      page.locator("[data-testid=stock-beta-top-five] .stock-beta-top-card"),
    ).toHaveCount(5);
  });

  test("renders all 100 configured signal rows without a client row bound", async ({
    page,
    request,
  }) => {
    await setScenario(request, { role: "owner", stockBetaRows: "100" });

    await page.goto("/stock-beta");

    await expect(page.locator("[data-testid=stock-beta-rank-table] tbody tr")).toHaveCount(100);
    await expect(page.getByText("000100.KRX", { exact: true })).toBeVisible();
  });

  test("adds an instrument, polls it to READY, and refreshes ranked/detail data", async ({
    page,
    request,
  }) => {
    await setScenario(request, {
      role: "owner",
      stockBetaRows: "31",
      stockBetaSeed: "empty",
      stockBetaWorkflow: "add-ready",
    });

    await page.goto("/stock-beta");
    await page.getByLabel("KRX stock code").fill("005930");
    await page.getByRole("button", { name: "Add instrument" }).click();

    await expect(
      page.locator('[data-testid="stock-beta-membership-card"][data-lifecycle="READY"]'),
    ).toContainText("005930.KRX", { timeout: 15_000 });
    await expect(page.getByTestId("stock-beta-rank-table")).toContainText("005930.KRX");
    await expect(page.getByTestId("stock-beta-snapshot")).toBeVisible();

    await page.getByRole("link", { name: "Open signal detail" }).first().click();
    await expect(page.getByRole("heading", { level: 2, name: "005930.KRX" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Snapshot" })).toBeVisible();
  });

  test("offers retry for a typed retryable failure and polls back to READY", async ({
    page,
    request,
  }) => {
    await setScenario(request, {
      role: "owner",
      stockBetaRows: "31",
      stockBetaSeed: "failed",
      stockBetaWorkflow: "retry",
    });

    await page.goto("/stock-beta");
    await expect(page.getByText("OWNER_EQUITY_BACKFILL_RETRYABLE", { exact: true })).toBeVisible();
    await page.getByRole("button", { name: "Retry preparation" }).click();

    await expect(
      page.locator('[data-testid="stock-beta-membership-card"][data-lifecycle="READY"]'),
    ).toBeVisible({ timeout: 15_000 });
    await expect(page.getByText("OWNER_EQUITY_BACKFILL_RETRYABLE", { exact: true })).toHaveCount(0);
  });

  test("requires confirmation before soft-disabling an instrument", async ({ page, request }) => {
    await setScenario(request, {
      role: "owner",
      stockBetaSeed: "ready",
      stockBetaWorkflow: "disable",
    });

    await page.goto("/stock-beta");
    const card = page.getByTestId("stock-beta-membership-card").first();
    await card.getByRole("button", { name: "Disable" }).click();
    await expect(card.getByText(/Disable this instrument/)).toBeVisible();
    const confirm = card.getByRole("button", { name: "Confirm disable" });
    await expect(confirm).toBeFocused();
    await confirm.click();

    await expect(card).toHaveAttribute("data-lifecycle", "DISABLED");
    await expect(card.getByRole("button", { name: "Disable" })).toHaveCount(0);
    await expect(page.getByText("000001.KRX", { exact: true })).toHaveCount(1);
    await expect(page.getByTestId("stock-beta-rank-table")).toHaveCount(0);
  });

  test("redirects an expired authenticated stock-beta request to login", async ({
    page,
    request,
  }) => {
    await setScenario(request, { authSession: "valid", role: "owner", stockBetaSeed: "ready" });
    await page.goto("/stock-beta");
    await expect(page.getByRole("heading", { level: 1, name: "Stock signal beta" })).toBeVisible();

    await setScenario(request, { authSession: "expired", role: "owner" });
    await page.reload({ waitUntil: "commit" });

    await expect(page).toHaveURL(/\/auth\/login$/);
  });

  test("hides the surface and denies a Member direct visit", async ({ page, request }) => {
    await setScenario(request, { role: "member" });

    await page.goto("/stock-beta");

    await expect(page.getByRole("alert", { name: "Owner access required" })).toBeVisible();
    await expect(page.getByTestId("stock-beta-rank-table")).toHaveCount(0);
    await expect(page.locator('a[href="/stock-beta"]')).toHaveCount(0);
  });
});
