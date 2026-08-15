//! Live configuration operations
//!
//! Handles reading and writing live configuration files for Claude, Codex, and Gemini.

use std::collections::HashMap;

use serde_json::{json, Value};
use toml_edit::{DocumentMut, Item, TableLike};

use crate::app_config::AppType;
use crate::codex_config::{get_codex_auth_path, get_codex_config_path};
use crate::config::{delete_file, read_json_file, write_json_file};
use crate::database::Database;
use crate::error::AppError;
use crate::provider::Provider;
use crate::services::mcp::McpService;
use crate::store::AppState;

/// ChatGPT Codex catalogs gpt-5.6 at a 372K context window with a ~353K
/// effective budget (openai/codex#31860), far below the 1.05M API spec.
/// Declare the catalog window for both knobs: Claude Code's built-in output
/// reserve and compact buffer already keep the actual compact trigger
/// (~278K-339K) below the effective budget, so anything lower only wastes
/// usable context.
const CODEX_OAUTH_CLAUDE_MAX_CONTEXT_TOKENS: &str = "372000";
const CODEX_OAUTH_CLAUDE_AUTO_COMPACT_WINDOW: &str = "372000";
const KIMI_FOR_CODING_CONTEXT_TOKENS: &str = "262144";

/// Model env keys Claude Code may route requests through. The defaults above
/// are calibrated against gpt-5.6's Codex catalog, so every configured model
/// must belong to that family before they are injected — gpt-5.5's upstream
/// catalog oscillates between 272K and 372K and must not inherit them.
const CODEX_OAUTH_MODEL_ENV_KEYS: [&str; 6] = [
    "ANTHROPIC_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_FABLE_MODEL",
    "CLAUDE_CODE_SUBAGENT_MODEL",
];

fn provider_env_targets_gpt56(provider_env: Option<&serde_json::Map<String, Value>>) -> bool {
    let Some(env) = provider_env else {
        return false;
    };
    let mut saw_model = false;
    for key in CODEX_OAUTH_MODEL_ENV_KEYS {
        let Some(value) = env.get(key) else {
            continue;
        };
        let Some(model) = value.as_str() else {
            return false;
        };
        let model = model.trim();
        if model.is_empty() {
            continue;
        }
        saw_model = true;
        if !model.to_ascii_lowercase().starts_with("gpt-5.6") {
            return false;
        }
    }
    saw_model
}

fn is_kimi_for_coding_provider(provider: &Provider) -> bool {
    provider
        .settings_config
        .pointer("/env/ANTHROPIC_BASE_URL")
        .and_then(Value::as_str)
        .map(str::trim)
        .map(|url| url.trim_end_matches('/'))
        == Some("https://api.kimi.com/coding")
}

/// Claude Code assigns unknown non-Claude model ids a 200K context window.
/// Codex OAuth deliberately exposes GPT ids through Claude Code, so enrich the
/// effective live settings for both newly-created and already-saved providers.
/// Explicit user values always win; the defaults are only injected when every
/// configured model targets gpt-5.6.
pub(crate) fn provider_exists_in_live_config(
    _app_type: &AppType,
    _provider_id: &str,
) -> Result<bool, AppError> {
    // 本项目仅支持 Codex：Codex 的 live 配置不按 provider 文件分片。
    Ok(false)
}

fn json_is_subset(target: &Value, source: &Value) -> bool {
    match source {
        Value::Object(source_map) => {
            let Some(target_map) = target.as_object() else {
                return false;
            };
            source_map.iter().all(|(key, source_value)| {
                target_map
                    .get(key)
                    .is_some_and(|target_value| json_is_subset(target_value, source_value))
            })
        }
        Value::Array(source_arr) => {
            let Some(target_arr) = target.as_array() else {
                return false;
            };
            json_array_contains_subset(target_arr, source_arr)
        }
        _ => target == source,
    }
}

fn json_array_contains_subset(target_arr: &[Value], source_arr: &[Value]) -> bool {
    let mut matched = vec![false; target_arr.len()];

    source_arr.iter().all(|source_item| {
        if let Some((index, _)) = target_arr.iter().enumerate().find(|(index, target_item)| {
            !matched[*index] && json_is_subset(target_item, source_item)
        }) {
            matched[index] = true;
            true
        } else {
            false
        }
    })
}

