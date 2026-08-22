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

test.beforeEach(async ({ request }) => {
  await setScenario(request, {
    entitlement: "active",
    exclusions: "present",
    recommendation: "fresh",
  });
});

test("member selects relative momentum, polls only its submitted run, and retains the prior report", async ({
  page,
}) => {
  const submittedRunPath = "/api/v1/recommendations/runs/00000000-0000-4000-8000-000000000202";
  const pollRequests: string[] = [];
  const pollStatuses: string[] = [];
  page.on("request", (request) => {
    if (request.method() === "GET" && request.url().includes("/api/v1/recommendations/runs/")) {
      pollRequests.push(new URL(request.url()).pathname);
    }
  });
  page.on("response", async (response) => {
    if (response.url().includes(submittedRunPath)) {
      pollStatuses.push((await response.json()).status);
    }
  });

  await page.goto("/strategies");
  const configuration = page.getByRole("form", { name: "Configure Relative momentum" });
  await configuration.getByLabel("Lookback months").fill("12");
  await configuration.getByLabel("Top holdings").fill("3");
  await configuration.getByRole("button", { name: "Save strategy configuration" }).click();
  await expect(page.getByRole("status")).toContainText("Configuration saved");

  await page.getByRole("link", { name: "Recommendations" }).click();
  const report = page.getByRole("region", { name: "Strategy-based proposal" });
  await expect(report.getByText("069500.KRX")).toBeVisible();
  await expect(report).toContainText("Cash allocation: 20.00%");
  await expect(report).toContainText("Synthetic QA data");
  await expect(report).toContainText("Dataset version");
  await expect(report).toContainText("Universe snapshot");
  await expect(report).toContainText("Factor snapshot");
  await expect(report).toContainText("Portfolio snapshot");

  const runForm = page.getByRole("form", { name: "Generate recommendation" });
  await expect(runForm.getByLabel("Strategy configuration")).toHaveValue(
    "00000000-0000-4000-8000-000000000101",
  );
  await runForm.getByLabel("As-of date").fill("2026-01-31");
  await runForm.getByRole("button", { name: "Generate strategy proposal" }).click();

  // Scope to the run section, not the page. The page renders a second
  // RecommendationRunStatus for `activeRun` outside this section
  // (recommendations/page.tsx:109), so once the submitted run and the active
  // run are both pending the page-wide locator matches two elements and
  // Playwright strict mode fails. It is the SUBMITTED run's status this
  // assertion is about, and that one lives inside the section the form is in.
  const runSection = page.getByRole("region", { name: "Generate recommendation" });
  await expect(
    runSection.getByRole("status", { name: "Recommendation is in progress" }),
  ).toContainText("Recommendation is in progress");
  await expect(report.getByText("069500.KRX")).toBeVisible();
  await expect(
    report
      .getByRole("table", { name: "Selected instruments and target weights" })
      .getByText("132030.KRX"),
  ).toBeVisible();
  await expect(page.getByRole("status", { name: "Recommendation is in progress" })).toHaveCount(0);
  expect(pollRequests).toEqual([submittedRunPath, submittedRunPath]);
  expect(pollStatuses).toEqual(["PENDING", "SUCCEEDED"]);
  const history = page.getByRole("region", { name: "Recommendation history" });
  await expect(history).toBeVisible();
  await history.getByRole("link", { name: "00000000-0000-4000-8000-000000000201" }).click();
  await expect(page).toHaveURL(/run_id=00000000-0000-4000-8000-000000000201/);
  await expect(report.getByText("069500.KRX")).toBeVisible();
});

test("invalid relative-momentum parameters and blocked access fail closed", async ({
  page,
  request,
}) => {
  await page.goto("/strategies");
  const configuration = page.getByRole("form", { name: "Configure Relative momentum" });
  await configuration.getByLabel("Top holdings").fill("0");
  await configuration.getByRole("button", { name: "Save strategy configuration" }).click();
  await expect(configuration.getByRole("alert")).toContainText(
    "Top holdings must be between 1 and 10",
  );

  await setScenario(request, { entitlement: "blocked" });
  await page.goto("/recommendations");
  await expect(appAlert(page)).toContainText("Recommendation data is blocked");
  await expect(page.getByText("069500.KRX")).toHaveCount(0);
  await expect(page.getByText("Synthetic QA data")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Generate strategy proposal" })).toHaveCount(0);
});

test("an empty exclusion set and stale result remain explicit", async ({ page, request }) => {
  await setScenario(request, { exclusions: "empty", recommendation: "stale" });
  await page.goto("/recommendations");

  await expect(page.getByRole("status", { name: "Recommendation warnings" })).toContainText(
    "Stale result",
  );
  await expect(page.getByText("No instruments were excluded.")).toBeVisible();
  await expect(page.getByText("As of Jan 31, 2026")).toBeVisible();
});
