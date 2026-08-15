//! Provider service module
//!
//! Handles provider CRUD operations, switching, and configuration management.

mod endpoints;
mod live;
mod usage;

use indexmap::IndexMap;
use regex::Regex;
use serde::Deserialize;
use serde_json::Value;

use crate::app_config::AppType;
use crate::database::{validate_cost_multiplier, validate_pricing_source};
use crate::error::AppError;
use crate::provider::{Provider, UsageResult};
use crate::services::mcp::McpService;
use crate::settings::CustomEndpoint;
use crate::store::AppState;

// Re-export sub-module functions for external access
pub use live::{
    import_default_config, read_live_settings, should_import_default_config_on_startup,
    sync_current_to_live, update_toml_common_config_snippet,
};

// Internal re-exports (pub(crate))
pub(crate) use live::{
    build_effective_settings_with_common_config, normalize_provider_common_config_for_storage,
    provider_exists_in_live_config, should_backfill_provider_from_live,
    strip_common_config_from_live_settings, sync_current_provider_for_app_to_live,
    write_live_with_common_config,
};
use usage::validate_usage_script;

/// The built-in Codex official provider is safe to select during takeover:
/// Codex keeps ownership of its ChatGPT login and the proxy only forwards the
/// authenticated request. Other official providers retain the existing block.
pub fn official_provider_supports_proxy_takeover(app_type: &AppType, provider: &Provider) -> bool {
    matches!(app_type, AppType::Codex)
        && crate::proxy::providers::is_codex_official_provider(provider)
}

/// 统一会话开关变更后，立即按新开关状态重写当前官方 Codex 供应商的
/// live 配置，使开关即时生效（无需等下一次切换）。
/// 当前供应商非官方（或不存在）时为 no-op：注入只作用于官方配置，
/// 第三方 live 配置不受开关影响。
pub fn reapply_current_codex_official_live(state: &AppState) -> Result<bool, AppError> {
    let current_id = ProviderService::current(state, AppType::Codex)?;
    if current_id.is_empty() {
        return Ok(false);
    }
    let providers = state.db.get_all_providers(AppType::Codex.as_str())?;
    let Some(provider) = providers.get(&current_id) else {
        return Ok(false);
    };
    if provider.category.as_deref() != Some("official") {
        return Ok(false);
    }

    // 代理接管期间 live 归代理所有（开启代理时官方供应商只警告不拦截，
    // 二者可以共存）。与切换/保存路径一致：以 backup/占位符为所有权信号，
    // 只更新备份，注入后的配置由接管释放时的恢复路径落盘。
    let has_live_backup =
        futures::executor::block_on(state.db.get_live_backup(AppType::Codex.as_str()))
            .ok()
            .flatten()
            .is_some();
    let live_taken_over = state
        .proxy_service
        .detect_takeover_in_live_config_for_app(&AppType::Codex);
    if has_live_backup || live_taken_over {
        futures::executor::block_on(
            state
                .proxy_service
                .update_live_backup_from_provider(AppType::Codex.as_str(), provider),
        )
        .map_err(|e| AppError::Message(format!("更新 Live 备份失败: {e}")))?;
        return Ok(true);
    }

    live::write_live_with_common_config(&state.db, &AppType::Codex, provider)?;
    // 重写 live 会整体替换 config.toml（有意设计），[mcp_servers] 随之丢失，
    // 写完必须立刻从 DB 重新投影启用的 MCP。只投影 Codex 而非
    // sync_all_enabled：后者按 AppType::all() 顺序逐应用短路，排在 Codex
    // 前面的无关应用 live 损坏（如 ~/.claude.json 坏 JSON）会阻断 Codex
    // 的重投影，让刚被清掉的 [mcp_servers] 无人补回。
    // 投影失败降级为警告：走到这里 live 已按新开关状态落盘，开关事实上
    // 已生效；若把错误上抛，save_settings 会回滚开关设置，制造"设置=旧值、
    // live=新桶"的会话分裂——正是该回滚要防止的状态。MCP 投影可自愈
    // （下次切换 / 任一 MCP 启停操作都会重新投影）。
    if let Err(err) = McpService::sync_enabled_for_app(state, &AppType::Codex) {
        log::warn!("统一会话开关重写 live 后重投影 Codex MCP 失败（将在下次同步时自愈）: {err}");
    }
    Ok(true)
}

/// Provider business logic service
pub struct ProviderService;

