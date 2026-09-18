import assert from "node:assert/strict";
import test from "node:test";

import {
  API_PATHS,
  APPROVED_INSTRUMENTS,
  LiveAcceptanceError,
  createPlaywrightBoundary,
  disposeAttachedBrowser,
  onboardingIdempotencyKey,
  parseCli,
  runAcceptance,
} from "./owner-equity-v2-live-acceptance.mjs";

const ORIGIN = "https://owner.example.test";
const BASE_MS = Date.parse("2026-09-18T01:00:30.000Z");
const SESSION_ID = "00000000-0000-4000-8000-000000000001";
const CALENDAR_HASH = "a".repeat(64);
const WINDOW_HASH = `sha256:${"b".repeat(64)}`;

function uuidFor(seed) {
  const digest = Buffer.from(seed).toString("hex").padEnd(32, "0").slice(0, 32);
  return `${digest.slice(0, 8)}-${digest.slice(8, 12)}-4${digest.slice(13, 16)}-8${digest.slice(17, 20)}-${digest.slice(20, 32)}`;
}

function membershipId(instrumentId) {
  return uuidFor(`membership:${instrumentId}`);
}

function jobId(instrumentId) {
  return uuidFor(`job:${instrumentId}`);
}

function coverage() {
  return {
    observed_sessions: 261,
    target_observed_sessions: 261,
    minimum_observed_sessions: 121,
  };
}

function makeMembership(instrumentId, overrides = {}) {
  const now = "2026-09-18T01:00:00.000Z";
  return {
    id: membershipId(instrumentId),
    instrument_id: instrumentId,
    lifecycle: "READY",
    generation: 1,
    coverage: coverage(),
    requested_at: now,
    updated_at: now,
    ...overrides,
  };
}

function policyFor(memberships) {
  const active = memberships.filter((membership) => membership.lifecycle !== "DISABLED").length;
  return {
    active_instruments: active,
    max_active_instruments: 100,
    minimum_observed_sessions: 121,
    remaining_capacity: 100 - active,
    target_observed_sessions: 261,
  };
}

function sessionFor(role = "owner") {
  return {
    user_id: SESSION_ID,
    role,
    expires_at_secs: Math.floor(BASE_MS / 1_000) + 3_600,
    auth_time_secs: Math.floor(BASE_MS / 1_000) - 60,
    owner_beta_access_mode: "owner_only",
    owner_beta_paper_mode: "disabled",
  };
}

function sessionEvidence() {
  return {
    calendar_content_sha256: CALENDAR_HASH,
    calendar_source: "kis",
    calendar_source_version: "kis-chk-holiday-v1:schema-1",
    date: "2026-09-18",
    timezone: "Asia/Seoul",
    window_contract_sha256: WINDOW_HASH,
  };
}

function timestamp(ms) {
  return new Date(ms).toISOString();
}

function quotePayload(version, atMs, price = "100.00") {
  const at = timestamp(atMs);
  return {
    base_price: "99.00",
    change_from_previous_day: "1.00",
    change_percent_from_previous_day: "1.01010101",
    direction: "UP",
    last_success_at: at,
    price,
    quote_version: String(version),
    received_at: at,
  };
}

function quoteResponse(identity, {
  active = true,
  version = 10,
  atMs = BASE_MS,
  freshness = "RECENT",
  reasonCode = active ? null : "NO_ACTIVE_DEMAND",
  wrongIdentity = false,
  price = "100.00",
  noQuote = false,
} = {}) {
  const responseIdentity = wrongIdentity
    ? {
        membership_id: uuidFor("wrong-membership"),
        instrument_id: APPROVED_INSTRUMENTS[1],
        generation: identity.generation,
      }
    : identity;
  return {
    schema_version: 1,
    membership_id: responseIdentity.membership_id,
    instrument_id: responseIdentity.instrument_id,
    venue: "KRX",
    currency: "KRW",
    generation: responseIdentity.generation,
    session: sessionEvidence(),
    market_state: "OPEN",
    freshness,
    reason_code: noQuote ? "QUOTE_PENDING" : reasonCode,
    quote: noQuote ? null : quotePayload(version, atMs, price),
    next_poll_after_ms: 5_000,
  };
}

