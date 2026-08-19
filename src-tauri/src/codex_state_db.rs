//! Locating Codex's per-thread state SQLite databases.
//!
//! Codex stores thread metadata in `state_5.sqlite`, normally inside the Codex
//! config dir (`CODEX_HOME` / `~/.codex`). The SQLite location can be moved with
//! the `sqlite_home` key in `config.toml` or the `CODEX_SQLITE_HOME` env var;
//! when set, a second DB lives there. Both history migration and the session
//! list's title lookup need the same resolution, so it lives here once.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{params_from_iter, Connection, OptionalExtension};
use toml_edit::DocumentMut;

use crate::config::get_home_dir;
use crate::database::Database;
use crate::provider::Provider;
use crate::proxy::providers::codex_provider_upstream_model;

/// Filename of Codex's per-thread state database. Codex bumps the version
/// number across releases; update this single source of truth when a new state
/// DB version ships.
pub(crate) const CODEX_STATE_DB_FILENAME: &str = "state_5.sqlite";

/// Env var that overrides the Codex SQLite state directory.
const CODEX_SQLITE_HOME_ENV: &str = "CODEX_SQLITE_HOME";

/// Resolve every candidate `state_5.sqlite` path: the config-dir DB plus, when
/// Codex is configured to keep its SQLite state elsewhere, that DB too.
///
/// `config_dir` is the Codex config dir (`~/.codex`); `config_text` is the raw
/// `config.toml` contents, used to detect a `sqlite_home` override.
pub(crate) fn codex_state_db_paths(config_dir: &Path, config_text: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    push_unique_path(&mut paths, config_dir.join(CODEX_STATE_DB_FILENAME));
    // Codex lets SQLite state move away from CODEX_HOME; config takes precedence.
    if let Some(sqlite_home) = sqlite_home_from_codex_config(config_text) {
        push_unique_path(&mut paths, sqlite_home.join(CODEX_STATE_DB_FILENAME));
    } else if let Some(sqlite_home) = sqlite_home_from_env() {
        push_unique_path(&mut paths, sqlite_home.join(CODEX_STATE_DB_FILENAME));
    }
    paths
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.contains(&path) {
        paths.push(path);
    }
}

fn sqlite_home_from_codex_config(config_text: &str) -> Option<PathBuf> {
    let doc = config_text.parse::<DocumentMut>().ok()?;
    let raw = doc.get("sqlite_home")?.as_str()?.trim();
    if raw.is_empty() {
        return None;
    }
    Some(resolve_user_path(raw))
}

fn sqlite_home_from_env() -> Option<PathBuf> {
    let raw = std::env::var(CODEX_SQLITE_HOME_ENV).ok()?;
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    Some(resolve_user_path(raw))
}

fn resolve_user_path(raw: &str) -> PathBuf {
    if raw == "~" {
        return get_home_dir();
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return get_home_dir().join(rest);
    }
    if let Some(rest) = raw.strip_prefix("~\\") {
        return get_home_dir().join(rest);
    }
    PathBuf::from(raw)
}

/// Outcome of syncing stale `custom` thread models after a Codex provider
/// switch. Failures never abort the already-committed switch; they surface here
/// so callers can log a warning instead of reporting a failed switch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CodexThreadModelSyncOutcome {
    /// Number of candidate state DBs that existed and were processed.
    pub db_count: usize,
    /// Total number of stale thread rows rewritten across all DBs.
    pub updated_rows: usize,
    /// Per-DB error messages; each includes the DB path.
    pub errors: Vec<String>,
}

/// Whether a Codex provider switch should rewrite stale `custom` thread models.
/// Only ordinary third-party providers qualify: aggregate providers route per
/// model through the proxy, and official/managed-account (OAuth, Copilot,
/// ChatGPT backend) providers must keep their thread buckets untouched.
pub(crate) fn should_sync_codex_thread_models(provider: &Provider) -> bool {
    !provider.is_aggregate()
        && provider.category.as_deref() != Some("official")
        && !provider.uses_managed_account_auth()
}

