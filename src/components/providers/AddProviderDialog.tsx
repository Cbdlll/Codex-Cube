import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { FullScreenPanel } from "@/components/common/FullScreenPanel";
import type { CustomEndpoint, Provider } from "@/types";
import {
  ProviderForm,
  type ProviderFormValues,
} from "@/components/providers/forms/ProviderForm";
import { AggregateProviderWizard } from "@/components/providers/AggregateProviderWizard";
import { codexProviderPresets } from "@/config/codexProviderPresets";
import { extractCodexBaseUrl } from "@/utils/providerConfigUtils";

interface AddProviderDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSubmit: (
    provider: Omit<Provider, "id"> & { ensureCodexOfficialSeed?: boolean },
  ) => Promise<void> | void;
}

export function AddProviderDialog({
  open,
  onOpenChange,
  onSubmit,
}: AddProviderDialogProps) {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = useState<"app-specific" | "aggregate">(
    "app-specific",
  );
  const [isFormSubmitting, setIsFormSubmitting] = useState(false);

  const handleAggregateAdd = useCallback(
    async (providerData: Omit<Provider, "id"> & { providerKey?: string }) => {
      await onSubmit(providerData);
      onOpenChange(false);
    },
    [onSubmit, onOpenChange],
  );

  const handleSubmit = useCallback(
    async (values: ProviderFormValues) => {
      const parsedConfig = JSON.parse(values.settingsConfig) as Record<
        string,
        unknown
      >;
      const providerData: Omit<Provider, "id"> & {
        ensureCodexOfficialSeed?: boolean;
      } = {
        name: values.name.trim(),
        notes: values.notes?.trim() || undefined,
        websiteUrl: values.websiteUrl?.trim() || undefined,
        settingsConfig: parsedConfig,
        icon: values.icon?.trim() || undefined,
        iconColor: values.iconColor?.trim() || undefined,
        ...(values.presetCategory ? { category: values.presetCategory } : {}),
        ...(values.meta ? { meta: values.meta } : {}),
      };

      if (values.presetId) {
        const presetIndex = Number.parseInt(
          values.presetId.replace("codex-", ""),
          10,
        );
        const preset = codexProviderPresets[presetIndex];
        providerData.ensureCodexOfficialSeed =
          values.presetCategory === "official" &&
          preset?.category === "official";
      }

      const hasCustomEndpoints =
        providerData.meta?.custom_endpoints &&
        Object.keys(providerData.meta.custom_endpoints).length > 0;

      if (!hasCustomEndpoints) {
        const urls = new Set<string>();
        const addUrl = (rawUrl?: string) => {
          const url = (rawUrl || "").trim().replace(/\/+$/, "");
          if (url.startsWith("http")) urls.add(url);
        };

        if (values.presetId) {
          const presetIndex = Number.parseInt(
            values.presetId.replace("codex-", ""),
            10,
          );
          const preset = codexProviderPresets[presetIndex];
          preset?.endpointCandidates?.forEach(addUrl);
        }

        const config = parsedConfig.config;
        if (typeof config === "string") {
          addUrl(extractCodexBaseUrl(config));
        }

        if (urls.size > 0) {
          const now = Date.now();
          const customEndpoints: Record<string, CustomEndpoint> = {};
          urls.forEach((url) => {
            customEndpoints[url] = {
              url,
              addedAt: now,
              lastUsed: undefined,
            };
          });
          providerData.meta = {
            ...(providerData.meta ?? {}),
            custom_endpoints: customEndpoints,
          };
        }
      }

      await onSubmit(providerData);
      onOpenChange(false);
    },
    [onSubmit, onOpenChange],
  );

  // 聚合模式：导航完全由 AggregateProviderWizard 自己的固定底部栏负责，
  // 这里不渲染重复的 Cancel footer（App 专属 footer 保持不变）。
  // 聚合模式：导航完全由 AggregateProviderWizard 自己的固定底部栏负责，
  // 这里不渲染重复的 Cancel footer（App 专属 footer 保持不变）。
  const footer =
    activeTab === "aggregate" ? undefined : (
      <>
        <span className="mr-auto min-w-0 truncate text-xs text-muted-foreground">
          {t("provider.addFooterHint")}
        </span>
        <Button
          variant="outline"
          onClick={() => onOpenChange(false)}
          className="border-border/20 hover:bg-accent hover:text-accent-foreground"
        >
          {t("common.cancel")}
        </Button>
        <Button
          type="submit"
          form="provider-form"
          disabled={isFormSubmitting}
          className="bg-primary text-primary-foreground hover:bg-primary/90"
        >
          <Plus className="mr-2 h-4 w-4" />
          {t("common.add")}
        </Button>
      </>
    );

  return (
    <FullScreenPanel
      isOpen={open}
      title={t("provider.addNewProvider")}
      onClose={() => onOpenChange(false)}
      footer={footer}
      scrollable={activeTab !== "aggregate"}
      contentClassName={activeTab === "aggregate" ? "px-6 pt-4" : "pt-3"}
    >
      <Tabs
        value={activeTab}
        onValueChange={(value) =>
          setActiveTab(value as "app-specific" | "aggregate")
        }
        className={
          activeTab === "aggregate" ? "flex h-full min-h-0 flex-col" : undefined
        }
      >
        <TabsList
          className={
            activeTab === "aggregate"
              ? "mb-4 grid w-full shrink-0 grid-cols-2"
              : "mb-6 grid w-full grid-cols-2"
          }
        >
          <TabsTrigger value="app-specific" className="px-2.5 text-xs">
            {t("apps.codex")} {t("provider.tabProvider")}
          </TabsTrigger>
          <TabsTrigger value="aggregate" className="px-2.5 text-xs">
            {t("aggregate.tabTitle", { defaultValue: "聚合 Provider" })}
          </TabsTrigger>
        </TabsList>

        <TabsContent value="app-specific" className="mt-0">
          <ProviderForm
            appId="codex"
            submitLabel={t("common.add")}
            onSubmit={handleSubmit}
            onCancel={() => onOpenChange(false)}
            onSubmittingChange={setIsFormSubmitting}
            showButtons={false}
          />
        </TabsContent>

        <TabsContent
          value="aggregate"
          className="mt-0 min-h-0 flex-1 overflow-hidden"
        >
          <AggregateProviderWizard
            appId="codex"
            onAdd={handleAggregateAdd}
            onCancel={() => onOpenChange(false)}
          />
        </TabsContent>
      </Tabs>
    </FullScreenPanel>
  );
}
