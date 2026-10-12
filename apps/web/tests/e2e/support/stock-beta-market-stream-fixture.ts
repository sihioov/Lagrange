import { randomUUID } from "node:crypto";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import type { Page, Route } from "@playwright/test";
import {
  IntradayStreamContractError,
  type IntradayStreamIdentity,
  parseIntradayStreamEvent,
} from "../../../lib/products/intraday-stream-contracts";
import {
  canonicalStreamIdentities,
  INTRADAY_STREAM_LEASE_MS,
  INTRADAY_STREAM_LEASE_PATH,
  INTRADAY_STREAM_PREFIX,
  INTRADAY_STREAM_RENEW_MS,
  type IntradayStreamLeaseRequest,
  intradayStreamLeaseRequestSchema,
  intradayStreamLeaseResponseSchema,
  intradayStreamReleaseRequestSchema,
  intradayStreamReleaseResponseSchema,
} from "../../../lib/products/intraday-stream-lease-contracts";
import { MARKET_STREAM_INSTRUMENTS } from "../../../lib/products/intraday-stream-universe";

const STREAM_PATH = INTRADAY_STREAM_PREFIX + "/market-stream";
const MAX_MUTATION_BYTES = 16 * 1024;
const UUID_PATTERN = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const FIXED_INSTRUMENTS = new Set(MARKET_STREAM_INSTRUMENTS.map((item) => item.id));

export type StockBetaMarketStreamOptions = {
  readonly readMemberships: () => Promise<readonly IntradayStreamIdentity[]>;
  readonly now: () => number;
};

type Failure = { readonly status: 409 | 503; readonly code: string };
type FrameKind = "snapshot" | "delta" | "status" | "reset";
type InvalidFrame = { readonly kind: FrameKind; readonly data: string };

type LeaseRecord = {
  readonly leaseId: string;
  readonly consumerId: string;
  renewalSequence: number;
  identities: IntradayStreamIdentity[];
  expiresAt: string;
  expiresAtMs: number;
  released: boolean;
  readonly acceptedRequest: IntradayStreamLeaseRequest;
};

type SentFrame = {
  readonly kind: FrameKind;
  readonly eventId: string;
  readonly streamId: string;
  readonly eventSequence: string;
  readonly serverTime: string;
  readonly invalid: boolean;
};

type Connection = {
  readonly id: string;
  readonly leaseId: string;
  readonly streamId: string;
  readonly response: ServerResponse;
  readonly openedAt: string;
  closedAt: string | null;
  sequence: number;
  readonly sent: SentFrame[];
};

type IdempotencyRecord = {
  readonly fingerprint: string;
  readonly status: number;
  readonly body: unknown;
};

type RouteRegistration = {
  readonly page: Page;
  readonly pattern: string;
  readonly handler: (route: Route) => Promise<void>;
};

type LeaseLedgerEntry = {
  readonly leaseId: string;
  readonly consumerId: string;
  readonly renewalSequence: number;
  readonly identities: readonly IntradayStreamIdentity[];
  readonly expiresAt: string;
  readonly released: boolean;
};

type ConnectionLedgerEntry = {
  readonly id: string;
  readonly leaseId: string;
  readonly streamId: string;
  readonly openedAt: string;
  readonly closedAt: string | null;
  readonly sent: readonly SentFrame[];
};

function iso(milliseconds: number): string {
  return new Date(milliseconds).toISOString();
}

function sameIdentity(left: IntradayStreamIdentity, right: IntradayStreamIdentity): boolean {
  return (
    left.membership_id === right.membership_id &&
    left.instrument_id === right.instrument_id &&
    left.generation === right.generation
  );
}

function sameIdentitySet(
  left: readonly IntradayStreamIdentity[],
  right: readonly IntradayStreamIdentity[],
): boolean {
  return (
    left.length === right.length &&
    left.every((identity, index) => {
      const candidate = right[index];
      return candidate !== undefined && sameIdentity(identity, candidate);
    })
  );
}

