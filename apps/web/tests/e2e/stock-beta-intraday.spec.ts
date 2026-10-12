import { randomUUID } from "node:crypto";
import {
  type APIRequestContext,
  type BrowserContext,
  expect,
  type Page,
  type Response,
  type TestInfo,
  test,
} from "@playwright/test";
import {
  ownerEquityV2MembershipListSchema,
  ownerEquityV2MembershipStatusSchema,
  ownerEquityV2MutationSchema,
} from "../../lib/products/equity-signals-contracts";
import {
  INTRADAY_STREAM_SOURCE,
  INTRADAY_STREAM_WIRE_VERSION,
  IntradayStreamContractError,
  type IntradayStreamIdentity,
} from "../../lib/products/intraday-stream-contracts";
import {
  INTRADAY_STREAM_LEASE_PATH as STREAM_LEASE_PATH,
  INTRADAY_STREAM_PREFIX as STREAM_PREFIX,
} from "../../lib/products/intraday-stream-lease-contracts";
import { readyStreamIdentities } from "../../lib/products/intraday-stream-universe";
import { startStockBetaMarketStreamFixture } from "./support/stock-beta-market-stream-fixture";

const appOrigin = process.env["PLAYWRIGHT_BASE_URL"] ?? "http://127.0.0.1:33043";
const syntheticOrigin = process.env["SYNTHETIC_API_ORIGIN"] ?? "http://127.0.0.1:38191";
const syntheticApiUrl = new URL(syntheticOrigin);
const configuredMode = process.env["OWNER_INTRADAY_QUOTES_MODE"];
const configuredTransport = process.env["OWNER_INTRADAY_QUOTE_TRANSPORT"];
const membershipPath = "/api/v1/research/owner-beta/equity-universe-v2/memberships";
const leasePath = STREAM_LEASE_PATH;
const streamPath = STREAM_PREFIX + "/market-stream";
const quoteA = "101200.00";
const quoteB = "198400.00";
const BASE_INSTANT = Date.parse("2026-10-12T03:00:00.500Z");
const forbiddenApiPath =
  /(?:^|\/)(?:account|accounts|balance|buying-power|sellable-quantity|execution-history|order|orders|live)(?:\/|$)/i;
const retiredQuotePath =
  /(?:quote-demands|equity-price-signals|\/instruments\/[^/]+\/quote(?:\/|$)|inquire-price|\/uapi\/|kis|opendart)/i;

if (
  appOrigin !== "http://127.0.0.1:33043" ||
  syntheticApiUrl.origin !== syntheticOrigin ||
  syntheticApiUrl.protocol !== "http:" ||
  syntheticApiUrl.hostname !== "127.0.0.1" ||
  syntheticApiUrl.port.length === 0 ||
  configuredMode !== "owner_only" ||
  configuredTransport !== "market_ws"
) {
  throw new Error(
    "This suite requires owner_only+market_ws and isolated app/synthetic API loopback origins",
  );
}

type RequestObservation = {
  readonly method: string;
  readonly protocol: string;
  readonly host: string;
  readonly pathname: string;
};

type SocketObservation = {
  readonly protocol: string;
  readonly host: string;
  readonly pathname: string;
};

type BrowserObservation = {
  readonly requests: RequestObservation[];
  readonly websocketAttempts: SocketObservation[];
  readonly websocketConnections: SocketObservation[];
};

type SyntheticClock = {
  readonly now: () => number;
  readonly advance: (milliseconds: number) => void;
  readonly pages: Set<Page>;
};

type StreamFixture = Awaited<ReturnType<typeof startStockBetaMarketStreamFixture>>;

type RowOptions = {
  readonly price?: string;
  readonly change?: string;
  readonly percent?: string;
  readonly direction?: "UP" | "DOWN" | "FLAT" | "LIMIT_UP" | "LIMIT_DOWN";
  readonly quoteVersion?: string;
  readonly stateVersion?: string;
  readonly receiveOrdinal?: string;
  readonly quotePresent?: boolean;
  readonly halted?: boolean;
  readonly marketState?: "OPEN" | "CLOSED" | "UNKNOWN";
  readonly freshness?: "RECENT" | "STALE" | "UNAVAILABLE";
  readonly availability?: "LIVE" | "LAST_KNOWN" | "AWAITING_FIRST_TRADE" | "UNAVAILABLE";
  readonly reasonCode?: string | null;
  readonly connection?: "DISCONNECTED" | "CONNECTING" | "CONNECTED" | "BACKOFF" | "STOPPED";
  readonly subscription?: "DESIRED" | "PENDING" | "ACKED" | "REJECTED" | "ABSENT";
  readonly sessionDate?: string | null;
  readonly businessDate?: string;
  readonly tradeAtMs?: number;
  readonly receivedAtMs?: number;
  readonly committedAtMs?: number;
  readonly gapOpen?: boolean;
  readonly sessionHasGap?: boolean;
  readonly gapGeneration?: string;
  readonly quoteOverrides?: Record<string, unknown>;
};

const observed = new WeakMap<Page, BrowserObservation>();
const browserClockBindings = new WeakMap<BrowserContext, SyntheticClock>();

function ownerScenario(overrides: Record<string, unknown> = {}) {
  return {
    authSession: "valid",
    ownerBetaAccessMode: "owner_only",
    role: "owner",
    stockBeta: "available",
    stockBetaRows: 1,
    stockBetaSeed: "ready",
    stockBetaSeedCode: "005930",
    ...overrides,
  };
}

async function resetScenario(request: APIRequestContext, scenario: Record<string, unknown>) {
  const response = await request.post(syntheticOrigin + "/__test/scenario", { data: scenario });
  expect(response.status(), "synthetic scenario reset status").toBe(200);
}

async function readyIdentities(request: APIRequestContext): Promise<IntradayStreamIdentity[]> {
  const response = await request.get(syntheticOrigin + membershipPath);
  expect(response.status(), "synthetic memberships status").toBe(200);
  const body = ownerEquityV2MembershipListSchema.parse(await response.json());
  return readyStreamIdentities(body.memberships);
}

async function prepareAAndB(request: APIRequestContext): Promise<IntradayStreamIdentity[]> {
  await resetScenario(request, ownerScenario());
  const aOnly = await readyIdentities(request);
  expect(aOnly).toHaveLength(1);
  expect(aOnly[0]).toMatchObject({
    instrument_id: "005930.KRX",
    generation: 1,
    membership_id: "00000000-0000-4000-8000-000000006530",
  });
  const added = await request.post(syntheticOrigin + membershipPath, {
    data: { instrument_code: "000660" },
    headers: {
      "x-csrf-token": "synthetic-stock-beta-market-stream",
      "idempotency-key": randomUUID(),
    },
  });
  expect(added.status(), "synthetic B membership admission request").toBe(202);
  const addedBody = (await added.json()) as {
    readonly resource?: { readonly id?: string };
  };
  const membershipId = addedBody.resource?.id;
  expect(membershipId).toMatch(/^[0-9a-f-]{36}$/);
  if (membershipId === undefined) throw new Error("synthetic B admission omitted its identity");
  const expected = ["VALIDATING", "BACKFILLING", "MATERIALIZING", "READY"] as const;
  for (const lifecycle of expected) {
    const status = await request.get(syntheticOrigin + membershipPath + "/" + membershipId);
    expect(status.status(), "bounded synthetic membership progress read").toBe(200);
    const parsed = ownerEquityV2MembershipStatusSchema.parse(await status.json());
    expect(parsed.membership.lifecycle).toBe(lifecycle);
  }
  const both = await readyIdentities(request);
  expect(
    both.map(({ instrument_id }) => instrument_id),
    "the page lease uses the two READY members in the checked-in fixed universe",
  ).toEqual(["005930.KRX", "000660.KRX"]);
  expect(both.map(({ generation }) => generation)).toEqual([1, 1]);
  expect(both.map(({ membership_id }) => membership_id)).toEqual([
    "00000000-0000-4000-8000-000000006530",
    membershipId,
  ]);
  return both;
}

function createClock(): SyntheticClock {
  let current = BASE_INSTANT;
  return {
    now: () => current,
    pages: new Set<Page>(),
    advance: (milliseconds) => {
      current += milliseconds;
    },
  };
}

async function runFor(page: Page, clock: SyntheticClock, milliseconds: number): Promise<void> {
  await runForPages([page], clock, milliseconds);
}

async function runForPages(
  pages: readonly Page[],
  clock: SyntheticClock,
  milliseconds: number,
): Promise<void> {
  // Playwright's clock belongs to a context. Advance every participating
  // context once, including other tabs that already share this fixture clock.
  const participating = new Map<BrowserContext, Page>();
  for (const page of new Set([...clock.pages, ...pages])) {
    if (!page.isClosed()) participating.set(page.context(), page);
  }
  let remaining = milliseconds;
  while (remaining > 0) {
    const step = Math.min(250, remaining);
    clock.advance(step);
    await Promise.all(
      [...participating.values()].map(async (page) => {
        expect(
          browserClockBindings.get(page.context()) === clock,
          "paused context clock binding",
        ).toBe(true);
        await page.clock.runFor(step);
        expect(await page.evaluate(() => Date.now()), "browser/fixture clock advancement").toBe(
          clock.now(),
        );
      }),
    );
    remaining -= step;
  }
}

function safeSocket(urlText: string): SocketObservation {
  const url = new URL(urlText);
  return { protocol: url.protocol, host: url.host, pathname: url.pathname };
}

function observeBrowserRequests(page: Page): BrowserObservation {
  const observation: BrowserObservation = {
    requests: [],
    websocketAttempts: [],
    websocketConnections: [],
  };
  page.on("request", (request) => {
    const url = new URL(request.url());
    observation.requests.push({
      method: request.method(),
      protocol: url.protocol,
      host: url.host,
      pathname: url.pathname,
    });
  });
  page.on("websocket", (socket) => {
    observation.websocketConnections.push(safeSocket(socket.url()));
  });
  observed.set(page, observation);
  return observation;
}

function observationFor(page: Page): BrowserObservation {
  return (
    observed.get(page) ?? {
      requests: [],
      websocketAttempts: [],
      websocketConnections: [],
    }
  );
}

function isLocalTestUrl(url: URL): boolean {
  const allowedPorts = new Set([new URL(appOrigin).port, new URL(syntheticOrigin).port]);
  return url.hostname === "127.0.0.1" && allowedPorts.has(url.port);
}

function isAllowedHmrSocket(url: URL): boolean {
  return (
    ["ws:", "wss:"].includes(url.protocol) &&
    url.hostname === "127.0.0.1" &&
    url.port === new URL(appOrigin).port &&
    url.pathname === "/_next/hmr"
  );
}

async function installProviderFreeNetworkGuard(page: Page): Promise<void> {
  const observation = observationFor(page);
  await page.routeWebSocket("**/*", async (socket) => {
    const safe = safeSocket(socket.url());
    observation.websocketAttempts.push(safe);
    if (isAllowedHmrSocket(new URL(socket.url()))) {
      socket.connectToServer();
      return;
    }
    await socket.close({ code: 1000, reason: "synthetic QA blocks non-HMR sockets" });
  });
  await page.route("**/*", async (route) => {
    const url = new URL(route.request().url());
    if (["http:", "https:"].includes(url.protocol) && isLocalTestUrl(url)) {
      await route.fallback();
      return;
    }
    await route.abort("blockedbyclient");
  });
}

function assertProviderFree(page: Page): void {
  const observation = observationFor(page);
  const external = observation.requests.filter((request) => {
    const url = new URL(request.protocol + "//" + request.host + request.pathname);
    return !isLocalTestUrl(url);
  });
  expect(external.map(({ protocol, host, pathname }) => ({ protocol, host, pathname }))).toEqual(
    [],
  );
  const forbidden = observation.requests.filter(
    ({ pathname }) => forbiddenApiPath.test(pathname) || retiredQuotePath.test(pathname),
  );
  expect(forbidden.map(({ method, pathname }) => ({ method, pathname }))).toEqual([]);
  const sockets = [...observation.websocketAttempts, ...observation.websocketConnections].filter(
    (socket) =>
      !isAllowedHmrSocket(new URL(socket.protocol + "//" + socket.host + socket.pathname)),
  );
  expect(sockets).toEqual([]);
}

function membershipCards(page: Page) {
  return page
    .getByRole("region", { name: "Membership status" })
    .getByTestId("stock-beta-membership-card");
}

function membershipCard(page: Page, instrumentId: string, lifecycle?: string) {
  const lifecycleSelector = lifecycle === undefined ? "" : '[data-lifecycle="' + lifecycle + '"]';
  return page
    .getByRole("region", { name: "Membership status" })
    .locator('li[data-testid="stock-beta-membership-card"]' + lifecycleSelector)
    .filter({ hasText: instrumentId });
}

function rankedRow(page: Page, instrumentId: string) {
  return page
    .getByRole("region", { name: "Ranked signals" })
    .getByTestId("stock-beta-row-" + instrumentId);
}

function dashboardQuoteWidget(page: Page) {
  return page.getByTestId("stock-beta-widget-current-quote");
}

function detailQuoteWidget(page: Page) {
  return page.getByTestId("stock-beta-detail-widget-current-quote");
}

function board(page: Page) {
  return page.getByTestId("stock-beta-stream-board");
}

function boardRow(page: Page, instrumentId: string) {
  return board(page).locator('tr[data-stream-instrument="' + instrumentId + '"]');
}

function selectedQuote(page: Page) {
  return page.getByTestId("stock-beta-stream-selected");
}

async function expectStockShell(page: Page): Promise<void> {
  await expect(page.locator('[data-shell="stock-beta-terminal"]')).toHaveCount(1);
  await expect(page.locator('[data-terminal-utility-bar="stock-beta"]')).toHaveCount(1);
}

async function openStockBeta(
  page: Page,
  request: APIRequestContext,
  fixture: StreamFixture,
  clock: SyntheticClock,
  options: { readonly installClock?: boolean; readonly waitForLease?: boolean } = {},
): Promise<IntradayStreamIdentity[]> {
  await fixture.installPageRoutes(page);
  const navigationInstant = clock.now();
  if (options.installClock !== false) {
    expect(browserClockBindings.has(page.context()), "clock is installed once per context").toBe(
      false,
    );
    const instant = new Date(navigationInstant);
    await page.clock.install({ time: instant });
    // Fixed Date is transient while pausing, and ends before app navigation.
    await page.clock.setFixedTime(instant);
    await page.clock.pauseAt(instant);
    await page.clock.setSystemTime(instant);
    browserClockBindings.set(page.context(), clock);
  } else {
    expect(browserClockBindings.get(page.context()) === clock, "reused paused clock binding").toBe(
      true,
    );
  }
  clock.pages.add(page);
  const browserAtNavigation = await page.evaluate(() => Date.now());
  expect(browserAtNavigation, "exact browser/fixture instant before navigation").toBe(
    navigationInstant,
  );
  const createTarget = fixture.metrics().acceptedCreates + 1;
  const connectionTarget = fixture.connectionLedger().length + 1;
  await page.goto("/stock-beta");
  await pumpAnimationFrames(page, clock);
  await expectStockShell(page);
  await expect(board(page)).toHaveCount(1);
  await expect(dashboardQuoteWidget(page)).toHaveCount(1);
  await expect(selectedQuote(page)).toHaveCount(1);
  const identities = await readyIdentities(request);
  if (options.waitForLease !== false && identities.length > 0) {
    await expect
      .poll(() => fixture.metrics().acceptedCreates, { timeout: 10_000 })
      .toBe(createTarget);
    await expect
      .poll(() => fixture.connectionLedger().length, { timeout: 10_000 })
      .toBe(connectionTarget);
  }
  const browserAfterPaint = await page.evaluate(() => Date.now());
  expect(browserAfterPaint, "paused browser/fixture instant after synchronized paint").toBe(
    clock.now(),
  );
  await test.info().attach("stock-beta-paused-clock-binding", {
    contentType: "application/json",
    body: JSON.stringify({
      installed: options.installClock !== false,
      fixture_at_navigation_ms: navigationInstant,
      browser_at_navigation_ms: browserAtNavigation,
      fixture_after_paint_ms: clock.now(),
      browser_after_paint_ms: browserAfterPaint,
    }),
  });
  return identities;
}

async function waitForAcceptedLease(
  fixture: StreamFixture,
  expectedCreates: number,
  expectedConnections: number,
): Promise<void> {
  await expect
    .poll(() => fixture.metrics().acceptedCreates, { timeout: 10_000 })
    .toBe(expectedCreates);
  await expect
    .poll(() => fixture.connectionLedger().length, { timeout: 10_000 })
    .toBe(expectedConnections);
}

function activeLease(fixture: StreamFixture) {
  const lease = fixture.leaseLedger().find((candidate) => !candidate.released);
  if (lease === undefined) throw new Error("Expected an accepted active stream lease");
  return lease;
}

function activeConnection(fixture: StreamFixture, leaseId?: string) {
  const connection = fixture
    .connectionLedger()
    .find(
      (candidate) =>
        candidate.closedAt === null && (leaseId === undefined || candidate.leaseId === leaseId),
    );
  if (connection === undefined) throw new Error("Expected an active controlled SSE connection");
  return connection;
}

function nextSequence(connection: ReturnType<StreamFixture["connectionLedger"]>[number]): string {
  const latest = connection.sent.reduce(
    (maximum, frame) => Math.max(maximum, Number(frame.eventSequence)),
    0,
  );
  return String(latest + 1);
}

function makeEnvelope(
  connection: ReturnType<StreamFixture["connectionLedger"]>[number],
  clock: SyntheticClock,
  sequence: string,
  body: unknown,
): Record<string, unknown> {
  return {
    schema_version: 2,
    stream_id: connection.streamId,
    event_sequence: sequence,
    server_time: new Date(clock.now()).toISOString(),
    body,
  };
}

