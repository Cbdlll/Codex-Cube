import { describe, it, expect } from "vitest";
import type { Provider } from "@/types";
import {
  providerNeedsRouting,
  providerShowsRoutingBadge,
} from "@/utils/providerCapabilities";

function mkProvider(overrides: Partial<Provider> = {}): Provider {
  return { id: "p1", name: "Test", settingsConfig: {}, ...overrides };
}

// wire_api 取自 config.toml；chat_completions 需转换（需路由），responses 直连。
const codexConfig = (wireApi: "chat_completions" | "responses") =>
  `model_provider = "custom"\n\n[model_providers.custom]\nname = "X"\nbase_url = "https://x.example/v1"\nwire_api = "${wireApi}"\n`;

describe("providerNeedsRouting", () => {
  it("官方供应商一律不需要路由（即便 providerType 是 OAuth）", () => {
    expect(
      providerNeedsRouting(
        "codex",
        mkProvider({
          category: "official",
          meta: { providerType: "xai_oauth" },
        }),
      ),
    ).toBe(false);
  });

  it("Codex 下 xai_oauth 需要路由（原生 Responses 也要注入 token）", () => {
    expect(
      providerNeedsRouting(
        "codex",
        mkProvider({
          meta: { providerType: "xai_oauth", apiFormat: "openai_responses" },
        }),
      ),
    ).toBe(true);
  });

  describe("Codex 非官方 Provider 仅真实转换/认证场景需要路由", () => {
    it("原生 Responses 独立订阅直连不需要路由", () => {
      expect(
        providerNeedsRouting(
          "codex",
          mkProvider({ meta: { apiFormat: "openai_responses" } }),
        ),
      ).toBe(false);
    });

    it("Chat 格式需要路由", () => {
      expect(
        providerNeedsRouting(
          "codex",
          mkProvider({ meta: { apiFormat: "openai_chat" } }),
        ),
      ).toBe(true);
    });

    it("Anthropic 格式需要路由", () => {
      expect(
        providerNeedsRouting(
          "codex",
          mkProvider({ meta: { apiFormat: "anthropic" } }),
        ),
      ).toBe(true);
    });

    it("config 里 wire_api=chat_completions 需要路由", () => {
      expect(
        providerNeedsRouting(
          "codex",
          mkProvider({
            settingsConfig: { config: codexConfig("chat_completions") },
          }),
        ),
      ).toBe(true);
    });

    it("config 里 wire_api=responses 原生直连不需要路由", () => {
      expect(
        providerNeedsRouting(
          "codex",
          mkProvider({ settingsConfig: { config: codexConfig("responses") } }),
        ),
      ).toBe(false);
    });

    it("旧数据缺少 apiFormat 且无转换标志不需要路由", () => {
      expect(providerNeedsRouting("codex", mkProvider())).toBe(false);
    });

    it("聚合 Provider 始终需要路由", () => {
      expect(
        providerNeedsRouting(
          "codex",
          mkProvider({
            settingsConfig: { aggregateModels: [] },
            meta: { providerType: "aggregate" },
          }),
        ),
      ).toBe(true);
    });
  });
});

describe("providerShowsRoutingBadge：Codex 徽标只在真正需要转换/认证/聚合时显示", () => {
  it("原生 Responses 独立订阅不显示徽标", () => {
    expect(
      providerShowsRoutingBadge(
        "codex",
        mkProvider({ meta: { apiFormat: "openai_responses" } }),
      ),
    ).toBe(false);
  });

  it("Chat / Anthropic 格式显示徽标", () => {
    expect(
      providerShowsRoutingBadge(
        "codex",
        mkProvider({ meta: { apiFormat: "openai_chat" } }),
      ),
    ).toBe(true);
    expect(
      providerShowsRoutingBadge(
        "codex",
        mkProvider({ meta: { apiFormat: "anthropic" } }),
      ),
    ).toBe(true);
  });

  it("聚合 Provider 显示徽标", () => {
    expect(
      providerShowsRoutingBadge(
        "codex",
        mkProvider({
          settingsConfig: { aggregateModels: [] },
          meta: { providerType: "aggregate" },
        }),
      ),
    ).toBe(true);
  });

  it("托管 OAuth 显示徽标（即便原生 Responses）", () => {
    expect(
      providerShowsRoutingBadge(
        "codex",
        mkProvider({
          meta: { providerType: "xai_oauth", apiFormat: "openai_responses" },
        }),
      ),
    ).toBe(true);
  });

  it("官方供应商不显示徽标", () => {
    expect(
      providerShowsRoutingBadge(
        "codex",
        mkProvider({ category: "official" }),
      ),
    ).toBe(false);
  });
});
