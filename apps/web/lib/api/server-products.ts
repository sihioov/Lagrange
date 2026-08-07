import "server-only";

import { createProductApiClient } from "./product-client";
import { serverApiClientOptions } from "./server-session";

export async function getProductApi() {
  return createProductApiClient(await serverApiClientOptions());
}
