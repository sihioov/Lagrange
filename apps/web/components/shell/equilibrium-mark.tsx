export type EquilibriumMarkProps = {
  readonly className?: string;
  readonly detailed?: boolean;
  readonly size?: number;
};

/**
 * Three-body triangulation glyph: two masses and the stable point their
 * gravity holds between them. The station's namesake, drawn once and reused
 * as the brand mark and as the state-panel icon — the same figure that means
 * "in balance" also has to be legible as "out of balance" when tinted by
 * state color.
 */
export function EquilibriumMark({ className, detailed = false, size = 20 }: EquilibriumMarkProps) {
  return (
    <svg
      aria-hidden="true"
      className={className}
      fill="none"
      height={size}
      viewBox="0 0 32 32"
      width={size}
    >
      {detailed ? (
        <ellipse
          cx="16"
          cy="17"
          rx="12.5"
          ry="6.5"
          stroke="currentColor"
          strokeDasharray="1.4 3"
          strokeOpacity="0.45"
          strokeWidth="1"
          transform="rotate(-11 16 17)"
        />
      ) : null}
      <line
        stroke="currentColor"
        strokeOpacity="0.55"
        strokeWidth="1"
        x1="9"
        x2="23"
        y1="10"
        y2="12"
      />
      <line
        stroke="currentColor"
        strokeOpacity="0.55"
        strokeWidth="1"
        x1="9"
        x2="17.5"
        y1="10"
        y2="23.5"
      />
      <line
        stroke="currentColor"
        strokeOpacity="0.55"
        strokeWidth="1"
        x1="23"
        x2="17.5"
        y1="12"
        y2="23.5"
      />
      <circle cx="9" cy="10" fill="currentColor" r="3" />
      <circle cx="23" cy="12" fill="currentColor" r="1.8" />
      <circle className="equilibrium-mark-point" cx="17.5" cy="23.5" r="2.4" />
    </svg>
  );
}