function sendJson(response: ServerResponse, status: number, code: string): void {
  if (response.destroyed || response.writableEnded) return;
  const bytes = JSON.stringify({ error: { code } });
  response.writeHead(status, {
    "cache-control": "no-store",
    "content-type": "application/json; charset=utf-8",
    "content-length": String(Buffer.byteLength(bytes)),
  });
  response.end(bytes);
}

function parseJson(value: string | null): unknown {
  if (value === null || Buffer.byteLength(value) > MAX_MUTATION_BYTES) return null;
  try {
    return JSON.parse(value) as unknown;
  } catch {
    return null;
  }
}

function safeRequestBody(route: Route): unknown {
  const request = route.request();
  const contentType = request.headers()["content-type"];
  if (contentType?.split(";", 1)[0]?.trim().toLowerCase() !== "application/json") return null;
  return parseJson(request.postData());
}

function hasMutationHeaders(route: Route): boolean {
  const headers = route.request().headers();
  const key = headers["idempotency-key"];
  return (
    typeof headers["x-csrf-token"] === "string" &&
    headers["x-csrf-token"].trim().length > 0 &&
    typeof key === "string" &&
    UUID_PATTERN.test(key)
  );
}

function identitySetFromBody(value: unknown): readonly IntradayStreamIdentity[] | null {
  if (typeof value !== "object" || value === null || !("identities" in value)) return null;
  const identities = (value as { readonly identities?: unknown }).identities;
  if (!Array.isArray(identities)) return null;
  try {
    return canonicalStreamIdentities(identities as IntradayStreamIdentity[]);
  } catch {
    return null;
  }
}

function buildLeaseReply(request: IntradayStreamLeaseRequest, leaseId: string, now: number) {
  return intradayStreamLeaseResponseSchema.parse({
    schema_version: 2,
    lease_id: leaseId,
    consumer_id: request.consumer_id,
    renewal_sequence: request.renewal_sequence,
    lease_expires_at: iso(now + INTRADAY_STREAM_LEASE_MS),
    renew_after_ms: INTRADAY_STREAM_RENEW_MS,
    identities: request.identities,
  });
}

async function readCurrentMemberships(
  callback: StockBetaMarketStreamOptions["readMemberships"],
): Promise<IntradayStreamIdentity[]> {
  const input = await callback();
  if (input.length === 0) return [];
  const identities = canonicalStreamIdentities(input);
  if (
    identities.some((identity) => !FIXED_INSTRUMENTS.has(identity.instrument_id)) ||
    identities.length > MARKET_STREAM_INSTRUMENTS.length
  ) {
    throw new IntradayStreamContractError();
  }
  return identities;
}

async function handleSseRequest(
  request: IncomingMessage,
  response: ServerResponse,
  leasesById: Map<string, LeaseRecord>,
  connections: Map<string, Connection>,
  options: StockBetaMarketStreamOptions,
  state: {
    getOpens: number;
    getCloses: number;
    activeSockets: number;
    peakSockets: number;
  },
  appOrigin: string,
): Promise<void> {
  const url = new URL(request.url ?? "/", "http://127.0.0.1");
  if (
    request.method !== "GET" ||
    url.pathname !== "/market-stream" ||
    url.searchParams.size !== 1 ||
    [...url.searchParams.keys()][0] !== "lease_id"
  ) {
    sendJson(response, 404, "RESOURCE_NOT_FOUND");
    return;
  }
  const leaseId = url.searchParams.get("lease_id");
  if (leaseId === null || !UUID_PATTERN.test(leaseId)) {
    sendJson(response, 404, "RESOURCE_NOT_FOUND");
    return;
  }
  const lease = leasesById.get(leaseId);
  if (lease === undefined || lease.released || options.now() >= lease.expiresAtMs) {
    sendJson(response, 404, "RESOURCE_NOT_FOUND");
    return;
  }
  let current: IntradayStreamIdentity[];
  try {
    current = await readCurrentMemberships(options.readMemberships);
  } catch {
    sendJson(response, 404, "RESOURCE_NOT_FOUND");
    return;
  }
  if (!sameIdentitySet(canonicalStreamIdentities(lease.identities), current)) {
    sendJson(response, 404, "RESOURCE_NOT_FOUND");
    return;
  }

  const id = randomUUID();
  const streamId = randomUUID();
  const connection: Connection = {
    id,
    leaseId,
    streamId,
    response,
    openedAt: iso(options.now()),
    closedAt: null,
    sequence: 0,
    sent: [],
  };
  connections.set(id, connection);
  state.getOpens += 1;
  state.activeSockets += 1;
  state.peakSockets = Math.max(state.peakSockets, state.activeSockets);
  response.writeHead(200, {
    "access-control-allow-origin": appOrigin,
    "access-control-allow-credentials": "true",
    "cache-control": "no-cache, no-transform",
    connection: "keep-alive",
    "content-type": "text/event-stream; charset=utf-8",
    "x-accel-buffering": "no",
    vary: "Origin",
  });
  response.flushHeaders();
  response.on("close", () => {
    if (connection.closedAt !== null) return;
    connection.closedAt = iso(options.now());
    state.activeSockets -= 1;
    state.getCloses += 1;
  });
}

