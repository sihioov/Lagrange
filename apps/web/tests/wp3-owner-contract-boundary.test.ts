import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { OwnerBetaReport } from "@/components/recommendations/owner-beta-report";
import { recommendationsDictionary } from "@/lib/i18n/dictionaries/recommendations";
import { ownerBetaRunSchema } from "@/lib/products/owner-beta-contracts";

type ApprovalRecord = {
  readonly action_manifest_sha256: string;
  readonly approval_registry_sha256: string;
  readonly artifact_manifest_sha256: string;
  readonly candidate_content_sha256: string;
  readonly instruments: readonly string[];
  readonly stage5_manifest_sha256: string;
};

function approvedRecord(): ApprovalRecord {
  const path = fileURLToPath(
    new URL(
      "../../../configs/evidence/kis-historical-price-only-beta-approved-artifacts.json",
      import.meta.url,
    ),
  );
  const bytes = readFileSync(path);
  const registry = JSON.parse(bytes.toString("utf8")) as {
    readonly approved_artifacts: readonly ApprovalRecord[];
  };
  const [record] = registry.approved_artifacts;
  if (record === undefined) {
    throw new Error("checked-in approval registry has no sole record");
  }
  return {
    ...record,
    approval_registry_sha256: `sha256:${createHash("sha256").update(bytes).digest("hex")}`,
  };
}

function registryBoundRun(record: ApprovalRecord) {
  const sha = `sha256:${"b".repeat(64)}`;
  return {
    action_manifest_sha256: record.action_manifest_sha256,
    approval_registry_sha256: record.approval_registry_sha256,
    artifact_manifest_sha256: record.artifact_manifest_sha256,
    as_of: "2026-08-19",
    audience: "OWNER_ONLY",
    candidate_content_sha256: record.candidate_content_sha256,
    capability: "PRICE_RETURN_ONLY",
    cash_weight: "0.800000",
    created_at: "2026-08-19T06:30:00Z",
    factor_snapshot_sha256: sha,
    finished_at: "2026-08-19T06:30:02Z",
    id: "00000000-0000-4000-8000-000000000601",
    input_kind: "owner_beta_historical_price_only_v1",
    items: record.instruments.map((instrument_id, index) => ({
      excluded: index !== 0,
      exclusion_reason: index === 0 ? undefined : "NOT_SELECTED_BY_STRATEGY",
      factors: { momentum_12_1: index === 0 ? "0.912300" : "0.000000" },
      instrument: {
        asset_class: null,
        exposure_group: null,
        id: instrument_id,
        name: null,
        tracking_index: null,
      },
      instrument_id,
      rank: index === 0 ? 1 : null,
      reason_codes: index === 0 ? ["SELECTED_TOP_N"] : ["NOT_SELECTED_BY_STRATEGY"],
      target_weight: index === 0 ? "0.200000" : null,
    })),
    job_id: "00000000-0000-4000-8000-000000000701",
    stage5_manifest_sha256: record.stage5_manifest_sha256,
    started_at: "2026-08-19T06:30:01Z",
    status: "SUCCEEDED",
    strategy_config_id: "00000000-0000-4000-8000-000000000101",
    strategy_config_sha256: sha,
    strategy_id: "relative_momentum",
    strategy_version: "1.0.0",
    strict_pit: false,
    target_snapshot_sha256: sha,
    updated_at: "2026-08-19T06:30:02Z",
    vendor_snapshot: true,
  };
}

