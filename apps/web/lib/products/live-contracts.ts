import { z } from "zod";

/**
 * Owner-only Live surfaces (Todo 37).
 *
 * Every schema is `.strict()`, which matters more here than anywhere else: an
 * unexpected field arriving from the server on a Live payload is exactly the
 * shape a credential leak would take, and `.strict()` makes it a loud parse
 * failure rather than an extra key nobody notices.
 *
 * Note what these schemas CANNOT describe. There is no field for an app key,
 * an app secret, or a full account number — only reference locations and a
 * masked account. If the server ever started sending one, parsing would fail
 * instead of rendering it.
 */

/** `env:VAR` or `file:/path` — a location, never a value. */
const credentialReferenceSchema = z
  .string()
  .refine((v) => /^env:[A-Za-z_][A-Za-z0-9_]*$/.test(v) || /^file:\/.+/.test(v), {
    message: "must be a credential reference (env:VAR or file:/path), never the credential itself",
  });

export const liveConnectionSchema = z
  .object({
    account_no_masked: z.string().startsWith("****"),
    account_product_code: z.string(),
    id: z.uuid(),
    kis_app_key_ref: credentialReferenceSchema,
    kis_app_secret_ref: credentialReferenceSchema,
    label: z.string(),
    profile: z.enum(["mock", "live"]),
    status: z.string(),
  })
  .strict();

export const liveNodeSchema = z
  .object({
    connection_id: z.uuid(),
    id: z.uuid(),
    started_at: z.string(),
    status: z.enum(["STARTING", "RUNNING", "STOPPED"]),
    stopped_at: z.string().nullable(),
  })
  .strict();

export const killSwitchSchema = z.object({ engaged: z.boolean() }).strict();

export type LiveConnectionModel = z.infer<typeof liveConnectionSchema>;
export type LiveNodeModel = z.infer<typeof liveNodeSchema>;

/**
 * Whether a connection talks to the real broker.
 *
 * Used to decide how loudly the UI marks a row. A mock connection placing
 * simulated orders and a live one placing real ones must never look alike.
 */
export function isLiveProfile(connection: LiveConnectionModel): boolean {
  return connection.profile === "live";
}

/**
 * The reason a Live action is unavailable, in the operator's terms.
 *
 * The server distinguishes "you are not the Owner" (404, indistinguishable
 * from a route that does not exist) from "you are, but your MFA is not fresh"
 * (403 with a STEP_UP_* code). Only the second is worth explaining, because
 * only the second is something the reader can act on.
 */
export function liveUnavailableReason(code: string): string {
  switch (code) {
    case "STEP_UP_MFA_REQUIRED":
      return "Live controls require multi-factor authentication. Sign in again with your second factor.";
    case "STEP_UP_AUTH_TIME_STALE":
      return "Your authentication is too old for Live controls. Re-authenticate to continue.";
    case "STEP_UP_AUTH_TIME_ABSENT":
      return "This session has no authentication timestamp, so Live controls cannot verify its freshness.";
    case "STEP_UP_NOT_OWNER":
      return "Live controls are restricted to the Owner.";
    case "LIVE_KILL_SWITCH_ENGAGED":
      return "The Live kill switch is engaged. No node can start until an Owner disengages it.";
    default:
      return "Live controls are unavailable for this session.";
  }
}
