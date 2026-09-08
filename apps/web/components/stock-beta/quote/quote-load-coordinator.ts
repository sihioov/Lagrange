import {
  createIntradayQuoteClient,
  IntradayQuoteApiError,
  type IntradayQuoteClient,
} from "@/lib/products/intraday-quotes-client";
import {
  INTRADAY_QUOTE_CACHE_MAX_AGE_MS,
  INTRADAY_QUOTE_DEMAND_PATH,
  INTRADAY_QUOTE_POLL_INTERVAL_MS,
  INTRADAY_QUOTE_RENEWAL_INTERVAL_MS,
  INTRADAY_QUOTE_STALE_AFTER_MS,
  type IntradayQuoteDemandRequest,
  type IntradayQuoteDemandResponse,
  type IntradayQuoteIdentity,
  type IntradayQuoteReasonCode,
  type IntradayQuoteResponse,
  intradayQuoteSessionKey,
  intradayQuoteVersion,
  isIntradayQuoteIdentityMatch,
} from "@/lib/products/intraday-quotes-contracts";

export type IntradayQuoteClock = {
  readonly now: () => number;
  readonly setTimeout: (callback: () => void, delayMs: number) => unknown;
  readonly clearTimeout: (handle: unknown) => void;
};

const systemClock: IntradayQuoteClock = {
  now: () => Date.now(),
  setTimeout: (callback, delayMs) => setTimeout(callback, delayMs),
  clearTimeout: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
};

export type IntradayQuoteLoadPhase =
  | "idle"
  | "demanding"
  | "polling"
  | "ready"
  | "stale"
  | "unavailable"
  | "offline"
  | "error";

export type IntradayQuoteLoadState = {
  readonly consumerId: string | null;
  readonly errorCode: string | null;
  readonly identity: IntradayQuoteIdentity | null;
  readonly lastSuccessAt: string | null;
  readonly phase: IntradayQuoteLoadPhase;
  readonly quote: IntradayQuoteResponse | null;
  readonly reasonCode: IntradayQuoteReasonCode | null;
};

export type IntradayQuoteLoadContext = {
  readonly enabled: boolean;
  readonly identity: IntradayQuoteIdentity | null;
  readonly mounted: boolean;
  readonly online: boolean;
  /** Snapshot identity is a browser-side invalidation boundary, not sent to the quote API. */
  readonly snapshotKey: string | null;
  readonly visible: boolean;
};

export type IntradayQuoteCoordinatorOptions = {
  readonly client?: IntradayQuoteClient;
  readonly clock?: IntradayQuoteClock;
  readonly createConsumerId?: () => string;
  readonly createIdempotencyKey?: (operation: string, sequence: number) => string;
  readonly fetcher?: typeof fetch;
  readonly navigate?: (href: string) => void;
  readonly onStateChange?: (state: IntradayQuoteLoadState) => void;
  readonly origin?: string;
};

type TimerHandle = unknown;

type QuoteSession = {
  readonly consumerId: string;
  readonly epoch: number;
  readonly identity: IntradayQuoteIdentity;
  readonly snapshotKey: string | null;
  abortController: AbortController | null;
  createAttempts: number;
  demand: IntradayQuoteDemandResponse | null;
  expiryTimer: TimerHandle | null;
  getInFlight: Promise<void> | null;
  readonly idempotencyKeys: Map<string, string>;
  lastGoodResponse: IntradayQuoteResponse | null;
  lastObservedNowMs: number;
  lastSessionKey: string | null;
  lastVersion: bigint | null;
  mutationTail: Promise<void>;
  nextSequence: number;
  pollTimer: TimerHandle | null;
  releaseRequested: boolean;
  releaseQueued: boolean;
  renewalAttempts: number;
  renewalTimer: TimerHandle | null;
  staleTimer: TimerHandle | null;
};

const EMPTY_CONTEXT: IntradayQuoteLoadContext = {
  enabled: false,
  identity: null,
  mounted: false,
  online: true,
  snapshotKey: null,
  visible: false,
};

const EMPTY_STATE: IntradayQuoteLoadState = {
  consumerId: null,
  errorCode: null,
  identity: null,
  lastSuccessAt: null,
  phase: "idle",
  quote: null,
  reasonCode: null,
};

