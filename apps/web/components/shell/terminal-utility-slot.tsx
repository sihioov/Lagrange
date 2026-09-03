"use client";

import { createContext, type ReactNode, useCallback, useContext, useMemo, useState } from "react";
import { createPortal } from "react-dom";

type TerminalUtilityHostContextValue = {
  readonly host: HTMLDivElement | null;
  readonly setHost: (host: HTMLDivElement | null) => void;
};

const TerminalUtilityHostContext = createContext<TerminalUtilityHostContextValue | null>(null);

export function TerminalUtilityHostProvider({ children }: { readonly children: ReactNode }) {
  const [host, setHost] = useState<HTMLDivElement | null>(null);
  const value = useMemo(() => ({ host, setHost }), [host]);

  return (
    <TerminalUtilityHostContext.Provider value={value}>
      {children}
    </TerminalUtilityHostContext.Provider>
  );
}

function useTerminalUtilityHost(): TerminalUtilityHostContextValue | null {
  return useContext(TerminalUtilityHostContext);
}

export function TerminalUtilityHost({
  className,
  slotId,
}: {
  readonly className?: string | undefined;
  readonly slotId: string;
}) {
  const context = useTerminalUtilityHost();
  if (context === null) {
    throw new Error("TerminalUtilityHostProvider is required for the terminal host.");
  }
  const { setHost } = context;
  const registerHost = useCallback(
    (host: HTMLDivElement | null) => {
      setHost(host);
    },
    [setHost],
  );

  return <div className={className} data-terminal-utility-host={slotId} ref={registerHost} />;
}

export function TerminalUtilitySlot({ children }: { readonly children: ReactNode }) {
  const host = useTerminalUtilityHost()?.host ?? null;

  // Server render and first hydration omit portal content. The host ref then commits and places
  // the existing product-owned nodes in the shell without cloning or losing their context.
  return host === null ? null : createPortal(children, host);
}
