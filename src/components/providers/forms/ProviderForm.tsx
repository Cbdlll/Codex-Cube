import { useEffect, useMemo, useState, useCallback } from "react";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Form, FormField, FormItem, FormMessage } from "@/components/ui/form";
import { providerSchema, type ProviderFormData } from "@/lib/schemas/provider";
import {
  buildLocalProxyRequestOverrides,
  formatRequestOverrideObject,
} from "@/lib/requestOverrides";
import { settingsApi, type AppId } from "@/lib/api";
import type {
  ProviderCategory,
  ProviderMeta,
  CodexApiFormat,
  CodexCatalogModel,
  CodexChatReasoning,
  PromptCacheRoutingMode,
  ClaudeApiKeyField,
} from "@/types";
import {
  codexProviderPresets,
  type CodexProviderPreset,
} from "@/config/codexProviderPresets";
import { mergeProviderMeta } from "@/utils/providerMetaUtils";
import {
  codexApiFormatFromWireApi,
  extractCodexWireApi,
  setCodexWireApi,
  extractCodexModelName,
  setCodexModelName as setCodexModelNameInConfig,
} from "@/utils/providerConfigUtils";
import { isNonNegativeDecimalString } from "@/types/usage";
import { getCodexCustomTemplate } from "@/config/codexTemplates";
import CodexConfigEditor from "./CodexConfigEditor";
import { ProviderPresetSelector } from "./ProviderPresetSelector";
import { BasicFormFields } from "./BasicFormFields";
import { CodexFormFields } from "./CodexFormFields";
import {
  useProviderCategory,
  useCodexConfigState,
  useApiKeyLink,
  useCodexCommonConfig,
  useSpeedTestEndpoints,
  useCodexTomlValidation,
  useCodexOauth,
  useXaiOauth,
} from "./hooks";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { useSettingsQuery } from "@/lib/query";
import {
  ProviderAdvancedConfig,
  type PricingModelSourceOption,
} from "./ProviderAdvancedConfig";
import { resolveManagedAccountId } from "@/lib/authBinding";

const CODEX_DEFAULT_CONFIG = JSON.stringify({ auth: {}, config: "" }, null, 2);

const normalizePricingSource = (
  value?: string,
): PricingModelSourceOption =>
  value === "request" || value === "response" ? value : "inherit";

type PresetEntry = {
  id: string;
  preset: CodexProviderPreset;
};

export const normalizeCodexCatalogModelsForSave = (
  models: CodexCatalogModel[],
): CodexCatalogModel[] => {
  const seen = new Set<string>();
  const normalized: CodexCatalogModel[] = [];

  for (const item of models) {
    const model = item.model.trim();
    if (!model || seen.has(model)) continue;
    seen.add(model);

    const displayName = item.displayName?.trim();
    const rawContextWindow = String(item.contextWindow ?? "").replace(
      /[^\d]/g,
      "",
    );
    const contextWindow = rawContextWindow
      ? Number.parseInt(rawContextWindow, 10)
      : undefined;

    const inputModalities = item.inputModalities?.filter(
      (m) => typeof m === "string" && m.trim(),
    );

    const baseInstructions = item.baseInstructions?.trim();

    normalized.push({
      model,
      ...(displayName ? { displayName } : {}),
      ...(contextWindow && contextWindow > 0 ? { contextWindow } : {}),
      // Native Responses profile overrides (ignored by the chat/proxy profile).
      ...(typeof item.supportsParallelToolCalls === "boolean"
        ? { supportsParallelToolCalls: item.supportsParallelToolCalls }
        : {}),
      ...(inputModalities && inputModalities.length > 0
        ? { inputModalities }
        : {}),
      ...(baseInstructions ? { baseInstructions } : {}),
    });
  }

  return normalized;
};

const normalizeCodexChatReasoningForSave = (
  value?: CodexChatReasoning,
): CodexChatReasoning | undefined => {
  const supportsEffort = value?.supportsEffort === true;
  const supportsThinking = value?.supportsThinking === true || supportsEffort;
  const hasExplicitConfig = value && Object.keys(value).length > 0;

  if (!supportsThinking && !supportsEffort) {
    return hasExplicitConfig
      ? {
          supportsThinking: false,
          supportsEffort: false,
          thinkingParam: "none",
          effortParam: "none",
          outputFormat: value?.outputFormat ?? "auto",
        }
      : undefined;
  }

  return {
    supportsThinking,
    supportsEffort,
    thinkingParam: supportsThinking
      ? (value?.thinkingParam ?? "thinking")
      : "none",
    effortParam: supportsEffort
      ? (value?.effortParam ?? "reasoning_effort")
      : "none",
    effortValueMode: supportsEffort
      ? (value?.effortValueMode ?? "passthrough")
      : undefined,
    outputFormat: value?.outputFormat ?? "auto",
  };
};