function datePartsKst(milliseconds: number) {
  const value = new Date(milliseconds + 9 * 60 * 60 * 1_000).toISOString();
  return {
    date: value.slice(0, 10),
    time: value.slice(11, 19),
    providerTime: value.slice(0, 19) + "+09:00",
  };
}

function kstDate(milliseconds: number): string {
  return datePartsKst(milliseconds).date;
}

function identityUuid(identity: IntradayStreamIdentity, group: string): string {
  return "00000000-0000-4000-" + group + "-" + identity.membership_id.slice(-12);
}

function makeRow(
  identity: IntradayStreamIdentity,
  clock: SyntheticClock,
  options: RowOptions = {},
): Record<string, unknown> {
  const price = options.price ?? (identity.instrument_id === "005930.KRX" ? quoteA : quoteB);
  const change =
    options.change ?? (identity.instrument_id === "005930.KRX" ? "1200.00" : "-1600.00");
  const percent = options.percent ?? (identity.instrument_id === "005930.KRX" ? "1.20" : "-0.80");
  const direction = options.direction ?? (identity.instrument_id === "005930.KRX" ? "UP" : "DOWN");
  const receivedAtMs = options.receivedAtMs ?? clock.now() - 100;
  const committedAtMs = options.committedAtMs ?? clock.now() - 50;
  const tradeAtMs = options.tradeAtMs ?? Math.floor((receivedAtMs - 1_000) / 1_000) * 1_000;
  const parts = datePartsKst(tradeAtMs);
  const businessDate = options.businessDate ?? parts.date;
  const quote = {
    price,
    base_price: null,
    base_price_reason: "NOT_PROVIDED_BY_CHANNEL",
    change_from_previous_day: change,
    change_percent_from_previous_day: percent,
    direction,
    trade_volume: "100",
    cumulative_volume: "1000",
    halted: options.halted ?? false,
    business_date: businessDate,
    trade_time: parts.time,
    provider_trade_at: parts.providerTime,
    received_at: new Date(receivedAtMs).toISOString(),
    committed_at: new Date(committedAtMs).toISOString(),
    epoch: identityUuid(identity, "9000"),
    quote_version: options.quoteVersion ?? "1",
    receive_ordinal: options.receiveOrdinal ?? "1",
    ...(options.quoteOverrides ?? {}),
  };
  const sessionDate = options.sessionDate === undefined ? businessDate : options.sessionDate;
  const session =
    sessionDate === null
      ? null
      : {
          date: sessionDate,
          timezone: "Asia/Seoul",
          calendar_source: "kis",
          calendar_source_version: "kis-chk-holiday-v1:schema-1",
          calendar_content_sha256: "a".repeat(64),
          window_contract_sha256: "sha256:" + "b".repeat(64),
        };
  const quotePresent = options.quotePresent !== false;
  const freshness = options.freshness ?? (quotePresent ? "RECENT" : "UNAVAILABLE");
  const availability = options.availability ?? (quotePresent ? "LIVE" : "UNAVAILABLE");
  return {
    membership_id: identity.membership_id,
    instrument_id: identity.instrument_id,
    generation: identity.generation,
    row_generation: identityUuid(identity, "9001"),
    venue: "KRX",
    currency: "KRW",
    source: INTRADAY_STREAM_SOURCE,
    wire_version: INTRADAY_STREAM_WIRE_VERSION,
    session,
    subscription: options.subscription ?? "ACKED",
    connection: options.connection ?? "CONNECTED",
    market_state: options.marketState ?? "OPEN",
    freshness,
    availability,
    reason_code: options.reasonCode ?? null,
    state_version: options.stateVersion ?? "1",
    gap_open: options.gapOpen ?? false,
    session_has_gap: options.sessionHasGap ?? false,
    gap_generation: options.gapGeneration ?? "0",
    quote: quotePresent ? quote : null,
  };
}

function snapshotBody(
  fixture: StreamFixture,
  identities: readonly IntradayStreamIdentity[],
  clock: SyntheticClock,
  options: RowOptions = {},
  leaseId?: string,
) {
  const lease =
    leaseId === undefined
      ? activeLease(fixture)
      : fixture.leaseLedger().find((candidate) => candidate.leaseId === leaseId);
  if (lease === undefined) throw new Error("Expected the snapshot's accepted lease");
  return {
    lease_id: lease.leaseId,
    lease_expires_at: lease.expiresAt,
    rows: identities.map((identity) => makeRow(identity, clock, options)),
  };
}

async function emitFrame(
  page: Page,
  clock: SyntheticClock,
  fixture: StreamFixture,
  connectionId: string,
  kind: "snapshot" | "delta" | "status" | "reset",
  body: unknown,
): Promise<void> {
  fixture.emit(connectionId, kind, body);
  await runFor(page, clock, 16);
  await runFor(page, clock, 16);
}

async function emitFrameOnPages(
  pages: readonly Page[],
  clock: SyntheticClock,
  fixture: StreamFixture,
  connectionId: string,
  kind: "snapshot" | "delta" | "status" | "reset",
  body: unknown,
): Promise<void> {
  fixture.emit(connectionId, kind, body);
  await runForPages(pages, clock, 16);
  await runForPages(pages, clock, 16);
}

async function expectLiveQuote(page: Page, instrumentId: string, price: string): Promise<void> {
  const selected = selectedQuote(page);
  await expect(selected).toHaveAttribute("data-availability", "LIVE");
  await expect(selected.locator('strong[data-quote-value="' + price + '"]')).toHaveCount(1);
  await expect(boardRow(page, instrumentId)).toHaveAttribute("data-availability", "LIVE");
  await expect(
    boardRow(page, instrumentId).locator('td[data-quote-value="' + price + '"]'),
  ).toHaveCount(1);
}

async function expectNoCapturedQuote(page: Page, instrumentId: string): Promise<void> {
  await expect(selectedQuote(page).locator("[data-quote-value]")).toHaveCount(0);
  await expect(boardRow(page, instrumentId).locator("[data-quote-value]")).toHaveCount(0);
  await expect(boardRow(page, instrumentId).locator("time[datetime]")).toHaveCount(0);
}

async function sendInitialSnapshot(
  page: Page,
  clock: SyntheticClock,
  fixture: StreamFixture,
  identities: readonly IntradayStreamIdentity[],
  options: RowOptions = {},
  connectionId?: string,
): Promise<string> {
  const lease = activeLease(fixture);
  const connection =
    connectionId === undefined
      ? activeConnection(fixture, lease.leaseId)
      : {
          id: connectionId,
          streamId:
            fixture.connectionLedger().find((item) => item.id === connectionId)?.streamId ?? "",
          leaseId: lease.leaseId,
          sent: [],
          closedAt: null,
        };
  if (connection.streamId.length === 0) throw new Error("Missing controlled stream identity");
  await emitFrame(
    page,
    clock,
    fixture,
    connection.id,
    "snapshot",
    snapshotBody(fixture, identities, clock, options),
  );
  return connection.id;
}

async function pumpAnimationFrames(page: Page, clock: SyntheticClock): Promise<void> {
  await runFor(page, clock, 16);
  await runFor(page, clock, 16);
}

async function attachClockPhase(
  page: Page,
  clock: SyntheticClock,
  fixture: StreamFixture,
  name: string,
): Promise<void> {
  const browser = await page.evaluate(() => ({
    date: Date.now(),
    performance: performance.now(),
  }));
  expect(browser.date, "diagnostic browser/fixture clock binding").toBe(clock.now());
  const metrics = fixture.metrics();
  await test.info().attach(name, {
    contentType: "application/json",
    body: Buffer.from(
      JSON.stringify({
        fixtureNow: clock.now(),
        browser,
        connections: fixture.connectionLedger().length,
        getOpens: metrics.getOpens,
        acceptedCreates: metrics.acceptedCreates,
        acceptedReleases: metrics.acceptedReleases,
        acceptedRenewals: metrics.acceptedRenewals,
        boardCount: await board(page).count(),
        loadingCount: await page.getByText("Loading workspace", { exact: true }).count(),
      }),
    ),
  });
}

async function advanceReconnectBoundary(
  page: Page,
  clock: SyntheticClock,
  fixture: StreamFixture,
  delay: 1_000 | 2_000 | 4_000,
  expectedConnections: number,
  phase: string,
): Promise<void> {
  const before = fixture.metrics();
  expect(fixture.connectionLedger()).toHaveLength(expectedConnections - 1);
  expect(before.getOpens).toBe(expectedConnections - 1);
  await attachClockPhase(page, clock, fixture, phase + "-after-purge");
  await runFor(page, clock, delay - 1);
  expect(fixture.connectionLedger()).toHaveLength(expectedConnections - 1);
  expect(fixture.metrics().getOpens, "no source before the full backoff bound").toBe(
    before.getOpens,
  );
  await attachClockPhase(page, clock, fixture, phase + "-before-bound");
  await runFor(page, clock, 1);
  await attachClockPhase(page, clock, fixture, phase + "-at-bound");
  if (fixture.connectionLedger().length < expectedConnections) {
    // The paused controller checks its deadline on its existing 250ms cadence.
    // Permit one scheduler opportunity, never a loop or a shorter backoff.
    await runFor(page, clock, 250);
    await attachClockPhase(page, clock, fixture, phase + "-scheduler-opportunity");
  }
  await expect
    .poll(() => fixture.connectionLedger().length, { timeout: 10_000 })
    .toBe(expectedConnections);
  expect(fixture.metrics().getOpens).toBe(expectedConnections);
  expect(fixture.metrics().acceptedCreates).toBe(before.acceptedCreates);
  expect(fixture.metrics().acceptedReleases).toBe(before.acceptedReleases);
  expect(fixture.metrics().acceptedRenewals).toBe(before.acceptedRenewals);
  await attachClockPhase(page, clock, fixture, phase + "-source-ready");
}

async function setVisibility(page: Page, state: "hidden" | "visible"): Promise<void> {
  await page.evaluate((nextState) => {
    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      value: nextState,
    });
    document.dispatchEvent(new Event("visibilitychange"));
  }, state);
}

async function setOnline(page: Page, online: boolean): Promise<void> {
  await page.evaluate((nextOnline) => {
    Object.defineProperty(Navigator.prototype, "onLine", {
      configurable: true,
      get: () => nextOnline,
    });
    window.dispatchEvent(new Event(nextOnline ? "online" : "offline"));
  }, online);
}

async function cleanupFixture(
  fixture: StreamFixture,
  pages: readonly Page[],
  expectedReleases: number,
  verify: boolean,
  verifyRelease?: () => void,
): Promise<void> {
  const settle = async () => {
    for (const page of pages) {
      if (page.isClosed()) continue;
      await setVisibility(page, "hidden").catch(() => undefined);
    }
    await expect.poll(() => fixture.metrics().activeLeases, { timeout: 7_000 }).toBe(0);
    await expect.poll(() => fixture.metrics().activeSockets, { timeout: 7_000 }).toBe(0);
    if (verify) {
      await expect
        .poll(() => fixture.metrics().acceptedReleases, { timeout: 7_000 })
        .toBe(expectedReleases);
      const final = fixture.metrics();
      expect(final.sourcesOpened).toBe(final.sourcesClosed);
    }
  };
  try {
    if (verify) await settle();
    else await settle().catch(() => undefined);
    if (verify) verifyRelease?.();
  } finally {
    await fixture.close();
  }
}

function assertNoFallback(page: Page, fixture: StreamFixture): void {
  const metrics = fixture.metrics();
  expect(metrics.legacyQuoteGets).toBe(0);
  expect(metrics.quoteDemandMutations).toBe(0);
  const oldBrowserRequests = observationFor(page).requests.filter(({ pathname }) =>
    retiredQuotePath.test(pathname),
  );
  expect(oldBrowserRequests.map(({ method, pathname }) => ({ method, pathname }))).toEqual([]);
}

async function attachSafeMetrics(
  testInfo: TestInfo,
  name: string,
  fixture: StreamFixture,
): Promise<void> {
  const metrics = fixture.metrics();
  await testInfo.attach(name, {
    contentType: "application/json",
    body: JSON.stringify({
      counters: metrics,
      lease_identity_sets: fixture.leaseLedger().map((lease) =>
        lease.identities.map((identity) => ({
          instrument_id: identity.instrument_id,
          generation: identity.generation,
          membership_id: identity.membership_id,
        })),
      ),
      connections: fixture.connectionLedger().map((connection) => ({
        lease_id: connection.leaseId,
        stream_id: connection.streamId,
        closed: connection.closedAt !== null,
        sent: connection.sent.map(({ kind, eventSequence, invalid }) => ({
          kind,
          event_sequence: eventSequence,
          invalid,
        })),
      })),
    }),
  });
}

function connectionEventData(
  connection: ReturnType<StreamFixture["connectionLedger"]>[number],
  clock: SyntheticClock,
  sequence: string,
  body: unknown,
): string {
  return JSON.stringify(makeEnvelope(connection, clock, sequence, body));
}

async function ensureCurrentUiRows(
  page: Page,
  clock: SyntheticClock,
  fixture: StreamFixture,
  identities: readonly IntradayStreamIdentity[],
): Promise<string> {
  const connection = activeConnection(fixture);
  await emitFrame(
    page,
    clock,
    fixture,
    connection.id,
    "snapshot",
    snapshotBody(fixture, identities, clock),
  );
  return connection.id;
}