fn json_remove_array_items(target_arr: &mut Vec<Value>, source_arr: &[Value]) {
    for source_item in source_arr {
        if let Some(index) = target_arr
            .iter()
            .position(|target_item| json_is_subset(target_item, source_item))
        {
            target_arr.remove(index);
        }
    }
}

fn json_deep_merge(target: &mut Value, source: &Value) {
    match (target, source) {
        (Value::Object(target_map), Value::Object(source_map)) => {
            for (key, source_value) in source_map {
                match target_map.get_mut(key) {
                    Some(target_value) => json_deep_merge(target_value, source_value),
                    None => {
                        target_map.insert(key.clone(), source_value.clone());
                    }
                }
            }
        }
        (target_value, source_value) => {
            *target_value = source_value.clone();
        }
    }
}

fn json_deep_remove(target: &mut Value, source: &Value) {
    let (Some(target_map), Some(source_map)) = (target.as_object_mut(), source.as_object()) else {
        return;
    };

    for (key, source_value) in source_map {
        let mut remove_key = false;

        if let Some(target_value) = target_map.get_mut(key) {
            if source_value.is_object() && target_value.is_object() {
                json_deep_remove(target_value, source_value);
                remove_key = target_value.as_object().is_some_and(|obj| obj.is_empty());
            } else if let (Some(target_arr), Some(source_arr)) =
                (target_value.as_array_mut(), source_value.as_array())
            {
                json_remove_array_items(target_arr, source_arr);
                remove_key = target_arr.is_empty();
            } else if json_is_subset(target_value, source_value) {
                remove_key = true;
            }
        }

        if remove_key {
            target_map.remove(key);
        }
    }
}

fn toml_value_is_subset(target: &toml_edit::Value, source: &toml_edit::Value) -> bool {
    match (target, source) {
        (toml_edit::Value::String(target), toml_edit::Value::String(source)) => {
            target.value() == source.value()
        }
        (toml_edit::Value::Integer(target), toml_edit::Value::Integer(source)) => {
            target.value() == source.value()
        }
        (toml_edit::Value::Float(target), toml_edit::Value::Float(source)) => {
            target.value() == source.value()
        }
        (toml_edit::Value::Boolean(target), toml_edit::Value::Boolean(source)) => {
            target.value() == source.value()
        }
        (toml_edit::Value::Datetime(target), toml_edit::Value::Datetime(source)) => {
            target.value() == source.value()
        }
        (toml_edit::Value::Array(target), toml_edit::Value::Array(source)) => {
            toml_array_contains_subset(target, source)
        }
        (toml_edit::Value::InlineTable(target), toml_edit::Value::InlineTable(source)) => {
            source.iter().all(|(key, source_item)| {
                target
                    .get(key)
                    .is_some_and(|target_item| toml_value_is_subset(target_item, source_item))
            })
        }
        _ => false,
    }
}

fn toml_array_contains_subset(target: &toml_edit::Array, source: &toml_edit::Array) -> bool {
    let mut matched = vec![false; target.len()];
    let target_items: Vec<&toml_edit::Value> = target.iter().collect();

    source.iter().all(|source_item| {
        if let Some((index, _)) = target_items
            .iter()
            .enumerate()
            .find(|(index, target_item)| {
                !matched[*index] && toml_value_is_subset(target_item, source_item)
            })
        {
            matched[index] = true;
            true
        } else {
            false
        }
    })
}

fn toml_remove_array_items(target: &mut toml_edit::Array, source: &toml_edit::Array) {
    for source_item in source.iter() {
        let index = {
            let target_items: Vec<&toml_edit::Value> = target.iter().collect();
            target_items
                .iter()
                .enumerate()
                .find(|(_, target_item)| toml_value_is_subset(target_item, source_item))
                .map(|(index, _)| index)
        };

        if let Some(index) = index {
            target.remove(index);
        }
    }
}

