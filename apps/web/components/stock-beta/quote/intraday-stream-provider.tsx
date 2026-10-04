"use client";

import {
  createContext,
  type ReactNode,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import type { OwnerEquityV2MembershipModel } from "@/lib/products/equity-signals-contracts";
import type { IntradayStreamIdentity } from "@/lib/products/intraday-stream-contracts";
import { readyStreamIdentities } from "@/lib/products/intraday-stream-universe";
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
const StreamContext = createContext<PageStream>({ view: empty, identities: [] });

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
    () => ({ enabled: enabled && valid, owner: enabled && valid, sessionKey, identities }),
    [enabled, valid, sessionKey, identities],
  );
  const latest = useRef(current);
  const binding = useRef<ReturnType<typeof bindIntradayStreamBrowser> | null>(null);
  const [delivery, setDelivery] = useState<{
    sessionKey: string | null;
    view: IntradayStreamControllerView;
  }>({ sessionKey: null, view: empty });
  // Apply a committed render before creating a new session's controller.
  useEffect(() => {
    latest.current = current;
    binding.current?.refresh();
  }, [current]);
  // A fresh instance for each effect setup also handles React's setup-cleanup-setup probe.
  useEffect(() => {
    const controller = new IntradayStreamController();
    const remove = controller.subscribe((view) => setDelivery({ sessionKey, view }));
    const owned = bindIntradayStreamBrowser(controller, () => latest.current);
    binding.current = owned;
    return () => {
      if (binding.current === owned) binding.current = null;
      remove();
      owned.dispose();
    };
  }, [sessionKey]);
  const view =
    enabled && valid && sessionKey !== null && delivery.sessionKey === sessionKey
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
