import { useMemo } from "react";
import type { AppId } from "@/lib/api";
import type { CodexProviderPreset } from "@/config/codexProviderPresets";
import type { ProviderMeta, EndpointCandidate } from "@/types";
import { extractCodexBaseUrl } from "@/utils/providerConfigUtils";

type PresetEntry = {
  id: string;
  preset: CodexProviderPreset;
};

interface UseSpeedTestEndpointsProps {
  appId: AppId;
  selectedPresetId: string | null;
  presetEntries: PresetEntry[];
  baseUrl: string;
  codexBaseUrl: string;
  initialData?: {
    settingsConfig?: Record<string, unknown>;
    meta?: ProviderMeta;
  };
}

/**
 * 收集端点测速弹窗的初始端点列表
 *
 * 收集来源：
 * 1. 当前选中的 Base URL
 * 2. 编辑模式下的初始数据 URL
 * 3. 预设中的 endpointCandidates
 *
 * 注意：已保存的自定义端点通过 getCustomEndpoints API 在 EndpointSpeedTest 组件中加载，
 * 不在此处读取，避免重复导入。
 */
export function useSpeedTestEndpoints({
  appId,
  selectedPresetId,
  presetEntries,
  baseUrl: _baseUrl,
  codexBaseUrl,
  initialData,
}: UseSpeedTestEndpointsProps) {
  const codexEndpoints = useMemo<EndpointCandidate[]>(() => {
    if (appId !== "codex") return [];

    const map = new Map<string, EndpointCandidate>();
    // 候选端点标记为 isCustom: false，表示来自预设或配置
    // 已保存的自定义端点会在 EndpointSpeedTest 组件中通过 API 加载
    const add = (url?: string, isCustom = false) => {
      if (!url) return;
      const sanitized = url.trim().replace(/\/+$/, "");
      if (!sanitized || map.has(sanitized)) return;
      map.set(sanitized, { url: sanitized, isCustom });
    };

    // 1. 当前 Codex Base URL
    if (codexBaseUrl) {
      add(codexBaseUrl);
    }

    // 2. 编辑模式：初始数据中的 URL
    const initialCodexConfig = initialData?.settingsConfig as
      | {
          config?: string;
        }
      | undefined;
    const configStr = initialCodexConfig?.config ?? "";
    const extractedBaseUrl = extractCodexBaseUrl(configStr);
    if (extractedBaseUrl) {
      add(extractedBaseUrl);
    }

    // 3. 预设中的 endpointCandidates
    if (selectedPresetId && selectedPresetId !== "custom") {
      const entry = presetEntries.find((item) => item.id === selectedPresetId);
      if (entry) {
        const preset = entry.preset as CodexProviderPreset;
        // 添加预设自己的 baseUrl
        const presetConfig = preset.config || "";
        const presetBaseUrl = extractCodexBaseUrl(presetConfig);
        if (presetBaseUrl) {
          add(presetBaseUrl);
        }
        // 添加预设的候选端点
        if (preset.endpointCandidates) {
          preset.endpointCandidates.forEach((url) => add(url));
        }
      }
    }

    return Array.from(map.values());
  }, [appId, codexBaseUrl, initialData, selectedPresetId, presetEntries]);

  return codexEndpoints;
}
