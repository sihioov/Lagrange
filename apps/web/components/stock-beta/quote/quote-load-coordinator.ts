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
  type IntradayQuoteMarketState,
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
  readonly fetching: boolean;
  readonly identity: IntradayQuoteIdentity | null;
  readonly lastSuccessAt: string | null;
  readonly marketState: IntradayQuoteMarketState | null;
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

type DemandMutation = {
  readonly body: IntradayQuoteDemandRequest;
  readonly idempotencyKey: string;
  readonly operation: "create" | "renew";
  readonly sequence: number;
};

type QuoteSession = {
  readonly consumerId: string;
  readonly epoch: number;
  readonly identity: IntradayQuoteIdentity;
  readonly snapshotKey: string | null;
  abortController: AbortController | null;
  createAttempts: number;
  demand: IntradayQuoteDemandResponse | null;
  demandExpiresAtMs: number | null;
  demandLeaseTimer: TimerHandle | null;
  expiryTimer: TimerHandle | null;
  getEpoch: number;
  getInFlight: Promise<void> | null;
  readonly idempotencyKeys: Map<string, string>;
  lastGoodResponse: IntradayQuoteResponse | null;
  lastObservedNowMs: number;
  lastSessionKey: string | null;
  lastVersion: bigint | null;
  leaseExpiredAtMs: number | null;
  mutationTail: Promise<void>;
  nextSequence: number;
  pendingMutation: DemandMutation | null;
  pollTimer: TimerHandle | null;
  pollFailureCode: string | null;
  recoveryAttempts: number;
  recoveryLastAttemptAtMs: number | null;
  recoveryTimer: TimerHandle | null;
  replayMutation: DemandMutation | null;
  releaseRequested: boolean;
  releaseQueued: boolean;
  leaseExpired: boolean;
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
  fetching: false,
  identity: null,
  lastSuccessAt: null,
  marketState: null,
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

const MAX_DEMAND_ATTEMPTS = 3;

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

function responsePhase(response: IntradayQuoteResponse, nowMs: number): IntradayQuoteLoadPhase {
  if (response.quote === null || response.freshness === "UNAVAILABLE") return "unavailable";
  if (response.freshness === "STALE") return "stale";
  const age = nowMs - Date.parse(response.quote.last_success_at);
  return age > INTRADAY_QUOTE_STALE_AFTER_MS ? "stale" : "ready";
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
    if (this.session !== null && previousKey === nextKey && sameContext(this.session, context)) {
      this.isLive(this.session);
      return;
    }
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
      demandExpiresAtMs: null,
      demandLeaseTimer: null,
      epoch: ++this.epoch,
      expiryTimer: null,
      getEpoch: 0,
      getInFlight: null,
      idempotencyKeys: new Map(),
      identity,
      lastGoodResponse: null,
      lastObservedNowMs: this.clock.now(),
      lastSessionKey: null,
      lastVersion: null,
      leaseExpiredAtMs: null,
      mutationTail: Promise.resolve(),
      nextSequence: 0,
      pendingMutation: null,
      pollTimer: null,
      pollFailureCode: null,
      recoveryAttempts: 0,
      recoveryLastAttemptAtMs: null,
      recoveryTimer: null,
      replayMutation: null,
      releaseQueued: false,
      releaseRequested: false,
      leaseExpired: false,
      renewalAttempts: 0,
      renewalTimer: null,
      snapshotKey,
      staleTimer: null,
    };
    this.session = session;
    this.publish({
      consumerId: session.consumerId,
      errorCode: null,
      fetching: false,
      identity: session.identity,
      lastSuccessAt: null,
      marketState: null,
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
      this.fenceGet(session);
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

  private isLive(session: QuoteSession): boolean {
    if (!this.isCurrent(session)) return false;
    if (session.leaseExpired || session.demand === null || session.demandExpiresAtMs === null) {
      return false;
    }
    if (session.demandExpiresAtMs !== null && this.clock.now() >= session.demandExpiresAtMs) {
      this.expireDemand(session);
      return false;
    }
    return true;
  }

  private expireDemand(session: QuoteSession): void {
    if (!this.isCurrent(session)) return;
    if (!session.leaseExpired) {
      session.leaseExpired = true;
      session.leaseExpiredAtMs = this.clock.now();
      if (session.demandLeaseTimer !== null) this.clock.clearTimeout(session.demandLeaseTimer);
      session.demandLeaseTimer = null;
      this.clearQuoteTimers(session);
      this.fenceGet(session);
      this.publish({
        ...this.state,
        consumerId: session.consumerId,
        errorCode: session.pollFailureCode,
        fetching: false,
        identity: session.identity,
        lastSuccessAt: session.lastGoodResponse?.quote?.last_success_at ?? this.state.lastSuccessAt,
        phase: "unavailable",
        quote: null,
        reasonCode: "NO_ACTIVE_DEMAND",
      });
    }
    this.scheduleRecovery(session);
  }

  private fenceGet(session: QuoteSession): void {
    session.getEpoch += 1;
    session.abortController?.abort();
    session.abortController = null;
    // A browser may ignore abort after the response has started. Dropping the
    // tracked promise permits the post-recovery GET while getEpoch prevents the
    // abandoned response from publishing into the new lease.
    session.getInFlight = null;
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
    this.enqueueDemandMutation(session, "create", 0);
  }

  private demandMutation(
    session: QuoteSession,
    operation: DemandMutation["operation"],
    sequence: number,
  ): DemandMutation {
    return {
      body: {
        consumer_id: session.consumerId,
        generation: session.identity.generation,
        membership_id: session.identity.membership_id,
        renewal_sequence: sequence,
        schema_version: 1,
      },
      idempotencyKey: this.sequenceKey(session, "quote-demand", sequence),
      operation,
      sequence,
    };
  }

  private enqueueDemandMutation(
    session: QuoteSession,
    operation: DemandMutation["operation"],
    sequence: number,
    replay: DemandMutation | null = null,
  ): void {
    if (!this.isCurrent(session) || session.pendingMutation !== null) return;
    const mutation = replay ?? this.demandMutation(session, operation, sequence);
    session.pendingMutation = mutation;
    session.replayMutation = mutation;
    void this.mutation(session, async () => {
      if (!this.isCurrent(session) || session.pendingMutation !== mutation) return;
      let demand: IntradayQuoteDemandResponse;
      try {
        demand = await this.client.createDemand(mutation.body, {
          idempotencyKey: mutation.idempotencyKey,
          navigate: (href) => this.navigateAfterStop(session, href),
        });
      } catch (error) {
        if (session.pendingMutation === mutation) session.pendingMutation = null;
        this.handleDemandFailure(session, error, mutation);
        return;
      }
      if (session.pendingMutation === mutation) session.pendingMutation = null;
      this.acceptDemand(session, demand, mutation);
    });
  }

  private acceptDemand(
    session: QuoteSession,
    demand: IntradayQuoteDemandResponse,
    mutation: DemandMutation,
  ): void {
    if (
      demand.consumer_id !== session.consumerId ||
      demand.membership_id !== session.identity.membership_id ||
      demand.instrument_id !== session.identity.instrument_id ||
      demand.generation !== session.identity.generation ||
      demand.renewal_sequence !== mutation.sequence
    ) {
      if (this.isCurrent(session)) {
        this.handleSessionError(session, "INTRADAY_QUOTE_DEMAND_CONTRACT_INVALID");
      }
      return;
    }
    const demandExpiresAtMs = Date.parse(demand.lease_expires_at);
    if (!Number.isFinite(demandExpiresAtMs)) {
      if (this.isCurrent(session)) {
        this.handleSessionError(session, "INTRADAY_QUOTE_DEMAND_LEASE_INVALID");
      }
      return;
    }
    session.demand = demand;
    session.demandExpiresAtMs = demandExpiresAtMs;
    session.nextSequence = demand.renewal_sequence + 1;
    session.replayMutation = null;
    if (!this.isCurrent(session)) {
      this.queueRelease(session);
      return;
    }
    if (demandExpiresAtMs <= this.clock.now()) {
      // An exact replay can legitimately return the prior, already-expired
      // lease. It proves this sequence was accepted, so recovery advances to a
      // new sequence rather than creating another consumer or reusing it.
      this.expireDemand(session);
      return;
    }
    session.leaseExpired = false;
    session.leaseExpiredAtMs = null;
    session.recoveryAttempts = 0;
    session.recoveryLastAttemptAtMs = null;
    if (session.recoveryTimer !== null) this.clock.clearTimeout(session.recoveryTimer);
    session.recoveryTimer = null;
    session.createAttempts = 0;
    session.renewalAttempts = 0;
    this.scheduleDemandLease(session, demandExpiresAtMs);
    if (!this.isLive(session)) return;
    this.scheduleRenewal(session, demand.renew_after_ms);
    void this.poll(session, true);
  }

  private scheduleDemandLease(session: QuoteSession, expiresAtMs: number): void {
    if (session.demandLeaseTimer !== null) this.clock.clearTimeout(session.demandLeaseTimer);
    session.demandExpiresAtMs = expiresAtMs;
    const timer = this.clock.setTimeout(
      () => {
        if (session.demandLeaseTimer !== timer) return;
        session.demandLeaseTimer = null;
        if (!this.isCurrent(session) || session.demandExpiresAtMs !== expiresAtMs) return;
        if (this.clock.now() < expiresAtMs) {
          this.scheduleDemandLease(session, expiresAtMs);
          return;
        }
        this.expireDemand(session);
      },
      Math.max(0, expiresAtMs - this.clock.now()),
    );
    session.demandLeaseTimer = timer;
    if (this.clock.now() >= expiresAtMs) this.expireDemand(session);
  }

  private handleDemandFailure(
    session: QuoteSession,
    error: unknown,
    mutation: DemandMutation,
  ): void {
    if (!this.isCurrent(session)) return;
    if (isAbortError(error)) {
      if (session.leaseExpired) this.scheduleRecovery(session);
      return;
    }
    const code = typedErrorCode(error);
    if (this.isTerminalError(error)) {
      this.handleSessionError(session, code);
      return;
    }
    const retryAfterMs = error instanceof IntradayQuoteApiError ? (error.retryAfterMs ?? 0) : 0;
    if (session.leaseExpired) {
      this.publish({
        ...this.state,
        errorCode: code,
        fetching: false,
        phase: "unavailable",
        quote: null,
        reasonCode: "NO_ACTIVE_DEMAND",
      });
      this.scheduleRecovery(session, retryAfterMs);
      return;
    }
    let attempts: number;
    if (mutation.operation === "create") {
      session.createAttempts += 1;
      attempts = session.createAttempts;
    } else {
      session.renewalAttempts += 1;
      attempts = session.renewalAttempts;
    }
    this.publish({
      ...this.state,
      errorCode: code,
      phase: this.state.quote === null ? "unavailable" : "stale",
    });
    if (attempts >= MAX_DEMAND_ATTEMPTS) {
      this.handleSessionError(session, code);
      return;
    }
    this.scheduleDemandRetry(session, retryAfterMs);
  }

  private scheduleRenewal(session: QuoteSession, delayMs: number): void {
    if (session.renewalTimer !== null) this.clock.clearTimeout(session.renewalTimer);
    session.renewalTimer = this.clock.setTimeout(
      () => {
        session.renewalTimer = null;
        if (!this.isLive(session) || session.demand === null) return;
        this.enqueueRenewal(session);
      },
      Math.max(delayMs, INTRADAY_QUOTE_RENEWAL_INTERVAL_MS),
    );
  }

  private enqueueRenewal(session: QuoteSession): void {
    if (!this.isLive(session) || session.demand === null) return;
    this.enqueueDemandMutation(session, "renew", session.nextSequence);
  }

  private scheduleDemandRetry(session: QuoteSession, retryAfterMs: number): void {
    this.schedule(
      session,
      "renewalTimer",
      Math.max(INTRADAY_QUOTE_RENEWAL_INTERVAL_MS, retryAfterMs),
      () => this.replayDemandMutation(session),
    );
  }

  private replayDemandMutation(session: QuoteSession): void {
    if (!this.isCurrent(session)) return;
    if (session.leaseExpired) {
      this.scheduleRecovery(session);
      return;
    }
    const replay = session.replayMutation;
    if (replay !== null) {
      this.enqueueDemandMutation(session, replay.operation, replay.sequence, replay);
      return;
    }
    this.enqueueDemandMutation(
      session,
      session.demand === null ? "create" : "renew",
      session.nextSequence,
    );
  }

  private scheduleRecovery(session: QuoteSession, retryAfterMs = 0): void {
    if (!this.isCurrent(session) || !session.leaseExpired || session.pendingMutation !== null)
      return;
    if (session.recoveryAttempts >= MAX_DEMAND_ATTEMPTS) {
      this.exhaustRecovery(session);
      return;
    }
    const nowMs = this.clock.now();
    const earliestAttemptMs = Math.max(
      (session.leaseExpiredAtMs ?? nowMs) + INTRADAY_QUOTE_RENEWAL_INTERVAL_MS,
      (session.recoveryLastAttemptAtMs ?? nowMs) + INTRADAY_QUOTE_RENEWAL_INTERVAL_MS,
      nowMs + retryAfterMs,
    );
    this.schedule(session, "recoveryTimer", Math.max(0, earliestAttemptMs - nowMs), () => {
      if (!this.isCurrent(session) || !session.leaseExpired || session.pendingMutation !== null) {
        return;
      }
      if (session.recoveryAttempts >= MAX_DEMAND_ATTEMPTS) {
        this.exhaustRecovery(session);
        return;
      }
      session.recoveryAttempts += 1;
      session.recoveryLastAttemptAtMs = this.clock.now();
      const replay = session.replayMutation;
      if (replay !== null) {
        this.enqueueDemandMutation(session, replay.operation, replay.sequence, replay);
        return;
      }
      this.enqueueDemandMutation(
        session,
        session.demand === null ? "create" : "renew",
        session.nextSequence,
      );
    });
  }

  private exhaustRecovery(session: QuoteSession): void {
    if (!this.isCurrent(session) || !session.leaseExpired) return;
    if (session.recoveryTimer !== null) this.clock.clearTimeout(session.recoveryTimer);
    session.recoveryTimer = null;
    this.publish({
      ...this.state,
      consumerId: session.consumerId,
      errorCode: this.state.errorCode ?? "INTRADAY_QUOTE_DEMAND_RECOVERY_EXHAUSTED",
      fetching: false,
      identity: session.identity,
      phase: "unavailable",
      quote: null,
      reasonCode: "NO_ACTIVE_DEMAND",
    });
  }

  private async poll(session: QuoteSession, immediate: boolean): Promise<void> {
    if (!this.isLive(session) || session.demand === null || session.getInFlight !== null) return;
    if (!immediate) await Promise.resolve();
    if (!this.isLive(session) || session.demand === null || session.getInFlight !== null) return;
    this.observeClock(session);
    if (!this.isLive(session)) return;
    const controller = new AbortController();
    const getEpoch = ++session.getEpoch;
    session.abortController = controller;
    this.publish({
      ...this.state,
      fetching: true,
      phase:
        session.lastGoodResponse === null && session.pollFailureCode === null
          ? "polling"
          : this.state.phase,
    });
    const request = Promise.resolve().then(() =>
      this.client.getQuote(session.identity, {
        now: this.clock.now,
        signal: controller.signal,
        navigate: (href) => this.navigateAfterStop(session, href),
      }),
    );
    const inFlight = request
      .then((response) => this.acceptQuote(session, response, getEpoch))
      .catch((error: unknown) => this.handlePollFailure(session, error, getEpoch));
    session.getInFlight = inFlight;
    void inFlight.then(
      () => {
        if (session.getInFlight === inFlight) session.getInFlight = null;
        if (session.abortController === controller) session.abortController = null;
      },
      () => {
        if (session.getInFlight === inFlight) session.getInFlight = null;
        if (session.abortController === controller) session.abortController = null;
      },
    );
    await inFlight;
  }

  private acceptQuote(
    session: QuoteSession,
    response: IntradayQuoteResponse,
    getEpoch: number,
  ): void {
    if (getEpoch !== session.getEpoch) return;
    if (!this.isLive(session)) return;
    if (!isIntradayQuoteIdentityMatch(response, session.identity)) {
      this.handleSessionError(session, "INTRADAY_QUOTE_IDENTITY_MISMATCH");
      return;
    }
    this.observeClock(session);
    if (
      response.quote !== null &&
      (() => {
        const age = this.clock.now() - Date.parse(response.quote.last_success_at);
        return !Number.isFinite(age) || age < 0 || age > INTRADAY_QUOTE_CACHE_MAX_AGE_MS;
      })()
    ) {
      this.invalidateQuote(session, "QUOTE_STALE", "INTRADAY_QUOTE_TIMESTAMP_INVALID");
      return;
    }
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
      session.pollFailureCode = null;
      this.scheduleStaleness(session, response.quote.last_success_at);
    } else if (
      response.market_state === "UNKNOWN" ||
      response.session === null ||
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
    const requestFailure = session.pollFailureCode;
    const reasonCode = requestFailure === null ? response.reason_code : "PRODUCER_UNAVAILABLE";
    this.publish({
      consumerId: session.consumerId,
      errorCode: requestFailure,
      fetching: false,
      identity: session.identity,
      lastSuccessAt: retained.quote?.last_success_at ?? null,
      marketState: response.market_state,
      phase:
        requestFailure !== null
          ? retained.quote === null
            ? "unavailable"
            : "stale"
          : retained !== response
            ? "stale"
            : responsePhase(retained, this.clock.now()),
      quote: retained,
      reasonCode,
    });
    this.schedulePoll(session, response.next_poll_after_ms);
  }

  private handlePollFailure(session: QuoteSession, error: unknown, getEpoch: number): void {
    if (getEpoch !== session.getEpoch || isAbortError(error) || !this.isLive(session)) return;
    const code = typedErrorCode(error);
    if (this.isTerminalError(error)) {
      this.handleSessionError(session, code);
      return;
    }
    session.pollFailureCode = code;
    const retained = this.validatedLastGood(session);
    this.publish({
      ...this.state,
      errorCode: code,
      fetching: false,
      // A transport failure is distinct from the retained cache age: a young
      // value stays visible with its timestamp, but it is never shown as ready.
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
    timer: "expiryTimer" | "pollTimer" | "recoveryTimer" | "staleTimer" | "renewalTimer",
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
    this.schedule(
      session,
      "staleTimer",
      age <= INTRADAY_QUOTE_STALE_AFTER_MS ? INTRADAY_QUOTE_STALE_AFTER_MS - age + 1 : 0,
      () => {
        const lastGood = this.validatedLastGood(session);
        if (lastGood === null || !this.isCurrent(session)) return;
        const currentAge = this.clock.now() - Date.parse(lastGood.quote?.last_success_at ?? "");
        if (currentAge <= INTRADAY_QUOTE_STALE_AFTER_MS) {
          this.scheduleStaleness(session, lastGood.quote?.last_success_at ?? lastSuccessAt);
          return;
        }
        this.publish({
          ...this.state,
          errorCode: session.pollFailureCode,
          fetching: session.getInFlight !== null,
          phase: "stale",
          reasonCode: session.pollFailureCode === null ? "QUOTE_STALE" : "PRODUCER_UNAVAILABLE",
          lastSuccessAt: lastGood.quote?.last_success_at ?? null,
        });
      },
    );
    this.schedule(
      session,
      "expiryTimer",
      age <= INTRADAY_QUOTE_CACHE_MAX_AGE_MS ? INTRADAY_QUOTE_CACHE_MAX_AGE_MS - age + 1 : 0,
      () => {
        const lastGood = this.validatedLastGood(session);
        if (
          lastGood !== null &&
          this.clock.now() - Date.parse(lastGood.quote?.last_success_at ?? "") <=
            INTRADAY_QUOTE_CACHE_MAX_AGE_MS
        ) {
          this.scheduleStaleness(session, lastGood.quote?.last_success_at ?? lastSuccessAt);
          return;
        }
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
    if (!this.isLive(session)) return;
    session.lastGoodResponse = null;
    session.lastVersion = null;
    this.publish({
      ...this.state,
      errorCode: errorCode ?? session.pollFailureCode,
      fetching: session.getInFlight !== null,
      lastSuccessAt: null,
      phase: "unavailable",
      quote: null,
      reasonCode: session.pollFailureCode === null ? reasonCode : "PRODUCER_UNAVAILABLE",
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
    if (session.demandLeaseTimer !== null) this.clock.clearTimeout(session.demandLeaseTimer);
    session.demandLeaseTimer = null;
    for (const timer of [
      "expiryTimer",
      "pollTimer",
      "recoveryTimer",
      "renewalTimer",
      "staleTimer",
    ] as const) {
      const handle = session[timer];
      if (handle !== null) this.clock.clearTimeout(handle);
      session[timer] = null;
    }
  }

  private clearQuoteTimers(session: QuoteSession): void {
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
