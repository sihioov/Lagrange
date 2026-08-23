import type { ReactNode } from "react";
import { OwnerAccessRefusal, OwnerBetaPaperUnavailable } from "@/components/pages/owner-route";
import { type OwnerBetaProduct, permitsOwnerBetaProduct } from "@/lib/api/contracts";
import { getServerSession } from "@/lib/api/server-session";
import { getLocale } from "@/lib/i18n/server";

export type OwnerBetaProductRouteProps = {
  readonly product: OwnerBetaProduct;
  readonly renderProduct: () => ReactNode | Promise<ReactNode>;
  readonly title: string;
};

/**
 * Defense-in-depth boundary for the temporary Owner-only beta.
 *
 * `renderProduct` is deliberately lazy: a refused Member never constructs the
 * product page and cannot start recommendation, backtest, or Paper requests.
 * The API middleware remains authoritative for every direct request.
 */
export async function OwnerBetaProductRoute({
  product,
  renderProduct,
  title,
}: OwnerBetaProductRouteProps) {
  const [session, locale] = await Promise.all([getServerSession(), getLocale()]);
  if (!permitsOwnerBetaProduct(session, product)) {
    if (
      session.role === "owner" &&
      session.owner_beta_access_mode === "owner_only" &&
      product === "paper"
    ) {
      return <OwnerBetaPaperUnavailable locale={locale} title={title} />;
    }
    return <OwnerAccessRefusal locale={locale} title={title} />;
  }
  return renderProduct();
}
