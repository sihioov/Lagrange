import {
  createIntradayStreamClient,
  type IntradayStreamClient,
  type IntradayStreamFailure,
  IntradayStreamHttpError,
} from "@/lib/products/intraday-stream-client";
import {
  IntradayStreamContractError,
  type IntradayStreamIdentity,
  intradayStreamUuidSchema,
} from "@/lib/products/intraday-stream-contracts";
import {
  canonicalStreamIdentities,
  INTRADAY_STREAM_LEASE_MS,
  INTRADAY_STREAM_RENEW_MS,
  type IntradayStreamLease,
  type IntradayStreamLeaseRequest,
  intradayStreamEventPath,
  matchStreamLease,
} from "@/lib/products/intraday-stream-lease-contracts";
import { IntradayStreamState, type IntradayStreamView } from "./intraday-stream-state";

type StreamSource = Pick<EventTarget, "addEventListener" | "removeEventListener"> & {
  close: () => void;
};
type Timer = ReturnType<typeof setTimeout>;
export type IntradayStreamClock = {
  now: () => number;
  setTimeout: (callback: () => void, delay: number) => Timer;
  clearTimeout: (timer: Timer) => void;
};
export type IntradayStreamContext = {
  readonly enabled: boolean;
  readonly owner: boolean;
  readonly visible: boolean;
  readonly online: boolean;
  /** Local session boundary only. Never sent in a body or persisted. */
  readonly sessionKey: string | null;
  readonly identities: readonly IntradayStreamIdentity[];
};
export type IntradayStreamControllerView = IntradayStreamView & {
  lifecycle: "inactive" | "leasing" | "connecting" | "active" | "failed";
  failure: IntradayStreamFailure | null;
};

type Scope = {
  closed: boolean;
  sessionKey: string;
  consumerId: string;
  lease: IntradayStreamLease | null;
  acceptedKey: string | null;
  state: IntradayStreamState | null;
  request: { abort: AbortController; startedAt: number } | null;
  source: { instance: StreamSource; token: number; removeListeners: () => void } | null;
  timer: Timer | null;
  renewAt: number;
  expiresAt: number;
  reconnectAt: number | null;
  reconnectFailures: number;
  connectedAt: number;
  lastDeliveryAt: number;
  releaseAttempted: boolean;
};

const EMPTY: IntradayStreamControllerView = {
  lifecycle: "inactive",
  failure: null,
  phase: "idle",
  rows: [],
  terminalReason: null,
};
const DEFAULT_CONTEXT: IntradayStreamContext = {
  enabled: false,
  owner: false,
  visible: false,
  online: false,
  sessionKey: null,
  identities: [],
};

/**
 * One page-level owner for a tab's complete demand set, not one owner per quote widget.
 * Every asynchronous completion belongs to its original scope. No mutation is retried;
 * an unknown result is abandoned to the server TTL, never reused as an accepted sequence.
 */
export class IntradayStreamController {
  private readonly client: IntradayStreamClient;
  private readonly clock: IntradayStreamClock;
  private readonly uuid: () => string;
  private readonly createSource: (path: string) => StreamSource;
  private context: IntradayStreamContext = DEFAULT_CONTEXT;
  private scope: Scope | null = null;
  private readonly listeners = new Set<(view: IntradayStreamControllerView) => void>();
  private lastView: IntradayStreamControllerView = EMPTY;
  private disposed = false;
  private loggedOutSession: string | null = null;
  private faultedContext: string | null = null;

  constructor(
    options: {
      readonly client?: IntradayStreamClient;
      readonly clock?: IntradayStreamClock;
      readonly uuid?: () => string;
      readonly createSource?: (path: string) => StreamSource;
    } = {},
  ) {
    this.client = options.client ?? createIntradayStreamClient();
    this.clock = options.clock ?? {
      now: () => performance.now(),
      // Window timers require their native receiver; the clock is an ordinary object.
      setTimeout: (callback, delay) => setTimeout(callback, delay),
      clearTimeout: (timer) => clearTimeout(timer),
    };
    this.uuid = options.uuid ?? (() => crypto.randomUUID());
    this.createSource = options.createSource ?? ((path) => new EventSource(path));
  }

  subscribe(listener: (view: IntradayStreamControllerView) => void): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  view(): IntradayStreamControllerView {
    if (this.scope !== null) this.updateView(this.scope, false);
    return structuredClone(this.lastView);
  }

