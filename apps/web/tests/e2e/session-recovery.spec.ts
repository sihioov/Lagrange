import { expect, test } from "@playwright/test";

const syntheticApiOrigin = process.env["SYNTHETIC_API_ORIGIN"] ?? "http://127.0.0.1:38180";

async function setAuthSession(
  request: import("@playwright/test").APIRequestContext,
  authSession: "valid" | "unknown" | "expired" | "internal",
): Promise<void> {
  const response = await request.post(`${syntheticApiOrigin}/__test/scenario`, {
    data: { authSession },
  });
  expect(response.ok()).toBe(true);
}

async function expectLoginHandoff(
  request: import("@playwright/test").APIRequestContext,
): Promise<void> {
  const response = await request.get("/login", { maxRedirects: 0 });
  expect(response.status()).toBe(307);
  const location = response.headers()["location"];
  expect(location).toBeDefined();
  expect(new URL(location as string, response.url()).pathname).toBe("/auth/login");
}

async function expectTypedSessionFailure(
  request: import("@playwright/test").APIRequestContext,
  code: "SESSION_UNKNOWN" | "SESSION_EXPIRED" | "INTERNAL",
  status: number,
): Promise<void> {
  for (const path of [
    "/api/v1/auth/session",
    "/api/v1/auth/csrf",
    "/api/v1/strategy-configs",
    "/api/v1/recommendations/latest",
  ]) {
    const response = await request.get(`${syntheticApiOrigin}${path}`);
    expect(response.status()).toBe(status);
    await expect(response.json()).resolves.toMatchObject({ error: { code } });
  }
}

test.describe("authenticated session recovery", () => {
  test("puts every protected synthetic endpoint behind the typed session boundary", async ({
    request,
  }) => {
    for (const [authSession, code, status] of [
      ["unknown", "SESSION_UNKNOWN", 401],
      ["expired", "SESSION_EXPIRED", 401],
      ["internal", "INTERNAL", 500],
    ] as const) {
      await setAuthSession(request, authSession);
      await expectTypedSessionFailure(request, code, status);
    }
    await setAuthSession(request, "valid");
  });

  test("reloads an expired session into the login handoff", async ({ page, request }) => {
    await setAuthSession(request, "valid");
    await page.goto("/strategies");
    await expect(page.getByRole("heading", { level: 1, name: "Strategies" })).toBeVisible();

    await setAuthSession(request, "expired");
    await page.reload({ waitUntil: "commit" });

    await expect(page).toHaveURL(/\/auth\/login$/);
    await expect(page.getByText("We could not load this workspace")).toHaveCount(0);
    await expectLoginHandoff(request);
  });

  test("reloads an unknown session into the login handoff", async ({ page, request }) => {
    await setAuthSession(request, "valid");
    await page.goto("/strategies");
    await expect(page.getByRole("heading", { level: 1, name: "Strategies" })).toBeVisible();

    await setAuthSession(request, "unknown");
    await page.reload({ waitUntil: "commit" });

    await expect(page).toHaveURL(/\/auth\/login$/);
    await expect(page.getByText("We could not load this workspace")).toHaveCount(0);
    await expectLoginHandoff(request);
  });

  test("does not misclassify an internal session failure as an expired login", async ({
    page,
    request,
  }) => {
    await setAuthSession(request, "valid");
    await page.goto("/strategies");
    await expect(page.getByRole("heading", { level: 1, name: "Strategies" })).toBeVisible();

    await setAuthSession(request, "internal");
    await page.reload({ waitUntil: "domcontentloaded" });

    await expect(page).toHaveURL(/\/strategies$/);
    await expect(
      page.getByRole("heading", { level: 1, name: "This page couldn’t load" }),
    ).toBeVisible();
    await expect(page).not.toHaveURL(/\/login/);
    await expect(page).not.toHaveURL(/\/auth\/login/);
  });
});
