import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

function source(relativePath: string): string {
  return readFileSync(resolve(process.cwd(), relativePath), "utf8");
}

describe("WP-4 Owner remaining-surface audit", () => {
  it("keeps the private saved-screen delete journey CSRF-protected, typed, reloadable, and recoverable", () => {
    const component = source("components/screener/saved-screens.tsx");

    expect(component).toMatch(
      /mutateWithCsrf\(`\/api\/v1\/screener\/screens\/\$\{encodeURIComponent\(id\)\}`/,
    );
    expect(component).toContain('method: "DELETE"');
    expect(component).toContain("deleteSavedScreenSchema");
    expect(component).toContain("router.refresh()");
    expect(component).toContain('message: "Saved screen deleted."');
    expect(component).toContain(
      'message: error instanceof Error ? error.message : "The screen could not be deleted."',
    );
    expect(component).toContain("disabled={deleting === screen.id}");
  });

  it("keeps the private saved-screen create journey on the same CSRF, typed, and reload boundary", () => {
    const component = source("components/screener/saved-screens.tsx");

    expect(component).toContain('mutateWithCsrf("/api/v1/screener/screens"');
    expect(component).toContain('method: "POST"');
    expect(component).toContain("savedScreenSchema");
    expect(component).toContain("Enter a screen name of 1 to 80 characters.");
    expect(component).toContain(
      'message: error instanceof Error ? error.message : "The screen could not be saved."',
    );
  });

  it("defines Admin as an explicit empty placeholder, not an operational data surface", () => {
    const page = source("app/(authenticated)/admin/page.tsx");
    const dictionary = source("lib/i18n/dictionaries/admin.ts");

    expect(page).toContain("<OwnerRoute");
    expect(page).toContain('<StatePanel kind="empty"');
    expect(page).toContain("noAreaMessage");
    expect(dictionary).toContain("Choose an administrative area");
    expect(dictionary).toContain("No administrative area is selected.");
    expect(page).not.toContain("getProductApi");
  });
});
