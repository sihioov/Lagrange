import { expect, test } from "@playwright/test";

const syntheticApiOrigin = process.env["SYNTHETIC_API_ORIGIN"] ?? "http://127.0.0.1:38180";

async function setScenario(
  request: import("@playwright/test").APIRequestContext,
  scenario: Record<string, string>,
): Promise<void> {
  const response = await request.post(`${syntheticApiOrigin}/__test/scenario`, {
    data: scenario,
  });
  expect(response.ok()).toBe(true);
}

test.beforeEach(async ({ request }) => {
  await setScenario(request, {
    entitlement: "active",
    exclusions: "present",
    recommendation: "fresh",
  });
});

test("member configures an allowed strategy and reads an explainable recommendation", async ({
  page,
}) => {
  // Given
  await page.goto("/strategies");
  await expect(page.getByRole("heading", { level: 1, name: "Strategies" })).toBeVisible();

  // When
  const configuration = page.getByRole("form", { name: "Configure Dual momentum" });
  await configuration.getByLabel("Lookback months").fill("12");
  await configuration.getByLabel("Top holdings").fill("4");
  await configuration.getByRole("button", { name: "Save strategy configuration" }).click();

  // Then
  await expect(page.getByRole("status")).toContainText("Configuration saved");

  // When
  await page.getByRole("link", { name: "Recommendations" }).click();
  const runForm = page.getByRole("form", { name: "Generate recommendation" });
  await runForm.getByLabel("As-of date").fill("2026-01-31");
  await runForm.getByRole("button", { name: "Generate strategy proposal" }).click();

  // Then
  const report = page.getByRole("region", { name: "Strategy-based proposal" });
  await expect(report.getByText("069500.KRX")).toBeVisible();
  await expect(report.getByText("40.00%")).toBeVisible();
  await expect(report.getByText("ABSOLUTE_MOMENTUM_PASS")).toBeVisible();
  await expect(report.getByText("114800.KRX")).toBeVisible();
  await expect(report).toContainText("Inverse products are outside the governed universe");
  await expect(report).toContainText("dual_momentum@2.3.1");
  await expect(report).toContainText("krx-eod@2026-01-31");
  await expect(report).toContainText("selector@1.4.0");
  await expect(report).toContainText("ACTIVE");
  await expect(report).toContainText("Strategy-based proposal, not investment advice");
  await expect(page.getByRole("region", { name: "Recommendation history" })).toBeVisible();
});

test("invalid parameters and blocked entitlement fail closed without proprietary rows", async ({
  page,
  request,
}) => {
  // Given
  await page.goto("/strategies");
  const configuration = page.getByRole("form", { name: "Configure Dual momentum" });

  // When
  await configuration.getByLabel("Lookback months").fill("0");
  await configuration.getByRole("button", { name: "Save strategy configuration" }).click();

  // Then
  await expect(configuration.getByRole("alert")).toContainText(
    "Lookback months must be between 1 and 24",
  );

  // When
  await setScenario(request, { entitlement: "blocked" });
  await page.goto("/recommendations");

  // Then
  await expect(page.getByRole("alert")).toContainText("Recommendation data is blocked");
  await expect(page.getByText("069500.KRX")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Generate strategy proposal" })).toHaveCount(0);
});

test("an empty exclusion set and stale result remain explicit", async ({ page, request }) => {
  // Given
  await setScenario(request, { exclusions: "empty", recommendation: "stale" });

  // When
  await page.goto("/recommendations");

  // Then
  await expect(page.getByRole("status")).toContainText("Stale result");
  await expect(page.getByText("No instruments were excluded.")).toBeVisible();
  await expect(page.getByText("As of Jan 31, 2026")).toBeVisible();
});
