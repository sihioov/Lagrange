import {
  ageIntradayStreamRow,
  IntradayStreamContractError,
  type IntradayStreamEvent,
  type IntradayStreamIdentity,
  type IntradayStreamRow,
  intradayStreamIdentitySchema,
  intradayStreamTimeNs,
  intradayStreamUuidSchema,
  parseIntradayStreamEvent,
} from "@/lib/products/intraday-stream-contracts";

export type IntradayStreamPhase =
  | "idle"
  | "awaiting_snapshot"
  | "ready"
  | "disconnected"
  | "resync_required"
  | "terminated";

export type IntradayStreamView = {
  phase: IntradayStreamPhase;
  rows: IntradayStreamRow[];
  terminalReason: "access_revoked" | "lease_expired" | "closed" | null;
};

type RowRecord = {
  row: IntradayStreamRow;
  quoteMark: { version: bigint; epoch: string; ordinal: bigint } | null;
};

function nonlive(row: IntradayStreamRow, reason: "CONNECTION_LOST" | "RESYNC_REQUIRED") {
  return {
    ...row,
    connection: "DISCONNECTED" as const,
    availability: row.quote === null ? row.availability : ("LAST_KNOWN" as const),
    reason_code: row.reason_code ?? reason,
  };
}

function record(row: IntradayStreamRow, previous?: RowRecord): RowRecord {
  const sameNamespace = previous?.row.row_generation === row.row_generation;
  const prior = sameNamespace ? previous : undefined;
  if (prior && BigInt(row.state_version) < BigInt(prior.row.state_version))
    throw new IntradayStreamContractError();
  let quoteMark = prior?.quoteMark ?? null;
  if (row.quote !== null) {
    const version = BigInt(row.quote.quote_version);
    const ordinal = BigInt(row.quote.receive_ordinal);
    if (quoteMark) {
      if (version < quoteMark.version) throw new IntradayStreamContractError();
      if (version === quoteMark.version) {
        if (
          row.quote.epoch !== quoteMark.epoch ||
          ordinal !== quoteMark.ordinal ||
          (prior?.row.quote && JSON.stringify(row.quote) !== JSON.stringify(prior.row.quote))
        )
          throw new IntradayStreamContractError();
      } else if (
        (row.quote.epoch === quoteMark.epoch && ordinal <= quoteMark.ordinal) ||
        (prior && BigInt(row.state_version) <= BigInt(prior.row.state_version))
      ) {
        throw new IntradayStreamContractError();
      }
    }
    quoteMark = { version, epoch: row.quote.epoch, ordinal };
  }
  return { row, quoteMark };
}

/**
 * One tab's bounded, in-memory delivery state. The network owner supplies a local
 * connection token so callbacks from an already-closed EventSource cannot re-enter.
 * This class does not create demand, make requests, persist prices or claim access.
 */
export class IntradayStreamState {
  private readonly identities = new Map<string, IntradayStreamIdentity>();
  private rows = new Map<string, RowRecord>();
  private phase: IntradayStreamPhase = "idle";
  private terminalReason: IntradayStreamView["terminalReason"] = null;
  private connectionToken = 0;
  private accepting = false;
  private streamId: string | null = null;
  private priorStreamId: string | null = null;
  private sequence = 0n;
  private serverTime: string | null = null;
  private observedAt = 0;
  private leaseExpiresAt: bigint | null = null;
  private resetSeen = false;

  constructor(
    private readonly leaseId: string,
    identities: readonly IntradayStreamIdentity[],
  ) {
    const lease = intradayStreamUuidSchema.safeParse(leaseId);
    if (!lease.success || identities.length < 1 || identities.length > 30)
      throw new IntradayStreamContractError();
    const instruments = new Set<string>();
    for (const input of identities) {
      const parsed = intradayStreamIdentitySchema.safeParse(input);
      if (
        !parsed.success ||
        this.identities.has(parsed.data.membership_id) ||
        instruments.has(parsed.data.instrument_id)
      )
        throw new IntradayStreamContractError();
      this.identities.set(parsed.data.membership_id, parsed.data);
      instruments.add(parsed.data.instrument_id);
    }
  }

  beginConnection(): number {
    if (this.phase === "terminated") throw new IntradayStreamContractError();
    this.priorStreamId = this.streamId ?? this.priorStreamId;
    this.streamId = null;
    this.sequence = 0n;
    this.resetSeen = false;
    this.accepting = true;
    this.phase = "awaiting_snapshot";
    for (const value of this.rows.values()) value.row = nonlive(value.row, "RESYNC_REQUIRED");
    return ++this.connectionToken;
  }

  /** Only call with an accepted mutation response for this same consumer/lease. */
  confirmLeaseExpiry(leaseId: string, expiresAt: string): void {
    if (this.phase === "terminated" || leaseId !== this.leaseId)
      throw new IntradayStreamContractError();
    const expiry = intradayStreamTimeNs(expiresAt);
    if (this.leaseExpiresAt !== null && expiry < this.leaseExpiresAt)
      throw new IntradayStreamContractError();
    this.leaseExpiresAt = expiry;
  }

