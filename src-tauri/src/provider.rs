use http::header::{HeaderValue, InvalidHeaderValue};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

// SSOT 模式：不再写供应商副本文件

/// 供应商结构体
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    pub id: String,
    pub name: String,
    #[serde(rename = "settingsConfig")]
    pub settings_config: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "websiteUrl")]
    pub website_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "createdAt")]
    pub created_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "sortIndex")]
    pub sort_index: Option<usize>,
    /// 备注信息
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// 供应商元数据（不写入 live 配置，仅存于 ~/.codex-cube/config.json）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<ProviderMeta>,
    /// 图标名称（如 "openai", "anthropic"）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// 图标颜色（Hex 格式，如 "#00A67E"）
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "iconColor")]
    pub icon_color: Option<String>,
    /// 是否加入故障转移队列
    #[serde(default)]
    #[serde(rename = "inFailoverQueue")]
    pub in_failover_queue: bool,
}

impl Provider {
    /// 从现有ID创建供应商
    pub fn with_id(
        id: String,
        name: String,
        settings_config: Value,
        website_url: Option<String>,
    ) -> Self {
        Self {
            id,
            name,
            settings_config,
            website_url,
            category: None,
            created_at: None,
            sort_index: None,
            notes: None,
            meta: None,
            icon: None,
            icon_color: None,
            in_failover_queue: false,
        }
    }

    pub fn is_codex_oauth(&self) -> bool {
        self.provider_type() == Some("codex_oauth")
    }

    pub fn is_xai_oauth(&self) -> bool {
        self.provider_type() == Some("xai_oauth")
    }

    pub fn is_github_copilot(&self) -> bool {
        self.provider_type() == Some("github_copilot")
            || self.claude_base_url_contains("githubcopilot.com")
    }

    /// 聚合 Provider：把多个 Responses 直连的 Codex 供应商聚合到一个虚拟
    /// 供应商，请求由本地代理按模型路由到对应成员。
    pub fn is_aggregate(&self) -> bool {
        self.provider_type() == Some("aggregate")
    }

    pub fn uses_managed_account_auth(&self) -> bool {
        self.is_github_copilot()
            || self.is_codex_oauth()
            || self.is_xai_oauth()
            || self.claude_base_url_contains("chatgpt.com/backend-api/codex")
    }

    /// Whether the provider form's "auth field" was explicitly set to
    /// ANTHROPIC_API_KEY. The form only persists `meta.apiKeyField` for the
    /// non-default choice, so `None` means the default ANTHROPIC_AUTH_TOKEN.
    pub fn claude_uses_api_key_field(&self) -> bool {
        self.meta
            .as_ref()
            .and_then(|m| m.api_key_field.as_deref())
            .map(|field| field.eq_ignore_ascii_case("ANTHROPIC_API_KEY"))
            .unwrap_or(false)
    }

    fn provider_type(&self) -> Option<&str> {
        self.meta.as_ref().and_then(|m| m.provider_type.as_deref())
    }

    fn claude_base_url_contains(&self, needle: &str) -> bool {
        self.settings_config
            .pointer("/env/ANTHROPIC_BASE_URL")
            .and_then(|value| value.as_str())
            .map(|base_url| base_url.contains(needle))
            .unwrap_or(false)
    }

    pub fn codex_fast_mode_enabled(&self) -> bool {
        self.meta
            .as_ref()
            .map(|m| m.codex_fast_mode_enabled())
            .unwrap_or(false)
    }

    pub fn has_usage_script_enabled(&self) -> bool {
        self.meta
            .as_ref()
            .and_then(|m| m.usage_script.as_ref())
            .map(|s| s.enabled)
            .unwrap_or(false)
    }

