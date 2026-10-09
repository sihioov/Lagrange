import {
  type APIRequestContext,
  type CDPSession,
  expect,
  type Locator,
  type Page,
  type Request,
  type Response,
  test,
} from "@playwright/test";
import {
  isCurrentServerSeed,
  readCommittedStockBetaChart,
} from "./support/stock-beta-refresh-evidence";

type IntradayQuotesMode = "off" | "owner_only";

type SyntheticIntradayState = {
  readonly active_consumer_count: number;
  readonly active_identity_count: number;
  readonly created_demands: readonly unknown[];
  readonly demand_posts: number;
  readonly quote_gets: number;
};

type SyntheticChartBar = {
  readonly close: number;
  readonly high: number;
  readonly low: number;
  readonly open: number;
  readonly session_date: string;
  readonly sma_20: number | null;
  readonly sma_60: number | null;
  readonly volume: number;
};

type SyntheticChartPayload = {
  readonly bars: readonly SyntheticChartBar[];
  readonly instrument_id: string;
};

type ChartRect = {
  readonly bottom: number;
  readonly height: number;
  readonly left: number;
  readonly right: number;
  readonly top: number;
  readonly width: number;
};

type CanvasPaintEvidence = {
  readonly canvasCount: number;
  readonly fingerprint: number;
  readonly paintedCanvasCount: number;
  readonly plotPaintColors: readonly number[];
  readonly plotPanes: readonly ChartRect[];
};

function intradayQuotesModeFromEnvironment(): IntradayQuotesMode {
  const configured = process.env["OWNER_INTRADAY_QUOTES_MODE"] ?? "off";
  if (configured === "off" || configured === "owner_only") return configured;
  throw new Error(
    `OWNER_INTRADAY_QUOTES_MODE must be "off" or "owner_only" for e2e tests; received ${JSON.stringify(configured)}`,
  );
}

const intradayQuotesMode = intradayQuotesModeFromEnvironment();
const appOrigin = process.env["PLAYWRIGHT_BASE_URL"] ?? "http://127.0.0.1:33000";
const syntheticOrigin = process.env["SYNTHETIC_API_ORIGIN"] ?? "http://127.0.0.1:38180";
const membershipsPath = "/api/v1/research/owner-beta/equity-universe-v2/memberships";
const chartPathFragment = "/api/v1/research/owner-beta/equity-universe-v2/signals/instruments/";
const chartAsOf = "2026-08-28";
const chartFirstSession = "2025-08-29";
const chartLatestClose = "12120";
const chartLatestVolume = "106000";
const chartLatestChange = "+75";
const chartLatestRate = "0.62%";
const uuidPattern = /[0-9a-f]{8}-[0-9a-f]{4}-[4-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}/i;
const forbiddenRoutePattern =
  /(?:^|\/)(?:account|accounts|balance|buying-power|sellable-quantity|execution-history|order|orders|live)(?:\/|$)/i;

const observedBrowserRequests = new WeakMap<Page, URL[]>();
const observedPageErrors = new WeakMap<Page, string[]>();
const observedBrowserSockets = new WeakMap<URL[], string[]>();

async function resetScenario(request: APIRequestContext, scenario: Record<string, unknown>) {
  const response = await request.post(`${syntheticOrigin}/__test/scenario`, { data: scenario });
  expect(response.ok()).toBeTruthy();
}

async function syntheticIntradayState(request: APIRequestContext): Promise<SyntheticIntradayState> {
  const response = await request.get(`${syntheticOrigin}/__test/stock-beta/intraday-state`);
  expect(response.ok(), "synthetic intraday state status").toBeTruthy();
  return (await response.json()) as SyntheticIntradayState;
}

function observeBrowserRequests(page: Page) {
  const requests: URL[] = [];
  const pageErrors: string[] = [];
  const sockets: string[] = [];
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.protocol === "http:" || url.protocol === "https:") requests.push(url);
  });
  page.on("pageerror", (error) => pageErrors.push(error.message));
  page.on("websocket", (socket) => sockets.push(socket.url()));
  observedBrowserRequests.set(page, requests);
  observedPageErrors.set(page, pageErrors);
  observedBrowserSockets.set(requests, sockets);
  return requests;
}

function isLocalTestUrl(url: URL) {
  const expectedPorts = new Set([new URL(appOrigin).port, new URL(syntheticOrigin).port]);
  return ["127.0.0.1", "localhost"].includes(url.hostname) && expectedPorts.has(url.port);
}

function isAppHmrSocket(url: URL) {
  return (
    ["ws:", "wss:"].includes(url.protocol) &&
    isLocalTestUrl(url) &&
    url.port === new URL(appOrigin).port &&
    url.pathname === "/_next/hmr"
  );
}

async function installProviderFreeNetworkGuard(page: Page) {
  await page.routeWebSocket("**/*", async (socket) => {
    observedBrowserSockets.get(requestsFor(page))?.push(socket.url());
    if (isAppHmrSocket(new URL(socket.url()))) {
      socket.connectToServer();
      return;
    }
    await socket.close({ code: 1000, reason: "synthetic chart QA blocks provider sockets" });
  });
  await page.route("**/*", async (route) => {
    const url = new URL(route.request().url());
    if (isLocalTestUrl(url)) {
      await route.continue();
    } else {
      await route.abort("blockedbyclient");
    }
  });
}

function expectProviderFree(requests: URL[]) {
  expect(
    (observedBrowserSockets.get(requests) ?? []).filter((url) => !isAppHmrSocket(new URL(url))),
    "browser WebSocket attempts outside the app's local development HMR socket",
  ).toEqual([]);
  const externalUrls = requests.filter((url) => !isLocalTestUrl(url)).map((url) => url.href);
  expect(externalUrls, `External browser requests: ${externalUrls.join(", ") || "none"}`).toEqual(
    [],
  );
  expect(
    requests.filter((url) => /kis|opendart/i.test(`${url.hostname}${url.pathname}`)),
  ).toHaveLength(0);
  expect(requests.filter((url) => url.pathname.includes("equity-price-signals"))).toHaveLength(0);
  const forbiddenApiUrls = requests
    .filter((url) => url.pathname.startsWith("/api/"))
    .filter((url) => forbiddenRoutePattern.test(url.pathname))
    .map((url) => url.href);
  expect(
    forbiddenApiUrls,
    `Forbidden API requests: ${forbiddenApiUrls.join(", ") || "none"}`,
  ).toEqual([]);
}

function requestsFor(page: Page) {
  return observedBrowserRequests.get(page) ?? [];
}

function chartRequests(requests: URL[], instrumentId?: string, range?: string) {
  return requests.filter((url) => {
    if (!url.pathname.includes(chartPathFragment) || !url.pathname.endsWith("/chart")) return false;
    if (instrumentId && !url.pathname.includes(encodeURIComponent(instrumentId))) return false;
    if (range && url.searchParams.get("range") !== range) return false;
    return true;
  });
}

async function waitForChartPayload(
  page: Page,
  range: string,
  instrumentId = "000001.KRX",
): Promise<SyntheticChartPayload> {
  const response = await page.waitForResponse((candidate) => {
    const url = new URL(candidate.url());
    return (
      candidate.request().method() === "GET" &&
      url.pathname.includes(`${chartPathFragment}${encodeURIComponent(instrumentId)}/chart`) &&
      url.searchParams.get("range") === range
    );
  });
  expect(response.ok(), `chart ${range} response status`).toBeTruthy();
  return (await response.json()) as SyntheticChartPayload;
}

async function fetchServerSeededChartPayload(
  request: APIRequestContext,
  snapshotId: string,
  range: string,
  instrumentId = "000001.KRX",
): Promise<SyntheticChartPayload> {
  const url = new URL(`${chartPathFragment}${encodeURIComponent(instrumentId)}/chart`, appOrigin);
  url.searchParams.set("range", range);
  url.searchParams.set("snapshot_id", snapshotId);
  const response = await request.get(url.toString());
  expect(response.ok(), `server-seeded chart ${range} response status`).toBeTruthy();
  return (await response.json()) as SyntheticChartPayload;
}

async function canvasPaintEvidence(host: Locator): Promise<CanvasPaintEvidence> {
  return host.evaluate((element) => {
    const canvases = Array.from(element.querySelectorAll("canvas"));
    const sampledCanvases = canvases.map((canvas) => {
      const context = canvas.getContext("2d", { willReadFrequently: true });
      if (context === null || canvas.width < 1 || canvas.height < 1)
        return { colors: 0, fingerprint: 0 };
      const pixels = context.getImageData(0, 0, canvas.width, canvas.height).data;
      const colors = new Set<string>();
      const pixelStep = Math.max(1, Math.floor(pixels.length / 16_384 / 4));
      let fingerprint = 2_166_136_261;
      for (let offset = 0; offset < pixels.length; offset += pixelStep * 4) {
        const alpha = pixels[offset + 3];
        if (alpha !== undefined && alpha > 0) {
          colors.add(`${pixels[offset]},${pixels[offset + 1]},${pixels[offset + 2]}`);
        }
        for (let channel = 0; channel < 4; channel += 1) {
          fingerprint = Math.imul(fingerprint ^ (pixels[offset + channel] ?? 0), 16_777_619);
        }
      }
      return { colors: colors.size, fingerprint: fingerprint >>> 0 };
    });
    const paintedCanvasCount = sampledCanvases.filter(({ colors }) => colors > 5).length;
    const fingerprint = sampledCanvases.reduce(
      (combined, sampled) => Math.imul(combined ^ sampled.fingerprint, 16_777_619) >>> 0,
      2_166_136_261,
    );
    // In pinned 5.2.1, PaneWidget marks its middle td position:relative.
    // TimeAxisWidget does not. This identifies plots independently of pane height.
    const plotCanvases = Array.from(element.querySelectorAll<HTMLTableRowElement>("tr"))
      .filter((row) => row.cells.length === 3 && row.cells.item(1)?.style.position === "relative")
      .flatMap((row) => {
        const canvas = Array.from(row.cells.item(1)?.querySelectorAll("canvas") ?? []).find(
          (candidate) => candidate.style.zIndex === "1",
        );
        return canvas === undefined ? [] : [canvas];
      });
    return {
      canvasCount: canvases.length,
      fingerprint,
      paintedCanvasCount,
      plotPaintColors: plotCanvases.map(
        (canvas) => sampledCanvases[canvases.indexOf(canvas)]?.colors ?? 0,
      ),
      plotPanes: plotCanvases.map((canvas) => {
        const { bottom, height, left, right, top, width } = canvas.getBoundingClientRect();
        return { bottom, height, left, right, top, width };
      }),
    };
  });
}

async function chartCanvasPalette(host: Locator): Promise<readonly string[]> {
  return host.evaluate((element) => {
    const canvas = element.querySelector<HTMLCanvasElement>(
      'td[style*="position: relative"] canvas[style*="z-index: 1"]',
    );
    const context = canvas?.getContext("2d", { willReadFrequently: true });
    if (canvas === null || context === null || context === undefined) return [];
    const pixels = context.getImageData(0, 0, canvas.width, canvas.height).data;
    const colors = new Set<string>();
    const pixelStep = Math.max(1, Math.floor(pixels.length / 16_384 / 4));
    for (let offset = 0; offset < pixels.length; offset += pixelStep * 4) {
      const alpha = pixels[offset + 3];
      if (alpha !== undefined && alpha > 0) {
        colors.add(`${pixels[offset]},${pixels[offset + 1]},${pixels[offset + 2]}`);
      }
    }
    return [...colors].sort();
  });
}

async function expectCanvasReady(chart: Locator): Promise<CanvasPaintEvidence> {
  await expect(chart).toHaveAttribute("data-renderer-status", "ready");
  const host = chart.getByTestId("stock-beta-price-chart-canvas");
  await expect
    .poll(async () => {
      const evidence = await canvasPaintEvidence(host);
      return (evidence.plotPaintColors[0] ?? 0) > 5 && (evidence.plotPaintColors[1] ?? 0) > 2;
    })
    .toBeTruthy();
  const evidence = await canvasPaintEvidence(host);
  expect(evidence.canvasCount).toBeGreaterThan(0);
  expect(evidence.paintedCanvasCount).toBeGreaterThan(0);
  await expectChartGeometry(chart);
  return evidence;
}

