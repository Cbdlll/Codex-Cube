//! Bind registered Codex subagents to real Cube SQLite providers.
//!
//! UI registration historically copied a Cube provider's URL/key into
//! `~/.codex/agents/*.toml` and `codex-cube-agent-keys/<slug>.key`, then stored
//! that slug as `providerId`. Dispatch routing looks up `providers.id`, so the
//! slug never resolved. This module links (or creates) the Cube row and writes
//! `cubeProviderId` onto the subagent manifest.

use crate::codex_config::{extract_codex_api_key, extract_codex_base_url};
use crate::codex_subagents::{
    agent_path_in, manifest_file_in, provider_key_path_in, save_manifest_in,
    write_private_file_atomic, AGENT_TYPE_WORKER, SUBAGENT_RUNTIME_PROVIDER_ID,
};
use crate::config::write_text_file;
use crate::database::Database;
use crate::error::AppError;
use crate::provider::Provider;
use serde_json::json;
use std::path::Path;
use toml_edit::{value, DocumentMut, Item, Table};

const APP_TYPE: &str = "codex";
const OWNED_NOTES_PREFIX: &str = "codex-cube-subagent:";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubagentCubeSyncReport {
    pub linked: usize,
    pub created: usize,
    pub repaired_agent_type: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BindOutcome {
    Unchanged,
    Linked,
    Created,
}

pub fn sync_managed_subagent_cube_providers(
    db: &Database,
    config_dir: &Path,
) -> Result<SubagentCubeSyncReport, AppError> {
    let mut manifest = manifest_file_in(config_dir)?;
    if manifest.agents.is_empty() {
        return Ok(SubagentCubeSyncReport::default());
    }
    let mut report = SubagentCubeSyncReport::default();
    let mut changed = false;
    for record in &mut manifest.agents {
        if record.agent_type.trim().is_empty() {
            record.agent_type = AGENT_TYPE_WORKER.to_owned();
            report.repaired_agent_type += 1;
            changed = true;
        }
        let requested = record
            .cube_provider_id
            .clone()
            .map(|id| id.trim().to_owned())
            .filter(|id| !id.is_empty());
        match bind_record(db, config_dir, record, requested.as_deref())? {
            BindOutcome::Unchanged => {}
            BindOutcome::Linked => {
                report.linked += 1;
                changed = true;
            }
            BindOutcome::Created => {
                report.created += 1;
                changed = true;
            }
        }
    }
    if changed {
        save_manifest_in(config_dir, &manifest)?;
    }
    Ok(report)
}

pub fn bind_subagent_cube_provider(
    db: &Database,
    config_dir: &Path,
    agent_name: &str,
    requested_cube_provider_id: Option<&str>,
) -> Result<Option<String>, AppError> {
    let mut manifest = manifest_file_in(config_dir)?;
    let Some(record) = manifest
        .agents
        .iter_mut()
        .find(|record| record.name == agent_name.trim())
    else {
        return Ok(None);
    };
    let requested = requested_cube_provider_id
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            record
                .cube_provider_id
                .clone()
                .map(|id| id.trim().to_owned())
                .filter(|id| !id.is_empty())
        });
    let outcome = bind_record(db, config_dir, record, requested.as_deref())?;
    let bound = record.cube_provider_id.clone();
    if !matches!(outcome, BindOutcome::Unchanged) {
        save_manifest_in(config_dir, &manifest)?;
    }
    Ok(bound)
}

pub fn match_existing_cube_provider(
    db: &Database,
    config_dir: &Path,
    agent_name: &str,
    model: &str,
) -> Option<Provider> {
    let upstream = read_agent_upstream(config_dir, agent_name, None)?;
    let model = if model.trim().is_empty() {
        upstream.model.as_str()
    } else {
        model.trim()
    };
    find_matching_provider(db, &upstream.base_url, model, upstream.api_key.as_deref()).ok()?
}

