import { NextRequest } from "next/server";
import { describe, expect, it } from "vitest";
import { GET as callback } from "@/app/callback/route";
import { GET as login } from "@/app/login/route";

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
  });
});
