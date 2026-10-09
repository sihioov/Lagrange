import { createServer } from "node:http";

const API_PREFIX = "/api/v1/research/owner-beta/equity-universe-v2";
const LEASE_PATH = `${API_PREFIX}/stream-leases`;
const STREAM_PATH = `${API_PREFIX}/market-stream`;
const LEASE_ID = "00000000-0000-4000-8000-000000000901";
const STREAM_ID = "00000000-0000-4000-8000-000000000903";

function uuid(number) {
  return `00000000-0000-4000-8000-${String(number).padStart(12, "0")}`;
}

function iso(milliseconds) {
  return new Date(milliseconds).toISOString();
}

function makeRow(identity, index, serverMs, sequence) {
  const receivedAt = iso(serverMs - 100);
  const committedAt = iso(serverMs - 50);
  const tradeUtcMs = Math.floor((serverMs - 250) / 1_000) * 1_000;
  const tradeKst = new Date(tradeUtcMs + 9 * 60 * 60 * 1_000).toISOString();
  const businessDate = tradeKst.slice(0, 10);
  const tradeTime = tradeKst.slice(11, 19);

  return {
    membership_id: identity.membership_id,
    instrument_id: identity.instrument_id,
    generation: identity.generation,
    row_generation: uuid(910 + index),
    venue: "KRX",
    currency: "KRW",
    source: "KIS_MARKET_WS",
    wire_version: "kis-h0stcnt0-20260914-v1",
    session: {
      date: businessDate,
      timezone: "Asia/Seoul",
      calendar_source: "kis",
      calendar_source_version: "kis-chk-holiday-v1:schema-1",
      calendar_content_sha256: "a".repeat(64),
      window_contract_sha256: `sha256:${"b".repeat(64)}`,
    },
    subscription: "ACKED",
    connection: "CONNECTED",
    market_state: "OPEN",
    freshness: "RECENT",
    availability: "LIVE",
    reason_code: null,
    state_version: String(sequence),
    gap_open: false,
    session_has_gap: false,
    gap_generation: "0",
    quote: {
      price: index === 0 ? "101200.00" : "198400.00",
      base_price: null,
      base_price_reason: "NOT_PROVIDED_BY_CHANNEL",
      change_from_previous_day: "1200.00",
      change_percent_from_previous_day: "1.20",
      direction: "UP",
      trade_volume: "100",
      cumulative_volume: "1234567",
      halted: false,
      business_date: businessDate,
      trade_time: tradeTime,
      provider_trade_at: `${tradeKst.slice(0, 19)}+09:00`,
      received_at: receivedAt,
      committed_at: committedAt,
      epoch: uuid(930 + index),
      quote_version: String(sequence),
      receive_ordinal: String(sequence),
    },
  };
}

function leaseExpiry(serverMs = Date.now()) {
  return iso(serverMs + 30_000);
}

function leaseResponse(requestBody) {
  return {
    schema_version: 2,
    lease_id: LEASE_ID,
    consumer_id: requestBody.consumer_id,
    renewal_sequence: requestBody.renewal_sequence,
    lease_expires_at: leaseExpiry(),
    renew_after_ms: 15_000,
    identities: requestBody.identities,
  };
}

function sendJson(response, status, body) {
  response.writeHead(status, {
    "cache-control": "no-store",
    "content-type": "application/json; charset=utf-8",
  });
  response.end(JSON.stringify(body));
}

function validMutationHeaders(request) {
  const headers = request.headers();
  return (
    headers["x-csrf-token"] !== undefined &&
    headers["idempotency-key"] !== undefined &&
    headers["content-type"]?.startsWith("application/json")
  );
}

function parseRequestBody(request) {
  const raw = request.postData();
  if (raw === null) return null;
  try {
    return JSON.parse(raw);
  } catch {
    return null;
  }
}

