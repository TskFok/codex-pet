# Codex Pet

macOS / Windows 双平台独立桌宠（基于 Tauri 2、Rust 与 TypeScript/Canvas），用于接收 Codex、DeepSeek Harness 与 Antigravity 的任务提醒并打开对应原会话或来源应用。首版聚焦提醒与会话跳转，不提供任务回答、批准或执行控制；现有 `pet-output/` 原始素材保持只读不被覆盖。

## 开发环境

- Rust 1.93.1（`rust-toolchain.toml`）
- Node 24.x LTS（`.node-version` 锁定 `24.11.1`，要求 `>=24.9.0 <25`）
- pnpm 10.28.2（根 `package.json`）

从仓库根目录执行常规检查：

```sh
pnpm install --frozen-lockfile
pnpm test
pnpm typecheck
pnpm build
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

## 构建与打包（未签名测试包）

正式签名、公证、应用商店上架与自动更新服务不在首版范围；打包产物明确标为**未签名个人测试包**。构建前会自动准备目标架构的 `pet-bridge` 转发器、离线 `harness-bundle` 模板与默认月影资源（只复制到构建目录，不改动 `pet-output/`）：

```sh
# macOS Apple Silicon (.app 与 .dmg)
pnpm --filter @codex-pet/desktop tauri build --target aarch64-apple-darwin --bundles app,dmg

# macOS Intel (.app 与 .dmg)
pnpm --filter @codex-pet/desktop tauri build --target x86_64-apple-darwin --bundles app,dmg

