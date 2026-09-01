import type { z } from "zod";
import { isLoginRequiredError, parseApiResponse } from "./response";

export type BrowserResponseOptions = {
  /** Test seam; production callers use the browser's full navigation below. */
  readonly navigate?: (href: string) => void;
};

function redirectToLogin(options: BrowserResponseOptions): void {
  if (options.navigate !== undefined) {
    options.navigate("/login");
    return;
  }
  if (typeof window !== "undefined") {
    window.location.replace("/login");
  }
}

/**
 * Parse an API response in a client component.
 *
 * Session failures are handled at the browser boundary so a mutation or
 * polling request cannot strand the user on an authenticated error state.
 * Every other ApiProblem and contract failure keeps the caller's existing
 * error handling.
 */
export async function parseBrowserApiResponse<Output>(
  response: Response,
  schema: z.ZodType<Output>,
  options: BrowserResponseOptions = {},
): Promise<Output> {
  try {
    return await parseApiResponse(response, schema);
  } catch (error) {
    if (isLoginRequiredError(error)) {
      redirectToLogin(options);
    }
    throw error;
  }
}
