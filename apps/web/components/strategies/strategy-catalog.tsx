import { StatusPill, type StatusTone } from "@/components/states/status-pill";
import type { StrategiesDictionary } from "@/lib/i18n/dictionaries/strategies";
import type { StrategyCatalogItem } from "@/lib/products/contracts";
import { StrategyConfigForm } from "./strategy-config-form";

const STATE_TONES = {
  Draft: "neutral",
  LiveCandidate: "info",
  Paper: "info",
  Retired: "error",
  Validated: "success",
} as const satisfies Record<StrategyCatalogItem["state"], StatusTone>;

function configurationBlockedCopy(
  state: StrategyCatalogItem["state"],
  t: StrategiesDictionary,
): string {
  return state === "Retired" ? t.retiredMessage : t.configurationUnavailableMessage;
}

export type StrategyCatalogProps = {
  readonly canConfigure: boolean;
  readonly strategies: readonly StrategyCatalogItem[];
  readonly t: StrategiesDictionary;
};

export function StrategyCatalog({ canConfigure, strategies, t }: StrategyCatalogProps) {
  return (
    <section aria-labelledby="strategy-catalog-title" className="product-section">
      <div className="section-heading">
        <div>
          <p className="eyebrow">{t.catalogEyebrow}</p>
          <h2 id="strategy-catalog-title">{t.catalogHeading}</h2>
        </div>
        <p>{t.schemaBoundNote}</p>
      </div>
      <div className="strategy-grid">
        {strategies.map((strategy) => (
          <article className="strategy-panel" key={strategy.id}>
            <header className="panel-heading">
              <div>
                <h3>{strategy.display_name}</h3>
                <p>{t.versionLabel(strategy.latest_version ?? t.notReported)}</p>
              </div>
              <StatusPill label={strategy.state} tone={STATE_TONES[strategy.state]} />
            </header>
            <p>{strategy.description ?? t.noStrategyDescriptionReported}</p>
            <div className="risk-note">
              <strong>{t.riskWarningLabel}</strong>
              <p>{strategy.risk_description ?? t.noRiskDescriptionReported}</p>
            </div>
            {canConfigure && strategy.state !== "Retired" ? (
              <StrategyConfigForm strategy={strategy} />
            ) : (
              <p className="blocked-inline" role="alert">
                {configurationBlockedCopy(strategy.state, t)}
              </p>
            )}
          </article>
        ))}
      </div>
    </section>
  );
}
