import type { Locator, Page } from "@playwright/test";

/**
 * The application's alerts, excluding Next.js's route announcer.
 *
 * Next renders `<div id="__next-route-announcer__" role="alert">` to read the
 * new page title to screen readers after a client-side navigation. It is empty
 * and invisible, but it IS an `alert` for locator purposes, so a bare
 * `page.getByRole("alert")` matches two elements and fails Playwright's strict
 * mode — and only sometimes, because the announcer is absent until the first
 * client-side navigation has occurred. That made every page-scoped alert
 * assertion order-dependent: the same test passed or failed according to which
 * spec ran before it.
 *
 * Region-scoped assertions (`someSection.getByRole("alert")`) are unaffected,
 * since the announcer lives outside them. Only page-scoped ones need this.
 */
export function appAlert(page: Page): Locator {
  // `.and()` intersects on the same element; `.locator()` would chain into
  // descendants and match nothing.
  return page.getByRole("alert").and(page.locator(":not(#__next-route-announcer__)"));
}
