//! 请求上下文模块
//!
//! 提供请求生命周期的上下文管理，封装通用初始化逻辑

use crate::app_config::AppType;
use crate::provider::Provider;
use crate::proxy::{
    extract_session_id,
    forwarder::RequestForwarder,
    server::ProxyState,
    types::{AppProxyConfig, CopilotOptimizerConfig, OptimizerConfig, RectifierConfig},
    ProxyError,
};
use axum::http::HeaderMap;
use std::time::Instant;

/// 流式超时配置
#[derive(Debug, Clone, Copy)]
pub struct StreamingTimeoutConfig {
    /// 首字节超时（秒），0 表示禁用
    pub first_byte_timeout: u64,
    /// 静默期超时（秒），0 表示禁用
    pub idle_timeout: u64,
}

/// 请求上下文
///
/// 贯穿整个请求生命周期，包含：
/// - 计时信息
/// - 应用级代理配置（per-app）
/// - 选中的 Provider 列表（用于故障转移）
/// - 请求模型名称
/// - 日志标签
/// - Session ID（用于日志关联）
pub struct RequestContext {
    /// 请求开始时间
    pub start_time: Instant,
    /// 应用级代理配置（per-app，包含重试次数和超时配置）
    pub app_config: AppProxyConfig,
    /// 选中的 Provider（故障转移链的第一个）
    pub provider: Provider,
    /// 完整的 Provider 列表（用于故障转移）
    providers: Vec<Provider>,
    /// 请求开始时的"当前供应商"（用于判断是否需要同步 UI/托盘）
    ///
    /// 这里使用本地 settings 的设备级 current provider。
    /// 代理模式下如果实际使用的 provider 与此不一致，会触发切换以确保 UI 始终准确。
    pub current_provider_id: String,
    /// 请求中的模型名称
    pub request_model: String,
    /// 实际发往上游的模型名（路由接管/模型映射后的真值，forward 成功后回填）。
    ///
    /// usage 归因的兜底顺序：上游响应回显 → outbound_model → request_model。
    /// 不能直接用 request_model 兜底：接管场景下它是映射前的客户端别名。
    pub outbound_model: Option<String>,
    /// 日志标签（如 "Claude"、"Codex"、"Gemini"）
    pub tag: &'static str,
    /// 应用类型字符串（如 "claude"、"codex"、"gemini"）
    pub app_type_str: &'static str,
    /// 应用类型（预留，目前通过 app_type_str 使用）
    #[allow(dead_code)]
    pub app_type: AppType,
    /// Session ID（从客户端请求提取或新生成）
    pub session_id: String,
    /// Session ID 是否由客户端提供。生成的 UUID 不能作为上游缓存 key，否则每个请求都会换 key。
    pub session_client_provided: bool,
    /// 整流器配置
    pub rectifier_config: RectifierConfig,
    /// 优化器配置
    pub optimizer_config: OptimizerConfig,
    /// Copilot 优化器配置
    pub copilot_optimizer_config: CopilotOptimizerConfig,
}

/// Resolve the aggregate Codex provider that should route the current request.
///
/// Hot switches update the running target only after the generated catalog and
/// persisted selection are ready. A stale aggregate target from an older build
/// must still yield to a persisted regular provider so old Desktop GPT slots do
/// not route a direct subscription through an aggregate mapping.
pub(crate) async fn effective_codex_aggregate_provider(
    state: &ProxyState,
) -> Option<crate::provider::Provider> {
    let persisted = crate::settings::get_effective_current_provider(&state.db, &AppType::Codex)
        .ok()
        .flatten()
        .or_else(|| state.db.get_current_provider("codex").ok().flatten())
        .and_then(|id| state.db.get_provider_by_id(&id, "codex").ok().flatten());

    let runtime = state
        .provider_router
        .get_runtime_current_provider("codex")
        .await;
    if let Some(id) = runtime {
        if let Some(provider) = state.db.get_provider_by_id(&id, "codex").ok().flatten() {
            if provider.is_aggregate()
                && persisted
                    .as_ref()
                    .is_some_and(|current| !current.is_aggregate())
            {
                return None;
            }
            // A valid runtime target is authoritative during the short switch
            // commit window. Hot-switch completion synchronizes it to persisted.
            return provider.is_aggregate().then_some(provider);
        }
        // A deleted runtime target falls back to the persisted selection, matching
        // ProviderRouter::select_providers.
    }

    persisted.filter(|provider| provider.is_aggregate())
}

/// Codex Desktop fork + model-picker: honor `state_5.sqlite` `threads.model`
/// when the `/responses` body still carries the parent turn's model.
pub(crate) async fn reconcile_codex_request_model_with_thread(
    state: &ProxyState,
    body: &mut serde_json::Value,
    headers: &HeaderMap,
) {
    reconcile_codex_request_model_with_thread_using(
        state,
        body,
        headers,
        crate::codex_state_db::read_custom_thread_model,
    )
    .await;
}

async fn reconcile_codex_request_model_with_thread_using<F>(
    state: &ProxyState,
    body: &mut serde_json::Value,
    headers: &HeaderMap,
    read_thread_model: F,
) where
    F: Fn(&str) -> Option<String>,
{
    if headers.get("x-openai-subagent").is_some() {
        return;
    }
    let Some(request_model) = body
        .get("model")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_string)
    else {
        return;
    };
    if crate::codex_agent_workflow::is_registered_dispatch_model(
        &crate::codex_config::get_codex_config_dir(),
        &request_model,
    ) {
        return;
    }
    let Some(thread_id) = crate::proxy::session::extract_codex_thread_id(headers, body) else {
        return;
    };
    let Some(thread_model) = read_thread_model(&thread_id) else {
        return;
    };
    let allowed = allowed_codex_thread_models(state).await;
    let Some(preferred) = preferred_custom_thread_request_model(
        &request_model,
        &thread_model,
        &allowed,
        false,
        false,
    ) else {
        return;
    };
    log::info!(
        "[Codex] 线程 `{thread_id}` 已选择 `{preferred}`，覆盖请求中的过期模型 `{request_model}`"
    );
    body["model"] = serde_json::Value::String(preferred);
}

