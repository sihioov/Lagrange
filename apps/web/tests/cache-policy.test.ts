import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

function source(relativePath: string): string {
  return readFileSync(resolve(process.cwd(), relativePath), "utf8");
}

describe("authenticated route cache policy", () => {
  it("forces dynamic rendering and disables the route fetch cache", () => {
    // Given
    const layout = source("app/(authenticated)/layout.tsx");

    // When
    const policy = {
      dynamic: layout.includes('export const dynamic = "force-dynamic"'),
      fetchCache: layout.includes('export const fetchCache = "force-no-store"'),
      revalidate: layout.includes("export const revalidate = 0"),
    };

    // Then
    expect(policy).toEqual({ dynamic: true, fetchCache: true, revalidate: true });
  });

  it("binds the API client to T24 generated OpenAPI types", () => {
    // Given
    const contracts = source("lib/api/contracts.ts");

    // When
    const generatedImport = contracts.includes('from "@lagrange/api-contract"');
    const generatedSession = contracts.includes('components["schemas"]["Session"]');
    const generatedPaths = contracts.includes("keyof paths");

    // Then
    expect({ generatedImport, generatedPaths, generatedSession }).toEqual({
      generatedImport: true,
      generatedPaths: true,
      generatedSession: true,
    });
  });

  it("marks every auth handoff response no-store", () => {
    // Given
    const login = source("app/login/route.ts");
    const callback = source("app/callback/route.ts");

    // When
    const handoffs = [login, callback];

    // Then
    expect(handoffs.every((route) => route.includes('"Cache-Control": "no-store"'))).toBe(true);
  });
});
