type BrowserLogoutListener = () => void;

const logoutListeners = new Set<BrowserLogoutListener>();

/** Subscribe to the local logout boundary without carrying auth or product data. */
export function subscribeToBrowserLogout(listener: BrowserLogoutListener): () => void {
  logoutListeners.add(listener);
  return () => logoutListeners.delete(listener);
}

/** Notify browser-only consumers before the logout request begins. */
export function notifyBrowserLogout(): void {
  for (const listener of [...logoutListeners]) listener();
}

export const onBrowserLogout = subscribeToBrowserLogout;
