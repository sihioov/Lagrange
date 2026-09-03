"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import type { ReactNode } from "react";

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

export function PrimaryNavigation({ className, items, labelClassName }: PrimaryNavigationProps) {
  const pathname = usePathname() ?? "";
  return (
    <nav aria-label="Primary" className={className ?? "shell-navigation"}>
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
