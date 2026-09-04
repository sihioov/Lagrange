"use client";

import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import type { OwnerEquityV2SignalModel } from "@/lib/products/equity-signals-contracts";
import type { StockBetaNumericRowKey } from "./metric-columns";

const DEFAULT_VISIBLE_METRIC_KEYS: readonly StockBetaNumericRowKey[] = [
  "score",
  "return_20",
  "volume_ratio_20_60",
];

type StockBetaSelectionContextValue = {
  readonly searchQuery: string;
  readonly selectRow: (instrumentId: string) => void;
  readonly selectedInstrumentId: string | null;
  readonly selectedRow: OwnerEquityV2SignalModel | undefined;
  readonly setSearchQuery: (query: string) => void;
  readonly toggleMetricColumn: (key: StockBetaNumericRowKey) => void;
  readonly visibleMetricKeys: readonly StockBetaNumericRowKey[];
  readonly visibleRows: readonly OwnerEquityV2SignalModel[];
};

const StockBetaSelectionContext = createContext<StockBetaSelectionContextValue | undefined>(
  undefined,
);

export function StockBetaSelectionProvider({
  children,
  initialSelectedInstrumentId,
  onSelectionChange,
  rows,
  selectedInstrumentId: controlledSelectedInstrumentId,
}: {
  readonly children: ReactNode;
  readonly initialSelectedInstrumentId?: string;
  readonly onSelectionChange?: (instrumentId: string | null) => void;
  readonly rows: readonly OwnerEquityV2SignalModel[];
  readonly selectedInstrumentId?: string | null;
}) {
  const initialSelection =
    initialSelectedInstrumentId !== undefined &&
    rows.some((row) => row.instrument_id === initialSelectedInstrumentId)
      ? initialSelectedInstrumentId
      : (rows[0]?.instrument_id ?? null);
  const [selectedInstrumentId, setSelectedInstrumentId] = useState(initialSelection);
  const [searchQuery, setSearchQuery] = useState("");
  const [visibleMetricKeys, setVisibleMetricKeys] = useState<readonly StockBetaNumericRowKey[]>(
    DEFAULT_VISIBLE_METRIC_KEYS,
  );
  const visibleRows = useMemo(() => {
    const query = searchQuery.trim().toLocaleLowerCase();
    if (query === "") return rows;
    return rows.filter((row) => row.instrument_id.toLocaleLowerCase().includes(query));
  }, [rows, searchQuery]);
  const isControlled = controlledSelectedInstrumentId !== undefined;
  const requestedSelectedInstrumentId = isControlled
    ? controlledSelectedInstrumentId
    : selectedInstrumentId;
  const selectedRow =
    visibleRows.find((row) => row.instrument_id === requestedSelectedInstrumentId) ??
    visibleRows[0];
  const effectiveSelectedInstrumentId = selectedRow?.instrument_id ?? null;
  const selectionUnset = useRef(Symbol("selection-unset"));
  const previousEffectiveSelection = useRef<string | null | symbol>(selectionUnset.current);
  useEffect(() => {
    if (!isControlled && requestedSelectedInstrumentId !== effectiveSelectedInstrumentId) {
      setSelectedInstrumentId(effectiveSelectedInstrumentId);
    }
    const previous = previousEffectiveSelection.current;
    if (previous !== selectionUnset.current && previous !== effectiveSelectedInstrumentId) {
      onSelectionChange?.(effectiveSelectedInstrumentId);
    }
    previousEffectiveSelection.current = effectiveSelectedInstrumentId;
  }, [
    effectiveSelectedInstrumentId,
    isControlled,
    onSelectionChange,
    requestedSelectedInstrumentId,
  ]);
  const selectRow = useCallback(
    (instrumentId: string) => {
      if (!rows.some((row) => row.instrument_id === instrumentId)) return;
      if (isControlled) {
        if (requestedSelectedInstrumentId === instrumentId) return;
        previousEffectiveSelection.current = instrumentId;
        onSelectionChange?.(instrumentId);
        return;
      }
      setSelectedInstrumentId(instrumentId);
    },
    [isControlled, onSelectionChange, requestedSelectedInstrumentId, rows],
  );
  const toggleMetricColumn = useCallback((key: StockBetaNumericRowKey) => {
    setVisibleMetricKeys((current) => {
      if (key === "score") return current;
      return current.includes(key) ? current.filter((item) => item !== key) : [...current, key];
    });
  }, []);
  const value = useMemo(
    () => ({
      searchQuery,
      selectRow,
      selectedInstrumentId: effectiveSelectedInstrumentId,
      selectedRow,
      setSearchQuery,
      toggleMetricColumn,
      visibleMetricKeys,
      visibleRows,
    }),
    [
      effectiveSelectedInstrumentId,
      searchQuery,
      selectRow,
      selectedRow,
      toggleMetricColumn,
      visibleMetricKeys,
      visibleRows,
    ],
  );

  return (
    <StockBetaSelectionContext.Provider value={value}>
      {children}
    </StockBetaSelectionContext.Provider>
  );
}

export function useStockBetaSelection(): StockBetaSelectionContextValue {
  const value = useContext(StockBetaSelectionContext);
  if (value === undefined)
    throw new Error("StockBetaSelectionProvider is required for interactive dashboard widgets.");
  return value;
}
