use crate::codex_subagents::SubagentRecord;
use crate::config::{delete_file, write_json_file, write_text_file};
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use toml_edit::{value, DocumentMut, Item, TableLike};

pub const WORKFLOW_MANIFEST_FILENAME: &str = "codex-cube-agent-workflow.json";
pub const WORKFLOW_BACKUP_FILENAME: &str = "codex-cube-workflow-instructions-backup.json";
pub const WORKFLOW_AGENT_DEFAULTS_BACKUP_FILENAME: &str =
    "codex-cube-workflow-agent-defaults-backup.json";
pub const WORKFLOW_SKILL_NAME: &str = "cube-dispatch";
pub const WORKFLOW_SKILL_DIRECTORY: &str = "cube-dispatch";
pub const WORKFLOW_SKILL_DB_ID: &str = "local:cube-dispatch";
pub const WORKFLOW_SKILL_DISPLAY_NAME: &str = "Cube Dispatch";
/// 旧版 skill 名（v1 曾用 subagent-workflow）；安装时清理，避免 Codex 同时看到两个 skill。
pub const LEGACY_WORKFLOW_SKILL_DIRECTORY: &str = "subagent-workflow";
pub const LEGACY_WORKFLOW_SKILL_DB_ID: &str = "local:subagent-workflow";
pub const WORKFLOW_MODE_SKILL: &str = "skill";
pub const WORKFLOW_MODE_AGENTS_MD: &str = "agents-md";
pub const MANAGED_BLOCK_BEGIN_MARKER: &str = "# >>> codex-cube managed block >>>";
pub const MANAGED_BLOCK_END_MARKER: &str = "# <<< codex-cube managed block <<<";

fn default_workflow_mode() -> String {
    WORKFLOW_MODE_AGENTS_MD.to_string()
}

fn default_skill_directory() -> String {
    WORKFLOW_SKILL_DIRECTORY.to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowManifest {
    pub version: u32,
    pub worker_agent: String,
    #[serde(default)]
    pub worker_agents: Vec<String>,
    /// 角色 → agent 映射（worker / explorer / default）；旧 manifest 无此字段时由
    /// worker_agents / worker_agent 推导，保持向后兼容。
    #[serde(default)]
    pub role_agents: RoleAgents,
    /// "skill"（新式 Workflow Skill）或遗留 "agents-md"（AGENTS.md 注入）。
    #[serde(default = "default_workflow_mode")]
    pub mode: String,
    /// 新式安装时写入的 Skill 目录名。
    #[serde(default = "default_skill_directory")]
    pub skill_directory: String,
}

/// 角色 → 注册 subagent 列表（顺序即优先级）。三个官方内置角色：
/// worker（实现/执行）、explorer（只读探索）、default（通用兜底）。
/// 同一个 subagent 可出现在多个角色中（复用）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RoleAgents {
    #[serde(default)]
    pub worker: Vec<String>,
    #[serde(default)]
    pub explorer: Vec<String>,
    #[serde(default)]
    pub default: Vec<String>,
}

impl RoleAgents {
    /// 全部角色选中的 agent（去重、保持角色顺序：default > worker > explorer）。
    pub fn union(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for name in self
            .default
            .iter()
            .chain(self.worker.iter())
            .chain(self.explorer.iter())
        {
            let name = name.trim();
            if name.is_empty() || !seen.insert(name.to_owned()) {
                continue;
            }
            out.push(name.to_owned());
        }
        out
    }

    /// 是否没有任何角色选中。
    pub fn is_empty(&self) -> bool {
        self.worker.is_empty() && self.explorer.is_empty() && self.default.is_empty()
    }

    fn names_for_role(&self, role: &str) -> &[String] {
        match role {
            crate::codex_subagents::AGENT_TYPE_EXPLORER => &self.explorer,
            crate::codex_subagents::AGENT_TYPE_DEFAULT => &self.default,
            _ => &self.worker,
        }
    }

    /// 派发选中的 agent：先取该角色列表第一项，空则按 worker → default → explorer 回退。
    pub fn dispatch_agent_for_role(&self, role: &str) -> Option<&str> {
        first_nonempty_agent(self.names_for_role(role))
            .or_else(|| first_nonempty_agent(&self.worker))
            .or_else(|| first_nonempty_agent(&self.default))
            .or_else(|| first_nonempty_agent(&self.explorer))
    }
}

fn first_nonempty_agent(names: &[String]) -> Option<&str> {
    names
        .iter()
        .map(|name| name.trim())
        .find(|name| !name.is_empty())
}

/// 默认 worker：default 角色第一个 → worker 角色第一个 → explorer 角色第一个 → 空。
pub fn derive_worker_agent(role_agents: &RoleAgents) -> String {
    role_agents
        .default
        .first()
        .or_else(|| role_agents.worker.first())
        .or_else(|| role_agents.explorer.first())
        .cloned()
        .unwrap_or_default()
}

/// cube-dispatch 运行时解析出的派发目标：注册名 + 角色 + agent TOML 中的模型/推理档位。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchProfile {
    pub agent_name: String,
    pub role: String,
    pub model: String,
    pub reasoning_effort: String,
}

/// 根据 spawn 的 `agent_type` 解析注册代理。无 Workflow / 无法解析时返回 None。
pub fn resolve_dispatch_profile(
    config_dir: &Path,
    agent_type: Option<&str>,
) -> Option<DispatchProfile> {
    match resolve_dispatch_target(config_dir, agent_type) {
        DispatchResolve::Resolved(profile) => Some(profile),
        DispatchResolve::NoWorkflow | DispatchResolve::Unresolved { .. } => None,
    }
}

/// 派发解析结果：区分「未安装 Workflow」与「已安装但解析失败」，避免后者静默落到全局默认模型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchResolve {
    NoWorkflow,
    Unresolved { agent_type: String },
    Resolved(DispatchProfile),
}

/// 根据 spawn 的 `agent_type` 解析注册代理。
///
/// - 无 Workflow manifest → [`DispatchResolve::NoWorkflow`]（不改写，避免影响未启用协作流的会话）
/// - 已安装但 TOML/角色无法解析 → [`DispatchResolve::Unresolved`]（禁止静默落到全局默认模型）
/// - 成功 → [`DispatchResolve::Resolved`]
pub fn resolve_dispatch_target(config_dir: &Path, agent_type: Option<&str>) -> DispatchResolve {
    let requested = agent_type.unwrap_or("").trim();
    let requested = if requested.is_empty() {
        crate::codex_subagents::AGENT_TYPE_WORKER
    } else {
        requested
    };
    let manifest = match load_manifest(config_dir) {
        Ok(Some(manifest)) => manifest,
        Ok(None) => return DispatchResolve::NoWorkflow,
        Err(_) => {
            return DispatchResolve::Unresolved {
                agent_type: requested.to_owned(),
            };
        }
    };

    let (agent_name, role) = if crate::codex_subagents::AGENT_TYPES.contains(&requested) {
        let Some(name) = manifest
            .role_agents
            .dispatch_agent_for_role(requested)
            .map(str::to_owned)
            .or_else(|| {
                let worker = manifest.worker_agent.trim();
                (!worker.is_empty()).then(|| worker.to_owned())
            })
        else {
            return DispatchResolve::Unresolved {
                agent_type: requested.to_owned(),
            };
        };
        (name, requested.to_owned())
    } else {
        let role = if manifest
            .role_agents
            .explorer
            .iter()
            .any(|name| name == requested)
        {
            crate::codex_subagents::AGENT_TYPE_EXPLORER.to_owned()
        } else if manifest
            .role_agents
            .default
            .iter()
            .any(|name| name == requested)
        {
            crate::codex_subagents::AGENT_TYPE_DEFAULT.to_owned()
        } else {
            crate::codex_subagents::AGENT_TYPE_WORKER.to_owned()
        };
        (requested.to_owned(), role)
    };

    let Some((model, reasoning_effort)) = read_agent_model_and_effort(config_dir, &agent_name)
    else {
        return DispatchResolve::Unresolved {
            agent_type: requested.to_owned(),
        };
    };
    DispatchResolve::Resolved(DispatchProfile {
        agent_name,
        role,
        model,
        reasoning_effort,
    })
}

/// Cube-dispatch 子代理在代理层的路由：注册名 + 模型 + 可选的 Cube 供应商 id。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchRoute {
    pub agent_name: String,
    pub model: String,
    pub cube_provider_id: Option<String>,
}

/// 当前 Workflow 已注册、且 agent TOML 中 `model` 等于 `model` 的派发目标。
///
/// 用于把 generic `worker` 子会话的请求与「切换供应商后的过期会话模型」区分开：
/// 前者应保留并路由到注册供应商，后者才回退到当前上游模型。
pub fn resolve_dispatch_route_for_model(config_dir: &Path, model: &str) -> Option<DispatchRoute> {
    let model = model.trim();
    if model.is_empty() {
        return None;
    }
    let Ok(Some(manifest)) = load_manifest(config_dir) else {
        return None;
    };
    for name in workflow_registered_agent_names(&manifest) {
        let Some((agent_model, _)) = read_agent_model_and_effort(config_dir, &name) else {
            continue;
        };
        if agent_model == model {
            return Some(DispatchRoute {
                cube_provider_id: managed_cube_provider_id(config_dir, &name),
                agent_name: name,
                model: agent_model,
            });
        }
    }
    None
}

pub fn is_registered_dispatch_model(config_dir: &Path, model: &str) -> bool {
    resolve_dispatch_route_for_model(config_dir, model).is_some()
}

fn workflow_registered_agent_names(manifest: &WorkflowManifest) -> Vec<String> {
    let mut names = manifest.role_agents.union();
    let worker = manifest.worker_agent.trim();
    if !worker.is_empty() && !names.iter().any(|name| name == worker) {
        names.push(worker.to_owned());
    }
    names
}

fn managed_cube_provider_id(config_dir: &Path, agent_name: &str) -> Option<String> {
    let path = config_dir.join(crate::codex_subagents::SUBAGENT_MANIFEST_FILENAME);
    let text = std::fs::read_to_string(path).ok()?;
    let manifest: serde_json::Value = serde_json::from_str(&text).ok()?;
    manifest
        .get("agents")
        .and_then(serde_json::Value::as_array)?
        .iter()
        .find_map(|agent| {
            let name = agent.get("name").and_then(serde_json::Value::as_str)?;
            if name != agent_name {
                return None;
            }
            let cube_id = agent
                .get("cubeProviderId")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_owned);
            cube_id.or_else(|| {
                agent
                    .get("providerId")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned)
            })
        })
}