fn preferred_custom_thread_request_model(
    request_model: &str,
    thread_model: &str,
    allowed_models: &[String],
    request_is_dispatch_worker: bool,
    is_subagent: bool,
) -> Option<String> {
    if is_subagent || request_is_dispatch_worker {
        return None;
    }
    let request_model = request_model.trim();
    let thread_model = thread_model.trim();
    if request_model.is_empty() || thread_model.is_empty() || request_model == thread_model {
        return None;
    }
    if allowed_models.is_empty() || !allowed_models.iter().any(|model| model == thread_model) {
        return None;
    }
    Some(thread_model.to_string())
}

async fn allowed_codex_thread_models(state: &ProxyState) -> Vec<String> {
    if let Some(aggregate) = effective_codex_aggregate_provider(state).await {
        let mut ids = Vec::new();
        for entry in crate::codex_config::codex_aggregate_model_entries(&aggregate.settings_config)
        {
            if !ids.iter().any(|model| model == &entry.model) {
                ids.push(entry.model.clone());
            }
            if let Some(upstream) = entry.upstream_model {
                if !ids.iter().any(|model| model == &upstream) {
                    ids.push(upstream);
                }
            }
        }
        return ids;
    }

    let current = crate::settings::get_effective_current_provider(&state.db, &AppType::Codex)
        .ok()
        .flatten()
        .or_else(|| state.db.get_current_provider("codex").ok().flatten())
        .and_then(|id| state.db.get_provider_by_id(&id, "codex").ok().flatten());
    let Some(provider) = current else {
        return Vec::new();
    };
    let mut ids = crate::codex_state_db::provider_catalog_model_ids(&provider);
    if let Some(upstream) = crate::proxy::providers::codex_provider_upstream_model(&provider) {
        if !ids.iter().any(|model| model == &upstream) {
            ids.push(upstream);
        }
    }
    ids
}

/// If a cube-dispatch child is talking to the coordinator's proxy with the
/// registered worker model, send that request to the worker's Cube provider
/// instead of rewriting the model onto the coordinator.
fn route_codex_dispatch_child_provider(
    state: &ProxyState,
    request_model: &str,
    providers: Vec<Provider>,
    provider: Provider,
) -> (Vec<Provider>, Provider) {
    let catalog_has_model = provider
        .settings_config
        .get("modelCatalog")
        .and_then(|catalog| catalog.get("models"))
        .and_then(|models| models.as_array())
        .is_some_and(|models| {
            models.iter().any(|entry| {
                entry
                    .get("model")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    == Some(request_model.trim())
            })
        });
    if catalog_has_model {
        return (providers, provider);
    }
    let Some(route) = crate::codex_agent_workflow::resolve_dispatch_route_for_model(
        &crate::codex_config::get_codex_config_dir(),
        request_model,
    ) else {
        return (providers, provider);
    };
    let dispatch_provider = route
        .cube_provider_id
        .as_deref()
        .filter(|id| *id != provider.id)
        .and_then(|id| state.db.get_provider_by_id(id, "codex").ok().flatten())
        .filter(|found| found.id != provider.id)
        .or_else(|| {
            crate::codex_subagent_providers::match_existing_cube_provider(
                state.db.as_ref(),
                &crate::codex_config::get_codex_config_dir(),
                &route.agent_name,
                request_model,
            )
            .filter(|found| found.id != provider.id)
        });
    let Some(dispatch_provider) = dispatch_provider else {
        return (providers, provider);
    };
    log::info!(
        "[Codex] cube-dispatch 模型 `{}` 路由到注册供应商 `{}` ({})",
        request_model,
        dispatch_provider.name,
        dispatch_provider.id
    );
    (vec![dispatch_provider.clone()], dispatch_provider)
}

impl RequestContext {
    /// 创建请求上下文
    ///
    /// # Arguments
    /// * `state` - 代理服务器状态
    /// * `body` - 请求体 JSON
    /// * `headers` - 请求头（用于提取 Session ID）
    /// * `app_type` - 应用类型
    /// * `tag` - 日志标签
    /// * `app_type_str` - 应用类型字符串
    ///
    /// # Errors
    /// 返回 `ProxyError` 如果 Provider 选择失败
    pub async fn new(
        state: &ProxyState,
        body: &serde_json::Value,
        headers: &HeaderMap,
        app_type: AppType,
        tag: &'static str,
        app_type_str: &'static str,
    ) -> Result<Self, ProxyError> {
        let start_time = Instant::now();

        // 从数据库读取应用级代理配置（per-app）
        let app_config = state
            .db
            .get_proxy_config_for_app(app_type_str)
            .await
            .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;

        // 从数据库读取整流器配置
        let rectifier_config = state.db.get_rectifier_config().unwrap_or_default();
        let optimizer_config = state.db.get_optimizer_config().unwrap_or_default();
        let copilot_optimizer_config = state.db.get_copilot_optimizer_config().unwrap_or_default();

        let mut current_provider_id =
            crate::settings::get_current_provider(&app_type).unwrap_or_default();

        // 从请求体提取模型名称
        let request_model = body
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown")
            .to_string();

        // 提取 Session ID
        let session_result = extract_session_id(headers, body, app_type_str);
        let session_id = session_result.session_id.clone();

        log::debug!(
            "[{}] Session ID: {} (from {:?}, client_provided: {})",
            tag,
            session_id,
            session_result.source,
            session_result.client_provided
        );

        // Codex 聚合 Provider：请求按模型名路由到成员供应商。命中后直接把成员
        // 作为唯一候选——不参与普通 failover、不消耗熔断名额、不触发切换；
        // 未命中返回 400，提示用户在聚合 Provider 中配置该模型。
        let aggregate_provider = if app_type == AppType::Codex {
            effective_codex_aggregate_provider(&state).await
        } else {
            None
        };

        let (providers, provider) = if let Some(aggregate) = aggregate_provider {
            let member = crate::proxy::providers::resolve_codex_aggregate_provider(
                &state.db,
                &aggregate,
                &request_model,
            )
            .map_err(|e| ProxyError::DatabaseError(e.to_string()))?;
            match member {
                Some(member) => {
                    log::info!(
                        "[Codex] 聚合 Provider 模型 `{request_model}` 路由到供应商 `{}`",
                        member.name
                    );
                    current_provider_id = member.id.clone();
                    (vec![member.clone()], member)
                }
                None => {
                    return Err(ProxyError::InvalidRequest(format!(
                        "Codex 聚合 Provider 未配置模型 `{request_model}`，请在 Codex-Cube 中检查聚合 Provider 的模型列表"
                    )));
                }
            }
        } else {
            // 使用共享的 ProviderRouter 选择 Provider（熔断器状态跨请求保持）
            // 注意：只在这里调用一次，结果传递给 forwarder，避免重复消耗 HalfOpen 名额
            let providers = state
                .provider_router
                .select_providers(app_type_str)
                .await
                .map_err(|e| match e {
                    crate::error::AppError::AllProvidersCircuitOpen => {
                        ProxyError::AllProvidersCircuitOpen
                    }
                    crate::error::AppError::NoProvidersConfigured => {
                        ProxyError::NoProvidersConfigured
                    }
                    _ => ProxyError::DatabaseError(e.to_string()),
                })?;

            let provider = providers
                .first()
                .cloned()
                .ok_or(ProxyError::NoAvailableProvider)?;
            route_codex_dispatch_child_provider(state, &request_model, providers, provider)
        };

        log::debug!(
            "[{}] Provider: {}, model: {}, failover chain: {} providers, session: {}",
            tag,
            provider.name,
            request_model,
            providers.len(),
            session_id
        );

        Ok(Self {
            start_time,
            app_config,
            provider,
            providers,
            current_provider_id,
            request_model,
            outbound_model: None,
            tag,
            app_type_str,
            app_type,
            session_id,
            session_client_provided: session_result.client_provided,
            rectifier_config,
            optimizer_config,
            copilot_optimizer_config,
        })
    }

