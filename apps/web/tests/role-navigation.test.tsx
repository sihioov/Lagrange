import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { AppShell } from "@/components/shell/app-shell";
import type { ApiSession } from "@/lib/api/contracts";

const MEMBER_SESSION = {
  user_id: "00000000-0000-4000-8000-000000000002",
  role: "member",
  expires_at_secs: 2_000_000_000,
} as const satisfies ApiSession;

const OWNER_SESSION = {
  user_id: "00000000-0000-4000-8000-000000000001",
  role: "owner",
  expires_at_secs: 2_000_000_000,
} as const satisfies ApiSession;

function renderShell(session: ApiSession): string {
  return renderToStaticMarkup(
    <AppShell session={session}>
      <h1>Dashboard</h1>
    </AppShell>,
  );
}

describe("role-aware primary navigation", () => {
  it("shows research destinations without Owner operations for a Member", () => {
    // Given
    const session = MEMBER_SESSION;

    // When
    const markup = renderShell(session);

    // Then
    expect(markup).toContain('href="/strategies"');
    expect(markup).toContain('href="/recommendations"');
    expect(markup).toContain('href="/backtests"');
    expect(markup).toContain('href="/paper"');
    expect(markup).not.toContain('href="/admin"');
    expect(markup).not.toContain('href="/live"');
  });

  it("adds explicit administration destinations for the Owner", () => {
    // Given
    const session = OWNER_SESSION;

    // When
    const markup = renderShell(session);

    // Then
    expect(markup).toContain('href="/admin"');
    expect(markup).toContain('href="/live"');
  });
});
