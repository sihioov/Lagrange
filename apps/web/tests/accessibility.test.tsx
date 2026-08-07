import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { AppShell } from "@/components/shell/app-shell";
import { StatePanel } from "@/components/states/state-panel";

const OWNER_SESSION = {
  user_id: "00000000-0000-4000-8000-000000000001",
  role: "owner",
  expires_at_secs: 2_000_000_000,
} as const;

describe("application shell accessibility", () => {
  it("provides named landmarks and a labeled logout form", () => {
    // Given
    const shell = (
      <AppShell session={OWNER_SESSION}>
        <h1>Dashboard</h1>
      </AppShell>
    );

    // When
    const markup = renderToStaticMarkup(shell);

    // Then
    expect(markup).toContain("<header");
    expect(markup).toContain('<nav aria-label="Primary"');
    expect(markup).toContain("<main");
    expect(markup).toContain('<form aria-label="Sign out"');
    expect(markup).toContain("<h1>Dashboard</h1>");
  });

  it("announces actionable error content", () => {
    // Given
    const errorState = (
      <StatePanel
        action={<button type="button">Try again</button>}
        kind="error"
        message="The authenticated request could not be completed."
        title="We could not load this view"
      />
    );

    // When
    const markup = renderToStaticMarkup(errorState);
    const headingId = markup.match(/aria-labelledby="([^"]+)"/)?.[1] ?? "";

    // Then
    expect(markup).toContain('role="alert"');
    expect(headingId).not.toBe("");
    expect(markup).toContain(`<h2 id="${headingId}">We could not load this view</h2>`);
    expect(markup).toContain("We could not load this view");
    expect(markup).toContain("Try again");
  });
});