class TestClock {
  constructor(start = BASE_MS) {
    this.current = start;
    this.sleeps = [];
  }

  now() {
    return this.current;
  }

  async sleep(ms) {
    assert.ok(
      Number.isSafeInteger(ms) && ms > 0,
      "synthetic clock must advance positive modeled time",
    );
    this.sleeps.push(ms);
    this.current += ms;
  }

  observe(atMs) {
    // Keep synthetic HTTP receipt timestamps behind the request's observed
    // clock. The real API cannot return a future last_success_at.
    this.current = Math.max(this.current, atMs + 1_000);
  }
}

class FakeBoundary {
  constructor({
    initialMemberships = [],
    role = "owner",
    authLossOnMembershipRead = false,
    addFailureFor = undefined,
    quoteMode = "pass",
    closeFailure = false,
    cleanupFailure = false,
  } = {}) {
    this.origin = ORIGIN;
    this.role = role;
    this.authLossOnMembershipRead = authLossOnMembershipRead;
    this.addFailureFor = addFailureFor;
    this.quoteMode = quoteMode;
    this.closeFailure = closeFailure;
    this.cleanupFailure = cleanupFailure;
    this.clock = null;
    this.memberships = new Map(initialMemberships.map((membership) => [membership.instrument_id, structuredClone(membership)]));
    this.requests = [];
    this.added = [];
    this.retried = [];
    this.openedQuotePages = 0;
    this.closedQuotePages = 0;
    this.widgetDemandCreated = 0;
    this.widgetDemandReleased = 0;
    this.activeDemand = false;
    this.ownedQuotePageOpened = false;
    this.baselineQuoteIndex = 0;
    this.activeQuoteIndex = 0;
    this.cleanupQuoteIndex = 0;
    this.quotePhases = [];
    this.cleanupReasons = [];
    this.openCalledAt = null;
    this.dom = null;
  }

  record(request) {
    this.requests.push({
      method: request.method,
      path: request.path,
      body: request.body === undefined ? undefined : structuredClone(request.body),
      headers: { ...(request.headers ?? {}) },
    });
  }

  membershipsBody() {
    const memberships = [...this.memberships.values()];
    return { policy: policyFor(memberships), memberships };
  }

  error(status, code) {
    return { status, body: { error: { code } } };
  }

  retainedStale(identity) {
    return quoteResponse(identity, {
      freshness: "STALE",
      reasonCode: "NO_ACTIVE_DEMAND",
      version: 10,
      atMs: BASE_MS - 1_000,
    });
  }

  baselineSequence(identity) {
    if (
      ["no-baseline-old-only", "no-baseline-old-then-valid", "not-ready-then-valid"].includes(
        this.quoteMode,
      )
    ) {
      return [quoteResponse(identity, { noQuote: true })];
    }
    if (["retained-stale-then-valid", "stale"].includes(this.quoteMode)) {
      return [this.retainedStale(identity)];
    }
    return [quoteResponse(identity, { version: 10, atMs: BASE_MS })];
  }

