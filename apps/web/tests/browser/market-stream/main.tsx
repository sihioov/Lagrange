import { StrictMode, useState } from "react";
import { createRoot } from "react-dom/client";
import type { StockBetaDashboardViewModel } from "@/components/stock-beta/dashboard/types";
import { CurrentQuoteWidget } from "@/components/stock-beta/quote/current-quote-widget";
import { IntradayStreamBoard } from "@/components/stock-beta/quote/intraday-stream-board";
import { IntradayStreamProvider } from "@/components/stock-beta/quote/intraday-stream-provider";
import { notifyBrowserLogout } from "@/lib/api/browser-lifecycle";
import { stockBetaDictionary } from "@/lib/i18n/dictionaries/stock-beta";
import { streamMembership } from "../../fixtures/market-stream";

const memberships = Array.from({ length: 30 }, (_, index) => streamMembership(index));
const mode = new URLSearchParams(window.location.search).get("mode") ?? "market_ws";
const noop = () => undefined;
function Fixture() {
  const [selected, setSelected] = useState(memberships[0]?.instrument_id ?? null);
  const [mounted, setMounted] = useState(true);
  const [remaining, setRemaining] = useState(memberships);
  const [session, setSession] = useState("synthetic-session-one");
  const viewModel: StockBetaDashboardViewModel = {
    actionError: null,
    actionMessage: null,
    busy: false,
    copy: stockBetaDictionary.en,
    disableId: null,
    inputError: null,
    instrumentCode: "",
    locale: "en",
    memberships: remaining,
    mutationPending: false,
    onAdd: async () => undefined,
    onCancelDisable: noop,
    onConfirmDisable: async () => undefined,
    onInstrumentCodeChange: noop,
    onRequestDisable: noop,
    onRetry: async () => undefined,
    pendingMembershipId: null,
    policy: {
      active_instruments: 30,
      max_active_instruments: 100,
      minimum_observed_sessions: 121,
      remaining_capacity: 70,
      target_observed_sessions: 261,
    },
    pollError: false,
    signalState: { kind: "not-ready" },
    signals: null,
    selectedInstrumentId: null,
    streamSelectedInstrumentId: selected,
    intradayEnabled: mode !== "off",
    marketStreamEnabled: mode === "market_ws",
  };
  return (
    <>
      <p>Synthetic API fixture — not a broker connection or authentication proof.</p>
      <nav aria-label="Fixture controls">
        <button type="button" onClick={() => setMounted((value) => !value)}>
          Toggle page
        </button>
        <button
          type="button"
          onClick={() =>
            setRemaining((items) => items.filter((item) => item.instrument_id !== selected))
          }
        >
          Remove selected membership
        </button>
        <button type="button" onClick={() => notifyBrowserLogout()}>
          Logout
        </button>
        <button type="button" onClick={() => setSession("synthetic-session-two")}>
          New session
        </button>
      </nav>
      {mounted ? (
        <IntradayStreamProvider
          enabled={mode === "market_ws"}
          sessionKey={session}
          memberships={remaining}
        >
          {mode === "market_ws" ? (
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
