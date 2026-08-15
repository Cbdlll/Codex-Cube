use crate::codex_subagents::SubagentRecord;
use crate::config::{delete_file, write_json_file, write_text_file};
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const WORKFLOW_MANIFEST_FILENAME: &str = "codex-cube-agent-workflow.json";
pub const WORKFLOW_BACKUP_FILENAME: &str = "codex-cube-workflow-instructions-backup.json";
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
    /// "skill"（新式 Workflow Skill）或遗留 "agents-md"（AGENTS.md 注入）。
    #[serde(default = "default_workflow_mode")]
    pub mode: String,
    /// 新式安装时写入的 Skill 目录名。
    #[serde(default = "default_skill_directory")]
    pub skill_directory: String,
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

/// 是否可以取消/恢复：有效备份存在 → true；无备份但 AGENTS.md 含受管块或
/// workflow manifest 存在 → true（cancel 仍可清理/恢复）；全部缺失 → false。
/// 备份不可读/损坏/结构非法 → 错误。不要用 backup_path().exists() 判断。
pub fn can_undo(config_dir: &Path) -> Result<bool, AppError> {
    if read_backup(config_dir)?.is_some() {
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
    Ok(Some(manifest))
}

pub fn selected_agents(config_dir: &Path) -> Result<Vec<String>, AppError> {
    let Some(manifest) = load_manifest(config_dir)? else {
        return Ok(Vec::new());
    };
    let mut selected = manifest.worker_agents;
    if selected.is_empty() {
        selected.push(manifest.worker_agent);
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

pub fn workflow_skill_dir() -> Result<PathBuf, AppError> {
    let ssot_dir = crate::services::skill::SkillService::get_ssot_dir()
        .map_err(|error| AppError::Message(error.to_string()))?;
    Ok(ssot_dir.join(WORKFLOW_SKILL_DIRECTORY))
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
/// 正文列出所有已注册 subagent 及其描述/模型/推理档位，并标记默认 worker。
pub fn workflow_skill_markdown(
    agents: &[SubagentRecord],
    selected: &[String],
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
- Registered custom agents live in `~/.codex/agents/*.toml` (personal) or `.codex/agents/*.toml` \
(project). Each file requires `name`, `description`, and `developer_instructions`; `model`, \
`model_reasoning_effort`, `sandbox_mode`, `mcp_servers`, and `skills.config` are optional. \
Codex identifies agents by their `name` field, and a custom name overrides the built-in \
agents (default, worker, explorer).\n\n",
    );
    output.push_str(
        "## Roles\n\
- The coordinator - the model running the current conversation - owns requirements, planning, \
architecture, task decomposition, integration, and final review (acceptance).\n\
- A worker is a registered subagent that executes one delegated scope and reports results.\n\
- Top-level decisions are not delegated to workers.\n\n",
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
    for name in selected {
        let Some(agent) = agents.iter().find(|a| &a.name == name) else {
            continue;
        };
        let description = agent.description.trim();
        let description = if description.is_empty() {
            "(no description provided)".to_owned()
        } else {
            agent.description.to_owned()
        };
        output.push_str(&format!(
            "- `{}`: {} (model: {}, reasoning: {})\n",
            agent.name, description, agent.model, agent.reasoning_effort
        ));
    }
    output.push_str(&format!("\n**Default worker:** `{worker_agent}`\n\n"));
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
- Choose the registered subagent whose description matches the work; use the default \
worker otherwise.\n\
- Assign explicit, non-overlapping write scopes before spawning.\n\
- Wait for all required results and keep the main thread focused on planning and review.\n\n",
    );
    output.push_str(
        "## Spawning protocol\n\
- Start each multi-round task with a NEW subagent spawned from the registered worker agent \
(`fork_turns=\"none\"`); never substitute the coordinator's own model.\n\
- Use the subagent-spawning tool actually exposed by this session: the collaboration tool \
(`spawn_agent` / `collaboration__spawn_agent`, then `wait_agent`/`followup_task`/`send_message` \
to continue) when available, or the app thread tools (`create_thread` + \
`send_message_to_thread` + `wait_threads`) when those are the only subagent tools exposed. \
Never call a tool that is not in the current tool list.\n\
- Inspect the current spawn tool schema before dispatching. If the selected registered agent \
name appears in the allowed `agent_type` values, pass that exact name as `agent_type`; this is \
the primary path and loads the custom agent file directly.\n\
- If the registered name is not an allowed `agent_type`, use the allowed generic `worker` role \
and pass `model` and `reasoning_effort` explicitly with the registered per-agent values. This \
is a compatibility fallback for sessions whose tool schema does not expose custom agent types. \
Never invent or pass an `agent_type` value absent from the current schema.\n\
- For fallback dispatch, resolution order is: explicit spawn value > custom agent file > `[agents]` \
defaults (`default_subagent_model`, `default_subagent_reasoning_effort`) > parent value.\n\
- Treat a successful compatibility fallback as normal dispatch; do not add a user-facing caveat \
solely because the registered custom `agent_type` was unavailable. Report it only if delegation \
fails or the fallback changes requested behavior.\n\
- Use the same registered reasoning_effort on every round and when resuming; never let \
the worker run at a different effort than registered.\n\
- Codex handles spawning, follow-up routing, waiting for results, and closing threads; \
you can also steer, stop, or close a subagent with a direct request.\n\n",
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
- Codex-level `[agents]` settings control subagent behavior: `enabled`, \
`max_concurrent_threads_per_session`, `default_subagent_model`, `default_subagent_reasoning_effort`, \
and `interrupt_message`. Explicit spawn values override the defaults.\n\n",
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

pub fn install(
    config_dir: &Path,
    worker_agent: &str,
    worker_agents: &[String],
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
            WORKFLOW_MODE_AGENTS_MD,
            "high",
        )
        .unwrap();
        install(
            temp.path(),
            "worker-b",
            &["worker-b".to_string()],
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
        }
    }

    #[test]
    fn workflow_skill_markdown_lists_agents_and_default_worker() {
        let agents = vec![
            test_subagent(
                "deepseek-flash",
                "适合前端重构",
                "deepseek-v4-flash",
                "xhigh",
            ),
            test_subagent("gpt-sol-worker", "Balanced worker", "gpt-5.6-sol", "high"),
        ];
        let markdown = workflow_skill_markdown(
            &agents,
            &["deepseek-flash".to_string(), "gpt-sol-worker".to_string()],
            "deepseek-flash",
        );

        assert!(markdown.starts_with("---\nname: cube-dispatch\n"));
        assert!(markdown.contains("description: Use for tasks"));
        assert!(markdown.contains("delegated to registered Codex subagents"));
        assert!(markdown
            .contains("dispatch registered workers by default instead of doing the work inline"));
        assert!(markdown.contains(
            "- `deepseek-flash`: 适合前端重构 (model: deepseek-v4-flash, reasoning: xhigh)"
        ));
        assert!(markdown
            .contains("- `gpt-sol-worker`: Balanced worker (model: gpt-5.6-sol, reasoning: high)"));
        assert!(markdown.contains("**Default worker:** `deepseek-flash`"));
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
        assert!(markdown.contains("local routing"));
        assert!(markdown.contains("## Triggering"));
        assert!(markdown.contains("## Spawning protocol"));
        assert!(markdown.contains("fork_turns=\"none\""));
        assert!(markdown.contains(
            "If the selected registered agent name appears in the allowed `agent_type` values"
        ));
        assert!(markdown.contains("pass that exact name as `agent_type`"));
        assert!(markdown.contains("use the allowed generic `worker` role"));
        assert!(markdown.contains("pass `model` and `reasoning_effort` explicitly"));
        assert!(markdown
            .contains("Never invent or pass an `agent_type` value absent from the current schema"));
        assert!(markdown.contains("do not add a user-facing caveat"));
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
    fn workflow_skill_markdown_uses_placeholder_for_empty_description() {
        let markdown = workflow_skill_markdown(
            &[test_subagent(
                "empty-worker",
                "   ",
                "deepseek-v4-flash",
                "high",
            )],
            &["empty-worker".to_string()],
            "empty-worker",
        );

        assert!(markdown.contains(
            "- `empty-worker`: (no description provided) (model: deepseek-v4-flash, reasoning: high)"
        ));
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
            WORKFLOW_MODE_SKILL,
            "high",
        )
        .unwrap();
        install(
            temp.path(),
            "worker-b",
            &["worker-b".to_string()],
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
            &["deepseek-flash".to_string()],
            "deepseek-flash",
        );
        let (name, description) = workflow_skill_metadata(&markdown);
        assert_eq!(name.as_deref(), Some("cube-dispatch"));
        assert!(description.is_some_and(|description| description.contains("bounded subtasks")));
        assert_eq!(workflow_skill_metadata("no frontmatter"), (None, None));
    }
}