  receive(
    token: number,
    kind: string,
    data: string,
    eventId: string,
    observedAt: number,
  ): "applied" | "ignored" | "resync" | "terminated" {
    if (token !== this.connectionToken || !this.accepting || this.phase === "terminated")
      return "ignored";
    try {
      const event = parseIntradayStreamEvent(kind, data, eventId);
      if (this.streamId !== null && this.streamId !== event.stream_id)
        throw new IntradayStreamContractError();
      if (this.streamId !== null && BigInt(event.event_sequence) <= this.sequence) return "ignored";
      if (
        !Number.isFinite(observedAt) ||
        observedAt < 0 ||
        (this.serverTime !== null &&
          (observedAt < this.observedAt ||
            intradayStreamTimeNs(event.server_time) < intradayStreamTimeNs(this.serverTime)))
      )
        throw new IntradayStreamContractError();
      if (this.streamId === null && event.stream_id === this.priorStreamId)
        throw new IntradayStreamContractError();
      if (
        (event.kind === "status" && event.body.reason_code === "ACCESS_REVOKED") ||
        ((event.kind === "snapshot" || event.kind === "delta") &&
          event.body.rows.some((row) => row.reason_code === "ACCESS_REVOKED"))
      ) {
        this.terminate("access_revoked");
        return "terminated";
      }
      if (event.kind === "reset") {
        this.rows.clear();
        this.resetSeen = true;
        this.phase = "awaiting_snapshot";
      } else if (event.kind === "snapshot") {
        if (event.body.lease_id !== this.leaseId || (this.connectionToken > 1 && !this.resetSeen))
          throw new IntradayStreamContractError();
        const next = this.checkedRows(event, false);
        this.rows = next;
        const expiry = intradayStreamTimeNs(event.body.lease_expires_at);
        this.leaseExpiresAt =
          this.leaseExpiresAt === null || expiry > this.leaseExpiresAt
            ? expiry
            : this.leaseExpiresAt;
        this.phase = "ready";
      } else {
        if (this.phase !== "ready") throw new IntradayStreamContractError();
        if (event.kind === "delta") this.rows = this.checkedRows(event, true);
        else this.applyStatus(event);
      }
      this.streamId = event.stream_id;
      this.sequence = BigInt(event.event_sequence);
      this.serverTime = event.server_time;
      this.observedAt = observedAt;
      return "applied";
    } catch {
      // A partial or malformed event must never leave a partially updated board.
      this.rows.clear();
      this.phase = "resync_required";
      this.accepting = false;
      return "resync";
    }
  }

  view(monotonicNow: number): IntradayStreamView {
    if (this.phase === "terminated" || this.serverTime === null)
      return { phase: this.phase, rows: [], terminalReason: this.terminalReason };
    const elapsed = monotonicNow - this.observedAt;
    if (!Number.isFinite(elapsed) || elapsed < 0 || elapsed > 90 * 86400_000) {
      this.rows.clear();
      this.phase = "resync_required";
      this.accepting = false;
      return { phase: this.phase, rows: [], terminalReason: null };
    }
    const now = intradayStreamTimeNs(this.serverTime) + BigInt(Math.ceil(elapsed * 1_000_000));
    if (this.leaseExpiresAt !== null && now >= this.leaseExpiresAt) {
      this.terminate("lease_expired");
      return { phase: this.phase, rows: [], terminalReason: this.terminalReason };
    }
    const silent = elapsed >= 5_000;
    const rows = [...this.rows.values()].map(({ row }) => {
      const aged = ageIntradayStreamRow(row, this.serverTime ?? "", elapsed);
      return silent ? nonlive(aged, "CONNECTION_LOST") : aged;
    });
    return {
      phase: silent && this.phase === "ready" ? "disconnected" : this.phase,
      rows: structuredClone(rows),
      terminalReason: this.terminalReason,
    };
  }

  close(): void {
    this.terminate("closed");
  }

  disconnect(token: number): void {
    if (token !== this.connectionToken || this.phase === "terminated") return;
    this.accepting = false;
    this.phase = "disconnected";
    for (const value of this.rows.values()) value.row = nonlive(value.row, "CONNECTION_LOST");
  }

  private terminate(reason: IntradayStreamView["terminalReason"]): void {
    this.rows.clear();
    this.identities.clear();
    this.accepting = false;
    this.serverTime = null;
    this.leaseExpiresAt = null;
    this.phase = "terminated";
    this.terminalReason = reason;
  }

  private checkedRows(
    event: Extract<IntradayStreamEvent, { kind: "snapshot" | "delta" }>,
    delta: boolean,
  ): Map<string, RowRecord> {
    const next = delta ? new Map(this.rows) : new Map<string, RowRecord>();
    for (const row of event.body.rows) {
      const identity = this.identities.get(row.membership_id);
      if (
        !identity ||
        identity.instrument_id !== row.instrument_id ||
        identity.generation !== row.generation ||
        (delta && !this.rows.has(row.membership_id))
      )
        throw new IntradayStreamContractError();
      next.set(row.membership_id, record(row, this.rows.get(row.membership_id)));
    }
    if (next.size > 30) throw new IntradayStreamContractError();
    return next;
  }

  private applyStatus(event: Extract<IntradayStreamEvent, { kind: "status" }>): void {
    const status = event.body;
    const unavailable =
      status.reason_code === "FEATURE_DISABLED" ||
      status.reason_code === "CALENDAR_UNAVAILABLE" ||
      status.reason_code === "SESSION_WINDOW_UNAVAILABLE";
    for (const value of this.rows.values()) {
      const row = ageIntradayStreamRow(value.row, event.server_time);
      const unhealthy =
        status.connection !== "CONNECTED" || status.reason_code !== null || status.gap_open;
      value.row = {
        ...row,
        connection: status.connection,
        gap_open: status.gap_open,
        session_has_gap: status.session_has_gap,
        gap_generation: status.gap_generation,
        ...(unhealthy
          ? {
              availability: row.quote === null ? row.availability : ("LAST_KNOWN" as const),
              reason_code: status.reason_code ?? "CONNECTION_LOST",
            }
          : {}),
        ...(unavailable
          ? {
              quote: null,
              freshness: "UNAVAILABLE" as const,
              availability: "UNAVAILABLE" as const,
              reason_code: status.reason_code,
            }
          : {}),
      };
    }
  }
}
