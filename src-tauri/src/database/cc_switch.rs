//! 从 cc-switch 的 SQLite 导出文件导入供应商。
//!
//! 合并语义（与 Codex Cube 自身整库替换式导入不同）：只复制 codex 相关行——
//! 供应商及其自定义端点、MCP 服务器、codex 提示词、模型定价；官方种子、
//! 其他应用的行、profiles/settings/skills、代理/用量/日志表一律跳过。
//! 同 id 已存在则覆盖，并先做整库备份。

use rusqlite::Connection;
use std::path::Path;
use tempfile::NamedTempFile;

use super::backup::import_authorizer;
use super::{is_official_seed_id, lock_conn, Database};
use crate::app_config::{McpApps, McpServer, Prompt};
use crate::error::AppError;
use crate::provider::{Provider, ProviderMeta};

/// cc-switch 导出文件的文件头（`-- CC Switch SQLite 导出`）。
pub const CC_SWITCH_SQL_EXPORT_HEADER: &str = "-- CC Switch SQLite 导出";

pub(crate) fn is_cc_switch_export(sql: &str) -> bool {
    sql.trim_start().starts_with(CC_SWITCH_SQL_EXPORT_HEADER)
}

/// cc-switch 导入结果统计。
#[derive(Debug, Default)]
pub struct CcSwitchImportReport {
    pub imported: usize,
    pub updated: usize,
    pub skipped_official: usize,
    pub skipped_invalid: usize,
    pub endpoints_added: usize,
    pub current_provider_name: Option<String>,
    pub mcp_merged: usize,
    pub prompts_merged: usize,
    pub pricing_merged: usize,
}

impl CcSwitchImportReport {
    /// 给前端 toast / 日志用的中文一句话总结。
    pub fn summary(&self) -> String {
        let mut parts = vec![format!(
            "cc-switch 导入完成：新增 {} 个，覆盖 {} 个",
            self.imported, self.updated
        )];
        if self.skipped_official > 0 {
            parts.push(format!("跳过官方种子 {} 个", self.skipped_official));
        }
        if self.skipped_invalid > 0 {
            parts.push(format!("跳过损坏 {} 个", self.skipped_invalid));
        }
        if self.endpoints_added > 0 {
            parts.push(format!("补端点 {} 个", self.endpoints_added));
        }
        if self.mcp_merged > 0 {
            parts.push(format!("MCP {} 个", self.mcp_merged));
        }
        if self.prompts_merged > 0 {
            parts.push(format!("提示词 {} 条", self.prompts_merged));
        }
        if self.pricing_merged > 0 {
            parts.push(format!("定价 {} 条", self.pricing_merged));
        }
        if let Some(name) = self.current_provider_name.as_deref() {
            parts.push(format!("已将「{name}」设为当前供应商"));
        }
        parts.join("；")
    }
}

impl Database {
    /// 从 cc-switch 导出文件导入，返回 `(备份 ID, 统计)`。
    pub fn import_cc_switch_sql_file(
        &self,
        source_path: &Path,
    ) -> Result<(String, CcSwitchImportReport), AppError> {
        if !source_path.exists() {
            return Err(AppError::InvalidInput(format!(
                "SQL 文件不存在: {}",
                source_path.display()
            )));
        }
        let sql_raw =
            std::fs::read_to_string(source_path).map_err(|e| AppError::io(source_path, e))?;
        self.import_cc_switch_sql_string(sql_raw.trim_start_matches('\u{feff}'))
    }