    /// 从 URI 提取模型名称（Gemini 专用）
    ///
    /// Gemini API 的模型名称在 URI 中，格式如：
    /// `/v1beta/models/gemini-pro:generateContent`

    /// 创建 RequestForwarder
    ///
    /// 使用共享的 ProviderRouter，确保熔断器状态跨请求保持
    ///
    /// 配置生效规则：
    /// - 故障转移开启：超时配置正常生效（0 表示禁用超时）
    /// - 故障转移关闭：超时配置不生效（全部传入 0）
    pub fn create_forwarder(&self, state: &ProxyState) -> RequestForwarder {
        let (non_streaming_timeout, first_byte_timeout, idle_timeout) =
            if self.app_config.auto_failover_enabled {
                // 故障转移开启：使用配置的值（0 = 禁用超时）
                (
                    self.app_config.non_streaming_timeout as u64,
                    self.app_config.streaming_first_byte_timeout as u64,
                    self.app_config.streaming_idle_timeout as u64,
                )
            } else {
                // 故障转移关闭：不启用超时配置
                log::debug!(
                    "[{}] Failover disabled, timeout configs are bypassed",
                    self.tag
                );
                (0, 0, 0)
            };

        // 重试次数尊重用户配置（默认 3）：故障转移关闭时仍允许同一 Provider 的
        // 瞬态传输错误（连接失败/超时）自动重试，不切换 Provider；0 表示不重试。
        // 与「故障转移关闭 = 只用当前 Provider」语义不冲突——重试同一 Provider
        // 不是故障转移。5xx/429 等非传输错误不会对同一上游重试（见
        // forward_with_retry_inner 的单 Provider 守卫）。
        let max_retries = self.app_config.max_retries;

        RequestForwarder::new(
            state.provider_router.clone(),
            non_streaming_timeout,
            state.status.clone(),
            state.current_providers.clone(),
            state.codex_chat_history.clone(),
            state.failover_manager.clone(),
            state.app_handle.clone(),
            self.current_provider_id.clone(),
            self.session_id.clone(),
            self.session_client_provided,
            first_byte_timeout,
            idle_timeout,
            self.rectifier_config.clone(),
            self.optimizer_config.clone(),
            self.copilot_optimizer_config.clone(),
            max_retries,
        )
    }

    /// 获取 Provider 列表（用于故障转移）
    ///
    /// 返回在创建上下文时已选择的 providers，避免重复调用 select_providers()
    pub fn get_providers(&self) -> Vec<Provider> {
        self.providers.clone()
    }

    /// 计算请求延迟（毫秒）
    #[inline]
    pub fn latency_ms(&self) -> u64 {
        self.start_time.elapsed().as_millis() as u64
    }

