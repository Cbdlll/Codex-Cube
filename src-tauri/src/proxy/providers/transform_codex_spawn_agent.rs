//! Rewrite Codex `spawn_agent` arguments so cube-dispatch cannot inherit
//! `[agents].default_subagent_model` or the parent conversation model, and so
//! the collaboration runtime receives a built-in role it can instantiate.
//!
//! Codex executes `spawn_agent` locally from the function-call arguments the
//! coordinator model emitted. A generic `{ "agent_type": "worker" }` with no
//! `model` falls through to the global subagent default instead of the
//! registered `~/.codex/agents/<name>.toml`. Current Codex collaboration spawn
//! only instantiates built-in roles (`worker` / `explorer` / `default`); a
//! registered file name such as `grok-4-6` returns "agent type is currently
//! not available" even when the tool schema lists it. This module keeps
//! `agent_type` as that built-in role (mapping a custom name back) and injects
//! `model` plus `reasoning_effort` from the registered TOML.

use std::collections::HashSet;

use serde_json::{json, Map, Value};

use crate::codex_agent_workflow::{resolve_dispatch_target, DispatchProfile, DispatchResolve};
use crate::codex_config::get_codex_config_dir;

const SPAWN_AGENT: &str = "spawn_agent";
const SPAWN_AGENT_FLAT: &str = "collaboration__spawn_agent";
const SPAWN_ID_KEYS: [&str; 3] = ["id", "item_id", "call_id"];
/// Generic-role spawn that cannot be resolved must not inherit `[agents]`
/// defaults. Codex treats this as an unknown agent / model and fails the spawn.
const UNRESOLVED_SPAWN_SENTINEL: &str = "cube-dispatch-unresolved";

#[derive(Debug, Default)]
pub(crate) struct SpawnRewriteState {
    spawn_item_ids: HashSet<String>,
}

impl SpawnRewriteState {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn has_pending_spawn_ids(&self) -> bool {
        !self.spawn_item_ids.is_empty()
    }
}

pub(crate) fn is_spawn_agent_name(name: &str) -> bool {
    let name = name.trim();
    name == SPAWN_AGENT || name == SPAWN_AGENT_FLAT || name.ends_with("__spawn_agent")
}

pub(crate) fn rewrite_spawn_agent_value(value: &mut Value) -> bool {
    let mut state = SpawnRewriteState::new();
    rewrite_spawn_agent_in_event(value, &mut state)
}

pub(crate) fn rewrite_spawn_agent_in_event(
    event: &mut Value,
    state: &mut SpawnRewriteState,
) -> bool {
    let mut changed = walk_function_calls(event, state);
    changed |= rewrite_function_call_argument_event(event, state);
    changed
}

fn walk_function_calls(value: &mut Value, state: &mut SpawnRewriteState) -> bool {
    let mut changed = false;
    match value {
        Value::Array(items) => {
            for item in items {
                changed |= walk_function_calls(item, state);
            }
        }
        Value::Object(_) => {
            changed |= rewrite_function_call_item(value, state);
            if let Some(obj) = value.as_object_mut() {
                for child in obj.values_mut() {
                    changed |= walk_function_calls(child, state);
                }
            }
        }
        _ => {}
    }
    changed
}

fn rewrite_function_call_item(item: &mut Value, state: &mut SpawnRewriteState) -> bool {
    let Some(obj) = item.as_object_mut() else {
        return false;
    };
    if obj.get("type").and_then(Value::as_str) != Some("function_call") {
        return false;
    }
    let name = obj
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    if !is_spawn_agent_name(&name) && !id_is_remembered_spawn(obj, state) {
        return false;
    }
    remember_spawn_item(obj, state);
    rewrite_arguments_value(obj)
}

fn remember_spawn_item(obj: &Map<String, Value>, state: &mut SpawnRewriteState) {
    for key in SPAWN_ID_KEYS {
        if let Some(id) = obj.get(key).and_then(Value::as_str) {
            let id = id.trim();
            if !id.is_empty() {
                state.spawn_item_ids.insert(id.to_owned());
            }
        }
    }
}

