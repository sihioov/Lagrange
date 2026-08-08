import { expect, test } from "@playwright/test";

const syntheticApiOrigin = process.env["SYNTHETIC_API_ORIGIN"] ?? "http://127.0.0.1:38180";

async function setScenario(
  request: import("@playwright/test").APIRequestContext,
  scenario: Record<string, string>,
): Promise<void> {
  const response = await request.post(`${syntheticApiOrigin}/__test/scenario`, { data: scenario });
  expect(response.ok()).toBe(true);
}

const owner = { liveMfa: "fresh", role: "owner", user: "u1" } as const;

/**
 * The kill-switch panel's own result line.
 *
 * Scoped to the panel on purpose: Next.js injects a page-level
 * `<div role="alert" id="__next-route-announcer__">` for route changes, so an
 * unscoped `getByRole("alert")` always resolves to that empty announcer and
 * every assertion against it fails -- or, worse, a `toHaveCount(0)` passes for
 * the wrong reason. The panel's own alert is the one that carries the refusal.
 */
function killSwitchPanel(page: import("@playwright/test").Page) {
  return page.getByRole("region", { name: "Kill switch" });
}

/**
 * Todo 41: the Live kill switch, at the surface an operator actually uses.
 *
 * The asymmetry between the two directions is what this file exists to prove,
 * and it is not a UI preference — it is the safety property:
 *
 *   * ENGAGING stops Live. It must never be blocked, gated, or slowed by a
 *     precondition, because a precondition on stopping is one that fails at
 *     the worst possible moment.
 *   * DISENGAGING permits real orders against a real account. It requires a
 *     typed reason, a fresh second factor, AND a green reconciliation, because
 *     re-enabling Live while our books disagree with the broker's is the
 *     specific way a kill switch causes the incident it was installed to
 *     prevent.
 *
 * The page renders ENGAGED unconditionally today (there is no read route for
 * the state yet), so every test here starts from the stopped state — which is
 * also the state a fresh system boots into.
 */
