import { describe, expect, it } from "vitest";
import type { Provider } from "@/types";
import {
  aggregateMetaFromCatalogEntry,
  applyAggregateModelMeta,
  buildAggregateConfigTomlPreview,
  buildAggregateModelCatalog,
  buildAggregateModels,
  buildAggregateSettingsConfig,
  extractCodexReasoningEffortFromConfig,
  generateAggregateProviderId,
  getAggregateModelApiFormat,
  getCodexMemberWireApi,
  hydrateAggregateConfigToml,
  isAggregateProvider,
  isResponsesCodexMember,
  knownCodexContextWindow,
  normalizeAggregateModelsForSave,
  parseAggregateSettings,
  setCodexReasoningEffortInConfig,
} from "@/utils/aggregateProvider";
import { extractCodexModelName } from "@/utils/providerConfigUtils";

function makeProvider(
  id: string,
  name: string,
  settingsConfig: Record<string, any>,
  meta?: Provider["meta"],
): Provider {
  return {
    id,
    name,
    settingsConfig,
    meta,
  } as Provider;
}

describe("aggregateProvider", () => {
  it("detects wire api from meta apiFormat / config wire_api / base url", () => {
    const chat = makeProvider("a", "A", { config: 'wire_api = "chat"' });
    expect(getCodexMemberWireApi(chat)).toBe("chat");
    expect(isResponsesCodexMember(chat)).toBe(false);

    const anthropic = makeProvider(
      "b",
      "B",
      { config: 'wire_api = "responses"' },
      { apiFormat: "anthropic" },
    );
    expect(getCodexMemberWireApi(anthropic)).toBe("anthropic");

    const responses = makeProvider("c", "C", {
      config: 'base_url = "https://api.example.com/v1"\nwire_api = "responses"',
    });
    expect(getCodexMemberWireApi(responses)).toBe("responses");
    expect(isResponsesCodexMember(responses)).toBe(true);
  });

  it("normalizes every alias to canonical wire api (meta/settings/TOML/URL)", () => {
    expect(
      getCodexMemberWireApi(
        makeProvider("a", "A", {}, { apiFormat: "openai_chat" }),
      ),
    ).toBe("chat");
    expect(
      getCodexMemberWireApi(makeProvider("b", "B", { apiFormat: "chat" })),
    ).toBe("chat");
    expect(
      getCodexMemberWireApi(
        makeProvider("c", "C", { api_format: "chat_completions" }),
      ),
    ).toBe("chat");
    expect(
      getCodexMemberWireApi(
        makeProvider("d", "D", { config: 'wire_api = "openai_chat"' }),
      ),
    ).toBe("chat");
    expect(
      getCodexMemberWireApi(
        makeProvider("e", "E", { config: 'wire_api = "anthropic_messages"' }),
      ),
    ).toBe("anthropic");
    expect(
      getCodexMemberWireApi(
        makeProvider("f", "F", { config: 'wire_api = "openai_responses"' }),
      ),
    ).toBe("responses");
    expect(
      getCodexMemberWireApi(
        makeProvider("g", "G", {
          base_url: "https://gateway.example.com/chat/completions",
        }),
      ),
    ).toBe("chat");
    expect(
      getCodexMemberWireApi(
        makeProvider("h", "H", {
          config:
            'base_url = "https://anthropic.example.com/v1/messages"\nwire_api = "responses"',
        }),
      ),
    ).toBe("responses");
    expect(
      getCodexMemberWireApi(
        makeProvider("i", "I", {
          config: 'base_url = "https://responses.example.com/v1/responses"',
        }),
      ),
    ).toBe("responses");
  });

  it("infers chat from a chat/completions URL when no explicit format exists", () => {
    const chatByUrl = makeProvider("j", "J", {
      base_url: "https://api.example.com/v1/chat/completions",
    });
    expect(getCodexMemberWireApi(chatByUrl)).toBe("chat");
    expect(getAggregateModelApiFormat(chatByUrl)).toBe("openai_chat");
    expect(isResponsesCodexMember(chatByUrl)).toBe(false);
  });

  it("inherits the member Provider protocol for every model", () => {
    const responsesProvider = makeProvider("go", "OpenCode Go", {
      config:
        'base_url = "https://opencode.ai/zen/go/v1"\nwire_api = "responses"',
      modelCatalog: {
        models: [
          {
            model: "deepseek-v4-pro",
            apiFormat: "openai_chat",
          },
        ],
      },
    });
    const chatProvider = makeProvider("chat", "Chat", {
      config: 'base_url = "https://chat.example/v1"\nwire_api = "chat"',
    });

    expect(getAggregateModelApiFormat(responsesProvider)).toBe(
      "openai_responses",
    );
    expect(getAggregateModelApiFormat(chatProvider)).toBe("openai_chat");
  });

  it("renames colliding models with provider names and keeps unique slugs", () => {
    const deepseek = makeProvider("deepseek", "DeepSeek", {});
    const kimi = makeProvider("kimi", "Kimi", {});
    const models = buildAggregateModels([
      {
        provider: deepseek,
        models: ["deepseek-chat", "kimi-k2", "deepseek-reasoner"],
      },
      { provider: kimi, models: ["deepseek-chat", "kimi-k2"] },
    ]);

    expect(models).toHaveLength(5);
    const deepseekChat = models.find(
      (m) => m.providerId === "deepseek" && m.upstreamModel === "deepseek-chat",
    );
    const kimiChat = models.find(
      (m) => m.providerId === "kimi" && m.upstreamModel === "deepseek-chat",
    );
    expect(deepseekChat?.model).toBe("deepseek-chat@deepseek");
    expect(deepseekChat?.displayName).toBe("deepseek-chat (DeepSeek)");
    expect(kimiChat?.model).toBe("deepseek-chat@kimi");
    expect(kimiChat?.displayName).toBe("deepseek-chat (Kimi)");

    // 唯一模型保持原名
    const unique = models.find((m) => m.upstreamModel === "deepseek-reasoner");
    expect(unique?.model).toBe("deepseek-reasoner");
    expect(unique?.displayName).toBe("deepseek-reasoner");
  });

  it("normalizes models for save (trim/dedupe/invalid drop)", () => {
    const normalized = normalizeAggregateModelsForSave([
      { model: "  a  ", providerId: "p1" },
      { model: "a", providerId: "p2" }, // duplicate slug → dropped
      { model: "  ", providerId: "p3" }, // invalid → dropped
      {
        model: "b",
        providerId: "p2",
        displayName: " B ",
        contextWindow: "128000",
        apiFormat: "chat" as any,
      },
    ]);
    expect(normalized).toHaveLength(2);
    expect(normalized[0]).toEqual({ model: "a", providerId: "p1" });
    expect(normalized[1]).toEqual({
      model: "b",
      providerId: "p2",
      displayName: "B",
      contextWindow: 128000,
      apiFormat: "openai_chat",
    });
  });

  it("builds catalog and settings config consistently", () => {
    const models = [
      {
        model: "deepseek-chat",
        providerId: "deepseek",
        displayName: "DeepSeek Chat",
        contextWindow: 128000,
      },
    ];
    const settings = buildAggregateSettingsConfig(models, ["deepseek"]);
    expect(settings.memberProviderIds).toEqual(["deepseek"]);
    expect(settings.aggregateModels).toEqual(models);
    expect(settings.defaultModel).toBe("deepseek-chat");
    expect(settings.defaultReasoningEffort).toBe("high");
    expect(settings.modelCatalog).toEqual(buildAggregateModelCatalog(models));
    expect(settings.config).toContain('model_provider = "custom"');
    expect(settings.config).toContain("[model_providers.custom]");
    expect(settings.config).toContain('model = "deepseek-chat"');
    expect(settings.config).toContain('model_reasoning_effort = "high"');
    expect(
      (settings.modelCatalog as { models: unknown[] }).models[0],
    ).toMatchObject({
      model: "deepseek-chat",
      displayName: "DeepSeek Chat",
      contextWindow: 128000,
    });
  });

  it("keeps extra config.toml keys and auth when the user edited them", () => {
    const models = [
      {
        model: "deepseek-chat",
        providerId: "deepseek",
      },
    ];
    const settings = buildAggregateSettingsConfig(
      models,
      ["deepseek"],
      "deepseek-chat",
      "max",
      {
        auth: { OPENAI_API_KEY: "sk-test" },
        config: `model_provider = "custom"
model = "old-model"
model_reasoning_effort = "low"

[model_providers.custom]
name = "custom"
wire_api = "responses"

[desktop]
localeOverride = "zh-CN"
`,
      },
    );
    expect(settings.auth).toEqual({ OPENAI_API_KEY: "sk-test" });
    expect(settings.config).toContain('model = "deepseek-chat"');
    expect(settings.config).toContain('model_reasoning_effort = "max"');
    expect(settings.config).toContain('localeOverride = "zh-CN"');
    expect(extractCodexReasoningEffortFromConfig(String(settings.config))).toBe(
      "max",
    );
  });

  it("injects default model_reasoning_effort into an existing config.toml that lacks it", () => {
    const settings = buildAggregateSettingsConfig(
      [{ model: "deepseek-chat", providerId: "deepseek" }],
      ["deepseek"],
      "deepseek-chat",
      "",
      {
        config: `model_provider = "custom"
model = "deepseek-chat"

[model_providers.custom]
name = "custom"
wire_api = "responses"
`,
      },
    );
    expect(settings.defaultReasoningEffort).toBe("high");
    expect(settings.config).toContain('model_reasoning_effort = "high"');
    expect(extractCodexReasoningEffortFromConfig(String(settings.config))).toBe(
      "high",
    );
  });

  it("honors an explicit default model in settings and preview", () => {
    const models = [
      {
        model: "deepseek-chat",
        providerId: "deepseek",
        displayName: "DeepSeek Chat",
      },
      {
        model: "kimi-k2",
        providerId: "kimi",
        displayName: "Kimi K2",
      },
    ];
    const settings = buildAggregateSettingsConfig(models, ["deepseek", "kimi"], "kimi-k2");
    expect(settings.defaultModel).toBe("kimi-k2");
    expect(settings.config).toContain('model = "kimi-k2"');
    expect(
      buildAggregateConfigTomlPreview("agg", models, "kimi-k2"),
    ).toContain('model = "kimi-k2"');
    expect(
      buildAggregateConfigTomlPreview("agg", models, "kimi-k2"),
    ).toContain('model_reasoning_effort = "high"');
    // 未传显式默认模型时回退到首项。
    expect(buildAggregateConfigTomlPreview("agg", models)).toContain(
      'model = "deepseek-chat"',
    );
  });

  it("honors an explicit default reasoning effort in settings and preview", () => {
    const models = [
      {
        model: "deepseek-chat",
        providerId: "deepseek",
        displayName: "DeepSeek Chat",
      },
    ];
    const settings = buildAggregateSettingsConfig(
      models,
      ["deepseek"],
      "deepseek-chat",
      "max",
    );
    expect(settings.defaultReasoningEffort).toBe("max");
    expect(settings.config).toContain('model_reasoning_effort = "max"');
    expect(
      buildAggregateConfigTomlPreview(
        "agg",
        models,
        "deepseek-chat",
        undefined,
        "xhigh",
      ),
    ).toContain('model_reasoning_effort = "xhigh"');
    expect(
      buildAggregateSettingsConfig(models, ["deepseek"], "", "not-a-level")
        .defaultReasoningEffort,
    ).toBe("high");
  });

  it("parses stored settings for edit mode (camelCase + snake_case)", () => {
    const parsed = parseAggregateSettings({
      memberProviderIds: ["deepseek", "kimi"],
      defaultModel: "kimi-k2",
      aggregateModels: [
        {
          model: "deepseek-chat@deepseek",
          providerId: "deepseek",
          upstream_model: "deepseek-chat",
          api_format: "chat",
        },
        { model: "kimi-k2", provider_id: "kimi" },
      ],
    });
    expect(parsed.memberProviderIds).toEqual(["deepseek", "kimi"]);
    expect(parsed.defaultModel).toBe("kimi-k2");
    expect(parsed.defaultReasoningEffort).toBe("high");
    expect(parsed.models).toHaveLength(2);
    expect(parsed.models[0].upstreamModel).toBe("deepseek-chat");
    expect(parsed.models[0].apiFormat).toBe("openai_chat");
    expect(parsed.models[1].providerId).toBe("kimi");
  });

  it("hydrates missing model_reasoning_effort into the provider config.toml", () => {
    const hydrated = hydrateAggregateConfigToml(
      `model_provider = "custom"
model = "gpt-5.6-sol"

[model_providers.custom]
name = "custom"
wire_api = "responses"
`,
      "gpt-5.6-sol",
      "low",
    );
    expect(hydrated).toContain('model = "gpt-5.6-sol"');
    expect(hydrated).toContain('model_reasoning_effort = "low"');
    expect(extractCodexReasoningEffortFromConfig(hydrated)).toBe("low");
    expect(hydrated.match(/^\s*model_reasoning_effort\s*=/gm)).toHaveLength(1);
  });

  it("collapses duplicate model_reasoning_effort keys to a single assignment", () => {
    const input = `model_provider = "custom"
model = "gpt-5.6-sol"
model_reasoning_effort = "high"
model_reasoning_effort = "low"
model_reasoning_effort = "low"

[model_providers.custom]
name = "custom"
wire_api = "responses"

[agents]
default_subagent_reasoning_effort = "max"
`;
    const result = setCodexReasoningEffortInConfig(input, "max");
    expect(result.match(/^\s*model_reasoning_effort\s*=/gm)).toHaveLength(1);
    expect(extractCodexReasoningEffortFromConfig(result)).toBe("max");
    expect(result).toContain('model = "gpt-5.6-sol"');
    expect(result).toContain('default_subagent_reasoning_effort = "max"');

    const saved = buildAggregateSettingsConfig(
      [{ model: "gpt-5.6-sol", providerId: "opencode" }],
      ["opencode"],
      "gpt-5.6-sol",
      "high",
      { config: input },
    );
    expect(String(saved.config).match(/^\s*model_reasoning_effort\s*=/gm)).toHaveLength(
      1,
    );
    expect(extractCodexReasoningEffortFromConfig(String(saved.config))).toBe(
      "high",
    );
  });

  it("parses default reasoning effort from settings key or config.toml", () => {
    expect(
      parseAggregateSettings({
        defaultReasoningEffort: "ultra",
        aggregateModels: [{ model: "kimi-k3", providerId: "kimi" }],
      }).defaultReasoningEffort,
    ).toBe("ultra");
    expect(
      parseAggregateSettings({
        default_reasoning_effort: "low",
        aggregateModels: [{ model: "kimi-k3", providerId: "kimi" }],
      }).defaultReasoningEffort,
    ).toBe("low");
    expect(
      parseAggregateSettings({
        config: 'model = "kimi-k3"\nmodel_reasoning_effort = "max"\n',
        aggregateModels: [{ model: "kimi-k3", providerId: "kimi" }],
      }).defaultReasoningEffort,
    ).toBe("max");
    expect(
      parseAggregateSettings({
        config: 'model = "deepseek-v4-flash"\nmodel_reasoning_effort = "low"\n',
        aggregateModels: [
          { model: "gpt-5.6-sol", providerId: "opencode" },
          { model: "deepseek-v4-flash", providerId: "opencode" },
        ],
      }).defaultModel,
    ).toBe("deepseek-v4-flash");
  });

  it("overwrites stale extras.config with the form default model and reasoning", () => {
    const models = [
      { model: "gpt-5.6-sol", providerId: "opencode" },
      { model: "deepseek-v4-flash", providerId: "opencode" },
    ];
    const saved = buildAggregateSettingsConfig(
      models,
      ["opencode"],
      "deepseek-v4-flash",
      "low",
      {
        config: `model_provider = "custom"
model = "gpt-5.6-sol"
model_reasoning_effort = "high"
`,
      },
    );
    expect(saved.defaultModel).toBe("deepseek-v4-flash");
    expect(saved.defaultReasoningEffort).toBe("low");
    expect(extractCodexModelName(String(saved.config))).toBe(
      "deepseek-v4-flash",
    );
    expect(extractCodexReasoningEffortFromConfig(String(saved.config))).toBe(
      "low",
    );
  });

  it("generates stable aggregate provider ids", () => {
    expect(generateAggregateProviderId("DeepSeek + Kimi")).toBe(
      "aggregate-deepseek-kimi",
    );
    expect(generateAggregateProviderId("!!!")).toBe("aggregate-provider");
  });

  it("extracts model metadata from catalog entries (camelCase + snake_case)", () => {
    expect(
      aggregateMetaFromCatalogEntry({
        model: "deepseek-v4-flash",
        context_window: 1048576,
        supports_parallel_tool_calls: true,
        input_modalities: ["text", "image"],
        wire_api: "responses",
      }),
    ).toEqual({
      contextWindow: 1048576,
      supportsParallelToolCalls: true,
      inputModalities: ["text", "image"],
    });
    expect(
      aggregateMetaFromCatalogEntry({
        model: "x",
        contextWindow: 131072,
        baseInstructions: "  system  ",
      }),
    ).toEqual({
      contextWindow: 131072,
      baseInstructions: "system",
    });
    expect(aggregateMetaFromCatalogEntry({ model: "x" })).toBeUndefined();
  });

  it("applies model metadata to aggregate snapshot while preserving displayName", () => {
    const applied = applyAggregateModelMeta(
      [
        {
          model: "deepseek-v4-flash",
          displayName: "DeepSeek V4 Flash",
          providerId: "deepseek",
          upstreamModel: "deepseek-v4-flash",
        },
        {
          model: "kimi-k2",
          providerId: "kimi",
          upstreamModel: "kimi-k2",
        },
      ],
      {
        "deepseek::deepseek-v4-flash": {
          contextWindow: 1048576,
          supportsParallelToolCalls: true,
        },
        "kimi::kimi-k2": { contextWindow: 131072 },
      },
    );
    expect(applied[0]).toMatchObject({
      model: "deepseek-v4-flash",
      displayName: "DeepSeek V4 Flash",
      contextWindow: 1048576,
      supportsParallelToolCalls: true,
    });
    expect(applied[1]).toMatchObject({
      model: "kimi-k2",
      contextWindow: 131072,
    });
  });

  it("knows Cube preset context windows and strips collision suffixes", () => {
    expect(knownCodexContextWindow("kimi-k3")).toBe(1048576);
    expect(knownCodexContextWindow("kimi-k3@kimi")).toBe(1048576);
    expect(knownCodexContextWindow("kimi-k2")).toBeUndefined();
  });

  it("detects aggregate providers by meta.providerType", () => {
    const agg = makeProvider("a", "A", {}, { providerType: "aggregate" });
    expect(isAggregateProvider(agg)).toBe(true);
    const plain = makeProvider("b", "B", {});
    expect(isAggregateProvider(plain)).toBe(false);
  });
});
