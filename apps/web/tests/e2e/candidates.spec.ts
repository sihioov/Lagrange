import { expect, test } from "@playwright/test";

const syntheticApiOrigin = process.env["SYNTHETIC_API_ORIGIN"] ?? "http://127.0.0.1:38180";

async function setScenario(
  request: import("@playwright/test").APIRequestContext,
  scenario: Record<string, string>,
): Promise<void> {
  const response = await request.post(`${syntheticApiOrigin}/__test/scenario`, { data: scenario });
  expect(response.ok()).toBe(true);
}

test.beforeEach(async ({ request }) => {
  await setScenario(request, { candidateState: "ready", user: "u1" });
});

test("daily Top 5 links to deterministic deep analysis without forecast copy", async ({ page }) => {
  await page.goto("/candidates");

  const feed = page.getByRole("region", { name: "Evidence-ranked stock candidates" });
  await expect(feed.getByRole("row")).toHaveCount(6);
  await expect(feed).toContainText("Common daily Top 5");
  await expect(feed).toContainText("Point-in-time provenance");
  await feed.getByRole("link", { name: /Synthetic 1/ }).click();

  await expect(page).toHaveURL(/\/stocks\/005930\.KRX\?date=2026-08-14&universe=kospi200$/);
  const analysis = page.getByRole("region", { name: "Synthetic 1" });
  await expect(analysis).toContainText("Financial-company profile");
  await expect(analysis).toContainText("Foreign & institution flow");
  await expect(analysis).toContainText("상승 경로");
  await expect(analysis).toContainText("중립 경로");
  await expect(analysis).toContainText("하락 경로");
  await expect(analysis).not.toContainText(/\b\d+(?:\.\d+)?% chance\b/i);
  await expect(analysis).toContainText("not probabilities or target prices");
});

test("screener filters one immutable run and saves private criteria", async ({ page }) => {
  await page.goto("/screener");
  const controls = page.getByRole("region", { name: "Screen the governed universe" });
  await controls.getByLabel("Sector codes").fill("G40");
  await controls.getByLabel("Minimum total score").fill("70");
  await controls.getByLabel("STRONG").check();
  await controls.getByRole("button", { name: "Apply screen" }).click();

  await expect(page).toHaveURL(/sectors=G40/);
  const results = page.getByRole("region", { name: "Screen results" });
  await expect(results.getByRole("row")).toHaveCount(2);
  await expect(results).toContainText("005930.KRX");
  await expect(results).not.toContainText("005931.KRX");

  const save = page.getByRole("form", { name: "Save current screen" });
  await save.getByLabel("Screen name").fill("My financial flow screen");
  await save.getByRole("button", { name: "Save current criteria" }).click();
  await expect(page.getByRole("status")).toContainText("Saved “My financial flow screen”.");
});

test("mobile stale and blocked states stay explicit and never leak rows", async ({
  page,
  request,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await setScenario(request, { candidateState: "stale" });
  await page.goto("/candidates");
  await expect(page.getByRole("status")).toContainText("Stale research snapshot");
  await expect(page.getByRole("table", { name: /Daily candidates/ })).toBeVisible();

  await setScenario(request, { candidateState: "blocked" });
  await page.goto("/candidates");
  await expect(page.getByRole("heading", { name: "Candidate research is blocked" })).toBeVisible();
  await expect(page.getByText("Synthetic 1")).toHaveCount(0);
  await expect(page.getByText(SHA)).toHaveCount(0);
});

const SHA = "a".repeat(64);