fn toml_item_is_subset(target: &Item, source: &Item) -> bool {
    if let Some(source_table) = source.as_table_like() {
        let Some(target_table) = target.as_table_like() else {
            return false;
        };
        return source_table.iter().all(|(key, source_item)| {
            target_table
                .get(key)
                .is_some_and(|target_item| toml_item_is_subset(target_item, source_item))
        });
    }

    match (target.as_value(), source.as_value()) {
        (Some(target_value), Some(source_value)) => {
            toml_value_is_subset(target_value, source_value)
        }
        _ => false,
    }
}

fn merge_toml_item(target: &mut Item, source: &Item) {
    if let Some(source_table) = source.as_table_like() {
        if let Some(target_table) = target.as_table_like_mut() {
            merge_toml_table_like(target_table, source_table);
            return;
        }
    }

    *target = source.clone();
}

fn merge_toml_table_like(target: &mut dyn TableLike, source: &dyn TableLike) {
    for (key, source_item) in source.iter() {
        match target.get_mut(key) {
            Some(target_item) => merge_toml_item(target_item, source_item),
            None => {
                target.insert(key, source_item.clone());
            }
        }
    }
}

fn remove_toml_item(target: &mut Item, source: &Item) {
    if let Some(source_table) = source.as_table_like() {
        if let Some(target_table) = target.as_table_like_mut() {
            remove_toml_table_like(target_table, source_table);
            if target_table.is_empty() {
                *target = Item::None;
            }
            return;
        }
    }

    if let Some(source_value) = source.as_value() {
        let mut remove_item = false;

        if let Some(target_value) = target.as_value_mut() {
            match (target_value, source_value) {
                (toml_edit::Value::Array(target_arr), toml_edit::Value::Array(source_arr)) => {
                    toml_remove_array_items(target_arr, source_arr);
                    remove_item = target_arr.is_empty();
                }
                (target_value, source_value)
                    if toml_value_is_subset(target_value, source_value) =>
                {
                    remove_item = true;
                }
                _ => {}
            }
        }

        if remove_item {
            *target = Item::None;
        }
    }
}

fn remove_toml_table_like(target: &mut dyn TableLike, source: &dyn TableLike) {
    let keys: Vec<String> = source.iter().map(|(key, _)| key.to_string()).collect();

    for key in keys {
        let mut remove_key = false;
        if let (Some(target_item), Some(source_item)) = (target.get_mut(&key), source.get(&key)) {
            remove_toml_item(target_item, source_item);
            remove_key = target_item.is_none()
                || target_item
                    .as_table_like()
                    .is_some_and(|table_like| table_like.is_empty());
        }

        if remove_key {
            target.remove(&key);
        }
    }
}

/// 前端表单勾选/取消"使用通用配置"时，对编辑器里的 config.toml 文本做
/// 结构化合并/剥离。必须在后端用 toml_edit 做：前端 smol-toml 只能
/// parse → merge → 整文档重序列化，注释全丢、键序重排，还会生成多余的
/// 空父表头（如 `[model_providers]`）。
pub fn update_toml_common_config_snippet(
    config_toml: &str,
    snippet_toml: &str,
    enabled: bool,
) -> Result<String, AppError> {
    let trimmed = snippet_toml.trim();
    if trimmed.is_empty() {
        return Ok(config_toml.to_string());
    }

    let mut target_doc = if config_toml.trim().is_empty() {
        DocumentMut::new()
    } else {
        config_toml
            .parse::<DocumentMut>()
            .map_err(|e| AppError::Message(format!("Invalid Codex config.toml: {e}")))?
    };
    let source_doc = trimmed
        .parse::<DocumentMut>()
        .map_err(|e| AppError::Message(format!("Invalid Codex common config snippet: {e}")))?;

    if enabled {
        merge_toml_table_like(target_doc.as_table_mut(), source_doc.as_table());
    } else {
        remove_toml_table_like(target_doc.as_table_mut(), source_doc.as_table());
    }

    Ok(target_doc.to_string())
}

