import { ServiceStatusRequestError } from "@/lib/operations/service-status-client";
import {
  parseServiceStatusTimestampMicros,
  type ServiceStatusId,
  type ServiceStatusResponse,
  type ServiceStatusService,
  type ServiceStatusState,
  serviceStatusContributesToOverall,
  serviceStatusOverall,
  serviceStatusSortRank,
} from "@/lib/operations/service-status-contract";

export const SERVICE_STATUS_BASE_POLL_MS = 30_000;
export const SERVICE_STATUS_MIN_REQUEST_GAP_MS = 5_000;
export const SERVICE_STATUS_REFRESH_LEAD_MS = 1_000;

export type ServiceStatusRequest = (signal: AbortSignal) => Promise<ServiceStatusResponse>;

export type ServiceStatusIssue = "loading" | "unavailable" | "rate_limited" | "denied" | null;

export type ServiceStatusDisplayedRow = {
  readonly service: ServiceStatusService;
  readonly status: ServiceStatusState;
  readonly processStatus: ServiceStatusService["process_status"];
  readonly workStatus: ServiceStatusService["work_status"];
  readonly reasonCodes: ServiceStatusService["reason_codes"];
  readonly expired: boolean;
};

export type ServiceStatusCounts = Readonly<Record<ServiceStatusState, number>>;

export type ServiceStatusSnapshot = {
  readonly rows: readonly ServiceStatusDisplayedRow[];
  readonly overallStatus: ServiceStatusState;
  readonly counts: ServiceStatusCounts;
  readonly issue: ServiceStatusIssue;
  readonly isFetching: boolean;
  readonly isStale: boolean;
  readonly lastRecordedAt: string | null;
  readonly policyState: ServiceStatusResponse["policy_state"] | null;
  readonly receivedAtMono: number | null;
  readonly nowMono: number;
};

export type ServiceStatusScheduler = {
  now(): number;
  setTimeout(callback: () => void, delayMs: number): unknown;
  clearTimeout(handle: unknown): void;
};

const browserScheduler: ServiceStatusScheduler = {
  now: () => globalThis.performance.now(),
  setTimeout: (callback, delayMs) => globalThis.setTimeout(callback, delayMs),
  clearTimeout: (handle) => globalThis.clearTimeout(handle as ReturnType<typeof setTimeout>),
};

function emptyCounts(): Record<ServiceStatusState, number> {
  return {
    healthy: 0,
    waiting: 0,
    degraded: 0,
    failed: 0,
    unknown: 0,
    disabled: 0,
  };
}

function responseOrder(response: ServiceStatusResponse): readonly ServiceStatusService[] {
  return response.services
    .map((service, index) => ({ service, index }))
    .sort((left, right) => {
      const rank =
        serviceStatusSortRank(left.service.status) - serviceStatusSortRank(right.service.status);
      return rank === 0 ? left.index - right.index : rank;
    })
    .map(({ service }) => service);
}

function deadlineFor(
  service: ServiceStatusService,
  generatedAt: string,
  settledAt: number,
  requestDurationMs: number,
): number {
  if (service.valid_until === null) return settledAt;
  const validUntil = parseServiceStatusTimestampMicros(service.valid_until);
  const generated = parseServiceStatusTimestampMicros(generatedAt);
  if (validUntil === null || generated === null) return settledAt;

  const requestMicros = BigInt(Math.ceil(Math.max(0, requestDurationMs) * 1_000));
  const remainingMicros = validUntil - generated - requestMicros;
  if (remainingMicros <= 0n) return settledAt;
  return settledAt + Number(remainingMicros) / 1_000;
}

export class ServiceStatusController {
  private readonly listeners = new Set<() => void>();
  private readonly scheduler: ServiceStatusScheduler;
  private readonly initialSnapshot: ServiceStatusSnapshot;
  private snapshot: ServiceStatusSnapshot;
  private response: ServiceStatusResponse | null = null;
  private orderedServices: readonly ServiceStatusService[] = [];
  private deadlines = new Map<string, number>();
  private issue: ServiceStatusIssue = "loading";
  private lastRecordedAt: string | null = null;
  private receivedAtMono: number | null = null;
  private pollTimer: unknown = null;
  private expiryTimer: unknown = null;
  private activeAbort: AbortController | null = null;
  private lastSettledAt: number | null = null;
  private retryAtMono: number | null = null;
  private generation = 0;
  private started = false;
  private active = false;
  private inFlight = false;
  private blocked = false;

