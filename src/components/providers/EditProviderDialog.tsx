import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Save } from "lucide-react";
import { Button } from "@/components/ui/button";
import { FullScreenPanel } from "@/components/common/FullScreenPanel";
import type { Provider } from "@/types";
import {
  ProviderForm,
  type ProviderFormValues,
} from "@/components/providers/forms/ProviderForm";
import { AggregateProviderWizard } from "@/components/providers/AggregateProviderWizard";
import { isAggregateProvider } from "@/utils/aggregateProvider";
import { providersApi, vscodeApi } from "@/lib/api";

interface EditProviderDialogProps {
  open: boolean;
  provider: Provider | null;
  onOpenChange: (open: boolean) => void;
  onSubmit: (payload: {
    provider: Provider;
    originalId?: string;
  }) => Promise<void> | void;
  isProxyTakeover?: boolean; // 代理接管模式下不读取 live（避免显示被接管后的代理配置）
}

export function EditProviderDialog({
  open,
  provider,
  onOpenChange,
  onSubmit,
  isProxyTakeover = false,
}: EditProviderDialogProps) {
  const { t } = useTranslation();
  const [isFormSubmitting, setIsFormSubmitting] = useState(false);

  // 编辑会话开始时快照自定义端点；取消编辑时恢复，保证"取消后配置文件恢复原样"。
  // 端点管理弹窗的"保存端点变更"会在编辑会话中立即写库，取消整个编辑必须还原。
  const endpointSnapshotRef = useRef<Set<string> | null>(null);
  const isCancellingRef = useRef(false);

  useEffect(() => {
    if (!open || !provider) {
      endpointSnapshotRef.current = null;
      isCancellingRef.current = false;
      return;
    }
    let cancelled = false;
    vscodeApi
      .getCustomEndpoints("codex", provider.id)
      .then((endpoints) => {
        if (!cancelled) {
          endpointSnapshotRef.current = new Set(
            endpoints.map((endpoint) => endpoint.url),
          );
        }
      })
      .catch(() => {
        // 快照失败则不执行恢复，避免误删用户数据
        if (!cancelled) {
          endpointSnapshotRef.current = null;
        }
      });
    return () => {
      cancelled = true;
    };
  }, [open, provider?.id]);

  // 取消编辑：把编辑会话期间写入的自定义端点恢复为编辑前状态，再关闭对话框。
  const handleCancel = useCallback(() => {
    if (isCancellingRef.current) return;
    isCancellingRef.current = true;

    const providerId = provider?.id;
    const snapshot = endpointSnapshotRef.current;
    const close = () => onOpenChange(false);

    if (!providerId || !snapshot) {
      close();
      return;
    }

    void (async () => {
      try {
        const current = await vscodeApi.getCustomEndpoints("codex", providerId);
        const currentUrls = new Set(current.map((endpoint) => endpoint.url));
        for (const endpoint of current) {
          if (!snapshot.has(endpoint.url)) {
            await vscodeApi.removeCustomEndpoint("codex", providerId, endpoint.url);
          }
        }
        for (const url of snapshot) {
          if (!currentUrls.has(url)) {
            await vscodeApi.addCustomEndpoint("codex", providerId, url);
          }
        }
      } catch (error) {
        console.error(
          "[EditProviderDialog] 取消编辑后恢复自定义端点失败",
          error,
        );
      } finally {
        close();
      }
    })();
  }, [provider?.id, onOpenChange]);

  // 默认使用传入的 provider.settingsConfig，若当前编辑对象是"当前生效供应商"，则尝试读取实时配置替换初始值
  const [liveSettings, setLiveSettings] = useState<Record<
    string,
    unknown
  > | null>(null);

  // 使用 ref 标记是否已经加载过，防止重复读取覆盖用户编辑
  const [hasLoadedLive, setHasLoadedLive] = useState(false);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      if (!open || !provider) {
        setLiveSettings(null);
        setHasLoadedLive(false);
        return;
      }

      // 关键修复：只在首次打开时加载一次
      if (hasLoadedLive) {
        return;
      }

      // 代理接管模式：Live 配置已被代理改写，读取 live 会导致编辑界面展示代理地址/占位符等内容
      // 因此直接回退到 SSOT（数据库）配置，避免用户困惑与误保存
      if (isProxyTakeover) {
        if (!cancelled) {
          setLiveSettings(null);
          setHasLoadedLive(true);
        }
        return;
      }

      try {
        const currentId = await providersApi.getCurrent("codex");
        if (currentId && provider.id === currentId) {
          try {
            const live = (await vscodeApi.getLiveProviderSettings(
              "codex",
            )) as Record<string, unknown>;
            if (!cancelled && live && typeof live === "object") {
              setLiveSettings(live);
              setHasLoadedLive(true);
            }
          } catch {
            // 读取实时配置失败则回退到 SSOT（不打断编辑流程）
            if (!cancelled) {
              setLiveSettings(null);
              setHasLoadedLive(true);
            }
          }
        } else {
          if (!cancelled) {
            setLiveSettings(null);
            setHasLoadedLive(true);
          }
        }
      } finally {
        // no-op
      }
    };
    void load();
    return () => {
      cancelled = true;
    };
  }, [open, provider?.id, hasLoadedLive, isProxyTakeover]); // 只依赖 provider.id，不依赖整个 provider 对象

  const initialSettingsConfig = useMemo(() => {
    const base = (liveSettings ?? provider?.settingsConfig ?? {}) as Record<
      string,
      unknown
    >;

    // Codex 的 modelCatalog 是 Codex Cube 私有字段，SSOT 在数据库。Live 的 config.toml
    // 仅在写入时投影出 model_catalog_json 指针；Codex.app 改写配置、代理接管/恢复周期、
    // 来回切换供应商都可能让 Live 丢失该投影，从而 read_live_settings 反解为空。
    // 若放任 Live 覆盖，编辑界面会显示空映射表，保存后连同数据库里的映射一起清空（数据丢失）。
    // 因此始终以数据库 SSOT 的 modelCatalog 为准，仅在数据库确实没有时才回退到 Live 反解结果。
    if (
      liveSettings &&
      provider?.settingsConfig &&
      typeof provider.settingsConfig === "object"
    ) {
      const dbCatalog = (provider.settingsConfig as Record<string, unknown>)
        .modelCatalog;
      if (dbCatalog !== undefined) {
        return { ...base, modelCatalog: dbCatalog };
      }
    }

    return base;
  }, [liveSettings, provider?.settingsConfig]); // 只依赖 settingsConfig，不依赖整个 provider

  // 固定 initialData，防止 provider 对象更新时重置表单
  const initialData = useMemo(() => {
    if (!provider) return null;
    return {
      name: provider.name,
      notes: provider.notes,
      websiteUrl: provider.websiteUrl,
      settingsConfig: initialSettingsConfig,
      category: provider.category,
      meta: provider.meta,
      icon: provider.icon,
      iconColor: provider.iconColor,
    };
  }, [
    open, // 修复：编辑保存后再次打开显示旧数据，依赖 open 确保每次打开时重新读取最新 provider 数据
    provider?.id, // 只依赖 ID，provider 对象更新不会触发重新计算
    provider?.meta, // 供应商元数据变化时重新初始化表单
    initialSettingsConfig,
  ]);

  const handleAggregateSubmit = useCallback(
    async ({
      provider: updated,
      originalId,
    }: {
      provider: Provider;
      originalId?: string;
    }) => {
      await onSubmit({ provider: updated, originalId });
      onOpenChange(false);
    },
    [onSubmit, onOpenChange],
  );

  const handleSubmit = useCallback(
    async (values: ProviderFormValues) => {
      if (!provider) return;

      // 注意：values.settingsConfig 已经是最终的配置字符串
      // ProviderForm 已经组装出最终的 Codex 配置。
      const parsedConfig = JSON.parse(values.settingsConfig) as Record<
        string,
        unknown
      >;
      const updatedProvider: Provider = {
        ...provider,
        id: provider.id,
        name: values.name.trim(),
        notes: values.notes?.trim() || undefined,
        websiteUrl: values.websiteUrl?.trim() || undefined,
        settingsConfig: parsedConfig,
        icon: values.icon?.trim() || undefined,
        iconColor: values.iconColor?.trim() || undefined,
        ...(values.presetCategory ? { category: values.presetCategory } : {}),
        // 保留或更新 meta 字段
        ...(values.meta ? { meta: values.meta } : {}),
      };

      await onSubmit({
        provider: updatedProvider,
        originalId: provider.id,
      });
      onOpenChange(false);
    },
    [onSubmit, onOpenChange, provider],
  );

  if (!provider || !initialData) {
    return null;
  }

  const isAggregate = isAggregateProvider(provider);
  if (isAggregate) {
    return (
      <FullScreenPanel
        isOpen={open}
        title={t("aggregate.editTitle", {
          defaultValue: "编辑聚合 Provider",
        })}
        onClose={handleCancel}
        footer={
          <Button
            variant="outline"
            onClick={handleCancel}
            className="border-border/20 hover:bg-accent hover:text-accent-foreground"
          >
            {t("common.cancel")}
          </Button>
        }
      >
        <div className="h-[calc(100dvh-210px)]">
          <AggregateProviderWizard
            appId="codex"
            initialProvider={provider}
            onAdd={async () => {}}
            onEdit={handleAggregateSubmit}
            onCancel={() => onOpenChange(false)}
          />
        </div>
      </FullScreenPanel>
    );
  }

  return (
    <FullScreenPanel
      isOpen={open}
      title={t("provider.editProvider")}
      onClose={handleCancel}
      footer={
        <Button
          type="submit"
          form="provider-form"
          disabled={isFormSubmitting}
          className="bg-primary text-primary-foreground hover:bg-primary/90"
        >
          <Save className="h-4 w-4 mr-2" />
          {t("common.save")}
        </Button>
      }
    >
      <ProviderForm
        appId="codex"
        providerId={provider.id}
        submitLabel={t("common.save")}
        onSubmit={handleSubmit}
        onCancel={handleCancel}
        onSubmittingChange={setIsFormSubmitting}
        initialData={initialData}
        showButtons={false}
        isProxyTakeover={isProxyTakeover}
      />
    </FullScreenPanel>
  );
}
