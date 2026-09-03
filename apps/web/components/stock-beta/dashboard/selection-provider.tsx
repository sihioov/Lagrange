"use client";

import { createContext, type ReactNode, useCallback, useContext, useMemo, useState } from "react";
import type { OwnerBetaEquitySignalRowModel } from "@/lib/products/equity-signals-contracts";
import type { StockBetaNumericRowKey } from "./metric-columns";

const DEFAULT_VISIBLE_METRIC_KEYS: readonly StockBetaNumericRowKey[] = [
  "score",
  "return_20",
  "volume_ratio_20_60",
];

type StockBetaSelectionContextValue = {
  readonly searchQuery: string;
  readonly selectRow: (instrumentId: string) => void;
  readonly setSearchQuery: (query: string) => void;
  readonly toggleMetricColumn: (key: StockBetaNumericRowKey) => void;
  readonly visibleMetricKeys: readonly StockBetaNumericRowKey[];
  readonly visibleRows: readonly OwnerBetaEquitySignalRowModel[];
  readonly selectedRow: OwnerBetaEquitySignalRowModel | undefined;
};

const StockBetaSelectionContext = createContext<StockBetaSelectionContextValue | undefined>(
  undefined,
);

export function StockBetaSelectionProvider({
  children,
  initialSelectedInstrumentId,
  rows,
}: {
  readonly children: ReactNode;
  readonly initialSelectedInstrumentId?: string;
  readonly rows: readonly OwnerBetaEquitySignalRowModel[];
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
    return rows.filter(
      (row) =>
        row.instrument_id.toLocaleLowerCase().includes(query) ||
        row.instrument_name.toLocaleLowerCase().includes(query),
    );
  }, [rows, searchQuery]);
  const selectedRow =
    visibleRows.find((row) => row.instrument_id === selectedInstrumentId) ?? visibleRows[0];
  const selectRow = useCallback(
    (instrumentId: string) => {
      if (rows.some((row) => row.instrument_id === instrumentId)) {
        setSelectedInstrumentId(instrumentId);
      }
    },
    [rows],
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
      setSearchQuery,
      toggleMetricColumn,
      visibleMetricKeys,
      visibleRows,
      selectedRow,
    }),
    [searchQuery, selectRow, toggleMetricColumn, visibleMetricKeys, visibleRows, selectedRow],
  );

  return (
    <StockBetaSelectionContext.Provider value={value}>
      {children}
    </StockBetaSelectionContext.Provider>
  );
}

export function useStockBetaSelection(): StockBetaSelectionContextValue {
  const value = useContext(StockBetaSelectionContext);
  if (value === undefined) {
    throw new Error("StockBetaSelectionProvider is required for interactive dashboard widgets.");
  }
  return value;
}
