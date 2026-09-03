import { type ReactNode, useId } from "react";
import styles from "./widget-frame.module.css";

export type StockBetaWidgetFrameState =
  | { readonly kind: "ready" }
  | { readonly kind: "loading"; readonly message: string }
  | { readonly kind: "empty"; readonly message: string; readonly action?: ReactNode }
  | { readonly kind: "error" | "blocked"; readonly message: string; readonly action?: ReactNode };

export type WidgetFrameProps = {
  readonly children: ReactNode;
  readonly description?: string;
  readonly headingLevel?: 2 | 3;
  readonly state?: StockBetaWidgetFrameState;
  readonly status?: ReactNode;
  readonly title: string;
};

const READY_STATE = { kind: "ready" } as const satisfies StockBetaWidgetFrameState;

export function WidgetFrame({
  children,
  description,
  headingLevel = 2,
  state = READY_STATE,
  status,
  title,
}: WidgetFrameProps) {
  const headingId = useId();
  const Heading = headingLevel === 2 ? "h2" : "h3";
  const isReady = state.kind === "ready";
  const isLoading = state.kind === "loading";
  const isAlert = state.kind === "error" || state.kind === "blocked";

  return (
    <section
      aria-busy={isLoading ? true : undefined}
      aria-labelledby={headingId}
      className={styles["frame"]}
      data-state={state.kind}
    >
      <header className={styles["header"]}>
        <div className={styles["headingGroup"]}>
          <Heading className={styles["heading"]} id={headingId}>
            {title}
          </Heading>
          {description === undefined ? null : (
            <p className={styles["description"]}>{description}</p>
          )}
        </div>
        {status === undefined ? null : <div className={styles["status"]}>{status}</div>}
      </header>
      {isReady ? (
        <div className={styles["content"]}>{children}</div>
      ) : (
        <div
          aria-live={isAlert ? "assertive" : "polite"}
          className={styles["state"]}
          role={isAlert ? "alert" : "status"}
        >
          <p>{state.message}</p>
          {"action" in state && state.action !== undefined ? (
            <div className={styles["stateAction"]}>{state.action}</div>
          ) : null}
        </div>
      )}
    </section>
  );
}
