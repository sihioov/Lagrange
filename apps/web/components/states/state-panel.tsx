import type { ReactNode } from "react";

export type StateKind = "blocked" | "empty" | "error" | "loading";

const STATE_ROLES = {
  blocked: "alert",
  empty: "status",
  error: "alert",
  loading: "status",
} as const satisfies Record<StateKind, "alert" | "status">;

const STATE_LIVE_REGIONS = {
  blocked: "assertive",
  empty: "polite",
  error: "assertive",
  loading: "polite",
} as const satisfies Record<StateKind, "assertive" | "polite">;

export type StatePanelProps = {
  readonly action?: ReactNode;
  readonly kind: StateKind;
  readonly message: string;
  readonly title: string;
};

export function StatePanel({ action, kind, message, title }: StatePanelProps) {
  return (
    <section
      aria-busy={kind === "loading" ? true : undefined}
      aria-labelledby="state-panel-title"
      aria-live={STATE_LIVE_REGIONS[kind]}
      className="state-panel"
      data-kind={kind}
      role={STATE_ROLES[kind]}
    >
      <span aria-hidden="true" className="state-panel-marker" />
      <h2 id="state-panel-title">{title}</h2>
      <p>{message}</p>
      {action === undefined ? null : <div className="state-panel-action">{action}</div>}
    </section>
  );
}
