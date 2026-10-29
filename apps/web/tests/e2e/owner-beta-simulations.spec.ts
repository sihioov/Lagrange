import { expect, test } from "@playwright/test";
import { appAlert } from "./support/alerts";

const syntheticApiOrigin = process.env["SYNTHETIC_API_ORIGIN"] ?? "http://127.0.0.1:38180";
const SIMULATION_RUNS_PATH = "/api/v1/backtests/owner-beta/price-only/runs";
const SIMULATION_CONFIG_ID = "00000000-0000-4000-8000-000000000801";
const SIMULATION_RUN_ID = "00000000-0000-4000-8000-000000000811";
const SIMULATION_SUBMITTED_RUN_ID = "00000000-0000-4000-8000-000000000812";
const SIMULATION_FAILED_RUN_ID = "00000000-0000-4000-8000-000000000813";
const SIMULATION_CANCELED_RUN_ID = "00000000-0000-4000-8000-000000000814";
const OWNER_SIMULATION = {
  entitlement: "active",
  ownerBetaAccessMode: "owner_only",
  ownerBetaSimulation: "enabled",
  role: "owner",
};

async function setScenario(
  request: import("@playwright/test").APIRequestContext,
  scenario: Record<string, string>,
): Promise<void> {
  const response = await request.post(`${syntheticApiOrigin}/__test/scenario`, { data: scenario });
  expect(response.ok()).toBe(true);
}

function apiProblemResponse(status: number, code: string, message: string) {
  return {
    body: JSON.stringify({
      error: { code, message, request_id: "request-e2e-owner-beta-simulation" },
    }),
    contentType: "application/json",
    status,
  };
}

async function interceptEnqueueProblem(
  page: import("@playwright/test").Page,
  status: number,
  code: string,
  message: string,
): Promise<() => number> {
  const origin = new URL(page.url()).origin;
  let injectedPosts = 0;
  await page.route(
    (url) => url.origin === origin && url.pathname === SIMULATION_RUNS_PATH,
    async (route) => {
      if (route.request().method() !== "POST") {
        await route.fallback();
        return;
      }
      injectedPosts += 1;
      await route.fulfill(apiProblemResponse(status, code, message));
    },
  );
  return () => injectedPosts;
}

test("member cannot enumerate or call the owner-only simulation surface", async ({
  page,
  request,
}) => {
  await setScenario(request, {
    entitlement: "active",
    ownerBetaAccessMode: "owner_only",
    role: "member",
  });
  const dedicatedRequests: string[] = [];
  page.on("request", (current) => {
    if (new URL(current.url()).pathname.startsWith(SIMULATION_RUNS_PATH)) {
      dedicatedRequests.push(current.url());
    }
  });

  await page.goto("/backtests");
  await expect(
    page.getByRole("link", { name: "Open the owner-only price-only simulation" }),
  ).toHaveCount(0);
  await page.goto("/backtests/owner-beta/price-only");

  await expect(appAlert(page)).toContainText("Owner access required");
  await expect(page.getByText("OWNER_ONLY", { exact: true })).toHaveCount(0);
  await expect(page.getByText("PRICE_RETURN_ONLY", { exact: true })).toHaveCount(0);
  await expect(page.getByText("Price return only", { exact: false })).toHaveCount(0);
  expect(dedicatedRequests).toEqual([]);
});

test("owner inspects only the validated complete paired price-return result", async ({
  page,
  request,
}) => {
  await setScenario(request, OWNER_SIMULATION);
  await page.goto("/backtests");
  await page.getByRole("link", { name: "Open the owner-only price-only simulation" }).click();

  const boundary = page.getByRole("status", { name: "Simulation scope and limitations" });
  await expect(boundary).toContainText("OWNER_ONLY");
  await expect(boundary).toContainText("PRICE_RETURN_ONLY");
  await expect(boundary).toContainText("vendor_snapshot=true");
  await expect(boundary).toContainText("strict_pit=false");
  await expect(boundary).toContainText("Dividends are excluded");
  await expect(boundary).toContainText("Execution is hypothetical");
  await expect(boundary).toContainText("dates and phase names only");

  const pairRequests: string[] = [];
  page.on("request", (current) => {
    if (new URL(current.url()).pathname === `${SIMULATION_RUNS_PATH}/${SIMULATION_RUN_ID}/pair`) {
      pairRequests.push(current.url());
    }
  });
  await page.getByRole("button", { name: "Select 2026-08-27 to 2026-08-28, SUCCEEDED" }).click();
  const report = page.getByTestId("owner-beta-simulation-report");
  await expect(report).toBeVisible();
  await expect(report.getByRole("heading", { name: "Illustrative 10 bp per side" })).toBeVisible();
  await expect(report.getByRole("heading", { name: "Zero-cost reference" })).toBeVisible();
  await expect(report).toContainText("−0.10%");
  await expect(report).toContainText("₩9,990.00");
  await expect(report.getByText("Exact stored value").first()).toBeVisible();
  await expect(report.getByText(SIMULATION_RUN_ID, { exact: true })).toHaveCount(0);
  await expect(page.getByText("Synthetic QA data", { exact: true })).toHaveCount(0);

  const refreshRequest = page.waitForRequest(
    (current) =>
      current.method() === "GET" &&
      new URL(current.url()).pathname === `${SIMULATION_RUNS_PATH}/${SIMULATION_RUN_ID}`,
  );
  await page.getByRole("button", { name: "Refresh selected run" }).click();
  await refreshRequest;
  await expect(report).toBeVisible();
  expect(pairRequests).toHaveLength(1);
});

