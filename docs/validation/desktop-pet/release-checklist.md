# 双平台桌宠发行验收清单

更新日期：2026-11-24
当前状态：**实现、打包流水线、自动化门禁与 macOS arm64 本地演练已完成；最终公开发行门保持未通过（存在 Windows 真机、macOS Intel、5 分钟正式资源与真实宿主三会话等外部待验阻断项）**。

> 严格区分“代码/构建通过”与“真实平台发行通过”：`scripts/desktop-pet/validate-release.mjs` 在缺少任一目标架构产物、转发器、兼容报告或真实双平台 GUI 验收报告时必须退出非零。

---

## 1. 自动化质量门禁（2026-11-24 实测通过）

| 检查项 | 命令 | 结果 |
| --- | --- | --- |
| Rust 格式检查 | `cargo fmt --all -- --check` | ✅ 通过 |
| Rust 严格静态检查 | `cargo clippy --workspace --all-targets --locked -- -D warnings` | ✅ 通过 |
| Rust 全工作区测试 | `cargo test --workspace --locked` | ✅ 通过（`pet-core`、`pet-bridge`、`pet-desktop` 全部通过） |
| 前端与发行脚本测试 | `pnpm test` | ✅ 通过（桌面 13 套 32 项、Harness 9 套 24 项通过 / 6 项宿主环境条件跳过、发行脚本行为测试 3 项通过） |
| TypeScript 类型检查 | `pnpm typecheck` | ✅ 通过 |
| 前端与适配器构建 | `pnpm build` | ✅ 通过 |
| 发行守卫负例校验 | `node scripts/desktop-pet/validate-release.mjs --artifacts tests/fixtures/release/incomplete --reports tests/fixtures/release/incomplete-reports` | ✅ 退出 1，准确列出 32 项缺口 |

---

## 2. 本机正式构建产物（macOS Apple Silicon，未签名测试包）

构建环境：macOS 27.0.1（26A434）、Apple Silicon（`aarch64-apple-darwin`）、Rust 1.93.1、Node v24.13.0、pnpm 10.28.2、Tauri 2.12.4。
构建命令：`pnpm --filter @codex-pet/desktop tauri build --target aarch64-apple-darwin --bundles app,dmg`

| 产物 / 随包资源 | 路径 / 检查结果 | 状态 |
| --- | --- | --- |
| macOS `.app` 应用包 | `target/aarch64-apple-darwin/release/bundle/macos/Codex Pet.app`（19.52 MiB，未签名测试包） | ✅ 已构建 |
| macOS `.dmg` 镜像包 | `target/aarch64-apple-darwin/release/bundle/dmg/Codex Pet_0.1.0_aarch64.dmg`（11.39 MiB，未签名测试包） | ✅ 已构建 |
| 随包转发器 `pet-bridge` | `Codex Pet.app/Contents/MacOS/pet-bridge`（Mach-O 64-bit `arm64`，与主程序同目录） | ✅ 已核验 |
| 随包 Harness 离线模板 | `Codex Pet.app/Contents/Resources/harness-bundle/`（含 `dist/`、`cordis.patch.json`、`package.json`、`compatibility.json`） | ✅ 已核验 |
| 随包默认月影素材 | `Codex Pet.app/Contents/Resources/default-pet/{pet.json,spritesheet.webp}` | ✅ SHA-256 与源一致 |
| 源 `pet-output/` 不可变性 | `yueying-cape-dancer/package/pet.json` (`353b9fe1...`) 与 `spritesheet.webp` (`70a06d18...`) | ✅ 构建前后逐字节一致 |
| macOS Intel (`x86_64-apple-darwin`) | `.app`、`.dmg`、`pet-bridge`（Mach-O `x86_64`） | ⬜ 待 Intel 机 / CI 构建与验收 |
| Windows x64 (`x86_64-pc-windows-msvc`) | 每用户 NSIS 安装包、WebView2 引导、`pet-bridge.exe`（PE `x86_64`） | ⬜ 待 Windows 真机 / CI 构建与验收 |

---

## 3. macOS arm64 正式包隔离演练与性能测量（2026-11-24）

原始证据保存于本地 `.superpowers/sdd/2026-11-11-tauri-desktop-pet/task-12-evidence/macos-arm64-drills.json`（全程使用独立隔离临时 `HOME`，未改动真实用户配置或宿主设置）。

