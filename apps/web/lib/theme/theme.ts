export type Theme = "dark" | "light";

export const THEME_COOKIE = "theme";

/**
 * `undefined` means no explicit preference: the shell renders with no
 * `data-theme` attribute at all, and `prefers-color-scheme` decides. Only a
 * stamped cookie value overrides the OS setting.
 */
export function parseTheme(value: string | undefined): Theme | undefined {
  return value === "dark" || value === "light" ? value : undefined;
}