/** One page-owned, deterministic market-stream server with no provider transport. */
export async function startServiceStatusMarketStreamFixture(appOrigin) {
  const appUrl = new URL(appOrigin);
  if (
    appUrl.origin !== appOrigin ||
    appUrl.protocol !== "http:" ||
    appUrl.hostname !== "127.0.0.1" ||
    appUrl.port !== "33043"
  ) {
    throw new Error("The fixture requires the exact isolated market-stream app origin");
  }
  let snapshotFrame = null;
  let acceptedLease = null;
  const state = {
    leaseCreates: 0,
    leaseRenewals: 0,
    leaseReleases: 0,
    latestRenewalSequence: -1,
    consumerId: null,
    identities: [],
    activeSseConnections: 0,
    sseConnections: 0,
    sseClosed: 0,
    providerRequests: 0,
    quoteDemandRequests: 0,
  };
  const clients = new Set();

  const server = createServer((request, response) => {
    const url = new URL(request.url ?? "/", "http://127.0.0.1");
    if (request.method !== "GET" || url.pathname !== "/market-stream") {
      sendJson(response, 404, { error: { code: "NOT_FOUND" } });
      return;
    }
    if (url.searchParams.get("lease_id") !== LEASE_ID || state.leaseCreates !== 1) {
      sendJson(response, 404, { error: { code: "LEASE_NOT_FOUND" } });
      return;
    }

    let sequence = 1;
    let lastServerMs = Date.now();
    const event = {
      schema_version: 2,
      stream_id: STREAM_ID,
      event_sequence: String(sequence),
      server_time: iso(lastServerMs),
      body: {
        lease_id: LEASE_ID,
        lease_expires_at: leaseExpiry(lastServerMs),
        rows: state.identities.map((identity, index) =>
          makeRow(identity, index, lastServerMs, sequence),
        ),
      },
    };
    snapshotFrame = { event, eventId: `${STREAM_ID}:1` };
    response.writeHead(200, {
      "access-control-allow-origin": appOrigin,
      "access-control-allow-credentials": "true",
      "cache-control": "no-cache, no-transform",
      connection: "keep-alive",
      "content-type": "text/event-stream; charset=utf-8",
      "x-accel-buffering": "no",
    });
    response.write(`event: snapshot\nid: ${STREAM_ID}:1\ndata: ${JSON.stringify(event)}\n\n`);
    const heartbeat = setInterval(() => {
      lastServerMs = Math.max(Date.now(), lastServerMs + 1);
      sequence += 1;
      const delta = {
        schema_version: 2,
        stream_id: STREAM_ID,
        event_sequence: String(sequence),
        server_time: iso(lastServerMs),
        body: {
          rows: state.identities.map((identity, index) =>
            makeRow(identity, index, lastServerMs, sequence),
          ),
        },
      };
      response.write(
        `event: delta\nid: ${STREAM_ID}:${sequence}\ndata: ${JSON.stringify(delta)}\n\n`,
      );
    }, 1_000);
    state.activeSseConnections += 1;
    state.sseConnections += 1;
    clients.add(response);
    response.on("close", () => {
      clearInterval(heartbeat);
      if (clients.delete(response)) {
        state.activeSseConnections -= 1;
        state.sseClosed += 1;
      }
    });
  });

  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const address = server.address();
  if (address === null || typeof address === "string")
    throw new Error("Fixture HTTP server did not bind TCP");
  const origin = `http://127.0.0.1:${address.port}`;

  async function installPageRoutes(page) {
    await page.route("**/quote-demands**", async (route) => {
      state.quoteDemandRequests += 1;
      await route.fulfill({ status: 599, body: "" });
    });
    await page.route("**/instruments/**/quote**", async (route) => {
      state.quoteDemandRequests += 1;
      await route.fulfill({ status: 599, body: "" });
    });
    await page.route(`**${LEASE_PATH}**`, async (route) => {
      const request = route.request();
      const url = new URL(request.url());
      if (request.method() === "POST" && url.pathname === LEASE_PATH) {
        if (!validMutationHeaders(request)) {
          await route.fulfill({ status: 400, body: "" });
          return;
        }
        const body = parseRequestBody(request);
        if (
          body === null ||
          body.schema_version !== 2 ||
          typeof body.consumer_id !== "string" ||
          !Array.isArray(body.identities) ||
          body.identities.length === 0 ||
          !Number.isSafeInteger(body.renewal_sequence)
        ) {
          await route.fulfill({ status: 400, body: "" });
          return;
        }
        if (body.renewal_sequence === 0) {
          if (state.latestRenewalSequence !== -1) {
            await route.fulfill({ status: 409, body: "" });
            return;
          }
          state.leaseCreates += 1;
          state.consumerId = body.consumer_id;
          state.identities = body.identities;
        } else {
          if (
            body.renewal_sequence !== state.latestRenewalSequence + 1 ||
            body.consumer_id !== state.consumerId ||
            JSON.stringify(body.identities) !== JSON.stringify(state.identities)
          ) {
            await route.fulfill({ status: 409, body: "" });
            return;
          }
          state.leaseRenewals += 1;
        }
        state.latestRenewalSequence = body.renewal_sequence;
        const reply = leaseResponse(body);
        acceptedLease = { request: body, response: reply };
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify(reply),
        });
        return;
      }

      const leaseMatch = new RegExp(`^${LEASE_PATH}/([0-9a-f-]+)$`).exec(url.pathname);
      if (request.method() === "DELETE" && leaseMatch?.[1] === LEASE_ID) {
        if (!validMutationHeaders(request)) {
          await route.fulfill({ status: 400, body: "" });
          return;
        }
        const body = parseRequestBody(request);
        if (
          body === null ||
          body.schema_version !== 2 ||
          body.consumer_id !== state.consumerId ||
          body.renewal_sequence !== state.latestRenewalSequence
        ) {
          await route.fulfill({ status: 400, body: "" });
          return;
        }
        state.leaseReleases += 1;
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({ schema_version: 2, lease_id: LEASE_ID, released: true }),
        });
        return;
      }

      await route.fulfill({ status: 404, body: "" });
    });

    await page.route(`**${STREAM_PATH}**`, async (route) => {
      const url = new URL(route.request().url());
      await route.continue({ url: `${origin}/market-stream${url.search}` });
    });
  }

  return {
    origin,
    installPageRoutes,
    metrics: () => ({ ...state }),
    snapshotFrame: () => structuredClone(snapshotFrame),
    acceptedLease: () => structuredClone(acceptedLease),
    close: async () => {
      for (const client of clients) client.end();
      await new Promise((resolve) => server.close(resolve));
    },
  };
}