test.describe("provider-free Stock Beta intraday integration", () => {
  test.beforeEach(async ({ page }) => {
    observeBrowserRequests(page);
    await installProviderFreeNetworkGuard(page);
  });

  test.afterEach(async ({ page }) => {
    assertProviderFree(page);
  });

  test("updates a READY membership quote without an analysis snapshot or page reload", async ({
    page,
    request,
  }, testInfo) => {
    test.setTimeout(45_000);
    await resetScenario(request, ownerScenario({ stockBeta: "unavailable" }));
    const identities = await readyIdentities(request);
    const identity = identities[0];
    expect(identity).toMatchObject({
      instrument_id: "005930.KRX",
      generation: 1,
      membership_id: "00000000-0000-4000-8000-000000006530",
    });
    if (identity === undefined) throw new Error("Expected the READY A membership");
    const clock = createClock();
    const fixture = await startStockBetaMarketStreamFixture(appOrigin, {
      readMemberships: () => readyIdentities(request),
      now: clock.now,
    });
    let passed = false;
    try {
      const current = await openStockBeta(page, request, fixture, clock);
      expect(current).toEqual([identity]);
      const firstConnection = activeConnection(fixture);
      const documentLoads = observationFor(page).requests.filter(
        ({ method, pathname }) => method === "GET" && pathname === "/stock-beta",
      ).length;
      await sendInitialSnapshot(page, clock, fixture, current);
      await expectLiveQuote(page, "005930.KRX", quoteA);
      await expect(page.getByTestId("stock-beta-dashboard")).toHaveAttribute(
        "data-has-snapshot",
        "false",
      );
      await expect(membershipCard(page, "005930.KRX", "READY")).toBeVisible();
      const advanced = makeRow(identity, clock, {
        price: "101300.00",
        change: "1300.00",
        percent: "1.30",
        quoteVersion: "2",
        receiveOrdinal: "2",
        stateVersion: "2",
        quoteOverrides: { trade_volume: "101", cumulative_volume: "1001" },
      });
      await emitFrame(page, clock, fixture, firstConnection.id, "delta", { rows: [advanced] });
      await pumpAnimationFrames(page, clock);
      await expectLiveQuote(page, "005930.KRX", "101300.00");
      await expect(selectedQuote(page)).toContainText("+1,300");
      await expect(selectedQuote(page)).toContainText("+1.30%");
      expect(
        observationFor(page).requests.filter(
          ({ method, pathname }) => method === "GET" && pathname === "/stock-beta",
        ),
      ).toHaveLength(documentLoads);
      const metrics = fixture.metrics();
      expect(metrics.acceptedCreates).toBe(1);
      expect(metrics.acceptedRenewals).toBe(0);
      expect(metrics.snapshots).toBe(1);
      expect(metrics.deltas).toBe(1);
      expect(metrics.connections).toBe(1);
      assertNoFallback(page, fixture);
      await attachSafeMetrics(testInfo, "stock-beta-ws-case-1", fixture);
      passed = true;
    } finally {
      await cleanupFixture(fixture, [page], 1, passed);
    }
  });

  test("renders the signed current quote beside an unchanged EOD chart on dashboard and detail", async ({
    page,
    request,
  }, testInfo) => {
    test.setTimeout(90_000);
    await resetScenario(request, ownerScenario());
    const clock = createClock();
    const fixture = await startStockBetaMarketStreamFixture(appOrigin, {
      readMemberships: () => readyIdentities(request),
      now: clock.now,
    });
    let passed = false;
    try {
      const identities = await openStockBeta(page, request, fixture, clock);
      expect(identities).toHaveLength(1);
      const identity = identities[0];
      if (identity === undefined) throw new Error("Expected READY A before dashboard render");
      const dashboard = dashboardQuoteWidget(page);
      const chart = page.getByTestId("stock-beta-price-chart");
      await expect(chart).toBeVisible();
      await expect(chart).toHaveAttribute("data-renderer-status", "ready");
      await expect(chart.getByTestId("stock-beta-price-chart-summary")).toContainText("2026-08-28");
      await expect(chart.getByTestId("stock-beta-price-chart-summary")).toContainText(
        "Open: 604,945 · High: 605,040 · Low: 604,925 · Close: 605,020 · Volume: 6,035,000",
      );
      await expect
        .poll(async () => {
          await pumpAnimationFrames(page, clock);
          return chart.getByTestId("stock-beta-price-chart-canvas").evaluate((host) => {
            const canvas = Array.from(host.querySelectorAll("canvas")).find(
              (candidate) => candidate.style.zIndex === "1" && candidate.width > 100,
            );
            const context = canvas?.getContext("2d", { willReadFrequently: true });
            if (canvas === undefined || context === null || context === undefined) return 0;
            const pixels = context.getImageData(0, 0, canvas.width, canvas.height).data;
            const colors = new Set<string>();
            for (let offset = 0; offset < pixels.length; offset += 24) {
              if ((pixels[offset + 3] ?? 0) > 0) {
                colors.add(`${pixels[offset]},${pixels[offset + 1]},${pixels[offset + 2]}`);
              }
            }
            return colors.size;
          });
        })
        .toBeGreaterThan(5);
      await expect(page.getByRole("tabpanel", { name: "Price" })).toContainText("605020");
      const connection = activeConnection(fixture);
      await sendInitialSnapshot(page, clock, fixture, identities);
      await emitFrame(page, clock, fixture, connection.id, "status", {
        connection: "CONNECTED",
        reason_code: null,
        gap_open: false,
        session_has_gap: false,
        gap_generation: "0",
      });
      await expectLiveQuote(page, identity.instrument_id, quoteA);
      await expect(dashboard).toContainText("101,200.00");
      await expect(dashboard).toContainText("+1,200");
      await expect(dashboard).toContainText("+1.20%");
      await expect(dashboard).toContainText("Not provided by this channel");
      await expect(boardRow(page, identity.instrument_id).locator("time[datetime]")).toHaveCount(2);
      const chartRequestsBeforeDelta = observationFor(page).requests.filter(({ pathname }) =>
        pathname.endsWith("/chart"),
      ).length;
      const snapshotStrip = page.getByTestId("stock-beta-snapshot-strip");
      const snapshotAsOf = await snapshotStrip
        .getByTestId("stock-beta-snapshot-as-of")
        .locator("data")
        .getAttribute("value");
      const snapshotId = await snapshotStrip
        .getByTestId("stock-beta-snapshot-statuses")
        .locator("strong")
        .first()
        .textContent();
      const signalGeneration = await rankedRow(page, identity.instrument_id)
        .getByRole("button")
        .textContent();
      expect(snapshotAsOf).not.toBeNull();
      expect(snapshotId).not.toBeNull();
      expect(signalGeneration).toContain("Generation " + String(identity.generation));
      const canvas = chart
        .getByTestId("stock-beta-price-chart-canvas")
        .locator('td[style*="position: relative"] canvas[style*="z-index: 1"]')
        .first();
      const bounds = await canvas.boundingBox();
      if (bounds === null) throw new Error("The visible synthetic price plot has no bounds");
      let hoveredLatest = false;
      let latestCandleOffset = 0;
      for (let offset = 1; offset <= 32; offset += 1) {
        await page.mouse.move(bounds.x + bounds.width - offset, bounds.y + bounds.height / 2);
        await pumpAnimationFrames(page, clock);
        const summary = await chart.getByTestId("stock-beta-price-chart-summary").textContent();
        if (summary?.includes("2026-08-28") === true) {
          const tooltip = chart.getByTestId("stock-beta-price-chart-tooltip");
          const values = await tooltip.textContent();
          if (
            values?.includes("604,945") === true &&
            values.includes("605,040") &&
            values.includes("604,925") &&
            values.includes("605,020") &&
            values.includes("6,035,000")
          ) {
            hoveredLatest = true;
            latestCandleOffset = offset;
            break;
          }
        }
      }
      expect(hoveredLatest, "the actual final synthetic candle is pointer-selected").toBe(true);
      const delta = makeRow(identity, clock, {
        price: "101300.00",
        change: "1300.00",
        percent: "1.30",
        quoteVersion: "2",
        receiveOrdinal: "2",
        stateVersion: "2",
      });
      await emitFrame(page, clock, fixture, connection.id, "delta", { rows: [delta] });
      await pumpAnimationFrames(page, clock);
      await expectLiveQuote(page, identity.instrument_id, "101300.00");
      expect(
        observationFor(page).requests.filter(({ pathname }) => pathname.endsWith("/chart")),
      ).toHaveLength(chartRequestsBeforeDelta);
      await expect(
        snapshotStrip.getByTestId("stock-beta-snapshot-as-of").locator("data"),
      ).toHaveAttribute("value", snapshotAsOf ?? "");
      await expect(
        snapshotStrip.getByTestId("stock-beta-snapshot-statuses").locator("strong").first(),
      ).toHaveText(snapshotId ?? "");
      await expect(rankedRow(page, identity.instrument_id).getByRole("button")).toContainText(
        signalGeneration ?? "",
      );
      await expect(chart).toHaveAttribute("data-renderer-status", "ready");
      await page.mouse.move(
        bounds.x + bounds.width - latestCandleOffset,
        bounds.y + bounds.height / 2,
      );
      await pumpAnimationFrames(page, clock);
      await expect(chart.getByTestId("stock-beta-price-chart-summary")).toContainText("2026-08-28");
      await expect(chart.getByTestId("stock-beta-price-chart-summary")).toContainText(
        "Open: 604,945 · High: 605,040 · Low: 604,925 · Close: 605,020 · Volume: 6,035,000",
      );
      for (const value of ["604,945", "605,040", "604,925", "605,020", "6,035,000"]) {
        await expect(chart.getByTestId("stock-beta-price-chart-tooltip")).toContainText(value);
      }
      await expect(page.getByRole("tabpanel", { name: "Price" })).toContainText("605020");

      const dashboardConsumer = activeLease(fixture).consumerId;
      await rankedRow(page, "005930.KRX").getByRole("link", { name: "Open detail" }).click();
      await expect(page).toHaveURL(/\/stock-beta\/005930\.KRX$/);
      await expect(page.getByTestId("stock-beta-detail-returns")).toContainText("Returns");
      const detailWidget = detailQuoteWidget(page);
      await expect(detailWidget).toHaveCount(1);
      await expect.poll(() => fixture.metrics().acceptedCreates, { timeout: 10_000 }).toBe(2);
      await expect.poll(() => fixture.connectionLedger().length, { timeout: 10_000 }).toBe(2);
      const detailLease = activeLease(fixture);
      expect(detailLease.consumerId).not.toBe(dashboardConsumer);
      const detailConnection = activeConnection(fixture, detailLease.leaseId);
      const detailProvenance = page.getByTestId("stock-beta-provenance").locator("dd");
      const detailHeader = page.getByTestId("stock-beta-instrument-header");
      const detailReturns = page.getByTestId("stock-beta-detail-returns").locator("data[value]");
      await expect(detailProvenance).toHaveCount(5);
      await expect(detailProvenance.nth(0)).toHaveText(snapshotId ?? "");
      await expect(detailProvenance.nth(1)).toHaveText(snapshotAsOf ?? "");
      await expect(detailHeader).toContainText("Generation " + String(identity.generation));
      await expect(detailReturns).toHaveCount(3);
      const detailProvenanceBefore = await detailProvenance.allTextContents();
      const detailHeaderBefore = await detailHeader.textContent();
      const detailReturnsBefore = await detailReturns.allTextContents();
      expect(detailProvenanceBefore.every((value) => value.trim().length > 0)).toBe(true);
      expect(detailHeaderBefore?.trim().length).toBeGreaterThan(0);
      for (const value of detailReturnsBefore) {
        expect(value.trim().length).toBeGreaterThan(0);
        expect(Number.isFinite(Number(value))).toBe(true);
      }
      const expectDetailAnalysisUnchanged = async () => {
        await expect(detailProvenance).toHaveText(detailProvenanceBefore);
        await expect(detailHeader).toHaveText(detailHeaderBefore ?? "");
        await expect(detailReturns).toHaveText(detailReturnsBefore);
        for (const [index, value] of detailReturnsBefore.entries()) {
          await expect(detailReturns.nth(index)).toHaveAttribute("value", value);
        }
      };
      await emitFrame(
        page,
        clock,
        fixture,
        detailConnection.id,
        "snapshot",
        snapshotBody(fixture, identities, clock),
      );
      await emitFrame(page, clock, fixture, detailConnection.id, "status", {
        connection: "CONNECTED",
        reason_code: null,
        gap_open: false,
        session_has_gap: false,
        gap_generation: "0",
      });
      await expect(detailWidget.getByTestId("stock-beta-stream-selected")).toHaveAttribute(
        "data-availability",
        "LIVE",
      );
      await expect(detailWidget.locator('strong[data-quote-value="' + quoteA + '"]')).toHaveCount(
        1,
      );
      await expectDetailAnalysisUnchanged();
      for (const width of [900, 390]) {
        await page.setViewportSize({ height: 900, width });
        await expectDetailAnalysisUnchanged();
        const selected = detailWidget.getByTestId("stock-beta-stream-selected");
        await expect(selected.locator("time[datetime]")).toHaveCount(2);
        const geometry = await selected.evaluate((element) => {
          const root = element.getBoundingClientRect();
          const price = element.querySelector("strong");
          const receipt = element.querySelectorAll("time").item(1);
          if (price === null || receipt === null) return false;
          const priceBox = price.getBoundingClientRect();
          const receiptBox = receipt.getBoundingClientRect();
          return (
            root.width > 0 &&
            priceBox.left >= root.left - 1 &&
            priceBox.right <= root.right + 1 &&
            receiptBox.left >= root.left - 1 &&
            receiptBox.right <= root.right + 1
          );
        });
        expect(geometry, "detail price and receipt stay inside the selected widget").toBe(true);
        await detailWidget.screenshot({
          animations: "disabled",
          path: testInfo.outputPath("stock-beta-ws-detail-" + String(width) + ".png"),
        });
      }
      await page.setViewportSize({ height: 900, width: 1280 });
      const metrics = fixture.metrics();
      expect(metrics.acceptedCreates).toBe(2);
      expect(metrics.acceptedRenewals).toBe(0);
      expect(metrics.snapshots).toBe(2);
      expect(metrics.deltas).toBe(1);
      expect(metrics.statuses).toBe(2);
      assertNoFallback(page, fixture);
      await attachSafeMetrics(testInfo, "stock-beta-ws-case-2", fixture);
      passed = true;
    } finally {
      await cleanupFixture(fixture, [page], 2, passed);
    }
  });

  test("selects B without replacing the board lease and rejects stale delivery", async ({
    page,
    request,
  }, testInfo) => {
    test.setTimeout(60_000);
    const identities = await prepareAAndB(request);
    const clock = createClock();
    const fixture = await startStockBetaMarketStreamFixture(appOrigin, {
      readMemberships: () => readyIdentities(request),
      now: clock.now,
    });
    let passed = false;
    try {
      const current = await openStockBeta(page, request, fixture, clock);
      expect(current).toEqual(identities);
      const lease = activeLease(fixture);
      expect(lease.identities).toEqual(identities);
      const connection = activeConnection(fixture, lease.leaseId);
      await sendInitialSnapshot(page, clock, fixture, identities);
      await expectLiveQuote(page, "005930.KRX", quoteA);
      const b = identities.find((identity) => identity.instrument_id === "000660.KRX");
      if (b === undefined) throw new Error("Expected the admitted B identity");
      await boardRow(page, b.instrument_id).locator("button").click();
      await expect(selectedQuote(page)).toContainText("000660.KRX");
      await expectLiveQuote(page, b.instrument_id, quoteB);
      await expect(selectedQuote(page)).toContainText("-1,600");
      await expect(selectedQuote(page)).toContainText("-0.80%");
      const bReceipt = await boardRow(page, b.instrument_id).getAttribute("data-received-at");
      const bVersion = await boardRow(page, b.instrument_id).getAttribute("data-quote-version");
      const bEpoch = await boardRow(page, b.instrument_id).getAttribute("data-epoch");
      const bOrdinal = await boardRow(page, b.instrument_id).getAttribute("data-receive-ordinal");
      const a = identities.find((identity) => identity.instrument_id === "005930.KRX");
      if (a === undefined) throw new Error("Expected the admitted A identity");
      const beforeRejectedDeltas = fixture.metrics();
      const beforeRejectedFrames = fixture.connectionLedger();
      for (const mismatched of [
        { ...b, generation: b.generation + 1 },
        { ...b, instrument_id: a.instrument_id },
      ]) {
        expect(() =>
          fixture.emit(connection.id, "delta", { rows: [makeRow(mismatched, clock)] }),
        ).toThrow(IntradayStreamContractError);
        expect(fixture.metrics()).toEqual(beforeRejectedDeltas);
        expect(fixture.connectionLedger()).toEqual(beforeRejectedFrames);
      }
      const nextA = makeRow(a, clock, {
        price: "101300.00",
        change: "1300.00",
        percent: "1.30",
        quoteVersion: "2",
        receiveOrdinal: "2",
        stateVersion: "2",
      });
      await emitFrame(page, clock, fixture, connection.id, "delta", { rows: [nextA] });
      await pumpAnimationFrames(page, clock);
      await expect(boardRow(page, a.instrument_id)).toHaveAttribute("data-quote-version", "2");
      await expect(
        boardRow(page, a.instrument_id).locator('td[data-quote-value="101300.00"]'),
      ).toHaveCount(1);
      await expect(selectedQuote(page)).toContainText(b.instrument_id);
      await expectLiveQuote(page, b.instrument_id, quoteB);
      await expect(boardRow(page, b.instrument_id)).toHaveAttribute(
        "data-received-at",
        bReceipt ?? "",
      );
      await expect(boardRow(page, b.instrument_id)).toHaveAttribute(
        "data-quote-version",
        bVersion ?? "",
      );
      await expect(boardRow(page, b.instrument_id)).toHaveAttribute("data-epoch", bEpoch ?? "");
      await expect(boardRow(page, b.instrument_id)).toHaveAttribute(
        "data-receive-ordinal",
        bOrdinal ?? "",
      );
      const connectionAfterDelta = fixture
        .connectionLedger()
        .find((item) => item.id === connection.id);
      if (connectionAfterDelta === undefined) throw new Error("Missing first source ledger");
      const duplicateRows = [makeRow(a, clock, { price: "101999.00" }), makeRow(b, clock)];
      const duplicateBody = {
        lease_id: lease.leaseId,
        lease_expires_at: lease.expiresAt,
        rows: duplicateRows,
      };
      const duplicateData = connectionEventData(connectionAfterDelta, clock, "1", duplicateBody);
      fixture.emitInvalid(
        connection.id,
        { kind: "snapshot", data: duplicateData },
        connection.streamId + ":1",
      );
      await pumpAnimationFrames(page, clock);
      await expect(boardRow(page, a.instrument_id)).toHaveAttribute("data-quote-version", "2");
      await expectLiveQuote(page, b.instrument_id, quoteB);
      await expect(boardRow(page, b.instrument_id)).toHaveAttribute(
        "data-received-at",
        bReceipt ?? "",
      );
      await expect(boardRow(page, b.instrument_id)).toHaveAttribute(
        "data-quote-version",
        bVersion ?? "",
      );
      await expect(boardRow(page, b.instrument_id)).toHaveAttribute("data-epoch", bEpoch ?? "");
      await expect(boardRow(page, b.instrument_id)).toHaveAttribute(
        "data-receive-ordinal",
        bOrdinal ?? "",
      );
      const wrongGeneration = makeRow({ ...b, generation: b.generation + 1 }, clock, {
        price: "1.00",
      });
      const invalidSequence = nextSequence(connectionAfterDelta);
      const invalidData = connectionEventData(connectionAfterDelta, clock, invalidSequence, {
        rows: [wrongGeneration],
      });
      await attachClockPhase(page, clock, fixture, "case-3-wrong-generation-before-delivery");
      fixture.emitInvalid(
        connection.id,
        { kind: "delta", data: invalidData },
        connection.streamId + ":" + invalidSequence,
      );
      await expect(boardRow(page, a.instrument_id)).toHaveAttribute(
        "data-availability",
        "UNAVAILABLE",
      );
      await expectNoCapturedQuote(page, a.instrument_id);
      await expectNoCapturedQuote(page, "000660.KRX");
      await advanceReconnectBoundary(page, clock, fixture, 1_000, 2, "case-3-wrong-generation");
      const recovered = activeConnection(fixture, lease.leaseId);
      expect(recovered.streamId).not.toBe(connection.streamId);
      await emitFrame(page, clock, fixture, recovered.id, "reset", {
        reason_code: "RESYNC_REQUIRED",
      });
      await emitFrame(
        page,
        clock,
        fixture,
        recovered.id,
        "snapshot",
        snapshotBody(fixture, identities, clock),
      );
      await pumpAnimationFrames(page, clock);
      await expect(boardRow(page, a.instrument_id)).toHaveAttribute("data-availability", "LIVE");
      await expect(
        boardRow(page, a.instrument_id).locator('td[data-quote-value="' + quoteA + '"]'),
      ).toHaveCount(1);
      await expect(selectedQuote(page)).toContainText(b.instrument_id);
      await expectLiveQuote(page, "000660.KRX", quoteB);
      const metrics = fixture.metrics();
      expect(metrics.acceptedCreates).toBe(1);
      expect(metrics.acceptedRenewals).toBe(0);
      expect(metrics.snapshots).toBe(3);
      expect(metrics.deltas).toBe(2);
      expect(metrics.resets).toBe(1);
      expect(metrics.invalidDeltas).toBe(1);
      expect(metrics.invalidSnapshots).toBe(1);
      assertNoFallback(page, fixture);
      await attachSafeMetrics(testInfo, "stock-beta-ws-case-3", fixture);
      passed = true;
    } finally {
      await cleanupFixture(fixture, [page], 1, passed);
    }
  });

  test("adds 005930 to READY, disables it, and re-registers the same instrument with a new identity", async ({
    page,
    request,
  }, testInfo) => {
    test.setTimeout(120_000);
    await resetScenario(
      request,
      ownerScenario({
        stockBetaRows: 0,
        stockBetaSeed: "empty",
        stockBetaLifecycle: "controlled",
      }),
    );
    const clock = createClock();
    const membershipAdvancePath = "/__test/stock-beta/membership-advance";
    const fixture = await startStockBetaMarketStreamFixture(appOrigin, {
      readMemberships: () => readyIdentities(request),
      now: clock.now,
    });
    const expectedAdmissionTransitions = [
      "REQUESTED",
      "VALIDATING",
      "BACKFILLING",
      "MATERIALIZING",
      "READY",
    ];
    const controlledTransitions = [
      {
        expectedLifecycle: "REQUESTED",
        expectedGeneration: 0,
        lifecycle: "VALIDATING",
        generation: 0,
        observedSessions: 20,
      },
      {
        expectedLifecycle: "VALIDATING",
        expectedGeneration: 0,
        lifecycle: "BACKFILLING",
        generation: 0,
        observedSessions: 80,
      },
      {
        expectedLifecycle: "BACKFILLING",
        expectedGeneration: 0,
        lifecycle: "MATERIALIZING",
        generation: 0,
        observedSessions: 200,
      },
      {
        expectedLifecycle: "MATERIALIZING",
        expectedGeneration: 0,
        lifecycle: "READY",
        generation: 1,
        observedSessions: 261,
      },
    ] as const;
    async function freshInvalidEmission(
      captured: ReturnType<StreamFixture["connectionLedger"]>[number],
      expectedPrior: 1 | 2,
      phase: string,
    ) {
      const connection = fixture.connectionLedger().find((item) => item.id === captured.id);
      if (connection === undefined) throw new Error("Captured SSE connection is missing");
      expect(connection.closedAt, "invalid frame uses the exact open source").toBeNull();
      expect(connection.leaseId).toBe(captured.leaseId);
      expect(connection.streamId).toBe(captured.streamId);
      const prior = connection.sent.reduce(
        (maximum, frame) => Math.max(maximum, Number(frame.eventSequence)),
        0,
      );
      const sequence = nextSequence(connection);
      expect(prior).toBe(expectedPrior);
      expect(sequence).toBe(String(expectedPrior + 1));
      expect(connection.sent.every((frame) => BigInt(sequence) > BigInt(frame.eventSequence))).toBe(
        true,
      );
      await testInfo.attach("case-4-fresh-invalid-" + phase, {
        contentType: "application/json",
        body: Buffer.from(
          JSON.stringify({
            phase,
            priorSequence: prior,
            nextSequence: Number(sequence),
            exactConnection: connection.id === captured.id,
            sameLease: connection.leaseId === captured.leaseId,
            sameStream: connection.streamId === captured.streamId,
          }),
        ),
      });
      return { connection, sequence };
    }

    async function expectPurgedSource(
      connectionId: string,
      expectedConnections: 2 | 3,
    ): Promise<void> {
      await expect
        .poll(
          () => {
            const connection = fixture.connectionLedger().find((item) => item.id === connectionId);
            return connection !== undefined && connection.closedAt !== null;
          },
          { timeout: 5_000 },
        )
        .toBe(true);
      expect(fixture.connectionLedger()).toHaveLength(expectedConnections);
      expect(fixture.metrics()).toMatchObject({
        getOpens: expectedConnections,
        acceptedCreates: 2,
        acceptedReleases: 1,
        acceptedRenewals: 0,
      });
    }

    async function admitAndObserveTransitions(phase: "initial" | "replacement") {
      const uiListResponses: Response[] = [];
      const observeUiList = (response: Response): void => {
        const url = new URL(response.url());
        if (
          url.origin === new URL(appOrigin).origin &&
          url.pathname === membershipPath &&
          url.search === "" &&
          response.request().method() === "GET"
        ) {
          uiListResponses.push(response);
        }
      };
      // Passive observation of the app's existing refreshes: no new GET and
      // no request/response rewriting. Cards render in this list's order.
      page.on("response", observeUiList);
      try {
        const receiptResponse = page.waitForResponse(
          (response) => {
            const url = new URL(response.url());
            return (
              url.origin === new URL(appOrigin).origin &&
              url.pathname === membershipPath &&
              url.search === "" &&
              response.request().method() === "POST"
            );
          },
          { timeout: 5_000 },
        );
        const [response] = await Promise.all([
          receiptResponse,
          page.getByRole("button", { name: "Add instrument" }).click(),
        ]);
        expect(response.status(), "actual UI membership POST acceptance").toBe(202);
        const receipt = ownerEquityV2MutationSchema.parse(await response.json());
        const admitted = receipt.resource;
        expect(admitted.lifecycle, "POST receipt lifecycle, not a DOM state").toBe("REQUESTED");
        expect(admitted.instrument_id).toBe("005930.KRX");
        expect(admitted.generation, "unmaterialized POST receipt generation").toBe(0);
        expect(receipt.duplicate_active).toBe(false);
        const domTransitions: string[] = [];
        let elapsed = 0;
        let heldLifecycle = admitted.lifecycle;
        let heldGeneration = admitted.generation;
        for (const transition of controlledTransitions) {
          expect(heldLifecycle).toBe(transition.expectedLifecycle);
          expect(heldGeneration).toBe(transition.expectedGeneration);
          const advanceResponse = await request.post(syntheticOrigin + membershipAdvancePath, {
            data: {
              membership_id: admitted.id,
              expected_generation: heldGeneration,
              expected_lifecycle: heldLifecycle,
            },
          });
          expect(advanceResponse.status(), "controlled membership advancement").toBe(200);
          const held = ownerEquityV2MembershipStatusSchema.parse(
            await advanceResponse.json(),
          ).membership;
          expect(held.id).toBe(admitted.id);
          expect(held.instrument_id).toBe(admitted.instrument_id);
          expect(held.requested_at).toBe(admitted.requested_at);
          expect(held.lifecycle).toBe(transition.lifecycle);
          expect(held.generation).toBe(transition.generation);
          expect(held.coverage).toMatchObject({
            minimum_observed_sessions: 121,
            observed_sessions: transition.observedSessions,
            target_observed_sessions: 261,
          });

          const expectedStateIndex = expectedAdmissionTransitions.indexOf(transition.lifecycle);
          let renderedHeldState = false;
          while (!renderedHeldState && elapsed < 90_000) {
            for (const uiResponse of uiListResponses.splice(0)) {
              expect(uiResponse.status(), "existing UI membership refresh").toBe(200);
              const list = ownerEquityV2MembershipListSchema.parse(await uiResponse.json());
              const index = list.memberships.findIndex((item) => item.id === admitted.id);
              if (index < 0) {
                expect(
                  list.memberships.filter((item) => item.lifecycle !== "DISABLED"),
                  "only an earlier empty active universe may precede the admission refresh",
                ).toHaveLength(0);
                continue;
              }
              const membership = list.memberships[index];
              if (membership === undefined) throw new Error("Missing admitted UI membership");
              expect(membership.id, "UI refresh retains the exact POST identity").toBe(admitted.id);
              expect(membership.instrument_id).toBe(admitted.instrument_id);
              const responseStateIndex = expectedAdmissionTransitions.indexOf(membership.lifecycle);
              if (responseStateIndex < 0) {
                throw new Error("Synthetic membership admission stopped before READY");
              }
              if (responseStateIndex < expectedStateIndex) continue;
              expect(membership.lifecycle, "UI response matches the held phase").toBe(
                transition.lifecycle,
              );
              expect(membership.generation, "held-phase admission generation").toBe(
                transition.generation,
              );
              expect(
                list.memberships.filter(
                  (item) =>
                    item.instrument_id === admitted.instrument_id && item.lifecycle !== "DISABLED",
                ),
              ).toHaveLength(1);
              // MembershipRow exposes generation and lifecycle, but not its ID.
              // Bind the DOM ordinal to the exact ID in the real typed UI list.
              const card = membershipCards(page).nth(index);
              await expect(membershipCards(page)).toHaveCount(list.memberships.length);
              await expect(card.locator("strong")).toHaveText(admitted.instrument_id);
              await expect(card.locator("small")).toHaveText(`Generation ${transition.generation}`);
              await expect(card).toHaveAttribute("data-lifecycle", transition.lifecycle);
              renderedHeldState = true;
            }
            if (!renderedHeldState) {
              await runFor(page, clock, 250);
              elapsed += 250;
            }
          }
          expect(renderedHeldState, "held lifecycle rendered by the real UI").toBe(true);
          domTransitions.push(transition.lifecycle);
          heldLifecycle = held.lifecycle;
          heldGeneration = held.generation;
        }
        expect(domTransitions, "four states rendered for the POST identity").toEqual(
          expectedAdmissionTransitions.slice(1),
        );
        expect(
          [admitted.lifecycle, ...domTransitions],
          "POST receipt plus actual DOM sequence",
        ).toEqual(expectedAdmissionTransitions);
        await testInfo.attach("stock-beta-ws-admission-" + phase, {
          contentType: "application/json",
          body: Buffer.from(
            JSON.stringify({
              receipt: { lifecycle: admitted.lifecycle, generation: admitted.generation },
              domTransitions,
              readyGeneration: 1,
              identityMatched: true,
            }),
          ),
        });
        return admitted;
      } finally {
        page.off("response", observeUiList);
      }
    }
    let passed = false;
    try {
      const empty = await openStockBeta(page, request, fixture, clock);
      expect(empty).toEqual([]);
      await expect(board(page).getByRole("status")).toHaveText("Paused");
      await expectNoCapturedQuote(page, "005930.KRX");
      expect(fixture.metrics().acceptedCreates).toBe(0);
      await page.getByLabel("KRX code").fill("005930");
      const firstAdmission = await admitAndObserveTransitions("initial");
      const firstCard = membershipCard(page, "005930.KRX");
      await expect(rankedRow(page, "005930.KRX")).toBeVisible();
      await waitForAcceptedLease(fixture, 1, 1);
      const firstIdentity = (await readyIdentities(request))[0];
      if (firstIdentity === undefined) throw new Error("initial A membership was not READY");
      expect(firstIdentity.membership_id, "READY identity equals actual POST identity").toBe(
        firstAdmission.id,
      );
      expect(firstIdentity.instrument_id).toBe("005930.KRX");
      expect(firstIdentity.generation).toBe(1);
      const firstLease = activeLease(fixture);
      const firstConnection = activeConnection(fixture, firstLease.leaseId);
      await emitFrame(
        page,
        clock,
        fixture,
        firstConnection.id,
        "snapshot",
        snapshotBody(fixture, [firstIdentity], clock),
      );
      await expectLiveQuote(page, "005930.KRX", quoteA);

      await firstCard.getByRole("button", { name: "Disable" }).click();
      await firstCard.getByRole("button", { name: "Confirm disable" }).click();
      await expect(firstCard).toHaveAttribute("data-lifecycle", "DISABLED", { timeout: 20_000 });
      await expectNoCapturedQuote(page, "005930.KRX");
      await expect.poll(() => fixture.metrics().acceptedReleases, { timeout: 7_000 }).toBe(1);
      await expect.poll(() => fixture.metrics().activeSockets, { timeout: 7_000 }).toBe(0);

      const stalePost = await page.evaluate(
        async (input) => {
          const response = await fetch(input.path, {
            method: "POST",
            headers: {
              "content-type": "application/json",
              "x-csrf-token": "synthetic-stale-membership-probe",
              "idempotency-key": input.idempotency,
            },
            body: JSON.stringify(input.body),
          });
          return { status: response.status, body: await response.json() };
        },
        {
          path: appOrigin + leasePath,
          idempotency: randomUUID(),
          body: {
            schema_version: 2,
            consumer_id: randomUUID(),
            renewal_sequence: 0,
            identities: [firstIdentity],
          },
        },
      );
      expect(stalePost.status).toBe(404);
      expect(stalePost.body).toMatchObject({ error: { code: "RESOURCE_NOT_FOUND" } });
      const staleGet = await page.evaluate(
        async (input) => {
          const response = await fetch(input.path, { cache: "no-store" });
          return { status: response.status, body: await response.json() };
        },
        {
          path: appOrigin + streamPath + "?lease_id=" + firstLease.leaseId,
        },
      );
      expect(staleGet.status).toBe(404);
      expect(staleGet.body).toMatchObject({ error: { code: "RESOURCE_NOT_FOUND" } });

      await page.getByLabel("KRX code").fill("005930");
      const replacementAdmission = await admitAndObserveTransitions("replacement");
      expect(replacementAdmission.id).not.toBe(firstAdmission.id);
      const afterReadd = await readyIdentities(request);
      const replacement = afterReadd.find((identity) => identity.instrument_id === "005930.KRX");
      if (replacement === undefined) throw new Error("replacement identity was not READY");
      expect(
        replacement.membership_id,
        "replacement READY identity equals actual POST identity",
      ).toBe(replacementAdmission.id);
      expect(replacement.membership_id).not.toBe(firstIdentity.membership_id);
      expect(replacement.generation).toBe(firstIdentity.generation);
      await waitForAcceptedLease(fixture, 2, 2);
      const secondLease = activeLease(fixture);
      const secondConnection = activeConnection(fixture, secondLease.leaseId);
      await emitFrame(
        page,
        clock,
        fixture,
        secondConnection.id,
        "snapshot",
        snapshotBody(fixture, [replacement], clock, {
          price: "106030.00",
          change: "6030.00",
          percent: "6.03",
        }),
      );
      await expectLiveQuote(page, "005930.KRX", "106030.00");

      const oldMemberRow = makeRow(firstIdentity, clock, { price: "9.00" });
      await attachClockPhase(page, clock, fixture, "case-4-stale-membership-before-delivery");
      const { connection: staleMemberConnection, sequence: staleMemberSequence } =
        await freshInvalidEmission(secondConnection, 1, "stale-membership");
      fixture.emitInvalid(
        staleMemberConnection.id,
        {
          kind: "delta",
          data: connectionEventData(staleMemberConnection, clock, staleMemberSequence, {
            rows: [oldMemberRow],
          }),
        },
        staleMemberConnection.streamId + ":" + staleMemberSequence,
      );
      await expectNoCapturedQuote(page, "005930.KRX");
      await expectPurgedSource(staleMemberConnection.id, 2);
      await advanceReconnectBoundary(page, clock, fixture, 1_000, 3, "case-4-stale-membership");
      let recoveryConnection = activeConnection(fixture, secondLease.leaseId);
      await emitFrame(page, clock, fixture, recoveryConnection.id, "reset", {
        reason_code: "RESYNC_REQUIRED",
      });
      await emitFrame(
        page,
        clock,
        fixture,
        recoveryConnection.id,
        "snapshot",
        snapshotBody(fixture, [replacement], clock, {
          price: "106030.00",
          change: "6030.00",
          percent: "6.03",
        }),
      );
      await expectLiveQuote(page, "005930.KRX", "106030.00");

      const wrongGeneration = makeRow(
        { ...replacement, generation: replacement.generation + 1 },
        clock,
        { price: "8.00" },
      );
      await attachClockPhase(page, clock, fixture, "case-4-wrong-generation-before-delivery");
      const { connection: wrongGenerationConnection, sequence: wrongGenerationSequence } =
        await freshInvalidEmission(recoveryConnection, 2, "wrong-generation");
      fixture.emitInvalid(
        wrongGenerationConnection.id,
        {
          kind: "delta",
          data: connectionEventData(wrongGenerationConnection, clock, wrongGenerationSequence, {
            rows: [wrongGeneration],
          }),
        },
        wrongGenerationConnection.streamId + ":" + wrongGenerationSequence,
      );
      await expectNoCapturedQuote(page, "005930.KRX");
      await expectPurgedSource(wrongGenerationConnection.id, 3);
      await advanceReconnectBoundary(page, clock, fixture, 2_000, 4, "case-4-wrong-generation");
      recoveryConnection = activeConnection(fixture, secondLease.leaseId);
      await emitFrame(page, clock, fixture, recoveryConnection.id, "reset", {
        reason_code: "RESYNC_REQUIRED",
      });
      await emitFrame(
        page,
        clock,
        fixture,
        recoveryConnection.id,
        "snapshot",
        snapshotBody(fixture, [replacement], clock, {
          price: "106030.00",
          change: "6030.00",
          percent: "6.03",
        }),
      );
      await expectLiveQuote(page, "005930.KRX", "106030.00");
      const metrics = fixture.metrics();
      expect(metrics.acceptedCreates).toBe(2);
      expect(metrics.acceptedRenewals).toBe(0);
      expect(metrics.acceptedReleases).toBe(1);
      expect(metrics.rejectedCreates).toBe(1);
      expect(metrics.rejectedReadProbes).toBe(1);
      expect(metrics.getOpens).toBe(4);
      expect(metrics.snapshots).toBe(4);
      expect(metrics.resets).toBe(2);
      expect(metrics.invalidDeltas).toBe(2);
      assertNoFallback(page, fixture);
      await attachSafeMetrics(testInfo, "stock-beta-ws-case-4", fixture);
      passed = true;
    } finally {
      await cleanupFixture(fixture, [page], 2, passed);
    }
  });

  test("isolates multi-tab board leases and fails closed at typed capacity", async ({
    browser,
    request,
  }, testInfo) => {
    test.setTimeout(90_000);
    const identities = await prepareAAndB(request);
    const clock = createClock();
    const partialSetupFailure = new Error("INJECTED_CASE5_PARTIAL_SETUP");
    type OwnedPhase = {
      readonly fixture: StreamFixture;
      readonly contexts: readonly BrowserContext[];
      readonly pages: readonly Page[];
      readonly closedContexts: Set<BrowserContext>;
      readonly cleanupFailures: string[];
      readonly newPage: () => Promise<Page>;
      fixtureClosed: boolean;
    };

    async function ownedPhase(
      expectedReleases: number,
      body: (scope: OwnedPhase) => Promise<void>,
      failureMode?: "partial-setup" | "release-assertion",
    ): Promise<StreamFixture> {
      const contexts: BrowserContext[] = [];
      const pages: Page[] = [];
      let fixture: StreamFixture | undefined;
      let scope: OwnedPhase | undefined;
      let passed = false;
      let bodyFailed = false;
      let primaryFailure: unknown;
      let cleanupFailed = false;
      let cleanupFailure: unknown;
      const recordCleanupFailure = (stage: string, error: unknown) => {
        scope?.cleanupFailures.push(stage);
        if (!cleanupFailed) cleanupFailure = error;
        cleanupFailed = true;
      };
      try {
        fixture = await startStockBetaMarketStreamFixture(appOrigin, {
          readMemberships: () => readyIdentities(request),
          now: clock.now,
        });
        scope = {
          fixture,
          contexts,
          pages,
          closedContexts: new Set<BrowserContext>(),
          cleanupFailures: [],
          fixtureClosed: false,
          newPage: async () => {
            const context = await browser.newContext({ baseURL: appOrigin });
            // Own the context before page creation or guard setup can fail.
            contexts.push(context);
            if (failureMode === "partial-setup" && contexts.length === 1) {
              throw partialSetupFailure;
            }
            const page = await context.newPage();
            pages.push(page);
            observeBrowserRequests(page);
            await installProviderFreeNetworkGuard(page);
            return page;
          },
        };
        await body(scope);
        passed = true;
      } catch (error) {
        bodyFailed = true;
        primaryFailure = error;
      } finally {
        try {
          if (fixture !== undefined) {
            const target = fixture;
            await cleanupFixture(
              target,
              pages,
              expectedReleases,
              passed,
              failureMode === "release-assertion"
                ? () => {
                    expect(
                      target.metrics().acceptedReleases,
                      "INJECTED_CASE5_RELEASE_ASSERTION",
                    ).toBe(expectedReleases + 1);
                  }
                : undefined,
            );
          }
        } catch (error) {
          recordCleanupFailure("APPLICATION_RELEASE_OR_ADAPTER_CLEANUP", error);
        } finally {
          try {
            if (fixture !== undefined) {
              await fixture.close();
              if (scope !== undefined) scope.fixtureClosed = true;
            }
          } catch (error) {
            recordCleanupFailure("ADAPTER_CLOSE", error);
          } finally {
            // Every acquired context gets its own close attempt even if an
            // application assertion, adapter close or previous context fails.
            for (const context of [...contexts].reverse()) {
              try {
                await context.close();
                scope?.closedContexts.add(context);
              } catch (error) {
                recordCleanupFailure("CONTEXT_CLOSE", error);
              }
            }
          }
        }
      }
      if (bodyFailed) {
        if (cleanupFailed) {
          testInfo.annotations.push({
            type: "case5-cleanup-failure",
            description: scope?.cleanupFailures.join(",") ?? "ADAPTER_ACQUISITION",
          });
        }
        throw primaryFailure;
      }
      if (cleanupFailed) throw cleanupFailure;
      if (fixture === undefined) throw new Error("Missing owned case5 fixture");
      return fixture;
    }

    await ownedPhase(2, async ({ fixture: firstFixture, newPage }) => {
      const firstPage = await newPage();
      const secondPage = await newPage();
      await openStockBeta(firstPage, request, firstFixture, clock);
      await openStockBeta(secondPage, request, firstFixture, clock);
      await waitForAcceptedLease(firstFixture, 2, 2);
      const leases = firstFixture.leaseLedger();
      expect(leases).toHaveLength(2);
      expect(leases[0]?.consumerId).not.toBe(leases[1]?.consumerId);
      expect(leases[0]?.leaseId).not.toBe(leases[1]?.leaseId);
      expect(leases[0]?.identities).toEqual(identities);
      expect(leases[1]?.identities).toEqual(identities);
      for (const lease of leases) {
        const connection = activeConnection(firstFixture, lease.leaseId);
        await emitFrameOnPages(
          [firstPage, secondPage],
          clock,
          firstFixture,
          connection.id,
          "snapshot",
          snapshotBody(firstFixture, identities, clock, {}, lease.leaseId),
        );
      }
      await expectLiveQuote(firstPage, "005930.KRX", quoteA);
      await expectLiveQuote(secondPage, "005930.KRX", quoteA);
      expect(firstFixture.metrics().snapshots).toBe(2);
      expect(firstFixture.metrics().getOpens).toBe(2);
      expect(firstFixture.metrics().activeSockets).toBe(2);
      const b = identities.find((identity) => identity.instrument_id === "000660.KRX");
      if (b === undefined) throw new Error("Expected admitted B for the second tab");
      await boardRow(secondPage, b.instrument_id).locator("button").click();
      await expectLiveQuote(secondPage, b.instrument_id, quoteB);
      await expectLiveQuote(firstPage, "005930.KRX", quoteA);
      expect(firstFixture.metrics().acceptedCreates).toBe(2);
      expect(firstFixture.metrics().acceptedRenewals).toBe(0);
      expect(firstFixture.metrics().acceptedReleases).toBe(0);
      expect(firstFixture.metrics().getOpens).toBe(2);
      expect(firstFixture.metrics().activeSockets).toBe(2);
      expect(firstFixture.metrics().activeIdentityCount).toBe(2);
      await setVisibility(firstPage, "hidden");
      await expect.poll(() => firstFixture.metrics().acceptedReleases, { timeout: 7_000 }).toBe(1);
      await expect.poll(() => firstFixture.metrics().getCloses, { timeout: 7_000 }).toBe(1);
      expect(firstFixture.metrics().activeLeases).toBe(1);
      expect(firstFixture.metrics().activeIdentityCount).toBe(2);
      await setVisibility(secondPage, "hidden");
      await expect.poll(() => firstFixture.metrics().acceptedReleases, { timeout: 7_000 }).toBe(2);
      await expect.poll(() => firstFixture.metrics().activeSockets, { timeout: 7_000 }).toBe(0);
      expect(firstFixture.metrics().getCloses).toBe(2);
      expect(firstFixture.metrics().snapshots).toBe(2);
      expect(firstFixture.metrics().activeIdentityCount).toBe(0);
      assertProviderFree(firstPage);
      assertProviderFree(secondPage);
    });

    const capFixture = await ownedPhase(
      2,
      async ({ fixture: capFixture, newPage, pages: capPages }) => {
        for (let index = 0; index < 3; index += 1) await newPage();
        const capFirst = capPages[0];
        const capSecond = capPages[1];
        const capThird = capPages[2];
        if (capFirst === undefined || capSecond === undefined || capThird === undefined) {
          throw new Error("The controlled capacity subphase requires three pages");
        }
        await openStockBeta(capFirst, request, capFixture, clock);
        await openStockBeta(capSecond, request, capFixture, clock);
        await waitForAcceptedLease(capFixture, 2, 2);
        for (const lease of capFixture.leaseLedger()) {
          const connection = activeConnection(capFixture, lease.leaseId);
          await emitFrameOnPages(
            [capFirst, capSecond],
            clock,
            capFixture,
            connection.id,
            "snapshot",
            snapshotBody(capFixture, identities, clock, {}, lease.leaseId),
          );
        }
        capFixture.failNextLease({ status: 409, code: "STREAM_LEASE_CAPACITY" });
        await openStockBeta(capThird, request, capFixture, clock, {
          waitForLease: false,
        });
        await expect.poll(() => capFixture.metrics().rejectedCreates, { timeout: 10_000 }).toBe(1);
        await expect(board(capThird).getByRole("status")).toHaveText("Updates unavailable");
        await expectNoCapturedQuote(capThird, "005930.KRX");
        for (const page of [capFirst, capSecond]) await setVisibility(page, "hidden");
        await expect.poll(() => capFixture.metrics().acceptedReleases, { timeout: 7_000 }).toBe(2);
        await expect.poll(() => capFixture.metrics().activeSockets, { timeout: 7_000 }).toBe(0);
        const beforeSilence = capFixture.metrics();
        await runFor(capThird, clock, 5_100);
        expect(capFixture.metrics().createAttempts).toBe(beforeSilence.createAttempts);
        expect(capFixture.metrics().getOpens).toBe(2);
        expect(capFixture.metrics().getCloses).toBe(2);
        expect(capFixture.metrics().activeLeases).toBe(0);
        expect(capFixture.metrics().rejectedCreates).toBe(1);
        for (const page of capPages) assertProviderFree(page);
      },
    );
    await attachSafeMetrics(testInfo, "stock-beta-ws-case-5", capFixture);

    for (const failureMode of ["partial-setup", "release-assertion"] as const) {
      await test.step(
        "owned cleanup after " + failureMode,
        async () => {
          const probe: { scope?: OwnedPhase } = {};
          const baselineContexts = new Set(browser.contexts());
          const work = ownedPhase(
            failureMode === "partial-setup" ? 0 : 2,
            async (scope) => {
              probe.scope = scope;
              const firstPage = await scope.newPage();
              const secondPage = await scope.newPage();
              await openStockBeta(firstPage, request, scope.fixture, clock);
              await openStockBeta(secondPage, request, scope.fixture, clock);
              await waitForAcceptedLease(scope.fixture, 2, 2);
              for (const lease of scope.fixture.leaseLedger()) {
                await emitFrameOnPages(
                  [firstPage, secondPage],
                  clock,
                  scope.fixture,
                  activeConnection(scope.fixture, lease.leaseId).id,
                  "snapshot",
                  snapshotBody(scope.fixture, identities, clock, {}, lease.leaseId),
                );
              }
              await expectLiveQuote(firstPage, "005930.KRX", quoteA);
              await expectLiveQuote(secondPage, "005930.KRX", quoteA);
              expect(scope.fixture.metrics().acceptedCreates).toBe(2);
              expect(scope.fixture.metrics().acceptedRenewals).toBe(0);
              expect(scope.fixture.metrics().snapshots).toBe(2);
              expect(scope.fixture.metrics().getOpens).toBe(2);
              for (const page of scope.pages) assertProviderFree(page);
            },
            failureMode,
          );
          if (failureMode === "partial-setup") {
            await expect(work).rejects.toBe(partialSetupFailure);
          } else {
            await expect(work).rejects.toThrow("INJECTED_CASE5_RELEASE_ASSERTION");
          }
          const scope = probe.scope;
          if (scope === undefined) throw new Error("Missing owned failure-probe scope");
          const expectedContexts = failureMode === "partial-setup" ? 1 : 2;
          expect(scope.contexts.length).toBe(expectedContexts);
          expect(scope.pages.length).toBe(failureMode === "partial-setup" ? 0 : 2);
          expect(scope.closedContexts.size).toBe(expectedContexts);
          expect(scope.fixtureClosed).toBe(true);
          expect(scope.pages.every((page) => page.isClosed())).toBe(true);
          expect(browser.contexts().every((context) => baselineContexts.has(context))).toBe(true);
          expect(scope.fixture.metrics().activeSockets).toBe(0);
          expect(scope.fixture.metrics().activeLeases).toBe(0);
          expect(scope.fixture.metrics().getOpens).toBe(scope.fixture.metrics().getCloses);
          expect(scope.cleanupFailures).toEqual(
            failureMode === "partial-setup" ? [] : ["APPLICATION_RELEASE_OR_ADAPTER_CLEANUP"],
          );
          expect(scope.fixture.metrics().acceptedReleases).toBe(
            failureMode === "partial-setup" ? 0 : 2,
          );
          await attachSafeMetrics(
            testInfo,
            "stock-beta-case5-expected-" + failureMode,
            scope.fixture,
          );
        },
        { timeout: 30_000 },
      );
    }
  });

  test("clears hidden/offline scopes, bounds reconnect and latches logout", async ({
    page,
    request,
  }, testInfo) => {
    test.setTimeout(150_000);
    await resetScenario(request, ownerScenario());
    const clock = createClock();
    const lifecycleFixture = await startStockBetaMarketStreamFixture(appOrigin, {
      readMemberships: () => readyIdentities(request),
      now: clock.now,
    });
    async function waitForReturnReadiness(): Promise<void> {
      await test.step(
        "observe Stock Beta readiness within the bounded public clock budget",
        async () => {
          let elapsed = 0;
          const observe = () =>
            page.evaluate(() => ({
              boardCount: document.querySelectorAll('[data-testid="stock-beta-stream-board"]')
                .length,
              loadingCount: [...document.querySelectorAll('[data-kind="loading"] > h2')].filter(
                (heading) => heading.textContent === "Loading workspace",
              ).length,
              date: Date.now(),
              performance: performance.now(),
            }));
          const attach = async (phase: string, state: Awaited<ReturnType<typeof observe>>) => {
            expect(state.date, "navigation browser/fixture clock binding").toBe(clock.now());
            const metrics = lifecycleFixture.metrics();
            await testInfo.attach("case-6-return-readiness-" + phase, {
              contentType: "application/json",
              body: Buffer.from(
                JSON.stringify({
                  phase,
                  elapsed,
                  totalVirtualMs: 32 + elapsed,
                  fixtureNow: clock.now(),
                  ...state,
                  creates: metrics.acceptedCreates,
                  releases: metrics.acceptedReleases,
                  opens: metrics.getOpens,
                }),
              ),
            });
          };
          let state = await observe();
          await attach("entry", state);
          const ready = () => state.boardCount === 1 && state.loadingCount === 0;
          try {
            // The existing two frames consumed 32ms. Allow at most the remaining
            // 268ms of React's 300ms fallback budget, stopping as soon as ready.
            for (let step = 0; step < 17 && !ready(); step += 1) {
              const milliseconds = step < 16 ? 16 : 12;
              await runFor(page, clock, milliseconds);
              elapsed += milliseconds;
              state = await observe();
            }
            await attach(ready() ? "ready" : "exhausted", state);
            expect(elapsed).toBeLessThanOrEqual(268);
            expect(state.boardCount, "returned board within the finite render budget").toBe(1);
            expect(state.loadingCount).toBe(0);
          } catch (error) {
            await attach("failure", await observe());
            throw error;
          }
        },
        { timeout: 5_000 },
      );
    }
    let lifecyclePassed = false;
    try {
      const identities = await openStockBeta(page, request, lifecycleFixture, clock);
      const identity = identities[0];
      if (identity === undefined) throw new Error("Expected A for lifecycle coverage");
      await sendInitialSnapshot(page, clock, lifecycleFixture, identities);
      await expectLiveQuote(page, identity.instrument_id, quoteA);
      await setVisibility(page, "hidden");
      await expectNoCapturedQuote(page, identity.instrument_id);
      await expect(board(page).getByRole("status")).toHaveText("Paused");
      await expect
        .poll(() => lifecycleFixture.metrics().acceptedReleases, { timeout: 7_000 })
        .toBe(1);
      const hidden = lifecycleFixture.metrics();
      await runFor(page, clock, 6_200);
      expect(lifecycleFixture.metrics().createAttempts).toBe(hidden.createAttempts);
      expect(lifecycleFixture.metrics().getOpens).toBe(hidden.getOpens);
      expect(lifecycleFixture.metrics().snapshots).toBe(hidden.snapshots);

      await setVisibility(page, "visible");
      await waitForAcceptedLease(lifecycleFixture, 2, 2);
      let lease = activeLease(lifecycleFixture);
      let connection = activeConnection(lifecycleFixture, lease.leaseId);
      await emitFrame(
        page,
        clock,
        lifecycleFixture,
        connection.id,
        "snapshot",
        snapshotBody(lifecycleFixture, identities, clock),
      );
      await expectLiveQuote(page, identity.instrument_id, quoteA);

      await page
        .getByRole("navigation", { name: "Primary" })
        .getByRole("link", { name: "Strategies", exact: true })
        .click();
      await expect(page).toHaveURL(/\/strategies$/);
      await expect(board(page)).toHaveCount(0);
      await expect
        .poll(() => lifecycleFixture.metrics().acceptedReleases, { timeout: 7_000 })
        .toBe(2);
      await attachClockPhase(page, clock, lifecycleFixture, "case-6-link-before-return");
      await test.step(
        "complete the existing local Stock Beta Link response",
        async () => {
          const returnResponse = page.waitForResponse(
            (response) => {
              const url = new URL(response.url());
              return (
                url.origin === new URL(appOrigin).origin &&
                url.pathname === "/stock-beta" &&
                response.request().method() === "GET" &&
                response.request().headers()["rsc"] === "1"
              );
            },
            { timeout: 5_000 },
          );
          const [response] = await Promise.all([
            returnResponse,
            page
              .getByRole("navigation", { name: "Primary" })
              .getByRole("link", { name: "Stock signal beta", exact: true })
              .click(),
          ]);
          expect(response.ok(), "existing local return RSC response succeeded").toBe(true);
          expect(
            await response.finished(),
            "existing local return RSC response completed",
          ).toBeNull();
        },
        { timeout: 5_000 },
      );
      await expect(page).toHaveURL(/\/stock-beta$/);
      await attachClockPhase(page, clock, lifecycleFixture, "case-6-link-response-complete");
      await pumpAnimationFrames(page, clock);
      await attachClockPhase(page, clock, lifecycleFixture, "case-6-link-after-two-frames");
      await waitForReturnReadiness();
      await expect(page.getByText("Loading workspace", { exact: true })).toHaveCount(0);
      await expect(board(page)).toHaveCount(1);
      await waitForAcceptedLease(lifecycleFixture, 3, 3);
      lease = activeLease(lifecycleFixture);
      connection = activeConnection(lifecycleFixture, lease.leaseId);
      await emitFrame(
        page,
        clock,
        lifecycleFixture,
        connection.id,
        "snapshot",
        snapshotBody(lifecycleFixture, identities, clock),
      );
      await setOnline(page, false);
      await expectNoCapturedQuote(page, identity.instrument_id);
      await expect(board(page).getByRole("status")).toHaveText("Paused");
      await expect
        .poll(() => lifecycleFixture.metrics().acceptedReleases, { timeout: 7_000 })
        .toBe(3);
      const offline = lifecycleFixture.metrics();
      await runFor(page, clock, 6_200);
      expect(lifecycleFixture.metrics().createAttempts).toBe(offline.createAttempts);
      expect(lifecycleFixture.metrics().getOpens).toBe(offline.getOpens);

      await setOnline(page, true);
      await waitForAcceptedLease(lifecycleFixture, 4, 4);
      lease = activeLease(lifecycleFixture);
      connection = activeConnection(lifecycleFixture, lease.leaseId);
      await emitFrame(
        page,
        clock,
        lifecycleFixture,
        connection.id,
        "snapshot",
        snapshotBody(lifecycleFixture, identities, clock),
      );
      await expectLiveQuote(page, identity.instrument_id, quoteA);
      await attachClockPhase(page, clock, lifecycleFixture, "case-6-lifecycle-before-disconnect");
      lifecycleFixture.disconnect(connection.id);
      await expect(selectedQuote(page)).toHaveAttribute("data-availability", "LAST_KNOWN");
      await expect(
        selectedQuote(page).locator('strong[data-quote-value="' + quoteA + '"]'),
      ).toHaveCount(1);
      await expect(board(page).getByRole("status")).toHaveText("Connecting");
      await advanceReconnectBoundary(
        page,
        clock,
        lifecycleFixture,
        1_000,
        5,
        "case-6-lifecycle-reconnect",
      );
      const reconnect = activeConnection(lifecycleFixture, lease.leaseId);
      expect(reconnect.streamId).not.toBe(connection.streamId);
      await emitFrame(page, clock, lifecycleFixture, reconnect.id, "reset", {
        reason_code: "RESYNC_REQUIRED",
      });
      await emitFrame(
        page,
        clock,
        lifecycleFixture,
        reconnect.id,
        "snapshot",
        snapshotBody(lifecycleFixture, identities, clock),
      );
      await expectLiveQuote(page, identity.instrument_id, quoteA);
      expect(lifecycleFixture.metrics().acceptedCreates).toBe(4);
      expect(lifecycleFixture.metrics().acceptedRenewals).toBe(0);
      expect(lifecycleFixture.metrics().snapshots).toBe(5);
      expect(lifecycleFixture.metrics().resets).toBe(1);
      assertNoFallback(page, lifecycleFixture);
      lifecyclePassed = true;
    } finally {
      await cleanupFixture(lifecycleFixture, [page], 4, lifecyclePassed);
    }

    const retryFixture = await startStockBetaMarketStreamFixture(appOrigin, {
      readMemberships: () => readyIdentities(request),
      now: clock.now,
    });
    let retryPassed = false;
    try {
      await page.goto("/strategies");
      await openStockBeta(page, request, retryFixture, clock, { installClock: false });
      const identities = await readyIdentities(request);
      const lease = activeLease(retryFixture);
      let connection = activeConnection(retryFixture, lease.leaseId);
      await emitFrame(
        page,
        clock,
        retryFixture,
        connection.id,
        "snapshot",
        snapshotBody(retryFixture, identities, clock),
      );
      await expectLiveQuote(page, "005930.KRX", quoteA);
      await attachClockPhase(page, clock, retryFixture, "case-6-retry-2-before-disconnect");
      retryFixture.disconnect(connection.id);
      // Observe the actual error handler before advancing the paused backoff clock.
      await expect(board(page).getByRole("status")).toHaveText("Connecting");
      await advanceReconnectBoundary(page, clock, retryFixture, 1_000, 2, "case-6-retry-2");
      connection = activeConnection(retryFixture, lease.leaseId);
      await emitFrame(page, clock, retryFixture, connection.id, "reset", {
        reason_code: "RESYNC_REQUIRED",
      });
      await emitFrame(
        page,
        clock,
        retryFixture,
        connection.id,
        "snapshot",
        snapshotBody(retryFixture, identities, clock),
      );
      await attachClockPhase(page, clock, retryFixture, "case-6-retry-3-before-disconnect");
      retryFixture.disconnect(connection.id);
      // Observe the actual error handler before advancing the paused backoff clock.
      await expect(board(page).getByRole("status")).toHaveText("Connecting");
      await advanceReconnectBoundary(page, clock, retryFixture, 2_000, 3, "case-6-retry-3");
      connection = activeConnection(retryFixture, lease.leaseId);
      await emitFrame(page, clock, retryFixture, connection.id, "reset", {
        reason_code: "RESYNC_REQUIRED",
      });
      await emitFrame(
        page,
        clock,
        retryFixture,
        connection.id,
        "snapshot",
        snapshotBody(retryFixture, identities, clock),
      );
      await attachClockPhase(page, clock, retryFixture, "case-6-retry-4-before-disconnect");
      retryFixture.disconnect(connection.id);
      // Observe the actual error handler before advancing the paused backoff clock.
      await expect(board(page).getByRole("status")).toHaveText("Connecting");
      await advanceReconnectBoundary(page, clock, retryFixture, 4_000, 4, "case-6-retry-4");
      connection = activeConnection(retryFixture, lease.leaseId);
      await emitFrame(page, clock, retryFixture, connection.id, "reset", {
        reason_code: "RESYNC_REQUIRED",
      });
      await emitFrame(
        page,
        clock,
        retryFixture,
        connection.id,
        "snapshot",
        snapshotBody(retryFixture, identities, clock),
      );
      retryFixture.disconnect(connection.id);
      await runFor(page, clock, 16);
      await expect.poll(() => retryFixture.metrics().acceptedReleases, { timeout: 7_000 }).toBe(1);
      await expect(board(page).getByRole("status")).toHaveText("Updates unavailable");
      await expectNoCapturedQuote(page, "005930.KRX");
      const terminal = retryFixture.metrics();
      expect(terminal.getOpens).toBe(4);
      await runFor(page, clock, 60_000);
      expect(retryFixture.metrics().getOpens).toBe(4);
      expect(retryFixture.metrics().createAttempts).toBe(1);
      expect(retryFixture.metrics().renewalAttempts).toBe(0);
      assertNoFallback(page, retryFixture);
      retryPassed = true;
    } finally {
      await cleanupFixture(retryFixture, [page], 1, retryPassed);
    }

    const logoutFixture = await startStockBetaMarketStreamFixture(appOrigin, {
      readMemberships: () => readyIdentities(request),
      now: clock.now,
    });
    let completeLogout: () => void = () => {};
    let finishLogoutRoute: (() => void) | null = null;
    let logoutHandlerStarted = false;
    const logoutRouteFinished = new Promise<void>((resolve) => {
      finishLogoutRoute = resolve;
    });
    let logoutRequestSeen: (() => void) | null = null;
    const logoutSeen = new Promise<void>((resolve) => {
      logoutRequestSeen = resolve;
    });
    let logoutPassed = false;
    const heldLogout = new Promise<void>((resolve) => {
      completeLogout = resolve;
    });
    const logoutHandler = async (route: import("@playwright/test").Route) => {
      const request = route.request();
      const url = new URL(request.url());
      if (
        url.origin === appOrigin &&
        url.pathname === "/api/v1/auth/logout" &&
        request.method() === "POST"
      ) {
        logoutHandlerStarted = true;
        logoutRequestSeen?.();
        try {
          await heldLogout;
          await route.fulfill({
            status: 503,
            contentType: "application/json",
            body: JSON.stringify({ error: { code: "SYNTHETIC_LOGOUT_UNAVAILABLE" } }),
          });
        } finally {
          finishLogoutRoute?.();
        }
        return;
      }
      await route.fallback();
    };
    try {
      await page.goto("/strategies");
      await page.route("**/api/v1/auth/logout", logoutHandler);
      await openStockBeta(page, request, logoutFixture, clock, { installClock: false });
      const identities = await readyIdentities(request);
      const lease = activeLease(logoutFixture);
      const connection = activeConnection(logoutFixture, lease.leaseId);
      await emitFrame(
        page,
        clock,
        logoutFixture,
        connection.id,
        "snapshot",
        snapshotBody(logoutFixture, identities, clock),
      );
      await expectLiveQuote(page, "005930.KRX", quoteA);
      await page.getByRole("button", { name: "Sign out" }).click();
      await logoutSeen;
      await expectNoCapturedQuote(page, "005930.KRX");
      await expect.poll(() => logoutFixture.metrics().acceptedReleases, { timeout: 7_000 }).toBe(1);
      await expect.poll(() => logoutFixture.metrics().activeSockets, { timeout: 7_000 }).toBe(0);
      const beforeReturn = logoutFixture.metrics();
      completeLogout();
      await expect(page.getByRole("button", { name: "Sign out" })).toBeVisible();
      await setVisibility(page, "hidden");
      await setVisibility(page, "visible");
      await runFor(page, clock, 2_000);
      expect(logoutFixture.metrics().createAttempts).toBe(beforeReturn.createAttempts);
      expect(logoutFixture.metrics().getOpens).toBe(beforeReturn.getOpens);
      await expectNoCapturedQuote(page, "005930.KRX");
      assertNoFallback(page, logoutFixture);
      logoutPassed = true;
    } finally {
      completeLogout();
      if (logoutHandlerStarted) await logoutRouteFinished.catch(() => undefined);
      await cleanupFixture(logoutFixture, [page], 1, logoutPassed);
      await page.unroute("**/api/v1/auth/logout", logoutHandler).catch(() => undefined);
    }
    await attachSafeMetrics(testInfo, "stock-beta-ws-case-6", logoutFixture);
  });

  test("advances one instrument's price, signed change, version, and receipt time", async ({
    page,
    request,
  }, testInfo) => {
    test.setTimeout(45_000);
    await resetScenario(request, ownerScenario());
    const clock = createClock();
    const fixture = await startStockBetaMarketStreamFixture(appOrigin, {
      readMemberships: () => readyIdentities(request),
      now: clock.now,
    });
    let passed = false;
    try {
      const identities = await openStockBeta(page, request, fixture, clock);
      const identity = identities[0];
      if (identity === undefined) throw new Error("Expected READY A for advancing quote");
      const connection = activeConnection(fixture);
      await sendInitialSnapshot(page, clock, fixture, identities);
      const firstReceipt = await boardRow(page, identity.instrument_id).getAttribute(
        "data-received-at",
      );
      await expectLiveQuote(page, identity.instrument_id, quoteA);
      if (firstReceipt === null) throw new Error("The initial quote omitted receipt R1");
      const receivedAtMs = clock.now() - 100;
      const expectedReceipt = new Date(receivedAtMs).toISOString();
      expect(Date.parse(expectedReceipt), "emitted receipt R2 strictly follows R1").toBeGreaterThan(
        Date.parse(firstReceipt),
      );
      const delta = makeRow(identity, clock, {
        price: "101300.00",
        change: "1300.00",
        percent: "1.30",
        quoteVersion: "2",
        receiveOrdinal: "2",
        stateVersion: "2",
        receivedAtMs,
        quoteOverrides: { trade_volume: "101", cumulative_volume: "1001" },
      });
      await emitFrame(page, clock, fixture, connection.id, "delta", { rows: [delta] });
      await pumpAnimationFrames(page, clock);
      const row = boardRow(page, identity.instrument_id);
      await expect(row).toHaveAttribute("data-quote-version", "2");
      await expect(row).toHaveAttribute("data-receive-ordinal", "2");
      await expect(row).toHaveAttribute("data-received-at", /2026-10-12T/);
      await expect(row).toHaveAttribute("data-received-at", expectedReceipt);
      await expect(row.locator("time[datetime]").nth(1)).toHaveAttribute(
        "datetime",
        expectedReceipt,
      );
      await expect(selectedQuote(page).locator("time[datetime]").nth(1)).toHaveAttribute(
        "datetime",
        expectedReceipt,
      );
      await expect(selectedQuote(page)).toContainText("+1,300.00 (+1.30%)");
      const advancedEpoch = await row.getAttribute("data-epoch");
      const advancedReceipt = await row.getAttribute("data-received-at");
      expect(advancedEpoch).toBe(identityUuid(identity, "9000"));
      expect(advancedReceipt).not.toBe(firstReceipt);
      expect(advancedReceipt).toBe(expectedReceipt);

      const currentConnection = fixture
        .connectionLedger()
        .find((item) => item.id === connection.id);
      if (currentConnection === undefined) throw new Error("Missing advancing source");
      const duplicate = makeRow(identity, clock, {
        price: "101999.00",
        change: "1999.00",
        percent: "1.99",
        quoteVersion: "9",
        receiveOrdinal: "9",
        stateVersion: "9",
      });
      const duplicateData = connectionEventData(currentConnection, clock, "2", {
        rows: [duplicate],
      });
      fixture.emitInvalid(
        connection.id,
        { kind: "delta", data: duplicateData },
        connection.streamId + ":2",
      );
      await pumpAnimationFrames(page, clock);
      await expect(row).toHaveAttribute("data-quote-version", "2");
      await expect(row).toHaveAttribute("data-receive-ordinal", "2");
      await expect(row).toHaveAttribute("data-epoch", advancedEpoch ?? "");
      await expect(row).toHaveAttribute("data-received-at", advancedReceipt ?? "");
      await expectLiveQuote(page, identity.instrument_id, "101300.00");

      const rollback = makeRow(identity, clock, {
        price: "101400.00",
        change: "1400.00",
        percent: "1.40",
        quoteVersion: "1",
        receiveOrdinal: "3",
        stateVersion: "3",
      });
      const rollbackSequence = nextSequence(currentConnection);
      await attachClockPhase(page, clock, fixture, "case-7-rollback-before-delivery");
      fixture.emitInvalid(
        connection.id,
        {
          kind: "delta",
          data: connectionEventData(currentConnection, clock, rollbackSequence, {
            rows: [rollback],
          }),
        },
        connection.streamId + ":" + rollbackSequence,
      );
      await expectNoCapturedQuote(page, identity.instrument_id);
      await advanceReconnectBoundary(page, clock, fixture, 1_000, 2, "case-7-rollback");
      const recovery = activeConnection(fixture);
      expect(recovery.streamId).not.toBe(connection.streamId);
      await emitFrame(page, clock, fixture, recovery.id, "reset", {
        reason_code: "RESYNC_REQUIRED",
      });
      await emitFrame(
        page,
        clock,
        fixture,
        recovery.id,
        "snapshot",
        snapshotBody(fixture, identities, clock, {
          price: "101300.00",
          change: "1300.00",
          percent: "1.30",
          quoteVersion: "2",
          receiveOrdinal: "2",
          stateVersion: "2",
        }),
      );
      await expectLiveQuote(page, identity.instrument_id, "101300.00");
      await expect(boardRow(page, identity.instrument_id)).toHaveAttribute(
        "data-quote-version",
        "2",
      );
      await expect(boardRow(page, identity.instrument_id)).toHaveAttribute(
        "data-receive-ordinal",
        "2",
      );
      const metrics = fixture.metrics();
      expect(metrics.snapshots).toBe(2);
      expect(metrics.deltas).toBe(3);
      expect(metrics.resets).toBe(1);
      expect(metrics.invalidDeltas).toBe(2);
      expect(metrics.acceptedCreates).toBe(1);
      expect(metrics.acceptedRenewals).toBe(0);
      assertNoFallback(page, fixture);
      await page.screenshot({
        animations: "disabled",
        path: testInfo.outputPath("stock-beta-ws-advancing.png"),
      });
      passed = true;
    } finally {
      await cleanupFixture(fixture, [page], 1, passed);
    }
  });

  test("retains typed invalid-trade status and purges malformed delivery", async ({
    page,
    request,
  }, testInfo) => {
    test.setTimeout(60_000);
    await resetScenario(request, ownerScenario());
    const clock = createClock();
    const fixture = await startStockBetaMarketStreamFixture(appOrigin, {
      readMemberships: () => readyIdentities(request),
      now: clock.now,
    });
    let passed = false;
    try {
      const identities = await openStockBeta(page, request, fixture, clock);
      const identity = identities[0];
      if (identity === undefined)
        throw new Error("Expected READY A for status and invalid-wire case");
      let connection = activeConnection(fixture);
      await sendInitialSnapshot(page, clock, fixture, identities);
      const originalRow = boardRow(page, identity.instrument_id);
      const originalReceipt = await originalRow.getAttribute("data-received-at");
      const originalVersion = await originalRow.getAttribute("data-quote-version");
      const originalEpoch = await originalRow.getAttribute("data-epoch");
      const originalOrdinal = await originalRow.getAttribute("data-receive-ordinal");
      await emitFrame(page, clock, fixture, connection.id, "status", {
        connection: "CONNECTED",
        reason_code: "QUOTE_VALUE_INVALID",
        gap_open: false,
        session_has_gap: false,
        gap_generation: "0",
      });
      await expect(boardRow(page, identity.instrument_id)).toHaveAttribute(
        "data-availability",
        "LAST_KNOWN",
      );
      await expect(originalRow).toHaveAttribute("data-received-at", originalReceipt ?? "");
      await expect(originalRow).toHaveAttribute("data-quote-version", originalVersion ?? "");
      await expect(originalRow).toHaveAttribute("data-epoch", originalEpoch ?? "");
      await expect(originalRow).toHaveAttribute("data-receive-ordinal", originalOrdinal ?? "");
      await expect(
        selectedQuote(page).locator('strong[data-quote-value="' + quoteA + '"]'),
      ).toHaveCount(1);
      await emitFrame(page, clock, fixture, connection.id, "status", {
        connection: "STOPPED",
        reason_code: "PROVIDER_RESPONSE_INVALID",
        gap_open: true,
        session_has_gap: true,
        gap_generation: "1",
      });
      await expect(boardRow(page, identity.instrument_id)).toContainText("Gap in this session");
      await expect(boardRow(page, identity.instrument_id)).toHaveAttribute(
        "data-received-at",
        originalReceipt ?? "",
      );
      await expect(boardRow(page, identity.instrument_id)).toHaveAttribute(
        "data-quote-version",
        "1",
      );
      await expect(boardRow(page, identity.instrument_id)).toHaveAttribute(
        "data-availability",
        "LAST_KNOWN",
      );
      await expect(originalRow).toHaveAttribute("data-epoch", originalEpoch ?? "");
      await expect(originalRow).toHaveAttribute("data-receive-ordinal", originalOrdinal ?? "");
      await expect(boardRow(page, identity.instrument_id)).not.toContainText("101,300.00");

      const current = fixture.connectionLedger().find((item) => item.id === connection.id);
      if (current === undefined) throw new Error("Missing original status connection");
      const zero = makeRow(identity, clock, {
        quoteOverrides: { price: "0.00" },
        stateVersion: "2",
        quoteVersion: "2",
        receiveOrdinal: "2",
      });
      const zeroSequence = nextSequence(current);
      await attachClockPhase(page, clock, fixture, "case-8-zero-before-delivery");
      fixture.emitInvalid(
        connection.id,
        {
          kind: "delta",
          data: connectionEventData(current, clock, zeroSequence, { rows: [zero] }),
        },
        connection.streamId + ":" + zeroSequence,
      );
      await expectNoCapturedQuote(page, identity.instrument_id);
      await advanceReconnectBoundary(page, clock, fixture, 1_000, 2, "case-8-zero");
      connection = activeConnection(fixture);
      await emitFrame(page, clock, fixture, connection.id, "reset", {
        reason_code: "RESYNC_REQUIRED",
      });
      await emitFrame(
        page,
        clock,
        fixture,
        connection.id,
        "snapshot",
        snapshotBody(fixture, identities, clock, {
          gapOpen: false,
          sessionHasGap: true,
          gapGeneration: "1",
        }),
      );
      await expectLiveQuote(page, identity.instrument_id, quoteA);
      await expect(boardRow(page, identity.instrument_id)).toContainText("Gap in this session");

      const recovered = fixture.connectionLedger().find((item) => item.id === connection.id);
      if (recovered === undefined) throw new Error("Missing first recovery connection");
      const nanRow = makeRow(identity, clock, {
        quoteOverrides: { price: "NaN" },
        stateVersion: "2",
        quoteVersion: "2",
        receiveOrdinal: "2",
      });
      const nanSequence = nextSequence(recovered);
      await attachClockPhase(page, clock, fixture, "case-8-nan-before-delivery");
      fixture.emitInvalid(
        connection.id,
        {
          kind: "delta",
          data: connectionEventData(recovered, clock, nanSequence, { rows: [nanRow] }),
        },
        connection.streamId + ":" + nanSequence,
      );
      await expectNoCapturedQuote(page, identity.instrument_id);
      await advanceReconnectBoundary(page, clock, fixture, 2_000, 3, "case-8-nan");
      connection = activeConnection(fixture);
      await emitFrame(page, clock, fixture, connection.id, "reset", {
        reason_code: "RESYNC_REQUIRED",
      });
      await emitFrame(
        page,
        clock,
        fixture,
        connection.id,
        "snapshot",
        snapshotBody(fixture, identities, clock, {
          gapOpen: false,
          sessionHasGap: true,
          gapGeneration: "1",
        }),
      );
      await expectLiveQuote(page, identity.instrument_id, quoteA);
      const metrics = fixture.metrics();
      expect(metrics.snapshots).toBe(3);
      expect(metrics.statuses).toBe(2);
      expect(metrics.deltas).toBe(2);
      expect(metrics.invalidDeltas).toBe(2);
      expect(metrics.resets).toBe(2);
      expect(metrics.acceptedCreates).toBe(1);
      expect(metrics.acceptedRenewals).toBe(0);
      assertNoFallback(page, fixture);
      await attachSafeMetrics(testInfo, "stock-beta-ws-case-8", fixture);
      passed = true;
    } finally {
      await cleanupFixture(fixture, [page], 1, passed);
    }
  });

  test("renders closed, halted, stale, unknown-calendar, and typed failure states", async ({
    page,
    request,
  }, testInfo) => {
    test.setTimeout(180_000);
    await resetScenario(request, ownerScenario());
    const clock = createClock();
    const allMetrics: unknown[] = [];
    async function runPositiveStateStep(
      name: string,
      options: RowOptions,
      label: string,
      unavailable: boolean,
      retainedClosed = false,
    ): Promise<void> {
      await test.step(name, async () => {
        await page.goto("about:blank");
        await resetScenario(request, ownerScenario());
        const fixture = await startStockBetaMarketStreamFixture(appOrigin, {
          readMemberships: () => readyIdentities(request),
          now: clock.now,
        });
        let passed = false;
        try {
          const identities = await openStockBeta(page, request, fixture, clock, {
            installClock: allMetrics.length === 0,
          });
          const identity = identities[0];
          if (identity === undefined) throw new Error("Expected A for every positive state step");
          const connection = activeConnection(fixture);
          const initialRow = makeRow(identity, clock, retainedClosed ? {} : options);
          const lease = activeLease(fixture);
          await emitFrame(page, clock, fixture, connection.id, "snapshot", {
            lease_id: lease.leaseId,
            lease_expires_at: lease.expiresAt,
            rows: [initialRow],
          });
          const row = boardRow(page, identity.instrument_id);
          if (retainedClosed) {
            await expectLiveQuote(page, identity.instrument_id, quoteA);
          } else {
            await expect(row).toContainText(label);
            await expect(selectedQuote(page)).toContainText(label);
            if (unavailable) {
              await expectNoCapturedQuote(page, identity.instrument_id);
            } else {
              const expectedPrice = options.price ?? quoteA;
              await expect(
                selectedQuote(page).locator('[data-quote-value="' + expectedPrice + '"]'),
              ).toHaveCount(1);
              await expect(row).toHaveAttribute(
                "data-availability",
                options.availability ?? "LIVE",
              );
            }
          }
          if (retainedClosed) {
            const originalProof = {
              epoch: await row.getAttribute("data-epoch"),
              ordinal: await row.getAttribute("data-receive-ordinal"),
              receivedAt: await row.getAttribute("data-received-at"),
              quoteVersion: await row.getAttribute("data-quote-version"),
            };
            const retainedRow = {
              ...initialRow,
              market_state: "CLOSED",
              freshness: "STALE",
              availability: "LAST_KNOWN",
              reason_code: "SESSION_CLOSED",
              state_version: "2",
            };
            await emitFrame(page, clock, fixture, connection.id, "delta", { rows: [retainedRow] });
            await expect(row).toContainText(label);
            await expect(selectedQuote(page)).toContainText(label);
            await expect(row).toHaveAttribute("data-availability", "LAST_KNOWN");
            await expect(row).not.toHaveAttribute("data-availability", "LIVE");
            await expect(
              selectedQuote(page).locator('strong[data-quote-value="' + quoteA + '"]'),
            ).toHaveCount(1);
            await expect(row).toHaveAttribute("data-epoch", originalProof.epoch ?? "");
            await expect(row).toHaveAttribute("data-receive-ordinal", originalProof.ordinal ?? "");
            await expect(row).toHaveAttribute("data-received-at", originalProof.receivedAt ?? "");
            await expect(row).toHaveAttribute(
              "data-quote-version",
              originalProof.quoteVersion ?? "",
            );
            const unavailableClosed = makeRow(identity, clock, {
              ...options,
              stateVersion: "3",
            });
            await emitFrame(page, clock, fixture, connection.id, "delta", {
              rows: [unavailableClosed],
            });
            await expect(row).toContainText(label);
            await expect(row).toHaveAttribute("data-availability", "UNAVAILABLE");
            await expect(selectedQuote(page)).toHaveAttribute("data-availability", "UNAVAILABLE");
            await expectNoCapturedQuote(page, identity.instrument_id);
          }
          const metrics = fixture.metrics();
          expect(metrics.acceptedCreates).toBe(1);
          expect(metrics.getOpens).toBe(1);
          expect(metrics.snapshots).toBe(1);
          expect(metrics.acceptedRenewals).toBe(0);
          if (retainedClosed) expect(metrics.deltas).toBe(2);
          assertNoFallback(page, fixture);
          allMetrics.push({
            subcase: name,
            input: retainedClosed
              ? [
                  { market_state: "OPEN", quote_present: true },
                  {
                    market_state: "CLOSED",
                    availability: "LAST_KNOWN",
                    reason_code: "SESSION_CLOSED",
                  },
                  { market_state: "CLOSED", quote_present: false, reason_code: "SESSION_CLOSED" },
                ]
              : {
                  market_state: options.marketState ?? "OPEN",
                  reason_code: options.reasonCode ?? null,
                  quote_present: options.quotePresent !== false,
                },
            metrics,
          });
          passed = true;
        } finally {
          await cleanupFixture(fixture, [page], 1, passed);
        }
      });
    }

    const now = clock.now();
    await runPositiveStateStep(
      "closed authority and retained closed observation",
      {
        quotePresent: false,
        marketState: "CLOSED",
        freshness: "UNAVAILABLE",
        availability: "UNAVAILABLE",
        reasonCode: "SESSION_CLOSED",
      },
      "Market closed",
      true,
      true,
    );
    await runPositiveStateStep(
      "halted instrument remains a valid open quote",
      { halted: true },
      "Halt observed",
      false,
    );
    await runPositiveStateStep(
      "stale last-known quote preserves its receipt",
      {
        tradeAtMs: now - 32_100,
        receivedAtMs: now - 31_100,
        committedAtMs: now - 31_050,
        freshness: "STALE",
        availability: "LAST_KNOWN",
        reasonCode: "QUOTE_STALE",
      },
      "Stale",
      false,
    );
    await runPositiveStateStep(
      "unknown calendar purges prices",
      {
        quotePresent: false,
        sessionDate: null,
        marketState: "UNKNOWN",
        freshness: "UNAVAILABLE",
        availability: "UNAVAILABLE",
        reasonCode: "CALENDAR_UNAVAILABLE",
      },
      "Unavailable",
      true,
    );
    await runPositiveStateStep(
      "unknown session window purges prices",
      {
        quotePresent: false,
        marketState: "UNKNOWN",
        freshness: "UNAVAILABLE",
        availability: "UNAVAILABLE",
        reasonCode: "SESSION_WINDOW_UNAVAILABLE",
      },
      "Unavailable",
      true,
    );
    await runPositiveStateStep(
      "budget exhaustion is unavailable without fallback",
      {
        quotePresent: false,
        connection: "STOPPED",
        marketState: "OPEN",
        freshness: "UNAVAILABLE",
        availability: "UNAVAILABLE",
        reasonCode: "BUDGET_EXHAUSTED",
      },
      "Unavailable",
      true,
    );
    await runPositiveStateStep(
      "producer unavailability is unavailable without fallback",
      {
        quotePresent: false,
        connection: "STOPPED",
        marketState: "OPEN",
        freshness: "UNAVAILABLE",
        availability: "UNAVAILABLE",
        reasonCode: "PRODUCER_UNAVAILABLE",
      },
      "Unavailable",
      true,
    );

    async function runInvalidSnapshotStep(
      name: string,
      makeInvalid: (
        identity: IntradayStreamIdentity,
        clock: SyntheticClock,
      ) => Record<string, unknown>,
      privatelyAssert: (row: Record<string, unknown>, eventTime: number) => void,
    ): Promise<void> {
      await test.step(name, async () => {
        await page.goto("about:blank");
        await resetScenario(request, ownerScenario());
        const fixture = await startStockBetaMarketStreamFixture(appOrigin, {
          readMemberships: () => readyIdentities(request),
          now: clock.now,
        });
        let passed = false;
        try {
          const identities = await openStockBeta(page, request, fixture, clock, {
            installClock: false,
          });
          const identity = identities[0];
          if (identity === undefined) throw new Error("Expected A for targeted invalid frame");
          const connection = activeConnection(fixture);
          const row = makeInvalid(identity, clock);
          const privateFrame = makeEnvelope(connection, clock, "1", {
            lease_id: activeLease(fixture).leaseId,
            lease_expires_at: activeLease(fixture).expiresAt,
            rows: [row],
          });
          privatelyAssert(row, clock.now());
          fixture.emitInvalid(
            connection.id,
            { kind: "snapshot", data: JSON.stringify(privateFrame) },
            connection.streamId + ":1",
          );
          await expectNoCapturedQuote(page, identity.instrument_id);
          await expect.poll(() => fixture.metrics().activeSockets, { timeout: 7_000 }).toBe(0);
          expect(fixture.metrics().snapshots).toBe(1);
          expect(fixture.metrics().invalidSnapshots).toBe(1);
          expect(fixture.metrics().acceptedCreates).toBe(1);
          assertNoFallback(page, fixture);
          await setVisibility(page, "hidden");
          const metrics = fixture.metrics();
          allMetrics.push({ subcase: name, input_kind: "invalid_snapshot", metrics });
          passed = true;
        } finally {
          await cleanupFixture(fixture, [page], 1, passed);
        }
      });
    }

    await runInvalidSnapshotStep(
      "zero price is rejected before any row paints",
      (identity, currentClock) => {
        const row = makeRow(identity, currentClock);
        (row["quote"] as Record<string, unknown>)["price"] = "0.00";
        return row;
      },
      (row) => expect((row["quote"] as Record<string, unknown>)["price"]).toBe("0.00"),
    );
    await runInvalidSnapshotStep(
      "malformed decimal is rejected before any row paints",
      (identity, currentClock) => {
        const row = makeRow(identity, currentClock);
        (row["quote"] as Record<string, unknown>)["price"] = "101,200";
        return row;
      },
      (row) => expect((row["quote"] as Record<string, unknown>)["price"]).toBe("101,200"),
    );
    await runInvalidSnapshotStep(
      "future receipt beyond authenticated server time is rejected",
      (identity, currentClock) =>
        makeRow(identity, currentClock, {
          receivedAtMs: currentClock.now() + 100,
          committedAtMs: currentClock.now() + 150,
        }),
      (row, eventTime) => {
        const quote = row["quote"] as Record<string, unknown>;
        expect(Date.parse(String(quote["received_at"]))).toBeGreaterThan(eventTime);
        expect(Date.parse(String(quote["committed_at"]))).toBeGreaterThan(eventTime);
      },
    );
    await runInvalidSnapshotStep(
      "prior KST session after rollover is rejected",
      (identity, currentClock) => {
        const prior = currentClock.now() - 24 * 60 * 60 * 1_000;
        const received = prior - 100;
        return makeRow(identity, currentClock, {
          tradeAtMs: Math.floor((received - 1_000) / 1_000) * 1_000,
          receivedAtMs: received,
          committedAtMs: received + 50,
          businessDate: kstDate(prior),
          sessionDate: kstDate(prior),
          freshness: "STALE",
          availability: "LAST_KNOWN",
          reasonCode: "QUOTE_STALE",
        });
      },
      (row, eventTime) => {
        const quote = row["quote"] as Record<string, unknown>;
        expect(
          String(row["session"] && (row["session"] as Record<string, unknown>)["date"]),
        ).not.toBe(kstDate(eventTime));
        expect(quote["business_date"]).not.toBe(kstDate(eventTime));
      },
    );

    for (const failure of [
      { name: "503 unavailable lease", status: 503 as const, code: "MARKET_STREAM_UNAVAILABLE" },
      {
        name: "typed capacity lease conflict",
        status: 409 as const,
        code: "STREAM_LEASE_CAPACITY",
      },
    ]) {
      await test.step(failure.name, async () => {
        await page.goto("about:blank");
        await resetScenario(request, ownerScenario());
        const fixture = await startStockBetaMarketStreamFixture(appOrigin, {
          readMemberships: () => readyIdentities(request),
          now: clock.now,
        });
        fixture.failNextLease({ status: failure.status, code: failure.code });
        let passed = false;
        try {
          await openStockBeta(page, request, fixture, clock, {
            installClock: false,
            waitForLease: false,
          });
          await expect.poll(() => fixture.metrics().rejectedCreates, { timeout: 10_000 }).toBe(1);
          await expect(board(page).getByRole("status")).toHaveText("Updates unavailable");
          await expectNoCapturedQuote(page, "005930.KRX");
          await runFor(page, clock, 5_100);
          expect(fixture.metrics().getOpens).toBe(0);
          expect(fixture.metrics().createAttempts).toBe(1);
          expect(fixture.metrics().acceptedCreates).toBe(0);
          expect(fixture.metrics().acceptedReleases).toBe(0);
          expect(fixture.metrics().rejectedCreates).toBe(1);
          assertNoFallback(page, fixture);
          allMetrics.push({
            subcase: failure.name,
            status: failure.status,
            code: failure.code,
            metrics: fixture.metrics(),
          });
          passed = true;
        } finally {
          await cleanupFixture(fixture, [page], 0, passed);
        }
      });
    }
    expect(allMetrics).toHaveLength(13);
    await testInfo.attach("stock-beta-ws-case-9-subcase-counts", {
      contentType: "application/json",
      body: JSON.stringify(allMetrics.map((entry, index) => ({ subcase: index + 1, entry }))),
    });
  });

  test("retains nonlive failure observations and restores valid stream delivery", async ({
    page,
    request,
  }, testInfo) => {
    test.setTimeout(75_000);
    await resetScenario(request, ownerScenario());
    const clock = createClock();
    let clockInstalled = false;
    for (const phase of [
      {
        name: "producer failure keeps the last observation and gap history",
        reason: "PROVIDER_RESPONSE_INVALID",
        connection: "STOPPED" as const,
        gapOpen: true,
        sessionHasGap: true,
        gapGeneration: "1",
      },
      {
        name: "no active demand status stays nonlive until a fresh delta",
        reason: "NO_ACTIVE_DEMAND",
        connection: "CONNECTED" as const,
        gapOpen: false,
        sessionHasGap: false,
        gapGeneration: "0",
      },
    ]) {
      await test.step(phase.name, async () => {
        await page.goto("about:blank");
        await resetScenario(request, ownerScenario());
        const fixture = await startStockBetaMarketStreamFixture(appOrigin, {
          readMemberships: () => readyIdentities(request),
          now: clock.now,
        });
        let passed = false;
        try {
          const identities = await openStockBeta(page, request, fixture, clock, {
            installClock: !clockInstalled,
          });
          clockInstalled = true;
          const identity = identities[0];
          if (identity === undefined) throw new Error("Expected A for typed status recovery");
          const connection = activeConnection(fixture);
          await emitFrame(
            page,
            clock,
            fixture,
            connection.id,
            "snapshot",
            snapshotBody(fixture, identities, clock),
          );
          await expectLiveQuote(page, identity.instrument_id, quoteA);
          const originalReceipt = await boardRow(page, identity.instrument_id).getAttribute(
            "data-received-at",
          );
          if (originalReceipt === null) throw new Error("The good quote omitted its receipt");
          const originalProof: [string, string][] = [["data-received-at", originalReceipt]];
          for (const attribute of ["data-epoch", "data-quote-version", "data-receive-ordinal"]) {
            const value = await boardRow(page, identity.instrument_id).getAttribute(attribute);
            if (value === null) throw new Error("The good quote omitted its proof attribute");
            originalProof.push([attribute, value]);
          }
          const expectRetainedObservation = async () => {
            await expect(
              boardRow(page, identity.instrument_id).locator(
                'td[data-quote-value="' + quoteA + '"]',
              ),
            ).toHaveCount(1);
            await expect(selectedQuote(page)).toHaveAttribute("data-availability", "LAST_KNOWN");
            await expect(
              selectedQuote(page).locator('strong[data-quote-value="' + quoteA + '"]'),
            ).toHaveCount(1);
            for (const [attribute, value] of originalProof) {
              await expect(boardRow(page, identity.instrument_id)).toHaveAttribute(
                attribute,
                value,
              );
            }
            await expect(selectedQuote(page).locator("time[datetime]").nth(1)).toHaveAttribute(
              "datetime",
              originalReceipt,
            );
          };
          await emitFrame(page, clock, fixture, connection.id, "status", {
            connection: phase.connection,
            reason_code: phase.reason,
            gap_open: phase.gapOpen,
            session_has_gap: phase.sessionHasGap,
            gap_generation: phase.gapGeneration,
          });
          await expect(boardRow(page, identity.instrument_id)).toHaveAttribute(
            "data-availability",
            "LAST_KNOWN",
          );
          await expect(boardRow(page, identity.instrument_id)).toHaveAttribute(
            "data-received-at",
            originalReceipt ?? "",
          );
          await expect(boardRow(page, identity.instrument_id)).toHaveAttribute(
            "data-quote-version",
            "1",
          );
          await expect(boardRow(page, identity.instrument_id)).not.toHaveAttribute(
            "data-availability",
            "LIVE",
          );
          await expectRetainedObservation();
          await emitFrame(page, clock, fixture, connection.id, "status", {
            connection: "CONNECTED",
            reason_code: null,
            gap_open: false,
            session_has_gap: phase.sessionHasGap,
            gap_generation: phase.gapGeneration,
          });
          await expect(boardRow(page, identity.instrument_id)).toHaveAttribute(
            "data-availability",
            "LAST_KNOWN",
          );
          await expect(boardRow(page, identity.instrument_id)).toHaveAttribute(
            "data-quote-version",
            "1",
          );
          await expectRetainedObservation();
          const later = makeRow(identity, clock, {
            price: "101300.00",
            change: "1300.00",
            percent: "1.30",
            quoteVersion: "2",
            receiveOrdinal: "2",
            stateVersion: "2",
            gapOpen: false,
            sessionHasGap: phase.sessionHasGap,
            gapGeneration: phase.gapGeneration,
          });
          await emitFrame(page, clock, fixture, connection.id, "delta", { rows: [later] });
          await expectLiveQuote(page, identity.instrument_id, "101300.00");
          if (phase.sessionHasGap) {
            await expect(boardRow(page, identity.instrument_id)).toContainText(
              "Gap in this session",
            );
          } else {
            await expect(boardRow(page, identity.instrument_id)).not.toContainText(
              "Gap in this session",
            );
          }
          expect(fixture.metrics().snapshots).toBe(1);
          expect(fixture.metrics().statuses).toBe(2);
          expect(fixture.metrics().deltas).toBe(1);
          expect(fixture.metrics().acceptedCreates).toBe(1);
          expect(fixture.metrics().acceptedRenewals).toBe(0);
          assertNoFallback(page, fixture);
          await page.screenshot({
            animations: "disabled",
            path: testInfo.outputPath("stock-beta-ws-" + phase.reason.toLowerCase() + ".png"),
          });
          passed = true;
        } finally {
          await cleanupFixture(fixture, [page], 1, passed);
        }
      });
    }
    await testInfo.attach("stock-beta-ws-case-10", {
      contentType: "application/json",
      body: JSON.stringify({ subphases: 2, source: "controlled test-only real EventSource" }),
    });
  });

  test("keeps quote catalog placement independent across dashboard and detail", async ({
    page,
    request,
  }, testInfo) => {
    test.setTimeout(60_000);
    await resetScenario(request, ownerScenario());
    const clock = createClock();
    const fixture = await startStockBetaMarketStreamFixture(appOrigin, {
      readMemberships: () => readyIdentities(request),
      now: clock.now,
    });
    let passed = false;
    try {
      const identities = await openStockBeta(page, request, fixture, clock);
      const identity = identities[0];
      if (identity === undefined) throw new Error("Expected A for widget placement case");
      const connection = activeConnection(fixture);
      await sendInitialSnapshot(page, clock, fixture, identities);
      await expectLiveQuote(page, identity.instrument_id, quoteA);
      await emitFrame(page, clock, fixture, connection.id, "status", {
        connection: "CONNECTED",
        reason_code: null,
        gap_open: false,
        session_has_gap: false,
        gap_generation: "0",
      });
      const dashboardIds = await page
        .locator('[data-testid^="stock-beta-widget-"][data-widget-id]')
        .evaluateAll((elements) =>
          elements.map((element) => element.getAttribute("data-widget-id")),
        );
      expect(dashboardIds.at(-1)).toBe("current-quote");
      const dashboardWidget = dashboardQuoteWidget(page);
      for (const breakpoint of ["desktop", "tablet", "mobile"] as const) {
        await expect(dashboardWidget).toHaveAttribute("data-" + breakpoint + "-visible", "true");
      }
      await page.setViewportSize({ width: 375, height: 800 });
      await expect(dashboardWidget).toBeVisible();
      await rankedRow(page, "005930.KRX").getByRole("link", { name: "Open detail" }).click();
      await expect(page).toHaveURL(/\/stock-beta\/005930\.KRX$/);
      const detailWidget = detailQuoteWidget(page);
      await expect(detailWidget).toBeVisible();
      await expect.poll(() => fixture.metrics().acceptedCreates, { timeout: 10_000 }).toBe(2);
      await expect.poll(() => fixture.connectionLedger().length, { timeout: 10_000 }).toBe(2);
      const detailLease = activeLease(fixture);
      const detailConnection = activeConnection(fixture, detailLease.leaseId);
      await emitFrame(
        page,
        clock,
        fixture,
        detailConnection.id,
        "snapshot",
        snapshotBody(fixture, identities, clock),
      );
      await emitFrame(page, clock, fixture, detailConnection.id, "status", {
        connection: "CONNECTED",
        reason_code: null,
        gap_open: false,
        session_has_gap: false,
        gap_generation: "0",
      });
      const detailIds = await page
        .locator('[data-testid^="stock-beta-detail-widget-"][data-widget-id]')
        .evaluateAll((elements) =>
          elements.map((element) => element.getAttribute("data-widget-id")),
        );
      expect(detailIds.at(-1)).toBe("current-quote");
      await expect(detailWidget.getByTestId("stock-beta-stream-selected")).toHaveAttribute(
        "data-availability",
        "LIVE",
      );
      await expect(page.getByTestId("stock-beta-detail-returns")).toBeVisible();
      const row = detailWidget.getByTestId("stock-beta-stream-selected");
      await expect(row.locator("time[datetime]")).toHaveCount(2);
      const width = await row.evaluate((element) => element.getBoundingClientRect().width);
      expect(width).toBeGreaterThan(0);
      await page.setViewportSize({ width: 375, height: 800 });
      await expect(row.locator("time[datetime]")).toHaveCount(2);
      expect(
        await row.evaluate((element) => element.getBoundingClientRect().width),
      ).toBeGreaterThan(0);
      const metrics = fixture.metrics();
      expect(metrics.acceptedCreates).toBe(2);
      expect(metrics.acceptedRenewals).toBe(0);
      expect(metrics.snapshots).toBe(2);
      expect(metrics.statuses).toBe(2);
      expect(metrics.getOpens).toBe(2);
      assertNoFallback(page, fixture);
      await page.screenshot({
        animations: "disabled",
        path: testInfo.outputPath("stock-beta-ws-placement.png"),
      });
      await attachSafeMetrics(testInfo, "stock-beta-ws-case-11", fixture);
      passed = true;
    } finally {
      await cleanupFixture(fixture, [page], 2, passed);
    }
  });

  test("repeats stream GETs 1, 10, and 100 times without lease mutations", async ({
    page,
    request,
  }, testInfo) => {
    test.setTimeout(90_000);
    await resetScenario(request, ownerScenario());
    const clock = createClock();
    const fixture = await startStockBetaMarketStreamFixture(appOrigin, {
      readMemberships: () => readyIdentities(request),
      now: clock.now,
    });
    let passed = false;
    try {
      const identities = await openStockBeta(page, request, fixture, clock);
      const identity = identities[0];
      if (identity === undefined) throw new Error("Expected A for read-only SSE probes");
      const lease = activeLease(fixture);
      const uiConnection = activeConnection(fixture, lease.leaseId);
      await emitFrame(
        page,
        clock,
        fixture,
        uiConnection.id,
        "snapshot",
        snapshotBody(fixture, identities, clock),
      );
      await expectLiveQuote(page, identity.instrument_id, quoteA);
      const initialLeaseLedger = fixture.leaseLedger();
      const initialQuoteVersion = await boardRow(page, identity.instrument_id).getAttribute(
        "data-quote-version",
      );
      const initialReceipt = await boardRow(page, identity.instrument_id).getAttribute(
        "data-received-at",
      );

      async function startBoundedRead(path: string): Promise<void> {
        await page.evaluate((input) => {
          type ProbeResult = {
            readonly status: number;
            readonly bytes: number;
            readonly event: string;
          };
          const target = window as Window & {
            __stockBetaStreamProbe?: Promise<ProbeResult>;
          };
          target.__stockBetaStreamProbe = (async () => {
            const abort = new AbortController();
            const timeout = window.setTimeout(() => abort.abort(), 3_000);
            let reader: ReadableStreamDefaultReader<Uint8Array> | null = null;
            try {
              const response = await fetch(input, {
                cache: "no-store",
                signal: abort.signal,
              });
              if (!response.ok || response.body === null) {
                return { status: response.status, bytes: 0, event: "" };
              }
              reader = response.body.getReader();
              const decoder = new TextDecoder("utf-8", { fatal: true });
              let bytes = 0;
              let text = "";
              while (!text.includes("\n\n")) {
                const part = await reader.read();
                if (part.done) break;
                bytes += part.value.byteLength;
                if (bytes > 256 * 1024) throw new Error("SSE probe exceeded its byte bound");
                text += decoder.decode(part.value, { stream: true });
              }
              if (!text.includes("\n\n")) throw new Error("SSE probe did not receive one frame");
              const event = text.match(/^event: ([a-z]+)$/m)?.[1] ?? "";
              return { status: response.status, bytes, event };
            } finally {
              window.clearTimeout(timeout);
              if (reader !== null) await reader.cancel().catch(() => undefined);
              abort.abort();
            }
          })();
        }, path);
      }

      async function finishBoundedRead(): Promise<{
        status: number;
        bytes: number;
        event: string;
      }> {
        return page.evaluate(async () => {
          const target = window as Window & {
            __stockBetaStreamProbe?: Promise<{ status: number; bytes: number; event: string }>;
          };
          if (target.__stockBetaStreamProbe === undefined) {
            throw new Error("Missing owned SSE probe promise");
          }
          return target.__stockBetaStreamProbe;
        });
      }

      let extraOpens = 0;
      for (const batchSize of [1, 10, 100]) {
        const before = fixture.metrics();
        for (let index = 0; index < batchSize; index += 1) {
          const expectedConnections = 2 + extraOpens;
          const eventPath =
            appOrigin + streamPath + "?lease_id=" + encodeURIComponent(lease.leaseId);
          await startBoundedRead(eventPath);
          await expect
            .poll(() => fixture.connectionLedger().length, { timeout: 5_000 })
            .toBe(expectedConnections);
          const auxiliary = fixture.connectionLedger().at(-1);
          if (auxiliary === undefined || auxiliary.id === uiConnection.id) {
            throw new Error("Expected one new auxiliary read connection");
          }
          await emitFrame(
            page,
            clock,
            fixture,
            auxiliary.id,
            "snapshot",
            snapshotBody(fixture, identities, clock),
          );
          const read = await finishBoundedRead();
          expect(read.status).toBe(200);
          expect(read.bytes).toBeGreaterThan(0);
          expect(read.bytes).toBeLessThanOrEqual(256 * 1024);
          expect(read.event).toBe("snapshot");
          extraOpens += 1;
          await expect.poll(() => fixture.metrics().getCloses, { timeout: 5_000 }).toBe(extraOpens);
          const current = fixture.metrics();
          expect(current.acceptedCreates).toBe(before.acceptedCreates);
          expect(current.acceptedRenewals).toBe(before.acceptedRenewals);
          expect(current.acceptedReleases).toBe(before.acceptedReleases);
          expect(current.activeIdentitySet).toEqual(before.activeIdentitySet);
          expect(current.activeSockets).toBe(1);
          expect(
            fixture.connectionLedger().find((item) => item.id === uiConnection.id)?.closedAt,
          ).toBeNull();
          expect(
            await boardRow(page, identity.instrument_id).getAttribute("data-quote-version"),
          ).toBe(initialQuoteVersion);
          expect(
            await boardRow(page, identity.instrument_id).getAttribute("data-received-at"),
          ).toBe(initialReceipt);
        }
      }
      expect(fixture.metrics().getOpens).toBe(112);
      expect(fixture.metrics().getCloses).toBe(111);
      expect(fixture.metrics().peakSockets).toBeLessThanOrEqual(2);
      expect(fixture.metrics().snapshots).toBe(112);
      expect(fixture.leaseLedger()).toEqual(initialLeaseLedger);
      const beforeUnknownProbe = fixture.metrics();
      const unknownLeaseId = "00000000-0000-4000-8000-000000009999";
      const unknown = await page.evaluate(
        async (input) => {
          const response = await fetch(input, { cache: "no-store" });
          return { status: response.status, body: await response.json() };
        },
        appOrigin + streamPath + "?lease_id=" + unknownLeaseId,
      );
      expect(unknown.status).toBe(404);
      expect(unknown.body).toMatchObject({ error: { code: "RESOURCE_NOT_FOUND" } });
      expect(fixture.metrics().rejectedReadProbes).toBe(1);
      expect(fixture.leaseLedger()).toEqual(initialLeaseLedger);
      expect(fixture.metrics().getOpens).toBe(beforeUnknownProbe.getOpens);
      expect(fixture.metrics().acceptedCreates).toBe(beforeUnknownProbe.acceptedCreates);
      expect(fixture.metrics().acceptedRenewals).toBe(beforeUnknownProbe.acceptedRenewals);
      expect(fixture.metrics().acceptedReleases).toBe(beforeUnknownProbe.acceptedReleases);

      await page
        .getByRole("navigation", { name: "Primary" })
        .getByRole("link", { name: "Strategies", exact: true })
        .click();
      await expect(board(page)).toHaveCount(0);
      await expect.poll(() => fixture.metrics().acceptedReleases, { timeout: 7_000 }).toBe(1);
      await expect.poll(() => fixture.metrics().activeSockets, { timeout: 7_000 }).toBe(0);
      expect(fixture.metrics().getOpens).toBe(112);
      expect(fixture.metrics().getCloses).toBe(112);
      expect(fixture.metrics().snapshots).toBe(112);
      expect(fixture.metrics().acceptedCreates).toBe(1);
      expect(fixture.metrics().acceptedRenewals).toBe(0);
      expect(fixture.metrics().acceptedReleases).toBe(1);
      expect(fixture.metrics().sourcesOpened).toBe(fixture.metrics().sourcesClosed);
      assertNoFallback(page, fixture);
      await attachSafeMetrics(testInfo, "stock-beta-ws-case-12", fixture);
      passed = true;
    } finally {
      await cleanupFixture(fixture, [page], 1, passed);
    }
  });
});
