//! Deep link module tests

use super::mcp::parse_mcp_apps;
use super::parser::parse_deeplink_url;
use super::prompt::import_prompt_from_deeplink;
use super::provider::parse_and_merge_config;
use super::utils::{infer_homepage_from_endpoint, validate_url};
use super::DeepLinkImportRequest;
use crate::AppType;
use crate::{store::AppState, Database};
use base64::prelude::*;
use std::{env, ffi::OsString, sync::Arc};

struct TestHomeGuard {
    _dir: tempfile::TempDir,
    original_home: Option<OsString>,
    original_userprofile: Option<OsString>,
    original_test_home: Option<OsString>,
}

impl TestHomeGuard {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("create isolated test home");
        let original_home = env::var_os("HOME");
        let original_userprofile = env::var_os("USERPROFILE");
        let original_test_home = env::var_os("CODEX_CUBE_TEST_HOME");

        env::set_var("HOME", dir.path());
        env::set_var("USERPROFILE", dir.path());
        env::set_var("CODEX_CUBE_TEST_HOME", dir.path());

        Self {
            _dir: dir,
            original_home,
            original_userprofile,
            original_test_home,
        }
    }
}

impl Drop for TestHomeGuard {
    fn drop(&mut self) {
        match &self.original_test_home {
            Some(value) => env::set_var("CODEX_CUBE_TEST_HOME", value),
            None => env::remove_var("CODEX_CUBE_TEST_HOME"),
        }
        match &self.original_userprofile {
            Some(value) => env::set_var("USERPROFILE", value),
            None => env::remove_var("USERPROFILE"),
        }
        match &self.original_home {
            Some(value) => env::set_var("HOME", value),
            None => env::remove_var("HOME"),
        }
    }
}

// =============================================================================
// Parser Tests
// =============================================================================

#[test]
fn test_parse_deeplink_with_notes() {
    let url = "codexcube://v1/import?resource=provider&app=codex&name=Codex&homepage=https%3A%2F%2Fcodex.com&endpoint=https%3A%2F%2Fapi.codex.com&apiKey=key123&notes=Test%20notes";

    let request = parse_deeplink_url(url).unwrap();

    assert_eq!(request.notes, Some("Test notes".to_string()));
}

#[test]
fn test_parse_ccswitch_alias_scheme() {
    // Relay stations generate ccswitch:// one-click import links; Codex Cube
    // accepts the scheme as an alias of codexcube://.
    let url = "ccswitch://v1/import?resource=provider&app=codex&name=Relay%20Station&endpoint=https%3A%2F%2Fapi.relay.example%2Fv1&apiKey=sk-relay-test";

    let request = parse_deeplink_url(url).unwrap();

    assert_eq!(request.resource, "provider");
    assert_eq!(request.app, Some("codex".to_string()));
    assert_eq!(request.name, Some("Relay Station".to_string()));
    assert_eq!(
        request.endpoint,
        Some("https://api.relay.example/v1".to_string())
    );
    assert_eq!(request.api_key, Some("sk-relay-test".to_string()));
}

#[test]
#[test]
fn test_parse_invalid_scheme() {
    let url = "https://v1/import?resource=provider&app=codex&name=Test";

    let result = parse_deeplink_url(url);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Invalid scheme"));
}

#[test]
fn test_parse_unsupported_version() {
    let url = "codexcube://v2/import?resource=provider&app=codex&name=Test";

    let result = parse_deeplink_url(url);
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Unsupported protocol version"));
}

#[test]
fn test_parse_missing_required_field() {
    // Name is still required even in v3.8+ (only homepage/endpoint/apiKey are optional)
    let url = "codexcube://v1/import?resource=provider&app=codex";

    let result = parse_deeplink_url(url);
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Missing 'name' parameter"));
}

// =============================================================================
// Utils Tests
// =============================================================================

#[test]
fn test_validate_invalid_url() {
    let result = validate_url("not-a-url", "test");
    assert!(result.is_err());
}

