import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AppShell } from "@/components/shell/app-shell";
import type { ApiSession } from "@/lib/api/contracts";

const navigationState = vi.hoisted(() => ({ pathname: "/recommendations" }));

vi.mock("next/navigation", () => ({
  useRouter: () => ({ refresh: () => undefined }),
  usePathname: () => navigationState.pathname,
}));

const MEMBER_SESSION = {
  user_id: "00000000-0000-4000-8000-000000000002",
  role: "member",
  expires_at_secs: 2_000_000_000,
  owner_beta_access_mode: "disabled",
  owner_beta_paper_mode: "disabled",
} as const satisfies ApiSession;

const OWNER_BETA_MEMBER_SESSION = {
  ...MEMBER_SESSION,
  owner_beta_access_mode: "owner_only",
} as const satisfies ApiSession;

const OWNER_SESSION = {
  user_id: "00000000-0000-4000-8000-000000000001",
  role: "owner",
  expires_at_secs: 2_000_000_000,
  owner_beta_access_mode: "disabled",
  owner_beta_paper_mode: "disabled",
} as const satisfies ApiSession;

function renderShell(session: ApiSession, pathname = "/recommendations"): string {
  navigationState.pathname = pathname;
  return renderToStaticMarkup(
    <AppShell session={session}>
      <h1>Dashboard</h1>
    </AppShell>,
  );
}

describe("role-aware primary navigation", () => {
  beforeEach(() => {
    navigationState.pathname = "/recommendations";
  });

  it("marks only the active destination as the current page", () => {
    // Given
    const session = MEMBER_SESSION;

    // When
    const markup = renderShell(session);
    const currentLabels = Array.from(markup.matchAll(/<a([^>]*)>([\s\S]*?)<\/a>/g), (match) => ({
      attributes: match[1] ?? "",
      label: (match[2] ?? "").replace(/<[^>]*>/g, "").trim(),
    }))
      .filter((link) => link.attributes.includes('aria-current="page"'))
      .map((link) => link.label);

    // Then
    expect(currentLabels).toEqual(["Recommendations"]);
  });

  it("shows research destinations without Owner operations for a Member", () => {
    // Given
    const session = MEMBER_SESSION;

    // When
    const markup = renderShell(session);

    // Then
    expect(markup).toContain('href="/strategies"');
    expect(markup).toContain('href="/recommendations"');
    expect(markup).toContain('href="/candidates"');
    expect(markup).toContain('href="/screener"');
    expect(markup).toContain('href="/backtests"');
    expect(markup).toContain('href="/paper"');
    expect(markup).not.toContain('href="/admin"');
    expect(markup).not.toContain('href="/live"');
    expect(markup).not.toContain('href="/stock-beta"');
  });

  it("adds explicit administration destinations for the Owner", () => {
    // Given
    const session = OWNER_SESSION;

    // When
    const markup = renderShell(session);

    // Then
    expect(markup).toContain('href="/admin"');
    expect(markup).toContain('href="/live"');
    expect(markup).toContain('href="/stock-beta"');
  });

  it("removes only owner-beta product destinations for a Member", () => {
    const markup = renderShell(OWNER_BETA_MEMBER_SESSION);

    expect(markup).not.toContain('href="/recommendations"');
    expect(markup).not.toContain('href="/backtests"');
    expect(markup).not.toContain('href="/paper"');
    expect(markup).toContain('href="/strategies"');
    expect(markup).toContain('href="/candidates"');
    expect(markup).toContain('href="/screener"');
    expect(markup).not.toContain('href="/admin"');
    expect(markup).not.toContain('href="/live"');
  });

  it("keeps beta products but hides locked Paper, Admin, and Live for the Owner", () => {
    const markup = renderShell({
      ...OWNER_SESSION,
      owner_beta_access_mode: "owner_only",
      owner_beta_paper_mode: "disabled",
    });

    expect(markup).toContain('href="/recommendations"');
    expect(markup).toContain('href="/backtests"');
    expect(markup).not.toContain('href="/paper"');
    expect(markup).not.toContain('href="/admin"');
    expect(markup).not.toContain('href="/live"');
  });

  it("shows Paper to the Owner only after its separate activation", () => {
    const markup = renderShell({
      ...OWNER_SESSION,
      owner_beta_access_mode: "owner_only",
      owner_beta_paper_mode: "enabled",
    });

    expect(markup).toContain('href="/recommendations"');
    expect(markup).toContain('href="/backtests"');
    expect(markup).toContain('href="/paper"');
    expect(markup).not.toContain('href="/admin"');
    expect(markup).not.toContain('href="/live"');
  });

  it("mounts only the Stock Beta terminal shell on Stock Beta routes", () => {
    const markup = renderShell(OWNER_SESSION, "/stock-beta/005930.KRX");
    const utilityHeader =
      /<header[^>]*data-terminal-utility-bar="stock-beta"[^>]*>([\s\S]*?)<\/header>/.exec(
        markup,
      )?.[1] ?? "";

    expect(markup).toContain('data-shell="stock-beta-terminal"');
    expect(markup).not.toContain('data-shell="general"');
    expect(markup).not.toContain('class="app-shell"');
    expect(markup).not.toContain('class="shell-header"');
    expect(markup).not.toContain("Switch to dark theme");
    expect(markup.match(/aria-label="Primary"/g)).toHaveLength(1);
    expect(markup.match(/<main/g)).toHaveLength(1);
    expect(utilityHeader).toContain('data-terminal-utility-host="stock-beta"');
    expect(markup).toContain('href="/stock-beta"');
    expect(markup).toContain("Sign out");
  });

  it("keeps the terminal shell contract for a non-Stock-Beta authenticated route", () => {
    const markup = renderShell(OWNER_SESSION, "/recommendations");

    expect(markup).toContain('data-shell="research-terminal"');
    expect(markup).not.toContain('data-shell="general"');
    expect(markup).not.toContain('class="app-shell"');
    expect(markup).not.toContain('class="shell-header"');
    expect(markup).not.toContain("Switch to dark theme");
    expect(markup).not.toContain('data-shell="stock-beta-terminal"');
    expect(markup).toContain('data-terminal-utility-host="stock-beta"');
    expect(markup).toContain('data-terminal-utility-bar="research"');
    expect(markup.match(/aria-label="Primary"/g)).toHaveLength(1);
    expect(markup.match(/<main/g)).toHaveLength(1);
  });
});
