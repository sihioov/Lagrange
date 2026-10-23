import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { OwnerBetaDailyPanel } from "@/components/recommendations/owner-beta-daily-panel";
import { StrategyConfigForm } from "@/components/strategies/strategy-config-form";
import { StrategyDailyControl } from "@/components/strategies/strategy-daily-control";
import { LocaleProvider } from "@/lib/i18n/client";
import { strategySchema } from "@/lib/products/contracts";
import type { OwnerBetaDailyStateModel } from "@/lib/products/owner-beta-daily-contracts";

vi.mock("next/navigation", () => ({
  useRouter: () => ({ refresh: () => undefined }),
}));

const CONFIG = "00000000-0000-4000-8000-000000000101";
const OBSERVATION: OwnerBetaDailyStateModel = {
  enabled: true,
  runtime_enabled: true,
  expected_session: "2026-10-15",
  data_support_end: "2026-10-15",
  recommendation_as_of: "2026-10-15",
  published_at: "2026-10-15T08:00:00Z",
  input_kind: "owner_beta_daily_etf11_price_only_v1",
  trigger_kind: "DAILY",
  current_reservation_run_id: null,
  state: "CURRENT",
  reason_code: null,
  data_due_at: "2026-10-15T09:30:00Z",
  observed_at: "2026-10-15T08:01:00Z",
};

describe("daily UI initial evidence boundary", () => {
  it("keeps four dates distinct and does not claim current from an SSR observation alone", () => {
    const markup = renderToStaticMarkup(
      <LocaleProvider initialLocale="en">
        <OwnerBetaDailyPanel configId={CONFIG} initialState={OBSERVATION} />
      </LocaleProvider>,
    );
    for (const label of [
      "Expected trading session",
      "Daily input coverage ends",
      "Latest daily result as-of",
      "Published at (KST)",
    ]) {
      expect(markup).toContain(label);
    }
    expect(markup).toContain("5:00 PM KST");
    expect(markup).toContain("6:30 PM KST");
    expect(markup).toContain("Current status not verified");
    expect(markup).not.toContain("Expected session published");
    console.info("R2_DAILY_UI_TEST:ssr_dates_are_observations:PASS");
  });

  it("distinguishes an unknown legacy preference from an explicitly disabled preference", () => {
    const render = (enabled: boolean | undefined) =>
      renderToStaticMarkup(
        <LocaleProvider initialLocale="en">
          <StrategyDailyControl configId={CONFIG} enabled={enabled} />
        </LocaleProvider>,
      ).replaceAll("<!-- -->", "");
    const unknown = render(undefined);
    expect(unknown).toContain("Not reported");
    expect(unknown).toContain('disabled=""');
    expect(unknown).not.toContain("· Off");
    const disabled = render(false);
    expect(disabled).toContain("· Off");
    expect(disabled).toContain('aria-checked="false"');
    expect(disabled).not.toContain('disabled=""');
    console.info("R2_DAILY_UI_TEST:unknown_optin_is_not_confirmed_off:PASS");
  });

  it("shows an unchecked create preference only with Owner capability", () => {
    const strategy = strategySchema.parse({
      id: "buy_and_hold",
      display_name: "Baseline",
      state: "Validated",
      latest_version: "1.0.0",
      parameter_schema: {
        type: "object",
        properties: { target_weight: { type: "number", default: 1 } },
        required: ["target_weight"],
      },
    });
    const render = (canDaily: boolean) =>
      renderToStaticMarkup(
        <LocaleProvider initialLocale="en">
          <StrategyConfigForm canDaily={canDaily} strategy={strategy} />
        </LocaleProvider>,
      );
    expect(render(true)).toContain('name="owner_beta_daily_enabled"');
    expect(render(true)).not.toContain('checked=""');
    expect(render(false)).not.toContain('name="owner_beta_daily_enabled"');
    console.info("R2_DAILY_UI_TEST:create_optin_is_explicit_and_off:PASS");
  });
});
