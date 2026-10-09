import type { StockBetaDashboardViewModel } from "../../../components/stock-beta/dashboard/types";
import type {
  OwnerEquityV2ChartModel,
  OwnerEquityV2LatestSignalsModel,
} from "../../../lib/products/equity-signals-contracts";

type ChartContext = {
  snapshotId: string;
  instrumentId: string;
  generation: number;
  range: string;
  asOf: string;
};

export type CommittedStockBetaChart = {
  chart: ChartContext | null;
  seed: ChartContext | null;
  signal: Omit<ChartContext, "range"> | null;
  serverSignal: Omit<ChartContext, "range"> | null;
  serverIncludesAddedInstrument: boolean;
  seedErrorNull: boolean;
  chartErrorNull: boolean;
  chartState: string | null;
  selectedRange: string | null;
  chartIsServerSeed: boolean;
};

// Test-only observation of the *committed* tree. DOM-attached fibers can be the
// stale alternate, so start at root.current and find this exact accessible host.
// This deliberately fails closed if React internals or component props change.
// No product marker, hook, data mutation, full chart, or Flight body is involved.
export function readCommittedStockBetaChart(element: Element): CommittedStockBetaChart | null {
  type Fiber = {
    child?: Fiber | null;
    sibling?: Fiber | null;
    return?: Fiber | null;
    stateNode?: unknown;
    memoizedProps?: Record<string, unknown>;
  };
  const key = Object.keys(element).find((name) => name.startsWith("__reactFiber$"));
  let root = key ? ((element as unknown as Record<string, unknown>)[key] as Fiber) : null;
  for (let depth = 0; root?.return && depth < 256; depth++) root = root.return;
  const current = (root?.stateNode as { current?: Fiber } | undefined)?.current;
  const pending = current ? [current] : [];
  let host: Fiber | null = null;
  for (let visited = 0; pending.length > 0 && visited < 5_000; visited++) {
    const fiber = pending.pop();
    if (!fiber) continue;
    if (fiber.stateNode === element) {
      host = fiber;
      break;
    }
    if (fiber.sibling) pending.push(fiber.sibling);
    if (fiber.child) pending.push(fiber.child);
  }
  if (host === null) return null;
  let view: StockBetaDashboardViewModel | undefined;
  let props: Record<string, unknown> | undefined;
  let depth = 0;
  for (let fiber: Fiber | null = host; fiber && depth++ < 256; fiber = fiber.return ?? null) {
    const candidate = fiber.memoizedProps;
    view ??= candidate?.["viewModel"] as StockBetaDashboardViewModel | undefined;
    if (candidate && Object.hasOwn(candidate, "initialChart")) props = candidate;
  }
  if (view === undefined || props === undefined) return null;
  const chart = view.chartData;
  const seed = props["initialChart"] as OwnerEquityV2ChartModel | null | undefined;
  const serverSignals = props["initialSignals"] as OwnerEquityV2LatestSignalsModel | null;
  const context = (value: OwnerEquityV2ChartModel | null | undefined): ChartContext | null => {
    if (!value) return null;
    // Permit only typed context in diagnostics, never arbitrary prop strings.
    if (
      !/^[0-9a-f-]{36}$/i.test(value.snapshot_id) ||
      !/^\d{6}\.KRX$/.test(value.instrument_id) ||
      !Number.isSafeInteger(value.generation) ||
      !/^(1m|3m|6m|1y)$/.test(value.range) ||
      !/^\d{4}-\d{2}-\d{2}$/.test(value.as_of)
    )
      return null;
    return {
      snapshotId: value.snapshot_id,
      instrumentId: value.instrument_id,
      generation: value.generation,
      range: value.range,
      asOf: value.as_of,
    };
  };
  const signal = (signals: OwnerEquityV2LatestSignalsModel | null | undefined) => {
    const row = signals?.rows.find((candidate) => candidate.instrument_id === "000001.KRX");
    const value =
      signals && row
        ? context({
            snapshot_id: signals.snapshot.snapshot_id,
            instrument_id: row.instrument_id,
            generation: row.generation,
            range: "1y",
            as_of: signals.snapshot.as_of,
          } as OwnerEquityV2ChartModel)
        : null;
    if (value === null) return null;
    const { range: _range, ...result } = value;
    return result;
  };
  return {
    chart: context(chart),
    seed: context(seed),
    signal: signal(view.signals),
    serverSignal: signal(serverSignals),
    serverIncludesAddedInstrument:
      serverSignals?.rows.some((row) => row.instrument_id === "005930.KRX") === true,
    seedErrorNull: props["initialChartError"] === null,
    chartErrorNull: view.chartError === null,
    chartState: view.chartState?.kind ?? null,
    selectedRange: view.chartRange ?? null,
    // Workspace applies initialChart directly; a successful browser fallback is
    // a different object. See stock-beta-workspace.tsx's initial-props effect.
    chartIsServerSeed: seed != null && chart === seed,
  };
}

export function isCurrentServerSeed(
  observed: CommittedStockBetaChart | null,
  expected: ChartContext,
): boolean {
  const matches = (value: ChartContext | null | undefined) =>
    value !== null &&
    value !== undefined &&
    value.snapshotId === expected.snapshotId &&
    value.instrumentId === expected.instrumentId &&
    value.generation === expected.generation &&
    value.range === expected.range &&
    value.asOf === expected.asOf;
  return (
    observed !== null &&
    observed.serverIncludesAddedInstrument &&
    observed.seedErrorNull &&
    observed.chartErrorNull &&
    observed.chartIsServerSeed &&
    observed.chartState === "ready" &&
    observed.selectedRange === expected.range &&
    matches(observed.seed) &&
    matches(observed.chart) &&
    matches(observed.signal && { ...observed.signal, range: expected.range }) &&
    matches(observed.serverSignal && { ...observed.serverSignal, range: expected.range })
  );
}