    /// Resolve `(base_url, api_key)` for usage queries (native balance /
    /// coding-plan and the JS-script `{{apiKey}}`/`{{baseUrl}}` fallback)
    /// from the stored provider config.
    ///
    /// Each app persists credentials in a different shape, so callers must pass
    /// the owning app type. This mirrors the frontend `getProviderCredentials`
    /// in `UsageScriptModal.tsx`.
    pub fn resolve_usage_credentials(
        &self,
        app_type: &crate::app_config::AppType,
    ) -> (String, String) {
        use crate::app_config::AppType;

        let settings = &self.settings_config;
        let str_at =
            |value: Option<&Value>| value.and_then(|v| v.as_str()).unwrap_or("").to_string();

        // First present, non-empty string among `keys`, mirroring the frontend's
        // `a || b || c` — JS `||` skips empty strings, and presets seed fields like
        // `ANTHROPIC_AUTH_TOKEN` as present-but-empty placeholders, so a plain
        // `.get().or_else()` chain (which only skips *absent* keys) would stop short.
        fn first_non_empty(env: Option<&Value>, keys: &[&str]) -> String {
            let Some(env) = env else {
                return String::new();
            };
            for key in keys {
                if let Some(s) = env.get(key).and_then(|v| v.as_str()) {
                    if !s.is_empty() {
                        return s.to_string();
                    }
                }
            }
            String::new()
        }

        let (base_url, api_key) = match app_type {
            // Codex keeps its key in `auth.OPENAI_API_KEY` and its base URL
            // inside a TOML `config` string, not in an `env` map.
            AppType::Codex => {
                let auth = settings.get("auth");
                let config_text = settings.get("config").and_then(|v| v.as_str());
                let api_key = crate::codex_config::extract_codex_api_key(auth, config_text)
                    .unwrap_or_default();
                let base_url = config_text
                    .and_then(crate::codex_config::extract_codex_base_url)
                    .unwrap_or_default();
                (base_url, api_key)
            }
        };

        // Normalize like the JS-script path (extract_base_url_from_provider) so a
        // future delegation from services/provider/usage.rs is behavior-preserving
        // and `{{baseUrl}}/path` concatenation never produces a double slash.
        (base_url.trim_end_matches('/').to_string(), api_key)
    }
}

/// 供应商管理器
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderManager {
    pub providers: IndexMap<String, Provider>,
    pub current: String,
}

/// 用量查询脚本配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageScript {
    pub enabled: bool,
    pub language: String,
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    /// 用量查询专用的 API Key（通用模板使用）
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "apiKey")]
    pub api_key: Option<String>,
    /// 用量查询专用的 Base URL（通用和 NewAPI 模板使用）
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "baseUrl")]
    pub base_url: Option<String>,
    /// 访问令牌（用于需要登录的接口，NewAPI 模板使用）
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "accessToken")]
    pub access_token: Option<String>,
    /// 用户ID（用于需要用户标识的接口，NewAPI 模板使用）
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "userId")]
    pub user_id: Option<String>,
    /// 模板类型（用于后端判断验证规则）
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "templateType")]
    pub template_type: Option<String>,
    /// 自动查询间隔（单位：分钟，0 表示禁用自动查询）
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "autoQueryInterval")]
    pub auto_query_interval: Option<u64>,
    /// Coding Plan 供应商标识（如 "kimi", "zhipu", "minimax"）
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "codingPlanProvider")]
    pub coding_plan_provider: Option<String>,
    /// 火山方舟控制面 OpenAPI 的 AccessKey ID（用量查询签名用，与推理 Key 是两套凭据）
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "accessKeyId")]
    pub access_key_id: Option<String>,
    /// 火山方舟控制面 OpenAPI 的 SecretAccessKey（同上）
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "secretAccessKey")]
    pub secret_access_key: Option<String>,
    /// 智谱团队套餐（Team Plan）的组织 ID（用量查询请求头 bigmodel-organization）
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "teamOrganizationId")]
    pub team_organization_id: Option<String>,
    /// 智谱团队套餐（Team Plan）的项目 ID（用量查询请求头 bigmodel-project）
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "teamProjectId")]
    pub team_project_id: Option<String>,
}

/// 用量数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageData {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "planName")]
    pub plan_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "isValid")]
    pub is_valid: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "invalidMessage")]
    pub invalid_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

/// 用量查询结果（支持多套餐）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Vec<UsageData>>, // 支持返回多个套餐
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 认证绑定来源
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuthBindingSource {
    /// 从 provider 自身配置读取认证信息（默认）
    #[default]
    ProviderConfig,
    /// 使用托管账号认证（如 GitHub Copilot OAuth）
    ManagedAccount,
}

/// 通用认证绑定
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthBinding {
    /// 认证来源
    #[serde(default)]
    pub source: AuthBindingSource,
    /// 托管认证供应商标识（如 github_copilot）
    #[serde(rename = "authProvider", skip_serializing_if = "Option::is_none")]
    pub auth_provider: Option<String>,
    /// 托管账号 ID；为空表示跟随该认证供应商的默认账号
    #[serde(rename = "accountId", skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
}

