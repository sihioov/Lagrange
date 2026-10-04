const LOCK_NAME = "lagrange.csrf-mutation.v1";
let serverTail: Promise<void> = Promise.resolve();

/** Own the synchronizer-token rotation until its mutation has completed.
 * Web Locks share this boundary across same-origin tabs without sharing tokens.
 * The caller's deadline also covers waiting; cancellation never starts a request.
 */
export async function withCsrfMutationLock<T>(
  signal: AbortSignal,
  operation: () => Promise<T>,
): Promise<T> {
  signal.throwIfAborted();
  if (typeof window !== "undefined") {
    if (typeof navigator.locks?.request !== "function") {
      throw new Error("CSRF mutation serialization is unavailable");
    }
    return navigator.locks.request(LOCK_NAME, { mode: "exclusive", signal }, async () => {
      signal.throwIfAborted();
      return operation();
    });
  }

  // Node callers and isolated tests have no browser tabs or Web Lock manager.
  const previous = serverTail;
  let release: () => void = () => undefined;
  serverTail = new Promise<void>((resolve) => {
    release = resolve;
  });
  let onAbort: () => void = () => undefined;
  const aborted = new Promise<never>((_, reject) => {
    onAbort = () => reject(signal.reason);
    signal.addEventListener("abort", onAbort, { once: true });
  });
  try {
    await Promise.race([previous, aborted]);
    signal.throwIfAborted();
    return await operation();
  } finally {
    signal.removeEventListener("abort", onAbort);
    // A cancelled waiter must not allow a later caller to overtake the owner.
    void previous.then(release, release);
  }
}
