#!/usr/bin/env node
// Real React components in an isolated browser; fixture responses are not runtime evidence.
import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { dirname, extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, expect } from "@playwright/test";
import { build } from "vite";

const repository = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const evidence = resolve(process.argv[2] ?? "/");
assert(evidence.startsWith("/tmp/"), "Pass a NEW absolute /tmp evidence directory");
await mkdir(evidence, { recursive: false, mode: 0o700 });
const fixture = join(evidence, "fixture");
const client = join(evidence, "client");
const emptyEnv = join(evidence, "empty-env");
await mkdir(fixture);
await mkdir(emptyEnv);
const membershipPath = "/api/v1/research/owner-beta/equity-universe-v2/memberships";
const id = "00000000-0000-4000-8000-000000000101";
const stamp = "2026-10-16T07:00:00Z";
function membership(lifecycle, generation = 0) {
  return {
    id,
    instrument_id: "005930.KRX",
    lifecycle,
    generation,
    requested_at: stamp,
    updated_at: stamp,
    coverage: {
      minimum_observed_sessions: 121,
      target_observed_sessions: 261,
      observed_sessions: 80,
    },
    ...(lifecycle === "DISABLED" ? { disabled_at: stamp } : {}),
  };
}
function list(lifecycle, generation = 0) {
  const active = lifecycle === "DISABLED" ? 0 : 1;
  return {
    policy: {
      active_instruments: active,
      max_active_instruments: 100,
      remaining_capacity: 100 - active,
      minimum_observed_sessions: 121,
      target_observed_sessions: 261,
    },
    memberships: [membership(lifecycle, generation)],
  };
}
const failure = {
  error: {
    code: "INTERNAL",
    message: "Fixture status failure",
    request_id: "fixture",
  },
};
const fixtures = {
  backfill: list("BACKFILLING"),
  ready: list("READY", 2),
  materializing: list("MATERIALIZING", 1),
  disabled: list("DISABLED"),
};
await writeFile(join(fixture, "fixtures.json"), JSON.stringify(fixtures));
await writeFile(
  join(fixture, "index.html"),
  '<!doctype html><div id="root"></div><script type="module" src="/main.tsx"></script>',
);
await writeFile(
  join(fixture, "tsconfig.json"),
  JSON.stringify({ compilerOptions: { jsx: "react-jsx" } }),
);
await writeFile(
  join(fixture, "navigation.ts"),
  "const router = { refresh() { window.refreshes = (window.refreshes ?? 0) + 1; } }; export function useRouter() { return router; } export function usePathname() { return '/stock-beta'; }",
);
await writeFile(
  join(fixture, "link.tsx"),
  "export default function Link({ children, ...props }) { return <a {...props}>{children}</a>; }",
);
await writeFile(
  join(fixture, "main.tsx"),
  await readFile(join(repository, "scripts/qa/stock-beta-membership-order-fixture.jsx")),
);
await build({
  configFile: false,
  envDir: emptyEnv,
  root: fixture,
  logLevel: "error",
  cacheDir: join(evidence, "vite-cache"),
  resolve: {
    alias: {
      "@": join(repository, "apps/web"),
      "next/navigation": join(fixture, "navigation.ts"),
      "next/link": join(fixture, "link.tsx"),
      react: join(repository, "node_modules/react"),
      "react-dom": join(repository, "node_modules/react-dom"),
    },
  },
  build: { outDir: client, emptyOutDir: false },
});
const server = createServer((request, response) => {
  const path = new URL(request.url, "http://localhost").pathname;
  const relative = path === "/" ? "index.html" : path.slice(1);
  const file = resolve(client, relative);
  if (!file.startsWith(`${client}/`)) {
    response.writeHead(404).end();
    return;
  }
  void readFile(file).then(
    (bytes) => {
      response.setHeader(
        "Content-Type",
        extname(file) === ".js"
          ? "text/javascript"
          : extname(file) === ".css"
            ? "text/css"
            : "text/html",
      );
      response.end(bytes);
    },
    () => response.writeHead(404).end(),
  );
});
const checks = [];
const foreignRequests = [];
const unexpectedRequests = [];
const pageErrors = [];
let browser;
const fulfill = (route, body, status = 200) =>
  route.fulfill({
    status,
    contentType: "application/json",
    body: JSON.stringify(body),
  });
