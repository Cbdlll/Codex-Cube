import {
  Fragment,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  Check,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  Loader2,
  Plus,
  RefreshCw,
  Save,
  X,
} from "lucide-react";
import { useForm } from "react-hook-form";
import { Form, FormLabel } from "@/components/ui/form";
import { BasicFormFields } from "@/components/providers/forms/BasicFormFields";
import { type ProviderFormData } from "@/lib/schemas/provider";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { cn } from "@/lib/utils";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { providersApi, type AppId } from "@/lib/api";
import { fetchModelsForConfig } from "@/lib/api/model-fetch";
import type { AggregateProviderModel, CodexApiFormat, Provider } from "@/types";
import CodexConfigEditor from "@/components/providers/forms/CodexConfigEditor";
import {
  ProviderAdvancedConfig,
  type PricingModelSourceOption,
} from "@/components/providers/forms/ProviderAdvancedConfig";
import { useCodexCommonConfig } from "@/components/providers/forms/hooks/useCodexCommonConfig";
import { useCodexTomlValidation } from "@/components/providers/forms/hooks/useCodexTomlValidation";
import {
  extractCodexModelName,
  setCodexModelName,
} from "@/utils/providerConfigUtils";
import {
  type AggregateModelMeta,
  aggregateMetaFromCatalogEntry,
  applyAggregateModelMeta,
  buildAggregateModels,
  buildAggregateSettingsConfig,
  CODEX_REASONING_EFFORTS,
  extractCodexReasoningEffortFromConfig,
  generateAggregateProviderId,
  getAggregateModelApiFormat,
  getCodexMemberCredentials,
  getCodexMemberWireApi,
  hydrateAggregateConfigToml,
  knownCodexContextWindow,
  normalizeAggregateModelsForSave,
  normalizeCodexReasoningEffort,
  parseAggregateSettings,
  setCodexReasoningEffortInConfig,
  type CodexReasoningEffort,
} from "@/utils/aggregateProvider";
import { resolveCodexContextWindow } from "@/utils/codexPresetContextWindows";

const AGGREGATE_FORM_ID = "aggregate-provider-form";

const normalizePricingSource = (
  value?: string,
): PricingModelSourceOption =>
  value === "request" || value === "response" ? value : "inherit";

function stringifyAuth(auth: unknown): string {
  try {
    return JSON.stringify(auth ?? {}, null, 2);
  } catch {
    return "{}";
  }
}

const STEPS = [
  { id: 1, key: "basic", fallbackLabel: "基本信息" },
  { id: 2, key: "members", fallbackLabel: "选择成员" },
  { id: 3, key: "models", fallbackLabel: "选择模型" },
  { id: 4, key: "preview", fallbackLabel: "预览保存" },
] as const;

type FetchState = "idle" | "loading" | "done" | "error";

function memberCatalogModels(provider: Provider | undefined): Record<string, any>[] {
  const config = provider?.settingsConfig as Record<string, any> | undefined;
  return Array.isArray(config?.modelCatalog?.models)
    ? (config.modelCatalog.models as Record<string, any>[])
    : [];
}

function hasDeclaredContextWindow(meta: AggregateModelMeta | undefined): boolean {
  if (meta?.contextWindow === undefined || meta.contextWindow === "") {
    return false;
  }
  const value = Number(meta.contextWindow);
  return Number.isFinite(value) && value > 0;
}

function resolveMemberModelContextWindow(
  provider: Provider | undefined,
  model: string,
  fetchedWindow?: number | null,
): number | undefined {
  const catalogMeta = aggregateMetaFromCatalogEntry(
    memberCatalogModels(provider).find((entry) => entry?.model === model),
  );
  const fromCatalogOrPreset = resolveCodexContextWindow(
    model,
    catalogMeta?.contextWindow,
  );
  if (fromCatalogOrPreset) return fromCatalogOrPreset;
  if (typeof fetchedWindow === "number" && fetchedWindow > 0) {
    return fetchedWindow;
  }
  return undefined;
}

interface AggregateProviderWizardProps {
  appId: AppId;
  initialProvider?: Provider | null;
  onAdd: (
    providerData: Omit<Provider, "id"> & { providerKey?: string },
  ) => Promise<void> | void;
  onEdit?: (payload: {
    provider: Provider;
    originalId?: string;
  }) => Promise<void> | void;
  onCancel: () => void;
  showButtons?: boolean;
  isProxyTakeover?: boolean;
  onSubmittingChange?: (isSubmitting: boolean) => void;
}