export async function startStockBetaMarketStreamFixture(
  appOrigin: string,
  options: StockBetaMarketStreamOptions,
) {
  const parsedAppOrigin = new URL(appOrigin);
  if (
    parsedAppOrigin.origin !== appOrigin ||
    parsedAppOrigin.protocol !== "http:" ||
    parsedAppOrigin.hostname !== "127.0.0.1" ||
    parsedAppOrigin.port !== "33043"
  ) {
    throw new Error("The fixture requires the exact isolated market-stream app origin");
  }

  const leasesByConsumer = new Map<string, LeaseRecord>();
  const leasesById = new Map<string, LeaseRecord>();
  const connections = new Map<string, Connection>();
  const idempotency = new Map<string, IdempotencyRecord>();
  const routeRegistrations: RouteRegistration[] = [];
  const identityHistory: IntradayStreamIdentity[][] = [];
  const state = {
    createAttempts: 0,
    acceptedCreates: 0,
    rejectedCreates: 0,
    renewalAttempts: 0,
    acceptedRenewals: 0,
    rejectedRenewals: 0,
    releaseAttempts: 0,
    acceptedReleases: 0,
    rejectedReleases: 0,
    getOpens: 0,
    getCloses: 0,
    activeSockets: 0,
    peakSockets: 0,
    snapshots: 0,
    deltas: 0,
    statuses: 0,
    resets: 0,
    invalidSnapshots: 0,
    invalidDeltas: 0,
    invalidStatuses: 0,
    invalidResets: 0,
    legacyQuoteGets: 0,
    quoteDemandMutations: 0,
    rejectedReadProbes: 0,
  };
  let nextFailure: Failure | null = null;
  let closePromise: Promise<void> | null = null;

  const server = createServer((request, response) => {
    void handleSseRequest(
      request,
      response,
      leasesById,
      connections,
      options,
      state,
      parsedAppOrigin.origin,
    ).catch(() => {
      sendJson(response, 503, "MARKET_STREAM_UNAVAILABLE");
    });
  });
  server.requestTimeout = 5_000;
  server.headersTimeout = 5_000;
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const address = server.address();
  if (address === null || typeof address === "string") {
    await new Promise<void>((resolve) => server.close(() => resolve()));
    throw new Error("Fixture HTTP server did not bind TCP");
  }
  const origin = "http://127.0.0.1:" + String(address.port);

  async function currentReadyIdentities(): Promise<IntradayStreamIdentity[]> {
    return readCurrentMemberships(options.readMemberships);
  }

  async function rejectMutation(route: Route, status: number, code: string): Promise<void> {
    await route.fulfill({
      status,
      contentType: "application/json",
      body: JSON.stringify({ error: { code } }),
    });
  }

  async function handleLeaseMutation(route: Route): Promise<void> {
    const request = route.request();
    const url = new URL(request.url());
    if (url.origin !== appOrigin || url.search !== "") {
      await rejectMutation(route, 404, "RESOURCE_NOT_FOUND");
      return;
    }
    if (!hasMutationHeaders(route)) {
      await rejectMutation(route, 400, "INVALID_REQUEST");
      return;
    }
    const idempotencyKey = request.headers()["idempotency-key"] ?? "";
    const input = safeRequestBody(route);
    if (request.method() === "POST" && url.pathname === INTRADAY_STREAM_LEASE_PATH) {
      const parsed = intradayStreamLeaseRequestSchema.safeParse(input);
      const sequence =
        typeof input === "object" && input !== null && "renewal_sequence" in input
          ? (input as { readonly renewal_sequence?: unknown }).renewal_sequence
          : null;
      const creating = sequence === 0;
      if (creating) state.createAttempts += 1;
      else state.renewalAttempts += 1;
      if (!parsed.success) {
        if (creating) state.rejectedCreates += 1;
        else state.rejectedRenewals += 1;
        await rejectMutation(route, 400, "INVALID_REQUEST");
        return;
      }
      const body = parsed.data;
      const fingerprint = JSON.stringify(body);
      const idemKey = "POST:" + idempotencyKey;
      const replay = idempotency.get(idemKey);
      if (replay !== undefined) {
        if (replay.fingerprint !== fingerprint) {
          if (creating) state.rejectedCreates += 1;
          else state.rejectedRenewals += 1;
          await rejectMutation(route, 409, "STREAM_LEASE_SEQUENCE_CONFLICT");
          return;
        }
        await route.fulfill({
          status: replay.status,
          contentType: "application/json",
          body: JSON.stringify(replay.body),
        });
        return;
      }
      if (nextFailure !== null) {
        const failure = nextFailure;
        nextFailure = null;
        if (creating) state.rejectedCreates += 1;
        else state.rejectedRenewals += 1;
        await rejectMutation(route, failure.status, failure.code);
        return;
      }
      let current: IntradayStreamIdentity[];
      try {
        current = await currentReadyIdentities();
      } catch {
        if (creating) state.rejectedCreates += 1;
        else state.rejectedRenewals += 1;
        await rejectMutation(route, 404, "RESOURCE_NOT_FOUND");
        return;
      }
      const requested = identitySetFromBody(body);
      if (
        requested === null ||
        requested.some((identity) => !FIXED_INSTRUMENTS.has(identity.instrument_id)) ||
        !sameIdentitySet(requested, current)
      ) {
        if (creating) state.rejectedCreates += 1;
        else state.rejectedRenewals += 1;
        await rejectMutation(route, 404, "RESOURCE_NOT_FOUND");
        return;
      }
      let lease = leasesByConsumer.get(body.consumer_id);
      if (body.renewal_sequence === 0) {
        if (lease !== undefined) {
          state.rejectedCreates += 1;
          await rejectMutation(route, 409, "STREAM_LEASE_SEQUENCE_CONFLICT");
          return;
        }
        const leaseId = randomUUID();
        const reply = buildLeaseReply(body, leaseId, options.now());
        lease = {
          leaseId,
          consumerId: body.consumer_id,
          renewalSequence: 0,
          identities: [...body.identities],
          expiresAt: reply.lease_expires_at,
          expiresAtMs: Date.parse(reply.lease_expires_at),
          released: false,
          acceptedRequest: body,
        };
        leasesByConsumer.set(body.consumer_id, lease);
        leasesById.set(leaseId, lease);
        state.acceptedCreates += 1;
        identityHistory.push([...body.identities]);
        idempotency.set(idemKey, {
          fingerprint,
          status: 200,
          body: reply,
        });
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify(reply),
        });
        return;
      }
      if (
        lease === undefined ||
        lease.released ||
        body.renewal_sequence !== lease.renewalSequence + 1
      ) {
        state.rejectedRenewals += 1;
        await rejectMutation(route, 409, "STREAM_LEASE_SEQUENCE_CONFLICT");
        return;
      }
      const reply = buildLeaseReply(body, lease.leaseId, options.now());
      lease.renewalSequence = body.renewal_sequence;
      lease.identities = [...body.identities];
      lease.expiresAt = reply.lease_expires_at;
      lease.expiresAtMs = Date.parse(reply.lease_expires_at);
      state.acceptedRenewals += 1;
      identityHistory.push([...body.identities]);
      idempotency.set(idemKey, {
        fingerprint,
        status: 200,
        body: reply,
      });
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify(reply),
      });
      return;
    }

    const match = new RegExp("^" + INTRADAY_STREAM_LEASE_PATH + "/([0-9a-f-]+)$").exec(
      url.pathname,
    );
    if (request.method() !== "DELETE" || match === null || url.origin !== appOrigin) {
      state.rejectedReleases += 1;
      await rejectMutation(route, 404, "RESOURCE_NOT_FOUND");
      return;
    }
    state.releaseAttempts += 1;
    const release = intradayStreamReleaseRequestSchema.safeParse(input);
    const leaseId = match[1] ?? "";
    if (!release.success) {
      state.rejectedReleases += 1;
      await rejectMutation(route, 400, "INVALID_REQUEST");
      return;
    }
    const releaseKey = "DELETE:" + idempotencyKey;
    const releaseFingerprint = leaseId + ":" + JSON.stringify(release.data);
    const replay = idempotency.get(releaseKey);
    if (replay !== undefined) {
      if (replay.fingerprint !== releaseFingerprint) {
        state.rejectedReleases += 1;
        await rejectMutation(route, 409, "STREAM_LEASE_SEQUENCE_CONFLICT");
        return;
      }
      await route.fulfill({
        status: replay.status,
        contentType: "application/json",
        body: JSON.stringify(replay.body),
      });
      return;
    }
    const lease = leasesById.get(leaseId);
    if (
      lease === undefined ||
      lease.released ||
      release.data.consumer_id !== lease.consumerId ||
      release.data.renewal_sequence !== lease.renewalSequence
    ) {
      state.rejectedReleases += 1;
      await rejectMutation(route, 404, "RESOURCE_NOT_FOUND");
      return;
    }
    const reply = intradayStreamReleaseResponseSchema.parse({
      schema_version: 2,
      lease_id: leaseId,
      released: true,
    });
    lease.released = true;
    state.acceptedReleases += 1;
    idempotency.set(releaseKey, {
      fingerprint: releaseFingerprint,
      status: 200,
      body: reply,
    });
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(reply),
    });
  }

  async function installPageRoutes(page: Page): Promise<void> {
    const legacyHandler = async (route: Route): Promise<void> => {
      const url = new URL(route.request().url());
      if (url.origin === appOrigin) {
        if (url.pathname.includes("quote-demands")) state.quoteDemandMutations += 1;
        else state.legacyQuoteGets += 1;
        await rejectMutation(route, 599, "LEGACY_QUOTE_ROUTE_DISABLED");
        return;
      }
      await route.fallback();
    };
    const leaseHandler = async (route: Route): Promise<void> => {
      const url = new URL(route.request().url());
      if (url.origin !== appOrigin || !url.pathname.startsWith(INTRADAY_STREAM_LEASE_PATH)) {
        await route.fallback();
        return;
      }
      await handleLeaseMutation(route);
    };
    const streamHandler = async (route: Route): Promise<void> => {
      const url = new URL(route.request().url());
      if (url.origin !== appOrigin || url.pathname !== STREAM_PATH) {
        await route.fallback();
        return;
      }
      const params = [...url.searchParams.entries()];
      if (params.length !== 1 || params[0]?.[0] !== "lease_id") {
        state.rejectedReadProbes += 1;
        await rejectMutation(route, 404, "RESOURCE_NOT_FOUND");
        return;
      }
      const leaseId = params[0]?.[1] ?? "";
      if (!UUID_PATTERN.test(leaseId)) {
        state.rejectedReadProbes += 1;
        await rejectMutation(route, 404, "RESOURCE_NOT_FOUND");
        return;
      }
      const lease = leasesById.get(leaseId);
      if (lease === undefined || lease.released || options.now() >= lease.expiresAtMs) {
        state.rejectedReadProbes += 1;
        await rejectMutation(route, 404, "RESOURCE_NOT_FOUND");
        return;
      }
      try {
        const current = await currentReadyIdentities();
        if (!sameIdentitySet(canonicalStreamIdentities(lease.identities), current)) {
          state.rejectedReadProbes += 1;
          await rejectMutation(route, 404, "RESOURCE_NOT_FOUND");
          return;
        }
      } catch {
        state.rejectedReadProbes += 1;
        await rejectMutation(route, 404, "RESOURCE_NOT_FOUND");
        return;
      }
      await route.continue({
        url: origin + "/market-stream?lease_id=" + encodeURIComponent(leaseId),
      });
    };
    const registrations: readonly [string, (route: Route) => Promise<void>][] = [
      ["**/quote-demands**", legacyHandler],
      ["**/instruments/**/quote**", legacyHandler],
      ["**" + INTRADAY_STREAM_LEASE_PATH + "**", leaseHandler],
      ["**" + STREAM_PATH + "**", streamHandler],
    ];
    for (const [pattern, handler] of registrations) {
      await page.route(pattern, handler);
      routeRegistrations.push({ page, pattern, handler });
    }
  }

  function emit(connectionId: string, kind: FrameKind, body: unknown): SentFrame {
    const connection = connections.get(connectionId);
    if (connection === undefined || connection.closedAt !== null) {
      throw new Error("The requested owned SSE connection is not active");
    }
    const lease = leasesById.get(connection.leaseId);
    if (lease === undefined || lease.released) {
      throw new Error("The requested owned SSE lease is not active");
    }
    const eventSequence = String(connection.sequence + 1);
    const serverTime = iso(options.now());
    const envelope = {
      schema_version: 2,
      stream_id: connection.streamId,
      event_sequence: eventSequence,
      server_time: serverTime,
      body,
    };
    const data = JSON.stringify(envelope);
    const eventId = connection.streamId + ":" + eventSequence;
    const parsed = parseIntradayStreamEvent(kind, data, eventId);
    if (parsed.kind === "snapshot") {
      const rows = parsed.body.rows.map((row) => ({
        membership_id: row.membership_id,
        instrument_id: row.instrument_id,
        generation: row.generation,
      }));
      if (!sameIdentitySet(rows, canonicalStreamIdentities(lease.identities))) {
        throw new IntradayStreamContractError();
      }
    }
    if (parsed.kind === "delta") {
      if (
        parsed.body.rows.some(
          (row) =>
            !lease.identities.some(
              (identity) =>
                identity.membership_id === row.membership_id &&
                identity.instrument_id === row.instrument_id &&
                identity.generation === row.generation,
            ),
        )
      ) {
        throw new IntradayStreamContractError();
      }
    }
    connection.sequence += 1;
    connection.sent.push({
      kind,
      eventId,
      streamId: connection.streamId,
      eventSequence,
      serverTime,
      invalid: false,
    });
    if (kind === "snapshot") state.snapshots += 1;
    else if (kind === "delta") state.deltas += 1;
    else if (kind === "status") state.statuses += 1;
    else state.resets += 1;
    connection.response.write("event: " + kind + "\nid: " + eventId + "\ndata: " + data + "\n\n");
    return connection.sent[connection.sent.length - 1] as SentFrame;
  }

  function emitInvalid(connectionId: string, frame: InvalidFrame, eventId: string): SentFrame {
    const connection = connections.get(connectionId);
    if (connection === undefined || connection.closedAt !== null) {
      throw new Error("The requested owned SSE connection is not active");
    }
    if (
      typeof frame !== "object" ||
      frame === null ||
      !["snapshot", "delta", "status", "reset"].includes(frame.kind) ||
      typeof frame.data !== "string" ||
      frame.data.length > 256 * 1024
    ) {
      throw new IntradayStreamContractError();
    }
    const parsedSequence = /:([1-9][0-9]*)$/.exec(eventId)?.[1] ?? "";
    if (parsedSequence.length === 0) throw new IntradayStreamContractError();
    connection.sequence = Math.max(connection.sequence, Number(parsedSequence));
    const metadata: SentFrame = {
      kind: frame.kind,
      eventId,
      streamId: connection.streamId,
      eventSequence: parsedSequence,
      serverTime: iso(options.now()),
      invalid: true,
    };
    connection.sent.push(metadata);
    if (frame.kind === "snapshot") {
      state.snapshots += 1;
      state.invalidSnapshots += 1;
    } else if (frame.kind === "delta") {
      state.deltas += 1;
      state.invalidDeltas += 1;
    } else if (frame.kind === "status") {
      state.statuses += 1;
      state.invalidStatuses += 1;
    } else {
      state.resets += 1;
      state.invalidResets += 1;
    }
    connection.response.write(
      "event: " + frame.kind + "\nid: " + eventId + "\ndata: " + frame.data + "\n\n",
    );
    return metadata;
  }

  function disconnect(connectionId: string): void {
    const connection = connections.get(connectionId);
    if (connection === undefined || connection.closedAt !== null) return;
    connection.response.end();
  }

  function failNextLease(failure: Failure): void {
    if (
      !(
        (failure.status === 409 && failure.code === "STREAM_LEASE_CAPACITY") ||
        (failure.status === 503 && failure.code === "MARKET_STREAM_UNAVAILABLE")
      )
    ) {
      throw new IntradayStreamContractError();
    }
    nextFailure = { status: failure.status, code: failure.code };
  }

  function leaseLedger(): readonly LeaseLedgerEntry[] {
    return [...leasesById.values()].map((lease) => ({
      leaseId: lease.leaseId,
      consumerId: lease.consumerId,
      renewalSequence: lease.renewalSequence,
      identities: structuredClone(lease.identities),
      expiresAt: lease.expiresAt,
      released: lease.released,
    }));
  }

  function connectionLedger(): readonly ConnectionLedgerEntry[] {
    return [...connections.values()].map((connection) => ({
      id: connection.id,
      leaseId: connection.leaseId,
      streamId: connection.streamId,
      openedAt: connection.openedAt,
      closedAt: connection.closedAt,
      sent: structuredClone(connection.sent),
    }));
  }

  function metrics() {
    const activeLeases = [...leasesById.values()].filter((lease) => !lease.released);
    const activeIdentityMap = new Map<string, IntradayStreamIdentity>();
    for (const lease of activeLeases) {
      for (const identity of lease.identities) {
        activeIdentityMap.set(identity.membership_id, identity);
      }
    }
    const activeIdentities = [...activeIdentityMap.values()];
    return {
      ...state,
      activeLeases: activeLeases.length,
      activeIdentityCount: activeIdentityMap.size,
      activeIdentitySet:
        activeIdentities.length === 0 ? [] : canonicalStreamIdentities(activeIdentities),
      successfulRequestIdentitySets: structuredClone(identityHistory),
      connections: connections.size,
      sourcesOpened: state.getOpens,
      sourcesClosed: state.getCloses,
    };
  }

  async function close(): Promise<void> {
    if (closePromise !== null) return closePromise;
    closePromise = (async () => {
      for (const registration of routeRegistrations) {
        await registration.page
          .unroute(registration.pattern, registration.handler)
          .catch(() => undefined);
      }
      routeRegistrations.length = 0;
      for (const connection of connections.values()) {
        if (connection.closedAt === null) connection.response.destroy();
      }
      await new Promise<void>((resolve) => {
        if (!server.listening) {
          resolve();
          return;
        }
        server.close(() => resolve());
      });
    })();
    return closePromise;
  }

  return {
    origin,
    installPageRoutes,
    leaseLedger,
    connectionLedger,
    emit,
    emitInvalid,
    disconnect,
    failNextLease,
    metrics,
    close,
  };
}
