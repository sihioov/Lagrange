import { beforeEach, describe, expect, it, vi } from "vitest";
import { logout } from "@/lib/api/browser-client";
import { notifyBrowserLogout, subscribeToBrowserLogout } from "@/lib/api/browser-lifecycle";

describe("Stock Beta intraday auth lifecycle seam", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it("notifies payload-free browser consumers before the logout CSRF request", async () => {
    const events: string[] = [];
    const unsubscribe = subscribeToBrowserLogout(() => events.push("logout"));
    const fetcher: typeof fetch = async (input) => {
      const url =
        typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url;
      events.push(url.endsWith("/auth/csrf") ? "csrf" : "request");
      if (url.endsWith("/auth/csrf"))
        return new Response(JSON.stringify({ csrf_token: "csrf" }), { status: 200 });
      return new Response(null, { status: 204 });
    };

    await logout({ fetcher, origin: "https://app.example" });
    unsubscribe();
    expect(events).toEqual(["logout", "csrf", "request"]);
  });

  it("keeps notification payload-free and supports independent subscribers", () => {
    const first = vi.fn();
    const second = vi.fn();
    const removeFirst = subscribeToBrowserLogout(first);
    const removeSecond = subscribeToBrowserLogout(second);
    notifyBrowserLogout();
    removeFirst();
    notifyBrowserLogout();
    removeSecond();
    expect(first).toHaveBeenCalledOnce();
    expect(second).toHaveBeenCalledTimes(2);
    expect(first).toHaveBeenCalledWith();
    expect(second).toHaveBeenCalledWith();
  });
});