test("owner queues a closed request with CSRF and idempotency, then treats cancel as acknowledgement", async ({
  page,
  request,
}) => {
  await setScenario(request, OWNER_SIMULATION);
  await page.goto("/backtests/owner-beta/price-only");
  const form = page.getByRole("form", { name: "Queue a simulation" });
  const configSelect = form.getByLabel("Eligible strategy configuration");
  await expect(configSelect.locator("option")).toHaveCount(3);
  await expect(
    configSelect.locator('option[value="00000000-0000-4000-8000-000000000899"]'),
  ).toHaveCount(0);
  await configSelect.selectOption(SIMULATION_CONFIG_ID);
  await form.getByLabel("Evaluation start date").fill("2026-08-27");
  await form.getByLabel("Evaluation end date").fill("2026-08-28");

  const enqueueRequest = page.waitForRequest(
    (current) =>
      current.method() === "POST" && new URL(current.url()).pathname === SIMULATION_RUNS_PATH,
  );
  await form.getByRole("button", { name: "Queue simulation" }).click();
  const enqueue = await enqueueRequest;
  expect(enqueue.postDataJSON()).toEqual({
    contract_version: "owner-beta-price-only-simulation-v2-draft-1",
    end_date: "2026-08-28",
    start_date: "2026-08-27",
    strategy_config_id: SIMULATION_CONFIG_ID,
  });
  expect(enqueue.headers()["x-csrf-token"]).toBeTruthy();
  expect(enqueue.headers()["idempotency-key"]).toBeTruthy();
  await expect(page.getByRole("heading", { name: "Simulation running" })).toBeVisible();

  const cancelRequest = page.waitForRequest(
    (current) =>
      current.method() === "POST" &&
      new URL(current.url()).pathname ===
        `${SIMULATION_RUNS_PATH}/${SIMULATION_SUBMITTED_RUN_ID}/cancel`,
  );
  await page.getByRole("button", { name: "Request cancellation" }).click();
  const cancel = await cancelRequest;
  expect(cancel.postDataJSON()).toEqual({});
  expect(cancel.headers()["x-csrf-token"]).toBeTruthy();
  expect(cancel.headers()["idempotency-key"]).toBeTruthy();
  await expect(
    page.getByText("Cancellation was acknowledged. Refreshing the server's current run state.", {
      exact: true,
    }),
  ).toBeVisible();
  await expect(page.getByRole("heading", { name: "Simulation running" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Simulation canceled" })).toHaveCount(0);
});

test("owner leaves v2 dates blank and sees the server-checked common period", async ({
  page,
  request,
}) => {
  await setScenario(request, OWNER_SIMULATION);
  await page.goto("/backtests/owner-beta/price-only");
  const form = page.getByRole("form", { name: "Queue a simulation" });
  await form.getByLabel("Eligible strategy configuration").selectOption(SIMULATION_CONFIG_ID);

  const enqueueRequest = page.waitForRequest(
    (current) =>
      current.method() === "POST" && new URL(current.url()).pathname === SIMULATION_RUNS_PATH,
  );
  await form.getByRole("button", { name: "Queue simulation" }).click();
  const enqueue = await enqueueRequest;
  expect(enqueue.postDataJSON()).toEqual({
    contract_version: "owner-beta-price-only-simulation-v2-draft-1",
    strategy_config_id: SIMULATION_CONFIG_ID,
  });
  await expect(page.getByRole("heading", { name: "Simulation running" })).toBeVisible();
  await expect(page.getByText("Evaluation period: 2026-08-27 to 2026-08-28")).toBeVisible();
});

test("owner sees the validated v2 strategy and ETF quantities in the paired report", async ({
  page,
  request,
}) => {
  await setScenario(request, {
    ...OWNER_SIMULATION,
    ownerBetaSimulation: "v2_result",
  });
  await page.goto("/backtests/owner-beta/price-only");

  const run = page.getByRole("button", { name: "Select 2026-08-27 to 2026-08-28, SUCCEEDED" });
  await run.click();
  const report = page.getByTestId("owner-beta-simulation-report");
  await expect(report).toBeVisible();
  await expect(report.getByRole("columnheader", { name: "Buy and hold · 069500" })).toHaveCount(2);
  await expect(report).toContainText("069500.KRX · 1 shares");
  await expect(report).toContainText(/Period turnover is not annualized/i);
  await expect(
    report.getByText(/^(?:Sharpe(?: ratio)?|annualized (?:return|volatility))$/i),
  ).toHaveCount(0);
});

test("purges a visible pair on enqueue entitlement loss and ignores an older status response", async ({
  page,
  request,
}) => {
  await setScenario(request, OWNER_SIMULATION);
  await page.goto("/backtests/owner-beta/price-only");
  await page.getByRole("button", { name: "Select 2026-08-27 to 2026-08-28, SUCCEEDED" }).click();
  const report = page.getByTestId("owner-beta-simulation-report");
  await expect(report).toBeVisible();

  const origin = new URL(page.url()).origin;
  let release: (() => void) | undefined;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  let intercepted = false;
  let delivered = false;
  const lateStatusRoute = async (route: import("@playwright/test").Route): Promise<void> => {
    const response = await route.fetch({ timeout: 5_000 });
    intercepted = true;
    await gate;
    try {
      await route.fulfill({ response });
    } catch {
      // The entitlement error invalidates this request before its response arrives.
    } finally {
      delivered = true;
    }
  };

  try {
    await page.route(
      (url) =>
        url.origin === origin && url.pathname === `${SIMULATION_RUNS_PATH}/${SIMULATION_RUN_ID}`,
      lateStatusRoute,
      { times: 1 },
    );
    const lateStatusRequest = page.waitForRequest(
      (current) =>
        current.method() === "GET" &&
        new URL(current.url()).pathname === `${SIMULATION_RUNS_PATH}/${SIMULATION_RUN_ID}`,
    );
    await page.getByRole("button", { name: "Refresh selected run" }).click();
    await lateStatusRequest;
    await expect.poll(() => intercepted).toBe(true);

    await page.route(
      (url) => url.origin === origin && url.pathname === SIMULATION_RUNS_PATH,
      (route) =>
        route.fulfill(
          apiProblemResponse(403, "DATA_ENTITLEMENT_REQUIRED", "Backtest entitlement is inactive"),
        ),
      { times: 1 },
    );
    const csrfResponse = page.waitForResponse(
      (current) =>
        new URL(current.url()).pathname === "/api/v1/auth/csrf" && current.status() === 200,
    );
    const enqueueRequest = page.waitForRequest(
      (current) =>
        current.method() === "POST" && new URL(current.url()).pathname === SIMULATION_RUNS_PATH,
    );
    const form = page.getByRole("form", { name: "Queue a simulation" });
    await form.getByLabel("Eligible strategy configuration").selectOption(SIMULATION_CONFIG_ID);
    await form.getByRole("button", { name: "Queue simulation" }).click();
    const [csrf, enqueue] = await Promise.all([csrfResponse, enqueueRequest]);
    expect(csrf.status()).toBe(200);
    expect(enqueue.headers()["x-csrf-token"]).toBeTruthy();
    expect(enqueue.headers()["idempotency-key"]).toBeTruthy();
    await expect(page.getByRole("heading", { name: "Simulation data is blocked" })).toBeVisible();
    await expect(report).toHaveCount(0);
    await expect(
      page.getByRole("button", { name: "Select 2026-08-27 to 2026-08-28, SUCCEEDED" }),
    ).toHaveAttribute("aria-pressed", "true");

    release?.();
    await expect.poll(() => delivered).toBe(true);
    await expect(report).toHaveCount(0);
  } finally {
    release?.();
  }
});

test("uses login recovery when enqueue expires after successful CSRF preflight", async ({
  page,
  request,
}) => {
  await setScenario(request, OWNER_SIMULATION);
  await page.goto("/backtests/owner-beta/price-only");
  await expect(
    page.getByRole("button", { name: "Select 2026-08-27 to 2026-08-28, SUCCEEDED" }),
  ).toBeVisible();
  const injectedPosts = await interceptEnqueueProblem(
    page,
    401,
    "SESSION_EXPIRED",
    "session expired",
  );
  const csrfResponse = page.waitForResponse(
    (current) =>
      new URL(current.url()).pathname === "/api/v1/auth/csrf" && current.status() === 200,
  );
  const enqueueRequest = page.waitForRequest(
    (current) =>
      current.method() === "POST" && new URL(current.url()).pathname === SIMULATION_RUNS_PATH,
  );
  const enqueueResponse = page.waitForResponse(
    (current) =>
      current.request().method() === "POST" &&
      new URL(current.url()).pathname === SIMULATION_RUNS_PATH,
  );
  const form = page.getByRole("form", { name: "Queue a simulation" });
  await form.getByLabel("Eligible strategy configuration").selectOption(SIMULATION_CONFIG_ID);
  await form.getByRole("button", { name: "Queue simulation" }).click();
  const [csrf, enqueue, rejected] = await Promise.all([
    csrfResponse,
    enqueueRequest,
    enqueueResponse,
  ]);
  expect(csrf.status()).toBe(200);
  expect(enqueue.headers()["x-csrf-token"]).toBeTruthy();
  expect(enqueue.headers()["idempotency-key"]).toBeTruthy();
  expect(rejected.status()).toBe(401);
  expect((await rejected.json()).error.code).toBe("SESSION_EXPIRED");
  await expect(page).toHaveURL(/\/auth\/login$/);
  expect(injectedPosts()).toBe(1);
});

test("shows the form-specific message for a typed INVALID_DATE enqueue rejection", async ({
  page,
  request,
}) => {
  await setScenario(request, OWNER_SIMULATION);
  await page.goto("/backtests/owner-beta/price-only");
  await expect(
    page.getByRole("button", { name: "Select 2026-08-27 to 2026-08-28, SUCCEEDED" }),
  ).toBeVisible();
  const injectedPosts = await interceptEnqueueProblem(
    page,
    400,
    "INVALID_DATE",
    "evaluation date is not allowed",
  );
  const enqueueResponse = page.waitForResponse(
    (current) =>
      current.request().method() === "POST" &&
      new URL(current.url()).pathname === SIMULATION_RUNS_PATH,
  );
  const form = page.getByRole("form", { name: "Queue a simulation" });
  await form.getByLabel("Eligible strategy configuration").selectOption(SIMULATION_CONFIG_ID);
  await form.getByLabel("Evaluation start date").fill("2026-08-27");
  await form.getByLabel("Evaluation end date").fill("2026-08-28");
  await form.getByRole("button", { name: "Queue simulation" }).click();

  const rejected = await enqueueResponse;
  expect(rejected.status()).toBe(400);
  expect((await rejected.json()).error.code).toBe("INVALID_DATE");
  expect(rejected.request().headers()["x-csrf-token"]).toBeTruthy();
  expect(rejected.request().headers()["idempotency-key"]).toBeTruthy();
  await expect(form.getByRole("alert")).toContainText("Enter valid dates in chronological order");
  await expect(page).toHaveURL(/\/backtests\/owner-beta\/price-only$/);
  await expect(page.getByRole("heading", { name: "Simulation running" })).toHaveCount(0);
  await expect(page.getByRole("heading", { name: "Simulation queued" })).toHaveCount(0);
  expect(injectedPosts()).toBe(1);
});

test("invalidates an in-flight success when cancellation loses entitlement", async ({
  page,
  request,
}) => {
  await setScenario(request, { ...OWNER_SIMULATION, ownerBetaSimulation: "pending" });
  await page.goto("/backtests/owner-beta/price-only");
  const selected = page.getByRole("button", { name: "Select 2026-08-27 to 2026-08-28, PENDING" });
  await selected.click();
  await expect(page.getByRole("heading", { name: "Simulation queued" })).toBeVisible();

  // A successful status response is held while the selected run still appears active.
  await setScenario(request, OWNER_SIMULATION);
  const origin = new URL(page.url()).origin;
  let release: (() => void) | undefined;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  let intercepted = false;
  let delivered = false;
  const lateStatusRoute = async (route: import("@playwright/test").Route): Promise<void> => {
    const response = await route.fetch({ timeout: 5_000 });
    intercepted = true;
    await gate;
    try {
      await route.fulfill({ response });
    } catch {
      // The rejected cancellation invalidates this earlier success response.
    } finally {
      delivered = true;
    }
  };
  const pairRequests: string[] = [];
  page.on("request", (current) => {
    if (new URL(current.url()).pathname === `${SIMULATION_RUNS_PATH}/${SIMULATION_RUN_ID}/pair`) {
      pairRequests.push(current.url());
    }
  });

  try {
    await page.route(
      (url) =>
        url.origin === origin && url.pathname === `${SIMULATION_RUNS_PATH}/${SIMULATION_RUN_ID}`,
      lateStatusRoute,
      { times: 1 },
    );
    const lateStatusRequest = page.waitForRequest(
      (current) =>
        current.method() === "GET" &&
        new URL(current.url()).pathname === `${SIMULATION_RUNS_PATH}/${SIMULATION_RUN_ID}`,
    );
    await page.getByRole("button", { name: "Refresh selected run" }).click();
    await lateStatusRequest;
    await expect.poll(() => intercepted).toBe(true);

    await page.route(
      (url) =>
        url.origin === origin &&
        url.pathname === `${SIMULATION_RUNS_PATH}/${SIMULATION_RUN_ID}/cancel`,
      (route) => route.fulfill(apiProblemResponse(403, "FORBIDDEN", "cancellation is forbidden")),
      { times: 1 },
    );
    const csrfResponse = page.waitForResponse(
      (current) =>
        new URL(current.url()).pathname === "/api/v1/auth/csrf" && current.status() === 200,
    );
    const cancelRequest = page.waitForRequest(
      (current) =>
        current.method() === "POST" &&
        new URL(current.url()).pathname === `${SIMULATION_RUNS_PATH}/${SIMULATION_RUN_ID}/cancel`,
    );
    await page.getByRole("button", { name: "Request cancellation" }).click();
    const [csrf, cancel] = await Promise.all([csrfResponse, cancelRequest]);
    expect(csrf.status()).toBe(200);
    expect(cancel.headers()["x-csrf-token"]).toBeTruthy();
    expect(cancel.headers()["idempotency-key"]).toBeTruthy();
    await expect(page.getByRole("heading", { name: "Simulation access is blocked" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Request cancellation" })).toBeVisible();
    await expect(selected).toHaveAttribute("aria-pressed", "true");
    await expect(page.getByTestId("owner-beta-simulation-report")).toHaveCount(0);

    release?.();
    await expect.poll(() => delivered).toBe(true);
    await expect(page.getByTestId("owner-beta-simulation-report")).toHaveCount(0);
    expect(pairRequests).toEqual([]);
  } finally {
    release?.();
  }
});

test("uses login recovery when cancellation expires after successful CSRF preflight", async ({
  page,
  request,
}) => {
  await setScenario(request, { ...OWNER_SIMULATION, ownerBetaSimulation: "pending" });
  await page.goto("/backtests/owner-beta/price-only");
  await page.getByRole("button", { name: "Select 2026-08-27 to 2026-08-28, PENDING" }).click();
  const origin = new URL(page.url()).origin;
  await page.route(
    (url) =>
      url.origin === origin &&
      url.pathname === `${SIMULATION_RUNS_PATH}/${SIMULATION_RUN_ID}/cancel`,
    (route) => route.fulfill(apiProblemResponse(401, "SESSION_EXPIRED", "session expired")),
    { times: 1 },
  );
  const csrfResponse = page.waitForResponse(
    (current) =>
      new URL(current.url()).pathname === "/api/v1/auth/csrf" && current.status() === 200,
  );
  const cancelRequest = page.waitForRequest(
    (current) =>
      current.method() === "POST" &&
      new URL(current.url()).pathname === `${SIMULATION_RUNS_PATH}/${SIMULATION_RUN_ID}/cancel`,
  );
  await page.getByRole("button", { name: "Request cancellation" }).click();
  const [csrf, cancel] = await Promise.all([csrfResponse, cancelRequest]);
  expect(csrf.status()).toBe(200);
  expect(cancel.headers()["x-csrf-token"]).toBeTruthy();
  expect(cancel.headers()["idempotency-key"]).toBeTruthy();
  await expect(page).toHaveURL(/\/auth\/login$/);
});

test("owner without exact owner-only mode gets the bounded page without API construction", async ({
  page,
  request,
}) => {
  await setScenario(request, {
    entitlement: "active",
    ownerBetaAccessMode: "disabled",
    role: "owner",
  });
  const dedicatedRequests: string[] = [];
  page.on("request", (current) => {
    if (new URL(current.url()).pathname.startsWith(SIMULATION_RUNS_PATH)) {
      dedicatedRequests.push(current.url());
    }
  });

  await page.goto("/backtests/owner-beta/price-only");

  await expect(appAlert(page)).toContainText("Owner-only mode is required");
  await expect(
    page.getByRole("status", { name: "Simulation scope and limitations" }),
  ).toContainText("strict_pit=false");
  expect(dedicatedRequests).toEqual([]);
});

test("owner with blocked Backtest data rights sees the boundary and no result controls", async ({
  page,
  request,
}) => {
  await setScenario(request, { ...OWNER_SIMULATION, entitlement: "blocked" });
  const dedicatedRequests: string[] = [];
  page.on("request", (current) => {
    if (new URL(current.url()).pathname.startsWith(SIMULATION_RUNS_PATH)) {
      dedicatedRequests.push(current.url());
    }
  });

  await page.goto("/backtests/owner-beta/price-only");

  await expect(
    page.getByRole("status", { name: "Simulation scope and limitations" }),
  ).toContainText("OWNER_ONLY");
  await expect(appAlert(page)).toContainText("Simulation data is blocked");
  await expect(page.getByRole("button", { name: "Queue simulation" })).toHaveCount(0);
  expect(dedicatedRequests).toEqual([]);
});

test("owner without an eligible saved configuration cannot queue or invent one", async ({
  page,
  request,
}) => {
  await setScenario(request, { ...OWNER_SIMULATION, ownerBetaSimulation: "empty" });
  await page.goto("/backtests/owner-beta/price-only");

  await expect(appAlert(page)).toContainText("No eligible strategy configuration");
  await expect(page.getByRole("button", { name: "Queue simulation" })).toHaveCount(0);
  await expect(
    page.getByRole("combobox", { name: "Eligible strategy configuration", exact: true }),
  ).toHaveCount(0);
});

test("rejects reversed dates before sending an enqueue request", async ({ page, request }) => {
  await setScenario(request, OWNER_SIMULATION);
  await page.goto("/backtests/owner-beta/price-only");
  const form = page.getByRole("form", { name: "Queue a simulation" });
  await form.getByLabel("Eligible strategy configuration").selectOption(SIMULATION_CONFIG_ID);
  await form.getByLabel("Evaluation start date").fill("2026-08-28");
  await form.getByLabel("Evaluation end date").fill("2026-08-27");
  const enqueueRequests: string[] = [];
  page.on("request", (current) => {
    if (current.method() === "POST" && new URL(current.url()).pathname === SIMULATION_RUNS_PATH) {
      enqueueRequests.push(current.url());
    }
  });

  await form.getByRole("button", { name: "Queue simulation" }).click();

  await expect(form.getByRole("alert")).toContainText("Enter valid dates in chronological order");
  expect(enqueueRequests).toEqual([]);
});

test("shows empty history without selecting an implicit result", async ({ page, request }) => {
  await setScenario(request, { ...OWNER_SIMULATION, ownerBetaSimulation: "no_runs" });
  await page.goto("/backtests/owner-beta/price-only");

  await expect(page.getByRole("heading", { name: "No simulations yet" })).toBeVisible();
  await expect(page.getByTestId("owner-beta-simulation-report")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Queue simulation" })).toBeVisible();
});

test("keeps a pending run truthful and offers a cancellation request", async ({
  page,
  request,
}) => {
  await setScenario(request, { ...OWNER_SIMULATION, ownerBetaSimulation: "pending" });
  await page.goto("/backtests/owner-beta/price-only");
  await page.getByRole("button", { name: "Select 2026-08-27 to 2026-08-28, PENDING" }).click();

  await expect(page.getByRole("heading", { name: "Simulation queued" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Request cancellation" })).toBeVisible();
  await expect(page.getByTestId("owner-beta-simulation-report")).toHaveCount(0);
});

test("keeps cancellation available after rights loss without fabricating a terminal state", async ({
  page,
  request,
}) => {
  await setScenario(request, { ...OWNER_SIMULATION, ownerBetaSimulation: "pending" });
  await page.goto("/backtests/owner-beta/price-only");
  await page.getByRole("button", { name: "Select 2026-08-27 to 2026-08-28, PENDING" }).click();
  await expect(page.getByRole("heading", { name: "Simulation queued" })).toBeVisible();

  await setScenario(request, {
    ...OWNER_SIMULATION,
    entitlement: "blocked",
    ownerBetaSimulation: "pending",
  });
  await page.getByRole("button", { name: "Refresh selected run" }).click();
  await expect(page.getByRole("heading", { name: "Simulation data is blocked" })).toBeVisible();
  const cancelRequest = page.waitForRequest(
    (current) =>
      current.method() === "POST" &&
      new URL(current.url()).pathname === `${SIMULATION_RUNS_PATH}/${SIMULATION_RUN_ID}/cancel`,
  );
  await page.getByRole("button", { name: "Request cancellation" }).click();
  await cancelRequest;
  await expect(
    page.getByText("Cancellation was acknowledged. Refreshing the server's current run state.", {
      exact: true,
    }),
  ).toBeVisible();
  await expect(page.getByRole("heading", { name: "Simulation canceled" })).toHaveCount(0);
});

test("rejects partial, mixed, wrong-label, wrong-window, and wrong-run pairs", async ({
  page,
  request,
}) => {
  for (const scenario of [
    "invalid_pair",
    "partial_pair",
    "wrong_label",
    "wrong_window",
    "wrong_pair_id",
  ]) {
    await setScenario(request, { ...OWNER_SIMULATION, ownerBetaSimulation: scenario });
    await page.goto("/backtests/owner-beta/price-only");
    await page.getByRole("button", { name: "Select 2026-08-27 to 2026-08-28, SUCCEEDED" }).click();
    await expect(
      page.getByRole("heading", { name: "Simulation result could not be verified" }),
    ).toBeVisible();
    await expect(page.getByTestId("owner-beta-simulation-report")).toHaveCount(0);
  }
});

test("hides metrics when the API reports a stored-result integrity failure", async ({
  page,
  request,
}) => {
  await setScenario(request, { ...OWNER_SIMULATION, ownerBetaSimulation: "integrity_failed" });
  await page.goto("/backtests/owner-beta/price-only");
  await page.getByRole("button", { name: "Select 2026-08-27 to 2026-08-28, SUCCEEDED" }).click();
  await expect(
    page.getByRole("heading", { name: "Simulation result could not be verified" }),
  ).toBeVisible();
  await expect(page.getByTestId("owner-beta-simulation-report")).toHaveCount(0);
});

test("shows unavailable and not-ready states while keeping manual refresh available", async ({
  page,
  request,
}) => {
  await setScenario(request, { ...OWNER_SIMULATION, ownerBetaSimulation: "unavailable" });
  await page.goto("/backtests/owner-beta/price-only");
  await page.getByRole("button", { name: "Select 2026-08-27 to 2026-08-28, SUCCEEDED" }).click();
  await expect(page.getByRole("heading", { name: "Simulation service unavailable" })).toBeVisible();
  const refreshRequest = page.waitForRequest(
    (current) =>
      current.method() === "GET" &&
      new URL(current.url()).pathname === `${SIMULATION_RUNS_PATH}/${SIMULATION_RUN_ID}`,
  );
  await page.getByRole("button", { name: "Refresh selected run" }).click();
  await refreshRequest;
  await expect(page.getByRole("heading", { name: "Simulation service unavailable" })).toBeVisible();
  await expect(page.getByTestId("owner-beta-simulation-report")).toHaveCount(0);

  await setScenario(request, { ...OWNER_SIMULATION, ownerBetaSimulation: "not_ready" });
  await page.reload();
  await page.getByRole("button", { name: "Select 2026-08-27 to 2026-08-28, SUCCEEDED" }).click();
  await expect(page.getByRole("heading", { name: "Complete results are not ready" })).toBeVisible();
  await expect(page.getByTestId("owner-beta-simulation-report")).toHaveCount(0);
});

test("keeps a 404 selection and clears a validated pair when data rights are revoked", async ({
  page,
  request,
}) => {
  await setScenario(request, { ...OWNER_SIMULATION, ownerBetaSimulation: "foreign" });
  await page.goto("/backtests/owner-beta/price-only");
  const selected = page.getByRole("button", {
    name: "Select 2026-08-27 to 2026-08-28, SUCCEEDED",
  });
  await selected.click();
  await expect(page.getByRole("heading", { name: "Selected run unavailable" })).toBeVisible();
  await expect(selected).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByTestId("owner-beta-simulation-report")).toHaveCount(0);

  await setScenario(request, OWNER_SIMULATION);
  await page.reload();
  await page.getByRole("button", { name: "Select 2026-08-27 to 2026-08-28, SUCCEEDED" }).click();
  await expect(page.getByTestId("owner-beta-simulation-report")).toBeVisible();
  await setScenario(request, { ...OWNER_SIMULATION, entitlement: "blocked" });
  await page.getByRole("button", { name: "Refresh selected run" }).click();
  await expect(page.getByRole("heading", { name: "Simulation data is blocked" })).toBeVisible();
  await expect(page.getByTestId("owner-beta-simulation-report")).toHaveCount(0);
});

test("ignores a late status response after selecting another run", async ({ page, request }) => {
  await setScenario(request, { ...OWNER_SIMULATION, ownerBetaSimulation: "pages" });
  await page.goto("/backtests/owner-beta/price-only");
  const origin = new URL(page.url()).origin;
  let release: (() => void) | undefined;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  let intercepted = false;
  let delivered = false;
  const lateRoute = async (route: import("@playwright/test").Route): Promise<void> => {
    const response = await route.fetch({ timeout: 5_000 });
    intercepted = true;
    await gate;
    try {
      await route.fulfill({ response });
    } catch {
      // Selecting another run is expected to abort this obsolete response.
    } finally {
      delivered = true;
    }
  };
  try {
    await page.route(
      (url) =>
        url.origin === origin && url.pathname === `${SIMULATION_RUNS_PATH}/${SIMULATION_RUN_ID}`,
      lateRoute,
      { times: 1 },
    );
    await page.getByRole("button", { name: "Select 2026-08-27 to 2026-08-28, SUCCEEDED" }).click();
    await expect.poll(() => intercepted).toBe(true);
    await page.getByRole("button", { name: "Load more runs" }).click();
    await page.getByRole("button", { name: "Select 2026-08-27 to 2026-08-28, FAILED" }).click();
    await expect(page.getByRole("heading", { name: "Simulation failed" })).toBeVisible();
    release?.();
    await expect.poll(() => delivered).toBe(true);
    await expect(page.getByRole("heading", { name: "Simulation failed" })).toBeVisible();
    await expect(page.getByTestId("owner-beta-simulation-report")).toHaveCount(0);
  } finally {
    release?.();
  }
});

test("logout and a fresh page do not retain the previously selected result", async ({
  page,
  request,
}) => {
  await setScenario(request, OWNER_SIMULATION);
  await page.goto("/backtests/owner-beta/price-only");
  await page.getByRole("button", { name: "Select 2026-08-27 to 2026-08-28, SUCCEEDED" }).click();
  await expect(page.getByTestId("owner-beta-simulation-report")).toBeVisible();
  await page.route("**/api/v1/auth/logout", (route) => route.fulfill({ status: 204 }));
  await page.getByRole("button", { name: "Sign out" }).click();
  await expect(page).toHaveURL(/\/auth\/login$/);

  await setScenario(request, OWNER_SIMULATION);
  await page.goto("/backtests/owner-beta/price-only");
  await expect(page.getByTestId("owner-beta-simulation-report")).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Select 2026-08-27 to 2026-08-28, SUCCEEDED" }),
  ).toHaveAttribute("aria-pressed", "false");
});

test("history preserves an opaque signed cursor while loading another page", async ({
  page,
  request,
}) => {
  await setScenario(request, { ...OWNER_SIMULATION, ownerBetaSimulation: "pages" });
  await page.goto("/backtests/owner-beta/price-only");
  const cursorRequest = page.waitForRequest(
    (current) =>
      current.method() === "GET" &&
      new URL(current.url()).pathname === SIMULATION_RUNS_PATH &&
      new URL(current.url()).searchParams.get("cursor") === "qa.signed.cursor+one=",
  );
  await page.getByRole("button", { name: "Load more runs" }).click();
  const match = await cursorRequest;
  await expect(page.getByRole("row").filter({ hasText: "FAILED" })).toBeVisible();
  expect(new URL(match.url()).searchParams.get("cursor")).toBe("qa.signed.cursor+one=");

  await page.getByRole("button", { name: "Select 2026-08-27 to 2026-08-28, FAILED" }).click();
  await expect(page.getByRole("heading", { name: "Simulation failed" })).toBeVisible();
  await expect(page.getByTestId("owner-beta-simulation-report")).toHaveCount(0);
  await expect(page.getByText(SIMULATION_FAILED_RUN_ID, { exact: true })).toHaveCount(0);

  const nextCursorRequest = page.waitForRequest(
    (current) =>
      current.method() === "GET" &&
      new URL(current.url()).pathname === SIMULATION_RUNS_PATH &&
      new URL(current.url()).searchParams.get("cursor") === "qa.signed.cursor+two=",
  );
  await page.getByRole("button", { name: "Load more runs" }).click();
  await nextCursorRequest;
  await page.getByRole("button", { name: "Select 2026-08-27 to 2026-08-28, CANCELED" }).click();
  await expect(page.getByRole("heading", { name: "Simulation canceled" })).toBeVisible();
  await expect(page.getByTestId("owner-beta-simulation-report")).toHaveCount(0);
  await expect(page.getByText(SIMULATION_CANCELED_RUN_ID, { exact: true })).toHaveCount(0);
});
