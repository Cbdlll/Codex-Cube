use crate::config::{delete_file, write_text_file};
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::io::Write;
use std::path::{Path, PathBuf};
use toml_edit::{value, DocumentMut, Item, Table};

pub const SUBAGENT_MANIFEST_FILENAME: &str = "codex-cube-subagents.json";
pub const SUBAGENT_KEY_DIRNAME: &str = "codex-cube-agent-keys";
pub const SUBAGENT_AGENT_DIRNAME: &str = "agents";
pub const SUBAGENT_RUNTIME_PROVIDER_ID: &str = "custom";
pub const INHERIT_SANDBOX_MODE: &str = "inherit";
// 注册 subagent 的官方角色类型：worker（实现/执行）、explorer（只读探索）、default（通用）。
// 类型驱动自动生成的 description / developer_instructions，并在 Workflow Skill 中决定
// 派发路由与 fallback 的 agent_type。
pub const AGENT_TYPE_WORKER: &str = "worker";
pub const AGENT_TYPE_EXPLORER: &str = "explorer";
pub const AGENT_TYPE_DEFAULT: &str = "default";
pub const AGENT_TYPES: [&str; 3] = [AGENT_TYPE_WORKER, AGENT_TYPE_EXPLORER, AGENT_TYPE_DEFAULT];

/// 解析 agent 类型：空/未知回退 worker（与历史行为一致）。
pub fn resolve_agent_type(value: Option<&str>) -> String {
    let value = value.unwrap_or("").trim();
    if AGENT_TYPES.contains(&value) {
        value.to_owned()
    } else {
        AGENT_TYPE_WORKER.to_owned()
    }
}

/// 按类型生成 agent 的 description（未提供自定义描述时的自动预填）。
pub fn default_agent_description(agent_type: &str, model: &str) -> String {
    let label = match agent_type {
        AGENT_TYPE_EXPLORER => "explorer",
        AGENT_TYPE_DEFAULT => "general-purpose",
        _ => "worker",
    };
    let model = model.trim();
    if model.is_empty() {
        label.to_owned()
    } else {
        format!("{model} {label}")
    }
}

/// 按类型生成 agent 的 developer_instructions（写进 agent TOML）。
pub fn default_agent_instructions(agent_type: &str) -> &'static str {
    match agent_type {
        AGENT_TYPE_EXPLORER => "Explore the delegated scope read-only: inspect files, trace behavior, verify assumptions, and report concrete findings, evidence, and recommended changes. Do not modify files unless the task explicitly asks for a write.",
        AGENT_TYPE_DEFAULT => "Work on the delegated scope, edit the shared workspace directly, preserve unrelated changes, run focused verification, and report changed files, tests, and risks.",
        _ => "Implement only the delegated scope, edit the shared workspace directly, preserve unrelated changes, run focused verification, and report changed files, tests, and risks.",
    }
}

// sandbox 固定 inherit：UI 不可编辑，upsert 强制覆盖为 inherit，不信任前端/旧 payload 传入的值。
// 读取旧 agent 时仍兼容其历史 sandbox_mode 字符串（仅展示，不再生效）。
pub const SUBAGENT_SANDBOX_MODES: &[&str] = &[INHERIT_SANDBOX_MODE];
// 固定官方推理档位（6 档，无 minimal）：low | medium | high | xhigh | max | ultra，新建默认 high。
pub const SUBAGENT_REASONING_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max", "ultra"];
/// 合法 wire_api 集合（沿用 codebase 已支持的 responses / chat / anthropic 变体）。
/// 自定义 subagent 只支持 Responses 协议：Codex 协作子代理的任务派发
/// （spawn_agent / send_message / followup_task 明文处理）只在 native Responses
/// 路径实现；Chat Completions / Anthropic 转换路径不支持，worker 会收不到任务。
/// chat / anthropic 仅保留用于读取历史 agent（upsert 时会被拒绝）。
pub const SUBAGENT_WIRE_APIS: &[&str] = &[
    "responses",
    "openai_responses",
    "openai-responses",
    "chat",
    "chat_completions",
    "chat-completions",
    "openai_chat",
    "openai-chat",
    "openai_chat_completions",
    "anthropic",
    "anthropic_messages",
    "anthropic-messages",
    "claude",
    "messages",
];

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SubagentManifest {
    pub version: u32,
    #[serde(default)]
    pub agents: Vec<ManagedSubagent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedSubagent {
    pub name: String,
    pub provider_id: String,
    pub key_path: Option<String>,
    #[serde(default)]
    pub provider_managed: bool,
    #[serde(default)]
    pub original_provider: Option<JsonValue>,
    pub original_agent_toml: Option<String>,
    #[serde(default)]
    pub original_key: Option<Vec<u8>>,
    pub created_at: String,
    /// 注册角色类型：worker | explorer | default；旧记录为空串，读取时回退 worker。
    #[serde(default)]
    pub agent_type: String,
    /// Cube SQLite 供应商 ID。manifest `providerId` 只是 key 文件命名空间，
    /// 不能当作 `providers.id` 去查库；派发路由必须用这个字段。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cube_provider_id: Option<String>,
    /// 该 Cube 供应商是否由注册 subagent 时创建（删除 subagent 时可一并删）。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cube_provider_owned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentRecord {
    pub managed: bool,
    pub available: bool,
    pub name: String,
    /// 自定义 subagent 文件（`~/.codex/agents/<name>.toml`）的完整路径。
    pub agent_path: String,
    pub description: String,
    pub model: String,
    pub model_provider_id: String,
    pub model_base_url: String,
    pub api_key: Option<String>,
    pub sandbox_mode: String,
    pub reasoning_effort: String,
    pub wire_api: String,
    /// 注册角色类型（已解析，worker/explorer/default）。
    pub agent_type: String,
    /// 已绑定的 Cube 供应商 ID；未绑定或已失效时为 None。
    pub cube_provider_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentUpsertPayload {
    pub name: String,
    pub description: String,
    pub model: String,
    pub model_provider_id: String,
    pub model_base_url: String,
    #[serde(default)]
    pub api_key: Option<String>,
    pub sandbox_mode: String,
    pub reasoning_effort: String,
    /// 复用 Provider / 外部导入 / 编辑时携带的协议；旧 payload 未提供时后端默认 responses。
    #[serde(default)]
    pub wire_api: Option<String>,
    /// 注册角色类型：worker | explorer | default；未提供时按旧行为默认 worker。
    #[serde(default)]
    pub agent_type: Option<String>,
    /// UI 复用已有 Cube 供应商时传入其真实 `providers.id`。
    /// 不要把 agent 名推导出的 slug 写到这里。
    #[serde(default)]
    pub cube_provider_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentModelsFetchPayload {
    pub model_provider_id: String,
    pub model_base_url: String,
    pub api_key: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentModelCandidate {
    pub model: String,
    pub display_name: Option<String>,
}

fn config_dir() -> PathBuf {
    crate::codex_config::get_codex_config_dir()
}

pub fn manifest_path() -> PathBuf {
    manifest_path_in(&config_dir())
}

pub fn key_dir() -> PathBuf {
    key_dir_in(&config_dir())
}

pub fn agent_dir() -> PathBuf {
    agent_dir_in(&config_dir())
}

fn manifest_path_in(config_dir: &Path) -> PathBuf {
    config_dir.join(SUBAGENT_MANIFEST_FILENAME)
}

fn key_dir_in(config_dir: &Path) -> PathBuf {
    config_dir.join(SUBAGENT_KEY_DIRNAME)
}

fn agent_dir_in(config_dir: &Path) -> PathBuf {
    config_dir.join(SUBAGENT_AGENT_DIRNAME)
}

pub(crate) fn manifest_file_in(config_dir: &Path) -> Result<SubagentManifest, AppError> {
    let path = manifest_path_in(config_dir);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(SubagentManifest {
                version: 1,
                agents: Vec::new(),
            });
        }
        Err(error) => return Err(AppError::io(&path, error)),
    };
    let manifest = serde_json::from_str::<SubagentManifest>(&text)
        .map_err(|error| AppError::json(&path, error))?;
    if manifest.version != 1 {
        return Err(AppError::InvalidInput(format!(
            "不支持的 subagent manifest 版本 {}",
            manifest.version
        )));
    }
    Ok(manifest)
}

pub fn save_manifest(manifest: &SubagentManifest) -> Result<(), AppError> {
    save_manifest_in(&config_dir(), manifest)
}

pub(crate) fn save_manifest_in(
    config_dir: &Path,
    manifest: &SubagentManifest,
) -> Result<(), AppError> {
    // 测试故障注入：目录中存在标记文件时模拟 manifest 写入失败（仅测试构建生效）。
    #[cfg(test)]
    if config_dir.join(".fail-manifest-write").exists() {
        return Err(AppError::Message(
            "测试故障注入: manifest 写入失败".to_string(),
        ));
    }
    let path = manifest_path_in(config_dir);
    write_private_json(&path, manifest)
}

/// 以私密权限原子写入文件：Unix 下临时文件用 0600 模式创建（同目录），
/// 写入并 flush/sync 后原子 rename，避免“先默认权限创建再 chmod”的暴露窗口。
/// Windows 下与 config::atomic_write 行为一致（ReplaceFileW + rename 回退）。
pub(crate) fn write_private_file_atomic(path: &Path, data: &[u8]) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| AppError::io(parent, error))?;
    }
    let parent = path
        .parent()
        .ok_or_else(|| AppError::Config("无效的路径".to_string()))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| AppError::Config("无效的文件名".to_string()))?
        .to_string_lossy()
        .to_string();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    static TEMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let (tmp, mut file) = (|| -> Result<(PathBuf, std::fs::File), AppError> {
        let mut last_collision = None;
        for _ in 0..16 {
            let counter = TEMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let candidate = parent.join(format!(
                "{file_name}.tmp.{}.{ts}.{counter}",
                std::process::id()
            ));
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&candidate) {
                Ok(file) => return Ok((candidate, file)),
                Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
                    last_collision = Some((candidate, source));
                }
                Err(source) => return Err(AppError::io(&candidate, source)),
            }
        }

        let (candidate, source) = last_collision.expect("temporary filename loop must run");
        Err(AppError::io(&candidate, source))
    })()?;

    if let Err(source) = file
        .write_all(data)
        .and_then(|_| file.flush())
        .and_then(|_| file.sync_all())
    {
        drop(file);
        let _ = std::fs::remove_file(&tmp);
        return Err(AppError::io(&tmp, source));
    }
    drop(file);

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::{
            Foundation::ERROR_NOT_SUPPORTED, Storage::FileSystem::ReplaceFileW,
        };

        let replaced: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let replacement: Vec<u16> = tmp
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut completed = false;
        let mut last_error = None;

        for _ in 0..3 {
            // SAFETY: both path buffers are NUL-terminated UTF-16 and remain alive for the
            // duration of the call. Backup, exclusion, and reserved pointers are intentionally null.
            let replaced_ok = unsafe {
                ReplaceFileW(
                    replaced.as_ptr(),
                    replacement.as_ptr(),
                    std::ptr::null(),
                    0,
                    std::ptr::null(),
                    std::ptr::null(),
                )
            };
            if replaced_ok != 0 {
                completed = true;
                break;
            }

            let replace_error = std::io::Error::last_os_error();
            // WSL UNC paths reject ReplaceFileW with ERROR_NOT_SUPPORTED (50).
            // std::fs::rename uses a different replace-existing API on Windows.
            let replace_not_supported =
                replace_error.raw_os_error() == Some(ERROR_NOT_SUPPORTED as i32);
            if replace_error.kind() != std::io::ErrorKind::NotFound && !replace_not_supported {
                last_error = Some(replace_error);
                break;
            }

            match std::fs::rename(&tmp, path) {
                Ok(()) => {
                    completed = true;
                    break;
                }
                Err(source)
                    if matches!(
                        source.kind(),
                        std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::PermissionDenied
                    ) =>
                {
                    last_error = Some(source);
                }
                Err(source) => {
                    last_error = Some(source);
                    break;
                }
            }
        }

        if !completed {
            let source = last_error.unwrap_or_else(std::io::Error::last_os_error);
            let _ = std::fs::remove_file(&tmp);
            return Err(AppError::IoContext {
                context: format!("原子替换失败: {} -> {}", tmp.display(), path.display()),
                source,
            });
        }
    }

    #[cfg(not(windows))]
    {
        if let Err(source) = std::fs::rename(&tmp, path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(AppError::IoContext {
                context: format!("原子替换失败: {} -> {}", tmp.display(), path.display()),
                source,
            });
        }
    }
    Ok(())
}

fn write_private_json<T: Serialize>(path: &Path, data: &T) -> Result<(), AppError> {
    let bytes =
        serde_json::to_vec(data).map_err(|error| AppError::JsonSerialize { source: error })?;
    write_private_file_atomic(path, &bytes)
}

pub fn validate_provider_id(id: &str) -> Result<(), AppError> {
    let trimmed = id.trim();
    if trimmed.is_empty()
        || trimmed.len() > 64
        || !trimmed.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && (byte == b'_' || byte == b'-'))
        })
        || !crate::codex_config::is_custom_codex_model_provider_id(trimmed)
    {
        return Err(AppError::InvalidInput(format!(
            "model_provider {id:?} 不是可用的自定义 Provider ID"
        )));
    }
    Ok(())
}

pub fn validate_subagent_name(name: &str) -> Result<(), AppError> {
    crate::codex_agent_workflow::validate_agent_name(name)
}

pub fn validate_url(url: &str) -> Result<(), AppError> {
    let parsed = url::Url::parse(url.trim())
        .map_err(|_| AppError::InvalidInput("base URL 不是有效 URL".into()))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(AppError::InvalidInput(
            "base URL 必须是 http 或 https URL".into(),
        ));
    }
    Ok(())
}