#[test]
fn test_validate_invalid_scheme() {
    let result = validate_url("ftp://example.com", "test");
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("must be http or https"));
}

#[test]
fn test_infer_homepage() {
    assert_eq!(
        infer_homepage_from_endpoint("https://api.anthropic.com/v1"),
        Some("https://anthropic.com".to_string())
    );
    assert_eq!(
        infer_homepage_from_endpoint("https://api-test.company.com/v1"),
        Some("https://test.company.com".to_string())
    );
    assert_eq!(
        infer_homepage_from_endpoint("https://example.com"),
        Some("https://example.com".to_string())
    );
}

// =============================================================================
// Provider Tests
// =============================================================================

#[test]
#[test]
#[test]
fn test_deeplink_usage_script_does_not_copy_provider_credentials() {
    use super::provider::build_provider_from_request;

    let request = DeepLinkImportRequest {
        version: "v1".to_string(),
        resource: "provider".to_string(),
        app: Some("codex".to_string()),
        name: Some("Test Codex".to_string()),
        homepage: Some("https://example.com".to_string()),
        endpoint: Some("https://api.example.com/v1/".to_string()),
        api_key: Some("sk-main".to_string()),
        icon: None,
        model: None,
        notes: None,
        haiku_model: None,
        sonnet_model: None,
        opus_model: None,
        config: None,
        config_format: None,
        config_url: None,
        apps: None,
        repo: None,
        directory: None,
        branch: None,
        content: None,
        description: None,
        enabled: None,
        usage_enabled: Some(true),
        usage_script: None,
        usage_api_key: None,
        usage_base_url: None,
        usage_access_token: None,
        usage_user_id: None,
        usage_auto_interval: None,
    };

    let provider = build_provider_from_request(&AppType::Codex, &request).unwrap();
    let script = provider
        .meta
        .as_ref()
        .and_then(|meta| meta.usage_script.as_ref())
        .expect("usage script should be created");

    assert!(script.enabled);
    assert_eq!(script.api_key, None);
    assert_eq!(script.base_url, None);
}

/// 构造一个只带用量脚本字段的 provider 请求，其余保持最小。
fn usage_script_request(code: &str, usage_enabled: Option<bool>) -> DeepLinkImportRequest {
    DeepLinkImportRequest {
        version: "v1".to_string(),
        resource: "provider".to_string(),
        app: Some("codex".to_string()),
        name: Some("Test Codex".to_string()),
        homepage: Some("https://example.com".to_string()),
        endpoint: Some("https://api.example.com/v1/".to_string()),
        api_key: Some("sk-main".to_string()),
        icon: None,
        model: None,
        notes: None,
        haiku_model: None,
        sonnet_model: None,
        opus_model: None,
        config: None,
        config_format: None,
        config_url: None,
        apps: None,
        repo: None,
        directory: None,
        branch: None,
        content: None,
        description: None,
        enabled: None,
        usage_enabled,
        usage_script: Some(BASE64_STANDARD.encode(code)),
        usage_api_key: None,
        usage_base_url: None,
        usage_access_token: None,
        usage_user_id: None,
        usage_auto_interval: None,
    }
}

#[test]
fn test_deeplink_usage_script_is_not_enabled_merely_by_carrying_code() {
    use super::provider::build_provider_from_request;

    // deeplink 是第三方构造、经浏览器抵达的不可信载荷。「带了代码」不是用户的
    // 启用决定——否则一条链接就能让这段 JS 在用户从未勾选的情况下进入启用态。
    let code = "export async function query() { return { cost: 0 }; }";
    let request = usage_script_request(code, None);

    let provider = build_provider_from_request(&AppType::Codex, &request).unwrap();
    let script = provider
        .meta
        .as_ref()
        .and_then(|meta| meta.usage_script.as_ref())
        .expect("usage script should still be created");

    assert!(
        !script.enabled,
        "缺省必须是未启用；`带了代码`不构成用户的启用决定"
    );
    // 代码本身仍要保留：确认框要展示它，用户之后也可在应用内手动开启。
    assert_eq!(script.code, code);
}

