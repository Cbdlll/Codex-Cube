export type ProviderCategory =
  | "official" // 官方
  | "cn_official" // 开源官方（原"国产官方"）
  | "cloud_provider" // 云服务商（AWS Bedrock 等）
  | "aggregator" // 聚合网站
  | "third_party" // 第三方供应商
  | "custom"; // 自定义

export interface Provider {
  id: string;
  name: string;
  settingsConfig: Record<string, any>; // 应用配置对象：Claude 为 settings.json；Codex 为 { auth, config }
  websiteUrl?: string;
  // 新增：供应商分类（用于差异化提示/能力开关）
  category?: ProviderCategory;
  createdAt?: number; // 添加时间戳（毫秒）
  sortIndex?: number; // 排序索引（用于自定义拖拽排序）
  // 备注信息
  notes?: string;
  // 可选：供应商元数据（仅存于 ~/.codex-cube/config.json，不写入 live 配置）
  meta?: ProviderMeta;
  // 图标配置
  icon?: string; // 图标名称（如 "openai", "anthropic"）
  iconColor?: string; // 图标颜色（Hex 格式，如 "#00A67E"）
  // 是否加入故障转移队列
  inFailoverQueue?: boolean;
}

export interface AppConfig {
  providers: Record<string, Provider>;
  current: string;
}

// 自定义端点配置
export interface CustomEndpoint {
  url: string;
  addedAt: number;
  lastUsed?: number;
}

// 端点候选项（用于端点测速弹窗）
export interface EndpointCandidate {
  id?: string;
  url: string;
  isCustom?: boolean;
}

import type { TemplateType } from "./config/constants";

// 用量查询脚本配置
export interface UsageScript {
  enabled: boolean; // 是否启用用量查询
  language: "javascript"; // 脚本语言
  code: string; // 脚本代码（JSON 格式配置）
  timeout?: number; // 超时时间（秒，默认 10）
  templateType?: TemplateType; // 模板类型（用于后端判断验证规则）
  apiKey?: string; // 用量查询专用的 API Key（通用模板使用）
  baseUrl?: string; // 用量查询专用的 Base URL（通用和 NewAPI 模板使用）
  accessToken?: string; // 访问令牌（NewAPI 模板使用）
  userId?: string; // 用户ID（NewAPI 模板使用）
  accessKeyId?: string; // 火山方舟 AccessKey ID（用量查询签名用，与推理 Key 分离）
  secretAccessKey?: string; // 火山方舟 SecretAccessKey
  teamOrganizationId?: string; // 智谱团队套餐组织 ID（请求头 bigmodel-organization）
  teamProjectId?: string; // 智谱团队套餐项目 ID（请求头 bigmodel-project）
  codingPlanProvider?: string; // Coding Plan 供应商标识（如 "kimi", "zhipu", "minimax"）
  autoQueryInterval?: number; // 自动查询间隔（单位：分钟，0 表示禁用）
  autoIntervalMinutes?: number; // 自动查询间隔（分钟）- 别名字段
  request?: {
    // 请求配置
    url?: string; // 请求 URL
    method?: string; // HTTP 方法
    headers?: Record<string, string>; // 请求头
    body?: any; // 请求体
  };
}

const DEFAULT_USAGE_SCRIPT: UsageScript = {
  enabled: false,
  language: "javascript",
  code: "",
  timeout: 10,
  autoQueryInterval: 5,
};

export function createUsageScript(
  overrides?: Partial<UsageScript>,
): UsageScript {
  return { ...DEFAULT_USAGE_SCRIPT, ...overrides };
}

// 单个套餐用量数据
export interface UsageData {
  planName?: string; // 套餐名称（可选）
  extra?: string; // 扩展字段，可自由补充需要展示的文本（可选）
  isValid?: boolean; // 套餐是否有效（可选）
  invalidMessage?: string; // 失效原因说明（可选，当 isValid 为 false 时显示）
  total?: number; // 总额度（可选）
  used?: number; // 已用额度（可选）
  remaining?: number; // 剩余额度（可选）
  unit?: string; // 单位（可选）
}

// 用量查询结果（支持多套餐）
export interface UsageResult {
  success: boolean;
  data?: UsageData[]; // 改为数组，支持返回多个套餐
  error?: string;
}

export type AuthBindingSource = "provider_config" | "managed_account";

export interface AuthBinding {
  source: AuthBindingSource;
  authProvider?: string;
  accountId?: string;
}

export type CodexChatThinkingParam =
  | "none"
  | "thinking"
  | "enable_thinking"
  | "reasoning_split";

export type CodexChatEffortParam =
  | "none"
  | "reasoning_effort"
  // OpenRouter 原生归一化对象 reasoning:{effort}（区别于顶层 OpenAI 别名 reasoning_effort）
  | "reasoning.effort";