  constructor(
    private readonly request: ServiceStatusRequest,
    scheduler: ServiceStatusScheduler = browserScheduler,
  ) {
    this.scheduler = scheduler;
    this.initialSnapshot = this.makeSnapshot(scheduler.now());
    this.snapshot = this.initialSnapshot;
  }

  readonly subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  readonly getSnapshot = (): ServiceStatusSnapshot => this.snapshot;

  readonly getServerSnapshot = (): ServiceStatusSnapshot => this.initialSnapshot;

  start(): void {
    if (this.started || this.blocked) return;
    this.started = true;
    this.scheduleExpiry();
    if (this.active) this.scheduleRequest(this.scheduler.now());
  }

  setActive(active: boolean): void {
    if (this.active === active) return;
    this.active = active;
    if (!active) {
      this.clearPollTimer();
      return;
    }
    if (this.started && !this.blocked && !this.inFlight) {
      this.scheduleRequest(this.scheduler.now());
    }
  }

  stop(): void {
    this.started = false;
    this.active = false;
    this.generation += 1;
    this.clearPollTimer();
    this.clearExpiryTimer();
    this.activeAbort?.abort();
    this.activeAbort = null;
    this.inFlight = false;
  }

  private makeSnapshot(nowMono: number): ServiceStatusSnapshot {
    const transportStale =
      this.issue === "unavailable" || this.issue === "rate_limited" || this.issue === "denied";
    const rows =
      this.response === null
        ? []
        : this.orderedServices.map((service): ServiceStatusDisplayedRow => {
            const deadline = this.deadlines.get(service.id);
            const expired = transportStale || deadline === undefined || nowMono >= deadline;
            return {
              service,
              status: expired ? "unknown" : service.status,
              processStatus: expired ? "unknown" : service.process_status,
              workStatus: expired ? "unknown" : service.work_status,
              reasonCodes: expired ? ["OBSERVATION_EXPIRED"] : service.reason_codes,
              expired,
            };
          });
    const counts = emptyCounts();
    for (const row of rows) {
      if (serviceStatusContributesToOverall(row.service.id)) counts[row.status] += 1;
    }
    const overallStatus =
      rows.length === 0
        ? "unknown"
        : serviceStatusOverall(rows.map(({ service, status }) => ({ id: service.id, status })));

    return {
      rows,
      overallStatus,
      counts,
      issue: this.issue,
      isFetching: this.inFlight,
      isStale: rows.some((row) => row.expired),
      lastRecordedAt: this.lastRecordedAt,
      policyState: this.response?.policy_state ?? null,
      receivedAtMono: this.receivedAtMono,
      nowMono,
    };
  }

  private emit(): void {
    this.snapshot = this.makeSnapshot(this.scheduler.now());
    for (const listener of this.listeners) listener();
  }

  private scheduleRequest(targetMono: number): void {
    if (!this.started || !this.active || this.blocked || this.inFlight) return;
    this.clearPollTimer();
    const now = this.scheduler.now();
    const floorAt =
      this.lastSettledAt === null ? now : this.lastSettledAt + SERVICE_STATUS_MIN_REQUEST_GAP_MS;
    const retryAt = this.retryAtMono ?? now;
    const scheduledAt = Math.max(targetMono, floorAt, retryAt);
    const delay = Math.max(0, scheduledAt - now);
    this.pollTimer = this.scheduler.setTimeout(() => {
      this.pollTimer = null;
      void this.fetchOnce();
    }, delay);
  }