fn read_agent_model_and_effort(config_dir: &Path, name: &str) -> Option<(String, String)> {
    validate_agent_name(name).ok()?;
    let path = config_dir
        .join(crate::codex_subagents::SUBAGENT_AGENT_DIRNAME)
        .join(format!("{name}.toml"));
    let text = std::fs::read_to_string(path).ok()?;
    let agent: toml::Value = text.parse().ok()?;
    let model = agent
        .get("model")
        .and_then(toml::Value::as_str)
        .unwrap_or("")
        .trim()
        .to_owned();
    if model.is_empty() {
        return None;
    }
    // Match list_subagents: a missing effort still yields a usable agent (default medium).
    // Requiring both fields here would skip model injection and fall back to [agents] defaults.
    let reasoning_effort = agent
        .get("model_reasoning_effort")
        .and_then(toml::Value::as_str)
        .unwrap_or("medium")
        .trim();
    let reasoning_effort = if reasoning_effort.is_empty() {
        "medium".to_owned()
    } else {
        reasoning_effort.to_owned()
    };
    Some((model, reasoning_effort))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowBackup {
    pub instructions_md: Option<String>,
    pub created_at: String,
}

pub fn manifest_path(config_dir: &Path) -> PathBuf {
    config_dir.join(WORKFLOW_MANIFEST_FILENAME)
}

pub fn backup_path(config_dir: &Path) -> PathBuf {
    config_dir.join(WORKFLOW_BACKUP_FILENAME)
}

pub fn agent_defaults_backup_path(config_dir: &Path) -> PathBuf {
    config_dir.join(WORKFLOW_AGENT_DEFAULTS_BACKUP_FILENAME)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct WorkflowAgentDefaultsBackup {
    #[serde(default)]
    had_config_file: bool,
    #[serde(default)]
    had_agents_table: bool,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    reasoning_effort: Option<String>,
}

pub fn instructions_path(config_dir: &Path) -> PathBuf {
    config_dir.join("AGENTS.md")
}

/// 读取可选文本文件：NotFound 视为"文件不存在"返回 None，
/// 其他 IO 错误（如权限不足）原样返回，避免把不可读文件当成空文件覆盖。
fn read_optional_text(path: &Path) -> Result<Option<String>, AppError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(AppError::io(path, error)),
    }
}

/// 严格读取 workflow 备份：NotFound 返回 None（才会新建备份）；
/// 不可读、损坏 JSON、结构非法（缺字段/类型错）都返回错误。
/// created_at 按最小改动不校验时间戳格式，只保证字段存在且类型正确。
fn read_backup(config_dir: &Path) -> Result<Option<WorkflowBackup>, AppError> {
    let path = backup_path(config_dir);
    match read_optional_text(&path)? {
        Some(text) => serde_json::from_str::<WorkflowBackup>(&text)
            .map(Some)
            .map_err(|error| AppError::Message(format!("解析 AGENTS.md 备份失败: {error}"))),
        None => Ok(None),
    }
}

fn read_agent_defaults_backup(
    config_dir: &Path,
) -> Result<Option<WorkflowAgentDefaultsBackup>, AppError> {
    let path = agent_defaults_backup_path(config_dir);
    match read_optional_text(&path)? {
        Some(text) => serde_json::from_str::<WorkflowAgentDefaultsBackup>(&text)
            .map(Some)
            .map_err(|error| AppError::Message(format!("解析 subagent 默认配置备份失败: {error}"))),
        None => Ok(None),
    }
}

fn agents_table_mut(doc: &mut DocumentMut) -> Result<&mut dyn TableLike, AppError> {
    if doc.get("agents").is_none() {
        doc["agents"] = toml_edit::table();
    }
    doc.get_mut("agents")
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| AppError::Config("config.toml 的 [agents] 必须是表或内联表".to_string()))
}

fn capture_agent_defaults_backup(
    config_text: Option<&str>,
) -> Result<WorkflowAgentDefaultsBackup, AppError> {
    let Some(config_text) = config_text else {
        return Ok(WorkflowAgentDefaultsBackup {
            had_config_file: false,
            had_agents_table: false,
            model: None,
            reasoning_effort: None,
        });
    };
    let doc = config_text
        .parse::<DocumentMut>()
        .map_err(|error| AppError::Message(format!("解析 Codex config.toml 失败: {error}")))?;
    let agents = match doc.get("agents") {
        None => {
            return Ok(WorkflowAgentDefaultsBackup {
                had_config_file: true,
                had_agents_table: false,
                model: None,
                reasoning_effort: None,
            });
        }
        Some(item) => item.as_table_like().ok_or_else(|| {
            AppError::Config("config.toml 的 [agents] 必须是表或内联表".to_string())
        })?,
    };
    let read_string = |key: &str| -> Result<Option<String>, AppError> {
        let Some(item) = agents.get(key) else {
            return Ok(None);
        };
        let Some(value) = item.as_str() else {
            return Err(AppError::Message(format!(
                "config.toml 的 [agents].{key} 必须是字符串"
            )));
        };
        Ok(Some(value.to_owned()))
    };
    Ok(WorkflowAgentDefaultsBackup {
        had_config_file: true,
        had_agents_table: true,
        model: read_string("default_subagent_model")?,
        reasoning_effort: read_string("default_subagent_reasoning_effort")?,
    })
}

/// 将 Workflow 当前默认 worker 的模型/推理档位写入 Codex 的 subagent fallback 配置。
/// 首次安装时保存原值；重复安装只更新值，不覆盖原始备份。
///
/// cube-dispatch 的 Skill 安装路径不再调用本函数：generic `worker` 必须从注册
/// agent TOML 解析模型，并由代理层显式注入，避免全局默认值覆盖后续更换的 worker。
pub fn sync_agent_defaults(
    config_dir: &Path,
    model: &str,
    reasoning_effort: &str,
) -> Result<(), AppError> {
    let model = model.trim();
    let reasoning_effort = reasoning_effort.trim();
    if model.is_empty() || reasoning_effort.is_empty() {
        return Err(AppError::InvalidInput(
            "workflow 默认 worker 的 model/reasoning_effort 不能为空".into(),
        ));
    }

    let config_path = config_dir.join("config.toml");
    let backup_path = agent_defaults_backup_path(config_dir);
    let current_config = read_optional_text(&config_path)?;
    let backup_exists = read_agent_defaults_backup(config_dir)?.is_some();
    let backup = if backup_exists {
        None
    } else {
        Some(capture_agent_defaults_backup(current_config.as_deref())?)
    };

    // Validate and construct the complete next config before creating the lifecycle backup.
    // A malformed existing [agents] value must fail without leaving install state behind.
    let mut doc = current_config
        .as_deref()
        .unwrap_or_default()
        .parse::<DocumentMut>()
        .map_err(|error| AppError::Message(format!("解析 Codex config.toml 失败: {error}")))?;
    let agents = agents_table_mut(&mut doc)?;
    agents.insert("default_subagent_model", value(model));
    agents.insert("default_subagent_reasoning_effort", value(reasoning_effort));

    if let Some(backup) = backup {
        write_private_json(&backup_path, &backup)?;
    }
    if let Err(error) = write_text_file(&config_path, &doc.to_string()) {
        if !backup_exists {
            let _ = delete_file(&backup_path);
        }
        return Err(error);
    }
    Ok(())
}

/// Skill 安装不再写入 `[agents].default_subagent_*`。有旧版备份时恢复安装前的值；
/// 没有备份时清掉残留键，避免 generic `worker` 继续继承上一任 worker 的模型。
pub fn restore_or_clear_agent_defaults_for_skill_install(
    config_dir: &Path,
) -> Result<(), AppError> {
    let had_backup = read_agent_defaults_backup(config_dir)?.is_some();
    restore_agent_defaults(config_dir)?;
    if had_backup {
        return Ok(());
    }
    clear_agent_default_fallback_keys(config_dir)
}

fn clear_agent_default_fallback_keys(config_dir: &Path) -> Result<(), AppError> {
    let config_path = config_dir.join("config.toml");
    let Some(config_text) = read_optional_text(&config_path)? else {
        return Ok(());
    };
    let mut doc = config_text
        .parse::<DocumentMut>()
        .map_err(|error| AppError::Message(format!("解析 Codex config.toml 失败: {error}")))?;
    let Some(agents_item) = doc.get_mut("agents") else {
        return Ok(());
    };
    let Some(agents) = agents_item.as_table_like_mut() else {
        return Err(AppError::Config(
            "config.toml 的 [agents] 必须是表或内联表".to_string(),
        ));
    };
    let removed = agents.remove("default_subagent_model").is_some()
        | agents.remove("default_subagent_reasoning_effort").is_some();
    if !removed {
        return Ok(());
    }
    if agents.is_empty() {
        doc.as_table_mut().remove("agents");
    }
    let next = doc.to_string();
    if next.trim().is_empty() {
        delete_file(&config_path)?;
    } else {
        write_text_file(&config_path, &next)?;
    }
    Ok(())
}

/// 恢复 Workflow 安装前的 subagent fallback 配置，仅触碰 Workflow 管理的两个键。
pub fn restore_agent_defaults(config_dir: &Path) -> Result<(), AppError> {
    let Some(backup) = read_agent_defaults_backup(config_dir)? else {
        return Ok(());
    };
    let config_path = config_dir.join("config.toml");
    let Some(config_text) = read_optional_text(&config_path)? else {
        if backup.had_config_file {
            return Err(AppError::Config(
                "Codex config.toml 在 Workflow 卸载前已被删除，无法安全恢复原配置".to_string(),
            ));
        }
        delete_file(&agent_defaults_backup_path(config_dir))?;
        return Ok(());
    };
    let mut doc = config_text
        .parse::<DocumentMut>()
        .map_err(|error| AppError::Message(format!("解析 Codex config.toml 失败: {error}")))?;
    let agents = agents_table_mut(&mut doc)?;
    match backup.model {
        Some(model) => {
            agents.insert("default_subagent_model", value(model));
        }
        None => {
            agents.remove("default_subagent_model");
        }
    }
    match backup.reasoning_effort {
        Some(effort) => {
            agents.insert("default_subagent_reasoning_effort", value(effort));
        }
        None => {
            agents.remove("default_subagent_reasoning_effort");
        }
    }
    let remove_agents_table = !backup.had_agents_table && agents.is_empty();
    if remove_agents_table {
        doc.as_table_mut().remove("agents");
    }
    let restored_config = doc.to_string();
    if !backup.had_config_file && restored_config.trim().is_empty() {
        delete_file(&config_path)?;
    } else {
        write_text_file(&config_path, &restored_config)?;
    }
    delete_file(&agent_defaults_backup_path(config_dir))
}

/// 是否可以取消/恢复：有效备份存在 → true；无备份但 AGENTS.md 含受管块或
/// workflow manifest 存在 → true（cancel 仍可清理/恢复）；全部缺失 → false。
/// 备份不可读/损坏/结构非法 → 错误。不要用 backup_path().exists() 判断。
pub fn can_undo(config_dir: &Path) -> Result<bool, AppError> {
    let has_workflow_backup = read_backup(config_dir)?.is_some();
    let has_agent_defaults_backup = read_agent_defaults_backup(config_dir)?.is_some();
    if has_workflow_backup || has_agent_defaults_backup {
        return Ok(true);
    }
    if load_manifest(config_dir)?.is_some() {
        return Ok(true);
    }
    let Some(text) = read_optional_text(&instructions_path(config_dir))? else {
        return Ok(false);
    };
    Ok(managed_block_range(&text).is_some())
}

