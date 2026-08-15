<div align="center">

<img src="src-tauri/icons/icon.png" width="96" alt="Codex Cube" />

# Codex Cube

### 把不同供应商聚合为一个，统一管理 Codex 订阅

[![Windows](https://img.shields.io/badge/Windows-0078D6?style=flat-square&logo=windows&logoColor=white)](https://github.com/Cbdlll/Codex-Cube/releases)
[![macOS](https://img.shields.io/badge/macOS-000000?style=flat-square&logo=apple&logoColor=white)](https://github.com/Cbdlll/Codex-Cube/releases)
[![Linux](https://img.shields.io/badge/Linux-FCC624?style=flat-square&logo=linux&logoColor=black)](https://github.com/Cbdlll/Codex-Cube/releases)
[![Tauri](https://img.shields.io/badge/Tauri%202-24C8D8?style=flat-square&logo=tauri&logoColor=white)](https://tauri.app/)
[![React](https://img.shields.io/badge/React%2018-61DAFB?style=flat-square&logo=react&logoColor=black)](https://react.dev/)
[![TypeScript](https://img.shields.io/badge/TypeScript-3178C6?style=flat-square&logo=typescript&logoColor=white)](https://www.typescriptlang.org/)
[![Rust](https://img.shields.io/badge/Rust-000000?style=flat-square&logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![SQLite](https://img.shields.io/badge/SQLite-003B57?style=flat-square&logo=sqlite&logoColor=white)](https://www.sqlite.org/)
[![License](https://img.shields.io/badge/License-MIT-blue?style=flat-square)](LICENSE)

[English](README.md) | 简体中文

</div>

## 💡 为什么选择 Codex Cube？

Codex 用户通常会同时使用多个 ChatGPT 订阅、第三方 API 供应商、Coding Plan 账号和本地模型端点。切换时通常要手动编辑 `~/.codex/auth.json` 和 `~/.codex/config.toml`，还容易丢失插件或 Hook，并需要反复重启 Codex。

**Codex Cube** 用一个桌面应用统一管理这些 Codex 配置。不需要手写 JSON 和 TOML，首次导入现有配置后，即可通过预设添加供应商，并在官方订阅和第三方供应商之间一键切换。它还能**把不同供应商聚合为一个 Provider**，支持**兼容 cc-switch 的一键导入**（中转站的 `ccswitch://` 链接），并提供本地路由的热切换、故障转移、请求修复和用量统计。

- 🧬 **聚合 Provider** - 把官方订阅、第三方 API、Coding Plan 和中转站合并为一个虚拟 Codex Provider，所有成员模型统一出现在同一个模型选择器中
- 🔗 **兼容 cc-switch 一键导入** - 支持中转站生成的 `ccswitch://` 一键导入链接，并内置 60+ 供应商预设（官方、聚合、第三方和国内供应商）
- 🎯 **专注 Codex** - 在一个界面管理 Codex 官方订阅、托管 OAuth 账号和第三方 API 供应商
- ✏️ **告别手动编辑** - 导入现有配置、使用 60+ 内置供应商预设（含中转站一键导入），或创建自定义供应商
- 🔄 **一键切换** - 从供应商列表或系统托盘直接切换
- ⚡ **本地路由与故障转移** - 协议转换、热切换、故障转移队列、熔断器、健康检查和请求整流
- 📊 **用量与成本追踪** - 官方订阅额度、Token Plan、余额脚本、请求日志、趋势图和自定义定价
- 🧩 **MCP、Prompts 与 Skills** - 在 Codex Cube 单一数据源中保存并同步到 Codex，支持 Deep Link 导入和项目快照
- 📁 **项目（Projects）** - 保存 Provider、MCP、Skills、Prompts 和 memory 文件快照，从顶部一键切换
- ☁️ **云同步** - 支持 WebDAV 和 S3 兼容服务跨设备同步，可选自动同步
- 🖥️ **跨平台** - 基于 Tauri 2 的原生桌面应用，支持 Windows、macOS 和 Linux

## ✨ 功能特性

### 🗂️ 供应商与订阅管理

- **多账号类型** - Codex/ChatGPT 官方账号、ChatGPT 和 xAI/Grok 托管 OAuth、第三方 API、Coding Plan 和自定义端点
- **60+ 内置预设** - 官方、聚合、第三方和国内供应商预设（含中转站一键导入）；填入 API Key 即可使用
- **导入当前配置** - 首次启动时把现有 Codex 配置保存为 `default` 供应商
- **通用配置片段** - 在不同供应商之间共享插件、Hook、环境变量等非敏感配置
- **供应商列表** - 一键切换、拖拽排序、复制、搜索、备注、图标和官网链接
- **连通性检测** - 不发送真实模型请求即可测试供应商可达性
- **供应商用量查询** - 配置官方订阅、Token Plan、余额、GitHub Copilot 或自定义脚本
- **系统托盘** - 不打开主界面即可切换供应商并查看订阅额度

### 🪢 聚合 Provider

- 把不同供应商（官方订阅、第三方 API、Coding Plan、中转站）合并为一个虚拟 Provider
- 从成员拉取模型列表，编辑显示名、上下文窗口和能力声明
- 在 Codex 模型选择器中同时看到所有成员模型，无需切换供应商
- 切换到聚合 Provider 后自动准备本地路由，无需重启 Codex

### 🔗 兼容 cc-switch 一键导入

- 兼容中转站生成的 `ccswitch://` 一键导入链接 - 点击链接即可导入供应商并直接使用
- `codexcube://` 深链还可导入 MCP 服务器、Prompts 和 Skills
- 内置 60+ 供应商预设，覆盖官方、聚合、第三方和国内中转站供应商

### ⚡ 本地路由、故障转移与稳定性

- **本地路由服务** - 可配置监听地址和端口，支持 Codex 接管
- **热切换** - 开启接管后，在列表或托盘中切换供应商无需重启 Codex
- **协议支持** - 按需在 OpenAI Responses、OpenAI Chat Completions 和 Anthropic Messages 之间转换
- **自动故障转移** - 按 P1 起的优先级队列、重试、超时控制和供应商健康状态
- **熔断器** - 连续失败阈值、错误率阈值、恢复等待时间和半开恢复
- **请求整流器** - 修复 thinking signature 和 thinking budget 错误、图片不支持时回退、预检纯文本模型
- **全局出站代理** - 让 Codex Cube 的全部流量，包括 API 和 Skills 下载，走 HTTP 代理
- **请求覆盖** - 自定义 User-Agent 和供应商级本地代理请求覆盖

### 📊 用量与成本

- **用量仪表盘** - 时间范围、供应商/模型筛选、趋势图、成功率、延迟、Token 和成本
- **请求日志** - 查询供应商、模型、状态、Token、成本和会话等详细请求记录
- **自定义定价** - 每百万 Token 输入/输出/缓存价格、成本倍率和计价模型来源
- **models.dev 自动同步** - 从 models.dev 导入模型定价，支持选择供应商和常见模型
- **会话用量** - 从本地会话日志同步或重建 Codex 用量

### 🧩 MCP、Prompts 与 Skills

- **MCP 服务器** - 内置预设、stdio/HTTP/SSE 配置向导、JSON/TOML 输入，以及从现有 Codex 配置导入
- **Prompts** - Codex Prompt 文件的 Markdown 编辑和启用/停用管理
- **Skills** - 从 GitHub 仓库或 ZIP 文件安装，搜索 skills.sh，管理仓库并导入现有技能
- **同步方式** - 主副本保存在 `~/.codex-cube/skills/` 或 `~/.agents/skills/`，支持软链接或文件复制
- **备份** - 卸载或变更前自动备份，并提供恢复流程
- **Deep Link** - 通过 `codexcube://` 导入供应商、MCP 服务器、Prompts 和 Skills，并兼容 `ccswitch://` 中转站一键导入链接

### 📁 项目、会话与协作

- **项目** - 对 Provider、MCP、Skills、Prompts 和 memory 文件配置拍快照，从顶部切换项目
- **会话管理器** - 浏览和搜索 Codex 会话历史、查看对话记录、复制或恢复命令、删除会话
- **统一会话历史** - 可选把官方和第三方 Codex 会话合并到一个历史列表，支持备份和恢复
- **协作工作流** - 把低成本模型注册为 Codex 子代理，选择 Worker，并安装 `@cube-dispatch` Workflow Skill
- **Codex Desktop 模型菜单解锁** - 以调试模式启动 Codex Desktop，并把真实模型名注入模型选择器

### ☁️ 云同步与数据安全

- **WebDAV 同步** - 内置坚果云、Nextcloud、群晖 NAS 和自定义服务预设
- **S3 兼容同步** - AWS S3、MinIO、Cloudflare R2、阿里云 OSS、腾讯云 COS、华为云 OBS 和自定义端点
- **自动同步** - 每次数据库变更后自动上传数据库和 Skills 变更
- **SQL 导入/导出** - 使用 Codex Cube 备份文件迁移或恢复数据库
- **数据库备份** - 自动快照、手动备份、重命名/删除，以及带安全备份的恢复
- **原子写入** - 临时文件加重命名方式保护 Codex 在线配置不损坏
- **目录覆盖** - 自定义 Codex Cube 和 Codex 配置目录，支持 WSL 路径和云同步文件夹

### 🖥️ 平台

- 基于 Tauri 2 的 Windows、macOS、Linux 原生桌面应用
- 系统托盘、关闭到托盘、静默启动和开机自启
- 深色、浅色和跟随系统主题
- 国际化支持简体中文、繁体中文、英文和日文
- 应用内更新、便携模式、Codex CLI 安装/升级和环境诊断

## 🚀 快速开始

### 基本使用

1. **添加供应商** - 点击顶部的 `+` 按钮，选择官方预设、供应商预设、自定义配置或聚合 Provider。
2. **中转站一键导入** - 点击 `ccswitch://` 导入链接即可添加供应商，或把多个供应商聚合为一个 Provider。
3. **导入现有配置** - 如果已经配置过 Codex，使用“导入当前配置”保存为 `default` 供应商。
4. **切换供应商** - 点击供应商卡片上的“启用”，或在托盘菜单中选择供应商。
5. **生效方式** - 未开启本地路由接管时，重启 Codex 或终端；开启接管后切换即时生效。
6. **恢复官方登录** - 添加官方预设，或在“设置 > 认证”中管理 ChatGPT / xAI / Grok 登录。

### 使用本地路由

1. 打开“设置 > 路由”。
2. 启动本地路由服务。
3. 开启 Codex 接管。
4. 按需配置故障转移队列和熔断器参数。

### 使用项目

1. 打开顶部项目切换器。
2. 创建项目，保存当前 Provider、MCP、Skills、Prompts 和 memory 文件配置。
3. 切换项目即可应用对应的 Codex 环境。

> 注意：本地路由接管期间会阻止切换到官方供应商。使用本地路由访问官方 API 可能存在封号风险。

## 📥 下载与安装

从 [Releases](https://github.com/Cbdlll/Codex-Cube/releases) 页面下载最新安装包。

- **Windows** - `Codex-Cube-*-Windows-x64-setup.exe` 或 `Codex-Cube-*-Windows-Portable.zip`
- **macOS** - `Codex-Cube-*-macOS.dmg` 或 `Codex-Cube-*-macOS.zip`
- **Linux** - 使用下方命令从源码构建

### 系统要求

- **Windows** - Windows 10 及以上
- **macOS** - macOS 12 (Monterey) 及以上
- **Linux** - 支持 WebKitGTK 的主流桌面发行版

## ❓ 常见问题

<details>
<summary><strong>Codex Cube 支持哪些 AI 工具？</strong></summary>

Codex Cube 专注于 **Codex**（CLI 和 Desktop），不支持 Claude Code、Gemini CLI 等工具。

</details>

<details>
<summary><strong>切换供应商后需要重启 Codex 吗？</strong></summary>

直接切换时需要重启 Codex 或终端才能生效。开启 Codex 本地路由接管后，切换会热生效，无需重启。

</details>

<details>
<summary><strong>切换后插件或 Hook 不见了怎么办？</strong></summary>

请使用“通用配置片段”。编辑供应商，打开“编辑通用配置”，点击“从编辑内容提取”。创建或编辑供应商时保持“应用通用配置”开启，即可在不同供应商之间共享插件和 Hook。

</details>

<details>
<summary><strong>如何切回官方登录？</strong></summary>

添加官方预设并切换过去。也可以在“设置 > 认证”中管理 ChatGPT 和 xAI/Grok 账号。切换后按需执行 Codex 登录/OAuth 流程。

</details>

<details>
<summary><strong>为什么不能删除当前正在使用的供应商？</strong></summary>

Codex Cube 始终保留一个激活中的供应商，确保 Codex 有可用的配置。请先切换到其他供应商，再删除不需要的配置。

</details>

<details>
<summary><strong>我的数据存储在哪里？</strong></summary>

- **数据库** - `~/.codex-cube/codex-cube.db`（SQLite：供应商、MCP、Prompts、Skills、用量和项目）
- **备份** - `~/.codex-cube/backups/`
- **Skills** - `~/.codex-cube/skills/` 或 `~/.agents/skills/`，取决于存储设置
- **Codex 在线配置** - `~/.codex/auth.json` 和 `~/.codex/config.toml`

Codex Cube 目录和 Codex 目录都可以在“设置 > 高级 > 配置目录”中覆盖。

</details>

<details>
<summary><strong>本地路由可以和官方供应商一起使用吗？</strong></summary>

本地路由接管期间会阻止切换到官方供应商。使用本地路由访问官方 API 可能存在封号风险。

</details>

## 🏗️ 架构总览

<details>
<summary><strong>设计原则</strong></summary>

```text
--------------------------------------------
前端（React + TypeScript + Vite）
  Components / Hooks / TanStack Query
                |
                | Tauri IPC
                v
后端（Tauri + Rust）
  Commands -> Services -> DAO -> SQLite
  本地路由代理（Tokio + Hyper/Axum）
--------------------------------------------
```

- **单一事实源** - 供应商、MCP、Prompt、Skill、用量和项目数据保存在 SQLite 中
- **双向同步** - 切换时写入 Codex 在线文件；编辑当前供应商时从在线文件回填
- **原子写入** - 临时文件加重命名防止配置损坏
- **并发安全** - 互斥锁保护的数据库连接避免竞态
- **分层架构** - Commands、Services、DAO、Database 清晰分离

**核心后端服务**

- `ProviderService` - 供应商增删改查、切换、回填、排序和聚合引用
- `ProxyService` - 本地路由、接管、热切换、故障转移和整流
- `McpService`、`PromptService`、`SkillService` - 资源管理和 Codex 同步
- `ProfileService` - 项目快照与应用
- `UsageService` / `SessionManager` - 请求日志、用量统计和会话历史
- `WebDavSyncService` / `S3SyncService` - 跨设备同步
- `Tray` - 供应商快速切换和订阅额度状态

</details>

## 🛠️ 开发指南

### 环境要求

- Node.js 22+
- pnpm 10+
- Rust 1.85+（仓库在 `rust-toolchain.toml` 中固定为 1.95）
- Tauri CLI 2.8+

### 前端命令

```bash
# 安装依赖
pnpm install

# 开发模式（热重载）
pnpm dev

# 类型检查
pnpm typecheck

# 格式化与检查
pnpm format
pnpm format:check

# 前端单元测试
pnpm test:unit
pnpm test:unit:watch

# 构建桌面应用
pnpm build

# 构建调试版桌面应用
pnpm build:debug
```

### Rust 后端

```bash
cd src-tauri

cargo fmt
cargo clippy
cargo test

# 需要 test-hooks feature 的测试
cargo test --features test-hooks
```

### 测试栈

- **前端** - Vitest、React Testing Library，使用 MSW 模拟 Tauri API
- **后端** - `src-tauri/tests/` 下的 Rust 集成测试

## 📂 项目结构

```text
├── src/                        # React + TypeScript 前端
│   ├── components/             # 供应商、路由、用量、设置、代理
│   ├── config/                 # 供应商预设和应用常量
│   ├── hooks/                  # 前端业务逻辑
│   ├── i18n/                   # zh / zh-TW / en / ja 语言包
│   ├── lib/                    # Tauri API 封装和 React Query
│   └── utils/                  # Provider、聚合和配置工具
├── src-tauri/                  # Tauri + Rust 后端
│   ├── src/commands/           # Tauri 命令层
│   ├── src/services/           # 业务逻辑层
│   ├── src/database/           # SQLite schema、DAO 和备份
│   ├── src/proxy/              # 本地路由和故障转移
│   └── src/session_manager/    # 会话历史
├── tests/                      # 前端测试
├── src-tauri/tests/            # Rust 集成测试
└── release/                    # 本地构建产物
```

## 🤝 贡献

欢迎提交 Issue 和 Pull Request。提交 PR 前请确认：

- `pnpm typecheck` 通过
- `pnpm format:check` 通过
- `pnpm test:unit` 通过
- 在 `src-tauri` 下执行 `cargo test` 通过

## 🙏 致谢

本项目参考 [CC Switch](https://github.com/farion1231/cc-switch) 进行开发，并调整为专注 Codex 的管理工具。

## 📄 License

MIT License