    /// 从 cc-switch 导出 SQL 字符串导入，返回 `(备份 ID, 统计)`。
    pub fn import_cc_switch_sql_string(
        &self,
        sql_raw: &str,
    ) -> Result<(String, CcSwitchImportReport), AppError> {
        let sql_content = sql_raw.trim_start_matches('\u{feff}');
        if !is_cc_switch_export(sql_content) {
            return Err(AppError::InvalidInput(
                "不是有效的 cc-switch 导出文件（缺少 `-- CC Switch SQLite 导出` 文件头）".to_string(),
            ));
        }

        // 先备份现有数据库，合并写坏可回滚。
        let backup_id = self
            .backup_database_file()?
            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
            .unwrap_or_default();

        // 在临时库执行外部 SQL，失败不污染主库（与整库导入同样的设防）。
        let temp_file = NamedTempFile::new().map_err(|e| AppError::IoContext {
            context: "创建临时数据库文件失败".to_string(),
            source: e,
        })?;
        let temp_conn =
            Connection::open(temp_file.path()).map_err(|e| AppError::Database(e.to_string()))?;
        temp_conn.authorizer(Some(import_authorizer));
        let batch_result = temp_conn.execute_batch(sql_content);
        temp_conn.authorizer(
            None::<fn(rusqlite::hooks::AuthContext<'_>) -> rusqlite::hooks::Authorization>,
        );
        batch_result.map_err(|e| AppError::Database(format!("执行 SQL 导入失败: {e}")))?;
        Database::ensure_complete_transaction(&temp_conn)?;

        let mut report = self.merge_codex_providers_from(&temp_conn)?;
        report.mcp_merged = self.merge_mcp_servers_from(&temp_conn)?;
        report.prompts_merged = self.merge_codex_prompts_from(&temp_conn)?;
        report.pricing_merged = self.merge_model_pricing_from(&temp_conn)?;
        log::info!("[Import] {}", report.summary());
        Ok((backup_id, report))
    }

    /// 从已载入的 cc-switch 临时库合并 codex 供应商到主库。
    fn merge_codex_providers_from(
        &self,
        temp_conn: &Connection,
    ) -> Result<CcSwitchImportReport, AppError> {
        let mut report = CcSwitchImportReport::default();

        let mut stmt = temp_conn
            .prepare(
                "SELECT id, name, settings_config, website_url, category,
                        created_at, sort_index, notes, icon, icon_color,
                        meta, is_current, in_failover_queue
                 FROM providers WHERE app_type = 'codex'",
            )
            .map_err(|_| {
                AppError::InvalidInput(
                    "不是有效的 cc-switch 导出文件（缺少 providers 表）".to_string(),
                )
            })?;
        let rows: Vec<(
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            Option<i64>,
            Option<i64>,
            Option<String>,
            Option<String>,
            Option<String>,
            String,
            i32,
            i32,
        )> = stmt
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                    row.get(10)?,
                    row.get(11)?,
                    row.get(12)?,
                ))
            })
            .map_err(|e| AppError::Database(e.to_string()))?
            .collect::<Result<_, _>>()
            .map_err(|e| AppError::Database(e.to_string()))?;
        drop(stmt);

        // 端点随供应商一起复制。
        let mut endpoint_stmt = temp_conn
            .prepare(
                "SELECT url, added_at FROM provider_endpoints
                 WHERE provider_id = ?1 AND app_type = 'codex'",
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut source_current: Option<(String, String)> = None;
        for (
            id,
            name,
            settings_config_text,
            website_url,
            category,
            created_at,
            sort_index,
            notes,
            icon,
            icon_color,
            meta_text,
            is_current,
            in_failover_queue,
        ) in rows
        {
            // 官方种子两边各自维护，互不覆盖。
            if is_official_seed_id(id.trim()) {
                report.skipped_official += 1;
                continue;
            }
            let settings_config: serde_json::Value =
                match serde_json::from_str(&settings_config_text) {
                    Ok(value) => value,
                    Err(_) => {
                        log::warn!("[Import] 供应商「{name}」的配置不是有效 JSON，已跳过");
                        report.skipped_invalid += 1;
                        continue;
                    }
                };
            // meta 解析失败不致命：端点另有 provider_endpoints 表兜底。
            let meta: Option<ProviderMeta> = serde_json::from_str(&meta_text).ok();
            let existed = self
                .get_provider_by_id(&id, "codex")
                .map_err(|e| AppError::Database(e.to_string()))?
                .is_some();
            let provider = Provider {
                id: id.clone(),
                name: name.clone(),
                settings_config,
                website_url,
                category,
                created_at,
                sort_index: sort_index.and_then(|v| usize::try_from(v).ok()),
                notes,
                meta,
                icon,
                icon_color,
                in_failover_queue: in_failover_queue != 0,
            };
            if let Err(error) = self.save_provider("codex", &provider) {
                log::warn!("[Import] 供应商「{name}」写入失败，已跳过: {error}");
                report.skipped_invalid += 1;
                continue;
            }
            if existed {
                report.updated += 1;
            } else {
                report.imported += 1;
            }

            let endpoint_rows: Vec<(String, Option<i64>)> = endpoint_stmt
                .query_map([id.as_str()], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(|e| AppError::Database(e.to_string()))?
                .collect::<Result<_, _>>()
                .map_err(|e| AppError::Database(e.to_string()))?;
            for (url, added_at) in endpoint_rows {
                if self.provider_endpoint_exists("codex", &id, &url)? {
                    continue;
                }
                let conn = lock_conn!(self.conn);
                conn.execute(
                    "INSERT INTO provider_endpoints (provider_id, app_type, url, added_at)
                     VALUES (?1, 'codex', ?2, ?3)",
                    rusqlite::params![id, url, added_at],
                )
                .map_err(|e| AppError::Database(e.to_string()))?;
                report.endpoints_added += 1;
            }

            if is_current != 0 {
                source_current = Some((id, name));
            }
        }

        // 还原导出时的当前供应商（非官方）：只改徽标，不碰 live。
        if let Some((id, name)) = source_current {
            self.set_current_provider("codex", &id)
                .map_err(|e| AppError::Database(e.to_string()))?;
            report.current_provider_name = Some(name);
        }
        Ok(report)
    }

    /// 合并 MCP 服务器（取 codex 启用位；Cube 是 codex 单应用视图）。
    fn merge_mcp_servers_from(&self, temp_conn: &Connection) -> Result<usize, AppError> {
        let mut stmt = match temp_conn.prepare(
            "SELECT id, name, server_config, description, homepage, docs, tags, enabled_codex
             FROM mcp_servers",
        ) {
            Ok(stmt) => stmt,
            // 旧导出可能没有该表：跳过而非报错。
            Err(_) => return Ok(0),
        };
        let rows: Vec<(
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            i32,
        )> = stmt
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            })
            .map_err(|e| AppError::Database(e.to_string()))?
            .collect::<Result<_, _>>()
            .map_err(|e| AppError::Database(e.to_string()))?;
        drop(stmt);

        let mut merged = 0;
        for (id, name, server_config_text, description, homepage, docs, tags_text, enabled_codex) in
            rows
        {
            let server: serde_json::Value = match serde_json::from_str(&server_config_text) {
                Ok(value) => value,
                Err(_) => {
                    log::warn!("[Import] MCP「{name}」配置不是有效 JSON，已跳过");
                    continue;
                }
            };
            let tags: Vec<String> = tags_text
                .as_deref()
                .and_then(|text| serde_json::from_str(text).ok())
                .unwrap_or_default();
            let server_record = McpServer {
                id: id.clone(),
                name,
                server,
                apps: McpApps { codex: enabled_codex != 0 },
                description,
                homepage,
                docs,
                tags,
            };
            self.save_mcp_server(&server_record)
                .map_err(|e| AppError::Database(e.to_string()))?;
            merged += 1;
        }
        Ok(merged)
    }

    /// 合并 codex 提示词。
    fn merge_codex_prompts_from(&self, temp_conn: &Connection) -> Result<usize, AppError> {
        let mut stmt = match temp_conn.prepare(
            "SELECT id, name, content, description, enabled, created_at, updated_at
             FROM prompts WHERE app_type = 'codex'",
        ) {
            Ok(stmt) => stmt,
            Err(_) => return Ok(0),
        };
        let rows: Vec<(
            String,
            String,
            String,
            Option<String>,
            i32,
            Option<i64>,
            Option<i64>,
        )> = stmt
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            })
            .map_err(|e| AppError::Database(e.to_string()))?
            .collect::<Result<_, _>>()
            .map_err(|e| AppError::Database(e.to_string()))?;
        drop(stmt);

        let mut merged = 0;
        for (id, name, content, description, enabled, created_at, updated_at) in rows {
            let prompt = Prompt {
                id,
                name,
                content,
                description,
                enabled: enabled != 0,
                created_at,
                updated_at,
            };
            self.save_prompt("codex", &prompt)
                .map_err(|e| AppError::Database(e.to_string()))?;
            merged += 1;
        }
        Ok(merged)
    }

    /// 合并模型定价（与导出端列定义一致，直接覆盖）。
    fn merge_model_pricing_from(&self, temp_conn: &Connection) -> Result<usize, AppError> {
        let mut stmt = match temp_conn.prepare(
            "SELECT model_id, display_name, input_cost_per_million, output_cost_per_million,
                    cache_read_cost_per_million, cache_creation_cost_per_million
             FROM model_pricing",
        ) {
            Ok(stmt) => stmt,
            Err(_) => return Ok(0),
        };
        let rows: Vec<(String, String, String, String, String, String)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            })
            .map_err(|e| AppError::Database(e.to_string()))?
            .collect::<Result<_, _>>()
            .map_err(|e| AppError::Database(e.to_string()))?;
        drop(stmt);

        let conn = lock_conn!(self.conn);
        let mut merged = 0;
        for (model_id, display_name, input, output, cache_read, cache_creation) in rows {
            conn.execute(
                "INSERT OR REPLACE INTO model_pricing (
                    model_id, display_name, input_cost_per_million, output_cost_per_million,
                    cache_read_cost_per_million, cache_creation_cost_per_million
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    model_id,
                    display_name,
                    input,
                    output,
                    cache_read,
                    cache_creation
                ],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
            merged += 1;
        }
        Ok(merged)
    }

    fn provider_endpoint_exists(
        &self,
        app_type: &str,
        provider_id: &str,
        url: &str,
    ) -> Result<bool, AppError> {
        let conn = lock_conn!(self.conn);
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM provider_endpoints
                 WHERE provider_id = ?1 AND app_type = ?2 AND url = ?3",
                rusqlite::params![provider_id, app_type, url],
                |row| row.get(0),
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(count > 0)
    }
}

