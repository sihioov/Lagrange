type StockBetaSignalSnapshot = {
  readonly rows: readonly {
    readonly instrument_id: string;
  }[];
};

export type StockBetaSignalRefreshOutcome = "blocked" | "failure" | "stale" | "success";

type StockBetaSignalRefreshHandlers<Snapshot> = {
  readonly onFailure: (error: unknown) => void;
  readonly onSuccess: (snapshot: Snapshot) => void;
};

export class StockBetaSignalRefreshCoordinator<Snapshot extends StockBetaSignalSnapshot> {
  private epoch = 0;
  private readonly blockedInstruments = new Set<string>();

  invalidate(): void {
    this.epoch += 1;
  }

  blockInstrument(instrumentId: string): void {
    this.invalidate();
    this.blockedInstruments.add(instrumentId);
  }

  releaseInstrument(instrumentId: string): void {
    this.blockedInstruments.delete(instrumentId);
  }

  acceptsSnapshot(snapshot: Snapshot): boolean {
    return !snapshot.rows.some((row) => this.blockedInstruments.has(row.instrument_id));
  }

  async run(
    load: () => Promise<Snapshot>,
    handlers: StockBetaSignalRefreshHandlers<Snapshot>,
  ): Promise<StockBetaSignalRefreshOutcome> {
    const requestEpoch = ++this.epoch;
    let snapshot: Snapshot;

    try {
      snapshot = await load();
    } catch (error) {
      if (requestEpoch !== this.epoch) return "stale";
      handlers.onFailure(error);
      return "failure";
    }

    if (requestEpoch !== this.epoch) return "stale";
    if (!this.acceptsSnapshot(snapshot)) return "blocked";

    handlers.onSuccess(snapshot);
    return "success";
  }
}
