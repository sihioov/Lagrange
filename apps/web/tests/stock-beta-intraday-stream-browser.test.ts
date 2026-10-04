import { describe, expect, it, vi } from "vitest";
import { bindIntradayStreamBrowser } from "@/components/stock-beta/quote/intraday-stream-browser";
import { notifyBrowserLogout } from "@/lib/api/browser-lifecycle";

class Tab extends EventTarget {
  readonly document = Object.assign(new EventTarget(), {
    visibilityState: "visible" as DocumentVisibilityState,
  });
  readonly navigator = { onLine: true };
}

// Listener wiring checks. Actual React/Chromium coverage is a separate browser gate.
describe("stream tab listener ownership", () => {
  it("suspends on hide/offline/pagehide and refreshes the current context on restore", () => {
    const tab = new Tab();
    const controller = { setContext: vi.fn(), logout: vi.fn(), destroy: vi.fn() };
    let sessionKey = "session-one";
    const bound = bindIntradayStreamBrowser(
      controller,
      () => ({
        enabled: true,
        owner: true,
        sessionKey,
        identities: [],
      }),
      tab,
    );
    try {
      expect(controller.setContext).toHaveBeenLastCalledWith(
        expect.objectContaining({ visible: true, online: true }),
      );
      tab.document.visibilityState = "hidden";
      tab.document.dispatchEvent(new Event("visibilitychange"));
      expect(controller.setContext).toHaveBeenLastCalledWith(
        expect.objectContaining({ visible: false }),
      );
      tab.document.visibilityState = "visible";
      tab.navigator.onLine = false;
      tab.dispatchEvent(new Event("offline"));
      expect(controller.setContext).toHaveBeenLastCalledWith(
        expect.objectContaining({ visible: true, online: false }),
      );
      tab.navigator.onLine = true;
      tab.dispatchEvent(new Event("pagehide"));
      tab.dispatchEvent(new Event("online"));
      expect(controller.setContext).toHaveBeenLastCalledWith(
        expect.objectContaining({ visible: false, online: true }),
      );
      sessionKey = "session-two";
      tab.dispatchEvent(new Event("pageshow"));
      expect(controller.setContext).toHaveBeenLastCalledWith(
        expect.objectContaining({ visible: true, sessionKey: "session-two" }),
      );
    } finally {
      bound.dispose();
    }
  });

  it("delivers logout and removes every listener exactly once on disposal", () => {
    const tab = new Tab();
    const controller = { setContext: vi.fn(), logout: vi.fn(), destroy: vi.fn() };
    const bound = bindIntradayStreamBrowser(
      controller,
      () => ({
        enabled: true,
        owner: true,
        sessionKey: "session",
        identities: [],
      }),
      tab,
    );
    notifyBrowserLogout();
    expect(controller.logout).toHaveBeenCalledOnce();
    bound.dispose();
    bound.dispose();
    const calls = controller.setContext.mock.calls.length;
    for (const event of ["pagehide", "pageshow", "online", "offline"])
      tab.dispatchEvent(new Event(event));
    tab.document.dispatchEvent(new Event("visibilitychange"));
    bound.refresh();
    notifyBrowserLogout();
    expect(controller.setContext).toHaveBeenCalledTimes(calls);
    expect(controller.logout).toHaveBeenCalledOnce();
    expect(controller.destroy).toHaveBeenCalledOnce();
  });
});
