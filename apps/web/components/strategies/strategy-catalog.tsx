import { StatusPill, type StatusTone } from "@/components/states/status-pill";
import type { StrategyCatalogItem } from "@/lib/products/contracts";
import { StrategyConfigForm } from "./strategy-config-form";

const STATE_TONES = {
  Draft: "neutral",
  LiveCandidate: "info",
  Paper: "info",
  Retired: "error",
  Validated: "success",
} as const satisfies Record<StrategyCatalogItem["state"], StatusTone>;

const CONFIGURATION_BLOCKED_COPY = {
  Draft: "Configuration is unavailable while the required data entitlement is inactive.",
  LiveCandidate: "Configuration is unavailable while the required data entitlement is inactive.",
  Paper: "Configuration is unavailable while the required data entitlement is inactive.",
  Retired: "This strategy version is retired and cannot be configured.",
  Validated: "Configuration is unavailable while the required data entitlement is inactive.",
} as const satisfies Record<StrategyCatalogItem["state"], string>;

export type StrategyCatalogProps = {
  readonly canConfigure: boolean;
  readonly strategies: readonly StrategyCatalogItem[];
};

export function StrategyCatalog({ canConfigure, strategies }: StrategyCatalogProps) {
  return (
    <section aria-labelledby="strategy-catalog-title" className="product-section">
      <div className="section-heading">
        <div>
          <p className="eyebrow">Approved catalog</p>
          <h2 id="strategy-catalog-title">Strategy versions</h2>
        </div>
        <p>Only schema-bound parameters can be changed. Strategy code remains server-managed.</p>
      </div>
      <div className="strategy-grid">
        {strategies.map((strategy) => (
          <article className="strategy-panel" key={strategy.id}>
            <header className="panel-heading">
              <div>
                <h3>{strategy.display_name}</h3>
                <p>Version {strategy.latest_version ?? "Not reported"}</p>
              </div>
              <StatusPill label={strategy.state} tone={STATE_TONES[strategy.state]} />
            </header>
            <p>{strategy.description ?? "No strategy description was reported."}</p>
            <div className="risk-note">
              <strong>Risk warning</strong>
              <p>{strategy.risk_description ?? "No additional risk description was reported."}</p>
            </div>
            {canConfigure && strategy.state !== "Retired" ? (
              <StrategyConfigForm strategy={strategy} />
            ) : (
              <p className="blocked-inline" role="alert">
                {CONFIGURATION_BLOCKED_COPY[strategy.state]}
              </p>
            )}
          </article>
        ))}
      </div>
    </section>
  );
}
