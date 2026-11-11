# Codex Pet

macOS / Windows 独立桌宠工程。当前交付事件协议与可重放会话状态核心；桌面界面、来源适配器、文件持久化及平台验收将由后续任务实现。现有 `pet-output/` 素材保留原样。

## 开发环境

- Rust 1.93.1（`rust-toolchain.toml`）。
- Node 24.11.1（`.node-version`，要求 24.x 且至少 24.9）。
- pnpm 10.28.2（根 `package.json`）。

从仓库根目录执行：

```sh
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

`pnpm install --frozen-lockfile` 使用已提交的锁文件。根 `pnpm test`、`pnpm typecheck`、`pnpm build` 将执行后续 `@codex-pet/desktop` 与 `@codex-pet/deepseek-integration` 包的对应命令；当前尚无这些包，命令会明确报无匹配项目，不将空工作区记为前端验证通过。后续前端测试使用 `vitest run`，空测试集必须失败。

## 核心入口

`crates/pet-core` 不依赖 GUI 或宿主。跨进程输入先经过 `protocol::decode_event`：协议版本固定为 1，JSON 不超过 64 KiB，摘要不超过 512 个 Unicode 字符。JSON 字段使用 camelCase，枚举值使用 snake_case，事件种类位于 `kind.type`；未设置的可选字段序列化时省略。接收时间由接收端填入。

`SessionStore` 提供 `apply`、`mark_read`、`clear_history`、`snapshot` 与 `restore`。`SessionKey` 使用来源、安装 ID、会话 ID 隔离状态；实例 ID 标识来源进程生命周期。`ApplyResult.changed` 只表示公开快照变化；接收事件的持久化与去重不能以此作为是否接收的判断。`should_notify` 仅表示本次实时新增的重要提醒，恢复批次后应统一展示最终快照中的未解决请求与未读结束提醒。

状态遵循以下规则：

- 按 `eventId` 去重；请求 tombstone 按会话、回合与请求 ID 匹配，撤回或已回答的请求不因倒序投递复活。
- 序号仅在同一来源、安装、实例、会话内可比。已知旧回合被忽略；回合 ID 不同且没有可比较序号时，保留现存请求并标为 `unknown`，时间戳不决定回合先后。
- 同一回合终态阻止迟到的开始或请求。`unknown` 结束也会形成终态，不推断业务目标成功。
- 标为已读与解决请求相互独立；可靠新回合不会清除未读结束历史。清空历史保留请求、去重与因果记录。
- `active` 按待处理、失败、未读结束、运行中排序，同级选最近事件；时间相同按会话 key 稳定排序。父子会话各自保留状态，`parentSessionId` 用于后续 UI 分组；`active` 始终指向实际存在且需要展示的会话。
- 恢复时工作中降为未知，未读提醒保留。历史至多 1,000 条、7 天；未知因果与去重记录独立于历史，本核心在内存中不裁剪这些记录，后续存储层需持久化并保留至少 24 小时的去重数据。

`source_snapshot` 不产生新提醒。新会话可导入；已有会话只有与 envelope 对应的同一会话拥有同实例可比较新序号时才能校正。当前 DTO 不含每个 projection 的序号或实例水位：其他已有会话的冲突快照保留已知回合及请求并标未知，遗漏会话也保守标未知；同会话已知旧序号的遗漏不覆盖更新事实。后续若要可靠校正多会话快照，需扩展每会话因果水位。来源断开事件仅影响同一安装与实例的会话，不凭静默判定断开。

## 测试样例

`tests/fixtures/events/` 保存脱敏事件及手工期望快照；`tests/fixtures/protocol/` 保存协议输入，供后续 Rust、TypeScript 与适配器共同使用。`pet-core` 集成测试使用本地 `tests/support` 辅助模块加载，不跨 crate 导入测试目录。

完整设计见 [双平台桌宠设计](docs/superpowers/specs/2026-11-11-tauri-desktop-pet-design.md)。本任务没有验证桌面事件覆盖、精确会话跳转、原生窗口、资源预算或双平台运行能力。