| 演练 / 指标 | 实测结果 | 目标 / 结论 |
| --- | --- | --- |
| 正常通知端到端延迟（正式 `Codex Pet.app` + 随包 `pet-bridge`，30 样本） | **P50 = 28.73 ms，P95 = 41.77 ms，最大 = 42.29 ms**；日志未泄露原始提示词 | ✅ 满足 `P95 < 500 ms` |
| 关闭桌宠离线入队与重启补发 | 关闭期间事件耐久落入 `outbox/q-*.json`，重启后自动补发至 `journal.jsonl` 并清空队列 | ✅ 通过 |
| 磁盘不可写故障演练 | 只读用户目录下 `pet-bridge codex` 仍输出 `{}` 且退出码 0，stderr 记录脱敏诊断不泄露提示词 | ✅ 通过 |
| 来源协议升级失配演练 | `pet-bridge harness` 收到未知 `schemaVersion: 99` 返回永久协议拒绝退出码 `2` | ✅ 通过 |
| 中文与空格路径安装、升级与卸载 | 路径含中文空格（`安装 路径 甲` → `升级 路径 乙`）安装、升级保留 UUID、卸载完整保留第三方 Hook | ✅ 通过 |
| 历史压缩保留去重 & ACK 丢失重试 | Rust 耐久与 broker 集成测试覆盖（`compaction_keeps_dedup_for_24_hours`、`response_lost_then_retry_is_deduplicated_across_restart_and_watch_scan`） | ✅ 通过 |
| 未聚焦 60 秒动画帧间隔（macOS arm64 实测，见 [platforms.md](platforms.md)） | 30 FPS 档 1,800 间隔 **P95 = 35 ms**（100% ≤ 50 ms）；隐藏 60 秒 **0 次绘制** | ✅ 满足 `≥95% ≤ 50 ms` |
| 发行校验守卫诚实性验证 | 对真实 macOS arm64 产物运行 `validate-release.mjs`：arm64 自身 0 错误，仍因缺 x64/Windows/GUI 报告退出 1 | ✅ 门禁有效 |

---

## 4. 来源兼容矩阵摘要

详见 [integrations/codex/compatibility.json](../../../integrations/codex/compatibility.json)、[integrations/deepseek-harness/compatibility.json](../../../integrations/deepseek-harness/compatibility.json)、[codex.md](codex.md) 与 [harness.md](harness.md)。

| 来源 | 支持范围 | 目标宿主版本 | 当前状态 |
| --- | --- | --- | --- |
| Codex | 分级支持：生命周期提醒（`UserPromptSubmit` / `Stop` / `Interrupt` / `SubagentStart` / `SubagentStop`）与 `codex://threads/<id>` 跳转；**明确不支持**精确审批/提问等待与解除 | 独立 CLI `0.145.0`；桌面内置 `0.169.0-alpha.11.1` | 代码与单测完成；仅 `0.145.0` 隔离 CLI 验证过开始 Hook；双平台 3 真实会话跳转待验（0/3） |
| DeepSeek Harness | 完整待处理目标：审批、直接提问、跨回合定时提问精确解除、观察器重载恢复、JSONL 崩溃代次隔离及 `dsh://session/<id>` 跳转（后三项需配套宿主补丁） | `0.2.2-rc.1`（基线 `fcc0ad4621`，补丁目标 `c507c32525`） | SDK→bridge→SessionStore、临时 profile 激活与崩溃代次演练通过；原应用插件页与双平台 3 真实会话跳转待验（0/3） |

---

## 5. 剩余发行阻断项（外部真机与真实宿主验收）

以下项目在当前环境尚无对应硬件或未授权修改真实宿主配置，**不得标为通过**，必须逐项补齐并通过 `pnpm validate:release` 后方可正式发布：

- [ ] **Windows 10 22H2 / Windows 11 x64 原生全链路验收**
  - [ ] 全新 Windows 用户目录每用户 NSIS 安装与卸载（中文与空格安装路径）
  - [ ] WebView2 缺失与下载失败时的清晰提示验证
  - [ ] 升级后旧 Hook 路径迁移，卸载时保留第三方 Hook 与用户配置
  - [ ] `SetWindowRgn` 透明命中/穿透、拖动、`SWP_NOACTIVATE` 不抢焦点、托盘菜单与 `pet-bridge.exe` 无控制台闪烁
  - [ ] Windows 正常通知延迟 `P95 < 500 ms`、未聚焦 60 秒动画 `P95 ≤ 50 ms`、静止/动画/隐藏各 5 分钟 CPU/RSS 实测
- [ ] **macOS 补充架构与硬件场景验收**
  - [ ] macOS Intel (`x86_64-apple-darwin`) 原生构建、透明命中与性能验收，以及 macOS 13 实测
  - [ ] 当前修复后架构下 macOS 静止、动画、隐藏三模式各 5 分钟正式 CPU/RSS（含严格归属的 WebView 子进程）复测
  - [ ] 真实硬件休眠/唤醒、锁屏、混合 DPI 跨屏物理拖动、屏幕热拔插、独占全屏避让、登录开机自启与原生文件选择器导入宠物包
- [ ] **真实宿主端到端会话矩阵（双平台）**
  - [ ] Codex：macOS 与 Windows 各 ≥3 个真实桌面会话（运行中 / 结束 / 中断）验证生命周期 Hook 触发与气泡/历史点击精确定位（冷启动 / 热启动 / 隐藏）
  - [ ] DeepSeek Harness：在原桌面插件管理页完成 bundle 安装/启用/撤销；macOS 与 Windows 各 ≥3 个真实会话验证原应用回答/撤回审批解除提醒，以及气泡/历史点击冷热隐藏精确定位
