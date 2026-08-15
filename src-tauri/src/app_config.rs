use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;

use crate::services::skill::SkillStore;

/// MCP 服务器应用状态（标记应用到哪些客户端）
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct McpApps {
    #[serde(default)]
    pub codex: bool,
}

impl McpApps {
    /// 检查指定应用是否启用
    pub fn is_enabled_for(&self, app: &AppType) -> bool {
        match app {
            AppType::Codex => self.codex,
        }
    }

    /// 设置指定应用的启用状态
    pub fn set_enabled_for(&mut self, app: &AppType, enabled: bool) {
        match app {
            AppType::Codex => self.codex = enabled,
        }
    }

    /// 获取所有启用的应用列表
    pub fn enabled_apps(&self) -> Vec<AppType> {
        let mut apps = Vec::new();
        if self.codex {
            apps.push(AppType::Codex);
        }
        apps
    }

    /// 检查是否所有应用都未启用
    pub fn is_empty(&self) -> bool {
        !self.codex
    }
}

/// Skill 应用启用状态（标记 Skill 应用到哪些客户端）
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct SkillApps {
    #[serde(default)]
    pub codex: bool,
}

impl SkillApps {
    /// 检查指定应用是否启用
    pub fn is_enabled_for(&self, app: &AppType) -> bool {
        match app {
            AppType::Codex => self.codex,
        }
    }

    /// 设置指定应用的启用状态
    pub fn set_enabled_for(&mut self, app: &AppType, enabled: bool) {
        match app {
            AppType::Codex => self.codex = enabled,
        }
    }

    /// 获取所有启用的应用列表
    pub fn enabled_apps(&self) -> Vec<AppType> {
        let mut apps = Vec::new();
        if self.codex {
            apps.push(AppType::Codex);
        }
        apps
    }

    /// 检查是否所有应用都未启用
    pub fn is_empty(&self) -> bool {
        !self.codex
    }

    /// 仅启用指定应用（其他应用设为禁用）
    pub fn only(app: &AppType) -> Self {
        let mut apps = Self::default();
        apps.set_enabled_for(app, true);
        apps
    }

    /// 从来源标签列表构建启用状态
    ///
    /// 标签与 AppType::as_str() 一致时启用对应应用，
    /// 其他标签（如 "agents", "codex-cube"）忽略。
    pub fn from_labels(labels: &[String]) -> Self {
        let mut apps = Self::default();
        for label in labels {
            if let Ok(app) = label.parse::<AppType>() {
                apps.set_enabled_for(&app, true);
            }
        }
        apps
    }
}

/// 已安装的 Skill（v3.10.0+ 统一结构）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledSkill {
    /// 唯一标识符（格式："owner/repo:directory" 或 "local:directory"）
    pub id: String,
    /// 显示名称
    pub name: String,
    /// 描述
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// 安装目录名（在 SSOT 目录中的子目录名）
    pub directory: String,
    /// 仓库所有者（GitHub 用户/组织）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_owner: Option<String>,
    /// 仓库名称
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_name: Option<String>,
    /// 仓库分支
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_branch: Option<String>,
    /// README URL
    #[serde(skip_serializing_if = "Option::is_none")]
    pub readme_url: Option<String>,
    /// 应用启用状态
    pub apps: SkillApps,
    /// 安装时间（Unix 时间戳）
    pub installed_at: i64,
    /// 内容哈希（SHA-256，用于更新检测）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    /// 最近更新时间（Unix 时间戳，0 = 从未更新）
    #[serde(default)]
    pub updated_at: i64,
}

/// 未管理的 Skill（在应用目录中发现但未被 Codex-Cube 管理）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnmanagedSkill {
    /// 目录名
    pub directory: String,
    /// 显示名称（从 SKILL.md 解析）
    pub name: String,
    /// 描述
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// 在哪些应用目录中发现（如 ["claude", "codex"]）
    pub found_in: Vec<String>,
    /// 发现路径（首个匹配的完整路径）
    pub path: String,
}

