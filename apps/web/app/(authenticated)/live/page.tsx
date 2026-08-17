import type { Metadata } from "next";
import { LiveConnections } from "@/components/live/live-connections";
import { LiveKillSwitch } from "@/components/live/live-kill-switch";
import { OwnerRoute } from "@/components/pages/owner-route";
import { StatePanel } from "@/components/states/state-panel";
import { ApiProblem } from "@/lib/api/response";
import { getProductApi } from "@/lib/api/server-products";
import { type LiveDictionary, liveDictionary } from "@/lib/i18n/dictionaries/live";
import { getLocale } from "@/lib/i18n/server";
import { liveUnavailableReason } from "@/lib/products/live-contracts";

export const dynamic = "force-dynamic";
export const fetchCache = "force-no-store";
export const revalidate = 0;

export const metadata: Metadata = {
  title: "Live controls",
};

/**
 * The Live controls page.
 *
 * Nothing on this page is a second line of defence. A Member never reaches it:
 * the navigation is derived from the session role so the link does not exist,
 * `OwnerRoute` refuses the render, and the API answers 404 rather than 403 so
 * even a typed URL cannot confirm the route is real. This page is the Owner's
 * surface, not a guard.
 *
 * The kill switch reads as ENGAGED whenever its state cannot be determined.
 * Live is the one place where an unknown state must display as "stopped": an
 * operator who glances at this page and sees "running" when the server could
 * not answer would draw exactly the wrong conclusion.
 */
export default async function LivePage() {
  const locale = await getLocale();
  const t = liveDictionary[locale];
  return (
    <OwnerRoute description={t.pageDescription} title={t.pageTitle}>
      <LiveWorkspace t={t} />
    </OwnerRoute>
  );
}

async function LiveWorkspace({ t }: { readonly t: LiveDictionary }) {
  try {
    const api = await getProductApi();
    const connections = await api.getLiveConnections();
    // The kill-switch state is not yet a read route; the safe default is
    // ENGAGED, which is also how migration 0016 initialises it.
    return (
      <>
        <LiveKillSwitch engaged />
        <LiveConnections connections={connections.items} />
      </>
    );
  } catch (error) {
    if (error instanceof ApiProblem) {
      // A step-up refusal is actionable and gets its own explanation; anything
      // else is reported without speculating about the cause.
      if (error.code.startsWith("STEP_UP_")) {
        return (
          <StatePanel
            kind="blocked"
            message={liveUnavailableReason(error.code)}
            title={t.freshAuthRequiredTitle}
          />
        );
      }
      if (error.code === "RESOURCE_NOT_FOUND") {
        return (
          <StatePanel kind="blocked" message={t.notAvailableMessage} title={t.unavailableTitle} />
        );
      }
    }
    return (
      <StatePanel kind="error" message={t.configLoadFailedMessage} title={t.unavailableTitle} />
    );
  }
}
