"use client";

import { useEffect, useMemo, useState } from "react";
import { subscribeToBrowserLogout } from "@/lib/api/browser-lifecycle";
import type { IntradayQuoteIdentity } from "@/lib/products/intraday-quotes-contracts";
import {
  createIntradayQuoteLoadCoordinator,
  type IntradayQuoteClock,
  type IntradayQuoteCoordinatorOptions,
  type IntradayQuoteLoadContext,
  type IntradayQuoteLoadCoordinator,
  type IntradayQuoteLoadPhase,
  type IntradayQuoteLoadState,
} from "./quote-load-coordinator";

export type IntradayQuoteSurface = "dashboard" | "detail";

export type UseIntradayQuoteOptions = {
  readonly clock?: IntradayQuoteClock;
  readonly coordinatorOptions?: Omit<IntradayQuoteCoordinatorOptions, "clock">;
  readonly enabled: boolean;
  readonly identity: IntradayQuoteIdentity | null;
  readonly placementVisibility?: Readonly<
    Partial<Record<"desktop" | "tablet" | "mobile", boolean>>
  >;
  readonly sessionKey: string | null;
  readonly surface: IntradayQuoteSurface;
};

export type UseIntradayQuoteResult = {
  readonly coordinator: IntradayQuoteLoadCoordinator;
  readonly isActive: boolean;
  readonly phase: IntradayQuoteLoadPhase;
  readonly state: IntradayQuoteLoadState;
};

function mediaQueries(surface: IntradayQuoteSurface): readonly string[] {
  return surface === "dashboard"
    ? [
        "(min-width: 80rem)",
        "(max-width: 79.9375rem) and (min-width: 51.3125rem)",
        "(max-width: 51.25rem)",
      ]
    : [
        "(min-width: 64.0625rem)",
        "(max-width: 64rem) and (min-width: 51.3125rem)",
        "(max-width: 51.25rem)",
      ];
}

function currentBreakpoint(surface: IntradayQuoteSurface): "desktop" | "tablet" | "mobile" {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") return "desktop";
  const queries = mediaQueries(surface);
  if (window.matchMedia(queries[0] ?? "").matches) return "desktop";
  if (window.matchMedia(queries[1] ?? "").matches) return "tablet";
  return "mobile";
}

function useResponsiveVisibility(
  surface: IntradayQuoteSurface,
  placementVisibility: UseIntradayQuoteOptions["placementVisibility"],
): boolean {
  const [breakpoint, setBreakpoint] = useState(() => currentBreakpoint(surface));
  useEffect(() => {
    if (typeof window === "undefined" || typeof window.matchMedia !== "function") return;
    const queries = mediaQueries(surface);
    const lists = queries.map((query) => window.matchMedia(query));
    const update = (): void => setBreakpoint(currentBreakpoint(surface));
    for (const list of lists) {
      if (list.addEventListener !== undefined) list.addEventListener("change", update);
      else list.addListener?.(update);
    }
    update();
    return () => {
      for (const list of lists) {
        if (list.removeEventListener !== undefined) list.removeEventListener("change", update);
        else list.removeListener?.(update);
      }
    };
  }, [surface]);
  return placementVisibility?.[breakpoint] !== false;
}

function useBrowserActivity(): { readonly online: boolean; readonly visible: boolean } {
  const [online, setOnline] = useState(
    () => typeof navigator === "undefined" || navigator.onLine !== false,
  );
  const [visible, setVisible] = useState(
    () => typeof document === "undefined" || document.visibilityState !== "hidden",
  );
  useEffect(() => {
    if (typeof window === "undefined") return;
    const updateOnline = (): void => setOnline(window.navigator.onLine !== false);
    const updateVisible = (): void => setVisible(document.visibilityState !== "hidden");
    updateOnline();
    updateVisible();
    window.addEventListener("online", updateOnline);
    window.addEventListener("offline", updateOnline);
    document.addEventListener("visibilitychange", updateVisible);
    return () => {
      window.removeEventListener("online", updateOnline);
      window.removeEventListener("offline", updateOnline);
      document.removeEventListener("visibilitychange", updateVisible);
    };
  }, []);
  return { online, visible };
}

function contextFor(
  enabled: boolean,
  identity: IntradayQuoteIdentity | null,
  sessionKey: string | null,
  online: boolean,
  visible: boolean,
): IntradayQuoteLoadContext {
  return {
    enabled,
    identity,
    mounted: true,
    online,
    snapshotKey: sessionKey,
    visible,
  };
}

export function useIntradayQuote(options: UseIntradayQuoteOptions): UseIntradayQuoteResult {
  const coordinator = useMemo(
    () =>
      createIntradayQuoteLoadCoordinator({
        ...(options.clock === undefined ? {} : { clock: options.clock }),
        ...options.coordinatorOptions,
      }),
    [options.clock, options.coordinatorOptions],
  );
  const [state, setState] = useState<IntradayQuoteLoadState>(coordinator.getState());
  const placementVisible = useResponsiveVisibility(options.surface, options.placementVisibility);
  const activity = useBrowserActivity();
  const visible = placementVisible && activity.visible;
  const identityValue = options.identity;
  const membershipId = identityValue?.membership_id;
  const instrumentId = identityValue?.instrument_id;
  const generation = identityValue?.generation;
  const stableIdentity = useMemo(
    () =>
      membershipId === undefined || instrumentId === undefined || generation === undefined
        ? null
        : { generation, instrument_id: instrumentId, membership_id: membershipId },
    [generation, instrumentId, membershipId],
  );
  const enabled = options.enabled;
  const sessionKey = options.sessionKey;

  useEffect(() => coordinator.subscribe(setState), [coordinator]);
  useEffect(() => {
    coordinator.setContext(
      contextFor(enabled, stableIdentity, sessionKey, activity.online, visible),
    );
    return () => coordinator.setContext({ mounted: false, visible: false });
  }, [activity.online, coordinator, enabled, sessionKey, stableIdentity, visible]);
  useEffect(() => subscribeToBrowserLogout(() => coordinator.stop()), [coordinator]);
  useEffect(() => () => coordinator.destroy(), [coordinator]);

  return {
    coordinator,
    isActive: options.enabled && options.identity !== null && activity.online && visible,
    phase: state.phase,
    state,
  };
}

export const useStockBetaIntradayQuote = useIntradayQuote;
