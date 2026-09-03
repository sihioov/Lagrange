import Link from "next/link";
import type { ReactNode } from "react";
import { LogoutForm } from "@/components/auth/logout-form";
import { EquilibriumMark } from "@/components/shell/equilibrium-mark";
import { LanguageToggle } from "@/components/shell/language-toggle";
import {
  PrimaryNavigation,
  type PrimaryNavigationItem,
} from "@/components/shell/primary-navigation";
import {
  TerminalUtilityHost,
  TerminalUtilityHostProvider,
} from "@/components/shell/terminal-utility-slot";
import styles from "./research-terminal-shell.module.css";

export type ResearchTerminalShellProps = {
  readonly children: ReactNode;
  readonly languageLabel: string;
  readonly navigation: readonly PrimaryNavigationItem[];
  readonly productLabel: string;
  readonly sessionLabel: string;
  readonly roleLabel: string;
  readonly shellKind?: "research-terminal" | "stock-beta-terminal";
  readonly skipToMainLabel: string;
};

export function ResearchTerminalShell({
  children,
  languageLabel,
  navigation,
  productLabel,
  sessionLabel,
  roleLabel,
  shellKind = "research-terminal",
  skipToMainLabel,
}: ResearchTerminalShellProps) {
  return (
    <TerminalUtilityHostProvider>
      <div className={styles["shell"]} data-shell={shellKind}>
        <a className={`skip-link ${styles["skipLink"]}`} href="#main-content">
          {skipToMainLabel}
        </a>
        <header
          className={styles["utilityBar"]}
          data-terminal-utility-bar={
            shellKind === "stock-beta-terminal" ? "stock-beta" : "research"
          }
        >
          <Link aria-label="Lagrange Station" className={styles["brand"]} href="/">
            <EquilibriumMark size={20} />
            <strong>LAGRANGE</strong>
          </Link>
          <div className={styles["utilityCenter"]}>
            <p className={styles["productContext"]}>{productLabel}</p>
            <TerminalUtilityHost className={styles["utilityHost"]} slotId="stock-beta" />
          </div>
          <div className={styles["utilityStatus"]}>
            <span>{roleLabel}</span>
            <span>{sessionLabel}</span>
            <LanguageToggle label={languageLabel} />
          </div>
        </header>
        <PrimaryNavigation
          className={styles["navigation"]}
          items={navigation}
          labelClassName={styles["navigationLabel"]}
        />
        <main className={styles["main"]} id="main-content">
          {children}
        </main>
        <div className={styles["signOut"]}>
          <LogoutForm />
        </div>
      </div>
    </TerminalUtilityHostProvider>
  );
}