async function settle(page, count) {
  await expect.poll(() => page.evaluate(() => window.completedMembershipResponses)).toBe(count);
  await page.evaluate(
    () => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))),
  );
}
try {
  await new Promise((yes, no) => {
    server.once("error", no);
    server.listen(0, "127.0.0.1", yes);
  });
  const origin = `http://127.0.0.1:${server.address().port}`;
  browser = await chromium.launch({ headless: true, timeout: 15000 });
  const context = await browser.newContext({ serviceWorkers: "block" });
  await context.route("**/*", (route) => {
    if (new URL(route.request().url()).origin === origin) return route.continue();
    foreignRequests.push(route.request().url());
    return route.abort();
  });
  for (const scenario of [
    "seed-late-session-expired",
    "seed-late-session-unknown",
    "disable-late-session-expired",
    "unmount-late-session-expired",
    "current-session-expired",
    "current-session-unknown",
    "disable",
    "mutation-refresh-seed",
    "terminal-seed",
    "nonterminal-seed",
    "seed-late-error",
    "poll-recovery",
    "unmount-aborts",
  ]) {
    const page = await context.newPage();
    page.on("pageerror", (error) => pageErrors.push(error.message));
    const pending = [];
    let loginRequests = 0;
    await page.route(`${origin}/login`, (route) => {
      // Observe the real location.replace using a loopback-only login destination.
      loginRequests++;
      return route.fulfill({
        contentType: "text/html",
        body: "<h1>Isolated login fixture</h1>",
      });
    });
    await page.route("**/api/v1/**", async (route) => {
      const url = new URL(route.request().url());
      const method = route.request().method();
      if (url.pathname === membershipPath && method === "GET") {
        pending.push(route);
        return;
      }
      if (url.pathname === "/api/v1/auth/csrf" && method === "GET")
        return fulfill(route, { csrf_token: "isolated-fixture-only" });
      if (url.pathname === `${membershipPath}/${id}/disable` && method === "POST")
        return fulfill(route, {
          resource: membership("DISABLED"),
          job_id: id,
          duplicate_active: false,
        });
      if (url.pathname.endsWith("/signals/latest") && method === "GET")
        return fulfill(
          route,
          {
            error: {
              code: "OWNER_EQUITY_SNAPSHOT_UNAVAILABLE",
              message: "Fixture unavailable",
              request_id: "fixture",
            },
          },
          503,
        );
      unexpectedRequests.push(`${method} ${url.pathname}`);
      return route.abort();
    });
    await page.goto(origin + (scenario === "unmount-aborts" ? "/?abort=real" : ""));
    const card = page.getByTestId("stock-beta-membership-card");
    await expect(card).toHaveAttribute("data-lifecycle", "BACKFILLING");
    await expect.poll(() => pending.length).toBe(1);
    const old = pending.shift();
    if (scenario.includes("session-")) {
      const current = scenario.startsWith("current-");
      const unmounted = scenario.startsWith("unmount-");
      const disabled = scenario.startsWith("disable-");
      const expectedLifecycle = disabled ? "DISABLED" : "READY";
      if (unmounted) {
        await page.evaluate(() => window.unmountWorkspace());
        await expect(card).toHaveCount(0);
      } else if (disabled) {
        await card.getByRole("button", { name: "Disable", exact: true }).click();
        await card.getByRole("button", { name: "Confirm disable", exact: true }).click();
        await expect.poll(() => pending.length).toBe(1);
        await fulfill(pending.shift(), list("DISABLED"));
        await expect(card).toHaveAttribute("data-lifecycle", "DISABLED");
      } else if (!current) {
        await page.evaluate(() => window.seedMemberships("ready"));
        await expect(card).toHaveAttribute("data-lifecycle", "READY");
      }
      await fulfill(
        old,
        {
          error: {
            code: scenario.endsWith("unknown") ? "SESSION_UNKNOWN" : "SESSION_EXPIRED",
            message: "Isolated session fixture",
            request_id: "isolated-session-fixture",
          },
        },
        401,
      );
      if (current) {
        await expect(page.getByRole("heading", { name: "Isolated login fixture" })).toBeVisible();
      } else {
        try {
          await settle(page, disabled ? 2 : 1);
        } catch (error) {
          // A stale redirect destroys the context: retain it as a failed product check.
          if (loginRequests === 0) throw error;
        }
      }
      const stateKept =
        current ||
        (loginRequests === 0 &&
          (unmounted
            ? (await card.count()) === 0
            : (await card.getAttribute("data-lifecycle")) === expectedLifecycle));
      checks.push({
        scenario,
        loginRequests,
        stateKept,
        passed: loginRequests === (current ? 1 : 0) && stateKept,
      });
      await page.screenshot({ path: join(evidence, `${scenario}.png`) });
      await page.close();
      continue;
    }
    if (scenario === "unmount-aborts") {
      const aborted = page.waitForEvent("requestfailed", {
        predicate: (request) => request === old.request(),
        timeout: 5000,
      });
      await page.evaluate(() => window.unmountWorkspace());
      const request = await aborted;
      assert.match(request.failure().errorText, /ABORTED/);
      await expect(card).toHaveCount(0);
      checks.push({ scenario, passed: true });
      await page.close();
      continue;
    }
    let expectedLifecycle;
    let expectedGeneration = 0;
    let responseCount = 1;
    if (scenario === "disable" || scenario === "mutation-refresh-seed") {
      await card.getByRole("button", { name: "Disable", exact: true }).click();
      await card.getByRole("button", { name: "Confirm disable", exact: true }).click();
      await expect.poll(() => pending.length).toBe(1);
      if (scenario === "mutation-refresh-seed") {
        await page.evaluate(() => window.seedMemberships("ready"));
        await expect(card).toHaveAttribute("data-lifecycle", "READY");
        expectedLifecycle = "READY";
        expectedGeneration = 2;
        await fulfill(pending.shift(), list("DISABLED"));
      } else {
        await fulfill(pending.shift(), list("DISABLED"));
        await expect(card).toHaveAttribute("data-lifecycle", "DISABLED");
        expectedLifecycle = "DISABLED";
      }
      responseCount = 2;
      await fulfill(old, list("BACKFILLING"));
    } else if (scenario === "poll-recovery") {
      await fulfill(old, failure, 500);
      await expect(
        page.getByText(
          "Status refresh failed; the last validated membership state remains visible.",
        ),
      ).toBeVisible();
      await expect.poll(() => pending.length).toBe(1);
      await fulfill(pending.shift(), list("MATERIALIZING", 1));
      expectedLifecycle = "MATERIALIZING";
      expectedGeneration = 1;
      responseCount = 2;
    } else {
      const terminal = scenario === "terminal-seed";
      await page.evaluate(
        (name) => window.seedMemberships(name),
        terminal ? "ready" : "materializing",
      );
      expectedLifecycle = terminal ? "READY" : "MATERIALIZING";
      expectedGeneration = terminal ? 2 : 1;
      await expect(card).toHaveAttribute("data-lifecycle", expectedLifecycle);
      await fulfill(
        old,
        scenario === "seed-late-error" ? failure : list("BACKFILLING"),
        scenario === "seed-late-error" ? 500 : 200,
      );
    }
    await settle(page, responseCount);
    const actualLifecycle = await card.getAttribute("data-lifecycle");
    const generationKept = (await card.innerText()).includes(`Generation ${expectedGeneration}`);
    const staleError = await page
      .getByText("Status refresh failed; the last validated membership state remains visible.")
      .count();
    const capacity = await page
      .getByTestId("stock-beta-policy-capacity")
      .locator("dd")
      .allTextContents();
    const expectedActive = expectedLifecycle === "DISABLED" ? "0" : "1";
    checks.push({
      scenario,
      expectedLifecycle,
      actualLifecycle,
      generationKept,
      staleError,
      capacity,
      loginRequests,
      passed:
        actualLifecycle === expectedLifecycle &&
        generationKept &&
        staleError === 0 &&
        loginRequests === 0 &&
        capacity[0] === expectedActive,
    });
    await page.screenshot({ path: join(evidence, `${scenario}.png`) });
    await page.close();
  }
  assert.deepEqual(foreignRequests, []);
  assert.deepEqual(unexpectedRequests, []);
  assert.deepEqual(pageErrors, []);
  assert(
    checks.every((check) => check.passed),
    "Membership state and session navigation must follow the current request",
  );
} finally {
  await browser?.close();
  server.closeAllConnections();
  await new Promise((done) => server.close(done));
  await writeFile(
    join(evidence, "result.json"),
    `${JSON.stringify(
      {
        checks,
        foreignRequests,
        unexpectedRequests,
        pageErrors,
        scope:
          "Isolated React/Chromium + fixture HTTP + router stub; no production, authentication, provider or receipt evidence",
      },
      null,
      2,
    )}\n`,
  );
  console.log(JSON.stringify({ checks, evidence }));
}