#[test]
fn test_deeplink_usage_script_honors_an_explicit_enable_request_from_the_link() {
    use super::provider::build_provider_from_request;

    // `usageEnabled=true` 是**链接作者**的请求，不是用户的选择——用户的同意体现在
    // 看过确认框里完整的脚本正文与启用状态之后点了导入。收紧默认值不能顺手把这条
    // 正常通路改坏：合作伙伴的预设链接靠它一次性配好用量查询。
    let code = "export async function query() { return { cost: 0 }; }";
    let request = usage_script_request(code, Some(true));

    let provider = build_provider_from_request(&AppType::Codex, &request).unwrap();
    let script = provider
        .meta
        .as_ref()
        .and_then(|meta| meta.usage_script.as_ref())
        .expect("usage script should be created");

    assert!(script.enabled);
    assert_eq!(script.code, code);
}

#[test]
fn test_deeplink_usage_script_omits_explicit_credentials_that_match_provider() {
    use super::provider::build_provider_from_request;

    let request = DeepLinkImportRequest {
        version: "v1".to_string(),
        resource: "provider".to_string(),
        app: Some("codex".to_string()),
        name: Some("Test Codex".to_string()),
        homepage: Some("https://example.com".to_string()),
        endpoint: Some("https://api.example.com/v1/".to_string()),
        api_key: Some("sk-main".to_string()),
        icon: None,
        model: None,
        notes: None,
        haiku_model: None,
        sonnet_model: None,
        opus_model: None,
        config: None,
        config_format: None,
        config_url: None,
        apps: None,
        repo: None,
        directory: None,
        branch: None,
        content: None,
        description: None,
        enabled: None,
        usage_enabled: Some(true),
        usage_script: None,
        usage_api_key: Some(" sk-main ".to_string()),
        usage_base_url: Some(" https://api.example.com/v1/ ".to_string()),
        usage_access_token: None,
        usage_user_id: None,
        usage_auto_interval: None,
    };

    let provider = build_provider_from_request(&AppType::Codex, &request).unwrap();
    let script = provider
        .meta
        .as_ref()
        .and_then(|meta| meta.usage_script.as_ref())
        .expect("usage script should be created");

    assert_eq!(script.api_key, None);
    assert_eq!(script.base_url, None);
}

#[test]
fn test_deeplink_usage_script_preserves_distinct_usage_credentials() {
    use super::provider::build_provider_from_request;

    let request = DeepLinkImportRequest {
        version: "v1".to_string(),
        resource: "provider".to_string(),
        app: Some("codex".to_string()),
        name: Some("Test Codex".to_string()),
        homepage: Some("https://example.com".to_string()),
        endpoint: Some("https://api.example.com/v1".to_string()),
        api_key: Some("sk-main".to_string()),
        icon: None,
        model: None,
        notes: None,
        haiku_model: None,
        sonnet_model: None,
        opus_model: None,
        config: None,
        config_format: None,
        config_url: None,
        apps: None,
        repo: None,
        directory: None,
        branch: None,
        content: None,
        description: None,
        enabled: None,
        usage_enabled: Some(true),
        usage_script: None,
        usage_api_key: Some(" sk-usage ".to_string()),
        usage_base_url: Some(" https://usage.example/api/ ".to_string()),
        usage_access_token: None,
        usage_user_id: None,
        usage_auto_interval: None,
    };

    let provider = build_provider_from_request(&AppType::Codex, &request).unwrap();
    let script = provider
        .meta
        .as_ref()
        .and_then(|meta| meta.usage_script.as_ref())
        .expect("usage script should be created");

    assert_eq!(script.api_key.as_deref(), Some("sk-usage"));
    assert_eq!(
        script.base_url.as_deref(),
        Some("https://usage.example/api")
    );
}

