import Link from "next/link";
import type { ReactNode } from "react";
import { LogoutForm } from "@/components/auth/logout-form";
import type { ApiSession } from "@/lib/api/contracts";

const ROLE_LABELS = {
  member: "Member",
  owner: "Owner",
} as const satisfies Record<ApiSession["role"], string>;

type NavigationItem = {
  readonly href: string;
  readonly label: string;
};

const MEMBER_NAVIGATION = [
  { href: "/", label: "Dashboard" },
  { href: "/strategies", label: "Strategies" },
  { href: "/recommendations", label: "Recommendations" },
  { href: "/backtests", label: "Backtests" },
  { href: "/paper", label: "Paper account" },
] as const satisfies readonly NavigationItem[];

const NAVIGATION_BY_ROLE = {
  member: MEMBER_NAVIGATION,
  owner: [
    ...MEMBER_NAVIGATION,
    { href: "/admin", label: "Administration" },
    { href: "/live", label: "Live controls" },
  ],
} as const satisfies Record<ApiSession["role"], readonly NavigationItem[]>;

export type AppShellProps = {
  readonly children: ReactNode;
  readonly session: ApiSession;
};

export function AppShell({ children, session }: AppShellProps) {
  return (
    <div className="app-shell">
      <a className="skip-link" href="#main-content">
        Skip to main content
      </a>
      <header className="shell-header">
        <Link href="/">Lagrange Station</Link>
        <p>{ROLE_LABELS[session.role]}</p>
      </header>
      <nav aria-label="Primary" className="shell-navigation">
        {NAVIGATION_BY_ROLE[session.role].map((item) => (
          <Link href={item.href} key={item.href}>
            {item.label}
          </Link>
        ))}
      </nav>
      <main className="shell-main" id="main-content">
        {children}
      </main>
      <LogoutForm />
    </div>
  );
}