pub fn unbind_cube_provider(config_dir: &Path, cube_provider_id: &str) -> Result<usize, AppError> {
    let cube_provider_id = cube_provider_id.trim();
    if cube_provider_id.is_empty() {
        return Ok(0);
    }
    let mut manifest = manifest_file_in(config_dir)?;
    let mut unbound = 0;
    for record in &mut manifest.agents {
        if record.cube_provider_id.as_deref() != Some(cube_provider_id) {
            continue;
        }
        record.cube_provider_id = None;
        record.cube_provider_owned = false;
        unbound += 1;
    }
    if unbound > 0 {
        save_manifest_in(config_dir, &manifest)?;
    }
    Ok(unbound)
}

pub fn take_owned_cube_provider_id(config_dir: &Path, agent_name: &str) -> Option<String> {
    let manifest = manifest_file_in(config_dir).ok()?;
    let record = manifest
        .agents
        .iter()
        .find(|record| record.name == agent_name.trim())?;
    if !record.cube_provider_owned {
        return None;
    }
    record
        .cube_provider_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
}

pub fn cube_provider_is_deletable(db: &Database, provider_id: &str) -> bool {
    let provider_id = provider_id.trim();
    if provider_id.is_empty() {
        return false;
    }
    let Ok(providers) = db.get_all_providers(APP_TYPE) else {
        return false;
    };
    if db
        .get_current_provider(APP_TYPE)
        .ok()
        .flatten()
        .as_deref()
        == Some(provider_id)
    {
        return false;
    }
    !providers.values().any(|provider| {
        provider.id != provider_id
            && provider.is_aggregate()
            && aggregate_references_provider(provider, provider_id)
    })
}

pub fn sync_linked_subagents_from_provider(
    config_dir: &Path,
    provider: &Provider,
) -> Result<usize, AppError> {
    let Some(base_url) = cube_provider_base_url(provider) else {
        return Ok(0);
    };
    let api_key = cube_provider_api_key(provider);
    let manifest = manifest_file_in(config_dir)?;
    let mut updated = 0;
    for record in &manifest.agents {
        if record.cube_provider_id.as_deref() != Some(provider.id.as_str()) {
            continue;
        }
        let agent_path = agent_path_in(config_dir, &record.name);
        let Ok(text) = std::fs::read_to_string(&agent_path) else {
            continue;
        };
        let Ok(mut doc) = text.parse::<DocumentMut>() else {
            continue;
        };
        let Some(custom) = doc
            .get_mut("model_providers")
            .and_then(Item::as_table_mut)
            .and_then(|providers| providers.get_mut(SUBAGENT_RUNTIME_PROVIDER_ID))
            .and_then(Item::as_table_mut)
        else {
            continue;
        };
        custom["base_url"] = value(base_url.as_str());
        write_text_file(&agent_path, &doc.to_string())?;
        if let Some(key) = api_key.as_deref() {
            let key_path = provider_key_path_in(config_dir, &record.provider_id);
            write_private_file_atomic(&key_path, key.as_bytes())?;
        }
        updated += 1;
    }
    Ok(updated)
}

fn bind_record(
    db: &Database,
    config_dir: &Path,
    record: &mut crate::codex_subagents::ManagedSubagent,
    requested: Option<&str>,
) -> Result<BindOutcome, AppError> {
    let Some(upstream) = read_agent_upstream(config_dir, &record.name, Some(&record.provider_id))
    else {
        return Ok(BindOutcome::Unchanged);
    };
    if let Some(id) = requested {
        if let Ok(Some(provider)) = db.get_provider_by_id(id, APP_TYPE) {
            let url_ok = cube_provider_base_url(&provider)
                .map(|url| canonical_endpoint(&url) == canonical_endpoint(&upstream.base_url))
                .unwrap_or(true);
            if url_ok {
                return apply_binding(
                    record,
                    &provider,
                    is_owned_provider(&provider, &record.name),
                    BindOutcome::Linked,
                );
            }
        }
    }
    if let Some(provider) = find_matching_provider(
        db,
        &upstream.base_url,
        &upstream.model,
        upstream.api_key.as_deref(),
    )? {
        return apply_binding(
            record,
            &provider,
            is_owned_provider(&provider, &record.name),
            BindOutcome::Linked,
        );
    }
    let Some(api_key) = upstream.api_key.as_deref().filter(|key| !key.is_empty()) else {
        log::warn!(
            "[Codex] subagent `{}` 无法创建 Cube 供应商：缺少 API key",
            record.name
        );
        return Ok(BindOutcome::Unchanged);
    };
    let provider = create_owned_cube_provider(
        db,
        &record.name,
        &upstream.model,
        &upstream.reasoning_effort,
        &upstream.base_url,
        api_key,
    )?;
    apply_binding(record, &provider, true, BindOutcome::Created)
}

