//! Third-party relay sanitization for native Codex `Responses` passthrough.
//!
//! OpenAI-compatible relays (non-official gateways such as 鑫旺Neko API) are
//! stricter than the real OpenAI backend and reject the following with
//! `400 {"error":{"message":"Upstream request failed"}}`:
//!
//! - Orphaned tool-call halves: `function_call` / `custom_tool_call` require a
//!   matching `function_call_output` / `custom_tool_call_output` with the same
//!   `call_id` in the same request, and vice versa. Codex trims history to the
//!   context window, which can leave orphans whose counterpart was pruned.
//! - `reasoning` items carrying OpenAI-private `encrypted_content`. Console Go
//!   (opencode.ai/zen/go) instead requires the plain `content`
//!   (`reasoning_text`) to be passed back verbatim and rejects summary-only
//!   thinking blocks; other relays (鑫旺Neko API, Hiyo codex) do the opposite —
//!   they reject `reasoning_text` content / `encrypted_content` and only accept
//!   `summary`-only thinking blocks. Per-upstream handling is selected by
//!   [`relay_profile_for_url`].
//! - `web_search_call` items with an `output` field. Only the Console Go
//!   upstream (opencode.ai/zen/go) additionally requires a top-level `queries`
//!   array, which Codex only carries inside `action` (it returns 400 `missing
//!   field queries` mid-conversation); there the field is synthesized from
//!   `action`. Other relays (e.g. 鑫旺Neko API) follow the official `action`
//!   shape and reject a synthesized top-level `queries` with a generic 400, so
//!   the synthesis is gated per-upstream.
//! - Item `id`s that do not use the expected per-type prefix (`msg_`, `rs_`,
//!   `fc_`, `fco_`, `ctc_`, `ctco_`, `ws_`). History produced by another
//!   provider (e.g. DeepSeek via the aggregate) uses raw UUID ids, which relays
//!   reject; they are normalized to the expected prefix here.
//! - The OpenAI-private `internal_chat_message_metadata_passthrough` field.
//!
//! This module is gated to non-official providers so real OpenAI passthrough
//! stays untouched. Deterministic and idempotent: running it twice changes
//! nothing the second time.

use std::collections::HashSet;

use serde_json::Value;

/// Expected `id` prefix per Responses item type.
const ITEM_ID_PREFIXES: &[(&str, &str)] = &[
    ("message", "msg_"),
    ("reasoning", "rs_"),
    ("function_call", "fc_"),
    ("function_call_output", "fco_"),
    ("custom_tool_call", "ctc_"),
    ("custom_tool_call_output", "ctco_"),
    ("web_search_call", "ws_"),
];