  private async fetchOnce(): Promise<void> {
    if (!this.started || !this.active || this.blocked || this.inFlight) return;
    this.clearPollTimer();
    const generation = this.generation;
    const startedAt = this.scheduler.now();
    const abort = new AbortController();
    this.activeAbort = abort;
    this.inFlight = true;
    this.emit();

    let result: ServiceStatusResponse | null = null;
    let errorKind: "unavailable" | "rate_limited" | "denied" = "unavailable";
    let retryAfterMs: number | null = null;
    try {
      result = await this.request(abort.signal);
    } catch (error) {
      if (error instanceof ServiceStatusRequestError) {
        errorKind =
          error.kind === "denied"
            ? "denied"
            : error.kind === "throttled"
              ? "rate_limited"
              : "unavailable";
        retryAfterMs = error.retryAfterMs;
      }
    }

    if (generation !== this.generation || abort.signal.aborted) return;
    const settledAt = this.scheduler.now();
    const requestDurationMs = Math.max(0, settledAt - startedAt);
    this.inFlight = false;
    this.activeAbort = null;
    this.lastSettledAt = settledAt;

    if (result !== null) {
      this.response = result;
      this.orderedServices = responseOrder(result);
      this.deadlines = new Map<ServiceStatusId, number>(
        result.services.map(
          (service) =>
            [
              service.id,
              deadlineFor(service, result.generated_at, settledAt, requestDurationMs),
            ] as const,
        ),
      );
      this.lastRecordedAt = result.generated_at;
      this.receivedAtMono = settledAt;
      this.issue = null;
      this.retryAtMono = null;
      this.blocked = false;
      this.scheduleExpiry();
      this.emit();
      if (this.active) {
        this.scheduleRequest(settledAt + this.nextPollDelay(settledAt));
      }
      return;
    }

    this.issue = errorKind;
    if (errorKind === "denied") {
      this.blocked = true;
      this.response = null;
      this.orderedServices = [];
      this.deadlines.clear();
      this.lastRecordedAt = null;
      this.receivedAtMono = null;
      this.retryAtMono = null;
      this.clearExpiryTimer();
    } else if (errorKind === "rate_limited") {
      const backoff = Math.max(SERVICE_STATUS_MIN_REQUEST_GAP_MS, retryAfterMs ?? 0);
      this.retryAtMono = settledAt + backoff;
    } else {
      this.retryAtMono = null;
    }
    this.emit();
    if (this.active && !this.blocked) {
      const target = this.retryAtMono ?? settledAt + SERVICE_STATUS_BASE_POLL_MS;
      this.scheduleRequest(target);
    }
  }

  private nextPollDelay(settledAt: number): number {
    let earliestFutureDeadline: number | null = null;
    for (const deadline of this.deadlines.values()) {
      if (deadline <= settledAt) continue;
      if (earliestFutureDeadline === null || deadline < earliestFutureDeadline) {
        earliestFutureDeadline = deadline;
      }
    }
    if (earliestFutureDeadline === null) return SERVICE_STATUS_BASE_POLL_MS;
    const beforeExpiry = earliestFutureDeadline - settledAt - SERVICE_STATUS_REFRESH_LEAD_MS;
    return Math.min(
      SERVICE_STATUS_BASE_POLL_MS,
      Math.max(SERVICE_STATUS_MIN_REQUEST_GAP_MS, beforeExpiry),
    );
  }

  private scheduleExpiry(): void {
    this.clearExpiryTimer();
    if (!this.started || this.response === null) return;
    const now = this.scheduler.now();
    const futureDeadlines = [...this.deadlines.values()].filter((deadline) => deadline > now);
    if (futureDeadlines.length === 0) {
      this.emit();
      return;
    }
    const earliest = Math.min(...futureDeadlines);
    this.expiryTimer = this.scheduler.setTimeout(
      () => {
        this.expiryTimer = null;
        this.emit();
        this.scheduleExpiry();
      },
      Math.max(0, Math.ceil(earliest - now)),
    );
  }

  private clearPollTimer(): void {
    if (this.pollTimer === null) return;
    this.scheduler.clearTimeout(this.pollTimer);
    this.pollTimer = null;
  }

  private clearExpiryTimer(): void {
    if (this.expiryTimer === null) return;
    this.scheduler.clearTimeout(this.expiryTimer);
    this.expiryTimer = null;
  }
}
