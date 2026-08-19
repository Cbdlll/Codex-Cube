//! Proxy Session - 请求会话管理
//!
//! 为每个代理请求创建会话上下文，在整个请求生命周期中跟踪状态和元数据。
//!
//! ## Session ID 提取
//!
//! 支持从客户端请求中提取 Session ID，用于关联同一对话的多个请求：
//! - 其他: 生成新的 UUID

use axum::http::HeaderMap;
use uuid::Uuid;

// ============================================================================
// Session ID 提取器
// ============================================================================

/// Session ID 来源
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionIdSource {
    /// 从 metadata.user_id 提取
    MetadataUserId,
    /// 从 metadata.session_id 提取
    MetadataSessionId,
    /// 从 headers 提取
    Header,
    /// 新生成
    Generated,
}

/// Session ID 提取结果
#[derive(Debug, Clone)]
pub struct SessionIdResult {
    /// 提取或生成的 Session ID
    pub session_id: String,
    /// Session ID 来源
    pub source: SessionIdSource,
    /// 是否为客户端提供的 ID（非新生成）
    pub client_provided: bool,
}

/// 从请求中提取或生成 Session ID
///
/// 轻量化实现，仅提取 session_id 用于日志记录，不做复杂的 Session 管理。
///
/// ## 提取优先级
///
/// ### Codex 请求
/// 1. Headers: `session_id` 或 `x-session-id`
/// 2. `metadata.session_id`
/// 3. 生成新 UUID
///
/// ## 示例
///
/// ```ignore
/// let result = extract_session_id(&headers, &body, "codex");
/// println!("Session ID: {} (from {:?})", result.session_id, result.source);
/// ```
pub fn extract_session_id(
    headers: &HeaderMap,
    body: &serde_json::Value,
    client_format: &str,
) -> SessionIdResult {
    // Responses 请求特殊处理（Codex 客户端协议）。
    if matches!(client_format, "codex" | "openai") {
        if let Some(result) = extract_responses_session(headers, body, "codex") {
            return result;
        }
    }

    // 从 metadata 提取
    if let Some(result) = extract_from_metadata(body) {
        return result;
    }

    // 兜底：生成新 Session ID
    generate_new_session_id()
}

/// 提取 Responses 客户端的 Session ID
fn extract_responses_session(
    headers: &HeaderMap,
    body: &serde_json::Value,
    prefix: &str,
) -> Option<SessionIdResult> {
    // 1. 从 headers 提取
    let header_names: &[&str] = &["session_id", "x-session-id"];
    for header_name in header_names {
        if let Some(value) = headers.get(*header_name) {
            if let Ok(session_id) = value.to_str() {
                let session_id = session_id.trim();
                // Responses 客户端的 Session ID 通常较长（UUID 格式）
                if session_id.len() > 20 {
                    return Some(SessionIdResult {
                        session_id: format!("{prefix}_{session_id}"),
                        source: SessionIdSource::Header,
                        client_provided: true,
                    });
                }
            }
        }
    }

    // 2. 从 body.metadata.session_id 提取
    if let Some(session_id) = body
        .get("metadata")
        .and_then(|m| m.get("session_id"))
        .and_then(|v| v.as_str())
    {
        let session_id = session_id.trim();
        if session_id.len() > 10 {
            return Some(SessionIdResult {
                session_id: format!("{prefix}_{session_id}"),
                source: SessionIdSource::MetadataSessionId,
                client_provided: true,
            });
        }
    }

    // previous_response_id 是 Responses 协议里的响应游标，不是稳定会话身份。
    // Chat/Responses 桥接时该值通常来自上游每轮返回的随机 response id；
    // 若把它当 prompt_cache_key 或 Codex session header，会导致每轮请求换缓存 key。

    None
}

/// 从 metadata 提取 Session ID
fn extract_from_metadata(body: &serde_json::Value) -> Option<SessionIdResult> {
    let metadata = body.get("metadata")?;

    // 1. 从 metadata.user_id 提取（格式: user_xxx_session_yyy）
    if let Some(user_id) = metadata.get("user_id").and_then(|v| v.as_str()) {
        if let Some(session_id) = parse_session_from_user_id(user_id) {
            return Some(SessionIdResult {
                session_id,
                source: SessionIdSource::MetadataUserId,
                client_provided: true,
            });
        }
    }

    // 2. 直接从 metadata.session_id 提取
    if let Some(session_id) = metadata.get("session_id").and_then(|v| v.as_str()) {
        if !session_id.is_empty() {
            return Some(SessionIdResult {
                session_id: session_id.to_string(),
                source: SessionIdSource::MetadataSessionId,
                client_provided: true,
            });
        }
    }

    None
}

/// 从 user_id 解析 session_id
///
/// 格式: `user_identifier_session_actual_session_id`
pub(super) fn parse_session_from_user_id(user_id: &str) -> Option<String> {
    // 查找 "_session_" 分隔符
    if let Some(pos) = user_id.find("_session_") {
        let session_id = &user_id[pos + 9..]; // "_session_" 长度为 9
        if !session_id.is_empty() {
            return Some(session_id.to_string());
        }
    }
    None
}

