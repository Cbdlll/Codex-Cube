import { describe, expect, it } from "vitest";
import {
  presetContextWindowForModel,
  resolveCodexContextWindow,
} from "@/utils/codexPresetContextWindows";

describe("codexPresetContextWindows", () => {
  it("fills OpenCode and official GPT ids the same way ordinary presets do", () => {
    expect(presetContextWindowForModel("glm-5.2")).toBe(204800);
    expect(presetContextWindowForModel("glm-5.2@opencode")).toBe(204800);
    expect(presetContextWindowForModel("openai/gpt-5.6-sol")).toBe(272000);
    expect(presetContextWindowForModel("deepseek-v4-flash")).toBe(1048576);
    expect(presetContextWindowForModel("glm-5.3")).toBe(1048576);
    expect(presetContextWindowForModel("glm-5.3[1m]")).toBe(1048576);
    expect(presetContextWindowForModel("minimax-m3")).toBe(1000000);
    expect(presetContextWindowForModel("MiniMax-M2.7")).toBe(204800);
    expect(presetContextWindowForModel("minimax-m2.5")).toBe(204800);
    expect(presetContextWindowForModel("qwen3.8-max")).toBe(1000000);
    expect(presetContextWindowForModel("kimi-k2.6")).toBe(262144);
    expect(presetContextWindowForModel("mimo-v2-omni")).toBe(262144);
  });

  it("keeps an explicit window and only fills when the field is empty", () => {
    expect(resolveCodexContextWindow("glm-5.2", 999)).toBe(999);
    expect(resolveCodexContextWindow("glm-5.2", "200000")).toBe(200000);
    expect(resolveCodexContextWindow("glm-5.2")).toBe(204800);
    expect(resolveCodexContextWindow("kimi-k2")).toBeUndefined();
  });
});