pub fn validate_payload(payload: &SubagentUpsertPayload) -> Result<(), AppError> {
    validate_subagent_name(&payload.name)?;
    validate_provider_id(&payload.model_provider_id)?;
    // description 可选：为空时后端按 agent_type 自动生成（"<model> worker/explorer/general-purpose"）。
    if let Some(agent_type) = payload.agent_type.as_deref() {
        if !AGENT_TYPES.contains(&agent_type.trim()) {
            return Err(AppError::InvalidInput(format!(
                "agent_type {:?} 不支持（worker | explorer | default）",
                agent_type
            )));
        }
    }
    if payload.model.trim().is_empty() {
        return Err(AppError::InvalidInput("model 不能为空".into()));
    }
    validate_url(&payload.model_base_url)?;
    if !SUBAGENT_SANDBOX_MODES.contains(&payload.sandbox_mode.trim()) {
        return Err(AppError::InvalidInput(format!(
            "sandbox_mode {:?} 不支持",
            payload.sandbox_mode
        )));
    }
    if !SUBAGENT_REASONING_EFFORTS.contains(&payload.reasoning_effort.trim()) {
        return Err(AppError::InvalidInput(format!(
            "reasoning effort {:?} 不支持",
            payload.reasoning_effort
        )));
    }
    if let Some(wire_api) = payload.wire_api.as_deref() {
        let wire_api = wire_api.trim();
        let legal = SUBAGENT_WIRE_APIS
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(wire_api));
        if !legal {
            return Err(AppError::InvalidInput(format!(
                "wire_api {:?} 不支持",
                wire_api
            )));
        }
        // 自定义 subagent 只支持 Responses：协作子代理任务派发依赖 native
        // Responses 路径的明文处理，Chat/Anthropic 转换路径下 worker 收不到任务。
        let normalized = crate::proxy::providers::normalize_codex_wire_api(wire_api);
        if normalized != Some("responses") {
            return Err(AppError::InvalidInput(format!(
                "自定义 subagent 仅支持 Responses 协议；wire_api {:?}（Chat Completions / Anthropic）不受支持，Codex 协作子代理任务派发依赖 Responses 明文处理",
                wire_api
            )));
        }
    }
    Ok(())
}

/// 为新建 subagent 生成不与现有 manifest 冲突的唯一 name 与内部 provider id。
/// name 冲突时追加另一字段（内部 provider id）后缀；provider id 冲突时追加
/// name 后缀；仍冲突则追加递增序号。保证 agent 文件 / Workflow Skill /
/// manifest 三处使用同一 name，且同模型不同供应商的 subagent 可共存。
fn unique_subagent_identity(
    manifest: &SubagentManifest,
    base_name: &str,
    base_provider: &str,
) -> (String, String) {
    let name_taken = |name: &str| manifest.agents.iter().any(|record| record.name == name);
    let provider_taken = |id: &str| {
        manifest
            .agents
            .iter()
            .any(|record| record.provider_id == id)
    };
    if !name_taken(base_name) && !provider_taken(base_provider) {
        return (base_name.to_owned(), base_provider.to_owned());
    }
    let mut name = base_name.to_owned();
    if name_taken(&name) {
        name = suffixed_identity(base_name, base_provider, 0);
        let mut n = 2u32;
        while name_taken(&name) {
            name = suffixed_identity(base_name, base_provider, n);
            n += 1;
        }
    }
    let mut provider = base_provider.to_owned();
    if provider_taken(&provider) {
        provider = suffixed_identity(base_provider, &name, 0);
        let mut n = 2u32;
        while provider_taken(&provider) {
            provider = suffixed_identity(base_provider, &name, n);
            n += 1;
        }
    }
    (name, provider)
}

/// `<base>-<suffix>`（counter 为 0 时不带序号），截断 base 保证总长 ≤ 64。
fn suffixed_identity(base: &str, suffix: &str, counter: u32) -> String {
    let suffix = if counter == 0 {
        suffix.to_owned()
    } else {
        format!("{suffix}-{counter}")
    };
    let max_base = 64usize.saturating_sub(suffix.len() + 1).max(1);
    let base = if base.len() > max_base {
        &base[..max_base]
    } else {
        base
    };
    format!("{base}-{suffix}")
}

fn normalized_cube_provider_id(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
}

fn agent_path(name: &str) -> PathBuf {
    agent_path_in(&config_dir(), name)
}

fn provider_key_path(provider_id: &str) -> PathBuf {
    provider_key_path_in(&config_dir(), provider_id)
}

pub(crate) fn agent_path_in(config_dir: &Path, name: &str) -> PathBuf {
    agent_dir_in(config_dir).join(format!("{}.toml", name.trim()))
}

pub(crate) fn provider_key_path_in(config_dir: &Path, provider_id: &str) -> PathBuf {
    key_dir_in(config_dir).join(format!("{}.key", provider_id.trim()))
}

/// 校验 manifest 中记录的 key_path 必须等于当前 config dir 下该 provider 的规范 key 路径，
/// 防止被篡改的 manifest 指向任意文件。非法路径安全报错，绝不接触其指向的文件。
fn validate_manifest_key_path(
    config_dir: &Path,
    provider_id: &str,
    key_path: Option<&str>,
) -> Result<(), AppError> {
    // 先校验 provider_id 本身：防止恶意 manifest 用 "../outside" 之类把
    // 期望路径 join 到 key 目录之外，再伪造 key_path 通过相等判断。
    validate_provider_id(provider_id)?;
    if let Some(path) = key_path {
        let canonical = provider_key_path_in(config_dir, provider_id);
        if Path::new(path) != canonical.as_path() {
            return Err(AppError::InvalidInput(format!(
                "subagent manifest 中的 key_path 非法: {}（应为 {}）",
                path,
                canonical.display()
            )));
        }
    }
    Ok(())
}

/// 读取可选文本文件：NotFound 视为"文件不存在"返回 None，
/// 权限/目录等其他 IO 错误原样返回，避免把不可读文件当作空快照。
fn read_optional_text(path: &Path) -> Result<Option<String>, AppError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(AppError::io(path, error)),
    }
}

#[cfg(test)]
mod embedded_agent_tests {
    use super::*;

    fn payload(name: &str, api_key: &str) -> SubagentUpsertPayload {
        SubagentUpsertPayload {
            name: name.to_owned(),
            description: "test worker".to_owned(),
            model: "deepseek-v4-flash".to_owned(),
            model_provider_id: "deepseek".to_owned(),
            model_base_url: "https://opencode.ai/zen/go/v1".to_owned(),
            api_key: Some(api_key.to_owned()),
            sandbox_mode: "read-only".to_owned(),
            reasoning_effort: "high".to_owned(),
            wire_api: None,
            agent_type: None,
            cube_provider_id: None,
        }
    }

    fn config_path(config_dir: &Path) -> PathBuf {
        config_dir.join("config.toml")
    }

    #[test]
    fn upsert_writes_embedded_provider_agent_toml() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let p = payload("flash-worker", "sk-test");
        upsert_subagent_in(dir, &config_path(dir), "", &p).unwrap();

        let agent = std::fs::read_to_string(agent_path_in(dir, "flash-worker")).unwrap();
        let doc = agent.parse::<toml::Value>().unwrap();
        // 新风格：provider 内嵌在 agent TOML，model_provider 固定为 custom。
        assert_eq!(
            doc.get("model_provider").and_then(toml::Value::as_str),
            Some("custom")
        );
        let embedded = &doc["model_providers"]["custom"];
        assert_eq!(
            embedded.get("base_url").and_then(toml::Value::as_str),
            Some("https://opencode.ai/zen/go/v1")
        );
        assert_eq!(
            embedded.get("wire_api").and_then(toml::Value::as_str),
            Some("responses")
        );
        assert!(embedded.get("auth").is_some());
        assert_eq!(
            embedded
                .get("requires_openai_auth")
                .and_then(toml::Value::as_bool),
            Some(false)
        );

        // 全局 config 不写入 subagent 段（upsert 不创建/修改 config 文件）。
        assert!(!config_path(dir).exists());
    }

    #[test]
    fn embedded_agent_is_available_without_global_provider_section() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let p = payload("flash-worker", "sk-test");
        upsert_subagent_in(dir, &config_path(dir), "", &p).unwrap();

        // 会话配置里只有主 provider 段、没有任何 subagent 全局段。
        let session_config = r#"model_provider = "custom"

[model_providers.custom]
name = "Main provider"
base_url = "https://main.example/v1"
wire_api = "responses"
"#;
        let records = list_subagents_in(dir, session_config).unwrap();
        assert_eq!(records.len(), 1);
        assert!(records[0].managed);
        assert!(records[0].available);
        assert_eq!(
            records[0].agent_path,
            dir.join("agents")
                .join("flash-worker.toml")
                .to_string_lossy()
                .into_owned()
        );
        assert_eq!(records[0].model_base_url, "https://opencode.ai/zen/go/v1");
        assert_eq!(records[0].wire_api, "responses");

        // preserve 钩子保持 no-op：不把 subagent 段合并回全局配置。
        let next = r#"model_provider = "custom"

[model_providers.custom]
name = "Next provider"
base_url = "https://next.example/v1"
wire_api = "responses"
"#;
        let merged =
            preserve_managed_providers_for_live_write_in(dir, session_config, next).unwrap();
        assert_eq!(merged, next);
        assert!(!merged.contains("opencode.ai"));
    }
}

fn read_optional_bytes(path: &Path) -> Result<Option<Vec<u8>>, AppError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(AppError::io(path, error)),
    }
}

fn read_manifest_record<'a>(
    manifest: &'a SubagentManifest,
    name: &str,
) -> Option<&'a ManagedSubagent> {
    manifest
        .agents
        .iter()
        .find(|record| record.name == name.trim())
}

fn read_manifest_record_mut<'a>(
    manifest: &'a mut SubagentManifest,
    name: &str,
) -> Option<&'a mut ManagedSubagent> {
    manifest
        .agents
        .iter_mut()
        .find(|record| record.name == name.trim())
}

fn provider_command(provider_id: &str) -> (String, Vec<String>, Option<PathBuf>) {
    provider_command_in(&config_dir(), provider_id)
}

fn provider_command_in(
    config_dir: &Path,
    provider_id: &str,
) -> (String, Vec<String>, Option<PathBuf>) {
    let key_path = provider_key_path_in(config_dir, provider_id);
    #[cfg(unix)]
    {
        (
            "/bin/cat".to_string(),
            vec![key_path.to_string_lossy().into_owned()],
            Some(key_path),
        )
    }
    #[cfg(windows)]
    {
        (
            "powershell.exe".to_string(),
            vec![
                "-NoLogo".into(),
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-Command".into(),
                "$value = Get-Content -Raw -LiteralPath $args[0]; [Console]::Out.Write($value.Trim())".into(),
                key_path.to_string_lossy().into_owned(),
            ],
            Some(key_path),
        )
    }
}

fn provider_table(payload: &SubagentUpsertPayload, command: &str, args: &[String]) -> Table {
    let mut provider = Table::new();
    provider["name"] = value(format!("Codex Cube subagent: {}", payload.name.trim()));
    provider["base_url"] = value(payload.model_base_url.trim_end_matches('/'));
    // wire_api 归一化为规范值（responses | chat | anthropic）后写入 Codex config，
    // 绝不原样写 openai_responses / chat_completions 等内部别名。
    let wire_api = payload
        .wire_api
        .as_deref()
        .map(str::trim)
        .and_then(crate::proxy::providers::normalize_codex_wire_api)
        .unwrap_or("responses");
    provider["wire_api"] = value(wire_api);
    // Agent files are configuration layers over the parent session. Explicitly
    // clear an inherited global custom provider's OpenAI login requirement so
    // it cannot conflict with this agent's auth.command.
    provider["requires_openai_auth"] = value(false);
    let mut auth = Table::new();
    auth["command"] = value(command);
    let mut array = toml_edit::Array::new();
    for arg in args {
        array.push(arg.as_str());
    }
    auth["args"] = Item::Value(toml_edit::Value::Array(array));
    auth["timeout_ms"] = value(5000i64);
    auth["refresh_interval_ms"] = value(300000i64);
    provider["auth"] = Item::Table(auth);
    provider
}

fn render_agent_toml_with_provider(
    payload: &SubagentUpsertPayload,
    command: &str,
    args: &[String],
) -> Result<String, AppError> {
    let agent_type = resolve_agent_type(payload.agent_type.as_deref());
    let mut doc = DocumentMut::new();
    doc["name"] = value(payload.name.trim());
    let description = if payload.description.trim().is_empty() {
        default_agent_description(&agent_type, &payload.model)
    } else {
        payload.description.trim().to_owned()
    };
    doc["description"] = value(description);
    doc["developer_instructions"] = value(default_agent_instructions(&agent_type));
    doc["model"] = value(payload.model.trim());
    doc["model_provider"] = value(SUBAGENT_RUNTIME_PROVIDER_ID);
    doc["model_reasoning_effort"] = value(payload.reasoning_effort.trim());
    let mut providers = Table::new();
    providers.set_implicit(true);
    providers.insert(
        SUBAGENT_RUNTIME_PROVIDER_ID,
        Item::Table(provider_table(payload, command, args)),
    );
    doc["model_providers"] = Item::Table(providers);
    // sandbox 固定 inherit：Codex 缺省即继承父会话权限，不写入 sandbox_mode。
    // 直接调用本函数时同样强制 inherit，不信任 payload 中的旧值。
    Ok(doc.to_string())
}

pub fn render_agent_toml(payload: &SubagentUpsertPayload) -> Result<String, AppError> {
    let (command, args, _) = provider_command(&payload.model_provider_id);
    render_agent_toml_with_provider(payload, &command, &args)
}

pub fn list_subagents() -> Result<Vec<SubagentRecord>, AppError> {
    let config_text = crate::codex_config::read_and_validate_codex_config_text()?;
    list_subagents_in(&config_dir(), &config_text)
}

