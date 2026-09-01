import "server-only";

import { createProductApiClient } from "./product-client";
import { getServerSession, serverApiClientOptions } from "./server-session";

export async function getProductApi() {
  await getServerSession();
  return createProductApiClient(await serverApiClientOptions());
}
