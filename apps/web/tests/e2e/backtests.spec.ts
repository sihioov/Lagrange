import { type APIRequestContext, expect, test } from "@playwright/test";
import { appAlert } from "./support/alerts";

const syntheticApiOrigin = process.env["SYNTHETIC_API_ORIGIN"] ?? "http://127.0.0.1:38180";

async function setScenario(
  request: APIRequestContext,
  scenario: Record<string, string>,
): Promise<void> {
  const response = await request.post(`${syntheticApiOrigin}/__test/scenario`, {
    data: scenario,
  });
  expect(response.ok()).toBe(true);
}

test.beforeEach(async ({ request }) => {
  await setScenario(request, {
    backtest: "running",
    entitlement: "active",
    tradePagination: "normal",
  });
});

test("member creates, monitors, cancels, compares, and reads server-produced backtests", async ({
  page,
}) => {
  // Given
  await page.goto("/backtests");
  await expect(page.getByRole("heading", { level: 1, name: "Backtests" })).toBeVisible();
  const createForm = page.getByRole("form", { name: "Create backtest" });

  // When
  await createForm.getByLabel("Start date").fill("2020-01-02");
  await createForm.getByLabel("End date").fill("2025-12-31");
  await createForm.getByLabel("Initial cash (KRW)").fill("100000000");
  await createForm.getByRole("button", { name: "Create backtest" }).click();

  // Then
  await expect(page.getByRole("status")).toContainText("Backtest queued");
  const progress = page.getByRole("region", { name: "Backtest progress" });
  await expect(progress).toContainText("RUNNING");
  await expect(progress).toContainText("65% complete");

  // When
  await progress.getByRole("button", { name: "Cancel backtest" }).click();

  // Then
  await expect(progress.getByRole("status")).toContainText("Cancellation requested");

  // When
  const comparisonForm = page.getByRole("form", { name: "Compare backtest runs" });
  await comparisonForm.getByLabel("Dual momentum baseline · You", { exact: true }).check();
  await comparisonForm.getByLabel("Dual momentum higher costs · You", { exact: true }).check();
  await comparisonForm.getByRole("button", { name: "Compare selected runs" }).click();

  // Then
  const comparison = page.getByRole("region", { name: "Run comparison" });
  await expect(comparison).toContainText("Total return delta");
  await expect(comparison).toContainText("−3.21%");

  const report = page.getByRole("region", { name: "Backtest result" });
  await expect(report.getByRole("heading", { name: "Equity and drawdown" })).toBeVisible();
  await expect(report.getByRole("heading", { name: "Monthly returns" })).toBeVisible();
  await expect(report.getByRole("heading", { name: "Trades and costs" })).toBeVisible();
  await expect(report).toContainText("₩128,450,000.00");
  await expect(report).toContainText("−18.42%");
  await expect(report).toContainText("₩128,450.00");
  await expect(report).toContainText("dual_momentum@2.3.1");
  await expect(report).toContainText("krx-eod@2025-12-31");
  await expect(report).toContainText("nautilus@1.231.0");
  await expect(report).toContainText(
    "Next-open execution can differ from close-to-close benchmarks.",
  );

  // When
  await report.getByRole("button", { name: "Run robustness evidence" }).click();

  // Then
  await expect(report.getByRole("status")).toContainText("Robustness queued");
  const robustness = page.getByRole("region", { name: "Robustness evidence" });
  await expect(robustness).toContainText("Parameter sensitivity");
  await expect(robustness).toContainText("Cost stress");
  await expect(robustness).toContainText("Validation periods");
  await expect(robustness).toContainText("Concentrated in three trades");
});

test("blocked entitlement prevents creation and hides proprietary backtest payloads", async ({
  page,
  request,
}) => {
  // Given
  await setScenario(request, { entitlement: "blocked" });

  // When
  await page.goto("/backtests");

  // Then
  await expect(appAlert(page)).toContainText("Backtest data is blocked");
  await expect(page.getByRole("form", { name: "Create backtest" })).toHaveCount(0);
  await expect(page.getByText("₩128,450,000.00")).toHaveCount(0);
  await expect(page.getByText("069500.KRX")).toHaveCount(0);
});

test("failed and canceled jobs stay explicit while a large trade page remains usable", async ({
  page,
  request,
}) => {
  // Given
  await setScenario(request, { backtest: "failed-canceled", tradePagination: "huge" });

  // When
  await page.goto("/backtests");

  // Then
  await expect(appAlert(page)).toContainText("Backtest failed");
  await expect(
    page.getByRole("row", { name: /Canceled member run.*CANCELED/ }).first(),
  ).toBeVisible();
  await expect(
    page.getByText("Canceled and failed runs do not expose result payloads."),
  ).toBeVisible();
  const trades = page.getByRole("region", { name: "Trades and costs" });
  await expect(trades).toContainText("1,200 trades");
  await expect(trades.getByText("Trade 1,200")).toBeVisible();
});