fn list_subagents_in(
    config_dir: &Path,
    config_text: &str,
) -> Result<Vec<SubagentRecord>, AppError> {
    let config = config_text
        .parse::<DocumentMut>()
        .map_err(|error| AppError::Message(error.to_string()))?;
    let manifest = manifest_file_in(config_dir)?;
    let mut records = Vec::new();
    let dir = agent_dir_in(config_dir);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(records),
        Err(error) => return Err(AppError::io(&dir, error)),
    };
    for entry in entries {
        let entry = entry.map_err(|error| AppError::io(&dir, error))?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("toml") {
            continue;
        }
        let text = std::fs::read_to_string(&path).map_err(|error| AppError::io(&path, error))?;
        let agent = text.parse::<toml::Value>().map_err(|error| {
            AppError::Message(format!("解析 agent TOML 失败 {}: {error}", path.display()))
        })?;
        let Some(name) = agent
            .get("name")
            .and_then(toml::Value::as_str)
            .map(str::to_owned)
        else {
            continue;
        };
        let manifest_record = read_manifest_record(&manifest, &name);
        // 受管项的 provider_id 以 manifest 记录为权威（UI Provider ID 漂移的根因之一：
        // 若从 agent TOML 读 model_provider，手工/旧编辑导致的漂移会回显到 UI）。
        // 只有 unmanaged 项才使用 TOML 的 model_provider。
        let provider_id = match manifest_record {
            Some(record) => {
                validate_manifest_key_path(
                    config_dir,
                    &record.provider_id,
                    record.key_path.as_deref(),
                )?;
                record.provider_id.trim().to_owned()
            }
            None => agent
                .get("model_provider")
                .and_then(toml::Value::as_str)
                .unwrap_or("")
                .to_owned(),
        };
        // New Cube-managed agents always carry their own runtime provider layer:
        // `model_provider = "custom"` + `[model_providers.custom]`. Older agents
        // used a manifest provider ID in the agent TOML and a matching global
        // config section, so retain that as a read-only compatibility fallback.
        let embedded_provider = agent
            .get("model_providers")
            .and_then(|providers| providers.get(SUBAGENT_RUNTIME_PROVIDER_ID));
        let global_provider = config
            .get("model_providers")
            .and_then(Item::as_table_like)
            .and_then(|providers| providers.get(&provider_id));
        let available = embedded_provider.is_some() || global_provider.is_some();
        let base_url = embedded_provider
            .and_then(|item| item.get("base_url"))
            .and_then(toml::Value::as_str)
            .or_else(|| {
                global_provider
                    .and_then(Item::as_table_like)
                    .and_then(|provider| provider.get("base_url"))
                    .and_then(Item::as_str)
            })
            .map(str::to_owned);
        let wire_api = embedded_provider
            .and_then(|item| item.get("wire_api"))
            .and_then(toml::Value::as_str)
            .or_else(|| {
                global_provider
                    .and_then(Item::as_table_like)
                    .and_then(|provider| provider.get("wire_api"))
                    .and_then(Item::as_str)
            })
            .unwrap_or("responses")
            .to_owned();
        let key_path = manifest_record.and_then(|record| record.key_path.as_deref());
        let api_key_configured = key_path.map(Path::new).is_some_and(|path| path.exists())
            || embedded_provider
                .and_then(|item| item.get("env_key"))
                .is_some()
            || embedded_provider
                .and_then(|item| item.get("auth"))
                .is_some()
            || global_provider
                .and_then(Item::as_table_like)
                .and_then(|provider| provider.get("env_key"))
                .is_some()
            || global_provider
                .and_then(Item::as_table_like)
                .and_then(|provider| provider.get("auth"))
                .is_some();
        let agent_type = manifest_record
            .map(|record| resolve_agent_type(Some(record.agent_type.as_str())))
            .unwrap_or_else(|| AGENT_TYPE_WORKER.to_owned());
        records.push(SubagentRecord {
            managed: manifest_record.is_some(),
            available,
            name,
            agent_path: path.to_string_lossy().into_owned(),
            description: agent
                .get("description")
                .and_then(toml::Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            model: agent
                .get("model")
                .and_then(toml::Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            model_provider_id: provider_id,
            model_base_url: base_url.unwrap_or_default(),
            api_key: api_key_configured.then(String::new),
            sandbox_mode: agent
                .get("sandbox_mode")
                .and_then(toml::Value::as_str)
                .unwrap_or(INHERIT_SANDBOX_MODE)
                .to_owned(),
            reasoning_effort: agent
                .get("model_reasoning_effort")
                .and_then(toml::Value::as_str)
                .unwrap_or("medium")
                .to_owned(),
            wire_api,
            agent_type,
            cube_provider_id: manifest_record
                .and_then(|record| record.cube_provider_id.clone())
                .map(|id| id.trim().to_owned())
                .filter(|id| !id.is_empty()),
        });
    }
    records.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(records)
}

/// Subagent providers are scoped to each standalone agent TOML. Keep this
/// compatibility hook for callers that write the live config, but never copy
/// a managed subagent provider back into the global config.
pub fn preserve_managed_providers_for_live_write(
    current_config: &str,
    next_config: &str,
) -> Result<String, AppError> {
    preserve_managed_providers_for_live_write_in(&config_dir(), current_config, next_config)
}

fn preserve_managed_providers_for_live_write_in(
    _config_dir: &Path,
    current_config: &str,
    next_config: &str,
) -> Result<String, AppError> {
    let _ = current_config;
    Ok(next_config.to_owned())
}

pub fn upsert_subagent(payload: &SubagentUpsertPayload) -> Result<SubagentRecord, AppError> {
    let config_path = crate::codex_config::get_codex_config_path();
    let original_config = crate::codex_config::read_codex_config_text()?;
    upsert_subagent_in(&config_dir(), &config_path, &original_config, payload)
}

fn upsert_subagent_in(
    config_dir: &Path,
    _config_path: &Path,
    original_config: &str,
    payload: &SubagentUpsertPayload,
) -> Result<SubagentRecord, AppError> {
    // sandbox 固定 inherit：无视前端/旧 payload 传入的值，强制覆盖。
    // wire_api 只做 trim / 默认值；规范别名归一化由 provider_table 写入时完成。
    let mut payload = payload.clone();
    payload.sandbox_mode = INHERIT_SANDBOX_MODE.to_owned();
    payload.wire_api = Some(
        payload
            .wire_api
            .as_deref()
            .unwrap_or("responses")
            .trim()
            .to_owned(),
    );
    let agent_type = resolve_agent_type(payload.agent_type.as_deref());
    // 任何路径构造/文件读取前必须先校验名称，防止 "../outside" 之类非法 name
    // 触碰 manifest/key 目录之外的文件。随后用 manifest provider_id 规范化，
    // 再调用完整 validate_payload。
    validate_subagent_name(&payload.name)?;
    let manifest_path = manifest_path_in(config_dir);
    let original_manifest = read_optional_text(&manifest_path)?;
    let mut manifest = manifest_file_in(config_dir)?;
    let existing_by_name = manifest
        .agents
        .iter()
        .find(|record| record.name == payload.name.trim())
        .cloned();
    // 编辑判定：同名且同内部 provider id 才是编辑；同名但 provider 不同视为
    // 新注册（同模型不同供应商的重复场景），后端追加其他字段后缀生成唯一
    // name/provider，避免第二个覆盖第一个的 agent 文件与 manifest 记录。
    let is_edit = existing_by_name
        .as_ref()
        .is_some_and(|record| record.provider_id == payload.model_provider_id.trim());
    // 编辑受管 subagent 时，manifest 中的 provider_id 是权威：忽略 payload 漂移
    // （前端/旧 payload 携带的 modelProviderId），强制使用稳定 ID 并写回。
    if is_edit {
        if let Some(record) = existing_by_name.as_ref() {
            validate_manifest_key_path(
                config_dir,
                &record.provider_id,
                record.key_path.as_deref(),
            )?;
            payload.model_provider_id = record.provider_id.trim().to_owned();
        }
    } else {
        // 新建：name / 内部 provider id 被现有记录占用时追加后缀去重。
        let (final_name, final_provider) = unique_subagent_identity(
            &manifest,
            payload.name.trim(),
            payload.model_provider_id.trim(),
        );
        payload.name = final_name;
        payload.model_provider_id = final_provider;
    }
    let agent_path = agent_path_in(config_dir, &payload.name);
    let original_agent = read_optional_text(&agent_path)?;
    let old_record = if is_edit { existing_by_name } else { None };
    // 编辑且未携带自定义描述：保留现有 TOML 描述；若描述仍是旧类型的自动生成文案
    // 且类型已变更，则跟随新类型重新生成，避免类型切换后描述与角色不一致。
    if is_edit && payload.description.trim().is_empty() {
        let existing_description = match original_agent
            .as_deref()
            .and_then(|text| text.parse::<toml::Value>().ok())
        {
            Some(value) => value
                .get("description")
                .and_then(toml::Value::as_str)
                .unwrap_or("")
                .to_owned(),
            None => String::new(),
        };
        let old_type = old_record
            .as_ref()
            .map(|record| resolve_agent_type(Some(record.agent_type.as_str())))
            .unwrap_or_else(|| AGENT_TYPE_WORKER.to_owned());
        if agent_type != old_type
            && existing_description == default_agent_description(&old_type, &payload.model)
        {
            payload.description = default_agent_description(&agent_type, &payload.model);
        } else {
            payload.description = existing_description;
        }
    }
    validate_payload(&payload)?;
    if manifest.agents.iter().any(|record| {
        record.provider_id == payload.model_provider_id.trim() && record.name != payload.name.trim()
    }) {
        return Err(AppError::InvalidInput(
            "Subagent 内部 Provider ID 已被占用".into(),
        ));
    }
    // The agent provider is always the runtime ID `custom`; the manifest ID is
    // only a Cube-owned namespace for key files and record ownership.
    let provider_managed = true;
    let (command, args, key_path) = provider_command_in(config_dir, &payload.model_provider_id);
    let original_key = match key_path.as_ref() {
        Some(path) => read_optional_bytes(path)?,
        None => None,
    };
    let key_existed = original_key.is_some();
    let new_key = payload
        .api_key
        .as_deref()
        .filter(|key| !key.trim().is_empty())
        .map(|key| key.trim().to_owned());
    if provider_managed && key_path.is_some() {
        ensure_key_dir_is_private_in(config_dir)?;
        if new_key.is_none() && old_record.is_none() && !key_existed {
            return Err(AppError::InvalidInput(
                "首次注册必须填写 API Key；已有 Provider 可留空".into(),
            ));
        }
    }
    let agent_text = render_agent_toml_with_provider(&payload, &command, &args)?;
    let original_record = old_record.unwrap_or(ManagedSubagent {
        name: payload.name.trim().to_owned(),
        provider_id: payload.model_provider_id.trim().to_owned(),
        key_path: key_path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
        provider_managed,
        original_provider: None,
        original_agent_toml: original_agent.clone(),
        original_key: original_key.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
        agent_type: agent_type.clone(),
        cube_provider_id: normalized_cube_provider_id(payload.cube_provider_id.as_deref()),
        cube_provider_owned: false,
    });
    if let Some(record) = read_manifest_record_mut(&mut manifest, &payload.name) {
        if !record.provider_managed && provider_managed {
            record.original_key = original_key.clone();
        }
        record.provider_id = payload.model_provider_id.trim().to_owned();
        record.key_path = key_path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned());
        record.provider_managed = provider_managed;
        record.agent_type = agent_type.clone();
        if let Some(cube_provider_id) =
            normalized_cube_provider_id(payload.cube_provider_id.as_deref())
        {
            record.cube_provider_id = Some(cube_provider_id);
        }
    } else {
        manifest.agents.push(original_record);
    }
    if let Err(error) = (|| -> Result<(), AppError> {
        write_text_file(&agent_path, &agent_text)?;
        if provider_managed {
            if let (Some(key_path), Some(key)) = (key_path.as_ref(), new_key.as_deref()) {
                write_private_file_atomic(key_path, key.as_bytes())?;
            }
        }
        save_manifest_in(config_dir, &manifest)
    })() {
        match original_agent {
            Some(text) => {
                let _ = write_text_file(&agent_path, &text);
            }
            None => {
                let _ = delete_file(&agent_path);
            }
        }
        match original_manifest {
            Some(text) => {
                let _ = write_text_file(&manifest_path, &text);
            }
            None => {
                let _ = delete_file(&manifest_path);
            }
        }
        if let Some(key_path) = key_path.as_ref() {
            if let Some(bytes) = original_key {
                let _ = write_private_file_atomic(key_path, &bytes);
            } else if !key_existed {
                let _ = delete_file(key_path);
            }
        }
        return Err(error);
    }
    list_subagents_in(config_dir, original_config)?
        .into_iter()
        .find(|record| record.name == payload.name.trim())
        .ok_or_else(|| AppError::Message("写入后无法读取 subagent".into()))
}

pub fn resolve_models_api_key(provider_id: &str, supplied: &str) -> Result<String, AppError> {
    resolve_models_api_key_in(&config_dir(), provider_id, supplied)
}

fn resolve_models_api_key_in(
    config_dir: &Path,
    provider_id: &str,
    supplied: &str,
) -> Result<String, AppError> {
    if !supplied.trim().is_empty() {
        return Ok(supplied.trim().to_owned());
    }
    let manifest = manifest_file_in(config_dir)?;
    let record = manifest
        .agents
        .iter()
        .find(|record| record.provider_id == provider_id.trim())
        .ok_or_else(|| AppError::InvalidInput("请填写 API Key 后获取模型列表".into()))?;
    validate_manifest_key_path(config_dir, &record.provider_id, record.key_path.as_deref())?;
    let key_path = record
        .key_path
        .as_deref()
        .ok_or_else(|| AppError::InvalidInput("请填写 API Key 后获取模型列表".into()))?;
    std::fs::read_to_string(key_path)
        .map(|key| key.trim().to_owned())
        .map_err(|error| AppError::io(Path::new(key_path), error))
}

