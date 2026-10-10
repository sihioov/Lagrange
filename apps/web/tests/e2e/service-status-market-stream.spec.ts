import { type APIRequestContext, expect, type Page, test } from "@playwright/test";
import { ownerEquityV2MembershipListSchema } from "../../lib/products/equity-signals-contracts";
import { parseIntradayStreamEvent } from "../../lib/products/intraday-stream-contracts";
import { matchStreamLease } from "../../lib/products/intraday-stream-lease-contracts";
// @ts-expect-error Playwright loads this local Node fixture at runtime; it is not product TypeScript.
import { installServiceStatusFixture } from "./support/service-status-fixture.mjs";
// @ts-expect-error Playwright loads this local Node fixture at runtime; it is not product TypeScript.
import { startServiceStatusMarketStreamFixture } from "./support/service-status-market-stream-fixture.mjs";

const appOrigin = process.env["PLAYWRIGHT_BASE_URL"] ?? "http://127.0.0.1:33043";
const syntheticOrigin = process.env["SYNTHETIC_API_ORIGIN"] ?? "http://127.0.0.1:38193";
const configuredMode = process.env["OWNER_INTRADAY_QUOTES_MODE"];
const configuredTransport = process.env["OWNER_INTRADAY_QUOTE_TRANSPORT"];
const forbiddenApiPath =
  /(?:^|\/)(?:account|accounts|balance|buying-power|sellable-quantity|execution-history|order|orders|live)(?:\/|$)/i;
const productApiPath =
  /^\/api\/v1\/(?:research(?:\/|$)|strategies(?:\/|$)|recommendations(?:\/|$)|backtests(?:\/|$)|paper(?:\/|$)|screener(?:\/|$))/;
if (configuredMode !== "owner_only" || configuredTransport !== "market_ws") {
  throw new Error(
    "This suite requires OWNER_INTRADAY_QUOTES_MODE=owner_only and OWNER_INTRADAY_QUOTE_TRANSPORT=market_ws",
  );
}

async function resetScenario(request: APIRequestContext) {
  const response = await request.post(`${syntheticOrigin}/__test/scenario`, {
    data: {
      authSession: "valid",
      ownerBetaAccessMode: "owner_only",
      role: "owner",
      stockBetaIntradayState: "open",
      stockBetaRows: 1,
      stockBetaSeed: "ready",
      // The real stream controller admits only the checked-in observation universe.
      stockBetaSeedCode: "005930",
    },
  });
  expect(response.ok(), "reset the isolated synthetic Owner scenario").toBeTruthy();
  const memberships = await request.get(
    `${syntheticOrigin}/api/v1/research/owner-beta/equity-universe-v2/memberships`,
  );
  expect(memberships.status()).toBe(200);
  const parsed = ownerEquityV2MembershipListSchema.parse(await memberships.json());
  expect(parsed.memberships).toHaveLength(1);
  const membership = parsed.memberships[0];
  expect(membership?.lifecycle).toBe("READY");
  if (membership === undefined) throw new Error("Expected the sole seeded stream membership");
  const identities = [
    {
      membership_id: membership.id,
      instrument_id: membership.instrument_id,
      generation: membership.generation,
    },
  ];
  expect(identities).toEqual([
    {
      generation: 1,
      instrument_id: "005930.KRX",
      membership_id: "00000000-0000-4000-8000-000000006530",
    },
  ]);
  return identities;
}

function permittedLoopback(url: URL): boolean {
  return (
    ["127.0.0.1", "localhost"].includes(url.hostname) &&
    new Set([new URL(appOrigin).port, new URL(syntheticOrigin).port]).has(url.port)
  );
}

async function installProviderFreeGuard(page: Page) {
  const externalRequests: string[] = [];
  const forbiddenApiRequests: string[] = [];
  const productApiRequests: string[] = [];
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (!permittedLoopback(url))
      externalRequests.push(`${url.protocol}//${url.host}${url.pathname}`);
    if (url.pathname.startsWith("/api/") && forbiddenApiPath.test(url.pathname)) {
      forbiddenApiRequests.push(url.pathname);
    }
    if (productApiPath.test(url.pathname)) productApiRequests.push(url.pathname);
  });
  await page.routeWebSocket("**/*", async (socket) => {
    const url = new URL(socket.url());
    if (
      permittedLoopback(url) &&
      url.port === new URL(appOrigin).port &&
      url.pathname === "/_next/hmr"
    ) {
      socket.connectToServer();
    } else {
      externalRequests.push(`${url.protocol}//${url.host}${url.pathname}`);
      await socket.close({ code: 1000, reason: "provider-free market-stream QA" });
    }
  });
  await page.route("**/*", async (route) => {
    if (permittedLoopback(new URL(route.request().url()))) await route.continue();
    else {
      const url = new URL(route.request().url());
      externalRequests.push(`${url.protocol}//${url.host}${url.pathname}`);
      await route.abort("blockedbyclient");
    }
  });
  return { externalRequests, forbiddenApiRequests, productApiRequests };
}