describe("WP-3 owner-beta contract boundary", () => {
  it("strict-parses and renders a successful report bound to the checked-in approval registry ETF11", () => {
    const record = approvedRecord();
    expect(record.instruments).toEqual([
      "069500.KRX",
      "102110.KRX",
      "114260.KRX",
      "132030.KRX",
      "133690.KRX",
      "143850.KRX",
      "148070.KRX",
      "153130.KRX",
      "192090.KRX",
      "195930.KRX",
      "229200.KRX",
    ]);

    const run = registryBoundRun(record);
    const cashWeightUnits = Number.parseInt(run.cash_weight.slice(2), 10);
    const selectedWeightUnits = run.items.reduce(
      (total, item) =>
        total +
        (item.excluded || item.target_weight === null
          ? 0
          : Number.parseInt(item.target_weight.slice(2), 10)),
      0,
    );
    expect(run.items).toHaveLength(11);
    expect(cashWeightUnits).toBe(800_000);
    expect(selectedWeightUnits).toBe(200_000);
    expect(cashWeightUnits + selectedWeightUnits).toBe(1_000_000);
    expect([
      run.candidate_content_sha256,
      run.artifact_manifest_sha256,
      run.stage5_manifest_sha256,
      run.action_manifest_sha256,
      run.approval_registry_sha256,
    ]).toEqual([
      record.candidate_content_sha256,
      record.artifact_manifest_sha256,
      record.stage5_manifest_sha256,
      record.action_manifest_sha256,
      record.approval_registry_sha256,
    ]);
    expect(run.audience).toBe("OWNER_ONLY");
    expect(run.capability).toBe("PRICE_RETURN_ONLY");
    expect(run.strict_pit).toBe(false);
    expect(run.vendor_snapshot).toBe(true);
    expect(run.items[0]?.factors).toEqual({ momentum_12_1: "0.912300" });
    expect(run.items[0]?.reason_codes).toEqual(["SELECTED_TOP_N"]);

    const parsed = ownerBetaRunSchema.safeParse(run);
    expect(parsed.success).toBe(true);
    if (!parsed.success) {
      throw new Error("Web owner-beta schema rejected the registry ETF11");
    }
    expect(parsed.data.status).toBe("SUCCEEDED");
    expect(parsed.data.items.map((item) => item.instrument_id)).toEqual(record.instruments);

    const t = recommendationsDictionary.en;
    const markup = renderToStaticMarkup(createElement(OwnerBetaReport, { run: parsed.data, t }));

    for (const instrumentId of record.instruments) {
      expect(markup).toContain(instrumentId);
    }
    for (const [label, commitment] of [
      [t.ownerBetaCandidateContentHashLabel, record.candidate_content_sha256],
      [t.ownerBetaArtifactManifestHashLabel, record.artifact_manifest_sha256],
      [t.ownerBetaStage5ManifestHashLabel, record.stage5_manifest_sha256],
      [t.ownerBetaActionManifestHashLabel, record.action_manifest_sha256],
      [t.ownerBetaApprovalRegistryHashLabel, record.approval_registry_sha256],
    ] as const) {
      expect(markup).toContain(label);
      expect(markup).toContain(commitment);
    }
    expect(markup).toContain("SUCCEEDED");
    expect(markup).toContain(t.cashAllocation("80.00%"));
    expect(markup).toContain("20.00%");
    expect(markup).toContain("12-month momentum excluding the most recent month");
    expect(markup).toContain("+91.23%");
    expect(markup).toContain("+0.00%");
    expect(markup).toContain("Selected under the selection criteria. Rank is shown separately.");
    expect(markup).toContain("This fixed-universe instrument was not selected by the strategy.");
    expect(markup).not.toContain("SELECTED_TOP_N");
    expect(markup).not.toContain("NOT_SELECTED_BY_STRATEGY");
    expect(markup).toContain(t.ownerBetaAudienceValue);
    expect(markup).toContain(t.ownerBetaCapabilityValue);
    expect(markup).toContain(t.ownerBetaVendorSnapshotValue);
    expect(markup).toContain(t.ownerBetaStrictPitValue);
    expect(markup).toContain(t.ownerBetaAuditDetails);
    expect(markup).toContain(t.ownerBetaInstrumentNameMissing);
    expect(markup).toContain(t.ownerBetaAssetClassMissing);
    expect(markup).toContain(t.ownerBetaTrackingIndexMissing);
  });
});