  activeSequence(identity) {
    if (this.quoteMode === "no-baseline-old-only") {
      return [quoteResponse(identity, { version: 10, atMs: BASE_MS - 1_000 })];
    }
    if (this.quoteMode === "no-baseline-old-then-valid") {
      return [
        quoteResponse(identity, { version: 10, atMs: BASE_MS - 1_000 }),
        quoteResponse(identity, { version: 11, atMs: BASE_MS + 1_000 }),
        quoteResponse(identity, { version: 11, atMs: BASE_MS + 1_000 }),
        quoteResponse(identity, { version: 12, atMs: BASE_MS + 2_000 }),
      ];
    }
    if (this.quoteMode === "not-ready-then-valid") {
      return [
        quoteResponse(identity, { noQuote: true }),
        quoteResponse(identity, { noQuote: true }),
        quoteResponse(identity, { version: 11, atMs: BASE_MS + 1_000 }),
        quoteResponse(identity, { version: 11, atMs: BASE_MS + 1_000 }),
        quoteResponse(identity, { version: 12, atMs: BASE_MS + 2_000 }),
      ];
    }
    if (this.quoteMode === "retained-stale-then-valid") {
      return [
        this.retainedStale(identity),
        this.retainedStale(identity),
        quoteResponse(identity, { version: 11, atMs: BASE_MS + 1_000 }),
        quoteResponse(identity, { version: 11, atMs: BASE_MS + 1_000 }),
        quoteResponse(identity, { version: 12, atMs: BASE_MS + 2_000 }),
      ];
    }
    if (this.quoteMode === "stale") return [this.retainedStale(identity)];

    const sequence = [
      quoteResponse(identity, { version: 10, atMs: BASE_MS }),
      quoteResponse(identity, { version: 10, atMs: BASE_MS }),
      quoteResponse(identity, { version: 11, atMs: BASE_MS + 1_000 }),
      quoteResponse(identity, { version: 11, atMs: BASE_MS + 1_000 }),
      quoteResponse(identity, { version: 12, atMs: BASE_MS + 2_000 }),
    ];
    if (this.quoteMode === "mismatch") sequence[0] = quoteResponse(identity, { wrongIdentity: true });
    if (this.quoteMode === "reason") sequence[0] = quoteResponse(identity, { reasonCode: "QUOTE_PENDING" });
    if (this.quoteMode === "budget-retry") {
      sequence[0] = quoteResponse(identity, { reasonCode: "QUOTE_BUDGET_EXHAUSTED" });
    }
    if (this.quoteMode === "terminal") sequence[0] = quoteResponse(identity, { reasonCode: "SESSION_CLOSED" });
    if (this.quoteMode === "time-backward") sequence[0] = quoteResponse(identity, { version: 11, atMs: BASE_MS - 1_000 });
    if (this.quoteMode === "version-backward") sequence[0] = quoteResponse(identity, { version: 9, atMs: BASE_MS });
    if (this.quoteMode === "same-version-mutation") sequence[0] = quoteResponse(identity, { price: "101.00" });
    if (this.quoteMode === "dom-mismatch") {
      sequence[0] = quoteResponse(identity, { version: 11, atMs: BASE_MS + 1_000, price: "101.00" });
    }
    return sequence;
  }

  cleanupSequence(identity) {
    if (this.quoteMode === "late-cleanup-result") {
      return [
        quoteResponse(identity, { active: false, version: 12, atMs: BASE_MS + 2_000 }),
        quoteResponse(identity, { active: true, version: 13, atMs: BASE_MS + 3_000 }),
        quoteResponse(identity, { active: false, version: 13, atMs: BASE_MS + 3_000 }),
      ];
    }
    return [
      quoteResponse(identity, { active: true, version: 99, atMs: BASE_MS }),
      quoteResponse(identity, { active: false, version: 12, atMs: BASE_MS + 2_000 }),
    ];
  }