pub fn delete_subagent(name: &str) -> Result<(), AppError> {
    validate_subagent_name(name)?;
    let config_path = crate::codex_config::get_codex_config_path();
    let original_config = crate::codex_config::read_and_validate_codex_config_text()?;
    delete_subagent_in(&config_dir(), &config_path, &original_config, name)
}

fn delete_subagent_in(
    config_dir: &Path,
    _config_path: &Path,
    _original_config: &str,
    name: &str,
) -> Result<(), AppError> {
    validate_subagent_name(name)?;
    let manifest_path = manifest_path_in(config_dir);
    let mut manifest = manifest_file_in(config_dir)?;
    let record = read_manifest_record(&manifest, name)
        .cloned()
        .ok_or_else(|| {
            AppError::InvalidInput(
                "该 subagent 不是 Codex Cube 管理的注册项；请先编辑并采用后再取消注册".into(),
            )
        })?;
    validate_manifest_key_path(config_dir, &record.provider_id, record.key_path.as_deref())?;
    let agent = agent_path_in(config_dir, name);
    if crate::codex_agent_workflow::workflow_uses_agent(config_dir, name)? {
        return Err(AppError::InvalidInput(
            "该 subagent 正被 Workflow Skill 选中；请先在 Workflow Skill 中取消选中后再删除".into(),
        ));
    }
    let original_agent = read_optional_text(&agent)?;
    let original_manifest = read_optional_text(&manifest_path)?;
    // key 仅属于 Cube 自建/接管的专用 provider：provider_managed=false（复用用户既有
    // provider）时即使异常 manifest 记录了规范 key_path 也不读取/不删除该 key。
    let key_path = if record.provider_managed {
        record.key_path.as_deref().map(PathBuf::from)
    } else {
        None
    };
    let current_key = match key_path.as_ref() {
        Some(path) => read_optional_bytes(path)?,
        None => None,
    };

    // 删除是永久的：不恢复首次注册前快照（original_provider / original_agent_toml /
    // original_key 不再回写）。只移除 Cube 自己创建/接管的专用 provider；复用用户既有
    // provider（provider_managed=false）时保留该配置不删。
    manifest.agents.retain(|record| record.name != name.trim());

    if let Err(error) = (|| -> Result<(), AppError> {
        // 永久删除受管 subagent 的 agent TOML。
        delete_file(&agent)?;
        // key 只删除 manifest 记录的规范路径（已在前面通过 validate_manifest_key_path 校验）。
        if let Some(key_path) = key_path.as_ref() {
            delete_file(key_path)?;
        }
        if manifest.agents.is_empty() {
            delete_file(&manifest_path)?;
        } else {
            save_manifest_in(config_dir, &manifest)?;
        }
        Ok(())
    })() {
        restore_optional_file(&agent, original_agent.as_deref()).ok();
        match (key_path.as_ref(), current_key.as_deref()) {
            (Some(path), Some(contents)) => {
                write_private_file_atomic(path, contents).ok();
            }
            (Some(path), None) => {
                delete_file(path).ok();
            }
            (None, _) => {}
        }
        restore_optional_file(&manifest_path, original_manifest.as_deref()).ok();
        return Err(error);
    }
    Ok(())
}

pub fn ensure_key_dir_is_private() -> Result<(), AppError> {
    ensure_key_dir_is_private_in(&config_dir())
}

fn ensure_key_dir_is_private_in(config_dir: &Path) -> Result<(), AppError> {
    let path = key_dir_in(config_dir);
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(&path)
        .map_err(|error| AppError::io(&path, error))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| AppError::io(&path, error))?;
    }
    Ok(())
}

fn restore_optional_file(path: &Path, contents: Option<&str>) -> Result<(), AppError> {
    match contents {
        Some(contents) => write_text_file(path, contents),
        None => delete_file(path),
    }
}

// Historical tests below describe the removed global-provider design. Keep
// them out of the active suite while the new scoped-custom tests exercise the
// supported contract.
#[cfg(any())]
mod tests {
    use super::*;

    fn test_payload(
        name: &str,
        api_key: Option<&str>,
        sandbox_mode: &str,
        reasoning_effort: &str,
    ) -> SubagentUpsertPayload {
        SubagentUpsertPayload {
            name: name.to_owned(),
            description: "test worker".to_owned(),
            model: "deepseek-v4-flash".to_owned(),
            model_provider_id: "deepseek".to_owned(),
            model_base_url: "https://api.deepseek.com".to_owned(),
            api_key: api_key.map(str::to_owned),
            sandbox_mode: sandbox_mode.to_owned(),
            reasoning_effort: reasoning_effort.to_owned(),
            wire_api: None,
            agent_type: None,
            cube_provider_id: None,
        }
    }

    fn config_path(config_dir: &Path) -> PathBuf {
        config_dir.join("config.toml")
    }

    #[test]
    fn live_write_preserves_managed_subagent_provider() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let payload = test_payload("flash-worker", Some("sk-test"), "inherit", "high");
        upsert_subagent_in(dir, &config_path(dir), "", &payload).unwrap();
        let current = std::fs::read_to_string(config_path(dir)).unwrap();

        let next = r#"model = "gpt-5.6-sol"
model_provider = "custom"

[model_providers.custom]
name = "Current provider"
base_url = "https://current.example/v1"
wire_api = "responses"
"#;
        let merged = preserve_managed_providers_for_live_write_in(dir, &current, next).unwrap();
        let doc = merged.parse::<toml::Value>().unwrap();

        assert_eq!(
            doc["model_providers"]["custom"]["base_url"].as_str(),
            Some("https://current.example/v1")
        );
        assert_eq!(
            doc["model_providers"]["deepseek"]["base_url"].as_str(),
            Some("https://api.deepseek.com")
        );
        assert!(doc["model_providers"]["deepseek"].get("auth").is_some());
    }

    #[test]
    fn live_write_does_not_recreate_missing_managed_provider() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let payload = test_payload("flash-worker", Some("sk-test"), "inherit", "high");
        upsert_subagent_in(dir, &config_path(dir), "", &payload).unwrap();

        let next = "model = \"gpt-5.6-sol\"\n";
        let merged = preserve_managed_providers_for_live_write_in(dir, next, next).unwrap();
        let doc = merged.parse::<toml::Value>().unwrap();
        assert!(doc.get("model_providers").is_none());

        let records = list_subagents_in(dir, next).unwrap();
        assert_eq!(records.len(), 1);
        assert!(records[0].managed);
        assert!(!records[0].available);
    }

    #[test]
    fn sandbox_mode_is_forced_to_inherit_even_from_old_payloads() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let first = test_payload("flash-worker", Some("sk-test"), "read-only", "high");
        upsert_subagent_in(dir, &config_path(dir), "", &first).unwrap();

        let agent = std::fs::read_to_string(agent_path_in(dir, "flash-worker")).unwrap();
        let doc = agent.parse::<toml::Value>().unwrap();
        // 旧 payload 传 read-only 也被强制 inherit：agent TOML 不写 sandbox_mode。
        assert!(doc.get("sandbox_mode").is_none());
        assert_eq!(
            doc.get("model_reasoning_effort")
                .and_then(toml::Value::as_str),
            Some("high")
        );
        assert_eq!(
            doc.get("model").and_then(toml::Value::as_str),
            Some("deepseek-v4-flash")
        );

        // 编辑时再传非 inherit 依旧被强制覆盖为 inherit。
        let current = std::fs::read_to_string(config_path(dir)).unwrap();
        let second = test_payload("flash-worker", None, "workspace-write", "xhigh");
        let record = upsert_subagent_in(dir, &config_path(dir), &current, &second).unwrap();
        assert_eq!(record.sandbox_mode, INHERIT_SANDBOX_MODE);
        let agent = std::fs::read_to_string(agent_path_in(dir, "flash-worker")).unwrap();
        let doc = agent.parse::<toml::Value>().unwrap();
        assert!(doc.get("sandbox_mode").is_none());

        // 渲染函数直接调用时同样强制 inherit（不信任 payload）。
        let rendered = render_agent_toml(&second).unwrap();
        let doc = rendered.parse::<toml::Value>().unwrap();
        assert!(doc.get("sandbox_mode").is_none());
    }

    #[test]
    fn editing_managed_subagent_ignores_payload_provider_drift_and_keeps_manifest_id() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();

        // 首次注册采用 stable provider id（deepseek）。
        let first = test_payload("flash-worker", Some("sk-stable"), "inherit", "high");
        let registered = upsert_subagent_in(dir, &config_path(dir), "", &first).unwrap();
        assert_eq!(registered.model_provider_id, "deepseek");

        // 编辑时 payload 漂移为另一个合法 provider id：后端必须以 manifest 记录为权威，
        // 忽略漂移并写回稳定 ID，而不是报错或改写 provider 归属。
        let current = std::fs::read_to_string(config_path(dir)).unwrap();
        let mut drifted = test_payload("flash-worker", None, "inherit", "xhigh");
        drifted.model = "gpt-5.6-sol".to_owned();
        drifted.model_provider_id = "drifted-provider".to_owned();
        let edited = upsert_subagent_in(dir, &config_path(dir), &current, &drifted).unwrap();
        assert_eq!(edited.model_provider_id, "deepseek");
        assert_eq!(edited.model, "gpt-5.6-sol");

        // config.toml 中 provider 仍为稳定 ID，漂移 ID 不产生任何配置。
        let config = std::fs::read_to_string(config_path(dir)).unwrap();
        let doc = config.parse::<toml::Value>().unwrap();
        let providers = doc.get("model_providers").unwrap();
        assert!(providers.get("deepseek").is_some());
        assert!(providers.get("drifted-provider").is_none());

        // agent TOML 的 model_provider 写回稳定 ID。
        let agent = std::fs::read_to_string(agent_path_in(dir, "flash-worker")).unwrap();
        let agent_doc = agent.parse::<toml::Value>().unwrap();
        assert_eq!(
            agent_doc
                .get("model_provider")
                .and_then(toml::Value::as_str),
            Some("deepseek")
        );

        // manifest 记录保持稳定 ID。
        let manifest = manifest_file_in(dir).unwrap();
        let record = read_manifest_record(&manifest, "flash-worker").unwrap();
        assert_eq!(record.provider_id, "deepseek");
        assert!(record.provider_managed);

        // 多次编辑持续漂移也不会累积或篡改稳定 ID。
        let current = std::fs::read_to_string(config_path(dir)).unwrap();
        let mut drifted_again = test_payload("flash-worker", None, "inherit", "max");
        drifted_again.model_provider_id = "another-drift".to_owned();
        upsert_subagent_in(dir, &config_path(dir), &current, &drifted_again).unwrap();
        let manifest = manifest_file_in(dir).unwrap();
        assert_eq!(
            read_manifest_record(&manifest, "flash-worker")
                .unwrap()
                .provider_id,
            "deepseek"
        );
    }

    #[test]
    fn upsert_with_invalid_name_fails_before_touching_any_file() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        // 目录外哨兵文件：若非法 name 逃过校验并被用于路径构造，会尝试触碰它。
        let outside = dir.parent().unwrap().join("sentinel-outside.toml");
        std::fs::write(&outside, "sentinel").unwrap();

        let mut payload = test_payload("../../sentinel-outside", Some("sk-x"), "inherit", "high");
        payload.model_provider_id = "deepseek".to_owned();
        let error = upsert_subagent_in(dir, &config_path(dir), "", &payload).unwrap_err();
        assert!(error.to_string().contains("不是安全文件名"));
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "sentinel");

        // 非法 name 不得创建 agent 文件、config 或 manifest。
        assert!(!agent_path_in(dir, "../../sentinel-outside").exists());
        assert!(!config_path(dir).exists());
        assert!(!manifest_path_in(dir).exists());
    }

    #[test]
    fn list_uses_manifest_provider_id_when_agent_toml_drifts_and_repairs_on_edit() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let first = test_payload("flash-worker", Some("sk-stable"), "inherit", "high");
        upsert_subagent_in(dir, &config_path(dir), "", &first).unwrap();

        // 手工把 agent TOML 的 model_provider 改为另一个合法 ID，并在 config 放诱饵 provider。
        let agent_path = agent_path_in(dir, "flash-worker");
        let drifted = std::fs::read_to_string(&agent_path).unwrap().replace(
            "model_provider = \"deepseek\"",
            "model_provider = \"decoy-provider\"",
        );
        std::fs::write(&agent_path, drifted).unwrap();
        let mut config = std::fs::read_to_string(config_path(dir)).unwrap();
        config.push_str(
            "\n[model_providers.decoy-provider]\nname = \"Decoy\"\nbase_url = \"https://decoy.example\"\nwire_api = \"responses\"\n",
        );
        std::fs::write(config_path(dir), &config).unwrap();

        // list 仍以 manifest 的 canonical ID 返回：provider、base_url、wire_api、key 全部一致。
        let records = list_subagents_in(dir, &config).unwrap();
        assert_eq!(records.len(), 1);
        assert!(records[0].managed);
        assert_eq!(records[0].model_provider_id, "deepseek");
        assert_eq!(records[0].model_base_url, "https://api.deepseek.com");
        assert_eq!(records[0].wire_api, "responses");
        assert_eq!(records[0].api_key, Some(String::new()));

        // 编辑保存（payload 即使继续漂移）后，agent TOML 被修复为 manifest ID。
        let current = std::fs::read_to_string(config_path(dir)).unwrap();
        let mut edit = test_payload("flash-worker", None, "inherit", "high");
        edit.model_provider_id = "decoy-provider".to_owned();
        upsert_subagent_in(dir, &config_path(dir), &current, &edit).unwrap();
        let agent = std::fs::read_to_string(agent_path).unwrap();
        let doc = agent.parse::<toml::Value>().unwrap();
        assert_eq!(
            doc.get("model_provider").and_then(toml::Value::as_str),
            Some("deepseek")
        );
        // config 中诱饵 provider 保留（用户既有配置不被删除），list 仍返回 canonical。
        let final_config = std::fs::read_to_string(config_path(dir)).unwrap();
        let records = list_subagents_in(dir, &final_config).unwrap();
        assert_eq!(records[0].model_provider_id, "deepseek");
        assert_eq!(records[0].model_base_url, "https://api.deepseek.com");
    }

    #[test]
    fn listing_old_agent_with_non_inherit_sandbox_stays_compatible() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        std::fs::create_dir_all(agent_dir_in(dir)).unwrap();
        // 旧版本注册的 agent：显式 sandbox_mode 与 minimal reasoning 均可正常读取。
        std::fs::write(
            agent_dir_in(dir).join("legacy.toml"),
            "name = \"legacy\"\nmodel = \"deepseek-v4-flash\"\nmodel_provider = \"deepseek\"\nsandbox_mode = \"read-only\"\nmodel_reasoning_effort = \"minimal\"\n",
        )
        .unwrap();
        let config = r#"
