import {
  BinocularsIcon,
  BroadcastIcon,
  CompassIcon,
  FlaskIcon,
  FunnelSimpleIcon,
  GaugeIcon,
  PlanetIcon,
  TargetIcon,
  WalletIcon,
} from "@phosphor-icons/react/ssr";
import Link from "next/link";
import type { ReactNode } from "react";
import { LogoutForm } from "@/components/auth/logout-form";
import { EquilibriumMark } from "@/components/shell/equilibrium-mark";
import { LanguageToggle } from "@/components/shell/language-toggle";
import {
  PrimaryNavigation,
  type PrimaryNavigationItem,
} from "@/components/shell/primary-navigation";
import { ThemeToggle } from "@/components/shell/theme-toggle";
import { type ApiSession, permitsOwnerBetaProduct } from "@/lib/api/contracts";
import { LocaleProvider } from "@/lib/i18n/client";
import { type ShellDictionary, shellDictionary } from "@/lib/i18n/dictionaries/shell";
import { DEFAULT_LOCALE, type Locale } from "@/lib/i18n/locale";
import type { Theme } from "@/lib/theme/theme";

const NAV_ICON_SIZE = 20;

function memberNavigation(
  t: ShellDictionary,
  includeOwnerBetaProducts: boolean,
): PrimaryNavigationItem[] {
  const items = [
    {
      href: "/",
      icon: <PlanetIcon aria-hidden={true} size={NAV_ICON_SIZE} weight="regular" />,
      label: t.navDashboard,
    },
    {
      href: "/strategies",
      icon: <TargetIcon aria-hidden={true} size={NAV_ICON_SIZE} weight="regular" />,
      label: t.navStrategies,
    },
    {
      href: "/recommendations",
      icon: <CompassIcon aria-hidden={true} size={NAV_ICON_SIZE} weight="regular" />,
      label: t.navRecommendations,
    },
    {
      href: "/candidates",
      icon: <BinocularsIcon aria-hidden={true} size={NAV_ICON_SIZE} weight="regular" />,
      label: t.navCandidates,
    },
    {
      href: "/screener",
      icon: <FunnelSimpleIcon aria-hidden={true} size={NAV_ICON_SIZE} weight="regular" />,
      label: t.navScreener,
    },
    {
      href: "/backtests",
      icon: <FlaskIcon aria-hidden={true} size={NAV_ICON_SIZE} weight="regular" />,
      label: t.navBacktests,
    },
    {
      href: "/paper",
      icon: <WalletIcon aria-hidden={true} size={NAV_ICON_SIZE} weight="regular" />,
      label: t.navPaperAccount,
    },
  ];
  if (includeOwnerBetaProducts) {
    return items;
  }
  return items.filter((item) => !["/recommendations", "/backtests", "/paper"].includes(item.href));
}

function navigationForRole(
  session: ApiSession,
  t: ShellDictionary,
): readonly PrimaryNavigationItem[] {
  const member = memberNavigation(t, permitsOwnerBetaProduct(session));
  if (session.role === "member") {
    return member;
  }
  return [
    ...member,
    {
      href: "/admin",
      icon: <GaugeIcon aria-hidden={true} size={NAV_ICON_SIZE} weight="regular" />,
      label: t.navAdministration,
    },
    {
      href: "/live",
      icon: <BroadcastIcon aria-hidden={true} size={NAV_ICON_SIZE} weight="regular" />,
      label: t.navLiveControls,
    },
  ];
}

export type AppShellProps = {
  readonly children: ReactNode;
  readonly locale?: Locale;
  readonly session: ApiSession;
  readonly theme?: Theme | undefined;
};

export function AppShell({ children, locale = DEFAULT_LOCALE, session, theme }: AppShellProps) {
  const t = shellDictionary[locale];
  const roleLabel = session.role === "owner" ? t.roleOwner : t.roleMember;
  return (
    <LocaleProvider initialLocale={locale}>
      <div className="app-shell">
        <a className="skip-link" href="#main-content">
          {t.skipToMain}
        </a>
        <header className="shell-header">
          <Link className="shell-brand" href="/">
            <EquilibriumMark size={26} />
            <span className="shell-brand-text">
              <strong>{t.brandName}</strong>
              <span>{t.brandTagline}</span>
            </span>
          </Link>
          <div className="shell-controls">
            <p className="shell-role-pill">{roleLabel}</p>
            <ThemeToggle
              initialTheme={theme}
              labelToDark={t.themeToggleToDark}
              labelToLight={t.themeToggleToLight}
            />
            <LanguageToggle label={t.languageToggleLabel} />
          </div>
        </header>
        <PrimaryNavigation items={navigationForRole(session, t)} />
        <main className="shell-main" id="main-content">
          {children}
        </main>
        <LogoutForm />
      </div>
    </LocaleProvider>
  );
}
