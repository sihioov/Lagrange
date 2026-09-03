"use client";

import type { ComponentProps } from "react";
import {
  TerminalUtilityHost,
  TerminalUtilityHostProvider,
  TerminalUtilitySlot,
} from "@/components/shell/terminal-utility-slot";

export const StockBetaTerminalUtilityHostProvider = TerminalUtilityHostProvider;
export const StockBetaTerminalUtilitySlot = TerminalUtilitySlot;

export function StockBetaTerminalUtilityHost(
  props: Omit<ComponentProps<typeof TerminalUtilityHost>, "slotId">,
) {
  return <TerminalUtilityHost {...props} slotId="stock-beta" />;
}
