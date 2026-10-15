import { expect, test } from "@playwright/test";
import { appAlert } from "./support/alerts";

const syntheticApiOrigin = process.env["SYNTHETIC_API_ORIGIN"] ?? "http://127.0.0.1:38180";

async function setScenario(
  request: import("@playwright/test").APIRequestContext,
  scenario: Record<string, string>,
): Promise<void> {
  const response = await request.post(`${syntheticApiOrigin}/__test/scenario`, { data: scenario });
  expect(response.ok()).toBe(true);
}

test("owner reads only the sealed price-return recommendation surface", async ({
  page,
  request,
}) => {
  await setScenario(request, {
    entitlement: "active",
    ownerBetaAccessMode: "owner_only",
    role: "owner",
  });

  await page.goto("/recommendations");

  await expect(
    page.getByRole("heading", { level: 1, name: "Owner-only recommendations" }),
  ).toBeVisible();
  const warnings = page.getByRole("status", { name: "Recommendation warnings" }).first();
  await expect(warnings).toContainText("Owner-only");
  await expect(warnings).toContainText("Price-return only");
  await expect(warnings).toContainText("Vendor snapshot");
  await expect(warnings).toContainText("Non-strict PIT");
  const report = page.getByRole("region", {
    name: "Owner-only recommendation",
    exact: true,
  });
  await expect(report.getByText("069500.KRX")).toBeVisible();
  await expect(report).toContainText("Selected under the selection criteria");
  await expect(report).toContainText("Cash allocation: 20.00%");
  await expect(page.getByText("Synthetic QA data")).toHaveCount(0);
});

test("member cannot enumerate owner-beta recommendations", async ({ page, request }) => {
  await setScenario(request, {
    entitlement: "active",
    ownerBetaAccessMode: "owner_only",
    role: "member",
  });

  await page.goto("/recommendations");

  await expect(appAlert(page)).toContainText("Owner access required");
  await expect(page.getByText("069500.KRX")).toHaveCount(0);
  await expect(page.getByText("Price-return only")).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Generate owner-only recommendation" }),
  ).toHaveCount(0);
});

test("owner keeps the beta contract labels when entitlement blocks payloads", async ({
  page,
  request,
}) => {
  await setScenario(request, {
    entitlement: "blocked",
    ownerBetaAccessMode: "owner_only",
    role: "owner",
  });

  await page.goto("/recommendations");

  const warnings = page.getByRole("status", { name: "Recommendation warnings" });
  await expect(warnings).toContainText("Owner-only");
  await expect(warnings).toContainText("Price-return only");
  await expect(warnings).toContainText("Vendor snapshot");
  await expect(warnings).toContainText("Non-strict PIT");
  await expect(appAlert(page)).toContainText("Recommendation data is blocked");
  await expect(page.getByText("069500.KRX")).toHaveCount(0);
});

const DAILY_CONFIG_ID = "00000000-0000-4000-8000-000000000101";
const SECOND_DAILY_CONFIG_ID = "00000000-0000-4000-8000-000000000102";
const DAILY_PATH = "/api/v1/recommendations/owner-beta/price-only/daily-state";
const OWNER_DAILY = {
  entitlement: "active",
  ownerBetaAccessMode: "owner_only",
  role: "owner",
};

test("daily configuration creation requires an explicit unchecked opt-in", async ({
  page,
  request,
}) => {
  await setScenario(request, OWNER_DAILY);
  await page.goto("/strategies");
  const form = page.getByRole("form", { name: "Configure Relative momentum" });
  const checkbox = form.getByRole("checkbox", { name: /^Automatic daily recommendations/ });
  await expect(checkbox).not.toBeChecked();
  const posted = page.waitForRequest(
    (value) =>
      value.method() === "POST" && value.url().endsWith("/strategies/relative_momentum/configs"),
  );
  await form.getByRole("button", { name: "Save strategy configuration" }).click();
  const create = await posted;
  expect(create.postDataJSON()).toEqual({
    config: { lookback_months: 12, top_n: 3 },
    is_active: true,
    owner_beta_daily_enabled: false,
    strategy_version: "1.0.0",
  });
  await expect(form.getByRole("status")).toContainText("Configuration saved");
  await expect(page.getByRole("switch").first()).toHaveAttribute("aria-checked", "false");
  await checkbox.check();
  const optedIn = page.waitForRequest(
    (value) =>
      value.method() === "POST" && value.url().endsWith("/strategies/relative_momentum/configs"),
  );
  await form.getByRole("button", { name: "Save strategy configuration" }).click();
  expect((await optedIn).postDataJSON().owner_beta_daily_enabled).toBe(true);
  await expect(page.getByRole("switch").first()).toHaveAttribute("aria-checked", "true");
});

