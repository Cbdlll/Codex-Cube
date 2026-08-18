//! Provider Adapters Module
//!
//! 供应商适配器模块，提供统一的接口抽象不同上游供应商的处理逻辑。
//!
//! ## 模块结构
//! - `adapter`: 定义 `ProviderAdapter` trait
//! - `auth`: 认证类型和策略
//! - `claude`: Claude (Anthropic) 适配器
//! - `codex`: Codex (OpenAI) 适配器
//! - `gemini`: Gemini (Google) 适配器
//! - `models`: API 数据模型
//! - `transform`: 格式转换

mod adapter;
mod auth;
mod codex;
pub(crate) mod codex_chat_common;
pub mod codex_chat_history;
pub mod codex_oauth_auth;
pub(crate) mod codex_responses_sse;
pub(crate) mod oauth_types;
pub mod streaming_codex_anthropic;
pub mod streaming_codex_chat;
pub mod transform_codex_anthropic;
pub mod transform_codex_chat;
pub mod transform_codex_responses_namespace;
pub mod transform_codex_responses_relay_sanitize;
pub mod transform_codex_responses_xai_sanitize;
pub(crate) mod transform_codex_spawn_agent;
pub mod transform_responses;
pub mod xai_oauth_auth;

use crate::app_config::AppType;
use crate::provider::Provider;
use serde::{Deserialize, Serialize};

pub const CHATGPT_CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
pub const XAI_API_BASE_URL: &str = "https://api.x.ai/v1";

// 公开导出
pub use adapter::ProviderAdapter;
pub use auth::{AuthInfo, AuthStrategy};
pub use codex::CodexAdapter;
pub use codex::{
    apply_codex_anthropic_upstream_model, apply_codex_chat_upstream_model,
    apply_codex_upstream_model, codex_provider_upstream_model, inject_codex_chat_prompt_cache_key,
    is_codex_official_provider, normalize_codex_wire_api,
    provider_needs_responses_namespace_flatten, resolve_codex_aggregate_provider,
    resolve_codex_catalog_tool_profile, resolve_codex_chat_reasoning_config,
    should_convert_codex_responses_to_anthropic, should_convert_codex_responses_to_chat,
};

/// 供应商类型枚举
///
/// 区分不同供应商的具体实现方式，决定认证和请求处理逻辑。
/// 本项目仅支持 Codex，保留 Codex 官方 / Codex OAuth / xAI OAuth 三种变体。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    /// OpenAI Codex Response API
    Codex,
    /// OpenAI Codex (ChatGPT Plus/Pro OAuth，需要 Anthropic ↔ Responses API 转换)
    CodexOAuth,
    /// xAI Grok OAuth（需要 Anthropic ↔ Responses API 转换）
    XaiOAuth,
}

impl ProviderType {
    /// 是否需要格式转换
    #[allow(dead_code)]
    pub fn needs_transform(&self) -> bool {
        match self {
            ProviderType::CodexOAuth => true,
            ProviderType::XaiOAuth => true,
            ProviderType::Codex => false,
        }
    }

    /// 获取默认端点
    #[allow(dead_code)]
    pub fn default_endpoint(&self) -> &'static str {
        match self {
            ProviderType::Codex => "https://api.openai.com",
            ProviderType::CodexOAuth => CHATGPT_CODEX_BASE_URL,
            ProviderType::XaiOAuth => XAI_API_BASE_URL,
        }
    }

    /// 从 AppType 和 Provider 配置推断供应商类型
    ///
    /// 根据配置中的 base_url、auth_mode、api_key 格式等信息推断具体的供应商类型
    #[allow(dead_code)]
    pub fn from_app_type_and_config(app_type: &AppType, provider: &Provider) -> Self {
        match app_type {
            AppType::Codex => ProviderType::Codex,
        }
    }

    /// 转换为字符串表示
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderType::Codex => "codex",
            ProviderType::CodexOAuth => "codex_oauth",
            ProviderType::XaiOAuth => "xai_oauth",
        }
    }
}

