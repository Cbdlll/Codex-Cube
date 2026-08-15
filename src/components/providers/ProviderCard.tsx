import { useMemo, useState, useEffect } from "react";
import { GripVertical, ChevronDown, ChevronUp } from "lucide-react";
import { useTranslation } from "react-i18next";
import type {
  DraggableAttributes,
  DraggableSyntheticListeners,
} from "@dnd-kit/core";
import type { Provider } from "@/types";
import type { AppId } from "@/lib/api";
import { cn } from "@/lib/utils";
import { ProviderActions } from "@/components/providers/ProviderActions";
import { ProviderIcon } from "@/components/ProviderIcon";
import UsageFooter from "@/components/UsageFooter";
import SubscriptionQuotaFooter from "@/components/SubscriptionQuotaFooter";
import CodexOauthQuotaFooter from "@/components/CodexOauthQuotaFooter";
import { PROVIDER_TYPES, TEMPLATE_TYPES } from "@/config/constants";
import { ProviderHealthBadge } from "@/components/providers/ProviderHealthBadge";
import { FailoverPriorityBadge } from "@/components/providers/FailoverPriorityBadge";
import {
  extractCodexBaseUrl,
  extractCodexExperimentalBearerToken,
} from "@/utils/providerConfigUtils";
import {
  supportsOfficialProxyTakeover,
  providerShowsRoutingBadge,
} from "@/utils/providerCapabilities";
import { useProviderHealth } from "@/lib/query/failover";
import { useUsageQuery } from "@/lib/query/queries";
import { resolveProviderIcon } from "@/utils/providerIcon";
import { isAggregateProvider } from "@/utils/aggregateProvider";

interface DragHandleProps {
  attributes: DraggableAttributes;
  listeners: DraggableSyntheticListeners;
  isDragging: boolean;
}

interface ProviderCardProps {
  provider: Provider;
  isCurrent: boolean;
  appId: AppId;
  onSwitch: (provider: Provider) => void;
  onEdit: (provider: Provider) => void;
  onDelete: (provider: Provider) => void;
  onConfigureUsage: (provider: Provider) => void;
  onOpenWebsite: (url: string) => void;
  onDuplicate: (provider: Provider) => void;
  onTest?: (provider: Provider) => void;
  onOpenTerminal?: (provider: Provider) => void;
  isTesting?: boolean;
  isProxyRunning: boolean;
  isProxyTakeover?: boolean; // 代理接管模式（Live配置已被接管，切换为热切换）
  dragHandleProps?: DragHandleProps;
  isAutoFailoverEnabled?: boolean; // 是否开启自动故障转移
  failoverPriority?: number; // 故障转移优先级（1 = P1, 2 = P2, ...）
  isInFailoverQueue?: boolean; // 是否在故障转移队列中
  onToggleFailover?: (enabled: boolean) => void; // 切换故障转移队列
  activeProviderId?: string; // 代理当前实际使用的供应商 ID（用于故障转移模式下标注绿色边框）
}

/** 判断是否为官方供应商（无自定义 base URL / API key，直连官方 API） */
function isOfficialProvider(provider: Provider, _appId: AppId): boolean {
  if (provider.category === "official") {
    return true;
  }

  // 聚合 Provider 是虚拟供应商（无自有凭据），不能按“无 Key 即官方登录”推断。
  if (isAggregateProvider(provider)) {
    return false;
  }

  const config = provider.settingsConfig as Record<string, any>;
  // 无 OPENAI_API_KEY → 使用 Codex CLI 内置 OAuth（官方）
  const apiKey = config?.auth?.OPENAI_API_KEY;
  const bearerToken =
    typeof config?.config === "string"
      ? extractCodexExperimentalBearerToken(config.config)
      : undefined;
  return (
    !bearerToken &&
    (!apiKey || (typeof apiKey === "string" && apiKey.trim() === ""))
  );
}

const extractApiUrl = (provider: Provider, fallbackText: string) => {
  if (provider.notes?.trim()) {
    return provider.notes.trim();
  }

  if (provider.websiteUrl) {
    return provider.websiteUrl;
  }

  const config = provider.settingsConfig;

  if (config && typeof config === "object") {
    const baseUrl = (config as Record<string, any>)?.config;

    if (typeof baseUrl === "string" && baseUrl.includes("base_url")) {
      const extractedBaseUrl = extractCodexBaseUrl(baseUrl);
      if (extractedBaseUrl) {
        return extractedBaseUrl;
      }
    }
  }

  return fallbackText;
};