/// Codex Responses -> Chat Completions 的 reasoning 能力描述。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct CodexChatReasoningConfig {
    #[serde(rename = "supportsThinking", skip_serializing_if = "Option::is_none")]
    pub supports_thinking: Option<bool>,
    #[serde(rename = "supportsEffort", skip_serializing_if = "Option::is_none")]
    pub supports_effort: Option<bool>,
    #[serde(rename = "thinkingParam", skip_serializing_if = "Option::is_none")]
    pub thinking_param: Option<String>,
    #[serde(rename = "effortParam", skip_serializing_if = "Option::is_none")]
    pub effort_param: Option<String>,
    #[serde(rename = "effortValueMode", skip_serializing_if = "Option::is_none")]
    pub effort_value_mode: Option<String>,
    /// 声明性字段：标注上游 reasoning 的回传位置（reasoning_content / reasoning /
    /// reasoning_details / think_tags）。当前响应侧 `extract_reasoning_field_text`
    /// 靠穷举字段提取、并不读取本字段；保留作文档说明与未来按格式分发（如 think_tags）的预留。
    #[serde(rename = "outputFormat", skip_serializing_if = "Option::is_none")]
    pub output_format: Option<String>,
}

/// Local proxy request overrides applied after route/protocol transforms.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalProxyRequestOverrides {
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub headers: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<serde_json::Value>,
}

impl LocalProxyRequestOverrides {
    pub fn is_empty(&self) -> bool {
        self.headers.is_empty() && self.body.is_none()
    }
}