test("daily toggle persists only the selected preference and keeps runtime-off distinct", async ({
  page,
  request,
}) => {
  await setScenario(request, OWNER_DAILY);
  await page.goto(`/recommendations?daily_config_id=${DAILY_CONFIG_ID}`);
  const toggle = page.getByRole("switch", { name: /^Automatic daily recommendations/ });
  await expect(toggle).toHaveAttribute("aria-checked", "false");
  const pending = page.waitForRequest(
    (value) =>
      value.method() === "PATCH" && value.url().endsWith(`/strategy-configs/${DAILY_CONFIG_ID}`),
  );
  await toggle.click();
  const mutation = await pending;
  expect(mutation.postDataJSON()).toEqual({ owner_beta_daily_enabled: true });
  expect(mutation.headers()["x-csrf-token"]).toBeTruthy();
  expect(mutation.headers()["idempotency-key"]).toBeTruthy();
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  const panel = page.getByRole("region", { name: "Daily recommendation status" });
  await expect(panel).toContainText("Daily execution is disabled for this release or service");
  await page.reload();
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-checked", "false");
});

test("daily preference rejection keeps the confirmed setting", async ({ page, request }) => {
  await setScenario(request, { ...OWNER_DAILY, dailyPatch: "denied" });
  await page.goto("/recommendations");
  const toggle = page.getByRole("switch", { name: /^Automatic daily recommendations/ });
  await expect(toggle).toHaveAttribute("aria-checked", "false");
  await toggle.click();
  await expect(appAlert(page)).toContainText("The setting could not be confirmed");
  await expect(toggle).toHaveAttribute("aria-checked", "false");
  await expect(toggle).toBeEnabled();
  expect(page.url()).not.toContain("/login");
});

test("daily result survives missing history discovery with four distinct dates", async ({
  page,
  request,
}) => {
  await setScenario(request, {
    ...OWNER_DAILY,
    dailyDiscovery: "unavailable",
    dailyEnabled: "true",
    dailyRuntime: "enabled",
    dailyState: "current",
  });
  await page.goto("/recommendations");
  const panel = page.getByRole("region", { name: "Daily recommendation status" });
  await expect(panel).toContainText("Expected session published");
  for (const label of [
    "Expected trading session",
    "Daily input coverage ends",
    "Latest daily result as-of",
    "Published at (KST)",
  ]) {
    await expect(panel.getByText(label, { exact: true })).toBeVisible();
  }
  await expect(panel).toContainText("Oct 15, 2026");
  await expect(panel).toContainText("5:00 PM KST");
  await expect(appAlert(page)).toContainText("Manual historical runs unavailable");
  const report = page.getByRole("region", { name: "Owner-only recommendation", exact: true });
  await expect(report).toContainText("Automatic daily run");
  await expect(report.getByText("069500.KRX")).toBeVisible();
  await report.getByText("Audit details", { exact: true }).click();
  await expect(report).toContainText("Daily admission commitment");
  await expect(report).toContainText("Historical base commitments");
});

test("daily refresh preserves results through blocked stale and unavailable states", async ({
  page,
  request,
}) => {
  const scenario = { ...OWNER_DAILY, dailyEnabled: "true", dailyRuntime: "enabled" };
  await setScenario(request, { ...scenario, dailyState: "current" });
  await page.goto("/recommendations");
  const panel = page.getByRole("region", { name: "Daily recommendation status" });
  const report = page.getByRole("region", { name: "Owner-only recommendation", exact: true });
  await expect(panel).toContainText("Expected session published");
  for (const [state, label] of [
    ["blocked", "Input blocked"],
    ["stale", "Update overdue"],
    ["unavailable", "Current status not verified"],
  ] as const) {
    await setScenario(request, { ...scenario, dailyState: state });
    await panel.getByRole("button", { name: "Refresh daily status" }).click();
    await expect(panel).toContainText(label);
    await expect(panel).not.toContainText("Expected session published");
    await expect(report.getByText("069500.KRX")).toBeVisible();
    await expect(report).toContainText("2026-10-15");
    await expect(panel).toContainText("Oct 15, 2026");
  }
});

