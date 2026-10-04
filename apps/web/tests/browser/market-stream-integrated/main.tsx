import { StrictMode, useCallback, useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import type { StockBetaDashboardViewModel } from "@/components/stock-beta/dashboard/types";
import { CurrentQuoteWidget } from "@/components/stock-beta/quote/current-quote-widget";
import { IntradayStreamBoard } from "@/components/stock-beta/quote/intraday-stream-board";
import { IntradayStreamProvider } from "@/components/stock-beta/quote/intraday-stream-provider";
import { logout } from "@/lib/api/browser-client";
import { stockBetaDictionary } from "@/lib/i18n/dictionaries/stock-beta";
import { getOwnerEquityV2Memberships } from "@/lib/products/equity-signals-client";
import type { OwnerEquityV2MembershipListModel } from "@/lib/products/equity-signals-contracts";

// This entry uses the real authenticated API for memberships, CSRF, leases,
// SSE and logout. It supplies neither prices nor a second lease-renewal loop.
// A private fixture seeds the isolated DB session before Chromium opens it.
const requestedMode = new URLSearchParams(window.location.search).get("mode") ?? "market_ws";
if (!["market_ws", "rest", "off"].includes(requestedMode)) throw new Error("Invalid fixture mode");
const localSessionKey = crypto.randomUUID();
const noop = () => undefined;

function Fixture() {
  const [list, setList] = useState<OwnerEquityV2MembershipListModel | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [mounted, setMounted] = useState(true);
  const [session, setSession] = useState<string | null>(localSessionKey);
  const request = useRef<AbortController | null>(null);
  const [failed, setFailed] = useState(false);

  const refreshMemberships = useCallback(() => {
    request.current?.abort();
    const abort = new AbortController();
    request.current = abort;
    void getOwnerEquityV2Memberships({ signal: abort.signal })
      .then((value) => {
        if (abort.signal.aborted) return;
        setList(value);
        setSelected((prior) => prior ?? value.memberships[0]?.instrument_id ?? null);
      })
      .catch(() => {
        if (!abort.signal.aborted) setFailed(true);
      });
  }, []);

  useEffect(() => {
    refreshMemberships();
    return () => request.current?.abort();
  }, [refreshMemberships]);

  const endSession = async () => {
    // The production client broadcasts stop before its authenticated mutation.
    try {
      const response = await logout();
      if (!response.ok) throw new Error("Fixture logout failed");
      setSession(null);
    } catch {
      setFailed(true);
    }
  };

  if (failed)
    return <p data-testid="integrated-fixture-error">Authenticated fixture request failed</p>;
  if (list === null) return <p data-testid="integrated-fixture-loading">Loading memberships</p>;

  const viewModel: StockBetaDashboardViewModel = {
    actionError: null,
    actionMessage: null,
    busy: false,
    copy: stockBetaDictionary.en,
    disableId: null,
    inputError: null,
    instrumentCode: "",
    locale: "en",
    memberships: list.memberships,
    mutationPending: false,
    onAdd: async () => undefined,
    onCancelDisable: noop,
    onConfirmDisable: async () => undefined,
    onInstrumentCodeChange: noop,
    onRequestDisable: noop,
    onRetry: async () => undefined,
    pendingMembershipId: null,
    policy: list.policy,
    pollError: false,
    signalState: { kind: "not-ready" },
    signals: null,
    selectedInstrumentId: null,
    streamSelectedInstrumentId: selected,
    intradayEnabled: requestedMode !== "off",
    marketStreamEnabled: requestedMode === "market_ws",
  };

  return (
    <>
      <p>Isolated runtime integration with synthetic market data and real API authentication.</p>
      <nav aria-label="Fixture controls">
        <button type="button" onClick={() => setMounted((value) => !value)}>
          Toggle page
        </button>
        <button type="button" onClick={refreshMemberships}>
          Refresh memberships
        </button>
        <button type="button" onClick={() => void endSession()}>
          Logout
        </button>
      </nav>
      {mounted ? (
        <IntradayStreamProvider
          enabled={requestedMode === "market_ws"}
          sessionKey={session}
          memberships={list.memberships}
        >
          {requestedMode === "market_ws" ? (
            <IntradayStreamBoard
              locale="en"
              selectedInstrumentId={selected}
              onSelect={setSelected}
            />
          ) : null}
          <CurrentQuoteWidget viewModel={viewModel} />
        </IntradayStreamProvider>
      ) : (
        <p>Page unmounted</p>
      )}
    </>
  );
}

const root = document.getElementById("root");
if (!root) throw new Error("Missing fixture root");
createRoot(root).render(
  <StrictMode>
    <Fixture />
  </StrictMode>,
);