  setContext(input: IntradayStreamContext): void {
    if (this.disposed) return;
    const previousKey = this.identityKey();
    try {
      this.context = {
        ...input,
        identities:
          input.identities.length === 0 ? [] : canonicalStreamIdentities(input.identities),
      };
    } catch {
      this.context = DEFAULT_CONTEXT;
      this.stopScope("invalid_response");
      return;
    }
    if (!this.isActive()) {
      this.faultedContext = null;
      this.stopScope(null);
      return;
    }
    if (this.faultedContext === this.contextKey()) return;
    if (this.scope?.sessionKey !== this.context.sessionKey) this.stopScope(null);
    if (this.scope === null) {
      this.faultedContext = null;
      const consumerId = this.uuid();
      if (!intradayStreamUuidSchema.safeParse(consumerId).success) {
        this.stopScope("invalid_response");
        return;
      }
      this.scope = {
        closed: false,
        sessionKey: this.context.sessionKey ?? "",
        consumerId,
        lease: null,
        acceptedKey: null,
        state: null,
        request: null,
        source: null,
        timer: null,
        renewAt: 0,
        expiresAt: Infinity,
        reconnectAt: null,
        reconnectFailures: 0,
        connectedAt: 0,
        lastDeliveryAt: 0,
        releaseAttempted: false,
      };
    } else if (previousKey !== this.identityKey()) {
      this.closeSource(this.scope);
      this.scope.state?.close();
      this.scope.state = null;
      this.scope.reconnectAt = null;
    }
    this.drive(this.scope);
  }

  logout(): void {
    this.loggedOutSession = this.context.sessionKey;
    this.stopScope(null);
  }

  destroy(): void {
    this.disposed = true;
    this.stopScope(null);
    this.listeners.clear();
  }

  private isActive(): boolean {
    const c = this.context;
    return (
      !this.disposed &&
      c.enabled &&
      c.owner &&
      c.visible &&
      c.online &&
      c.sessionKey !== null &&
      c.sessionKey !== this.loggedOutSession &&
      c.identities.length > 0
    );
  }

  private identityKey(): string {
    return JSON.stringify(this.context.identities);
  }
  private contextKey(): string {
    return JSON.stringify([this.context.sessionKey, this.context.identities]);
  }
  private current(scope: Scope): boolean {
    return this.scope === scope && !scope.closed && this.isActive();
  }

  private drive(scope: Scope): void {
    if (!this.current(scope)) return;
    if (scope.timer !== null) {
      this.clock.clearTimeout(scope.timer);
      scope.timer = null;
    }
    const now = this.clock.now();
    if (!Number.isFinite(now) || now < 0 || now >= scope.expiresAt) {
      this.stopScope("unavailable");
      return;
    }
    if (scope.request !== null && now - scope.request.startedAt >= 5_000) {
      this.stopScope("timeout");
      return;
    }
    if (
      scope.request === null &&
      (scope.acceptedKey !== this.identityKey() || now >= scope.renewAt)
    )
      this.replace(scope);
    if (!this.current(scope)) return;
    if (scope.source !== null && now - scope.lastDeliveryAt >= 5_000) this.reconnect(scope);
    if (!this.current(scope)) return;
    if (
      scope.source !== null &&
      now - scope.connectedAt >= 5_000 &&
      now - scope.lastDeliveryAt < 2_000
    )
      scope.reconnectFailures = 0;
    if (
      scope.reconnectAt !== null &&
      now >= scope.reconnectAt &&
      scope.acceptedKey === this.identityKey()
    )
      this.openSource(scope);
    this.updateView(scope, true);
    if (this.current(scope)) scope.timer = this.clock.setTimeout(() => this.drive(scope), 250);
  }

  private replace(scope: Scope): void {
    const sequence = scope.lease === null ? 0 : scope.lease.renewal_sequence + 1;
    if (!Number.isSafeInteger(sequence)) {
      this.stopScope("conflict");
      return;
    }
    const request: IntradayStreamLeaseRequest = {
      schema_version: 2,
      consumer_id: scope.consumerId,
      renewal_sequence: sequence,
      identities: canonicalStreamIdentities(this.context.identities),
    };
    const previousLeaseId = scope.lease?.lease_id;
    const operation = { abort: new AbortController(), startedAt: this.clock.now() };
    scope.request = operation;
    let response: Promise<IntradayStreamLease>;
    try {
      response = this.client.replaceLease(request, {
        signal: operation.abort.signal,
        idempotencyKey: this.uuid(),
        ...(previousLeaseId === undefined ? {} : { previousLeaseId }),
      });
    } catch (error) {
      response = Promise.reject(error);
    }
    void response
      .then((input) => {
        const lease = matchStreamLease(input, request, previousLeaseId);
        if (!this.current(scope) || scope.request !== operation) {
          this.release(scope, lease);
          return;
        }
        scope.lease = lease;
        scope.acceptedKey = JSON.stringify(request.identities);
        // Start at request dispatch, never extend the local watchdog by network latency.
        scope.renewAt = operation.startedAt + INTRADAY_STREAM_RENEW_MS;
        scope.expiresAt = operation.startedAt + INTRADAY_STREAM_LEASE_MS;
        if (this.clock.now() - operation.startedAt >= 5_000) {
          this.stopScope("timeout");
          return;
        }
        if (scope.acceptedKey === this.identityKey()) {
          if (scope.state === null) {
            scope.state = new IntradayStreamState(lease.lease_id, lease.identities);
            scope.state.confirmLeaseExpiry(lease.lease_id, lease.lease_expires_at);
            this.openSource(scope);
          } else scope.state.confirmLeaseExpiry(lease.lease_id, lease.lease_expires_at);
        }
        scope.request = null;
        this.drive(scope);
      })
      .catch((error: unknown) => {
        if (!this.current(scope) || scope.request !== operation) return;
        const failure =
          error instanceof IntradayStreamHttpError
            ? error.kind
            : error instanceof IntradayStreamContractError
              ? "invalid_response"
              : "network";
        this.stopScope(failure);
      });
  }

