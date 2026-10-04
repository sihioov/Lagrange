import { subscribeToBrowserLogout } from "@/lib/api/browser-lifecycle";
import type { IntradayStreamContext, IntradayStreamController } from "./intraday-stream-controller";

type BrowserEvents = Pick<EventTarget, "addEventListener" | "removeEventListener">;
export type IntradayStreamBrowser = BrowserEvents & {
  readonly document: BrowserEvents & Pick<Document, "visibilityState">;
  readonly navigator: Pick<Navigator, "onLine">;
};

/** The tab owns listeners as well as its controller. pagehide also covers bfcache entry. */
export function bindIntradayStreamBrowser(
  controller: Pick<IntradayStreamController, "setContext" | "logout" | "destroy">,
  readContext: () => Omit<IntradayStreamContext, "visible" | "online">,
  browser: IntradayStreamBrowser = window,
): { refresh: () => void; dispose: () => void } {
  let pageActive = true;
  let disposed = false;
  const refresh = (): void => {
    if (disposed) return;
    controller.setContext({
      ...readContext(),
      visible: pageActive && browser.document.visibilityState === "visible",
      online: browser.navigator.onLine === true,
    });
  };
  const hide = (): void => {
    pageActive = false;
    refresh();
  };
  const show = (): void => {
    pageActive = true;
    refresh();
  };
  browser.addEventListener("online", refresh);
  browser.addEventListener("offline", refresh);
  browser.addEventListener("pagehide", hide);
  browser.addEventListener("pageshow", show);
  browser.document.addEventListener("visibilitychange", refresh);
  const unsubscribeLogout = subscribeToBrowserLogout(() => controller.logout());
  refresh();
  return {
    refresh,
    dispose() {
      if (disposed) return;
      disposed = true;
      browser.removeEventListener("online", refresh);
      browser.removeEventListener("offline", refresh);
      browser.removeEventListener("pagehide", hide);
      browser.removeEventListener("pageshow", show);
      browser.document.removeEventListener("visibilitychange", refresh);
      unsubscribeLogout();
      controller.destroy();
    },
  };
}
