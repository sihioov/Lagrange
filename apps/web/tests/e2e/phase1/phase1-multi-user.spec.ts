import { type APIRequestContext, expect, test } from "@playwright/test";

const syntheticApiOrigin = process.env["SYNTHETIC_API_ORIGIN"] ?? "http://127.0.0.1:38180";
const USERS = ["u1", "u2", "u3", "u4", "u5"] as const;

function userIdFor(user: string): string {
  const idx = Number(user.slice(1));
  return `00000000-0000-4000-8000-${"00000000000"}${idx}`;
}

// Mirrors the identity stamping the synthetic fixtures apply: u1 keeps the
// canonical id, later identities carry their index one digit above the trailing
// run number.
function runIdFor(user: string, canonical: string): string {
  const idx = Number(user.slice(1));
  return idx === 1 ? canonical : `${canonical.slice(0, -4)}${idx}${canonical.slice(-3)}`;
}

function recommendationRunIdFor(user: string): string {
  return runIdFor(user, "00000000-0000-4000-8000-000000000201");
}

function backtestRunIdFor(user: string): string {
  return runIdFor(user, "00000000-0000-4000-8000-000000000301");
}

async function setScenario(
  request: APIRequestContext,
  scenario: Record<string, string>,
): Promise<void> {
  const response = await request.post(`${syntheticApiOrigin}/__test/scenario`, {
    data: scenario,
  });
  expect(response.ok()).toBe(true);
}

test("five invited members independently run and retrieve their own recommendation outputs", async ({
  page,
  request,
}) => {
  for (const user of USERS) {
    await setScenario(request, {
      user,
      role: "member",
      entitlement: "active",
      exclusions: "present",
      recommendation: "fresh",
    });

    // The identity switch is real: the API resolves this user's own id.
    const session = await request.get(`${syntheticApiOrigin}/api/v1/auth/session`);
    expect(session.ok()).toBe(true);
    const sessionBody = (await session.json()) as { user_id: string; role: string };
    expect(sessionBody.user_id).toBe(userIdFor(user));
    expect(sessionBody.role).toBe("member");

    // This identity independently configures and runs a recommendation.
    await page.goto("/strategies");
    await expect(page.getByRole("heading", { level: 1, name: "Strategies" })).toBeVisible();
    const configuration = page.getByRole("form", { name: "Configure Relative momentum" });
    await configuration.getByLabel("Lookback months").fill("12");
    await configuration.getByLabel("Top holdings").fill("4");
    await configuration.getByRole("button", { name: "Save strategy configuration" }).click();
    await expect(page.getByRole("status")).toContainText("Configuration saved");

    await page.getByRole("link", { name: "Recommendations" }).click();
    const runForm = page.getByRole("form", { name: "Generate recommendation" });
    await runForm.getByLabel("As-of date").fill("2026-01-31");
    await runForm.getByRole("button", { name: "Generate strategy proposal" }).click();

    // The member retrieves the run and sees ONLY their own run id in history.
    const report = page.getByRole("region", { name: "Strategy-based proposal" });
    await expect(report.getByText("069500.KRX")).toBeVisible();
    const history = page.getByRole("table", { name: "Recommendation run history" });
    await expect(history.getByText(recommendationRunIdFor(user))).toBeVisible();
    for (const other of USERS.filter((candidate) => candidate !== user)) {
      await expect(history.getByText(recommendationRunIdFor(other))).toHaveCount(0);
    }
  }
});

test("five invited members independently create and read backtest runs", async ({
  page,
  request,
}) => {
  for (const user of USERS) {
    await setScenario(request, {
      user,
      role: "member",
      entitlement: "active",
      backtest: "running",
      tradePagination: "normal",
    });

    const session = await request.get(`${syntheticApiOrigin}/api/v1/auth/session`);
    const sessionBody = (await session.json()) as { user_id: string };
    expect(sessionBody.user_id).toBe(userIdFor(user));

    await page.goto("/backtests");
    await expect(page.getByRole("heading", { level: 1, name: "Backtests" })).toBeVisible();
    const createForm = page.getByRole("form", { name: "Create backtest" });
    await createForm.getByLabel("Start date").fill("2020-01-02");
    await createForm.getByLabel("End date").fill("2025-12-31");
    await createForm.getByLabel("Initial cash (KRW)").fill("100000000");
    await createForm.getByRole("button", { name: "Create backtest" }).click();
    await expect(page.getByRole("status")).toContainText("Backtest queued");

    // The member reads back only their own runs in the history table.
    const history = page.getByRole("table", { name: "Backtest jobs and result availability" });
    await expect(history.getByText(backtestRunIdFor(user))).toBeVisible();
    for (const other of USERS.filter((candidate) => candidate !== user)) {
      await expect(history.getByText(backtestRunIdFor(other))).toHaveCount(0);
    }
  }
});

test("member sessions cannot reach the Owner admin workspace while owner can", async ({
  page,
  request,
}) => {
  await setScenario(request, { user: "u3", role: "member", entitlement: "active" });
  await page.goto("/admin");
  await expect(page.getByText("Owner access required")).toBeVisible();
  await expect(page.getByText("Choose an administrative area")).toHaveCount(0);

  await setScenario(request, { user: "u3", role: "owner", entitlement: "active" });
  await page.goto("/admin");
  await expect(page.getByRole("heading", { level: 1, name: "Administration" })).toBeVisible();
  await expect(page.getByText("Owner access required")).toHaveCount(0);
});

test("blocked entitlement keeps every member KR-derived surface denied", async ({
  page,
  request,
}) => {
  for (const user of USERS) {
    await setScenario(request, { user, role: "member", entitlement: "blocked" });

    // Name the panel: Next's route announcer is also role="alert", so an
    // unqualified lookup is ambiguous once client-side navigation has happened.
    await page.goto("/recommendations");
    await expect(page.getByRole("alert", { name: "Recommendation data is blocked" })).toBeVisible();
    await expect(page.getByText("069500.KRX")).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Generate strategy proposal" })).toHaveCount(0);

    await page.goto("/backtests");
    await expect(page.getByRole("alert", { name: "Backtest data is blocked" })).toBeVisible();
    await expect(page.getByRole("form", { name: "Create backtest" })).toHaveCount(0);
    await expect(page.getByText("069500.KRX")).toHaveCount(0);
  }
});
