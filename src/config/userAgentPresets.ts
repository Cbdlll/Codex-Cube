/**
 * 自定义 User-Agent 预设。
 *
 * 非 Codex 客户端（Claude CLI 等）的 UA 预设已随应用收窄移除；
 * 保留空数组以维持 CustomUserAgentField 预设下拉的渲染契约。
 */
export const USER_AGENT_PRESETS: readonly string[] = [];
