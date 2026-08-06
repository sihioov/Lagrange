import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { StatePanel } from "@/components/states/state-panel";

describe("application state announcements", () => {
  it("announces loading without interrupting the user", () => {
    // Given
    const state = (
      <StatePanel
        kind="loading"
        message="Authenticated data is being requested."
        title="Loading dashboard"
      />
    );

    // When
    const markup = renderToStaticMarkup(state);

    // Then
    expect(markup).toContain('role="status"');
    expect(markup).toContain('aria-live="polite"');
    expect(markup).toContain('aria-busy="true"');
  });

  it("announces an entitlement block immediately", () => {
    // Given
    const state = (
      <StatePanel
        kind="blocked"
        message="Your current entitlement does not allow this operation."
        title="Access blocked"
      />
    );

    // When
    const markup = renderToStaticMarkup(state);

    // Then
    expect(markup).toContain('role="alert"');
    expect(markup).toContain('aria-live="assertive"');
    expect(markup).toContain('data-kind="blocked"');
  });
});