async function expectChartGeometry(chart: Locator) {
  const geometry = await chart.getByTestId("stock-beta-price-chart-canvas").evaluate((host) => {
    const rect = (element: Element) => {
      const { bottom, height, left, right, top, width } = element.getBoundingClientRect();
      return { bottom, height, left, right, top, width };
    };
    const surface = host.closest('[data-testid="stock-beta-price-chart-surface"]');
    const table = host.querySelector("table");
    const panes = Array.from(table?.rows ?? [])
      .filter((row) => row.cells.length === 3 && row.cells.item(1)?.style.position === "relative")
      .flatMap((row) => {
        const leftAxis = row.cells.item(0);
        const center = row.cells.item(1);
        const rightAxis = row.cells.item(2);
        const plot = Array.from(center?.querySelectorAll("canvas") ?? []).find(
          (canvas) => canvas.style.zIndex === "1",
        );
        if (leftAxis === null || rightAxis === null || plot === undefined) return [];
        return [
          {
            leftAxis: rect(leftAxis),
            leftAxisEmpty: leftAxis.childElementCount === 0,
            leftAxisInlineWidth: leftAxis.style.width,
            plot: rect(plot),
            rightAxis: rect(rightAxis),
          },
        ];
      });
    return {
      canvases: Array.from(host.querySelectorAll("canvas")).map(rect),
      host: rect(host),
      hostClientHeight: host.clientHeight,
      hostScrollHeight: host.scrollHeight,
      panes,
      surface: surface === null ? null : rect(surface),
      table: table === null ? null : rect(table),
      zeroWidthCells: Array.from(host.querySelectorAll<HTMLTableCellElement>("td"))
        .filter(
          (cell) =>
            Number.parseFloat(cell.style.width) === 0 ||
            (cell.cellIndex === 0 &&
              cell.parentElement?.children.length === 3 &&
              cell.childElementCount === 0),
        )
        .map(rect),
    };
  });
  const fits = (inner: ChartRect, outer: ChartRect, label: string) => {
    expect(inner.left, `${label}: left`).toBeGreaterThanOrEqual(outer.left - 1);
    expect(inner.right, `${label}: right`).toBeLessThanOrEqual(outer.right + 1);
    expect(inner.top, `${label}: top`).toBeGreaterThanOrEqual(outer.top - 1);
    expect(inner.bottom, `${label}: bottom`).toBeLessThanOrEqual(outer.bottom + 1);
  };
  expect(geometry.surface, "chart surface exists").not.toBeNull();
  expect(geometry.table, "library table exists").not.toBeNull();
  if (geometry.surface === null || geometry.table === null) throw new Error("Missing chart layout");
  fits(geometry.host, geometry.surface, "host inside unclipped surface");
  fits(geometry.table, geometry.host, "library table inside host");
  expect(geometry.hostScrollHeight).toBeLessThanOrEqual(geometry.hostClientHeight + 1);
  for (const [index, canvas] of geometry.canvases.entries()) {
    fits(canvas, geometry.host, `canvas/axis ${index} inside host`);
  }
  expect(geometry.panes, "price and volume plot rows").toHaveLength(2);
  expect(
    geometry.zeroWidthCells.length,
    "empty or declared-zero left axes exist",
  ).toBeGreaterThanOrEqual(2);
  for (const cell of geometry.zeroWidthCells) {
    expect(
      cell.width,
      "declared-zero cell must not inherit a global minimum width",
    ).toBeLessThanOrEqual(1);
  }
  for (const pane of geometry.panes) {
    expect(pane.leftAxisEmpty, "configured hidden left axis has no widget").toBe(true);
    if (pane.leftAxisInlineWidth !== "")
      expect(Number.parseFloat(pane.leftAxisInlineWidth)).toBe(0);
    expect(pane.leftAxis.width, "hidden left axis has no gutter").toBeLessThanOrEqual(1);
    expect(
      Math.abs(pane.plot.left - geometry.host.left),
      "plot begins at host left",
    ).toBeLessThanOrEqual(1);
    expect(
      Math.abs(pane.plot.right - pane.rightAxis.left),
      "plot joins right axis",
    ).toBeLessThanOrEqual(1);
    expect(
      Math.abs(pane.rightAxis.right - geometry.host.right),
      "right axis ends at host right",
    ).toBeLessThanOrEqual(1);
  }
  const [price, volume] = geometry.panes;
  if (price === undefined || volume === undefined) throw new Error("Missing chart pane");
  expect(Math.abs(price.plot.left - volume.plot.left), "pane left alignment").toBeLessThanOrEqual(
    1,
  );
  expect(
    Math.abs(price.plot.right - volume.plot.right),
    "pane right alignment",
  ).toBeLessThanOrEqual(1);
  return geometry;
}

async function visiblePlotTarget(chart: Locator, xFraction = 0.5, paneIndex = 0) {
  const canvas = chart
    .getByTestId("stock-beta-price-chart-canvas")
    .locator('td[style*="position: relative"] canvas[style*="z-index: 1"]')
    .nth(paneIndex);
  await canvas.scrollIntoViewIfNeeded();
  const target = await canvas.evaluate((element, fraction) => {
    const bounds = element.getBoundingClientRect();
    let left = Math.max(0, bounds.left);
    let right = Math.min(window.innerWidth, bounds.right);
    let top = Math.max(0, bounds.top);
    let bottom = Math.min(window.innerHeight, bounds.bottom);
    for (
      let ancestor = element.parentElement;
      ancestor !== null;
      ancestor = ancestor.parentElement
    ) {
      const style = getComputedStyle(ancestor);
      const clip = ancestor.getBoundingClientRect();
      if (style.overflowX !== "visible") {
        left = Math.max(left, clip.left);
        right = Math.min(right, clip.right);
      }
      if (style.overflowY !== "visible") {
        top = Math.max(top, clip.top);
        bottom = Math.min(bottom, clip.bottom);
      }
    }
    const x = bounds.left + bounds.width * fraction;
    const y = (top + bottom) / 2;
    const hit = document.elementFromPoint(x, y);
    return {
      hitCanvas: hit instanceof HTMLCanvasElement && element.parentElement?.contains(hit) === true,
      plot: {
        bottom: bounds.bottom,
        height: bounds.height,
        left: bounds.left,
        right: bounds.right,
        top: bounds.top,
        width: bounds.width,
      },
      visible: { bottom, height: bottom - top, left, right, top, width: right - left },
      x,
      y,
    };
  }, xFraction);
  expect(target.visible.height, "visible price plot height").toBeGreaterThan(20);
  expect(target.visible.left, "plot left is not horizontally clipped").toBeLessThanOrEqual(
    target.plot.left + 1,
  );
  expect(target.visible.right, "plot right is not horizontally clipped").toBeGreaterThanOrEqual(
    target.plot.right - 1,
  );
  expect(target.x).toBeGreaterThanOrEqual(target.visible.left);
  expect(target.x).toBeLessThanOrEqual(target.visible.right);
  expect(target.hitCanvas, "gesture target must hit this plot's real Canvas").toBe(true);
  return target;
}

async function expectUnclipped(target: Locator, label: string) {
  await target.evaluate((element) =>
    element.scrollIntoView({ block: "center", inline: "nearest", behavior: "instant" }),
  );
  await target
    .page()
    .evaluate(
      () =>
        new Promise<void>((resolve) =>
          requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
        ),
    );
  const bounds = await target.evaluate((element) => {
    const rect = element.getBoundingClientRect();
    let left = 0;
    let right = window.innerWidth;
    let top = 0;
    let bottom = window.innerHeight;
    for (let parent = element.parentElement; parent !== null; parent = parent.parentElement) {
      const style = getComputedStyle(parent);
      const clip = parent.getBoundingClientRect();
      if (style.overflowX !== "visible") {
        left = Math.max(left, clip.left);
        right = Math.min(right, clip.right);
      }
      if (style.overflowY !== "visible") {
        top = Math.max(top, clip.top);
        bottom = Math.min(bottom, clip.bottom);
      }
    }
    return {
      bottom: rect.bottom,
      left: rect.left,
      right: rect.right,
      top: rect.top,
      clip: { bottom, left, right, top },
    };
  });
  expect(bounds.left, `${label} left is reachable`).toBeGreaterThanOrEqual(bounds.clip.left - 1);
  expect(bounds.right, `${label} right is reachable`).toBeLessThanOrEqual(bounds.clip.right + 1);
  expect(bounds.top, `${label} top is reachable`).toBeGreaterThanOrEqual(bounds.clip.top - 1);
  expect(bounds.bottom, `${label} bottom is reachable`).toBeLessThanOrEqual(bounds.clip.bottom + 1);
}

async function moveToPlotObservation(chart: Locator, xFraction: number) {
  const target = await visiblePlotTarget(chart, xFraction);
  await chart.page().mouse.move(target.x, target.y);
  await chart
    .page()
    .evaluate(
      () =>
        new Promise<void>((resolve) =>
          requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
        ),
    );
  return selectedChartDate(chart.getByTestId("stock-beta-price-chart-summary"));
}

async function inspectTouchPlotObservation(chart: Locator, cdp: CDPSession, xFraction: number) {
  const target = await visiblePlotTarget(chart, xFraction);
  await cdp.send("Input.dispatchTouchEvent", {
    type: "touchStart",
    touchPoints: [{ id: 1, x: target.x, y: target.y, force: 1 }],
  });
  await chart.page().waitForTimeout(600);
  await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  await chart
    .page()
    .evaluate(
      () =>
        new Promise<void>((resolve) =>
          requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
        ),
    );
  const date = await selectedChartDate(chart.getByTestId("stock-beta-price-chart-summary"));
  // The native long-press mode exits on the next ordinary tap.
  const exitTarget = await visiblePlotTarget(chart, xFraction);
  await chart.page().touchscreen.tap(exitTarget.x, exitTarget.y);
  return date;
}

async function expectPointerBoundary(
  chart: Locator,
  payload: SyntheticChartPayload,
  boundary: "first" | "latest",
) {
  const index = boundary === "first" ? 0 : payload.bars.length - 1;
  const expected = requiredChartBar(payload, index);
  const { plot } = await visiblePlotTarget(chart);
  let actual: string | null = null;
  // Sweep a bounded edge strip to find the actual candle center, including fitted right whitespace.
  // Keep the right target clear of the adjacent axis's rounded hit-test boundary.
  const inset = boundary === "first" ? 0.5 : 1.5;
  for (let distance = inset; distance <= Math.min(32, plot.width * 0.12); distance += 0.5) {
    const fraction = boundary === "first" ? distance / plot.width : 1 - distance / plot.width;
    actual = await moveToPlotObservation(chart, fraction);
    if (actual === expected.session_date) break;
  }
  expect(actual, `${boundary} loaded observation fits the plot and is pointer-selectable`).toBe(
    expected.session_date,
  );
  await expect(chart).toHaveAttribute("data-selected-index", String(index));
  await expectTooltipMatchesBar(chart, expected);
}

async function chartScrollPositions(page: Page) {
  return page.evaluate(() => {
    const host = document.querySelector('[data-testid="stock-beta-price-chart-canvas"]');
    const widget = host?.closest('[data-testid="stock-beta-widget-signal-profile"]');
    if (host === null || widget == null) throw new Error("Missing chart scroll structure");
    const nested: { depth: number; scrollTop: number; testId: string | null }[] = [];
    let depth = 0;
    for (
      let element = host.parentElement;
      element !== null && element !== widget.parentElement;
      element = element.parentElement
    ) {
      const style = getComputedStyle(element);
      if (
        ["auto", "scroll"].includes(style.overflowY) &&
        element.scrollHeight > element.clientHeight
      ) {
        nested.push({
          depth,
          scrollTop: element.scrollTop,
          testId: element.getAttribute("data-testid"),
        });
      }
      depth += 1;
    }
    return {
      documentScrollTop: document.scrollingElement?.scrollTop ?? window.scrollY,
      mainScrollTop: document.querySelector("main")?.scrollTop ?? 0,
      nested,
      windowScrollY: window.scrollY,
    };
  });
}

type TouchPoint = { readonly id: number; readonly x: number; readonly y: number };

async function dispatchTimedTouches(
  page: Page,
  cdp: CDPSession,
  start: readonly TouchPoint[],
  end: readonly TouchPoint[],
) {
  expect(end.map(({ id }) => id)).toEqual(start.map(({ id }) => id));
  const frame = (type: "touchStart" | "touchMove" | "touchEnd", points: readonly TouchPoint[]) =>
    cdp.send("Input.dispatchTouchEvent", {
      type,
      touchPoints: points.map((point) => ({ ...point, force: 1, radiusX: 4, radiusY: 4 })),
    });
  await frame("touchStart", start);
  await page.waitForTimeout(40);
  for (let step = 1; step <= 10; step += 1) {
    await frame(
      "touchMove",
      start.map((point, index) => {
        const destination = end[index];
        if (destination === undefined) throw new Error("Missing touch destination");
        return {
          id: point.id,
          x: point.x + ((destination.x - point.x) * step) / 10,
          y: point.y + ((destination.y - point.y) * step) / 10,
        };
      }),
    );
    await page.waitForTimeout(20);
  }
  await frame("touchEnd", []);
  await page.waitForTimeout(80);
}

function formattedChartValue(value: number): string {
  return new Intl.NumberFormat("en-US", { maximumFractionDigits: 2 }).format(value);
}

function requiredChartBar(payload: SyntheticChartPayload, index: number): SyntheticChartBar {
  const bar = payload.bars[index];
  if (bar === undefined) throw new Error(`Expected chart observation at index ${index}`);
  return bar;
}

function selectedChartDateFromSummary(text: string | null): string | null {
  return text?.match(/\b\d{4}-\d{2}-\d{2}\b/)?.[0] ?? null;
}

async function selectedChartDate(summary: Locator): Promise<string | null> {
  return selectedChartDateFromSummary(await summary.textContent());
}

async function expectTooltipMatchesBar(chart: Locator, bar: SyntheticChartBar): Promise<void> {
  const details = await chart.getByTestId("stock-beta-price-chart-tooltip").evaluate((tooltip) =>
    Array.from(tooltip.children).map((item) => ({
      label: item.children.item(0)?.textContent?.trim() ?? "",
      value: item.children.item(1)?.textContent?.trim() ?? "",
    })),
  );
  const values = new Map(details.map(({ label, value }) => [label, value]));
  for (const [label, value] of [
    ["Open", bar.open],
    ["High", bar.high],
    ["Low", bar.low],
    ["Close", bar.close],
    ["Volume", bar.volume],
  ] as const) {
    expect(values.get(label), `${label} for ${bar.session_date}`).toBe(formattedChartValue(value));
  }
}

