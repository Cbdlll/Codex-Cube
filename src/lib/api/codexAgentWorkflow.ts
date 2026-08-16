import { invoke } from "@tauri-apps/api/core";

export type CodexSubagentSandboxMode =
  | "inherit"
  | "read-only"
  | "workspace-write"
  | "danger-full-access";

/** 已注册的官方 Codex subagent。 */
export interface CodexSubagent {
  managed: boolean;
  available?: boolean;
  name: string;
  /** 自定义 subagent 文件（~/.codex/agents/<name>.toml）的完整路径。 */
  agentPath: string;
  description: string;
  model: string;
  modelProviderId: string;
  modelBaseUrl: string;
  /** 出于安全后端不回传明文；"" 表示已配置，null 表示未设置。 */
  apiKey: string | null;
  sandboxMode: CodexSubagentSandboxMode;
  reasoningEffort: string;
  /** Provider 协议（responses / chat 等），由后端从 provider config 读出。 */
  wireApi: string;
  /** 注册角色类型：worker | explorer | default。 */
  agentType: string;
}

export interface CodexSubagentUpsertPayload {
  name: string;
  description: string;
  model: string;
  modelProviderId: string;
  modelBaseUrl: string;
  /** 可选；省略或空串表示编辑时保留现有 key。 */
  apiKey?: string;
  sandboxMode: CodexSubagentSandboxMode;
  reasoningEffort: string;
  /** 可选；旧 payload 未提供时后端默认 responses。 */
  wireApi?: string;
  /** 注册角色类型：worker | explorer | default；省略时后端默认 worker。 */
  agentType?: string;
}

export interface CodexSubagentModelsFetchPayload {
  modelProviderId: string;
  modelBaseUrl: string;
  apiKey: string;
}

export interface CodexSubagentModelCandidate {
  model: string;
  displayName: string | null;
}

export interface CodexAgentWorkflowRoleAgents {
  worker: string[];
  explorer: string[];
  default: string[];
}

export interface CodexAgentWorkflowStatus {
  installed: boolean;
  canUndo: boolean;
  workerAgent: string;
  workerAgents: string[];
  /** 角色 → agent 映射（worker / explorer / default）；旧 manifest 由 workerAgents 推导。 */
  roleAgents: CodexAgentWorkflowRoleAgents;
  workerModel: string;
  workerReasoningEffort: string;
  modelProvider: string | null;
  sandboxMode: string;
  /** AGENTS.md 中当前存在的 Codex Cube 受管约束块；不存在时为 null。 */
  workerInstructions: string | null;
  manifestPath: string;
  agentPath: string;
  instructionsPath: string;
  /** 安装模式："skill"（Workflow Skill）或 "agents-md"（遗留 AGENTS.md 注入）。 */
  mode: string;
  /** Workflow Skill 是否已安装（DB 记录存在且 SSOT SKILL.md 存在）。 */
  skillInstalled: boolean;
  /** 固定 Skill 记录 ID："local:cube-dispatch"。 */
  skillId: string;
  /** 固定 Skill 目录名："cube-dispatch"。 */
  skillDirectory: string;
  /** SSOT 中 SKILL.md 的完整路径。 */
  skillPath: string;
  /** 当前 SKILL.md 内容；不存在时为 null。 */
  skillContent: string | null;
  /** 已安装 Skill 内容与当前 subagent 列表不一致（建议更新）。 */
  skillStale: boolean;
}

export interface CodexAgentWorkflowInstallPayload {
  workerAgent: string;
  workerAgents: string[];
  /** 角色 → agent 映射；不传时后端把 workerAgents 归入 worker 角色。 */
  roleAgents?: CodexAgentWorkflowRoleAgents;
}

export const codexAgentWorkflowApi = {
  async listSubagents(): Promise<CodexSubagent[]> {
    return invoke<CodexSubagent[]>("list_codex_subagents");
  },

  async upsertSubagent(
    payload: CodexSubagentUpsertPayload,
  ): Promise<CodexSubagent> {
    return invoke<CodexSubagent>("upsert_codex_subagent", { payload });
  },

  async deleteSubagent(name: string): Promise<void> {
    return invoke<void>("delete_codex_subagent", { name });
  },

  async fetchSubagentModels(
    payload: CodexSubagentModelsFetchPayload,
  ): Promise<CodexSubagentModelCandidate[]> {
    return invoke<CodexSubagentModelCandidate[]>(
      "fetch_codex_subagent_models",
      { payload },
    );
  },

  async getWorkflowStatus(): Promise<CodexAgentWorkflowStatus> {
    return invoke<CodexAgentWorkflowStatus>("get_codex_agent_workflow_status");
  },

  async installWorkflow(
    payload: CodexAgentWorkflowInstallPayload,
  ): Promise<CodexAgentWorkflowStatus> {
    return invoke<CodexAgentWorkflowStatus>("install_codex_agent_workflow", {
      payload,
    });
  },

  async cancelWorkflowInstructions(): Promise<CodexAgentWorkflowStatus> {
    return invoke<CodexAgentWorkflowStatus>(
      "cancel_codex_agent_workflow_instructions",
    );
  },
};