fn settings_contain_common_config(app_type: &AppType, settings: &Value, snippet: &str) -> bool {
    let trimmed = snippet.trim();
    if trimmed.is_empty() {
        return false;
    }

    match app_type {
        AppType::Codex => {
            let config_toml = settings.get("config").and_then(Value::as_str).unwrap_or("");
            if config_toml.trim().is_empty() {
                return false;
            }

            let target_doc = match config_toml.parse::<DocumentMut>() {
                Ok(doc) => doc,
                Err(_) => return false,
            };
            let source_doc = match trimmed.parse::<DocumentMut>() {
                Ok(doc) => doc,
                Err(_) => return false,
            };

            toml_item_is_subset(target_doc.as_item(), source_doc.as_item())
        }
    }
}

pub(crate) fn provider_uses_common_config(
    app_type: &AppType,
    provider: &Provider,
    snippet: Option<&str>,
) -> bool {
    match provider
        .meta
        .as_ref()
        .and_then(|meta| meta.common_config_enabled)
    {
        Some(explicit) => explicit && snippet.is_some_and(|value| !value.trim().is_empty()),
        None => snippet.is_some_and(|value| {
            settings_contain_common_config(app_type, &provider.settings_config, value)
        }),
    }
}

pub(crate) fn remove_common_config_from_settings(
    app_type: &AppType,
    settings: &Value,
    snippet: &str,
) -> Result<Value, AppError> {
    let trimmed = snippet.trim();
    if trimmed.is_empty() {
        return Ok(settings.clone());
    }

    match app_type {
        AppType::Codex => {
            let mut result = settings.clone();
            let config_toml = settings.get("config").and_then(Value::as_str).unwrap_or("");
            let mut target_doc = if config_toml.trim().is_empty() {
                DocumentMut::new()
            } else {
                config_toml.parse::<DocumentMut>().map_err(|e| {
                    AppError::Message(format!(
                        "Invalid Codex config.toml while removing common config: {e}"
                    ))
                })?
            };
            let source_doc = trimmed.parse::<DocumentMut>().map_err(|e| {
                AppError::Message(format!("Invalid Codex common config snippet: {e}"))
            })?;

            remove_toml_table_like(target_doc.as_table_mut(), source_doc.as_table());
            if let Some(obj) = result.as_object_mut() {
                obj.insert("config".to_string(), Value::String(target_doc.to_string()));
            }
            Ok(result)
        }
    }
}

fn apply_common_config_to_settings(
    app_type: &AppType,
    settings: &Value,
    snippet: &str,
) -> Result<Value, AppError> {
    let trimmed = snippet.trim();
    if trimmed.is_empty() {
        return Ok(settings.clone());
    }

    match app_type {
        AppType::Codex => {
            let mut result = settings.clone();
            let config_toml = settings.get("config").and_then(Value::as_str).unwrap_or("");
            let mut target_doc = if config_toml.trim().is_empty() {
                DocumentMut::new()
            } else {
                config_toml.parse::<DocumentMut>().map_err(|e| {
                    AppError::Message(format!(
                        "Invalid Codex config.toml while applying common config: {e}"
                    ))
                })?
            };
            let source_doc = trimmed.parse::<DocumentMut>().map_err(|e| {
                AppError::Message(format!("Invalid Codex common config snippet: {e}"))
            })?;

            merge_toml_table_like(target_doc.as_table_mut(), source_doc.as_table());
            if let Some(obj) = result.as_object_mut() {
                obj.insert("config".to_string(), Value::String(target_doc.to_string()));
            }
            Ok(result)
        }
    }
}

pub(crate) fn build_effective_settings_with_common_config(
    db: &Database,
    app_type: &AppType,
    provider: &Provider,
) -> Result<Value, AppError> {
    let snippet = db.get_config_snippet(app_type.as_str())?;
    let mut effective_settings = provider.settings_config.clone();

    if provider_uses_common_config(app_type, provider, snippet.as_deref()) {
        if let Some(snippet_text) = snippet.as_deref() {
            match apply_common_config_to_settings(app_type, &effective_settings, snippet_text) {
                Ok(settings) => effective_settings = settings,
                Err(err) => {
                    log::warn!(
                        "Failed to apply common config for {} provider '{}': {err}",
                        app_type.as_str(),
                        provider.id
                    );
                }
            }
        }
    }

    Ok(effective_settings)
}

