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