  async request(request) {
    this.record(request);
    const { method, path, body } = request;
    if (method === "GET" && path === API_PATHS.session) return { status: 200, body: sessionFor(this.role) };
    if (method === "GET" && path === API_PATHS.csrf) return { status: 200, body: { csrf_token: "synthetic-csrf" } };
    if (method === "GET" && path === API_PATHS.memberships) {
      if (this.authLossOnMembershipRead) return this.error(401, "SESSION_EXPIRED");
      return { status: 200, body: this.membershipsBody() };
    }
    if (method === "POST" && path === API_PATHS.memberships) {
      const instrumentId = `${body.instrument_code}.KRX`;
      if (this.addFailureFor === instrumentId) return this.error(409, "OWNER_EQUITY_CAPACITY_EXCEEDED");
      assert.equal(request.headers["X-CSRF-Token"], "synthetic-csrf");
      assert.equal(request.headers["Content-Type"], "application/json");
      this.added.push({ instrumentId, idempotencyKey: request.headers["Idempotency-Key"] });
      const membership = makeMembership(instrumentId);
      this.memberships.set(instrumentId, membership);
      return { status: 202, body: { resource: membership, job_id: jobId(instrumentId), duplicate_active: false } };
    }
    if (method === "POST" && path.endsWith("/retry")) {
      const membership = [...this.memberships.values()].find((candidate) => path.endsWith(`/${candidate.id}/retry`));
      assert.ok(membership);
      assert.equal(request.headers["X-CSRF-Token"], "synthetic-csrf");
      this.retried.push({ instrumentId: membership.instrument_id, idempotencyKey: request.headers["Idempotency-Key"] });
      const ready = makeMembership(membership.instrument_id);
      this.memberships.set(ready.instrument_id, ready);
      return { status: 202, body: { resource: ready, job_id: jobId(`${ready.instrument_id}:retry`), duplicate_active: false } };
    }
    if (method === "GET" && path.startsWith(`${API_PATHS.memberships}/`)) {
      const id = decodeURIComponent(path.slice(`${API_PATHS.memberships}/`.length));
      const membership = [...this.memberships.values()].find((candidate) => candidate.id === id);
      assert.ok(membership);
      return { status: 200, body: { policy: policyFor([...this.memberships.values()]), membership } };
    }
    if (path.includes("quote-demands")) assert.fail("manual quote demand mutation is forbidden");
    if (method === "GET" && path.startsWith(`${API_PATHS.quoteCache}/`)) {
      const url = new URL(path, ORIGIN);
      const instrumentId = decodeURIComponent(
        url.pathname.slice(`${API_PATHS.quoteCache}/`.length, url.pathname.lastIndexOf("/quote")),
      );
      const membership = this.memberships.get(instrumentId);
      assert.ok(membership);
      const identity = {
        membership_id: url.searchParams.get("membership_id"),
        instrument_id: instrumentId,
        generation: Number(url.searchParams.get("generation")),
      };
      const phase = !this.ownedQuotePageOpened
        ? "baseline"
        : this.activeDemand
          ? "active"
          : "cleanup";
      const baselineValues = this.baselineSequence(identity);
      const activeValues = this.activeSequence(identity);
      const cleanupValues = this.cleanupSequence(identity);
      const sequence = phase === "baseline"
        ? baselineValues[Math.min(this.baselineQuoteIndex++, baselineValues.length - 1)]
        : phase === "active"
          ? activeValues[Math.min(this.activeQuoteIndex++, activeValues.length - 1)]
          : cleanupValues[Math.min(this.cleanupQuoteIndex++, cleanupValues.length - 1)];
      this.quotePhases.push(phase);
      if (phase === "cleanup") this.cleanupReasons.push(sequence.reason_code);
      if (this.clock !== null && sequence.quote !== null) this.clock.observe(Date.parse(sequence.quote.last_success_at));
      this.dom = sequence.quote === null
        ? null
        : {
            instrument_id: sequence.instrument_id,
            last_success_at: sequence.quote.last_success_at,
            price:
              this.quoteMode === "dom-mismatch" && phase === "active"
                ? "999.00"
                : sequence.quote.price,
            status_phase: "ready",
          };
      if (this.cleanupFailure && phase === "cleanup") throw new Error("synthetic cleanup transport failure");
      return { status: 200, body: sequence };
    }
    throw new Error(`unexpected fake route ${method} ${path}`);
  }