function snapshotIdFromText(text: string | null) {
  const snapshotId = text?.match(uuidPattern)?.[0];
  if (!snapshotId) throw new Error("Expected the snapshot strip to contain a snapshot id");
  return snapshotId;
}

function observeMembershipPostCount(page: Page) {
  let count = 0;
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (request.method() === "POST" && url.pathname === membershipsPath) count += 1;
  });
  return () => count;
}

function membershipRegion(page: Page) {
  return page.getByRole("region", { name: "Membership status" });
}

function universeManagementRegion(page: Page) {
  return page.getByRole("region", { name: "V2 universe management" });
}

function membershipCards(page: Page) {
  return membershipRegion(page).getByTestId("stock-beta-membership-card");
}

function membershipCard(page: Page, instrumentId: string) {
  return membershipCards(page).filter({ hasText: instrumentId });
}

function stockBetaRegion(page: Page) {
  return page.getByRole("region", { name: "Stock signal beta" });
}

function snapshotStrip(page: Page) {
  return stockBetaRegion(page).getByTestId("stock-beta-snapshot-strip");
}

function rankedSignalsRegion(page: Page) {
  return page.getByRole("region", { name: "Ranked signals" });
}

function rankedSignalsTable(page: Page) {
  return rankedSignalsRegion(page).getByRole("table", {
    name: /V2 ranked price-and-volume signal table/,
  });
}

function signalProfileRegion(page: Page) {
  return page.getByRole("region", { name: "Selected signal profile" });
}

function signalProfilePanel(page: Page, name = "Price") {
  return signalProfileRegion(page).getByRole("tabpanel", { name });
}

function detailInstrumentRegion(page: Page, instrumentId: string) {
  return page.getByRole("region", { name: instrumentId, exact: true });
}

function rankedRow(page: Page, instrumentId: string) {
  return rankedSignalsTable(page).getByTestId(`stock-beta-row-${instrumentId}`);
}

function signalPreview(page: Page) {
  return signalProfileRegion(page).getByTestId("stock-beta-signal-preview");
}

function priceChart(page: Page) {
  // Next can retain a hidden streamed copy while hydrating the visible profile.
  // Keep the accessible panel unique without depending on its translated label.
  return page
    .getByRole("tabpanel")
    .and(page.getByTestId("stock-beta-signal-profile"))
    .getByTestId("stock-beta-price-chart");
}

function priceChartPart(page: Page, testId: string) {
  return priceChart(page).getByTestId(testId);
}

function dashboardRegion(page: Page, name: string) {
  return page.getByRole("region", { name });
}

async function expectNoSignalWidgets(page: Page) {
  for (const testId of [
    "stock-beta-snapshot-strip",
    "stock-beta-rank-table",
    "stock-beta-signal-preview",
    "stock-beta-signal-decomposition",
    "stock-beta-condition-matrix",
    "stock-beta-snapshot-tape",
  ]) {
    await expect(page.getByTestId(testId)).toHaveCount(0);
  }
}

async function expectStockShell(page: Page) {
  await expect(page.locator('[data-shell="stock-beta-terminal"]')).toHaveCount(1);
  await expect(page.locator('[data-terminal-utility-bar="stock-beta"]')).toHaveCount(1);
  await expect(page.locator("main")).toHaveCount(1);
}

async function expectNoHorizontalOverflow(page: Page) {
  await expect
    .poll(() => page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth))
    .toBeTruthy();
}

async function box(locator: Locator) {
  const result = await locator.boundingBox();
  if (result === null) {
    throw new Error("Expected the locator to have a bounding box");
  }
  return result;
}

function expectBoxInside(
  inner: { x: number; y: number; width: number; height: number },
  outer: {
    x: number;
    y: number;
    width: number;
    height: number;
  },
) {
  expect(inner.x).toBeGreaterThanOrEqual(outer.x);
  expect(inner.y).toBeGreaterThanOrEqual(outer.y);
  expect(inner.x + inner.width).toBeLessThanOrEqual(outer.x + outer.width);
  expect(inner.y + inner.height).toBeLessThanOrEqual(outer.y + outer.height);
}

async function clippingAncestorBoxes(locator: Locator) {
  return locator.evaluate((element) => {
    const clippingValues = new Set(["auto", "clip", "hidden", "scroll"]);
    const ancestors: Array<{
      readonly box: { height: number; width: number; x: number; y: number };
      readonly overflowX: string;
      readonly overflowY: string;
      readonly tagName: string;
    }> = [];
    for (
      let ancestor = element.parentElement;
      ancestor !== null;
      ancestor = ancestor.parentElement
    ) {
      const style = getComputedStyle(ancestor);
      if (!clippingValues.has(style.overflowX) && !clippingValues.has(style.overflowY)) continue;
      const bounds = ancestor.getBoundingClientRect();
      ancestors.push({
        box: { height: bounds.height, width: bounds.width, x: bounds.x, y: bounds.y },
        overflowX: style.overflowX,
        overflowY: style.overflowY,
        tagName: ancestor.tagName,
      });
    }
    return ancestors;
  });
}

async function installPrimaryNavigationRevealInstrumentation(page: Page) {
  await page.addInitScript({
    content: `
      window.__stockBetaPrimaryNavigationRevealEvents = [];
      const nativeScrollTo = HTMLElement.prototype.scrollTo;
      HTMLElement.prototype.scrollTo = function (...args) {
        if (this.getAttribute("aria-label") !== "Primary") {
          return nativeScrollTo.apply(this, args);
        }
        const focusedBefore = document.activeElement;
        const bodyScrollYBefore = window.scrollY;
        const result = nativeScrollTo.apply(this, args);
        window.__stockBetaPrimaryNavigationRevealEvents.push({
          bodyScrollYAfter: window.scrollY,
          bodyScrollYBefore,
          focusedAfter: document.activeElement === null ? null : document.activeElement.getAttribute("href"),
          focusedBefore: focusedBefore === null ? null : focusedBefore.getAttribute("href"),
        });
        return result;
      };
    `,
  });
}

async function primaryNavigationRevealEvents(page: Page) {
  return page.evaluate(() => {
    const instrumentedWindow = window as Window & {
      __stockBetaPrimaryNavigationRevealEvents?: Array<{
        readonly bodyScrollYAfter: number;
        readonly bodyScrollYBefore: number;
        readonly focusedAfter: string | null;
        readonly focusedBefore: string | null;
      }>;
    };
    return instrumentedWindow.__stockBetaPrimaryNavigationRevealEvents ?? [];
  });
}

