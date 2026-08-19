import type {
  AggregateProviderModel,
  CodexApiFormat,
  CodexCatalogModel,
  Provider,
} from "@/types";
import {
  extractCodexBaseUrl,
  extractCodexExperimentalBearerToken,
  resolveCodexWireApi,
} from "@/utils/providerConfigUtils";

/** 聚合 Provider 在 `meta.providerType` 中的标识（与后端一致）。 */
export const AGGREGATE_PROVIDER_TYPE = "aggregate";

/** 聚合 Provider settingsConfig 中模型映射的键（与后端一致）。 */
export const AGGREGATE_MODELS_KEY = "aggregateModels";
/** 聚合 Provider settingsConfig 中成员 ID 列表的键（与后端一致）。 */
export const AGGREGATE_MEMBERS_KEY = "memberProviderIds";
/** 聚合 Provider settingsConfig 中默认模型（写入 config.toml 的 model）的键。 */
export const AGGREGATE_DEFAULT_MODEL_KEY = "defaultModel";

export type CodexMemberWireApi = "responses" | "chat" | "anthropic";

/** 聚合模型可携带的元数据（上下文窗口 / 能力声明），来自成员目录或实时获取。 */
export interface AggregateModelMeta {
  contextWindow?: number | string;
  supportsParallelToolCalls?: boolean;
  inputModalities?: string[];
  baseInstructions?: string;
}

/**
 * Cube 预设 / 官方 Codex 已知上下文窗口。聚合映射未声明 contextWindow 时
 * 用这些值，避免生成目录时回退到 128k（Desktop 再按 95% 显示成约 122k）。
 * 冲突 slug `model@provider` 按裸模型 id 查找。
 */
const KNOWN_CODEX_CONTEXT_WINDOWS: Record<string, number> = {
  "gpt-5.6-sol": 272000,
  "gpt-5.6-terra": 272000,
  "gpt-5.6-luna": 272000,
  "gpt-5.5": 272000,
  "gpt-5.4": 272000,
  "gpt-5.4-mini": 272000,
  "gpt-5.2": 272000,
  "kimi-k3": 1048576,
  "kimi-k2.7-code": 262144,
  "kimi-for-coding": 262144,
  "deepseek-v4-flash": 1048576,
  "deepseek-v4-pro": 1048576,
  "grok-4.5": 500000,
  "grok-4.6": 500000,
  "mimo-v2.5": 1048576,
  "mimo-v2.5-pro": 1048576,
};

/** 已知模型的上下文窗口；未收录时返回 undefined（由 catalog 生成回退 128k）。 */
export function knownCodexContextWindow(model: string): number | undefined {
  const id = model.trim().split("@")[0]?.trim() ?? "";
  return id ? KNOWN_CODEX_CONTEXT_WINDOWS[id] : undefined;
}

function normalizeCodexApiFormat(value: unknown): CodexApiFormat | undefined {
  const format = String(value ?? "")
    .trim()
    .toLowerCase();
  if (format === "openai_responses" || format === "responses") {
    return "openai_responses";
  }
  if (
    format === "openai_chat" ||
    format === "chat" ||
    format === "chat_completions" ||
    format === "completions"
  ) {
    return "openai_chat";
  }
  if (format === "anthropic" || format === "messages") {
    return "anthropic";
  }
  return undefined;
}

/** 从成员供应商 modelCatalog 条目提取可携带的模型元数据。 */
export function aggregateMetaFromCatalogEntry(
  entry: Record<string, any> | undefined | null,
): AggregateModelMeta | undefined {
  const contextWindow = entry?.contextWindow ?? entry?.context_window;
  const supportsParallelToolCalls =
    entry?.supportsParallelToolCalls ?? entry?.supports_parallel_tool_calls;
  const inputModalities = Array.isArray(entry?.inputModalities)
    ? entry.inputModalities
    : Array.isArray(entry?.input_modalities)
      ? entry.input_modalities
      : undefined;
  const baseInstructions = entry?.baseInstructions ?? entry?.base_instructions;
  if (
    contextWindow === undefined &&
    supportsParallelToolCalls === undefined &&
    !inputModalities?.length &&
    !baseInstructions?.trim()
  ) {
    return undefined;
  }
  return {
    ...(contextWindow !== undefined && contextWindow !== ""
      ? { contextWindow }
      : {}),
    ...(supportsParallelToolCalls !== undefined
      ? { supportsParallelToolCalls }
      : {}),
    ...(inputModalities && inputModalities.length > 0
      ? { inputModalities }
      : {}),
    ...(baseInstructions?.trim()
      ? { baseInstructions: baseInstructions.trim() }
      : {}),
  };
}