export type CodexChatEffortValueMode =
  | "passthrough"
  | "low_high"
  | "deepseek"
  // OpenRouter effort 枚举 xhigh|high|medium|low|minimal（无 max，max 钳到 xhigh）
  | "openrouter";

export type CodexChatReasoningOutputFormat =
  | "auto"
  | "reasoning_content"
  | "reasoning"
  | "reasoning_details"
  | "think_tags";

export interface CodexChatReasoning {
  supportsThinking?: boolean;
  supportsEffort?: boolean;
  thinkingParam?: CodexChatThinkingParam;
  effortParam?: CodexChatEffortParam;
  effortValueMode?: CodexChatEffortValueMode;
  // 声明性字段：标注上游 reasoning 回传位置。当前提取靠穷举字段，未读取此值（think_tags 尚未接线）。
  outputFormat?: CodexChatReasoningOutputFormat;
}

export type PromptCacheRoutingMode = "auto" | "enabled" | "disabled";

export interface LocalProxyRequestOverrides {
  headers?: Record<string, string>;
  body?: Record<string, unknown>;
}

// 供应商元数据（字段名与后端一致，保持 snake_case）
export interface ProviderMeta {
  // 自定义端点：以 URL 为键，值为端点信息
  custom_endpoints?: Record<string, CustomEndpoint>;
  // 是否在切换/同步到 live 时应用通用配置片段
  commonConfigEnabled?: boolean;
  // Claude Desktop 3P 配置写入模式
  // 用量查询脚本配置
  usage_script?: UsageScript;
  // 请求地址管理：测速后自动选择最佳端点
  endpointAutoSelect?: boolean;
  // 供应商成本倍率
  costMultiplier?: string;
  // 供应商计费模式来源
  pricingModelSource?: string;
  // API 格式（Codex 供应商使用）
  // - "anthropic": 原生 Anthropic Messages API 格式，直接透传
  // - "openai_chat": OpenAI Chat Completions 格式，需要格式转换
  // - "openai_responses": OpenAI Responses API 格式，需要格式转换
  apiFormat?: "anthropic" | "openai_chat" | "openai_responses";
  // 通用认证绑定
  authBinding?: AuthBinding;
  // Claude 认证字段名
  apiKeyField?: ClaudeApiKeyField;
  // 是否将 base_url 视为完整 API 端点（代理直接使用此 URL，不拼接路径）
  isFullUrl?: boolean;
  // Prompt cache key for OpenAI Responses-compatible endpoints (improves cache hit rate)
  promptCacheKey?: string;
  // Session-based prompt-cache routing for Codex Responses -> Chat conversions.
  // auto enables only for known-compatible upstreams; enabled/disabled are user overrides.
  promptCacheRouting?: PromptCacheRoutingMode;
  // Codex OAuth FAST mode: injects service_tier="priority" on ChatGPT Codex requests
  codexFastMode?: boolean;
  // Codex Responses -> Chat Completions reasoning capability metadata
  codexChatReasoning?: CodexChatReasoning;
  // Codex → Anthropic path: emulate the Claude Code client (disabled by default; only an explicit true enables it)
  impersonateClaudeCode?: boolean;
  // Codex → Anthropic path: override the Anthropic max_tokens (output ceiling).
  // Codex does not forward model_max_output_tokens in the request body; without
  // this the path falls back to a conservative 8192 default, which can truncate
  // long/thinking-heavy responses. When set (>0) it takes precedence over the
  // request value and the default.
  maxOutputTokens?: number;
  // Custom User-Agent for local proxy routing. Only applied by the local proxy.
  customUserAgent?: string;
  // Local proxy request overrides. Only applied by the local proxy after route transforms.
  localProxyRequestOverrides?: LocalProxyRequestOverrides;
  // 供应商类型（用于识别 Copilot 等特殊供应商）
  providerType?: string;
  // GitHub Copilot 关联账号 ID（旧字段，保留兼容读取）
  githubAccountId?: string;
}

// Codex API 格式类型
// - "openai_responses": OpenAI Responses API 格式，直接透传
// - "openai_chat": OpenAI Chat Completions 格式，需要本地路由转换
// - "anthropic": native Anthropic Messages format, needs local routing to convert to Responses
export type CodexApiFormat = "openai_responses" | "openai_chat" | "anthropic";

