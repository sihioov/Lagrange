"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";

export type PrimaryNavigationItem = {
  readonly href: string;
  readonly label: string;
};

export type PrimaryNavigationProps = {
  readonly items: readonly PrimaryNavigationItem[];
};

export function PrimaryNavigation({ items }: PrimaryNavigationProps) {
  const pathname = usePathname() ?? "";
  return (
    <nav aria-label="Primary" className="shell-navigation">
      {items.map((item) => {
        const isCurrent =
          pathname === item.href || (item.href !== "/" && pathname.startsWith(`${item.href}/`));
        return (
          <Link aria-current={isCurrent ? "page" : undefined} href={item.href} key={item.href}>
            {item.label}
          </Link>
        );
      })}
    </nav>
  );
}