pub(crate) fn write_live_with_common_config(
    db: &Database,
    app_type: &AppType,
    provider: &Provider,
) -> Result<(), AppError> {
    let mut effective_provider = provider.clone();
    effective_provider.settings_config =
        build_effective_settings_with_common_config(db, app_type, provider)?;

    // sandbox/approval/project trust are Codex user permission state, not
    // provider routing state. Preserve them across independent provider
    // snapshots without carrying model aliases, endpoints, protocols, or
    // catalogs between providers.
    if matches!(app_type, AppType::Codex) && !provider.is_aggregate() {
        match crate::codex_config::read_codex_live_settings() {
            Ok(live_settings) => {
                effective_provider.settings_config =
                    crate::codex_config::merge_codex_live_permission_settings(
                        &effective_provider.settings_config,
                        &live_settings,
                    );
            }
            Err(e) => {
                log::warn!("读取 Codex Live 权限设置失败，切换将不保留 sandbox/approval 设置: {e}");
            }
        }
    }

    write_live_snapshot(app_type, &effective_provider)
}

pub(crate) fn strip_common_config_from_live_settings(
    db: &Database,
    app_type: &AppType,
    provider: &Provider,
    live_settings: Value,
) -> Value {
    let snippet = match db.get_config_snippet(app_type.as_str()) {
        Ok(snippet) => snippet,
        Err(err) => {
            log::warn!(
                "Failed to load common config for {} while backfilling '{}': {err}",
                app_type.as_str(),
                provider.id
            );
            return restore_live_settings_for_provider_backfill(app_type, provider, live_settings);
        }
    };

    let backfill_settings = if provider_uses_common_config(app_type, provider, snippet.as_deref()) {
        match snippet.as_deref() {
            Some(snippet_text) => {
                match remove_common_config_from_settings(app_type, &live_settings, snippet_text) {
                    Ok(settings) => settings,
                    Err(err) => {
                        log::warn!(
                            "Failed to strip common config for {} provider '{}': {err}",
                            app_type.as_str(),
                            provider.id
                        );
                        live_settings
                    }
                }
            }
            None => live_settings,
        }
    } else {
        live_settings
    };

    restore_live_settings_for_provider_backfill(app_type, provider, backfill_settings)
}

/// 与 `apply_codex_oauth_claude_context_defaults` 严格对称：注入产物只活在
/// live，切走回填时必须剥掉，否则程序默认值会固化成供应商的"用户显式值"，
/// 之后调整默认值或更换模型时旧值永远压住新默认。仅当"注入会发生且注入的
/// 就是这个值、且存储配置本来没有显式值"时才剥；用户显式存储的值和手改
/// live 成其他数字的值都保留。
fn restore_live_settings_for_provider_backfill(
    app_type: &AppType,
    provider: &Provider,
    live_settings: Value,
) -> Value {
    let mut settings = live_settings;
    let restore_provider_token =
        crate::codex_config::should_restore_codex_provider_token_for_backfill(
            provider.category.as_deref(),
            &provider.settings_config,
        );
    if let Err(err) = crate::codex_config::restore_codex_settings_for_backfill(
        &mut settings,
        &provider.settings_config,
        restore_provider_token,
    ) {
        log::warn!(
            "Failed to restore Codex settings while backfilling '{}': {err}",
            provider.id
        );
    }

    // MCP 服务器归 DB mcp_servers 表所有，live 里的 [mcp_servers] 是同步投影；
    // 回填时剥掉，否则已删除的服务器会随供应商快照复活（逐条 reconcile 清不掉孤儿）。
    if let Err(err) = crate::codex_config::strip_codex_mcp_servers_from_settings(&mut settings) {
        log::warn!(
            "Failed to strip mcp_servers while backfilling '{}': {err}",
            provider.id
        );
    }

    // 统一会话开关注入的共享 `custom` 路由只属于 live 配置；切换回填时
    // 必须剥掉，否则官方供应商的存储配置被污染，关闭开关后无法还原。
    if provider.category.as_deref() == Some("official") {
        if let Err(err) =
            crate::codex_config::strip_codex_unified_session_bucket_from_settings(&mut settings)
        {
            log::warn!(
                "Failed to strip unified session bucket while backfilling '{}': {err}",
                provider.id
            );
        }
    }

    // `modelCatalog` is a codex-cube–private field whose SSOT is the DB. Live's
    // `config.toml` only carries a lossy projection (`model_catalog_json` →
    // generated catalog file) that proxy takeover/restore cycles and Codex.app
    // config rewrites can drop, so `read_live_settings` may reconstruct it as
    // absent. Never let a switch-away backfill from Live erase the stored
    // mapping: prefer the DB provider's `modelCatalog`, falling back to whatever
    // Live reconstructed only when the DB has none.
    if let Some(stored_catalog) = provider.settings_config.get("modelCatalog") {
        if let Some(obj) = settings.as_object_mut() {
            obj.insert("modelCatalog".to_string(), stored_catalog.clone());
        }
    }

    settings
}