pub fn load_manifest(config_dir: &Path) -> Result<Option<WorkflowManifest>, AppError> {
    let path = manifest_path(config_dir);
    let Some(text) = read_optional_text(&path)? else {
        return Ok(None);
    };
    let manifest = serde_json::from_str::<WorkflowManifest>(&text)
        .map_err(|error| AppError::json(&path, error))?;
    if manifest.version != 1 {
        return Err(AppError::Message(format!(
            "不支持的 workflow manifest 版本 {}",
            manifest.version
        )));
    }
    validate_agent_name(&manifest.worker_agent)?;
    for name in &manifest.worker_agents {
        validate_agent_name(name)?;
    }
    for name in manifest
        .role_agents
        .default
        .iter()
        .chain(manifest.role_agents.worker.iter())
        .chain(manifest.role_agents.explorer.iter())
    {
        validate_agent_name(name)?;
    }
    Ok(Some(manifest))
}

pub fn selected_agents(config_dir: &Path) -> Result<Vec<String>, AppError> {
    let Some(manifest) = load_manifest(config_dir)? else {
        return Ok(Vec::new());
    };
    let mut selected = manifest.role_agents.union();
    if selected.is_empty() {
        selected = manifest.worker_agents;
    }
    if selected.is_empty() && !manifest.worker_agent.trim().is_empty() {
        selected.push(manifest.worker_agent.trim().to_owned());
    }
    selected.retain(|name| !name.trim().is_empty());
    Ok(selected)
}

pub fn workflow_uses_agent(config_dir: &Path, name: &str) -> Result<bool, AppError> {
    let name = name.trim();
    if name.is_empty() {
        return Ok(false);
    }
    // 删除保护只看 Workflow Skill 的选中集合（manifest），与 AGENTS.md 无关。
    // 严格读取 manifest：损坏/不支持版本视为错误，不能当作“未选中”。
    Ok(selected_agents(config_dir)?
        .iter()
        .any(|selected| selected == name))
}

/// Codex 实际读取的 skills 目录：`~/.codex/skills`，或 settings 覆盖目录下的 `skills`。
pub fn codex_skills_dir() -> PathBuf {
    if let Some(custom) = crate::settings::get_codex_override_dir() {
        custom.join("skills")
    } else {
        crate::config::get_home_dir().join(".codex").join("skills")
    }
}

pub fn workflow_skill_dir() -> Result<PathBuf, AppError> {
    Ok(codex_skills_dir().join(WORKFLOW_SKILL_DIRECTORY))
}

/// 清掉旧 SkillService SSOT / 旧 skill 名，避免 Codex 同时发现两个 dispatch skill。
pub fn remove_legacy_workflow_skill_dirs() {
    let skills = codex_skills_dir();
    let _ = std::fs::remove_dir_all(skills.join(LEGACY_WORKFLOW_SKILL_DIRECTORY));
    let leftover_roots = [
        crate::config::get_app_config_dir().join("skills"),
        crate::config::get_home_dir().join(".agents").join("skills"),
    ];
    for root in leftover_roots {
        let _ = std::fs::remove_dir_all(root.join(WORKFLOW_SKILL_DIRECTORY));
        let _ = std::fs::remove_dir_all(root.join(LEGACY_WORKFLOW_SKILL_DIRECTORY));
    }
}

pub fn uninstall_workflow_skill_files() {
    remove_legacy_workflow_skill_dirs();
    let _ = std::fs::remove_dir_all(codex_skills_dir().join(WORKFLOW_SKILL_DIRECTORY));
}

pub fn workflow_skill_path() -> Result<PathBuf, AppError> {
    Ok(workflow_skill_dir()?.join("SKILL.md"))
}

/// 读取当前生成的 Workflow Skill 内容；文件不存在返回 None，其他 IO 错误原样返回。
pub fn workflow_skill_content() -> Result<Option<String>, AppError> {
    read_optional_text(&workflow_skill_path()?)
}

/// 解析 Workflow Skill 的 frontmatter 元数据（name / description）。
pub fn workflow_skill_metadata(content: &str) -> (Option<String>, Option<String>) {
    #[derive(Deserialize)]
    struct SkillMetadata {
        name: Option<String>,
        description: Option<String>,
    }
    let content = content.trim_start_matches('\u{feff}');
    let parts: Vec<&str> = content.splitn(3, "---").collect();
    if parts.len() < 3 {
        return (None, None);
    }
    let metadata: Option<SkillMetadata> = serde_yaml::from_str(parts[1].trim()).ok();
    metadata
        .map(|metadata| (metadata.name, metadata.description))
        .unwrap_or((None, None))
}

/// 生成 Workflow Skill 的 SKILL.md：frontmatter 只含 name/description，
/// 正文按角色列出已注册 subagent 名称（不写入路径、model、reasoning）。
/// 派出时 `agent_type` 只用内置角色；Cube 代理从对应 TOML 注入 model / reasoning_effort。
pub fn workflow_skill_markdown(
    agents: &[SubagentRecord],
    role_agents: &RoleAgents,
    worker_agent: &str,
) -> String {
    let mut output = String::new();
    output.push_str(&format!(
        "---\n\
name: {WORKFLOW_SKILL_NAME}\n\
description: Use for tasks that can be split into independent, bounded subtasks that \
should be delegated to registered Codex subagents, or when multiple registered subagents \
have distinct specialties. When this skill is active (for example after @cube-dispatch), \
dispatch registered workers by default instead of doing the work inline; only genuinely \
trivial single-step requests stay inline.\n\
---\n\n"
    ));
    output.push_str("# Cube Dispatch\n\n");
    output.push_str(
        "Coordinator/worker delegation protocol for Codex-Cube. This skill is installed \
into Codex skills and is never injected into AGENTS.md.\n\n",
    );
    output.push_str(
        "## Purpose\n\
- Enable the coordinator to delegate bounded implementation, review, exploration, testing, \
and research subtasks to registered subagents.\n\
- Keep the main thread focused on planning, integration, and acceptance.\n\n",
    );
    output.push_str(
        "## Prerequisites\n\
- Custom subagents require local routing (local proxy takeover for Codex). multi-agent v2 \
encrypts dispatched messages, so without local routing workers receive empty or unreadable tasks.\n\
- Registered agents live in `~/.codex/agents/*.toml`. Codex Cube reads those files at spawn \
time. The coordinator does not pass file paths to `spawn_agent`.\n\n",
    );
    output.push_str(
        "## Roles\n\
- The coordinator - the model running the current conversation - owns requirements, planning, \
architecture, task decomposition, integration, and final review (acceptance).\n\
- A worker executes one delegated implementation/testing scope and reports results.\n\
- An explorer investigates a scoped question read-only (codebase questions, research, triage, \
review) and reports findings and evidence.\n\
- A default (general-purpose) agent handles bounded tasks that match neither role.\n\
- Top-level decisions are not delegated to subagents.\n\n",
    );
    output.push_str(
        "## Triggering\n\
- `@cube-dispatch` in the user's message activates this skill. While it is active, delegate \
suitable bounded work to a registered worker instead of doing it inline; do not treat \
delegation as optional for tasks that match the delegation rules below. On activation, run \
the Dispatch flow immediately: decompose the request, spawn workers, then integrate. Do not \
ask for confirmation and do not do the subtask work inline first.\n\
- Current local Codex releases spawn subagents after a direct request or when an applicable \
skill or AGENTS.md instruction asks for it. The app surfaces each subagent thread for inspection.\n\n",
    );
    output.push_str(
        "## Dispatch flow (execute on activation)\n\
1. Decompose the user's request into independent, bounded subtasks; include one subtask per \
distinct deliverable.\n\
2. Spawn one worker per subtask immediately and in parallel (non-overlapping scopes) before \
doing any subtask work inline.\n\
3. Make each worker task self-contained: concrete scope, target files/behaviors, acceptance \
criteria, and tests to run. Never send a task that only says \"implement the delegated scope\" \
without concrete scope.\n\
4. Wait for all workers, then integrate, verify with tests, and report.\n\n",
    );
    output.push_str("## Registered subagents\n");
    output.push_str(
        "- Roles are assigned in Codex Cube: each registered subagent can serve one or more \
of the `worker` / `explorer` / `default` roles, and the same subagent may be listed under \
multiple roles.\n",
    );
    output.push_str(
        "- Spawn `agent_type` as the role, not as the bullet name. Codex Cube binds the first \
listed agent for that role and injects `model` / `reasoning_effort` from its registration \
file. This document must not duplicate model names, reasoning levels, or a default worker.\n",
    );
    let mut listed_any_role = false;
    for (role, names) in [
        ("worker", &role_agents.worker),
        ("explorer", &role_agents.explorer),
        ("default", &role_agents.default),
    ] {
        if names.is_empty() {
            continue;
        }
        listed_any_role = true;
        output.push_str(&format!("- **{role}:**\n"));
        for name in names {
            let Some(agent) = agents.iter().find(|a| &a.name == name) else {
                continue;
            };
            output.push_str(&format!("  - `{}`\n", agent.name));
        }
    }
    if !listed_any_role && !worker_agent.trim().is_empty() {
        output.push_str(&format!(
            "- No role mapping is currently listed. Spawn `agent_type=\"worker\"`; Codex Cube \
binds `{worker_agent}`.\n"
        ));
    }
    output.push('\n');
    output.push_str(
        "## When to delegate\n\
- By default, delegate every bounded subtask identified by the Dispatch flow; do not keep \
work inline just because it is quick.\n\
- Keep inline only genuinely trivial single-step changes (one obvious edit with no independent \
decision), decisions reserved for the coordinator, or work that requires the coordinator's \
own context.\n\
- Parallelize independent subtasks by default: spawn all workers before waiting, one worker \
per non-overlapping scope. Prefer parallel subagents for read-heavy work (exploration, tests, \
triage, summarization).\n\n",
    );
    output.push_str(
        "## Delegation rules\n\
- Delegate suitable bounded implementation, review, exploration, testing, or research subtasks.\n\
- Pick the subagent by registered role: implementation/testing and other write work → the \
`worker` role list (first listed agent wins); exploration, codebase questions, review, triage, \
research → the `explorer` role list; bounded tasks matching neither → the `default` role list.\n\
- If the preferred role list is empty or its agents are unavailable, fall back to the next \
role list (`worker` → `default` → `explorer`), then to the default worker.\n\
- Assign explicit, non-overlapping write scopes before spawning.\n\
- Wait for all required results and keep the main thread focused on planning and review.\n\n",
    );
    output.push_str(
        "## Spawning protocol\n\
- Start each multi-round task with a NEW subagent (`fork_turns=\"none\"`); never substitute \
the coordinator's own model.\n\
- Use the subagent-spawning tool actually exposed by this session: `spawn_agent` / \
`collaboration__spawn_agent` (then `wait_agent` / `followup_task` / `send_message`), or the \
app thread tools (`create_thread` + `send_message_to_thread` + `wait_threads`) when those \
are the only subagent tools exposed. Never call a tool that is not in the current tool list.\n\
- Pass only these `spawn_agent` arguments:\n\
  `agent_type`: `worker` for write/implementation, `explorer` for read-only investigation, \
`default` otherwise.\n\
  `fork_turns`: `\"none\"`.\n\
  `message`: the self-contained task.\n\
  `task_name`: a short instance id for this child, not an agent selector.\n\
- Do not pass a registered bullet name, a `.toml` path, or a model id as `agent_type`. That \
returns \"agent type is currently not available\".\n\
- Do not pass `model`, `reasoning_effort`, or `model_reasoning_effort`. Codex Cube injects \
`model` and `reasoning_effort` from the first listed agent's registration for that role. \
If the child's `turn_context.model` does not match that registration, treat the dispatch \
as failed.\n\
- Never invent or pass an `agent_type` value absent from the current schema.\n\n",
    );

    output.push_str(
        "## Troubleshooting\n\
- If a worker reports receiving only wrapper/developer instructions (for example \
\"implement only the delegated scope…\") with no task scope attached, the dispatched message \
was not delivered as plaintext: local routing (Codex Cube proxy takeover for Codex) must be \
enabled for custom-provider workers. Tell the user to enable local routing and retry; do not \
re-dispatch in a loop.\n\
- If the session exposes no subagent-spawning tool at all, state that this Codex surface \
cannot spawn subagents and list the missing tool, instead of fabricating one.\n\n",
    );
    output.push_str(
        "## Worker lifecycle\n\
- While a worker subagent is still open - even if its previous round was completed - \
continue the next round with `followup_task` to keep the same registered model and context.\n\
- Do not close the worker after each round; close it only after the final review accepts the work.\n\
- If a closed worker must continue, create a new subagent from the same registered agent \
with a concise handoff of the previous round's conclusions.\n\n",
    );
    output.push_str(
        "## Worker behavior\n\
- Workers inherit the parent session's sandbox permission mode unless the custom agent file \
sets `sandbox_mode`.\n\
- Stay within the delegated scope and directly edit the shared workspace files (not only \
propose patches).\n\
- Report changed files, tests, and remaining risks.\n\
- Do not assign overlapping write scopes.\n\n",
    );
    output.push_str(
        "## Global settings\n\
- Codex-level `[agents]` settings control subagent behavior such as `enabled`, \
`max_concurrent_threads_per_session`, and `interrupt_message`. cube-dispatch routing does not \
use `[agents].default_subagent_model` or `default_subagent_reasoning_effort`; the registered \
agent TOML and the proxy rewrite are authoritative.\n\n",
    );
    output.push_str(
        "## Coordination & acceptance\n\
- The coordinator must review the actual diff and run integrated tests before accepting the work.\n\
- Do not accept work until the worker's report matches the verified state.\n\
- Close the worker only after acceptance; continue in the main thread.\n\n",
    );
    output.push_str(
        "## Explicit invocation\n\
- Invoke this workflow explicitly in any Codex conversation with `@cube-dispatch` \
(or the skill's display name) to force delegation behavior even when it would not auto-trigger.\n\
- The skill is installed into Codex skills (for example `~/.codex/skills/cube-dispatch/`) \
and is not injected into AGENTS.md.\n",
    );
    output
}