# Windows x64 每用户 NSIS 安装包（需在 Windows 原生环境运行）
pnpm --filter @codex-pet/desktop tauri build --target x86_64-pc-windows-msvc --bundles nsis
```

校验发行产物与真实验收报告完整性（缺少任一目标架构、转发器、兼容报告或真实双平台 GUI 验收时必须退出非零，不将构建成功视为发行通过）：

```sh
pnpm validate:release --artifacts <产物目录> --reports <报告目录>
```

## 发布

```sh
pnpm release            # 默认递增 patch，例如 0.1.0 → 0.1.1
pnpm release 0.2.0      # 发布指定的更高稳定版本（同样支持 pnpm release -- 0.2.0）
pnpm release --current  # 强制重打并推送当前版本标签（同样支持 pnpm release -- --current）
pnpm run test:release   # 运行发布脚本单元测试
```

发布前必须保持工作区干净，且当前分支与 `origin` 上的同名分支完全同步。`release` 会校验所有版本源一致性，依次运行发布脚本测试、前端/脚本测试、类型检查、构建以及 Rust `fmt` / `clippy` / `test`；全部通过后同步更新 `package.json`、`apps/desktop/package.json`、`integrations/deepseek-harness/package.json`、`apps/desktop/src-tauri/tauri.conf.json`、`apps/desktop/src-tauri/Cargo.toml`、`crates/pet-core/Cargo.toml`、`crates/pet-bridge/Cargo.toml` 和 `Cargo.lock`，以中文提交版本变更并推送当前分支与 `vX.Y.Z` 标签。`--current` 保持版本号不变，强制将当前版本的本地和远程标签指向当前提交，适用于重新触发发布。

## 目录与模块

- `crates/pet-core`：统一事件协议（生产协议 v2/v3，兼容读取旧 v1/v2）、可重放多会话状态机（`SessionStore`）、持久化日志与去重存储（`JournalStore`）、旧 v1/v2 宠物包导入与独立动画格式（`pet_package`）。无 GUI 或宿主依赖。
- `crates/pet-bridge`：原生转发器与安全安装器（`codex` / `antigravity` / `harness` / `hooks` / `antigravity-hooks` 子命令），负责 8 MiB 原始 Hook 归一化、64 KiB 统一事件校验、有界待发送区（2,000 条 / 10 MiB / 24 小时）与 200 ms 本机 HTTP 投递。
- `apps/desktop`：Tauri 2 桌面应用与 TypeScript/Canvas 前端，包含本机 loopback broker、透明窗口与逐帧命中区域、系统托盘、通知历史面板、设置与来源接入状态、原会话安全导航。
- `integrations/codex`：Codex 分级生命周期 Hook 模板、能力兼容表与接入/撤销文档（[Codex 接入说明](integrations/codex/README.md) · [Codex 兼容表](integrations/codex/compatibility.json)）。
- `integrations/antigravity`：Antigravity（2.0 桌面 / IDE / CLI）分级生命周期 Hook 模板、能力兼容表与接入/撤销文档（[Antigravity 接入说明](integrations/antigravity/README.md) · [Antigravity 兼容表](integrations/antigravity/compatibility.json)）。
- `integrations/deepseek-harness`：DeepSeek Harness 完整待处理与精确解除插件、离线 `dsh.bundle` 安装包生成器与最小宿主补丁（[Harness 接入说明](integrations/deepseek-harness/README.md) · [Harness 兼容表](integrations/deepseek-harness/compatibility.json)）。
- `scripts/desktop-pet`：发行资源准备脚本（`prepare-resources.mjs`）、发行验收守卫（`validate-release.mjs`）及行为测试。
- `docs/validation/desktop-pet`：双平台窗口与性能实测（[platforms.md](docs/validation/desktop-pet/platforms.md)）、来源验证（[codex.md](docs/validation/desktop-pet/codex.md)、[harness.md](docs/validation/desktop-pet/harness.md)）及发行检查清单（[release-checklist.md](docs/validation/desktop-pet/release-checklist.md)）。

## 核心协议与状态规则

- 跨进程输入先经 `protocol::decode_event` 校验：JSON 不超过 64 KiB，摘要不超过 512 个 Unicode 字符；字段使用 `camelCase`，枚举值使用 `snake_case`，事件种类位于 `kind.type`，未设置的可选字段省略。接收时间由接收端填入。
- `SessionStore` 按 `(source, installationId, sessionId)` 隔离会话状态，按 `eventId` 独立去重（去重记录独立于 1,000 条 / 7 天的可见历史保留至少 24 小时）。
- 持续请求使用 `RequestLifetime::UntilResolved`（JSON `until_resolved`），保留原始出生回合，跨回合结束、新回合和普通快照保留，仅由原会话/原请求精确解除；普通 `Turn` 请求保持原回合边界语义。
- `Stop` 等未知终态映射为 `unknown` 而非“任务成功”；标为已读与清空历史均不解除仍待处理的请求，也不删除宠物素材或去重索引。

## 从通知打开会话

点击桌宠气泡的「打开会话」，或在托盘「通知」中展开会话并点击「尝试打开会话」。桌宠会重新核对本机登记的来源安装与系统协议注册，再通过操作系统原生入口发送受限固定深链（绝不执行事件中的任意命令或 URL）：

- **「已发送打开请求」**（`Requested`）仅表示操作系统接受了深链请求，不宣称来源界面已完成定位。
- **「打开应用」**（`ApplicationOnly`）在宿主版本不支持会话深链时显式提示，需用户再次点击才唤起应用。
- 失败或无桌面入口时保留通知并提供「复制会话 ID」按钮。
- Codex 使用 `codex://threads/<thread-id>`（共用 Hooks 的旧版通知可能来自 CLI，桌面端未必存在对应会话）；Harness 需要桌面 profile 已选用配套 `dsh.bundle` 且系统注册的 `0.2.2-rc.1` 桌面包包含会话深链补丁。

## 兼容范围与当前验收状态

- **Codex（分级支持）**：提供可验证生命周期提醒（`UserPromptSubmit`、`Stop`、`Interrupt`、`SubagentStart`、`SubagentStop`）与会话跳转；**不支持**精确审批/提问等待与解除同步。
- **Antigravity（分级支持）**：通过全局 `~/.gemini/config/hooks.json` 订阅 `PreInvocation` 与 `Stop`，提供回合开始、精确回合结束/中断/失败提醒与项目名识别；**不支持**精确审批/提问等待同步，导航降级为打开应用 + 复制 `conversationId`。
- **DeepSeek Harness（完整待处理目标）**：支持审批、直接提问与跨回合定时提问的精确解除、观察器重载恢复与 JSONL 崩溃代次隔离（后两项需配套宿主补丁）。
- 完整架构设计见 [双平台桌宠设计](docs/superpowers/specs/2026-11-11-tauri-desktop-pet-design.md)，当前已验证项与剩余发行阻断项（含 Windows 真机全链路、macOS Intel、双平台 3 真实宿主会话跳转及 5 分钟正式资源实测）详见 [发行检查清单](docs/validation/desktop-pet/release-checklist.md)。
