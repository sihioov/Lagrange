import { afterEach, describe, expect, it, vi } from "vitest";
import { z } from "zod";
import { parseBrowserApiResponse } from "@/lib/api/browser-response";

const payloadSchema = z.object({ ok: z.literal(true) });

function errorResponse(code: string, status: number): Response {
  return Response.json(
    {
      error: {
        code,
        message: code === "INTERNAL" ? "internal failure" : "session failure",
        request_id: `request-${code.toLowerCase()}`,
      },
    },
    { status },
  );
}

describe("browser API response handling", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it.each([
    ["SESSION_UNKNOWN", 401],
    ["SESSION_EXPIRED", 401],
  ])("navigates to login for %s", async (code, status) => {
    const navigations: string[] = [];

    await expect(
      parseBrowserApiResponse(errorResponse(code, status), payloadSchema, {
        navigate: (href) => navigations.push(href),
      }),
    ).rejects.toMatchObject({ code, name: "ApiProblem" });

    expect(navigations).toEqual(["/login"]);
  });

  it("keeps non-session failures in the caller's existing error path", async () => {
    const navigations: string[] = [];

    await expect(
      parseBrowserApiResponse(errorResponse("INTERNAL", 500), payloadSchema, {
        navigate: (href) => navigations.push(href),
      }),
    ).rejects.toMatchObject({ code: "INTERNAL", name: "ApiProblem" });

    expect(navigations).toEqual([]);
  });

  it.each(["SESSION_UNKNOWN", "SESSION_EXPIRED"])(
    "does not navigate for %s when cancelled while the response body is parsing",
    async (code) => {
      const navigations: string[] = [];
      const request = new AbortController();
      const response = errorResponse(code, 401);
      const body = await response.json();
      let finishBody!: (value: unknown) => void;
      const json = vi.spyOn(response, "json").mockImplementation(
        () =>
          new Promise((resolve) => {
            finishBody = resolve;
          }),
      );
      const parsed = parseBrowserApiResponse(response, payloadSchema, {
        signal: request.signal,
        navigate: (href) => navigations.push(href),
      });
      expect(json).toHaveBeenCalledOnce();
      request.abort();
      finishBody(body);

      await expect(parsed).rejects.toMatchObject({ code, name: "ApiProblem", status: 401 });
      expect(navigations).toEqual([]);
    },
  );

  it("still navigates for a current request with an uncancelled signal", async () => {
    const navigations: string[] = [];
    await expect(
      parseBrowserApiResponse(errorResponse("SESSION_EXPIRED", 401), payloadSchema, {
        signal: new AbortController().signal,
        navigate: (href) => navigations.push(href),
      }),
    ).rejects.toMatchObject({ code: "SESSION_EXPIRED", name: "ApiProblem" });
    expect(navigations).toEqual(["/login"]);
  });

  it("uses a full browser navigation when no test seam is provided", async () => {
    const replace = vi.fn();
    vi.stubGlobal("window", { location: { replace } });

    await expect(
      parseBrowserApiResponse(errorResponse("SESSION_EXPIRED", 401), payloadSchema),
    ).rejects.toMatchObject({ code: "SESSION_EXPIRED" });

    expect(replace).toHaveBeenCalledOnce();
    expect(replace).toHaveBeenCalledWith("/login");
  });

  it("returns a valid generated-contract payload unchanged", async () => {
    await expect(
      parseBrowserApiResponse(Response.json({ ok: true }), payloadSchema),
    ).resolves.toEqual({ ok: true });
  });
});
