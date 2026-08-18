use serde::{Deserialize, Serialize};
use tauri::{command, State};

use crate::codex_agent_workflow;
use crate::codex_subagents;
use crate::error::AppError;
use crate::store::AppState;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexAgentWorkflowStatus {
    pub installed: bool,
    pub can_undo: bool,
    pub worker_agent: String,
    pub worker_agents: Vec<String>,
    /// 角色 → agent 映射（worker / explorer / default）；旧 manifest 由 worker_agents 推导。
    pub role_agents: codex_agent_workflow::RoleAgents,
    pub worker_model: String,
    pub worker_reasoning_effort: String,
    pub model_provider: Option<String>,
    pub sandbox_mode: String,
    pub worker_instructions: Option<String>,
    pub manifest_path: String,
    pub agent_path: String,
    pub instructions_path: String,
    pub mode: String,
    pub skill_installed: bool,
    pub skill_id: String,
    pub skill_directory: String,
    pub skill_path: String,
    pub skill_content: Option<String>,
    pub skill_stale: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexAgentWorkflowInstallPayload {
    pub worker_agent: String,
    pub worker_agents: Vec<String>,
    /// 角色 → agent 映射；旧客户端不携带时由 worker_agents 推导为 worker 角色。
    #[serde(default)]
    pub role_agents: codex_agent_workflow::RoleAgents,
}

fn get_status(_app_state: &AppState) -> Result<CodexAgentWorkflowStatus, AppError> {
    let config_dir = crate::codex_config::get_codex_config_dir();
    let manifest = codex_agent_workflow::load_manifest(&config_dir)?;
    let agents = codex_subagents::list_subagents()?;
    // 旧 manifest（无 role_agents）：worker_agents 整体归入 worker 角色，
    // 默认 worker 沿用原 manifest.worker_agent，避免迁移后默认 worker 漂移。
    let has_role_agents = manifest
        .as_ref()
        .is_some_and(|manifest| !manifest.role_agents.is_empty());
    let role_agents = manifest
        .as_ref()
        .map(|manifest| {
            if manifest.role_agents.is_empty() {
                let mut role = codex_agent_workflow::RoleAgents::default();
                role.worker = manifest.worker_agents.clone();
                role
            } else {
                manifest.role_agents.clone()
            }
        })
        .unwrap_or_default();
    let worker_agents = if role_agents.is_empty() {
        manifest
            .as_ref()
            .map(|_| codex_agent_workflow::selected_agents(&config_dir))
            .transpose()?
            .unwrap_or_default()
    } else {
        role_agents.union()
    };
    let worker_agent = if has_role_agents {
        codex_agent_workflow::derive_worker_agent(&role_agents)
    } else {
        manifest
            .as_ref()
            .map(|manifest| {
                if !manifest.worker_agent.trim().is_empty() {
                    manifest.worker_agent.clone()
                } else {
                    codex_agent_workflow::derive_worker_agent(&role_agents)
                }
            })
            .unwrap_or_default()
    };
    let selected = agents.iter().find(|agent| agent.name == worker_agent);
    let mode = manifest
        .as_ref()
        .map(|manifest| manifest.mode.clone())
        .unwrap_or_default();
    let skill_id = codex_agent_workflow::WORKFLOW_SKILL_DB_ID.to_string();
    let skill_directory = codex_agent_workflow::WORKFLOW_SKILL_DIRECTORY.to_string();
    let skill_path = codex_agent_workflow::workflow_skill_path()?;
    let skill_content = codex_agent_workflow::workflow_skill_content()?;
    let skill_installed = skill_content.is_some();
    let managed_agents: Vec<_> = agents
        .iter()
        .filter(|agent| agent.managed && agent.available)
        .cloned()
        .collect();
    let skill_stale = if manifest
        .as_ref()
        .is_some_and(|manifest| manifest.mode == codex_agent_workflow::WORKFLOW_MODE_SKILL)
        && skill_installed
    {
        let expected = codex_agent_workflow::workflow_skill_markdown(
            &managed_agents,
            &role_agents,
            &worker_agent,
        );
        Some(expected) != skill_content
    } else {
        false
    };
    let installed = if manifest
        .as_ref()
        .is_some_and(|manifest| manifest.mode == codex_agent_workflow::WORKFLOW_MODE_SKILL)
    {
        selected.is_some() && skill_installed
    } else {
        !worker_agent.is_empty()
            && selected.is_some()
            && codex_agent_workflow::workflow_uses_agent(&config_dir, &worker_agent)?
    };

    Ok(CodexAgentWorkflowStatus {
        installed,
        can_undo: codex_agent_workflow::can_undo(&config_dir)?,
        worker_agent: worker_agent.clone(),
        worker_model: selected
            .map(|agent| agent.model.clone())
            .unwrap_or_default(),
        worker_reasoning_effort: selected
            .map(|agent| agent.reasoning_effort.clone())
            .unwrap_or_default(),
        model_provider: selected.map(|agent| agent.model_provider_id.clone()),
        sandbox_mode: selected
            .map(|agent| agent.sandbox_mode.clone())
            .unwrap_or_default(),
        worker_instructions: codex_agent_workflow::current_managed_block(&config_dir)?,
        manifest_path: codex_agent_workflow::manifest_path(&config_dir)
            .to_string_lossy()
            .into_owned(),
        agent_path: if worker_agent.is_empty() {
            String::new()
        } else {
            config_dir
                .join("agents")
                .join(format!("{worker_agent}.toml"))
                .to_string_lossy()
                .into_owned()
        },
        instructions_path: codex_agent_workflow::instructions_path(&config_dir)
            .to_string_lossy()
            .into_owned(),
        mode,
        skill_installed,
        skill_id,
        skill_directory,
        skill_path: skill_path.to_string_lossy().into_owned(),
        skill_content,
        skill_stale,
        worker_agents,
        role_agents,
    })
}

#[command]
pub fn get_codex_agent_workflow_status(
    app_state: State<'_, AppState>,
) -> Result<CodexAgentWorkflowStatus, String> {
    get_status(&app_state).map_err(|error| error.to_string())
}

#[command]
pub fn install_codex_agent_workflow(
    payload: CodexAgentWorkflowInstallPayload,
    app_state: State<'_, AppState>,
) -> Result<CodexAgentWorkflowStatus, String> {
    let config_dir = crate::codex_config::get_codex_config_dir();
    let agents = codex_subagents::list_subagents().map_err(|error| error.to_string())?;
    // 角色映射优先；旧客户端不携带 role_agents 时把 worker_agents 归入 worker 角色。
    let mut role_agents = payload.role_agents.clone();
    if role_agents.is_empty() {
        role_agents.worker = payload.worker_agents.clone();
    }
    let mut selected = role_agents.union();
    if selected.is_empty() {
        selected.push(payload.worker_agent.trim().to_string());
    }
    selected.sort();
    selected.dedup();
    let derived_worker = codex_agent_workflow::derive_worker_agent(&role_agents);
    let worker_name = if derived_worker.trim().is_empty() {
        payload.worker_agent.trim().to_string()
    } else {
        derived_worker
    };
    let worker = agents
        .iter()
        .find(|agent| agent.name == worker_name)
        .ok_or_else(|| "workflow worker 必须先注册为 Codex subagent".to_string())?;
    if !worker.managed {
        return Err("workflow worker 必须先由 Codex Cube 注册或采用".to_string());
    }
    let managed_agents: Vec<_> = agents
        .iter()
        .filter(|agent| agent.managed && agent.available)
        .cloned()
        .collect();
    if selected.is_empty()
        || !selected
            .iter()
            .all(|name| managed_agents.iter().any(|agent| agent.name == *name))
    {
        return Err("workflow worker 当前不可用（供应商配置缺失）".to_string());
    }

    if let Err(error) = install_workflow_skill(
        &app_state,
        &managed_agents,
        &role_agents,
        &worker.name,
        &config_dir,
    ) {
        // 回滚已写入的 Skill 目录，避免半安装状态；AGENTS.md/manifest
        // 由 codex_agent_workflow::install 自身的原子回滚负责恢复。
        codex_agent_workflow::uninstall_workflow_skill_files();
        return Err(error.to_string());
    }
    get_status(&app_state).map_err(|error| error.to_string())
}

pub(crate) fn install_workflow_skill(
    _app_state: &AppState,
    agents: &[codex_subagents::SubagentRecord],
    role_agents: &codex_agent_workflow::RoleAgents,
    worker_agent: &str,
    config_dir: &Path,
) -> Result<(), AppError> {
    let skill_dir = codex_agent_workflow::workflow_skill_dir()?;
    // 迁移旧名：v1 的 skill 叫 subagent-workflow。直接删目录，避免 Codex 同时发现新旧两个 skill。
    codex_agent_workflow::remove_legacy_workflow_skill_dirs();
    let skill_dir_existed = skill_dir.exists();
    let agents_dir = skill_dir.join("agents");
    let result = (|| -> Result<(), AppError> {
        std::fs::create_dir_all(&agents_dir).map_err(|error| AppError::io(&agents_dir, error))?;
        let markdown =
            codex_agent_workflow::workflow_skill_markdown(agents, role_agents, worker_agent);
        crate::config::write_text_file(&skill_dir.join("SKILL.md"), &markdown)?;
        crate::config::write_text_file(
            &agents_dir.join("openai.yaml"),
            &codex_agent_workflow::workflow_skill_openai_yaml(),
        )?;
        let selected = role_agents.union();
        // Routing is resolved at spawn time from the registered agent TOML.
        // Do not write `[agents].default_subagent_model` — a global fallback
        // cannot represent per-role models and silently overrides later worker
        // changes. Restore or strip leftover keys from older Cube installs.
        if let Err(error) =
            codex_agent_workflow::restore_or_clear_agent_defaults_for_skill_install(config_dir)
        {
            log::warn!(
                "安装 cube-dispatch skill 时清理 [agents].default_subagent_* 失败，继续安装: {error}"
            );
        }
        let default_worker_name = {
            let derived = codex_agent_workflow::derive_worker_agent(role_agents);
            if derived.trim().is_empty() {
                worker_agent.trim().to_owned()
            } else {
                derived
            }
        };
        if !agents.iter().any(|agent| agent.name == default_worker_name) {
            return Err(AppError::Message(format!(
                "workflow 默认 worker {} 未在可用 subagent 列表中找到",
                default_worker_name
            )));
        }
        codex_agent_workflow::install(
            config_dir,
            worker_agent,
            &selected,
            role_agents,
            codex_agent_workflow::WORKFLOW_MODE_SKILL,
            "high",
        )
    })();
    if let Err(error) = result {
        if !skill_dir_existed {
            let _ = std::fs::remove_dir_all(&skill_dir);
        }
        return Err(error);
    }
    Ok(())
}

/// 当 Workflow Skill 已安装时，按当前 subagent 列表重新生成 SKILL.md。
/// 未安装/manifest 缺失/worker 不可用时静默返回，保持现状（UI 的 stale 徽标兜底）。
pub(crate) fn refresh_workflow_skill_if_installed(app_state: &AppState) -> Result<(), AppError> {
    let config_dir = crate::codex_config::get_codex_config_dir();
    let Some(manifest) = codex_agent_workflow::load_manifest(&config_dir)? else {
        return Ok(());
    };
    if manifest.mode != codex_agent_workflow::WORKFLOW_MODE_SKILL {
        return Ok(());
    }
    if codex_agent_workflow::workflow_skill_content()?.is_none() {
        return Ok(());
    }
    let agents = codex_subagents::list_subagents()?;
    let managed_agents: Vec<_> = agents
        .iter()
        .filter(|agent| agent.managed && agent.available)
        .cloned()
        .collect();
    if !managed_agents
        .iter()
        .any(|agent| agent.name == manifest.worker_agent)
    {
        return Ok(());
    }
    let legacy = manifest.role_agents.is_empty();
    let role_agents = if legacy {
        let mut role = codex_agent_workflow::RoleAgents::default();
        role.worker = codex_agent_workflow::selected_agents(&config_dir)?;
        // Legacy manifests did not persist role priority. Preserve the manifest's
        // explicit worker as the first/default entry during refresh.
        if !manifest.worker_agent.trim().is_empty() {
            role.worker.retain(|name| name != &manifest.worker_agent);
            role.worker.insert(0, manifest.worker_agent.clone());
        }
        role
    } else {
        manifest.role_agents.clone()
    };
    // 旧 manifest 迁移时保持原默认 worker；新 manifest 由角色推导。
    let worker_agent = if legacy && !manifest.worker_agent.trim().is_empty() {
        manifest.worker_agent.clone()
    } else {
        codex_agent_workflow::derive_worker_agent(&role_agents)
    };
    // The fallback defaults are sourced from the same app-selected default worker.
    // If that worker is currently unavailable, leave the installed workflow untouched
    // and let the next refresh retry after the subagent becomes available.
    let default_worker_name = codex_agent_workflow::derive_worker_agent(&role_agents);
    if !managed_agents
        .iter()
        .any(|agent| agent.name == default_worker_name)
    {
        return Ok(());
    }
    install_workflow_skill(
        app_state,
        &managed_agents,
        &role_agents,
        &worker_agent,
        &config_dir,
    )
}

#[command]
pub fn cancel_codex_agent_workflow_instructions(
    app_state: State<'_, AppState>,
) -> Result<CodexAgentWorkflowStatus, String> {
    let config_dir = crate::codex_config::get_codex_config_dir();
    codex_agent_workflow::cancel(&config_dir).map_err(|error| error.to_string())?;
    codex_agent_workflow::uninstall_workflow_skill_files();
    get_status(&app_state).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use crate::store::AppState;
    use std::sync::Arc;

    struct TestHomeGuard(Option<std::ffi::OsString>);
    impl TestHomeGuard {
        fn set(home: &Path) -> Self {
            let guard = Self(std::env::var_os("CODEX_CUBE_TEST_HOME"));
            std::env::set_var("CODEX_CUBE_TEST_HOME", home);
            guard
        }
    }
    impl Drop for TestHomeGuard {
        fn drop(&mut self) {
            match self.0.take() {
                Some(value) => std::env::set_var("CODEX_CUBE_TEST_HOME", value),
                None => std::env::remove_var("CODEX_CUBE_TEST_HOME"),
            }
        }
    }

    fn setup() -> (tempfile::TempDir, TestHomeGuard, AppState) {
        let home = tempfile::tempdir().unwrap();
        let home_guard = TestHomeGuard::set(home.path());
        let app_state = AppState::new(Arc::new(Database::memory().unwrap()));
        (home, home_guard, app_state)
    }

    fn upsert_payload(name: &str, description: &str) -> codex_subagents::SubagentUpsertPayload {
        codex_subagents::SubagentUpsertPayload {
            name: name.to_owned(),
            description: description.to_owned(),
            model: "deepseek-v4-flash".to_owned(),
            model_provider_id: "deepseek".to_owned(),
            model_base_url: "https://api.deepseek.com".to_owned(),
            api_key: Some("test".to_owned()),
            sandbox_mode: crate::codex_subagents::INHERIT_SANDBOX_MODE.to_owned(),
            reasoning_effort: "xhigh".to_owned(),
            wire_api: Some("responses".to_owned()),
            agent_type: None,
            cube_provider_id: None,
        }
    }

    fn register_subagent(name: &str, description: &str) -> codex_subagents::SubagentRecord {
        codex_subagents::upsert_subagent(&upsert_payload(name, description)).unwrap()
    }

    fn managed_agents() -> Vec<codex_subagents::SubagentRecord> {
        codex_subagents::list_subagents()
            .unwrap()
            .into_iter()
            .filter(|agent| agent.managed && agent.available)
            .collect()
    }

    fn worker_role(names: &[&str]) -> codex_agent_workflow::RoleAgents {
        let mut role = codex_agent_workflow::RoleAgents::default();
        role.worker = names.iter().map(|name| name.to_string()).collect();
        role
    }

    fn register_subagent_with_reasoning(
        name: &str,
        reasoning: &str,
    ) -> codex_subagents::SubagentRecord {
        let mut payload = upsert_payload(name, "test");
        payload.reasoning_effort = reasoning.to_owned();
        codex_subagents::upsert_subagent(&payload).unwrap()
    }

    #[test]
    #[serial_test::serial]
    fn workflow_skill_install_writes_ssot_and_reports_installed_status() {
        let (_home, _home_guard, app_state) = setup();
        let record = register_subagent("deepseek-flash", "适合前端重构");
        let config_dir = crate::codex_config::get_codex_config_dir();

        install_workflow_skill(
            &app_state,
            &[record],
            &worker_role(&["deepseek-flash"]),
            "deepseek-flash",
            &config_dir,
        )
        .unwrap();

        let skill_path = codex_agent_workflow::workflow_skill_path().unwrap();
        let content = std::fs::read_to_string(&skill_path).unwrap();
        assert!(content.contains("- **worker:**"));
        assert!(content.contains("  - `deepseek-flash`"));
        assert!(!content.contains("model: deepseek-v4-flash"));
        assert!(!content.contains("**Default worker:** `deepseek-flash`"));
        assert!(codex_agent_workflow::workflow_skill_dir()
            .unwrap()
            .join("agents")
            .join("openai.yaml")
            .exists());
        assert!(skill_path.exists());

        let status = get_status(&app_state).unwrap();
        assert!(status.installed);
        assert_eq!(status.mode, codex_agent_workflow::WORKFLOW_MODE_SKILL);
        assert!(status.skill_installed);
        assert!(!status.skill_stale);
        assert!(status
            .skill_path
            .contains(".codex/skills/cube-dispatch/SKILL.md"));
        assert!(status
            .skill_content
            .is_some_and(|content| content.contains("  - `deepseek-flash`")));
        assert_eq!(
            status.role_agents.worker,
            vec!["deepseek-flash".to_string()]
        );
    }

    #[test]
    #[serial_test::serial]
    fn workflow_skill_install_does_not_write_codex_global_subagent_defaults() {
        let (_home, _home_guard, app_state) = setup();
        let worker = register_subagent_with_reasoning("worker-agent", "xhigh");
        let mut default_payload = upsert_payload("default-agent", "通用默认 worker");
        default_payload.model = "deepseek-v4-pro".to_owned();
        default_payload.reasoning_effort = "max".to_owned();
        default_payload.agent_type = Some(codex_subagents::AGENT_TYPE_DEFAULT.to_owned());
        let default_worker = codex_subagents::upsert_subagent(&default_payload).unwrap();
        let config_dir = crate::codex_config::get_codex_config_dir();
        std::fs::write(config_dir.join("config.toml"), "model = \"gpt-5.6-sol\"\n").unwrap();
        let role_agents = codex_agent_workflow::RoleAgents {
            worker: vec![worker.name.clone()],
            explorer: Vec::new(),
            default: vec![default_worker.name.clone()],
        };

        install_workflow_skill(
            &app_state,
            &[worker, default_worker],
            &role_agents,
            "worker-agent",
            &config_dir,
        )
        .unwrap();

        let config = std::fs::read_to_string(config_dir.join("config.toml")).unwrap();
        assert!(!config.contains("default_subagent_model"));
        assert!(!config.contains("default_subagent_reasoning_effort"));
        let profile = codex_agent_workflow::resolve_dispatch_profile(&config_dir, Some("worker"))
            .expect("worker role must resolve from registered TOML");
        assert_eq!(profile.agent_name, "worker-agent");
        assert_eq!(profile.model, "deepseek-v4-flash");
        assert_eq!(profile.reasoning_effort, "xhigh");
        let default_profile =
            codex_agent_workflow::resolve_dispatch_profile(&config_dir, Some("default"))
                .expect("default role must resolve from registered TOML");
        assert_eq!(default_profile.agent_name, "default-agent");
        assert_eq!(default_profile.model, "deepseek-v4-pro");
        assert_eq!(default_profile.reasoning_effort, "max");
    }

    #[test]
    #[serial_test::serial]
    fn workflow_skill_install_clears_leftover_codex_global_subagent_defaults() {
        let (_home, _home_guard, app_state) = setup();
        let worker = register_subagent("worker-agent", "适合修 leftover defaults");
        let config_dir = crate::codex_config::get_codex_config_dir();
        std::fs::write(config_dir.join("config.toml"), "model = \"gpt-5.6-sol\"\n").unwrap();
        codex_agent_workflow::sync_agent_defaults(&config_dir, "deepseek-v4-flash", "max").unwrap();
        assert!(std::fs::read_to_string(config_dir.join("config.toml"))
            .unwrap()
            .contains("default_subagent_model = \"deepseek-v4-flash\""));

        install_workflow_skill(
            &app_state,
            &[worker],
            &worker_role(&["worker-agent"]),
            "worker-agent",
            &config_dir,
        )
        .unwrap();

        let config = std::fs::read_to_string(config_dir.join("config.toml")).unwrap();
        assert!(!config.contains("default_subagent_model"));
        assert!(!config.contains("default_subagent_reasoning_effort"));
        assert!(!codex_agent_workflow::agent_defaults_backup_path(&config_dir).exists());
    }

    #[test]
    #[serial_test::serial]
    fn install_workflow_skill_removes_legacy_subagent_workflow() {
        let (_home, _home_guard, app_state) = setup();
        let record = register_subagent("deepseek-flash", "适合前端重构");
        let config_dir = crate::codex_config::get_codex_config_dir();

        // 模拟旧版安装：Codex skills 目录 + Cube SSOT 残留的 subagent-workflow。
        let legacy_dir = codex_agent_workflow::LEGACY_WORKFLOW_SKILL_DIRECTORY.to_string();
        let app_root = codex_agent_workflow::codex_skills_dir();
        let leftover_ssot = crate::config::get_app_config_dir()
            .join("skills")
            .join(&legacy_dir);
        let legacy_app = app_root.join(&legacy_dir);
        std::fs::create_dir_all(leftover_ssot.join("agents")).unwrap();
        std::fs::create_dir_all(&legacy_app).unwrap();
        std::fs::write(
            leftover_ssot.join("SKILL.md"),
            "---\nname: subagent-workflow\n---\n",
        )
        .unwrap();

        install_workflow_skill(
            &app_state,
            &[record],
            &worker_role(&["deepseek-flash"]),
            "deepseek-flash",
            &config_dir,
        )
        .unwrap();

        assert!(!leftover_ssot.exists());
        assert!(!legacy_app.exists());
        let status = get_status(&app_state).unwrap();
        assert_eq!(status.skill_id, "local:cube-dispatch");
        assert!(status
            .skill_path
            .ends_with(".codex/skills/cube-dispatch/SKILL.md"));
    }

    #[test]
    #[serial_test::serial]
    fn refresh_workflow_skill_picks_up_subagent_reasoning_without_staling_skill() {
        let (_home, _home_guard, app_state) = setup();
        register_subagent_with_reasoning("deepseek-flash", "xhigh");
        let config_dir = crate::codex_config::get_codex_config_dir();

        install_workflow_skill(
            &app_state,
            &managed_agents(),
            &worker_role(&["deepseek-flash"]),
            "deepseek-flash",
            &config_dir,
        )
        .unwrap();
        register_subagent_with_reasoning("deepseek-flash", "max");

        assert!(!get_status(&app_state).unwrap().skill_stale);

        let profile = codex_agent_workflow::resolve_dispatch_profile(&config_dir, Some("worker"))
            .expect("worker profile");
        assert_eq!(profile.model, "deepseek-v4-flash");
        assert_eq!(profile.reasoning_effort, "max");
        let config = std::fs::read_to_string(config_dir.join("config.toml")).unwrap_or_default();
        assert!(!config.contains("default_subagent_model"));
        assert!(!config.contains("default_subagent_reasoning_effort"));
    }

    #[test]
    #[serial_test::serial]
    fn refresh_workflow_skill_is_noop_without_manifest() {
        let (_home, _home_guard, app_state) = setup();
        register_subagent("deepseek-flash", "适合前端重构");

        refresh_workflow_skill_if_installed(&app_state).unwrap();

        assert!(!codex_agent_workflow::workflow_skill_path()
            .unwrap()
            .exists());
    }
}
