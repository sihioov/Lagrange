import { describe, expect, it } from "vitest";
import {
  isLiveProfile,
  liveConnectionSchema,
  liveUnavailableReason,
} from "@/lib/products/live-contracts";

/**
 * Todo 37: the browser's last line of defence on Live payloads.
 *
 * `storage-audit.test.ts` permits the identifiers `kis_app_key_ref` and
 * `kis_app_secret_ref` in web source on the grounds that a `_ref` carries a
 * location and never a value. That exemption is only sound while something
 * actually enforces it on this side of the wire. This file is that something:
 * if the server were compromised, misconfigured, or simply changed, and it
 * began sending a real app key in the `_ref` field, parsing must fail rather
 * than the UI rendering the key onto the Owner's screen.
 */

function connection(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    account_no_masked: "****6-01",
    account_product_code: "01",
    id: "00000000-0000-4000-8000-000000000901",
    kis_app_key_ref: "env:KIS_APP_KEY",
    kis_app_secret_ref: "file:/run/secrets/kis_app_secret",
    label: "KIS simulator",
    profile: "mock",
    status: "CONFIGURED",
    ...overrides,
  };
}

describe("live connection payload parsing", () => {
  it("accepts the two reference forms the server is constrained to emit", () => {
    // Given the shapes migration 0016's CHECK constraints allow.
    const envRef = connection({ kis_app_key_ref: "env:KIS_APP_KEY" });
    const fileRef = connection({ kis_app_key_ref: "file:/run/secrets/kis_app_key" });

    // When / Then
    expect(liveConnectionSchema.safeParse(envRef).success).toBe(true);
    expect(liveConnectionSchema.safeParse(fileRef).success).toBe(true);
  });

  it("refuses a credential VALUE where a reference belongs", () => {
    // Given payloads whose `_ref` field carries the secret itself. These are
    // the shapes a leak would actually take: a raw key, and a key wearing a
    // reference-ish prefix that is not one of the two permitted forms.
    const leaks = [
      "PSxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
      "env:",
      "envKIS_APP_KEY",
      "file:relative/not/absolute",
      "env:KIS APP KEY",
      "",
    ];

    // When
    const accepted = leaks.filter(
      (value) => liveConnectionSchema.safeParse(connection({ kis_app_key_ref: value })).success,
    );

    // Then
    expect(accepted).toEqual([]);
  });

  it("refuses an unexpected field, which is the shape a leak arrives in", () => {
    // Given a payload carrying a field the schema never declared. `.strict()`
    // means a server that starts sending `kis_app_secret` alongside the
    // reference is a parse failure, not an extra key that renders by accident.
    const smuggled = connection({ kis_app_secret: "PSyyyyyyyyyyyyyyyyyyyy" });

    // When
    const result = liveConnectionSchema.safeParse(smuggled);

    // Then
    expect(result.success).toBe(false);
  });

  it("refuses an unmasked account number", () => {
    // Given
    const unmasked = connection({ account_no_masked: "50123456-01" });

    // When / Then
    expect(liveConnectionSchema.safeParse(unmasked).success).toBe(false);
  });
});

describe("live profile labelling", () => {
  it("separates a connection that places real orders from one that simulates", () => {
    // Given
    const live = liveConnectionSchema.parse(connection({ profile: "live" }));
    const mock = liveConnectionSchema.parse(connection({ profile: "mock" }));

    // When / Then
    expect(isLiveProfile(live)).toBe(true);
    expect(isLiveProfile(mock)).toBe(false);
  });

  it("admits no third profile that would be neither and render as mock", () => {
    // Given
    const unknown = connection({ profile: "paper" });

    // When / Then
    expect(liveConnectionSchema.safeParse(unknown).success).toBe(false);
  });
});

describe("live unavailability messages", () => {
  it("tells an Owner with stale MFA what action to take", () => {
    // Given / When
    const stale = liveUnavailableReason("STEP_UP_AUTH_TIME_STALE");

    // Then
    expect(stale).toMatch(/re-authenticate/i);
  });

  it("never echoes an unrecognised server code back to the reader", () => {
    // Given a code this build does not know. Interpolating it would turn the
    // message into an oracle for server-side vocabulary.
    // When
    const message = liveUnavailableReason("SOME_UNSHIPPED_INTERNAL_CODE");

    // Then
    expect(message).not.toContain("SOME_UNSHIPPED_INTERNAL_CODE");
    expect(message).toBe("Live controls are unavailable for this session.");
  });
});