/// Result of a provider switch operation, including any non-fatal warnings
#[derive(Debug, serde::Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SwitchResult {
    pub warnings: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(any(target_os = "macos", windows))]
    use crate::database::Database;
    #[cfg(any(target_os = "macos", windows))]
    use crate::provider::{ProviderMeta, UsageScript};
    use crate::proxy::types::ProxyConfig;
    use crate::store::AppState;
    use serde_json::json;
    use serial_test::serial;
    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex, OnceLock};
    use tempfile::TempDir;
    struct TempHome {
        dir: TempDir,
        original_home: Option<String>,
        original_local_app_data: Option<String>,
        original_userprofile: Option<String>,
        original_test_home: Option<String>,
    }
    impl TempHome {
        fn new() -> Self {
            let dir = TempDir::new().expect("failed to create temp home");
            let original_home = env::var("HOME").ok();
            let original_local_app_data = env::var("LOCALAPPDATA").ok();
            let original_userprofile = env::var("USERPROFILE").ok();
            let original_test_home = env::var("CODEX_CUBE_TEST_HOME").ok();

            env::set_var("HOME", dir.path());
            env::set_var("LOCALAPPDATA", dir.path().join("AppData").join("Local"));
            env::set_var("USERPROFILE", dir.path());
            env::set_var("CODEX_CUBE_TEST_HOME", dir.path());

            Self {
                dir,
                original_home,
                original_local_app_data,
                original_userprofile,
                original_test_home,
            }
        }
    }
    impl Drop for TempHome {
        fn drop(&mut self) {
            match &self.original_home {
                Some(value) => env::set_var("HOME", value),
                None => env::remove_var("HOME"),
            }

            {
                match &self.original_local_app_data {
                    Some(value) => env::set_var("LOCALAPPDATA", value),
                    None => env::remove_var("LOCALAPPDATA"),
                }
            }

            match &self.original_userprofile {
                Some(value) => env::set_var("USERPROFILE", value),
                None => env::remove_var("USERPROFILE"),
            }

            match &self.original_test_home {
                Some(value) => env::set_var("CODEX_CUBE_TEST_HOME", value),
                None => env::remove_var("CODEX_CUBE_TEST_HOME"),
            }
        }
    }
    fn test_guard() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|err| err.into_inner())
    }

    fn with_test_home<T>(test: impl FnOnce(&AppState, &Path) -> T) -> T {
        let _guard = test_guard();
        let temp = tempfile::tempdir().expect("tempdir");
        let old_test_home = std::env::var_os("CODEX_CUBE_TEST_HOME");
        let old_home = std::env::var_os("HOME");
        std::env::set_var("CODEX_CUBE_TEST_HOME", temp.path());
        std::env::set_var("HOME", temp.path());

        let db = Arc::new(Database::memory().expect("in-memory database"));
        let state = AppState::new(db);
        let result = test(&state, temp.path());

        match old_test_home {
            Some(value) => std::env::set_var("CODEX_CUBE_TEST_HOME", value),
            None => std::env::remove_var("CODEX_CUBE_TEST_HOME"),
        }
        match old_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }

        result
    }
    fn codex_settings(base_url: &str, api_key: &str) -> Value {
        json!({
            "auth": {
                "OPENAI_API_KEY": api_key
            },
            "config": format!(
                "model_provider = \"custom\"\n\
                 [model_providers.custom]\n\
                 name = \"custom\"\n\
                 base_url = \"{base_url}\"\n\
                 wire_api = \"chat\"\n"
            )
        })
    }
    fn live_codex_api_key() -> Option<String> {
        let live = read_live_settings(AppType::Codex).expect("read live Codex settings");
        crate::codex_config::extract_codex_api_key(
            live.get("auth"),
            live.get("config").and_then(Value::as_str),
        )
    }
    #[test]
    #[serial]
    fn update_current_codex_provider_writes_new_key_to_live() {
        with_test_home(|state, _| {
            let provider = Provider::with_id(
                "codex-update-current-live".to_string(),
                "Current provider".to_string(),
                codex_settings("https://api.old.example/v1", "sk-old-current"),
                None,
            );
            state
                .db
                .save_provider(AppType::Codex.as_str(), &provider)
                .expect("seed current provider");
            state
                .db
                .set_current_provider(AppType::Codex.as_str(), &provider.id)
                .expect("mark provider current");
            write_live_with_common_config(state.db.as_ref(), &AppType::Codex, &provider)
                .expect("seed live config");

            let mut updated = provider.clone();
            updated.settings_config =
                codex_settings("https://api.new.example/v1", "sk-new-current");
            ProviderService::update(state, AppType::Codex, None, updated)
                .expect("update current provider");

            let saved = state
                .db
                .get_provider_by_id(&provider.id, AppType::Codex.as_str())
                .expect("query updated provider")
                .expect("updated provider should exist");
            assert_eq!(
                saved
                    .settings_config
                    .pointer("/auth/OPENAI_API_KEY")
                    .and_then(Value::as_str),
                Some("sk-new-current")
            );
            assert_eq!(live_codex_api_key().as_deref(), Some("sk-new-current"));
            assert!(!crate::codex_config::read_codex_config_text()
                .expect("read live config.toml")
                .contains("sk-old-current"));
        });
    }
    #[test]
    #[serial]
    fn update_non_current_codex_provider_does_not_overwrite_live() {
        with_test_home(|state, _| {
            let current = Provider::with_id(
                "codex-live-owner".to_string(),
                "Live owner".to_string(),
                codex_settings("https://api.owner.example/v1", "sk-live-owner"),
                None,
            );
            let background = Provider::with_id(
                "codex-background-edit".to_string(),
                "Background provider".to_string(),
                codex_settings("https://api.background.example/v1", "sk-background-old"),
                None,
            );
            state
                .db
                .save_provider(AppType::Codex.as_str(), &current)
                .expect("seed current provider");
            state
                .db
                .save_provider(AppType::Codex.as_str(), &background)
                .expect("seed background provider");
            state
                .db
                .set_current_provider(AppType::Codex.as_str(), &current.id)
                .expect("mark live owner current");
            write_live_with_common_config(state.db.as_ref(), &AppType::Codex, &current)
                .expect("seed live config");

            let mut updated = background.clone();
            updated.settings_config =
                codex_settings("https://api.background-new.example/v1", "sk-background-new");
            ProviderService::update(state, AppType::Codex, None, updated)
                .expect("update non-current provider");

            let saved = state
                .db
                .get_provider_by_id(&background.id, AppType::Codex.as_str())
                .expect("query updated background provider")
                .expect("updated background provider should exist");
            assert_eq!(
                saved
                    .settings_config
                    .pointer("/auth/OPENAI_API_KEY")
                    .and_then(Value::as_str),
                Some("sk-background-new")
            );
            assert_eq!(live_codex_api_key().as_deref(), Some("sk-live-owner"));
            assert!(!crate::codex_config::read_codex_config_text()
                .expect("read live config.toml")
                .contains("sk-background-new"));
        });
    }
    fn usage_script_with_credentials(
        api_key: Option<&str>,
        base_url: Option<&str>,
        template_type: Option<&str>,
    ) -> UsageScript {
        UsageScript {
            enabled: true,
            language: "javascript".to_string(),
            code: "return { remaining: 1, unit: 'USD' };".to_string(),
            timeout: Some(10),
            api_key: api_key.map(str::to_string),
            base_url: base_url.map(str::to_string),
            access_token: None,
            user_id: None,
            template_type: template_type.map(str::to_string),
            auto_query_interval: None,
            coding_plan_provider: None,
            access_key_id: Some("ak-test".to_string()),
            secret_access_key: Some("sk-test".to_string()),
            team_organization_id: None,
            team_project_id: None,
        }
    }
    fn codex_provider_with_usage(
        id: &str,
        base_url: &str,
        api_key: &str,
        usage_api_key: Option<&str>,
        usage_base_url: Option<&str>,
        template_type: Option<&str>,
    ) -> Provider {
        let mut provider = Provider::with_id(
            id.to_string(),
            format!("Provider {id}"),
            codex_settings(base_url, api_key),
            None,
        );
        provider.meta = Some(ProviderMeta {
            usage_script: Some(usage_script_with_credentials(
                usage_api_key,
                usage_base_url,
                template_type,
            )),
            ..Default::default()
        });
        provider
    }
    fn add_clears_usage_credentials_that_match_provider_config() {
        with_test_home(|state, _| {
            let provider = codex_provider_with_usage(
                "codex-a",
                "https://api.a.example/v1/",
                "sk-a",
                Some(" sk-a "),
                Some(" https://api.a.example/v1/ "),
                None,
            );

            ProviderService::add(state, AppType::Codex, provider, false).expect("add provider");

            let saved = state
                .db
                .get_provider_by_id("codex-a", AppType::Codex.as_str())
                .expect("query saved provider")
                .expect("saved provider should exist");
            let script = saved
                .meta
                .as_ref()
                .and_then(|meta| meta.usage_script.as_ref())
                .expect("usage script should remain");

            assert_eq!(script.api_key, None);
            assert_eq!(script.base_url, None);
        });
    }
    #[test]
    #[serial]
    fn update_preserves_usage_credentials_that_only_match_previous_config() {
        with_test_home(|state, _| {
            let provider = codex_provider_with_usage(
                "codex-usage-old",
                "https://api.a.example/v1/",
                "sk-a",
                Some("sk-a"),
                Some("https://api.a.example/v1/"),
                None,
            );
            state
                .db
                .save_provider(AppType::Codex.as_str(), &provider)
                .expect("seed provider with explicit usage credentials");

            let mut updated = provider.clone();
            updated.settings_config = codex_settings("https://api.b.example/v1/", "sk-b");

            ProviderService::update(state, AppType::Codex, None, updated)
                .expect("update provider main credentials");

            let saved = state
                .db
                .get_provider_by_id("codex-usage-old", AppType::Codex.as_str())
                .expect("query updated provider")
                .expect("updated provider should exist");
            let script = saved
                .meta
                .as_ref()
                .and_then(|meta| meta.usage_script.as_ref())
                .expect("usage script should remain");

            assert_eq!(script.api_key.as_deref(), Some("sk-a"));
            assert_eq!(
                script.base_url.as_deref(),
                Some("https://api.a.example/v1/")
            );
            assert_eq!(
                saved.resolve_usage_credentials(&AppType::Codex),
                ("https://api.b.example/v1".to_string(), "sk-b".to_string())
            );
        });
    }
    #[test]
    #[serial]
    fn copied_provider_uses_edited_credentials_after_add_clears_mirrored_usage_credentials() {
        with_test_home(|state, _| {
            let copied_provider = codex_provider_with_usage(
                "codex-copy",
                "https://api.a.example/v1/",
                "sk-a",
                Some("sk-a"),
                Some("https://api.a.example/v1/"),
                None,
            );

            ProviderService::add(state, AppType::Codex, copied_provider, false)
                .expect("add copied provider");

            let saved_after_add = state
                .db
                .get_provider_by_id("codex-copy", AppType::Codex.as_str())
                .expect("query copied provider")
                .expect("copied provider should exist");
            let script_after_add = saved_after_add
                .meta
                .as_ref()
                .and_then(|meta| meta.usage_script.as_ref())
                .expect("usage script should remain");
            assert_eq!(script_after_add.api_key, None);
            assert_eq!(script_after_add.base_url, None);

            let mut edited_provider = saved_after_add.clone();
            edited_provider.settings_config = codex_settings("https://api.b.example/v1/", "sk-b");

            ProviderService::update(state, AppType::Codex, None, edited_provider)
                .expect("edit copied provider credentials");

            let saved_after_update = state
                .db
                .get_provider_by_id("codex-copy", AppType::Codex.as_str())
                .expect("query edited provider")
                .expect("edited provider should exist");
            let script_after_update = saved_after_update
                .meta
                .as_ref()
                .and_then(|meta| meta.usage_script.as_ref())
                .expect("usage script should remain");

            assert_eq!(script_after_update.api_key, None);
            assert_eq!(script_after_update.base_url, None);
            assert_eq!(
                saved_after_update.resolve_usage_credentials(&AppType::Codex),
                ("https://api.b.example/v1".to_string(), "sk-b".to_string())
            );
        });
    }
    #[test]
    #[serial]
    fn update_clears_usage_credentials_that_match_current_config() {
        with_test_home(|state, _| {
            let provider = codex_provider_with_usage(
                "codex-current",
                "https://api.a.example/v1",
                "sk-a",
                Some("sk-usage"),
                Some("https://usage.example/api"),
                None,
            );
            state
                .db
                .save_provider(AppType::Codex.as_str(), &provider)
                .expect("seed provider with distinct usage credentials");

            let mut updated = provider.clone();
            updated.settings_config = codex_settings("https://api.b.example/v1/", "sk-b");
            updated.meta = Some(ProviderMeta {
                usage_script: Some(usage_script_with_credentials(
                    Some(" sk-b "),
                    Some(" https://api.b.example/v1/ "),
                    None,
                )),
                ..Default::default()
            });

            ProviderService::update(state, AppType::Codex, None, updated)
                .expect("update provider with redundant usage credentials");

            let saved = state
                .db
                .get_provider_by_id("codex-current", AppType::Codex.as_str())
                .expect("query updated provider")
                .expect("updated provider should exist");
            let script = saved
                .meta
                .as_ref()
                .and_then(|meta| meta.usage_script.as_ref())
                .expect("usage script should remain");

            assert_eq!(script.api_key, None);
            assert_eq!(script.base_url, None);
        });
    }
    #[test]
    #[serial]
    fn add_preserves_distinct_usage_credentials() {
        with_test_home(|state, _| {
            let provider = codex_provider_with_usage(
                "codex-distinct",
                "https://api.main.example/v1",
                "sk-main",
                Some("sk-usage"),
                Some("https://usage.example/api"),
                None,
            );

            ProviderService::add(state, AppType::Codex, provider, false).expect("add provider");

            let saved = state
                .db
                .get_provider_by_id("codex-distinct", AppType::Codex.as_str())
                .expect("query saved provider")
                .expect("saved provider should exist");
            let script = saved
                .meta
                .as_ref()
                .and_then(|meta| meta.usage_script.as_ref())
                .expect("usage script should remain");

            assert_eq!(script.api_key.as_deref(), Some("sk-usage"));
            assert_eq!(
                script.base_url.as_deref(),
                Some("https://usage.example/api")
            );
        });
    }
    #[test]
    #[serial]
    fn add_does_not_clear_token_plan_credentials() {
        with_test_home(|state, _| {
            let provider = codex_provider_with_usage(
                "codex-token-plan",
                "https://api.plan.example/v1",
                "sk-plan",
                Some("sk-plan"),
                Some("https://api.plan.example/v1"),
                Some("token_plan"),
            );

            ProviderService::add(state, AppType::Codex, provider, false).expect("add provider");

            let saved = state
                .db
                .get_provider_by_id("codex-token-plan", AppType::Codex.as_str())
                .expect("query saved provider")
                .expect("saved provider should exist");
            let script = saved
                .meta
                .as_ref()
                .and_then(|meta| meta.usage_script.as_ref())
                .expect("usage script should remain");

            assert_eq!(script.api_key.as_deref(), Some("sk-plan"));
            assert_eq!(
                script.base_url.as_deref(),
                Some("https://api.plan.example/v1")
            );
            assert_eq!(script.access_key_id.as_deref(), Some("ak-test"));
            assert_eq!(script.secret_access_key.as_deref(), Some("sk-test"));
        });
    }
    #[test]
    fn validate_provider_settings_rejects_missing_auth() {
        let provider = Provider::with_id(
            "codex".into(),
            "Codex".into(),
            json!({ "config": "base_url = \"https://example.com\"" }),
            None,
        );
        let err = ProviderService::validate_provider_settings(&AppType::Codex, &provider)
            .expect_err("missing auth should be rejected");
        assert!(
            err.to_string().contains("auth"),
            "expected auth error, got {err:?}"
        );
    }
    /// 造一个「已被污染」的现场：片段里带 A 账号的凭据 + 一个合法可共享键。
    #[test]
    fn sensitive_key_matcher_covers_common_credential_namings() {
        for key in [
            // 裸 `_KEY`：最常见的写法，却曾被"只枚举 `_API_KEY` 这些子类"漏在外面
            "OPENAI_KEY",
            "GROQ_KEY",
            "XAI_KEY",
            // 不带分隔符的复合写法
            "VOLC_ACCESSKEY",
            "ALIYUN_SECRETKEY",
            "SOME_APITOKEN",
            // personal access token：既不含 TOKEN 也不含 KEY
            "GITHUB_PAT",
            "gitlab_pat",
            // 口令类缩写
            "MYSQL_PWD",
            "DB_PASS",
            "GPG_PASSPHRASE",
            "AWS_CREDS",
        ] {
            assert!(
                ProviderService::is_sensitive_config_key(key),
                "{key} must be treated as a credential"
            );
        }

        // 后缀必须带下划线，不能把正常配置一起卷进来
        for key in [
            "PATH",
            "OLDPWD",
            "GEMINI_COMPAT",
            "SSL_BYPASS",
            "GEMINI_TIMEOUT_MS",
            "CLAUDE_CODE_MAX_OUTPUT_TOKENS",
        ] {
            assert!(
                !ProviderService::is_sensitive_config_key(key),
                "{key} is ordinary shareable config and must not be stripped"
            );
        }
    }
    /// Regression for issue #4272: Fable tier env keys must not enter the shared
    /// Claude common-config snippet (same class as haiku/sonnet/opus model pins).
    #[test]
    fn extract_codex_common_config_strips_provider_fields_and_injected_artifacts() {
        // 顶层 experimental_bearer_token 模拟无活跃路由时的 fallback 注入；
        // web_search = "disabled" 是 Codex Cube 对黑名单网关注入的哨兵；
        // 顶层 wire_api 模拟无 model_provider 时的 fallback 写法；
        // [mcp.servers] 是历史错误格式，sync_all_enabled 清不掉它。
        let config_toml = r#"model_provider = "azure"
model = "gpt-4"
wire_api = "chat"
disable_response_storage = true
experimental_bearer_token = "sk-live-secret"
model_catalog_json = "codex-cube-model-catalog.json"
web_search = "disabled"

[model_providers.azure]
name = "Azure OpenAI"
base_url = "https://azure.example/v1"
wire_api = "responses"

[mcp_servers.my_server]
base_url = "http://localhost:8080"

[mcp.servers.legacy_server]
command = "legacy-cmd"
"#;

        let settings = json!({ "config": config_toml });
        let extracted = ProviderService::extract_codex_common_config(&settings)
            .expect("extract_codex_common_config should succeed");

        assert!(
            !extracted
                .lines()
                .any(|line| line.trim_start().starts_with("model_provider")),
            "should remove top-level model_provider"
        );
        assert!(
            !extracted
                .lines()
                .any(|line| line.trim_start().starts_with("model =")),
            "should remove top-level model"
        );
        assert!(
            !extracted.contains("[model_providers"),
            "should remove entire model_providers table"
        );
        // MCP 归 DB mcp_servers 表所有，不得进共享片段（含历史错误格式 [mcp.servers]）
        assert!(
            !extracted.contains("mcp_servers") && !extracted.contains("http://localhost:8080"),
            "should strip mcp_servers from the shared snippet, got: {extracted}"
        );
        assert!(
            !extracted.contains("[mcp") && !extracted.contains("legacy-cmd"),
            "should strip the legacy [mcp.servers] form from the shared snippet, got: {extracted}"
        );
        // 顶层 wire_api 是供应商路由语义（model_providers 整表已剥，
        // 剩余任何 wire_api 都意味着泄漏）
        assert!(
            !extracted.contains("wire_api"),
            "should strip top-level wire_api from the shared snippet, got: {extracted}"
        );
        // 注入产物不得进共享片段（bearer token 泄漏为密钥级问题）
        assert!(
            !extracted.contains("experimental_bearer_token")
                && !extracted.contains("sk-live-secret"),
            "should strip top-level fallback bearer token, got: {extracted}"
        );
        assert!(
            !extracted.contains("model_catalog_json"),
            "should strip catalog projection pointer, got: {extracted}"
        );
        assert!(
            !extracted.contains("web_search"),
            "should strip the codex-cube web_search disabled sentinel, got: {extracted}"
        );
        // 真正可共享的键保留
        assert!(
            extracted.contains("disable_response_storage = true"),
            "shareable keys must survive extraction, got: {extracted}"
        );
    }
    #[test]
    fn extract_codex_common_config_keeps_user_set_web_search() {
        let config_toml = "web_search = \"enabled\"\ndisable_response_storage = true\n";
        let settings = json!({ "config": config_toml });
        let extracted = ProviderService::extract_codex_common_config(&settings)
            .expect("extract should succeed");
        assert!(
            extracted.contains("web_search = \"enabled\""),
            "a user-set web_search value is a shareable preference, got: {extracted}"
        );
    }

    #[cfg(any(target_os = "macos", windows))]
    #[test]
    #[serial]

    fn normal_switch_to_ordinary_provider_syncs_stale_custom_thread_models() {
        // 普通（非接管）切换到普通第三方单模型 Provider 后，Codex state DB 里
        // 指向旧 Provider 的 custom 会话模型应同步为新 Provider 的上游模型；
        // openai 等其他桶保持不变。复用 state helper + 真实 switch_normal 流程
        // （live 写入成功后才触发同步），与热切换集成测试互补。
        with_test_home(|state, _| {
            let codex_dir = crate::codex_config::get_codex_config_dir();
            let state_db = codex_dir.join(crate::codex_state_db::CODEX_STATE_DB_FILENAME);
            std::fs::create_dir_all(&codex_dir).expect("create codex dir");
            {
                let conn = rusqlite::Connection::open(&state_db).expect("open state db");
                conn.execute_batch(
                    "CREATE TABLE threads (
                        id TEXT PRIMARY KEY,
                        model_provider TEXT NOT NULL,
                        model TEXT
                    );",
                )
                .expect("create threads table");
                conn.execute(
                    "INSERT INTO threads (id, model_provider, model)                      VALUES ('t1', 'custom', 'gpt-5.6-luna')",
                    [],
                )
                .expect("insert stale custom thread");
                conn.execute(
                    "INSERT INTO threads (id, model_provider, model)                      VALUES ('t2', 'openai', 'gpt-5.6-luna')",
                    [],
                )
                .expect("insert official bucket thread");
            }

            let provider = Provider::with_id(
                "deepseek".to_string(),
                "DeepSeek".to_string(),
                json!({
                    "auth": { "OPENAI_API_KEY": "deepseek-key" },
                    "model": "deepseek-v4-flash",
                    "modelCatalog": {
                        "models": [{ "model": "deepseek-v4-flash" }]
                    },
                    "config": r#"model_provider = "custom"
model = "deepseek-v4-flash"

[model_providers.custom]
name = "DeepSeek"
base_url = "https://api.deepseek.example/v1"
wire_api = "responses"
"#
                }),
                None,
            );
            state
                .db
                .save_provider(AppType::Codex.as_str(), &provider)
                .expect("save target provider");

            ProviderService::switch(state, AppType::Codex, &provider.id)
                .expect("normal switch should succeed");

            let conn = rusqlite::Connection::open(&state_db).expect("reopen state db");
            let mut stmt = conn
                .prepare("SELECT model_provider, model FROM threads ORDER BY id")
                .expect("prepare query");
            let rows: Vec<(String, Option<String>)> = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .expect("query rows")
                .map(|row| row.expect("read row"))
                .collect();
            assert_eq!(
                rows[0],
                ("custom".to_string(), Some("deepseek-v4-flash".to_string())),
                "stale custom thread must follow the switched provider's upstream model"
            );
            assert_eq!(
                rows[1],
                ("openai".to_string(), Some("gpt-5.6-luna".to_string())),
                "official thread bucket must stay untouched"
            );
        });
    }
}
impl ProviderService {
    /// Check whether a provider exists in live config, tolerating parse errors
    /// only for providers that are explicitly marked as DB-only.
    fn check_live_config_exists(
        app_type: &AppType,
        provider_id: &str,
        live_config_managed: Option<bool>,
    ) -> Result<bool, AppError> {
        if live_config_managed == Some(false) {
            Ok(provider_exists_in_live_config(app_type, provider_id).unwrap_or(false))
        } else {
            provider_exists_in_live_config(app_type, provider_id)
        }
    }