  private openSource(scope: Scope): void {
    if (!this.current(scope) || scope.lease === null || scope.state === null) return;
    this.closeSource(scope);
    scope.reconnectAt = null;
    const token = scope.state.beginConnection();
    let source: StreamSource;
    try {
      source = this.createSource(intradayStreamEventPath(scope.lease.lease_id));
    } catch {
      this.reconnect(scope);
      return;
    }
    const remove: Array<() => void> = [];
    for (const kind of ["snapshot", "delta", "status", "reset"]) {
      const listener: EventListener = (event) => {
        if (!this.current(scope) || scope.source?.instance !== source) return;
        const message = event as MessageEvent<unknown>;
        const result = scope.state?.receive(
          token,
          kind,
          typeof message.data === "string" ? message.data : "",
          typeof message.lastEventId === "string" ? message.lastEventId : "",
          this.clock.now(),
        );
        if (result === "terminated") {
          this.stopScope("forbidden");
          return;
        }
        if (result === "resync") {
          this.reconnect(scope);
          return;
        }
        if (result === "applied") {
          scope.lastDeliveryAt = this.clock.now();
          if (
            scope.state
              ?.view(this.clock.now())
              .rows.some((row) => row.reason_code === "FEATURE_DISABLED")
          ) {
            this.stopScope("feature_disabled");
            return;
          }
        }
        this.updateView(scope, true);
      };
      source.addEventListener(kind, listener);
      remove.push(() => source.removeEventListener(kind, listener));
    }
    const onError: EventListener = () => {
      if (this.current(scope) && scope.source?.instance === source) this.reconnect(scope);
    };
    source.addEventListener("error", onError);
    remove.push(() => source.removeEventListener("error", onError));
    scope.source = {
      instance: source,
      token,
      removeListeners: () => {
        for (const fn of remove) fn();
      },
    };
    scope.lastDeliveryAt = scope.connectedAt = this.clock.now();
  }

  private closeSource(scope: Scope): void {
    const owned = scope.source;
    scope.source = null;
    if (owned === null) return;
    owned.removeListeners();
    owned.instance.close();
    scope.state?.disconnect(owned.token);
  }

  private reconnect(scope: Scope): void {
    this.closeSource(scope);
    if (++scope.reconnectFailures > 3) {
      this.stopScope("unavailable");
      return;
    }
    scope.reconnectAt = this.clock.now() + 1_000 * 2 ** (scope.reconnectFailures - 1);
    this.updateView(scope, true);
  }

  private stopScope(failure: IntradayStreamFailure | null): void {
    const scope = this.scope;
    this.scope = null;
    if (failure !== null) this.faultedContext = this.contextKey();
    if (scope !== null) {
      scope.closed = true;
      if (scope.timer !== null) this.clock.clearTimeout(scope.timer);
      this.closeSource(scope);
      scope.state?.close();
      scope.request?.abort.abort();
      if (scope.lease !== null) this.release(scope, scope.lease);
    }
    this.lastView = { ...EMPTY, lifecycle: failure === null ? "inactive" : "failed", failure };
    this.emit();
  }

  private release(scope: Scope, lease: IntradayStreamLease): void {
    if (scope.releaseAttempted) return;
    scope.releaseAttempted = true;
    const abort = new AbortController();
    const timer = this.clock.setTimeout(() => abort.abort(), 3_000);
    try {
      void this.client
        .releaseLease(
          lease.lease_id,
          {
            schema_version: 2,
            consumer_id: lease.consumer_id,
            renewal_sequence: lease.renewal_sequence,
          },
          { signal: abort.signal, idempotencyKey: this.uuid() },
        )
        .catch(() => undefined)
        .finally(() => this.clock.clearTimeout(timer));
    } catch {
      this.clock.clearTimeout(timer);
    }
    // A missing/ambiguous release is left to TTL. No mutation retries or resurrection.
  }

  private updateView(scope: Scope, emit: boolean): void {
    if (!this.current(scope)) return;
    const state = scope.state?.view(this.clock.now()) ?? EMPTY;
    if (state.phase === "terminated") {
      this.stopScope("unavailable");
      return;
    }
    this.lastView = {
      phase: state.phase,
      rows: state.rows,
      terminalReason: state.terminalReason,
      failure: null,
      lifecycle:
        scope.lease === null || scope.acceptedKey !== this.identityKey()
          ? "leasing"
          : state.phase === "ready"
            ? "active"
            : "connecting",
    };
    if (emit) this.emit();
  }

  private emit(): void {
    for (const listener of this.listeners) listener(structuredClone(this.lastView));
  }
}