/// 聚合 Provider 是虚拟供应商：`settingsConfig` 保存的是成员/模型映射，不是
/// live 配置的镜像。切走/同步时绝不能拿 live 配置回填覆盖，否则
/// `memberProviderIds` / `aggregateModels` 会丢失。
pub(crate) fn should_backfill_provider_from_live(provider: &Provider) -> bool {
    !provider.is_aggregate()
}

pub(crate) fn normalize_provider_common_config_for_storage(
    db: &Database,
    app_type: &AppType,
    provider: &mut Provider,
) -> Result<(), AppError> {
    let uses_common_config = provider
        .meta
        .as_ref()
        .and_then(|meta| meta.common_config_enabled)
        .unwrap_or(false);

    if !uses_common_config {
        return Ok(());
    }

    let Some(snippet) = db.get_config_snippet(app_type.as_str())? else {
        return Ok(());
    };

    if snippet.trim().is_empty() {
        return Ok(());
    }

    match remove_common_config_from_settings(app_type, &provider.settings_config, &snippet) {
        Ok(settings) => provider.settings_config = settings,
        Err(err) => {
            log::warn!(
                "Failed to normalize common config before saving {} provider '{}': {err}",
                app_type.as_str(),
                provider.id
            );
        }
    }

    Ok(())
}

/// Live configuration snapshot for backup/restore
#[derive(Clone)]
#[allow(dead_code)]
pub(crate) enum LiveSnapshot {
    Codex {
        auth: Option<Value>,
        config: Option<String>,
    },
}

impl LiveSnapshot {
    #[allow(dead_code)]
    pub(crate) fn restore(&self) -> Result<(), AppError> {
        match self {
            LiveSnapshot::Codex { auth, config } => {
                let auth_path = get_codex_auth_path();
                let config_path = get_codex_config_path();
                if let Some(value) = auth {
                    write_json_file(&auth_path, value)?;
                } else if auth_path.exists() {
                    delete_file(&auth_path)?;
                }

                if let Some(text) = config {
                    crate::config::write_text_file(&config_path, text)?;
                } else if config_path.exists() {
                    delete_file(&config_path)?;
                }
            }
        }
        Ok(())
    }
}

/// Write live configuration snapshot for a provider
pub(crate) fn write_live_snapshot(app_type: &AppType, provider: &Provider) -> Result<(), AppError> {
    match app_type {
        AppType::Codex => {
            let obj = provider
                .settings_config
                .as_object()
                .ok_or_else(|| AppError::Config("Codex 供应商配置必须是 JSON 对象".to_string()))?;
            let auth = obj
                .get("auth")
                .ok_or_else(|| AppError::Config("Codex 供应商配置缺少 'auth' 字段".to_string()))?;
            let config_str = obj.get("config").and_then(|v| v.as_str());

            // Native (direct) Responses and Anthropic providers must suppress Codex's
            // freeform apply_patch custom tool via the generated catalog; chat/proxy
            // providers keep the default tool set. Uses the same Anthropic detection as
            // the proxy router (apiFormat meta/settings + TOML wire_api).
            let profile = crate::proxy::providers::resolve_codex_catalog_tool_profile(provider);

            crate::codex_config::write_codex_provider_live_with_catalog(
                &provider.settings_config,
                provider.category.as_deref(),
                auth,
                config_str,
                profile,
            )?;
        }
    }
    Ok(())
}

