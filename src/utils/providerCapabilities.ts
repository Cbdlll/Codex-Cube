import type { AppId } from "@/lib/api";
import type { Provider } from "@/types";
import { isOAuthProviderType } from "@/config/constants";
import { isAggregateProvider } from "@/utils/aggregateProvider";
import {
  extractCodexWireApi,
  isCodexAnthropicWireApi,
  isCodexChatWireApi,
} from "@/utils/providerConfigUtils";

export const CODEX_OFFICIAL_PROVIDER_ID = "codex-official";

/** Keep the UI capability rule aligned with the Rust takeover policy. */
export function supportsOfficialProxyTakeover(
  appId: AppId,
  provider: Pick<Provider, "id" | "category">,
): boolean {
  return (
    appId === "codex" &&
    provider.id === CODEX_OFFICIAL_PROVIDER_ID &&
    provider.category === "official"
  );
}

/**
 * UI 徽标谓词：仅当供应商确实需要本地路由的转换/认证/聚合能力时才显示
 * "需要路由"。与 `providerNeedsRouting` 不同：原生 Responses 的普通独立
 * 订阅在切换时仍会走本地代理（隔离聚合残留模型 slug），但徽标不再显示。
 */
export function providerShowsRoutingBadge(
  _appId: AppId,
  provider: Provider,
): boolean {
  if (provider.category === "official") return false;
  if (isAggregateProvider(provider)) return true;
  if (isOAuthProviderType(provider.meta?.providerType)) return true;

  // Codex：只有格式转换、完整 URL 或 config 中声明 Chat/Anthropic 才显示徽标。
  const apiFormat = provider.meta?.apiFormat;
  if (apiFormat === "openai_chat" || apiFormat === "anthropic") return true;
  if (provider.meta?.isFullUrl === true) return true;
  const config = (provider.settingsConfig as Record<string, unknown>)?.config;
  if (typeof config === "string") {
    const wire = extractCodexWireApi(config);
    if (isCodexChatWireApi(wire) || isCodexAnthropicWireApi(wire)) return true;
  }
  return false;
}

/**
 * 供应商在指定应用下是否必须开启路由接管才能正常工作（badge 与切换警告共用的权威谓词）。
 *
 * 权威信号是 `providerType`：托管 OAuth 供应商的凭据由本地代理按请求注入
 * （见 `forwarder.rs`，注入发生在转发路径上，请求必须经过代理 = 接管当前应用），
 * 且后端按 providerType 强制托管认证/格式而**无视 apiFormat**。因此 apiFormat
 * 只是可能被用户改动或旧数据缺省的次要信号，OAuth 供应商一律以 providerType 判定。
 *
 * Codex 的聚合 Provider 与托管 OAuth 始终需要本地路由；普通第三方 Provider
 * 仅当需要 Chat / Anthropic 格式转换时路由，原生 Responses 直连。
 */
export function providerNeedsRouting(
  _appId: AppId,
  provider: Provider,
): boolean {
  if (provider.category === "official") return false;

  // 聚合 Provider 是虚拟供应商：请求必须经过本地代理按模型路由到成员。
  if (isAggregateProvider(provider)) return true;

  const isManagedOAuth = isOAuthProviderType(provider.meta?.providerType);

  // 托管 OAuth：凭据由代理注入，与 apiFormat 无关，必须接管。
  if (isManagedOAuth) return true;

  // 普通第三方原生 Responses Provider 直连，不强制本地路由；仅当需要
  // Chat / Anthropic 转换、完整 URL 模式时才需要路由。
  if (provider.meta?.isFullUrl === true) return true;
  const fmt = provider.meta?.apiFormat;
  if (fmt === "openai_chat" || fmt === "anthropic") return true;
  const config = (provider.settingsConfig as Record<string, unknown>)?.config;
  if (typeof config === "string") {
    const wire = extractCodexWireApi(config);
    if (isCodexChatWireApi(wire) || isCodexAnthropicWireApi(wire)) return true;
  }
  return false;
}