/// 供应商元数据
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderMeta {
    /// 自定义端点列表（按 URL 去重存储）
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub custom_endpoints: HashMap<String, crate::settings::CustomEndpoint>,
    /// 是否在写入 live 时应用通用配置片段
    #[serde(
        rename = "commonConfigEnabled",
        skip_serializing_if = "Option::is_none"
    )]
    pub common_config_enabled: Option<bool>,
    /// 用量查询脚本配置
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_script: Option<UsageScript>,
    /// 请求地址管理：测速后自动选择最佳端点
    #[serde(rename = "endpointAutoSelect", skip_serializing_if = "Option::is_none")]
    pub endpoint_auto_select: Option<bool>,
    /// 成本倍数（用于计算实际成本）
    #[serde(rename = "costMultiplier", skip_serializing_if = "Option::is_none")]
    pub cost_multiplier: Option<String>,
    /// 计费模式来源（response/request）
    #[serde(rename = "pricingModelSource", skip_serializing_if = "Option::is_none")]
    pub pricing_model_source: Option<String>,
    /// 每日消费限额（USD）
    #[serde(rename = "limitDailyUsd", skip_serializing_if = "Option::is_none")]
    pub limit_daily_usd: Option<String>,
    /// 每月消费限额（USD）
    #[serde(rename = "limitMonthlyUsd", skip_serializing_if = "Option::is_none")]
    pub limit_monthly_usd: Option<String>,
    /// Claude API 格式（仅 Claude 供应商使用）
    /// - "anthropic": 原生 Anthropic Messages API，直接透传
    /// - "openai_chat": OpenAI Chat Completions 格式，需要转换
    /// - "openai_responses": OpenAI Responses API 格式，需要转换
    #[serde(rename = "apiFormat", skip_serializing_if = "Option::is_none")]
    pub api_format: Option<String>,
    /// 通用认证绑定（provider_config / managed_account）
    ///
    /// 新代码应只写入该字段；githubAccountId 仅保留兼容读取。
    #[serde(rename = "authBinding", skip_serializing_if = "Option::is_none")]
    pub auth_binding: Option<AuthBinding>,
    /// Claude 认证字段名（"ANTHROPIC_AUTH_TOKEN" 或 "ANTHROPIC_API_KEY"）
    #[serde(rename = "apiKeyField", skip_serializing_if = "Option::is_none")]
    pub api_key_field: Option<String>,
    /// 是否将 base_url 视为完整 API 端点（不拼接 endpoint 路径）
    #[serde(rename = "isFullUrl", skip_serializing_if = "Option::is_none")]
    pub is_full_url: Option<bool>,
    /// Prompt cache key for OpenAI Responses-compatible endpoints.
    /// When set, injected into converted Responses requests to improve cache hit rate.
    /// If not set, Claude -> Responses conversions use a client-provided session/thread
    /// identity when available; generated session IDs are not sent upstream.
    #[serde(rename = "promptCacheKey", skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
    /// Session-based prompt-cache routing for Codex Responses -> Chat conversions.
    /// "auto" enables known-compatible upstreams; "enabled" / "disabled" are overrides.
    #[serde(rename = "promptCacheRouting", skip_serializing_if = "Option::is_none")]
    pub prompt_cache_routing: Option<String>,
    /// Codex OAuth FAST mode: inject `service_tier = "priority"` for ChatGPT Codex requests.
    #[serde(rename = "codexFastMode", skip_serializing_if = "Option::is_none")]
    pub codex_fast_mode: Option<bool>,
    /// Codex Responses -> Chat Completions reasoning capability metadata.
    #[serde(rename = "codexChatReasoning", skip_serializing_if = "Option::is_none")]
    pub codex_chat_reasoning: Option<CodexChatReasoningConfig>,
    /// Codex → Anthropic path: whether to emulate the Claude Code client
    /// (User-Agent / anthropic-beta / x-app + injecting the Claude Code system
    /// prompt first line). Disabled by default; only an explicit `true` enables it.
    #[serde(
        rename = "impersonateClaudeCode",
        skip_serializing_if = "Option::is_none"
    )]
    pub impersonate_claude_code: Option<bool>,
    /// Codex → Anthropic path: override the Anthropic `max_tokens` (output ceiling).
    ///
    /// Codex does not forward its `model_max_output_tokens` in the Responses
    /// request body, so without this the path falls back to a conservative
    /// default (8192), which truncates long or thinking-heavy responses
    /// (`stop_reason=max_tokens`). When set (>0), this value is injected as the
    /// request's `max_output_tokens` before conversion, taking precedence over
    /// both any request-supplied value and the default. Kept per-provider on
    /// purpose: a global large default would hard-400 on low-output-ceiling
    /// models/gateways (and that error is non-retryable).
    #[serde(rename = "maxOutputTokens", skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    /// Custom User-Agent for local proxy routing.
    #[serde(rename = "customUserAgent", skip_serializing_if = "Option::is_none")]
    pub custom_user_agent: Option<String>,
    /// Local proxy request overrides applied to the transformed upstream request.
    #[serde(
        rename = "localProxyRequestOverrides",
        skip_serializing_if = "Option::is_none"
    )]
    pub local_proxy_request_overrides: Option<LocalProxyRequestOverrides>,
    /// 累加模式应用中，该 provider 是否已写入 live config。
    /// `None` 表示旧数据/未知状态，`Some(false)` 表示明确仅存在于数据库中。
    #[serde(rename = "liveConfigManaged", skip_serializing_if = "Option::is_none")]
    pub live_config_managed: Option<bool>,
    /// 供应商类型标识（用于特殊供应商检测）
    /// - "github_copilot": GitHub Copilot 供应商
    #[serde(rename = "providerType", skip_serializing_if = "Option::is_none")]
    pub provider_type: Option<String>,
    /// GitHub Copilot 关联账号 ID（仅 github_copilot 供应商使用）
    /// 用于多账号支持，关联到特定的 GitHub 账号
    #[serde(rename = "githubAccountId", skip_serializing_if = "Option::is_none")]
    pub github_account_id: Option<String>,
}