/// Sanitize a native Codex Responses request body in place for third-party
/// relay upstreams. Returns whether anything changed.
pub(crate) fn sanitize_relay_responses_request(body: &mut Value, profile: RelayProfile) -> bool {
    let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) else {
        return false;
    };

    let function_calls = tool_call_ids(input, "function_call");
    let function_outputs = tool_call_ids(input, "function_call_output");
    let custom_calls = tool_call_ids(input, "custom_tool_call");
    let custom_outputs = tool_call_ids(input, "custom_tool_call_output");

    let mut changed = false;
    input.retain(|item| {
        let Some(item) = item.as_object() else {
            return true;
        };
        let Some(item_type) = item.get("type").and_then(Value::as_str) else {
            return true;
        };
        let call_id = item.get("call_id").and_then(Value::as_str);
        let keep = match item_type {
            "function_call" => call_id.is_none_or(|id| function_outputs.contains(id)),
            "function_call_output" => call_id.is_none_or(|id| function_calls.contains(id)),
            "custom_tool_call" => call_id.is_none_or(|id| custom_outputs.contains(id)),
            "custom_tool_call_output" => call_id.is_none_or(|id| custom_calls.contains(id)),
            "reasoning" => match profile {
                // Console Go rejects summary-only thinking blocks.
                RelayProfile::ConsoleGo => has_non_empty_reasoning_text(item.get("content")),
                // Other relays accept summary-only (and even empty) blocks.
                RelayProfile::OpenAiCompat => true,
            },
            _ => true,
        };
        changed |= !keep;
        keep
    });

    for item in input.iter_mut() {
        let Some(obj) = item.as_object_mut() else {
            continue;
        };
        let item_type = obj
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        changed |= obj
            .remove("internal_chat_message_metadata_passthrough")
            .is_some();
        if item_type == "reasoning" {
            match profile {
                RelayProfile::ConsoleGo => {
                    // Keep `content` (`reasoning_text`): Console Go returns 400
                    // `The reasoning_text ... must be passed back` when it is
                    // missing. Only OpenAI-private `encrypted_content` is
                    // dropped.
                    changed |= obj.remove("encrypted_content").is_some();
                }
                RelayProfile::OpenAiCompat => {
                    // 鑫旺Neko API / Hiyo codex reject `reasoning_text` content
                    // and `encrypted_content` (generic 400), but accept
                    // `summary`-only thinking blocks. Drop both so history
                    // keeps only the opaque summary.
                    changed |= obj.remove("content").is_some();
                    changed |= obj.remove("encrypted_content").is_some();
                }
            }
        }
        if item_type == "web_search_call" {
            changed |= obj.remove("output").is_some();
            // Only Console Go requires the top-level `queries` array; other
            // relays (e.g. 鑫旺Neko API) follow the official `action` shape and
            // reject a synthesized `queries` with a generic 400.
            if profile == RelayProfile::ConsoleGo && !obj.contains_key("queries") {
                if let Some(queries) = web_search_call_queries(obj) {
                    obj.insert("queries".to_string(), Value::Array(queries));
                    changed = true;
                }
            }
        }
        if let Some((_, prefix)) = ITEM_ID_PREFIXES.iter().find(|(item, _)| *item == item_type) {
            if let Some(id) = obj.get("id").and_then(Value::as_str) {
                if !id.starts_with(prefix) {
                    let normalized = format!("{prefix}{id}");
                    obj.insert("id".to_string(), Value::String(normalized));
                    changed = true;
                }
            }
        }
    }

    changed
}

fn tool_call_ids(input: &[Value], item_type: &str) -> HashSet<String> {
    input
        .iter()
        .filter_map(|item| {
            let obj = item.as_object()?;
            if obj.get("type").and_then(Value::as_str) != Some(item_type) {
                return None;
            }
            obj.get("call_id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
        })
        .collect()
}

fn has_non_empty_reasoning_text(content: Option<&Value>) -> bool {
    content.and_then(Value::as_array).is_some_and(|entries| {
        entries.iter().any(|entry| {
            entry.get("type").and_then(Value::as_str) == Some("reasoning_text")
                && entry
                    .get("text")
                    .and_then(Value::as_str)
                    .is_some_and(|text| !text.trim().is_empty())
        })
    })
}

/// Upstream-specific relay behavior for native Codex `Responses` history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RelayProfile {
    /// Console Go (opencode.ai/zen/go): keeps `reasoning_text` content,
    /// drops summary-only thinking blocks, and synthesizes the top-level
    /// `web_search_call.queries` array.
    ConsoleGo,
    /// Other OpenAI-compatible relays (鑫旺Neko API, Hiyo codex, ...): keep
    /// `summary`-only thinking blocks, strip `reasoning_text` content and
    /// `encrypted_content`, and keep the official `action`-shaped
    /// `web_search_call` items untouched.
    OpenAiCompat,
}

/// Resolve the [`RelayProfile`] for an upstream URL.
pub(crate) fn relay_profile_for_url(url: &str) -> RelayProfile {
    let host = url.to_ascii_lowercase();
    if host.contains("opencode.ai") && host.contains("/zen/go") {
        RelayProfile::ConsoleGo
    } else {
        RelayProfile::OpenAiCompat
    }
}

/// Derive the relay-required top-level `queries` array for a
/// `web_search_call` history item from Codex's private `action` field.
/// Returns `None` when the item already has a usable field so the caller can
/// leave it untouched; otherwise prefers `action.queries`, then
/// `action.query`, then falls back to an empty array so strict serde
/// deserializers that require the field still accept the item.
fn web_search_call_queries(obj: &serde_json::Map<String, Value>) -> Option<Vec<Value>> {
    if obj.get("queries").is_some() {
        return None;
    }
    let action = obj.get("action");
    if let Some(queries) = action
        .and_then(|a| a.get("queries"))
        .and_then(Value::as_array)
    {
        let queries: Vec<Value> = queries
            .iter()
            .filter_map(|q| q.as_str().map(|text| Value::String(text.to_string())))
            .collect();
        if !queries.is_empty() {
            return Some(queries);
        }
    }
    if let Some(query) = action
        .and_then(|a| a.get("query"))
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
    {
        return Some(vec![Value::String(query.to_string())]);
    }
    Some(Vec::new())
}