[model_providers.deepseek]
name = "My DeepSeek"
base_url = "https://api.deepseek.com"
wire_api = "responses"
"#;
        std::fs::write(config_path(dir), config).unwrap();
        let records = list_subagents_in(dir, config).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].sandbox_mode, "read-only");
        assert_eq!(records[0].reasoning_effort, "minimal");
        assert_eq!(records[0].wire_api, "responses");
        assert!(!records[0].managed);
    }

    #[test]
    fn wire_api_defaults_to_responses_for_old_payloads() {
        // 旧 payload 不携带 wire_api → 后端默认 responses，保持兼容。
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let payload = test_payload("flash-worker", Some("sk-test"), "inherit", "high");
        assert!(payload.wire_api.is_none());
        upsert_subagent_in(dir, &config_path(dir), "", &payload).unwrap();

        let config = std::fs::read_to_string(config_path(dir)).unwrap();
        let doc = config.parse::<toml::Value>().unwrap();
        assert_eq!(
            doc["model_providers"]["deepseek"]["wire_api"].as_str(),
            Some("responses")
        );
        // 返回记录同样默认 responses。
        let records = list_subagents_in(dir, &config).unwrap();
        assert_eq!(records[0].wire_api, "responses");
    }

    #[test]
    fn wire_api_is_preserved_when_reusing_chat_provider() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let mut payload = test_payload("flash-worker", Some("sk-test"), "inherit", "high");
        payload.wire_api = Some("chat".to_owned());
        upsert_subagent_in(dir, &config_path(dir), "", &payload).unwrap();

        let config = std::fs::read_to_string(config_path(dir)).unwrap();
        let doc = config.parse::<toml::Value>().unwrap();
        assert_eq!(
            doc["model_providers"]["deepseek"]["wire_api"].as_str(),
            Some("chat")
        );
    }

    #[test]
    fn list_subagents_reads_wire_api_from_provider_config() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        std::fs::create_dir_all(agent_dir_in(dir)).unwrap();
        std::fs::write(
            agent_dir_in(dir).join("chat-worker.toml"),
            "name = \"chat-worker\"\nmodel = \"deepseek-v4-flash\"\nmodel_provider = \"deepseek\"\n",
        )
        .unwrap();
        std::fs::write(
            agent_dir_in(dir).join("no-wire-worker.toml"),
            "name = \"no-wire-worker\"\nmodel = \"deepseek-v4-flash\"\nmodel_provider = \"other\"\n",
        )
        .unwrap();
        let config = r#"
[model_providers.deepseek]
name = "My DeepSeek"
base_url = "https://api.deepseek.com"
wire_api = "chat"

[model_providers.other]
name = "Other"
base_url = "https://other.example.com/v1"
"#;
        std::fs::write(config_path(dir), config).unwrap();
        let records = list_subagents_in(dir, config).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(
            records
                .iter()
                .find(|record| record.name == "chat-worker")
                .map(|record| record.wire_api.as_str()),
            Some("chat")
        );
        // provider 未声明 wire_api 时默认 responses。
        assert_eq!(
            records
                .iter()
                .find(|record| record.name == "no-wire-worker")
                .map(|record| record.wire_api.as_str()),
            Some("responses")
        );
    }

    #[test]
    fn wire_api_requires_legal_set() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let mut payload = test_payload("flash-worker", Some("sk-test"), "inherit", "high");
        payload.wire_api = Some("bogus".to_owned());
        let error = upsert_subagent_in(dir, &config_path(dir), "", &payload).unwrap_err();
        assert!(error.to_string().contains("wire_api"));
        assert!(!config_path(dir).exists());

        // 已有合法集合（chat_completions / anthropic / openai_responses 等）同样接受，
        // 但写入 Codex config 前必须归一化为规范值 responses / chat / anthropic。
        for (wire_api, canonical) in [
            ("chat_completions", "chat"),
            ("openai_chat", "chat"),
            ("anthropic_messages", "anthropic"),
            ("openai_responses", "responses"),
            ("openai-responses", "responses"),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let dir = temp.path();
            let mut payload = test_payload("flash-worker", Some("sk-test"), "inherit", "high");
            payload.wire_api = Some(wire_api.to_owned());
            upsert_subagent_in(dir, &config_path(dir), "", &payload).unwrap();
            let config = std::fs::read_to_string(config_path(dir)).unwrap();
            let doc = config.parse::<toml::Value>().unwrap();
            assert_eq!(
                doc["model_providers"]["deepseek"]["wire_api"].as_str(),
                Some(canonical)
            );
        }
    }

    #[test]
    fn list_subagents_fails_on_unreadable_or_corrupt_agent_files() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        std::fs::create_dir_all(agent_dir_in(dir)).unwrap();
        std::fs::write(config_path(dir), "").unwrap();

        // .toml 路径是目录（模拟不可读）→ 返回带路径的错误，不是静默空列表。
        let dir_as_toml = agent_dir_in(dir).join("blocked.toml");
        std::fs::create_dir(&dir_as_toml).unwrap();
        let error = list_subagents_in(dir, "").unwrap_err();
        assert!(error.to_string().contains("blocked.toml"));

        // 损坏 TOML → 返回带路径的解析错误，而不是被跳过。
        std::fs::remove_dir(&dir_as_toml).unwrap();
        std::fs::write(agent_dir_in(dir).join("corrupt.toml"), "name = ").unwrap();
        let error = list_subagents_in(dir, "").unwrap_err();
        assert!(error.to_string().contains("corrupt.toml"));

        // 非 .toml 与语法有效但无 name 的文件仍被忽略。
        std::fs::remove_file(agent_dir_in(dir).join("corrupt.toml")).unwrap();
        std::fs::write(agent_dir_in(dir).join("notes.txt"), "name = \"ignored\"\n").unwrap();
        std::fs::write(agent_dir_in(dir).join("orphan.toml"), "model = \"x\"\n").unwrap();
        let records = list_subagents_in(dir, "").unwrap();
        assert!(records.is_empty());
    }

    #[test]
    fn tampered_manifest_key_path_is_rejected_without_touching_target() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let payload = test_payload("flash-worker", Some("sk-real"), "workspace-write", "high");
        upsert_subagent_in(dir, &config_path(dir), "", &payload).unwrap();

        // 篡改 manifest：把 key_path 指向任意文件（decoy，非规范路径）。
        let decoy = dir.join("decoy.key");
        std::fs::write(&decoy, "sk-decoy-secret\n").unwrap();
        let manifest_path = manifest_path_in(dir);
        let mut manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
        manifest["agents"][0]["keyPath"] =
            serde_json::Value::String(decoy.to_string_lossy().into_owned());
        std::fs::write(&manifest_path, serde_json::to_string(&manifest).unwrap()).unwrap();

        // fetch models key 解析：拒绝且不读取 decoy。
        let error = resolve_models_api_key_in(dir, "deepseek", "").unwrap_err();
        assert!(error.to_string().contains("key_path"));
        assert_eq!(
            std::fs::read_to_string(&decoy).unwrap(),
            "sk-decoy-secret\n"
        );

        // 删除/恢复：拒绝且 config/agent/manifest/decoy 零变化。
        let config_before = std::fs::read_to_string(config_path(dir)).unwrap();
        let agent_before = std::fs::read_to_string(agent_path_in(dir, "flash-worker")).unwrap();
        let manifest_before = std::fs::read_to_string(&manifest_path).unwrap();
        let error =
            delete_subagent_in(dir, &config_path(dir), &config_before, "flash-worker").unwrap_err();
        assert!(error.to_string().contains("key_path"));
        assert_eq!(
            std::fs::read_to_string(config_path(dir)).unwrap(),
            config_before
        );
        assert_eq!(
            std::fs::read_to_string(agent_path_in(dir, "flash-worker")).unwrap(),
            agent_before
        );
        assert_eq!(
            std::fs::read_to_string(&manifest_path).unwrap(),
            manifest_before
        );
        assert_eq!(
            std::fs::read_to_string(&decoy).unwrap(),
            "sk-decoy-secret\n"
        );

        // 编辑/upsert：拒绝且零变化。
        let edit = test_payload("flash-worker", Some("sk-new"), "read-only", "high");
        let error = upsert_subagent_in(dir, &config_path(dir), &config_before, &edit).unwrap_err();
        assert!(error.to_string().contains("key_path"));
        assert_eq!(
            std::fs::read_to_string(config_path(dir)).unwrap(),
            config_before
        );
        assert_eq!(
            std::fs::read_to_string(&manifest_path).unwrap(),
            manifest_before
        );
        assert_eq!(
            std::fs::read_to_string(&decoy).unwrap(),
            "sk-decoy-secret\n"
        );
    }

    #[test]
    fn malicious_provider_id_cannot_escape_key_dir() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let payload = test_payload("flash-worker", Some("sk-real"), "workspace-write", "high");
        upsert_subagent_in(dir, &config_path(dir), "", &payload).unwrap();

        // 攻击面：providerId 带 "../" 逃逸 + keyPath 精确指向 join 后的逃逸路径。
        let outside = dir.join("outside.key");
        std::fs::write(&outside, "sk-outside-secret\n").unwrap();
        let escaped_provider = format!("../{}", outside.file_stem().unwrap().to_string_lossy());
        let escaped_key_path = key_dir_in(dir).join(format!("{escaped_provider}.key"));
        let manifest_path = manifest_path_in(dir);
        let mut manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
        manifest["agents"][0]["providerId"] = serde_json::Value::String(escaped_provider.clone());
        manifest["agents"][0]["keyPath"] =
            serde_json::Value::String(escaped_key_path.to_string_lossy().into_owned());
        std::fs::write(&manifest_path, serde_json::to_string(&manifest).unwrap()).unwrap();
        let config_before = std::fs::read_to_string(config_path(dir)).unwrap();
        let agent_before = std::fs::read_to_string(agent_path_in(dir, "flash-worker")).unwrap();
        let manifest_before = std::fs::read_to_string(&manifest_path).unwrap();

        // fetch/edit/delete/list 全部在接触任何外部文件前失败。
        let error = resolve_models_api_key_in(dir, &escaped_provider, "").unwrap_err();
        assert!(error.to_string().contains("model_provider"));
        let edit = test_payload("flash-worker", Some("sk-new"), "read-only", "high");
        let error = upsert_subagent_in(dir, &config_path(dir), &config_before, &edit).unwrap_err();
        assert!(error.to_string().contains("model_provider"));
        let error =
            delete_subagent_in(dir, &config_path(dir), &config_before, "flash-worker").unwrap_err();
        assert!(error.to_string().contains("model_provider"));
        let error = list_subagents_in(dir, "").unwrap_err();
        assert!(error.to_string().contains("model_provider"));

        // 外部文件从未被接触，config/agent/manifest 零变化。
        assert_eq!(
            std::fs::read_to_string(&outside).unwrap(),
            "sk-outside-secret\n"
        );
        assert_eq!(
            std::fs::read_to_string(config_path(dir)).unwrap(),
            config_before
        );
        assert_eq!(
            std::fs::read_to_string(agent_path_in(dir, "flash-worker")).unwrap(),
            agent_before
        );
        assert_eq!(
            std::fs::read_to_string(&manifest_path).unwrap(),
            manifest_before
        );
    }

    #[test]
    fn managed_provider_uses_auth_command_and_keeps_plaintext_out_of_config_and_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let secret = "sk-super-secret-new";
        let payload = test_payload("flash-worker", Some(secret), "workspace-write", "xhigh");
        upsert_subagent_in(dir, &config_path(dir), "", &payload).unwrap();

        // config.toml 不含明文 key，且使用官方 auth.command 从 key 文件读取。
        let config_text = std::fs::read_to_string(config_path(dir)).unwrap();
        assert!(!config_text.contains(secret));
        let config = config_text.parse::<toml::Value>().unwrap();
        let provider = config
            .get("model_providers")
            .and_then(|item| item.get("deepseek"))
            .expect("managed provider must be written");
        let auth = provider
            .get("auth")
            .expect("managed provider must use auth table");
        #[cfg(unix)]
        assert_eq!(
            auth.get("command").and_then(toml::Value::as_str),
            Some("/bin/cat")
        );
        #[cfg(windows)]
        assert_eq!(
            auth.get("command").and_then(toml::Value::as_str),
            Some("powershell.exe")
        );
        let args = auth.get("args").and_then(toml::Value::as_array).unwrap();
        let key_path = provider_key_path_in(dir, "deepseek");
        assert!(args
            .iter()
            .any(|arg| arg.as_str() == Some(&key_path.to_string_lossy())));
        assert_eq!(
            auth.get("timeout_ms").and_then(toml::Value::as_integer),
            Some(5000)
        );

        // manifest 不含明文 key；key 只存在于 key 文件。
        let manifest_text = std::fs::read_to_string(manifest_path_in(dir)).unwrap();
        assert!(!manifest_text.contains(secret));
        assert_eq!(std::fs::read_to_string(&key_path).unwrap(), secret);

        // key 文件 0600、key 目录 0700。
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let key_mode = std::fs::metadata(&key_path).unwrap().permissions().mode();
            assert_eq!(key_mode & 0o777, 0o600);
            let dir_mode = std::fs::metadata(key_dir_in(dir))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(dir_mode & 0o777, 0o700);
        }
    }

    #[test]
    fn editing_with_blank_key_keeps_existing_key() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let first = test_payload(
            "flash-worker",
            Some("sk-first"),
            "workspace-write",
            "medium",
        );
        upsert_subagent_in(dir, &config_path(dir), "", &first).unwrap();
        let key_path = provider_key_path_in(dir, "deepseek");
        assert_eq!(std::fs::read_to_string(&key_path).unwrap(), "sk-first");

        // 编辑时 api_key = None → 保留原 key。
        let current = std::fs::read_to_string(config_path(dir)).unwrap();
        let edit_none = test_payload("flash-worker", None, "workspace-write", "high");
        let record = upsert_subagent_in(dir, &config_path(dir), &current, &edit_none).unwrap();
        assert_eq!(std::fs::read_to_string(&key_path).unwrap(), "sk-first");
        assert_eq!(record.api_key.as_deref(), Some(""));

        // 编辑时空字符串 → 保留原 key。
        let current = std::fs::read_to_string(config_path(dir)).unwrap();
        let edit_blank = test_payload("flash-worker", Some("   "), "workspace-write", "high");
        upsert_subagent_in(dir, &config_path(dir), &current, &edit_blank).unwrap();
        assert_eq!(std::fs::read_to_string(&key_path).unwrap(), "sk-first");

        // config/manifest 始终不含明文。
        let config_text = std::fs::read_to_string(config_path(dir)).unwrap();
        assert!(!config_text.contains("sk-first"));
        let manifest_text = std::fs::read_to_string(manifest_path_in(dir)).unwrap();
        assert!(!manifest_text.contains("sk-first"));
    }

    fn assert_private_mode(path: &Path, expected: u32) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(path).unwrap().permissions().mode();
            assert_eq!(
                mode & 0o777,
                expected,
                "unexpected mode for {}",
                path.display()
            );
        }
        #[cfg(not(unix))]
        {
            let _ = (path, expected);
        }
    }

    #[test]
    fn private_key_and_manifest_are_0600_from_first_write_and_on_overwrite() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        // 首次注册前已有原始 key 文件（模拟原 provider 认证备份）。
        let key_path = provider_key_path_in(dir, "deepseek");
        std::fs::create_dir_all(key_path.parent().unwrap()).unwrap();
        std::fs::write(&key_path, "orig-key").unwrap();

        let first = test_payload("flash-worker", Some("sk-first"), "workspace-write", "high");
        upsert_subagent_in(dir, &config_path(dir), "", &first).unwrap();

        // 首次创建即 0600：key 文件与可能含原认证备份的 manifest 都必须是私密权限。
        assert_eq!(std::fs::read_to_string(&key_path).unwrap(), "sk-first");
        assert_private_mode(&key_path, 0o600);
        assert_private_mode(&manifest_path_in(dir), 0o600);
        assert_private_mode(&key_dir_in(dir), 0o700);

        // 覆盖写（编辑换 key）后权限保持 0600、内容更新。
        let current = std::fs::read_to_string(config_path(dir)).unwrap();
        let second = test_payload("flash-worker", Some("sk-second"), "read-only", "xhigh");
        upsert_subagent_in(dir, &config_path(dir), &current, &second).unwrap();
        assert_eq!(std::fs::read_to_string(&key_path).unwrap(), "sk-second");
        assert_private_mode(&key_path, 0o600);
        assert_private_mode(&manifest_path_in(dir), 0o600);
        assert_private_mode(&key_dir_in(dir), 0o700);

        // 删除是永久的：key 文件被删除（不再恢复首次注册前的 orig-key）。
        let current = std::fs::read_to_string(config_path(dir)).unwrap();
        delete_subagent_in(dir, &config_path(dir), &current, "flash-worker").unwrap();
        assert!(!key_path.exists(), "key 文件应被永久删除");

        // 原子写入不残留临时文件。
        let mut leftovers = Vec::new();
        for scan_dir in [dir, &key_dir_in(dir)] {
            let mut entries: Vec<String> = std::fs::read_dir(scan_dir)
                .unwrap()
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| name.contains(".tmp."))
                .collect();
            leftovers.append(&mut entries);
        }
        assert!(leftovers.is_empty(), "残留临时文件: {leftovers:?}");
    }

    #[test]
    fn upsert_provider_table_error_leaves_no_partial_key() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        // model_providers 被写成数组时，provider 表构造必然失败——
        // 该错误必须发生在任何 key 写入之前。
        let original_config = "model = \"deepseek-v4-flash\"\nmodel_providers = []\n";
        std::fs::write(config_path(dir), original_config).unwrap();
        let key_path = provider_key_path_in(dir, "deepseek");

        let payload = test_payload("flash-worker", Some("sk-new"), "workspace-write", "high");
        let error =
            upsert_subagent_in(dir, &config_path(dir), original_config, &payload).unwrap_err();
        assert!(error.to_string().contains("model_providers"));

        assert!(!key_path.exists(), "key 文件不得在失败后残留");
        assert_eq!(
            std::fs::read_to_string(config_path(dir)).unwrap(),
            original_config
        );
        assert!(!agent_path_in(dir, "flash-worker").exists());
        assert!(!manifest_path_in(dir).exists());
    }

    #[test]
    fn upsert_manifest_write_failure_restores_key_and_config() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let original_config = r#"
