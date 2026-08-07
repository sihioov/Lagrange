export type StatusTone = "error" | "info" | "neutral" | "success" | "warning";

export type StatusPillProps = {
  readonly label: string;
  readonly tone?: StatusTone;
};

export function StatusPill({ label, tone = "neutral" }: StatusPillProps) {
  return (
    <span className="status-pill" data-tone={tone}>
      <span aria-hidden="true" />
      {label}
    </span>
  );
}