fn sync_all_providers_to_live(state: &AppState, app_type: &AppType) -> Result<(), AppError> {
    let providers = state.db.get_all_providers(app_type.as_str())?;
    let mut synced_count = 0usize;

    for provider in providers.values() {
        if provider
            .meta
            .as_ref()
            .and_then(|meta| meta.live_config_managed)
            == Some(false)
        {
            continue;
        }

        if let Err(e) = write_live_with_common_config(state.db.as_ref(), app_type, provider) {
            log::warn!(
                "Failed to sync {:?} provider '{}' to live: {e}",
                app_type,
                provider.id
            );
            continue;
        }
        synced_count += 1;
    }

    log::info!("Synced {synced_count} {app_type:?} providers to live config");
    Ok(())
}

pub(crate) fn sync_current_provider_for_app_to_live(
    state: &AppState,
    app_type: &AppType,
) -> Result<(), AppError> {
    if app_type.is_additive_mode() {
        sync_all_providers_to_live(state, app_type)?;
    } else {
        let current_id = match crate::settings::get_effective_current_provider(&state.db, app_type)?
        {
            Some(id) => id,
            None => return Ok(()),
        };

        let providers = state.db.get_all_providers(app_type.as_str())?;
        if let Some(provider) = providers.get(&current_id) {
            write_live_with_common_config(state.db.as_ref(), app_type, provider)?;
        }
    }

    // 本函数语义是"把这个应用同步到 live"，MCP 重投影也只针对该应用；
    // 全量 sync_all_enabled 会把无关应用的 live 损坏牵连进来。投影失败
    // 上抛（不降级）：这里没有已变更的 DB 状态需要保护，调用方重试即可。
    McpService::sync_enabled_for_app(state, app_type)?;

    Ok(())
}

fn sync_current_provider_for_app_respecting_takeover(
    state: &AppState,
    app_type: &AppType,
) -> Result<(), AppError> {
    let current_id = match crate::settings::get_effective_current_provider(&state.db, app_type)? {
        Some(id) => id,
        None => return Ok(()),
    };

    let providers = state.db.get_all_providers(app_type.as_str())?;
    let Some(provider) = providers.get(&current_id) else {
        return Ok(());
    };

    let has_live_backup = futures::executor::block_on(state.db.get_live_backup(app_type.as_str()))
        .ok()
        .flatten()
        .is_some();
    let live_taken_over = state
        .proxy_service
        .detect_takeover_in_live_config_for_app(app_type);

    // `enabled` is set only after takeover writes complete. During that
    // activation window, backup/live placeholders are the authoritative signal
    // that normal provider sync must not rewrite the managed live file.
    if has_live_backup || live_taken_over {
        futures::executor::block_on(
            state
                .proxy_service
                .update_live_backup_from_provider(app_type.as_str(), provider),
        )
        .map_err(|e| AppError::Message(format!("更新 Live 备份失败: {e}")))?;
        return Ok(());
    }

    write_live_with_common_config(state.db.as_ref(), app_type, provider)
}

/// Sync current provider to live configuration
///
/// 使用有效的当前供应商 ID（验证过存在性）。
/// 优先从本地 settings 读取，验证后 fallback 到数据库的 is_current 字段。
/// 这确保了配置导入后无效 ID 会自动 fallback 到数据库。
///
/// For additive mode apps (OpenCode), all providers are synced instead of just the current one.
pub fn sync_current_to_live(state: &AppState) -> Result<(), AppError> {
    // Sync providers based on mode
    for app_type in AppType::all() {
        if app_type.is_additive_mode() {
            // Additive mode: sync ALL providers
            sync_all_providers_to_live(state, &app_type)?;
        } else {
            // Switch mode: sync only current provider. During proxy takeover,
            // update the restore backup instead of rewriting the taken-over
            // live file.
            sync_current_provider_for_app_respecting_takeover(state, &app_type)?;
        }
    }

    // MCP sync（best-effort 逐应用投影，内部已聚合失败）。错误暂存到
    // Skill 同步之后再返回：MCP 的失败不该跳过 Skill 同步，但调用方
    //（配置导入 / 云同步恢复）需要知道结果不完整。
    let mcp_result = McpService::sync_all_enabled(state);

    // Skill sync
    for app_type in AppType::all() {
        if let Err(e) = crate::services::skill::SkillService::sync_to_app(&state.db, &app_type) {
            log::warn!("同步 Skill 到 {app_type:?} 失败: {e}");
            // Continue syncing other apps, don't abort
        }
    }

    mcp_result
}

