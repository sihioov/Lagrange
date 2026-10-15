import type {
  OwnerBetaRunListItemModel,
  OwnerBetaRunPageModel,
} from "@/lib/products/owner-beta-contracts";

export type OwnerBetaHistoryQuery = {
  readonly cursor?: string;
  readonly limit?: number;
};

type ReadHistory = (query?: OwnerBetaHistoryQuery) => Promise<OwnerBetaRunPageModel>;

export function ownerBetaHistoryQueryString(query?: OwnerBetaHistoryQuery): string {
  if (query === undefined) {
    return "";
  }
  const params = new URLSearchParams();
  if (query.cursor !== undefined) {
    if (query.cursor.length === 0 || query.cursor.length > 4096) {
      throw new Error("Invalid history cursor.");
    }
    params.set("cursor", query.cursor);
  }
  if (query.limit !== undefined) {
    if (!Number.isInteger(query.limit) || query.limit < 1 || query.limit > 100) {
      throw new Error("Invalid history limit.");
    }
    params.set("limit", String(query.limit));
  }
  const encoded = params.toString();
  return encoded === "" ? "" : `?${encoded}`;
}

/** Bounded search; an unfinished window is never proof that no result exists. */
export async function retainOwnerBetaHistory(
  read: ReadHistory,
  first: OwnerBetaRunPageModel,
  configId: string | undefined,
  initialCursor?: string,
) {
  const runs: OwnerBetaRunListItemModel[] = [];
  const seenRuns = new Set<string>();
  const seenCursors = new Set(initialCursor === undefined ? [] : [initialCursor]);
  let page = first;
  let pages = 0;
  for (;;) {
    pages += 1;
    if (
      page.has_more !== (page.next_cursor !== null) ||
      (page.has_more && page.items.length === 0) ||
      (page.next_cursor !== null &&
        (page.next_cursor.length === 0 || page.next_cursor.length > 4096))
    ) {
      throw new Error("Inconsistent history pagination.");
    }
    if (page.next_cursor !== null && seenCursors.has(page.next_cursor)) {
      throw new Error("Repeated history cursor.");
    }
    for (const run of page.items) {
      if (seenRuns.has(run.id)) {
        throw new Error("Repeated run in history.");
      }
      seenRuns.add(run.id);
      if (configId === undefined || run.strategy_config_id === configId) {
        runs.push(run);
      }
    }
    if (
      !page.has_more ||
      pages >= 6 ||
      runs.some((run) => run.status === "SUCCEEDED") ||
      configId === undefined
    ) {
      return {
        hasMore: page.has_more,
        nextCursor: page.next_cursor,
        previousResultSearchIncomplete:
          page.has_more && !runs.some((run) => run.status === "SUCCEEDED"),
        runs,
      };
    }
    const cursor = page.next_cursor;
    if (cursor === null || seenCursors.has(cursor)) {
      throw new Error("Repeated history cursor.");
    }
    seenCursors.add(cursor);
    page = await read({ cursor, limit: 100 });
  }
}