    fn provider_live_config_managed(provider: &Provider) -> Option<bool> {
        provider
            .meta
            .as_ref()
            .and_then(|meta| meta.live_config_managed)
    }

    fn set_provider_live_config_managed(provider: &mut Provider, managed: bool) {
        provider
            .meta
            .get_or_insert_with(Default::default)
            .live_config_managed = Some(managed);
    }

    fn normalize_usage_script_credential_overrides(app_type: &AppType, provider: &mut Provider) {
        let current_credentials = provider.resolve_usage_credentials(app_type);

        let Some(usage_script) = provider
            .meta
            .as_mut()
            .and_then(|meta| meta.usage_script.as_mut())
        else {
            return;
        };

        if usage_script.template_type.as_deref() == Some("token_plan") {
            return;
        }

        if usage_script.api_key.as_deref().is_some_and(|api_key| {
            Self::should_clear_usage_api_key_override(api_key, &current_credentials)
        }) {
            usage_script.api_key = None;
        }

        if usage_script.base_url.as_deref().is_some_and(|base_url| {
            Self::should_clear_usage_base_url_override(base_url, &current_credentials)
        }) {
            usage_script.base_url = None;
        }
    }

    fn should_clear_usage_api_key_override(
        script_api_key: &str,
        current_credentials: &(String, String),
    ) -> bool {
        let candidate = script_api_key.trim();
        if candidate.is_empty() {
            return true;
        }

        let matches_provider_key = |api_key: &str| {
            let api_key = api_key.trim();
            !api_key.is_empty() && api_key == candidate
        };

        matches_provider_key(&current_credentials.1)
    }