#[test]
#[test]
fn test_parse_and_merge_config_codex_uses_bearer_token() {
    let config_toml = r#"model_provider = "rightcode"
model = "gpt-5-codex"

[model_providers.rightcode]
base_url = "https://rightcode.example/v1"
wire_api = "responses"
experimental_bearer_token = "sk-rightcode"
"#;
    let config_json = serde_json::json!({
        "auth": {},
        "config": config_toml,
    })
    .to_string();
    let config_b64 = BASE64_STANDARD.encode(config_json.as_bytes());

    let request = DeepLinkImportRequest {
        version: "v1".to_string(),
        resource: "provider".to_string(),
        app: Some("codex".to_string()),
        name: Some("RightCode".to_string()),
        config: Some(config_b64),
        config_format: Some("json".to_string()),
        ..Default::default()
    };

    let merged = parse_and_merge_config(&request).unwrap();

    assert_eq!(merged.api_key, Some("sk-rightcode".to_string()));
    assert_eq!(
        merged.endpoint,
        Some("https://rightcode.example/v1".to_string())
    );
    assert_eq!(
        merged.homepage,
        Some("https://rightcode.example".to_string())
    );
    assert_eq!(merged.model, Some("gpt-5-codex".to_string()));
}

#[test]
fn test_parse_and_merge_config_url_override() {
    let config_json = r#"{"auth":{"OPENAI_API_KEY":"sk-old"},"config":"base_url = \"https://old.example/v1\"\nwire_api = \"responses\""}"#;
    let config_b64 = BASE64_STANDARD.encode(config_json.as_bytes());

    let request = DeepLinkImportRequest {
        version: "v1".to_string(),
        resource: "provider".to_string(),
        app: Some("codex".to_string()),
        name: Some("Test".to_string()),
        homepage: None,
        endpoint: Some("https://old.example/v1".to_string()),
        api_key: Some("sk-new".to_string()), // URL param should override
        icon: None,
        model: None,
        notes: None,
        haiku_model: None,
        sonnet_model: None,
        opus_model: None,
        config: Some(config_b64),
        config_format: Some("json".to_string()),
        config_url: None,
        apps: None,
        repo: None,
        directory: None,
        branch: None,
        content: None,
        description: None,
        enabled: None,
        usage_enabled: None,
        usage_script: None,
        usage_api_key: None,
        usage_base_url: None,
        usage_access_token: None,
        usage_user_id: None,
        usage_auto_interval: None,
    };

    let merged = parse_and_merge_config(&request).unwrap();

    // URL param should take priority
    assert_eq!(merged.api_key, Some("sk-new".to_string()));
    // Codex endpoint comes from the request endpoint param
    assert_eq!(merged.endpoint, Some("https://old.example/v1".to_string()));
}

#[test]
#[test]
#[test]
#[serial_test::serial]
fn test_import_prompt_allows_space_in_base64_content() {
    let _test_home = TestHomeGuard::new();
    let url = "codexcube://v1/import?resource=prompt&app=codex&name=PromptPlus&content=Pj4+";
    let request = parse_deeplink_url(url).unwrap();

    // URL decoded content may have "+" become space
    assert_eq!(request.content.as_deref(), Some("Pj4 "));

    let db = Arc::new(Database::memory().expect("create memory db"));
    let state = AppState::new(db.clone());

    let prompt_id = import_prompt_from_deeplink(&state, request.clone()).expect("import prompt");

    let prompts = state.db.get_prompts("codex").expect("get prompts");
    let prompt = prompts.get(&prompt_id).expect("prompt saved");

    assert_eq!(prompt.content, ">>>");
    assert_eq!(prompt.name, request.name.unwrap());
}

// =============================================================================
// MCP Tests
// =============================================================================

#[test]
fn test_parse_mcp_apps() {
    let apps = parse_mcp_apps("codex").unwrap();
    assert!(apps.codex);

    let err = parse_mcp_apps("invalid").unwrap_err();
    assert!(err.to_string().contains("Invalid app"));
}