export function ProviderCard({
  provider,
  isCurrent,
  appId,
  onSwitch,
  onEdit,
  onDelete,
  onConfigureUsage,
  onOpenWebsite,
  onDuplicate,
  onTest,
  onOpenTerminal,
  isTesting,
  isProxyRunning,
  isProxyTakeover = false,
  dragHandleProps,
  isAutoFailoverEnabled = false,
  failoverPriority,
  isInFailoverQueue = false,
  onToggleFailover,
  activeProviderId,
}: ProviderCardProps) {
  const { t } = useTranslation();

  const { data: health } = useProviderHealth(provider.id, appId);

  const fallbackUrlText = t("provider.notConfigured", {
    defaultValue: "未配置接口地址",
  });

  const displayUrl = useMemo(() => {
    return extractApiUrl(provider, fallbackUrlText);
  }, [provider, fallbackUrlText]);

  const isClickableUrl = useMemo(() => {
    if (provider.notes?.trim()) {
      return false;
    }
    if (displayUrl === fallbackUrlText) {
      return false;
    }
    return true;
  }, [provider.notes, displayUrl, fallbackUrlText]);

  const usageEnabled = provider.meta?.usage_script?.enabled ?? false;
  const isOfficial = isOfficialProvider(provider, appId);
  const officialSubscriptionUsage =
    provider.meta?.usage_script?.templateType ===
    TEMPLATE_TYPES.OFFICIAL_SUBSCRIPTION;
  const officialSubscriptionEnabled =
    isOfficial && usageEnabled && officialSubscriptionUsage;
  // 官方判定只认显式 category === "official"（SSOT），不回退 isOfficial 的空字段启发式。
  // 理由（此判定曾在「纯 category ↔ category+isOfficial 回退」间反复，结论钉死于此）：
  //  1) 封号保护是高代价决策，不该建立在「base_url/key 缺失」这种脆弱信号上——它无法区分
  //     「想直连官方」与「自定义但还没填完」，两者都表现为字段为空，必然误伤后者。
  //  2) 启发式在 UI 多拦的部分，执行层 useProviderActions.ts 也只认 category === "official"、
  //     并不兑现（绕过 UI 即可切换）→ 属虚保护，却以误伤 category 缺失的自定义供应商为代价。
  //  3) 预设导入的官方一定带 category="official"，category 缺失的「真官方」现实中≈不存在。
  // 真官方就该有显式 category；手动新建官方应引导标注，而不是靠空字段猜。
  const supportsOfficialRouting = supportsOfficialProxyTakeover(
    appId,
    provider,
  );
  const isOfficialBlockedByProxy =
    isProxyTakeover &&
    provider.category === "official" &&
    !supportsOfficialRouting;
  const isCodexOauth =
    provider.meta?.providerType === PROVIDER_TYPES.CODEX_OAUTH;
  // xAI OAuth (SuperGrok 反代)：额度经自管 OAuth token 自动显示，与 codex_oauth 同构
  const isXaiOauth = provider.meta?.providerType === PROVIDER_TYPES.XAI_OAUTH;
  // 徽标使用展示谓词：原生 Responses 的独立订阅不显示"需要路由"（但切换时
  // 仍走本地代理隔离聚合残留模型 slug，见 providerNeedsRouting 功能谓词）。
  const codexNeedsRouting = providerShowsRoutingBadge(appId, provider);

  const autoQueryInterval = isCurrent
    ? provider.meta?.usage_script?.autoQueryInterval || 0
    : 0;

  const { data: usage } = useUsageQuery(provider.id, appId, {
    enabled: usageEnabled && !isOfficial && !officialSubscriptionUsage,
    autoQueryInterval,
  });

  const isTokenPlan =
    provider.meta?.usage_script?.templateType === "token_plan";
  const hasMultiplePlans =
    usage?.success && usage.data && usage.data.length > 1 && !isTokenPlan;

  const [isExpanded, setIsExpanded] = useState(false);

  useEffect(() => {
    if (hasMultiplePlans) {
      setIsExpanded(true);
    }
  }, [hasMultiplePlans]);

  const handleOpenWebsite = () => {
    if (!isClickableUrl) {
      return;
    }
    onOpenWebsite(displayUrl);
  };

  // 判断是否是"当前使用中"的供应商
  // - 故障转移模式：代理实际使用的供应商（activeProviderId）
  // - 普通模式：isCurrent
  const isActiveProvider = isAutoFailoverEnabled
    ? activeProviderId === provider.id
    : isCurrent;

  const shouldUseGreen = isProxyTakeover && isActiveProvider;
  const shouldUseBlue = !isProxyTakeover && isActiveProvider;
  const isHighlighted = isActiveProvider;

  return (
    <div
      className={cn(
        "relative rounded-xl p-px transition-all duration-300 group",
        shouldUseGreen
          ? "bg-gradient-to-r from-emerald-500/70 via-emerald-400/40 to-emerald-500/70 shadow-sm shadow-emerald-500/10"
          : shouldUseBlue
            ? "bg-gradient-to-r from-primary via-primary/50 to-primary shadow-sm shadow-primary/10"
            : "bg-border",
        dragHandleProps?.isDragging &&
          "scale-[1.02] z-10 shadow-lg shadow-primary/10",
      )}
    >
      <div
        className={cn(
          "relative overflow-hidden rounded-[13px] border border-transparent bg-card p-4 text-card-foreground transition-all duration-300",
          (isAutoFailoverEnabled || isProxyTakeover) &&
            !dragHandleProps?.isDragging
            ? "hover:border-emerald-500/50"
            : "hover:border-border-active",
          !isHighlighted && "hover:-translate-y-0.5 hover:shadow-md",
          dragHandleProps?.isDragging && "cursor-grabbing",
        )}
      >
        <div
          className={cn(
            "absolute inset-0 bg-gradient-to-r to-transparent transition-opacity duration-500 pointer-events-none",
            shouldUseGreen ? "from-emerald-500/10" : "from-primary/10",
            isHighlighted ? "opacity-100" : "opacity-0",
          )}
        />
        <div className="relative flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
          <div className="flex min-w-0 flex-1 items-center gap-2">
            <button
              type="button"
              className={cn(
                "-ml-1.5 flex-shrink-0 cursor-grab active:cursor-grabbing p-1.5",
                "text-muted-foreground/50 hover:text-muted-foreground transition-colors",
                dragHandleProps?.isDragging && "cursor-grabbing",
              )}
              aria-label={t("provider.dragHandle")}
              {...(dragHandleProps?.attributes ?? {})}
              {...(dragHandleProps?.listeners ?? {})}
            >
              <GripVertical className="h-4 w-4" />
            </button>

            <div className="h-8 w-8 flex-shrink-0 rounded-lg bg-muted flex items-center justify-center border border-border group-hover:scale-105 transition-transform duration-300">
              <ProviderIcon
                icon={resolveProviderIcon(
                  appId,
                  provider.icon,
                  provider.iconColor,
                )}
                name={provider.name}
                color={provider.iconColor}
                size={20}
              />
            </div>

            <div className="min-w-0 flex-1 space-y-1">
              <div className="flex flex-wrap items-center gap-2 min-h-7">
                <h3
                  className="min-w-0 truncate text-base font-semibold leading-none"
                  title={provider.name}
                >
                  {provider.name}
                </h3>

                {isActiveProvider && (
                  <span className="inline-flex items-center rounded-full bg-primary/10 px-2 py-0.5 text-[10px] font-semibold text-primary">
                    {t("provider.currentlyUsing", {
                      defaultValue: "Currently Using",
                    })}
                  </span>
                )}

                {isAggregateProvider(provider) && (
                  <span className="inline-flex items-center rounded-md bg-emerald-100 px-1.5 py-0.5 text-[10px] font-semibold text-emerald-700 dark:bg-emerald-900/40 dark:text-emerald-300">
                    {t("aggregate.badge", { defaultValue: "聚合" })}
                  </span>
                )}

                {codexNeedsRouting && (
                  <span className="inline-flex items-center rounded-md bg-sky-100 px-1.5 py-0.5 text-[10px] font-semibold text-sky-700 dark:bg-sky-900/40 dark:text-sky-300">
                    {t("codex.needsRouting", {
                      defaultValue: "需要路由",
                    })}
                  </span>
                )}

                {supportsOfficialRouting && (
                  <span className="inline-flex items-center rounded-md bg-sky-100 px-1.5 py-0.5 text-[10px] font-semibold text-sky-700 dark:bg-sky-900/40 dark:text-sky-300">
                    {isProxyTakeover
                      ? t("codex.officialRouting", {
                          defaultValue: "官方账号路由",
                        })
                      : t("codex.nativeLogin", {
                          defaultValue: "Codex 登录",
                        })}
                  </span>
                )}

                {provider.category === "official" &&
                  !supportsOfficialRouting && (
                    <span className="inline-flex items-center rounded-md bg-slate-200 px-1.5 py-0.5 text-[10px] font-semibold text-slate-700 dark:bg-slate-700/60 dark:text-slate-200">
                      {t("codex.noRoutingSupport", {
                        defaultValue: "不支持路由",
                      })}
                    </span>
                  )}

                {isProxyRunning && isInFailoverQueue && health && (
                  <ProviderHealthBadge
                    consecutiveFailures={health.consecutive_failures}
                    isHealthy={health.is_healthy}
                  />
                )}

                {isAutoFailoverEnabled &&
                  isInFailoverQueue &&
                  failoverPriority && (
                    <FailoverPriorityBadge priority={failoverPriority} />
                  )}

              </div>

              {displayUrl && (
                <button
                  type="button"
                  onClick={handleOpenWebsite}
                  className={cn(
                    "inline-flex min-w-0 max-w-full flex-1 items-center overflow-hidden text-left text-sm",
                    isClickableUrl
                      ? "text-blue-500 transition-colors hover:underline dark:text-blue-400 cursor-pointer"
                      : "text-muted-foreground cursor-default",
                  )}
                  title={displayUrl}
                  disabled={!isClickableUrl}
                >
                  <span className="min-w-0 truncate">{displayUrl}</span>
                </button>
              )}
            </div>
          </div>

          <div className="flex items-center ml-auto min-w-0 gap-3">
            <div className="ml-auto">
              <div className="flex items-center gap-1">
                {isCodexOauth ? (
                  <CodexOauthQuotaFooter
                    meta={provider.meta}
                    inline={true}
                    isCurrent={isCurrent}
                  />
                ) : isOfficial ? (
                  officialSubscriptionEnabled ? (
                    <SubscriptionQuotaFooter
                      appId={appId}
                      inline={true}
                      isCurrent={isCurrent}
                      autoQueryInterval={
                        provider.meta?.usage_script?.autoQueryInterval ?? 0
                      }
                    />
                  ) : null
                ) : hasMultiplePlans ? (
                  <div className="flex items-center gap-2 text-xs text-gray-600 dark:text-gray-400">
                    <span className="font-medium">
                      {t("usage.multiplePlans", {
                        count: usage?.data?.length || 0,
                        defaultValue: `${usage?.data?.length || 0} 个套餐`,
                      })}
                    </span>
                  </div>
                ) : (
                  <UsageFooter
                    provider={provider}
                    providerId={provider.id}
                    appId={appId}
                    usageEnabled={usageEnabled}
                    isCurrent={isCurrent}
                    inline={true}
                  />
                )}
                {hasMultiplePlans && (
                  <button
                    onClick={(e) => {
                      e.stopPropagation();
                      setIsExpanded(!isExpanded);
                    }}
                    className="p-1 rounded hover:bg-gray-100 dark:hover:bg-gray-800 transition-colors text-gray-500 dark:text-gray-400 flex-shrink-0"
                    title={
                      isExpanded
                        ? t("usage.collapse", { defaultValue: "收起" })
                        : t("usage.expand", { defaultValue: "展开" })
                    }
                  >
                    {isExpanded ? (
                      <ChevronUp size={14} />
                    ) : (
                      <ChevronDown size={14} />
                    )}
                  </button>
                )}
              </div>
            </div>

            <div className="flex items-center gap-1.5 flex-shrink-0 opacity-0 pointer-events-none group-hover:opacity-100 group-focus-within:opacity-100 group-hover:pointer-events-auto group-focus-within:pointer-events-auto transition-opacity duration-200">
              <ProviderActions
                appId={appId}
                isCurrent={isCurrent}
                isTesting={isTesting}
                isProxyTakeover={isProxyTakeover}
                isOfficialBlockedByProxy={isOfficialBlockedByProxy}
                onSwitch={() => onSwitch(provider)}
                onEdit={() => onEdit(provider)}
                onDuplicate={() => onDuplicate(provider)}
                onTest={
                  // 连通检测对第三方/自定义/Codex-OAuth 供应商开放（这些正是旧的
                  // 真实请求探测会误报、而可达性探测能正确处理的对象）。官方供应商
                  // (category === "official") 一律隐藏：它们 base_url 故意留空、走客户端
                  // 默认/OAuth 端点，Codex Cube 没有可靠的探测目标。
                  onTest && provider.category !== "official"
                    ? () => onTest(provider)
                    : undefined
                }
                onConfigureUsage={
                  isCodexOauth || isXaiOauth
                    ? undefined
                    : () => onConfigureUsage(provider)
                }
                onDelete={() => onDelete(provider)}
                onOpenTerminal={
                  onOpenTerminal ? () => onOpenTerminal(provider) : undefined
                }
                isAutoFailoverEnabled={isAutoFailoverEnabled}
                isInFailoverQueue={isInFailoverQueue}
                onToggleFailover={onToggleFailover}
              />
            </div>
          </div>
        </div>

        {isExpanded && hasMultiplePlans && (
          <div className="mt-4 pt-4 border-t border-border-default">
            <UsageFooter
              provider={provider}
              providerId={provider.id}
              appId={appId}
              usageEnabled={usageEnabled}
              isCurrent={isCurrent}
              inline={false}
            />
          </div>
        )}
      </div>
    </div>
  );
}
