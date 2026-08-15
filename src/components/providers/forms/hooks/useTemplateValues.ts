import type { CodexProviderPreset } from "@/config/codexProviderPresets";
import type { TemplateValueConfig } from "@/utils/providerConfigUtils";

type TemplateValueMap = Record<string, TemplateValueConfig>;

interface PresetEntry {
  id: string;
  preset: CodexProviderPreset;
}

interface UseTemplateValuesProps {
  selectedPresetId: string | null;
  presetEntries: PresetEntry[];
  settingsConfig: string;
  onConfigChange: (config: string) => void;
}

/**
 * 模板变量仅属于已移除的 Claude 预设；Codex 预设不含 templateValues，
 * 此 hook 保留 API 形状但恒为空。
 */
export function useTemplateValues(
  _props: UseTemplateValuesProps,
): {
  templateValues: TemplateValueMap;
  templateValueEntries: Array<[string, TemplateValueConfig]>;
  selectedPreset: null;
  handleTemplateValueChange: (key: string, value: string) => void;
  validateTemplateValues: () => { isValid: true };
} {
  return {
    templateValues: {},
    templateValueEntries: [],
    selectedPreset: null,
    handleTemplateValueChange: () => undefined,
    validateTemplateValues: () => ({ isValid: true }),
  };
}