/// 解析 Provider 级自定义 User-Agent 字符串（单一真理来源）。
///
/// 转发（forwarder）、流式检测（stream_check）、获取模型列表（model_fetch）三条路径
/// 共用同一口径，避免出现"某条路径用了 UA、另一条没用 / 报错"的不一致。
///
/// 合法性由 `http::HeaderValue::from_str` 按**字节**判定（`b >= 32 && b != 127 || b == '\t'`），
/// 与前端 `src/lib/userAgent.ts::isValidUserAgentHeader` 严格一致：
/// - `Ok(None)`：未设置或纯空白（trim 后为空）。
/// - `Ok(Some(hv))`：合法。制表符、可见 ASCII（0x20–0x7E）、以及任意非 ASCII 字符
///   （UTF-8 字节均 ≥ 0x80）都合法。
/// - `Err(_)`：仅含控制字符时——除 `\t` 外的 0x00–0x1F（含换行）与 0x7F（DEL）。
///
/// 非法值的处理：三条运行时路径**均静默忽略**（`.ok().flatten()`，绝不让某条路径报错而
/// 另一条放行）；前端在输入框处给出非阻断提示。当前**不在保存时阻断**——deeplink 导入等
/// 非表单路径应宽容，运行时静默忽略即为安全网。
pub fn parse_custom_user_agent(
    raw: Option<&str>,
) -> Result<Option<HeaderValue>, InvalidHeaderValue> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        Some(ua) => HeaderValue::from_str(ua).map(Some),
        None => Ok(None),
    }
}

impl ProviderMeta {
    /// Codex OAuth FAST mode 是否启用。默认关闭，因为 `service_tier="priority"`
    /// 会按更高速率消耗 ChatGPT 订阅配额，用户需显式开启以换取更低延迟。
    pub fn codex_fast_mode_enabled(&self) -> bool {
        self.codex_fast_mode.unwrap_or(false)
    }

    /// 经校验的 Provider 级自定义 User-Agent。见 [`parse_custom_user_agent`]。
    pub fn custom_user_agent_header(&self) -> Result<Option<HeaderValue>, InvalidHeaderValue> {
        parse_custom_user_agent(self.custom_user_agent.as_deref())
    }

    /// 解析指定托管认证供应商绑定的账号 ID。
    ///
    /// 新版优先读取 authBinding，旧版继续兼容 githubAccountId。
    pub fn managed_account_id_for(&self, auth_provider: &str) -> Option<String> {
        if let Some(binding) = self.auth_binding.as_ref() {
            if binding.source == AuthBindingSource::ManagedAccount
                && binding.auth_provider.as_deref() == Some(auth_provider)
            {
                return binding.account_id.clone();
            }
        }

        if auth_provider == "github_copilot" {
            return self.github_account_id.clone();
        }

        None
    }
}

impl ProviderManager {
    /// 获取所有供应商
    pub fn get_all_providers(&self) -> &IndexMap<String, Provider> {
        &self.providers
    }
}

#[cfg(test)]
mod tests {
    use super::{LocalProxyRequestOverrides, Provider, ProviderManager, ProviderMeta};
    use crate::app_config::AppType;
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn provider_meta_serializes_pricing_model_source() {
        let meta = ProviderMeta {
            pricing_model_source: Some("response".to_string()),
            ..ProviderMeta::default()
        };

        let value = serde_json::to_value(&meta).expect("serialize ProviderMeta");

        assert_eq!(
            value
                .get("pricingModelSource")
                .and_then(|item| item.as_str()),
            Some("response")
        );
        assert!(value.get("pricing_model_source").is_none());
    }

    #[test]
    fn provider_meta_omits_pricing_model_source_when_none() {
        let meta = ProviderMeta::default();
        let value = serde_json::to_value(&meta).expect("serialize ProviderMeta");

        assert!(value.get("pricingModelSource").is_none());
    }

    #[test]
    fn provider_meta_roundtrips_max_output_tokens() {
        let meta = ProviderMeta {
            max_output_tokens: Some(64000),
            ..ProviderMeta::default()
        };

        let value = serde_json::to_value(&meta).expect("serialize ProviderMeta");
        assert_eq!(
            value.get("maxOutputTokens").and_then(|v| v.as_u64()),
            Some(64000)
        );
        assert!(value.get("max_output_tokens").is_none());

        let parsed: ProviderMeta = serde_json::from_value(value).expect("deserialize ProviderMeta");
        assert_eq!(parsed.max_output_tokens, Some(64000));
    }

    #[test]
    fn provider_meta_omits_max_output_tokens_when_none() {
        let value = serde_json::to_value(ProviderMeta::default()).expect("serialize ProviderMeta");
        assert!(value.get("maxOutputTokens").is_none());
    }