  async openOwnedQuotePage(instrumentId) {
    this.openedQuotePages += 1;
    this.openCalledAt = this.clock?.now() ?? null;
    this.ownedQuotePageOpened = true;
    this.widgetDemandCreated += 1;
    this.activeDemand = true;
    const expectedInstrument = instrumentId;
    let closed = false;
    return {
      readDom: async (requestedInstrument) => {
        assert.equal(requestedInstrument, expectedInstrument);
        assert.equal(this.activeDemand, true);
        return this.dom;
      },
      close: async () => {
        if (closed) return { closed: true };
        if (this.closeFailure) throw new Error("synthetic page close failure");
        closed = true;
        this.activeDemand = false;
        this.widgetDemandReleased += 1;
        this.closedQuotePages += 1;
        return { closed: true };
      },
    };
  }
}

function readyExceptPilot() {
  return APPROVED_INSTRUMENTS.slice(1).map((instrumentId) => makeMembership(instrumentId));
}

function runClock(boundary, start = BASE_MS) {
  const value = new TestClock(start);
  boundary.clock = value;
  return value;
}

async function runApply(boundary, options = {}) {
  const testClock = boundary.clock ?? runClock(boundary);
  return runAcceptance({
    mode: "apply",
    browser: boundary,
    clock: testClock,
    domWaitMs: 0,
    lifecycleTimeoutMs: 1_000,
    observationTimeoutMs: 90_000,
    pollIntervalMs: 5_000,
    quiescenceTimeoutMs: 60_000,
    ...options,
  });
}

test("plan is the default, reads exactly the approved thirty, and performs no mutation", async () => {
  const boundary = new FakeBoundary();
  const report = await runAcceptance({ mode: "plan", browser: boundary, clock: runClock(boundary) });

  assert.equal(report.status, "PLAN_READY");
  assert.equal(report.universe.count, 30);
  assert.equal(report.plan.pilot.instrument_id, APPROVED_INSTRUMENTS[0]);
  assert.deepEqual(
    boundary.requests.map(({ method, path }) => [method, path]),
    [["GET", API_PATHS.session], ["GET", API_PATHS.memberships]],
  );
  assert.equal(boundary.openedQuotePages, 0);
});

test("SYNTHETIC apply uses pilot-first sequential onboarding, captures a pre-open baseline, and creates one widget demand", async () => {
  const boundary = new FakeBoundary({ initialMemberships: readyExceptPilot() });
  const report = await runApply(boundary);

  assert.equal(report.status, "APPLIED");
  assert.equal(report.onboarding.completed_count, 30);
  assert.deepEqual(boundary.added.map(({ instrumentId }) => instrumentId), [APPROVED_INSTRUMENTS[0]]);
  assert.equal(boundary.openedQuotePages, 1);
  assert.equal(boundary.closedQuotePages, 1);
  assert.equal(boundary.widgetDemandCreated, 1);
  assert.equal(boundary.widgetDemandReleased, 1);
  assert.equal(boundary.quotePhases[0], "baseline");
  assert.equal(boundary.openCalledAt, BASE_MS + 1_000);
  assert.equal(boundary.quotePhases.includes("baseline"), true);
  assert.equal(boundary.cleanupReasons[0], null);
  assert.ok(boundary.cleanupReasons.slice(1).every((reason) => reason === "NO_ACTIVE_DEMAND"));
  assert.equal(boundary.requests.some(({ path }) => path.includes("quote-demands")), false);
  assert.equal(report.receipt.quiescent, true);
  assert.ok(report.receipt.quiescence_samples >= 7);
  assert.ok(report.receipt.quiescence_stable_ms >= 30_000);
  assert.equal(report.receipt.owned_page_closed, true);
  assert.equal(report.receipt.cleanup_status, "verified");
  assert.ok(boundary.clock.sleeps.every((ms) => ms > 0));
  assert.equal(
    onboardingIdempotencyKey("add", APPROVED_INSTRUMENTS[0]),
    onboardingIdempotencyKey("add", APPROVED_INSTRUMENTS[0]),
  );
  assert.match(boundary.added[0].idempotencyKey, /^owner-equity-v2-live-v1-add-005930$/);
});

