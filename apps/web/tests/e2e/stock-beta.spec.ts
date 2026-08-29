import { expect, test } from "@playwright/test";

const syntheticApiOrigin = process.env["SYNTHETIC_API_ORIGIN"] ?? "http://127.0.0.1:38180";

async function setScenario(
  request: import("@playwright/test").APIRequestContext,
  scenario: Record<string, string>,
): Promise<void> {
  const response = await request.post(`${syntheticApiOrigin}/__test/scenario`, { data: scenario });
  expect(response.ok()).toBe(true);
}

test.describe("Owner stock signal beta", () => {
  test("renders the Top 5, complete ranked table, and policy boundary", async ({
    page,
    request,
  }) => {
    await setScenario(request, { role: "owner" });

    await page.goto("/stock-beta");

    await expect(page.getByRole("heading", { level: 1, name: "Stock signal beta" })).toBeVisible();
    await expect(page.getByTestId("stock-beta-top-five")).toBeVisible();
    await expect(page.locator(".stock-beta-top-card")).toHaveCount(5);
    await expect(page.locator("[data-testid=stock-beta-rank-table] tbody tr")).toHaveCount(30);
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
    await expect(page.locator("[data-testid=stock-beta-rank-table] tbody tr")).toHaveCount(20);
    await expect(page.locator('input[name="condition"][value="BULLISH"]')).toBeChecked();
    await expect(page.locator('select[name="trend"]')).toHaveValue("up");
  });

  test("renders every detail evidence section and provenance", async ({ page, request }) => {
    await setScenario(request, { role: "owner" });

    await page.goto("/stock-beta/000001.KRX");

    await expect(
      page.getByRole("heading", { level: 2, name: "Configured instrument 1" }),
    ).toBeVisible();
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

    await page.goto("/stock-beta");

    await expect(page.getByRole("alert")).toContainText("Owner access required");
    await expect(page.getByTestId("stock-beta-rank-table")).toHaveCount(0);
    await expect(page.getByText("Configured instrument 1")).toHaveCount(0);
  });
});
