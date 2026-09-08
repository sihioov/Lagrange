import type { OwnerEquityV2MembershipModel } from "@/lib/products/equity-signals-contracts";
import type { IntradayQuoteIdentity } from "@/lib/products/intraday-quotes-contracts";

export function matchReadyIntradayQuoteMembership(
  memberships: readonly OwnerEquityV2MembershipModel[],
  identity: Pick<IntradayQuoteIdentity, "instrument_id" | "generation">,
): OwnerEquityV2MembershipModel | null {
  return (
    memberships.find(
      (membership) =>
        membership.lifecycle === "READY" &&
        membership.instrument_id === identity.instrument_id &&
        membership.generation === identity.generation,
    ) ?? null
  );
}

export function intradayQuoteIdentityForMembership(
  membership: OwnerEquityV2MembershipModel | null,
): IntradayQuoteIdentity | null {
  if (membership === null || membership.lifecycle !== "READY") return null;
  return {
    generation: membership.generation,
    instrument_id: membership.instrument_id,
    membership_id: membership.id,
  };
}