test("SYNTHETIC unchanged baseline and last-seen polls are ignored; equal prices still need advancing receipts after the cutoff", async () => {
  const boundary = new FakeBoundary({ initialMemberships: readyExceptPilot() });
  const report = await runApply(boundary);

  assert.equal(report.status, "APPLIED");
  assert.deepEqual(report.receipt.receipts.map(({ version }) => version), ["11", "12"]);
  assert.equal(report.receipt.receipt_count, 2);
  assert.ok(report.receipt.ignored_repetitions >= 2);
  assert.ok(Date.parse(report.receipt.receipts[0].timestamp) > Date.parse(report.receipt.observation_started_at));
  assert.ok(Date.parse(report.receipt.receipts[1].timestamp) > Date.parse(report.receipt.receipts[0].timestamp));
});

test("SYNTHETIC fresh cached quote before observation cannot pass when baseline is absent", async () => {
  const boundary = new FakeBoundary({
    initialMemberships: readyExceptPilot(),
    quoteMode: "no-baseline-old-only",
  });
  const report = await runApply(boundary, { observationTimeoutMs: 5 });

  assert.equal(report.status, "BLOCKED");
  assert.equal(report.error.code, "RECEIPT_TIMEOUT");
  assert.equal(report.receipt.receipt_count, 0);
  assert.ok(report.receipt.ignored_before_start > 0);
  assert.equal(report.receipt.owned_page_closed, true);
  assert.equal(report.receipt.quiescent, true);
});

test("SYNTHETIC retained stale, NO_ACTIVE_DEMAND, and not-ready cache states wait for post-start receipts", async (t) => {
  for (const quoteMode of [
    "retained-stale-then-valid",
    "no-baseline-old-then-valid",
    "not-ready-then-valid",
    "reason",
    "budget-retry",
  ]) {
    await t.test(quoteMode, async () => {
      const boundary = new FakeBoundary({ initialMemberships: readyExceptPilot(), quoteMode });
      const report = await runApply(boundary);

      assert.equal(report.status, "APPLIED");
      assert.deepEqual(report.receipt.receipts.map(({ version }) => version), ["11", "12"]);
      assert.ok(
        report.receipt.ignored_repetitions +
          report.receipt.ignored_non_receipts +
          report.receipt.ignored_before_start >
          0,
      );
      assert.ok(
        report.receipt.receipts.every(
          ({ timestamp }) => Date.parse(timestamp) > Date.parse(report.receipt.observation_started_at),
        ),
      );
    });
  }
});

test("owner mismatch and lost authentication are typed and redacted", async (t) => {
  await t.test("owner mismatch", async () => {
    const boundary = new FakeBoundary({ role: "member" });
    const report = await runAcceptance({ mode: "plan", browser: boundary, clock: runClock(boundary) });
    assert.equal(report.status, "BLOCKED");
    assert.equal(report.error.code, "OWNER_MISMATCH");
    assert.deepEqual(report.error.details, { phase: "authentication" });
  });
  await t.test("authentication loss", async () => {
    const boundary = new FakeBoundary({ authLossOnMembershipRead: true });
    const report = await runAcceptance({ mode: "plan", browser: boundary, clock: runClock(boundary) });
    assert.equal(report.status, "BLOCKED");
    assert.equal(report.error.code, "AUTH_LOST");
    assert.equal(report.error.details.api_code, "SESSION_EXPIRED");
    assert.equal("message" in report.error.details, false);
  });
});

test("partial enrolment stops without retrying or disabling the first successful symbol", async () => {
  const boundary = new FakeBoundary({ addFailureFor: APPROVED_INSTRUMENTS[1] });
  const report = await runApply(boundary);

  assert.equal(report.status, "BLOCKED");
  assert.equal(report.error.code, "OWNER_EQUITY_CAPACITY_EXCEEDED");
  assert.equal(report.onboarding.completed_count, 1);
  assert.deepEqual(boundary.added.map(({ instrumentId }) => instrumentId), [APPROVED_INSTRUMENTS[0]]);
  assert.deepEqual(boundary.retried, []);
  assert.equal(boundary.openedQuotePages, 0);
});