export interface CodexCatalogModel {
  model: string;
  displayName?: string;
  contextWindow?: string | number;
  // Hidden provider capability metadata for the generated model catalog.
  // supportsParallelToolCalls is native-profile-only; inputModalities wins over
  // automatic text-only model detection for every profile.
  supportsParallelToolCalls?: boolean;
  inputModalities?: string[];
  // Vendor's OFFICIAL base_instructions (model identity / system preamble).
  // Codex requires this field in every catalog entry; when omitted the backend
  // falls back to a neutral default. e.g. MiMo "developed by Xiaomi".
  baseInstructions?: string;
}

// 聚合 Provider 的一条模型映射：对外展示给 Codex 的插槽名 -> 成员供应商 + 上游模型。
export interface AggregateProviderModel {
  // Codex 看到的模型 id（同名冲突时自动加 `@供应商` 后缀保证唯一）
  model: string;
  // Codex 列表显示名（同名冲突时自动改为 `模型 (供应商名)`，可编辑）
  displayName?: string;
  // 绑定的成员供应商 id
  providerId: string;
  // 实际发往上游的模型名（缺省 = 成员原本的模型 id）
  upstreamModel?: string;
  // 该模型的上游协议；缺省时继承成员 Provider。
  apiFormat?: CodexApiFormat;
  contextWindow?: string | number;
  supportsParallelToolCalls?: boolean;
  inputModalities?: string[];
  baseInstructions?: string;
}

// Claude 认证字段类型
export type ClaudeApiKeyField = "ANTHROPIC_AUTH_TOKEN" | "ANTHROPIC_API_KEY";

// 主页面显示的应用配置
export interface VisibleApps {
  codex: boolean;
}

// 应用设置类型（用于设置对话框与 Tauri API）
// 存储在本地 ~/.codex-cube/settings.json，不随数据库同步
export interface Settings {
  // ===== 设备级 UI 设置 =====
  // 是否在系统托盘（macOS 菜单栏）显示图标
  showInTray: boolean;
  // 点击关闭按钮时是否最小化到托盘而不是关闭应用
  minimizeToTrayOnClose: boolean;
  // 是否启用应用级窗口控制按钮（最小化/最大化/关闭）
  useAppWindowControls?: boolean;
  // 是否开机自启
  launchOnStartup?: boolean;
  // 静默启动（程序启动时不显示主窗口）
  silentStartup?: boolean;
  // User has confirmed the local proxy first-run notice
  proxyConfirmed?: boolean;
  // User has confirmed the usage query first-run notice
  usageConfirmed?: boolean;
  usageDashboardRefreshIntervalMs?: number;
  // Whether to show the failover toggle independently on the main page
  enableFailoverToggle?: boolean;
  // Whether to show the project profile switcher on the main page header
  showProfileSwitcher?: boolean;
  // Preserve Codex ChatGPT login in auth.json when switching third-party providers
  preserveCodexOfficialAuthOnSwitch?: boolean;
  // Run official Codex under the shared "custom" provider id so future
  // sessions share one resume-history bucket with third-party providers
  unifyCodexSessionHistory?: boolean;
  // User opted in (enable dialog checkbox) to migrate existing official sessions
  unifyCodexMigrateExisting?: boolean;
  // User has confirmed the failover toggle first-run notice
  failoverConfirmed?: boolean;
  // User has confirmed the first-run welcome notice
  firstRunNoticeConfirmed?: boolean;
  // User has confirmed the common config first-run notice
  commonConfigConfirmed?: boolean;
  // 首选语言（可选，默认中文）
  language?: "en" | "zh" | "zh-TW" | "ja";

  // 主页面显示的应用（默认全部显示）
  visibleApps?: VisibleApps;

  // ===== 设备级目录覆盖 =====
  // 覆盖 Codex 配置目录（可选）
  codexConfigDir?: string;

  // ===== 当前供应商 ID（设备级）=====
  // 当前 Codex 供应商 ID（优先于数据库 is_current）
  currentProviderCodex?: string;

  // ===== 备份策略设置 =====
  // Auto-backup interval in hours (0=disabled, default 24)
  backupIntervalHours?: number;
  // Maximum backup files to retain (default 10)
  backupRetainCount?: number;

  // ===== 终端设置 =====
  // 首选终端应用（可选，默认使用系统默认终端）
  // macOS: "terminal" | "iterm2" | "warp" | "alacritty" | "kitty" | "ghostty" | "wezterm" | "kaku"
  // Windows: "cmd" | "powershell" | "wt"
  // Linux: "gnome-terminal" | "konsole" | "xfce4-terminal" | "alacritty" | "kitty" | "ghostty"
  preferredTerminal?: string;

  // ===== 本机自动迁移状态 =====
  localMigrations?: {
    codexThirdPartyHistoryProviderBucketV1?: {
      completedAt: string;
      targetProviderId: string;
      sourceProviderIds?: string[];
      migratedJsonlFiles?: number;
      migratedStateRows?: number;
    };
  };
}