/// 生成 Workflow Skill 的 agents/openai.yaml（仅 interface + policy 字段）。
pub fn workflow_skill_openai_yaml() -> String {
    // 注意：不能用 `\` 续行——Rust 字符串续行会吞掉下一行的前导空格，
    // 导致 `interface:`/`policy:` 的子键被解析成顶层键（interface/policy 变 null）。
    // 必须用显式 `\n  ` 转义保留缩进。
    "interface:\n  display_name: \"Cube Dispatch\"\n  short_description: \"Delegate bounded subtasks to registered subagents\"\n  default_prompt: \"Use @cube-dispatch to dispatch independent subtasks to registered subagents.\"\npolicy:\n  allow_implicit_invocation: true\n"
        .to_string()
}

/// Read the currently injected managed block for the UI preview.
///
/// This intentionally does not require a valid workflow manifest or a matching
/// worker: the preview should expose the actual AGENTS.md content even when it
/// is stale or was edited after installation.
pub fn current_managed_block(config_dir: &Path) -> Result<Option<String>, AppError> {
    let Some(text) = read_optional_text(&instructions_path(config_dir))? else {
        return Ok(None);
    };
    Ok(managed_block_range(&text).map(|(start, end)| text[start..end].to_owned()))
}

pub fn validate_agent_name(name: &str) -> Result<(), AppError> {
    let trimmed = name.trim();
    let valid = !trimmed.is_empty()
        && trimmed.len() <= 64
        && trimmed.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && (byte == b'_' || byte == b'-'))
        });
    if !valid {
        return Err(AppError::InvalidInput(format!(
            "agent 名称 {name:?} 不是安全文件名（要求 [A-Za-z0-9][A-Za-z0-9_-]{{0,63}}）"
        )));
    }
    Ok(())
}

/// 安装 Workflow，并把当前默认 worker 的模型/推理档位同步到 Codex fallback 配置。
/// 安装失败时恢复安装前的 config.toml 与 fallback 备份状态。
pub fn install_with_agent_defaults(
    config_dir: &Path,
    worker_agent: &str,
    worker_agents: &[String],
    role_agents: &RoleAgents,
    mode: &str,
    reasoning_effort: &str,
    default_model: &str,
) -> Result<(), AppError> {
    let config_path = config_dir.join("config.toml");
    let original_config = read_optional_text(&config_path)?;
    let defaults_backup = agent_defaults_backup_path(config_dir);
    let original_defaults_backup = read_optional_text(&defaults_backup)?;
    if let Err(error) = sync_agent_defaults(config_dir, default_model, reasoning_effort) {
        restore_optional_file(&config_path, original_config.as_deref()).ok();
        restore_optional_file(&defaults_backup, original_defaults_backup.as_deref()).ok();
        return Err(error);
    }
    if let Err(error) = install(
        config_dir,
        worker_agent,
        worker_agents,
        role_agents,
        mode,
        reasoning_effort,
    ) {
        restore_optional_file(&config_path, original_config.as_deref()).ok();
        restore_optional_file(&defaults_backup, original_defaults_backup.as_deref()).ok();
        return Err(error);
    }
    Ok(())
}