test.describe("the Live kill switch", () => {
  test.beforeEach(async ({ request }) => {
    await setScenario(request, { ...owner, reconciliation: "never" });
  });

  test("engaging is a single click with no reason and no preconditions", async ({ page }) => {
    // This scenario has never reconciled, which blocks DISENGAGING. Engaging
    // must still work: that is the whole point of the asymmetry.
    await page.goto("/live");
    const panel = page.getByRole("region", { name: "Kill switch" });
    await expect(panel).toBeVisible();

    // The page boots stopped, so the form on offer is the disengage one. To
    // reach the engage form the switch must first be off; what is asserted
    // here instead is the server contract that engaging carries no
    // precondition, exercised directly.
    const response = await page.request.post("/api/v1/admin/live/kill-switch/enable", {
      data: {},
    });
    expect(response.status()).toBe(200);
    expect(await response.json()).toMatchObject({ engaged: true });
  });

  test("disengaging is refused, and says what to do, when nothing has reconciled", async ({
    page,
  }) => {
    await page.goto("/live");
    const form = page.getByRole("form", { name: "Disengage kill switch" });
    await expect(form).toBeVisible();

    await form.getByLabel("Reason for disengaging").fill("resuming after maintenance");
    await form.getByRole("button", { name: "Disengage kill switch" }).click();

    // An alert, not a status: this refused.
    const result = killSwitchPanel(page).getByRole("alert");
    await expect(result).toBeVisible();

    // The operator is told the NEXT ACTION, not handed a code to look up.
    // "LIVE_RECONCILIATION_REQUIRED" on a kill-switch page leaves a reader
    // guessing what reconciliation has to do with it.
    await expect(result).toContainText(/reconciliation run finishes green/i);
    await expect(result).toContainText(/resolve any mismatch/i);
    await expect(result).not.toContainText("LIVE_RECONCILIATION_REQUIRED");

    // And Live is still stopped: the form to disengage is still the one shown.
    await expect(form).toBeVisible();
  });

  test("a reconciliation that found a mismatch is not permission either", async ({
    page,
    request,
  }) => {
    // A run that COMPLETED but disagreed with the broker. The distinction from
    // "never ran" matters to an operator and neither one may re-enable Live.
    await setScenario(request, { ...owner, reconciliation: "mismatch" });
    await page.goto("/live");

    const form = page.getByRole("form", { name: "Disengage kill switch" });
    await form.getByLabel("Reason for disengaging").fill("I checked, it is fine");
    await form.getByRole("button", { name: "Disengage kill switch" }).click();

    await expect(killSwitchPanel(page).getByRole("alert")).toContainText(
      /reconciliation run finishes green/i,
    );
  });

  test("a reconciliation still running is not permission either", async ({ page, request }) => {
    // "We do not know yet" is not a yes. Treating an in-progress run as
    // permission would make the block escapable by clicking during one.
    await setScenario(request, { ...owner, reconciliation: "running" });
    await page.goto("/live");

    const form = page.getByRole("form", { name: "Disengage kill switch" });
    await form.getByLabel("Reason for disengaging").fill("it is probably fine");
    await form.getByRole("button", { name: "Disengage kill switch" }).click();

    await expect(killSwitchPanel(page).getByRole("alert")).toContainText(
      /reconciliation run finishes green/i,
    );
  });

  test("a green reconciliation lets the Owner disengage, and records the reason", async ({
    page,
    request,
  }) => {
    await setScenario(request, { ...owner, reconciliation: "green" });
    await page.goto("/live");

    const form = page.getByRole("form", { name: "Disengage kill switch" });
    await form.getByLabel("Reason for disengaging").fill("reconciled and cleared for trading");
    await form.getByRole("button", { name: "Disengage kill switch" }).click();

    // A status, not an alert: this succeeded.
    const panel = killSwitchPanel(page);
    const result = panel.getByRole("status");
    await expect(result).toBeVisible();
    await expect(result).toContainText(/disengaged/i);
    await expect(panel.getByRole("alert")).toHaveCount(0);
  });

  test("disengaging cannot be submitted without a typed reason", async ({ page, request }) => {
    // The friction is deliberate and lands in the audit trail. A green
    // reconciliation does not remove it: the button stays disabled until a
    // reason exists, so the requirement cannot be satisfied by reconciling.
    await setScenario(request, { ...owner, reconciliation: "green" });
    await page.goto("/live");

    const form = page.getByRole("form", { name: "Disengage kill switch" });
    const submit = form.getByRole("button", { name: "Disengage kill switch" });
    await expect(submit).toBeDisabled();

    // Whitespace is not a reason.
    await form.getByLabel("Reason for disengaging").fill("   ");
    await expect(submit).toBeDisabled();

    await form.getByLabel("Reason for disengaging").fill("cleared");
    await expect(submit).toBeEnabled();
  });

  test("a stale second factor withholds the control entirely, green or not", async ({
    page,
    request,
  }) => {
    // The preconditions are independent, not alternatives: reconciling green
    // must not substitute for proving who is at the keyboard. This scenario is
    // green, so the ONLY thing standing between this session and a live
    // trading system is the age of its second factor.
    await setScenario(request, {
      liveMfa: "stale",
      reconciliation: "green",
      role: "owner",
      user: "u1",
    });
    await page.goto("/live");

    // The control is not offered at all. That is stronger than offering it and
    // refusing on submit: a control that renders invites the click, and every
    // refusal after that point is one more thing that has to keep working.
    await expect(killSwitchPanel(page)).toHaveCount(0);
    await expect(page.getByRole("form", { name: "Disengage kill switch" })).toHaveCount(0);

    // The page says what to do, rather than that something went wrong.
    await expect(page.getByText(/re-authenticate/i)).toBeVisible();

    // And the guard is NOT only in the UI: the route refuses the same session
    // directly, so hiding the form is a courtesy rather than the mechanism.
    const response = await page.request.post("/api/v1/admin/live/kill-switch/disable", {
      data: { reason: "cleared for trading" },
      failOnStatusCode: false,
    });
    expect(response.status()).toBe(403);
    const body = (await response.json()) as { error: { code: string } };
    expect(body.error.code).toMatch(/^STEP_UP_/);
    // The MFA refusal is reported, NOT the reconciliation one -- and here that
    // is trivially true, because this scenario IS reconciled. Sending an
    // operator to go reconcile when the real problem is their session would
    // have them fix something that is already fine.
    expect(body.error.code).not.toBe("LIVE_RECONCILIATION_REQUIRED");
  });

  test("a Member cannot reach the kill switch at all", async ({ page, request }) => {
    // Todo 37's boundary, re-asserted here because this file is where someone
    // will look for it: the control is absent, and the route answers as though
    // it was never built.
    await setScenario(request, {
      liveMfa: "fresh",
      reconciliation: "green",
      role: "member",
      user: "u2",
    });
    await page.goto("/live");

    await expect(page.getByRole("region", { name: "Kill switch" })).toHaveCount(0);
    await expect(page.getByRole("button", { name: /kill switch/i })).toHaveCount(0);

    const response = await page.request.post("/api/v1/admin/live/kill-switch/enable", {
      data: {},
      failOnStatusCode: false,
    });
    // 404, never 403: a 403 would confirm the route exists.
    expect(response.status()).toBe(404);
  });
});