/// Aggregate counts about `reasoning` items in a relay Responses `body`, for
/// forwarder debug logging. Counters only — never contains text, tokens, or
/// secrets. Returns `None` when `body` has no `input` array or no `reasoning`
/// items to summarize.
pub(crate) fn summarize_relay_reasoning_history(body: &Value) -> Option<ReasoningHistoryStats> {
    let input = body.get("input").and_then(Value::as_array)?;
    let reasoning_items: Vec<&Value> = input
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("reasoning"))
        .collect();
    if reasoning_items.is_empty() {
        return None;
    }

    let mut stats = ReasoningHistoryStats {
        reasoning_total: 0,
        reasoning_text_valid: 0,
        content_missing_null_empty: 0,
        encrypted_content_present: 0,
        summary_only: 0,
    };
    for item in reasoning_items {
        stats.reasoning_total += 1;
        let has_valid_text = has_non_empty_reasoning_text(item.get("content"));
        if has_valid_text {
            stats.reasoning_text_valid += 1;
        }
        let content_usable = item
            .get("content")
            .and_then(Value::as_array)
            .is_some_and(|entries| !entries.is_empty());
        if !content_usable {
            stats.content_missing_null_empty += 1;
        }
        if item.get("encrypted_content").is_some() {
            stats.encrypted_content_present += 1;
        }
        let has_summary = item
            .get("summary")
            .and_then(Value::as_array)
            .is_some_and(|entries| !entries.is_empty());
        if !has_valid_text && has_summary {
            stats.summary_only += 1;
        }
    }
    Some(stats)
}

/// Counts about `reasoning` items in a relay Responses body. Debug-log only;
/// no content, tokens, or secrets are stored.
#[derive(Debug)]
pub(crate) struct ReasoningHistoryStats {
    /// Total number of `reasoning` items in the input.
    pub reasoning_total: usize,
    /// Items whose `content` has at least one non-empty `reasoning_text` entry.
    pub reasoning_text_valid: usize,
    /// Items whose `content` is absent, null, not an array, or an empty array.
    pub content_missing_null_empty: usize,
    /// Items carrying the OpenAI-private `encrypted_content` field.
    pub encrypted_content_present: usize,
    /// Items with a non-empty `summary` but no valid `reasoning_text`.
    pub summary_only: usize,
}

#[cfg(test)]
mod tests {
    use super::{
        relay_profile_for_url, sanitize_relay_responses_request, summarize_relay_reasoning_history,
        RelayProfile,
    };
    use serde_json::json;