/// Rewrite stale `custom` thread models to the provider's upstream model after
/// a successful Codex provider switch or in-place save of the current provider.
///
/// Scope (mirrors the proxy's active-session fallback):
/// - Only threads with `model_provider = 'custom'` are touched; `openai` and
///   other provider buckets are left alone, and JSONL is never modified.
/// - A thread model already equal to the target, or explicitly present in the
///   provider's `modelCatalog`, is preserved as a legitimate selection.
/// - An empty `modelCatalog` with a unique configured upstream model is treated
///   as a single-model provider, so every stale custom thread is rewritten.
///
/// `config_dir` is the Codex config dir; `config_text` is the post-switch
/// `config.toml` contents so a `sqlite_home` override is honored. Missing DBs,
/// missing `threads` table, or missing `model`/`model_provider` columns are
/// skipped for future schema compatibility.
pub(crate) fn sync_stale_custom_thread_models(
    provider: &Provider,
    config_dir: &Path,
    config_text: &str,
) -> CodexThreadModelSyncOutcome {
    let mut outcome = CodexThreadModelSyncOutcome::default();
    if !should_sync_codex_thread_models(provider) {
        return outcome;
    }
    let Some(target_model) = codex_provider_upstream_model(provider) else {
        return outcome;
    };
    let catalog = provider_catalog_model_ids(provider);

    for db_path in codex_state_db_paths(config_dir, config_text) {
        if !db_path.exists() {
            continue;
        }
        outcome.db_count += 1;
        match sync_stale_custom_thread_models_in_db(&db_path, &target_model, &catalog) {
            Ok(updated) => outcome.updated_rows += updated,
            Err(error) => outcome
                .errors
                .push(format!("{}: {error}", db_path.display())),
        }
    }
    outcome
}

/// Same as [`sync_stale_custom_thread_models`], reading the live Codex home and
/// `config.toml` so callers after a live write do not have to thread those paths.
pub(crate) fn sync_stale_custom_thread_models_from_live(
    provider: &Provider,
) -> CodexThreadModelSyncOutcome {
    sync_stale_custom_thread_models(
        provider,
        &crate::codex_config::get_codex_config_dir(),
        &crate::codex_config::read_codex_config_text().unwrap_or_default(),
    )
}

/// Live/import/add paths share this so a successful live write cannot leave
/// `custom` threads pointing at the previous provider's model.
pub(crate) fn sync_and_log_stale_custom_thread_models_from_live(
    provider: &Provider,
    context: &str,
) {
    let outcome = sync_stale_custom_thread_models_from_live(provider);
    if outcome.updated_rows > 0 {
        log::info!("{context}已同步 {} 条 Codex 线程模型", outcome.updated_rows);
    }
    if !outcome.errors.is_empty() {
        log::warn!(
            "{context}同步 Codex 线程模型失败: {}",
            outcome.errors.join("; ")
        );
    }
}

