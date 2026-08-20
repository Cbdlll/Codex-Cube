import { codexProviderPresets } from "@/config/codexProviderPresets";

/**
 * 官方 Codex 捆绑 GPT 槽位窗口（`codex debug models --bundled`）。
 * OpenAI Official 预设没有 modelCatalog，聚合 / OpenCode 选中这些 id 时仍要带上。
 */
const OFFICIAL_GPT_CONTEXT_WINDOW = 272_000;

const EXTRA_KNOWN_CONTEXT_WINDOWS: Record<string, number> = {
  "gpt-5.6-sol": OFFICIAL_GPT_CONTEXT_WINDOW,
  "gpt-5.6-terra": OFFICIAL_GPT_CONTEXT_WINDOW,
  "gpt-5.6-luna": OFFICIAL_GPT_CONTEXT_WINDOW,
  "gpt-5.5": OFFICIAL_GPT_CONTEXT_WINDOW,
  "gpt-5.4": OFFICIAL_GPT_CONTEXT_WINDOW,
  "gpt-5.4-mini": OFFICIAL_GPT_CONTEXT_WINDOW,
  "gpt-5.2": OFFICIAL_GPT_CONTEXT_WINDOW,
  "openai/gpt-5.6-sol": OFFICIAL_GPT_CONTEXT_WINDOW,
  "grok-4.6": 500_000,
};

function addContextWindow(
  map: Map<string, number>,
  model: string,
  window: number,
) {
  const key = model.trim();
  if (!key || !(window > 0)) return;
  const previous = map.get(key);
  if (previous === undefined || window > previous) {
    map.set(key, window);
  }
}

function indexModelAliases(map: Map<string, number>, model: string, window: number) {
  addContextWindow(map, model, window);
  const bare = model.trim().split("@")[0]?.trim() ?? "";
  if (bare) addContextWindow(map, bare, window);
  const slash = bare.split("/").pop()?.trim() ?? "";
  if (slash) addContextWindow(map, slash, window);
}

function buildPresetContextWindowMap(): Map<string, number> {
  const map = new Map<string, number>();
  for (const [model, window] of Object.entries(EXTRA_KNOWN_CONTEXT_WINDOWS)) {
    indexModelAliases(map, model, window);
  }
  for (const preset of codexProviderPresets) {
    for (const entry of preset.modelCatalog ?? []) {
      const model = typeof entry?.model === "string" ? entry.model : "";
      const window =
        typeof entry?.contextWindow === "number"
          ? entry.contextWindow
          : typeof entry?.contextWindow === "string"
            ? Number(entry.contextWindow)
            : Number.NaN;
      if (!model || !Number.isFinite(window) || window <= 0) continue;
      indexModelAliases(map, model, window);
    }
  }
  return map;
}

const PRESET_CONTEXT_WINDOWS = buildPresetContextWindowMap();

function lookupKeys(model: string): string[] {
  const id = model.trim();
  if (!id) return [];
  const bare = id.split("@")[0]?.trim() || id;
  const slash = bare.split("/").pop()?.trim() || bare;
  return Array.from(new Set([id, bare, slash].filter(Boolean)));
}

/**
 * 从 Cube 内置供应商预设（及官方 GPT 槽位）查找模型上下文窗口。
 * `/models` 多数不返回 context；聚合 / OpenCode 选中后用这份表自动填。
 * 冲突 slug `model@provider`、聚合前缀 `vendor/model` 按裸 id 回退。
 * 同一 id 在多个预设里不一致时取较大值，避免把窗口写小。
 */
export function presetContextWindowForModel(model: string): number | undefined {
  for (const key of lookupKeys(model)) {
    const window = PRESET_CONTEXT_WINDOWS.get(key);
    if (window && window > 0) return window;
  }
  return undefined;
}

/** 把拉取结果或用户输入整理成正整数窗口；无效则试预设表。 */
export function resolveCodexContextWindow(
  model: string,
  declared?: number | string | null,
): number | undefined {
  if (typeof declared === "number" && Number.isFinite(declared) && declared > 0) {
    return declared;
  }
  if (typeof declared === "string") {
    const parsed = Number(declared.replace(/[^\d]/g, ""));
    if (Number.isFinite(parsed) && parsed > 0) return parsed;
  }
  return presetContextWindowForModel(model);
}