    #[test]
    fn drops_orphan_halves_and_strips_relay_unsupported_fields() {
        let mut body = json!({
            "model": "gpt-5.6-sol",
            "input": [
                { "type": "message", "id": "fd73811f-5595-4b0c-aaa4-4a7f8f1b7910", "role": "user", "content": [{ "type": "input_text", "text": "hi" }], "internal_chat_message_metadata_passthrough": "x" },
                { "type": "function_call", "id": "fc-1", "call_id": "call-1", "name": "read", "arguments": "{}" },
                { "type": "function_call_output", "id": "fco-1", "call_id": "call-1", "output": "ok" },
                { "type": "function_call_output", "id": "fco-2", "call_id": "call-orphan", "output": "stale" },
                { "type": "custom_tool_call", "id": "ct-1", "call_id": "ct-call-1", "name": "apply_patch", "input": {}, "status": "completed" },
                { "type": "custom_tool_call_output", "id": "cto-1", "call_id": "ct-call-1", "output": [] },
                { "type": "custom_tool_call_output", "id": "cto-2", "call_id": "ct-call-orphan", "output": [] },
                { "type": "reasoning", "id": "da239668-bde3-40e9-9d5b-64aac87928a5", "summary": [{ "type": "summary_text", "text": "plan" }], "content": [{ "type": "reasoning_text", "text": "think" }], "encrypted_content": "gAAAA", "internal_chat_message_metadata_passthrough": "y" },
                { "type": "web_search_call", "id": "ws-1", "status": "completed", "output": [{ "type": "web_search_results", "results": [] }] }
            ]
        });

        let changed = sanitize_relay_responses_request(&mut body, RelayProfile::ConsoleGo);
        assert!(changed, "sanitizer must report a change");

        let input = body["input"].as_array().expect("input array");
        let types: Vec<&str> = input
            .iter()
            .filter_map(|item| item.get("type").and_then(|v| v.as_str()))
            .collect();
        assert_eq!(
            types,
            vec![
                "message",
                "function_call",
                "function_call_output",
                "custom_tool_call",
                "custom_tool_call_output",
                "reasoning",
                "web_search_call"
            ],
            "orphan halves must be dropped, paired items must survive"
        );
        assert_eq!(
            input[0].get("id").and_then(|v| v.as_str()),
            Some("msg_fd73811f-5595-4b0c-aaa4-4a7f8f1b7910"),
            "raw-UUID message id must be normalized to the msg_ prefix"
        );
        let reasoning = &input[5];
        assert_eq!(
            reasoning.get("id").and_then(|v| v.as_str()),
            Some("rs_da239668-bde3-40e9-9d5b-64aac87928a5"),
            "raw-UUID reasoning id must be normalized to the rs_ prefix"
        );
        assert!(
            reasoning.get("encrypted_content").is_none(),
            "OpenAI-private encrypted_content must be stripped"
        );
        assert_eq!(
            reasoning
                .get("content")
                .and_then(|v| v.as_array())
                .map(|a| a.len()),
            Some(1),
            "reasoning_text content must be preserved for relays that require it"
        );
        assert!(
            input.iter().all(|item| item
                .get("internal_chat_message_metadata_passthrough")
                .is_none()),
            "OpenAI-internal passthrough field must be stripped"
        );
        assert!(
            input.iter().all(|item| {
                item.get("type").and_then(|v| v.as_str()) != Some("web_search_call")
                    || item.get("output").is_none()
            }),
            "web_search_call output must be stripped"
        );

        let changed_again = sanitize_relay_responses_request(&mut body, RelayProfile::ConsoleGo);
        assert!(!changed_again, "sanitizer must be idempotent");
    }

    #[test]
    fn synthesizes_queries_for_codex_web_search_call_history() {
        let mut body = json!({
            "model": "deepseek-v4-flash",
            "input": [
                {
                    "type": "web_search_call",
                    "id": "ws_tmp_fe12a25czdw",
                    "status": "completed",
                    "action": { "type": "search", "queries": ["a", "b"] },
                    "internal_chat_message_metadata_passthrough": "x"
                },
                {
                    "type": "web_search_call",
                    "id": "ws_tmp_p89zbc1dsle",
                    "status": "completed",
                    "action": { "type": "search", "query": "single query" }
                },
                {
                    "type": "web_search_call",
                    "id": "ws_tmp_open",
                    "status": "completed",
                    "action": { "type": "open_page", "url": "https://example.com" }
                },
                {
                    "type": "web_search_call",
                    "id": "ws_keep",
                    "status": "completed",
                    "queries": ["already present"]
                }
            ]
        });

        assert!(sanitize_relay_responses_request(
            &mut body,
            RelayProfile::ConsoleGo
        ));
        let input = body["input"].as_array().expect("input array");
        assert_eq!(input[0]["queries"], json!(["a", "b"]));
        assert_eq!(input[1]["queries"], json!(["single query"]));
        assert_eq!(
            input[2]["queries"],
            json!([]),
            "no query/url intent still needs a present field"
        );
        assert_eq!(
            input[3]["queries"],
            json!(["already present"]),
            "existing queries must be untouched"
        );
        assert!(
            input.iter().all(|item| item
                .get("internal_chat_message_metadata_passthrough")
                .is_none()),
            "passthrough must still be stripped"
        );

        let changed_again = sanitize_relay_responses_request(&mut body, RelayProfile::ConsoleGo);
        assert!(!changed_again, "sanitizer must be idempotent");
    }

