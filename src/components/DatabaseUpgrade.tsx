import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { exit } from "@tauri-apps/plugin-process";
import { Database, ExternalLink, FolderOpen } from "lucide-react";
import { Button } from "@/components/ui/button";

const RELEASES_URL = "https://github.com/Cbdlll/Codex-Cube/releases";

interface DatabaseUpgradeProps {
  payload: {
    path?: string;
    error?: string;
    kind?: string;
    db_version?: number;
    supported_version?: number;
  };
}

/**
 * 数据库版本过新（应用过旧）时的恢复界面。
 * 引导用户从 GitHub Releases 下载新版本，不在应用内检查或安装更新。
 */
export function DatabaseUpgrade({ payload }: DatabaseUpgradeProps) {
  const { t } = useTranslation();
  const dbVersion = payload.db_version;
  const supportedVersion = payload.supported_version;

  return (
    <div className="flex min-h-screen items-center justify-center bg-background p-6 text-foreground">
      <div className="w-full max-w-lg space-y-5 rounded-2xl border border-border/60 bg-card/80 p-7 shadow-xl">
        <div className="flex items-start gap-4">
          <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-xl bg-amber-100 text-amber-600 dark:bg-amber-950/50 dark:text-amber-400">
            <Database className="h-6 w-6" />
          </div>
          <div className="space-y-1">
            <h1 className="text-lg font-semibold">
              {t("dbUpgrade.title", "数据库版本过新")}
            </h1>
            <p className="text-sm text-muted-foreground">
              {t(
                "dbUpgrade.description",
                "当前数据库由更新版本的 Codex Cube 创建。请从 GitHub Releases 下载最新版本后再打开。数据不会被删除。",
              )}
            </p>
            {dbVersion != null && supportedVersion != null && (
              <p className="pt-0.5 text-xs text-muted-foreground tabular-nums">
                {t("dbUpgrade.versionInfo", {
                  db: dbVersion,
                  supported: supportedVersion,
                  defaultValue: "数据库版本 v{{db}} · 应用支持 v{{supported}}",
                })}
              </p>
            )}
          </div>
        </div>

        <div className="space-y-1 rounded-lg border border-border/50 bg-muted/40 p-3 text-xs text-muted-foreground">
          {payload.error && (
            <p className="break-words font-mono">{payload.error}</p>
          )}
          {payload.path && (
            <p className="break-all">
              {t("dbUpgrade.dbPath", "数据库文件")}：{payload.path}
            </p>
          )}
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <Button
            className="gap-2 bg-amber-500 text-white hover:bg-amber-600"
            onClick={() => void invoke("open_external", { url: RELEASES_URL })}
          >
            <ExternalLink className="h-4 w-4" />
            {t("dbUpgrade.openReleases", "打开发布页")}
          </Button>

          <Button
            variant="outline"
            className="gap-2"
            onClick={() => void invoke("open_app_config_folder")}
          >
            <FolderOpen className="h-4 w-4" />
            {t("dbUpgrade.openConfigDir", "打开配置目录")}
          </Button>

          <Button
            variant="ghost"
            className="ml-auto text-muted-foreground"
            onClick={() => void exit(0)}
          >
            {t("dbUpgrade.quit", "退出")}
          </Button>
        </div>
      </div>
    </div>
  );
}

export default DatabaseUpgrade;
