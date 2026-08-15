//! 官方供应商种子数据
//!
//! 启动时调用 `Database::init_default_official_providers` 把这些条目
//! 写入 `providers` 表，让所有用户都能看到一个"一键切回官方"的入口。
//!
//! 字段与前端预设保持一致，参见：
//! - `src/config/codexProviderPresets.ts`（"OpenAI Official"）

use crate::app_config::AppType;

pub(crate) const CODEX_OFFICIAL_PROVIDER_ID: &str = "codex-official";

/// 单条官方供应商种子定义。
pub(crate) struct OfficialProviderSeed {
    pub id: &'static str,
    pub app_type: AppType,
    pub name: &'static str,
    pub website_url: &'static str,
    pub icon: &'static str,
    pub icon_color: &'static str,
    /// settings_config 的 JSON 字符串，每个 app 结构不同。
    pub settings_config_json: &'static str,
}

/// Codex 的官方预设。
///
/// id 固定，便于幂等检查；name 直接用英文原名（与前端预设一致），不做 i18n。
pub(crate) const OFFICIAL_SEEDS: &[OfficialProviderSeed] = &[OfficialProviderSeed {
    id: CODEX_OFFICIAL_PROVIDER_ID,
    app_type: AppType::Codex,
    name: "OpenAI Official",
    website_url: "https://chatgpt.com/codex",
    icon: "openai",
    icon_color: "#00A67E",
    // 空 auth + 空 config 让用户走 ChatGPT Plus/Pro OAuth
    settings_config_json: r#"{"auth":{},"config":""}"#,
}];

/// 判断给定的 provider id 是否属于内置官方种子。
///
/// 单一事实源：直接扫描 `OFFICIAL_SEEDS`，避免在多处重复维护 id 列表。
pub(crate) fn is_official_seed_id(id: &str) -> bool {
    OFFICIAL_SEEDS.iter().any(|seed| seed.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
}