fn id_is_remembered_spawn(obj: &Map<String, Value>, state: &SpawnRewriteState) -> bool {
    SPAWN_ID_KEYS.iter().any(|key| {
        obj.get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .is_some_and(|id| !id.is_empty() && state.spawn_item_ids.contains(id))
    })
}

fn rewrite_function_call_argument_event(event: &mut Value, state: &mut SpawnRewriteState) -> bool {
    let Some(obj) = event.as_object_mut() else {
        return false;
    };
    let event_type = obj.get("type").and_then(Value::as_str).unwrap_or("");
    let is_delta = event_type == "response.function_call_arguments.delta";
    let is_done = event_type == "response.function_call_arguments.done";
    if !is_delta && !is_done {
        return false;
    }
    let name = obj.get("name").and_then(Value::as_str).unwrap_or("");
    let nested_is_spawn = obj
        .get("item")
        .and_then(Value::as_object)
        .and_then(|item| item.get("name"))
        .and_then(Value::as_str)
        .is_some_and(is_spawn_agent_name);
    let known_spawn =
        is_spawn_agent_name(name) || nested_is_spawn || id_is_remembered_spawn(obj, state);
    if !known_spawn {
        return false;
    }
    remember_spawn_item(obj, state);
    let mut changed = rewrite_arguments_value(obj);
    if is_delta {
        changed |= rewrite_named_arguments_field(obj, "delta");
    }
    changed
}

fn rewrite_arguments_value(obj: &mut Map<String, Value>) -> bool {
    rewrite_named_arguments_field(obj, "arguments")
}

fn rewrite_named_arguments_field(obj: &mut Map<String, Value>, key: &str) -> bool {
    match obj.get(key) {
        Some(Value::String(raw)) => {
            let Some(rewritten) = rewrite_spawn_agent_arguments(raw) else {
                return false;
            };
            if rewritten == *raw {
                return false;
            }
            obj.insert(key.to_string(), json!(rewritten));
            true
        }
        Some(Value::Object(_)) => {
            let Some(Value::Object(args)) = obj.get_mut(key) else {
                return false;
            };
            apply_resolved_profile_to_args(args)
        }
        _ => false,
    }
}

pub(crate) fn rewrite_spawn_agent_arguments(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut value: Value = serde_json::from_str(trimmed).ok()?;
    let Some(args) = value.as_object_mut() else {
        return None;
    };
    if !apply_resolved_profile_to_args(args) {
        return None;
    }
    serde_json::to_string(&value).ok()
}

fn apply_resolved_profile_to_args(args: &mut Map<String, Value>) -> bool {
    let agent_type = args
        .get("agent_type")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    match resolve_dispatch_target(&get_codex_config_dir(), agent_type.as_deref()) {
        DispatchResolve::NoWorkflow => false,
        DispatchResolve::Unresolved { agent_type } => {
            log::error!(
                "[Codex] cube-dispatch spawn_agent agent_type={agent_type:?} could not be resolved from the registered agent TOML; refusing to inherit [agents] defaults or the parent model"
            );
            apply_unresolved_fail_closed(args, agent_type.as_str())
        }
        DispatchResolve::Resolved(profile) => apply_profile_to_args(args, &profile),
    }
}

fn apply_unresolved_fail_closed(args: &mut Map<String, Value>, requested: &str) -> bool {
    let mut changed = false;
    if requested.is_empty() || crate::codex_subagents::AGENT_TYPES.contains(&requested) {
        args.insert("agent_type".to_string(), json!(UNRESOLVED_SPAWN_SENTINEL));
        changed = true;
    }
    let model = args
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    if model.is_empty() {
        args.insert("model".to_string(), json!(UNRESOLVED_SPAWN_SENTINEL));
        changed = true;
    }
    changed
}