/** 把按 `providerId::upstreamModel` 索引的元数据合并进聚合模型快照（保留原有显示名）。 */
export function applyAggregateModelMeta(
  models: AggregateProviderModel[],
  metaByKey: Record<string, AggregateModelMeta>,
): AggregateProviderModel[] {
  return models.map((model) => {
    const meta =
      metaByKey[`${model.providerId}::${model.upstreamModel ?? model.model}`];
    if (!meta) return model;
    return {
      ...model,
      ...(meta.contextWindow !== undefined && meta.contextWindow !== ""
        ? { contextWindow: meta.contextWindow }
        : {}),
      ...(meta.supportsParallelToolCalls !== undefined
        ? { supportsParallelToolCalls: meta.supportsParallelToolCalls }
        : {}),
      ...(meta.inputModalities && meta.inputModalities.length > 0
        ? { inputModalities: meta.inputModalities }
        : {}),
      ...(meta.baseInstructions?.trim()
        ? { baseInstructions: meta.baseInstructions.trim() }
        : {}),
    };
  });
}

/**
 * 解析 Codex 供应商的规范 wire API（responses | chat | anthropic）：
 * 优先 meta.apiFormat / settings 的 api_format / apiFormat，其次 config.toml 的
 * wire_api，最后按完整 endpoint URL 推断；都无法判定时视为默认的 responses。
 * 复用 providerConfigUtils 的既有归一化规则，避免协议别名集合漂移。
 */
export function getCodexMemberWireApi(provider: Provider): CodexMemberWireApi {
  const config = provider.settingsConfig as Record<string, any>;
  const configText = typeof config.config === "string" ? config.config : "";
  const baseUrl =
    (typeof config.base_url === "string" && config.base_url.trim()
      ? config.base_url
      : typeof config.baseURL === "string" && config.baseURL.trim()
        ? config.baseURL
        : undefined) || extractCodexBaseUrl(configText);
  return resolveCodexWireApi({
    metaApiFormat: provider.meta?.apiFormat,
    settingsApiFormat: config.apiFormat,
    settingsApiFormatSnake: config.api_format,
    configText,
    baseUrl,
  });
}

export function getAggregateModelApiFormat(provider: Provider): CodexApiFormat {
  const wireApi = getCodexMemberWireApi(provider);
  if (wireApi === "chat") return "openai_chat";
  if (wireApi === "anthropic") return "anthropic";
  return "openai_responses";
}

/** 判断成员 Provider 的默认协议是否为 Responses。 */
export function isResponsesCodexMember(provider: Provider): boolean {
  return getCodexMemberWireApi(provider) === "responses";
}

/** 从 Codex 供应商配置提取模型拉取所需的凭据。 */
export function getCodexMemberCredentials(provider: Provider): {
  baseUrl?: string;
  apiKey?: string;
  isFullUrl?: boolean;
  customUserAgent?: string;
} {
  const config = provider.settingsConfig as Record<string, any>;
  const configText = typeof config.config === "string" ? config.config : "";
  const baseUrl =
    (typeof config.base_url === "string" && config.base_url.trim()
      ? config.base_url
      : typeof config.baseURL === "string" && config.baseURL.trim()
        ? config.baseURL
        : undefined) || extractCodexBaseUrl(configText);
  const authKey =
    typeof config.auth?.OPENAI_API_KEY === "string"
      ? config.auth.OPENAI_API_KEY
      : "";
  const apiKey =
    authKey || extractCodexExperimentalBearerToken(configText) || "";
  return {
    baseUrl,
    apiKey,
    isFullUrl: provider.meta?.isFullUrl,
    customUserAgent: provider.meta?.customUserAgent,
  };
}