test.describe("provider-free Stock Beta V2", () => {
  test.beforeEach(async ({ page }) => {
    observeBrowserRequests(page);
    await installProviderFreeNetworkGuard(page);
  });

  test.afterEach(async ({ page }) => {
    expectProviderFree(requestsFor(page));
    expect(observedPageErrors.get(page) ?? [], "browser page errors").toEqual([]);
  });

  test("renders an empty owner capacity with no signal rows", async ({ page, request }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 0,
      stockBetaSeed: "empty",
    });

    await page.goto("/stock-beta");
    await expectStockShell(page);
    const membershipEmpty = page
      .getByRole("region", { name: "Membership status" })
      .getByText(/No research instruments are configured\./);
    await expect(membershipEmpty).toHaveCount(1);
    await expect(membershipEmpty).toBeVisible();
    await expect(
      universeManagementRegion(page).getByTestId("stock-beta-policy-capacity"),
    ).toContainText("100");
    await expect(page.getByRole("heading", { name: "Signal snapshot unavailable" })).toBeVisible();
    await expectNoSignalWidgets(page);
    await expect(page.getByLabel("KRX code")).toBeVisible();
    const disabledNotice = page
      .getByRole("status")
      .filter({ hasText: "Intraday quote collection is not enabled." });
    const quoteWidget = page.getByTestId("stock-beta-widget-current-quote");
    if (intradayQuotesMode === "off") {
      await expect(disabledNotice).toHaveText("Intraday quote collection is not enabled.");
      await expect(quoteWidget).toHaveCount(0);
    } else {
      await expect(disabledNotice).toHaveCount(0);
      await expect(quoteWidget).toHaveCount(1);
      await expect(quoteWidget.getByTestId("stock-beta-current-quote")).toHaveCount(0);
      await expect(quoteWidget.locator("[data-status-phase]")).toHaveAttribute(
        "data-status-phase",
        "idle",
      );
    }
    const intradayState = await syntheticIntradayState(request);
    expect(intradayState.active_consumer_count).toBe(0);
    expect(intradayState.active_identity_count).toBe(0);
    expect(intradayState.created_demands).toEqual([]);
    expect(intradayState.demand_posts).toBe(0);
    expect(intradayState.quote_gets).toBe(0);
    await expect(page.locator('[data-terminal-utility-content="stock-beta"]')).toHaveCount(0);
  });

  test("renders a typed V2 snapshot with zero rows", async ({ page, request }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 0,
      stockBetaSeed: "empty",
      stockBetaSnapshot: "empty",
    });

    await page.goto("/stock-beta");
    await expectStockShell(page);
    const terminalSnapshot = snapshotStrip(page);
    await expect(terminalSnapshot).toBeVisible();
    await expect(terminalSnapshot.getByTestId("stock-beta-snapshot-universe")).toContainText("0");
    await expect(
      rankedSignalsRegion(page).getByText("The current V2 snapshot has no signal rows."),
    ).toBeVisible();
    await expect(page.getByTestId("stock-beta-rank-table")).toHaveCount(0);
    await expectNoHorizontalOverflow(page);
  });

  test("renders exactly 31 V2 memberships and ranked rows", async ({ page, request }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 31,
      stockBetaSeed: "ready",
    });

    await page.goto("/stock-beta");
    await expect(membershipCards(page)).toHaveCount(31);
    await expect(
      page.getByRole("table", { name: /V2 ranked price-and-volume signal table/ }),
    ).toBeVisible();
    await expect(rankedSignalsTable(page).locator("tbody tr")).toHaveCount(31);
    await expect(rankedRow(page, "000031.KRX")).toBeVisible();
    await expect(rankedRow(page, "000001.KRX")).toHaveAttribute("data-top-five", "true");
    await expect(page.locator('[data-top-five="true"]')).toHaveCount(5);
    await expect(snapshotStrip(page).getByTestId("stock-beta-snapshot-universe")).toContainText(
      "31",
    );
    await expectNoHorizontalOverflow(page);
  });

  test("renders exactly 100 rows and closes the capacity boundary", async ({ page, request }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 100,
      stockBetaSeed: "ready",
    });

    await page.goto("/stock-beta");
    await expect(membershipCards(page)).toHaveCount(100);
    await expect(rankedSignalsTable(page).locator("tbody tr")).toHaveCount(100);
    await expect(rankedRow(page, "000100.KRX")).toBeVisible();
    await expect(snapshotStrip(page).getByTestId("stock-beta-snapshot-universe")).toContainText(
      "100",
    );
    await expect(page.getByRole("button", { name: "Add instrument" })).toBeDisabled();
  });

  test("rejects every invalid six-digit shape without a membership POST", async ({
    page,
    request,
  }) => {
    const getMembershipPostCount = observeMembershipPostCount(page);
    const invalidCodes = [
      { name: "short five-digit input", value: "12345" },
      { name: "long seven-digit input", value: "1234567" },
      { name: "six digits plus a suffix", value: "123456X" },
      { name: "non-digit input", value: "ABCDEF" },
    ];
    const locales = [
      {
        addButton: "Add instrument",
        codeLabel: "KRX code",
        cookie: "en",
        message: "Enter exactly six ASCII digits.",
      },
      {
        addButton: "종목 추가",
        codeLabel: "KRX 코드",
        cookie: "ko",
        message: "ASCII 숫자 6자리를 정확히 입력하세요.",
      },
    ];

    for (const locale of locales) {
      await resetScenario(request, {
        authSession: "valid",
        role: "owner",
        stockBetaRows: 0,
        stockBetaSeed: "empty",
      });
      await page.context().addCookies([{ name: "locale", value: locale.cookie, url: appOrigin }]);
      await page.goto("/stock-beta");
      const input = page.getByLabel(locale.codeLabel);
      const addButton = page.getByRole("button", { name: locale.addButton });
      const validationMessage = page.locator('[role="alert"]').filter({ hasText: locale.message });

      for (const invalidCode of invalidCodes) {
        await input.fill(invalidCode.value);
        await expect(input).toHaveValue(invalidCode.value);
        await addButton.click();
        await expect(validationMessage).toBeVisible();
        await expect(input).toHaveValue(invalidCode.value);
        expect(getMembershipPostCount(), invalidCode.name).toBe(0);
        await input.fill("");
      }
    }
  });

  test("adds, polls to READY, refreshes the rank, and opens V2 detail", async ({
    page,
    request,
  }) => {
    const observed = observeBrowserRequests(page);
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 0,
      stockBetaSeed: "empty",
    });

    await page.goto("/stock-beta");
    await page.getByLabel("KRX code").fill("005930");
    await page.getByRole("button", { name: "Add instrument" }).click();

    const card = membershipCard(page, "005930.KRX");
    await expect(card).toHaveAttribute("data-lifecycle", "READY", { timeout: 20_000 });
    const row = rankedRow(page, "005930.KRX");
    await expect(row).toBeVisible({ timeout: 20_000 });
    await expect(snapshotStrip(page).getByTestId("stock-beta-snapshot-universe")).toContainText(
      "1",
      {
        timeout: 20_000,
      },
    );
    await expect(row.getByRole("link", { name: "Open detail" })).toBeVisible();
    await expect(observed.some((url) => url.pathname.includes("equity-universe-v2"))).toBeTruthy();
    expectProviderFree(observed);

    await row.getByRole("link", { name: "Open detail" }).click();
    await expect(page).toHaveURL(/\/stock-beta\/005930\.KRX$/);
    await expect(detailInstrumentRegion(page, "005930.KRX")).toBeVisible();
    await expectStockShell(page);
  });

  test("retries a typed failure and polls the membership to READY", async ({ page, request }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 31,
      stockBetaSeed: "failed",
    });

    await page.goto("/stock-beta");
    const card = membershipCard(page, "000001.KRX");
    await expect(card).toHaveAttribute("data-lifecycle", "FAILED");
    await expect(card).toContainText("OWNER_EQUITY_BACKFILL_RETRYABLE");
    await card.getByRole("button", { name: "Retry" }).click();
    await expect(card).toHaveAttribute("data-lifecycle", "READY", { timeout: 20_000 });
    await expect(card).not.toContainText("OWNER_EQUITY_BACKFILL_RETRYABLE");
    await expect(rankedRow(page, "000001.KRX")).toBeVisible({ timeout: 20_000 });
    await expect(snapshotStrip(page)).toBeVisible({ timeout: 20_000 });
  });

  test("removes a disabled row and stale snapshot signal immediately", async ({
    page,
    request,
  }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 31,
      stockBetaSeed: "ready",
    });

    await page.goto("/stock-beta");
    const card = membershipCard(page, "000001.KRX");
    const oldRow = rankedRow(page, "000001.KRX");
    await expect(oldRow).toBeVisible();
    await card.getByRole("button", { name: "Disable" }).click();
    const confirm = card.getByRole("button", { name: "Confirm disable" });
    await expect(confirm).toBeFocused();
    await confirm.click();

    await expect(oldRow).toHaveCount(0);
    await expect(page.getByTestId("stock-beta-snapshot-strip")).toHaveCount(0);
    await expect(card).toHaveAttribute("data-lifecycle", "DISABLED", { timeout: 20_000 });
    await expect(rankedRow(page, "000001.KRX")).toHaveCount(0);
    await expect(snapshotStrip(page).getByTestId("stock-beta-snapshot-universe")).toContainText(
      "30",
      {
        timeout: 20_000,
      },
    );
  });

  test("keeps typed latest-unavailable and integrity failures free of stale signals", async ({
    page,
    request,
  }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 31,
      stockBetaSeed: "ready",
      stockBeta: "unavailable",
    });
    await page.goto("/stock-beta");
    await expect(page.getByRole("heading", { name: "Signal snapshot unavailable" })).toBeVisible();
    await expectNoSignalWidgets(page);
    await expect(membershipCards(page)).toHaveCount(31);

    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 31,
      stockBetaSeed: "ready",
      stockBeta: "integrity",
    });
    await page.goto("/stock-beta");
    await expect(
      dashboardRegion(page, "Latest V2 signals").getByText(
        "Request failed with typed code OWNER_EQUITY_INTEGRITY_FAILED.",
      ),
    ).toBeVisible();
    await expectNoSignalWidgets(page);
    await expect(membershipCards(page)).toHaveCount(31);
    await expect(page.getByLabel("KRX code")).toBeVisible();
  });

  test("renders a typed detail not-found state without signal data", async ({ page, request }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 31,
      stockBetaSeed: "ready",
    });

    await page.goto("/stock-beta/999999.KRX");
    await expectStockShell(page);
    await expect(page.getByRole("heading", { name: "Instrument signal not found" })).toBeVisible();
    await expect(page.getByTestId("stock-beta-detail-board")).toHaveCount(0);
    await expect(page.locator('[data-terminal-utility-content="stock-beta"]')).toHaveCount(0);
  });

  test("enforces owner-only redirect and forbidden boundaries", async ({ page, request }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "member",
      stockBetaRows: 31,
      stockBetaSeed: "ready",
    });

    await page.goto("/stock-beta");
    await expect(page.getByRole("alert", { name: "Owner access required" })).toBeVisible();
    await expect(page.getByTestId("stock-beta-rank-table")).toHaveCount(0);
    await page.goto("/stock-beta/000001.KRX");
    await expect(page.getByRole("alert", { name: "Owner access required" })).toBeVisible();
    await expect(page.getByTestId("stock-beta-detail-board")).toHaveCount(0);

    await resetScenario(request, {
      authSession: "expired",
      role: "owner",
      stockBetaRows: 31,
      stockBetaSeed: "ready",
    });
    await page.goto("/stock-beta");
    await expect.poll(() => new URL(page.url()).pathname, { timeout: 10_000 }).toBe("/auth/login");
  });

  test("keeps the terminal shell continuous through dashboard, detail, and navigation", async ({
    page,
    request,
  }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 31,
      stockBetaSeed: "ready",
    });

    await page.goto("/stock-beta");
    await expectStockShell(page);
    await rankedRow(page, "000001.KRX").getByRole("link", { name: "Open detail" }).click();
    await expect(detailInstrumentRegion(page, "000001.KRX")).toBeVisible();
    await expectStockShell(page);
    await page.getByRole("link", { name: "Back to stock signal beta" }).click();
    await expect(rankedSignalsTable(page)).toBeVisible();
    await expectStockShell(page);

    await page.goto("/strategies");
    await expect(page.locator('[data-shell="research-terminal"]')).toHaveCount(1);
    await expect(page.locator('[data-shell="stock-beta-terminal"]')).toHaveCount(0);
    await expect(page.locator('[data-terminal-utility-bar="research"]')).toHaveCount(1);
  });

  test("supports instrument search, slash/Escape, matrix selection, and keyboard activation", async ({
    page,
    request,
  }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 31,
      stockBetaSeed: "ready",
    });

    await page.goto("/stock-beta");
    const search = page.locator("#stock-beta-instrument-search-input");
    await expect(search).toHaveAccessibleName("Search signals");
    await page.locator("body").press("/");
    await expect(search).toBeFocused();
    await search.fill("000004");
    await expect(rankedRow(page, "000004.KRX")).toBeVisible();
    await expect(rankedSignalsTable(page).locator("tbody tr")).toHaveCount(1);
    await search.press("Escape");
    await expect(search).toHaveValue("");
    await expect(rankedSignalsTable(page).locator("tbody tr")).toHaveCount(31);

    const matrixTile = dashboardRegion(page, "Condition matrix").getByTestId(
      "stock-beta-matrix-000004.KRX",
    );
    await matrixTile.focus();
    await expect(matrixTile).toBeFocused();
    await matrixTile.press("Space");
    await expect(matrixTile).toHaveAttribute("aria-pressed", "true");
    await expect(rankedRow(page, "000004.KRX")).toHaveAttribute("data-selected", "true");

    const secondRow = rankedRow(page, "000002.KRX");
    const secondSelect = secondRow.getByRole("button", {
      name: "Select signal: 000002.KRX",
    });
    await secondSelect.focus();
    await secondSelect.press("Enter");
    await expect(secondRow).toHaveAttribute("data-selected", "true");
    await expect(signalPreview(page)).toHaveAttribute("data-selected-instrument", "000002.KRX");
  });

  test("renders the deterministic 1Y OHLCV chart, latest price, and every range", async ({
    page,
    request,
  }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 1,
      stockBetaSeed: "ready",
      stockBetaChartFreshness: "CURRENT",
    });

    await page.goto("/stock-beta");
    const chart = priceChart(page);
    await expect(chart).toBeVisible();
    await expect(priceChartPart(page, "stock-beta-price-chart-surface")).toBeVisible();
    const initialCanvas = await expectCanvasReady(chart);
    expect(initialCanvas.canvasCount).toBeGreaterThan(0);
    await expect(priceChartPart(page, "stock-beta-price-chart-reset")).toHaveAccessibleName(
      "Reset zoom",
    );
    const legend = priceChartPart(page, "stock-beta-price-chart-legend");
    await expect(legend).toBeVisible();
    await expect(legend.locator("li").filter({ hasText: "20-session moving average" })).toHaveCount(
      1,
    );
    await expect(legend.locator("li").filter({ hasText: "60-session moving average" })).toHaveCount(
      1,
    );

    const profile = signalProfilePanel(page);
    const latestStrip = profile.locator("dl").first();
    await expect(latestStrip).toContainText("Latest EOD close");
    await expect(profile.locator("dl")).toContainText(chartLatestClose);
    await expect(profile.locator("dl")).toContainText(chartLatestChange);
    await expect(profile.locator("dl")).toContainText(chartLatestRate);
    await expect(profile.locator("dl")).toContainText(chartLatestVolume);
    await expect(profile.locator("dl")).toContainText(chartAsOf);
    expect(await selectedChartDate(priceChartPart(page, "stock-beta-price-chart-summary"))).toBe(
      chartAsOf,
    );
    await expectTooltipMatchesBar(chart, {
      session_date: chartAsOf,
      open: 12_045,
      high: 12_140,
      low: 12_025,
      close: 12_120,
      volume: 106_000,
      sma_20: null,
      sma_60: null,
    });

    const expectedRangeCounts = {
      "1m": 24,
      "3m": 67,
      "6m": 130,
      "1y": 261,
    } as const;
    const rangePayloads = new Map<string, SyntheticChartPayload>();
    const snapshotId = snapshotIdFromText(await snapshotStrip(page).textContent());
    for (const [label, range] of [
      ["1M", "1m"],
      ["3M", "3m"],
      ["6M", "6m"],
      ["1Y", "1y"],
    ] as const) {
      const rangeButton = page.getByRole("button", { name: label, exact: true });
      let payload: SyntheticChartPayload;
      if (range === "1y") {
        await rangeButton.click();
        await expect(rangeButton).toHaveAttribute("aria-pressed", "true");
        payload = await fetchServerSeededChartPayload(request, snapshotId, range);
      } else {
        const payloadPromise = waitForChartPayload(page, range);
        await rangeButton.click();
        await expect(rangeButton).toHaveAttribute("aria-pressed", "true");
        payload = await payloadPromise;
      }
      rangePayloads.set(range, payload);
      expect(payload.bars).toHaveLength(expectedRangeCounts[range]);
      expect(payload.bars.at(-1)?.session_date).toBe(chartAsOf);
      expect(
        payload.bars.every(
          (bar, index, bars) =>
            index === 0 || (bars[index - 1]?.session_date ?? "") < bar.session_date,
        ),
      ).toBeTruthy();
      await expect(chart).toHaveAttribute("data-renderer-status", "ready");
    }
    const oneYear = rangePayloads.get("1y");
    if (oneYear === undefined) throw new Error("Expected the 1Y chart response");
    expect(oneYear?.bars[0]?.session_date).toBe(chartFirstSession);
    expect(oneYear?.bars[0]).toMatchObject({
      close: 10_100,
      high: 10_120,
      low: 10_080,
      open: 10_100,
      volume: 0,
    });
    expect(oneYear?.bars.at(-1)).toMatchObject({
      close: 12_120,
      high: 12_140,
      low: 12_025,
      open: 12_045,
      session_date: chartAsOf,
      volume: 106_000,
    });

    const surface = priceChartPart(page, "stock-beta-price-chart-surface");
    await surface.focus();
    await surface.press("Home");
    expect(await selectedChartDate(priceChartPart(page, "stock-beta-price-chart-summary"))).toBe(
      chartFirstSession,
    );
    await expectTooltipMatchesBar(chart, requiredChartBar(oneYear, 0));
    await surface.press("End");
    expect(await selectedChartDate(priceChartPart(page, "stock-beta-price-chart-summary"))).toBe(
      chartAsOf,
    );
  });

  test("keeps an internal SMA null interval in the chart response and rendered Canvas", async ({
    page,
    request,
  }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 1,
      stockBetaSeed: "ready",
      stockBetaChartSmaNullGap: true,
    });
    await page.goto("/stock-beta");
    const threeMonthResponse = waitForChartPayload(page, "3m");
    await page.getByRole("button", { name: "3M", exact: true }).click();
    await threeMonthResponse;
    await page.getByRole("button", { name: "1Y", exact: true }).click();
    const payload = await fetchServerSeededChartPayload(
      request,
      snapshotIdFromText(await snapshotStrip(page).textContent()),
      "1y",
    );

    expect(payload.bars).toHaveLength(261);
    expect(payload.bars[119]?.sma_20).not.toBeNull();
    expect(payload.bars.slice(120, 125).map((bar) => bar.sma_20)).toEqual([
      null,
      null,
      null,
      null,
      null,
    ]);
    expect(payload.bars[125]?.sma_20).not.toBeNull();
    await expectCanvasReady(priceChart(page));
  });

  test("selects a chart from both a ranked row and filtered search result", async ({
    page,
    request,
  }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 4,
      stockBetaSeed: "ready",
    });

    await page.goto("/stock-beta");
    const rowTwo = rankedRow(page, "000002.KRX");
    await rowTwo.getByRole("button", { name: "Select signal: 000002.KRX" }).click();
    await expect(rowTwo).toHaveAttribute("data-selected", "true");
    await expect(signalPreview(page)).toHaveAttribute("data-selected-instrument", "000002.KRX");

    const search = page.locator("#stock-beta-instrument-search-input");
    await search.fill("000004");
    const rowFour = rankedRow(page, "000004.KRX");
    await expect(rowFour).toBeVisible();
    await rowFour.getByRole("button", { name: "Select signal: 000004.KRX" }).click();
    await expect(rowFour).toHaveAttribute("data-selected", "true");
    await expect(signalPreview(page)).toHaveAttribute("data-selected-instrument", "000004.KRX");
    await expect(priceChart(page)).toBeVisible();
  });

  test("discards a late A chart response after selecting B", async ({ page, request }) => {
    const observed = observeBrowserRequests(page);
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 2,
      stockBetaSeed: "ready",
      stockBetaSeedCode: "000002",
      stockBetaChartDelays: { "000001.KRX:1y": 350 },
    });

    await page.goto("/stock-beta");
    const rowA = rankedRow(page, "000001.KRX");
    const rowB = rankedRow(page, "000002.KRX");
    const lateA = page.waitForRequest(
      (request) =>
        request.url().includes("000001.KRX/chart") &&
        new URL(request.url()).searchParams.get("range") === "1y",
    );
    await rowA.getByRole("button", { name: "Select signal: 000001.KRX" }).click();
    await lateA;
    await expect(rowA).toHaveAttribute("data-selected", "true");

    await rowB.getByRole("button", { name: "Select signal: 000002.KRX" }).click();
    await expect(rowB).toHaveAttribute("data-selected", "true");
    await expect(signalPreview(page)).toHaveAttribute("data-selected-instrument", "000002.KRX");
    const chart = priceChart(page);
    const surface = priceChartPart(page, "stock-beta-price-chart-surface");
    await expect(chart).toBeVisible();
    await surface.press("End");
    await expect(priceChartPart(page, "stock-beta-price-chart-summary")).toContainText("12,220");
    expect(observed.some((url) => url.pathname.includes("000001.KRX/chart"))).toBeTruthy();
  });

  test("keeps the previous range visible during delayed loading and rejects a stale response after a rapid change", async ({
    page,
    request,
  }) => {
    const observed = observeBrowserRequests(page);
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 1,
      stockBetaSeed: "ready",
      stockBetaChartDelays: { "000001.KRX:1m": 2_000 },
    });

    await page.goto("/stock-beta");
    const profile = signalProfilePanel(page);
    const chart = profile.getByTestId("stock-beta-price-chart");
    const oneMonthRangeButton = profile.getByRole("button", { name: "1M", exact: true });
    const threeMonthRangeButton = profile.getByRole("button", { name: "3M", exact: true });
    const oneYearRangeButton = profile.getByRole("button", { name: "1Y", exact: true });
    await expect(chart).toHaveCount(1);
    await expect(chart).toBeVisible();
    const previousCanvas = await expectCanvasReady(chart);
    await expect(oneYearRangeButton).toHaveAttribute("aria-pressed", "true");
    const lateOneMonth = page.waitForRequest(
      (request) =>
        request.url().endsWith("/chart?range=1m") ||
        (request.url().includes("/chart?") &&
          new URL(request.url()).searchParams.get("range") === "1m"),
    );
    await oneMonthRangeButton.click();
    await lateOneMonth;

    await expect(chart).toBeVisible();
    await expect(chart).toHaveAttribute("data-renderer-status", "ready");
    expect(
      await canvasPaintEvidence(priceChartPart(page, "stock-beta-price-chart-canvas")),
    ).toEqual(previousCanvas);
    expect(await selectedChartDate(priceChartPart(page, "stock-beta-price-chart-summary"))).toBe(
      chartAsOf,
    );
    await expect(oneYearRangeButton).toHaveAttribute("aria-pressed", "true");
    await expect(oneMonthRangeButton).toHaveAttribute("aria-pressed", "false");
    await expect(profile.getByText("Updating EOD chart…", { exact: true })).toBeVisible();

    const threeMonth = waitForChartPayload(page, "3m");
    await threeMonthRangeButton.click();
    const threeMonthPayload = await threeMonth;

    await expect(threeMonthRangeButton).toHaveAttribute("aria-pressed", "true");
    await expect(oneMonthRangeButton).toHaveAttribute("aria-pressed", "false");
    await expect(oneYearRangeButton).toHaveAttribute("aria-pressed", "false");
    expect(threeMonthPayload.bars).toHaveLength(67);
    await expect(chart).toHaveAttribute("data-renderer-status", "ready");
    expect(chartRequests(observed, "000001.KRX", "1m").length).toBeGreaterThan(0);
    expect(chartRequests(observed, "000001.KRX", "3m").length).toBeGreaterThan(0);
  });

  test("pins the server-seeded chart and range request to the newly displayed snapshot after refresh", async ({
    page,
    request,
  }, testInfo) => {
    type RefreshRecord = {
      requestId: number;
      status: number | null;
      body: "not-read" | "reading" | "complete" | "response-missing" | "read-failed";
      initialChart:
        | "unobserved"
        | "missing"
        | "null"
        | "seed"
        | "invalid-seed-id"
        | "present-unmatched";
      initialChartError: "unobserved" | "missing" | "null" | "present";
      seededSnapshotId: string | null;
      failure: "none" | "aborted" | "other";
    };
    const clockOrigin = { unixMs: Date.now(), monotonicMs: performance.now() };
    const elapsedMs = () =>
      Math.round((performance.now() - clockOrigin.monotonicMs) * 1_000) / 1_000;
    const refreshes = new Map<Request, RefreshRecord>();
    const events: Array<{
      sequence: number;
      atMs: number;
      requestId: number | null;
      kind: string;
    }> = [];
    let requestSequence = 0;
    let eventSequence = 0;
    let droppedRequests = 0;
    let droppedEvents = 0;
    let observationClosed = false;
    let listenersInstalled = false;
    let acceptanceCompleted = false;
    let initialSnapshotId: string | null = null;
    let capturedSnapshotId: string | null = null;
    const accepted = {
      snapshotId: "",
      seed: null as ReturnType<typeof readCommittedStockBetaChart>,
    };
    let lastObservedSeed: ReturnType<typeof readCommittedStockBetaChart> = null;
    let intentionalRangeRequests = 0;
    const recordEvent = (kind: string, record?: RefreshRecord) => {
      if (observationClosed) return;
      const sequence = ++eventSequence;
      if (events.length < 128) {
        events.push({ sequence, atMs: elapsedMs(), requestId: record?.requestId ?? null, kind });
      } else {
        droppedEvents += 1;
      }
    };
    const isRefresh = (candidate: Request) => {
      const url = new URL(candidate.url());
      return (
        url.pathname === "/stock-beta" &&
        url.searchParams.has("_rsc") &&
        candidate.method() === "GET" &&
        candidate.headers()["next-router-prefetch"] === undefined
      );
    };
    const onRequest = (candidate: Request) => {
      const url = new URL(candidate.url());
      if (
        candidate.method() === "GET" &&
        url.pathname === `${chartPathFragment}000001.KRX/chart` &&
        url.searchParams.get("range") === "1m"
      ) {
        intentionalRangeRequests += 1;
        recordEvent("intentional-range-request");
      }
      if (!isRefresh(candidate)) return;
      const requestId = ++requestSequence;
      if (refreshes.size >= 16) {
        droppedRequests += 1;
        return;
      }
      const record: RefreshRecord = {
        requestId,
        status: null,
        body: "not-read",
        initialChart: "unobserved",
        initialChartError: "unobserved",
        seededSnapshotId: null,
        failure: "none",
      };
      refreshes.set(candidate, record);
      recordEvent("request", record);
    };
    const onResponse = (response: Response) => {
      const record = refreshes.get(response.request());
      if (record === undefined) return;
      record.status = response.status();
      recordEvent("response", record);
    };
    const onFinished = (candidate: Request) => {
      if (!isRefresh(candidate)) return;
      const record = refreshes.get(candidate);
      recordEvent("finished", record);
      const completion = (async () => {
        try {
          const response = await candidate.response();
          if (response === null) {
            if (record !== undefined) record.body = "response-missing";
            throw new Error("The completed refresh has no response");
          }
          if (record !== undefined) {
            record.status = response.status();
            record.body = "reading";
          }
          recordEvent("body-read-started", record);
          const flight = await response.text();
          // Optional completed-body diagnostic, anchored to the actual initialChart field.
          const seededSnapshotId = flight.match(/"initialChart":\{"snapshot_id":"([^"]+)"/)?.[1];
          if (record !== undefined) {
            record.body = "complete";
            // Only a complete UUID may enter the attachment; never free-form Flight text.
            const safeId = seededSnapshotId?.match(
              /^[0-9a-f]{8}-[0-9a-f]{4}-[4-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i,
            )?.[0];
            record.seededSnapshotId = safeId ?? null;
            record.initialChart = !/"initialChart"\s*:/.test(flight)
              ? "missing"
              : /"initialChart"\s*:\s*null\b/.test(flight)
                ? "null"
                : seededSnapshotId === undefined
                  ? "present-unmatched"
                  : safeId === undefined
                    ? "invalid-seed-id"
                    : "seed";
            record.initialChartError = !/"initialChartError"\s*:/.test(flight)
              ? "missing"
              : /"initialChartError"\s*:\s*null\b/.test(flight)
                ? "null"
                : "present";
          }
          recordEvent("body-read-complete", record);
          return { status: response.status(), seededSnapshotId };
        } catch (error) {
          if (record !== undefined && record.body !== "response-missing")
            record.body = "read-failed";
          recordEvent("body-read-failed", record);
          throw error; // Preserve this diagnostic read failure in the lifecycle record.
        }
      })();
      // The committed-seed assertion is independent of this optional body read.
      void completion.catch(() => undefined);
    };
    const onFailed = (candidate: Request) => {
      const record = refreshes.get(candidate);
      if (record === undefined) return;
      record.failure = candidate.failure()?.errorText === "net::ERR_ABORTED" ? "aborted" : "other";
      recordEvent("failed", record);
    };

    try {
      await resetScenario(request, {
        authSession: "valid",
        role: "owner",
        stockBetaRows: 1,
        stockBetaSeed: "ready",
      });
      await page.goto("/stock-beta");
      // Select the accessible Price surface, excluding Next's hidden streaming template.
      const chart = signalProfilePanel(page).getByTestId("stock-beta-price-chart");
      await expect(chart).toHaveCount(1);
      await expectCanvasReady(chart);
      const oldSnapshotId = snapshotIdFromText(await snapshotStrip(page).textContent());
      initialSnapshotId = oldSnapshotId;

      await resetScenario(request, {
        authSession: "valid",
        role: "owner",
        stockBetaRows: 2,
        stockBetaSeed: "ready",
      });
      await page.getByLabel("KRX code").fill("005930");
      // READY does not establish RSC completion. Observe request identity without
      // changing requests or the application's response consumption.
      page.on("request", onRequest);
      page.on("response", onResponse);
      page.on("requestfinished", onFinished);
      page.on("requestfailed", onFailed);
      listenersInstalled = true;
      recordEvent("listeners-installed");
      await page.getByRole("button", { name: "Add instrument" }).click();
      await expect(membershipCard(page, "005930.KRX")).toHaveAttribute("data-lifecycle", "READY", {
        timeout: 20_000,
      });
      recordEvent("membership-ready-observed");
      // Chromium can report RSC requestfailed after the app reader reached EOF.
      // Observe the decoded AND applied server prop instead of requiring the
      // observer's requestfinished event. An equal client fallback is insufficient.
      await expect
        .poll(async () => {
          if ((await chart.count()) !== 1) return false;
          const displayedId = snapshotIdFromText(await snapshotStrip(page).textContent());
          if (displayedId === oldSnapshotId) return false;
          const seed = await chart.evaluate(readCommittedStockBetaChart);
          lastObservedSeed = seed;
          if (
            !isCurrentServerSeed(seed, {
              snapshotId: displayedId,
              instrumentId: "000001.KRX",
              generation: 1,
              range: "1y",
              asOf: chartAsOf,
            })
          )
            return false;
          // All observed non-prefetch refresh responses must be HTTP 200. Next's
          // fetchServerResponse also rejects non-OK responses before applying props.
          if (
            refreshes.size === 0 ||
            [...refreshes.values()].some((response) => response.status !== 200)
          )
            return false;
          accepted.snapshotId = displayedId;
          accepted.seed = seed;
          return true;
        })
        .toBe(true);
      const newSnapshotId = accepted.snapshotId;
      capturedSnapshotId = newSnapshotId;
      recordEvent("current-server-seed-applied");
      expect(newSnapshotId).not.toBe(oldSnapshotId);
      const refreshedResponse = {
        status: [...refreshes.values()].every((response) => response.status === 200) ? 200 : null,
        seededSnapshotId: accepted.seed?.seed?.snapshotId,
      };
      expect(refreshedResponse?.status).toBe(200);
      expect(refreshedResponse?.seededSnapshotId).toBe(newSnapshotId);
      await expect(chart).toHaveCount(1);
      await expectCanvasReady(chart);
      await expect(
        signalProfilePanel(page).getByRole("button", { name: "1Y", exact: true }),
      ).toHaveAttribute("aria-pressed", "true");
      expect(snapshotIdFromText(await snapshotStrip(page).textContent())).toBe(newSnapshotId);

      // The default chart comes from the server seed. An intentional range change
      // still proves that the browser uses the newly displayed snapshot binding.
      const rangeRequest = page.waitForRequest((request) => {
        const url = new URL(request.url());
        return (
          request.method() === "GET" &&
          url.pathname.includes(`${chartPathFragment}000001.KRX/chart`) &&
          url.searchParams.get("range") === "1m"
        );
      });
      const oneMonthButton = signalProfilePanel(page).getByRole("button", {
        name: "1M",
        exact: true,
      });
      await oneMonthButton.click();
      const requestedRange = await rangeRequest;
      expect(new URL(requestedRange.url()).searchParams.get("snapshot_id")).toBe(newSnapshotId);
      const rangeResponse = await requestedRange.response();
      expect(rangeResponse).not.toBeNull();
      if (rangeResponse === null) throw new Error("The intentional 1M request has no response");
      expect(rangeResponse.request()).toBe(requestedRange);
      expect(rangeResponse.status()).toBe(200);
      const payload = await rangeResponse.json();
      expect(payload.snapshot_id).toBe(newSnapshotId);
      expect(payload.range).toBe("1m");
      expect(payload.generation).toBe(1);
      expect(payload.as_of).toBe(chartAsOf);
      expect(payload.instrument_id).toBe("000001.KRX");
      expect(payload.bars).toHaveLength(24);
      await expect(oneMonthButton).toHaveAttribute("aria-pressed", "true");
      await expectCanvasReady(chart);
      await expect
        .poll(() => chart.evaluate(readCommittedStockBetaChart))
        .toMatchObject({
          chart: {
            snapshotId: newSnapshotId,
            instrumentId: "000001.KRX",
            generation: 1,
            range: "1m",
            asOf: chartAsOf,
          },
          chartState: "ready",
          chartErrorNull: true,
          selectedRange: "1m",
        });
      expect(intentionalRangeRequests).toBe(1);
      expect(snapshotIdFromText(await snapshotStrip(page).textContent())).toBe(newSnapshotId);
      acceptanceCompleted = true;
    } finally {
      recordEvent("test-body-ended");
      page.off("request", onRequest);
      page.off("response", onResponse);
      page.off("requestfinished", onFinished);
      page.off("requestfailed", onFailed);
      observationClosed = true;
      // Freeze before any await. Every recorded failure precedes fixture teardown;
      // cancellation after this boundary remains unobserved, not successful.
      const summary = {
        clockOrigin,
        observationClosedAtMs: elapsedMs(),
        boundary: "test-body-finally-before-fixture-teardown",
        listenersInstalled,
        acceptanceCompleted,
        initialSnapshotId,
        capturedSnapshotId,
        acceptedSeed: accepted.seed,
        lastObservedSeed,
        intentionalRangeRequests,
        droppedRequests,
        droppedEvents,
        requests: Array.from(refreshes.values(), (record) => ({ ...record })),
        events: events.slice(),
      };
      const displayedAtClose = await snapshotStrip(page)
        .evaluateAll((elements) => ({
          count: elements.length,
          snapshotIds: elements
            .slice(0, 2)
            .map(
              (element) =>
                element.textContent?.match(
                  /[0-9a-f]{8}-[0-9a-f]{4}-[4-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}/i,
                )?.[0] ?? null,
            ),
        }))
        .catch(() => null);
      await testInfo.attach("stock-beta-chart-refresh-lifecycle", {
        body: Buffer.from(JSON.stringify({ ...summary, displayedAtClose }, null, 2)),
        contentType: "application/json",
      });
    }
  });

  test("updates the tooltip and live summary for pointer and keyboard selection", async ({
    page,
    request,
  }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 1,
      stockBetaSeed: "ready",
    });
    await page.goto("/stock-beta");
    const oneYear = await fetchServerSeededChartPayload(
      request,
      snapshotIdFromText(await snapshotStrip(page).textContent()),
      "1y",
    );
    const chart = priceChart(page);
    await expectCanvasReady(chart);
    const surface = priceChartPart(page, "stock-beta-price-chart-surface");
    const summary = priceChartPart(page, "stock-beta-price-chart-summary");
    await expectPointerBoundary(chart, oneYear, "first");
    await expectPointerBoundary(chart, oneYear, "latest");
    await moveToPlotObservation(chart, 0.48);
    await expect.poll(() => selectedChartDate(summary)).not.toBe(chartAsOf);
    const pointerDate = await selectedChartDate(summary);
    expect(pointerDate).not.toBeNull();
    const pointerBar = oneYear.bars.find((bar) => bar.session_date === pointerDate);
    expect(pointerBar, `pointer selection ${pointerDate}`).toBeDefined();
    if (pointerBar === undefined) throw new Error(`Expected chart observation for ${pointerDate}`);
    await expectTooltipMatchesBar(chart, pointerBar);
    const pointerIndex = oneYear.bars.findIndex((bar) => bar.session_date === pointerDate);
    await expect(chart).toHaveAttribute("data-selected-index", String(pointerIndex));
    await expect(priceChartPart(page, "stock-beta-price-chart-tooltip")).toBeVisible();

    await surface.focus();
    await surface.press("Home");
    const firstDate = await selectedChartDate(summary);
    expect(firstDate).toBe(chartFirstSession);
    await expectTooltipMatchesBar(chart, requiredChartBar(oneYear, 0));
    await expect(chart).toHaveAttribute("data-selected-index", "0");
    await surface.press("ArrowRight");
    const secondDate = await selectedChartDate(summary);
    expect(secondDate).not.toBe(firstDate);
    await expectTooltipMatchesBar(chart, requiredChartBar(oneYear, 1));
    await surface.press("ArrowLeft");
    await expect.poll(() => selectedChartDate(summary)).toBe(chartFirstSession);
    await surface.press("End");
    await expect.poll(() => selectedChartDate(summary)).toBe(chartAsOf);
    await expectTooltipMatchesBar(chart, requiredChartBar(oneYear, oneYear.bars.length - 1));
    await expect(chart).toHaveAttribute("data-selected-index", String(oneYear.bars.length - 1));
  });

  test("uses aligned Canvas panes for zoom, pan, and reset without requesting more bars", async ({
    page,
    request,
  }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 1,
      stockBetaSeed: "ready",
    });
    await page.goto("/stock-beta");
    const oneYear = await fetchServerSeededChartPayload(
      request,
      snapshotIdFromText(await snapshotStrip(page).textContent()),
      "1y",
    );

    const chart = priceChart(page);
    const surface = priceChartPart(page, "stock-beta-price-chart-surface");
    const summary = priceChartPart(page, "stock-beta-price-chart-summary");
    const canvas = await expectCanvasReady(chart);
    expect(canvas.plotPanes).toHaveLength(2);
    const [pricePane, volumePane] = canvas.plotPanes;
    if (pricePane === undefined || volumePane === undefined) {
      throw new Error("Expected a price pane and a volume pane");
    }
    expect(Math.abs(pricePane.left - volumePane.left)).toBeLessThanOrEqual(1);
    expect(Math.abs(pricePane.width - volumePane.width)).toBeLessThanOrEqual(1);
    const requestsBeforeGestures = chartRequests(requestsFor(page)).length;
    const moveToObservationAt = async (fraction: number) => {
      await moveToPlotObservation(chart, fraction);
      await expect.poll(() => selectedChartDate(summary)).toMatch(/^\d{4}-\d{2}-\d{2}$/);
      return selectedChartDate(summary);
    };

    const fullRangeDate = await moveToObservationAt(0.42);
    expect(oneYear.bars.some((bar) => bar.session_date === fullRangeDate)).toBeTruthy();
    const zoomAnchor = await visiblePlotTarget(chart, 0.68);
    await page.mouse.move(zoomAnchor.x, zoomAnchor.y);
    await page.mouse.wheel(0, -480);
    await expect.poll(() => moveToObservationAt(0.42)).not.toBe(fullRangeDate);
    const zoomedDate = await moveToObservationAt(0.42);
    expect(zoomedDate).not.toBe(fullRangeDate);
    expect(oneYear.bars.some((bar) => bar.session_date === zoomedDate)).toBeTruthy();

    const dragStart = await visiblePlotTarget(chart, 0.68);
    const dragEnd = await visiblePlotTarget(chart, 0.4);
    await page.mouse.move(dragStart.x, dragStart.y);
    await page.mouse.down();
    await page.mouse.move(dragEnd.x, dragEnd.y, { steps: 8 });
    await page.mouse.up();
    await expect.poll(() => moveToObservationAt(0.42)).not.toBe(zoomedDate);
    const pannedDate = await moveToObservationAt(0.42);
    expect(pannedDate).not.toBe(zoomedDate);
    expect(oneYear.bars.some((bar) => bar.session_date === pannedDate)).toBeTruthy();

    await chart.getByTestId("stock-beta-price-chart-reset").click();
    await expect.poll(() => moveToObservationAt(0.42)).toBe(fullRangeDate);
    const resetDate = await moveToObservationAt(0.42);
    expect(resetDate).toBe(fullRangeDate);
    await expect(surface).toBeVisible();
    expect(chartRequests(requestsFor(page))).toHaveLength(requestsBeforeGestures);
  });

  // Isolated real-component import failure and data-bearing no-JS checks are prepared in
  // /tmp/wp3-lightweight-harness/{import-failure,ssr-check}.mjs, with commands in its README.
  // Blocking a Next entry chunk prevents workspace hydration and cannot isolate the renderer.
  // Route unavailable/integrity/error-removal coverage remains in this file.
  test("preserves the existing authenticated loading shell with JavaScript disabled", async ({
    browser,
    request,
  }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 1,
      stockBetaSeed: "ready",
    });
    const context = await browser.newContext({ baseURL: appOrigin, javaScriptEnabled: false });
    const page = await context.newPage();
    observeBrowserRequests(page);
    await installProviderFreeNetworkGuard(page);
    try {
      await page.goto("/stock-beta");
      await expect(page.getByRole("status", { name: "Loading workspace" })).toBeVisible();
      await expect(page.getByRole("heading", { name: "Loading workspace" })).toBeVisible();
      await expect(page.getByTestId("stock-beta-price-chart")).not.toBeVisible();
      await expect(page.locator("canvas")).toHaveCount(0);
    } finally {
      expectProviderFree(requestsFor(page));
      expect(observedPageErrors.get(page) ?? [], "browser page errors without JavaScript").toEqual(
        [],
      );
      await context.close();
    }
  });

  test("keeps a single renderer across profile tabs and applies Canvas forced-colors", async ({
    page,
    request,
  }, testInfo) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 1,
      stockBetaSeed: "ready",
    });
    await page.goto("/stock-beta");
    const chart = priceChart(page);
    const host = priceChartPart(page, "stock-beta-price-chart-canvas");
    await expect(chart).toHaveCount(1);
    const initial = await expectCanvasReady(chart);
    const reset = priceChartPart(page, "stock-beta-price-chart-reset");
    await expect(reset).toBeVisible();
    await expect(reset).toHaveAccessibleName("Reset zoom");
    await expect(priceChartPart(page, "stock-beta-price-chart-legend")).toContainText(
      "20-session moving average",
    );
    await expect(priceChartPart(page, "stock-beta-price-chart-attribution")).toContainText(
      "Chart attribution",
    );
    await expect(priceChartPart(page, "stock-beta-price-chart-attribution")).toContainText(
      "TradingView Lightweight Charts",
    );
    await expect(
      priceChartPart(page, "stock-beta-price-chart-attribution").getByRole("link", {
        name: "NOTICE",
      }),
    ).toHaveAttribute("href", "/licenses/lightweight-charts/NOTICE.txt");
    await expect(
      priceChartPart(page, "stock-beta-price-chart-attribution").getByRole("link", {
        name: "LICENSE",
      }),
    ).toHaveAttribute("href", "/licenses/lightweight-charts/LICENSE.txt");
    const notice = await request.get(`${appOrigin}/licenses/lightweight-charts/NOTICE.txt`);
    const license = await request.get(`${appOrigin}/licenses/lightweight-charts/LICENSE.txt`);
    expect(notice.status()).toBe(200);
    expect(license.status()).toBe(200);
    expect(await notice.text()).toContain("TradingView");
    expect(await license.text()).toContain("Apache License");

    const initialPalette = await chartCanvasPalette(host);
    expect(initialPalette.length).toBeGreaterThan(5);
    for (const name of ["Returns", "Price", "Volatility", "Price"] as const) {
      const tab = page.getByRole("tab", { name, exact: true });
      await tab.click();
      await expect(tab).toHaveAttribute("aria-selected", "true");
      if (name !== "Price") {
        expect(await chart.count()).toBeLessThanOrEqual(1);
        continue;
      }
      await expectCanvasReady(chart);
      expect(
        await priceChartPart(page, "stock-beta-price-chart-canvas").locator("canvas").count(),
      ).toBe(initial.canvasCount);
    }

    await page.emulateMedia({ forcedColors: "active" });
    await expect
      .poll(() => page.evaluate(() => window.matchMedia("(forced-colors: active)").matches))
      .toBeTruthy();
    await expect
      .poll(async () => JSON.stringify(await chartCanvasPalette(host)))
      .not.toBe(JSON.stringify(initialPalette));
    await testInfo.attach("stock-beta-chart-forced-colors-palettes", {
      body: JSON.stringify(
        { normal: initialPalette, forced: await chartCanvasPalette(host) },
        null,
        2,
      ),
      contentType: "application/json",
    });
    const forcedSurface = chart.getByTestId("stock-beta-price-chart-surface");
    await expectUnclipped(forcedSurface, "forced-colors Canvas surface");
    await forcedSurface.screenshot({
      path: testInfo.outputPath("stock-beta-chart-forced-colors.png"),
    });

    await page.context().addCookies([{ name: "locale", value: "ko", url: appOrigin }]);
    await page.reload();
    const koreanChart = page
      .getByRole("region", { name: "선택 신호 프로필" })
      .getByRole("tabpanel", { name: "가격" })
      .getByTestId("stock-beta-price-chart");
    await expectCanvasReady(koreanChart);
    await expect(koreanChart.getByTestId("stock-beta-price-chart-reset")).toHaveAccessibleName(
      "확대/축소 초기화",
    );
    await expect(koreanChart.getByTestId("stock-beta-price-chart-attribution")).toContainText(
      "차트 출처",
    );
    await expect(
      koreanChart
        .getByTestId("stock-beta-price-chart-attribution")
        .getByRole("link", { name: "NOTICE" }),
    ).toHaveAttribute("href", "/licenses/lightweight-charts/NOTICE.txt");
    await expect(
      koreanChart
        .getByTestId("stock-beta-price-chart-attribution")
        .getByRole("link", { name: "LICENSE" }),
    ).toHaveAttribute("href", "/licenses/lightweight-charts/LICENSE.txt");
  });

  test("switches Returns, Volatility, and Activity profile tabs", async ({ page, request }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 1,
      stockBetaSeed: "ready",
    });
    await page.goto("/stock-beta");
    for (const [name, metric] of [
      ["Returns", "20-session return"],
      ["Volatility", "20-session volatility"],
      ["Activity", "20-session average volume"],
    ] as const) {
      const tab = page.getByRole("tab", { name, exact: true });
      await tab.click();
      await expect(tab).toHaveAttribute("aria-selected", "true");
      await expect(signalProfilePanel(page, name)).toContainText(metric);
    }
  });

  test("renders current, stale, and unverifiable chart freshness without inventing data", async ({
    page,
    request,
  }) => {
    for (const [freshness, message] of [
      ["CURRENT", null],
      ["STALE", "EOD data is stale: as of 2026-08-28; expected 2026-08-31."],
      ["UNVERIFIABLE", "The latest EOD close cannot be verified against the expected market date."],
    ] as const) {
      await resetScenario(request, {
        authSession: "valid",
        role: "owner",
        stockBetaRows: 1,
        stockBetaSeed: "ready",
        stockBetaChartFreshness: freshness,
      });
      await page.goto("/stock-beta");
      const profile = signalProfilePanel(page);
      await expect(profile).toContainText(chartAsOf);
      if (message === null) await expect(profile).not.toContainText("cannot be verified");
      else await expect(profile).toContainText(message);
    }
  });

  test("fails chart closed for unavailable, integrity, missing, and forbidden chart responses", async ({
    page,
    request,
  }) => {
    for (const [state, expected] of [
      [
        "unavailable",
        { kind: "profile", text: "The EOD chart is being prepared or is unavailable." },
      ],
      [
        "integrity",
        { kind: "profile", text: "Chart integrity could not be verified. No price data is shown." },
      ],
      [
        "not_found",
        {
          kind: "profile",
          text: "The EOD chart could not be loaded. No previous price data is shown.",
        },
      ],
      [
        "forbidden",
        {
          kind: "profile",
          text: "The EOD chart could not be loaded. No previous price data is shown.",
        },
      ],
    ] as const) {
      await resetScenario(request, {
        authSession: "valid",
        role: "owner",
        stockBetaRows: 1,
        stockBetaSeed: "ready",
        stockBetaChartState: state,
      });
      await page.goto("/stock-beta");
      await expect(page.getByTestId("stock-beta-price-chart")).toHaveCount(0);
      if (expected.kind === "profile") {
        await expect(
          signalProfilePanel(page)
            .filter({ hasText: expected.text })
            .getByText(expected.text, { exact: true }),
        ).toBeVisible();
      } else {
        await expect(page.getByRole("tabpanel", { name: "Price" })).toHaveCount(0);
        await expect(
          page.getByRole("alert", { name: "Stock signal beta unavailable" }),
        ).toContainText(expected.text);
      }
    }
  });

  test("removes the previous Canvas and tooltip when a later range response fails", async ({
    page,
    request,
  }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 1,
      stockBetaSeed: "ready",
    });
    await page.goto("/stock-beta");
    const chart = priceChart(page);
    await expectCanvasReady(chart);
    await expect(priceChartPart(page, "stock-beta-price-chart-tooltip")).toBeVisible();
    expect(
      await priceChartPart(page, "stock-beta-price-chart-canvas").locator("canvas").count(),
    ).toBeGreaterThan(0);

    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 1,
      stockBetaSeed: "ready",
      stockBetaChartState: "not_found",
    });
    const failedRange = page.waitForResponse((response) => {
      const url = new URL(response.url());
      return (
        url.pathname.includes(`${chartPathFragment}000001.KRX/chart`) &&
        url.searchParams.get("range") === "3m"
      );
    });
    await page.getByRole("button", { name: "3M", exact: true }).click();
    expect((await failedRange).ok()).toBeFalsy();
    await expect(chart).toHaveCount(0);
    await expect(signalProfilePanel(page)).toContainText(
      "The EOD chart could not be loaded. No previous price data is shown.",
    );
    await expect(priceChartPart(page, "stock-beta-price-chart-tooltip")).toHaveCount(0);
  });

  test("uses the requested desktop geometry at 1280 and captures evidence", async ({
    page,
    request,
  }, testInfo) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 31,
      stockBetaSeed: "ready",
    });

    await page.setViewportSize({ width: 1280, height: 720 });
    await page.goto("/stock-beta");
    await expect(snapshotStrip(page)).toBeVisible();
    await expectCanvasReady(priceChart(page));
    const ranked = await box(page.getByTestId("stock-beta-widget-ranked-signals"));
    const profile = await box(signalProfileRegion(page));
    const profileWidget = await box(page.getByTestId("stock-beta-widget-signal-profile"));
    const grid = await box(page.getByTestId("stock-beta-dashboard").locator(":scope > div").last());
    const decomposition = await box(page.getByTestId("stock-beta-widget-signal-decomposition"));
    const matrix = await box(page.getByTestId("stock-beta-widget-condition-matrix"));
    const tape = await box(page.getByTestId("stock-beta-widget-snapshot-tape"));
    const management = await box(page.getByTestId("stock-beta-widget-universe-management"));
    const membershipBounds = await box(page.getByTestId("stock-beta-widget-membership-status"));
    expect(Math.abs(profileWidget.x - grid.x)).toBeLessThanOrEqual(1);
    expect(Math.abs(profileWidget.width - grid.width)).toBeLessThanOrEqual(1);
    expect(Math.abs(profile.x - profileWidget.x)).toBeLessThanOrEqual(1);
    expect(Math.abs(profile.width - profileWidget.width)).toBeLessThanOrEqual(1);
    expect(profileWidget.y).toBeLessThan(ranked.y);
    expect(profileWidget.y + profileWidget.height).toBeLessThan(ranked.y);
    expect(ranked.x).toBeLessThan(decomposition.x);
    expect(Math.abs(ranked.y - decomposition.y)).toBeLessThan(2);
    expect(matrix.x).toBeLessThan(tape.x);
    expect(Math.abs(matrix.y - tape.y)).toBeLessThan(2);
    expect(ranked.y).toBeLessThan(matrix.y);
    expect(matrix.y).toBeLessThan(management.y);
    expect(management.x).toBeLessThan(membershipBounds.x);
    expect(Math.abs(management.y - membershipBounds.y)).toBeLessThan(2);
    const membership = membershipRegion(page);
    const firstCard = membershipCard(page, "000001.KRX");
    const detailAction = firstCard.getByRole("link", { name: "Open detail" });
    const disableAction = firstCard.getByRole("button", { name: "Disable" });
    await detailAction.focus();
    await page.keyboard.press("Tab");
    await expect(disableAction).toBeFocused();
    const disableBounds = await box(disableAction);
    const disableFocusOutline = {
      height: disableBounds.height + 6,
      width: disableBounds.width + 6,
      x: disableBounds.x - 3,
      y: disableBounds.y - 3,
    };
    const membershipList = membership.getByTestId("stock-beta-memberships");
    const membershipContent = membership.locator(":scope > div").first();
    expectBoxInside(disableFocusOutline, await box(firstCard));
    expectBoxInside(disableFocusOutline, { x: 0, y: 0, width: 1280, height: 720 });
    expectBoxInside(disableFocusOutline, await box(membershipList));
    expectBoxInside(disableFocusOutline, await box(membershipContent));
    for (const ancestor of await clippingAncestorBoxes(disableAction)) {
      expectBoxInside(disableFocusOutline, ancestor.box);
    }
    await expectNoHorizontalOverflow(page);
    await page.screenshot({
      animations: "disabled",
      fullPage: true,
      path: testInfo.outputPath("stock-beta-1280x720.png"),
    });
  });

  test("reveals active navigation initially and after a client transition without stealing focus or scrolling the page", async ({
    page,
    request,
  }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 1,
      stockBetaSeed: "ready",
    });
    await installPrimaryNavigationRevealInstrumentation(page);
    await page.setViewportSize({ width: 375, height: 800 });
    await page.goto("/stock-beta");

    const navigation = page.getByRole("navigation", { name: "Primary" });
    const activeNavigationLink = navigation.getByRole("link", { name: "Stock signal beta" });
    await expect
      .poll(() => primaryNavigationRevealEvents(page).then((events) => events.length))
      .toBeGreaterThan(0);
    const initialReveal = (await primaryNavigationRevealEvents(page)).at(-1);
    expect(initialReveal).toBeDefined();
    expect(initialReveal?.bodyScrollYAfter).toBe(initialReveal?.bodyScrollYBefore);
    expect(initialReveal?.focusedAfter).toBe(initialReveal?.focusedBefore);
    expectBoxInside(await box(activeNavigationLink), await box(navigation));

    await navigation.evaluate((element) => {
      element.scrollLeft = 0;
    });
    const revealCountBeforeTransition = (await primaryNavigationRevealEvents(page)).length;
    await membershipCard(page, "000001.KRX").getByRole("link", { name: "Open detail" }).click();
    await expect(page).toHaveURL(/\/stock-beta\/000001\.KRX$/);
    await expect
      .poll(() => primaryNavigationRevealEvents(page).then((events) => events.length))
      .toBeGreaterThan(revealCountBeforeTransition);
    const transitionReveal = (await primaryNavigationRevealEvents(page)).at(-1);
    expect(transitionReveal?.bodyScrollYAfter).toBe(transitionReveal?.bodyScrollYBefore);
    expect(transitionReveal?.focusedAfter).toBe(transitionReveal?.focusedBefore);
    expectBoxInside(await box(activeNavigationLink), await box(navigation));
  });

  // Each viewport gets the default test context and its own trace/video lifetime.
  for (const viewport of [
    { name: "375x800", width: 375, height: 800 },
    { name: "640x360-200-percent-equivalent", width: 640, height: 360 },
    { name: "768x1024", width: 768, height: 1024 },
    { name: "1024x900", width: 1024, height: 900 },
    { name: "1280x720", width: 1280, height: 720 },
    { name: "1440x900", width: 1440, height: 900 },
  ]) {
    test(`reflows at ${viewport.name} with the complete chart interaction coverage`, async ({
      page,
      request,
    }, testInfo) => {
      test.setTimeout(90_000);
      await resetScenario(request, {
        authSession: "valid",
        role: "owner",
        stockBetaRows: 31,
        stockBetaSeed: "ready",
      });
      await page.setViewportSize({ width: viewport.width, height: viewport.height });
      await page.goto("/stock-beta");
      await expect(rankedSignalsTable(page)).toBeVisible();
      await expect(signalPreview(page)).toBeVisible();
      const chart = priceChart(page);
      await expect(chart).toBeVisible();
      await chart.scrollIntoViewIfNeeded();
      await expectCanvasReady(chart);
      await expect(priceChartPart(page, "stock-beta-price-chart-summary")).toBeVisible();
      await expect(priceChartPart(page, "stock-beta-price-chart-tooltip")).toBeVisible();
      await expect(priceChartPart(page, "stock-beta-price-chart-legend")).toBeVisible();
      await expect(priceChartPart(page, "stock-beta-price-chart-reset")).toBeVisible();
      await expect(priceChartPart(page, "stock-beta-price-chart-attribution")).toBeVisible();
      await visiblePlotTarget(chart);
      const oneYear = await fetchServerSeededChartPayload(
        request,
        snapshotIdFromText(await snapshotStrip(page).textContent()),
        "1y",
      );
      await expectPointerBoundary(chart, oneYear, "first");
      await expectPointerBoundary(chart, oneYear, "latest");
      const profileBounds = await box(signalProfilePanel(page));
      const chartBounds = await box(chart);
      expect(chartBounds.x).toBeGreaterThanOrEqual(profileBounds.x);
      expect(chartBounds.x + chartBounds.width).toBeLessThanOrEqual(
        profileBounds.x + profileBounds.width,
      );
      const surface = priceChartPart(page, "stock-beta-price-chart-surface");
      await expectUnclipped(surface, "complete price/volume surface and axes");
      await surface.screenshot({
        animations: "disabled",
        path: testInfo.outputPath(`stock-beta-price-chart-${viewport.name}.png`),
      });
      for (const testId of [
        "stock-beta-price-chart-summary",
        "stock-beta-price-chart-tooltip",
        "stock-beta-price-chart-legend",
        "stock-beta-price-chart-reset",
        "stock-beta-price-chart-attribution",
      ]) {
        await expectUnclipped(chart.getByTestId(testId), testId);
      }
      await surface.focus();
      await expect(surface).toBeFocused();
      await surface.press("Home");
      expect(await selectedChartDate(chart.getByTestId("stock-beta-price-chart-summary"))).toBe(
        chartFirstSession,
      );
      await expectTooltipMatchesBar(chart, requiredChartBar(oneYear, 0));
      await surface.press("End");
      expect(await selectedChartDate(priceChartPart(page, "stock-beta-price-chart-summary"))).toBe(
        chartAsOf,
      );
      await expectTooltipMatchesBar(chart, requiredChartBar(oneYear, oneYear.bars.length - 1));
      await testInfo.attach(`stock-beta-chart-geometry-${viewport.name}`, {
        body: JSON.stringify(
          {
            viewport,
            geometry: await expectChartGeometry(chart),
            paint: await canvasPaintEvidence(chart.getByTestId("stock-beta-price-chart-canvas")),
            summary: await chart.getByTestId("stock-beta-price-chart-summary").textContent(),
          },
          null,
          2,
        ),
        contentType: "application/json",
      });
      await expectNoHorizontalOverflow(page);
    });
  }

  test("reveals the active destination again when its locale label changes on the same pathname", async ({
    page,
    request,
  }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 1,
      stockBetaSeed: "ready",
    });
    await page.context().addCookies([{ name: "locale", value: "ko", url: appOrigin }]);
    await installPrimaryNavigationRevealInstrumentation(page);
    await page.setViewportSize({ width: 375, height: 800 });
    await page.goto("/stock-beta");

    const navigation = page.getByRole("navigation", { name: "Primary" });
    await expect(navigation.getByRole("link", { name: "종목 신호 베타" })).toBeVisible();
    await navigation.evaluate((element) => {
      element.scrollLeft = 0;
    });
    const revealCountBeforeLocaleChange = (await primaryNavigationRevealEvents(page)).length;
    await page.getByRole("button", { name: "EN", exact: true }).click();
    const activeNavigationLink = navigation.getByRole("link", { name: "Stock signal beta" });
    await expect(activeNavigationLink).toBeVisible();
    await expect
      .poll(() => primaryNavigationRevealEvents(page).then((events) => events.length))
      .toBeGreaterThan(revealCountBeforeLocaleChange);
    const localeReveal = (await primaryNavigationRevealEvents(page)).at(-1);
    expect(localeReveal?.bodyScrollYAfter).toBe(localeReveal?.bodyScrollYBefore);
    expect(localeReveal?.focusedAfter).toBe(localeReveal?.focusedBefore);
    expectBoxInside(await box(activeNavigationLink), await box(navigation));
  });

  test("keeps membership actions unclipped around the container-width reflow boundary", async ({
    page,
    request,
  }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 1,
      stockBetaSeed: "ready",
    });

    for (const width of [1280, 1360, 1361, 1400]) {
      await page.setViewportSize({ width, height: 720 });
      await page.goto("/stock-beta");
      const card = membershipCard(page, "000001.KRX");
      const detail = card.getByRole("link", { name: "Open detail" });
      const disable = card.getByRole("button", { name: "Disable" });
      await detail.focus();
      await page.keyboard.press("Tab");
      await expect(disable).toBeFocused();
      const bounds = await box(disable);
      const focusOutline = {
        height: bounds.height + 6,
        width: bounds.width + 6,
        x: bounds.x - 3,
        y: bounds.y - 3,
      };
      expectBoxInside(focusOutline, await box(card));
      expectBoxInside(
        focusOutline,
        await box(membershipRegion(page).getByTestId("stock-beta-memberships")),
      );
      expectBoxInside(focusOutline, { x: 0, y: 0, width, height: 720 });
      for (const ancestor of await clippingAncestorBoxes(disable)) {
        expectBoxInside(focusOutline, ancestor.box);
      }
    }
  });

  test("supports Korean and English locale plus forced colors and reduced motion", async ({
    page,
    request,
  }) => {
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 31,
      stockBetaSeed: "ready",
    });

    await page.context().addCookies([{ name: "locale", value: "ko", url: appOrigin }]);
    await page.goto("/stock-beta");
    await expect(page.locator("html")).toHaveAttribute("lang", /^ko/);
    await expect(page.getByRole("heading", { name: "종목 신호 베타" })).toBeVisible();
    await expect(page.getByRole("table", { name: /V2 가격·거래량 신호 순위 표/ })).toBeVisible();

    await page.context().addCookies([{ name: "locale", value: "en", url: appOrigin }]);
    await page.goto("/stock-beta");
    await expect(page.locator("html")).toHaveAttribute("lang", /^en/);
    await expect(page.getByRole("heading", { name: "Stock signal beta" })).toBeVisible();

    await page.emulateMedia({ forcedColors: "active", reducedMotion: "reduce" });
    await page.goto("/stock-beta");
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            window.matchMedia("(forced-colors: active)").matches &&
            window.matchMedia("(prefers-reduced-motion: reduce)").matches,
        ),
      )
      .toBeTruthy();
    await expect(
      page.getByRole("table", { name: /V2 ranked price-and-volume signal table/ }),
    ).toBeVisible();
    await expectNoHorizontalOverflow(page);
  });

  test("keeps touch targets usable with coarse input", async ({ browser, request }, testInfo) => {
    test.setTimeout(60_000);
    await resetScenario(request, {
      authSession: "valid",
      role: "owner",
      stockBetaRows: 31,
      stockBetaSeed: "ready",
    });

    const context = await browser.newContext({
      baseURL: appOrigin,
      hasTouch: true,
      isMobile: true,
      viewport: { width: 375, height: 800 },
    });
    const page = await context.newPage();
    let cdp: CDPSession | undefined;
    try {
      const requests = observeBrowserRequests(page);
      await installProviderFreeNetworkGuard(page);
      await page.goto("/stock-beta");
      const selectTarget = rankedRow(page, "000001.KRX").getByRole("button", {
        name: "Select signal: 000001.KRX",
      });
      const targets = [
        page.getByLabel("Search signals"),
        selectTarget,
        dashboardRegion(page, "Condition matrix").getByTestId("stock-beta-matrix-000001.KRX"),
      ];
      for (const target of targets) {
        const targetBox = await box(target);
        expect(targetBox.height).toBeGreaterThanOrEqual(44);
      }
      for (const range of ["1M", "3M", "6M", "1Y"]) {
        expect(
          (await box(page.getByRole("button", { name: range, exact: true }))).height,
        ).toBeGreaterThanOrEqual(44);
      }
      const oneYear = await fetchServerSeededChartPayload(
        request,
        snapshotIdFromText(await snapshotStrip(page).textContent()),
        "1y",
      );
      const chart = priceChart(page);
      await expectCanvasReady(chart);
      const chartGesturesRequests = chartRequests(requests).length;
      await selectTarget.tap();
      await expect(rankedRow(page, "000001.KRX")).toHaveAttribute("data-selected", "true");
      const summary = priceChartPart(page, "stock-beta-price-chart-summary");
      const touchSession = await context.newCDPSession(page);
      cdp = touchSession;
      await touchSession.send("Emulation.setTouchEmulationEnabled", {
        enabled: true,
        maxTouchPoints: 2,
      });
      const scrollSignature = (state: Awaited<ReturnType<typeof chartScrollPositions>>) =>
        JSON.stringify([state.documentScrollTop, state.mainScrollTop, state.nested]);

      // Verify independent page scrolling before entering native long-press tracking mode.
      const verticalPlot = await visiblePlotTarget(chart);
      const plotScrollBefore = await chartScrollPositions(page);
      const verticalDistance = Math.min(120, verticalPlot.y - 8);
      expect(verticalDistance, "visible plot provides a real swipe path").toBeGreaterThan(20);
      await dispatchTimedTouches(
        page,
        cdp,
        [{ id: 1, x: verticalPlot.x, y: verticalPlot.y }],
        [{ id: 1, x: verticalPlot.x, y: verticalPlot.y - verticalDistance }],
      );
      await expect
        .poll(async () => scrollSignature(await chartScrollPositions(page)))
        .not.toBe(scrollSignature(plotScrollBefore));
      const plotScrollAfter = await chartScrollPositions(page);

      // The fixed terminal shell scrolls main, while the profile widget has its own scroller.
      // Swipe outside that widget to prove page movement separately from nested widget movement.
      const heading = page.getByRole("heading", { name: "Stock signal beta", exact: true });
      await heading.scrollIntoViewIfNeeded();
      const pageTarget = await heading.evaluate((element) => {
        const bounds = element.getBoundingClientRect();
        const mainTop = document.querySelector("main")?.getBoundingClientRect().top ?? 0;
        const x = (bounds.left + bounds.right) / 2;
        const y = (bounds.top + bounds.bottom) / 2;
        const hit = document.elementFromPoint(x, y);
        return {
          distance: Math.min(120, y - Math.max(0, mainTop) - 4),
          hit: hit === element || (hit !== null && element.contains(hit)),
          x,
          y,
        };
      });
      expect(pageTarget.hit, "page swipe hits its visible heading").toBe(true);
      expect(pageTarget.distance).toBeGreaterThan(20);
      const pageScrollBefore = await chartScrollPositions(page);
      await dispatchTimedTouches(
        page,
        cdp,
        [{ id: 1, x: pageTarget.x, y: pageTarget.y }],
        [{ id: 1, x: pageTarget.x, y: pageTarget.y - pageTarget.distance }],
      );
      await expect
        .poll(async () => {
          const state = await chartScrollPositions(page);
          return state.documentScrollTop + state.mainScrollTop;
        })
        .toBeGreaterThan(pageScrollBefore.documentScrollTop + pageScrollBefore.mainScrollTop);
      const pageScrollAfter = await chartScrollPositions(page);
      expect(pageScrollAfter.nested, "page gesture does not stand in for widget scrolling").toEqual(
        pageScrollBefore.nested,
      );
      await testInfo.attach("mobile-chart-scroll-observations", {
        body: JSON.stringify(
          { plotScrollBefore, plotScrollAfter, pageScrollBefore, pageScrollAfter },
          null,
          2,
        ),
        contentType: "application/json",
      });

      await expect
        .poll(() => inspectTouchPlotObservation(chart, touchSession, 0.3))
        .not.toBe(chartAsOf);
      await expect(priceChartPart(page, "stock-beta-price-chart-tooltip")).toBeVisible();
      const selectedDate = await selectedChartDate(summary);
      const selectedBar = oneYear.bars.find((bar) => bar.session_date === selectedDate);
      expect(selectedBar, `touch selected ${selectedDate}`).toBeDefined();
      if (selectedBar === undefined)
        throw new Error(`Expected chart observation for ${selectedDate}`);
      await expectTooltipMatchesBar(chart, selectedBar);
      await chart.screenshot({ path: testInfo.outputPath("stock-beta-mobile-long-press.png") });

      const dateBeforePan = await inspectTouchPlotObservation(chart, touchSession, 0.3);
      const panStart = await visiblePlotTarget(chart, 0.7);
      const panEnd = await visiblePlotTarget(chart, 0.35);
      await dispatchTimedTouches(
        page,
        cdp,
        [{ id: 1, x: panStart.x, y: panStart.y }],
        [{ id: 1, x: panEnd.x, y: panEnd.y }],
      );
      await expect
        .poll(() => inspectTouchPlotObservation(chart, touchSession, 0.3))
        .not.toBe(dateBeforePan);
      const pannedDate = await selectedChartDate(summary);
      expect(oneYear.bars.some((bar) => bar.session_date === pannedDate)).toBeTruthy();
      const pannedBar = oneYear.bars.find((bar) => bar.session_date === pannedDate);
      if (pannedBar === undefined) throw new Error(`Expected panned observation ${pannedDate}`);
      await expectTooltipMatchesBar(chart, pannedBar);

      const pinchLeftStart = await visiblePlotTarget(chart, 0.45);
      const pinchRightStart = await visiblePlotTarget(chart, 0.65);
      const pinchLeftEnd = await visiblePlotTarget(chart, 0.35);
      const pinchRightEnd = await visiblePlotTarget(chart, 0.75);
      await dispatchTimedTouches(
        page,
        cdp,
        [
          { id: 1, x: pinchLeftStart.x, y: pinchLeftStart.y },
          { id: 2, x: pinchRightStart.x, y: pinchRightStart.y },
        ],
        [
          { id: 1, x: pinchLeftEnd.x, y: pinchLeftEnd.y },
          { id: 2, x: pinchRightEnd.x, y: pinchRightEnd.y },
        ],
      );
      await expect
        .poll(() => inspectTouchPlotObservation(chart, touchSession, 0.3))
        .not.toBe(pannedDate);
      const pinchedDate = await selectedChartDate(summary);
      expect(oneYear.bars.some((bar) => bar.session_date === pinchedDate)).toBeTruthy();
      const pinchedBar = oneYear.bars.find((bar) => bar.session_date === pinchedDate);
      if (pinchedBar === undefined) throw new Error(`Expected pinched observation ${pinchedDate}`);
      await expectTooltipMatchesBar(chart, pinchedBar);
      await testInfo.attach("mobile-chart-observation-dates", {
        body: JSON.stringify({ selectedDate, dateBeforePan, pannedDate, pinchedDate }, null, 2),
        contentType: "application/json",
      });
      await expectNoHorizontalOverflow(page);
      expect(chartRequests(requests)).toHaveLength(chartGesturesRequests);
      expectProviderFree(requests);
      expect(observedPageErrors.get(page) ?? [], "mobile browser page errors").toEqual([]);
    } finally {
      await cdp?.detach();
      await context.close();
    }
  });
});
