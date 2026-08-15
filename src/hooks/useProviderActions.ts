import { useCallback } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { useTranslation } from "react-i18next";
import { providersApi, type AppId } from "@/lib/api";
import type { Provider, UsageScript } from "@/types";
import {
  useAddProviderMutation,
  useUpdateProviderMutation,
  useDeleteProviderMutation,
  useSwitchProviderMutation,
} from "@/lib/query";
import { usageKeys } from "@/lib/query/usage";
import { extractErrorMessage } from "@/utils/errorUtils";
import {
  extractCodexWireApi,
  isCodexAnthropicWireApi,
  isCodexChatWireApi,
} from "@/utils/providerConfigUtils";
import {
  providerNeedsRouting,
  supportsOfficialProxyTakeover,
} from "@/utils/providerCapabilities";
import { isAggregateProvider } from "@/utils/aggregateProvider";
import { isOAuthProviderType } from "@/config/constants";

/**
 * Hook for managing provider actions (add, update, delete, switch)
 * Extracts business logic from App.tsx
 */
export function useProviderActions(
  activeApp: AppId,
  _isProxyRunning?: boolean,
  isProxyTakeover?: boolean,
) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();

  const addProviderMutation = useAddProviderMutation(activeApp);
  const updateProviderMutation = useUpdateProviderMutation(activeApp);
  const deleteProviderMutation = useDeleteProviderMutation(activeApp);
  const switchProviderMutation = useSwitchProviderMutation(activeApp);

  // 添加供应商
  const addProvider = useCallback(
    async (
      provider: Omit<Provider, "id"> & {
        providerKey?: string;
        addToLive?: boolean;
        ensureCodexOfficialSeed?: boolean;
        ensureGrokBuildOfficialSeed?: boolean;
      },
    ) => {
      await addProviderMutation.mutateAsync(provider);

    },
    [addProviderMutation, activeApp, t],
  );

  // 更新供应商
  const updateProvider = useCallback(
    async (provider: Provider, originalId?: string) => {
      await updateProviderMutation.mutateAsync({ provider, originalId });

      // 更新托盘菜单（失败不影响主操作）
      try {
        await providersApi.updateTrayMenu();
      } catch (trayError) {
        console.error(
          "Failed to update tray menu after updating provider",
          trayError,
        );
      }
    },
    [updateProviderMutation],
  );

  // 切换供应商
  const switchProvider = useCallback(
    async (provider: Provider) => {
      const isCodexChatFormat =
        provider.meta?.apiFormat === "openai_chat" ||
        (typeof (provider.settingsConfig as Record<string, any>)?.config ===
          "string" &&
          isCodexChatWireApi(
            extractCodexWireApi(
              (provider.settingsConfig as Record<string, any>).config,
            ),
          ));
      const isCodexAnthropicFormat =
        provider.meta?.apiFormat === "anthropic" ||
        (typeof (provider.settingsConfig as Record<string, any>)?.config ===
          "string" &&
          isCodexAnthropicWireApi(
            extractCodexWireApi(
              (provider.settingsConfig as Record<string, any>).config,
            ),
          ));

      const isAggregateCodex = isAggregateProvider(provider);

      // 聚合 Provider 是虚拟供应商，必须依赖本地路由；不自动开启路由。
      if (isAggregateCodex && !isProxyTakeover) {
        toast.error(
          t("notifications.aggregateRoutingRequired", {
            defaultValue:
              "切换聚合 Provider 前需要先开启 Codex 本地路由，请到代理页手动开启后再切换",
          }),
          { duration: 6000 },
        );
        return;
      }

      // Determine why this provider requires the proxy.
      let proxyRequiredReason: string | null = null;
      if (!isProxyTakeover && providerNeedsRouting(activeApp, provider)) {
        if (isOAuthProviderType(provider.meta?.providerType)) {
          // 托管 OAuth（codex_oauth / xai_oauth 等）：凭据由本地代理注入，
          // 是否需路由由 providerType 权威决定，不看 apiFormat（后端亦无视，
          // 见 forwarder.rs）——避免 codex_oauth 被改成 anthropic / 旧数据缺省
          // apiFormat 时漏判。
          proxyRequiredReason = t("notifications.proxyReasonManagedOAuth", {
            defaultValue: "使用托管 OAuth 登录（令牌由本地路由注入）",
          });
        } else if (isCodexChatFormat) {
          proxyRequiredReason = t("notifications.proxyReasonOpenAIChat", {
            defaultValue: "使用 OpenAI Chat 接口格式",
          });
        } else if (isCodexAnthropicFormat) {
          proxyRequiredReason = t(
            "notifications.proxyReasonAnthropicMessages",
            {
              defaultValue: "使用 Anthropic Messages 接口格式",
            },
          );
        } else if (provider.meta?.isFullUrl) {
          proxyRequiredReason = t("notifications.proxyReasonFullUrl", {
            defaultValue: "开启了完整 URL 连接模式",
          });
        } else {
          proxyRequiredReason = t("notifications.proxyReasonRoutingRequired", {
            defaultValue: "需要本地路由处理请求",
          });
        }
      }

      if (proxyRequiredReason && !isAggregateCodex) {
        toast.warning(
          t("notifications.proxyRequiredForSwitch", {
            reason: proxyRequiredReason,
            defaultValue:
              "此供应商{{reason}}，需要代理服务才能正常使用，请先启动代理",
          }),
        );
      }

      // The built-in Codex official provider can reuse Codex's native ChatGPT
      // login through local routing. Other official providers remain blocked.
      const officialSupportsTakeover = supportsOfficialProxyTakeover(
        activeApp,
        provider,
      );
      if (
        isProxyTakeover &&
        provider.category === "official" &&
        !officialSupportsTakeover
      ) {
        toast.error(
          t("notifications.officialBlockedByProxy", {
            defaultValue:
              "代理接管模式下不能切换到官方供应商，使用代理访问官方 API 可能导致账号被封禁",
          }),
          { duration: 6000 },
        );
        return;
      }

      try {
        const result = await switchProviderMutation.mutateAsync(provider.id);

        // Show backfill warning if present
        if (result?.warnings?.length) {
          toast.warning(
            t("notifications.backfillWarning", {
              defaultValue:
                "切换成功，但旧供应商配置回填失败，您手动修改的配置可能未保存",
            }),
            { duration: 5000 },
          );
        }

        // 若已弹过 proxyRequired 警告则不再弹 success
        if (!proxyRequiredReason || isAggregateCodex) {
          let messageKey = "notifications.switchSuccess";
          let defaultMessage = "切换成功！";
          if (isAggregateCodex) {
            messageKey = "notifications.aggregateSwitchSuccess";
            defaultMessage =
              "聚合路由目标已立即切换；请重启 Codex Desktop 重新加载模型目录";
          } else {
            messageKey = "notifications.codexRestartRequired";
            defaultMessage = "切换成功，请重启客户端以生效";
          }
          toast.success(t(messageKey, { defaultValue: defaultMessage }), {
            closeButton: true,
          });
        }
      } catch {
        // 错误提示由 mutation 处理
      }
    },
    [
      switchProviderMutation,
      activeApp,
      isProxyTakeover,
      t,
    ],
  );

  // 删除供应商
  const deleteProvider = useCallback(
    async (id: string) => {
      await deleteProviderMutation.mutateAsync(id);
    },
    [deleteProviderMutation],
  );

  // 保存用量脚本
  const saveUsageScript = useCallback(
    async (provider: Provider, script: UsageScript) => {
      try {
        const updatedProvider: Provider = {
          ...provider,
          meta: {
            ...provider.meta,
            usage_script: script,
          },
        };

        await providersApi.update(updatedProvider, activeApp);
        await queryClient.invalidateQueries({
          queryKey: ["providers", activeApp],
        });
        // 🔧 保存用量脚本后，也应该失效该 provider 的用量查询缓存
        // 这样主页列表会使用新配置重新查询，而不是使用测试时的缓存
        await queryClient.invalidateQueries({
          queryKey: usageKeys.script(provider.id, activeApp),
        });
        await queryClient.invalidateQueries({
          queryKey: ["subscription", "quota", activeApp],
        });
        toast.success(
          t("provider.usageSaved", {
            defaultValue: "用量查询配置已保存",
          }),
          { closeButton: true },
        );
      } catch (error) {
        const detail =
          extractErrorMessage(error) ||
          t("provider.usageSaveFailed", {
            defaultValue: "用量查询配置保存失败",
          });
        toast.error(detail);
      }
    },
    [activeApp, queryClient, t],
  );

  return {
    addProvider,
    updateProvider,
    switchProvider,
    deleteProvider,
    saveUsageScript,
    isLoading:
      addProviderMutation.isPending ||
      updateProviderMutation.isPending ||
      deleteProviderMutation.isPending ||
      switchProviderMutation.isPending,
  };
}