    fn should_clear_usage_base_url_override(
        script_base_url: &str,
        current_credentials: &(String, String),
    ) -> bool {
        let candidate = Self::normalize_usage_base_url_for_compare(script_base_url);
        if candidate.is_empty() {
            return true;
        }

        let matches_provider_base_url = |base_url: &str| {
            let base_url = Self::normalize_usage_base_url_for_compare(base_url);
            !base_url.is_empty() && base_url == candidate
        };

        matches_provider_base_url(&current_credentials.0)
    }

    fn normalize_usage_base_url_for_compare(base_url: &str) -> String {
        base_url.trim().trim_end_matches('/').to_string()
    }

    /// List all providers for an app type
    pub fn list(
        state: &AppState,
        app_type: AppType,
    ) -> Result<IndexMap<String, Provider>, AppError> {
        state.db.get_all_providers(app_type.as_str())
    }

    /// Get current provider ID
    ///
    /// 使用有效的当前供应商 ID（验证过存在性）。
    /// 优先从本地 settings 读取，验证后 fallback 到数据库的 is_current 字段。
    /// 这确保了云同步场景下多设备可以独立选择供应商，且返回的 ID 一定有效。
    ///
    /// 当前供应商概念由 Codex 维护。
    pub fn current(state: &AppState, app_type: AppType) -> Result<String, AppError> {
        // Additive mode apps have no "current" provider concept
        if app_type.is_additive_mode() {
            return Ok(String::new());
        }
        crate::settings::get_effective_current_provider(&state.db, &app_type)
            .map(|opt| opt.unwrap_or_default())
    }

    /// Add a new provider
    pub fn add(
        state: &AppState,
        app_type: AppType,
        provider: Provider,
        add_to_live: bool,
    ) -> Result<bool, AppError> {
        let mut provider = provider;
        Self::validate_provider_settings(&app_type, &provider)?;
        normalize_provider_common_config_for_storage(state.db.as_ref(), &app_type, &mut provider)?;
        Self::normalize_usage_script_credential_overrides(&app_type, &mut provider);
        if app_type.is_additive_mode() {
            Self::set_provider_live_config_managed(&mut provider, add_to_live);
        }

        // Save to database
        state.db.save_provider(app_type.as_str(), &provider)?;

        // Check if sync is needed (if this is current provider, or no current provider)
        let current = state.db.get_current_provider(app_type.as_str())?;
        if current.is_none() {
            // No current provider, set as current and sync
            state
                .db
                .set_current_provider(app_type.as_str(), &provider.id)?;
            write_live_with_common_config(state.db.as_ref(), &app_type, &provider)?;
        }

        Ok(true)
    }

    /// Update a provider
    pub fn update(
        state: &AppState,
        app_type: AppType,
        original_id: Option<&str>,
        provider: Provider,
    ) -> Result<bool, AppError> {
        let mut provider = provider;
        let original_id = original_id.unwrap_or(provider.id.as_str()).to_string();
        let provider_id_changed = original_id != provider.id;
        let _existing_provider = state
            .db
            .get_provider_by_id(&original_id, app_type.as_str())?;
        Self::validate_provider_settings(&app_type, &provider)?;
        normalize_provider_common_config_for_storage(state.db.as_ref(), &app_type, &mut provider)?;
        Self::normalize_usage_script_credential_overrides(&app_type, &mut provider);

        if provider_id_changed {
            // Codex 不支持修改 provider key（id 由前端生成，改名会导致引用丢失）。
            return Err(AppError::Message(
                "Only additive-mode providers support changing provider key".to_string(),
            ));
        }

        // Save to database
        state.db.save_provider(app_type.as_str(), &provider)?;

        // For other apps: Check if this is current provider (use effective current, not just DB)
        let effective_current =
            crate::settings::get_effective_current_provider(&state.db, &app_type)?;
        let is_current = effective_current.as_deref() == Some(provider.id.as_str());

        if is_current {
            // 如果 Codex 代理接管处于激活状态，并且代理服务正在运行：
            // - 不直接走普通 Live 写入逻辑
            // - 改为更新 Live 备份，并同步代理安全的 Live 配置
            let has_live_backup =
                futures::executor::block_on(state.db.get_live_backup(app_type.as_str()))
                    .ok()
                    .flatten()
                    .is_some();
            let live_taken_over = state
                .proxy_service
                .detect_takeover_in_live_config_for_app(&app_type);
            // Backup or live placeholders mean the live file is currently owned
            // by proxy takeover, including the short activation window before
            // proxy_config.enabled is committed.
            let should_sync_via_proxy = has_live_backup || live_taken_over;

            if should_sync_via_proxy {
                futures::executor::block_on(
                    state
                        .proxy_service
                        .update_live_backup_from_provider(app_type.as_str(), &provider),
                )
                .map_err(|e| AppError::Message(format!("更新 Live 备份失败: {e}")))?;

                if futures::executor::block_on(state.proxy_service.is_running()) {
                    if live_taken_over && matches!(app_type, AppType::Codex) {
                        // Codex model mappings are projected into a generated
                        // model_catalog_json file. Refresh takeover-owned Live
                        // immediately so adding/removing mappings cannot leave
                        // the previous catalog pointer and capabilities active.
                        futures::executor::block_on(
                            state
                                .proxy_service
                                .sync_codex_live_from_provider_while_proxy_active(&provider),
                        )
                        .map_err(|e| AppError::Message(format!("同步 Codex Live 配置失败: {e}")))?;
                    }
                }
            } else {
                write_live_with_common_config(state.db.as_ref(), &app_type, &provider)?;
                // 重写 live 后只重投影本应用的 MCP：全量 sync_all_enabled 会把
                // 无关应用的 live 损坏（如 ~/.claude.json 坏 JSON）牵连进保存
                // 流程。走到这里 DB 与 live 都已按新配置落盘，保存事实上已
                // 成功；投影失败降级为警告，避免制造"保存失败"假象（MCP
                // 投影可自愈：下次切换 / 任一 MCP 启停都会重新投影）。
                if let Err(err) = McpService::sync_enabled_for_app(state, &app_type) {
                    log::warn!(
                        "保存供应商后重投影 {app_type:?} MCP 失败（将在下次同步时自愈）: {err}"
                    );
                }
            }
        }

        Ok(true)
    }

    /// Delete a provider
    ///
    /// 同时检查本地 settings 和数据库的当前供应商，防止删除任一端正在使用的供应商。
    /// 删除供应商并同步从 live 配置中移除。
    pub fn delete(state: &AppState, app_type: AppType, id: &str) -> Result<(), AppError> {
        // Codex providers leave config.toml / sidecar / cache effects behind;
        // deleting them restores or scrubs those effects instead of refusing.
        match app_type {
            AppType::Codex => Self::delete_codex_provider(state, id),
        }
    }

