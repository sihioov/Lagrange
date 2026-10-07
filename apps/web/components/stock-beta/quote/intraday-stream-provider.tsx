"use client";

import {
  createContext,
  type ReactNode,
  useContext,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import type { OwnerEquityV2MembershipModel } from "@/lib/products/equity-signals-contracts";
import type { IntradayStreamIdentity } from "@/lib/products/intraday-stream-contracts";
import { readyStreamIdentities } from "@/lib/products/intraday-stream-universe";
import {
  classifyIntradayStreamUpdate,
  createIntradayFrameCoalescer,
  type IntradayFrameCoalescer,
} from "./intraday-frame-coalescer";
import { bindIntradayStreamBrowser } from "./intraday-stream-browser";
import {
  IntradayStreamController,
  type IntradayStreamControllerView,
} from "./intraday-stream-controller";

const empty: IntradayStreamControllerView = {
  lifecycle: "inactive",
  failure: null,
  phase: "idle",
  rows: [],
  terminalReason: null,
};
type PageStream = {
  view: IntradayStreamControllerView;
  identities: readonly IntradayStreamIdentity[];
};
type ScopedDelivery = {
  scopeKey: string;
  view: IntradayStreamControllerView;
};
const StreamContext = createContext<PageStream>({ view: empty, identities: [] });
const useCommittedEffect = typeof window === "undefined" ? useEffect : useLayoutEffect;

export function IntradayStreamProvider({
  enabled,
  sessionKey,
  memberships,
  children,
}: {
  readonly enabled: boolean;
  readonly sessionKey: string | null;
  readonly memberships: readonly OwnerEquityV2MembershipModel[];
  readonly children: ReactNode;
}) {
  const { identities, valid } = useMemo(() => {
    try {
      return { identities: readyStreamIdentities(memberships), valid: true };
    } catch {
      return { identities: [], valid: false };
    }
  }, [memberships]);
  const current = useMemo(
    () => ({
      enabled: enabled && valid,
      owner: enabled && valid,
      sessionKey,
      identities,
      scopeKey: JSON.stringify([enabled && valid, sessionKey, identities]),
    }),
    [enabled, valid, sessionKey, identities],
  );
  const latest = useRef(current);
  const previousScope = useRef(current.scopeKey);
  const lastView = useRef<ScopedDelivery | null>(null);
  const coalescer = useRef<IntradayFrameCoalescer<ScopedDelivery> | null>(null);
  const binding = useRef<ReturnType<typeof bindIntradayStreamBrowser> | null>(null);
  const [delivery, setDelivery] = useState<ScopedDelivery>({ scopeKey: "", view: empty });
  // Clear data and cancel queued frames before a changed scope can paint.
  useCommittedEffect(() => {
    latest.current = current;
    if (previousScope.current === current.scopeKey) return;
    previousScope.current = current.scopeKey;
    coalescer.current?.cancel();
    lastView.current = null;
    setDelivery({ scopeKey: current.scopeKey, view: empty });
  }, [current]);
  // Apply a committed render to the active browser binding.
  useEffect(() => {
    latest.current = current;
    binding.current?.refresh();
  }, [current]);
  // A fresh instance for each effect setup also handles React's setup-cleanup-setup probe.
  useEffect(() => {
    const controller = new IntradayStreamController();
    const updates = createIntradayFrameCoalescer<ScopedDelivery>((next) => {
      if (latest.current.scopeKey === next.scopeKey) setDelivery(next);
    });
    coalescer.current = updates;
    const remove = controller.subscribe((view) => {
      const scope = latest.current;
      if (!scope.enabled || scope.sessionKey !== sessionKey) return;
      const next = { scopeKey: scope.scopeKey, view };
      const prior = lastView.current?.scopeKey === scope.scopeKey ? lastView.current.view : null;
      const kind = classifyIntradayStreamUpdate(prior, view);
      lastView.current = next;
      if (kind === "quote") updates.schedule(next);
      else if (kind === "control") updates.flush(next);
    });
    const owned = bindIntradayStreamBrowser(controller, () => latest.current);
    binding.current = owned;
    return () => {
      if (binding.current === owned) binding.current = null;
      if (coalescer.current === updates) coalescer.current = null;
      lastView.current = null;
      updates.dispose();
      remove();
      owned.dispose();
    };
  }, [sessionKey]);
  const view =
    current.enabled && sessionKey !== null && delivery.scopeKey === current.scopeKey
      ? {
          ...delivery.view,
          rows: delivery.view.rows.filter((row) =>
            identities.some(
              (identity) =>
                identity.membership_id === row.membership_id &&
                identity.instrument_id === row.instrument_id &&
                identity.generation === row.generation,
            ),
          ),
        }
      : empty;
  return <StreamContext value={{ view, identities }}>{children}</StreamContext>;
}

export function usePageIntradayStream(): PageStream {
  return useContext(StreamContext);
}
