"use client";

import { type KeyboardEvent as ReactKeyboardEvent, useEffect, useRef } from "react";
import type { OwnerBetaEquitySignalRowModel } from "@/lib/products/equity-signals-contracts";
import styles from "./dashboard.module.css";
import { useStockBetaSelection } from "./selection-provider";

export function StockBetaInstrumentSearch({
  copy,
  rows,
}: {
  readonly copy: {
    readonly searchHint: string;
    readonly searchLabel: string;
    readonly searchMatchesLabel: string;
    readonly searchPlaceholder: string;
  };
  readonly rows: readonly OwnerBetaEquitySignalRowModel[];
}) {
  const inputRef = useRef<HTMLInputElement>(null);
  const { searchQuery, setSearchQuery, visibleRows } = useStockBetaSelection();

  useEffect(() => {
    function handleGlobalKeyDown(event: globalThis.KeyboardEvent) {
      const target = event.target;
      if (
        event.key !== "/" ||
        (target instanceof HTMLElement &&
          (target.isContentEditable || ["INPUT", "SELECT", "TEXTAREA"].includes(target.tagName)))
      ) {
        return;
      }
      event.preventDefault();
      inputRef.current?.focus();
    }

    window.addEventListener("keydown", handleGlobalKeyDown);
    return () => window.removeEventListener("keydown", handleGlobalKeyDown);
  }, []);

  function clearOnEscape(event: ReactKeyboardEvent<HTMLInputElement>) {
    if (event.key !== "Escape") return;
    event.preventDefault();
    setSearchQuery("");
    event.currentTarget.blur();
  }

  return (
    <div className={styles["instrumentSearch"]} data-testid="stock-beta-instrument-search">
      <label htmlFor="stock-beta-instrument-search-input">
        <span>{copy.searchLabel}</span>
        <input
          aria-describedby="stock-beta-search-hint"
          autoComplete="off"
          id="stock-beta-instrument-search-input"
          list="stock-beta-instrument-options"
          onChange={(event) => setSearchQuery(event.target.value)}
          onKeyDown={clearOnEscape}
          placeholder={copy.searchPlaceholder}
          ref={inputRef}
          type="search"
          value={searchQuery}
        />
      </label>
      <kbd>/</kbd>
      <span className={styles["searchHint"]} id="stock-beta-search-hint">
        {copy.searchHint} · {visibleRows.length} {copy.searchMatchesLabel}
      </span>
      <datalist id="stock-beta-instrument-options">
        {rows.map((row) => (
          <option key={row.instrument_id} label={row.instrument_name} value={row.instrument_id} />
        ))}
      </datalist>
    </div>
  );
}
