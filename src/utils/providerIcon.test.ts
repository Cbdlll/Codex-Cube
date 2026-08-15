import { describe, expect, it } from "vitest";
import { resolveProviderIcon } from "./providerIcon";

describe("resolveProviderIcon", () => {
  it("preserves a provider icon", () => {
    expect(resolveProviderIcon("codex", "openai", "")).toBe("openai");
  });

  it("does not reinterpret another app's provider icon", () => {
    expect(resolveProviderIcon("codex", "grok", "")).toBe("grok");
  });

  it("normalizes an empty icon to the initials fallback", () => {
    expect(resolveProviderIcon("codex", "  ", "")).toBeUndefined();
  });
});