    #[test]
    fn provider_meta_roundtrips_local_proxy_request_overrides() {
        let meta = ProviderMeta {
            local_proxy_request_overrides: Some(LocalProxyRequestOverrides {
                headers: HashMap::from([("X-Test".to_string(), "yes".to_string())]),
                body: Some(json!({ "temperature": 0.2 })),
            }),
            ..ProviderMeta::default()
        };

        let value = serde_json::to_value(&meta).expect("serialize ProviderMeta");
        assert_eq!(
            value["localProxyRequestOverrides"]["headers"]["X-Test"],
            "yes"
        );
        assert_eq!(
            value["localProxyRequestOverrides"]["body"]["temperature"],
            0.2
        );

        let decoded: ProviderMeta =
            serde_json::from_value(value).expect("deserialize ProviderMeta");
        let overrides = decoded.local_proxy_request_overrides.unwrap();
        assert_eq!(overrides.headers.get("X-Test"), Some(&"yes".to_string()));
        assert_eq!(overrides.body.unwrap()["temperature"], 0.2);
    }

    #[test]
    fn provider_with_id_populates_defaults() {
        let settings_config = json!({
            "env": { "API_KEY": "test" }
        });
        let provider = Provider::with_id(
            "provider-1".to_string(),
            "Provider".to_string(),
            settings_config.clone(),
            Some("https://example.com".to_string()),
        );

        assert_eq!(provider.id, "provider-1");
        assert_eq!(provider.name, "Provider");
        assert_eq!(provider.settings_config, settings_config);
        assert_eq!(provider.website_url.as_deref(), Some("https://example.com"));
        assert!(provider.category.is_none());
        assert!(provider.created_at.is_none());
        assert!(provider.sort_index.is_none());
        assert!(provider.notes.is_none());
        assert!(provider.meta.is_none());
        assert!(provider.icon.is_none());
        assert!(provider.icon_color.is_none());
        assert!(!provider.in_failover_queue);
    }

    #[test]
    fn provider_managed_account_auth_detection_uses_type_or_known_endpoint() {
        let mut copilot = Provider::with_id(
            "copilot".to_string(),
            "Copilot".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_BASE_URL": "https://api.githubcopilot.com"
                }
            }),
            None,
        );
        assert!(copilot.is_github_copilot());
        assert!(copilot.uses_managed_account_auth());

        let mut codex = Provider::with_id(
            "codex".to_string(),
            "Codex".to_string(),
            json!({ "env": {} }),
            None,
        );
        codex.meta = Some(ProviderMeta {
            provider_type: Some("codex_oauth".to_string()),
            ..Default::default()
        });
        assert!(codex.is_codex_oauth());
        assert!(codex.uses_managed_account_auth());

        let codex_endpoint = Provider::with_id(
            "codex-endpoint".to_string(),
            "Codex Endpoint".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_BASE_URL": "https://chatgpt.com/backend-api/codex"
                }
            }),
            None,
        );
        assert!(codex_endpoint.uses_managed_account_auth());

        copilot.meta = Some(ProviderMeta {
            provider_type: Some("github_copilot".to_string()),
            ..Default::default()
        });
        assert!(copilot.is_github_copilot());
    }

    #[test]
    fn provider_manager_get_all_providers_returns_map() {
        let mut manager = ProviderManager::default();
        let provider = Provider::with_id(
            "provider-1".to_string(),
            "Provider".to_string(),
            json!({ "env": {} }),
            None,
        );
        manager.providers.insert("provider-1".to_string(), provider);

        assert_eq!(manager.get_all_providers().len(), 1);
        assert!(manager.get_all_providers().contains_key("provider-1"));
    }

    fn provider_with(settings_config: serde_json::Value) -> Provider {
        Provider::with_id("p".to_string(), "P".to_string(), settings_config, None)
    }

    #[test]
    fn resolve_credentials_codex_auth_and_toml() {
        let p = provider_with(json!({
            "auth": { "OPENAI_API_KEY": "sk-codex" },
            "config": "model_provider = \"deepseek\"\n\
                       [model_providers.deepseek]\n\
                       base_url = \"https://api.deepseek.com\"\n",
        }));
        assert_eq!(
            p.resolve_usage_credentials(&AppType::Codex),
            (
                "https://api.deepseek.com".to_string(),
                "sk-codex".to_string()
            )
        );
    }

    #[test]
    fn resolve_credentials_missing_fields_yield_empty() {
        let p = provider_with(json!({}));
        assert_eq!(
            p.resolve_usage_credentials(&AppType::Codex),
            (String::new(), String::new())
        );
    }
}