export function AggregateProviderWizard({
  appId,
  initialProvider,
  onAdd,
  onEdit,
  onCancel,
  showButtons = true,
  isProxyTakeover = false,
  onSubmittingChange,
}: AggregateProviderWizardProps) {
  const { t } = useTranslation();
  const isEdit = Boolean(initialProvider);
  const form = useForm<ProviderFormData>({
    defaultValues: {
      name: initialProvider?.name ?? "",
      notes: initialProvider?.notes ?? "",
      websiteUrl: initialProvider?.websiteUrl ?? "",
      icon: initialProvider?.icon ?? "",
      iconColor: initialProvider?.iconColor ?? "",
      settingsConfig: "{}",
    },
    mode: "onSubmit",
  });
  const name = form.watch("name") ?? "";
  const notes = form.watch("notes") ?? "";
  const websiteUrl = form.watch("websiteUrl") ?? "";
  const icon = form.watch("icon") ?? "";

  // ---- 步骤 2：成员 ----
  const [providers, setProviders] = useState<Provider[]>([]);
  const [memberIds, setMemberIds] = useState<string[]>(() =>
    initialProvider
      ? parseAggregateSettings(
          initialProvider.settingsConfig as Record<string, any>,
        ).memberProviderIds
      : [],
  );
  const [expandedMemberIds, setExpandedMemberIds] = useState<
    Record<string, boolean>
  >({});
  const [expandedMappingIds, setExpandedMappingIds] = useState<
    Record<string, boolean>
  >({});

  // ---- 步骤 3：模型 ----
  const [fetchStates, setFetchStates] = useState<Record<string, FetchState>>(
    {},
  );
  const [fetchedModels, setFetchedModels] = useState<Record<string, string[]>>(
    {},
  );
  // 模型元数据：providerId -> 上游模型 id -> 上下文窗口/能力声明
  const [modelMeta, setModelMeta] = useState<
    Record<string, Record<string, AggregateModelMeta>>
  >({});
  const [selectedModels, setSelectedModels] = useState<
    Record<string, Set<string>>
  >({});
  const [manualModels, setManualModels] = useState<Record<string, string[]>>(
    {},
  );
  const [displayNameOverrides, setDisplayNameOverrides] = useState<
    Record<string, string>
  >({});
  const [apiFormatOverrides, setApiFormatOverrides] = useState<
    Record<string, CodexApiFormat>
  >({});
  const [modelSearch, setModelSearch] = useState("");
  const [manualInputs, setManualInputs] = useState<Record<string, string>>({});
  /** 聚合 Provider 的默认模型：写入接管 config.toml 的 model，可在预览步骤选择。 */
  const [defaultModel, setDefaultModel] = useState(() =>
    initialProvider
      ? parseAggregateSettings(
          initialProvider.settingsConfig as Record<string, any>,
        ).defaultModel
      : "",
  );
  /** 聚合 Provider 的默认推理强度：与普通供应商一样写在 config.toml 的 model_reasoning_effort。 */
  const [defaultReasoningEffort, setDefaultReasoningEffort] =
    useState<CodexReasoningEffort>(() =>
      initialProvider
        ? parseAggregateSettings(
            initialProvider.settingsConfig as Record<string, any>,
          ).defaultReasoningEffort
        : "high",
    );

  // 编辑模式直接落在「预览并保存」步骤：用户点编辑立刻看到已保存的成员/模型，
  // 而不是看起来像新建的初始化流程；通过「上一步 / 修改成员与模型」再调整。
  const [step, setStep] = useState(isEdit ? 4 : 1);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const providersLoadedRef = useRef(false);
  const configTouchedRef = useRef(
    typeof (initialProvider?.settingsConfig as Record<string, unknown> | undefined)
      ?.config === "string" &&
      String(
        (initialProvider?.settingsConfig as Record<string, unknown>).config,
      ).trim().length > 0,
  );
  const [codexAuth, setCodexAuth] = useState(() =>
    stringifyAuth(
      (initialProvider?.settingsConfig as Record<string, unknown> | undefined)
        ?.auth,
    ),
  );
  const [codexConfig, setCodexConfig] = useState(() => {
    const settings = (initialProvider?.settingsConfig ?? {}) as Record<
      string,
      unknown
    >;
    const existing = typeof settings.config === "string" ? settings.config : "";
    if (!initialProvider) return existing;
    const parsed = parseAggregateSettings(settings as Record<string, any>);
    return hydrateAggregateConfigToml(
      existing,
      parsed.defaultModel,
      parsed.defaultReasoningEffort,
    );
  });
  const [pricingConfig, setPricingConfig] = useState<{
    enabled: boolean;
    costMultiplier?: string;
    pricingModelSource: PricingModelSourceOption;
  }>(() => ({
    enabled:
      initialProvider?.meta?.costMultiplier !== undefined ||
      initialProvider?.meta?.pricingModelSource !== undefined,
    costMultiplier: initialProvider?.meta?.costMultiplier,
    pricingModelSource: normalizePricingSource(
      initialProvider?.meta?.pricingModelSource,
    ),
  }));

  const { configError: codexConfigError, debouncedValidate, validateToml } =
    useCodexTomlValidation();

  const handleCodexConfigChange = useCallback(
    (value: string) => {
      configTouchedRef.current = true;
      setCodexConfig(value);
      debouncedValidate(value);
      const model = extractCodexModelName(value)?.trim();
      if (model) setDefaultModel(model);
      const effort = extractCodexReasoningEffortFromConfig(value);
      if (effort) setDefaultReasoningEffort(normalizeCodexReasoningEffort(effort));
    },
    [debouncedValidate],
  );

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
    initialData: initialProvider
      ? {
          settingsConfig: initialProvider.settingsConfig as Record<
            string,
            unknown
          >,
        }
      : undefined,
    initialEnabled: initialProvider?.meta?.commonConfigEnabled,
  });

  useEffect(() => {
    onSubmittingChange?.(isSubmitting);
  }, [isSubmitting, onSubmittingChange]);

  // 加载可用供应商（编辑/创建共用）
  useEffect(() => {
    if (providersLoadedRef.current || appId !== "codex") return;
    providersLoadedRef.current = true;
    providersApi
      .getAll("codex")
      .then((map) => {
        const list = Object.values(map).sort((a, b) =>
          a.name.localeCompare(b.name),
        );
        setProviders(list);
      })
      .catch(() => {
        toast.error(
          t("aggregate.providersLoadFailed", {
            defaultValue: "加载供应商列表失败",
          }),
        );
      });
  }, [appId, t]);

  // 编辑模式：剔除已删除的成员供应商（其模型已不可用），避免保存时静默丢失
  // 模型、或全部成员失效时无法保存的死锁。
  useEffect(() => {
    if (providers.length === 0) return;
    setMemberIds((prev) => {
      const stale = prev.filter((id) => !providers.some((p) => p.id === id));
      if (stale.length === 0) return prev;
      toast.warning(
        t("aggregate.staleMembersRemoved", {
          defaultValue: `以下成员供应商已删除，将从聚合中移除: ${stale.join(", ")}`,
          names: stale.join(", "),
        }),
      );
      return prev.filter((id) => providers.some((p) => p.id === id));
    });
  }, [providers, t]);

  // 编辑模式回填
  useEffect(() => {
    if (!initialProvider) return;
    const {
      memberProviderIds,
      models,
      defaultModel: savedDefault,
      defaultReasoningEffort: savedEffort,
    } = parseAggregateSettings(
      initialProvider.settingsConfig as Record<string, any>,
    );
    setMemberIds(memberProviderIds);
    setDefaultModel(savedDefault);
    setDefaultReasoningEffort(savedEffort);
    setCodexConfig((prev) =>
      hydrateAggregateConfigToml(prev, savedDefault, savedEffort),
    );
    const sel: Record<string, Set<string>> = {};
    const manual: Record<string, string[]> = {};
    const overrides: Record<string, string> = {};
    const meta: Record<string, Record<string, AggregateModelMeta>> = {};
    const apiFormats: Record<string, CodexApiFormat> = {};
    for (const model of models) {
      const original =
        model.upstreamModel?.trim() ||
        model.model.replace(/@[^@]+$/, "").trim();
      if (!original) continue;
      const key = `${model.providerId}::${original}`;
      sel[model.providerId] = sel[model.providerId] ?? new Set();
      sel[model.providerId].add(original);
      manual[model.providerId] = manual[model.providerId] ?? [];
      if (!manual[model.providerId].includes(original)) {
        manual[model.providerId].push(original);
      }
      if (model.displayName?.trim()) {
        overrides[key] = model.displayName.trim();
      }
      if (model.apiFormat) {
        apiFormats[key] = model.apiFormat;
      }
      const entry: AggregateModelMeta = {};
      if (model.contextWindow !== undefined && model.contextWindow !== "") {
        entry.contextWindow = model.contextWindow;
      }
      if (model.supportsParallelToolCalls !== undefined) {
        entry.supportsParallelToolCalls = model.supportsParallelToolCalls;
      }
      if (model.inputModalities && model.inputModalities.length > 0) {
        entry.inputModalities = model.inputModalities;
      }
      if (model.baseInstructions?.trim()) {
        entry.baseInstructions = model.baseInstructions.trim();
      }
      if (Object.keys(entry).length > 0) {
        meta[model.providerId] = meta[model.providerId] ?? {};
        meta[model.providerId][original] = {
          ...meta[model.providerId][original],
          ...entry,
        };
      }
    }
    setSelectedModels(sel);
    setManualModels(manual);
    setDisplayNameOverrides(overrides);
    setApiFormatOverrides(apiFormats);
    setModelMeta(meta);
  }, [initialProvider]);

  // 可绑定成员：排除官方 Codex 与其它聚合 Provider。
  const memberCandidates = useMemo(() => {
    return providers.filter((provider) => {
      if (
        provider.id === "codex-official" &&
        provider.category === "official"
      ) {
        return false;
      }
      if (provider.meta?.providerType === "aggregate") {
        return false;
      }
      return true;
    });
  }, [providers]);

  // 从成员配置提取已存模型作为回退列表
  const storedModels = useCallback((provider: Provider): string[] => {
    const config = provider.settingsConfig as Record<string, any>;
    const fromCatalog = Array.isArray(config.modelCatalog?.models)
      ? config.modelCatalog.models
          .map((m: any) => (typeof m?.model === "string" ? m.model.trim() : ""))
          .filter(Boolean)
      : [];
    const fromConfig =
      typeof config.config === "string"
        ? (config.config.match(/^\s*model\s*=\s*"([^"]+)"/m)?.[1] ?? "").trim()
        : "";
    const fromTop =
      typeof config.model === "string" && config.model.trim()
        ? config.model.trim()
        : "";
    return Array.from(
      new Set([...fromCatalog, fromConfig, fromTop].filter(Boolean)),
    );
  }, []);

  const handleFetchMemberModels = useCallback(
    async (providerId: string) => {
      const provider = providers.find((p) => p.id === providerId);
      if (!provider) return;
      setFetchStates((s) => ({ ...s, [providerId]: "loading" }));
      const { baseUrl, apiKey, isFullUrl, customUserAgent } =
        getCodexMemberCredentials(provider);
      const fallback = storedModels(provider);

      if (!baseUrl || !apiKey) {
        setFetchedModels((s) => ({ ...s, [providerId]: fallback }));
        setFetchStates((s) => ({ ...s, [providerId]: "error" }));
        return;
      }

      try {
        const list = await fetchModelsForConfig(
          baseUrl,
          apiKey,
          isFullUrl,
          undefined,
          customUserAgent,
        );
        const ids = list.map((m) => m.id).filter(Boolean);
        setFetchedModels((s) => ({ ...s, [providerId]: ids }));
        setFetchStates((s) => ({
          ...s,
          [providerId]: ids.length > 0 ? "done" : "error",
        }));
        setModelMeta((current) => {
          const previous = current[providerId] ?? {};
          const next = { ...previous };
          let changed = false;
          for (const m of list) {
            if (!m.id) continue;
            if (hasDeclaredContextWindow(next[m.id])) continue;
            const window = resolveMemberModelContextWindow(
              provider,
              m.id,
              m.contextWindow,
            );
            if (window === undefined) continue;
            next[m.id] = { ...next[m.id], contextWindow: window };
            changed = true;
          }
          if (!changed) return current;
          return { ...current, [providerId]: next };
        });
      } catch (err) {
        console.warn(
          `[Aggregate] Failed to fetch models for ${providerId}:`,
          err,
        );
        setFetchedModels((s) => ({ ...s, [providerId]: fallback }));
        setFetchStates((s) => ({ ...s, [providerId]: "error" }));
      }
    },
    [providers, storedModels],
  );

  // 进入步骤 3 时自动拉取所有已选成员模型
  const handleNextFromMembers = useCallback(() => {
    if (memberIds.length === 0) {
      toast.error(
        t("aggregate.needMember", { defaultValue: "请至少选择一个成员供应商" }),
      );
      return;
    }
    setStep(3);
    for (const providerId of memberIds) {
      if (
        fetchStates[providerId] === "idle" ||
        fetchStates[providerId] === undefined
      ) {
        void handleFetchMemberModels(providerId);
      }
    }
  }, [memberIds, fetchStates, handleFetchMemberModels, t]);

  useEffect(() => {
    if (!isEdit || memberIds.length === 0) return;
    for (const providerId of memberIds) {
      if (
        fetchStates[providerId] === "idle" ||
        fetchStates[providerId] === undefined
      ) {
        void handleFetchMemberModels(providerId);
      }
    }
  }, [isEdit, memberIds, fetchStates, handleFetchMemberModels]);

  const toggleMember = useCallback((providerId: string, enabled: boolean) => {
    setMemberIds((current) =>
      enabled
        ? Array.from(new Set([...current, providerId]))
        : current.filter((id) => id !== providerId),
    );
    if (!enabled) {
      setExpandedMemberIds((current) => {
        if (!(providerId in current)) return current;
        const next = { ...current };
        delete next[providerId];
        return next;
      });
      setExpandedMappingIds((current) => {
        if (!(providerId in current)) return current;
        const next = { ...current };
        delete next[providerId];
        return next;
      });
    }
  }, []);

  const availableModelsFor = useCallback(
    (providerId: string): string[] => {
      const fetched = fetchedModels[providerId] ?? [];
      const manual = manualModels[providerId] ?? [];
      return Array.from(new Set([...fetched, ...manual]));
    },
    [fetchedModels, manualModels],
  );

  const toggleModel = useCallback(
    (providerId: string, model: string, enabled: boolean) => {
      setSelectedModels((current) => {
        const next = new Set(current[providerId] ?? []);
        if (enabled) {
          next.add(model);
        } else {
          next.delete(model);
        }
        return { ...current, [providerId]: next };
      });
      if (!enabled) return;
      const provider = providers.find((item) => item.id === providerId);
      const window = resolveMemberModelContextWindow(provider, model);
      if (window === undefined) return;
      setModelMeta((current) => {
        if (hasDeclaredContextWindow(current[providerId]?.[model])) {
          return current;
        }
        return {
          ...current,
          [providerId]: {
            ...(current[providerId] ?? {}),
            [model]: {
              ...(current[providerId]?.[model] ?? {}),
              contextWindow: window,
            },
          },
        };
      });
    },
    [providers],
  );

  const toggleAllForProvider = useCallback(
    (providerId: string, enabled: boolean) => {
      const models = availableModelsFor(providerId);
      setSelectedModels((current) => ({
        ...current,
        [providerId]: enabled ? new Set(models) : new Set(),
      }));
      if (!enabled || models.length === 0) return;
      const provider = providers.find((item) => item.id === providerId);
      setModelMeta((current) => {
        const previous = current[providerId] ?? {};
        const next = { ...previous };
        let changed = false;
        for (const model of models) {
          if (hasDeclaredContextWindow(next[model])) continue;
          const window = resolveMemberModelContextWindow(provider, model);
          if (window === undefined) continue;
          next[model] = { ...next[model], contextWindow: window };
          changed = true;
        }
        if (!changed) return current;
        return { ...current, [providerId]: next };
      });
    },
    [availableModelsFor, providers],
  );

  const addManualModel = useCallback(
    (providerId: string) => {
      const value = (manualInputs[providerId] ?? "").trim();
      if (!value) return;
      setManualModels((current) => ({
        ...current,
        [providerId]: Array.from(
          new Set([...(current[providerId] ?? []), value]),
        ),
      }));
      setSelectedModels((current) => {
        const next = new Set(current[providerId] ?? []);
        next.add(value);
        return { ...current, [providerId]: next };
      });
      setManualInputs((current) => ({ ...current, [providerId]: "" }));
      const provider = providers.find((item) => item.id === providerId);
      const window = resolveMemberModelContextWindow(provider, value);
      if (window === undefined) return;
      setModelMeta((current) => {
        if (hasDeclaredContextWindow(current[providerId]?.[value])) {
          return current;
        }
        return {
          ...current,
          [providerId]: {
            ...(current[providerId] ?? {}),
            [value]: {
              ...(current[providerId]?.[value] ?? {}),
              contextWindow: window,
            },
          },
        };
      });
    },
    [manualInputs, providers],
  );

  const removeManualModel = useCallback((providerId: string, model: string) => {
    setManualModels((current) => ({
      ...current,
      [providerId]: (current[providerId] ?? []).filter((m) => m !== model),
    }));
    setSelectedModels((current) => {
      const next = new Set(current[providerId] ?? []);
      next.delete(model);
      return { ...current, [providerId]: next };
    });
  }, []);

  // 编辑回填 / 成员目录异步就绪后，给已选且仍缺窗口的模型补上预设上下文。
  // 已有用户填写或已保存的值不覆盖。
  useEffect(() => {
    if (providers.length === 0) return;
    setModelMeta((current) => {
      let changed = false;
      const next: Record<string, Record<string, AggregateModelMeta>> = {
        ...current,
      };
      for (const providerId of memberIds) {
        const selected = selectedModels[providerId];
        if (!selected || selected.size === 0) continue;
        const provider = providers.find((item) => item.id === providerId);
        const bucket = { ...(next[providerId] ?? {}) };
        for (const model of selected) {
          if (hasDeclaredContextWindow(bucket[model])) continue;
          const window = resolveMemberModelContextWindow(provider, model);
          if (window === undefined) continue;
          bucket[model] = { ...bucket[model], contextWindow: window };
          changed = true;
        }
        next[providerId] = bucket;
      }
      return changed ? next : current;
    });
  }, [providers, memberIds, selectedModels]);

  // 按当前选择生成模型映射（用于步骤 3 预览/步骤 4 保存），保留真实模型 ID。
  const preview = useMemo<{ models: AggregateProviderModel[] }>(() => {
    const selected = memberIds.flatMap((providerId) => {
      const provider = providers.find((p) => p.id === providerId);
      if (!provider) return [];
      const models = Array.from(selectedModels[providerId] ?? []);
      return models.length > 0 ? [{ provider, models }] : [];
    });
    const raw = buildAggregateModels(selected);
    const metaByKey: Record<string, AggregateModelMeta> = {};
    for (const { provider, models } of selected) {
      const config = provider.settingsConfig as Record<string, any>;
      const catalogModels = Array.isArray(config.modelCatalog?.models)
        ? (config.modelCatalog.models as Record<string, any>[])
        : [];
      for (const model of models) {
        const meta = modelMeta[provider.id]?.[model];
        const catalogMeta = aggregateMetaFromCatalogEntry(
          catalogModels.find((entry) => entry?.model === model),
        );
        const knownWindow = knownCodexContextWindow(model);
        if (meta || catalogMeta || knownWindow) {
          metaByKey[`${provider.id}::${model}`] = {
            ...(knownWindow ? { contextWindow: knownWindow } : {}),
            ...(catalogMeta ?? {}),
            ...(meta ?? {}),
          };
        }
      }
    }
    const withMeta = applyAggregateModelMeta(raw, metaByKey);
    const models = withMeta.map((m) => {
      const key = `${m.providerId}::${m.upstreamModel}`;
      const displayName = displayNameOverrides[key];
      const apiFormat = apiFormatOverrides[key];
      return {
        ...m,
        ...(apiFormat ? { apiFormat } : {}),
        ...(displayName?.trim() ? { displayName: displayName.trim() } : {}),
      };
    });
    return { models };
  }, [
    memberIds,
    providers,
    selectedModels,
    displayNameOverrides,
    apiFormatOverrides,
    modelMeta,
  ]);

  const builtModels = preview.models;

  // 默认模型跟随模型列表：选择变化后若当前默认模型已不在列表中，回退到首项。
  // 模型列表尚未就绪（编辑回填后成员/供应商异步加载中）时不重置，否则会把
  // 已保存的默认模型先清空、再在列表就绪时错误回退到首项。
  useEffect(() => {
    const ids = builtModels.map((model) => model.model);
    if (ids.length === 0) return;
    setDefaultModel((current) => {
      const next = current && ids.includes(current) ? current : (ids[0] ?? "");
      if (next && next !== current && configTouchedRef.current) {
        setCodexConfig((prev) => setCodexModelName(prev, next));
      }
      return next;
    });
  }, [builtModels]);

  const generatedConfig = useMemo(() => {
    return String(
      buildAggregateSettingsConfig(
        builtModels,
        memberIds,
        defaultModel,
        defaultReasoningEffort,
      ).config ?? "",
    );
  }, [builtModels, memberIds, defaultModel, defaultReasoningEffort]);

  useEffect(() => {
    if (configTouchedRef.current) return;
    if (!generatedConfig.trim()) return;
    setCodexConfig(generatedConfig);
  }, [generatedConfig]);

  const selectedCount = useMemo(
    () =>
      memberIds.reduce(
        (sum, providerId) => sum + (selectedModels[providerId]?.size ?? 0),
        0,
      ),
    [memberIds, selectedModels],
  );

  const handleSave = useCallback(async () => {
    if (builtModels.length === 0) {
      toast.error(
        t("aggregate.needModel", { defaultValue: "请至少选择一个模型" }),
      );
      return;
    }
    if (!name.trim()) {
      toast.error(
        t("aggregate.needName", { defaultValue: "请填写聚合 Provider 名称" }),
      );
      setStep(1);
      return;
    }
    // 聚合 Provider 的 id 由名称生成：同名会导致 id 冲突/覆盖，先拦截。
    const generatedId = generateAggregateProviderId(name);
    const duplicateId = providers.some(
      (p) => p.id === generatedId && p.id !== initialProvider?.id,
    );
    if (duplicateId) {
      toast.error(
        t("aggregate.duplicateName", {
          defaultValue: "已存在同名的聚合 Provider，请更换名称",
        }),
      );
      return;
    }
    const normalized = normalizeAggregateModelsForSave(builtModels);

    let auth: unknown = {};
    try {
      auth = JSON.parse(codexAuth);
    } catch {
      toast.error(
        t("codexConfig.authJsonInvalid", {
          defaultValue: "Auth JSON 格式无效",
        }),
      );
      return;
    }
    if (codexConfigError || !validateToml(codexConfig)) {
      toast.error(
        t("codexConfig.configTomlInvalid", {
          defaultValue: "config.toml 格式无效",
        }),
      );
      return;
    }

    const resolvedEffort = normalizeCodexReasoningEffort(defaultReasoningEffort);
    const settingsConfig = buildAggregateSettingsConfig(
      normalized,
      memberIds,
      defaultModel,
      resolvedEffort,
      { config: codexConfig, auth },
    );
    const meta: Provider["meta"] = {
      ...(initialProvider?.meta ?? {}),
      providerType: "aggregate",
      apiFormat: "openai_responses",
      commonConfigEnabled: useCodexCommonConfigFlag,
      costMultiplier: pricingConfig.enabled
        ? pricingConfig.costMultiplier
        : undefined,
      pricingModelSource: pricingConfig.enabled
        ? pricingConfig.pricingModelSource
        : undefined,
    };
    const base = {
      name: name.trim(),
      notes: notes.trim() || undefined,
      websiteUrl: websiteUrl.trim() || undefined,
      settingsConfig,
      meta,
      icon: icon.trim() || undefined,
    };

    setIsSubmitting(true);
    try {
      if (isEdit && initialProvider && onEdit) {
        await onEdit({
          provider: { ...base, id: initialProvider.id } as Provider,
          originalId: initialProvider.id,
        });
      } else {
        await onAdd({
          ...base,
          providerKey: generateAggregateProviderId(name),
        } as Omit<Provider, "id"> & { providerKey?: string });
      }
    } catch {
      // 错误提示由外层 mutation 处理
    } finally {
      setIsSubmitting(false);
    }
  }, [
    builtModels,
    memberIds,
    defaultModel,
    defaultReasoningEffort,
    name,
    notes,
    websiteUrl,
    icon,
    initialProvider,
    isEdit,
    onAdd,
    onEdit,
    t,
    providers,
    codexAuth,
    codexConfig,
    codexConfigError,
    validateToml,
    useCodexCommonConfigFlag,
    pricingConfig,
  ]);

  const selectedMemberProviders = useMemo(
    () =>
      memberIds
        .map((id) => providers.find((p) => p.id === id))
        .filter(Boolean) as Provider[],
    [memberIds, providers],
  );

  const isRecordExpanded = (map: Record<string, boolean>, id: string) =>
    map[id] === true;

  const canGoNext =
    (step === 1 && name.trim().length > 0) ||
    (step === 2 && memberIds.length > 0) ||
    (step === 3 && selectedCount > 0);

  const filteredModelsFor = useCallback(
    (providerId: string): string[] => {
      const all = availableModelsFor(providerId);
      const query = modelSearch.trim().toLowerCase();
      if (!query) return all;
      return all.filter((model) => model.toLowerCase().includes(query));
    },
    [availableModelsFor, modelSearch],
  );

  const handleDefaultModelChange = useCallback((value: string) => {
    setDefaultModel(value);
    configTouchedRef.current = true;
    setCodexConfig((prev) => setCodexModelName(prev, value));
  }, []);

  const handleDefaultReasoningEffortChange = useCallback((value: string) => {
    const effort = normalizeCodexReasoningEffort(value);
    setDefaultReasoningEffort(effort);
    configTouchedRef.current = true;
    setCodexConfig((prev) => setCodexReasoningEffortInConfig(prev, effort));
  }, []);

  const defaultModelOptions = useMemo(() => {
    const models = [...builtModels];
    if (defaultModel && !models.some((model) => model.model === defaultModel)) {
      models.unshift({
        model: defaultModel,
        providerId: "",
      });
    }
    return models;
  }, [builtModels, defaultModel]);

  const configEditor = (
    <CodexConfigEditor
      authValue={codexAuth}
      configValue={codexConfig}
      providerName={name}
      showRemoteCompaction
      isProxyTakeover={isProxyTakeover}
      onAuthChange={setCodexAuth}
      onConfigChange={handleCodexConfigChange}
      useCommonConfig={useCodexCommonConfigFlag}
      onCommonConfigToggle={handleCodexCommonConfigToggle}
      commonConfigSnippet={codexCommonConfigSnippet}
      onCommonConfigSnippetChange={handleCodexCommonConfigSnippetChange}
      onCommonConfigErrorClear={clearCodexCommonConfigError}
      commonConfigError={codexCommonConfigError}
      authError=""
      configError={codexConfigError}
      onExtract={handleCodexExtract}
      isExtracting={isCodexExtracting}
      showAuth={false}
    />
  );

  const advancedConfig = (
    <ProviderAdvancedConfig
      pricingConfig={pricingConfig}
      onPricingConfigChange={setPricingConfig}
    />
  );

  const mappingGridClass =
    "md:grid-cols-[minmax(0,1.2fr)_112px_minmax(0,1fr)_minmax(0,1fr)]";

  const renderProviderMappingTable = (provider: Provider) => {
    const groupModels = builtModels.filter(
      (model) => model.providerId === provider.id,
    );
    if (groupModels.length === 0) return null;
    return (
      <div className="space-y-2">
        <div
          className={cn(
            "hidden gap-2 px-1 text-xs font-medium text-muted-foreground md:grid",
            mappingGridClass,
          )}
        >
          <span>
            {t("codexConfig.catalogColumnDisplay", {
              defaultValue: "菜单显示名",
            })}
          </span>
          <span>
            {t("codexConfig.catalogColumnContext", {
              defaultValue: "上下文窗口",
            })}
          </span>
          <span>
            {t("codexConfig.catalogColumnModel", {
              defaultValue: "实际请求模型",
            })}
          </span>
          <span>
            {t("codexConfig.upstreamFormatLabel", {
              defaultValue: "上游格式",
            })}
          </span>
        </div>
        {groupModels.map((model, index) => {
          const key = `${model.providerId}::${model.upstreamModel}`;
          const upstreamModel = model.upstreamModel ?? model.model;
          const contextWindowValue =
            model.contextWindow === undefined || model.contextWindow === ""
              ? ""
              : String(model.contextWindow);
          return (
            <div
              key={`${key}-${index}`}
              className={cn("grid grid-cols-1 gap-2", mappingGridClass)}
            >
              <Input
                value={
                  displayNameOverrides[key] ??
                  model.displayName ??
                  model.model
                }
                onChange={(e) =>
                  setDisplayNameOverrides((s) => ({
                    ...s,
                    [key]: e.target.value,
                  }))
                }
                placeholder={t("codexConfig.catalogDisplayNamePlaceholder", {
                  defaultValue: "例如: DeepSeek V4 Flash",
                })}
                aria-label={t("codexConfig.catalogColumnDisplay", {
                  defaultValue: "菜单显示名",
                })}
              />
              <Input
                type="text"
                inputMode="numeric"
                value={contextWindowValue}
                onChange={(event) => {
                  const next = event.target.value.replace(/[^\d]/g, "");
                  setModelMeta((current) => {
                    const existing =
                      current[model.providerId]?.[upstreamModel] ?? {};
                    const rest = { ...existing };
                    delete rest.contextWindow;
                    return {
                      ...current,
                      [model.providerId]: {
                        ...(current[model.providerId] ?? {}),
                        [upstreamModel]: next
                          ? {
                              ...existing,
                              contextWindow: Number(next),
                            }
                          : rest,
                      },
                    };
                  });
                }}
                placeholder={t("codexConfig.contextWindowPlaceholder", {
                  defaultValue: "例如: 128000",
                })}
                aria-label={t("codexConfig.catalogColumnContext", {
                  defaultValue: "上下文窗口",
                })}
              />
              <Input
                value={upstreamModel}
                readOnly
                className="min-w-0"
                aria-label={t("codexConfig.catalogColumnModel", {
                  defaultValue: "实际请求模型",
                })}
              />
              <div className="min-w-0">
                <Select
                  value={
                    apiFormatOverrides[key] ??
                    getAggregateModelApiFormat(provider)
                  }
                  onValueChange={(value) => {
                    setApiFormatOverrides((current) => {
                      if (value === getAggregateModelApiFormat(provider)) {
                        const next = { ...current };
                        delete next[key];
                        return next;
                      }
                      return {
                        ...current,
                        [key]: value as CodexApiFormat,
                      };
                    });
                  }}
                >
                  <SelectTrigger
                    aria-label={t("codexConfig.upstreamFormatLabel", {
                      defaultValue: "上游格式",
                    })}
                    title={t("codexConfig.upstreamFormatHint", {
                      defaultValue:
                        "供应商原生为 Responses API 就选 Responses（直连，不转换格式）；使用 Chat Completions 协议就选 Chat；供应商只提供原生 Anthropic Messages 协议就选 Anthropic Messages。Chat 与 Anthropic Messages 均需开启路由接管才能转换为 Responses。",
                    })}
                    className="w-full min-w-0 overflow-hidden [&>span]:min-w-0 [&>span]:flex-1 [&>span]:truncate [&>svg]:shrink-0"
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="openai_responses">
                      {t("aggregate.upstreamFormatShortResponses", {
                        defaultValue: "Responses",
                      })}
                    </SelectItem>
                    <SelectItem value="openai_chat">
                      {t("aggregate.upstreamFormatShortChat", {
                        defaultValue: "Chat Completions",
                      })}
                    </SelectItem>
                    <SelectItem value="anthropic">
                      {t("aggregate.upstreamFormatShortAnthropic", {
                        defaultValue: "Anthropic Messages",
                      })}
                    </SelectItem>
                  </SelectContent>
                </Select>
              </div>
            </div>
          );
        })}
      </div>
    );
  };

  const formClassName = "space-y-6 glass rounded-xl p-6 border border-white/10";

  const sections = (
        <div className="w-full space-y-6">
          {(isEdit || step === 1) && <BasicFormFields form={form} />}

          {/* 步骤 2：选择成员 */}
          {(isEdit || step === 2) && (
            <div className="space-y-1.5">
              <FormLabel>
                {t("aggregate.step.members", { defaultValue: "选择成员" })}
              </FormLabel>
              {memberCandidates.length === 0 ? (
                <p className="text-sm text-muted-foreground">
                  {t("aggregate.noMembers", {
                    defaultValue:
                      "还没有可用的 Codex 供应商，请先添加普通供应商。",
                  })}
                </p>
              ) : (
                <div className="divide-y divide-border-default rounded-lg border border-border-default">
                  {memberCandidates.map((provider) => {
                    const wireApi = getCodexMemberWireApi(provider);
                    const checked = memberIds.includes(provider.id);
                    const { baseUrl } = getCodexMemberCredentials(provider);
                    return (
                      <label
                        key={provider.id}
                        htmlFor={`member-${provider.id}`}
                        className="flex cursor-pointer items-center gap-3 px-3 py-2.5 hover:bg-muted/30"
                      >
                        <Checkbox
                          id={`member-${provider.id}`}
                          checked={checked}
                          onCheckedChange={(v) =>
                            toggleMember(provider.id, Boolean(v))
                          }
                        />
                        <span className="min-w-0 flex-1 truncate text-sm">
                          {provider.name}
                        </span>
                        <span className="shrink-0 text-xs text-muted-foreground">
                          {wireApi}
                          {baseUrl ? ` · ${baseUrl}` : ""}
                        </span>
                      </label>
                    );
                  })}
                </div>
              )}
            </div>
          )}

          {(isEdit || step === 3) && (
            <div className="space-y-4">
              <div className="flex items-center justify-between gap-3">
                <FormLabel>
                  {t("aggregate.step.models", { defaultValue: "选择模型" })}
                </FormLabel>
                <Input
                  value={modelSearch}
                  onChange={(e) => setModelSearch(e.target.value)}
                  placeholder={t("aggregate.searchModels", {
                    defaultValue: "搜索模型…",
                  })}
                  className="h-9 max-w-[220px]"
                />
              </div>

              {selectedMemberProviders.length === 0 ? (
                <p className="text-sm text-muted-foreground">
                  {t("aggregate.noSelectedMembers", {
                    defaultValue: "请先选择成员供应商",
                  })}
                </p>
              ) : (
                selectedMemberProviders.map((provider) => {
                  const state = fetchStates[provider.id] ?? "idle";
                  const available = availableModelsFor(provider.id);
                  const filtered = filteredModelsFor(provider.id);
                  const selected =
                    selectedModels[provider.id] ?? new Set<string>();
                  const expanded = isRecordExpanded(
                    expandedMemberIds,
                    provider.id,
                  );
                  return (
                    <Collapsible
                      key={provider.id}
                      open={expanded}
                      onOpenChange={(open) =>
                        setExpandedMemberIds((prev) => ({
                          ...prev,
                          [provider.id]: open,
                        }))
                      }
                      className="space-y-3 rounded-lg border border-border-default p-3"
                    >
                      <div className="flex flex-wrap items-center justify-between gap-2">
                        <CollapsibleTrigger asChild>
                          <Button
                            type="button"
                            variant={null}
                            size="sm"
                            className="h-8 min-w-0 flex-1 justify-start gap-1.5 px-0 text-sm font-medium text-foreground hover:opacity-70"
                          >
                            {expanded ? (
                              <ChevronDown className="h-4 w-4 shrink-0" />
                            ) : (
                              <ChevronRight className="h-4 w-4 shrink-0" />
                            )}
                            <span className="min-w-0 truncate">
                              {provider.name}
                            </span>
                            <span className="shrink-0 text-xs font-normal text-muted-foreground">
                              {t("aggregate.selectedCount", {
                                count: selected.size,
                                defaultValue: "已选 {{count}} 个模型",
                              })}
                            </span>
                          </Button>
                        </CollapsibleTrigger>
                        <div className="flex gap-1">
                          <Button
                            type="button"
                            variant="outline"
                            size="sm"
                            className="h-7 gap-1"
                            disabled={state === "loading"}
                            onClick={() =>
                              void handleFetchMemberModels(provider.id)
                            }
                          >
                            {state === "loading" ? (
                              <Loader2 className="h-3.5 w-3.5 animate-spin" />
                            ) : (
                              <RefreshCw className="h-3.5 w-3.5" />
                            )}
                            {t("providerForm.fetchModels")}
                          </Button>
                          <Button
                            type="button"
                            variant="outline"
                            size="sm"
                            className="h-7 gap-1"
                            disabled={available.length === 0}
                            onClick={() =>
                              toggleAllForProvider(
                                provider.id,
                                selected.size !== available.length,
                              )
                            }
                          >
                            {selected.size === available.length &&
                            available.length > 0
                              ? t("aggregate.deselectAll", {
                                  defaultValue: "取消全选",
                                })
                              : t("aggregate.selectAll", {
                                  defaultValue: "全选",
                                })}
                          </Button>
                        </div>
                      </div>

                      <CollapsibleContent className="space-y-3">
                        {filtered.length === 0 ? (
                          <p className="text-sm text-muted-foreground">
                            {t("aggregate.noModelsForMember", {
                              defaultValue:
                                "没有可用模型，可手动添加模型 ID。",
                            })}
                          </p>
                        ) : (
                          <div className="grid grid-cols-1 gap-1 sm:grid-cols-2">
                            {filtered.map((model) => {
                              const isManual = (
                                manualModels[provider.id] ?? []
                              ).includes(model);
                              return (
                                <label
                                  key={model}
                                  className="flex cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-sm hover:bg-muted/40"
                                >
                                  <Checkbox
                                    checked={selected.has(model)}
                                    onCheckedChange={(v) =>
                                      toggleModel(
                                        provider.id,
                                        model,
                                        Boolean(v),
                                      )
                                    }
                                  />
                                  <span className="min-w-0 flex-1 truncate">
                                    {model}
                                  </span>
                                  {isManual && (
                                    <button
                                      type="button"
                                      className="text-muted-foreground hover:text-destructive"
                                      title={t("common.delete", {
                                        defaultValue: "删除",
                                      })}
                                      onClick={(e) => {
                                        e.preventDefault();
                                        removeManualModel(provider.id, model);
                                      }}
                                    >
                                      <X className="h-3.5 w-3.5" />
                                    </button>
                                  )}
                                </label>
                              );
                            })}
                          </div>
                        )}

                        <div className="flex gap-1">
                          <Input
                            value={manualInputs[provider.id] ?? ""}
                            onChange={(e) =>
                              setManualInputs((s) => ({
                                ...s,
                                [provider.id]: e.target.value,
                              }))
                            }
                            onKeyDown={(e) => {
                              if (e.key === "Enter") {
                                e.preventDefault();
                                addManualModel(provider.id);
                              }
                            }}
                            placeholder={t(
                              "codexConfig.catalogModelPlaceholder",
                              {
                                defaultValue: "例如: deepseek-v4-flash",
                              },
                            )}
                            className="flex-1"
                          />
                          <Button
                            type="button"
                            variant="outline"
                            size="sm"
                            className="h-9 gap-1"
                            onClick={() => addManualModel(provider.id)}
                          >
                            <Plus className="h-3.5 w-3.5" />
                            {t("codexConfig.addCatalogModel", {
                              defaultValue: "添加模型",
                            })}
                          </Button>
                        </div>
                      </CollapsibleContent>
                    </Collapsible>
                  );
                })
              )}
            </div>
          )}

          {(isEdit || step === 4) && (
            <div className="space-y-4">
              {builtModels.length > 0 && (
                <div className="space-y-3">
                  <FormLabel>
                    {t("aggregate.selectedModelsHeading", {
                      defaultValue: "已选设置",
                    })}
                  </FormLabel>
                  <div className="divide-y divide-border-default rounded-lg border border-border-default bg-muted/20">
                    {selectedMemberProviders.map((provider) => {
                      const groupModels = builtModels.filter(
                        (model) => model.providerId === provider.id,
                      );
                      if (groupModels.length === 0) return null;
                      const expanded = isRecordExpanded(
                        expandedMappingIds,
                        provider.id,
                      );
                      return (
                        <Collapsible
                          key={provider.id}
                          open={expanded}
                          onOpenChange={(open) =>
                            setExpandedMappingIds((prev) => ({
                              ...prev,
                              [provider.id]: open,
                            }))
                          }
                        >
                          <CollapsibleTrigger asChild>
                            <button
                              type="button"
                              className="flex w-full items-center gap-2 px-3 py-2.5 text-left text-sm hover:bg-muted/40"
                            >
                              {expanded ? (
                                <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground" />
                              ) : (
                                <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" />
                              )}
                              <span className="min-w-0 flex-1 truncate font-medium">
                                {provider.name}
                              </span>
                              <span className="shrink-0 text-xs text-muted-foreground">
                                {t("aggregate.modelCount", {
                                  count: groupModels.length,
                                  defaultValue: "{{count}} 个模型",
                                })}
                              </span>
                            </button>
                          </CollapsibleTrigger>
                          <CollapsibleContent className="px-3 pb-3">
                            {renderProviderMappingTable(provider)}
                          </CollapsibleContent>
                        </Collapsible>
                      );
                    })}
                  </div>
                </div>
              )}

              <div className="grid gap-4 sm:grid-cols-2">
              <div className="space-y-1.5">
                <FormLabel htmlFor="aggregate-default-model">
                  {t("codexConfig.defaultModelLabel", {
                    defaultValue: "默认模型",
                  })}
                </FormLabel>
                <Select
                  value={defaultModel}
                  onValueChange={handleDefaultModelChange}
                  disabled={builtModels.length === 0}
                >
                  <SelectTrigger id="aggregate-default-model" className="w-full">
                    <SelectValue
                      placeholder={t("codexConfig.defaultModelPlaceholder", {
                        defaultValue: "例如: gpt-5.6",
                      })}
                    />
                  </SelectTrigger>
                  <SelectContent>
                    {defaultModelOptions.map((model) => (
                      <SelectItem key={model.model} value={model.model}>
                        {model.displayName &&
                        model.displayName !== model.model
                          ? `${model.displayName} (${model.model})`
                          : model.model}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              <div className="space-y-1.5">
                <FormLabel htmlFor="aggregate-reasoning-effort">
                  {t("aggregate.defaultReasoningEffort", {
                    defaultValue: "默认推理强度",
                  })}
                </FormLabel>
                <Select
                  value={defaultReasoningEffort}
                  onValueChange={handleDefaultReasoningEffortChange}
                >
                  <SelectTrigger
                    id="aggregate-reasoning-effort"
                    className="w-full"
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {CODEX_REASONING_EFFORTS.map((effort) => (
                      <SelectItem key={effort} value={effort}>
                        {effort}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              </div>

              <div>
                {configEditor}
              </div>
              {advancedConfig}
            </div>
          )}
        </div>
  );

  if (isEdit) {
    return (
      <Form {...form}>
        <form
          id={AGGREGATE_FORM_ID}
          onSubmit={(event) => {
            event.preventDefault();
            void handleSave();
          }}
          className={formClassName}
        >
          {sections}
        </form>
      </Form>
    );
  }

  return (
    <Form {...form}>
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <div className="mb-6 mt-2 flex shrink-0 justify-center">
        <div
          data-testid="aggregate-wizard-stepper"
          className="flex w-full items-start justify-center"
        >
          {STEPS.map((s, index) => {
            const active = step === s.id;
            const done = step > s.id;
            return (
              <Fragment key={s.id}>
                <div className="flex min-w-0 flex-1 flex-col items-center text-center">
                  <div
                    className={cn(
                      "flex h-7 w-7 shrink-0 items-center justify-center rounded-full text-xs font-medium",
                      done && "bg-primary text-primary-foreground",
                      active &&
                        "bg-primary/15 text-primary ring-1 ring-primary",
                      !done && !active && "bg-muted text-muted-foreground",
                    )}
                  >
                    {done ? <Check className="h-3.5 w-3.5" /> : s.id}
                  </div>
                  <div
                    className={cn(
                      "mt-1.5 min-h-8 break-words text-xs leading-4",
                      active
                        ? "font-medium text-foreground"
                        : "text-muted-foreground",
                    )}
                  >
                    {t(`aggregate.step.${s.key}`, {
                      defaultValue: s.fallbackLabel,
                    })}
                  </div>
                </div>
                {index < STEPS.length - 1 && (
                  <div
                    aria-hidden="true"
                    className={cn(
                      "mt-3.5 h-px min-w-2 flex-1",
                      step > s.id ? "bg-primary" : "bg-border",
                    )}
                  />
                )}
              </Fragment>
            );
          })}
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto scroll-overlay">
        {sections}
      </div>

      {showButtons && (
        <div className="flex shrink-0 items-center justify-between gap-3 border-t border-border-default bg-background py-4">
          <Button variant="outline" disabled={isSubmitting} onClick={onCancel}>
            {t("common.cancel", { defaultValue: "取消" })}
          </Button>
          <div className="flex items-center gap-2">
            {step > 1 && (
              <Button
                variant="ghost"
                disabled={isSubmitting}
                onClick={() => setStep((s) => s - 1)}
              >
                <ChevronLeft className="mr-1 h-4 w-4" />
                {t("aggregate.previous", { defaultValue: "上一步" })}
              </Button>
            )}
            {step < 4 ? (
              <Button
                disabled={!canGoNext}
                onClick={
                  step === 2
                    ? handleNextFromMembers
                    : () => setStep((s) => s + 1)
                }
              >
                {t("aggregate.next", { defaultValue: "下一步" })}
                <ChevronRight className="ml-1 h-4 w-4" />
              </Button>
            ) : (
              <Button disabled={isSubmitting} onClick={() => void handleSave()}>
                {isSubmitting ? (
                  <Loader2 className="mr-1 h-4 w-4 animate-spin" />
                ) : (
                  <Save className="mr-1 h-4 w-4" />
                )}
                {t("common.add", { defaultValue: "添加" })}
              </Button>
            )}
          </div>
        </div>
      )}
    </div>
    </Form>
  );
}
