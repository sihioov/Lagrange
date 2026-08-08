import type { Metadata } from "next";
import { LiveConnections } from "@/components/live/live-connections";
import { LiveKillSwitch } from "@/components/live/live-kill-switch";
import { OwnerRoute } from "@/components/pages/owner-route";
import { StatePanel } from "@/components/states/state-panel";
import { ApiProblem } from "@/lib/api/response";
import { getProductApi } from "@/lib/api/server-products";
import { liveUnavailableReason } from "@/lib/products/live-contracts";

export const dynamic = "force-dynamic";
export const fetchCache = "force-no-store";
export const revalidate = 0;

export const metadata: Metadata = {
  title: "Live controls",
};

const DESCRIPTION =
  "Owner-only broker connections, node lifecycle, and the Live kill switch. Every action requires a fresh multi-factor authentication.";

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
  return (
    <OwnerRoute description={DESCRIPTION} title="Live controls">
      <LiveWorkspace />
    </OwnerRoute>
  );
}

async function LiveWorkspace() {
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
            title="Fresh authentication required"
          />
        );
      }
      if (error.code === "RESOURCE_NOT_FOUND") {
        return (
          <StatePanel
            kind="blocked"
            message="Live controls are not available to this session."
            title="Live controls unavailable"
          />
        );
      }
    }
    return (
      <StatePanel
        kind="error"
        message="Live configuration could not be loaded. The kill switch remains engaged until this resolves."
        title="Live controls unavailable"
      />
    );
  }
}
