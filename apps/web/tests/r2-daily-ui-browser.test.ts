import { describe, expect, it, vi } from "vitest";
import { fetchOwnerBetaDailyState, saveOwnerBetaDailyPreference } from "@/lib/api/owner-beta-daily";

const ID = "00000000-0000-4000-8000-000000000101";
const OTHER = "00000000-0000-4000-8000-000000000102";
const ORIGIN = "https://lagrange.test";
const TIME = "2026-10-16T07:00:00Z";
const config = (id = ID, enabled = true) => ({
  id,
  strategy_id: "buy_and_hold",
  strategy_version: "1.0.0",
  config: { lookback: 200 },
  is_active: true,
  owner_beta_daily_enabled: enabled,
  owner_beta_daily_epoch: 1,
  created_at: TIME,
  updated_at: TIME,
});
const state = {
  enabled: false,
  runtime_enabled: false,
  state: "DISABLED",
  reason_code: "CONFIG_DISABLED",
  expected_session: null,
  data_support_end: null,
  recommendation_as_of: null,
  published_at: null,
  input_kind: null,
  trigger_kind: null,
  current_reservation_run_id: null,
  data_due_at: null,
  observed_at: TIME,
};

describe("daily preference and observation transport", () => {
  it("sends only explicit opt-in through CSRF and idempotent PATCH", async () => {
    const seen: Request[] = [];
    const fetcher: typeof fetch = async (input, init) => {
      const request = new Request(input, init);
      seen.push(request.clone());
      return request.method === "GET"
        ? Response.json({ csrf_token: "daily-test-csrf" })
        : Response.json(config());
    };
    const result = await saveOwnerBetaDailyPreference(ID, true, { fetcher, origin: ORIGIN });
    expect(result.owner_beta_daily_enabled).toBe(true);
    expect(seen).toHaveLength(2);
    const csrf = seen[0];
    const patch = seen[1];
    expect(csrf?.url).toBe(ORIGIN + "/api/v1/auth/csrf");
    expect(patch?.url).toBe(ORIGIN + "/api/v1/strategy-configs/" + ID);
    expect(patch?.method).toBe("PATCH");
    expect(patch?.headers.get("X-CSRF-Token")).toBe("daily-test-csrf");
    expect(patch?.headers.get("Idempotency-Key")).toMatch(/^[0-9a-f-]{36}$/);
    expect(await patch?.json()).toEqual({ owner_beta_daily_enabled: true });
    console.info("R2_DAILY_UI_TEST:optin_patch_only:PASS");
  });

  it("rejects wrong configuration and unconfirmed setting responses", async () => {
    for (const response of [config(OTHER), config(ID, false)]) {
      const fetcher: typeof fetch = async (input, init) =>
        new Request(input, init).method === "GET"
          ? Response.json({ csrf_token: "daily-test-csrf" })
          : Response.json(response);
      await expect(
        saveOwnerBetaDailyPreference(ID, true, { fetcher, origin: ORIGIN }),
      ).rejects.toThrow();
    }
    console.info("R2_DAILY_UI_TEST:optin_confirmation_identity:PASS");
  });

  it("preserves rejection and recovers expired sessions at the browser boundary", async () => {
    for (const [code, status] of [["CSRF_DENIED", 403], ["SESSION_EXPIRED", 401]] as const) {
      const navigate = vi.fn();
      const fetcher: typeof fetch = async (input, init) =>
        new Request(input, init).method === "GET"
          ? Response.json({ csrf_token: "daily-test-csrf" })
          : Response.json(
              { error: { code, message: "static error", request_id: "daily-test" } },
              { status },
            );
      await expect(
        saveOwnerBetaDailyPreference(ID, true, { fetcher, navigate, origin: ORIGIN }),
      ).rejects.toThrow();
      if (status === 401) {
        expect(navigate).toHaveBeenCalledWith("/login");
      } else {
        expect(navigate).not.toHaveBeenCalled();
      }
    }
    console.info("R2_DAILY_UI_TEST:optin_auth_failure:PASS");
  });

  it("reads only the selected daily observation and propagates cancellation", async () => {
    const abort = new AbortController();
    const requests: Request[] = [];
    const fetcher: typeof fetch = async (input, init) => {
      requests.push(new Request(input, init));
      return Response.json(state);
    };
    expect(
      await fetchOwnerBetaDailyState(ID, { fetcher, origin: ORIGIN, signal: abort.signal }),
    ).toEqual(state);
    expect(requests).toHaveLength(1);
    const request = requests[0];
    expect(request?.method).toBe("GET");
    expect(request?.url).toBe(
      ORIGIN + "/api/v1/recommendations/owner-beta/price-only/daily-state?strategy_config_id=" + ID,
    );
    expect(request?.cache).toBe("no-store");
    expect(request?.credentials).toBe("same-origin");
    abort.abort();
    expect(request?.signal.aborted).toBe(true);
    console.info("R2_DAILY_UI_TEST:observation_get_only:PASS");
  });

  it("rejects malformed or nil configuration identity before any request", async () => {
    const fetcher = vi.fn<typeof fetch>();
    for (const id of ["../other", "00000000-0000-0000-0000-000000000000"]) {
      await expect(
        saveOwnerBetaDailyPreference(id, true, { fetcher, origin: ORIGIN }),
      ).rejects.toThrow();
      await expect(fetchOwnerBetaDailyState(id, { fetcher, origin: ORIGIN })).rejects.toThrow();
    }
    expect(fetcher).not.toHaveBeenCalled();
    console.info("R2_DAILY_UI_TEST:identity_preflight:PASS");
  });
});