fn apply_profile_to_args(args: &mut Map<String, Value>, profile: &DispatchProfile) -> bool {
    let previous_agent_type = args
        .get("agent_type")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("")
        .to_owned();
    let previous_model = args
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("")
        .to_owned();
    let previous_effort = args
        .get("reasoning_effort")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("")
        .to_owned();
    let mut changed = false;
    // Collaboration spawn instantiates built-in roles only. Schema may list a
    // registered file name, but passing it as agent_type fails with
    // "agent type is currently not available". Keep or map to the role and
    // route via model + reasoning_effort instead.
    let target_agent_type = if crate::codex_subagents::AGENT_TYPES.contains(&profile.role.as_str())
    {
        profile.role.as_str()
    } else {
        crate::codex_subagents::AGENT_TYPE_WORKER
    };
    if previous_agent_type != target_agent_type {
        log::info!(
            "[Codex] cube-dispatch spawn_agent role={} mapped agent_type {} → {} (runtime instantiates built-in roles only)",
            profile.role,
            previous_agent_type,
            target_agent_type
        );
        args.insert("agent_type".to_string(), json!(target_agent_type));
        changed = true;
    }
    if previous_model != profile.model {
        if previous_model.is_empty() {
            log::info!(
                "[Codex] cube-dispatch spawn_agent role={} agent={} injected model={}",
                profile.role,
                profile.agent_name,
                profile.model
            );
        } else {
            log::warn!(
                "[Codex] cube-dispatch spawn_agent role={} agent={} corrected model {} → {}",
                profile.role,
                profile.agent_name,
                previous_model,
                profile.model
            );
        }
        args.insert("model".to_string(), json!(profile.model));
        changed = true;
    }
    if previous_effort != profile.reasoning_effort {
        if previous_effort.is_empty() {
            log::info!(
                "[Codex] cube-dispatch spawn_agent role={} agent={} injected reasoning_effort={}",
                profile.role,
                profile.agent_name,
                profile.reasoning_effort
            );
        } else {
            log::warn!(
                "[Codex] cube-dispatch spawn_agent role={} agent={} corrected reasoning_effort {} → {}",
                profile.role,
                profile.agent_name,
                previous_effort,
                profile.reasoning_effort
            );
        }
        args.insert(
            "reasoning_effort".to_string(),
            json!(profile.reasoning_effort),
        );
        changed = true;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_agent_workflow::{install, RoleAgents, WORKFLOW_MODE_SKILL};
    use serde_json::json;

    struct TestHomeGuard(Option<std::ffi::OsString>);
    impl TestHomeGuard {
        fn set(home: &std::path::Path) -> Self {
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

    fn write_agent(dir: &std::path::Path, name: &str, model: &str, effort: &str) {
        let agents = dir.join("agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(
            agents.join(format!("{name}.toml")),
            format!(
                "name = \"{name}\"\n\
description = \"{name} worker\"\n\
developer_instructions = \"do the work\"\n\
model = \"{model}\"\n\
model_reasoning_effort = \"{effort}\"\n"
            ),
        )
        .unwrap();
    }

    fn install_worker(dir: &std::path::Path, name: &str) {
        let mut roles = RoleAgents::default();
        roles.worker = vec![name.to_string()];
        install(
            dir,
            name,
            &[name.to_string()],
            &roles,
            WORKFLOW_MODE_SKILL,
            "high",
        )
        .unwrap();
    }

    fn isolated_codex_home() -> (tempfile::TempDir, TestHomeGuard, std::path::PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let home = TestHomeGuard::set(temp.path());
        let dir = crate::codex_config::get_codex_config_dir();
        std::fs::create_dir_all(&dir).unwrap();
        (temp, home, dir)
    }

    #[test]
    #[serial_test::serial]
    fn rewrite_injects_model_for_generic_worker_without_model() {
        let (_temp, _home, dir) = isolated_codex_home();
        write_agent(&dir, "grok-4-6", "grok-4.6", "high");
        install_worker(&dir, "grok-4-6");

        let rewritten = rewrite_spawn_agent_arguments(
            r#"{"agent_type":"worker","fork_turns":"none","message":"implement the scope"}"#,
        )
        .expect("generic worker spawn must be rewritten");
        let value: Value = serde_json::from_str(&rewritten).unwrap();
        assert_eq!(value["agent_type"], "worker");
        assert_eq!(value["model"], "grok-4.6");
        assert_eq!(value["reasoning_effort"], "high");
        assert_eq!(value["fork_turns"], "none");
    }

    #[test]
    #[serial_test::serial]
    fn rewrite_corrects_mismatched_generic_worker_model() {
        let (_temp, _home, dir) = isolated_codex_home();
        write_agent(&dir, "grok-4-6", "grok-4.6", "high");
        install_worker(&dir, "grok-4-6");

        let rewritten = rewrite_spawn_agent_arguments(
            r#"{"agent_type":"worker","model":"deepseek-v4-flash","reasoning_effort":"max"}"#,
        )
        .expect("mismatched model must be corrected");
        let value: Value = serde_json::from_str(&rewritten).unwrap();
        assert_eq!(value["agent_type"], "worker");
        assert_eq!(value["model"], "grok-4.6");
        assert_eq!(value["reasoning_effort"], "high");
    }

    #[test]
    #[serial_test::serial]
    fn rewrite_fail_closes_unresolved_generic_worker() {
        let (_temp, _home, dir) = isolated_codex_home();
        let mut roles = crate::codex_agent_workflow::RoleAgents::default();
        roles.worker = vec!["missing-agent".to_string()];
        crate::codex_agent_workflow::install(
            &dir,
            "missing-agent",
            &["missing-agent".to_string()],
            &roles,
            crate::codex_agent_workflow::WORKFLOW_MODE_SKILL,
            "high",
        )
        .unwrap();

        let rewritten = rewrite_spawn_agent_arguments(
            r#"{"agent_type":"worker","fork_turns":"none","message":"implement the scope"}"#,
        )
        .expect("unresolved generic spawn must be rewritten fail-closed");
        let value: Value = serde_json::from_str(&rewritten).unwrap();
        assert_eq!(value["agent_type"], UNRESOLVED_SPAWN_SENTINEL);
        assert_eq!(value["model"], UNRESOLVED_SPAWN_SENTINEL);
    }

    #[test]
    #[serial_test::serial]
    fn rewrite_maps_custom_agent_type_to_builtin_role() {
        let (_temp, _home, dir) = isolated_codex_home();
        write_agent(&dir, "grok-4-6", "grok-4.6", "high");
        install_worker(&dir, "grok-4-6");

        let rewritten =
            rewrite_spawn_agent_arguments(r#"{"agent_type":"grok-4-6","fork_turns":"none"}"#)
                .expect("custom agent_type must map to a built-in role and receive explicit model");
        let value: Value = serde_json::from_str(&rewritten).unwrap();
        assert_eq!(value["agent_type"], "worker");
        assert_eq!(value["model"], "grok-4.6");
        assert_eq!(value["reasoning_effort"], "high");
    }

    #[test]
    #[serial_test::serial]
    fn rewrite_keeps_explorer_role_when_injecting_registered_model() {
        let (_temp, _home, dir) = isolated_codex_home();
        write_agent(&dir, "grok-4-6", "grok-4.6", "high");
        let mut roles = RoleAgents::default();
        roles.explorer = vec!["grok-4-6".to_string()];
        install(
            &dir,
            "grok-4-6",
            &["grok-4-6".to_string()],
            &roles,
            WORKFLOW_MODE_SKILL,
            "high",
        )
        .unwrap();

        let rewritten = rewrite_spawn_agent_arguments(
            r#"{"agent_type":"explorer","fork_turns":"none","message":"inspect the scope"}"#,
        )
        .expect("explorer spawn must keep the built-in role");
        let value: Value = serde_json::from_str(&rewritten).unwrap();
        assert_eq!(value["agent_type"], "explorer");
        assert_eq!(value["model"], "grok-4.6");
        assert_eq!(value["reasoning_effort"], "high");
    }

    #[test]
    #[serial_test::serial]
    fn rewrite_skips_when_workflow_is_not_installed() {
        let (_temp, _home, _dir) = isolated_codex_home();
        assert_eq!(
            rewrite_spawn_agent_arguments(r#"{"agent_type":"worker","fork_turns":"none"}"#),
            None
        );
    }

    #[test]
    #[serial_test::serial]
    fn rewrite_function_call_item_and_arguments_done_event() {
        let (_temp, _home, dir) = isolated_codex_home();
        write_agent(&dir, "grok-4-6", "grok-4.6", "high");
        install_worker(&dir, "grok-4-6");

        let mut added = json!({
            "type": "response.output_item.added",
            "item": {
                "id": "fc_spawn",
                "type": "function_call",
                "namespace": "collaboration",
                "name": "spawn_agent",
                "arguments": "{\"agent_type\":\"worker\",\"fork_turns\":\"none\"}"
            }
        });
        let mut state = SpawnRewriteState::new();
        assert!(rewrite_spawn_agent_in_event(&mut added, &mut state));
        assert!(added["item"]["arguments"]
            .as_str()
            .unwrap()
            .contains("grok-4.6"));

        let mut done = json!({
            "type": "response.function_call_arguments.done",
            "item_id": "fc_spawn",
            "arguments": "{\"agent_type\":\"worker\"}"
        });
        assert!(rewrite_spawn_agent_in_event(&mut done, &mut state));
        let args: Value = serde_json::from_str(done["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(args["model"], "grok-4.6");
        assert_eq!(args["reasoning_effort"], "high");

        let mut done_by_call_id = json!({
            "type": "response.function_call_arguments.done",
            "call_id": "fc_spawn",
            "arguments": "{\"agent_type\":\"worker\",\"fork_turns\":\"none\"}"
        });
        assert!(rewrite_spawn_agent_in_event(
            &mut done_by_call_id,
            &mut state
        ));
        let args: Value =
            serde_json::from_str(done_by_call_id["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(args["model"], "grok-4.6");
    }

    #[test]
    #[serial_test::serial]
    fn rewrite_output_item_done_without_repeating_tool_name() {
        let (_temp, _home, dir) = isolated_codex_home();
        write_agent(&dir, "grok-4-6", "grok-4.6", "high");
        install_worker(&dir, "grok-4-6");

        let mut added = json!({
            "type": "response.output_item.added",
            "item": {
                "id": "fc_spawn",
                "type": "function_call",
                "name": "spawn_agent",
                "arguments": ""
            }
        });
        let mut state = SpawnRewriteState::new();
        assert!(!rewrite_spawn_agent_in_event(&mut added, &mut state));
        assert!(state.has_pending_spawn_ids());

        let mut done = json!({
            "type": "response.output_item.done",
            "item": {
                "id": "fc_spawn",
                "type": "function_call",
                "arguments": "{\"agent_type\":\"worker\",\"fork_turns\":\"none\"}"
            }
        });
        assert!(rewrite_spawn_agent_in_event(&mut done, &mut state));
        let args: Value =
            serde_json::from_str(done["item"]["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(args["model"], "grok-4.6");
        assert_eq!(args["reasoning_effort"], "high");
    }

    #[test]
    #[serial_test::serial]
    fn rewrite_leaves_unrelated_function_calls_untouched() {
        let (_temp, _home, dir) = isolated_codex_home();
        write_agent(&dir, "grok-4-6", "grok-4.6", "high");
        install_worker(&dir, "grok-4-6");

        let original = json!({
            "type": "function_call",
            "name": "send_message",
            "arguments": "{\"message\":\"continue\"}"
        });
        let mut value = original.clone();
        assert!(!rewrite_spawn_agent_value(&mut value));
        assert_eq!(value, original);
    }
}