/** 从已选成员与模型生成聚合模型映射（含同名冲突自动改名）。 */
export function buildAggregateModels(
  selected: { provider: Provider; models: string[] }[],
): AggregateProviderModel[] {
  const counts = new Map<string, number>();
  for (const { models } of selected) {
    for (const model of models) {
      counts.set(model, (counts.get(model) ?? 0) + 1);
    }
  }

  const usedSlugs = new Set<string>();
  const result: AggregateProviderModel[] = [];
  for (const { provider, models } of selected) {
    for (const model of models) {
      const collide = (counts.get(model) ?? 0) > 1;
      const baseSlug = collide
        ? `${model}@${slugifyProviderName(provider.name)}`
        : model;
      let slug = baseSlug;
      let suffix = 2;
      while (usedSlugs.has(slug)) {
        slug = `${baseSlug}-${suffix}`;
        suffix += 1;
      }
      usedSlugs.add(slug);
      result.push({
        model: slug,
        displayName: collide
          ? `${model} (${provider.name.trim() || provider.id})`
          : model,
        providerId: provider.id,
        upstreamModel: model,
      });
    }
  }
  return result;
}

/** 生成稳定、Codex 可接受的供应商标识（用于冲突 slug 后缀）。 */
export function slugifyProviderName(name: string): string {
  const slug = name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return slug || "provider";
}

/** 保存前归一化：trim、去重（按插槽名）、剔除无效条目。 */
export function normalizeAggregateModelsForSave(
  models: AggregateProviderModel[],
): AggregateProviderModel[] {
  const seen = new Set<string>();
  const result: AggregateProviderModel[] = [];
  for (const item of models) {
    const model = item.model.trim();
    const providerId = item.providerId.trim();
    const apiFormat = normalizeCodexApiFormat(item.apiFormat);
    if (!model || !providerId || seen.has(model)) {
      continue;
    }
    seen.add(model);
    result.push({
      model,
      providerId,
      ...(item.displayName?.trim()
        ? { displayName: item.displayName.trim() }
        : {}),
      ...(item.upstreamModel?.trim()
        ? { upstreamModel: item.upstreamModel.trim() }
        : {}),
      ...(apiFormat ? { apiFormat } : {}),
      ...(item.contextWindow !== undefined && item.contextWindow !== ""
        ? {
            contextWindow:
              typeof item.contextWindow === "string"
                ? Number(item.contextWindow)
                : item.contextWindow,
          }
        : {}),
      ...(item.supportsParallelToolCalls !== undefined
        ? { supportsParallelToolCalls: item.supportsParallelToolCalls }
        : {}),
      ...(item.inputModalities && item.inputModalities.length > 0
        ? { inputModalities: item.inputModalities }
        : {}),
      ...(item.baseInstructions?.trim()
        ? { baseInstructions: item.baseInstructions.trim() }
        : {}),
    });
  }
  return result;
}

/** 生成聚合 Provider 写入 Codex 的 modelCatalog（终端/桌面模型列表来源）。 */
export function buildAggregateModelCatalog(models: AggregateProviderModel[]): {
  models: CodexCatalogModel[];
} {
  return {
    models: models.map((item) => ({
      model: item.model,
      ...(item.displayName?.trim()
        ? { displayName: item.displayName.trim() }
        : {}),
      ...(item.contextWindow !== undefined && item.contextWindow !== ""
        ? { contextWindow: item.contextWindow }
        : {}),
      ...(item.supportsParallelToolCalls !== undefined
        ? { supportsParallelToolCalls: item.supportsParallelToolCalls }
        : {}),
      ...(item.inputModalities && item.inputModalities.length > 0
        ? { inputModalities: item.inputModalities }
        : {}),
      ...(item.baseInstructions?.trim()
        ? { baseInstructions: item.baseInstructions.trim() }
        : {}),
    })),
  };
}

/** 组装聚合 Provider 的 settingsConfig。 */
export function buildAggregateSettingsConfig(
  models: AggregateProviderModel[],
  memberProviderIds: string[],
  defaultModel = "",
): Record<string, unknown> {
  const resolvedDefault =
    defaultModel.trim() || models[0]?.model.trim() || "gpt-5.6-sol";
  const config = `model_provider = "custom"
model = ${JSON.stringify(resolvedDefault)}

[model_providers.custom]
name = "custom"
wire_api = "responses"
requires_openai_auth = true`;
  return {
    auth: {},
    config,
    [AGGREGATE_MEMBERS_KEY]: memberProviderIds,
    [AGGREGATE_MODELS_KEY]: models,
    [AGGREGATE_DEFAULT_MODEL_KEY]: resolvedDefault,
    modelCatalog: buildAggregateModelCatalog(models),
  };
}