fn apply_binding(
    record: &mut crate::codex_subagents::ManagedSubagent,
    provider: &Provider,
    owned: bool,
    outcome: BindOutcome,
) -> Result<BindOutcome, AppError> {
    let already = record.cube_provider_id.as_deref() == Some(provider.id.as_str())
        && record.cube_provider_owned == owned;
    record.cube_provider_id = Some(provider.id.clone());
    record.cube_provider_owned = owned;
    if already {
        return Ok(BindOutcome::Unchanged);
    }
    log::info!(
        "[Codex] subagent `{}` 绑定 Cube 供应商 `{}` ({})",
        record.name,
        provider.name,
        provider.id
    );
    Ok(outcome)
}

struct AgentUpstream {
    model: String,
    reasoning_effort: String,
    base_url: String,
    api_key: Option<String>,
}

fn read_agent_upstream(
    config_dir: &Path,
    name: &str,
    provider_id: Option<&str>,
) -> Option<AgentUpstream> {
    let path = agent_path_in(config_dir, name);
    let text = std::fs::read_to_string(path).ok()?;
    let agent: toml::Value = text.parse().ok()?;
    let model = agent
        .get("model")
        .and_then(toml::Value::as_str)
        .unwrap_or("")
        .trim()
        .to_owned();
    let base_url = agent
        .get("model_providers")
        .and_then(|providers| providers.get(SUBAGENT_RUNTIME_PROVIDER_ID))
        .and_then(|provider| provider.get("base_url"))
        .and_then(toml::Value::as_str)
        .unwrap_or("")
        .trim()
        .to_owned();
    if model.is_empty() || base_url.is_empty() {
        return None;
    }
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
    let key_path = agent
        .get("model_providers")
        .and_then(|providers| providers.get(SUBAGENT_RUNTIME_PROVIDER_ID))
        .and_then(|provider| provider.get("auth"))
        .and_then(|auth| auth.get("args"))
        .and_then(toml::Value::as_array)
        .and_then(|args| args.first())
        .and_then(toml::Value::as_str)
        .map(Path::new)
        .filter(|path| path.is_absolute())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| {
            provider_key_path_in(config_dir, provider_id.unwrap_or(name.trim()))
        });
    let api_key = std::fs::read_to_string(key_path)
        .ok()
        .map(|key| key.trim().to_owned())
        .filter(|key| !key.is_empty());
    Some(AgentUpstream {
        model,
        reasoning_effort,
        base_url,
        api_key,
    })
}

fn find_matching_provider(
    db: &Database,
    base_url: &str,
    model: &str,
    api_key: Option<&str>,
) -> Result<Option<Provider>, AppError> {
    let target = canonical_endpoint(base_url);
    if target.is_empty() {
        return Ok(None);
    }
    let providers = db.get_all_providers(APP_TYPE)?;
    let mut best: Option<(i32, Provider)> = None;
    for provider in providers.into_values() {
        if provider.is_aggregate() {
            continue;
        }
        let Some(url) = cube_provider_base_url(&provider) else {
            continue;
        };
        if canonical_endpoint(&url) != target {
            continue;
        }
        let mut score = 1;
        if provider_lists_model(&provider, model) {
            score += 100;
        }
        if provider_default_model(&provider).as_deref() == Some(model) {
            score += 20;
        }
        if let (Some(expected), Some(actual)) = (api_key, cube_provider_api_key(&provider)) {
            if expected.trim() == actual.trim() {
                score += 10;
            }
        }
        let replace = match &best {
            Some((best_score, best_provider)) => {
                score > *best_score || (score == *best_score && provider.id < best_provider.id)
            }
            None => true,
        };
        if replace {
            best = Some((score, provider));
        }
    }
    Ok(best.map(|(_, provider)| provider))
}

