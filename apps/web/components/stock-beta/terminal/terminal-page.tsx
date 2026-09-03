import type { ReactNode } from "react";
import styles from "./terminal-page.module.css";
import { StockBetaTerminalUtilitySlot } from "./terminal-utility-slot";

export type StockBetaTerminalPageSlots = {
  /** Search for instruments already present in the page DTO. */
  readonly search?: ReactNode;
  /** Compact as-of value supplied by the page DTO. */
  readonly asOf?: ReactNode;
  /** The page DTO's as-of/snapshot state. */
  readonly snapshot?: ReactNode;
  /** Working controls such as filters or column visibility. */
  readonly titleTools?: ReactNode;
};

export type StockBetaTerminalPageProps = StockBetaTerminalPageSlots & {
  readonly children: ReactNode;
  readonly context?: ReactNode;
  readonly title: string;
};

/**
 * Page-owned composition boundary for DTO-dependent terminal chrome. Slots stay close to the
 * server page that owns the DTO; this component performs no data access and adds no controls.
 */
export function StockBetaTerminalPage({
  asOf,
  children,
  context,
  search,
  snapshot,
  title,
  titleTools,
}: StockBetaTerminalPageProps) {
  return (
    <div className={styles["page"]} data-testid="stock-beta-terminal-page">
      {search === undefined && asOf === undefined ? null : (
        <StockBetaTerminalUtilitySlot>
          <div
            className={styles["utilityContent"]}
            data-has-search={search === undefined ? "false" : "true"}
            data-terminal-utility-content="stock-beta"
          >
            {search === undefined ? null : (
              <div className={styles["utilitySearch"]} data-terminal-slot="search">
                {search}
              </div>
            )}
            {asOf === undefined ? null : (
              <div className={styles["asOf"]} data-terminal-slot="as-of">
                {asOf}
              </div>
            )}
          </div>
        </StockBetaTerminalUtilitySlot>
      )}
      {snapshot === undefined ? null : (
        <section aria-label={title} className={styles["snapshot"]} data-terminal-slot="snapshot">
          {snapshot}
        </section>
      )}
      <header className={styles["titleBar"]}>
        <div>
          {context === undefined ? null : <div className={styles["context"]}>{context}</div>}
          <h1>{title}</h1>
        </div>
        {titleTools === undefined ? null : (
          <div className={styles["titleTools"]} data-terminal-slot="title-tools">
            {titleTools}
          </div>
        )}
      </header>
      <div className={styles["content"]}>{children}</div>
    </div>
  );
}
