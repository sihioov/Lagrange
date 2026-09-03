"use client";

import { usePathname } from "next/navigation";
import type { ReactNode } from "react";
import type { PrimaryNavigationItem } from "@/components/shell/primary-navigation";
import { ResearchTerminalShell } from "@/components/shell/research-terminal-shell";

export function isStockBetaPathname(pathname: string): boolean {
  return pathname === "/stock-beta" || pathname.startsWith("/stock-beta/");
}

export type RouteAwareShellProps = {
  readonly children: ReactNode;
  readonly languageLabel: string;
  readonly navigation: readonly PrimaryNavigationItem[];
  readonly privateSessionLabel: string;
  readonly productLabel: string;
  readonly readOnlyLabel: string;
  readonly roleLabel: string;
  readonly skipToMainLabel: string;
};

/**
 * The authenticated layout cannot read the active pathname on the server. This narrow client
 * boundary chooses one complete shell tree; it never mounts both shells and does not depend on
 * CSS visibility for route selection.
 */
export function RouteAwareShell({
  children,
  languageLabel,
  navigation,
  privateSessionLabel,
  productLabel,
  readOnlyLabel,
  roleLabel,
  skipToMainLabel,
}: RouteAwareShellProps) {
  const pathname = usePathname() ?? "";
  const currentDestination = navigation.find(
    (item) => pathname === item.href || (item.href !== "/" && pathname.startsWith(`${item.href}/`)),
  );
  const contextLabel =
    currentDestination?.label ??
    (pathname.startsWith("/stocks/") ? "Stock analysis" : productLabel);
  return (
    <ResearchTerminalShell
      languageLabel={languageLabel}
      navigation={navigation}
      productLabel={contextLabel}
      sessionLabel={isStockBetaPathname(pathname) ? readOnlyLabel : privateSessionLabel}
      roleLabel={roleLabel}
      shellKind={isStockBetaPathname(pathname) ? "stock-beta-terminal" : "research-terminal"}
      skipToMainLabel={skipToMainLabel}
    >
      {children}
    </ResearchTerminalShell>
  );
}