    #[test]
    fn keeps_valid_conversation_unchanged() {
        let mut body = json!({
            "model": "gpt-5.6-sol",
            "input": [
                { "type": "message", "id": "msg_1", "role": "user", "content": [{ "type": "input_text", "text": "hi" }] },
                { "type": "function_call", "id": "fc_1", "call_id": "call-1", "name": "read", "arguments": "{}" },
                { "type": "function_call_output", "id": "fco_1", "call_id": "call-1", "output": "ok" }
            ]
        });
        let before = body.to_string();
        let changed = sanitize_relay_responses_request(&mut body, RelayProfile::OpenAiCompat);
        assert!(!changed, "clean history must be untouched");
        assert_eq!(body.to_string(), before);
    }

    #[test]
    fn removes_reasoning_items_without_valid_reasoning_text() {
        let mut body = json!({
            "model": "gpt-5.6-sol",
            "input": [
                { "type": "reasoning", "id": "rs_1", "content": [{ "type": "reasoning_text", "text": "think" }] },
                { "type": "reasoning", "id": "rs_2", "content": null, "encrypted_content": "gAAAA" },
                { "type": "reasoning", "id": "rs_3", "summary": [{ "type": "summary_text", "text": "plan" }] },
                { "type": "reasoning", "id": "rs_4", "content": [] },
                { "type": "reasoning", "id": "rs_5", "summary": [{ "type": "summary_text", "text": "opaque plan" }], "encrypted_content": "gAAAA" }
            ]
        });

        let changed = sanitize_relay_responses_request(&mut body, RelayProfile::ConsoleGo);
        assert!(changed, "sanitizer must report a change");

        let input = body["input"].as_array().expect("input array");
        assert_eq!(
            input.len(),
            1,
            "null-content, absent-content, summary-only, and empty-content reasoning items must be removed"
        );
        assert_eq!(
            input[0].get("id").and_then(|v| v.as_str()),
            Some("rs_1"),
            "reasoning item with valid reasoning_text must be retained"
        );
        assert_eq!(
            input[0]["content"][0]["text"].as_str(),
            Some("think"),
            "retained item must keep its original reasoning_text, not an invented one"
        );
        assert!(
            input
                .iter()
                .all(|item| item.get("encrypted_content").is_none()),
            "surviving reasoning items must not carry encrypted_content"
        );

        let changed_again = sanitize_relay_responses_request(&mut body, RelayProfile::ConsoleGo);
        assert!(!changed_again, "sanitizer must be idempotent");
    }