    /// Delete a Codex provider and restore/scrub the Codex live-config effects
    /// it left behind.
    ///
    /// A deleted current provider has its `[model_providers.*]` section and
    /// routing fields scrubbed from `config.toml` (or the pre-takeover live
    /// config restored when proxy takeover owns the live file), the
    /// codex-cube-owned `models_cache.json` cleared, and its
    /// `auth-<name>.json` / `config-<name>.toml` sidecars removed. A deleted
    /// non-current provider only loses its sidecar files, never the shared
    /// live config or models cache another provider may own.
    fn delete_codex_provider(state: &AppState, id: &str) -> Result<(), AppError> {
        let Some(provider) = state.db.get_provider_by_id(id, "codex")? else {
            return Ok(());
        };

        let current_provider_id =
            crate::settings::get_effective_current_provider(state.db.as_ref(), &AppType::Codex)?
                .or(state.db.get_current_provider("codex")?);
        let is_current = current_provider_id.as_deref() == Some(id);
        let proxy_takeover_active = futures::executor::block_on(state.db.get_live_backup("codex"))?
            .is_some()
            || state
                .proxy_service
                .detect_takeover_in_live_config_for_app(&AppType::Codex);

        // 持有应用切换锁，避免与热切换并发改写 Live 配置。
        let _switch_guard =
            futures::executor::block_on(state.proxy_service.lock_switch_for_app("codex"));

        // 记录删除前的 Live 配置；后续任一步失败时回滚，避免“配置已清但供应商仍在”的撕裂态。
        let config_path = crate::codex_config::get_codex_config_path();
        let pre_delete_config = if config_path.exists() {
            Some(crate::codex_config::read_codex_config_text()?)
        } else {
            None
        };

        let result: Result<(), AppError> = (|| {
            if is_current {
                let has_live_backup =
                    futures::executor::block_on(state.db.get_live_backup("codex"))?.is_some();
                let live_taken_over = state
                    .proxy_service
                    .detect_takeover_in_live_config_for_app(&AppType::Codex);

                if has_live_backup || live_taken_over {
                    // Live config is owned by proxy takeover: restore the original
                    // pre-takeover config (backup → SSOT → placeholder cleanup).
                    futures::executor::block_on(
                        state
                            .proxy_service
                            .restore_live_config_for_app_with_fallback_inner(&AppType::Codex),
                    )
                    .map_err(AppError::Message)?;
                } else {
                    let config_text = crate::codex_config::read_codex_config_text()?;
                    // base_url 可能直接存在 settings_config["base_url"]，也可能藏在
                    // settings_config["config"] 的 TOML 里（Codex provider 常见形态）；
                    // 两者都提取，才能正确清理旧式顶层 base_url。
                    let provider_base_url = provider
                        .settings_config
                        .get("base_url")
                        .and_then(|value| value.as_str())
                        .map(str::to_owned)
                        .or_else(|| {
                            provider
                                .settings_config
                                .get("config")
                                .and_then(|value| value.as_str())
                                .and_then(crate::codex_config::extract_codex_base_url)
                        });
                    // Live 里实际使用的 model_provider 才是需要清理的路由键：
                    // - 聚合 Provider 接管固定使用 `custom`（CODEX_MODEL_PROVIDER_ID），
                    //   不是聚合自身的 id；
                    // - 第三方 Provider 的 config TOML 通常声明自己的 model_provider
                    //   （可能是 `custom` 或模板内置 id），未必等于数据库 provider id。
                    let scrub_id = if provider.is_aggregate() {
                        crate::codex_config::CODEX_MODEL_PROVIDER_ID.to_string()
                    } else {
                        provider
                            .settings_config
                            .get("config")
                            .and_then(|value| value.as_str())
                            .and_then(|text| text.parse::<toml::Value>().ok())
                            .and_then(|doc| {
                                doc.get("model_provider")
                                    .and_then(|value| value.as_str())
                                    .map(str::to_owned)
                            })
                            .unwrap_or_else(|| id.to_string())
                    };
                    let cleaned = crate::codex_config::scrub_codex_provider_from_config(
                        &config_text,
                        &scrub_id,
                        provider_base_url.as_deref(),
                    )?;
                    crate::codex_config::write_codex_live_config_atomic(Some(&cleaned))?;
                }

                crate::settings::set_current_provider(&AppType::Codex, None)?;
                crate::codex_config::restore_or_clear_codex_models_cache(
                    &crate::codex_config::get_codex_config_dir(),
                )?;
                crate::codex_config::delete_codex_provider_config(id, &provider.name)?;
            } else {
                crate::codex_config::delete_codex_provider_config(id, &provider.name)?;
                if crate::settings::get_current_provider(&AppType::Codex).as_deref() == Some(id) {
                    crate::settings::set_current_provider(&AppType::Codex, None)?;
                }
            }

            Ok(())
        })();

        if let Err(e) = result {
            // 回滚 Live 配置到删除前状态（尽力而为）
            match &pre_delete_config {
                Some(text) => {
                    if let Err(rollback_err) =
                        crate::codex_config::write_codex_live_config_atomic(Some(text))
                    {
                        log::error!("删除 Codex 供应商失败后回滚 Live 配置失败: {rollback_err}");
                    }
                }
                None => {
                    let _ = crate::config::delete_file(&config_path);
                }
            }
            return Err(e);
        }

        let changed_aggregate_ids = state
            .db
            .delete_codex_provider_and_prune_aggregate_references(id)?;
        futures::executor::block_on(
            state
                .proxy_service
                .clear_active_target_if_matches("codex", id),
        );

        // 删除的是聚合成员且当前聚合仍处于接管态时，立即重投影其目录与
        // models_cache，避免已删除成员留下的 Desktop 槽位继续展示/请求。
        if proxy_takeover_active
            && futures::executor::block_on(state.proxy_service.is_running())
            && current_provider_id
                .as_ref()
                .is_some_and(|current_id| changed_aggregate_ids.contains(current_id))
        {
            if let Some(current_id) = current_provider_id.as_deref() {
                match state.db.get_provider_by_id(current_id, "codex") {
                    Ok(Some(aggregate)) if aggregate.is_aggregate() => {
                        if let Err(error) = futures::executor::block_on(
                            state
                                .proxy_service
                                .sync_codex_live_from_provider_while_proxy_active(&aggregate),
                        ) {
                            // 删除及数据库清理已提交；目录投影可在下次接管时自愈，
                            // 不能把成功删除伪装为失败并留下用户无法重试的状态。
                            log::warn!(
                                "删除 Codex 聚合成员后重投影当前聚合目录失败（下次接管会重试）: {error}"
                            );
                        }
                    }
                    Ok(_) => {}
                    Err(error) => log::warn!(
                        "删除 Codex 聚合成员后读取当前聚合 Provider 失败（下次接管会重试）: {error}"
                    ),
                }
            }
        }

        Ok(())
    }

    /// Remove provider from live config only
    ///
    /// Does NOT delete from database - provider remains in the list.
    /// This is used when user wants to "remove" a provider from active config
    /// but keep it available for future use.
    pub fn remove_from_live_config(
        _state: &AppState,
        app_type: AppType,
        _id: &str,
    ) -> Result<(), AppError> {
        match app_type {
            AppType::Codex => Err(AppError::Message(format!(
                "App {} does not support remove from live config",
                app_type.as_str()
            ))),
        }
    }

    /// Switch to a provider
    ///
    /// Switch flow:
    /// 1. Validate target provider exists
    /// 2. Check if proxy takeover mode is active AND proxy server is running
    /// 3. If takeover mode active: hot-switch proxy target and refresh proxy-safe Live labels
    /// 4. If normal mode:
    ///    a. **Backfill mechanism**: Backfill current live config to current provider
    ///    b. Update local settings current_provider_xxx (device-level)
    ///    c. Update database is_current (as default for new devices)
    ///    d. Write target provider config to live files
    ///    e. Sync MCP configuration
    pub fn switch(state: &AppState, app_type: AppType, id: &str) -> Result<SwitchResult, AppError> {
        // Check if provider exists
        let providers = state.db.get_all_providers(app_type.as_str())?;
        let _provider = providers
            .get(id)
            .ok_or_else(|| AppError::Message(format!("供应商 {id} 不存在")))?;

        // Provider switches and takeover toggles both mutate live config and the
        // restore backup. Serialize them per app, then decide from the locked
        // current state so a just-started takeover cannot be overwritten by a
        // normal live write.
        let _switch_guard = Some(futures::executor::block_on(
            state.proxy_service.lock_switch_for_app(app_type.as_str()),
        ));

        // Backup or live placeholders mean the live file is owned by proxy
        // takeover, even if the proxy server is temporarily stopped or is in the
        // activation window before enabled=true is committed.
        let is_app_taken_over =
            futures::executor::block_on(state.db.get_live_backup(app_type.as_str()))
                .ok()
                .flatten()
                .is_some();
        let live_taken_over = state
            .proxy_service
            .detect_takeover_in_live_config_for_app(&app_type);

        let should_hot_switch = is_app_taken_over || live_taken_over;

        // Block switching to official providers when proxy takeover is active.
        // Using a proxy with official APIs (Anthropic/OpenAI/Google) may cause account bans.
        if should_hot_switch
            && _provider.category.as_deref() == Some("official")
            && !official_provider_supports_proxy_takeover(&app_type, _provider)
        {
            return Err(AppError::localized(
                "switch.official_blocked_by_proxy",
                "代理接管模式下不能切换到官方供应商，使用代理访问官方 API 可能导致账号被封禁。请先关闭代理接管，或选择第三方供应商。",
                "Cannot switch to official provider while proxy takeover is active. Using proxy with official APIs may cause account bans.",
            ));
        }

        if should_hot_switch {
            // Proxy takeover mode: hot-switch without restoring upstream Live config.
            // The proxy layer may still refresh proxy-safe Live fields so client labels
            // follow the selected provider while endpoints remain local.
            log::info!(
                "代理接管模式：热切换 {} 的目标供应商为 {}",
                app_type.as_str(),
                id
            );

            futures::executor::block_on(
                state
                    .proxy_service
                    .hot_switch_provider_inner(app_type.as_str(), id),
            )
            .map_err(|e| AppError::Message(format!("热切换失败: {e}")))?;

            // The proxy server will route requests to the new provider via is_current.
            // MCP sync is intentionally skipped while Live config is owned by takeover.
            return Ok(SwitchResult::default());
        }

        // Normal mode: full switch with Live config write
        Self::switch_normal(state, app_type, id, &providers)
    }