/// MCP 服务器定义（v3.7.0 统一结构）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServer {
    pub id: String,
    pub name: String,
    pub server: serde_json::Value,
    pub apps: McpApps,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docs: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// MCP 配置：单客户端维度（v3.6.x 及以前，保留用于向后兼容）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpConfig {
    /// 以 id 为键的服务器定义（宽松 JSON 对象，包含 enabled/source 等 UI 辅助字段）
    #[serde(default)]
    pub servers: HashMap<String, serde_json::Value>,
}

impl McpConfig {
    /// 检查配置是否为空
    pub fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }
}

/// MCP 根配置（v3.7.0 新旧结构并存）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpRoot {
    /// 统一的 MCP 服务器存储（v3.7.0+）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub servers: Option<HashMap<String, McpServer>>,

    /// 旧的分应用存储（仅保留 codex，用于迁移兼容）
    #[serde(default, skip_serializing_if = "McpConfig::is_empty")]
    pub codex: McpConfig,
}

impl Default for McpRoot {
    fn default() -> Self {
        Self {
            // v3.7.0+ 默认使用新的统一结构（空 HashMap）
            servers: Some(HashMap::new()),
            // 旧结构保持空，仅用于反序列化旧配置时的迁移
            codex: McpConfig::default(),
        }
    }
}

/// Prompt 配置：单客户端维度
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PromptConfig {
    #[serde(default)]
    pub prompts: HashMap<String, crate::prompt::Prompt>,
}

/// Prompt 根：按客户端分开维护
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PromptRoot {
    #[serde(default)]
    pub codex: PromptConfig,
}

use crate::config::{copy_file, get_app_config_dir, get_app_config_path, write_json_file};
use crate::error::AppError;
use crate::prompt_files::prompt_file_path;
use crate::provider::ProviderManager;

/// 应用类型（本项目仅支持 Codex）
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppType {
    Codex,
}

impl AppType {
    pub fn as_str(&self) -> &str {
        match self {
            AppType::Codex => "codex",
        }
    }

    /// 本项目只有 Codex（非累加模式）。
    pub fn is_additive_mode(&self) -> bool {
        false
    }

    /// Return an iterator over all app types
    pub fn all() -> impl Iterator<Item = AppType> {
        [AppType::Codex].into_iter()
    }
}

impl FromStr for AppType {
    type Err = AppError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let normalized = s.trim().to_lowercase();
        match normalized.as_str() {
            "codex" => Ok(AppType::Codex),
            other => Err(AppError::localized(
                "unsupported_app",
                format!("不支持的应用标识: '{other}'。本项目仅支持 codex。"),
                format!("Unsupported app id: '{other}'. This project supports only codex."),
            )),
        }
    }
}

/// 通用配置片段（按应用分治）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommonConfigSnippets {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex: Option<String>,
}

impl CommonConfigSnippets {
    /// 获取指定应用的通用配置片段
    pub fn get(&self, app: &AppType) -> Option<&String> {
        match app {
            AppType::Codex => self.codex.as_ref(),
        }
    }

    /// 设置指定应用的通用配置片段
    pub fn set(&mut self, app: &AppType, snippet: Option<String>) {
        match app {
            AppType::Codex => self.codex = snippet,
        }
    }
}

/// 多应用配置结构（向后兼容）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiAppConfig {
    #[serde(default = "default_version")]
    pub version: u32,
    /// 应用管理器（claude/codex）
    #[serde(flatten)]
    pub apps: HashMap<String, ProviderManager>,
    /// MCP 配置（按客户端分治）
    #[serde(default)]
    pub mcp: McpRoot,
    /// Prompt 配置（按客户端分治）
    #[serde(default)]
    pub prompts: PromptRoot,
    /// Claude Skills 配置
    #[serde(default)]
    pub skills: SkillStore,
    /// 通用配置片段（按应用分治）
    #[serde(default)]
    pub common_config_snippets: CommonConfigSnippets,
}

fn default_version() -> u32 {
    2
}

impl Default for MultiAppConfig {
    fn default() -> Self {
        let mut apps = HashMap::new();
        apps.insert("codex".to_string(), ProviderManager::default());

        Self {
            version: 2,
            apps,
            mcp: McpRoot::default(),
            prompts: PromptRoot::default(),
            skills: SkillStore::default(),
            common_config_snippets: CommonConfigSnippets::default(),
        }
    }
}

