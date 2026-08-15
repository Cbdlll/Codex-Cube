import React from "react";
import type { AppId } from "@/lib/api/types";
import { CodexIcon } from "@/components/BrandIcons";

export interface AppConfig {
  label: string;
  icon: React.ReactNode;
  activeClass: string;
  badgeClass: string;
}

export const APP_IDS: AppId[] = ["codex"];

/** App IDs shown in Skills panels */
export const SKILLS_APP_IDS: AppId[] = ["codex"];

/** App IDs shown in MCP panels */
export const MCP_APP_IDS: AppId[] = [...SKILLS_APP_IDS];

export const APP_ICON_MAP: Record<AppId, AppConfig> = {
  codex: {
    label: "Codex",
    icon: <CodexIcon size={14} />,
    activeClass:
      "bg-green-500/10 ring-1 ring-green-500/20 hover:bg-green-500/20 text-green-600 dark:text-green-400",
    badgeClass:
      "bg-green-500/10 text-green-700 dark:text-green-300 hover:bg-green-500/20 border-0 gap-1.5",
  },
};