model = "deepseek-v4-flash"

[model_providers.deepseek]
name = "My DeepSeek"
base_url = "https://api.deepseek.com"
wire_api = "responses"
"#;
        std::fs::write(config_path(dir), original_config).unwrap();
        std::fs::create_dir_all(agent_dir_in(dir)).unwrap();
        let original_agent = "name = \"flash-worker\"\ncustom_flag = true\n";
        std::fs::write(agent_dir_in(dir).join("flash-worker.toml"), original_agent).unwrap();
        let key_path = provider_key_path_in(dir, "deepseek");
        std::fs::create_dir_all(key_path.parent().unwrap()).unwrap();
        std::fs::write(&key_path, "orig-key").unwrap();

        // key 写入成功后再注入 manifest 写入失败，验证回滚恢复全部原状。
        let payload = test_payload("flash-worker", Some("sk-new"), "workspace-write", "high");
        std::fs::write(dir.join(".fail-manifest-write"), "").unwrap();
        let error =
            upsert_subagent_in(dir, &config_path(dir), original_config, &payload).unwrap_err();
        assert!(error.to_string().contains("故障注入"));

        assert_eq!(
            std::fs::read_to_string(config_path(dir)).unwrap(),
            original_config
        );
        assert_eq!(
            std::fs::read_to_string(agent_dir_in(dir).join("flash-worker.toml")).unwrap(),
            original_agent
        );
        assert_eq!(std::fs::read_to_string(&key_path).unwrap(), "orig-key");
        assert!(!manifest_path_in(dir).exists());

        let leftovers: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".tmp."))
            .collect();
        assert!(leftovers.is_empty(), "残留临时文件: {leftovers:?}");
    }

    #[test]
    fn delete_is_blocked_when_workflow_state_is_corrupt_or_unreadable() {
        // workflow manifest 损坏 → 删除失败且所有文件原样。
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let payload = test_payload("flash-worker", Some("sk-x"), "workspace-write", "high");
        upsert_subagent_in(dir, &config_path(dir), "", &payload).unwrap();
        let config_before = std::fs::read_to_string(config_path(dir)).unwrap();
        let agent_before = std::fs::read_to_string(agent_path_in(dir, "flash-worker")).unwrap();
        let manifest_before = std::fs::read_to_string(manifest_path_in(dir)).unwrap();
        let key_before = std::fs::read_to_string(provider_key_path_in(dir, "deepseek")).unwrap();
        std::fs::write(dir.join("codex-cube-agent-workflow.json"), "{ not json").unwrap();

        let error =
            delete_subagent_in(dir, &config_path(dir), &config_before, "flash-worker").unwrap_err();
        assert!(error.to_string().contains("JSON 解析错误"));
        assert_eq!(
            std::fs::read_to_string(config_path(dir)).unwrap(),
            config_before
        );
        assert_eq!(
            std::fs::read_to_string(agent_path_in(dir, "flash-worker")).unwrap(),
            agent_before
        );
        assert_eq!(
            std::fs::read_to_string(manifest_path_in(dir)).unwrap(),
            manifest_before
        );
        assert_eq!(
            std::fs::read_to_string(provider_key_path_in(dir, "deepseek")).unwrap(),
            key_before
        );

        // workflow 安装且选中 flash-worker → 删除失败（需先取消选中），状态不变。
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let payload = test_payload("flash-worker", Some("sk-x"), "workspace-write", "high");
        upsert_subagent_in(dir, &config_path(dir), "", &payload).unwrap();
        crate::codex_agent_workflow::install(
            dir,
            "flash-worker",
            &["flash-worker".to_string()],
            crate::codex_agent_workflow::WORKFLOW_MODE_SKILL,
            "high",
        )
        .unwrap();
        let config_before = std::fs::read_to_string(config_path(dir)).unwrap();
        let manifest_before = std::fs::read_to_string(manifest_path_in(dir)).unwrap();
        let key_before = std::fs::read_to_string(provider_key_path_in(dir, "deepseek")).unwrap();

        let error =
            delete_subagent_in(dir, &config_path(dir), &config_before, "flash-worker").unwrap_err();
        assert!(error.to_string().contains("取消选中"));
        assert_eq!(
            std::fs::read_to_string(config_path(dir)).unwrap(),
            config_before
        );
        assert_eq!(
            std::fs::read_to_string(manifest_path_in(dir)).unwrap(),
            manifest_before
        );
        assert_eq!(
            std::fs::read_to_string(provider_key_path_in(dir, "deepseek")).unwrap(),
            key_before
        );
        // 未选中的其他 agent 不受 workflow 阻塞，可正常删除。
        let other = test_payload("other-worker", Some("sk-y"), "workspace-write", "high");
        upsert_subagent_in(dir, &config_path(dir), &config_before, &other).unwrap();
        delete_subagent_in(dir, &config_path(dir), &config_before, "other-worker").unwrap();
        assert!(!agent_path_in(dir, "other-worker").exists());
    }

    #[test]
    fn first_adoption_fails_when_original_agent_or_key_is_unreadable() {
        // 原 agent TOML 路径是目录 → 读取失败，且 config/manifest/key 均未写入。
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        std::fs::create_dir_all(agent_dir_in(dir)).unwrap();
        std::fs::create_dir(agent_path_in(dir, "flash-worker")).unwrap();
        let payload = test_payload("flash-worker", Some("sk-x"), "workspace-write", "high");
        let error = upsert_subagent_in(dir, &config_path(dir), "", &payload).unwrap_err();
        assert!(error.to_string().contains("flash-worker.toml"));
        assert!(!config_path(dir).exists());
        assert!(!manifest_path_in(dir).exists());
        assert!(!provider_key_path_in(dir, "deepseek").exists());

        // 原 key 路径是目录 → 读取失败，且 config/manifest 均未写入。
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let key_path = provider_key_path_in(dir, "deepseek");
        std::fs::create_dir_all(key_path.parent().unwrap()).unwrap();
        std::fs::create_dir(&key_path).unwrap();
        let payload = test_payload("flash-worker", Some("sk-x"), "workspace-write", "high");
        let error = upsert_subagent_in(dir, &config_path(dir), "", &payload).unwrap_err();
        assert!(error.to_string().contains("deepseek.key"));
        assert!(!config_path(dir).exists());
        assert!(!manifest_path_in(dir).exists());
    }

    #[test]
    fn cannot_take_over_provider_referenced_by_other_agent() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        std::fs::create_dir_all(agent_dir_in(dir)).unwrap();
        let other_path = agent_dir_in(dir).join("other-agent.toml");
        std::fs::write(
            &other_path,
            "name = \"other-agent\"\nmodel = \"deepseek-v3\"\nmodel_provider = \"deepseek\"\n",
        )
        .unwrap();
        let other_before = std::fs::read_to_string(&other_path).unwrap();

        let payload = test_payload("flash-worker", Some("sk-x"), "workspace-write", "high");
        let error = upsert_subagent_in(dir, &config_path(dir), "", &payload).unwrap_err();
        assert!(error.to_string().contains("Provider ID"));
        assert!(!config_path(dir).exists());
        assert!(!manifest_path_in(dir).exists());
        assert!(!provider_key_path_in(dir, "deepseek").exists());
        assert_eq!(std::fs::read_to_string(&other_path).unwrap(), other_before);
    }

    #[test]
    fn adopting_agent_with_own_provider_reference_is_not_blocked() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        std::fs::create_dir_all(agent_dir_in(dir)).unwrap();
        std::fs::write(
            agent_dir_in(dir).join("flash-worker.toml"),
            "name = \"flash-worker\"\nmodel = \"deepseek-v4-flash\"\nmodel_provider = \"deepseek\"\n",
        )
        .unwrap();

        let payload = test_payload("flash-worker", Some("sk-x"), "workspace-write", "high");
        upsert_subagent_in(dir, &config_path(dir), "", &payload).unwrap();
        assert!(provider_key_path_in(dir, "deepseek").exists());
        assert!(manifest_path_in(dir).exists());
        assert!(std::fs::read_to_string(config_path(dir))
            .unwrap()
            .contains("Codex Cube subagent"));
    }

    #[test]
    fn provider_scan_cannot_be_bypassed_with_forged_name() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        std::fs::create_dir_all(agent_dir_in(dir)).unwrap();
        // 另一文件伪造与当前 agent 相同的 name，仍必须被阻止（按规范路径排除自身）。
        std::fs::write(
            agent_dir_in(dir).join("other.toml"),
            "name = \"flash-worker\"\nmodel = \"deepseek-v3\"\nmodel_provider = \"deepseek\"\n",
        )
        .unwrap();
        let other_before = std::fs::read_to_string(agent_dir_in(dir).join("other.toml")).unwrap();

        let payload = test_payload("flash-worker", Some("sk-x"), "workspace-write", "high");
        let error = upsert_subagent_in(dir, &config_path(dir), "", &payload).unwrap_err();
        assert!(error.to_string().contains("Provider ID"));
        assert!(!config_path(dir).exists());
        assert!(!manifest_path_in(dir).exists());
        assert!(!provider_key_path_in(dir, "deepseek").exists());
        assert_eq!(
            std::fs::read_to_string(agent_dir_in(dir).join("other.toml")).unwrap(),
            other_before
        );
    }

    #[test]
    fn delete_is_blocked_when_other_agent_references_the_provider() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let payload = test_payload("flash-worker", Some("sk-x"), "workspace-write", "high");
        upsert_subagent_in(dir, &config_path(dir), "", &payload).unwrap();
        let config_before = std::fs::read_to_string(config_path(dir)).unwrap();
        let agent_before = std::fs::read_to_string(agent_path_in(dir, "flash-worker")).unwrap();
        let manifest_before = std::fs::read_to_string(manifest_path_in(dir)).unwrap();
        let key_before = std::fs::read_to_string(provider_key_path_in(dir, "deepseek")).unwrap();

        // 另一 unmanaged agent 共享同一 provider。
        std::fs::write(
            agent_dir_in(dir).join("other-agent.toml"),
            "name = \"other-agent\"\nmodel = \"deepseek-v3\"\nmodel_provider = \"deepseek\"\n",
        )
        .unwrap();

        let error =
            delete_subagent_in(dir, &config_path(dir), &config_before, "flash-worker").unwrap_err();
        assert!(error.to_string().contains("Provider ID"));
        assert!(error.to_string().contains("不能删除其配置"));
        // 零变化：config/manifest/agent/key 全部原样。
        assert_eq!(
            std::fs::read_to_string(config_path(dir)).unwrap(),
            config_before
        );
        assert_eq!(
            std::fs::read_to_string(agent_path_in(dir, "flash-worker")).unwrap(),
            agent_before
        );
        assert_eq!(
            std::fs::read_to_string(manifest_path_in(dir)).unwrap(),
            manifest_before
        );
        assert_eq!(
            std::fs::read_to_string(provider_key_path_in(dir, "deepseek")).unwrap(),
            key_before
        );

        // 移除共享引用后可以正常删除：provider 配置、agent、key、manifest 全部永久移除。
        std::fs::remove_file(agent_dir_in(dir).join("other-agent.toml")).unwrap();
        delete_subagent_in(dir, &config_path(dir), &config_before, "flash-worker").unwrap();
        assert!(!manifest_path_in(dir).exists());
        assert!(!agent_path_in(dir, "flash-worker").exists());
        assert!(!provider_key_path_in(dir, "deepseek").exists());
        let after = std::fs::read_to_string(config_path(dir)).unwrap();
        assert!(!after.contains("deepseek"));
    }

    #[test]
    fn missing_optional_snapshots_still_work() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        // 空目录：无 config/agent/key/manifest，全部按 None 处理。
        let payload = test_payload("flash-worker", Some("sk-x"), "workspace-write", "high");
        upsert_subagent_in(dir, &config_path(dir), "", &payload).unwrap();
        let current = std::fs::read_to_string(config_path(dir)).unwrap();
        delete_subagent_in(dir, &config_path(dir), &current, "flash-worker").unwrap();
        assert!(!agent_path_in(dir, "flash-worker").exists());
        assert!(!provider_key_path_in(dir, "deepseek").exists());
        assert!(!manifest_path_in(dir).exists());
    }

    #[test]
    fn delete_keeps_user_provider_when_not_provider_managed() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        // 复用用户既有 provider（URL 匹配、未提供 key）→ provider_managed=false。
        // provider 的 auth 指向用户自有的 key 文件（非 Cube key 目录）。
        let user_key = dir.join("user-key.txt");
        let original_config = format!(
            r#"
model = "deepseek-v4-flash"

[model_providers.deepseek]
name = "My DeepSeek"
base_url = "https://api.deepseek.com"
wire_api = "responses"

[model_providers.deepseek.auth]
command = "/bin/cat"
args = ["{}"]
"#,
            user_key.display()
        );
        std::fs::write(config_path(dir), &original_config).unwrap();
        std::fs::write(&user_key, "sk-user-key\n").unwrap();
        let payload = test_payload("flash-worker", None, "workspace-write", "high");
        upsert_subagent_in(dir, &config_path(dir), &original_config, &payload).unwrap();
        let manifest = manifest_file_in(dir).unwrap();
        let record = read_manifest_record(&manifest, "flash-worker").unwrap();
        assert!(!record.provider_managed);
        assert!(record.key_path.is_none());

        // 删除 → agent TOML 与 manifest 记录移除；用户 provider 配置与其自有 key 保留，
        // Cube key 目录不产生/不删除任何文件。
        let current = std::fs::read_to_string(config_path(dir)).unwrap();
        delete_subagent_in(dir, &config_path(dir), &current, "flash-worker").unwrap();
        assert!(!agent_path_in(dir, "flash-worker").exists());
        assert!(!manifest_path_in(dir).exists());
        let after = std::fs::read_to_string(config_path(dir)).unwrap();
        assert!(after.contains("My DeepSeek"));
        assert!(after.contains("https://api.deepseek.com"));
        assert!(after.contains(&user_key.display().to_string()));
        assert_eq!(std::fs::read_to_string(&user_key).unwrap(), "sk-user-key\n");
        assert!(!provider_key_path_in(dir, "deepseek").exists());
    }

    #[test]
    fn delete_keeps_key_for_non_provider_managed_record_with_canonical_key_path() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        // 正常复用用户既有 provider → provider_managed=false、key_path=None。
        let original_config = r#"