    #[test]
    fn handles_codex_reasoning_shapes_seen_in_session_history() {
        let mut body = json!({
            "model": "deepseek-v4-flash",
            "input": [
                {
                    "type": "reasoning",
                    "id": "opaque-null-content",
                    "summary": [],
                    "content": null,
                    "encrypted_content": "opaque"
                },
                {
                    "type": "reasoning",
                    "id": "opaque-null-encrypted",
                    "summary": [],
                    "content": [
                        { "type": "reasoning_text", "text": "keep this text" }
                    ],
                    "encrypted_content": null
                },
                {
                    "type": "reasoning",
                    "id": "opaque-blank-text",
                    "summary": [],
                    "content": [
                        { "type": "reasoning_text", "text": "  " }
                    ],
                    "encrypted_content": "opaque"
                }
            ]
        });

        assert!(sanitize_relay_responses_request(
            &mut body,
            RelayProfile::ConsoleGo
        ));
        let input = body["input"].as_array().expect("input array");
        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["type"], "reasoning");
        assert_eq!(input[0]["id"], "rs_opaque-null-encrypted");
        assert_eq!(input[0]["content"][0]["text"], "keep this text");
        assert!(input[0].get("encrypted_content").is_none());
    }

    #[test]
    fn summarizes_reasoning_history_without_secrets() {
        let body = json!({
            "model": "gpt-5.6-sol",
            "input": [
                { "type": "message", "id": "msg_1", "role": "user", "content": [] },
                { "type": "reasoning", "id": "rs_1", "content": [{ "type": "reasoning_text", "text": "think" }] },
                { "type": "reasoning", "id": "rs_2", "content": null, "encrypted_content": "gAAAA" },
                { "type": "reasoning", "id": "rs_3", "summary": [{ "type": "summary_text", "text": "plan" }] },
                { "type": "reasoning", "id": "rs_4", "content": [] },
                { "type": "reasoning", "id": "rs_5", "summary": [{ "type": "summary_text", "text": "opaque plan" }], "encrypted_content": "gAAAA" }
            ]
        });

        let stats = summarize_relay_reasoning_history(&body).expect("stats");
        assert_eq!(stats.reasoning_total, 5);
        assert_eq!(stats.reasoning_text_valid, 1);
        assert_eq!(stats.content_missing_null_empty, 4);
        assert_eq!(stats.encrypted_content_present, 2);
        assert_eq!(stats.summary_only, 2);

        assert!(summarize_relay_reasoning_history(&json!({})).is_none());
        assert!(
            summarize_relay_reasoning_history(&json!({ "input": [{ "type": "message" }] }))
                .is_none()
        );
    }

    #[test]
    fn resolves_relay_profile_per_upstream_url() {
        assert_eq!(
            relay_profile_for_url("https://opencode.ai/zen/go/v1/responses"),
            RelayProfile::ConsoleGo
        );
        assert_eq!(
            relay_profile_for_url("https://opencode.ai/zen/go/v1/responses?stream=true"),
            RelayProfile::ConsoleGo
        );
        assert_eq!(
            relay_profile_for_url("https://api.790053500.com/v1/responses"),
            RelayProfile::OpenAiCompat
        );
        assert_eq!(
            relay_profile_for_url("https://codex.hiyo.top/v1/responses"),
            RelayProfile::OpenAiCompat
        );
        assert_eq!(
            relay_profile_for_url("https://api.openai.com/v1/responses"),
            RelayProfile::OpenAiCompat
        );
    }

    #[test]
    fn strips_reasoning_content_for_openai_compat_relays() {
        let mut body = json!({
            "model": "gpt-5.6-terra",
            "input": [
                { "type": "message", "id": "msg_1", "role": "user", "content": [{ "type": "input_text", "text": "hi" }] },
                {
                    "type": "reasoning",
                    "id": "rs_1",
                    "summary": [{ "type": "summary_text", "text": "plan" }],
                    "content": [{ "type": "reasoning_text", "text": "think" }],
                    "encrypted_content": "gAAAA"
                },
                {
                    "type": "reasoning",
                    "id": "rs_2",
                    "content": [{ "type": "reasoning_text", "text": "no summary" }]
                }
            ]
        });

        assert!(sanitize_relay_responses_request(
            &mut body,
            RelayProfile::OpenAiCompat
        ));
        let input = body["input"].as_array().expect("input array");
        assert_eq!(
            input.len(),
            3,
            "summary-only and empty reasoning blocks are accepted"
        );
        for item in input.iter().skip(1) {
            assert!(
                item.get("content").is_none(),
                "reasoning_text content must be stripped for OpenAI-compat relays"
            );
            assert!(
                item.get("encrypted_content").is_none(),
                "encrypted_content must be stripped for OpenAI-compat relays"
            );
        }
        assert_eq!(
            input[1]["summary"][0]["text"], "plan",
            "summary must be preserved"
        );
        assert!(
            input[2].get("summary").is_none() || input[2]["summary"].as_array().is_some(),
            "content-only reasoning keeps no invented summary"
        );

        let changed_again = sanitize_relay_responses_request(&mut body, RelayProfile::OpenAiCompat);
        assert!(!changed_again, "sanitizer must be idempotent");
    }

    #[test]
    fn does_not_synthesize_queries_for_relays_without_console_go_requirement() {
        let mut body = json!({
            "model": "gpt-5.6-terra",
            "input": [
                { "type": "web_search_call", "id": "ws_1", "status": "completed", "action": { "type": "search", "query": "q1" } }
            ]
        });
        let before = body.to_string();
        assert!(!sanitize_relay_responses_request(
            &mut body,
            RelayProfile::OpenAiCompat
        ));
        assert_eq!(
            body.to_string(),
            before,
            "non-Console-Go relays keep the official action-only shape"
        );
    }
}