/** 聚合 Provider 投影到 Codex 的 config.toml 预览（与后端接管投影一致）。 */
export function buildAggregateConfigTomlPreview(
  name: string,
  models: AggregateProviderModel[],
  defaultModel = "",
  proxyBaseUrl = "http://127.0.0.1:15721/v1",
): string {
  const resolvedDefault =
    defaultModel.trim() ||
    models
      .map((model) => model.model.trim())
      .find((model) => model.length > 0) || "gpt-5.6-sol";
  return [
    'model_provider = "custom"',
    `model = ${JSON.stringify(resolvedDefault)}`,
    "",
    "[model_providers.custom]",
    `name = ${JSON.stringify(name.trim() || "custom")}`,
    `base_url = ${JSON.stringify(proxyBaseUrl.replace(/\/+$/, ""))}`,
    'wire_api = "responses"',
    "requires_openai_auth = true",
    "",
  ].join("\n");
}

/** 从聚合 Provider 的 settingsConfig 解析成员与模型（编辑模式回填）。 */
export function parseAggregateSettings(settingsConfig: Record<string, any>): {
  memberProviderIds: string[];
  models: AggregateProviderModel[];
  defaultModel: string;
} {
  const memberProviderIds = Array.isArray(settingsConfig[AGGREGATE_MEMBERS_KEY])
    ? settingsConfig[AGGREGATE_MEMBERS_KEY].map((id: unknown) =>
        String(id).trim(),
      ).filter(Boolean)
    : [];
  const rawModels = Array.isArray(settingsConfig[AGGREGATE_MODELS_KEY])
    ? settingsConfig[AGGREGATE_MODELS_KEY]
    : [];
  const models: AggregateProviderModel[] = rawModels
    .map((item: any) => ({
      model: typeof item?.model === "string" ? item.model : "",
      providerId:
        typeof item?.providerId === "string"
          ? item.providerId
          : typeof item?.provider_id === "string"
            ? item.provider_id
            : "",
      displayName:
        typeof item?.displayName === "string"
          ? item.displayName
          : typeof item?.display_name === "string"
            ? item.display_name
            : undefined,
      upstreamModel:
        typeof item?.upstreamModel === "string"
          ? item.upstreamModel
          : typeof item?.upstream_model === "string"
            ? item.upstream_model
            : undefined,
      contextWindow:
        typeof item?.contextWindow === "string" ||
        typeof item?.contextWindow === "number"
          ? item.contextWindow
          : typeof item?.context_window === "string" ||
              typeof item?.context_window === "number"
            ? item.context_window
            : undefined,
      supportsParallelToolCalls:
        typeof item?.supportsParallelToolCalls === "boolean"
          ? item.supportsParallelToolCalls
          : typeof item?.supports_parallel_tool_calls === "boolean"
            ? item.supports_parallel_tool_calls
            : undefined,
      inputModalities: Array.isArray(item?.inputModalities)
        ? item.inputModalities
        : Array.isArray(item?.input_modalities)
          ? item.input_modalities
          : undefined,
      baseInstructions:
        typeof item?.baseInstructions === "string"
          ? item.baseInstructions
          : typeof item?.base_instructions === "string"
            ? item.base_instructions
            : undefined,
      apiFormat: normalizeCodexApiFormat(
        item?.apiFormat ?? item?.api_format ?? item?.wireApi ?? item?.wire_api,
      ),
    }))
    .filter(
      (item: AggregateProviderModel) =>
        item.model.trim() && item.providerId.trim(),
    );
  const defaultModel =
    typeof settingsConfig[AGGREGATE_DEFAULT_MODEL_KEY] === "string"
      ? settingsConfig[AGGREGATE_DEFAULT_MODEL_KEY].trim()
      : models[0]?.model.trim() ?? "";
  return { memberProviderIds, models, defaultModel };
}

/** 由名称生成稳定的聚合 Provider id（如 aggregate-deepseek-kimi）。 */
export function generateAggregateProviderId(name: string): string {
  const slug = name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 48);
  return `aggregate-${slug || "provider"}`;
}

/** 判断是否为聚合 Provider（前端能力谓词，与后端 Provider::is_aggregate 一致）。 */
export function isAggregateProvider(provider: Provider): boolean {
  return provider.meta?.providerType === AGGREGATE_PROVIDER_TYPE;
}
