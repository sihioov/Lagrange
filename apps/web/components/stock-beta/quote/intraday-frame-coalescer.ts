import type { IntradayStreamControllerView } from "./intraday-stream-controller";

export type IntradayFrameScheduler = {
  readonly request: (callback: FrameRequestCallback) => number;
  readonly cancel: (handle: number) => void;
};

export type IntradayFrameCoalescer<T> = {
  readonly schedule: (value: T) => void;
  readonly flush: (value: T) => void;
  readonly cancel: () => void;
  readonly dispose: () => void;
};

export type IntradayStreamUpdateKind = "unchanged" | "quote" | "control";

function rowIdentity(row: IntradayStreamControllerView["rows"][number]): string {
  return JSON.stringify([row.membership_id, row.instrument_id, row.generation]);
}

function sameRowControl(
  previous: IntradayStreamControllerView["rows"][number],
  next: IntradayStreamControllerView["rows"][number],
): boolean {
  return (
    previous.row_generation === next.row_generation &&
    previous.subscription === next.subscription &&
    previous.connection === next.connection &&
    previous.market_state === next.market_state &&
    previous.freshness === next.freshness &&
    previous.availability === next.availability &&
    previous.reason_code === next.reason_code &&
    previous.gap_open === next.gap_open &&
    previous.session_has_gap === next.session_has_gap &&
    previous.gap_generation === next.gap_generation
  );
}

/** Batch only quote changes whose stream, row set, and control state stayed stable. */
export function classifyIntradayStreamUpdate(
  previous: IntradayStreamControllerView | null,
  next: IntradayStreamControllerView,
): IntradayStreamUpdateKind {
  if (
    previous === null ||
    previous.lifecycle !== next.lifecycle ||
    previous.failure !== next.failure ||
    previous.phase !== next.phase ||
    previous.terminalReason !== next.terminalReason ||
    previous.rows.length !== next.rows.length
  )
    return "control";

  const previousRows = new Map(previous.rows.map((row) => [rowIdentity(row), row]));
  if (previousRows.size !== next.rows.length) return "control";

  let quoteChanged = false;
  for (const row of next.rows) {
    const prior = previousRows.get(rowIdentity(row));
    if (
      prior === undefined ||
      !sameRowControl(prior, row) ||
      (prior.quote !== null && row.quote === null)
    )
      return "control";
    if (JSON.stringify(prior.quote) !== JSON.stringify(row.quote)) quoteChanged = true;
  }
  return quoteChanged ? "quote" : "unchanged";
}

/** Keep only the latest quote view until the next browser frame. */
export function createIntradayFrameCoalescer<T>(
  deliver: (value: T) => void,
  scheduler: IntradayFrameScheduler = {
    request: (callback) => requestAnimationFrame(callback),
    cancel: (handle) => cancelAnimationFrame(handle),
  },
): IntradayFrameCoalescer<T> {
  let frame: number | null = null;
  let pending: T | undefined;
  let hasPending = false;
  let ticket = 0;
  let disposed = false;

  const cancel = (): void => {
    ticket += 1;
    if (frame !== null) scheduler.cancel(frame);
    frame = null;
    pending = undefined;
    hasPending = false;
  };

  return {
    schedule(value) {
      if (disposed) return;
      pending = value;
      hasPending = true;
      if (frame !== null) return;
      const scheduledTicket = ++ticket;
      frame = scheduler.request(() => {
        if (disposed || scheduledTicket !== ticket) return;
        frame = null;
        if (!hasPending) return;
        const latest = pending as T;
        pending = undefined;
        hasPending = false;
        deliver(latest);
      });
    },
    flush(value) {
      if (disposed) return;
      cancel();
      deliver(value);
    },
    cancel,
    dispose() {
      if (disposed) return;
      disposed = true;
      cancel();
    },
  };
}
