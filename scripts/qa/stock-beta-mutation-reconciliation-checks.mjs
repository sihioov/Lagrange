import { expect } from "@playwright/test";

// A successful POST is not a successful reconciliation GET. Keep them distinct,
// recover through reads only, and retain the seed/unmount cancellation boundary.
export async function checkMutationReconciliation({
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
  const statusError = "Status refresh failed; the last validated membership state remains visible.";
  for (const action of ["add", "retry", "disable"]) {
    for (const transition of ["recover", "seed", "unmount"]) {
      const page = await context.newPage();
      page.on("pageerror", (error) => pageErrors.push(error.message));
      let posts = 0;
      const gets = [];
      const terminal = action === "disable" ? "DISABLED" : "READY";
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
          return respond(
            route,
            {
              resource: membership(action === "disable" ? "DISABLED" : "REQUESTED"),
              job_id: id,
              duplicate_active: false,
            },
            202,
          );
        }
        if (pathname === membershipPath && method === "GET") {
          gets.push(route);
          return;
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
      const scenario = `reconcile-${action}-${transition}`;
      let passed = false;
      let failure;
      let firstReadErrorVisible;
      try {
        await page.goto(
          `${origin}/?seed=${action === "add" ? "empty" : action === "retry" ? "failed" : "ready"}`,
        );
        const card = page.getByTestId("stock-beta-membership-card");
        if (action === "add") {
          await page.locator("#stock-beta-instrument-code").fill("005930");
          await page.getByRole("button", { name: "Add instrument", exact: true }).click();
        } else if (action === "retry") {
          await card.getByRole("button", { name: "Retry", exact: true }).click();
        } else {
          await card.getByRole("button", { name: "Disable", exact: true }).click();
          await card.getByRole("button", { name: "Confirm disable", exact: true }).click();
        }
        await expect.poll(() => gets.length).toBe(1);
        if (transition === "seed") {
          // Always replace the initial seed (Disable starts at READY already).
          await page.evaluate(() => window.seedMemberships("disabled"));
          await expect(card).toHaveAttribute("data-lifecycle", "DISABLED");
        }
        await respond(
          gets[0],
          {
            error: {
              code: "INTERNAL",
              message: "Fixture status failure",
              request_id: "fixture",
            },
          },
          500,
        );
        await expect.poll(() => page.evaluate(() => window.completedMembershipResponses)).toBe(1);
        await page.evaluate(
          () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))),
        );
        firstReadErrorVisible = await page.getByText(statusError, { exact: true }).isVisible();
        if (transition === "unmount") {
          await page.evaluate(() => window.unmountWorkspace());
          await expect(card).toHaveCount(0);
          await page.waitForTimeout(1100);
          expect(gets).toHaveLength(1);
          expect(posts).toBe(1);
        } else {
          await expect.poll(() => gets.length, { timeout: 2500 }).toBe(2);
          expect(firstReadErrorVisible).toBe(transition === "recover");
          await expect(
            page.getByTestId("stock-beta-policy-capacity").getByRole("alert"),
          ).toHaveCount(0);
          await respond(gets[1], list(terminal, 3));
          await expect(card).toHaveAttribute("data-lifecycle", terminal);
          await expect(page.getByText(statusError, { exact: true })).toHaveCount(0);
          await page.waitForTimeout(1100);
          expect(gets).toHaveLength(2);
          expect(posts).toBe(1);
        }
        passed = true;
      } catch (error) {
        failure = String(error.message).split("\n")[0];
      } finally {
        checks.push({
          scenario,
          passed,
          posts,
          gets: gets.length,
          firstReadErrorVisible,
          ...(failure ? { failure } : {}),
        });
        await page.close();
      }
    }
  }
}
