use crate::codex_subagent_providers::{
    bind_subagent_cube_provider, cube_provider_is_deletable, sync_managed_subagent_cube_providers,
    take_owned_cube_provider_id,
};
use crate::codex_subagents::{
    self, SubagentModelCandidate, SubagentModelsFetchPayload, SubagentRecord, SubagentUpsertPayload,
};
use crate::store::AppState;
use tauri::{command, State};

fn sync_subagent_cube_providers(app_state: &AppState) {
    let config_dir = crate::codex_config::get_codex_config_dir();
    match sync_managed_subagent_cube_providers(app_state.db.as_ref(), &config_dir) {
        Ok(report)
            if report.linked > 0 || report.created > 0 || report.repaired_agent_type > 0 =>
        {
            log::info!(
                "[Codex] subagent Cube 供应商同步: linked={}, created={}, agent_type={}",
                report.linked,
                report.created,
                report.repaired_agent_type
            );
        }
        Ok(_) => {}
        Err(error) => log::warn!("[Codex] subagent Cube 供应商同步失败: {error}"),
    }
}

#[command(rename_all = "camelCase")]
pub fn list_codex_subagents(
    app_state: State<'_, AppState>,
) -> Result<Vec<SubagentRecord>, String> {
    sync_subagent_cube_providers(app_state.inner());
    codex_subagents::list_subagents().map_err(|error| error.to_string())
}

#[command(rename_all = "camelCase")]
pub fn upsert_codex_subagent(
    payload: SubagentUpsertPayload,
    app_state: State<'_, AppState>,
) -> Result<SubagentRecord, String> {
    let record = codex_subagents::upsert_subagent(&payload).map_err(|error| error.to_string())?;
    let config_dir = crate::codex_config::get_codex_config_dir();
    if let Err(error) = bind_subagent_cube_provider(
        app_state.db.as_ref(),
        &config_dir,
        &record.name,
        payload.cube_provider_id.as_deref(),
    ) {
        log::warn!(
            "[Codex] 绑定 subagent `{}` 的 Cube 供应商失败: {error}",
            record.name
        );
    }
    if let Err(error) =
        crate::commands::codex_agent_workflow::refresh_workflow_skill_if_installed(&app_state)
    {
        log::warn!("refresh workflow skill after subagent upsert failed: {error}");
    }
    codex_subagents::list_subagents()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|item| item.name == record.name)
        .ok_or_else(|| "写入后无法读取 subagent".to_string())
}

#[command(rename_all = "camelCase")]
pub fn delete_codex_subagent(name: String, app_state: State<'_, AppState>) -> Result<(), String> {
    let config_dir = crate::codex_config::get_codex_config_dir();
    let owned_provider_id = take_owned_cube_provider_id(&config_dir, &name);
    codex_subagents::delete_subagent(&name).map_err(|error| error.to_string())?;
    if let Some(provider_id) = owned_provider_id {
        let still_bound = crate::codex_subagents::manifest_file_in(&config_dir)
            .ok()
            .is_some_and(|manifest| {
                manifest.agents.iter().any(|record| {
                    record.cube_provider_id.as_deref() == Some(provider_id.as_str())
                })
            });
        if !still_bound && cube_provider_is_deletable(app_state.db.as_ref(), &provider_id) {
            if let Err(error) = crate::services::ProviderService::delete(
                app_state.inner(),
                crate::app_config::AppType::Codex,
                &provider_id,
            ) {
                log::warn!(
                    "[Codex] 删除 subagent `{}` 自建 Cube 供应商 `{provider_id}` 失败: {error}",
                    name
                );
            }
        }
    }
    if let Err(error) =
        crate::commands::codex_agent_workflow::refresh_workflow_skill_if_installed(&app_state)
    {
        log::warn!("refresh workflow skill after subagent delete failed: {error}");
    }
    Ok(())
}

#[command(rename_all = "camelCase")]
pub async fn fetch_codex_subagent_models(
    payload: SubagentModelsFetchPayload,
) -> Result<Vec<SubagentModelCandidate>, String> {
    codex_subagents::validate_provider_id(&payload.model_provider_id)
        .map_err(|error| error.to_string())?;
    let api_key =
        codex_subagents::resolve_models_api_key(&payload.model_provider_id, &payload.api_key)
            .map_err(|error| error.to_string())?;
    let models = crate::services::model_fetch::fetch_models(
        payload.model_base_url.trim(),
        &api_key,
        false,
        None,
        None,
    )
    .await?;
    Ok(models
        .into_iter()
        .map(|model| SubagentModelCandidate {
            model: model.id,
            display_name: model.owned_by,
        })
        .collect())
}
