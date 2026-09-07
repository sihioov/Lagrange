"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { type ReactNode, useEffect, useRef } from "react";

export type PrimaryNavigationItem = {
  readonly href: string;
  readonly icon: ReactNode;
  readonly label: string;
};

export type PrimaryNavigationProps = {
  readonly className?: string | undefined;
  readonly items: readonly PrimaryNavigationItem[];
  readonly labelClassName?: string | undefined;
};

export function revealCurrentPrimaryNavigationItem(navigation: HTMLElement): void {
  const current = navigation.querySelector<HTMLAnchorElement>('a[aria-current="page"]');
  if (current === null) return;

  const navigationBounds = navigation.getBoundingClientRect();
  const currentBounds = current.getBoundingClientRect();
  const startDelta = currentBounds.left - navigationBounds.left;
  const endDelta = currentBounds.right - navigationBounds.right;
  const horizontalDelta = startDelta < 0 ? startDelta : endDelta > 0 ? endDelta : 0;

  if (horizontalDelta !== 0) {
    navigation.scrollTo({ left: navigation.scrollLeft + horizontalDelta, behavior: "auto" });
  }
}

export function PrimaryNavigation({ className, items, labelClassName }: PrimaryNavigationProps) {
  const pathname = usePathname() ?? "";
  const navigationRef = useRef<HTMLElement>(null);

  useEffect(() => {
    if (pathname.length === 0) return;
    const navigation = navigationRef.current;
    if (navigation === null) return;

    const revealCurrentDestination = () => revealCurrentPrimaryNavigationItem(navigation);

    revealCurrentDestination();
    window.addEventListener("resize", revealCurrentDestination);
    const resizeObserver = new ResizeObserver(revealCurrentDestination);
    resizeObserver.observe(navigation);
    return () => {
      window.removeEventListener("resize", revealCurrentDestination);
      resizeObserver.disconnect();
    };
  }, [pathname]);

  return (
    <nav aria-label="Primary" className={className ?? "shell-navigation"} ref={navigationRef}>
      {items.map((item) => {
        const isCurrent =
          pathname === item.href || (item.href !== "/" && pathname.startsWith(`${item.href}/`));
        return (
          <Link aria-current={isCurrent ? "page" : undefined} href={item.href} key={item.href}>
            {item.icon}
            <span className={labelClassName}>{item.label}</span>
          </Link>
        );
      })}
    </nav>
  );
}