/// Read current live settings for an app type
pub fn read_live_settings(app_type: AppType) -> Result<Value, AppError> {
    match app_type {
        AppType::Codex => {
            let mut result = crate::codex_config::read_codex_live_settings()?;
            // `modelCatalog` is a codex-cube private field that lives only in
            // the DB SSOT plus the `codex-cube-model-catalog.json` projection
            // file — it is never inlined into `auth.json` or `config.toml`.
            // Reverse-parse the projection so the edit form for the active
            // Codex provider doesn't see an empty mapping table.
            if let Ok(Some(model_catalog)) =
                crate::codex_config::read_codex_model_catalog_simplified_from_live()
            {
                if let Some(obj) = result.as_object_mut() {
                    obj.insert("modelCatalog".to_string(), model_catalog);
                }
            }
            Ok(result)
        }
    }
}

pub fn import_default_config(state: &AppState, app_type: AppType) -> Result<bool, AppError> {
    // 允许 "只有官方 seed 预设" 的情况下继续导入 live：
    // - 启动编排顺序是先 import 后 seed，新用户启动时 providers 为空，导入照常
    // - 老用户已有非 seed provider，跳过导入（正确）
    // - 用户手动点 ProviderEmptyState 的导入按钮时，与官方 seed 共存而不被阻塞
    if state.db.has_non_official_seed_provider(app_type.as_str())? {
        return Ok(false);
    }

    // 拒绝把"被代理接管的 Live"导入为供应商：接管期间 Live 里只有
    // PROXY_MANAGED 占位符和本地代理地址，不是用户的真实配置。一旦导入，
    // 它会成为 current provider（SSOT），后续"无备份恢复"路径会把占位符
    // 当真实配置写回 Live，永久卡在已失效的本地代理上。
    // 典型触发场景：代理接管开启时切换 app_config_dir 并重启，新数据库首启导入。
    if state
        .proxy_service
        .detect_takeover_in_live_config_for_app(&app_type)
    {
        return Err(AppError::localized(
            "provider.import.live_taken_over",
            "Live 配置当前处于代理接管状态（包含占位符），不能导入为供应商。请先关闭代理接管或恢复 Live 配置后重试。",
            "The live config is currently taken over by the proxy (contains placeholders) and cannot be imported as a provider. Disable proxy takeover or restore the live config first.",
        ));
    }

    let settings_config = match app_type {
        AppType::Codex => crate::codex_config::read_codex_live_settings()?,
    };

    let mut provider = Provider::with_id(
        "default".to_string(),
        "default".to_string(),
        settings_config,
        None,
    );
    provider.category = Some(
        {
            let config_text = provider
                .settings_config
                .get("config")
                .and_then(Value::as_str);
            let has_provider_key = crate::codex_config::extract_codex_api_key(
                provider.settings_config.get("auth"),
                config_text,
            )
            .is_some();
            let has_login_material = provider
                .settings_config
                .get("auth")
                .is_some_and(crate::codex_config::codex_auth_has_login_material);

            if has_login_material && !has_provider_key {
                "official"
            } else {
                "custom"
            }
        }
        .to_string(),
    );

    state.db.save_provider(app_type.as_str(), &provider)?;
    state
        .db
        .set_current_provider(app_type.as_str(), &provider.id)?;
    crate::settings::set_current_provider(&app_type, Some(provider.id.as_str()))?;

    Ok(true) // 真正导入了
}

pub fn should_import_default_config_on_startup(
    state: &AppState,
    app_type: &AppType,
) -> Result<bool, AppError> {
    if app_type.is_additive_mode() {
        return Ok(false);
    }

    Ok(!state.db.has_any_provider_for_app(app_type.as_str())?)
}
