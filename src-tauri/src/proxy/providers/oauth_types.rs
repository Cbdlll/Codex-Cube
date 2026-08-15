//! 通用 OAuth 账号/设备码类型（原 copilot_auth.rs 中共享给 Codex/xAI OAuth 的部分）。
//!
//! GitHub Copilot 支持已移除，但 Codex OAuth / xAI OAuth 的设备码流程与账号
//! 展示复用了这两个结构（字段语义分别对应 OpenAI / xAI 的响应），故保留。

use serde::{Deserialize, Serialize};

pub const DEFAULT_GITHUB_DOMAIN: &str = "github.com";

fn default_github_domain() -> String {
    DEFAULT_GITHUB_DOMAIN.to_string()
}

/// OAuth 设备码流程响应（前端展示用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubDeviceCodeResponse {
    /// 设备码（用于轮询）
    pub device_code: String,
    /// 用户码（显示给用户）
    pub user_code: String,
    /// 验证 URL
    pub verification_uri: String,
    /// 过期时间（秒）
    pub expires_in: u64,
    /// 轮询间隔（秒）
    pub interval: u64,
}

/// OAuth 账号（公开信息，返回给前端）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubAccount {
    /// 账号 ID（字符串形式，作为唯一标识）
    pub id: String,
    /// 登录名
    pub login: String,
    /// 头像 URL
    pub avatar_url: Option<String>,
    /// 认证时间戳
    pub authenticated_at: i64,
    /// 域名（github.com 或 GHES 域名）
    #[serde(default = "default_github_domain")]
    pub github_domain: String,
}
