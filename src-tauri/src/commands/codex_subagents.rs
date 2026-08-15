use crate::codex_subagents::{
    self, SubagentModelCandidate, SubagentModelsFetchPayload, SubagentRecord, SubagentUpsertPayload,
};
use crate::store::AppState;
use tauri::{command, State};

#[command(rename_all = "camelCase")]
pub fn list_codex_subagents() -> Result<Vec<SubagentRecord>, String> {
    codex_subagents::list_subagents().map_err(|error| error.to_string())
}

#[command(rename_all = "camelCase")]
pub fn upsert_codex_subagent(
    payload: SubagentUpsertPayload,
    app_state: State<'_, AppState>,
) -> Result<SubagentRecord, String> {
    let record = codex_subagents::upsert_subagent(&payload).map_err(|error| error.to_string())?;
    if let Err(error) =
        crate::commands::codex_agent_workflow::refresh_workflow_skill_if_installed(&app_state)
    {
        log::warn!("refresh workflow skill after subagent upsert failed: {error}");
    }
    Ok(record)
}

#[command(rename_all = "camelCase")]
pub fn delete_codex_subagent(name: String, app_state: State<'_, AppState>) -> Result<(), String> {
    codex_subagents::delete_subagent(&name).map_err(|error| error.to_string())?;
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
