import {
  getRewrittenUrl,
  unstable_getResponseFromNextConfig,
} from "next/experimental/testing/server";
import { NextRequest } from "next/server";
import { describe, expect, it } from "vitest";
import { GET as callback } from "@/app/callback/route";
import { GET as login } from "@/app/login/route";
import nextConfig from "@/next.config";

describe("server-auth handoff routes", () => {
  it("hands login to the existing same-origin auth authority", () => {
    // Given
    const request = new NextRequest("https://lagrange.test/login");

    // When
    const response = login(request);

    // Then
    expect(response.status).toBe(307);
    expect(response.headers.get("location")).toBe("https://lagrange.test/auth/login");
    expect(response.headers.get("cache-control")).toBe("no-store");
  });

  it("forwards only OIDC callback code and state, dropping unapproved URL data", () => {
    // Given
    const request = new NextRequest(
      "https://lagrange.test/callback?code=opaque-code&state=opaque-state&user_id=leak&token=leak",
    );

    // When
    const response = callback(request);

    // Then
    expect(response.status).toBe(307);
    expect(response.headers.get("location")).toBe(
      "https://lagrange.test/auth/callback?code=opaque-code&state=opaque-state",
    );
    expect(response.headers.get("location")).not.toContain("user_id");
    expect(response.headers.get("location")).not.toContain("token=leak");
    expect(response.headers.get("cache-control")).toBe("no-store");
  });

  it("fails closed without a complete callback pair", () => {
    // Given
    const request = new NextRequest("https://lagrange.test/callback?code=opaque-code");

    // When
    const response = callback(request);

    // Then
    expect(response.status).toBe(307);
    expect(response.headers.get("location")).toBe(
      "https://lagrange.test/login?reason=callback-invalid",
    );
    expect(response.headers.get("cache-control")).toBe("no-store");
  });

  it("proxies the unversioned auth authority to the internal API", async () => {
    // Given
    const rewrites = await nextConfig.rewrites?.();

    // Then
    expect(Array.isArray(rewrites)).toBe(true);
    expect(rewrites).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          destination: expect.stringMatching(/\/auth\/:path\*$/),
          source: "/auth/:path*",
        }),
      ]),
    );
  });

  it("preserves callback code and state through the internal auth rewrite", async () => {
    // Given
    const response = await unstable_getResponseFromNextConfig({
      nextConfig,
      url: "https://lagrange.test/auth/callback?code=opaque-code&state=opaque-state",
    });

    // When
    const rewrittenUrl = getRewrittenUrl(response);

    // Then
    expect(rewrittenUrl).not.toBeNull();
    const destination = new URL(rewrittenUrl as string);
    expect(destination.pathname).toBe("/auth/callback");
    expect(destination.searchParams.get("code")).toBe("opaque-code");
    expect(destination.searchParams.get("state")).toBe("opaque-state");
  });
});