pub fn install(
    config_dir: &Path,
    worker_agent: &str,
    worker_agents: &[String],
    role_agents: &RoleAgents,
    mode: &str,
    reasoning_effort: &str,
) -> Result<(), AppError> {
    validate_agent_name(worker_agent)?;
    let worker_agent = worker_agent.trim();
    let instructions_file = instructions_path(config_dir);
    let manifest_file = manifest_path(config_dir);
    let backup_file = backup_path(config_dir);
    let existing_instructions = read_optional_text(&instructions_file)?;
    let (next_instructions, manifest) = if mode == WORKFLOW_MODE_SKILL {
        // 新式安装完全不修改 AGENTS.md：仅当历史安装残留受管块时才清理（移除受管块），
        // 其余情况不写入，用户已有内容原样保留。
        let next_instructions = existing_instructions
            .as_deref()
            .filter(|text| managed_block_range(text).is_some())
            .map(remove_managed_block);
        let manifest = WorkflowManifest {
            version: 1,
            worker_agent: worker_agent.to_owned(),
            worker_agents: worker_agents.to_vec(),
            role_agents: role_agents.clone(),
            mode: WORKFLOW_MODE_SKILL.to_string(),
            skill_directory: WORKFLOW_SKILL_DIRECTORY.to_string(),
        };
        (next_instructions, manifest)
    } else {
        let next_instructions = Some(upsert_managed_block(
            existing_instructions.as_deref().unwrap_or_default(),
            worker_agent,
            reasoning_effort,
        ));
        let manifest = WorkflowManifest {
            version: 1,
            worker_agent: worker_agent.to_owned(),
            worker_agents: worker_agents.to_vec(),
            role_agents: role_agents.clone(),
            mode: WORKFLOW_MODE_AGENTS_MD.to_string(),
            skill_directory: WORKFLOW_SKILL_DIRECTORY.to_string(),
        };
        (next_instructions, manifest)
    };

    // Validate all generated data before taking the first snapshot.
    serde_json::to_string(&manifest).map_err(|source| AppError::JsonSerialize { source })?;
    let original_manifest = read_optional_text(&manifest_file)?;
    let backup_created = read_backup(config_dir)?.is_none();
    if backup_created {
        let backup = WorkflowBackup {
            instructions_md: existing_instructions.clone(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        write_private_json(&backup_file, &backup)?;
    }

    if let Err(error) = (|| -> Result<(), AppError> {
        if let Some(next_instructions) = &next_instructions {
            write_text_file(&instructions_file, next_instructions)?;
        }
        write_json_file(&manifest_file, &manifest)
    })() {
        restore_optional_file(&instructions_file, existing_instructions.as_deref()).ok();
        restore_optional_file(&manifest_file, original_manifest.as_deref()).ok();
        if backup_created {
            delete_file(&backup_file).ok();
        }
        return Err(error);
    }
    Ok(())
}

pub fn cancel(config_dir: &Path) -> Result<(), AppError> {
    let instructions_file = instructions_path(config_dir);
    let backup_file = backup_path(config_dir);
    // AGENTS.md 不可读时直接失败，不能当作不存在而默删/改写。
    let existing = read_optional_text(&instructions_file)?;
    if let Some(backup) = read_backup(config_dir)? {
        restore_optional_file(&instructions_file, backup.instructions_md.as_deref())?;
        restore_agent_defaults(config_dir)?;
        delete_file(&manifest_path(config_dir))?;
        delete_file(&backup_file)?;
        return Ok(());
    }

    let without = remove_managed_block(existing.as_deref().unwrap_or_default());
    if without.is_empty() {
        delete_file(&instructions_file)?;
    } else {
        write_text_file(&instructions_file, &without)?;
    }
    restore_agent_defaults(config_dir)?;
    delete_file(&manifest_path(config_dir))
}

fn restore_optional_file(path: &Path, contents: Option<&str>) -> Result<(), AppError> {
    match contents {
        Some(contents) => write_text_file(path, contents),
        None => delete_file(path),
    }
}

/// 私密原子写入：Unix 下临时文件从 create_new 起即为 0600（与目标同目录），
/// 写入并 flush/sync 后原子 rename，避免"先默认权限创建再 chmod"的暴露窗口。
/// Windows 下 std::fs::rename 使用 MOVEFILE_REPLACE_EXISTING，可安全覆盖。
fn write_private_file_atomic(path: &Path, data: &[u8]) -> Result<(), AppError> {
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

    if let Err(source) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(AppError::io(path, source));
    }
    Ok(())
}

fn write_private_json<T: Serialize>(path: &Path, data: &T) -> Result<(), AppError> {
    let bytes =
        serde_json::to_vec(data).map_err(|error| AppError::JsonSerialize { source: error })?;
    write_private_file_atomic(path, &bytes)
}

pub fn managed_block_matches(existing: &str, worker_agent: &str, reasoning_effort: &str) -> bool {
    let Some((start, end)) = managed_block_range(existing) else {
        return false;
    };
    existing[start..end].trim_end() == managed_block(worker_agent, reasoning_effort).trim_end()
}

pub fn upsert_managed_block(existing: &str, worker_agent: &str, reasoning_effort: &str) -> String {
    let block = managed_block(worker_agent, reasoning_effort);
    match managed_block_range(existing) {
        Some((start, end)) => {
            let mut output = String::with_capacity(existing.len() + block.len());
            output.push_str(&existing[..start]);
            output.push_str(&block);
            output.push_str(&existing[end..]);
            output
        }
        None if existing.is_empty() => block,
        None => {
            let mut output = existing.to_owned();
            if !output.ends_with('\n') {
                output.push('\n');
            }
            if !output.ends_with("\n\n") {
                output.push('\n');
            }
            output.push_str(&block);
            output
        }
    }
}

pub fn managed_block(worker_agent: &str, reasoning_effort: &str) -> String {
    format!(
        "{MANAGED_BLOCK_BEGIN_MARKER}\n\
## Coordinator / Worker Workflow\n\
The coordinator - the model you are currently using in this conversation -\n\
owns requirements, planning, architecture, task decomposition, integration,\n\
and final review (acceptance). These top-level decisions are not delegated\n\
to workers. For non-trivial implementation tasks with clear boundaries, the\n\
coordinator may proactively delegate them to the registered\n\
`{worker_agent}` agent without requiring the user to explicitly request\n\
delegation. The default workflow uses this single worker; advanced users\n\
may register multiple subagents and define their own role mapping in\n\
AGENTS.md to pick different workers per role. For parallel independent\n\
tasks, dispatch each to a separate non-overlapping worker instead of\n\
serializing them all through one worker. Start each multi-round task by\n\
creating a NEW subagent from the registered worker agent with\n\
fork_context=false; never substitute the coordinator's own current model\n\
for the registered worker. While a worker subagent is still open - even\n\
if its previous round was completed - continue the next round with\n\
send_input to keep the same registered model and context. Do not close\n\
the worker after each round; close_agent only after the final review\n\
accepts the work. A closed worker may be resumed only by a resume path\n\
that can explicitly replay the registered worker's model and config; the\n\
current resume_agent wrapper has no model/agent override, so it could\n\
drift to the coordinator's current model and must not resume a closed\n\
worker. If a closed worker needs to continue, create a new subagent from\n\
the same registered `{worker_agent}` agent with a concise handoff of the\n\
previous round's conclusions. When spawning the worker, pass\n\
`reasoning_effort` explicitly with the registered value\n\
(`{reasoning_effort}`) - the agent TOML's model_reasoning_effort is\n\
otherwise ignored by spawn_agent, so without the explicit override the\n\
worker silently inherits the coordinator's current effort level. Use the\n\
same registered reasoning_effort on every round and when resuming; never\n\
let the worker run at a different effort than registered. Workers inherit\n\
the parent session's sandbox permission mode - the registered agent TOML\n\
does not set a custom sandbox. Workers must stay within their delegated\n\
scope, directly edit the shared workspace files (not only output patches\n\
or proposed content), and report changed files, tests, and remaining\n\
risks. Do not assign overlapping write scopes. The coordinator must\n\
review the actual diff and run integrated tests before accepting the\n\
work.\n\
{MANAGED_BLOCK_END_MARKER}\n"
    )
}

pub fn remove_managed_block(existing: &str) -> String {
    match managed_block_range(existing) {
        Some((start, end)) => {
            let mut output = String::with_capacity(existing.len() - (end - start));
            output.push_str(&existing[..start]);
            output.push_str(&existing[end..]);
            output
        }
        None => existing.to_owned(),
    }
}

fn managed_block_range(existing: &str) -> Option<(usize, usize)> {
    let start = existing.find(MANAGED_BLOCK_BEGIN_MARKER)?;
    let mut end = existing[start..]
        .find(MANAGED_BLOCK_END_MARKER)
        .map(|index| start + index + MANAGED_BLOCK_END_MARKER.len())?;
    if existing[end..].starts_with("\r\n") {
        end += 2;
    } else if existing[end..].starts_with('\n') {
        end += 1;
    }
    (start < end).then_some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_install_keeps_first_agents_md_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(instructions_path(temp.path()), "original\n").unwrap();

        install(
            temp.path(),
            "worker-a",
            &["worker-a".to_string()],
            &RoleAgents::default(),
            WORKFLOW_MODE_AGENTS_MD,
            "high",
        )
        .unwrap();
        install(
            temp.path(),
            "worker-b",
            &["worker-b".to_string()],
            &RoleAgents::default(),
            WORKFLOW_MODE_AGENTS_MD,
            "high",
        )
        .unwrap();
        cancel(temp.path()).unwrap();

        assert_eq!(
            std::fs::read_to_string(instructions_path(temp.path())).unwrap(),
            "original\n"
        );
        assert!(!manifest_path(temp.path()).exists());
        assert!(!backup_path(temp.path()).exists());
    }

    #[test]
    fn cancel_removes_agents_md_when_it_did_not_exist_initially() {
        let temp = tempfile::tempdir().unwrap();
        install(
            temp.path(),
            "worker",
            &["worker".to_string()],
            &RoleAgents::default(),
            WORKFLOW_MODE_AGENTS_MD,
            "high",
        )
        .unwrap();
        assert!(managed_block_matches(
            &std::fs::read_to_string(instructions_path(temp.path())).unwrap(),
            "worker",
            "high"
        ));

        cancel(temp.path()).unwrap();
        assert!(!instructions_path(temp.path()).exists());
    }

    #[test]
    fn sync_and_restore_agent_defaults_preserve_existing_agents_settings() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("config.toml");
        std::fs::write(
            &config,
            "model = \"gpt-5.6-sol\"\n\n[agents]\nenabled = true\ndefault_subagent_model = \"old-model\"\ndefault_subagent_reasoning_effort = \"low\"\n",
        )
        .unwrap();

        sync_agent_defaults(temp.path(), "deepseek-v4-flash", "max").unwrap();
        let synced = std::fs::read_to_string(&config).unwrap();
        assert!(synced.contains("enabled = true"));
        assert!(synced.contains("default_subagent_model = \"deepseek-v4-flash\""));
        assert!(synced.contains("default_subagent_reasoning_effort = \"max\""));
        assert!(agent_defaults_backup_path(temp.path()).exists());

        restore_agent_defaults(temp.path()).unwrap();
        let restored = std::fs::read_to_string(&config).unwrap();
        assert!(restored.contains("enabled = true"));
        assert!(restored.contains("default_subagent_model = \"old-model\""));
        assert!(restored.contains("default_subagent_reasoning_effort = \"low\""));
        assert!(!agent_defaults_backup_path(temp.path()).exists());
    }

    #[test]
    fn sync_and_restore_agent_defaults_remove_generated_config_when_absent_initially() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("config.toml");

        sync_agent_defaults(temp.path(), "deepseek-v4-flash", "max").unwrap();
        assert!(config.exists());
        restore_agent_defaults(temp.path()).unwrap();

        assert!(!config.exists());
        assert!(!agent_defaults_backup_path(temp.path()).exists());
    }

    #[test]
    fn sync_agent_defaults_rejects_invalid_agents_value_without_backup() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("config.toml");
        std::fs::write(&config, "agents = \"not-a-table\"\n").unwrap();

        let error = sync_agent_defaults(temp.path(), "deepseek-v4-flash", "max").unwrap_err();
        assert!(error.to_string().contains("[agents]"));
        assert_eq!(
            std::fs::read_to_string(&config).unwrap(),
            "agents = \"not-a-table\"\n"
        );
        assert!(!agent_defaults_backup_path(temp.path()).exists());
    }

    #[test]
    fn install_with_agent_defaults_rolls_back_sync_when_workflow_install_fails() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("config.toml");
        std::fs::write(&config, "model = \"gpt-5.6-sol\"\n").unwrap();

        let error = install_with_agent_defaults(
            temp.path(),
            "../invalid",
            &[],
            &RoleAgents::default(),
            WORKFLOW_MODE_SKILL,
            "max",
            "deepseek-v4-flash",
        )
        .unwrap_err();
        assert!(error.to_string().contains("agent 名称"));
        assert_eq!(
            std::fs::read_to_string(&config).unwrap(),
            "model = \"gpt-5.6-sol\"\n"
        );
        assert!(!agent_defaults_backup_path(temp.path()).exists());
        assert!(!manifest_path(temp.path()).exists());
    }

    #[test]
    fn cancel_restores_agent_defaults_on_workflow_backup_path() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("config.toml");
        std::fs::write(
            &config,
            "[agents]\nenabled = true\ndefault_subagent_model = \"old-model\"\ndefault_subagent_reasoning_effort = \"low\"\n",
        )
        .unwrap();

        install_with_agent_defaults(
            temp.path(),
            "worker",
            &["worker".to_string()],
            &RoleAgents::default(),
            WORKFLOW_MODE_SKILL,
            "max",
            "deepseek-v4-flash",
        )
        .unwrap();
        assert!(agent_defaults_backup_path(temp.path()).exists());

        cancel(temp.path()).unwrap();
        let restored = std::fs::read_to_string(&config).unwrap();
        assert!(restored.contains("enabled = true"));
        assert!(restored.contains("default_subagent_model = \"old-model\""));
        assert!(restored.contains("default_subagent_reasoning_effort = \"low\""));
        assert!(!agent_defaults_backup_path(temp.path()).exists());
    }

    #[test]
    fn install_preserves_existing_content_and_replaces_managed_worker() {
        let once = upsert_managed_block("user content\n", "worker-a", "high");
        let twice = upsert_managed_block(&once, "worker-b", "high");
        assert!(twice.starts_with("user content\n"));
        assert!(managed_block_matches(&twice, "worker-b", "high"));
        assert!(!twice.contains("`worker-a`"));
    }

    #[test]
    fn managed_block_removal_lf_leaves_no_extra_newline() {
        let block = managed_block("worker-a", "high");
        // 块自带尾部换行：移除时必须连该换行一起消耗，不能残留空行。
        let with_trailing = format!("user content\n{block}trailing\n");
        assert_eq!(
            remove_managed_block(&with_trailing),
            "user content\ntrailing\n"
        );
        // 块在文件末尾时同样不留多余换行。
        let at_eof = format!("user content\n{block}");
        assert_eq!(remove_managed_block(&at_eof), "user content\n");
    }

    #[test]
    fn managed_block_removal_crlf_leaves_no_extra_newline() {
        let block = "# >>> codex-cube managed block >>>\r\ninstructions\r\n# <<< codex-cube managed block <<<\r\n";
        let existing = format!("user content\r\n{block}trailing\r\n");
        assert_eq!(
            remove_managed_block(&existing),
            "user content\r\ntrailing\r\n"
        );
    }

    #[test]
    fn managed_block_keeps_open_worker_across_rounds_and_only_bans_resuming_closed_workers() {
        let block = managed_block("deepseek-flash", "max");
        // 展平换行后断言语义约束（块文本按行折行，短语可能跨行）。
        let flat = block.split_whitespace().collect::<Vec<_>>().join(" ");
        // 主 agent 负责规划/架构/拆分/集成/验收，顶层决策不下放。
        assert!(flat.contains(
            "owns requirements, planning, architecture, task decomposition, integration, and final review"
        ));
        assert!(flat.contains("These top-level decisions are not delegated to workers"));
        // 每个多轮任务从已注册 worker 新建 subagent，fork_context=false。
        assert!(flat.contains("registered `deepseek-flash`"));
        assert!(flat.contains("NEW subagent"));
        assert!(flat.contains("fork_context=false"));
        assert!(flat.contains("Start each multi-round task"));
        // 默认 workflow 只选一个 worker；高级用户可在 AGENTS.md 自定义多模型角色。
        assert!(flat.contains("The default workflow uses this single worker"));
        assert!(flat.contains("register multiple subagents"));
        assert!(flat.contains("role mapping in AGENTS.md"));
        // 并行独立任务派多个不重叠 worker，不全串给一个。
        assert!(flat.contains("parallel independent tasks"));
        assert!(flat.contains("separate non-overlapping worker"));
        assert!(flat.contains("instead of serializing them all through one worker"));
        // worker 仍 open 时（即使上一轮 completed）用 send_input 继续，保持同一模型和上下文。
        assert!(flat.contains("still open - even if its previous round was completed"));
        assert!(flat.contains("continue the next round with send_input"));
        assert!(flat.contains("keep the same registered model and context"));
        // 不要每轮 close，最终验收后才 close_agent。
        assert!(flat.contains("Do not close the worker after each round"));
        assert!(flat.contains("close_agent only after the final review accepts the work"));
        // 能力边界：仅当 resume 路径能显式重放注册 worker 的 model/config 才可恢复；
        // 当前 resume_agent wrapper 无 model/agent override，可能漂移到 coordinator 当前模型。
        assert!(flat.contains(
            "A closed worker may be resumed only by a resume path that can explicitly replay the registered worker's model and config"
        ));
        assert!(flat.contains("current resume_agent wrapper has no model/agent override"));
        assert!(flat.contains("could drift to the coordinator's current model"));
        assert!(flat.contains("must not resume a closed worker"));
        // closed worker 需继续时：从同一 registered worker 新建 subagent 并传精简 handoff。
        assert!(flat.contains("If a closed worker needs to continue, create a new subagent"));
        assert!(flat.contains("same registered `deepseek-flash`"));
        assert!(flat.contains("handoff of the previous round's conclusions"));
        // 推理档位必须与注册值一致：spawn 时显式传 registered reasoning_effort。
        assert!(flat.contains("When spawning the worker, pass `reasoning_effort` explicitly"));
        assert!(flat.contains("with the registered value (`max`)"));
        assert!(flat.contains("model_reasoning_effort is otherwise ignored by spawn_agent"));
        assert!(flat.contains("never let the worker run at a different effort than registered"));
        // 永远不得用 coordinator 当前 model 替代注册 worker。
        assert!(flat.contains("never substitute the coordinator's own current model"));
        // sandbox 继承 parent；注册 TOML 不写自定义 sandbox。
        assert!(flat.contains("inherit the parent session's sandbox permission mode"));
        assert!(flat.contains("does not set a custom sandbox"));
        // worker 直接改共享工作区并报告文件/测试/风险。
        assert!(flat.contains("directly edit the shared workspace files"));
        assert!(flat.contains("report changed files, tests, and remaining risks"));
        // 旧语义不再存在：不再要求每轮新建 fresh worker，也不再笼统禁止 resume completed worker。
        assert!(!flat.contains("fresh worker each round"));
        assert!(!flat.contains("never resume a closed or completed worker"));
    }

    #[test]
    fn managed_block_upsert_does_not_accumulate_newlines() {
        // LF 文件：替换旧块后不叠加换行。
        let existing_lf = format!(
            "user content\n{}trailing\n",
            managed_block("worker-a", "high")
        );
        let replaced_lf = upsert_managed_block(&existing_lf, "worker-b", "high");
        assert_eq!(
            replaced_lf,
            format!(
                "user content\n{}trailing\n",
                managed_block("worker-b", "high")
            )
        );
        assert!(!replaced_lf.contains("\n\n"));

        // CRLF 文件：旧块尾部 \r\n 被消耗，替换后不残留空行。
        let crlf_block = "# >>> codex-cube managed block >>>\r\ninstructions\r\n# <<< codex-cube managed block <<<\r\n";
        let existing_crlf = format!("user content\r\n{crlf_block}trailing\r\n");
        let replaced_crlf = upsert_managed_block(&existing_crlf, "worker-b", "high");
        assert_eq!(
            replaced_crlf,
            format!(
                "user content\r\n{}trailing\r\n",
                managed_block("worker-b", "high")
            )
        );
        assert!(!replaced_crlf.contains("\n\n"));
    }

    #[test]
    fn invalid_agent_names_are_rejected_before_backup() {
        let temp = tempfile::tempdir().unwrap();
        assert!(install(
            temp.path(),
            "../bad",
            &["../bad".to_string()],
            &RoleAgents::default(),
            WORKFLOW_MODE_SKILL,
            "high"
        )
        .is_err());
        assert!(!backup_path(temp.path()).exists());
    }

    #[cfg(unix)]
    #[test]
    fn backup_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        install(
            temp.path(),
            "worker",
            &["worker".to_string()],
            &RoleAgents::default(),
            WORKFLOW_MODE_SKILL,
            "high",
        )
        .unwrap();
        let mode = std::fs::metadata(backup_path(temp.path()))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        // 原子写入不残留同目录临时文件。
        let leftovers: Vec<_> = std::fs::read_dir(temp.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn install_fails_when_agents_md_is_not_readable() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let path = instructions_path(temp.path());
        std::fs::write(&path, "user private content\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();

        let error = install(
            temp.path(),
            "worker",
            &["worker".to_string()],
            &RoleAgents::default(),
            WORKFLOW_MODE_SKILL,
            "high",
        )
        .unwrap_err();
        assert!(error.to_string().contains("AGENTS.md"));
        // 不可读文件未被改写，且未创建备份/清单。
        assert!(std::fs::read_to_string(&path).is_err());
        assert!(!backup_path(temp.path()).exists());
        assert!(!manifest_path(temp.path()).exists());
    }

    #[cfg(unix)]
    #[test]
    fn cancel_fails_when_agents_md_is_not_readable_without_backup() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let path = instructions_path(temp.path());
        std::fs::write(&path, "user private content\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();

        let error = cancel(temp.path()).unwrap_err();
        assert!(error.to_string().contains("AGENTS.md"));
        // 不可读文件不能被当作空文件删除。
        assert!(path.exists());
        assert!(std::fs::read_to_string(&path).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn cancel_fails_when_agents_md_is_not_readable_with_backup() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let path = instructions_path(temp.path());
        std::fs::write(&path, "original\n").unwrap();
        install(
            temp.path(),
            "worker",
            &["worker".to_string()],
            &RoleAgents::default(),
            WORKFLOW_MODE_SKILL,
            "high",
        )
        .unwrap();
        // AGENTS.md 变为不可读后，cancel 必须失败而不是从备份覆盖。
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();

        let error = cancel(temp.path()).unwrap_err();
        assert!(error.to_string().contains("AGENTS.md"));
        assert!(backup_path(temp.path()).exists(), "备份不得被清理");
        assert!(manifest_path(temp.path()).exists());
        assert!(std::fs::read_to_string(&path).is_err());
    }

    #[test]
    fn install_fails_when_existing_backup_is_corrupt_or_unreadable() {
        // 损坏 JSON → 失败，AGENTS.md 原样、manifest 不创建。
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        std::fs::write(instructions_path(dir), "keep me\n").unwrap();
        std::fs::write(backup_path(dir), "{ not json").unwrap();
        let error = install(
            dir,
            "worker",
            &["worker".to_string()],
            &RoleAgents::default(),
            WORKFLOW_MODE_SKILL,
            "high",
        )
        .unwrap_err();
        assert!(error.to_string().contains("AGENTS.md 备份"));
        assert_eq!(
            std::fs::read_to_string(instructions_path(dir)).unwrap(),
            "keep me\n"
        );
        assert!(!manifest_path(dir).exists());

        // 结构非法（缺 created_at）→ 失败。
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        std::fs::write(backup_path(dir), r#"{"instructionsMd": null}"#).unwrap();
        let error = install(
            dir,
            "worker",
            &["worker".to_string()],
            &RoleAgents::default(),
            WORKFLOW_MODE_SKILL,
            "high",
        )
        .unwrap_err();
        assert!(error.to_string().contains("AGENTS.md 备份"));
        assert!(!instructions_path(dir).exists());
        assert!(!manifest_path(dir).exists());

        // 备份路径是目录（不可读）→ 失败。
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        std::fs::create_dir(backup_path(dir)).unwrap();
        let error = install(
            dir,
            "worker",
            &["worker".to_string()],
            &RoleAgents::default(),
            WORKFLOW_MODE_SKILL,
            "high",
        )
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("codex-cube-workflow-instructions-backup.json"));
        assert!(!instructions_path(dir).exists());
        assert!(!manifest_path(dir).exists());
    }

    #[test]
    fn can_undo_is_strict_about_backup_state() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();

        // 缺失 → false。
        assert!(!can_undo(dir).unwrap());

        // 有效备份 → true。
        let backup = WorkflowBackup {
            instructions_md: Some("original".to_owned()),
            created_at: "2024-01-01T00:00:00Z".to_owned(),
        };
        std::fs::write(backup_path(dir), serde_json::to_string(&backup).unwrap()).unwrap();
        assert!(can_undo(dir).unwrap());

        // 损坏 JSON → 错误。
        std::fs::remove_file(backup_path(dir)).unwrap();
        std::fs::write(backup_path(dir), "{ bad").unwrap();
        assert!(can_undo(dir).is_err());

        // 路径是目录（不可读）→ 错误。
        std::fs::remove_file(backup_path(dir)).unwrap();
        std::fs::create_dir(backup_path(dir)).unwrap();
        assert!(can_undo(dir).is_err());
    }

    #[test]
    fn can_undo_is_true_without_backup_when_managed_block_or_manifest_exists() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();

        // 无备份、无 manifest、无受管块 → false。
        assert!(!can_undo(dir).unwrap());

        // 无备份但 AGENTS.md 含受管块（旧式/手工注入）→ true，cancel 仍可恢复。
        std::fs::write(
            instructions_path(dir),
            "user content\n# >>> codex-cube managed block >>>\nold rules\n# <<< codex-cube managed block <<<\n",
        )
        .unwrap();
        assert!(can_undo(dir).unwrap());

        // 无备份、无受管块但 manifest 存在 → true。
        std::fs::remove_file(instructions_path(dir)).unwrap();
        let manifest = WorkflowManifest {
            version: 1,
            worker_agent: "worker".to_owned(),
            worker_agents: Vec::new(),
            role_agents: RoleAgents::default(),
            mode: WORKFLOW_MODE_AGENTS_MD.to_string(),
            skill_directory: WORKFLOW_SKILL_DIRECTORY.to_string(),
        };
        std::fs::write(
            manifest_path(dir),
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();
        assert!(can_undo(dir).unwrap());

        // 只剩普通 AGENTS.md（无受管块）→ false。
        std::fs::remove_file(manifest_path(dir)).unwrap();
        std::fs::write(instructions_path(dir), "plain content\n").unwrap();
        assert!(!can_undo(dir).unwrap());
    }

    #[test]
    fn current_managed_block_returns_the_injected_text_without_requiring_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let existing = format!(
            "user content\n{}\ntrailing content\n",
            managed_block("worker", "high")
        );
        std::fs::write(instructions_path(dir), &existing).unwrap();

        assert_eq!(
            current_managed_block(dir).unwrap(),
            Some(managed_block("worker", "high"))
        );
        // workflow_uses_agent 只看 manifest 选中集合，与 AGENTS.md 受管块无关。
        assert!(!workflow_uses_agent(dir, "worker").unwrap());
        std::fs::write(
            manifest_path(dir),
            serde_json::to_string(&WorkflowManifest {
                version: 1,
                worker_agent: "worker".to_owned(),
                worker_agents: vec!["worker".to_owned()],
                role_agents: RoleAgents::default(),
                mode: WORKFLOW_MODE_SKILL.to_string(),
                skill_directory: WORKFLOW_SKILL_DIRECTORY.to_string(),
            })
            .unwrap(),
        )
        .unwrap();
        assert!(workflow_uses_agent(dir, "worker").unwrap());
        assert!(!workflow_uses_agent(dir, "other-worker").unwrap());
    }

    #[test]
    fn workflow_usage_comes_from_manifest_selection_and_matches_complete_agent_names() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let manifest = WorkflowManifest {
            version: 1,
            worker_agent: "worker-long".to_owned(),
            worker_agents: vec!["worker-long".to_owned(), "review-worker".to_owned()],
            role_agents: RoleAgents::default(),
            mode: WORKFLOW_MODE_SKILL.to_string(),
            skill_directory: WORKFLOW_SKILL_DIRECTORY.to_string(),
        };
        std::fs::write(
            manifest_path(dir),
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();

        assert!(workflow_uses_agent(dir, "worker-long").unwrap());
        assert!(workflow_uses_agent(dir, "review-worker").unwrap());
        assert!(!workflow_uses_agent(dir, "worker").unwrap());
        assert!(!workflow_uses_agent(dir, "").unwrap());
    }

    #[test]
    fn current_managed_block_is_none_when_agents_file_has_no_managed_block() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        std::fs::write(instructions_path(dir), "user content\n").unwrap();

        assert_eq!(current_managed_block(dir).unwrap(), None);
    }

    #[test]
    fn workflow_state_reads_are_strict_about_errors() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();

        // 缺失 → None/false，不是错误。
        assert_eq!(load_manifest(dir).unwrap(), None);
        assert!(!workflow_uses_agent(dir, "worker").unwrap());

        // 损坏 manifest → 错误，不能当作"未安装"。
        std::fs::write(manifest_path(dir), "{ not json").unwrap();
        assert!(load_manifest(dir).is_err());
        assert!(workflow_uses_agent(dir, "worker").is_err());
        std::fs::remove_file(manifest_path(dir)).unwrap();

        // 不支持的版本 → 错误。
        std::fs::write(
            manifest_path(dir),
            r#"{"version": 2, "workerAgent": "worker"}"#,
        )
        .unwrap();
        assert!(load_manifest(dir).is_err());
        std::fs::remove_file(manifest_path(dir)).unwrap();

        // manifest 路径是目录（不可读）→ 错误。
        std::fs::create_dir(manifest_path(dir)).unwrap();
        assert!(load_manifest(dir).is_err());
        assert!(workflow_uses_agent(dir, "worker").is_err());
        std::fs::remove_dir(manifest_path(dir)).unwrap();

        // manifest 有效且 worker_agents 为空 → 回退 worker_agent，视为选中。
        std::fs::write(
            manifest_path(dir),
            serde_json::to_string(&WorkflowManifest {
                version: 1,
                worker_agent: "worker".to_owned(),
                worker_agents: Vec::new(),
                role_agents: RoleAgents::default(),
                mode: WORKFLOW_MODE_SKILL.to_string(),
                skill_directory: WORKFLOW_SKILL_DIRECTORY.to_string(),
            })
            .unwrap(),
        )
        .unwrap();
        assert!(workflow_uses_agent(dir, "worker").unwrap());
        assert!(!workflow_uses_agent(dir, "other-worker").unwrap());
    }

    #[test]
    fn install_fails_when_existing_workflow_manifest_is_unreadable() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        std::fs::create_dir(manifest_path(dir)).unwrap();

        let error = install(
            dir,
            "worker",
            &["worker".to_string()],
            &RoleAgents::default(),
            WORKFLOW_MODE_SKILL,
            "high",
        )
        .unwrap_err();
        assert!(error.to_string().contains("codex-cube-agent-workflow.json"));
        assert!(!backup_path(dir).exists());
        assert!(!instructions_path(dir).exists());
    }

    fn test_subagent(
        name: &str,
        description: &str,
        model: &str,
        reasoning_effort: &str,
    ) -> SubagentRecord {
        SubagentRecord {
            managed: true,
            available: true,
            name: name.to_string(),
            agent_path: format!("/tmp/agents/{name}.toml"),
            description: description.to_string(),
            model: model.to_string(),
            model_provider_id: "custom".to_string(),
            model_base_url: "https://example.test".to_string(),
            api_key: None,
            sandbox_mode: "inherit".to_string(),
            reasoning_effort: reasoning_effort.to_string(),
            wire_api: "responses".to_string(),
            agent_type: "worker".to_string(),
            cube_provider_id: None,
        }
    }

    #[test]
    fn workflow_skill_markdown_lists_role_mapping_and_default_worker() {
        let agents = vec![
            test_subagent(
                "deepseek-flash",
                "适合前端重构",
                "deepseek-v4-flash",
                "xhigh",
            ),
            test_subagent("gpt-sol-worker", "Balanced worker", "gpt-5.6-sol", "high"),
            test_subagent(
                "explorer-agent",
                "Codebase questions",
                "deepseek-v4-flash",
                "max",
            ),
        ];
        let mut role_agents = RoleAgents::default();
        role_agents.worker = vec!["deepseek-flash".to_string(), "gpt-sol-worker".to_string()];
        role_agents.explorer = vec!["explorer-agent".to_string()];
        let markdown = workflow_skill_markdown(&agents, &role_agents, "deepseek-flash");

        assert!(markdown.starts_with("---\nname: cube-dispatch\n"));
        assert!(markdown.contains("description: Use for tasks"));
        assert!(markdown.contains("delegated to registered Codex subagents"));
        assert!(markdown
            .contains("dispatch registered workers by default instead of doing the work inline"));
        assert!(markdown.contains("- **worker:**"));
        assert!(markdown.contains("  - `deepseek-flash`"));
        assert!(!markdown.contains("/tmp/agents/deepseek-flash.toml"));
        assert!(!markdown.contains("model: deepseek-v4-flash"));
        assert!(!markdown.contains("reasoning: xhigh"));
        assert!(markdown.contains("  - `gpt-sol-worker`"));
        assert!(markdown.contains("- **explorer:**"));
        assert!(markdown.contains("  - `explorer-agent`"));
        assert!(!markdown.contains("- **default:**"));
        assert!(markdown.contains("same subagent may be listed under multiple roles"));
        assert!(!markdown.contains("**Default worker:**"));
        assert!(markdown.contains("must not duplicate model names"));
        assert!(markdown.contains("Spawn `agent_type` as the role, not as the bullet name"));
        assert!(markdown.contains("## Dispatch flow (execute on activation)"));
        assert!(markdown.contains("1. Decompose the user's request"));
        assert!(markdown.contains("2. Spawn one worker per subtask immediately and in parallel"));
        assert!(markdown.contains("## Roles"));
        assert!(markdown.contains("Top-level decisions are not delegated"));
        assert!(markdown.contains("## When to delegate"));
        assert!(markdown.contains("## Delegation rules"));
        assert!(markdown.contains("one worker per non-overlapping scope"));
        assert!(markdown.contains("## Prerequisites"));
        assert!(markdown.contains("~/.codex/agents"));
        assert!(markdown.contains("does not pass file paths"));
        assert!(markdown.contains("local routing"));
        assert!(markdown.contains("## Triggering"));
        assert!(markdown.contains("## Spawning protocol"));
        assert!(markdown.contains("fork_turns=\"none\""));
        assert!(markdown.contains("Pass only these `spawn_agent` arguments"));
        assert!(markdown.contains("`agent_type`: `worker` for write/implementation"));
        assert!(markdown.contains("Do not pass a registered bullet name"));
        assert!(markdown.contains("agent type is currently not available"));
        assert!(markdown.contains("Do not pass `model`, `reasoning_effort`, or `model_reasoning_effort`"));
        assert!(markdown.contains("Codex Cube injects"));
        assert!(markdown.contains("turn_context.model"));
        assert!(markdown.contains("`worker` role list (first listed agent wins)"));
        assert!(markdown
            .contains("Never invent or pass an `agent_type` value absent from the current schema"));
        assert!(markdown.contains("max_concurrent_threads_per_session"));
        assert!(markdown.contains("## Worker lifecycle"));
        assert!(markdown.contains("followup_task"));
        assert!(markdown.contains("## Worker behavior"));
        assert!(markdown.contains("## Global settings"));
        assert!(markdown.contains("directly edit the shared workspace files"));
        assert!(markdown.contains("## Coordination & acceptance"));
        assert!(markdown.contains("## Explicit invocation"));
        assert!(markdown.contains("@cube-dispatch"));
        assert!(markdown.contains("never injected into AGENTS.md"));
    }

    #[test]
    fn workflow_skill_markdown_falls_back_to_default_worker_when_roles_empty() {
        let markdown = workflow_skill_markdown(
            &[test_subagent(
                "empty-worker",
                "   ",
                "deepseek-v4-flash",
                "high",
            )],
            &RoleAgents::default(),
            "empty-worker",
        );

        assert!(markdown.contains(
            "No role mapping is currently listed. Spawn `agent_type=\"worker\"`; Codex Cube binds `empty-worker`."
        ));
        // 空角色映射不列出任何角色分组的 agent。
        assert!(!markdown.contains("- **worker:**"));
        assert!(!markdown.contains("- **explorer:**"));
        assert!(!markdown.contains("- **default:**"));
        assert!(!markdown.contains("**Default worker:**"));
        assert!(markdown.contains("then to the default worker"));
    }

    #[test]
    fn workflow_skill_openai_yaml_contains_required_fields() {
        let yaml = workflow_skill_openai_yaml();
        assert!(yaml.contains("display_name: \"Cube Dispatch\""));
        assert!(yaml
            .contains("short_description: \"Delegate bounded subtasks to registered subagents\""));
        assert!(yaml.contains(
            "default_prompt: \"Use @cube-dispatch to dispatch independent subtasks to registered subagents.\""
        ));
        assert!(yaml.contains("allow_implicit_invocation: true"));
        assert!(!yaml.contains("README"));
    }

    #[test]
    fn workflow_skill_openai_yaml_is_valid_nested_yaml() {
        // `interface:`/`policy:` 的子键必须缩进；若被解析成顶层键，
        // interface/policy 会变成 null，Codex 读不到 display_name 和
        // allow_implicit_invocation（显式触发策略失效）。
        let yaml = workflow_skill_openai_yaml();
        let parsed: serde_yaml::Value =
            serde_yaml::from_str(&yaml).expect("openai.yaml 必须是合法 YAML");
        assert_eq!(
            parsed["interface"]["display_name"].as_str(),
            Some("Cube Dispatch")
        );
        assert_eq!(
            parsed["interface"]["short_description"].as_str(),
            Some("Delegate bounded subtasks to registered subagents")
        );
        assert_eq!(
            parsed["interface"]["default_prompt"].as_str(),
            Some("Use @cube-dispatch to dispatch independent subtasks to registered subagents.")
        );
        assert_eq!(
            parsed["policy"]["allow_implicit_invocation"].as_bool(),
            Some(true)
        );
        // 缩进错误时子键会泄漏到顶层；断言它们不在顶层。
        assert!(parsed.get("display_name").is_none());
        assert!(parsed.get("short_description").is_none());
        assert!(parsed.get("default_prompt").is_none());
        assert!(parsed.get("allow_implicit_invocation").is_none());
    }

    #[test]
    fn install_skill_mode_removes_managed_block_and_cancel_restores() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let original = format!(
            "user content\n{}trailing content\n",
            managed_block("worker-a", "high")
        );
        std::fs::write(instructions_path(dir), &original).unwrap();

        install(
            dir,
            "worker-a",
            &["worker-a".to_string()],
            &RoleAgents::default(),
            WORKFLOW_MODE_SKILL,
            "high",
        )
        .unwrap();

        let after_install = std::fs::read_to_string(instructions_path(dir)).unwrap();
        assert!(!after_install.contains(MANAGED_BLOCK_BEGIN_MARKER));
        assert!(after_install.contains("user content"));
        assert!(after_install.contains("trailing content"));
        let manifest = load_manifest(dir).unwrap().unwrap();
        assert_eq!(manifest.mode, WORKFLOW_MODE_SKILL);
        assert_eq!(manifest.skill_directory, WORKFLOW_SKILL_DIRECTORY);
        assert!(backup_path(dir).exists());

        cancel(dir).unwrap();
        assert_eq!(
            std::fs::read_to_string(instructions_path(dir)).unwrap(),
            original
        );
        assert!(!manifest_path(dir).exists());
        assert!(!backup_path(dir).exists());
    }

    #[test]
    fn install_skill_mode_does_not_create_agents_md_when_missing() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();

        install(
            dir,
            "worker",
            &["worker".to_string()],
            &RoleAgents::default(),
            WORKFLOW_MODE_SKILL,
            "high",
        )
        .unwrap();

        assert!(!instructions_path(dir).exists());
        let manifest = load_manifest(dir).unwrap().unwrap();
        assert_eq!(manifest.mode, WORKFLOW_MODE_SKILL);

        cancel(dir).unwrap();
        assert!(!manifest_path(dir).exists());
        assert!(!backup_path(dir).exists());
        assert!(!instructions_path(dir).exists());
    }

    #[test]
    fn repeated_skill_install_keeps_first_agents_md_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(instructions_path(temp.path()), "original\n").unwrap();

        install(
            temp.path(),
            "worker-a",
            &["worker-a".to_string()],
            &RoleAgents::default(),
            WORKFLOW_MODE_SKILL,
            "high",
        )
        .unwrap();
        install(
            temp.path(),
            "worker-b",
            &["worker-b".to_string()],
            &RoleAgents::default(),
            WORKFLOW_MODE_SKILL,
            "high",
        )
        .unwrap();
        cancel(temp.path()).unwrap();

        assert_eq!(
            std::fs::read_to_string(instructions_path(temp.path())).unwrap(),
            "original\n"
        );
        assert!(!manifest_path(temp.path()).exists());
        assert!(!backup_path(temp.path()).exists());
    }

    #[test]
    fn workflow_uses_agent_checks_manifest_selection() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();

        // 无 manifest → 未选中任何 agent。
        assert!(!workflow_uses_agent(dir, "deepseek-flash").unwrap());

        // 旧 manifest（无 worker_agents）→ 回退到 worker_agent。
        std::fs::write(
            manifest_path(dir),
            serde_json::to_string(&WorkflowManifest {
                version: 1,
                worker_agent: "deepseek-flash".to_owned(),
                worker_agents: Vec::new(),
                role_agents: RoleAgents::default(),
                mode: WORKFLOW_MODE_SKILL.to_string(),
                skill_directory: WORKFLOW_SKILL_DIRECTORY.to_string(),
            })
            .unwrap(),
        )
        .unwrap();
        assert!(workflow_uses_agent(dir, "deepseek-flash").unwrap());
        assert!(!workflow_uses_agent(dir, "other-worker").unwrap());

        // 新 manifest（worker_agents 多选）→ 仅选中集合命中。
        std::fs::write(
            manifest_path(dir),
            serde_json::to_string(&WorkflowManifest {
                version: 1,
                worker_agent: "deepseek-flash".to_owned(),
                worker_agents: vec!["deepseek-flash".to_owned(), "review-worker".to_owned()],
                role_agents: RoleAgents::default(),
                mode: WORKFLOW_MODE_SKILL.to_string(),
                skill_directory: WORKFLOW_SKILL_DIRECTORY.to_string(),
            })
            .unwrap(),
        )
        .unwrap();
        assert!(workflow_uses_agent(dir, "deepseek-flash").unwrap());
        assert!(workflow_uses_agent(dir, "review-worker").unwrap());
        assert!(!workflow_uses_agent(dir, "other-worker").unwrap());
    }

    #[test]
    fn workflow_skill_metadata_parses_frontmatter() {
        let markdown = workflow_skill_markdown(
            &[test_subagent(
                "deepseek-flash",
                "Flash worker",
                "deepseek-v4-flash",
                "xhigh",
            )],
            &RoleAgents::default(),
            "deepseek-flash",
        );
        let (name, description) = workflow_skill_metadata(&markdown);
        assert_eq!(name.as_deref(), Some("cube-dispatch"));
        assert!(description.is_some_and(|description| description.contains("bounded subtasks")));
        assert_eq!(workflow_skill_metadata("no frontmatter"), (None, None));
    }

    #[test]
    fn resolve_dispatch_profile_reads_role_agent_toml() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        std::fs::create_dir_all(dir.join("agents")).unwrap();
        std::fs::write(
            dir.join("agents/grok-4-6.toml"),
            "name = \"grok-4-6\"\nmodel = \"grok-4.6\"\nmodel_reasoning_effort = \"high\"\n",
        )
        .unwrap();
        let mut roles = RoleAgents::default();
        roles.worker = vec!["grok-4-6".to_string()];
        install(
            dir,
            "grok-4-6",
            &["grok-4-6".to_string()],
            &roles,
            WORKFLOW_MODE_SKILL,
            "high",
        )
        .unwrap();

        let worker = resolve_dispatch_profile(dir, Some("worker")).unwrap();
        assert_eq!(worker.agent_name, "grok-4-6");
        assert_eq!(worker.role, "worker");
        assert_eq!(worker.model, "grok-4.6");
        assert_eq!(worker.reasoning_effort, "high");

        let custom = resolve_dispatch_profile(dir, Some("grok-4-6")).unwrap();
        assert_eq!(custom.model, "grok-4.6");
        assert_eq!(
            resolve_dispatch_target(dir, Some("worker")),
            DispatchResolve::Resolved(worker)
        );
    }

    #[test]
    fn resolve_dispatch_target_is_no_workflow_without_manifest() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(
            resolve_dispatch_target(temp.path(), Some("worker")),
            DispatchResolve::NoWorkflow
        );
    }

    #[test]
    fn resolve_dispatch_target_is_unresolved_when_manifest_is_unreadable() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(manifest_path(temp.path()), "{not-json").unwrap();
        assert_eq!(
            resolve_dispatch_target(temp.path(), Some("worker")),
            DispatchResolve::Unresolved {
                agent_type: "worker".to_owned()
            }
        );
    }

    #[test]
    fn resolve_dispatch_profile_defaults_missing_reasoning_effort() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        std::fs::create_dir_all(dir.join("agents")).unwrap();
        std::fs::write(
            dir.join("agents/grok-4-6.toml"),
            "name = \"grok-4-6\"\nmodel = \"grok-4.6\"\n",
        )
        .unwrap();
        let mut roles = RoleAgents::default();
        roles.worker = vec!["grok-4-6".to_string()];
        install(
            dir,
            "grok-4-6",
            &["grok-4-6".to_string()],
            &roles,
            WORKFLOW_MODE_SKILL,
            "high",
        )
        .unwrap();

        let profile = resolve_dispatch_profile(dir, Some("worker")).unwrap();
        assert_eq!(profile.model, "grok-4.6");
        assert_eq!(profile.reasoning_effort, "medium");
    }

    #[test]
    fn restore_or_clear_agent_defaults_restores_backup_on_skill_install() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("config.toml");
        std::fs::write(&config, "model = \"gpt-5.6-sol\"\n").unwrap();
        sync_agent_defaults(temp.path(), "deepseek-v4-flash", "max").unwrap();
        assert!(std::fs::read_to_string(&config)
            .unwrap()
            .contains("default_subagent_model"));

        restore_or_clear_agent_defaults_for_skill_install(temp.path()).unwrap();
        let restored = std::fs::read_to_string(&config).unwrap_or_default();
        assert!(!restored.contains("default_subagent_model"));
        assert!(!restored.contains("default_subagent_reasoning_effort"));
        assert!(!agent_defaults_backup_path(temp.path()).exists());
    }

    #[test]
    fn restore_or_clear_agent_defaults_strips_leftover_keys_without_backup() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("config.toml");
        std::fs::write(
            &config,
            "model = \"gpt-5.6-sol\"\n\n[agents]\nenabled = true\ndefault_subagent_model = \"deepseek-v4-flash\"\ndefault_subagent_reasoning_effort = \"max\"\n",
        )
        .unwrap();

        restore_or_clear_agent_defaults_for_skill_install(temp.path()).unwrap();
        let cleared = std::fs::read_to_string(&config).unwrap();
        assert!(cleared.contains("enabled = true"));
        assert!(!cleared.contains("default_subagent_model"));
        assert!(!cleared.contains("default_subagent_reasoning_effort"));
    }

    #[test]
    fn resolve_dispatch_route_for_model_reads_registered_provider_id() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        std::fs::create_dir_all(dir.join("agents")).unwrap();
        std::fs::write(
            dir.join("agents/grok-4-6.toml"),
            "name = \"grok-4-6\"\nmodel = \"grok-4.6\"\nmodel_reasoning_effort = \"high\"\n",
        )
        .unwrap();
        let mut roles = RoleAgents::default();
        roles.worker = vec!["grok-4-6".to_string()];
        install(
            dir,
            "grok-4-6",
            &["grok-4-6".to_string()],
            &roles,
            WORKFLOW_MODE_SKILL,
            "high",
        )
        .unwrap();
        std::fs::write(
            dir.join(crate::codex_subagents::SUBAGENT_MANIFEST_FILENAME),
            r#"{"agents":[{"name":"grok-4-6","providerId":"slug-id","cubeProviderId":"cube-grok"}]}"#,
        )
        .unwrap();

        let route = resolve_dispatch_route_for_model(dir, "grok-4.6").unwrap();
        assert_eq!(route.agent_name, "grok-4-6");
        assert_eq!(route.model, "grok-4.6");
        assert_eq!(route.cube_provider_id.as_deref(), Some("cube-grok"));
        assert!(is_registered_dispatch_model(dir, "grok-4.6"));
        assert!(!is_registered_dispatch_model(dir, "gpt-5.6-luna"));
    }
}
