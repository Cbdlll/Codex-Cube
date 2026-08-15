import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import JsonEditor from "@/components/JsonEditor";
import {
  isCodexGoalModeEnabled,
  isCodexRemoteCompactionEnabled,
  setCodexGoalMode,
  setCodexRemoteCompaction,
} from "@/utils/providerConfigUtils";

interface CodexAuthSectionProps {
  value: string;
  onChange: (value: string) => void;
  onBlur?: () => void;
  error?: string;
  isProxyTakeover?: boolean;
}

/**
 * CodexAuthSection - Auth JSON editor section
 */
export const CodexAuthSection: React.FC<CodexAuthSectionProps> = ({
  value,
  onChange,
  onBlur,
  error,
  isProxyTakeover = false,
}) => {
  const { t } = useTranslation();
  const [isDarkMode, setIsDarkMode] = useState(false);

  useEffect(() => {
    setIsDarkMode(document.documentElement.classList.contains("dark"));

    const observer = new MutationObserver(() => {
      setIsDarkMode(document.documentElement.classList.contains("dark"));
    });

    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class"],
    });

    return () => observer.disconnect();
  }, []);

  const handleChange = (newValue: string) => {
    onChange(newValue);
    if (onBlur) {
      onBlur();
    }
  };

  return (
    <div className="space-y-2">
      <label
        htmlFor="codexAuth"
        className="block text-sm font-medium text-foreground"
      >
        {t("codexConfig.authJson")}
      </label>

      <JsonEditor
        value={value}
        onChange={handleChange}
        placeholder={t("codexConfig.authJsonPlaceholder")}
        darkMode={isDarkMode}
        rows={6}
        showValidation={true}
        language="json"
      />

      {error && (
        <p className="text-xs text-red-500 dark:text-red-400">{error}</p>
      )}

      {!error && (
        <p className="text-xs text-muted-foreground">
          {t(
            isProxyTakeover
              ? "codexConfig.authJsonStorageHint"
              : "codexConfig.authJsonHint",
          )}
        </p>
      )}
    </div>
  );
};

interface CodexConfigSectionProps {
  value: string;
  onChange: (value: string) => void;
  providerName?: string;
  showRemoteCompaction?: boolean;
  useCommonConfig: boolean;
  onCommonConfigToggle: (checked: boolean) => void;
  onEditCommonConfig: () => void;
  commonConfigError?: string;
  configError?: string;
  isProxyTakeover?: boolean;
}

/**
 * CodexConfigSection - Config TOML editor section
 */
export const CodexConfigSection: React.FC<CodexConfigSectionProps> = ({
  value,
  onChange,
  providerName,
  showRemoteCompaction = true,
  useCommonConfig,
  onCommonConfigToggle,
  onEditCommonConfig,
  commonConfigError,
  configError,
  isProxyTakeover = false,
}) => {
  const { t } = useTranslation();
  const [isDarkMode, setIsDarkMode] = useState(false);

  useEffect(() => {
    setIsDarkMode(document.documentElement.classList.contains("dark"));

    const observer = new MutationObserver(() => {
      setIsDarkMode(document.documentElement.classList.contains("dark"));
    });

    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class"],
    });

    return () => observer.disconnect();
  }, []);

  // Mirror value prop to local state (same pattern as CommonConfigEditor)
  const [localValue, setLocalValue] = useState(value);
  const localValueRef = useRef(value);
  useEffect(() => {
    setLocalValue(value);
    localValueRef.current = value;
  }, [value]);

  const handleLocalChange = useCallback(
    (newValue: string) => {
      if (newValue === localValueRef.current) return;
      localValueRef.current = newValue;
      setLocalValue(newValue);
      onChange(newValue);
    },
    [onChange],
  );

  const goalModeEnabled = useMemo(
    () => isCodexGoalModeEnabled(localValue),
    [localValue],
  );
  const remoteCompactionEnabled = useMemo(
    () => isCodexRemoteCompactionEnabled(localValue),
    [localValue],
  );

  const handleGoalModeToggle = useCallback(
    (checked: boolean) => {
      handleLocalChange(setCodexGoalMode(localValueRef.current || "", checked));
    },
    [handleLocalChange],
  );

  const handleRemoteCompactionToggle = useCallback(
    (checked: boolean) => {
      handleLocalChange(
        setCodexRemoteCompaction(
          localValueRef.current || "",
          checked,
          providerName,
        ),
      );
    },
    [handleLocalChange, providerName],
  );

  // 不写入 model_auto_compact_token_limit：自动压缩阈值遵循 Codex 默认行为。

  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <label
          htmlFor="codexConfig"
          className="block text-sm font-medium text-foreground"
        >
          {t("codexConfig.configToml")}
        </label>

        <div className="flex flex-wrap items-center justify-end gap-x-4 gap-y-1">
          <label className="inline-flex cursor-pointer items-center gap-2 text-sm text-muted-foreground">
            <input
              type="checkbox"
              checked={goalModeEnabled}
              onChange={(e) => handleGoalModeToggle(e.target.checked)}
              className="w-4 h-4 text-blue-500 bg-white dark:bg-gray-800 border-border-default rounded focus:ring-blue-500 dark:focus:ring-blue-400 focus:ring-2"
            />
            {t("codexConfig.enableGoalMode")}
          </label>

          {showRemoteCompaction && (
            <label
              className="inline-flex cursor-pointer items-center gap-2 text-sm text-muted-foreground"
              title={t("codexConfig.remoteCompactionHint")}
            >
              <input
                type="checkbox"
                checked={remoteCompactionEnabled}
                onChange={(e) => handleRemoteCompactionToggle(e.target.checked)}
                className="w-4 h-4 text-blue-500 bg-white dark:bg-gray-800 border-border-default rounded focus:ring-blue-500 dark:focus:ring-blue-400 focus:ring-2"
              />
              {t("codexConfig.enableRemoteCompaction")}
            </label>
          )}

          <label className="inline-flex cursor-pointer items-center gap-2 text-sm text-muted-foreground">
            <input
              type="checkbox"
              checked={useCommonConfig}
              onChange={(e) => onCommonConfigToggle(e.target.checked)}
              className="w-4 h-4 text-blue-500 bg-white dark:bg-gray-800 border-border-default rounded focus:ring-blue-500 dark:focus:ring-blue-400 focus:ring-2"
            />
            {t("codexConfig.writeCommonConfig")}
          </label>
        </div>
      </div>

      <div className="flex items-center justify-end">
        <button
          type="button"
          onClick={onEditCommonConfig}
          className="text-xs text-blue-500 dark:text-blue-400 hover:underline"
        >
          {t("codexConfig.editCommonConfig")}
        </button>
      </div>

      {commonConfigError && (
        <p className="text-xs text-red-500 dark:text-red-400 text-right">
          {commonConfigError}
        </p>
      )}

      <JsonEditor
        value={localValue}
        onChange={handleLocalChange}
        placeholder=""
        darkMode={isDarkMode}
        rows={8}
        showValidation={false}
        language="javascript"
      />

      {configError && (
        <p className="text-xs text-red-500 dark:text-red-400">{configError}</p>
      )}

      {!configError && (
        <p className="text-xs text-muted-foreground">
          {t(
            isProxyTakeover
              ? "codexConfig.configTomlStorageHint"
              : "codexConfig.configTomlHint",
          )}
        </p>
      )}
    </div>
  );
};