model = "deepseek-v4-flash"

[model_providers.deepseek]
name = "My DeepSeek"
base_url = "https://api.deepseek.com"
wire_api = "responses"
"#;
        std::fs::write(config_path(dir), original_config).unwrap();
        let payload = test_payload("flash-worker", None, "workspace-write", "high");
        upsert_subagent_in(dir, &config_path(dir), original_config, &payload).unwrap();

        // 异常/旧 manifest：provider_managed=false 但 keyPath 指向规范 Cube key 路径，
        // 且该 key 文件真实存在（模拟不应被触碰的既有 key）。
        let key_path = provider_key_path_in(dir, "deepseek");
        std::fs::create_dir_all(key_path.parent().unwrap()).unwrap();
        std::fs::write(&key_path, "sk-existing\n").unwrap();
        let manifest_path = manifest_path_in(dir);
        let mut manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
        manifest["agents"][0]["providerManaged"] = serde_json::Value::Bool(false);
        manifest["agents"][0]["keyPath"] =
            serde_json::Value::String(key_path.to_string_lossy().into_owned());
        std::fs::write(&manifest_path, serde_json::to_string(&manifest).unwrap()).unwrap();

        // 删除 → 仅移除 agent TOML 与 manifest 记录；provider 配置与规范路径 key 均保留。
        let current = std::fs::read_to_string(config_path(dir)).unwrap();
        delete_subagent_in(dir, &config_path(dir), &current, "flash-worker").unwrap();
        assert!(!agent_path_in(dir, "flash-worker").exists());
        assert!(!manifest_path_in(dir).exists());
        assert_eq!(std::fs::read_to_string(&key_path).unwrap(), "sk-existing\n");
        assert_eq!(std::fs::read_to_string(config_path(dir)).unwrap(), current);
    }

    #[test]
    fn delete_failure_rolls_back_to_pre_delete_state() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        // 两个受管 subagent：删除其中一个时 manifest 写入失败 → 全部恢复删除前状态。
        let p1 = test_payload(
            "flash-worker",
            Some("sk-deepseek"),
            "workspace-write",
            "high",
        );
        upsert_subagent_in(dir, &config_path(dir), "", &p1).unwrap();
        let current = std::fs::read_to_string(config_path(dir)).unwrap();
        let mut p2 = test_payload("gpt-worker", Some("sk-gpt"), "workspace-write", "high");
        p2.model_provider_id = "gpt-provider".to_owned();
        p2.model_base_url = "https://api.openai.com/v1".to_owned();
        upsert_subagent_in(dir, &config_path(dir), &current, &p2).unwrap();

        let config_before = std::fs::read_to_string(config_path(dir)).unwrap();
        let agent_before = std::fs::read_to_string(agent_path_in(dir, "flash-worker")).unwrap();
        let manifest_before = std::fs::read_to_string(manifest_path_in(dir)).unwrap();
        let key_before = std::fs::read_to_string(provider_key_path_in(dir, "deepseek")).unwrap();

        std::fs::write(dir.join(".fail-manifest-write"), "").unwrap();
        let error =
            delete_subagent_in(dir, &config_path(dir), &config_before, "flash-worker").unwrap_err();
        assert!(error.to_string().contains("故障注入"));

        // 回滚到删除前的当前状态：config/agent/manifest/key 全部原样。
        assert_eq!(
            std::fs::read_to_string(config_path(dir)).unwrap(),
            config_before
        );
        assert_eq!(
            std::fs::read_to_string(agent_path_in(dir, "flash-worker")).unwrap(),
            agent_before
        );
        assert_eq!(
            std::fs::read_to_string(manifest_path_in(dir)).unwrap(),
            manifest_before
        );
        assert_eq!(
            std::fs::read_to_string(provider_key_path_in(dir, "deepseek")).unwrap(),
            key_before
        );
        assert!(agent_path_in(dir, "gpt-worker").exists());
        assert_eq!(
            std::fs::read_to_string(provider_key_path_in(dir, "gpt-provider")).unwrap(),
            "sk-gpt"
        );

        // 移除故障标记后重试成功：两套 agent/provider/key 与 manifest 全部移除。
        std::fs::remove_file(dir.join(".fail-manifest-write")).unwrap();
        delete_subagent_in(dir, &config_path(dir), &config_before, "flash-worker").unwrap();
        assert!(!agent_path_in(dir, "flash-worker").exists());
        assert!(!provider_key_path_in(dir, "deepseek").exists());
        let after = std::fs::read_to_string(config_path(dir)).unwrap();
        assert!(!after.contains("deepseek"));
        // 另一个 subagent 不受影响。
        assert!(agent_path_in(dir, "gpt-worker").exists());
        assert!(manifest_path_in(dir).exists());
    }

    #[test]
    fn delete_permanently_removes_agent_provider_and_key_after_multiple_edits() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        // 首次注册前的原始状态：已有 provider、agent 文件、key 文件。
        let original_config = r#"
model = "deepseek-v4-flash"

[model_providers.deepseek]
name = "My DeepSeek"
base_url = "https://api.deepseek.com"
wire_api = "responses"

[model_providers.deepseek.auth]
command = "/bin/cat"
args = ["/custom/orig-key"]
"#;
        std::fs::write(config_path(dir), original_config).unwrap();
        std::fs::create_dir_all(agent_dir_in(dir)).unwrap();
        std::fs::write(
            agent_dir_in(dir).join("flash-worker.toml"),
            "name = \"flash-worker\"\ncustom_flag = true\n",
        )
        .unwrap();
        let key_path = provider_key_path_in(dir, "deepseek");
        std::fs::create_dir_all(key_path.parent().unwrap()).unwrap();
        std::fs::write(&key_path, "orig-key").unwrap();

        // 第一次注册接管。
        let p1 = test_payload("flash-worker", Some("sk-1"), "workspace-write", "high");
        upsert_subagent_in(dir, &config_path(dir), original_config, &p1).unwrap();

        // 多次编辑：换 key、换模型与沙箱、留空 key。
        let current = std::fs::read_to_string(config_path(dir)).unwrap();
        let mut p2 = test_payload("flash-worker", Some("sk-2"), "read-only", "xhigh");
        p2.model = "gpt-5.6-sol".to_owned();
        upsert_subagent_in(dir, &config_path(dir), &current, &p2).unwrap();
        let current = std::fs::read_to_string(config_path(dir)).unwrap();
        let p3 = test_payload("flash-worker", None, "read-only", "xhigh");
        upsert_subagent_in(dir, &config_path(dir), &current, &p3).unwrap();

        // 删除 → 永久删除 agent TOML、Cube provider 配置、key 与 manifest 记录，
        // 不再恢复首次注册前的内容（My DeepSeek / orig-key / 原 agent 文件）。
        let current = std::fs::read_to_string(config_path(dir)).unwrap();
        delete_subagent_in(dir, &config_path(dir), &current, "flash-worker").unwrap();

        let after = std::fs::read_to_string(config_path(dir)).unwrap();
        assert!(!after.contains("My DeepSeek"));
        assert!(!after.contains("/custom/orig-key"));
        assert!(!after.contains("Codex Cube subagent"));
        assert!(!after.contains("sk-1") && !after.contains("sk-2"));
        assert!(!agent_path_in(dir, "flash-worker").exists());
        assert!(!key_path.exists());
        assert!(!manifest_path_in(dir).exists());
    }

    #[test]
    fn unmanaged_agent_cannot_be_deleted() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        std::fs::create_dir_all(agent_dir_in(dir)).unwrap();
        std::fs::write(
            agent_dir_in(dir).join("rogue.toml"),
            "name = \"rogue\"\nmodel = \"some-model\"\n",
        )
        .unwrap();

        let error = delete_subagent_in(dir, &config_path(dir), "", "rogue").unwrap_err();
        assert!(error.to_string().contains("不是 Codex Cube 管理的注册项"));
        // 未受管 agent 文件必须原样保留。
        assert!(agent_dir_in(dir).join("rogue.toml").exists());
        assert!(!config_path(dir).exists());
    }

    #[test]
    fn corrupted_manifest_is_not_silently_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let garbage = "{ this is not valid json";
        std::fs::write(manifest_path_in(dir), garbage).unwrap();

        let payload = test_payload("flash-worker", Some("sk-x"), "workspace-write", "high");
        let error = upsert_subagent_in(dir, &config_path(dir), "", &payload).unwrap_err();
        assert!(error.to_string().contains("JSON 解析错误"));
        assert_eq!(
            std::fs::read_to_string(manifest_path_in(dir)).unwrap(),
            garbage
        );
        assert!(!agent_path_in(dir, "flash-worker").exists());
        assert!(!config_path(dir).exists());

        let error = delete_subagent_in(dir, &config_path(dir), "", "flash-worker").unwrap_err();
        assert!(error.to_string().contains("JSON 解析错误"));
        assert_eq!(
            std::fs::read_to_string(manifest_path_in(dir)).unwrap(),
            garbage
        );
    }

    #[test]
    fn reasoning_effort_requires_official_enum() {
        // 固定 6 档：minimal 已移除，新建默认 high 由前端负责。
        assert_eq!(
            SUBAGENT_REASONING_EFFORTS.to_vec(),
            vec!["low", "medium", "high", "xhigh", "max", "ultra"]
        );

        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        for effort in ["low", "medium", "high", "xhigh", "max", "ultra"] {
            let current = std::fs::read_to_string(config_path(dir)).unwrap_or_default();
            let payload = test_payload("flash-worker", Some("sk-x"), "inherit", effort);
            upsert_subagent_in(dir, &config_path(dir), &current, &payload).unwrap();
            let agent = std::fs::read_to_string(agent_path_in(dir, "flash-worker")).unwrap();
            let doc = agent.parse::<toml::Value>().unwrap();
            assert_eq!(
                doc.get("model_reasoning_effort")
                    .and_then(toml::Value::as_str),
                Some(effort)
            );
        }

        let current = std::fs::read_to_string(config_path(dir)).unwrap();
        // minimal 已从固定选项移除，应被拒绝。
        let bad_minimal = test_payload("flash-worker", None, "inherit", "minimal");
        let error = upsert_subagent_in(dir, &config_path(dir), &current, &bad_minimal).unwrap_err();
        assert!(error.to_string().contains("reasoning effort"));
        // none 会完全关闭推理，同样应被拒绝。
        let bad_none = test_payload("flash-worker", None, "inherit", "none");
        let error = upsert_subagent_in(dir, &config_path(dir), &current, &bad_none).unwrap_err();
        assert!(error.to_string().contains("reasoning effort"));
    }
}

