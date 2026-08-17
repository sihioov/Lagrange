import type { UniverseKey } from "@/lib/products/candidate-contracts";
import { universeLabel } from "@/lib/products/candidate-contracts";

export function UniverseBadge({ universe }: { readonly universe: UniverseKey }) {
  return (
    <span className="universe-badge" data-universe={universe}>
      {universeLabel(universe)}
    </span>
  );
}
