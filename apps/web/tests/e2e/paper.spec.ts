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

const BASELINE = {
  notification: "delivered",
  paperAccount: "present",
  paperEntitlement: "active",
  parity: "match",
  user: "u1",
};

test.beforeEach(async ({ request }) => {
  await setScenario(request, BASELINE);
});

test("member reads ledger-derived performance, matching parity, and the fill-model difference", async ({
  page,
}) => {
  // Given
  await page.goto("/paper");
  await expect(page.getByRole("heading", { level: 1, name: "Paper account" })).toBeVisible();

  // Then — account identity and holdings come from the server.
  const holdings = page.getByRole("region", { name: "Account and holdings" });
  await expect(holdings).toContainText("Paper account 1");
  await expect(holdings).toContainText("KRX_ETF_DEFAULT");
  await expect(holdings.getByRole("rowheader", { name: "069500.KRX" })).toBeVisible();
  await expect(holdings).toContainText("paper-2026-02-02-069500");

  // Then — performance is exact, ledger-derived, and disclaimed.
  const performance = page.getByRole("region", { name: "Daily performance" });
  await expect(performance).toContainText("Simulated results from a paper account");
  await expect(performance).toContainText("not a guarantee of future returns");
  await expect(performance).toContainText("10042180.0000");
  await expect(performance).toContainText("8047860.0000");
  await expect(performance).toContainText("0.004218");

  // Then — a match is a status, not an alert, but the fill-model difference
  // is still stated so the two executions are never read as interchangeable.
  const parity = page.getByRole("region", { name: "Backtest parity" });
  await expect(parity.getByRole("status")).toContainText("Match");
  await expect(parity).toContainText("Fill model difference");
  await expect(parity).toContainText("modeled at the next session's raw open");
  await expect(parity.getByRole("alert")).toHaveCount(0);

  // Then — strategy/data/target/execution versions are all visible.
  const lineage = page.getByRole("region", { name: "Strategy and target lineage" });
  await expect(lineage).toContainText("dual_momentum");
  await expect(lineage).toContainText("2.3.1");
  await expect(lineage).toContainText("buy_and_hold");
  await expect(lineage).toContainText("Branched");
  await expect(lineage).toContainText("2026-01-30");
  await expect(lineage).toContainText("2026-02-02");
  await expect(lineage).toContainText("EXECUTED");

  // Then — the completion notice carries its delivery outcome.
  const notices = page.getByRole("region", { name: "Session notifications" });
  await expect(notices).toContainText("Paper session 2026-02-02 completed");
  await expect(notices).toContainText("web: SUCCESS");
});

test("a divergence is raised as an alert with its reason and the diverging weights", async ({
  page,
  request,
}) => {
  // Given
  await setScenario(request, { ...BASELINE, parity: "divergent" });

  // When
  await page.goto("/paper");

  // Then
  const parity = page.getByRole("region", { name: "Backtest parity" });
  const alert = parity.getByRole("alert", { name: "Paper parity Divergent" });
  await expect(alert).toContainText("Divergent");
  await expect(alert).toContainText("different target weights for 2 instrument(s)");
  await expect(parity.getByRole("rowheader", { name: "069500.KRX" })).toBeVisible();
  await expect(parity).toContainText("0.900000");
  await expect(parity).toContainText("0.600000");

  // The divergence is also announced, never hidden behind the panel alone.
  await expect(page.getByRole("region", { name: "Session notifications" })).toContainText(
    "diverged from its backtest",
  );
});

test("changed lineage blocks the parity claim instead of asserting a match", async ({
  page,
  request,
}) => {
  // Given
  await setScenario(request, { ...BASELINE, parity: "incomparable" });

  // When
  await page.goto("/paper");

  // Then
  const parity = page.getByRole("region", { name: "Backtest parity" });
  const alert = parity.getByRole("alert", { name: "Paper parity Not comparable" });
  await expect(alert).toContainText("Not comparable");
  await expect(alert).toContainText("different inputs (dataset_version)");
  // The mismatching field is shown with both sides.
  await expect(parity.getByRole("rowheader", { name: "dataset_version" })).toBeVisible();
  await expect(parity).toContainText("krx-eod.2026-01-29");
});

test("a notification outage is recorded on the page, not silent", async ({ page, request }) => {
  // Given
  await setScenario(request, { ...BASELINE, notification: "outage" });

  // When
  await page.goto("/paper");

  // Then
  const notices = page.getByRole("region", { name: "Session notifications" });
  await expect(notices).toContainText("email: FAILED");
  await expect(notices.getByRole("alert")).toContainText(
    "email delivery not configured in this release",
  );
  // The channel that worked is still reported, so the notice is not lost.
  await expect(notices).toContainText("web: SUCCESS");
});

test("rebinding an account branches it rather than rewriting its history", async ({ page }) => {
  // Given
  await page.goto("/paper");
  const form = page.getByRole("form", { name: "Bind strategy" });
  await expect(form).toContainText("Currently bound to dual_momentum@2.3.1");
  await expect(form).toContainText("execution history never mixes strategy versions");

  // When
  await form.getByLabel("Strategy configuration").selectOption({ label: "buy_and_hold@1.0.0" });
  await form.getByRole("button", { name: "Bind strategy" }).click();

  // Then
  await expect(form.getByRole("status")).toContainText("Bound buy_and_hold@1.0.0");
  await expect(form.getByRole("status")).toContainText("earlier sessions keep theirs");
});

test("a blocked paper entitlement renders no account data", async ({ page, request }) => {
  // Given
  await setScenario(request, { ...BASELINE, paperEntitlement: "blocked" });

  // When
  await page.goto("/paper");

  // Then
  await expect(page.getByRole("heading", { name: "Paper data is blocked" })).toBeVisible();
  await expect(page.getByRole("region", { name: "Daily performance" })).toHaveCount(0);
  await expect(page.getByRole("region", { name: "Backtest parity" })).toHaveCount(0);
});

test("invited members can switch to each other's read-only accounts", async ({ page, request }) => {
  // Given — member 1 defaults to their own account.
  await page.goto("/paper");
  await expect(page.getByRole("region", { name: "Account and holdings" })).toContainText(
    "Paper account 1",
  );
  const firstOrder = await page
    .getByRole("region", { name: "Account and holdings" })
    .getByRole("rowheader", { name: "paper-2026-02-02-069500" })
    .isVisible();
  expect(firstOrder).toBe(true);

  // When — the second invited identity loads the same route.
  await setScenario(request, { ...BASELINE, user: "u2" });
  await page.goto("/paper");

  // Then — member 2 defaults to their own account but can open member 1's.
  const holdings = page.getByRole("region", { name: "Account and holdings" });
  await expect(holdings).toContainText("Paper account 2");
  await page
    .getByRole("navigation", { name: "Shared paper accounts" })
    .getByRole("link", { name: "Paper account 1 · Shared" })
    .click();
  await expect(holdings).toContainText("Paper account 1");
  await expect(holdings).toContainText("Shared account · 00000000");
  await expect(page.getByRole("form", { name: "Bind strategy" })).toHaveCount(0);
});