impl MultiAppConfig {
    /// 从文件加载配置（仅支持 v2 结构）
    pub fn load() -> Result<Self, AppError> {
        let config_path = get_app_config_path();

        if !config_path.exists() {
            log::info!("配置文件不存在，创建新的多应用配置并自动导入提示词");
            // 使用新的方法，支持自动导入提示词
            let config = Self::default_with_auto_import()?;
            // 立即保存到磁盘
            config.save()?;
            return Ok(config);
        }

        // 尝试读取文件
        let content =
            std::fs::read_to_string(&config_path).map_err(|e| AppError::io(&config_path, e))?;

        // 先解析为 Value，以便严格判定是否为 v1 结构；
        // 满足：顶层同时包含 providers(object) + current(string)，且不包含 version/apps/mcp 关键键，即视为 v1
        let value: serde_json::Value =
            serde_json::from_str(&content).map_err(|e| AppError::json(&config_path, e))?;
        let is_v1 = value.as_object().is_some_and(|map| {
            let has_providers = map.get("providers").map(|v| v.is_object()).unwrap_or(false);
            let has_current = map.get("current").map(|v| v.is_string()).unwrap_or(false);
            // v1 的充分必要条件：有 providers 和 current，且 apps 不存在（version/mcp 可能存在但不作为 v2 判据）
            let has_apps = map.contains_key("apps");
            has_providers && has_current && !has_apps
        });
        if is_v1 {
            return Err(AppError::localized(
                "config.unsupported_v1",
                "检测到旧版 v1 配置格式。当前版本已不再支持运行时自动迁移。\n\n解决方案：\n1. 安装 v3.2.x 版本进行一次性自动迁移\n2. 或手动编辑 ~/.codex-cube/config.json，将顶层结构调整为：\n   {\"version\": 2, \"claude\": {...}, \"codex\": {...}, \"mcp\": {...}}\n\n",
                "Detected legacy v1 config. Runtime auto-migration is no longer supported.\n\nSolutions:\n1. Install v3.2.x for one-time auto-migration\n2. Or manually edit ~/.codex-cube/config.json to adjust the top-level structure:\n   {\"version\": 2, \"claude\": {...}, \"codex\": {...}, \"mcp\": {...}}\n\n",
            ));
        }

        let has_skills_in_config = value
            .as_object()
            .is_some_and(|map| map.contains_key("skills"));

        // 解析 v2 结构
        let mut config: Self =
            serde_json::from_value(value).map_err(|e| AppError::json(&config_path, e))?;
        let mut updated = false;

        if !has_skills_in_config {
            let skills_path = get_app_config_dir().join("skills.json");
            if skills_path.exists() {
                match std::fs::read_to_string(&skills_path) {
                    Ok(content) => match serde_json::from_str::<SkillStore>(&content) {
                        Ok(store) => {
                            config.skills = store;
                            updated = true;
                            log::info!("已从旧版 skills.json 导入 Claude Skills 配置");
                        }
                        Err(e) => {
                            log::warn!("解析旧版 skills.json 失败: {e}");
                        }
                    },
                    Err(e) => {
                        log::warn!("读取旧版 skills.json 失败: {e}");
                    }
                }
            }
        }

        // 确保 codex 应用存在（兼容旧配置文件）
        if !config.apps.contains_key("codex") {
            config
                .apps
                .insert("codex".to_string(), ProviderManager::default());
            updated = true;
        }

        // 执行 MCP 迁移（v3.6.x → v3.7.0）
        let migrated = config.migrate_mcp_to_unified()?;
        if migrated {
            log::info!("MCP 配置已迁移到 v3.7.0 统一结构，保存配置...");
            updated = true;
        }

        // 对于已经存在的配置文件，如果此前版本还没有 Prompt 功能，
        // 且 prompts 仍然是空的，则尝试自动导入现有提示词文件。
        let imported_prompts = config.maybe_auto_import_prompts_for_existing_config()?;
        if imported_prompts {
            updated = true;
        }

        if updated {
            log::info!("配置结构已更新（包括 MCP 迁移或 Prompt 自动导入），保存配置...");
            config.save()?;
        }

        Ok(config)
    }

