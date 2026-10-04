import universe from "../../../../configs/universes/kr-stock-price-beta-v1.json";
import type { OwnerEquityV2MembershipModel } from "./equity-signals-contracts";
import {
  IntradayStreamContractError,
  type IntradayStreamIdentity,
} from "./intraday-stream-contracts";
import { canonicalStreamIdentities } from "./intraday-stream-lease-contracts";

// One checked-in observation list is shared by onboarding, the collector and the board.
if (
  universe.instrument_count !== 30 ||
  universe.instruments.length !== 30 ||
  new Set(universe.instruments.map((instrument) => instrument.id)).size !== 30
)
  throw new IntradayStreamContractError();

export const MARKET_STREAM_INSTRUMENTS: readonly Readonly<{ id: string; name: string }>[] =
  universe.instruments.map((instrument) => Object.freeze({ ...instrument }));
const instruments = new Set(MARKET_STREAM_INSTRUMENTS.map((instrument) => instrument.id));

export function readyStreamIdentities(
  memberships: readonly OwnerEquityV2MembershipModel[],
): IntradayStreamIdentity[] {
  const ready = memberships.filter(
    (membership) => membership.lifecycle === "READY" && instruments.has(membership.instrument_id),
  );
  if (ready.length === 0) return [];
  return canonicalStreamIdentities(
    ready.map((membership) => ({
      membership_id: membership.id,
      instrument_id: membership.instrument_id,
      generation: membership.generation,
    })),
  );
}