test("daily pending and failed runs retain a prior result across paginated history", async ({
  page,
  request,
}) => {
  for (const state of ["queued", "failed"] as const) {
    await setScenario(request, {
      ...OWNER_DAILY,
      dailyEnabled: "true",
      dailyHistory: "paged",
      dailyRuntime: "enabled",
      dailyState: state,
    });
    await page.goto("/recommendations");
    const panel = page.getByRole("region", { name: "Daily recommendation status" });
    await expect(panel).toContainText(state === "queued" ? "Queued" : "Run failed");
    await expect(page.getByText("Previous successful result", { exact: true })).toBeVisible();
    const report = page.getByRole("region", { name: "Owner-only recommendation", exact: true });
    await expect(report).toContainText("2026-10-15");
    await expect(report.getByText("069500.KRX")).toBeVisible();
  }
  await setScenario(request, {
    ...OWNER_DAILY,
    dailyEnabled: "true",
    dailyHistory: "manual",
    dailyRuntime: "enabled",
    dailyState: "queued",
  });
  await page.goto("/recommendations");
  const report = page.getByRole("region", { name: "Owner-only recommendation", exact: true });
  await expect(report).toContainText("Manual historical run");
  const panel = page.getByRole("region", { name: "Daily recommendation status" });
  await expect(panel).toContainText("Queued");
  await expect(panel).not.toContainText("Expected session published");
  await expect(panel).not.toContainText("Aug 19, 2026");
});

test("daily configuration switch isolates dates and late client observations", async ({
  page,
  request,
}) => {
  await setScenario(request, {
    ...OWNER_DAILY,
    dailyConfigs: "two",
    dailyEnabled: "true",
    dailyRuntime: "enabled",
    dailyState: "current",
  });
  await page.goto("/recommendations");
  const panel = page.getByRole("region", { name: "Daily recommendation status" });
  await expect(panel).toContainText("Expected session published");
  let release: (() => void) | undefined;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  let reached: (() => void) | undefined;
  const intercepted = new Promise<void>((resolve) => {
    reached = resolve;
  });
  let finished: (() => void) | undefined;
  const delivered = new Promise<void>((resolve) => {
    finished = resolve;
  });
  await page.route(`**${DAILY_PATH}?strategy_config_id=${DAILY_CONFIG_ID}`, async (route) => {
    reached?.();
    await gate;
    try {
      await route.continue();
    } catch {
      // Navigation cancels the old configuration request.
    } finally {
      finished?.();
    }
  });
  await panel.getByRole("button", { name: "Refresh daily status" }).click();
  await intercepted;
  try {
    await page
      .getByLabel("Select a strategy configuration.", { exact: true })
      .selectOption(SECOND_DAILY_CONFIG_ID);
    await page.getByRole("button", { name: "View this configuration" }).click();
    await expect(page).toHaveURL(new RegExp(`daily_config_id=${SECOND_DAILY_CONFIG_ID}`));
    await expect(panel).toContainText("Automatic updates off");
  } finally {
    release?.();
  }
  await delivered;
  await expect(panel).not.toContainText("Oct 15, 2026");
  await expect(page.getByRole("switch")).toHaveAttribute("aria-checked", "false");
  await expect(
    page.getByRole("region", { name: "Owner-only recommendation", exact: true }),
  ).toHaveCount(0);
});

test("daily status session expiry returns to login without inventing a fresh result", async ({
  page,
  request,
}) => {
  await setScenario(request, {
    ...OWNER_DAILY,
    dailyEnabled: "true",
    dailyRuntime: "enabled",
    dailyState: "current",
  });
  await page.goto("/recommendations");
  const panel = page.getByRole("region", { name: "Daily recommendation status" });
  await expect(panel).toContainText("Expected session published");
  await setScenario(request, { ...OWNER_DAILY, authSession: "expired" });
  await panel.getByRole("button", { name: "Refresh daily status" }).click();
  await expect(page).toHaveURL(/\/auth\/login$/);
});
