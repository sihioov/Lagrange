import { afterEach, describe, expect, it, vi } from "vitest";
import { logout } from "@/lib/api/browser-client";
import { withCsrfMutationLock } from "@/lib/api/csrf-mutation-lock";
import { createIntradayStreamClient } from "@/lib/products/intraday-stream-client";

const uuid = (n: number) => `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`;
afterEach(() => vi.unstubAllGlobals());

describe("shared CSRF mutation ownership", () => {
  it("keeps concurrent stream release and logout valid under rotating CSRF", async () => {
    let generation = 0;
    const methods: string[] = [];
    const fetcher: typeof fetch = async (input, init) => {
      const request = new Request(input, init);
      methods.push(request.method);
      if (new URL(request.url).pathname === "/api/v1/auth/csrf") {
        const token = `synthetic-csrf-${++generation}`;
        // Match the real server: every completed GET changes the sole accepted
        // synchronizer hash. Yield one turn so concurrent requests can overlap.
        await new Promise<void>((resolve) => setTimeout(resolve, 0));
        return Response.json({ csrf_token: token });
      }
      if (request.headers.get("X-CSRF-Token") !== `synthetic-csrf-${generation}`) {
        return Response.json(
          { error: { code: "CSRF_DENIED", message: "denied", request_id: "fixture" } },
          { status: 403 },
        );
      }
      if (request.method === "DELETE") {
        return Response.json({ schema_version: 2, lease_id: uuid(1), released: true });
      }
      return new Response(null, { status: 204 });
    };
    const origin = "https://stream.example.test";
    const client = createIntradayStreamClient({ fetcher, origin });
    const results = await Promise.allSettled([
      client.releaseLease(
        uuid(1),
        { schema_version: 2, consumer_id: uuid(2), renewal_sequence: 0 },
        { idempotencyKey: uuid(3), signal: new AbortController().signal },
      ),
      logout({ fetcher, origin }),
    ]);
    expect(results.map((result) => result.status)).toEqual(["fulfilled", "fulfilled"]);
    const response = results[1];
    expect(
      response?.status === "fulfilled" &&
        response.value instanceof Response &&
        response.value.status,
    ).toBe(204);
    expect(methods).toEqual(["GET", "DELETE", "GET", "POST"]);
  });

  it("cancels queued work without releasing another mutation's ownership", async () => {
    let release: () => void = () => undefined;
    let entered: () => void = () => undefined;
    const started = new Promise<void>((resolve) => {
      entered = resolve;
    });
    const hold = new Promise<void>((resolve) => {
      release = resolve;
    });
    const first = withCsrfMutationLock(new AbortController().signal, async () => {
      entered();
      await hold;
    });
    await started;
    const abort = new AbortController();
    const cancelledOperation = vi.fn(async () => undefined);
    const cancelled = withCsrfMutationLock(abort.signal, cancelledOperation);
    const rejected = expect(cancelled).rejects.toMatchObject({ name: "AbortError" });
    abort.abort();
    await rejected;
    const lastOperation = vi.fn(async () => undefined);
    const last = withCsrfMutationLock(new AbortController().signal, lastOperation);
    try {
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
      expect(cancelledOperation).not.toHaveBeenCalled();
      expect(lastOperation).not.toHaveBeenCalled();
    } finally {
      release();
      await Promise.all([first, last]);
    }
    expect(lastOperation).toHaveBeenCalledOnce();
  });

  it("releases ownership after a failed operation without retrying it", async () => {
    const failed = vi.fn(async () => {
      throw new Error("synthetic failure");
    });
    await expect(withCsrfMutationLock(new AbortController().signal, failed)).rejects.toThrow(
      "synthetic failure",
    );
    const next = vi.fn(async () => 7);
    await expect(withCsrfMutationLock(new AbortController().signal, next)).resolves.toBe(7);
    expect(failed).toHaveBeenCalledOnce();
    expect(next).toHaveBeenCalledOnce();
  });

  it("uses the browser's shared lock and refuses a browser without that facility", async () => {
    vi.stubGlobal("window", {});
    vi.stubGlobal("navigator", {});
    const operation = vi.fn(async () => 9);
    const signal = new AbortController().signal;
    await expect(withCsrfMutationLock(signal, operation)).rejects.toThrow(
      "serialization is unavailable",
    );
    expect(operation).not.toHaveBeenCalled();
    const request = vi.fn(async (_name, _options, callback) => callback());
    vi.stubGlobal("navigator", { locks: { request } });
    await expect(withCsrfMutationLock(signal, operation)).resolves.toBe(9);
    expect(request).toHaveBeenCalledWith(
      "lagrange.csrf-mutation.v1",
      { mode: "exclusive", signal },
      expect.any(Function),
    );
    expect(operation).toHaveBeenCalledOnce();
  });
});
