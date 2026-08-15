<div align="center">

<img src="src-tauri/icons/icon.png" width="96" alt="Codex Cube" />

# Codex Cube

### Aggregate multiple providers into one and manage Codex subscriptions

[![Windows](https://img.shields.io/badge/Windows-0078D6?style=flat-square&logo=windows&logoColor=white)](https://github.com/Cbdlll/Codex-Cube/releases)
[![macOS](https://img.shields.io/badge/macOS-000000?style=flat-square&logo=apple&logoColor=white)](https://github.com/Cbdlll/Codex-Cube/releases)
[![Linux](https://img.shields.io/badge/Linux-FCC624?style=flat-square&logo=linux&logoColor=black)](https://github.com/Cbdlll/Codex-Cube/releases)
[![Tauri](https://img.shields.io/badge/Tauri%202-24C8D8?style=flat-square&logo=tauri&logoColor=white)](https://tauri.app/)
[![React](https://img.shields.io/badge/React%2018-61DAFB?style=flat-square&logo=react&logoColor=black)](https://react.dev/)
[![TypeScript](https://img.shields.io/badge/TypeScript-3178C6?style=flat-square&logo=typescript&logoColor=white)](https://www.typescriptlang.org/)
[![Rust](https://img.shields.io/badge/Rust-000000?style=flat-square&logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![SQLite](https://img.shields.io/badge/SQLite-003B57?style=flat-square&logo=sqlite&logoColor=white)](https://www.sqlite.org/)
[![License](https://img.shields.io/badge/License-MIT-blue?style=flat-square)](LICENSE)

English | [简体中文](README_ZH.md)

</div>

## 💡 Why Codex Cube?

Codex users often keep multiple ChatGPT subscriptions, third-party API providers, coding-plan accounts, and local model endpoints. Switching between them normally means manually editing `~/.codex/auth.json` and `~/.codex/config.toml`, losing plugins or hooks, and restarting Codex repeatedly.

**Codex Cube** gives you a desktop app for managing all of those Codex configurations. Instead of editing JSON and TOML by hand, you can import the current config once, add providers from presets, and switch between official subscriptions and third-party providers with one click. It also **combines different suppliers into one aggregate provider**, supports **cc-switch compatible one-click import** (`ccswitch://` links from relay stations), and adds local routing for hot switching, failover, request repair, and usage tracking.

- 🧬 **Aggregate providers** - Combine official subscriptions, third-party APIs, coding-plan accounts, and relay stations into a single virtual Codex provider; all member models appear in one model picker
- 🔗 **cc-switch compatible one-click import** - Accept `ccswitch://` one-click import links from relay stations and 60+ built-in presets (official, aggregator, third-party, and Chinese providers)
- 🎯 **Codex-focused** - Manage official Codex subscriptions, managed OAuth accounts, and third-party API providers from one interface
- ✏️ **No more manual editing** - Import your existing config, use 60+ built-in provider presets (one-click relay import included), or create custom providers
- 🔄 **One-click switching** - Switch from the provider list or the system tray menu
- ⚡ **Local routing and failover** - Protocol conversion, hot switching, failover queue, circuit breaker, health checks, and request rectifier
- 📊 **Usage and cost tracking** - Official subscription quota, token plans, balance scripts, request logs, trend charts, and custom pricing
- 🧩 **MCP, Prompts, and Skills** - Keep these resources in the Codex Cube SSOT, sync them with Codex, import via deep links, and preserve them in project snapshots
- 📁 **Projects** - Save provider, MCP, Skills, Prompts, and memory-file snapshots and switch them from the header
- ☁️ **Cloud sync** - WebDAV and S3-compatible sync across devices, with optional auto-sync
- 🖥️ **Cross-platform** - Native desktop app built with Tauri 2 for Windows, macOS, and Linux

## ✨ Features

### 🗂️ Provider and subscription management

- **Multiple account types** - Official Codex/ChatGPT accounts, managed ChatGPT and xAI/Grok OAuth, third-party API providers, coding-plan providers, and custom endpoints
- **60+ presets** - Built-in official, aggregator, third-party, and Chinese provider presets (one-click relay import included); fill in an API key and save
- **Import current config** - Turn an existing Codex setup into a `default` provider on first launch
- **Common Config Snippet** - Share plugins, hooks, environment variables, and other non-sensitive settings between providers
- **Provider list** - One-click switch, drag-and-drop sorting, duplicate, search, notes, icons, and website links
- **Connectivity checks** - Test provider reachability without sending real model requests
- **Usage query per provider** - Configure official subscription, token plan, balance, GitHub Copilot, or custom scripts
- **System tray** - Switch providers and view subscription quota summaries without opening the app

### 🪢 Aggregate provider

- Combine different suppliers (official subscriptions, third-party APIs, coding plans, relay stations) into one virtual provider
- Fetch model lists from each member, edit display names, context windows, and capability metadata
- Show all member models in the Codex model picker without switching providers
- Switching to an aggregate provider automatically prepares local routing, with no Codex restart required

### 🔗 cc-switch compatible one-click import

- Compatible with `ccswitch://` one-click import links generated by relay stations - click a link and the provider is imported and ready to use
- `codexcube://` deep links also import providers, MCP servers, prompts, and skills
- 60+ built-in provider presets cover official, aggregator, third-party, and Chinese relay-station providers

### ⚡ Local routing, failover, and reliability

- **Local routing service** - Configurable listen address and port, with Codex takeover
- **Hot switching** - With takeover active, switch providers from the list or tray without restarting Codex
- **Protocol support** - Convert between OpenAI Responses, OpenAI Chat Completions, and Anthropic Messages formats where needed
- **Auto failover** - Priority queue from P1 onward, retries, timeout controls, and provider health state
- **Circuit breaker** - Consecutive-failure threshold, error-rate threshold, recovery wait, and half-open recovery
- **Request rectifier** - Fixes thinking-signature and thinking-budget errors, falls back when images are unsupported, and preflights known text-only models
- **Global outbound proxy** - Route all Codex Cube traffic, including API and Skills downloads, through an HTTP proxy
- **Request overrides** - Custom User-Agent and per-provider local proxy request overrides

### 📊 Usage and cost

- **Usage dashboard** - Date ranges, provider/model filters, trend charts, success rates, latency, tokens, and cost
- **Request logs** - Search detailed request history with provider, model, status, tokens, cost, and session details
- **Custom pricing** - Per-million input/output/cache pricing, cost multipliers, and pricing model source
- **models.dev auto-sync** - Import model pricing from models.dev with selected providers and common models
- **Session usage** - Sync and rebuild Codex usage from local session logs

### 🧩 MCP, Prompts, and Skills

- **MCP servers** - Presets, wizard-based stdio/HTTP/SSE configuration, JSON/TOML input, and import from existing Codex config
- **Prompts** - Markdown editing and enable/disable management for Codex prompt files
- **Skills** - Install from GitHub repositories or ZIP files, search skills.sh, manage repositories, and import existing skills
- **Sync options** - Store master copies under `~/.codex-cube/skills/` or `~/.agents/skills/`, with symlink or copy sync
- **Backups** - Automatic backups before uninstall or changes, plus restore flows
- **Deep links** - Import providers, MCP servers, prompts, and skills through `codexcube://`, with `ccswitch://` relay-station one-click import links supported

### 📁 Projects, sessions, and collaboration

- **Projects** - Snapshot provider, MCP, Skills, Prompts, and memory-file configuration; switch projects from the header
- **Session manager** - Browse and search Codex session history, view transcripts, copy or resume commands, and delete sessions
- **Unified history** - Optionally merge official and third-party Codex sessions into one history list, with backup and restore
- **Collaborative workflow** - Register cheap models as Codex subagents, choose worker agents, and install the `@cube-dispatch` Workflow Skill
- **Codex Desktop model picker** - Launch Codex Desktop with debug mode and inject real model names into the model picker

### ☁️ Cloud sync and data safety

- **WebDAV sync** - Presets for Jianguoyun, Nextcloud, Synology NAS, and custom servers
- **S3-compatible sync** - AWS S3, MinIO, Cloudflare R2, Alibaba OSS, Tencent COS, Huawei OBS, and custom endpoints
- **Auto sync** - Upload database and skill changes automatically after each change
- **SQL import/export** - Migrate or restore the database from Codex Cube backup files
- **Database backups** - Automatic snapshots, manual backups, rename/delete, and safe restore with safety backup
- **Atomic writes** - Temp-file plus rename writes protect live Codex configs from corruption
- **Directory overrides** - Customize Codex Cube and Codex config directories, including WSL paths and cloud-sync folders

### 🖥️ Platform

- Windows, macOS, and Linux desktop app built with Tauri 2
- System tray, minimize to tray, silent startup, and auto-launch
- Dark, light, and system themes
- i18n for Simplified Chinese, Traditional Chinese, English, and Japanese
- In-app updater, portable mode, Codex CLI install/update, and environment diagnostics

## 🚀 Quick Start

### Basic usage

1. **Add a provider** - Click the `+` button in the header and choose an Official preset, provider preset, custom provider, or aggregate provider.
2. **One-click import from a relay station** - Click a `ccswitch://` import link to add a provider instantly, or build your own aggregate provider from multiple suppliers.
3. **Import existing config** - If Codex is already configured, use "Import Current Config" to save it as a `default` provider.
4. **Switch provider** - Click "Enable" on a provider card, or choose a provider from the tray menu.
5. **Apply changes** - Without local routing takeover, restart Codex or the terminal. With takeover active, switching is hot and no restart is needed.
6. **Return to official login** - Add the Official preset, or manage ChatGPT / xAI / Grok login in Settings > Auth.

### Use local routing

1. Open Settings > Routing.
2. Start the local routing service.
3. Enable the Codex takeover switch.
4. Configure the failover queue and circuit breaker if needed.

### Use projects

1. Open the project switcher in the header.
2. Create a project to snapshot the current provider, MCP, Skills, Prompts, and memory-file configuration.
3. Switch projects to apply the saved Codex environment.

> Note: Official providers are blocked while local routing takeover is active. Using routing with official APIs may carry account-ban risk.

## 📥 Download and Installation

Download the latest installer from the [Releases](https://github.com/Cbdlll/Codex-Cube/releases) page.

- **Windows** - `Codex-Cube-*-Windows-x64-setup.exe` or `Codex-Cube-*-Windows-Portable.zip`
- **macOS** - `Codex-Cube-*-macOS.dmg` or `Codex-Cube-*-macOS.zip`
- **Linux** - Build from source with the commands below

### System requirements

- **Windows** - Windows 10 or later
- **macOS** - macOS 12 (Monterey) or later
- **Linux** - Mainstream desktop distributions with WebKitGTK support

## ❓ FAQ

<details>
<summary><strong>Which AI tools does Codex Cube support?</strong></summary>

Codex Cube focuses on **Codex** (CLI and Desktop). It does not manage Claude Code, Gemini CLI, or other agent tools.

</details>

<details>
<summary><strong>Do I need to restart Codex after switching providers?</strong></summary>

For direct provider switching, restart Codex or the terminal to apply changes. When Codex local routing takeover is active, switching is hot and no restart is required.

</details>

<details>
<summary><strong>Why did my plugins or hooks disappear after switching?</strong></summary>

Use the "Common Config Snippet" feature. Edit a provider, open "Edit Common Config", and click "Extract from Editor". When creating or editing providers, keep "Apply Common Config" enabled so plugins and hooks are shared.

</details>

<details>
<summary><strong>How do I switch back to official login?</strong></summary>

Add the Official preset and switch to it. You can also use Settings > Auth to manage ChatGPT and xAI/Grok accounts. After switching, run the Codex login/OAuth flow if needed.

</details>

<details>
<summary><strong>Why can't I delete the currently active provider?</strong></summary>

Codex Cube always keeps one active provider so Codex still has a usable configuration. Switch to another provider first, then delete the one you no longer need.

</details>

<details>
<summary><strong>Where is my data stored?</strong></summary>

- **Database** - `~/.codex-cube/codex-cube.db` (SQLite: providers, MCP, prompts, skills, usage, and profiles)
- **Backups** - `~/.codex-cube/backups/`
- **Skills** - `~/.codex-cube/skills/` or `~/.agents/skills/` depending on storage settings
- **Live Codex config** - `~/.codex/auth.json` and `~/.codex/config.toml`

Both the Codex Cube directory and the Codex directory can be overridden in Settings > Advanced > Configuration Directory.

</details>

<details>
<summary><strong>Can I use local routing with official providers?</strong></summary>

The app blocks switching to official providers while local routing takeover is active. Routing official APIs through a local proxy can carry account-ban risk.

</details>

## 🏗️ Architecture Overview

<details>
<summary><strong>Design principles</strong></summary>

```
--------------------------------------------
Frontend (React + TypeScript + Vite)
  Components / Hooks / TanStack Query
                |
                | Tauri IPC
                v
Backend (Tauri + Rust)
  Commands -> Services -> DAO -> SQLite
  Local routing proxy (Tokio + Hyper/Axum)
--------------------------------------------
```

- **Single source of truth** - Provider, MCP, Prompt, Skill, usage, and profile data live in the SQLite database
- **Two-way sync** - Switching writes live Codex files; editing the active provider can backfill from live files
- **Atomic writes** - Temp-file plus rename prevents config corruption
- **Concurrency-safe** - Mutex-protected database connections avoid races
- **Layered architecture** - Clear separation between commands, services, DAOs, and database

**Key backend services**

- `ProviderService` - provider CRUD, switching, backfill, sorting, aggregate references
- `ProxyService` - local routing, takeover, hot switching, failover, rectifier
- `McpService`, `PromptService`, `SkillService` - resource management and Codex sync
- `ProfileService` - project snapshots and apply
- `UsageService` / `SessionManager` - request logs, usage stats, session history
- `WebDavSyncService` / `S3SyncService` - cross-device sync
- `Tray` - provider quick switch and subscription quota status

</details>

## 🛠️ Development

### Environment

- Node.js 22+
- pnpm 10+
- Rust 1.85+ (the repo pins Rust 1.95 in `rust-toolchain.toml`)
- Tauri CLI 2.8+

### Frontend commands

```bash
# Install dependencies
pnpm install

# Development mode with hot reload
pnpm dev

# Type check
pnpm typecheck

# Format and check
pnpm format
pnpm format:check

# Frontend unit tests
pnpm test:unit
pnpm test:unit:watch

# Build the desktop app
pnpm build

# Build a debug desktop app
pnpm build:debug
```

### Rust backend

```bash
cd src-tauri

cargo fmt
cargo clippy
cargo test

# Tests that require the test-hooks feature
cargo test --features test-hooks
```

### Testing stack

- **Frontend** - Vitest, React Testing Library, and MSW for mocking Tauri APIs
- **Backend** - Rust integration tests under `src-tauri/tests/`

## 📂 Project Structure

```text
├── src/                        # React + TypeScript frontend
│   ├── components/             # Providers, proxy, usage, settings, agents
│   ├── config/                 # Provider presets and app constants
│   ├── hooks/                  # Frontend business logic
│   ├── i18n/                   # zh / zh-TW / en / ja locales
│   ├── lib/                    # Tauri API wrappers and React Query
│   └── utils/                  # Provider, aggregate, and config helpers
├── src-tauri/                  # Tauri + Rust backend
│   ├── src/commands/           # Tauri command layer
│   ├── src/services/           # Business logic
│   ├── src/database/           # SQLite schema, DAO, and backups
│   ├── src/proxy/              # Local routing and failover
│   └── src/session_manager/    # Session history
├── tests/                      # Frontend tests
├── src-tauri/tests/            # Rust integration tests
└── release/                    # Local build artifacts
```

## 🤝 Contributing

Feel free to open issues and pull requests. Before submitting a PR, make sure:

- `pnpm typecheck` passes
- `pnpm format:check` passes
- `pnpm test:unit` passes
- Rust tests pass with `cargo test` in `src-tauri`

## 🙏 Acknowledgments

This project is developed with reference to [CC Switch](https://github.com/farion1231/cc-switch) and adapted as a Codex-focused manager.

## 📄 License

MIT License