    /// 获取流式超时配置
    ///
    /// 配置生效规则：
    /// - 故障转移开启：返回配置的值（0 表示禁用超时检查）
    /// - 故障转移关闭：返回 0（禁用超时检查）
    #[inline]
    pub fn streaming_timeout_config(&self) -> StreamingTimeoutConfig {
        if self.app_config.auto_failover_enabled {
            // 故障转移开启：使用配置的值（0 = 禁用超时）
            StreamingTimeoutConfig {
                first_byte_timeout: self.app_config.streaming_first_byte_timeout as u64,
                idle_timeout: self.app_config.streaming_idle_timeout as u64,
            }
        } else {
            // 故障转移关闭：禁用流式超时检查
            StreamingTimeoutConfig {
                first_byte_timeout: 0,
                idle_timeout: 0,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_config::AppType;
    use crate::database::Database;
    use crate::provider::Provider;
    use crate::proxy::{
        failover_switch::FailoverSwitchManager, provider_router::ProviderRouter,
        providers::codex_chat_history::CodexChatHistoryStore, server::ProxyState, ProxyConfig,
        ProxyError, ProxyStatus,
    };

    struct TempHome {
        #[allow(dead_code)]
        dir: tempfile::TempDir,
        original_home: Option<String>,
        original_userprofile: Option<String>,
        original_test_home: Option<String>,
    }

    impl TempHome {
        fn new() -> Self {
            let dir = tempfile::TempDir::new().expect("create temp home");
            let original_home = std::env::var("HOME").ok();
            let original_userprofile = std::env::var("USERPROFILE").ok();
            let original_test_home = std::env::var("CODEX_CUBE_TEST_HOME").ok();
            std::env::set_var("HOME", dir.path());
            std::env::set_var("USERPROFILE", dir.path());
            std::env::set_var("CODEX_CUBE_TEST_HOME", dir.path());
            crate::settings::reload_settings().expect("reload temp settings");
            Self {
                dir,
                original_home,
                original_userprofile,
                original_test_home,
            }
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            match &self.original_home {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
            match &self.original_test_home {
                Some(value) => std::env::set_var("CODEX_CUBE_TEST_HOME", value),
                None => std::env::remove_var("CODEX_CUBE_TEST_HOME"),
            }
            match &self.original_userprofile {
                Some(value) => std::env::set_var("USERPROFILE", value),
                None => std::env::remove_var("USERPROFILE"),
            }
        }
    }

    fn build_state(db: std::sync::Arc<Database>) -> ProxyState {
        ProxyState {
            db: db.clone(),
            config: std::sync::Arc::new(tokio::sync::RwLock::new(ProxyConfig::default())),
            status: std::sync::Arc::new(tokio::sync::RwLock::new(ProxyStatus::default())),
            start_time: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
            current_providers: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            provider_router: std::sync::Arc::new(ProviderRouter::new(db.clone())),
            codex_chat_history: std::sync::Arc::new(CodexChatHistoryStore::default()),
            app_handle: None,
            failover_manager: std::sync::Arc::new(FailoverSwitchManager::new(db)),
        }
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn aggregate_route_resolves_member_before_failover_queue() {
        let _home = TempHome::new();
        let db = std::sync::Arc::new(Database::memory().expect("memory db"));

        let member = Provider::with_id(
            "deepseek".to_string(),
            "DeepSeek".to_string(),
            serde_json::json!({
                "auth": { "OPENAI_API_KEY": "sk-deepseek" },
                "config": "base_url = \"https://api.deepseek.com/v1\"\nwire_api = \"responses\"",
                "model": "deepseek-chat"
            }),
            None,
        );
        db.save_provider("codex", &member).expect("save member");

        let mut aggregate = Provider::with_id(
            "agg-test".to_string(),
            "My Aggregate".to_string(),
            serde_json::json!({
                "auth": {},
                "config": "",
                "aggregateModels": [{
                    "model": "deepseek-chat@deepseek",
                    "providerId": "deepseek",
                    "upstreamModel": "deepseek-chat"
                }]
            }),
            None,
        );
        aggregate.meta = Some(crate::provider::ProviderMeta {
            provider_type: Some("aggregate".to_string()),
            ..Default::default()
        });
        db.save_provider("codex", &aggregate)
            .expect("save aggregate");
        db.set_current_provider("codex", "agg-test")
            .expect("set current provider");

        let state = build_state(db.clone());
        let body = serde_json::json!({ "model": "deepseek-chat@deepseek", "input": "hi" });
        let headers = axum::http::HeaderMap::new();

        let ctx = RequestContext::new(&state, &body, &headers, AppType::Codex, "Codex", "codex")
            .await
            .expect("create context");

        assert_eq!(
            ctx.provider.id, "deepseek",
            "aggregate must route to the member"
        );
        assert_eq!(ctx.request_model, "deepseek-chat@deepseek");
        // 成员路由必须旁路普通 failover 链（providers 只含成员自己）
        assert_eq!(ctx.get_providers().len(), 1);
        assert_eq!(ctx.current_provider_id, "deepseek");
    }

    fn install_dispatch_worker(dir: &std::path::Path, name: &str, model: &str, provider_id: &str) {
        std::fs::create_dir_all(dir.join("agents")).unwrap();
        std::fs::write(
            dir.join("agents").join(format!("{name}.toml")),
            format!("name = \"{name}\"\nmodel = \"{model}\"\nmodel_reasoning_effort = \"high\"\n"),
        )
        .unwrap();
        let mut roles = crate::codex_agent_workflow::RoleAgents::default();
        roles.worker = vec![name.to_string()];
        crate::codex_agent_workflow::install(
            dir,
            name,
            &[name.to_string()],
            &roles,
            crate::codex_agent_workflow::WORKFLOW_MODE_SKILL,
            "high",
        )
        .unwrap();
        std::fs::write(
            dir.join(crate::codex_subagents::SUBAGENT_MANIFEST_FILENAME),
            format!(r#"{{"agents":[{{"name":"{name}","providerId":"{provider_id}"}}]}}"#),
        )
        .unwrap();
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn dispatch_child_routes_to_registered_worker_provider() {
        let _home = TempHome::new();
        let db = std::sync::Arc::new(Database::memory().expect("memory db"));
        let dir = crate::codex_config::get_codex_config_dir();
        std::fs::create_dir_all(&dir).unwrap();
        install_dispatch_worker(&dir, "grok-4-6", "grok-4.6", "grok-worker");

        let coordinator = Provider::with_id(
            "coordinator".to_string(),
            "Coordinator".to_string(),
            serde_json::json!({
                "auth": { "OPENAI_API_KEY": "sk-coord" },
                "config": "base_url = \"https://opencode.ai/zen/go/v1\"\nwire_api = \"responses\"",
                "model": "deepseek-v4-flash",
                "modelCatalog": { "models": [{ "model": "deepseek-v4-flash" }] }
            }),
            None,
        );
        let worker = Provider::with_id(
            "grok-worker".to_string(),
            "Grok worker".to_string(),
            serde_json::json!({
                "auth": { "OPENAI_API_KEY": "sk-grok" },
                "config": "base_url = \"https://790053500.com/v1\"\nwire_api = \"responses\"",
                "model": "grok-4.6",
                "modelCatalog": { "models": [{ "model": "grok-4.6" }] }
            }),
            None,
        );
        db.save_provider("codex", &coordinator)
            .expect("save coordinator");
        db.save_provider("codex", &worker).expect("save worker");
        db.set_current_provider("codex", "coordinator")
            .expect("set current");
        crate::settings::set_current_provider(&AppType::Codex, Some("coordinator"))
            .expect("set settings current");

        let state = build_state(db.clone());
        let body = serde_json::json!({ "model": "grok-4.6", "input": "hi" });
        let headers = axum::http::HeaderMap::new();
        let ctx = RequestContext::new(&state, &body, &headers, AppType::Codex, "Codex", "codex")
            .await
            .expect("create context");

        assert_eq!(ctx.provider.id, "grok-worker");
        assert_eq!(ctx.get_providers().len(), 1);
        assert_eq!(
            ctx.current_provider_id, "coordinator",
            "dispatch child routing must not switch the UI current provider"
        );
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn dispatch_child_matches_worker_url_when_slug_is_not_a_cube_id() {
        let _home = TempHome::new();
        let db = std::sync::Arc::new(Database::memory().expect("memory db"));
        let dir = crate::codex_config::get_codex_config_dir();
        std::fs::create_dir_all(&dir).unwrap();
        install_dispatch_worker(&dir, "grok-4-6", "grok-4.6", "grok-4-6");
        std::fs::write(
            dir.join("agents/grok-4-6.toml"),
            "name = \"grok-4-6\"\nmodel = \"grok-4.6\"\nmodel_reasoning_effort = \"high\"\n\n[model_providers.custom]\nbase_url = \"https://790053500.com\"\nwire_api = \"responses\"\n",
        )
        .unwrap();

        let coordinator = Provider::with_id(
            "coordinator".to_string(),
            "Coordinator".to_string(),
            serde_json::json!({
                "auth": { "OPENAI_API_KEY": "sk-coord" },
                "config": "base_url = \"https://opencode.ai/zen/go/v1\"\nwire_api = \"responses\"",
                "model": "deepseek-v4-flash",
                "modelCatalog": { "models": [{ "model": "deepseek-v4-flash" }] }
            }),
            None,
        );
        let worker = Provider::with_id(
            "xai-uuid".to_string(),
            "xAI (Grok)".to_string(),
            serde_json::json!({
                "auth": { "OPENAI_API_KEY": "sk-grok" },
                "config": "model_provider = \"custom\"\nmodel = \"grok-4.6\"\n\n[model_providers.custom]\nbase_url = \"https://790053500.com/v1\"\nwire_api = \"responses\"",
                "modelCatalog": { "models": [{ "model": "grok-4.6" }] }
            }),
            None,
        );
        db.save_provider("codex", &coordinator)
            .expect("save coordinator");
        db.save_provider("codex", &worker).expect("save worker");
        db.set_current_provider("codex", "coordinator")
            .expect("set current");
        crate::settings::set_current_provider(&AppType::Codex, Some("coordinator"))
            .expect("set settings current");

        let state = build_state(db.clone());
        let body = serde_json::json!({ "model": "grok-4.6", "input": "hi" });
        let headers = axum::http::HeaderMap::new();
        let ctx = RequestContext::new(&state, &body, &headers, AppType::Codex, "Codex", "codex")
            .await
            .expect("create context");

        assert_eq!(ctx.provider.id, "xai-uuid");
        assert_eq!(ctx.get_providers().len(), 1);
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn dispatch_child_keeps_coordinator_when_catalog_lists_the_model() {
        let _home = TempHome::new();
        let db = std::sync::Arc::new(Database::memory().expect("memory db"));
        let dir = crate::codex_config::get_codex_config_dir();
        std::fs::create_dir_all(&dir).unwrap();
        install_dispatch_worker(&dir, "grok-4-6", "grok-4.6", "grok-worker");

        let coordinator = Provider::with_id(
            "coordinator".to_string(),
            "Coordinator".to_string(),
            serde_json::json!({
                "auth": { "OPENAI_API_KEY": "sk-coord" },
                "config": "base_url = \"https://opencode.ai/zen/go/v1\"\nwire_api = \"responses\"",
                "model": "deepseek-v4-flash",
                "modelCatalog": {
                    "models": [
                        { "model": "deepseek-v4-flash" },
                        { "model": "grok-4.6" }
                    ]
                }
            }),
            None,
        );
        let worker = Provider::with_id(
            "grok-worker".to_string(),
            "Grok worker".to_string(),
            serde_json::json!({
                "auth": { "OPENAI_API_KEY": "sk-grok" },
                "config": "base_url = \"https://790053500.com/v1\"\nwire_api = \"responses\"",
                "model": "grok-4.6"
            }),
            None,
        );
        db.save_provider("codex", &coordinator)
            .expect("save coordinator");
        db.save_provider("codex", &worker).expect("save worker");
        db.set_current_provider("codex", "coordinator")
            .expect("set current");
        crate::settings::set_current_provider(&AppType::Codex, Some("coordinator"))
            .expect("set settings current");

        let state = build_state(db.clone());
        let body = serde_json::json!({ "model": "grok-4.6", "input": "hi" });
        let headers = axum::http::HeaderMap::new();
        let ctx = RequestContext::new(&state, &body, &headers, AppType::Codex, "Codex", "codex")
            .await
            .expect("create context");

        assert_eq!(ctx.provider.id, "coordinator");
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn aggregate_unmapped_model_rejected_with_400() {
        let _home = TempHome::new();
        let db = std::sync::Arc::new(Database::memory().expect("memory db"));

        let mut aggregate = Provider::with_id(
            "agg-test".to_string(),
            "My Aggregate".to_string(),
            serde_json::json!({
                "auth": {},
                "config": "",
                "aggregateModels": [{
                    "model": "deepseek-chat@deepseek",
                    "providerId": "deepseek",
                    "upstreamModel": "deepseek-chat"
                }]
            }),
            None,
        );
        aggregate.meta = Some(crate::provider::ProviderMeta {
            provider_type: Some("aggregate".to_string()),
            ..Default::default()
        });
        db.save_provider("codex", &aggregate)
            .expect("save aggregate");
        db.set_current_provider("codex", "agg-test")
            .expect("set current provider");

        let state = build_state(db.clone());
        let body = serde_json::json!({ "model": "gpt-5.5", "input": "hi" });
        let headers = axum::http::HeaderMap::new();

        let result =
            RequestContext::new(&state, &body, &headers, AppType::Codex, "Codex", "codex").await;
        let err = result.err().expect("unmapped model must be rejected");
        match err {
            ProxyError::InvalidRequest(message) => {
                assert!(message.contains("未配置模型 `gpt-5.5`"), "{message}");
            }
            other => panic!("expected InvalidRequest, got {other:?}"),
        }
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn aggregate_upstream_model_request_hits_own_entry_not_first_fallback() {
        let _home = TempHome::new();
        let db = std::sync::Arc::new(Database::memory().expect("memory db"));

        let member = Provider::with_id(
            "deepseek".to_string(),
            "DeepSeek".to_string(),
            serde_json::json!({
                "auth": { "OPENAI_API_KEY": "sk-deepseek" },
                "config": "base_url = \"https://api.deepseek.com/v1\"\nwire_api = \"responses\"",
                "model": "deepseek-chat"
            }),
            None,
        );
        db.save_provider("codex", &member).expect("save member");

        let mut aggregate = Provider::with_id(
            "agg-test".to_string(),
            "My Aggregate".to_string(),
            serde_json::json!({
                "auth": {},
                "config": "",
                "aggregateModels": [
                    {
                        "model": "gpt-5.6-sol@coordinator",
                        "providerId": "coordinator",
                        "upstreamModel": "gpt-5.6-sol"
                    },
                    {
                        "model": "deepseek-chat@deepseek",
                        "providerId": "deepseek",
                        "upstreamModel": "deepseek-chat"
                    }
                ]
            }),
            None,
        );
        aggregate.meta = Some(crate::provider::ProviderMeta {
            provider_type: Some("aggregate".to_string()),
            ..Default::default()
        });
        db.save_provider("codex", &aggregate)
            .expect("save aggregate");
        db.set_current_provider("codex", "agg-test")
            .expect("set current provider");

        let state = build_state(db.clone());

        // 请求上游模型名 `deepseek-chat`：必须命中 deepseek 条目自身，
        // 而不是被其他槽位替换。
        let body = serde_json::json!({ "model": "deepseek-chat", "input": "hi" });
        let headers = axum::http::HeaderMap::new();
        let ctx = RequestContext::new(&state, &body, &headers, AppType::Codex, "Codex", "codex")
            .await
            .expect("create context");
        assert_eq!(
            ctx.provider.id, "deepseek",
            "upstream model request must route to the mapped member"
        );
        assert_eq!(ctx.get_providers().len(), 1);
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn old_session_routes_to_new_provider_after_provider_switch() {
        let _home = TempHome::new();
        let db = std::sync::Arc::new(Database::memory().expect("memory db"));

        // 旧供应商 neko：旧会话曾由它成功服务
        let neko = Provider::with_id(
            "neko".to_string(),
            "Neko".to_string(),
            serde_json::json!({
                "auth": { "OPENAI_API_KEY": "sk-neko" },
                "config": "base_url = \"https://neko.example/v1\"\nwire_api = \"responses\"",
                "model": "gpt-5.6-terra"
            }),
            None,
        );
        db.save_provider("codex", &neko).expect("save neko");

        // 当前选中供应商 opencode（普通供应商，非聚合）：已有会话应跟随
        // Provider 切换；旧会话携带的过期 model 会回退到其配置的上游模型。
        let opencode = Provider::with_id(
            "opencode".to_string(),
            "OpenCode".to_string(),
            serde_json::json!({
                "auth": { "OPENAI_API_KEY": "sk-opencode" },
                "config": "base_url = \"https://opencode.example/v1\"\nwire_api = \"responses\"",
                "model": "deepseek-v4-flash"
            }),
            None,
        );
        db.save_provider("codex", &opencode).expect("save opencode");
        db.set_current_provider("codex", "opencode")
            .expect("set current provider");
        crate::settings::set_current_provider(&AppType::Codex, Some("opencode"))
            .expect("set device current provider");

        // 旧会话的成功请求日志（切换之前由 neko 服务）
        let session_id = "codex_session-after-switch-019f227b-ff9e-7ef2-8f65-4132b9a753a1";
        let logger = crate::proxy::usage::logger::UsageLogger::new(&db);
        logger
            .log_with_calculation(
                "req-old".to_string(),
                "neko".to_string(),
                "codex".to_string(),
                "gpt-5.6-terra".to_string(),
                "gpt-5.6-terra".to_string(),
                "gpt-5.6-terra".to_string(),
                Default::default(),
                Default::default(),
                10,
                None,
                200,
                Some(session_id.to_string()),
                None,
                false,
            )
            .expect("log old success");

        let state = build_state(db.clone());
        let body = serde_json::json!({ "model": "gpt-5.6-terra", "input": "hi" });
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "x-session-id",
            "session-after-switch-019f227b-ff9e-7ef2-8f65-4132b9a753a1"
                .parse()
                .unwrap(),
        );

        let ctx = RequestContext::new(&state, &body, &headers, AppType::Codex, "Codex", "codex")
            .await
            .expect("create context");
        assert_eq!(
            ctx.provider.id, "opencode",
            "old session must follow the provider switch instead of sticking to neko"
        );
        assert_eq!(
            ctx.provider
                .settings_config
                .get("model")
                .and_then(|value| value.as_str()),
            Some("deepseek-v4-flash"),
            "the new provider configuration must be selected after the switch"
        );
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn stale_runtime_aggregate_does_not_capture_persisted_direct_provider() {
        let _home = TempHome::new();
        let db = std::sync::Arc::new(Database::memory().expect("memory db"));

        // 用户已切回单独的普通供应商（持久化 current = single）。
        let single = Provider::with_id(
            "single".to_string(),
            "OpenCode Direct".to_string(),
            serde_json::json!({
                "auth": { "OPENAI_API_KEY": "sk-single" },
                "config": "base_url = \"https://opencode.ai/zen/go/v1\"\nwire_api = \"responses\"",
                "model": "deepseek-v4-flash",
                "modelCatalog": { "models": [{ "model": "deepseek-v4-flash" }] }
            }),
            None,
        );
        db.save_provider("codex", &single).expect("save single");
        db.set_current_provider("codex", "single")
            .expect("set single current");
        crate::settings::set_current_provider(&AppType::Codex, Some("single"))
            .expect("set local single current");

        // 运行态目标意外残留为旧聚合 Provider（例如历史热切换遗漏）。
        let mut aggregate = Provider::with_id(
            "agg-test".to_string(),
            "My Aggregate".to_string(),
            serde_json::json!({
                "auth": {},
                "config": "",
                "aggregateModels": [
                    {
                        "model": "gpt-5.4",
                        "providerId": "single",
                        "upstreamModel": "deepseek-v4-flash"
                    }
                ]
            }),
            None,
        );
        aggregate.meta = Some(crate::provider::ProviderMeta {
            provider_type: Some("aggregate".to_string()),
            ..Default::default()
        });
        db.save_provider("codex", &aggregate)
            .expect("save aggregate");

        let state = build_state(db.clone());
        state
            .provider_router
            .set_runtime_current_provider("codex", "agg-test")
            .await;

        // 旧槽位名不能再激活残留聚合路由：请求必须使用当前的独立 Provider。
        // 它不是 forced aggregate member，但由于 gpt-5.4 不在该 Provider 目录中，
        // 作为旧会话的过期模型会回退到其配置的上游模型。
        let headers = axum::http::HeaderMap::new();
        let ctx = RequestContext::new(
            &state,
            &serde_json::json!({ "model": "gpt-5.4", "input": "hi" }),
            &headers,
            AppType::Codex,
            "Codex",
            "codex",
        )
        .await
        .expect("create context");
        assert_eq!(ctx.provider.id, "single");
        assert_eq!(ctx.get_providers().len(), 1);
        assert!(
            ctx.provider
                .settings_config
                .get("codex_cube_custom_route")
                .is_none(),
            "the direct provider must not inherit the aggregate route marker"
        );

        let mut outbound = serde_json::json!({ "model": "gpt-5.4" });
        let applied =
            crate::proxy::providers::apply_codex_upstream_model(&ctx.provider, &mut outbound);
        assert_eq!(
            applied.as_deref(),
            Some("deepseek-v4-flash"),
            "a stale model outside the provider catalog must fall back to the provider model"
        );
        assert_eq!(
            outbound.get("model").and_then(serde_json::Value::as_str),
            Some("deepseek-v4-flash"),
            "an unsupported native Responses model must be rewritten to the provider model"
        );
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn aggregate_runtime_target_routes_to_new_member_with_its_credentials() {
        let _home = TempHome::new();
        let db = std::sync::Arc::new(Database::memory().expect("memory db"));

        // 切换前的普通 Provider 使用不同凭据，不能泄漏给新聚合成员。
        let single = Provider::with_id(
            "single".to_string(),
            "Single Provider".to_string(),
            serde_json::json!({
                "auth": { "OPENAI_API_KEY": "sk-single" },
                "config": "base_url = \"https://single.example/v1\"\nwire_api = \"responses\"",
                "model": "deepseek-chat"
            }),
            None,
        );
        db.save_provider("codex", &single).expect("save single");

        // 聚合成员是一个与旧单 provider 不同的 id：只有走聚合分支才会路由到它。
        let member = Provider::with_id(
            "member-new".to_string(),
            "Member New".to_string(),
            serde_json::json!({
                "auth": { "OPENAI_API_KEY": "sk-member" },
                "config": "base_url = \"https://member.example/v1\"\nwire_api = \"responses\"",
                "model": "deepseek-chat"
            }),
            None,
        );
        db.save_provider("codex", &member).expect("save member");

        // 持久化 current 已切到聚合 Provider。
        let mut aggregate = Provider::with_id(
            "agg-test".to_string(),
            "My Aggregate".to_string(),
            serde_json::json!({
                "auth": {},
                "config": "",
                "aggregateModels": [
                    {
                        "model": "deepseek-chat@member-new",
                        "providerId": "member-new",
                        "upstreamModel": "deepseek-chat"
                    }
                ]
            }),
            None,
        );
        aggregate.meta = Some(crate::provider::ProviderMeta {
            provider_type: Some("aggregate".to_string()),
            ..Default::default()
        });
        db.save_provider("codex", &aggregate)
            .expect("save aggregate");
        db.set_current_provider("codex", "agg-test")
            .expect("set aggregate current");
        crate::settings::set_current_provider(&AppType::Codex, Some("agg-test"))
            .expect("set local aggregate current");

        let state = build_state(db.clone());
        state
            .provider_router
            .set_runtime_current_provider("codex", "agg-test")
            .await;
        assert!(effective_codex_aggregate_provider(&state).await.is_some());

        // 热切换完成后的下一次请求必须走新成员及其凭据。
        let headers = axum::http::HeaderMap::new();
        let ctx = RequestContext::new(
            &state,
            &serde_json::json!({ "model": "deepseek-chat", "input": "hi" }),
            &headers,
            AppType::Codex,
            "Codex",
            "codex",
        )
        .await
        .expect("create context");
        assert_eq!(ctx.provider.id, "member-new");
        assert_eq!(
            ctx.provider.settings_config["auth"]["OPENAI_API_KEY"], "sk-member",
            "aggregate resolution must load credentials from the mapped member clone"
        );
        assert_ne!(
            ctx.provider.settings_config["auth"]["OPENAI_API_KEY"],
            single.settings_config["auth"]["OPENAI_API_KEY"]
        );
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn aggregate_runtime_refresh_after_restart_activates_persisted_aggregate() {
        let _home = TempHome::new();
        let db = std::sync::Arc::new(Database::memory().expect("memory db"));

        let single = Provider::with_id(
            "single".to_string(),
            "Single Provider".to_string(),
            serde_json::json!({
                "auth": { "OPENAI_API_KEY": "sk-single" },
                "config": "base_url = \"https://single.example/v1\"\nwire_api = \"responses\"",
                "model": "deepseek-chat"
            }),
            None,
        );
        db.save_provider("codex", &single).expect("save single");
        let member = Provider::with_id(
            "member-new".to_string(),
            "Member New".to_string(),
            serde_json::json!({
                "auth": { "OPENAI_API_KEY": "sk-member" },
                "config": "base_url = \"https://member.example/v1\"\nwire_api = \"responses\"",
                "model": "deepseek-chat"
            }),
            None,
        );
        db.save_provider("codex", &member).expect("save member");
        let mut aggregate = Provider::with_id(
            "agg-test".to_string(),
            "My Aggregate".to_string(),
            serde_json::json!({
                "auth": {},
                "config": "",
                "aggregateModels": [
                    {
                        "model": "deepseek-chat@member-new",
                        "providerId": "member-new",
                        "upstreamModel": "deepseek-chat"
                    }
                ]
            }),
            None,
        );
        aggregate.meta = Some(crate::provider::ProviderMeta {
            provider_type: Some("aggregate".to_string()),
            ..Default::default()
        });
        db.save_provider("codex", &aggregate)
            .expect("save aggregate");
        db.set_current_provider("codex", "agg-test")
            .expect("set aggregate current");
        crate::settings::set_current_provider(&AppType::Codex, Some("agg-test"))
            .expect("set local aggregate current");

        let state = build_state(db.clone());
        // 重启后运行态目标被 refresh_active_target_from_current_provider 刷新为
        // 持久化 current（聚合）→ 聚合分支激活。
        state
            .provider_router
            .set_runtime_current_provider("codex", "agg-test")
            .await;

        let headers = axum::http::HeaderMap::new();
        let ctx = RequestContext::new(
            &state,
            &serde_json::json!({ "model": "deepseek-chat", "input": "hi" }),
            &headers,
            AppType::Codex,
            "Codex",
            "codex",
        )
        .await
        .expect("create context");
        assert_eq!(
            ctx.provider.id, "member-new",
            "after restart runtime refresh, aggregate routing must be active"
        );
        assert_eq!(ctx.get_providers().len(), 1);
    }

    #[test]
    fn preferred_custom_thread_model_rewrites_stale_parent() {
        let allowed = vec!["gpt-5.6-sol".to_string(), "kimi-k3".to_string()];
        assert_eq!(
            preferred_custom_thread_request_model("gpt-5.6-sol", "kimi-k3", &allowed, false, false)
                .as_deref(),
            Some("kimi-k3")
        );
        assert_eq!(
            preferred_custom_thread_request_model("kimi-k3", "kimi-k3", &allowed, false, false),
            None
        );
        assert_eq!(
            preferred_custom_thread_request_model("gpt-5.6-sol", "kimi-k3", &allowed, true, false),
            None,
            "dispatch worker requests must keep the worker model"
        );
        assert_eq!(
            preferred_custom_thread_request_model("gpt-5.6-sol", "kimi-k3", &allowed, false, true),
            None,
            "subagent requests must keep the child model"
        );
        assert_eq!(
            preferred_custom_thread_request_model(
                "gpt-5.6-sol",
                "not-in-catalog",
                &allowed,
                false,
                false
            ),
            None
        );
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn fork_thread_model_overrides_stale_parent_request_model() {
        let _home = TempHome::new();
        let db = std::sync::Arc::new(Database::memory().expect("memory db"));

        let neko = Provider::with_id(
            "neko".to_string(),
            "Neko API".to_string(),
            serde_json::json!({
                "auth": { "OPENAI_API_KEY": "sk-neko" },
                "config": "base_url = \"https://neko.example/v1\"\nwire_api = \"responses\"",
                "model": "gpt-5.6-sol"
            }),
            None,
        );
        db.save_provider("codex", &neko).expect("save neko");

        let kimi = Provider::with_id(
            "kimi".to_string(),
            "Kimi".to_string(),
            serde_json::json!({
                "auth": { "OPENAI_API_KEY": "sk-kimi" },
                "config": "base_url = \"https://kimi.example/v1\"\nwire_api = \"responses\"",
                "model": "kimi-k3"
            }),
            None,
        );
        db.save_provider("codex", &kimi).expect("save kimi");

        let mut aggregate = Provider::with_id(
            "agg-test".to_string(),
            "My Aggregate".to_string(),
            serde_json::json!({
                "auth": {},
                "config": "",
                "aggregateModels": [
                    {
                        "model": "gpt-5.6-sol",
                        "providerId": "neko",
                        "upstreamModel": "gpt-5.6-sol"
                    },
                    {
                        "model": "kimi-k3",
                        "providerId": "kimi",
                        "upstreamModel": "kimi-k3"
                    }
                ]
            }),
            None,
        );
        aggregate.meta = Some(crate::provider::ProviderMeta {
            provider_type: Some("aggregate".to_string()),
            ..Default::default()
        });
        db.save_provider("codex", &aggregate)
            .expect("save aggregate");
        db.set_current_provider("codex", "agg-test")
            .expect("set current provider");

        let state = build_state(db.clone());
        let mut body = serde_json::json!({ "model": "gpt-5.6-sol", "input": "hi" });
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "thread-id",
            "01a017fe-7519-7ce2-9c7e-872de1c2394c".parse().unwrap(),
        );

        reconcile_codex_request_model_with_thread_using(&state, &mut body, &headers, |thread_id| {
            assert_eq!(thread_id, "01a017fe-7519-7ce2-9c7e-872de1c2394c");
            Some("kimi-k3".to_string())
        })
        .await;

        assert_eq!(body["model"].as_str(), Some("kimi-k3"));

        let ctx = RequestContext::new(&state, &body, &headers, AppType::Codex, "Codex", "codex")
            .await
            .expect("create context");
        assert_eq!(ctx.provider.id, "kimi");
        assert_eq!(ctx.request_model, "kimi-k3");
        assert_eq!(ctx.provider.name, "Kimi");
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn subagent_header_keeps_stale_request_model() {
        let _home = TempHome::new();
        let db = std::sync::Arc::new(Database::memory().expect("memory db"));

        let neko = Provider::with_id(
            "neko".to_string(),
            "Neko API".to_string(),
            serde_json::json!({
                "auth": { "OPENAI_API_KEY": "sk-neko" },
                "config": "base_url = \"https://neko.example/v1\"\nwire_api = \"responses\"",
                "model": "gpt-5.6-sol"
            }),
            None,
        );
        db.save_provider("codex", &neko).expect("save neko");

        let mut aggregate = Provider::with_id(
            "agg-test".to_string(),
            "My Aggregate".to_string(),
            serde_json::json!({
                "auth": {},
                "config": "",
                "aggregateModels": [{
                    "model": "gpt-5.6-sol",
                    "providerId": "neko",
                    "upstreamModel": "gpt-5.6-sol"
                }, {
                    "model": "kimi-k3",
                    "providerId": "neko",
                    "upstreamModel": "kimi-k3"
                }]
            }),
            None,
        );
        aggregate.meta = Some(crate::provider::ProviderMeta {
            provider_type: Some("aggregate".to_string()),
            ..Default::default()
        });
        db.save_provider("codex", &aggregate)
            .expect("save aggregate");
        db.set_current_provider("codex", "agg-test")
            .expect("set current provider");

        let state = build_state(db.clone());
        let mut body = serde_json::json!({ "model": "gpt-5.6-sol", "input": "hi" });
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "thread-id",
            "01a017fe-7519-7ce2-9c7e-872de1c2394c".parse().unwrap(),
        );
        headers.insert("x-openai-subagent", "worker".parse().unwrap());

        reconcile_codex_request_model_with_thread_using(&state, &mut body, &headers, |_| {
            Some("kimi-k3".to_string())
        })
        .await;

        assert_eq!(body["model"].as_str(), Some("gpt-5.6-sol"));
    }
}
