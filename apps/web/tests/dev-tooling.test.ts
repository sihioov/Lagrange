import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

describe("React development inspection tooling", () => {
  it("keeps react-grab and react-scan inside a development-only gate", () => {
    // Given
    const layoutPath = resolve(process.cwd(), "app/layout.tsx");
    const layout = existsSync(layoutPath) ? readFileSync(layoutPath, "utf8") : "";

    // When
    const toolingContract = {
      developmentGate: layout.includes('process.env.NODE_ENV === "development"'),
      reactGrab: layout.includes("react-grab"),
      reactScan: layout.includes("react-scan"),
    };

    // Then
    expect(toolingContract).toEqual({
      developmentGate: true,
      reactGrab: true,
      reactScan: true,
    });
  });
});