type LocalProxyRequestOverridesBuildResult = ReturnType<
  typeof buildLocalProxyRequestOverrides
>;

export interface ProviderFormProps {
  appId: AppId;
  providerId?: string;
  submitLabel: string;
  onSubmit: (values: ProviderFormValues) => Promise<void> | void;
  onCancel: () => void;
  onSubmittingChange?: (isSubmitting: boolean) => void;
  initialData?: {
    name?: string;
    websiteUrl?: string;
    notes?: string;
    settingsConfig?: Record<string, unknown>;
    category?: ProviderCategory;
    meta?: ProviderMeta;
    icon?: string;
    iconColor?: string;
  };
  showButtons?: boolean;
  isProxyTakeover?: boolean;
}

export function ProviderForm(props: ProviderFormProps) {
  return <ProviderFormFull {...props} />;
}

function ProviderFormFull({
  appId,
  providerId,
  submitLabel,
  onSubmit,
  onCancel,
  onSubmittingChange,
  initialData,
  showButtons = true,
  isProxyTakeover = false,
}: ProviderFormProps) {
  const { t } = useTranslation();
  const isEditMode = Boolean(initialData);
  const queryClient = useQueryClient();
  const { data: settingsData } = useSettingsQuery();
  const showCommonConfigNotice =
    settingsData != null && settingsData.commonConfigConfirmed !== true;

  const handleCommonConfigConfirm = async () => {
    try {
      if (settingsData) {
        const { webdavSync: _, ...rest } = settingsData;
        await settingsApi.save({ ...rest, commonConfigConfirmed: true });
        await queryClient.invalidateQueries({ queryKey: ["settings"] });
      }
    } catch (error) {
      console.error("Failed to save commonConfigConfirmed:", error);
    }
  };

  const [selectedPresetId, setSelectedPresetId] = useState<string | null>(
    initialData ? null : "custom",
  );
  const [activePreset, setActivePreset] = useState<{
    id: string;
    category?: ProviderCategory;
  } | null>(null);
  const [isCodexEndpointModalOpen, setIsCodexEndpointModalOpen] =
    useState(false);

  const [draftCustomEndpoints, setDraftCustomEndpoints] = useState<string[]>(
    () => {
      if (initialData) return [];
      return [];
    },
  );
  const [endpointAutoSelect, setEndpointAutoSelect] = useState<boolean>(
    () => initialData?.meta?.endpointAutoSelect ?? true,
  );
  const supportsFullUrl = appId === "codex";
  const [localIsFullUrl, setLocalIsFullUrl] = useState<boolean>(() => {
    if (!supportsFullUrl) return false;
    return initialData?.meta?.isFullUrl ?? false;
  });

  const [pricingConfig, setPricingConfig] = useState<{
    enabled: boolean;
    costMultiplier?: string;
    pricingModelSource: PricingModelSourceOption;
  }>(() => ({
    enabled:
      initialData?.meta?.costMultiplier !== undefined ||
      initialData?.meta?.pricingModelSource !== undefined,
    costMultiplier: initialData?.meta?.costMultiplier,
    pricingModelSource: normalizePricingSource(
      initialData?.meta?.pricingModelSource,
    ),
  }));

  const { category } = useProviderCategory({
    appId,
    selectedPresetId,
    isEditMode,
    initialCategory: initialData?.category,
  });

  useEffect(() => {
    setSelectedPresetId(initialData ? null : "custom");
    setActivePreset(null);

    if (!initialData) {
      setDraftCustomEndpoints([]);
    }
    setEndpointAutoSelect(initialData?.meta?.endpointAutoSelect ?? true);
    setLocalIsFullUrl(
      supportsFullUrl ? (initialData?.meta?.isFullUrl ?? false) : false,
    );
    setPricingConfig({
      enabled:
        initialData?.meta?.costMultiplier !== undefined ||
        initialData?.meta?.pricingModelSource !== undefined,
      costMultiplier: initialData?.meta?.costMultiplier,
      pricingModelSource: normalizePricingSource(
        initialData?.meta?.pricingModelSource,
      ),
    });
    setCodexChatReasoning(initialData?.meta?.codexChatReasoning ?? {});
    setPromptCacheRouting(initialData?.meta?.promptCacheRouting ?? "auto");
    setCustomUserAgent(initialData?.meta?.customUserAgent ?? "");
    setLocalProxyHeadersOverride(
      formatRequestOverrideObject(
        initialData?.meta?.localProxyRequestOverrides?.headers,
      ),
    );
    setLocalProxyBodyOverride(
      formatRequestOverrideObject(
        initialData?.meta?.localProxyRequestOverrides?.body,
      ),
    );
  }, [appId, initialData, supportsFullUrl]);

  const defaultValues: ProviderFormData = useMemo(
    () => ({
      name: initialData?.name ?? "",
      websiteUrl: initialData?.websiteUrl ?? "",
      notes: initialData?.notes ?? "",
      settingsConfig: initialData?.settingsConfig
        ? JSON.stringify(initialData.settingsConfig, null, 2)
        : CODEX_DEFAULT_CONFIG,
      icon: initialData?.icon ?? "",
      iconColor: initialData?.iconColor ?? "",
    }),
    [initialData],
  );

  const form = useForm<ProviderFormData>({
    resolver: zodResolver(providerSchema),
    defaultValues,
    mode: "onSubmit",
  });
  const { isSubmitting } = form.formState;

  // 软校验：收集"业务约束"类问题（空值/缺项），由用户决定是否仍要保存
  const [softIssues, setSoftIssues] = useState<string[] | null>(null);
  const [pendingFormValues, setPendingFormValues] =
    useState<ProviderFormData | null>(null);
  const [
    pendingLocalProxyRequestOverridesResult,
    setPendingLocalProxyRequestOverridesResult,
  ] = useState<LocalProxyRequestOverridesBuildResult | null>(null);
  // 确认框走的提交路径绕过了 react-hook-form 的 isSubmitting，单独追踪
  const [isConfirmSubmitting, setIsConfirmSubmitting] = useState(false);

  useEffect(() => {
    onSubmittingChange?.(isSubmitting || isConfirmSubmitting);
  }, [isSubmitting, isConfirmSubmitting, onSubmittingChange]);

  const {
    codexAuth,
    codexConfig,
    codexApiKey,
    codexBaseUrl,
    codexModel,
    codexCatalogModels,
    codexAuthError,
    setCodexAuth,
    setCodexConfig,
    setCodexCatalogModels,
    handleCodexApiKeyChange,
    handleCodexBaseUrlChange,
    handleCodexModelChange,
    handleCodexConfigChange: originalHandleCodexConfigChange,
    resetCodexConfig,
  } = useCodexConfigState({ initialData });

  const initialCodexApiFormat: CodexApiFormat =
    initialData?.meta?.apiFormat === "openai_chat"
      ? "openai_chat"
      : initialData?.meta?.apiFormat === "anthropic"
        ? "anthropic"
        : initialData?.meta?.apiFormat === "openai_responses"
          ? "openai_responses"
          : (codexApiFormatFromWireApi(
              extractCodexWireApi(
                typeof initialData?.settingsConfig?.config === "string"
                  ? initialData.settingsConfig.config
                  : "",
              ),
            ) ?? "openai_responses");

  const [localCodexApiFormat, setLocalCodexApiFormat] =
    useState<CodexApiFormat>(initialCodexApiFormat);

  // Auth-field choice for the Anthropic Messages upstream (defaults to the Bearer form)
  const initialCodexAnthropicAuthField: ClaudeApiKeyField =
    initialData?.meta?.apiKeyField === "ANTHROPIC_API_KEY"
      ? "ANTHROPIC_API_KEY"
      : "ANTHROPIC_AUTH_TOKEN";
  const [localCodexAnthropicAuthField, setLocalCodexAnthropicAuthField] =
    useState<ClaudeApiKeyField>(initialCodexAnthropicAuthField);

  // Emulate the Claude Code client: off by default, enabled only when the user explicitly turns it on (true)
  const [localCodexImpersonateClaudeCode, setLocalCodexImpersonateClaudeCode] =
    useState<boolean>(initialData?.meta?.impersonateClaudeCode === true);

  // Codex → Anthropic output ceiling override (empty string = use the 8192 default).
  // Kept as a string so the numeric input can be cleared; parsed on save.
  const [localCodexMaxOutputTokens, setLocalCodexMaxOutputTokens] =
    useState<string>(
      typeof initialData?.meta?.maxOutputTokens === "number" &&
        initialData.meta.maxOutputTokens > 0
        ? String(initialData.meta.maxOutputTokens)
        : "",
    );

  const { configError: codexConfigError, debouncedValidate } =
    useCodexTomlValidation();

  const handleCodexConfigChange = useCallback(
    (value: string) => {
      originalHandleCodexConfigChange(value);
      debouncedValidate(value);
    },
    [originalHandleCodexConfigChange, debouncedValidate],
  );

  const handleCodexApiFormatChange = useCallback(
    (format: CodexApiFormat) => {
      setLocalCodexApiFormat(format);
      // wire_api is always "responses" for Codex; format controls proxy-layer conversion
      setCodexConfig((prev) => {
        const updated = setCodexWireApi(prev, "responses");
        debouncedValidate(updated);
        return updated;
      });
    },
    [setCodexConfig, debouncedValidate],
  );

  useEffect(() => {
    if (!initialData && selectedPresetId === "custom") {
      const template = getCodexCustomTemplate();
      resetCodexConfig(template.auth, template.config);
      setCodexChatReasoning({});
      setPromptCacheRouting("auto");
    }
  }, [initialData, selectedPresetId, resetCodexConfig]);

  useEffect(() => {
    form.reset(defaultValues);
  }, [defaultValues, form]);

  const presetCategoryLabels: Record<string, string> = useMemo(
    () => ({
      official: t("providerForm.categoryOfficial", {
        defaultValue: "官方",
      }),
      cn_official: t("providerForm.categoryCnOfficial", {
        defaultValue: "国内官方",
      }),
      aggregator: t("providerForm.categoryAggregation", {
        defaultValue: "聚合服务",
      }),
      third_party: t("providerForm.categoryThirdParty", {
        defaultValue: "第三方",
      }),
    }),
    [t],
  );

  const presetEntries = useMemo(() => {
    return codexProviderPresets.map<PresetEntry>((preset, index) => ({
      id: `codex-${index}`,
      preset,
    }));
  }, []);

  // 预设声明的托管身份类型（codex_oauth / xai_oauth）
  const presetProviderType = useMemo(() => {
    if (!selectedPresetId) return undefined;
    const preset = presetEntries.find(
      (entry) => entry.id === selectedPresetId,
    )?.preset;
    return preset && "providerType" in preset ? preset.providerType : undefined;
  }, [presetEntries, selectedPresetId]);

  const {
    useCommonConfig: useCodexCommonConfigFlag,
    commonConfigSnippet: codexCommonConfigSnippet,
    commonConfigError: codexCommonConfigError,
    handleCommonConfigToggle: handleCodexCommonConfigToggle,
    handleCommonConfigSnippetChange: handleCodexCommonConfigSnippetChange,
    isExtracting: isCodexExtracting,
    handleExtract: handleCodexExtract,
    clearCommonConfigError: clearCodexCommonConfigError,
  } = useCodexCommonConfig({
    codexConfig,
    onConfigChange: handleCodexConfigChange,
    initialData: appId === "codex" ? initialData : undefined,
    initialEnabled:
      appId === "codex" ? initialData?.meta?.commonConfigEnabled : undefined,
    selectedPresetId: selectedPresetId ?? undefined,
  });

  // Codex OAuth 认证状态（ChatGPT Plus/Pro 反代）
  const {
    isAuthenticated: isCodexOauthAuthenticated,
    accounts: codexOauthAccounts,
  } = useCodexOauth();

  const {
    isAuthenticated: isXaiOauthAuthenticated,
    accounts: xaiOauthAccounts,
  } = useXaiOauth();

  // 选中的 ChatGPT 账号 ID（Codex OAuth 多账号支持）
  const [selectedCodexAccountId] = useState<string | null>(() =>
    resolveManagedAccountId(initialData?.meta, "codex_oauth"),
  );
  const [selectedXaiAccountId, setSelectedXaiAccountId] = useState<
    string | null
  >(() => resolveManagedAccountId(initialData?.meta, "xai_oauth"));
  const [codexFastMode] = useState<boolean>(
    () => initialData?.meta?.codexFastMode ?? false,
  );
  const [codexChatReasoning, setCodexChatReasoning] =
    useState<CodexChatReasoning>(
      () => initialData?.meta?.codexChatReasoning ?? {},
    );
  const [promptCacheRouting, setPromptCacheRouting] =
    useState<PromptCacheRoutingMode>(
      () => initialData?.meta?.promptCacheRouting ?? "auto",
    );
  const [customUserAgent, setCustomUserAgent] = useState<string>(
    () => initialData?.meta?.customUserAgent ?? "",
  );
  const [localProxyHeadersOverride, setLocalProxyHeadersOverride] =
    useState<string>(() =>
      formatRequestOverrideObject(
        initialData?.meta?.localProxyRequestOverrides?.headers,
      ),
    );
  const [localProxyBodyOverride, setLocalProxyBodyOverride] = useState<string>(
    () =>
      formatRequestOverrideObject(
        initialData?.meta?.localProxyRequestOverrides?.body,
      ),
  );

  const shouldApplyLocalProxyRequestOverrides =
    appId === "codex" && category !== "official";

  const handleSubmit = async (values: ProviderFormData) => {
    const overridesResult = shouldApplyLocalProxyRequestOverrides
      ? buildLocalProxyRequestOverrides(
          localProxyHeadersOverride,
          localProxyBodyOverride,
        )
      : {};
    if (overridesResult.error) {
      toast.error(
        t("providerForm.localProxyRequestOverridesInvalid", {
          defaultValue: `本地代理请求覆盖格式错误：${overridesResult.error}`,
          error: overridesResult.error,
        }),
      );
      return;
    }

    // 软性问题（业务约束，用户可选择仍要保存）
    const issues: string[] = [];

    // 供应商名空：A 类
    if (!values.name.trim()) {
      issues.push(
        t("providerForm.fillSupplierName", {
          defaultValue: "请填写供应商名称",
        }),
      );
    }

    const costMultiplier = pricingConfig.costMultiplier?.trim();
    if (
      pricingConfig.enabled &&
      costMultiplier &&
      !isNonNegativeDecimalString(costMultiplier)
    ) {
      toast.error(
        t("settings.globalProxy.defaultCostMultiplierInvalid", {
          defaultValue: "成本倍率必须为非负数",
        }),
      );
      return;
    }

    // OAuth 未登录：B 类（token 根本不存在，保存了也没法建立）
    const isCodexOauthProvider =
      initialData?.meta?.providerType === "codex_oauth";
    const isXaiOauthProvider =
      presetProviderType === "xai_oauth" ||
      initialData?.meta?.providerType === "xai_oauth";
    if (isCodexOauthProvider && !isCodexOauthAuthenticated) {
      toast.error(
        t("codexOauth.loginRequired", {
          defaultValue: "请先登录 ChatGPT 账号",
        }),
      );
      return;
    }
    if (isXaiOauthProvider && !isXaiOauthAuthenticated) {
      toast.error(
        t("xaiOauth.loginRequired", {
          defaultValue: "请先登录 xAI 账号",
        }),
      );
      return;
    }

    const selectedAccountIsUsable = (
      accountId: string | null,
      accounts: Array<{ id: string; requires_reauth: boolean }>,
    ) =>
      accountId === null ||
      accounts.some(
        (account) => account.id === accountId && !account.requires_reauth,
      );
    if (
      isCodexOauthProvider &&
      !selectedAccountIsUsable(selectedCodexAccountId, codexOauthAccounts)
    ) {
      toast.error(
        t("managedAuth.selectedAccountUnavailable", {
          defaultValue: "已绑定账号不存在，请重新选择账号",
        }),
      );
      return;
    }
    if (
      isXaiOauthProvider &&
      !selectedAccountIsUsable(selectedXaiAccountId, xaiOauthAccounts)
    ) {
      toast.error(
        t("managedAuth.selectedAccountNeedsReauth", {
          defaultValue: "已绑定 xAI 账号不存在或需要重新登录",
        }),
      );
      return;
    }

    // 非官方供应商端点 / API Key 空：A 类
    if (category !== "official") {
      // 托管 OAuth 预设（xAI）：端点由 adapter 硬定向、token 由代理注入，
      // 两项都不需要用户填写
      if (!isXaiOauthProvider && !codexBaseUrl.trim()) {
        issues.push(
          t("providerForm.endpointRequired", {
            defaultValue: "非官方供应商请填写 API 端点",
          }),
        );
      }
      if (!isXaiOauthProvider && !codexApiKey.trim()) {
        issues.push(
          t("providerForm.apiKeyRequired", {
            defaultValue: "非官方供应商请填写 API Key",
          }),
        );
      }
    }

    if (issues.length > 0) {
      // 弹确认框让用户决定是否仍要保存
      setSoftIssues(issues);
      setPendingFormValues(values);
      setPendingLocalProxyRequestOverridesResult(overridesResult);
      return;
    }

    await performSubmit(values, overridesResult);
  };

  const performSubmit = async (
    values: ProviderFormData,
    overridesResult: LocalProxyRequestOverridesBuildResult,
  ) => {
    if (overridesResult.error) {
      toast.error(
        t("providerForm.localProxyRequestOverridesInvalid", {
          defaultValue: `本地代理请求覆盖格式错误：${overridesResult.error}`,
          error: overridesResult.error,
        }),
      );
      return;
    }

    // OAuth / 其它身份识别（与 handleSubmit 保持一致）
    const isCodexOauthProvider =
      initialData?.meta?.providerType === "codex_oauth";
    const isXaiOauthProvider =
      presetProviderType === "xai_oauth" ||
      initialData?.meta?.providerType === "xai_oauth";

    let settingsConfig: string;

    try {
      const authJson = JSON.parse(codexAuth);
      let normalizedCodexConfig =
        category !== "official" && (codexConfig ?? "").trim()
          ? setCodexWireApi(codexConfig ?? "", "responses")
          : (codexConfig ?? "");
      // 模型映射与「路由接管」解耦：对所有非官方供应商，填了就持久化
      //（Chat 生成兼容路由、原生 Responses 生成 model-catalogs.json），
      // 留空归一化为 [] 即不写。后端只看 modelCatalog.models 是否非空。
      const normalizedCatalogModels =
        category !== "official"
          ? normalizeCodexCatalogModelsForSave(codexCatalogModels)
          : [];
      // The default-model field writes the top-level `model` into the TOML
      // as the user types; only when it was left empty fall back to the
      // first catalog row so "fill mapping only" keeps its old behavior.
      if (
        normalizedCatalogModels.length > 0 &&
        !extractCodexModelName(normalizedCodexConfig)
      ) {
        normalizedCodexConfig = setCodexModelNameInConfig(
          normalizedCodexConfig,
          normalizedCatalogModels[0].model,
        );
      }
      const configObj = {
        auth: authJson,
        config: normalizedCodexConfig,
      } as {
        auth: unknown;
        config: string;
        modelCatalog?: { models: CodexCatalogModel[] };
      };
      if (normalizedCatalogModels.length > 0) {
        configObj.modelCatalog = { models: normalizedCatalogModels };
      }
      settingsConfig = JSON.stringify(configObj);
    } catch (err) {
      settingsConfig = values.settingsConfig.trim();
    }

    const payload: ProviderFormValues = {
      ...values,
      name: values.name.trim(),
      websiteUrl: values.websiteUrl?.trim() ?? "",
      settingsConfig,
    };

    if (activePreset) {
      payload.presetId = activePreset.id;
      if (activePreset.category) {
        payload.presetCategory = activePreset.category;
      }
    }

    if (!isEditMode && draftCustomEndpoints.length > 0) {
      const customEndpointsToSave: Record<
        string,
        import("@/types").CustomEndpoint
      > = draftCustomEndpoints.reduce(
        (acc, url) => {
          const now = Date.now();
          acc[url] = { url, addedAt: now, lastUsed: undefined };
          return acc;
        },
        {} as Record<string, import("@/types").CustomEndpoint>,
      );

      const hadEndpoints =
        initialData?.meta?.custom_endpoints &&
        Object.keys(initialData.meta.custom_endpoints).length > 0;
      const needsClearEndpoints =
        hadEndpoints && draftCustomEndpoints.length === 0;

      let mergedMeta = needsClearEndpoints
        ? mergeProviderMeta(initialData?.meta, {})
        : mergeProviderMeta(initialData?.meta, customEndpointsToSave);

      if (mergedMeta !== undefined) {
        payload.meta = mergedMeta;
      }
    }

    const baseMeta: ProviderMeta | undefined =
      payload.meta ?? (initialData?.meta ? { ...initialData.meta } : undefined);

    // 确定 providerType（新建时从预设获取，编辑时从现有数据获取）
    const providerType = presetProviderType || initialData?.meta?.providerType;

    const nextMeta: ProviderMeta = {
      ...(baseMeta ?? {}),
      commonConfigEnabled: useCodexCommonConfigFlag,
      endpointAutoSelect,
      // 保存 providerType（用于识别 Codex OAuth 等特殊供应商）
      providerType,
      authBinding: isCodexOauthProvider
        ? {
            source: "managed_account",
            authProvider: "codex_oauth",
            accountId: selectedCodexAccountId ?? undefined,
          }
        : isXaiOauthProvider
          ? {
              source: "managed_account",
              authProvider: "xai_oauth",
              accountId: selectedXaiAccountId ?? undefined,
            }
          : undefined,
      codexFastMode: isCodexOauthProvider ? codexFastMode : undefined,
      codexChatReasoning:
        category !== "official" && localCodexApiFormat === "openai_chat"
          ? normalizeCodexChatReasoningForSave(codexChatReasoning)
          : undefined,
      promptCacheRouting:
        category !== "official" &&
        localCodexApiFormat === "openai_chat" &&
        promptCacheRouting !== "auto"
          ? promptCacheRouting
          : undefined,
      customUserAgent:
        category !== "official"
          ? customUserAgent.trim() || undefined
          : undefined,
      localProxyRequestOverrides: shouldApplyLocalProxyRequestOverrides
        ? overridesResult.overrides
        : undefined,
      costMultiplier: pricingConfig.enabled
        ? pricingConfig.costMultiplier
        : undefined,
      pricingModelSource:
        pricingConfig.enabled && pricingConfig.pricingModelSource !== "inherit"
          ? pricingConfig.pricingModelSource
          : undefined,
      apiFormat:
        category !== "official"
          ? isXaiOauthProvider
            ? "openai_responses"
            : localCodexApiFormat
          : undefined,
      apiKeyField:
        category !== "official" &&
        localCodexApiFormat === "anthropic" &&
        localCodexAnthropicAuthField !== "ANTHROPIC_AUTH_TOKEN"
          ? localCodexAnthropicAuthField
          : undefined,
      // Off by default; persist true only for codex+anthropic when the user explicitly enables it
      impersonateClaudeCode:
        category !== "official" &&
        localCodexApiFormat === "anthropic" &&
        localCodexImpersonateClaudeCode
          ? true
          : undefined,
      // Persist only for codex+anthropic when a positive value was entered
      maxOutputTokens:
        category !== "official" &&
        localCodexApiFormat === "anthropic" &&
        localCodexMaxOutputTokens.trim() !== "" &&
        Number(localCodexMaxOutputTokens) > 0
          ? Number(localCodexMaxOutputTokens)
          : undefined,
      isFullUrl:
        supportsFullUrl &&
        category !== "official" &&
        !isXaiOauthProvider &&
        localIsFullUrl
          ? true
          : undefined,
    };

    if (!isCodexOauthProvider && "codexFastMode" in nextMeta) {
      delete nextMeta.codexFastMode;
    }

    payload.meta = nextMeta;

    await onSubmit(payload);
  };

  const shouldShowSpeedTest =
    category !== "official" && category !== "cloud_provider";

  const {
    shouldShowApiKeyLink: shouldShowCodexApiKeyLink,
    websiteUrl: codexWebsiteUrl,
  } = useApiKeyLink({
    appId: "codex",
    category,
    selectedPresetId,
    presetEntries,
    formWebsiteUrl: form.watch("websiteUrl") || "",
  });

  // 使用端点测速候选 hook
  const speedTestEndpoints = useSpeedTestEndpoints({
    appId,
    selectedPresetId,
    presetEntries,
    baseUrl: "",
    codexBaseUrl,
    initialData,
  });

  const handlePresetChange = (value: string) => {
    setSelectedPresetId(value);
    if (value === "custom") {
      setActivePreset(null);
      form.reset(defaultValues);

      const template = getCodexCustomTemplate();
      resetCodexConfig(template.auth, template.config);
      setCodexChatReasoning({});
      setPromptCacheRouting("auto");
      setLocalCodexApiFormat(
        codexApiFormatFromWireApi(extractCodexWireApi(template.config)) ??
          "openai_responses",
      );
      return;
    }

    const entry = presetEntries.find((item) => item.id === value);
    if (!entry) {
      return;
    }

    setActivePreset({
      id: value,
      category: entry.preset.category,
    });

    const preset = entry.preset as CodexProviderPreset;
    const auth = preset.auth ?? {};
    const config = preset.config ?? "";

    resetCodexConfig(auth, config, preset.modelCatalog ?? []);
    setCodexChatReasoning(preset.codexChatReasoning ?? {});
    setPromptCacheRouting(preset.promptCacheRouting ?? "auto");
    setLocalCodexApiFormat(
      preset.apiFormat ??
        codexApiFormatFromWireApi(extractCodexWireApi(config)) ??
        "openai_responses",
    );

    form.reset({
      name: preset.nameKey ? t(preset.nameKey) : preset.name,
      websiteUrl: preset.websiteUrl ?? "",
      settingsConfig: JSON.stringify({ auth, config }, null, 2),
      icon: preset.icon ?? "",
      iconColor: preset.iconColor ?? "",
    });
  };

  const settingsConfigErrorField = (
    <FormField
      control={form.control}
      name="settingsConfig"
      render={() => (
        <FormItem className="space-y-0">
          <FormMessage />
        </FormItem>
      )}
    />
  );

  return (
    <>
      <Form {...form}>
        <form
          id="provider-form"
          onSubmit={form.handleSubmit(handleSubmit)}
          className="space-y-6 glass rounded-xl p-6 border border-white/10"
        >
          {!initialData && (
            <ProviderPresetSelector
              selectedPresetId={selectedPresetId}
              presetEntries={presetEntries}
              presetCategoryLabels={presetCategoryLabels}
              onPresetChange={handlePresetChange}
              category={category}
            />
          )}

          <BasicFormFields form={form} />

          <CodexFormFields
            providerId={providerId}
            isXaiOauthPreset={
              presetProviderType === "xai_oauth" ||
              initialData?.meta?.providerType === "xai_oauth"
            }
            isXaiOauthAuthenticated={isXaiOauthAuthenticated}
            selectedXaiAccountId={selectedXaiAccountId}
            onXaiAccountSelect={setSelectedXaiAccountId}
            codexApiKey={codexApiKey}
            onApiKeyChange={handleCodexApiKeyChange}
            category={category}
            shouldShowApiKeyLink={shouldShowCodexApiKeyLink}
            websiteUrl={codexWebsiteUrl}
            shouldShowSpeedTest={shouldShowSpeedTest}
            codexBaseUrl={codexBaseUrl}
            onBaseUrlChange={handleCodexBaseUrlChange}
            isFullUrl={localIsFullUrl}
            onFullUrlChange={setLocalIsFullUrl}
            isEndpointModalOpen={isCodexEndpointModalOpen}
            onEndpointModalToggle={setIsCodexEndpointModalOpen}
            onCustomEndpointsChange={
              isEditMode ? undefined : setDraftCustomEndpoints
            }
            autoSelect={endpointAutoSelect}
            onAutoSelectChange={setEndpointAutoSelect}
            codexModel={codexModel}
            onModelChange={handleCodexModelChange}
            apiFormat={localCodexApiFormat}
            onApiFormatChange={handleCodexApiFormatChange}
            anthropicAuthField={localCodexAnthropicAuthField}
            onAnthropicAuthFieldChange={setLocalCodexAnthropicAuthField}
            impersonateClaudeCode={localCodexImpersonateClaudeCode}
            onImpersonateClaudeCodeChange={setLocalCodexImpersonateClaudeCode}
            maxOutputTokens={localCodexMaxOutputTokens}
            onMaxOutputTokensChange={setLocalCodexMaxOutputTokens}
            codexChatReasoning={codexChatReasoning}
            onCodexChatReasoningChange={setCodexChatReasoning}
            promptCacheRouting={promptCacheRouting}
            onPromptCacheRoutingChange={setPromptCacheRouting}
            catalogModels={codexCatalogModels}
            onCatalogModelsChange={setCodexCatalogModels}
            speedTestEndpoints={speedTestEndpoints}
            customUserAgent={customUserAgent}
            onCustomUserAgentChange={setCustomUserAgent}
            localProxyHeadersOverride={localProxyHeadersOverride}
            onLocalProxyHeadersOverrideChange={setLocalProxyHeadersOverride}
            localProxyBodyOverride={localProxyBodyOverride}
            onLocalProxyBodyOverrideChange={setLocalProxyBodyOverride}
          />

          <>
            <CodexConfigEditor
              authValue={codexAuth}
              configValue={codexConfig}
              providerName={form.watch("name")}
              showRemoteCompaction={category !== "official"}
              isProxyTakeover={isProxyTakeover}
              onAuthChange={setCodexAuth}
              onConfigChange={handleCodexConfigChange}
              useCommonConfig={useCodexCommonConfigFlag}
              onCommonConfigToggle={handleCodexCommonConfigToggle}
              commonConfigSnippet={codexCommonConfigSnippet}
              onCommonConfigSnippetChange={
                handleCodexCommonConfigSnippetChange
              }
              onCommonConfigErrorClear={clearCodexCommonConfigError}
              commonConfigError={codexCommonConfigError}
              authError={codexAuthError}
              configError={codexConfigError}
              onExtract={handleCodexExtract}
              isExtracting={isCodexExtracting}
            />
            {settingsConfigErrorField}
          </>

          <ProviderAdvancedConfig
            pricingConfig={pricingConfig}
            onPricingConfigChange={setPricingConfig}
          />

          {showButtons && (
            <div className="flex justify-end gap-2">
              <Button variant="outline" type="button" onClick={onCancel}>
                {t("common.cancel")}
              </Button>
              <Button
                type="submit"
                disabled={isSubmitting || isConfirmSubmitting}
              >
                {submitLabel}
              </Button>
            </div>
          )}
        </form>
      </Form>

      <ConfirmDialog
        isOpen={showCommonConfigNotice}
        variant="info"
        title={t("confirm.commonConfig.title")}
        message={t("confirm.commonConfig.message")}
        confirmText={t("confirm.commonConfig.confirm")}
        onConfirm={() => void handleCommonConfigConfirm()}
        onCancel={() => void handleCommonConfigConfirm()}
      />

      <ConfirmDialog
        isOpen={softIssues !== null && softIssues.length > 0}
        variant="info"
        title={t("providerForm.softValidation.title", {
          defaultValue: "配置存在以下问题",
        })}
        message={
          (softIssues ?? []).map((issue) => `• ${issue}`).join("\n") +
          "\n\n" +
          t("providerForm.softValidation.hint", {
            defaultValue:
              "仍要保存吗？保存后切换此供应商时可能失败，可以之后再补全。",
          })
        }
        confirmText={t("providerForm.softValidation.saveAnyway", {
          defaultValue: "仍要保存",
        })}
        cancelText={t("common.cancel")}
        onConfirm={async () => {
          if (isConfirmSubmitting) return;
          const values = pendingFormValues;
          const overridesResult = pendingLocalProxyRequestOverridesResult;
          if (!values || !overridesResult) {
            setSoftIssues(null);
            setPendingFormValues(null);
            setPendingLocalProxyRequestOverridesResult(null);
            return;
          }
          setIsConfirmSubmitting(true);
          try {
            await performSubmit(values, overridesResult);
            setSoftIssues(null);
            setPendingFormValues(null);
            setPendingLocalProxyRequestOverridesResult(null);
          } catch (error) {
            console.error("[ProviderForm] soft-confirm submit failed:", error);
            // 保留确认框和 pending values，让用户可以重试或取消
          } finally {
            setIsConfirmSubmitting(false);
          }
        }}
        onCancel={() => {
          if (isConfirmSubmitting) return;
          setSoftIssues(null);
          setPendingFormValues(null);
          setPendingLocalProxyRequestOverridesResult(null);
        }}
      />
    </>
  );
}

export type ProviderFormValues = ProviderFormData & {
  presetId?: string;
  presetCategory?: ProviderCategory;
  meta?: ProviderMeta;
};
