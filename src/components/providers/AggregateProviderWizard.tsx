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
  AlertTriangle,
  Check,
  ChevronLeft,
  ChevronRight,
  Layers,
  Loader2,
  Plus,
  RefreshCw,
  Save,
  Search,
  X,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Badge } from "@/components/ui/badge";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { IconPicker } from "@/components/IconPicker";
import { cn } from "@/lib/utils";
import { providersApi, type AppId } from "@/lib/api";
import { fetchModelsForConfig } from "@/lib/api/model-fetch";
import type { AggregateProviderModel, CodexApiFormat, Provider } from "@/types";
import {
  type AggregateModelMeta,
  aggregateMetaFromCatalogEntry,
  applyAggregateModelMeta,
  buildAggregateConfigTomlPreview,
  buildAggregateModels,
  buildAggregateSettingsConfig,
  generateAggregateProviderId,
  getAggregateModelApiFormat,
  getCodexMemberCredentials,
  getCodexMemberWireApi,
  normalizeAggregateModelsForSave,
  parseAggregateSettings,
} from "@/utils/aggregateProvider";

const STEPS = [
  { id: 1, key: "basic", fallbackLabel: "基本信息" },
  { id: 2, key: "members", fallbackLabel: "选择成员" },
  { id: 3, key: "models", fallbackLabel: "选择模型" },
  { id: 4, key: "preview", fallbackLabel: "预览保存" },
] as const;

type FetchState = "idle" | "loading" | "done" | "error";

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
}