impl std::fmt::Display for ProviderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for ProviderType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "codex" => Ok(ProviderType::Codex),
            "codex_oauth" | "codex-oauth" | "codexoauth" => Ok(ProviderType::CodexOAuth),
            "xai_oauth" | "xai-oauth" | "xaioauth" => Ok(ProviderType::XaiOAuth),
            _ => Err(format!("Invalid provider type: {s}")),
        }
    }
}

/// 根据 AppType 获取对应的适配器
pub fn get_adapter(app_type: &AppType) -> Box<dyn ProviderAdapter> {
    match app_type {
        AppType::Codex => Box::new(CodexAdapter::new()),
    }
}

/// 根据 ProviderType 获取对应的适配器
#[allow(dead_code)]
pub fn get_adapter_for_provider_type(provider_type: &ProviderType) -> Box<dyn ProviderAdapter> {
    match provider_type {
        _ => Box::new(CodexAdapter::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn create_provider(config: serde_json::Value) -> Provider {
        Provider {
            id: "test".to_string(),
            name: "Test Provider".to_string(),
            settings_config: config,
            website_url: None,
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

    #[test]
    fn test_provider_type_needs_transform() {
        assert!(!ProviderType::Codex.needs_transform());
        assert!(ProviderType::CodexOAuth.needs_transform());
        assert!(ProviderType::XaiOAuth.needs_transform());
    }

    #[test]
    fn test_provider_type_default_endpoint() {
        assert_eq!(
            ProviderType::Codex.default_endpoint(),
            "https://api.openai.com"
        );
        assert_eq!(
            ProviderType::CodexOAuth.default_endpoint(),
            CHATGPT_CODEX_BASE_URL
        );
        assert_eq!(ProviderType::XaiOAuth.default_endpoint(), XAI_API_BASE_URL);
    }

    #[test]
    fn test_provider_type_from_str() {
        assert_eq!(
            "codex".parse::<ProviderType>().unwrap(),
            ProviderType::Codex
        );
        assert_eq!(
            "codex-oauth".parse::<ProviderType>().unwrap(),
            ProviderType::CodexOAuth
        );
        assert_eq!(
            "xai_oauth".parse::<ProviderType>().unwrap(),
            ProviderType::XaiOAuth
        );
        assert!("claude".parse::<ProviderType>().is_err());
        assert!("invalid".parse::<ProviderType>().is_err());
    }

    #[test]
    fn test_provider_type_as_str() {
        assert_eq!(ProviderType::Codex.as_str(), "codex");
        assert_eq!(ProviderType::CodexOAuth.as_str(), "codex_oauth");
        assert_eq!(ProviderType::XaiOAuth.as_str(), "xai_oauth");
    }

    #[test]
    fn test_provider_type_serde() {
        let codex = ProviderType::Codex;
        let serialized = serde_json::to_string(&codex).unwrap();
        assert_eq!(serialized, "\"codex\"");

        let serialized_oauth = serde_json::to_string(&ProviderType::CodexOAuth).unwrap();
        assert_eq!(serialized_oauth, "\"codex_o_auth\"");

        let deserialized: ProviderType = serde_json::from_str("\"codex_o_auth\"").unwrap();
        assert_eq!(deserialized, ProviderType::CodexOAuth);

        let deserialized_xai: ProviderType = serde_json::from_str("\"xai_o_auth\"").unwrap();
        assert_eq!(deserialized_xai, ProviderType::XaiOAuth);
    }

    #[test]
    fn test_from_app_type_codex() {
        let provider = create_provider(json!({
            "env": {
                "OPENAI_API_KEY": "sk-test"
            }
        }));

        let provider_type = ProviderType::from_app_type_and_config(&AppType::Codex, &provider);
        assert_eq!(provider_type, ProviderType::Codex);
    }
}