#[cfg(test)]
mod tests {
    use crate::database::Database;

    const FIXTURE: &str = r#"-- CC Switch SQLite 导出
-- 生成时间: 2026-09-18 10:35:10
PRAGMA foreign_keys=OFF;
BEGIN TRANSACTION;
CREATE TABLE providers (
    id TEXT NOT NULL, app_type TEXT NOT NULL, name TEXT NOT NULL,
    settings_config TEXT NOT NULL, website_url TEXT, category TEXT,
    created_at INTEGER, sort_index INTEGER, notes TEXT, icon TEXT,
    icon_color TEXT, meta TEXT NOT NULL DEFAULT '{}',
    is_current BOOLEAN NOT NULL DEFAULT 0,
    in_failover_queue BOOLEAN NOT NULL DEFAULT 0,
    PRIMARY KEY (id, app_type));
CREATE TABLE provider_endpoints (
    id INTEGER PRIMARY KEY AUTOINCREMENT, provider_id TEXT NOT NULL,
    app_type TEXT NOT NULL, url TEXT NOT NULL, added_at INTEGER);
INSERT INTO providers VALUES ('codex-official','codex','OpenAI Official','{"auth":{},"config":""}','https://chatgpt.com/codex','official',1788621944531,0,NULL,'openai','#00A67E','{}',0,0);
INSERT INTO providers VALUES ('relay-a','codex','Relay A','{"auth":{"OPENAI_API_KEY":"sk-test"},"config":"model_provider = \"custom\""}','https://a.example',NULL,NULL,NULL,NULL,NULL,NULL,'{}',1,0);
INSERT INTO providers VALUES ('relay-b','codex','Relay B','not-json','https://b.example',NULL,NULL,NULL,NULL,NULL,NULL,'{}',0,1);
INSERT INTO providers VALUES ('other-app','claude','Claude X','{"env":{}}',NULL,NULL,NULL,NULL,NULL,NULL,NULL,'{}',1,0);
INSERT INTO provider_endpoints (provider_id, app_type, url, added_at) VALUES ('relay-a','codex','https://a.example/v1',1788621944531);
CREATE TABLE mcp_servers (
    id TEXT PRIMARY KEY, name TEXT NOT NULL, server_config TEXT NOT NULL,
    description TEXT, homepage TEXT, docs TEXT, tags TEXT NOT NULL DEFAULT '[]',
    enabled_codex BOOLEAN NOT NULL DEFAULT 0);
CREATE TABLE prompts (
    id TEXT NOT NULL, app_type TEXT NOT NULL, name TEXT NOT NULL, content TEXT NOT NULL,
    description TEXT, enabled BOOLEAN NOT NULL DEFAULT 1, created_at INTEGER, updated_at INTEGER,
    PRIMARY KEY (id, app_type));
CREATE TABLE model_pricing (
    model_id TEXT PRIMARY KEY, display_name TEXT NOT NULL,
    input_cost_per_million TEXT NOT NULL, output_cost_per_million TEXT NOT NULL,
    cache_read_cost_per_million TEXT NOT NULL DEFAULT '0',
    cache_creation_cost_per_million TEXT NOT NULL DEFAULT '0');
INSERT INTO mcp_servers VALUES ('mcp-a','MCP A','{"command":"echo"}',NULL,NULL,NULL,'[]',1);
INSERT INTO prompts VALUES ('p1','codex','P1','hello',NULL,1,NULL,NULL);
INSERT INTO prompts VALUES ('p9','claude','P9','hi',NULL,1,NULL,NULL);
INSERT INTO model_pricing VALUES ('m1','M1','1','2','0','0');
COMMIT;"#;

