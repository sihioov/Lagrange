import { expect, test } from "@playwright/test";

const syntheticApiOrigin = process.env["SYNTHETIC_API_ORIGIN"] ?? "http://127.0.0.1:38180";

async function setScenario(
  request: import("@playwright/test").APIRequestContext,
  scenario: Record<string, string>,
): Promise<void> {
  const response = await request.post(`${syntheticApiOrigin}/__test/scenario`, { data: scenario });
  expect(response.ok()).toBe(true);
}

/**
 * Todo 37: Live must be ABSENT for a Member, not hidden from one.
 *
 * The distinction is the whole test file. Hiding a control with CSS, or
 * omitting a nav link while the route still answers, leaves the capability
 * reachable by anyone who types the URL or reads the bundle. What is asserted
 * here is that the link is not generated, the page renders no Live control,
 * and the API answers as though the route does not exist.
 */

test.describe("Live is absent for a Member", () => {
  test.beforeEach(async ({ request }) => {
    await setScenario(request, { liveMfa: "fresh", role: "member", user: "u1" });
  });

  test("the navigation never generates a Live link", async ({ page }) => {
    await page.goto("/");
    const nav = page.getByRole("navigation");
    await expect(nav).toBeVisible();

    // Absent from the DOM entirely - not present-but-hidden. `toHaveCount(0)`
    // fails if the link exists with `display:none`, which a visibility
    // assertion would let pass.
    await expect(nav.getByRole("link", { name: /live/i })).toHaveCount(0);
    await expect(page.locator('a[href="/live"]')).toHaveCount(0);

    // The Member's own surfaces are still there, so this is not a blank page.
    await expect(nav.getByRole("link", { name: "Paper account" })).toBeVisible();
  });

  test("typing the Live URL exposes no control, credential, or configuration", async ({ page }) => {
    await page.goto("/live");

    // The Owner gate renders a refusal, never the workspace.
    await expect(page.getByRole("heading", { name: "Owner access required" })).toBeVisible();

    // No Live control of any kind is reachable.
    await expect(page.getByRole("button", { name: /kill switch/i })).toHaveCount(0);
    await expect(page.getByRole("form", { name: /kill switch/i })).toHaveCount(0);
    await expect(page.getByRole("region", { name: "Broker connections" })).toHaveCount(0);

    // And no Live vocabulary leaks into the page at all - a Member must not
    // learn that connections, credential references, or nodes exist.
    const body = (await page.locator("body").innerText()).toLowerCase();
    for (const leak of ["kis_app", "broker connection", "account_no_masked", "env:"]) {
      expect(body).not.toContain(leak);
    }
  });

  test("the Live API answers a Member as though it does not exist", async ({ page, request }) => {
    // Straight at the API, bypassing the UI entirely.
    const resp = await request.get(`${syntheticApiOrigin}/api/v1/admin/live/connections`);
    expect(resp.status(), "403 would confirm the route exists; it must be 404").toBe(404);
    const body = await resp.json();
    expect(body.error.code).toBe("RESOURCE_NOT_FOUND");

    // The refusal itself carries no Live field names.
    const rendered = JSON.stringify(body);
    for (const leak of ["kis_app_key_ref", "account_no_masked", "profile", "connection"]) {
      expect(rendered).not.toContain(leak);
    }
    await page.close();
  });
});

test.describe("Live for the Owner", () => {
  test("an Owner with fresh MFA sees connections as credential LOCATIONS", async ({
    page,
    request,
  }) => {
    await setScenario(request, { liveMfa: "fresh", role: "owner", user: "u1" });
    await page.goto("/live");

    await expect(page.getByRole("heading", { level: 1, name: "Live controls" })).toBeVisible();
    const connections = page.getByRole("region", { name: "Broker connections" });
    await expect(connections).toContainText("KIS simulator");
    await expect(connections).toContainText("****6-01");

    // Locations are shown; values cannot be, because the server has no field
    // capable of carrying one.
    await expect(connections).toContainText("env:KIS_APP_KEY");
    await expect(connections).toContainText("file:/run/secrets/kis_app_secret");
    await expect(connections).toContainText("Credentials are shown as locations, never values");

    // A mock connection is labelled in words, so it cannot be mistaken for one
    // that places real orders.
    await expect(connections).toContainText("Mock — simulated");
  });

  test("the kill switch starts engaged and disengaging demands a stated reason", async ({
    page,
    request,
  }) => {
    // `reconciliation: "green"` because since Todo 41 disengaging also
    // requires a green reconciliation run. That precondition has its own
    // coverage in live-kill-switch.spec.ts; here it is simply satisfied so
    // this test keeps testing what it is about -- the stated reason.
    await setScenario(request, {
      liveMfa: "fresh",
      reconciliation: "green",
      role: "owner",
      user: "u1",
    });
    await page.goto("/live");

    const form = page.getByRole("form", { name: "Disengage kill switch" });
    await expect(form).toBeVisible();
    await expect(form).toContainText("permits Live nodes to start and place real orders");

    // The button is unavailable until a reason exists: disengaging is the
    // dangerous direction and the reason lands in the audit trail.
    const button = form.getByRole("button", { name: "Disengage kill switch" });
    await expect(button).toBeDisabled();

    await form.getByLabel("Reason for disengaging").fill("scheduled Live drill");
    await expect(button).toBeEnabled();
    await button.click();
    await expect(page.getByRole("status")).toContainText("Kill switch disengaged");
  });

  test("an Owner whose MFA is stale is told what to do, not merely refused", async ({
    page,
    request,
  }) => {
    await setScenario(request, { liveMfa: "stale", role: "owner", user: "u1" });
    await page.goto("/live");

    await expect(
      page.getByRole("heading", { name: "Fresh authentication required" }),
    ).toBeVisible();
    await expect(page.getByText(/re-authenticate to continue/i)).toBeVisible();

    // Refused, so no configuration is rendered.
    await expect(page.getByRole("region", { name: "Broker connections" })).toHaveCount(0);
  });
});