export function AggregateProviderWizard({
  appId,
  initialProvider,
  onAdd,
  onEdit,
  onCancel,
}: AggregateProviderWizardProps) {
  const { t } = useTranslation();
  const isEdit = Boolean(initialProvider);

  // ---- 步骤 1：基本信息 ----
  const [name, setName] = useState(initialProvider?.name ?? "");
  const [icon, setIcon] = useState(initialProvider?.icon ?? "");
  const [notes, setNotes] = useState(initialProvider?.notes ?? "");
  const [websiteUrl, setWebsiteUrl] = useState(
    initialProvider?.websiteUrl ?? "",
  );

  // ---- 步骤 2：成员 ----
  const [providers, setProviders] = useState<Provider[]>([]);
  const [memberIds, setMemberIds] = useState<string[]>([]);

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
  const [defaultModel, setDefaultModel] = useState("");

  // 编辑模式直接落在「预览并保存」步骤：用户点编辑立刻看到已保存的成员/模型，
  // 而不是看起来像新建的初始化流程；通过「上一步 / 修改成员与模型」再调整。
  const [step, setStep] = useState(isEdit ? 4 : 1);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const providersLoadedRef = useRef(false);

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
    const { memberProviderIds, models, defaultModel: savedDefault } =
      parseAggregateSettings(
      initialProvider.settingsConfig as Record<string, any>,
      );
    setMemberIds(memberProviderIds);
    setDefaultModel(savedDefault);
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
        const meta: Record<string, AggregateModelMeta> = {};
        for (const m of list) {
          if (
            m.id &&
            typeof m.contextWindow === "number" &&
            m.contextWindow > 0
          ) {
            meta[m.id] = { contextWindow: m.contextWindow };
          }
        }
        if (Object.keys(meta).length > 0) {
          setModelMeta((current) => ({
            ...current,
            [providerId]: { ...(current[providerId] ?? {}), ...meta },
          }));
        }
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

  const toggleMember = useCallback((providerId: string, enabled: boolean) => {
    setMemberIds((current) =>
      enabled
        ? Array.from(new Set([...current, providerId]))
        : current.filter((id) => id !== providerId),
    );
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
    },
    [],
  );

  const toggleAllForProvider = useCallback(
    (providerId: string, enabled: boolean) => {
      const models = availableModelsFor(providerId);
      setSelectedModels((current) => ({
        ...current,
        [providerId]: enabled ? new Set(models) : new Set(),
      }));
    },
    [availableModelsFor],
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
    },
    [manualInputs],
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
        if (meta || catalogMeta) {
          metaByKey[`${provider.id}::${model}`] = {
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
    setDefaultModel((current) =>
      current && ids.includes(current) ? current : (ids[0] ?? ""),
    );
  }, [builtModels]);

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

    const settingsConfig = buildAggregateSettingsConfig(
      normalized,
      memberIds,
      defaultModel,
    );
    const meta: Provider["meta"] = {
      ...(initialProvider?.meta ?? {}),
      providerType: "aggregate",
      apiFormat: "openai_responses",
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
    name,
    notes,
    websiteUrl,
    icon,
    initialProvider,
    isEdit,
    onAdd,
    onEdit,
    t,
  ]);

  const selectedMemberProviders = useMemo(
    () =>
      memberIds
        .map((id) => providers.find((p) => p.id === id))
        .filter(Boolean) as Provider[],
    [memberIds, providers],
  );

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

  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      {/* 步骤指示器：节点与标签垂直排列，连线独立于标签布局，避免重叠。 */}
      <div className="mb-6 mt-2 flex shrink-0 justify-center">
        <div
          data-testid="aggregate-wizard-stepper"
          className="flex w-full max-w-3xl items-start justify-center"
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

      {isEdit && (
        <div className="mb-4 rounded-lg border border-primary/25 bg-primary/5 px-3 py-2.5 text-xs text-foreground">
          <span className="font-medium">
            {t("aggregate.editModeHint", {
              name,
              defaultValue:
                "编辑模式：已载入「{{name}}」的成员与模型，可直接修改或保存。",
            })}
          </span>
          <span className="ml-2 text-muted-foreground">
            {t("aggregate.editModeHintSub", {
              defaultValue:
                "增减成员请进入「选择成员」，增减模型请进入「选择模型」。",
            })}
          </span>
        </div>
      )}

      <ScrollArea className="min-h-0 flex-1 pr-3">
        <div className="mx-auto w-full max-w-5xl">
          {/* 步骤 1：基本信息 */}
          {step === 1 && (
            <div className="space-y-4">
              <div className="rounded-lg border border-border/60 bg-muted/30 p-3 text-xs text-muted-foreground">
                {t("aggregate.basicHint", {
                  defaultValue:
                    "聚合 Provider 会把多个第三方 Codex 供应商聚合成一个虚拟供应商；每个模型可独立选择上游 API 协议。",
                })}
              </div>
              <div className="space-y-4 rounded-lg border border-border/60 bg-card p-4">
                <div className="space-y-2">
                  <Label htmlFor="aggregate-name">
                    {t("aggregate.name", { defaultValue: "名称" })}
                    <span className="text-destructive"> *</span>
                  </Label>
                  <Input
                    id="aggregate-name"
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                    placeholder={t("aggregate.namePlaceholder", {
                      defaultValue: "例如：我的多供应商聚合",
                    })}
                  />
                </div>
                <div className="space-y-2">
                  <Label>{t("aggregate.icon", { defaultValue: "图标" })}</Label>
                  <IconPicker value={icon} onValueChange={setIcon} />
                </div>
                <div className="space-y-2">
                  <Label htmlFor="aggregate-notes">
                    {t("aggregate.notes", { defaultValue: "备注" })}
                  </Label>
                  <Input
                    id="aggregate-notes"
                    value={notes}
                    onChange={(e) => setNotes(e.target.value)}
                    placeholder={t("aggregate.notesPlaceholder", {
                      defaultValue: "可选，说明这个聚合的用途",
                    })}
                  />
                </div>
                <div className="space-y-2">
                  <Label htmlFor="aggregate-website-url">
                    {t("provider.websiteUrl", { defaultValue: "官网地址" })}
                  </Label>
                  <Input
                    id="aggregate-website-url"
                    value={websiteUrl}
                    onChange={(e) => setWebsiteUrl(e.target.value)}
                    placeholder={t("providerForm.websiteUrlPlaceholder", {
                      defaultValue: "可选，填写供应商官网地址",
                    })}
                  />
                </div>
              </div>
            </div>
          )}

          {/* 步骤 2：选择成员 */}
          {step === 2 && (
            <div className="space-y-4">
              <div className="rounded-lg border border-border/60 bg-muted/30 p-3 text-xs text-muted-foreground">
                {t("aggregate.membersHint", {
                  defaultValue:
                    "选择要聚合的 Codex 供应商。成员可使用 Responses、Chat Completions 或 Anthropic Messages；具体协议可在预览页按模型调整。",
                })}
              </div>
              {memberCandidates.length === 0 ? (
                <div className="flex flex-col items-center gap-2 py-10 text-center">
                  <Layers className="h-8 w-8 text-muted-foreground" />
                  <p className="text-sm text-muted-foreground">
                    {t("aggregate.noMembers", {
                      defaultValue:
                        "还没有可用的 Codex 供应商，请先添加普通供应商。",
                    })}
                  </p>
                </div>
              ) : (
                <div className="space-y-2">
                  {memberCandidates.map((provider) => {
                    const wireApi = getCodexMemberWireApi(provider);
                    const checked = memberIds.includes(provider.id);
                    const { baseUrl, apiKey } =
                      getCodexMemberCredentials(provider);
                    return (
                      <div
                        key={provider.id}
                        className="flex items-start gap-3 rounded-lg border border-border/60 p-3 hover:bg-accent/40"
                      >
                        <Checkbox
                          id={`member-${provider.id}`}
                          checked={checked}
                          onCheckedChange={(v) =>
                            toggleMember(provider.id, Boolean(v))
                          }
                        />
                        <div className="min-w-0 flex-1">
                          <label
                            htmlFor={`member-${provider.id}`}
                            className="flex cursor-pointer items-center gap-2 text-sm font-medium"
                          >
                            {provider.name}
                            <Badge variant="secondary">{wireApi}</Badge>
                          </label>
                          <p className="mt-0.5 truncate text-xs text-muted-foreground">
                            {[
                              baseUrl ||
                                t("aggregate.noBaseUrl", {
                                  defaultValue: "无端点",
                                }),
                              apiKey
                                ? t("aggregate.hasKey", {
                                    defaultValue: "已配置 Key",
                                  })
                                : t("aggregate.noKey", {
                                    defaultValue:
                                      "缺少 Key（可能无法拉取模型）",
                                  }),
                            ].join(" · ")}
                          </p>
                        </div>
                        <Badge
                          variant={checked ? "default" : "outline"}
                          className="shrink-0"
                        >
                          {checked
                            ? t("aggregate.selected", {
                                defaultValue: "已选择",
                              })
                            : t("aggregate.select", { defaultValue: "选择" })}
                        </Badge>
                      </div>
                    );
                  })}
                </div>
              )}
            </div>
          )}

          {/* 步骤 3：选择模型 */}
          {step === 3 && (
            <div className="space-y-4">
              <div className="flex flex-wrap items-center gap-2 rounded-lg border border-border/60 bg-muted/30 p-3 text-xs text-muted-foreground">
                <Search className="h-3.5 w-3.5 shrink-0" />
                <Input
                  value={modelSearch}
                  onChange={(e) => setModelSearch(e.target.value)}
                  placeholder={t("aggregate.searchModels", {
                    defaultValue: "搜索模型…",
                  })}
                  className="h-7 w-48 bg-background text-xs"
                />
                <span className="ml-auto">
                  {t("aggregate.selectedCount", {
                    count: selectedCount,
                    defaultValue: `已选 ${selectedCount} 个模型`,
                  })}
                </span>
              </div>

              {selectedMemberProviders.length === 0 ? (
                <div className="py-8 text-center text-sm text-muted-foreground">
                  {t("aggregate.noSelectedMembers", {
                    defaultValue: "请先在上一步选择成员供应商",
                  })}
                </div>
              ) : (
                selectedMemberProviders.map((provider) => {
                  const state = fetchStates[provider.id] ?? "idle";
                  const available = availableModelsFor(provider.id);
                  const filtered = filteredModelsFor(provider.id);
                  const selected =
                    selectedModels[provider.id] ?? new Set<string>();
                  return (
                    <div
                      key={provider.id}
                      className="rounded-lg border border-border/60"
                    >
                      <div className="flex flex-wrap items-center gap-2 border-b border-border/60 px-3 py-2">
                        <span className="text-sm font-medium">
                          {provider.name}
                        </span>
                        <Button
                          variant="ghost"
                          size="sm"
                          className="h-6 px-2 text-xs"
                          disabled={state === "loading"}
                          onClick={() =>
                            void handleFetchMemberModels(provider.id)
                          }
                        >
                          <RefreshCw
                            className={cn(
                              "mr-1 h-3 w-3",
                              state === "loading" && "animate-spin",
                            )}
                          />
                          {t("aggregate.fetchModels", {
                            defaultValue: "获取模型",
                          })}
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          className="h-6 px-2 text-xs"
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
                        {state === "error" && (
                          <Badge variant="secondary" className="gap-1">
                            <AlertTriangle className="h-3 w-3" />
                            {t("aggregate.usingFallback", {
                              defaultValue: "已用本地模型目录回退",
                            })}
                          </Badge>
                        )}
                      </div>

                      {state === "loading" ? (
                        <div className="flex items-center gap-2 px-3 py-4 text-xs text-muted-foreground">
                          <Loader2 className="h-3.5 w-3.5 animate-spin" />
                          {t("aggregate.fetching", {
                            defaultValue: "正在获取模型…",
                          })}
                        </div>
                      ) : filtered.length === 0 ? (
                        <div className="px-3 py-4 text-xs text-muted-foreground">
                          {t("aggregate.noModelsForMember", {
                            defaultValue:
                              "没有可用模型，可手动添加下方模型 ID。",
                          })}
                        </div>
                      ) : (
                        <div className="grid max-h-64 grid-cols-1 gap-1 overflow-y-auto p-2 sm:grid-cols-2">
                          {filtered.map((model) => {
                            const key = `${provider.id}::${model}`;
                            const override = displayNameOverrides[key];
                            const isManual = (
                              manualModels[provider.id] ?? []
                            ).includes(model);
                            return (
                              <label
                                key={model}
                                className="flex cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-sm hover:bg-accent/40"
                              >
                                <Checkbox
                                  checked={selected.has(model)}
                                  onCheckedChange={(v) =>
                                    toggleModel(provider.id, model, Boolean(v))
                                  }
                                />
                                <span className="min-w-0 flex-1 truncate">
                                  {model}
                                </span>
                                {isManual && (
                                  <button
                                    type="button"
                                    className="text-muted-foreground hover:text-destructive"
                                    title={t("common.remove", {
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
                                {override ? (
                                  <span className="max-w-[40%] truncate text-xs text-muted-foreground">
                                    → {override}
                                  </span>
                                ) : null}
                              </label>
                            );
                          })}
                        </div>
                      )}

                      <div className="flex items-center gap-2 border-t border-border/60 px-3 py-2">
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
                          placeholder={t("aggregate.manualModelPlaceholder", {
                            defaultValue: "手动添加模型 ID（回车确认）",
                          })}
                          className="h-7 flex-1 text-xs"
                        />
                        <Button
                          variant="outline"
                          size="sm"
                          className="h-7 px-2 text-xs"
                          onClick={() => addManualModel(provider.id)}
                        >
                          <Plus className="mr-1 h-3 w-3" />
                          {t("aggregate.addModel", { defaultValue: "添加" })}
                        </Button>
                      </div>
                    </div>
                  );
                })
              )}
            </div>
          )}

          {/* 步骤 4：预览并保存 */}
          {step === 4 && (
            <div className="space-y-4">
              <div className="flex items-start justify-between gap-2 rounded-lg border border-border/60 bg-muted/30 p-3 text-xs text-muted-foreground">
                <span className="flex-1">
                  {t("aggregate.previewHint", {
                    defaultValue: "预览确认模型显示名、上游模型与协议后保存。",
                  })}
                </span>
              </div>
              <div className="flex flex-wrap items-center gap-2">
                <Badge>
                  {t("aggregate.name", { defaultValue: "名称" })}: {name || "—"}
                </Badge>
                <Badge variant="secondary">
                  {t("aggregate.memberCount", {
                    count: memberIds.length,
                    defaultValue: `${memberIds.length} 个成员`,
                  })}
                </Badge>
                <Badge variant="secondary">
                  {t("aggregate.modelCount", {
                    count: builtModels.length,
                    defaultValue: `${builtModels.length} 个模型`,
                  })}
                </Badge>
                <Button
                  variant="outline"
                  size="sm"
                  className="ml-auto h-7 px-2.5 text-xs"
                  onClick={() => setStep(2)}
                >
                  <Layers className="mr-1 h-3.5 w-3.5" />
                  {t("aggregate.editMembersModels", {
                    defaultValue: "修改成员与模型",
                  })}
                </Button>
              </div>
              <div className="space-y-3">
                {selectedMemberProviders.map((provider) => {
                  const groupModels = builtModels.filter(
                    (model) => model.providerId === provider.id,
                  );
                  if (groupModels.length === 0) return null;
                  return (
                    <div
                      key={provider.id}
                      className="overflow-hidden rounded-lg border border-border/60"
                    >
                      <div className="flex items-center gap-2 border-b border-border/60 bg-muted/20 px-3 py-2">
                        <span className="min-w-0 truncate text-sm font-medium">
                          {provider.name}
                        </span>
                        <Badge variant="secondary" className="shrink-0 text-xs">
                          {groupModels.length}
                        </Badge>
                      </div>
                      <div className="grid grid-cols-[minmax(0,1.2fr)_minmax(0,1.2fr)_minmax(0,1fr)] gap-2 border-b border-border/60 px-3 py-2 text-xs font-medium text-muted-foreground">
                        <span>
                          {t("aggregate.previewDisplayName", {
                            defaultValue: "显示名（可编辑）",
                          })}
                        </span>
                        <span>
                          {t("aggregate.previewUpstreamModel", {
                            defaultValue: "实际上游模型",
                          })}
                        </span>
                        <span>
                          {t("aggregate.previewApiFormat", {
                            defaultValue: "协议",
                          })}
                        </span>
                      </div>
                      <div className="divide-y divide-border/40">
                        {groupModels.map((model, index) => {
                          const key = `${model.providerId}::${model.upstreamModel}`;
                          const upstreamModel =
                            model.upstreamModel ?? model.model;
                          return (
                            <div
                              key={`${key}-${index}`}
                              className="grid grid-cols-[minmax(0,1.2fr)_minmax(0,1.2fr)_minmax(0,1fr)] items-center gap-2 px-3 py-2 text-sm"
                            >
                              <Input
                                aria-label={t("aggregate.previewDisplayName", {
                                  defaultValue: "显示名（可编辑）",
                                })}
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
                                className="h-7 min-w-0 text-xs"
                              />
                              <span
                                className="min-w-0 truncate font-mono text-xs"
                                title={upstreamModel}
                              >
                                {upstreamModel}
                              </span>
                              <Select
                                value={
                                  apiFormatOverrides[key] ??
                                  getAggregateModelApiFormat(provider)
                                }
                                onValueChange={(value) => {
                                  setApiFormatOverrides((current) => {
                                    if (
                                      value ===
                                      getAggregateModelApiFormat(provider)
                                    ) {
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
                                  aria-label={t("aggregate.previewApiFormat", {
                                    defaultValue: "协议",
                                  })}
                                  className="h-7 w-full min-w-0 text-xs"
                                >
                                  <SelectValue />
                                </SelectTrigger>
                                <SelectContent>
                                  <SelectItem value="openai_responses">
                                    Responses
                                  </SelectItem>
                                  <SelectItem value="openai_chat">
                                    Chat Completions
                                  </SelectItem>
                                  <SelectItem value="anthropic">
                                    Anthropic Messages
                                  </SelectItem>
                                </SelectContent>
                              </Select>
                            </div>
                          );
                        })}
                      </div>
                    </div>
                  );
                })}
              </div>

              <div className="grid gap-2 sm:grid-cols-[180px_minmax(0,1fr)] sm:items-center rounded-lg border border-border/60 bg-muted/30 p-3">
                <Label className="text-xs font-medium">
                  {t("aggregate.defaultModel", {
                    defaultValue: "默认模型",
                  })}
                </Label>
                <Select
                  value={defaultModel}
                  onValueChange={setDefaultModel}
                  disabled={builtModels.length === 0}
                >
                  <SelectTrigger
                    className="h-8 text-xs"
                    aria-label={t("aggregate.defaultModel", {
                      defaultValue: "默认模型",
                    })}
                  >
                    <SelectValue
                      placeholder={t("aggregate.defaultModelPlaceholder", {
                        defaultValue: "选择默认模型…",
                      })}
                    />
                  </SelectTrigger>
                  <SelectContent>
                    {builtModels.map((model) => (
                      <SelectItem key={model.model} value={model.model}>
                        {model.model}
                        {model.displayName &&
                        model.displayName !== model.model
                          ? ` · ${model.displayName}`
                          : ""}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <p className="text-xs text-muted-foreground sm:col-start-2">
                  {t("aggregate.defaultModelHint", {
                    defaultValue:
                      "该模型将写入接管的 config.toml，作为 Codex 打开时的默认模型；运行时仍可按模型切换。",
                  })}
                </p>
              </div>

              <div className="rounded-lg border border-border/60">
                <div className="border-b border-border/60 bg-muted/20 px-3 py-2 text-sm font-medium">
                  {t("aggregate.configTomlTitle", {
                    defaultValue: "对应 config.toml 文件",
                  })}
                </div>
                <pre className="max-h-72 overflow-auto whitespace-pre-wrap break-all px-3 py-2 font-mono text-xs text-foreground/90">
                  {buildAggregateConfigTomlPreview(
                    name,
                    builtModels,
                    defaultModel,
                  )}
                </pre>
              </div>
            </div>
          )}
        </div>
      </ScrollArea>

      {/* 底部导航 */}
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
                step === 2 ? handleNextFromMembers : () => setStep((s) => s + 1)
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
              {isEdit
                ? t("common.save", { defaultValue: "保存" })
                : t("common.add", { defaultValue: "添加" })}
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}
