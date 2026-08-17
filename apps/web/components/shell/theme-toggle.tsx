"use client";

import { MoonIcon, SunIcon } from "@phosphor-icons/react/ssr";
import { useEffect, useState } from "react";
import { THEME_COOKIE, type Theme } from "@/lib/theme/theme";

function applyTheme(theme: Theme): void {
  document.documentElement.dataset["theme"] = theme;
  document.cookie = `${THEME_COOKIE}=${theme}; path=/; max-age=31536000; samesite=lax`;
}

export type ThemeToggleProps = {
  readonly initialTheme: Theme | undefined;
  readonly labelToDark: string;
  readonly labelToLight: string;
};

/**
 * `initialTheme` comes from the `theme` cookie the server already read, so a
 * returning visitor's explicit choice renders correctly with no flash. A
 * first-time visitor has no cookie yet — `initialTheme` is `undefined` on
 * both server and client, which the initial render must honor identically
 * (defaulting the icon to "light") to avoid a hydration mismatch: the server
 * has no access to `matchMedia` at all, so branching on it during the
 * initial render, even guarded by `typeof window`, produces different
 * markup on each side. The `useEffect` below corrects the icon after mount,
 * once `matchMedia` is actually available — the page's background is
 * already correct throughout via the `prefers-color-scheme` CSS media
 * query, so only the icon itself needs this one-time correction.
 */
export function ThemeToggle({ initialTheme, labelToDark, labelToLight }: ThemeToggleProps) {
  const [theme, setTheme] = useState<Theme>(initialTheme ?? "light");

  useEffect(() => {
    if (initialTheme !== undefined) {
      return;
    }
    if (window.matchMedia("(prefers-color-scheme: dark)").matches) {
      setTheme("dark");
    }
  }, [initialTheme]);

  const isDark = theme === "dark";

  return (
    <button
      aria-label={isDark ? labelToLight : labelToDark}
      className="theme-toggle"
      onClick={() => {
        const next: Theme = isDark ? "light" : "dark";
        setTheme(next);
        applyTheme(next);
      }}
      type="button"
    >
      {isDark ? (
        <SunIcon aria-hidden={true} size={18} weight="regular" />
      ) : (
        <MoonIcon aria-hidden={true} size={18} weight="regular" />
      )}
    </button>
  );
}
