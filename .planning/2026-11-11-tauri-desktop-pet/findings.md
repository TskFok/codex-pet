# 设计依据

本文件记录核查结果及资料，不包含执行指令。

## 用户确认

- 独立桌面宠物，改善原有宠物播放节奏。
- 同时支持 macOS、Windows。
- 第一版只接收提醒、跳转原会话。
- 采用 Tauri。

## 已核查事实

- 当前仓库主要保存宠物图集与制作记录，没有现成应用工程，当前分支 main。
- Codex 本机独立 CLI 为 0.145.0，桌面内置 CLI 为 0.167.0-alpha.3.2；两者 features list 均显示 hooks stable true。尚未对真实任务触发链进行验证。
- 官方 Codex Hooks 提供 UserPromptSubmit、PermissionRequest、Stop、Interrupt 等事件；部分专用工具可能跳过通用工具 Hook，不能预先保证所有提问事件均被覆盖。
- Codex App Server 有 turn/completed 等结构化事件，但新建服务实例不能据此自动观察另一个正在运行的桌面实例。
- 官方 Codex 会话链接为 codex://threads/<thread-id>。
- 本机官方 DeepSeek Harness 仓库提交为 fcc0ad4621，版本 0.2.2-rc.1；有 session/event、agent/status、approval/asked、approval/decided、user-questions/request 等接口。
- Harness 当前 apps/desktop/src/main.ts 的外部 dsh://open 入口只唤起主窗口，没有接收目标会话 ID。
- 本仓库 deepseek-big-fish 是 Codex v2 图集，1536×2288，每格 192×208，8 列 11 行。旧 v1 为 1536×1872，9 行。
- 已核查本机 Codex 动画组件：待机 6 帧约 6.6 秒，其余状态主要每秒换帧 6～8 次。这些是代码设定，非运行性能实测。
- Tauri 在 Windows 使用 WebView2，在 macOS 使用 WKWebView；透明窗口与后台节流必须按实际依赖版本及平台验证。

## 来源

- https://learn.chatgpt.com/docs/hooks
- https://learn.chatgpt.com/docs/app-server
- https://learn.chatgpt.com/docs/reference/commands
- https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/core/agent/README.md
- https://v2.tauri.app/reference/webview-versions/
- https://v2.tauri.app/distribute/windows-installer/
- https://tauri.app/reference/config/
- 本机 /Volumes/CaseSensitive/workspace/myself/deepseek-harness/packages/core/session/src/index.ts
- 本机 /Volumes/CaseSensitive/workspace/myself/deepseek-harness/packages/interaction/user-questions/src/types.ts
- 本机 /Volumes/CaseSensitive/workspace/myself/deepseek-harness/apps/desktop/src/main.ts

## 状态

资料和源码核查已完成；跨平台运行、真实通知接入和性能均待实现阶段验证。

## 独立审阅

2026-11-11，review_pet_design 的实施者检查结论为“没有重要遗留问题”。作者另外明确了去重索引与可见历史分别保留、补发批次归并后再提示，避免离线历史请求产生过期气泡。

## 实现计划准备

- 本机已安装 Rust/Cargo 1.93.1、Node 25.5.0、pnpm 10.28.2；实现时锁定受支持的 Node LTS 与依赖版本，不依赖本机非 LTS 版本行为。
- Harness 的远程事件来自显式注册的事件源，不能假定任意自定义 ctx.emit 都会到达桌面 Client；精确跳转必须先验证 Host/Client 注册路径，失败时维护明确的最小宿主补丁。
- 实现计划只描述未来工作，本阶段不运行依赖安装、脚手架、来源配置变更或接入实验。
- Node 官方 Release 表确认 24.x 仍处于 Maintenance LTS，计划选 24.x 与 Harness 的 24.9+ 要求对齐。
- 计划将宿主原始 Hook 输入（8 MiB）和归一化事件（64 KiB）分开限额，避免长提示词造成状态提醒丢失；已读、清空、revision、未解决请求与因果标记都要独立于可见历史持久化。
