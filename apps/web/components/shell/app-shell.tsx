import {
  BinocularsIcon,
  BroadcastIcon,
  ChartLineUpIcon,
  CompassIcon,
  FlaskIcon,
  FunnelSimpleIcon,
  GaugeIcon,
  PlanetIcon,
  TargetIcon,
  WalletIcon,
} from "@phosphor-icons/react/ssr";
import type { ReactNode } from "react";
import type { PrimaryNavigationItem } from "@/components/shell/primary-navigation";
import { RouteAwareShell } from "@/components/stock-beta/terminal";
import {
  type ApiSession,
  type OwnerBetaProduct,
  permitsOwnerBetaProduct,
} from "@/lib/api/contracts";
import { LocaleProvider } from "@/lib/i18n/client";
import { type ShellDictionary, shellDictionary } from "@/lib/i18n/dictionaries/shell";
import { stockBetaDictionary } from "@/lib/i18n/dictionaries/stock-beta";
import { DEFAULT_LOCALE, type Locale } from "@/lib/i18n/locale";

const NAV_ICON_SIZE = 20;

function memberNavigation(t: ShellDictionary, session: ApiSession): PrimaryNavigationItem[] {
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
  const ownerBetaDestinations = {
    "/backtests": "backtests",
    "/paper": "paper",
    "/recommendations": "recommendations",
  } as const satisfies Record<string, OwnerBetaProduct>;
  return items.filter((item) => {
    const product = ownerBetaDestinations[item.href as keyof typeof ownerBetaDestinations];
    return product === undefined || permitsOwnerBetaProduct(session, product);
  });
}

function navigationForRole(
  session: ApiSession,
  t: ShellDictionary,
): readonly PrimaryNavigationItem[] {
  const member = memberNavigation(t, session);
  if (session.role === "member") {
    return member;
  }
  const ownerItems: PrimaryNavigationItem[] = [
    ...member,
    {
      href: "/stock-beta",
      icon: <ChartLineUpIcon aria-hidden={true} size={NAV_ICON_SIZE} weight="regular" />,
      label: t.navStockBeta,
    },
  ];
  if (session.owner_beta_access_mode === "disabled") {
    ownerItems.push({
      href: "/admin",
      icon: <GaugeIcon aria-hidden={true} size={NAV_ICON_SIZE} weight="regular" />,
      label: t.navAdministration,
    });
    ownerItems.push({
      href: "/live",
      icon: <BroadcastIcon aria-hidden={true} size={NAV_ICON_SIZE} weight="regular" />,
      label: t.navLiveControls,
    });
  }
  return ownerItems;
}

export type AppShellProps = {
  readonly children: ReactNode;
  readonly locale?: Locale;
  readonly session: ApiSession;
};

export function AppShell({ children, locale = DEFAULT_LOCALE, session }: AppShellProps) {
  const t = shellDictionary[locale];
  const roleLabel = session.role === "owner" ? t.roleOwner : t.roleMember;
  const navigation = navigationForRole(session, t);
  return (
    <LocaleProvider initialLocale={locale}>
      <RouteAwareShell
        languageLabel={t.languageToggleLabel}
        navigation={navigation}
        privateSessionLabel={t.privateSession}
        productLabel={t.navStockBeta}
        readOnlyLabel={stockBetaDictionary[locale].filtersEyebrow}
        roleLabel={roleLabel}
        skipToMainLabel={t.skipToMain}
      >
        {children}
      </RouteAwareShell>
    </LocaleProvider>
  );
}