const TRANSIENT_QUOTE_REASONS = new Set<IntradayQuoteReasonCode>([
  "QUOTE_STALE",
  "PROVIDER_TIMEOUT",
  "PROVIDER_RATE_LIMITED",
  "PROVIDER_UNAVAILABLE",
  "PROVIDER_RESPONSE_INVALID",
  "QUOTE_VALUE_INVALID",
  "QUOTE_BUDGET_EXHAUSTED",
  "PRODUCER_UNAVAILABLE",
]);

const MAX_CREATE_ATTEMPTS = 3;

function defaultConsumerId(): string {
  return crypto.randomUUID();
}

function defaultIdempotencyKey(operation: string, sequence: number): string {
  return `${operation}-${sequence}-${crypto.randomUUID()}`;
}

function identityKey(identity: IntradayQuoteIdentity, snapshotKey: string | null): string {
  return [
    identity.membership_id,
    identity.instrument_id,
    String(identity.generation),
    snapshotKey ?? "",
  ].join("\u0000");
}

function typedErrorCode(error: unknown): string {
  if (error instanceof IntradayQuoteApiError) return error.code;
  return "INTRADAY_QUOTE_REQUEST_FAILED";
}

function isAbortError(error: unknown): boolean {
  return (
    (typeof DOMException !== "undefined" &&
      error instanceof DOMException &&
      error.name === "AbortError") ||
    (error instanceof Error && error.name === "AbortError")
  );
}

function isSessionStopping(session: QuoteSession): boolean {
  return session.releaseRequested;
}

function sameContext(
  session: QuoteSession,
  context: IntradayQuoteLoadContext,
): context is IntradayQuoteLoadContext & {
  readonly identity: IntradayQuoteIdentity;
} {
  return (
    context.enabled &&
    context.mounted &&
    context.visible &&
    context.online &&
    context.identity !== null &&
    identityKey(context.identity, context.snapshotKey) ===
      identityKey(session.identity, session.snapshotKey)
  );
}

function responsePhase(response: IntradayQuoteResponse): IntradayQuoteLoadPhase {
  if (response.quote === null || response.freshness === "UNAVAILABLE") return "unavailable";
  return response.freshness === "STALE" ? "stale" : "ready";
}

export class IntradayQuoteLoadCoordinator {
  private readonly clock: IntradayQuoteClock;
  private readonly client: IntradayQuoteClient;
  private readonly createConsumerId: () => string;
  private readonly createIdempotencyKey: (operation: string, sequence: number) => string;
  private readonly navigate: ((href: string) => void) | undefined;
  private readonly listeners = new Set<(state: IntradayQuoteLoadState) => void>();
  private readonly onStateChange: ((state: IntradayQuoteLoadState) => void) | undefined;
  private navigateAfterStop(session: QuoteSession, href: string): void {
    if (!this.isCurrent(session)) return;
    this.stop("idle");
    if (this.navigate !== undefined) {
      this.navigate(href);
      return;
    }
    if (typeof window !== "undefined") window.location.replace(href);
  }
  private context = EMPTY_CONTEXT;
  private epoch = 0;
  private session: QuoteSession | null = null;
  private state = EMPTY_STATE;

  constructor(options: IntradayQuoteCoordinatorOptions = {}) {
    this.clock = options.clock ?? systemClock;
    this.client =
      options.client ??
      createIntradayQuoteClient({
        ...(options.fetcher === undefined ? {} : { fetcher: options.fetcher }),
        ...(options.navigate === undefined ? {} : { navigate: options.navigate }),
        ...(options.origin === undefined ? {} : { origin: options.origin }),
      });
    this.createConsumerId = options.createConsumerId ?? defaultConsumerId;
    this.createIdempotencyKey = options.createIdempotencyKey ?? defaultIdempotencyKey;
    this.navigate = options.navigate;
    this.onStateChange = options.onStateChange;
  }

  getState(): IntradayQuoteLoadState {
    return this.state;
  }