test("permanent FAILED membership stops without automatic retry", async () => {
  const pilot = APPROVED_INSTRUMENTS[0];
  const boundary = new FakeBoundary({
    initialMemberships: [
      makeMembership(pilot, {
        lifecycle: "FAILED",
        generation: 0,
        failure: { code: "BACKFILL_NOT_ALLOWED", retryable: false },
      }),
      ...readyExceptPilot().slice(0, 1),
    ],
  });
  const report = await runApply(boundary);

  assert.equal(report.status, "BLOCKED");
  assert.equal(report.error.code, "PERMANENT_FAILURE");
  assert.deepEqual(boundary.retried, []);
});

test("SYNTHETIC hard receipt violations stay typed and still clean up the owned widget", async (t) => {
  for (const [quoteMode, expectedCode] of [
    ["mismatch", "RECEIPT_IDENTITY_MISMATCH"],
    ["time-backward", "RECEIPT_TIME_REVERSED"],
    ["version-backward", "RECEIPT_VERSION_REVERSED"],
    ["same-version-mutation", "RECEIPT_SAME_VERSION_MUTATION"],
    ["terminal", "RECEIPT_TERMINAL_REASON"],
    ["dom-mismatch", "DOM_API_MISMATCH"],
  ]) {
    await t.test(quoteMode, async () => {
      const boundary = new FakeBoundary({ initialMemberships: readyExceptPilot(), quoteMode });
      const report = await runApply(boundary);

      assert.equal(report.status, "BLOCKED");
      assert.equal(report.error.code, expectedCode);
      assert.equal(boundary.openedQuotePages, 1);
      assert.equal(boundary.closedQuotePages, 1);
      assert.equal(boundary.widgetDemandReleased, 1);
      assert.equal(report.receipt.owned_page_closed, true);
      assert.equal(report.receipt.quiescent, true);
    });
  }
});

test("SYNTHETIC persistent retained stale data is never counted and remains bounded", async () => {
  const boundary = new FakeBoundary({ initialMemberships: readyExceptPilot(), quoteMode: "stale" });
  const report = await runApply(boundary, { observationTimeoutMs: 15_000 });

  assert.equal(report.status, "BLOCKED");
  assert.equal(report.error.code, "RECEIPT_TIMEOUT");
  assert.equal(report.receipt.receipt_count, 0);
  assert.ok(report.receipt.ignored_repetitions > 0);
  assert.equal(report.receipt.owned_page_closed, true);
  assert.equal(report.receipt.quiescent, true);
  assert.ok(report.receipt.quiescence_stable_ms >= 30_000);
});

test("SYNTHETIC a late post-close result resets the required stable-time window", async () => {
  const boundary = new FakeBoundary({
    initialMemberships: readyExceptPilot(),
    quoteMode: "late-cleanup-result",
  });
  const report = await runApply(boundary);

  assert.equal(report.status, "APPLIED");
  assert.deepEqual(boundary.cleanupReasons.slice(0, 3), ["NO_ACTIVE_DEMAND", null, "NO_ACTIVE_DEMAND"]);
  assert.ok(report.receipt.quiescence_stable_ms >= 30_000);
  assert.ok(
    boundary.cleanupReasons.filter((reason) => reason === "NO_ACTIVE_DEMAND").length >= 8,
  );
});

test("SYNTHETIC primary receipt failure is not replaced by an unverified page cleanup", async () => {
  const boundary = new FakeBoundary({
    initialMemberships: readyExceptPilot(),
    quoteMode: "terminal",
    closeFailure: true,
  });
  const report = await runApply(boundary);

  assert.equal(report.status, "BLOCKED");
  assert.equal(report.error.code, "RECEIPT_TERMINAL_REASON");
  assert.equal(report.receipt.cleanup_status, "unverified");
  assert.equal(report.receipt.owned_page_closed, false);
  assert.equal(report.receipt.quiescent, false);
});