/// Model ids the provider's `modelCatalog` explicitly supports. Empty when the
/// provider carries no catalog, in which case callers fall back to treating
/// the configured upstream model as the only supported selection.
pub(crate) fn provider_catalog_model_ids(provider: &Provider) -> Vec<String> {
    provider
        .settings_config
        .get("modelCatalog")
        .and_then(|catalog| catalog.get("models"))
        .and_then(|models| models.as_array())
        .map(|models| {
            models
                .iter()
                .filter_map(|model| model.get("model").and_then(|value| value.as_str()))
                .map(str::trim)
                .filter(|model| !model.is_empty())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn sync_stale_custom_thread_models_in_db(
    db_path: &Path,
    target_model: &str,
    catalog: &[String],
) -> Result<usize, String> {
    let mut conn = Connection::open(db_path).map_err(|e| format!("打开失败: {e}"))?;
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(|e| format!("设置 busy_timeout 失败: {e}"))?;

    if !Database::table_exists(&conn, "threads").map_err(|e| format!("检查 threads 表失败: {e}"))?
        || !Database::has_column(&conn, "threads", "model_provider")
            .map_err(|e| format!("检查 model_provider 列失败: {e}"))?
        || !Database::has_column(&conn, "threads", "model")
            .map_err(|e| format!("检查 model 列失败: {e}"))?
    {
        return Ok(0);
    }

    let mut sql = String::from(
        "UPDATE threads SET model = ?1 \
         WHERE model_provider = 'custom' \
         AND model IS NOT NULL AND model <> ?1",
    );
    let mut params: Vec<&dyn rusqlite::ToSql> = vec![&target_model];
    if !catalog.is_empty() {
        let placeholders: Vec<String> = (2..=catalog.len() + 1).map(|i| format!("?{i}")).collect();
        sql.push_str(&format!(" AND model NOT IN ({})", placeholders.join(", ")));
        params.extend(catalog.iter().map(|model| model as &dyn rusqlite::ToSql));
    }

    let tx = conn
        .transaction()
        .map_err(|e| format!("开启事务失败: {e}"))?;
    let updated = tx
        .execute(&sql, params_from_iter(params))
        .map_err(|e| format!("更新线程模型失败: {e}"))?;
    tx.commit().map_err(|e| format!("提交事务失败: {e}"))?;
    Ok(updated)
}

/// Read the current `custom` thread model from Codex state DBs.
///
/// Forked Desktop turns often keep the parent `turn_context.model` in the
/// `/responses` body after the user picks a different model in the UI. The UI
/// selection is stored here; JSONL is never modified.
pub(crate) fn read_custom_thread_model(thread_id: &str) -> Option<String> {
    let thread_id = normalize_codex_thread_id(thread_id)?;
    read_custom_thread_model_from(
        &thread_id,
        &crate::codex_config::get_codex_config_dir(),
        &crate::codex_config::read_codex_config_text().unwrap_or_default(),
    )
}

pub(crate) fn normalize_codex_thread_id(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let stripped = trimmed
        .strip_prefix("codex_")
        .or_else(|| trimmed.strip_prefix("codex-"))
        .unwrap_or(trimmed)
        .trim();
    if stripped.is_empty() {
        None
    } else {
        Some(stripped.to_string())
    }
}

fn read_custom_thread_model_from(
    thread_id: &str,
    config_dir: &Path,
    config_text: &str,
) -> Option<String> {
    let thread_id = normalize_codex_thread_id(thread_id)?;
    for db_path in codex_state_db_paths(config_dir, config_text) {
        if !db_path.exists() {
            continue;
        }
        match read_custom_thread_model_in_db(&db_path, &thread_id) {
            Ok(Some(model)) => return Some(model),
            Ok(None) => {}
            Err(error) => log::debug!("读取 Codex 线程模型失败 {}: {error}", db_path.display()),
        }
    }
    None
}

fn read_custom_thread_model_in_db(
    db_path: &Path,
    thread_id: &str,
) -> Result<Option<String>, String> {
    let conn = Connection::open(db_path).map_err(|e| format!("打开失败: {e}"))?;
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(|e| format!("设置 busy_timeout 失败: {e}"))?;
    if !Database::table_exists(&conn, "threads").map_err(|e| format!("检查 threads 表失败: {e}"))?
        || !Database::has_column(&conn, "threads", "model_provider")
            .map_err(|e| format!("检查 model_provider 列失败: {e}"))?
        || !Database::has_column(&conn, "threads", "model")
            .map_err(|e| format!("检查 model 列失败: {e}"))?
    {
        return Ok(None);
    }
    let model: Option<String> = conn
        .query_row(
            "SELECT model FROM threads \
             WHERE id = ?1 AND model_provider = 'custom' \
             AND model IS NOT NULL AND trim(model) <> ''",
            [thread_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("查询线程模型失败: {e}"))?;
    Ok(model
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ProviderMeta;
    use rusqlite::params;
    use serde_json::json;
    use tempfile::tempdir;

    fn ordinary_provider(model: &str, catalog: &[&str]) -> Provider {
        let catalog_models: Vec<serde_json::Value> = catalog
            .iter()
            .map(|model| json!({ "model": model }))
            .collect();
        Provider::with_id(
            "deepseek".to_string(),
            "DeepSeek".to_string(),
            json!({
                "model": model,
                "modelCatalog": { "models": catalog_models },
            }),
            None,
        )
    }

    fn provider_with_meta(model: &str, provider_type: &str, category: Option<&str>) -> Provider {
        let mut provider = ordinary_provider(model, &["deepseek-v4-flash"]);
        provider.meta = Some(ProviderMeta {
            provider_type: Some(provider_type.to_string()),
            ..Default::default()
        });
        provider.category = category.map(ToString::to_string);
        provider
    }

    fn create_threads_db(path: &Path, rows: &[(&str, Option<&str>)]) {
        let conn = Connection::open(path).expect("open db");
        conn.execute_batch(
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                model_provider TEXT NOT NULL,
                model TEXT
            );",
        )
        .expect("create threads table");
        for (index, (model_provider, model)) in rows.iter().enumerate() {
            conn.execute(
                "INSERT INTO threads (id, model_provider, model) VALUES (?1, ?2, ?3)",
                params![format!("t{index}"), model_provider, model],
            )
            .expect("insert thread");
        }
    }

    fn thread_models(path: &Path) -> Vec<(String, Option<String>)> {
        let conn = Connection::open(path).expect("open db");
        let mut stmt = conn
            .prepare("SELECT model_provider, model FROM threads ORDER BY id")
            .expect("prepare query");
        stmt.query_map([], |row| {
            let provider: String = row.get(0)?;
            let model: Option<String> = row.get(1)?;
            Ok((provider, model))
        })
        .expect("query rows")
        .map(|row| row.expect("read row"))
        .collect()
    }

    #[test]
    fn stale_custom_updated_supported_and_target_preserved() {
        let temp = tempdir().expect("tempdir");
        let db_path = temp.path().join(CODEX_STATE_DB_FILENAME);
        create_threads_db(
            &db_path,
            &[
                ("custom", Some("gpt-5.6-luna")),      // stale -> rewritten
                ("custom", Some("deepseek-v4-flash")), // target -> preserved
                ("custom", Some("deepseek-v4-pro")),   // catalog -> preserved
                ("custom", None),                      // NULL -> untouched
                ("openai", Some("gpt-5.6-luna")),      // official bucket -> untouched
                ("opencode", Some("gpt-5.6-luna")),    // other bucket -> untouched
            ],
        );

        let provider = ordinary_provider(
            "deepseek-v4-flash",
            &["deepseek-v4-flash", "deepseek-v4-pro"],
        );
        let outcome = sync_stale_custom_thread_models(&provider, temp.path(), "");

        assert_eq!(outcome.db_count, 1);
        assert_eq!(outcome.updated_rows, 1);
        assert!(outcome.errors.is_empty());
        let rows = thread_models(&db_path);
        assert_eq!(
            rows[0],
            ("custom".to_string(), Some("deepseek-v4-flash".to_string()))
        );
        assert_eq!(
            rows[1],
            ("custom".to_string(), Some("deepseek-v4-flash".to_string()))
        );
        assert_eq!(
            rows[2],
            ("custom".to_string(), Some("deepseek-v4-pro".to_string()))
        );
        assert_eq!(rows[3], ("custom".to_string(), None));
        assert_eq!(
            rows[4],
            ("openai".to_string(), Some("gpt-5.6-luna".to_string()))
        );
        assert_eq!(
            rows[5],
            ("opencode".to_string(), Some("gpt-5.6-luna".to_string()))
        );
    }

    #[test]
    fn empty_catalog_treats_provider_as_single_model() {
        let temp = tempdir().expect("tempdir");
        let db_path = temp.path().join(CODEX_STATE_DB_FILENAME);
        create_threads_db(
            &db_path,
            &[
                ("custom", Some("gpt-5.6-luna")),
                ("custom", Some("deepseek-v4-flash")),
                ("openai", Some("gpt-5.6-luna")),
            ],
        );

        let provider = ordinary_provider("deepseek-v4-flash", &[]);
        let outcome = sync_stale_custom_thread_models(&provider, temp.path(), "");

        assert_eq!(outcome.updated_rows, 1);
        let rows = thread_models(&db_path);
        assert_eq!(
            rows[0],
            ("custom".to_string(), Some("deepseek-v4-flash".to_string()))
        );
        assert_eq!(
            rows[1],
            ("custom".to_string(), Some("deepseek-v4-flash".to_string()))
        );
        assert_eq!(
            rows[2],
            ("openai".to_string(), Some("gpt-5.6-luna".to_string()))
        );
    }

    #[test]
    fn missing_db_is_safe() {
        let temp = tempdir().expect("tempdir");
        let provider = ordinary_provider("deepseek-v4-flash", &["deepseek-v4-flash"]);
        let outcome = sync_stale_custom_thread_models(&provider, temp.path(), "");

        assert_eq!(outcome.db_count, 0);
        assert_eq!(outcome.updated_rows, 0);
        assert!(outcome.errors.is_empty());
    }

    #[test]
    fn missing_threads_table_or_columns_is_safe() {
        let temp = tempdir().expect("tempdir");
        let db_path = temp.path().join(CODEX_STATE_DB_FILENAME);

        // No threads table at all.
        let conn = Connection::open(&db_path).expect("open db");
        conn.execute_batch("CREATE TABLE other (id TEXT PRIMARY KEY);")
            .expect("create other table");
        drop(conn);
        let provider = ordinary_provider("deepseek-v4-flash", &["deepseek-v4-flash"]);
        let outcome = sync_stale_custom_thread_models(&provider, temp.path(), "");
        assert_eq!(outcome.db_count, 1);
        assert_eq!(outcome.updated_rows, 0);
        assert!(outcome.errors.is_empty());

        // threads table missing the model column (future schema drift).
        let conn = Connection::open(&db_path).expect("open db");
        conn.execute_batch(
            "DROP TABLE other; CREATE TABLE threads (id TEXT PRIMARY KEY, model_provider TEXT NOT NULL);",
        )
        .expect("recreate threads");
        drop(conn);
        let outcome = sync_stale_custom_thread_models(&provider, temp.path(), "");
        assert_eq!(outcome.db_count, 1);
        assert_eq!(outcome.updated_rows, 0);
        assert!(outcome.errors.is_empty());
    }

    #[test]
    fn sqlite_home_dual_path_updates_both_dbs() {
        let temp = tempdir().expect("tempdir");
        let sqlite_home = temp.path().join("sqlite-home");
        std::fs::create_dir_all(&sqlite_home).expect("create sqlite_home");

        let primary = temp.path().join(CODEX_STATE_DB_FILENAME);
        let secondary = sqlite_home.join(CODEX_STATE_DB_FILENAME);
        create_threads_db(&primary, &[("custom", Some("gpt-5.6-luna"))]);
        create_threads_db(&secondary, &[("custom", Some("gpt-5.6-luna"))]);

        let config_text = format!("sqlite_home = '{}'\n", sqlite_home.display());
        let provider = ordinary_provider("deepseek-v4-flash", &["deepseek-v4-flash"]);
        let outcome = sync_stale_custom_thread_models(&provider, temp.path(), &config_text);

        assert_eq!(outcome.db_count, 2);
        assert_eq!(outcome.updated_rows, 2);
        assert!(outcome.errors.is_empty());
        assert_eq!(
            thread_models(&primary)[0],
            ("custom".to_string(), Some("deepseek-v4-flash".to_string()))
        );
        assert_eq!(
            thread_models(&secondary)[0],
            ("custom".to_string(), Some("deepseek-v4-flash".to_string()))
        );
    }

    #[test]
    fn skips_aggregate_official_and_oauth_providers() {
        assert!(should_sync_codex_thread_models(&ordinary_provider(
            "deepseek-v4-flash",
            &["deepseek-v4-flash"]
        )));

        let aggregate = provider_with_meta("deepseek-v4-flash", "aggregate", None);
        assert!(!should_sync_codex_thread_models(&aggregate));

        let official = provider_with_meta("deepseek-v4-flash", "codex", Some("official"));
        assert!(!should_sync_codex_thread_models(&official));

        let codex_oauth = provider_with_meta("deepseek-v4-flash", "codex_oauth", None);
        assert!(!should_sync_codex_thread_models(&codex_oauth));

        let xai_oauth = provider_with_meta("deepseek-v4-flash", "xai_oauth", None);
        assert!(!should_sync_codex_thread_models(&xai_oauth));

        // sync entry point also returns an empty outcome for skipped providers.
        let temp = tempdir().expect("tempdir");
        let db_path = temp.path().join(CODEX_STATE_DB_FILENAME);
        create_threads_db(&db_path, &[("custom", Some("gpt-5.6-luna"))]);
        let outcome = sync_stale_custom_thread_models(&aggregate, temp.path(), "");
        assert_eq!(outcome, CodexThreadModelSyncOutcome::default());
    }

    #[test]
    fn read_custom_thread_model_returns_selected_model() {
        let temp = tempdir().expect("tempdir");
        let db_path = temp.path().join(CODEX_STATE_DB_FILENAME);
        let conn = Connection::open(&db_path).expect("open db");
        conn.execute_batch(
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                model_provider TEXT NOT NULL,
                model TEXT
            );",
        )
        .expect("create threads table");
        conn.execute(
            "INSERT INTO threads (id, model_provider, model) VALUES (?1, ?2, ?3)",
            params!["01a017fe-7519-7ce2-9c7e-872de1c2394c", "custom", "kimi-k3"],
        )
        .expect("insert custom thread");
        conn.execute(
            "INSERT INTO threads (id, model_provider, model) VALUES (?1, ?2, ?3)",
            params!["openai-thread", "openai", "gpt-5.6-sol"],
        )
        .expect("insert openai thread");
        drop(conn);

        assert_eq!(
            read_custom_thread_model_from("01a017fe-7519-7ce2-9c7e-872de1c2394c", temp.path(), "")
                .as_deref(),
            Some("kimi-k3")
        );
        assert_eq!(
            read_custom_thread_model_from(
                "codex_01a017fe-7519-7ce2-9c7e-872de1c2394c",
                temp.path(),
                ""
            )
            .as_deref(),
            Some("kimi-k3")
        );
        assert_eq!(
            read_custom_thread_model_from("openai-thread", temp.path(), ""),
            None
        );
    }

    #[test]
    fn missing_upstream_model_skips_sync() {
        let temp = tempdir().expect("tempdir");
        let db_path = temp.path().join(CODEX_STATE_DB_FILENAME);
        create_threads_db(&db_path, &[("custom", Some("gpt-5.6-luna"))]);

        let mut provider = ordinary_provider("deepseek-v4-flash", &[]);
        provider.settings_config = json!({ "modelCatalog": { "models": [] } });
        let outcome = sync_stale_custom_thread_models(&provider, temp.path(), "");

        assert_eq!(outcome, CodexThreadModelSyncOutcome::default());
        assert_eq!(
            thread_models(&db_path)[0],
            ("custom".to_string(), Some("gpt-5.6-luna".to_string()))
        );
    }

    #[test]
    fn includes_config_sqlite_home() {
        let temp = tempdir().expect("tempdir");
        let sqlite_home = temp.path().join("sqlite-home");
        // 用 TOML 字面量字符串(单引号)承载路径：Windows 路径含反斜杠，basic string(双引号)
        // 会把 `\U`/`\s` 等当作非法转义导致解析失败。
        let config_text = format!("sqlite_home = '{}'\n", sqlite_home.display());

        let paths = codex_state_db_paths(temp.path(), &config_text);

        assert_eq!(
            paths,
            vec![
                temp.path().join(CODEX_STATE_DB_FILENAME),
                sqlite_home.join(CODEX_STATE_DB_FILENAME),
            ]
        );
    }
}