  subscribe(listener: (state: IntradayQuoteLoadState) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  setContext(next: Partial<IntradayQuoteLoadContext>): void {
    const context: IntradayQuoteLoadContext = {
      ...this.context,
      ...next,
    };
    const shouldRun =
      context.enabled &&
      context.mounted &&
      context.visible &&
      context.online &&
      context.identity !== null;
    const previousKey =
      this.context.identity === null
        ? null
        : identityKey(this.context.identity, this.context.snapshotKey);
    const nextKey =
      context.identity === null ? null : identityKey(context.identity, context.snapshotKey);
    this.context = context;

    if (!shouldRun) {
      this.stop(context.online ? "idle" : "offline");
      return;
    }
    if (this.session !== null && previousKey === nextKey && sameContext(this.session, context))
      return;
    this.stop("idle");
    this.start(context.identity, context.snapshotKey);
  }

  start(identity: IntradayQuoteIdentity, snapshotKey: string | null = null): void {
    this.stop("idle");
    const session: QuoteSession = {
      abortController: null,
      consumerId: this.createConsumerId(),
      createAttempts: 0,
      demand: null,
      epoch: ++this.epoch,
      expiryTimer: null,
      getInFlight: null,
      idempotencyKeys: new Map(),
      identity,
      lastGoodResponse: null,
      lastObservedNowMs: this.clock.now(),
      lastSessionKey: null,
      lastVersion: null,
      mutationTail: Promise.resolve(),
      nextSequence: 0,
      pollTimer: null,
      releaseQueued: false,
      releaseRequested: false,
      renewalAttempts: 0,
      renewalTimer: null,
      snapshotKey,
      staleTimer: null,
    };
    this.session = session;
    this.publish({
      consumerId: session.consumerId,
      errorCode: null,
      identity: session.identity,
      lastSuccessAt: null,
      phase: "demanding",
      quote: null,
      reasonCode: null,
    });
    this.enqueueCreate(session);
  }

  stop(phase: "idle" | "offline" = "idle"): void {
    const session = this.session;
    this.session = null;
    this.epoch += 1;
    if (session !== null) {
      session.releaseRequested = true;
      this.clearTimers(session);
      session.abortController?.abort();
      session.abortController = null;
      this.queueRelease(session);
    }
    this.publish({
      ...EMPTY_STATE,
      phase,
    });
  }

  destroy(): void {
    this.stop("idle");
    this.listeners.clear();
  }

  private publish(next: IntradayQuoteLoadState): void {
    this.state = next;
    this.onStateChange?.(next);
    for (const listener of [...this.listeners]) listener(next);
  }

  private isCurrent(session: QuoteSession): boolean {
    return this.session === session && session.epoch === this.epoch && !isSessionStopping(session);
  }

  private mutation<T>(session: QuoteSession, task: () => Promise<T>): Promise<T> {
    const run = session.mutationTail.then(task, task);
    session.mutationTail = run.then(
      () => undefined,
      () => undefined,
    );
    return run;
  }

  private sequenceKey(session: QuoteSession, operation: string, sequence: number): string {
    const key = `${operation}:${sequence}`;
    const existing = session.idempotencyKeys.get(key);
    if (existing !== undefined) return existing;
    const created = this.createIdempotencyKey(`${operation}-${session.consumerId}`, sequence);
    session.idempotencyKeys.set(key, created);
    return created;
  }

  private enqueueCreate(session: QuoteSession): void {
    const sequence = 0;
    const body: IntradayQuoteDemandRequest = {
      consumer_id: session.consumerId,
      generation: session.identity.generation,
      membership_id: session.identity.membership_id,
      renewal_sequence: sequence,
      schema_version: 1,
    };
    const idempotencyKey = this.sequenceKey(session, "quote-demand", sequence);
    void this.mutation(session, () =>
      Promise.resolve()
        .then(() =>
          this.client.createDemand(body, {
            idempotencyKey,
            navigate: (href) => this.navigateAfterStop(session, href),
          }),
        )
        .then((demand) => this.acceptDemand(session, demand))
        .catch((error: unknown) => this.handleCreateFailure(session, error)),
    );
  }

  private acceptDemand(session: QuoteSession, demand: IntradayQuoteDemandResponse): void {
    if (
      demand.consumer_id !== session.consumerId ||
      demand.membership_id !== session.identity.membership_id ||
      demand.instrument_id !== session.identity.instrument_id ||
      demand.generation !== session.identity.generation ||
      demand.renewal_sequence !== session.nextSequence
    ) {
      this.handleSessionError(session, "INTRADAY_QUOTE_DEMAND_CONTRACT_INVALID");
      return;
    }
    session.demand = demand;
    session.nextSequence = demand.renewal_sequence + 1;
    session.renewalAttempts = 0;
    if (!this.isCurrent(session)) {
      this.queueRelease(session);
      return;
    }
    session.createAttempts = 0;
    this.scheduleRenewal(session, demand.renew_after_ms);
    void this.poll(session, true);
  }

  private handleCreateFailure(session: QuoteSession, error: unknown): void {
    if (!this.isCurrent(session)) return;
    if (isAbortError(error)) return;
    const code = typedErrorCode(error);
    if (this.isTerminalError(error)) {
      this.handleSessionError(session, code);
      return;
    }
    session.createAttempts += 1;
    this.publish({ ...this.state, phase: "unavailable", errorCode: code });
    if (session.createAttempts < MAX_CREATE_ATTEMPTS) {
      const delay =
        error instanceof IntradayQuoteApiError ? (error.retryAfterMs ?? 15_000) : 15_000;
      this.schedule(session, "pollTimer", Math.max(delay, 15_000), () =>
        this.enqueueCreate(session),
      );
    }
  }

  private scheduleRenewal(session: QuoteSession, delayMs: number): void {
    if (session.renewalTimer !== null) this.clock.clearTimeout(session.renewalTimer);
    session.renewalTimer = this.clock.setTimeout(
      () => {
        session.renewalTimer = null;
        if (!this.isCurrent(session) || session.demand === null) return;
        this.enqueueRenewal(session);
      },
      Math.max(delayMs, INTRADAY_QUOTE_RENEWAL_INTERVAL_MS),
    );
  }

  private enqueueRenewal(session: QuoteSession): void {
    const sequence = session.nextSequence;
    const body: IntradayQuoteDemandRequest = {
      consumer_id: session.consumerId,
      generation: session.identity.generation,
      membership_id: session.identity.membership_id,
      renewal_sequence: sequence,
      schema_version: 1,
    };
    const idempotencyKey = this.sequenceKey(session, "quote-demand", sequence);
    void this.mutation(session, () =>
      Promise.resolve()
        .then(() =>
          this.client.createDemand(body, {
            idempotencyKey,
            navigate: (href) => this.navigateAfterStop(session, href),
          }),
        )
        .then((demand) => this.acceptDemand(session, demand))
        .catch((error: unknown) => this.handleRenewalFailure(session, error)),
    );
  }

  private handleRenewalFailure(session: QuoteSession, error: unknown): void {
    if (!this.isCurrent(session) || isAbortError(error)) return;
    const code = typedErrorCode(error);
    if (this.isTerminalError(error)) {
      this.handleSessionError(session, code);
      return;
    }
    session.renewalAttempts += 1;
    this.publish({
      ...this.state,
      phase: this.state.quote === null ? "unavailable" : "stale",
      errorCode: code,
    });
    if (session.renewalAttempts >= MAX_CREATE_ATTEMPTS) {
      this.handleSessionError(session, code);
      return;
    }
    this.scheduleRenewal(
      session,
      Math.max(
        INTRADAY_QUOTE_RENEWAL_INTERVAL_MS,
        error instanceof IntradayQuoteApiError ? (error.retryAfterMs ?? 0) : 0,
      ),
    );
  }

  private async poll(session: QuoteSession, immediate: boolean): Promise<void> {
    if (!this.isCurrent(session) || session.demand === null || session.getInFlight !== null) return;
    if (!immediate) await Promise.resolve();
    if (!this.isCurrent(session) || session.demand === null || session.getInFlight !== null) return;
    this.observeClock(session);
    const controller = new AbortController();
    session.abortController = controller;
    this.publish({ ...this.state, phase: "polling", errorCode: null });
    const request = Promise.resolve().then(() =>
      this.client.getQuote(session.identity, {
        signal: controller.signal,
        navigate: (href) => this.navigateAfterStop(session, href),
        nowMs: this.clock.now(),
      }),
    );
    session.getInFlight = request
      .then((response) => this.acceptQuote(session, response))
      .catch((error: unknown) => this.handlePollFailure(session, error))
      .finally(() => {
        if (session.getInFlight !== null) session.getInFlight = null;
        if (session.abortController === controller) session.abortController = null;
      });
    await session.getInFlight;
  }

  private acceptQuote(session: QuoteSession, response: IntradayQuoteResponse): void {
    if (!this.isCurrent(session)) return;
    if (!isIntradayQuoteIdentityMatch(response, session.identity)) {
      this.handleSessionError(session, "INTRADAY_QUOTE_IDENTITY_MISMATCH");
      return;
    }
    this.observeClock(session);
    const nextSessionKey = intradayQuoteSessionKey(response);
    if (nextSessionKey !== session.lastSessionKey) {
      session.lastGoodResponse = null;
      session.lastVersion = null;
      session.lastSessionKey = nextSessionKey;
    }
    if (response.quote !== null) {
      const nextVersion = intradayQuoteVersion(response.quote);
      if (session.lastVersion !== null && nextVersion < session.lastVersion) {
        this.invalidateQuote(session, "QUOTE_STALE", "INTRADAY_QUOTE_VERSION_REVERSED");
        return;
      }
      session.lastVersion = nextVersion;
      session.lastGoodResponse = response;
      this.scheduleStaleness(session, response.quote.last_success_at);
    } else if (
      response.reason_code === null ||
      !TRANSIENT_QUOTE_REASONS.has(response.reason_code)
    ) {
      session.lastGoodResponse = null;
      session.lastVersion = null;
    }
    const lastGood = this.validatedLastGood(session);
    const retained =
      response.quote === null &&
      lastGood !== null &&
      session.lastSessionKey === nextSessionKey &&
      response.reason_code !== null &&
      TRANSIENT_QUOTE_REASONS.has(response.reason_code)
        ? lastGood
        : response;
    const reasonCode = response.reason_code;
    this.publish({
      consumerId: session.consumerId,
      errorCode: null,
      identity: session.identity,
      lastSuccessAt: retained.quote?.last_success_at ?? null,
      phase: retained !== response ? "stale" : responsePhase(retained),
      quote: retained,
      reasonCode,
    });
    this.schedulePoll(session, response.next_poll_after_ms);
  }

  private handlePollFailure(session: QuoteSession, error: unknown): void {
    if (!this.isCurrent(session) || isAbortError(error)) return;
    const code = typedErrorCode(error);
    if (this.isTerminalError(error)) {
      this.handleSessionError(session, code);
      return;
    }
    const retained = this.validatedLastGood(session);
    this.publish({
      ...this.state,
      errorCode: code,
      phase: retained === null ? "unavailable" : "stale",
      quote: retained,
      lastSuccessAt: retained?.quote?.last_success_at ?? null,
      reasonCode: "PRODUCER_UNAVAILABLE",
    });
    const delay = error instanceof IntradayQuoteApiError ? (error.retryAfterMs ?? 0) : 0;
    this.schedulePoll(session, Math.max(INTRADAY_QUOTE_POLL_INTERVAL_MS, delay));
  }

  private schedulePoll(session: QuoteSession, delayMs: number): void {
    this.schedule(session, "pollTimer", Math.max(delayMs, INTRADAY_QUOTE_POLL_INTERVAL_MS), () => {
      void this.poll(session, false);
    });
  }

  private schedule(
    session: QuoteSession,
    timer: "expiryTimer" | "pollTimer" | "staleTimer" | "renewalTimer",
    delayMs: number,
    callback: () => void,
  ): void {
    const current = session[timer];
    if (current !== null) this.clock.clearTimeout(current);
    session[timer] = this.clock.setTimeout(
      () => {
        session[timer] = null;
        if (this.isCurrent(session)) callback();
      },
      Math.max(0, delayMs),
    );
  }

  private scheduleStaleness(session: QuoteSession, lastSuccessAt: string): void {
    const lastSuccessMs = Date.parse(lastSuccessAt);
    const age = this.clock.now() - lastSuccessMs;
    this.schedule(session, "staleTimer", Math.max(0, INTRADAY_QUOTE_STALE_AFTER_MS - age), () => {
      if (session.lastGoodResponse === null || !this.isCurrent(session)) return;
      this.publish({
        ...this.state,
        phase: "stale",
        reasonCode: "QUOTE_STALE",
        lastSuccessAt: session.lastGoodResponse.quote?.last_success_at ?? null,
      });
    });
    this.schedule(
      session,
      "expiryTimer",
      Math.max(0, INTRADAY_QUOTE_CACHE_MAX_AGE_MS - age),
      () => {
        this.invalidateQuote(session, "QUOTE_STALE", null);
      },
    );
  }

  private validatedLastGood(session: QuoteSession): IntradayQuoteResponse | null {
    const candidate = session.lastGoodResponse;
    if (candidate === null || candidate.quote === null) return null;
    if (intradayQuoteSessionKey(candidate) !== session.lastSessionKey) {
      session.lastGoodResponse = null;
      session.lastVersion = null;
      return null;
    }
    const age = this.clock.now() - Date.parse(candidate.quote.last_success_at);
    if (!Number.isFinite(age) || age < 0 || age > INTRADAY_QUOTE_CACHE_MAX_AGE_MS) {
      session.lastGoodResponse = null;
      session.lastVersion = null;
      return null;
    }
    return candidate;
  }

  private observeClock(session: QuoteSession): void {
    const nowMs = this.clock.now();
    if (nowMs < session.lastObservedNowMs) {
      session.lastGoodResponse = null;
      session.lastVersion = null;
      session.lastSessionKey = null;
      this.publish({
        ...this.state,
        errorCode: "INTRADAY_QUOTE_CLOCK_REGRESSION",
        lastSuccessAt: null,
        phase: "unavailable",
        quote: null,
        reasonCode: "QUOTE_STALE",
      });
    }
    session.lastObservedNowMs = nowMs;
  }

  private invalidateQuote(
    session: QuoteSession,
    reasonCode: IntradayQuoteReasonCode,
    errorCode: string | null,
  ): void {
    if (!this.isCurrent(session)) return;
    session.lastGoodResponse = null;
    session.lastVersion = null;
    this.publish({
      ...this.state,
      errorCode,
      lastSuccessAt: null,
      phase: "unavailable",
      quote: null,
      reasonCode,
    });
    this.schedulePoll(session, INTRADAY_QUOTE_POLL_INTERVAL_MS);
  }

  private handleSessionError(session: QuoteSession, code: string): void {
    if (!this.isCurrent(session)) return;
    this.stop("idle");
    this.publish({ ...EMPTY_STATE, errorCode: code, phase: "error" });
  }

  private isTerminalError(error: unknown): boolean {
    return (
      error instanceof IntradayQuoteApiError &&
      (error.code === "INVALID_PARAMETER" ||
        error.code === "SESSION_UNKNOWN" ||
        error.code === "SESSION_EXPIRED" ||
        error.code === "FORBIDDEN" ||
        error.code === "CSRF_DENIED" ||
        error.code === "RESOURCE_NOT_FOUND" ||
        error.code === "IDEMPOTENCY_MISMATCH" ||
        error.code === "QUOTE_DEMAND_SEQUENCE_CONFLICT")
    );
  }

  private clearTimers(session: QuoteSession): void {
    for (const timer of ["expiryTimer", "pollTimer", "renewalTimer", "staleTimer"] as const) {
      const handle = session[timer];
      if (handle !== null) this.clock.clearTimeout(handle);
      session[timer] = null;
    }
  }

  private queueRelease(session: QuoteSession): void {
    if (session.releaseQueued) return;
    session.releaseQueued = true;
    void this.mutation(session, async () => {
      if (session.demand === null) return;
      const sequence = session.nextSequence;
      const body = {
        consumer_id: session.consumerId,
        renewal_sequence: sequence,
        schema_version: 1 as const,
      };
      try {
        await this.client.releaseDemand(session.demand.demand_id, body, {
          idempotencyKey: this.sequenceKey(session, "quote-release", sequence),
        });
      } catch {
        // The lease is bounded by the API. Cleanup remains best effort and never
        // retries by instrument, wildcard, or another consumer.
      }
    });
  }
}

export const StockBetaQuoteLoadCoordinator = IntradayQuoteLoadCoordinator;

export function createIntradayQuoteLoadCoordinator(
  options: IntradayQuoteCoordinatorOptions = {},
): IntradayQuoteLoadCoordinator {
  return new IntradayQuoteLoadCoordinator(options);
}

export { INTRADAY_QUOTE_DEMAND_PATH };
