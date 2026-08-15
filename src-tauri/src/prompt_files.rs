use std::path::PathBuf;

use crate::app_config::AppType;
use crate::codex_config::get_codex_auth_path;
use crate::error::AppError;

/// 返回指定应用所使用的提示词文件路径。
pub fn prompt_file_path(app: &AppType) -> Result<PathBuf, AppError> {
    let base_dir: PathBuf = match app {
        AppType::Codex => get_base_dir_with_fallback(get_codex_auth_path(), ".codex")?,
    };

    let filename = match app {
        AppType::Codex => "AGENTS.md",
    };

    Ok(base_dir.join(filename))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_prompt_file_uses_agents_md() {
        let path = prompt_file_path(&AppType::Codex).expect("Codex prompt path");

        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("AGENTS.md")
        );
    }
}

fn get_base_dir_with_fallback(
    primary_path: PathBuf,
    fallback_dir: &str,
) -> Result<PathBuf, AppError> {
    primary_path
        .parent()
        .map(|p| p.to_path_buf())
        .or_else(|| dirs::home_dir().map(|h| h.join(fallback_dir)))
        .ok_or_else(|| {
            AppError::localized(
                "home_dir_not_found",
                format!("无法确定 {fallback_dir} 配置目录：用户主目录不存在"),
                format!("Cannot determine {fallback_dir} config directory: user home not found"),
            )
        })
}