async function waitForStatusCalls(fixture: { calls(): number }, count: number) {
  await expect.poll(() => fixture.calls(), { timeout: 35_000 }).toBe(count);
}

test("Owner status polling adds no market lease or SSE subscription in owner_only+market_ws", async ({
  page,
  request,
}, testInfo) => {
  test.setTimeout(120_000);
  const stockBetaGuard = await installProviderFreeGuard(page);
  const dashboardPage = await page.context().newPage();
  const dashboardGuard = await installProviderFreeGuard(dashboardPage);
  const status = await installServiceStatusFixture(dashboardPage);
  const stockBetaStatus = await installServiceStatusFixture(page);
  status.setFault({ kind: "delayed", status: 200, body: status.currentBody() });
  const stream = await startServiceStatusMarketStreamFixture(appOrigin);
  try {
    await stream.installPageRoutes(page);
    const identities = await resetScenario(request);
    await page.goto("/stock-beta");

    const chart = page.getByTestId("stock-beta-price-chart");
    const currentQuote = page.getByTestId("stock-beta-stream-selected");
    await expect(chart).toBeVisible();
    await expect(chart).toHaveAttribute("data-renderer-status", "ready");
    await expect(currentQuote).toBeVisible();
    await expect(currentQuote).toHaveAttribute("data-availability", "LIVE");
    await expect(currentQuote.locator('[data-quote-value="101200.00"]')).toBeVisible();
    await expect(page.getByTestId("service-status-panel")).toHaveCount(0);
    expect(stockBetaStatus.calls()).toBe(0);

    await dashboardPage.goto("/");
    await expect(dashboardPage.getByTestId("service-status-panel")).toBeVisible();
    await expect.poll(() => status.isPending(), { timeout: 10_000 }).toBe(true);
    expect(dashboardGuard.productApiRequests).toEqual([]);

    await expect.poll(() => stream.metrics().leaseCreates, { timeout: 10_000 }).toBe(1);
    await expect.poll(() => stream.metrics().activeSseConnections, { timeout: 10_000 }).toBe(1);
    const lease = stream.acceptedLease();
    expect(matchStreamLease(lease.response, lease.request).identities).toEqual(identities);
    const frame = stream.snapshotFrame();
    const parsedFrame = parseIntradayStreamEvent(
      "snapshot",
      JSON.stringify(frame.event),
      frame.eventId,
    );
    expect(parsedFrame.kind).toBe("snapshot");
    if (parsedFrame.kind !== "snapshot") throw new Error("Expected the first snapshot frame");
    expect(parsedFrame.body.rows).toHaveLength(1);
    expect(parsedFrame.body.rows[0]?.instrument_id).toBe("005930.KRX");
    expect(parsedFrame.body.rows[0]?.membership_id).toBe(identities[0]?.membership_id);
    expect(parsedFrame.body.rows[0]?.generation).toBe(identities[0]?.generation);
    const whileStatusRequestPending = stream.metrics();
    // Delay only the first observation; subsequent real controller polls must complete.
    status.setFault(null);
    await status.releaseDelayed();
    await waitForStatusCalls(status, 1);
    const afterInitialStatusResponse = stream.metrics();
    expect(afterInitialStatusResponse.leaseCreates).toBe(whileStatusRequestPending.leaseCreates);
    expect(afterInitialStatusResponse.leaseRenewals).toBe(whileStatusRequestPending.leaseRenewals);
    expect(afterInitialStatusResponse.leaseReleases).toBe(whileStatusRequestPending.leaseReleases);
    expect(afterInitialStatusResponse.sseConnections).toBe(
      whileStatusRequestPending.sseConnections,
    );
    expect(afterInitialStatusResponse.activeSseConnections).toBe(
      whileStatusRequestPending.activeSseConnections,
    );
    expect(afterInitialStatusResponse.providerRequests).toBe(0);
    expect(afterInitialStatusResponse.quoteDemandRequests).toBe(0);
    const beforeDisclosure = stream.metrics();
    await dashboardPage.getByTestId("service-status-panel").locator("summary").click();
    await expect(dashboardPage.getByTestId("service-status-panel").locator("tbody tr")).toHaveCount(
      22,
    );
    const afterDisclosure = stream.metrics();
    expect(afterDisclosure.leaseCreates).toBe(beforeDisclosure.leaseCreates);
    expect(afterDisclosure.leaseRenewals).toBe(beforeDisclosure.leaseRenewals);
    expect(afterDisclosure.leaseReleases).toBe(beforeDisclosure.leaseReleases);
    expect(afterDisclosure.sseConnections).toBe(beforeDisclosure.sseConnections);

    await waitForStatusCalls(status, 2);
    await waitForStatusCalls(status, 3);
    const afterStatusPolls = stream.metrics();
    expect(afterStatusPolls.leaseCreates).toBe(1);
    expect(afterStatusPolls.leaseRenewals).toBeGreaterThan(beforeDisclosure.leaseRenewals);
    expect(afterStatusPolls.leaseReleases).toBe(0);
    expect(afterStatusPolls.sseConnections).toBe(1);
    expect(afterStatusPolls.activeSseConnections).toBe(1);
    expect(afterStatusPolls.providerRequests).toBe(0);
    expect(afterStatusPolls.quoteDemandRequests).toBe(0);
    expect(dashboardGuard.productApiRequests).toEqual([]);
    await expect(currentQuote).toBeVisible();
    await expect(currentQuote).toHaveAttribute("data-availability", "LIVE");
    await expect(currentQuote.locator('[data-quote-value="101200.00"]')).toBeVisible();
    await expect(chart).toHaveAttribute("data-renderer-status", "ready");

    // Keep the document alive so the real provider cleanup can finish its bounded DELETE.
    await page
      .getByRole("navigation", { name: "Primary", exact: true })
      .getByRole("link", { name: "Dashboard", exact: true })
      .click();
    await expect(page).toHaveURL(`${appOrigin}/`);
    await expect(page.getByTestId("service-status-panel")).toBeVisible();
    await expect.poll(() => stockBetaStatus.calls(), { timeout: 10_000 }).toBe(1);
    await expect.poll(() => stream.metrics().leaseReleases, { timeout: 10_000 }).toBe(1);
    await expect.poll(() => stream.metrics().activeSseConnections, { timeout: 10_000 }).toBe(0);
    const afterUnmount = stream.metrics();
    expect(afterUnmount.leaseCreates).toBe(1);
    expect(afterUnmount.leaseRenewals).toBeGreaterThan(0);
    expect(afterUnmount.leaseReleases).toBe(1);
    expect(afterUnmount.sseConnections).toBe(1);
    expect(afterUnmount.sseClosed).toBe(1);
    expect(afterUnmount.providerRequests).toBe(0);
    expect(afterUnmount.quoteDemandRequests).toBe(0);
    expect(stockBetaGuard.externalRequests).toEqual([]);
    expect(stockBetaGuard.forbiddenApiRequests).toEqual([]);
    expect(dashboardGuard.externalRequests).toEqual([]);
    expect(dashboardGuard.forbiddenApiRequests).toEqual([]);
    expect(dashboardGuard.productApiRequests).toEqual([]);
    await testInfo.attach("synthetic-market-stream-measurements", {
      body: JSON.stringify(
        {
          fixture_scope:
            "one page-owned in-process synthetic SSE server; no provider transport or live data",
          status_poll_count: status.calls(),
          validated_wire_identities: identities,
          wire_schema_version: parsedFrame.schema_version,
          while_status_request_pending: whileStatusRequestPending,
          after_initial_status_response: afterInitialStatusResponse,
          before_disclosure: beforeDisclosure,
          after_disclosure: afterDisclosure,
          after_status_polls: afterStatusPolls,
          after_unmount: afterUnmount,
          dashboard_product_api_requests: dashboardGuard.productApiRequests,
          blocked_external_requests: stockBetaGuard.externalRequests,
          dashboard_blocked_external_requests: dashboardGuard.externalRequests,
          forbidden_api_requests: stockBetaGuard.forbiddenApiRequests,
          dashboard_forbidden_api_requests: dashboardGuard.forbiddenApiRequests,
        },
        null,
        2,
      ),
      contentType: "application/json",
    });
  } finally {
    await stream.close();
  }
});
