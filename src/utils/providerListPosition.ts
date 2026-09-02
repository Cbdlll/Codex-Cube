import type { Provider } from "@/types";

type ProviderListPosition = Pick<
  Provider,
  "sortIndex" | "createdAt" | "inFailoverQueue"
>;

/**
 * Keep list order fields when an edit payload omits them.
 *
 * Ordinary and aggregate edit forms have both dropped `sortIndex` /
 * `createdAt` before; writing those as missing moves the card to the end.
 */
export const preserveProviderListPosition = (
  existing: ProviderListPosition | null | undefined,
  next: Provider,
): Provider => ({
  ...next,
  sortIndex: next.sortIndex ?? existing?.sortIndex,
  createdAt: next.createdAt ?? existing?.createdAt,
  inFailoverQueue: next.inFailoverQueue ?? existing?.inFailoverQueue,
});
