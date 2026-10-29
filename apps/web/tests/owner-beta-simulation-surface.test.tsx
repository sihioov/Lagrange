import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { OwnerBetaSimulationForm } from "@/components/backtests/owner-beta-simulation-form";
import { OwnerBetaSimulationHistory } from "@/components/backtests/owner-beta-simulation-history";
import { ownerBetaSimulationRunSchema } from "@/lib/products/owner-beta-simulation-contracts";

const CONFIG_ID = "00000000-0000-4000-8000-000000000801";
const RUN_ID = "00000000-0000-4000-8000-000000000811";
const JOB_ID = "00000000-0000-4000-8000-000000000821";

const selectedRun = ownerBetaSimulationRunSchema.parse({
  audience: "OWNER_ONLY",
  capability: "PRICE_RETURN_ONLY",
  contract_document_sha256:
    "sha256:8c5c6bf7f07de09f14f1e5eaa2b6479be4b10e8b98ed5a89a280ac95568e87cd",
  contract_version: "owner-beta-price-only-simulation-v1-draft-1",
  created_at: "2026-08-29T06:00:00Z",
  end_date: "2026-08-28",
  error_code: null,
  evaluation_session_count: 2,
  finished_at: "2026-08-29T06:01:00Z",
  id: RUN_ID,
  job_id: JOB_ID,
  result_schema_version: "owner-beta-price-only-result-v1",
  started_at: "2026-08-29T06:00:30Z",
  start_date: "2026-08-27",
  status: "SUCCEEDED",
  strategy_config_id: CONFIG_ID,
  strategy_id: "trend_following",
  strategy_version: "1.0.0",
  strict_pit: false,
  updated_at: "2026-08-29T06:01:00Z",
  vendor_snapshot: true,
});

describe("owner-beta simulation surface", () => {
  it("blocks queueing without an existing eligible configuration", () => {
    const markup = renderToStaticMarkup(
      <OwnerBetaSimulationForm configs={[]} onAccessLoss={vi.fn()} onQueued={vi.fn()} />,
    );

    expect(markup).toContain("No eligible strategy configuration");
    expect(markup).not.toContain("<form");
    expect(markup).not.toContain("Queue simulation");
  });

  it("shows selected history metadata without exposing internal identifiers", () => {
    const markup = renderToStaticMarkup(
      <OwnerBetaSimulationHistory
        error={false}
        hasMore
        loading={false}
        loadingMore={false}
        locale="en"
        onLoadMore={vi.fn()}
        onSelect={vi.fn()}
        runs={[selectedRun]}
        selectedRunId={RUN_ID}
      />,
    );

    expect(markup).toContain("2026-08-27");
    expect(markup).toContain("2026-08-28");
    expect(markup).toContain("SUCCEEDED");
    expect(markup).toContain('aria-pressed="true"');
    expect(markup).not.toContain(RUN_ID);
    expect(markup).not.toContain(JOB_ID);
    expect(markup).not.toContain(CONFIG_ID);
  });
});
