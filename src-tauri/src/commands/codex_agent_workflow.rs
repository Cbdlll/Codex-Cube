use serde::{Deserialize, Serialize};
use tauri::{command, State};

use crate::app_config::{AppType, InstalledSkill, SkillApps};
use crate::codex_agent_workflow;
use crate::codex_subagents;
use crate::error::AppError;
use crate::services::skill::SkillService;
use crate::store::AppState;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexAgentWorkflowStatus {
    pub installed: bool,
    pub can_undo: bool,
    pub worker_agent: String,
    pub worker_agents: Vec<String>,
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
}

fn get_status(app_state: &AppState) -> Result<CodexAgentWorkflowStatus, AppError> {
    let config_dir = crate::codex_config::get_codex_config_dir();
    let manifest = codex_agent_workflow::load_manifest(&config_dir)?;
    let agents = codex_subagents::list_subagents()?;
    let selected = manifest.as_ref().and_then(|manifest| {
        agents
            .iter()
            .find(|agent| agent.name == manifest.worker_agent)
    });
    let worker_agents = manifest
        .as_ref()
        .map(|m| codex_agent_workflow::selected_agents(&config_dir))
        .transpose()?
        .unwrap_or_default();
    let worker_agent = manifest
        .as_ref()
        .map(|manifest| manifest.worker_agent.clone())
        .unwrap_or_default();
    let mode = manifest
        .as_ref()
        .map(|manifest| manifest.mode.clone())
        .unwrap_or_default();
    let skill_id = codex_agent_workflow::WORKFLOW_SKILL_DB_ID.to_string();
    let skill_directory = codex_agent_workflow::WORKFLOW_SKILL_DIRECTORY.to_string();
    let skill_path = SkillService::get_app_skills_dir(&AppType::Codex)
        .map_err(|error| AppError::Message(error.to_string()))?
        .join(codex_agent_workflow::WORKFLOW_SKILL_DIRECTORY)
        .join("SKILL.md");
    let skill_content = codex_agent_workflow::workflow_skill_content()?;
    let skill_installed =
        app_state.db.get_installed_skill(&skill_id)?.is_some() && skill_content.is_some();
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
            &worker_agents,
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
    let mut selected = payload.worker_agents.clone();
    if selected.is_empty() {
        selected.push(payload.worker_agent.trim().to_string());
    }
    selected.sort();
    selected.dedup();
    let worker_name = if payload.worker_agent.trim().is_empty() {
        selected.first().cloned().unwrap_or_default()
    } else {
        payload.worker_agent.trim().to_string()
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
        &selected,
        &worker.name,
        &config_dir,
    ) {
        // 回滚已写入的 Skill（含 DB 记录），避免半安装状态；AGENTS.md/manifest
        // 由 codex_agent_workflow::install 自身的原子回滚负责恢复。
        if app_state
            .db
            .get_installed_skill(codex_agent_workflow::WORKFLOW_SKILL_DB_ID)
            .is_ok_and(|skill| skill.is_some())
        {
            let _ =
                SkillService::uninstall(&app_state.db, codex_agent_workflow::WORKFLOW_SKILL_DB_ID);
        }
        return Err(error.to_string());
    }
    get_status(&app_state).map_err(|error| error.to_string())
}