fn create_owned_cube_provider(
    db: &Database,
    agent_name: &str,
    model: &str,
    reasoning_effort: &str,
    base_url: &str,
    api_key: &str,
) -> Result<Provider, AppError> {
    let mut doc = DocumentMut::new();
    doc["model_provider"] = value(SUBAGENT_RUNTIME_PROVIDER_ID);
    doc["model"] = value(model);
    doc["model_reasoning_effort"] = value(reasoning_effort);
    let mut custom = Table::new();
    custom["name"] = value(format!("Codex Cube subagent: {agent_name}"));
    custom["base_url"] = value(base_url);
    custom["wire_api"] = value("responses");
    custom["requires_openai_auth"] = value(true);
    let mut providers = Table::new();
    providers.set_implicit(true);
    providers.insert(SUBAGENT_RUNTIME_PROVIDER_ID, Item::Table(custom));
    doc["model_providers"] = Item::Table(providers);

    let mut provider = Provider::with_id(
        uuid::Uuid::new_v4().to_string(),
        format!("Codex Cube subagent: {agent_name}"),
        json!({
            "auth": { "OPENAI_API_KEY": api_key },
            "config": doc.to_string(),
            "modelCatalog": {
                "models": [{ "model": model, "displayName": model }]
            }
        }),
        None,
    );
    provider.notes = Some(owned_notes(agent_name));
    db.save_provider(APP_TYPE, &provider)?;
    log::info!(
        "[Codex] 为 subagent `{}` 创建 Cube 供应商 `{}`",
        agent_name,
        provider.id
    );
    Ok(provider)
}

fn cube_provider_base_url(provider: &Provider) -> Option<String> {
    provider
        .settings_config
        .get("base_url")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            provider
                .settings_config
                .get("config")
                .and_then(|value| value.as_str())
                .and_then(extract_codex_base_url)
        })
}

fn cube_provider_api_key(provider: &Provider) -> Option<String> {
    extract_codex_api_key(
        provider.settings_config.get("auth"),
        provider
            .settings_config
            .get("config")
            .and_then(|value| value.as_str()),
    )
    .map(|key| key.trim().to_owned())
    .filter(|key| !key.is_empty())
}

fn provider_default_model(provider: &Provider) -> Option<String> {
    provider
        .settings_config
        .get("config")
        .and_then(|value| value.as_str())
        .and_then(|text| text.parse::<toml::Value>().ok())
        .and_then(|doc| {
            doc.get("model")
                .and_then(toml::Value::as_str)
                .map(str::trim)
                .filter(|model| !model.is_empty())
                .map(str::to_owned)
        })
}

fn provider_lists_model(provider: &Provider, model: &str) -> bool {
    let model = model.trim();
    if model.is_empty() {
        return false;
    }
    if provider_default_model(provider).as_deref() == Some(model) {
        return true;
    }
    let catalog_hit = provider
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
                    == Some(model)
            })
        });
    if catalog_hit {
        return true;
    }
    provider
        .settings_config
        .get("aggregateModels")
        .and_then(|models| models.as_array())
        .is_some_and(|models| {
            models.iter().any(|entry| {
                let listed = entry
                    .get("model")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    == Some(model);
                let upstream = entry
                    .get("upstreamModel")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    == Some(model);
                listed || upstream
            })
        })
}

