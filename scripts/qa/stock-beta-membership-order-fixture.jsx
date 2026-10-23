// Bundled only by stock-beta-membership-order-browser.mjs into a new /tmp directory.
import { StrictMode, useState } from "react";
import { createRoot } from "react-dom/client";
import { StockBetaWorkspace } from "@/components/stock-beta/stock-beta-workspace";
import { LocaleProvider } from "@/lib/i18n/client";
import {
  ownerEquityV2MembershipListSchema,
  ownerEquityV2MutationSchema,
} from "@/lib/products/equity-signals-contracts";
import fixtures from "./fixtures.json";

for (const value of Object.values(fixtures)) ownerEquityV2MembershipListSchema.parse(value);
ownerEquityV2MutationSchema.parse({
  resource: fixtures.disabled.memberships[0],
  job_id: "00000000-0000-4000-8000-000000000101",
  duplicate_active: false,
});
const originalFetch = window.fetch;
window.completedMembershipResponses = 0;
window.fetch = async (input, init) => {
  // Force delivery even if transport cancellation comes too late: state must still reject it.
  const isMembership =
    input === "/api/v1/research/owner-beta/equity-universe-v2/memberships" &&
    (!init?.method || init.method === "GET");
  const forceLateResponse = new URLSearchParams(location.search).get("abort") !== "real";
  const response = await originalFetch(
    input,
    isMembership && forceLateResponse ? { ...init, signal: undefined } : init,
  );
  if (isMembership) {
    const json = response.json.bind(response);
    response.json = async () => {
      const body = await json();
      window.completedMembershipResponses++;
      return body;
    };
  }
  return response;
};
function Fixture() {
  const [seed, setSeed] = useState(fixtures.backfill);
  const [mounted, setMounted] = useState(true);
  window.seedMemberships = (name) => setSeed(fixtures[name]);
  window.unmountWorkspace = () => setMounted(false);
  return (
    <LocaleProvider initialLocale="en">
      {mounted && <StockBetaWorkspace initialMemberships={seed} initialSignals={null} />}
    </LocaleProvider>
  );
}
createRoot(document.getElementById("root")).render(
  <StrictMode>
    <Fixture />
  </StrictMode>,
);