    /// Normal switch flow (non-proxy mode)
    fn switch_normal(
        state: &AppState,
        app_type: AppType,
        id: &str,
        providers: &indexmap::IndexMap<String, Provider>,
    ) -> Result<SwitchResult, AppError> {
        let provider = providers
            .get(id)
            .ok_or_else(|| AppError::Message(format!("供应商 {id} 不存在")))?;

        let mut result = SwitchResult::default();

        // Backfill: Backfill current live config to current provider
        // Use effective current provider (validated existence) to ensure backfill targets valid provider
        let current_id = crate::settings::get_effective_current_provider(&state.db, &app_type)?;

        let mut backfill_completed = false;
        if let Some(current_id) = current_id {
            if current_id != id {
                // Additive mode apps - all providers coexist in the same file,
                // no backfill needed (backfill is for exclusive mode apps like Claude/Codex/Gemini)
                if !app_type.is_additive_mode() {
                    // Only backfill when switching to a different provider
                    if let Ok(live_config) = read_live_settings(app_type.clone()) {
                        if let Some(mut current_provider) = providers.get(&current_id).cloned() {
                            // 聚合 Provider 是虚拟供应商：settingsConfig 保存
                            // 成员/模型映射而非 live 配置，切走时绝不能拿 live
                            // 配置回填覆盖（否则 memberProviderIds /
                            // aggregateModels 会丢失）。
                            if should_backfill_provider_from_live(&current_provider) {
                                // 切走前先把 live 里的可共享改动（含用户直接在应用内
                                // 装插件/加 hook/改偏好）同步进通用配置片段，再做剥离回填。
                                // 详见 sync_common_config_snippet_from_live 的文档。
                                Self::sync_common_config_snippet_from_live(
                                    state,
                                    &app_type,
                                    &current_provider,
                                    &live_config,
                                    &mut result,
                                );

                                current_provider.settings_config =
                                    strip_common_config_from_live_settings(
                                        state.db.as_ref(),
                                        &app_type,
                                        &current_provider,
                                        live_config,
                                    );
                                if let Err(e) =
                                    state.db.save_provider(app_type.as_str(), &current_provider)
                                {
                                    log::warn!("Backfill failed: {e}");
                                    result
                                        .warnings
                                        .push(format!("backfill_failed:{current_id}"));
                                } else {
                                    backfill_completed = true;
                                }
                            } else {
                                log::info!(
                                    "跳过聚合 Provider '{}' 的 live 回填（虚拟供应商）",
                                    current_provider.id
                                );
                            }
                        }
                    }
                }
            }
        }

        // Additive mode apps skip setting is_current (no such concept)
        if !app_type.is_additive_mode() {
            // Update local settings (device-level, takes priority)
            crate::settings::set_current_provider(&app_type, Some(id))?;

            // Update database is_current (as default for new devices)
            state.db.set_current_provider(app_type.as_str(), id)?;
        }

        // Sync to live (write_gemini_live handles security flag internally for Gemini)
        write_live_with_common_config(state.db.as_ref(), &app_type, provider)?;

        // Live config write succeeded: rewrite stale `custom` thread models in
        // the Codex state DB to the new provider's upstream model (ordinary
        // third-party providers only). The post-switch config text is read so a
        // `sqlite_home` override is honored. Failure must not report the
        // already-successful switch as failed: log a warning and append a
        // SwitchResult warning following current backfill semantics.
        if matches!(app_type, AppType::Codex) {
            let outcome = crate::codex_state_db::sync_stale_custom_thread_models(
                provider,
                &crate::codex_config::get_codex_config_dir(),
                &crate::codex_config::read_codex_config_text().unwrap_or_default(),
            );
            if outcome.updated_rows > 0 {
                log::info!("切换后已同步 {} 条 Codex 线程模型", outcome.updated_rows);
            }
            if !outcome.errors.is_empty() {
                log::warn!(
                    "切换后同步 Codex 线程模型失败（不影响已成功的切换）: {}",
                    outcome.errors.join("; ")
                );
                result
                    .warnings
                    .push(format!("codex_thread_model_sync_failed:{}", provider.id));
            }
        }

        // A material-less official Codex provider gets a config-only live
        // write, which can leave the previous third-party key in
        // ~/.codex/auth.json and strand the user on a 401 with no login
        // screen. Only clean up after a successful backfill — the DB copy
        // made above is what keeps that key recoverable. Failures degrade to
        // a log entry: config.toml and is_current are already committed, so
        // failing the switch here would report a switch that in fact happened.
        if matches!(app_type, AppType::Codex)
            && backfill_completed
            && provider.category.as_deref() == Some("official")
        {
            let db_auth = provider.settings_config.get("auth");
            match crate::codex_config::clear_stale_codex_live_auth_after_official_switch(
                db_auth.unwrap_or(&serde_json::Value::Null),
            ) {
                Ok(true) => log::info!(
                    "Removed stale third-party auth.json after switching to official Codex provider '{}'",
                    provider.id
                ),
                Ok(false) => {}
                Err(e) => log::warn!("Failed to clean stale Codex auth.json: {e}"),
            }
        }

        // 切换重写了目标应用的 live，只重投影该应用的 MCP（Codex 的
        // [mcp_servers] 与 live 同文件，整体替换后必须补回；其余应用的
        // MCP 文件独立于 live，投影是幂等维护）。不用全量 sync_all_enabled：
        // 无关应用的 live 损坏（如 ~/.claude.json 坏 JSON）不该阻断切换。
        // 走到这里 DB is_current 与 live 都已落盘，切换事实上已成功；
        // 投影失败上抛会让前端报"切换失败"制造分裂假象，故降级为警告
        // （MCP 投影可自愈：下次切换 / 任一 MCP 启停都会重新投影）。
        if let Err(err) = McpService::sync_enabled_for_app(state, &app_type) {
            log::warn!("切换供应商后重投影 {app_type:?} MCP 失败（将在下次同步时自愈）: {err}");
        }

        Ok(result)
    }

    /// Sync current provider to live configuration (re-export)
    pub fn sync_current_to_live(state: &AppState) -> Result<(), AppError> {
        sync_current_to_live(state)
    }

    pub fn sync_current_provider_for_app(
        state: &AppState,
        app_type: AppType,
    ) -> Result<(), AppError> {
        if app_type.is_additive_mode() {
            return sync_current_provider_for_app_to_live(state, &app_type);
        }

        let current_id =
            match crate::settings::get_effective_current_provider(&state.db, &app_type)? {
                Some(id) => id,
                None => return Ok(()),
            };

        let providers = state.db.get_all_providers(app_type.as_str())?;
        let Some(provider) = providers.get(&current_id) else {
            return Ok(());
        };

        let has_live_backup =
            futures::executor::block_on(state.db.get_live_backup(app_type.as_str()))
                .ok()
                .flatten()
                .is_some();

        let live_taken_over = state
            .proxy_service
            .detect_takeover_in_live_config_for_app(&app_type);

        // See the save path above: backup/placeholders are the ownership signal
        // here, not just proxy_config.enabled.
        if has_live_backup || live_taken_over {
            futures::executor::block_on(
                state
                    .proxy_service
                    .update_live_backup_from_provider(app_type.as_str(), provider),
            )
            .map_err(|e| AppError::Message(format!("更新 Live 备份失败: {e}")))?;
            return Ok(());
        }

        sync_current_provider_for_app_to_live(state, &app_type)
    }

    pub fn migrate_legacy_common_config_usage(
        state: &AppState,
        app_type: AppType,
        legacy_snippet: &str,
    ) -> Result<(), AppError> {
        if app_type.is_additive_mode() || legacy_snippet.trim().is_empty() {
            return Ok(());
        }

        let providers = state.db.get_all_providers(app_type.as_str())?;

        for provider in providers.values() {
            if provider
                .meta
                .as_ref()
                .and_then(|meta| meta.common_config_enabled)
                .is_some()
            {
                continue;
            }

            if !live::provider_uses_common_config(&app_type, provider, Some(legacy_snippet)) {
                continue;
            }

            let mut updated_provider = provider.clone();
            updated_provider
                .meta
                .get_or_insert_with(Default::default)
                .common_config_enabled = Some(true);

            match live::remove_common_config_from_settings(
                &app_type,
                &updated_provider.settings_config,
                legacy_snippet,
            ) {
                Ok(settings) => updated_provider.settings_config = settings,
                Err(err) => {
                    log::warn!(
                        "Failed to normalize legacy common config for {} provider '{}': {err}",
                        app_type.as_str(),
                        updated_provider.id
                    );
                }
            }

            state
                .db
                .save_provider(app_type.as_str(), &updated_provider)?;
        }

        Ok(())
    }

    pub fn migrate_legacy_common_config_usage_if_needed(
        state: &AppState,
        app_type: AppType,
    ) -> Result<(), AppError> {
        if app_type.is_additive_mode() {
            return Ok(());
        }

        let Some(snippet) = state.db.get_config_snippet(app_type.as_str())? else {
            return Ok(());
        };

        if snippet.trim().is_empty() {
            return Ok(());
        }

        Self::migrate_legacy_common_config_usage(state, app_type, &snippet)
    }

    /// 切走某供应商前，把它 live 配置里的可共享部分重新提取并**整体替换**到
    /// 通用配置片段，使在 live 应用里直接做的改动不会因切换而丢失。
    ///
    /// 采用"整体重提取 + 替换"而非"只合并新增"，是为了同时覆盖三种情况：
    /// - **新增**：用户直接在应用里装了插件、加了 hook、改了 env/主题/权限等共享
    ///   偏好，被捕获进通用配置，切到别的供应商也带得过去；
    /// - **删除**：被删掉的键不在新提取结果里，于是从片段里消失、下次切换不会被
    ///   重新注入——否则会出现"插件怎么删也删不掉"的反直觉 bug；
    /// - **密钥安全**：提取器已剥掉 auth / model / endpoint，密钥永不进共享片段。
    ///
    /// 之所以"整体替换"是安全的：每次写 live 都会把当前片段合并进去，所以切走时
    /// 读到的 live 一定是"片段 + 本地改动"的超集，重提取只会丢掉用户真正删掉的键，
    /// 不会误删其它供应商共享的内容。
    ///
    /// **作用域**：Claude + Codex。Codex 提取器（`extract_codex_common_config`）
    /// 已剥离全部供应商专属与 Codex Cube 注入内容：`model` / `model_provider` /
    /// 顶层 `base_url` / 整张 `model_providers` 表（含端点与统一会话桶）、
    /// `mcp_servers`（SSOT 在 DB 表）、顶层 `experimental_bearer_token`
    /// fallback、`model_catalog_json`、`web_search = "disabled"` 哨兵——密钥与
    /// 注入产物不会进共享片段。Gemini 暂未纳入，如需支持应单独验证后再加。
    ///
    /// 仅对**显式勾选"写入通用配置"**（`meta.common_config_enabled == Some(true)`）的
    /// 供应商生效；用户**显式清空**过片段（`_cleared`）时跳过，避免把用户主动清掉的
    /// 配置又塞回来。所有失败均为非致命，只记 warning，绝不阻断切换。
    fn sync_common_config_snippet_from_live(
        state: &AppState,
        app_type: &AppType,
        provider: &Provider,
        live_config: &Value,
        result: &mut SwitchResult,
    ) {
        // 本项目仅 Codex 使用通用配置片段。
        if !matches!(app_type, AppType::Codex) {
            return;
        }

        let opted_in = provider
            .meta
            .as_ref()
            .and_then(|meta| meta.common_config_enabled)
            == Some(true);
        if !opted_in {
            return;
        }

        match state.db.is_config_snippet_cleared(app_type.as_str()) {
            Ok(true) => return, // 用户显式清空过通用配置，尊重其选择，不再自动塞回
            Ok(false) => {}
            Err(err) => {
                log::warn!(
                    "Failed to read common config cleared flag for {}: {err}",
                    app_type.as_str()
                );
                return;
            }
        }

        let new_snippet = match Self::extract_common_config_snippet_from_settings(
            app_type.clone(),
            live_config,
        ) {
            Ok(snippet) => snippet,
            Err(err) => {
                log::warn!(
                    "Failed to extract common config from live for {} provider '{}': {err}",
                    app_type.as_str(),
                    provider.id
                );
                return;
            }
        };

        // 未变化则跳过，避免无谓写库（不切 live 配置时这是常态路径）。
        let current = state
            .db
            .get_config_snippet(app_type.as_str())
            .ok()
            .flatten();
        if current.as_deref() == Some(new_snippet.as_str()) {
            return;
        }

        if let Err(err) = state
            .db
            .set_config_snippet(app_type.as_str(), Some(new_snippet))
        {
            log::warn!(
                "Failed to persist synced common config for {} provider '{}': {err}",
                app_type.as_str(),
                provider.id
            );
            result
                .warnings
                .push(format!("common_config_sync_failed:{}", provider.id));
        }
    }