fn aggregate_references_provider(provider: &Provider, provider_id: &str) -> bool {
    let members = provider
        .settings_config
        .get("memberProviderIds")
        .and_then(|value| value.as_array())
        .is_some_and(|ids| {
            ids.iter()
                .any(|id| id.as_str().map(str::trim) == Some(provider_id))
        });
    if members {
        return true;
    }
    provider
        .settings_config
        .get("aggregateModels")
        .and_then(|models| models.as_array())
        .is_some_and(|models| {
            models.iter().any(|entry| {
                entry
                    .get("providerId")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    == Some(provider_id)
            })
        })
}

fn is_owned_provider(provider: &Provider, agent_name: &str) -> bool {
    provider.notes.as_deref() == Some(&owned_notes(agent_name))
}

fn owned_notes(agent_name: &str) -> String {
    format!("{OWNED_NOTES_PREFIX}{agent_name}")
}

pub(crate) fn canonical_endpoint(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    if trimmed.len() >= 3 && trimmed[trimmed.len() - 3..].eq_ignore_ascii_case("/v1") {
        trimmed[..trimmed.len() - 3]
            .trim_end_matches('/')
            .to_owned()
    } else {
        trimmed.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_subagents::{ManagedSubagent, SubagentManifest};

    fn write_agent(dir: &Path, name: &str, model: &str, url: &str, key: Option<&str>) {
        std::fs::create_dir_all(dir.join("agents")).unwrap();
        let key_path = dir.join("codex-cube-agent-keys").join(format!("{name}.key"));
        if let Some(key) = key {
            std::fs::create_dir_all(key_path.parent().unwrap()).unwrap();
            std::fs::write(&key_path, key).unwrap();
        }
        let auth = if key.is_some() {
            format!(
                "\n[model_providers.custom.auth]\ncommand = \"/bin/cat\"\nargs = [\"{}\"]\n",
                key_path.display()
            )
        } else {
            String::new()
        };
        std::fs::write(
            dir.join("agents").join(format!("{name}.toml")),
            format!(
                "name = \"{name}\"\nmodel = \"{model}\"\nmodel_reasoning_effort = \"high\"\n\n[model_providers.custom]\nbase_url = \"{url}\"\nwire_api = \"responses\"\n{auth}"
            ),
        )
        .unwrap();
    }

    fn write_manifest(dir: &Path, name: &str, provider_id: &str, agent_type: &str) {
        let manifest = SubagentManifest {
            version: 1,
            agents: vec![ManagedSubagent {
                name: name.to_owned(),
                provider_id: provider_id.to_owned(),
                key_path: Some(
                    dir.join("codex-cube-agent-keys")
                        .join(format!("{provider_id}.key"))
                        .to_string_lossy()
                        .into_owned(),
                ),
                provider_managed: true,
                original_provider: None,
                original_agent_toml: None,
                original_key: None,
                created_at: "2026-08-18T00:00:00Z".to_owned(),
                agent_type: agent_type.to_owned(),
                cube_provider_id: None,
                cube_provider_owned: false,
            }],
        };
        save_manifest_in(dir, &manifest).unwrap();
    }

    fn cube_provider(id: &str, name: &str, url: &str, model: &str, key: &str) -> Provider {
        Provider::with_id(
            id.to_owned(),
            name.to_owned(),
            json!({
                "auth": { "OPENAI_API_KEY": key },
                "config": format!(
                    "model_provider = \"custom\"\nmodel = \"{model}\"\n\n[model_providers.custom]\nname = \"{name}\"\nbase_url = \"{url}\"\nwire_api = \"responses\"\n"
                ),
                "modelCatalog": { "models": [{ "model": model }] }
            }),
            None,
        )
    }

    #[test]
    fn canonical_endpoint_strips_trailing_slash_and_v1() {
        assert_eq!(
            canonical_endpoint("https://790053500.com/v1/"),
            "https://790053500.com"
        );
        assert_eq!(
            canonical_endpoint("https://opencode.ai/zen/go/v1"),
            "https://opencode.ai/zen/go"
        );
    }

    #[test]
    fn sync_links_existing_cube_provider_by_url_and_model() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        write_agent(
            dir,
            "grok-4-6",
            "grok-4.6",
            "https://790053500.com",
            Some("sk-grok"),
        );
        write_manifest(dir, "grok-4-6", "grok-4-6", "");
        let db = Database::memory().unwrap();
        db.save_provider(
            APP_TYPE,
            &cube_provider(
                "xai-uuid",
                "xAI (Grok)",
                "https://790053500.com/v1",
                "grok-4.6",
                "sk-grok",
            ),
        )
        .unwrap();

        let report = sync_managed_subagent_cube_providers(&db, dir).unwrap();
        assert_eq!(report.linked, 1);
        assert_eq!(report.created, 0);
        assert_eq!(report.repaired_agent_type, 1);
        let manifest = manifest_file_in(dir).unwrap();
        assert_eq!(manifest.agents[0].cube_provider_id.as_deref(), Some("xai-uuid"));
        assert!(!manifest.agents[0].cube_provider_owned);
        assert_eq!(manifest.agents[0].agent_type, "worker");
    }

    #[test]
    fn sync_creates_owned_provider_when_no_url_match() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        write_agent(
            dir,
            "grok-4-6",
            "grok-4.6",
            "https://790053500.com",
            Some("sk-grok"),
        );
        write_manifest(dir, "grok-4-6", "grok-4-6", "worker");
        let db = Database::memory().unwrap();
        db.save_provider(
            APP_TYPE,
            &cube_provider(
                "opencode",
                "OpenCode",
                "https://opencode.ai/zen/go/v1",
                "deepseek-v4-flash",
                "sk-other",
            ),
        )
        .unwrap();

        let report = sync_managed_subagent_cube_providers(&db, dir).unwrap();
        assert_eq!(report.created, 1);
        let manifest = manifest_file_in(dir).unwrap();
        let bound = manifest.agents[0].cube_provider_id.clone().unwrap();
        assert!(manifest.agents[0].cube_provider_owned);
        let created = db.get_provider_by_id(&bound, APP_TYPE).unwrap().unwrap();
        assert!(created.name.contains("grok-4-6"));
        assert_eq!(
            canonical_endpoint(&cube_provider_base_url(&created).unwrap()),
            "https://790053500.com"
        );
    }

    #[test]
    fn bind_prefers_requested_cube_provider_id() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        write_agent(
            dir,
            "grok-4-6",
            "grok-4.6",
            "https://790053500.com",
            Some("sk-grok"),
        );
        write_manifest(dir, "grok-4-6", "grok-4-6", "worker");
        let db = Database::memory().unwrap();
        db.save_provider(
            APP_TYPE,
            &cube_provider(
                "xai-a",
                "Grok A",
                "https://790053500.com",
                "grok-4.6",
                "sk-grok",
            ),
        )
        .unwrap();
        db.save_provider(
            APP_TYPE,
            &cube_provider(
                "xai-b",
                "Grok B",
                "https://790053500.com",
                "grok-4.6",
                "sk-grok",
            ),
        )
        .unwrap();

        let bound = bind_subagent_cube_provider(&db, dir, "grok-4-6", Some("xai-b"))
            .unwrap()
            .unwrap();
        assert_eq!(bound, "xai-b");
    }

    #[test]
    fn unbind_clears_stale_cube_provider_id() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        write_agent(
            dir,
            "grok-4-6",
            "grok-4.6",
            "https://790053500.com",
            Some("sk-grok"),
        );
        write_manifest(dir, "grok-4-6", "grok-4-6", "worker");
        let mut manifest = manifest_file_in(dir).unwrap();
        manifest.agents[0].cube_provider_id = Some("deleted-id".to_owned());
        save_manifest_in(dir, &manifest).unwrap();
        assert_eq!(unbind_cube_provider(dir, "deleted-id").unwrap(), 1);
        assert!(manifest_file_in(dir).unwrap().agents[0]
            .cube_provider_id
            .is_none());
    }
}