/// Raw Codex thread id for looking up `state_5.sqlite` `threads.model`.
///
/// Unlike [`extract_session_id`], this does not invent a UUID and does not
/// prefix `codex_`. Desktop sends the thread on `thread-id` / `conversation_id`
/// / `session_id` after a fork, even when the `/responses` body still carries
/// the parent turn's model.
pub fn extract_codex_thread_id(headers: &HeaderMap, body: &serde_json::Value) -> Option<String> {
    const HEADER_NAMES: &[&str] = &[
        "thread-id",
        "x-codex-thread-id",
        "conversation_id",
        "session_id",
        "session-id",
        "x-session-id",
        "x-thread-id",
    ];
    for header_name in HEADER_NAMES {
        if let Some(value) = headers
            .get(*header_name)
            .and_then(|value| value.to_str().ok())
        {
            if let Some(thread_id) = crate::codex_state_db::normalize_codex_thread_id(value) {
                return Some(thread_id);
            }
        }
    }

    let metadata = body.get("metadata");
    for key in ["session_id", "thread_id", "conversation_id"] {
        if let Some(value) = metadata
            .and_then(|metadata| metadata.get(key))
            .and_then(|value| value.as_str())
            .and_then(crate::codex_state_db::normalize_codex_thread_id)
        {
            return Some(value);
        }
    }
    for key in ["conversation_id", "thread_id"] {
        if let Some(value) = body
            .get(key)
            .and_then(|value| value.as_str())
            .and_then(crate::codex_state_db::normalize_codex_thread_id)
        {
            return Some(value);
        }
    }
    None
}

/// 生成新的 Session ID
fn generate_new_session_id() -> SessionIdResult {
    SessionIdResult {
        session_id: Uuid::new_v4().to_string(),
        source: SessionIdSource::Generated,
        client_provided: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ========== Session ID 提取测试 ==========

    #[test]
    fn test_codex_previous_response_id_is_not_stable_session_identity() {
        let headers = HeaderMap::new();
        let body = json!({
            "input": "Write a function",
            "previous_response_id": "resp_abc123def456789"
        });

        let result = extract_session_id(&headers, &body, "codex");

        assert!(!result.session_id.is_empty());
        assert_eq!(result.source, SessionIdSource::Generated);
        assert!(!result.client_provided);
    }

    #[test]
    fn test_codex_keeps_existing_response_session_headers() {
        let body = json!({ "input": "Write a function" });

        for header_name in ["session_id", "x-session-id"] {
            let mut headers = HeaderMap::new();
            headers.insert(
                header_name,
                "d937243f-2702-4f20-97b6-c9682235ab81".parse().unwrap(),
            );

            let result = extract_session_id(&headers, &body, "codex");

            assert_eq!(
                result.session_id,
                "codex_d937243f-2702-4f20-97b6-c9682235ab81"
            );
            assert_eq!(result.source, SessionIdSource::Header);
            assert!(result.client_provided);
        }
    }

    #[test]
    fn test_extract_session_generates_new_when_not_found() {
        let headers = HeaderMap::new();
        let body = json!({
            "model": "claude-3-5-sonnet",
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = extract_session_id(&headers, &body, "codex");

        assert!(!result.session_id.is_empty());
        assert_eq!(result.source, SessionIdSource::Generated);
        assert!(!result.client_provided);
    }

    #[test]
    fn test_parse_session_from_user_id() {
        assert_eq!(
            parse_session_from_user_id("user_john_session_abc123"),
            Some("abc123".to_string())
        );
        assert_eq!(
            parse_session_from_user_id("my_app_session_xyz789"),
            Some("xyz789".to_string())
        );
        // 注意: "_session_" 是分隔符，所以下面的字符串会匹配
        assert_eq!(
            parse_session_from_user_id("no_session_marker"),
            Some("marker".to_string())
        );
        // 没有 "_session_" 分隔符的情况
        assert_eq!(parse_session_from_user_id("user_john_abc123"), None);
        assert_eq!(parse_session_from_user_id("_session_"), None);
    }

    #[test]
    fn extract_codex_thread_id_from_thread_id_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "thread-id",
            "01a017fe-7519-7ce2-9c7e-872de1c2394c".parse().unwrap(),
        );
        let body = json!({ "model": "gpt-5.6-sol" });

        assert_eq!(
            extract_codex_thread_id(&headers, &body).as_deref(),
            Some("01a017fe-7519-7ce2-9c7e-872de1c2394c")
        );
    }

    #[test]
    fn extract_codex_thread_id_strips_codex_prefix() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "session_id",
            "codex_01a017fe-7519-7ce2-9c7e-872de1c2394c"
                .parse()
                .unwrap(),
        );
        let body = json!({});

        assert_eq!(
            extract_codex_thread_id(&headers, &body).as_deref(),
            Some("01a017fe-7519-7ce2-9c7e-872de1c2394c")
        );
    }
}
