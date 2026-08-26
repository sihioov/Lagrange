import { expect, test } from "@playwright/test";

const syntheticApiOrigin = process.env["SYNTHETIC_API_ORIGIN"] ?? "http://127.0.0.1:38180";
const ownerBetaPath = "/api/v1/recommendations/owner-beta/price-only/runs";
const ownerBetaRunId = "00000000-0000-4000-8000-000000000601";
const ownerBetaJobId = "00000000-0000-4000-8000-000000000701";
const strategyConfigId = "00000000-0000-4000-8000-000000000101";

async function setScenario(request: import("@playwright/test").APIRequestContext): Promise<void> {
  const response = await request.post(`${syntheticApiOrigin}/__test/scenario`, {
    data: {
      entitlement: "active",
      ownerBetaAccessMode: "owner_only",
      role: "owner",
    },
  });
  expect(response.ok()).toBe(true);
}

test("browser preserves Owner CSRF order and safely replays the existing beta report fixture", async ({
  page,
  request,
}) => {
  await setScenario(request);

  const browserRequests: Array<{
    readonly csrf: string | null;
    readonly idempotency: string | null;
    readonly method: string;
    readonly path: string;
  }> = [];
  let ownerBetaPostCount = 0;
  let ownerBetaPollCount = 0;
  let firstIdempotencyKey: string | null = null;

  page.on("request", (browserRequest) => {
    const url = new URL(browserRequest.url());
    if (
      url.pathname === "/api/v1/auth/csrf" ||
      url.pathname === ownerBetaPath ||
      url.pathname === `${ownerBetaPath}/${ownerBetaRunId}` ||
      url.pathname.startsWith("/api/v1/strategies/")
    ) {
      browserRequests.push({
        csrf: browserRequest.headers()["x-csrf-token"] ?? null,
        idempotency: browserRequest.headers()["idempotency-key"] ?? null,
        method: browserRequest.method(),
        path: url.pathname,
      });
    }
  });

  // The existing synthetic fixture does not implement this POST route. Keep
  // the browser transport evidence honest by intercepting only the dedicated
  // POST and terminal-detail transitions while continuing all other requests
  // through the repository's existing fixture server.
  await page.route(`**${ownerBetaPath}**`, async (route) => {
    const browserRequest = route.request();
    const url = new URL(browserRequest.url());
    if (url.pathname === ownerBetaPath && browserRequest.method() === "POST") {
      const csrf = browserRequest.headers()["x-csrf-token"];
      const idempotency = browserRequest.headers()["idempotency-key"];
      if (csrf === undefined) {
        await route.fulfill({
          status: 403,
          contentType: "application/json",
          body: JSON.stringify({
            error: {
              code: "CSRF_DENIED",
              message: "missing or invalid CSRF token",
              request_id: "request-wp3-csrf",
            },
          }),
        });
        return;
      }
      expect(idempotency).not.toBeUndefined();
      const body = browserRequest.postDataJSON() as {
        readonly as_of: string;
        readonly strategy_config_id: string;
      };
      expect(body).toEqual({ as_of: "2026-08-19", strategy_config_id: strategyConfigId });
      if (ownerBetaPostCount === 0) {
        ownerBetaPostCount += 1;
        firstIdempotencyKey = idempotency ?? null;
        await route.fulfill({
          status: 202,
          contentType: "application/json",
          body: JSON.stringify({
            job_id: ownerBetaJobId,
            run_id: ownerBetaRunId,
            status: "PENDING",
          }),
        });
        return;
      }
      ownerBetaPostCount += 1;
      expect(idempotency).toBe(firstIdempotencyKey);
      await route.fulfill({
        status: 202,
        headers: {
          "content-type": "application/json",
          "x-idempotent-replay": "true",
        },
        body: JSON.stringify({
          job_id: ownerBetaJobId,
          run_id: ownerBetaRunId,
          status: "PENDING",
        }),
      });
      return;
    }

    if (
      url.pathname === `${ownerBetaPath}/${ownerBetaRunId}` &&
      browserRequest.method() === "GET"
    ) {
      const upstream = await route.fetch();
      const body = (await upstream.json()) as Record<string, unknown>;
      ownerBetaPollCount += 1;
      if (ownerBetaPollCount === 1) {
        body["status"] = "PENDING";
        body["items"] = [];
        body["factor_snapshot_sha256"] = null;
        body["target_snapshot_sha256"] = null;
        body["cash_weight"] = null;
        body["started_at"] = null;
        body["finished_at"] = null;
      } else {
        expect(ownerBetaPollCount).toBe(2);
        body["status"] = "SUCCEEDED";
      }
      await route.fulfill({
        status: upstream.status(),
        headers: { "content-type": "application/json" },
        body: JSON.stringify(body),
      });
      return;
    }

    await route.continue();
  });

  // Authenticated session -> GET CSRF -> save active strategy config.
  await page.goto("/strategies");
  const configuration = page.getByRole("form", { name: "Configure Relative momentum" });
  await configuration.getByLabel("Lookback months").fill("12");
  await configuration.getByLabel("Top holdings").fill("3");
  await configuration.getByRole("button", { name: "Save strategy configuration" }).click();
  await expect(configuration.getByRole("status")).toContainText("Configuration saved");

  const configCsrf = browserRequests.findIndex(
    (entry) => entry.method === "GET" && entry.path === "/api/v1/auth/csrf",
  );
  const configPost = browserRequests.findIndex(
    (entry) => entry.method === "POST" && entry.path.startsWith("/api/v1/strategies/"),
  );
  expect(configCsrf).toBeGreaterThanOrEqual(0);
  expect(configPost).toBe(configCsrf + 1);
  expect(browserRequests[configPost]?.csrf).not.toBeNull();
  // The strategy-config mutation is sent through the Next rewrite to the
  // existing synthetic server, so the request header is the browser's only
  // CSRF evidence available at this layer.
  expect(browserRequests[configPost]?.idempotency).not.toBeNull();

  await page.getByRole("link", { name: "Recommendations" }).click();
  const warnings = page.getByRole("status", { name: "Recommendation warnings" });
  await expect(warnings).toContainText("Owner-only");
  await expect(warnings).toContainText("Price-return only");
  await expect(warnings).toContainText("Vendor snapshot");
  await expect(warnings).toContainText("Non-strict PIT");

  const report = page.getByRole("region", {
    name: "Owner-only recommendation",
    exact: true,
  });
  await expect(report).toBeVisible();
  await expect(report.getByRole("row")).toHaveCount(12);
  await expect(report).toContainText("Cash allocation: 20.00%");
  await expect(report).toContainText("0.912300");
  await expect(report).toContainText("SELECTED_TOP_N");

  const runForm = page.getByRole("form", { name: "Generate owner-only recommendation" });
  await expect(runForm.getByLabel("Strategy configuration")).toHaveValue(strategyConfigId);
  await runForm.getByLabel("As-of date").fill("2026-08-19");
  await runForm.getByRole("button", { name: "Generate owner-only recommendation" }).click();
  await expect(page.getByRole("status", { name: "Recommendation is in progress" })).toBeVisible();
  await expect(page.getByRole("status", { name: "Recommendation is in progress" })).toHaveCount(0);
  expect(ownerBetaPollCount).toBe(2);

  const ownerBetaCsrf = browserRequests.findIndex(
    (entry, index) =>
      index > configPost && entry.method === "GET" && entry.path === "/api/v1/auth/csrf",
  );
  const ownerBetaPost = browserRequests.findIndex(
    (entry, index) => index > configPost && entry.method === "POST" && entry.path === ownerBetaPath,
  );
  expect(ownerBetaCsrf).toBeGreaterThan(configPost);
  expect(ownerBetaPost).toBe(ownerBetaCsrf + 1);
  expect(browserRequests[ownerBetaPost]?.csrf).toBe("synthetic-csrf");
  expect(browserRequests[ownerBetaPost]?.idempotency).toBe(firstIdempotencyKey);

  // Negative CSRF is exercised against the same dedicated browser route.
  const denied = await page.evaluate(
    async ({ configId, path }) => {
      const response = await fetch(path, {
        body: JSON.stringify({ as_of: "2026-08-19", strategy_config_id: configId }),
        headers: {
          "Content-Type": "application/json",
          "Idempotency-Key": "wp3-browser-negative-csrf",
        },
        method: "POST",
      });
      return { body: await response.json(), status: response.status };
    },
    { configId: strategyConfigId, path: ownerBetaPath },
  );
  expect(denied.status).toBe(403);
  expect(denied.body.error.code).toBe("CSRF_DENIED");

  // A retry first fetches CSRF again and reuses the exact idempotency key and
  // body. The fixture returns the same pending identity with a replay marker.
  const replayCsrf = await page.evaluate(async () => {
    const response = await fetch("/api/v1/auth/csrf", { credentials: "same-origin" });
    return (await response.json()) as { readonly csrf_token: string };
  });
  expect(firstIdempotencyKey).not.toBeNull();
  if (firstIdempotencyKey === null) {
    throw new Error("owner-beta first request did not carry an idempotency key");
  }
  const replayKey = firstIdempotencyKey;
  const replay = await page.evaluate(
    async ({ configId, csrf, key, path }) => {
      const response = await fetch(path, {
        body: JSON.stringify({ as_of: "2026-08-19", strategy_config_id: configId }),
        credentials: "same-origin",
        headers: {
          "Content-Type": "application/json",
          "Idempotency-Key": key,
          "X-CSRF-Token": csrf,
        },
        method: "POST",
      });
      return {
        body: await response.json(),
        replay: response.headers.get("x-idempotent-replay"),
        status: response.status,
      };
    },
    {
      configId: strategyConfigId,
      csrf: replayCsrf.csrf_token,
      key: replayKey,
      path: ownerBetaPath,
    },
  );
  expect(replay.status).toBe(202);
  expect(replay.replay).toBe("true");
  expect(replay.body).toEqual({
    job_id: ownerBetaJobId,
    run_id: ownerBetaRunId,
    status: "PENDING",
  });
  expect(ownerBetaPostCount).toBe(2);

  // This browser test intentionally uses the existing synthetic report
  // fixture. The dedicated Web contract test derives the exact current ETF11
  // and five immutable pins and proves the schema accepts the report; durable
  // queue/runner publication remains unproven here because the approved
  // artifact bytes are unavailable to this worktree.
  await expect(page.getByRole("link", { name: ownerBetaRunId })).toBeVisible();
});