pub(crate) fn install_workflow_skill(
    app_state: &AppState,
    agents: &[codex_subagents::SubagentRecord],
    selected: &[String],
    worker_agent: &str,
    config_dir: &Path,
) -> Result<(), AppError> {
    let skill_dir = codex_agent_workflow::workflow_skill_dir()?;
    // 迁移旧名：v1 的 skill 叫 subagent-workflow。先走标准卸载（含备份），
    // 再兜底清理可能残留的目录，避免 Codex 同时发现新旧两个 skill。
    let _ = SkillService::uninstall(
        &app_state.db,
        codex_agent_workflow::LEGACY_WORKFLOW_SKILL_DB_ID,
    );
    let ssot_root =
        SkillService::get_ssot_dir().map_err(|error| AppError::Message(error.to_string()))?;
    let _ = std::fs::remove_dir_all(
        ssot_root.join(codex_agent_workflow::LEGACY_WORKFLOW_SKILL_DIRECTORY),
    );
    let app_skills_dir = SkillService::get_app_skills_dir(&AppType::Codex)
        .map_err(|error| AppError::Message(error.to_string()))?;
    let _ = std::fs::remove_dir_all(
        app_skills_dir.join(codex_agent_workflow::LEGACY_WORKFLOW_SKILL_DIRECTORY),
    );
    let skill_dir_existed = skill_dir.exists();
    let agents_dir = skill_dir.join("agents");
    let result = (|| -> Result<(), AppError> {
        std::fs::create_dir_all(&agents_dir).map_err(|error| AppError::io(&agents_dir, error))?;
        let markdown =
            codex_agent_workflow::workflow_skill_markdown(agents, selected, worker_agent);
        crate::config::write_text_file(&skill_dir.join("SKILL.md"), &markdown)?;
        crate::config::write_text_file(
            &agents_dir.join("openai.yaml"),
            &codex_agent_workflow::workflow_skill_openai_yaml(),
        )?;

        let existing = app_state
            .db
            .get_installed_skill(codex_agent_workflow::WORKFLOW_SKILL_DB_ID)?;
        let (_, description) = codex_agent_workflow::workflow_skill_metadata(&markdown);
        let installed_skill = InstalledSkill {
            id: codex_agent_workflow::WORKFLOW_SKILL_DB_ID.to_string(),
            name: codex_agent_workflow::WORKFLOW_SKILL_DISPLAY_NAME.to_string(),
            description,
            directory: codex_agent_workflow::WORKFLOW_SKILL_DIRECTORY.to_string(),
            repo_owner: None,
            repo_name: None,
            repo_branch: None,
            readme_url: None,
            apps: existing
                .as_ref()
                .map(|skill| skill.apps.clone())
                .unwrap_or_else(|| SkillApps::only(&AppType::Codex)),
            installed_at: existing
                .as_ref()
                .map(|skill| skill.installed_at)
                .unwrap_or_else(|| chrono::Utc::now().timestamp()),
            content_hash: Some(
                SkillService::compute_dir_hash(&skill_dir)
                    .map_err(|error| AppError::Message(error.to_string()))?,
            ),
            updated_at: chrono::Utc::now().timestamp(),
        };
        app_state.db.save_skill(&installed_skill)?;
        SkillService::sync_to_app_dir(
            codex_agent_workflow::WORKFLOW_SKILL_DIRECTORY,
            &AppType::Codex,
        )
        .map_err(|error| AppError::Message(error.to_string()))?;
        codex_agent_workflow::install(
            config_dir,
            worker_agent,
            selected,
            codex_agent_workflow::WORKFLOW_MODE_SKILL,
            agents
                .iter()
                .find(|agent| agent.name == worker_agent)
                .map(|agent| agent.reasoning_effort.as_str())
                .unwrap_or_default(),
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
    if app_state
        .db
        .get_installed_skill(codex_agent_workflow::WORKFLOW_SKILL_DB_ID)?
        .is_none()
    {
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
    install_workflow_skill(
        app_state,
        &managed_agents,
        &codex_agent_workflow::selected_agents(&config_dir)?,
        &manifest.worker_agent,
        &config_dir,
    )
}

#[command]
pub fn cancel_codex_agent_workflow_instructions(
    app_state: State<'_, AppState>,
) -> Result<CodexAgentWorkflowStatus, String> {
    let config_dir = crate::codex_config::get_codex_config_dir();
    codex_agent_workflow::cancel(&config_dir).map_err(|error| error.to_string())?;
    if app_state
        .db
        .get_installed_skill(codex_agent_workflow::WORKFLOW_SKILL_DB_ID)
        .map_err(|error| error.to_string())?
        .is_some()
    {
        SkillService::uninstall(&app_state.db, codex_agent_workflow::WORKFLOW_SKILL_DB_ID)
            .map_err(|error| error.to_string())?;
    }
    get_status(&app_state).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use crate::services::skill::SkillStorageLocation;
    use crate::settings;
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

    struct SkillLocationGuard(SkillStorageLocation);
    impl Drop for SkillLocationGuard {
        fn drop(&mut self) {
            let _ = settings::set_skill_storage_location(self.0);
        }
    }

    fn setup() -> (
        tempfile::TempDir,
        TestHomeGuard,
        SkillLocationGuard,
        AppState,
    ) {
        let home = tempfile::tempdir().unwrap();
        let home_guard = TestHomeGuard::set(home.path());
        let location_guard = SkillLocationGuard(settings::get_skill_storage_location());
        settings::set_skill_storage_location(SkillStorageLocation::CodexCube).unwrap();
        let app_state = AppState::new(Arc::new(Database::memory().unwrap()));
        (home, home_guard, location_guard, app_state)
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

    #[test]
    #[serial_test::serial]
    fn workflow_skill_install_writes_ssot_and_reports_installed_status() {
        let (_home, _home_guard, _location_guard, app_state) = setup();
        let record = register_subagent("deepseek-flash", "适合前端重构");
        let config_dir = crate::codex_config::get_codex_config_dir();

        install_workflow_skill(
            &app_state,
            &[record],
            &["deepseek-flash".to_string()],
            "deepseek-flash",
            &config_dir,
        )
        .unwrap();

        let skill_path = codex_agent_workflow::workflow_skill_path().unwrap();
        let content = std::fs::read_to_string(&skill_path).unwrap();
        assert!(content.contains("- `deepseek-flash`: 适合前端重构"));
        assert!(content.contains("**Default worker:** `deepseek-flash`"));
        assert!(codex_agent_workflow::workflow_skill_dir()
            .unwrap()
            .join("agents")
            .join("openai.yaml")
            .exists());
        assert!(app_state
            .db
            .get_installed_skill(codex_agent_workflow::WORKFLOW_SKILL_DB_ID)
            .unwrap()
            .is_some());

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
            .is_some_and(|content| content.contains("适合前端重构")));
    }

    #[test]
    #[serial_test::serial]
    fn install_workflow_skill_removes_legacy_subagent_workflow() {
        let (_home, _home_guard, _location_guard, app_state) = setup();
        let record = register_subagent("deepseek-flash", "适合前端重构");
        let config_dir = crate::codex_config::get_codex_config_dir();

        // 模拟旧版安装：DB 记录 + SSOT/应用目录下的 subagent-workflow。
        let legacy_id = codex_agent_workflow::LEGACY_WORKFLOW_SKILL_DB_ID.to_string();
        let legacy_dir = codex_agent_workflow::LEGACY_WORKFLOW_SKILL_DIRECTORY.to_string();
        let ssot_root = SkillService::get_ssot_dir().unwrap();
        let app_root = SkillService::get_app_skills_dir(&AppType::Codex).unwrap();
        let legacy_ssot = ssot_root.join(&legacy_dir);
        let legacy_app = app_root.join(&legacy_dir);
        std::fs::create_dir_all(legacy_ssot.join("agents")).unwrap();
        std::fs::create_dir_all(&legacy_app).unwrap();
        std::fs::write(
            legacy_ssot.join("SKILL.md"),
            "---\nname: subagent-workflow\n---\n",
        )
        .unwrap();
        let legacy = crate::app_config::InstalledSkill {
            id: legacy_id.clone(),
            name: "Subagent Workflow".to_string(),
            description: None,
            directory: legacy_dir,
            repo_owner: None,
            repo_name: None,
            repo_branch: None,
            readme_url: None,
            apps: crate::app_config::SkillApps::only(&AppType::Codex),
            installed_at: chrono::Utc::now().timestamp(),
            content_hash: None,
            updated_at: chrono::Utc::now().timestamp(),
        };
        app_state.db.save_skill(&legacy).unwrap();

        install_workflow_skill(
            &app_state,
            &[record],
            &["deepseek-flash".to_string()],
            "deepseek-flash",
            &config_dir,
        )
        .unwrap();

        assert!(!legacy_ssot.exists());
        assert!(!legacy_app.exists());
        assert!(app_state
            .db
            .get_installed_skill(&legacy_id)
            .unwrap()
            .is_none());
        let status = get_status(&app_state).unwrap();
        assert_eq!(status.skill_id, "local:cube-dispatch");
        assert!(status
            .skill_path
            .ends_with(".codex/skills/cube-dispatch/SKILL.md"));
    }

    #[test]
    #[serial_test::serial]
    fn refresh_workflow_skill_regenerates_after_subagent_description_change() {
        let (_home, _home_guard, _location_guard, app_state) = setup();
        register_subagent("deepseek-flash", "适合前端重构");
        let config_dir = crate::codex_config::get_codex_config_dir();

        install_workflow_skill(
            &app_state,
            &managed_agents(),
            &["deepseek-flash".to_string()],
            "deepseek-flash",
            &config_dir,
        )
        .unwrap();
        register_subagent("deepseek-flash", "适合后端重构");

        assert!(get_status(&app_state).unwrap().skill_stale);

        refresh_workflow_skill_if_installed(&app_state).unwrap();

        let status = get_status(&app_state).unwrap();
        assert!(!status.skill_stale);
        assert!(status
            .skill_content
            .is_some_and(|content| content.contains("适合后端重构")));
    }

    #[test]
    #[serial_test::serial]
    fn refresh_workflow_skill_is_noop_without_manifest() {
        let (_home, _home_guard, _location_guard, app_state) = setup();
        register_subagent("deepseek-flash", "适合前端重构");

        refresh_workflow_skill_if_installed(&app_state).unwrap();

        assert!(app_state
            .db
            .get_installed_skill(codex_agent_workflow::WORKFLOW_SKILL_DB_ID)
            .unwrap()
            .is_none());
        assert!(!codex_agent_workflow::workflow_skill_path()
            .unwrap()
            .exists());
    }
}