    /// Extract common config snippet from current provider
    ///
    /// Extracts the current provider's configuration and removes provider-specific fields
    /// (API keys, model settings, endpoints) to create a reusable common config snippet.
    pub fn extract_common_config_snippet(
        state: &AppState,
        app_type: AppType,
    ) -> Result<String, AppError> {
        // Get current provider
        let current_id = Self::current(state, app_type.clone())?;
        if current_id.is_empty() {
            return Err(AppError::Message("No current provider".to_string()));
        }

        let providers = state.db.get_all_providers(app_type.as_str())?;
        let provider = providers
            .get(&current_id)
            .ok_or_else(|| AppError::Message(format!("Provider {current_id} not found")))?;

        match app_type {
            AppType::Codex => Self::extract_codex_common_config(&provider.settings_config),
        }
    }

    /// Extract common config snippet from a config value (e.g. editor content).
    pub fn extract_common_config_snippet_from_settings(
        app_type: AppType,
        settings_config: &Value,
    ) -> Result<String, AppError> {
        match app_type {
            AppType::Codex => Self::extract_codex_common_config(settings_config),
        }
    }

    /// 判断一个 env / 顶层配置键名是否为凭据/机密：凡命中一律不得写入共享的
    /// 通用配置片段。**故意从严**——多剥一个非机密键只是它不被共享（可恢复的小
    /// 不便），漏剥一个凭据则会把密钥注入到每个供应商（不可恢复的泄漏）。因此用
    /// 模式匹配覆盖整类，而非枚举具体名字（枚举永远会漏掉下一个 `*_API_KEY`）。
    ///
    /// 覆盖：Anthropic / OpenRouter / Google / OpenAI / Gemini 等 `*_API_KEY`
    /// （Claude provider 的凭据见 `Provider::resolve_usage_credentials`，确实支持
    /// `OPENROUTER_API_KEY` / `GOOGLE_API_KEY` 等回退）、各类 `*_AUTH_TOKEN` /
    /// 单数 `*_TOKEN`、AWS Bedrock / Vertex 凭据、以及通用 secret / password /
    /// 私钥命名。
    pub(crate) fn is_sensitive_config_key(name: &str) -> bool {
        let upper = name.to_ascii_uppercase();

        // 单数 `_TOKEN` 命中 AWS_SESSION_TOKEN 等，但**不**误伤复数 `_TOKENS`
        // （CLAUDE_CODE_MAX_OUTPUT_TOKENS / MAX_THINKING_TOKENS 是正常可共享配置）。
        const SENSITIVE_SUFFIXES: &[&str] = &[
            // 裸 `_KEY` 是最常见的凭据写法（OPENAI_KEY / GROQ_KEY / XAI_KEY…），
            // 必须单列：只枚举 `_API_KEY` / `_ACCESS_KEY` 这些子类，等于把最普通
            // 的那一种漏在外面。下面几条 `_*_KEY` 被它蕴含，保留是为了说明覆盖面。
            "_KEY",
            "_API_KEY",
            "_ACCESS_KEY",
            "_ACCESS_KEY_ID",
            "_KEY_ID",
            "_PRIVATE_KEY",
            // 不带分隔符的复合写法各走各的后缀：`_KEY` 够不着 `..._APIKEY`
            // （倒数第四个字符是 I 不是下划线）。VOLC_ACCESSKEY 是火山引擎文档
            // 里的正式变量名，本仓库就实现了火山 AK/SK 用量查询。
            "_APIKEY",
            "_ACCESSKEY",
            "_SECRETKEY",
            "_APITOKEN",
            "_AUTH_TOKEN",
            "_TOKEN",
            // GITHUB_PAT / GITLAB_PAT 等 personal access token 的惯用写法，
            // 既不含 TOKEN 也不含 KEY，前面每一条规则都够不着。
            "_PAT",
            // 口令类的常见缩写。`_PASS` 不会误伤 `*_BYPASS`（那个以 `_BYPASS`
            // 结尾），`_PWD` 也不会误伤 shell 的 PWD / OLDPWD。
            "_PWD",
            "_PASS",
            "_PASSPHRASE",
            "_CREDS",
        ];
        const SENSITIVE_EXACT: &[&str] = &[
            "APIKEY",
            "API_KEY",
            "TOKEN",
            "SECRET",
            "PASSWORD",
            "CREDENTIALS",
        ];
        // contains：覆盖 AWS_SECRET_ACCESS_KEY / *_CLIENT_SECRET /
        // GOOGLE_APPLICATION_CREDENTIALS / AWS_BEARER_TOKEN_BEDROCK 等变体。
        const SENSITIVE_CONTAINS: &[&str] = &[
            "SECRET",
            "PASSWORD",
            "PASSWD",
            "CREDENTIAL",
            "PRIVATE_KEY",
            "BEARER_TOKEN",
        ];

        SENSITIVE_EXACT.contains(&upper.as_str())
            || SENSITIVE_SUFFIXES.iter().any(|s| upper.ends_with(s))
            || SENSITIVE_CONTAINS.iter().any(|c| upper.contains(c))
    }

    /// Extract common config for Claude (JSON format)

    /// Extract common config for Codex (TOML format)
    fn extract_codex_common_config(settings: &Value) -> Result<String, AppError> {
        // Codex config is stored as { "auth": {...}, "config": "toml string" }
        let config_toml = settings
            .get("config")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if config_toml.is_empty() {
            return Ok(String::new());
        }

        let mut doc = config_toml
            .parse::<toml_edit::DocumentMut>()
            .map_err(|e| AppError::Message(format!("TOML parse error: {e}")))?;

        // Remove provider-specific fields.
        let root = doc.as_table_mut();
        root.remove("model");
        root.remove("model_provider");
        // Legacy/alt formats might use a top-level base_url.
        root.remove("base_url");
        // wire_api 与 base_url 同属供应商路由语义：无 model_provider 时
        // update_codex_toml_field / 前端 setCodexWireApi 都会把它落在顶层，
        // 进了片段会改写其它供应商的协议选择（chat vs responses）。
        root.remove("wire_api");

        // Remove entire model_providers table (provider-specific configuration)
        root.remove("model_providers");

        // MCP 服务器归 DB mcp_servers 表所有：进了共享片段会绕过按应用的
        // 启用状态被合并进所有勾选通用配置的供应商，且在通用配置编辑框里
        // 显示为一份"重复"的 MCP 配置。
        root.remove("mcp_servers");
        // 历史错误格式 [mcp.servers] 一并剥离（与 strip_codex_mcp_servers_from_settings
        // 一致）：sync_all_enabled 只管理 [mcp_servers.*]，legacy 形态一旦进了
        // 片段就会被合并进所有供应商，且没有任何同步路径能清掉这个孤儿。
        if let Some(mcp_tbl) = root
            .get_mut("mcp")
            .and_then(|item| item.as_table_like_mut())
        {
            mcp_tbl.remove("servers");
            if mcp_tbl.is_empty() {
                root.remove("mcp");
            }
        }

        // Codex Cube 写 live 时注入的产物一律不进共享片段：
        // - experimental_bearer_token 正常写在 [model_providers.<id>] 内（上面
        //   整表已剥），但无活跃路由 / 内建保留 id / 路由表缺失三种 fallback
        //   会落在顶层——不剥等于把 API 密钥写进共享片段。
        root.remove("experimental_bearer_token");
        // - model_catalog_json 指向按供应商生成的 catalog 投影文件（DB 为 SSOT）。
        root.remove("model_catalog_json");
        // - web_search 只剥 Codex Cube 注入的 "disabled" 哨兵；用户手设的其它值
        //   属于可共享偏好，保留。
        if root
            .get(crate::codex_config::CODEX_WEB_SEARCH_FIELD)
            .and_then(|item| item.as_str())
            == Some(crate::codex_config::CODEX_WEB_SEARCH_DISABLED)
        {
            root.remove(crate::codex_config::CODEX_WEB_SEARCH_FIELD);
        }

        // Clean up multiple empty lines (keep at most one blank line).
        let mut cleaned = String::new();
        let mut blank_run = 0usize;
        for line in doc.to_string().lines() {
            if line.trim().is_empty() {
                blank_run += 1;
                if blank_run <= 1 {
                    cleaned.push('\n');
                }
                continue;
            }
            blank_run = 0;
            cleaned.push_str(line);
            cleaned.push('\n');
        }

        Ok(cleaned.trim().to_string())
    }