    /// 保存配置到文件
    pub fn save(&self) -> Result<(), AppError> {
        let config_path = get_app_config_path();
        // 先备份旧版（若存在）到 ~/.codex-cube/config.json.bak，再写入新内容
        if config_path.exists() {
            let backup_path = get_app_config_dir().join("config.json.bak");
            if let Err(e) = copy_file(&config_path, &backup_path) {
                log::warn!("备份 config.json 到 .bak 失败: {e}");
            }
        }

        write_json_file(&config_path, self)?;
        Ok(())
    }

    /// 获取指定应用的管理器
    pub fn get_manager(&self, app: &AppType) -> Option<&ProviderManager> {
        self.apps.get(app.as_str())
    }

    /// 获取指定应用的管理器（可变引用）
    pub fn get_manager_mut(&mut self, app: &AppType) -> Option<&mut ProviderManager> {
        self.apps.get_mut(app.as_str())
    }

    /// 确保应用存在
    pub fn ensure_app(&mut self, app: &AppType) {
        if !self.apps.contains_key(app.as_str()) {
            self.apps
                .insert(app.as_str().to_string(), ProviderManager::default());
        }
    }

    /// 获取指定客户端的 MCP 配置（不可变引用）
    pub fn mcp_for(&self, app: &AppType) -> &McpConfig {
        match app {
            AppType::Codex => &self.mcp.codex,
        }
    }

    /// 获取指定客户端的 MCP 配置（可变引用）
    pub fn mcp_for_mut(&mut self, app: &AppType) -> &mut McpConfig {
        match app {
            AppType::Codex => &mut self.mcp.codex,
        }
    }

    /// 创建默认配置并自动导入已存在的提示词文件
    fn default_with_auto_import() -> Result<Self, AppError> {
        log::info!("首次启动，创建默认配置并检测提示词文件");

        let mut config = Self::default();

        // 尝试自动导入 Codex 提示词
        Self::auto_import_prompt_if_exists(&mut config, AppType::Codex)?;

        Ok(config)
    }

    /// 已存在配置文件时的 Prompt 自动导入逻辑
    ///
    /// 适用于「老版本已经生成过 config.json，但当时还没有 Prompt 功能」的升级场景。
    /// 判定规则：
    /// - 仅当所有应用的 prompts 都为空时才尝试导入（避免打扰已经在使用 Prompt 功能的用户）
    /// - 每个应用最多导入一次，对应各自的提示词文件（如 CLAUDE.md/AGENTS.md/GEMINI.md）
    ///
    /// 返回值：
    /// - Ok(true)  表示至少有一个应用成功导入了提示词
    /// - Ok(false) 表示无需导入或未导入任何内容
    fn maybe_auto_import_prompts_for_existing_config(&mut self) -> Result<bool, AppError> {
        // 如果 Codex 已有提示词配置，说明用户已经在使用 Prompt 功能，避免再次自动导入
        if !self.prompts.codex.prompts.is_empty() {
            return Ok(false);
        }

        log::info!("检测到已存在配置文件且 Prompt 列表为空，将尝试从现有提示词文件自动导入");

        let mut imported = false;
        for app in [AppType::Codex] {
            // 复用已有的单应用导入逻辑
            if Self::auto_import_prompt_if_exists(self, app)? {
                imported = true;
            }
        }

        Ok(imported)
    }

