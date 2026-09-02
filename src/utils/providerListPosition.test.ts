import { describe, expect, it } from "vitest";
import type { Provider } from "@/types";
import { preserveProviderListPosition } from "./providerListPosition";

const ordinaryProvider = (
  overrides: Partial<Provider> = {},
): Provider => ({
  id: "ordinary-1",
  name: "ccode",
  settingsConfig: { auth: {}, config: "" },
  sortIndex: 2,
  createdAt: 1_700_000_000_000,
  inFailoverQueue: true,
  ...overrides,
});

describe("preserveProviderListPosition", () => {
  it("keeps an ordinary supplier's list position when the edit payload omits it", () => {
    const existing = ordinaryProvider();
    const edited = ordinaryProvider({
      name: "ccode renamed",
      sortIndex: undefined,
      createdAt: undefined,
      inFailoverQueue: undefined,
    });

    expect(preserveProviderListPosition(existing, edited)).toEqual({
      ...edited,
      sortIndex: 2,
      createdAt: 1_700_000_000_000,
      inFailoverQueue: true,
    });
  });

  it("keeps an aggregate supplier's list position when the edit payload omits it", () => {
    const existing = ordinaryProvider({
      id: "agg-1",
      name: "Aggregate",
      meta: { providerType: "aggregate" },
    });
    const edited = {
      id: "agg-1",
      name: "Aggregate renamed",
      settingsConfig: existing.settingsConfig,
    } as Provider;

    expect(preserveProviderListPosition(existing, edited)).toMatchObject({
      sortIndex: 2,
      createdAt: 1_700_000_000_000,
    });
  });

  it("does not override an explicit new sortIndex", () => {
    const existing = ordinaryProvider({ sortIndex: 2 });
    const edited = ordinaryProvider({ sortIndex: 7 });
    expect(preserveProviderListPosition(existing, edited).sortIndex).toBe(7);
  });
});