    #[test]
    fn rejects_non_cc_switch_header() {
        let db = Database::memory().unwrap();
        let err = db
            .import_cc_switch_sql_string("-- Codex-Cube SQLite 导出\nSELECT 1;")
            .unwrap_err()
            .to_string();
        assert!(err.contains("cc-switch"), "unexpected: {err}");
    }

    #[test]
    fn rejects_missing_providers_table() {
        let db = Database::memory().unwrap();
        let err = db
            .import_cc_switch_sql_string("-- CC Switch SQLite 导出\nSELECT 1;")
            .unwrap_err()
            .to_string();
        assert!(err.contains("providers"), "unexpected: {err}");
    }

    #[test]
    fn merges_codex_providers_only() {
        let db = Database::memory().unwrap();
        let (_backup, report) = db.import_cc_switch_sql_string(FIXTURE).unwrap();
        assert_eq!(report.imported, 1);
        assert_eq!(report.updated, 0);
        assert_eq!(report.skipped_official, 1);
        assert_eq!(report.skipped_invalid, 1);
        assert_eq!(report.endpoints_added, 1);
        assert_eq!(report.current_provider_name.as_deref(), Some("Relay A"));
        assert_eq!(report.mcp_merged, 1);
        assert_eq!(report.prompts_merged, 1);
        assert_eq!(report.pricing_merged, 1);

        // claude 行未导入；当前供应商已还原。
        assert!(db.get_provider_by_id("other-app", "claude").unwrap().is_none());
        assert!(db.get_provider_by_id("relay-a", "codex").unwrap().is_some());
        assert_eq!(
            db.get_current_provider("codex").unwrap().as_deref(),
            Some("relay-a")
        );
        assert!(report.summary().contains("新增 1 个"));
    }

    #[test]
    fn reimport_updates_and_dedups_endpoints() {
        let db = Database::memory().unwrap();
        db.import_cc_switch_sql_string(FIXTURE).unwrap();
        let (_backup, report) = db.import_cc_switch_sql_string(FIXTURE).unwrap();
        assert_eq!(report.imported, 0);
        assert_eq!(report.updated, 1);
        assert_eq!(report.endpoints_added, 0);
    }

}
