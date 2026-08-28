import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import AuthenticatedError from "@/app/(authenticated)/error";

describe("authenticated error recovery", () => {
  it("keeps retry and exposes a full-navigation sign-in escape hatch", () => {
    const markup = renderToStaticMarkup(<AuthenticatedError reset={() => undefined} />);

    expect(markup).toContain("Try again");
    expect(markup).toContain('<a class="secondary-action" href="/login">Sign in again</a>');
  });
});
