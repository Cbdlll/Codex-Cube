//! Deep link URL parser
//!
//! Parses codexcube:// and ccswitch:// URLs into DeepLinkImportRequest
//! structures. The ccswitch:// scheme is accepted as an alias so one-click
//! import links from relay stations keep working in Codex Cube.

use super::utils::validate_url;
use super::DeepLinkImportRequest;
use crate::error::AppError;
use std::collections::HashMap;
use url::Url;

/// Parse a codexcube:// or ccswitch:// URL into a DeepLinkImportRequest.
///
/// Expected format:
/// codexcube://v1/import?resource={type}&... (also accepts ccswitch://)
pub fn parse_deeplink_url(url_str: &str) -> Result<DeepLinkImportRequest, AppError> {
    // Parse URL
    let url = Url::parse(url_str)
        .map_err(|e| AppError::InvalidInput(format!("Invalid deep link URL: {e}")))?;

    // Validate scheme: ccswitch:// is an alias kept for relay-station
    // one-click import compatibility.
    let scheme = url.scheme();
    if !matches!(scheme, "codexcube" | "ccswitch") {
        return Err(AppError::InvalidInput(format!(
            "Invalid scheme: expected 'codexcube' or 'ccswitch', got '{scheme}'"
        )));
    }

    // Extract version from host
    let version = url
        .host_str()
        .ok_or_else(|| AppError::InvalidInput("Missing version in URL host".to_string()))?
        .to_string();

    // Validate version
    if version != "v1" {
        return Err(AppError::InvalidInput(format!(
            "Unsupported protocol version: {version}"
        )));
    }

    // Extract path (should be "/import")
    let path = url.path();
    if path != "/import" {
        return Err(AppError::InvalidInput(format!(
            "Invalid path: expected '/import', got '{path}'"
        )));
    }

    // Parse query parameters
    let params: HashMap<String, String> = url.query_pairs().into_owned().collect();

    // Extract and validate resource type
    let resource = params
        .get("resource")
        .ok_or_else(|| AppError::InvalidInput("Missing 'resource' parameter".to_string()))?
        .clone();

    // Dispatch to appropriate parser based on resource type
    match resource.as_str() {
        "provider" => parse_provider_deeplink(&params, version, resource),
        other => Err(AppError::InvalidInput(format!(
            "Unsupported resource type: {other}"
        ))),
    }
}

/// Parse provider deep link parameters
fn parse_provider_deeplink(
    params: &HashMap<String, String>,
    version: String,
    resource: String,
) -> Result<DeepLinkImportRequest, AppError> {
    let app = params
        .get("app")
        .ok_or_else(|| AppError::InvalidInput("Missing 'app' parameter".to_string()))?
        .clone();

    // Validate app type
    if !matches!(app.as_str(), "codex") {
        return Err(AppError::InvalidInput(format!(
            "Invalid app type: must be 'codex', got '{app}'"
        )));
    }

    let name = params
        .get("name")
        .ok_or_else(|| AppError::InvalidInput("Missing 'name' parameter".to_string()))?
        .clone();

    // Make these optional for config file auto-fill (v3.8+)
    let homepage = params.get("homepage").cloned();
    let endpoint = params.get("endpoint").cloned();
    let api_key = params.get("apiKey").cloned();

    // Validate URLs only if provided
    if let Some(ref hp) = homepage {
        if !hp.is_empty() {
            validate_url(hp, "homepage")?;
        }
    }
    // Validate each endpoint (supports comma-separated multiple URLs)
    if let Some(ref ep) = endpoint {
        for (i, url) in ep.split(',').enumerate() {
            let trimmed = url.trim();
            if !trimmed.is_empty() {
                validate_url(trimmed, &format!("endpoint[{i}]"))?;
            }
        }
    }

    // Extract optional fields
    let model = params.get("model").cloned();
    let notes = params.get("notes").cloned();
    let haiku_model = params.get("haikuModel").cloned();
    let sonnet_model = params.get("sonnetModel").cloned();
    let opus_model = params.get("opusModel").cloned();
    let icon = params
        .get("icon")
        .map(|v| v.trim().to_lowercase())
        .filter(|v| !v.is_empty());
    let config = params.get("config").cloned();
    let config_format = params.get("configFormat").cloned();
    let config_url = params.get("configUrl").cloned();
    let enabled = params.get("enabled").and_then(|v| v.parse::<bool>().ok());

    // Extract usage script fields (v3.9+)
    let usage_enabled = params
        .get("usageEnabled")
        .and_then(|v| v.parse::<bool>().ok());
    let usage_script = params.get("usageScript").cloned();
    let usage_api_key = params.get("usageApiKey").cloned();
    let usage_base_url = params.get("usageBaseUrl").cloned();
    let usage_access_token = params.get("usageAccessToken").cloned();
    let usage_user_id = params.get("usageUserId").cloned();
    let usage_auto_interval = params
        .get("usageAutoInterval")
        .and_then(|v| v.parse::<u64>().ok());

    Ok(DeepLinkImportRequest {
        version,
        resource,
        app: Some(app),
        name: Some(name),
        enabled,
        homepage,
        endpoint,
        api_key,
        icon,
        model,
        notes,
        haiku_model,
        sonnet_model,
        opus_model,
        content: None,
        description: None,
        apps: None,
        repo: None,
        directory: None,
        branch: None,
        config,
        config_format,
        config_url,
        usage_enabled,
        usage_script,
        usage_api_key,
        usage_base_url,
        usage_access_token,
        usage_user_id,
        usage_auto_interval,
    })
}

