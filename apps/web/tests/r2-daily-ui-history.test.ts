import { describe, expect, it, vi } from "vitest";
import { ApiProblem } from "@/lib/api/response";
import {
  type OwnerBetaRunPageModel,
  ownerBetaRunListItemSchema,
} from "@/lib/products/owner-beta-contracts";
import {
  ownerBetaHistoryQueryString,
  retainOwnerBetaHistory,
} from "@/lib/products/owner-beta-history";

const CONFIG = "00000000-0000-4000-8000-000000000101";
const OTHER = "00000000-0000-4000-8000-000000000102";
const SHA = `sha256:${"a".repeat(64)}`;

function row(index: number, status = "PENDING", config = CONFIG) {
  return ownerBetaRunListItemSchema.parse({
    action_manifest_sha256: SHA,
    approval_registry_sha256: SHA,
    artifact_manifest_sha256: SHA,
    candidate_content_sha256: SHA,
    stage5_manifest_sha256: SHA,
    as_of: "2026-10-15",
    audience: "OWNER_ONLY",
    capability: "PRICE_RETURN_ONLY",
    created_at: "2026-10-15T07:00:00Z",
    id: `00000000-0000-4000-8000-${String(index).padStart(12, "0")}`,
    input_kind: "owner_beta_historical_price_only_v1",
    job_id: "00000000-0000-4000-8000-000000000701",
    status,
    strategy_config_id: config,
    strategy_config_sha256: SHA,
    strategy_id: "buy_and_hold",
    strategy_version: "1.0.0",
    updated_at: "2026-10-15T07:00:00Z",
    vendor_snapshot: true,
    strict_pit: false,
  });
}

function page(
  items: OwnerBetaRunPageModel["items"],
  cursor: string | null = null,
): OwnerBetaRunPageModel {
  return { has_more: cursor !== null, items, next_cursor: cursor };
}

describe("bounded same-configuration recommendation history", () => {
  it("retains a later same-config success without selecting another config", async () => {
    const read = vi.fn().mockResolvedValue(page([row(3, "SUCCEEDED")]));
    const result = await retainOwnerBetaHistory(
      read,
      page([row(1), row(2, "SUCCEEDED", OTHER)], "next"),
      CONFIG,
    );
    expect(read).toHaveBeenCalledTimes(1);
    expect(read).toHaveBeenCalledWith({ cursor: "next", limit: 100 });
    expect(result.runs.map((run) => run.id)).toEqual([row(1).id, row(3).id]);
    expect(result.runs[1]?.status).toBe("SUCCEEDED");
    expect(result.previousResultSearchIncomplete).toBe(false);
    console.info("R2_DAILY_UI_TEST:history_retains_same_config_success:PASS");
  });

  it("stops after six pages and keeps an explicit unfinished-search state", async () => {
    let index = 1;
    const read = vi.fn().mockImplementation(async () => {
      index += 1;
      return page([row(index)], `cursor-${index}`);
    });
    const result = await retainOwnerBetaHistory(read, page([row(1)], "cursor-1"), CONFIG);
    expect(read).toHaveBeenCalledTimes(5);
    expect(result.runs).toHaveLength(6);
    expect(result.previousResultSearchIncomplete).toBe(true);
    expect(result.hasMore).toBe(true);
    expect(result.nextCursor).toBe("cursor-6");
    console.info("R2_DAILY_UI_TEST:history_bound_is_not_absence:PASS");
  });

  it("rejects inconsistent pages duplicate results and cursor loops", async () => {
    await expect(
      retainOwnerBetaHistory(vi.fn(), { has_more: true, items: [], next_cursor: "x" }, CONFIG),
    ).rejects.toThrow("Inconsistent history");
    await expect(
      retainOwnerBetaHistory(
        vi.fn(),
        { has_more: false, items: [row(1)], next_cursor: "x" },
        CONFIG,
      ),
    ).rejects.toThrow("Inconsistent history");
    await expect(
      retainOwnerBetaHistory(
        vi.fn().mockResolvedValue(page([row(1, "SUCCEEDED")])),
        page([row(1)], "next"),
        CONFIG,
      ),
    ).rejects.toThrow("Repeated run");
    await expect(
      retainOwnerBetaHistory(
        vi.fn().mockResolvedValue(page([row(2)], "next")),
        page([row(1)], "next"),
        CONFIG,
      ),
    ).rejects.toThrow("Repeated history cursor");
    console.info("R2_DAILY_UI_TEST:history_rejects_loop_and_duplicates:PASS");
  });

  it("propagates authorization failure without a partial result", async () => {
    const denied = new ApiProblem(403, {
      error: { code: "FORBIDDEN", message: "forbidden", request_id: "test" },
    });
    await expect(
      retainOwnerBetaHistory(
        vi.fn().mockRejectedValue(denied),
        page([row(1)], "next"),
        CONFIG,
      ),
    ).rejects.toBe(denied);
    console.info("R2_DAILY_UI_TEST:history_access_failure_propagates:PASS");
  });

  it("keeps opaque cursor bytes encoded and rejects unbounded or invalid query inputs", () => {
    expect(ownerBetaHistoryQueryString()).toBe("");
    const params = new URLSearchParams(
      ownerBetaHistoryQueryString({ cursor: "a&b=1", limit: 100 }),
    );
    expect(params.get("cursor")).toBe("a&b=1");
    expect(params.get("b")).toBeNull();
    expect(params.get("limit")).toBe("100");
    for (const query of [
      { cursor: "" },
      { cursor: "x".repeat(4097) },
      { limit: 0 },
      { limit: 101 },
      { limit: 1.5 },
      { limit: Number.NaN },
    ]) {
      expect(() => ownerBetaHistoryQueryString(query)).toThrow();
    }
    console.info("R2_DAILY_UI_TEST:history_query_is_bounded:PASS");
  });
});
