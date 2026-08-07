import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { StatePanel } from "@/components/states/state-panel";

describe("application state announcements", () => {
  it("gives each rendered panel a distinct accessible heading", () => {
    // Given
    const states = (
      <>
        <StatePanel kind="blocked" message="Creation is unavailable." title="Creation blocked" />
        <StatePanel kind="empty" message="No history is available." title="No results" />
      </>
    );

    // When
    const markup = renderToStaticMarkup(states);
    const headingIds = Array.from(markup.matchAll(/<h2 id="([^"]+)"/g), (match) => match[1]);
    const labelledBy = Array.from(
      markup.matchAll(/<section aria-labelledby="([^"]+)"/g),
      (match) => match[1],
    );

    // Then
    expect(new Set(headingIds).size).toBe(2);
    expect(labelledBy).toEqual(headingIds);
  });

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