    /// Extract common config for Gemini (JSON format)
    ///
    /// Extracts `.env` values while excluding provider-specific credentials:
    /// - GOOGLE_GEMINI_BASE_URL
    /// - GEMINI_API_KEY

    /// 一次性清理：把历史泄漏进 Gemini 共享片段的凭据从所有存储位置抹掉。
    ///
    /// 背景：`extract_gemini_common_config` 曾只剥离两个固定键名，`GOOGLE_API_KEY`
    /// 等一等凭据会进入共享片段，再被 `apply_common_config_to_settings` 深合并进
    /// **其它** Gemini 供应商的 env，随请求发往对方的 base_url。
    ///
    /// 光修提取器不够：Gemini 的片段一旦生成就**永不自动重提取**（启动期
    /// auto-extract 与导入后补提取都要求 `snippet.is_none()`，切换时的回写又只对
    /// Claude / Codex 生效），所以存量片段会一直带着密钥继续注入。
    ///
    /// 两个关键约束：
    ///
    /// 1. **不能只清片段**。合并与剥离是一对靠「值相等」严格抵消的操作：切走供应商时
    ///    `remove_common_config_from_settings` 依据片段内容把注入的键删掉。片段里一旦
    ///    没了这个键，backfill 就会把 live 中残留的密钥原样写进受害供应商的
    ///    `settings_config`——泄漏从瞬时污染变成永久污染。所以片段、各供应商配置、
    ///    live 文件必须一起清。
    /// 2. **按值相等定向删除，不按键名一刀切**。复用 `remove_common_config_from_settings`
    ///    可以只清掉扩散出去的那一份，保留某个供应商自己写的、值不同的同名键。
    ///
    /// 步骤顺序本身是安全属性的一部分：**清片段必须排在最后**。片段是
    /// `remove_common_config_from_settings` 唯一的"该剥哪些键"来源，一旦清空，任何
    /// 残留（live 文件里的、下一轮重试要处理的）都再也无法被识别和剥离。所以所有
    /// 可能失败的步骤都排在它前面，失败即带错返回，让下次启动能原样重来。
    ///
    /// 清理后部分供应商会显示缺少 API Key，需用户重填——这是正确行为：那把密钥本就
    /// 不属于它们。（受害者原有的同名键在合并时已被覆盖，无法恢复。）动手前会往
    /// settings 的 `gemini_common_config_scrub_audit_v1` 写一条审计记录，内容是
    /// **键名与受影响的供应商 id，不含值**：`settings` 会随 WebDAV/S3 同步上传，
    /// 而这里处理的正是必须销毁的凭据，留值等于把一次清除换成一份跨设备扩散、
    /// 没有界面入口、永不过期的明文副本。
    /// Import default configuration from live files (re-export)
    ///
    /// Returns `Ok(true)` if imported, `Ok(false)` if skipped.
    pub fn import_default_config(state: &AppState, app_type: AppType) -> Result<bool, AppError> {
        import_default_config(state, app_type)
    }

    pub fn should_import_default_config_on_startup(
        state: &AppState,
        app_type: &AppType,
    ) -> Result<bool, AppError> {
        should_import_default_config_on_startup(state, app_type)
    }

    /// Read current live settings (re-export)
    pub fn read_live_settings(app_type: AppType) -> Result<Value, AppError> {
        read_live_settings(app_type)
    }

    /// Get custom endpoints list (re-export)
    pub fn get_custom_endpoints(
        state: &AppState,
        app_type: AppType,
        provider_id: &str,
    ) -> Result<Vec<CustomEndpoint>, AppError> {
        endpoints::get_custom_endpoints(state, app_type, provider_id)
    }

    /// Add custom endpoint (re-export)
    pub fn add_custom_endpoint(
        state: &AppState,
        app_type: AppType,
        provider_id: &str,
        url: String,
    ) -> Result<(), AppError> {
        endpoints::add_custom_endpoint(state, app_type, provider_id, url)
    }

    /// Remove custom endpoint (re-export)
    pub fn remove_custom_endpoint(
        state: &AppState,
        app_type: AppType,
        provider_id: &str,
        url: String,
    ) -> Result<(), AppError> {
        endpoints::remove_custom_endpoint(state, app_type, provider_id, url)
    }

    /// Update endpoint last used timestamp (re-export)
    pub fn update_endpoint_last_used(
        state: &AppState,
        app_type: AppType,
        provider_id: &str,
        url: String,
    ) -> Result<(), AppError> {
        endpoints::update_endpoint_last_used(state, app_type, provider_id, url)
    }

    /// Update provider sort order
    pub fn update_sort_order(
        state: &AppState,
        app_type: AppType,
        updates: Vec<ProviderSortUpdate>,
    ) -> Result<bool, AppError> {
        let mut providers = state.db.get_all_providers(app_type.as_str())?;

        for update in updates {
            if let Some(provider) = providers.get_mut(&update.id) {
                provider.sort_index = Some(update.sort_index);
                state.db.save_provider(app_type.as_str(), provider)?;
            }
        }

        Ok(true)
    }

    /// Query provider usage (re-export)
    pub async fn query_usage(
        state: &AppState,
        app_type: AppType,
        provider_id: &str,
    ) -> Result<UsageResult, AppError> {
        usage::query_usage(state, app_type, provider_id).await
    }

    /// Test usage script (re-export)
    pub async fn test_usage_script(
        state: &AppState,
        app_type: AppType,
        provider_id: &str,
        script_code: &str,
        timeout: u64,
        api_key: Option<&str>,
        base_url: Option<&str>,
        access_token: Option<&str>,
        user_id: Option<&str>,
        template_type: Option<&str>,
    ) -> Result<UsageResult, AppError> {
        usage::test_usage_script(
            state,
            app_type,
            provider_id,
            script_code,
            timeout,
            api_key,
            base_url,
            access_token,
            user_id,
            template_type,
        )
        .await
    }

    fn validate_provider_settings(app_type: &AppType, provider: &Provider) -> Result<(), AppError> {
        match app_type {
            AppType::Codex => {
                let settings = provider.settings_config.as_object().ok_or_else(|| {
                    AppError::localized(
                        "provider.codex.settings.not_object",
                        "Codex 配置必须是 JSON 对象",
                        "Codex configuration must be a JSON object",
                    )
                })?;

                let auth = settings.get("auth").ok_or_else(|| {
                    AppError::localized(
                        "provider.codex.auth.missing",
                        format!("供应商 {} 缺少 auth 配置", provider.id),
                        format!("Provider {} is missing auth configuration", provider.id),
                    )
                })?;
                if !auth.is_object() {
                    return Err(AppError::localized(
                        "provider.codex.auth.not_object",
                        format!("供应商 {} 的 auth 配置必须是 JSON 对象", provider.id),
                        format!(
                            "Provider {} auth configuration must be a JSON object",
                            provider.id
                        ),
                    ));
                }

                if let Some(config_value) = settings.get("config") {
                    if !(config_value.is_string() || config_value.is_null()) {
                        return Err(AppError::localized(
                            "provider.codex.config.invalid_type",
                            "Codex config 字段必须是字符串",
                            "Codex config field must be a string",
                        ));
                    }
                    if let Some(cfg_text) = config_value.as_str() {
                        crate::codex_config::validate_config_toml(cfg_text)?;
                    }
                }
            }
        }

        // Validate and clean UsageScript configuration (common for all app types)
        if let Some(meta) = &provider.meta {
            if let Some(multiplier) = meta.cost_multiplier.as_deref() {
                validate_cost_multiplier(multiplier)?;
            }
            if let Some(source) = meta.pricing_model_source.as_deref() {
                validate_pricing_source(source)?;
            }
            if let Some(usage_script) = &meta.usage_script {
                validate_usage_script(usage_script)?;
            }
        }

        Ok(())
    }

    #[allow(dead_code)]
    fn extract_credentials(
        provider: &Provider,
        app_type: &AppType,
    ) -> Result<(String, String), AppError> {
        match app_type {
            AppType::Codex => {
                let _auth = provider
                    .settings_config
                    .get("auth")
                    .and_then(|v| v.as_object())
                    .ok_or_else(|| {
                        AppError::localized(
                            "provider.codex.auth.missing",
                            "配置格式错误: 缺少 auth",
                            "Invalid configuration: missing auth section",
                        )
                    })?;

                let config_toml = provider
                    .settings_config
                    .get("config")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                let api_key = crate::codex_config::extract_codex_api_key(
                    provider.settings_config.get("auth"),
                    Some(config_toml),
                )
                .ok_or_else(|| {
                    AppError::localized(
                        "provider.codex.api_key.missing",
                        "缺少 API Key",
                        "API key is missing",
                    )
                })?;

                let base_url = if config_toml.contains("base_url") {
                    let re = Regex::new(r#"base_url\s*=\s*["']([^"']+)["']"#).map_err(|e| {
                        AppError::localized(
                            "provider.regex_init_failed",
                            format!("正则初始化失败: {e}"),
                            format!("Failed to initialize regex: {e}"),
                        )
                    })?;
                    re.captures(config_toml)
                        .and_then(|caps| caps.get(1))
                        .map(|m| m.as_str().to_string())
                        .ok_or_else(|| {
                            AppError::localized(
                                "provider.codex.base_url.invalid",
                                "config.toml 中 base_url 格式错误",
                                "base_url in config.toml has invalid format",
                            )
                        })?
                } else {
                    return Err(AppError::localized(
                        "provider.codex.base_url.missing",
                        "config.toml 中缺少 base_url 配置",
                        "base_url is missing from config.toml",
                    ));
                };

                Ok((api_key, base_url))
            }
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProviderSortUpdate {
    pub id: String,
    pub sort_index: usize,
}