#[cfg(test)]
mod custom_provider_tests {
    use super::*;

    fn payload(name: &str, provider_id: &str, key: Option<&str>) -> SubagentUpsertPayload {
        SubagentUpsertPayload {
            name: name.to_owned(),
            description: format!("{name} worker"),
            model: "deepseek-v4-flash".to_owned(),
            model_provider_id: provider_id.to_owned(),
            model_base_url: format!("https://{provider_id}.example/v1"),
            api_key: key.map(str::to_owned),
            sandbox_mode: INHERIT_SANDBOX_MODE.to_owned(),
            reasoning_effort: "high".to_owned(),
            wire_api: Some("responses".to_owned()),
            agent_type: None,
            cube_provider_id: None,
        }
    }

    fn config_path(config_dir: &Path) -> PathBuf {
        config_dir.join("config.toml")
    }

    #[test]
    fn upsert_rejects_non_responses_wire_api_for_custom_subagent() {
        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path();
        for wire_api in ["chat", "anthropic", "chat_completions"] {
            let mut invalid = payload("flash-worker", "internal-deepseek", Some("sk-agent"));
            invalid.wire_api = Some(wire_api.to_owned());
            let error = upsert_subagent_in(config_dir, &config_path(config_dir), "", &invalid)
                .expect_err("非 Responses wire_api 必须被拒绝");
            assert!(
                error.to_string().contains("仅支持 Responses"),
                "错误信息应说明仅支持 Responses: {error}"
            );
            assert!(!agent_path_in(config_dir, "flash-worker").exists());
        }

        // Responses（含别名）仍可正常注册。
        let ok = upsert_subagent_in(
            config_dir,
            &config_path(config_dir),
            "",
            &payload("flash-worker", "internal-deepseek", Some("sk-agent")),
        )
        .unwrap();
        assert_eq!(ok.wire_api, "responses");
    }

    #[test]
    fn rendered_agent_uses_scoped_custom_provider() {
        let payload = payload("flash-worker", "internal-deepseek", Some("sk-secret"));
        let rendered = render_agent_toml(&payload).unwrap();
        let doc = rendered.parse::<toml::Value>().unwrap();

        assert_eq!(
            doc.get("model_provider").and_then(toml::Value::as_str),
            Some(SUBAGENT_RUNTIME_PROVIDER_ID)
        );
        let custom = &doc["model_providers"][SUBAGENT_RUNTIME_PROVIDER_ID];
        assert_eq!(
            custom.get("base_url").and_then(toml::Value::as_str),
            Some("https://internal-deepseek.example/v1")
        );
        assert!(custom.get("auth").is_some());
        assert_eq!(
            custom
                .get("requires_openai_auth")
                .and_then(toml::Value::as_bool),
            Some(false)
        );
        assert!(!rendered.contains("sk-secret"));
        assert!(doc["model_providers"].get("internal-deepseek").is_none());
    }

    #[test]
    fn upsert_keeps_global_custom_provider_unchanged() {
        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path();
        let global = r#"model_provider = "custom"
model = "gpt-5.6-sol"

[model_providers.custom]
name = "Main subscription"
base_url = "https://main.example/v1"
wire_api = "responses"
"#;
        std::fs::write(config_path(config_dir), global).unwrap();

        let record = upsert_subagent_in(
            config_dir,
            &config_path(config_dir),
            global,
            &payload("flash-worker", "internal-deepseek", Some("sk-agent")),
        )
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(config_path(config_dir)).unwrap(),
            global
        );
        assert_eq!(record.model_provider_id, "internal-deepseek");
        assert!(record.available);
        let agent = std::fs::read_to_string(agent_path_in(config_dir, "flash-worker")).unwrap();
        let doc = agent.parse::<toml::Value>().unwrap();
        assert_eq!(doc["model_provider"].as_str(), Some("custom"));
        assert_eq!(
            doc["model_providers"]["custom"]["base_url"].as_str(),
            Some("https://internal-deepseek.example/v1")
        );
    }

    #[test]
    fn multiple_agents_have_independent_custom_provider_layers() {
        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path();
        let config_path = config_path(config_dir);

        upsert_subagent_in(
            config_dir,
            &config_path,
            "",
            &payload("flash-worker", "internal-deepseek", Some("sk-one")),
        )
        .unwrap();
        let mut second = payload("kimi-worker", "internal-kimi", Some("sk-two"));
        second.model = "kimi-k2.5".to_owned();
        upsert_subagent_in(config_dir, &config_path, "", &second).unwrap();

        for (agent_name, expected_url) in [
            ("flash-worker", "https://internal-deepseek.example/v1"),
            ("kimi-worker", "https://internal-kimi.example/v1"),
        ] {
            let text = std::fs::read_to_string(agent_path_in(config_dir, agent_name)).unwrap();
            let doc = text.parse::<toml::Value>().unwrap();
            assert_eq!(doc["model_provider"].as_str(), Some("custom"));
            assert_eq!(
                doc["model_providers"]["custom"]["base_url"].as_str(),
                Some(expected_url)
            );
        }
        assert!(!config_path.exists());
    }

    #[test]
    fn duplicate_internal_provider_id_gets_suffixed_for_key_isolation() {
        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path();
        let config_path = config_path(config_dir);
        upsert_subagent_in(
            config_dir,
            &config_path,
            "",
            &payload("first-worker", "shared-internal-id", Some("sk-one")),
        )
        .unwrap();

        // 同名不冲突时保留 name；内部 provider id 被占用时追加 name 后缀去重，
        // 保证每个 subagent 的 key 文件相互隔离而不是报错拒绝。
        let second = upsert_subagent_in(
            config_dir,
            &config_path,
            "",
            &payload("second-worker", "shared-internal-id", Some("sk-two")),
        )
        .unwrap();

        assert_eq!(second.name, "second-worker");
        assert_eq!(second.model_provider_id, "shared-internal-id-second-worker");
        assert!(agent_path_in(config_dir, "second-worker").exists());
        assert_eq!(
            std::fs::read_to_string(provider_key_path_in(config_dir, "shared-internal-id"))
                .unwrap(),
            "sk-one"
        );
        assert_eq!(
            std::fs::read_to_string(provider_key_path_in(
                config_dir,
                "shared-internal-id-second-worker",
            ))
            .unwrap(),
            "sk-two"
        );
    }

    #[test]
    fn same_name_different_provider_registers_distinct_subagent() {
        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path();
        let cfg_path = config_path(config_dir);

        // 同模型（同名）不同供应商：两个 subagent 必须共存，不能互相覆盖。
        upsert_subagent_in(
            config_dir,
            &cfg_path,
            "",
            &payload("flash-worker", "internal-deepseek", Some("sk-one")),
        )
        .unwrap();
        let second = upsert_subagent_in(
            config_dir,
            &cfg_path,
            "",
            &payload("flash-worker", "internal-gpt", Some("sk-two")),
        )
        .unwrap();

        // 第二个追加另一字段（内部 provider id）后缀，成为独立 subagent。
        assert_eq!(second.name, "flash-worker-internal-gpt");
        assert_eq!(second.model_provider_id, "internal-gpt");
        // 原始 agent 文件与 key 未被覆盖。
        assert_eq!(
            std::fs::read_to_string(provider_key_path_in(config_dir, "internal-deepseek")).unwrap(),
            "sk-one"
        );
        assert_eq!(
            std::fs::read_to_string(provider_key_path_in(config_dir, "internal-gpt")).unwrap(),
            "sk-two"
        );
        // manifest 同时包含两条记录，agent 文件两个。
        let manifest = manifest_file_in(config_dir).unwrap();
        assert_eq!(manifest.agents.len(), 2);
        assert!(agent_path_in(config_dir, "flash-worker").exists());
        assert!(agent_path_in(config_dir, "flash-worker-internal-gpt").exists());

        // 编辑同名同 provider 的记录仍是编辑（名称不变），不产生新记录。
        let mut edit = payload("flash-worker", "internal-deepseek", None);
        edit.description = "edited worker".to_owned();
        let edited = upsert_subagent_in(config_dir, &cfg_path, "", &edit).unwrap();
        assert_eq!(edited.name, "flash-worker");
        assert_eq!(edited.model_provider_id, "internal-deepseek");
        let manifest = manifest_file_in(config_dir).unwrap();
        assert_eq!(manifest.agents.len(), 2);
    }

    #[test]
    fn list_prefers_embedded_custom_and_falls_back_to_legacy_global_provider() {
        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path();
        std::fs::create_dir_all(agent_dir_in(config_dir)).unwrap();
        std::fs::write(
            agent_path_in(config_dir, "embedded"),
            r#"name = "embedded"
description = "embedded worker"
model = "model-a"
model_provider = "custom"

[model_providers.custom]
name = "Embedded"
base_url = "https://embedded.example/v1"
wire_api = "chat"
env_key = "EMBEDDED_KEY"
"#,
        )
        .unwrap();
        std::fs::write(
            agent_path_in(config_dir, "legacy"),
            r#"name = "legacy"
description = "legacy worker"
model = "model-b"
model_provider = "legacy-provider"
"#,
        )
        .unwrap();
        let global = r#"[model_providers.legacy-provider]
name = "Legacy"
base_url = "https://legacy.example/v1"
wire_api = "responses"
env_key = "LEGACY_KEY"
"#;

        let records = list_subagents_in(config_dir, global).unwrap();
        let embedded = records
            .iter()
            .find(|record| record.name == "embedded")
            .unwrap();
        assert!(embedded.available);
        assert_eq!(embedded.model_provider_id, "custom");
        assert_eq!(embedded.model_base_url, "https://embedded.example/v1");
        assert_eq!(embedded.wire_api, "chat");
        let legacy = records
            .iter()
            .find(|record| record.name == "legacy")
            .unwrap();
        assert!(legacy.available);
        assert_eq!(legacy.model_provider_id, "legacy-provider");
        assert_eq!(legacy.model_base_url, "https://legacy.example/v1");
    }

    #[test]
    fn delete_removes_only_agent_owned_files() {
        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path();
        let global = r#"model_provider = "custom"
[model_providers.custom]
base_url = "https://main.example/v1"
wire_api = "responses"
"#;
        let config_path = config_path(config_dir);
        std::fs::write(&config_path, global).unwrap();
        upsert_subagent_in(
            config_dir,
            &config_path,
            global,
            &payload("flash-worker", "internal-deepseek", Some("sk-agent")),
        )
        .unwrap();

        delete_subagent_in(config_dir, &config_path, global, "flash-worker").unwrap();

        assert_eq!(std::fs::read_to_string(&config_path).unwrap(), global);
        assert!(!agent_path_in(config_dir, "flash-worker").exists());
        assert!(!provider_key_path_in(config_dir, "internal-deepseek").exists());
        assert!(!manifest_path_in(config_dir).exists());
    }

    #[test]
    fn live_config_write_never_restores_agent_provider() {
        let temp = tempfile::tempdir().unwrap();
        let next = r#"model_provider = "custom"
[model_providers.custom]
base_url = "https://next.example/v1"
wire_api = "responses"
"#;
        let output = preserve_managed_providers_for_live_write_in(
            temp.path(),
            "[model_providers.old-agent]\nbase_url = \"https://old.example\"\n",
            next,
        )
        .unwrap();
        assert_eq!(output, next);
        assert!(!output.contains("old-agent"));
    }
}