    /// 检查并自动导入单个应用的提示词文件
    ///
    /// 返回值：
    /// - Ok(true)  表示成功导入了非空文件
    /// - Ok(false) 表示未导入（文件不存在、内容为空或读取失败）
    fn auto_import_prompt_if_exists(config: &mut Self, app: AppType) -> Result<bool, AppError> {
        let file_path = prompt_file_path(&app)?;

        // 检查文件是否存在
        if !file_path.exists() {
            log::debug!("提示词文件不存在，跳过自动导入: {file_path:?}");
            return Ok(false);
        }

        // 读取文件内容
        let content = match std::fs::read_to_string(&file_path) {
            Ok(c) => c,
            Err(e) => {
                log::warn!("读取提示词文件失败: {file_path:?}, 错误: {e}");
                return Ok(false); // 失败时不中断，继续处理其他应用
            }
        };

        // 检查内容是否为空
        if content.trim().is_empty() {
            log::debug!("提示词文件内容为空，跳过导入: {file_path:?}");
            return Ok(false);
        }

        log::info!("发现提示词文件，自动导入: {file_path:?}");

        // 创建提示词对象
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or_else(|_| {
                log::warn!("Failed to get system time, using 0 as timestamp");
                0
            });

        let id = format!("auto-imported-{timestamp}");
        let prompt = crate::prompt::Prompt {
            id: id.clone(),
            name: format!(
                "Auto-imported Prompt {}",
                chrono::Local::now().format("%Y-%m-%d %H:%M")
            ),
            content,
            description: Some("Automatically imported on first launch".to_string()),
            enabled: true, // 自动启用
            created_at: Some(timestamp),
            updated_at: Some(timestamp),
        };

        // 插入到 Codex 配置中
        let prompts = match app {
            AppType::Codex => &mut config.prompts.codex.prompts,
        };

        prompts.insert(id, prompt);

        log::info!("自动导入完成: {}", app.as_str());
        Ok(true)
    }

    /// 将 v3.6.x 的分应用 MCP 结构迁移到 v3.7.0 的统一结构
    ///
    /// 迁移策略：
    /// 1. 检查是否已经迁移（mcp.servers 是否存在）
    /// 2. 收集所有应用的 MCP，按 ID 去重合并
    /// 3. 生成统一的 McpServer 结构，标记应用到哪些客户端
    /// 4. 清空旧的分应用配置
    pub fn migrate_mcp_to_unified(&mut self) -> Result<bool, AppError> {
        // 检查是否已经是新结构
        if self.mcp.servers.is_some() {
            log::debug!("MCP 配置已是统一结构，跳过迁移");
            return Ok(false);
        }

        log::info!("检测到旧版 MCP 配置格式，开始迁移到 v3.7.0 统一结构...");

        let mut unified_servers: HashMap<String, McpServer> = HashMap::new();
        let mut conflicts = Vec::new();

        // 收集 Codex 的 MCP
        for app in [AppType::Codex] {
            let old_servers = match app {
                AppType::Codex => &self.mcp.codex.servers,
            };

            for (id, entry) in old_servers {
                let enabled = entry
                    .get("enabled")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);

                if let Some(existing) = unified_servers.get_mut(id) {
                    // 该 ID 已存在，合并 apps 字段
                    existing.apps.set_enabled_for(&app, enabled);

                    // 检测配置冲突（同 ID 但配置不同）
                    if existing.server != *entry.get("server").unwrap_or(&serde_json::json!({})) {
                        conflicts.push(format!(
                            "MCP '{id}' 在 {} 和之前的应用中配置不同，将使用首次遇到的配置",
                            app.as_str()
                        ));
                    }
                } else {
                    // 首次遇到该 MCP，创建新条目
                    let name = entry
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or(id)
                        .to_string();

                    let server = entry
                        .get("server")
                        .cloned()
                        .unwrap_or(serde_json::json!({}));

                    let description = entry
                        .get("description")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());

                    let homepage = entry
                        .get("homepage")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());

                    let docs = entry
                        .get("docs")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());

                    let tags = entry
                        .get("tags")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                .collect()
                        })
                        .unwrap_or_default();

                    let mut apps = McpApps::default();
                    apps.set_enabled_for(&app, enabled);

                    unified_servers.insert(
                        id.clone(),
                        McpServer {
                            id: id.clone(),
                            name,
                            server,
                            apps,
                            description,
                            homepage,
                            docs,
                            tags,
                        },
                    );
                }
            }
        }

        // 记录冲突警告
        if !conflicts.is_empty() {
            log::warn!("MCP 迁移过程中检测到配置冲突：");
            for conflict in &conflicts {
                log::warn!("  - {conflict}");
            }
        }

        log::info!(
            "MCP 迁移完成，共迁移 {} 个服务器{}",
            unified_servers.len(),
            if !conflicts.is_empty() {
                format!("（存在 {} 个冲突）", conflicts.len())
            } else {
                String::new()
            }
        );

        // 替换为新结构
        self.mcp.servers = Some(unified_servers);

        // 清空旧的分应用配置
        self.mcp.codex = McpConfig::default();

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
}