#[test]
fn test_parse_prompt_deeplink() {
    let content = "Hello World";
    let content_b64 = BASE64_STANDARD.encode(content);
    let url = format!(
        "codexcube://v1/import?resource=prompt&app=codex&name=test&content={}&description=desc&enabled=true",
        content_b64
    );

    let request = parse_deeplink_url(&url).unwrap();
    assert_eq!(request.resource, "prompt");
    assert_eq!(request.app.unwrap(), "codex");
    assert_eq!(request.name.unwrap(), "test");
    assert_eq!(request.content.unwrap(), content_b64);
    assert_eq!(request.description.unwrap(), "desc");
    assert!(request.enabled.unwrap());
}

#[test]
#[test]
fn test_parse_mcp_deeplink() {
    let config = r#"{"mcpServers":{"test":{"command":"echo"}}}"#;
    let config_b64 = BASE64_STANDARD.encode(config);
    let url = format!(
        "codexcube://v1/import?resource=mcp&apps=codex&config={}&enabled=true",
        config_b64
    );

    let request = parse_deeplink_url(&url).unwrap();
    assert_eq!(request.resource, "mcp");
    assert_eq!(request.apps.unwrap(), "codex");
    assert_eq!(request.config.unwrap(), config_b64);
    assert!(request.enabled.unwrap());
}

#[test]
#[test]
fn test_parse_skill_deeplink() {
    let url = "codexcube://v1/import?resource=skill&repo=owner/repo&directory=skills&branch=dev";
    let request = parse_deeplink_url(url).unwrap();

    assert_eq!(request.resource, "skill");
    assert_eq!(request.repo.unwrap(), "owner/repo");
    assert_eq!(request.directory.unwrap(), "skills");
    assert_eq!(request.branch.unwrap(), "dev");
}

// =============================================================================
// Multiple Endpoints Tests
// =============================================================================

#[test]
fn test_parse_multiple_endpoints_comma_separated() {
    let url = "codexcube://v1/import?resource=provider&app=codex&name=Test&endpoint=https%3A%2F%2Fapi1.example.com,https%3A%2F%2Fapi2.example.com,https%3A%2F%2Fapi3.example.com&apiKey=sk-test";

    let request = parse_deeplink_url(url).unwrap();

    assert!(request.endpoint.is_some());
    let endpoint = request.endpoint.unwrap();
    // Should contain all endpoints comma-separated
    assert!(endpoint.contains("https://api1.example.com"));
    assert!(endpoint.contains("https://api2.example.com"));
    assert!(endpoint.contains("https://api3.example.com"));
}

#[test]
fn test_parse_single_endpoint_backward_compatible() {
    // Old format with single endpoint should still work
    let url = "codexcube://v1/import?resource=provider&app=codex&name=Test&endpoint=https%3A%2F%2Fapi.example.com&apiKey=sk-test";

    let request = parse_deeplink_url(url).unwrap();

    assert_eq!(
        request.endpoint,
        Some("https://api.example.com".to_string())
    );
}

#[test]
fn test_parse_endpoints_with_spaces_trimmed() {
    let url = "codexcube://v1/import?resource=provider&app=codex&name=Test&endpoint=https%3A%2F%2Fapi1.example.com%20,%20https%3A%2F%2Fapi2.example.com&apiKey=sk-test";

    let request = parse_deeplink_url(url).unwrap();

    // Validation should pass (spaces are trimmed during validation)
    assert!(request.endpoint.is_some());
}

#[test]
fn test_infer_homepage_from_endpoint_without_homepage() {
    // Test that homepage is auto-inferred from endpoint when not provided
    assert_eq!(
        infer_homepage_from_endpoint("https://api.cubence.com/v1"),
        Some("https://cubence.com".to_string())
    );
    assert_eq!(
        infer_homepage_from_endpoint("https://cubence.com"),
        Some("https://cubence.com".to_string())
    );
}
