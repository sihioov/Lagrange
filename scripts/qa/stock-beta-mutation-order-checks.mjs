import { expect } from "@playwright/test";

// POSTs are held after dispatch. A new server seed cannot establish whether the
// mutation has committed, so mounted workspaces must still reconcile through GET.
// Unmounted workspaces must not start follow-up work or navigate on late errors.
export async function checkMutationLifetimes({
  context,
  origin,
  checks,
  pageErrors,
  unexpectedRequests,
  membershipPath,
  id,
  list,
  membership,
}) {
  for (const action of ["add", "retry", "disable"]) {
    for (const transition of ["current", "seed", "unmount"]) {
      for (const outcome of ["success", "failure", "session"]) {
        const page = await context.newPage();
        page.on("pageerror", (error) => pageErrors.push(error.message));
        let post;
        let posts = 0;
        let gets = 0;
        let logins = 0;
        const finalLifecycle = action === "disable" ? "DISABLED" : "READY";
        await page.route(`${origin}/login`, (route) => {
          logins++;
          return route.fulfill({
            contentType: "text/html",
            body: "<h1>Mutation login fixture</h1>",
          });
        });
        const respond = (route, body, status = 200) =>
          route.fulfill({
            status,
            contentType: "application/json",
            body: JSON.stringify(body),
          });
        await page.route("**/api/v1/**", (route) => {
          const { pathname } = new URL(route.request().url());
          const method = route.request().method();
          if (pathname === "/api/v1/auth/csrf" && method === "GET")
            return respond(route, { csrf_token: "isolated-fixture-only" });
          const path = action === "add" ? membershipPath : `${membershipPath}/${id}/${action}`;
          if (pathname === path && method === "POST") {
            posts++;
            post = route;
            return;
          }
          if (pathname === membershipPath && method === "GET") {
            gets++;
            return respond(route, list(finalLifecycle, 3));
          }
          if (pathname.endsWith("/signals/latest") && method === "GET")
            return respond(
              route,
              {
                error: {
                  code: "OWNER_EQUITY_SNAPSHOT_UNAVAILABLE",
                  message: "Fixture",
                  request_id: "fixture",
                },
              },
              503,
            );
          unexpectedRequests.push(`${method} ${pathname}`);
          return route.abort();
        });
        await page.goto(`${origin}/?seed=${action === "retry" ? "failed" : "ready"}`);
        const card = page.getByTestId("stock-beta-membership-card");
        await expect(card).toBeVisible();
        if (action === "add") {
          await page.locator("#stock-beta-instrument-code").fill("000660");
          await page.getByRole("button", { name: "Add instrument", exact: true }).click();
        } else if (action === "retry") {
          await card.getByRole("button", { name: "Retry", exact: true }).click();
        } else {
          await card.getByRole("button", { name: "Disable", exact: true }).click();
          await card.getByRole("button", { name: "Confirm disable", exact: true }).click();
        }
        await expect.poll(() => posts).toBe(1);
        if (transition === "unmount") {
          await page.evaluate(() => window.unmountWorkspace());
          await expect(card).toHaveCount(0);
        } else if (transition === "seed") {
          await page.evaluate(() => window.seedMemberships("disabled"));
          await expect(card).toHaveAttribute("data-lifecycle", "DISABLED");
          // A seed is not acknowledgement of the in-flight mutation: keep the lock.
          await expect(page.locator("#stock-beta-instrument-code")).toBeDisabled();
        }
        await respond(
          post,
          outcome === "success"
            ? {
                resource: membership(finalLifecycle),
                job_id: id,
                duplicate_active: false,
              }
            : {
                error: {
                  code: outcome === "session" ? "SESSION_EXPIRED" : "INTERNAL",
                  message: "Isolated mutation fixture",
                  request_id: "fixture",
                },
              },
          outcome === "success" ? 200 : outcome === "session" ? 401 : 500,
        );
        if (outcome === "session" && transition !== "unmount") {
          await expect(page.getByRole("heading", { name: "Mutation login fixture" })).toBeVisible();
        } else {
          try {
            await expect.poll(() => page.evaluate(() => window.completedMutationResponses)).toBe(1);
            await page.evaluate(
              () =>
                new Promise((resolve) =>
                  requestAnimationFrame(() => requestAnimationFrame(resolve)),
                ),
            );
          } catch (error) {
            if (logins === 0) throw error;
          }
        }
        let stateKept = true;
        if (logins === 0 && transition !== "unmount") {
          if (outcome === "success") {
            await expect(card).toHaveAttribute("data-lifecycle", finalLifecycle);
            await expect(card).toContainText("Generation 3");
          } else {
            stateKept =
              (await page.getByTestId("stock-beta-policy-capacity").getByRole("alert").count()) ===
              1;
          }
          await expect(page.locator("#stock-beta-instrument-code")).toBeEnabled();
        }
        checks.push({
          scenario: `mutation-${action}-${transition}-${outcome}`,
          posts,
          gets,
          logins,
          stateKept,
          passed:
            posts === 1 &&
            stateKept &&
            gets === (outcome === "success" && transition !== "unmount" ? 1 : 0) &&
            logins === (outcome === "session" && transition !== "unmount" ? 1 : 0),
        });
        await page.close();
      }
    }
  }
}