test("foreign API redirects are rejected by redirect:error and foreign navigation is aborted before network", async () => {
  let evaluatedSource = "";
  let originalClosed = false;
  let ownedClosed = false;
  let foreignNetworkSent = false;
  const ownedPage = {
    currentUrl: "about:blank",
    routeHandler: null,
    url: () => ownedPage.currentUrl,
    evaluate: async () => null,
    isClosed: () => ownedClosed,
    close: async () => { ownedClosed = true; },
    route: async (_pattern, handler) => { ownedPage.routeHandler = handler; },
    goto: async () => {
      const foreignRequest = {
        url: () => "https://foreign.example.test/login",
        isNavigationRequest: () => true,
        redirectedFrom: () => ({ url: () => `${ORIGIN}/stock-beta/005930.KRX` }),
      };
      const route = {
        request: () => foreignRequest,
        abort: async () => {},
        continue: async () => { foreignNetworkSent = true; },
      };
      await ownedPage.routeHandler(route);
      throw new Error("navigation aborted");
    },
  };
  const page = {
    url: () => `${ORIGIN}/owner`,
    context: () => ({ newPage: async () => ownedPage }),
    evaluate: async (fn) => {
      evaluatedSource = fn.toString();
      return { status: 200, redirected: false, url: `${ORIGIN}/api/v1/auth/session`, body: null };
    },
    close: async () => { originalClosed = true; },
  };
  const boundary = createPlaywrightBoundary({ page, origin: ORIGIN });
  await boundary.request({ method: "GET", path: API_PATHS.session, headers: {} });
  assert.match(evaluatedSource, /redirect:\s*["']error["']/);
  await assert.rejects(
    boundary.openOwnedQuotePage(APPROVED_INSTRUMENTS[0]),
    (error) => error instanceof LiveAcceptanceError && error.code === "WRONG_ORIGIN",
  );
  assert.equal(foreignNetworkSent, false);
  assert.equal(ownedClosed, true);
  assert.equal(originalClosed, false);
});

test("Playwright CDP disposal uses supported Browser.close and never calls a nonexistent disconnect", async () => {
  let connected = true;
  let closeCount = 0;
  let disconnectCount = 0;
  let closeOptions;
  const browser = {
    isConnected: () => connected,
    close: async (options) => {
      closeCount += 1;
      closeOptions = options;
      connected = false;
    },
    disconnect: () => { disconnectCount += 1; },
  };
  assert.deepEqual(await disposeAttachedBrowser(browser), { detached: true });
  assert.equal(closeCount, 1);
  assert.equal(disconnectCount, 0);
  assert.equal(closeOptions.reason, "WP-2 CDP client detach");
  await assert.rejects(
    disposeAttachedBrowser({ isConnected: () => true, disconnect: () => {} }),
    (error) => error instanceof LiveAcceptanceError && error.code === "PREREQUISITE_PLAYWRIGHT_UNAVAILABLE",
  );
});

test("CLI requires explicit attachment inputs and documents the bounded default plan", () => {
  assert.equal(parseCli(["--help"]).help, true);
  assert.throws(
    () => parseCli(["--plan", "--apply", "--cdp-url", "http://127.0.0.1:9222", "--origin", ORIGIN]),
    (error) => error instanceof LiveAcceptanceError && error.code === "CLI_ARGUMENT",
  );
  assert.throws(
    () => parseCli(["--apply", "--origin", ORIGIN]),
    (error) => error instanceof LiveAcceptanceError && error.code === "PREREQUISITE_BROWSER_CONTEXT",
  );
  assert.throws(
    () =>
      parseCli([
        "--plan",
        "--cdp-url",
        "http://127.0.0.1:9222",
        "--origin",
        ORIGIN,
        "--poll-ms",
        "0",
      ]),
    (error) => error instanceof LiveAcceptanceError && error.code === "CLI_ARGUMENT",
  );
});
