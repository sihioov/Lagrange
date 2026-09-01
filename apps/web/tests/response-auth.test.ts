import { describe, expect, it } from "vitest";
import type { ApiErrorCode, ApiErrorEnvelope } from "@/lib/api/contracts";
import { ApiContractError, ApiProblem, isLoginRequiredError } from "@/lib/api/response";

function problem(code: ApiErrorCode): ApiProblem {
  return new ApiProblem(401, {
    error: {
      code,
      message: "static test message",
      request_id: "request-test",
    },
  } satisfies ApiErrorEnvelope);
}

describe("authentication response classification", () => {
  it.each(["SESSION_UNKNOWN", "SESSION_EXPIRED"] as const)("requires login for %s", (code) => {
    expect(isLoginRequiredError(problem(code))).toBe(true);
  });

  it.each(["FORBIDDEN", "INTERNAL"] as const)("does not treat %s as a login failure", (code) => {
    expect(isLoginRequiredError(problem(code))).toBe(false);
  });

  it("does not classify contract or unrelated errors as a login failure", () => {
    expect(isLoginRequiredError(new ApiContractError(502, "invalid response"))).toBe(false);
    expect(isLoginRequiredError(new Error("network failure"))).toBe(false);
    expect(isLoginRequiredError(null)).toBe(false);
  });
});
