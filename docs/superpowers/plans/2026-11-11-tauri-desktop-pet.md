# Tauri 双平台独立桌宠实现计划

> 面向自动化执行者：逐任务实现时必须使用 `superpowers:executing-plans`，或在用户选择子代理执行后使用 `superpowers:subagent-driven-development`。按复选框跟踪进度，不跳过关键路径验证。

**目标：** 制作可在 macOS、Windows 独立运行的桌宠，接收 Codex 与 DeepSeek Harness 的提醒，并打开对应会话。

**架构：** Rust 核心负责事件、状态与文件持久化，原生 `pet-bridge` 转发器连接宿主事件和桌宠本地入口。Tauri 承担窗口和托盘，TypeScript/Canvas 负责角色绘制，两个来源分别适配并报告经过验证的能力。

**技术栈：** Tauri 2、Rust 1.93.1、Node 24 LTS、pnpm 10.28.2、Vite、TypeScript、Vitest；Rust 使用 serde/serde_json、Tokio、Axum、notify、image，具体兼容补丁版本在任务 1/2 安装时锁定并提交锁文件。前端先使用 TypeScript DOM 组件，不增加大型 UI 框架。

**设计依据：** [双平台桌宠设计](../specs/2026-11-11-tauri-desktop-pet-design.md)。主控持有全计划；子代理使用任务brief与共享约束，按需读取设计对应章节。

## 全局约束

- 始终在当前分支修改；当前为 `main`，用户没有要求新分支，不创建 worktree 或分支。
- 说明、界面文案和提交信息使用简体中文；技术标识保持原名。
- “首版验收目标为 macOS 13+（Apple Silicon、Intel）及 Windows 10 22H2 / Windows 11 x64。”Windows ARM64 不在首版目标。
- “先覆盖同一台电脑、同一登录用户下的原生桌面程序和本地 CLI 会话。”WSL、SSH、容器、云端接入另行扩展。
- “第一版提供提醒与会话跳转；任务回答、批准、执行控制不在本版范围。”
- 2026-11-16用户确认分级支持：Codex按准确版本提供可验证的生命周期提醒/会话跳转，精确审批/提问等待和解除不列为Codex首版保证；Harness仍提供完整待处理提醒。来源卡必须区分已验证、不支持、未验证。
- 复用现有素材，不覆盖 `pet-output/` 原包；不包含自动补帧、Live2D、多桌宠和养成系统。
- 默认 30 FPS、60 FPS 可选；手动启动、声音关闭，开机启动可选；后台通知不抢键盘焦点。
- 待发送区上限 2,000 条、10 MiB、24 小时；历史最多 1,000 条、7 天；去重记录独立保留至少 24 小时。
- 文件存储，不引入 SQL；未来变更也禁止在循环遍历中查询 SQL。
- 正常通知端到端 P95 小于 500 ms；30 FPS 模式连续 60 秒至少 95% 绘制间隔不超过 50 ms；离线恢复单独统计。
- 两平台分别记录待机、动画、隐藏各 5 分钟资源占用；隐藏/休眠停止绘制。资源预算在阶段一实测后锁定。
- 个人测试包可未签名；正式签名、公证、商店和自动更新服务不在本计划范围。Windows 验收不能用交叉编译代替。
- 当前处于用户已授权的子代理逐任务实现阶段，可以安装项目依赖和创建工程；真实来源安装/信任仍为显式操作，不在开发中默认修改宿主配置。

## 审阅重点

1. 安装路径有空格、中文或发生升级，已有第三方 Hook 被用户修改：自有接入仍可安装/撤销，其他配置不被覆盖；归任务 3、12。
2. 写入中崩溃、响应丢失、历史压缩后重发：已确认事件不丢、不重复提醒，半条日志不破坏其余历史；归任务 5、6。
3. 审批在原应用中处理、请求撤回、旧回合迟到、timed问题跨回合回答：桌宠解除准确的请求，不恢复过期气泡，不覆盖新回合；归任务 1、4、6。
4. 混合 DPI、显示器移除、未聚焦和休眠恢复：宠物仍在可见工作区、不抢焦点、按目标绘制且隐藏时停止；归任务 2、8、9。
5. 宿主接口不支持、CLI 身份与桌面不一致、深链只唤起应用：能力表明确缺失，跳转失败可恢复，不误报精确定位成功；归任务 3、4、11。

## 目录与职责

根目录建立 Cargo workspace 与 pnpm workspace；开发命令均从仓库根执行。Node 用受支持的 24.x，避免依赖本机 25.5.0 的行为；最低 24.9，实际补丁版本写入 `.node-version`。根脚本提供 `pnpm test`、`pnpm typecheck`、`pnpm build`；`@codex-pet/desktop` 和 `@codex-pet/deepseek-integration` 为包名，Tauri Cargo 包名为 `pet-desktop`，其余为 `pet-core`、`pet-bridge`。前端 test 脚本使用 `vitest run`，空测试集必须失败。

| 位置 | 职责与主要文件 |
| --- | --- |
| `crates/pet-core/src/` | `protocol.rs`、`state.rs`、`storage.rs`、`pet_package.rs`、`lib.rs`；无 GUI、无宿主依赖 |
| `crates/pet-bridge/src/` | `main.rs`、`codex.rs`、`outbox.rs`、`delivery.rs`、`hook_install.rs`；原生命令与 Hook 转换 |
| `apps/desktop/src-tauri/src/` | `lib.rs`、`main.rs`、`broker.rs`、`commands.rs`、`navigation.rs`、`windows.rs`、`platform/{mod,macos,windows}.rs` |
| `apps/desktop/src/` | `main.ts`、`api.ts`、`types.ts`、`animation/{clock,renderer}.ts`、`components/{pet,notifications,settings}.ts`、`locales/zh-CN.ts`、`styles.css` |
| `integrations/codex/` | `README.md`、`hooks.template.json`、`compatibility.json`、`fixtures/` |
| `integrations/deepseek-harness/` | `src/{index,events,open-session}.ts`、`tests/`、`package.json`、`README.md`、`compatibility.json`、必要的 `patches/` |
| `tests/fixtures/` | `events/`、`protocol/`、`pet-packages/`；脱敏、版本化、可重放样例 |
| `docs/validation/desktop-pet/` | `platforms.md`、`codex.md`、`harness.md`、`release-checklist.md`；记录真实环境和证据 |
| `.github/workflows/desktop-pet.yml` | Rust/前端检查和三个目标构建，先作为 CI 产物，不自动公开发布 |

新增根文件：`Cargo.toml`、`Cargo.lock`、`rust-toolchain.toml`、`package.json`、`pnpm-workspace.yaml`、`pnpm-lock.yaml`、`.node-version`、`.gitignore`、`README.md`。忽略构建、缓存和本地诊断数据，不忽略源素材与兼容性结论。

## 共享接口与测试约定

Rust 协议使用 serde 的 camelCase JSON 字段，枚举值为 snake_case，EventKind 使用 `type` 作为内部标签，None 字段省略；TypeScript 定义相同字段与可选项。`tests/fixtures/protocol/*.json` 同时用于 Rust、前端和适配器验证。跨进程输入在入口验证，宿主原始 payload 不直接传给界面。

- `Source = Codex | Deepseek`；`SourceIdentity { source, installation_id: String, instance_id: String }`。安装 ID 持久化。支持宿主连接的实例 ID 按其运行生命周期创建；公开 Codex Hooks 缺宿主实例字段时，允许带 `hook-session:` 前缀的稳定会话观察范围，保持 `sequence = None`，不生成快照/断开或推断存活/重启。不能给每次 Hook 调用随意换安装 ID。
- `EventEnvelope { schema_version: u16, event_id: String, identity: SourceIdentity, session_id: String, parent_session_id: Option<String>, turn_id: Option<String>, upstream_event_id: Option<String>, sequence: Option<u64>, occurred_at_ms: u64, received_at_ms: Option<u64>, summary: Option<String>, project: Option<ProjectContext>, kind: EventKind }`；`ProjectContext { name: String, directory: Option<String> }`。接收端填写 received_at_ms。
- `EventKind = TurnStarted | AttentionRequested { request_id, reason: Answer|Approval, lifetime: RequestLifetime } | AttentionResolved { request_id } | TurnEnded { outcome: Completed|Interrupted|Failed|Unknown } | SourceDisconnected | SourceSnapshot { sessions: Vec<SessionProjection> }`。
- `RequestLifetime = Turn | UntilResolved`，JSON为`turn`/`until_resolved`，默认Turn且省略默认值。当前生产事件协议2，兼容读取v1普通Turn语义；v1携持续请求、未知版本拒绝。持续请求跨回合保留，仅原会话/原请求turn/requestId精确解除；pending持续请求与投影当前turn可不同，普通Turn约束不放宽。
- `SessionProjection { key: SessionKey, parent_session_id: Option<String>, turn_id: Option<String>, status: Running|NeedsAttention|Ended|Interrupted|Failed|Unknown, pending: Vec<PendingRequest>, project: Option<ProjectContext>, summary: Option<String>, unread: bool, last_event_at_ms: u64 }`；`PendingRequest { id: String, turn_id: Option<String>, reason: Answer|Approval, summary: Option<String>, lifetime: RequestLifetime }`；`SessionKey { source: Source, installation_id: String, session_id: String }`。
- `NotificationRecord { id: String, key: SessionKey, turn_id: Option<String>, kind: NotificationKind, summary: Option<String>, project: Option<ProjectContext>, occurred_at_ms: u64, read: bool }`；NotificationKind 为 `Attention { request_id: String, reason: Answer|Approval, resolved: bool, lifetime: RequestLifetime }` 或 `TurnEnded { outcome: Completed|Interrupted|Failed|Unknown }`。
- `PetSnapshot { revision: u64, sessions: Vec<SessionProjection>, notifications: Vec<NotificationRecord>, active: Option<SessionKey> }`。结束不是业务成功；无可靠因果信息时保持 Unknown。通知最多保留设计规定的 1,000 条，角色绘制不因通知列表刷新而重新初始化。
- `SessionStore::apply(&mut self, event: &EventEnvelope) -> ApplyResult`、`mark_read(&mut self, key: &SessionKey) -> ApplyResult`、`clear_history(&mut self) -> ApplyResult`、`snapshot(&self) -> PetSnapshot`、`restore(events: &[EventEnvelope], now_ms: u64) -> Self`；`ApplyResult { changed: bool, should_notify: bool }`。
- 任务 1 建立 `crates/pet-core/tests/support/mod.rs`，`load_case(name: &str) -> EventCase`，`EventCase { events: Vec<EventEnvelope>, expected: PetSnapshot }`。后续 Rust 测试内联读取 fixture，或使用各 crate 的本地辅助模块，不直接导入另一个 crate 的 tests 目录。
- 应用数据由 `AppPaths::for_user() -> Result<AppPaths, StorageError>` 定位；字段为 root、outbox、journal、dedup、settings、discovery。测试用 `AppPaths::under(&Path)`，不写真实用户目录。
- 统一事件最长 64 KiB，摘要最多 512 个 Unicode 字符。宿主原始 Hook 输入单独限制为 8 MiB，先归一化再检查统一事件上限，不能因为原提示词超过 64 KiB 就丢弃状态提醒。原始日志不留提示词、工具参数、API Key；超限输入报告诊断，不截断 JSON 后继续解析。

## 执行次序与检查点

`1 → 2 → 3 → 4 → 检查点 A → 5 → 6 → 7 → 8 → 9 → 10 → 11 → 12`。

检查点 A：记录两平台窗口/后台绘制、分级后的Codex生命周期、Harness完整事件与精确跳转证据。代码审阅门与真实运行验收分开；缺环境的实测项保留未通过，并进入最终发行阻断清单，可以继续不依赖该缺口的后续实现，不把界面开发当验证完成。来源能力与用户批准范围不符的代码问题仍须先修复，不能以未验标签掩盖。

每个任务先运行会失败的行为测试，再实现并运行同一测试；配置和脚手架不写仅检查配置文本的镜像测试，原生行为使用真实应用实验。每次只提交该任务文件，提交信息用中文；提交前运行相关检查。独立任务接口发生变化时，同步更新本计划与全部消费者。

### Task 1:可重放的统一事件与会话状态

**文件：** 新建根 Cargo/pnpm workspace 配置、`crates/pet-core/Cargo.toml`、`src/{lib,protocol,state}.rs`、`tests/{protocol,state}.rs`、`tests/support/mod.rs`、`tests/fixtures/{events,protocol}/*.json`。

**接口：** 产出上文 EventEnvelope、SessionStore、PetSnapshot。`protocol::decode_event(bytes: &[u8]) -> Result<EventEnvelope, ProtocolError>` 负责协议版本、必填字段、长度与字段枚举检查。

- [x] **1. 写失败测试。** `protocol_rejects_invalid_input` 对未知版本、64 KiB 以上事件、字段缺失断言 `decode_event(...).is_err()`；`state_replay_cases` 覆盖两来源同名会话、重复事件、先 resolved 后迟到 requested、旧回合开始晚到、子代理折叠及结束为 Unknown：
  ```rust
  let case = load_case("late-old-turn");
  let mut store = SessionStore::default();
  for event in &case.events { store.apply(event); }
  assert_eq!(store.snapshot(), case.expected);
  ```
- [x] **2. 运行并确认失败。** `cargo test -p pet-core --test state`，失败来自缺少状态行为，不将网络/编译环境错误当成红灯用例。
- [x] **3. 实现协议与状态。** 初始协议版本1（Task4升级生产2并兼容读取v1），去重按事件 ID，顺序按上游序号/回合/请求关联；resolve tombstone 防止请求倒序复活。缺少可比较的回合/顺序信息时不猜新旧，返回 Unknown；restore 将旧 Running 变为 Unknown，优先级按设计。
- [x] **4. 验证。** `cargo test -p pet-core --test protocol --test state`，全部通过；确认 snapshot 对同一事件重放稳定，已读和新回合的关系有独立用例。
- [x] **5. 提交。** 暂存本任务文件，`git commit -m "添加桌宠事件协议与会话状态管理"`。

### Task 2:双平台最小窗口与运行时诊断

**文件：** 新建 `apps/desktop/{package.json,index.html,vite.config.ts,tsconfig.json,src/main.ts}`、`src-tauri/{Cargo.toml,build.rs,tauri.conf.json,capabilities/default.json,src/lib.rs,src/main.rs,src/windows.rs,src/platform/mod.rs,src/platform/macos.rs,src/platform/windows.rs}`、`src/diagnostics/{frame-meter,platform-report}.ts`、`docs/validation/desktop-pet/platforms.md`、`.github/workflows/desktop-pet.yml`。

**接口：** `platform::create_pet_window(app: &tauri::AppHandle) -> Result<(), PlatformError>`；前端 `FrameMeter.record(timestampMs: number): void`、`report(): { intervalsMs: number[]; p95Ms: number }`。诊断报告必须包含 OS、架构、WebView、依赖版本及窗口可见/聚焦状态。

- [x] **1. 固定实验。** 在两个系统上定义“透明窄窗口、拖动、外缘穿透、后台不抢焦点、未聚焦播放 60 秒”用例，先记录当前没有对应应用的基线；为 frame-meter 写 `intervals [33,34,33] → p95≤50` 的失败测试。
- [x] **2. 运行失败测试。** `pnpm --filter @codex-pet/desktop test -- frame-meter`，确认统计行为尚未实现。
- [x] **3. 实现最小壳。** 建立 Tauri 2 与 Vite；只画测试角色/已有首帧和移动轨迹，不做设置页。透明、置顶、焦点和后台节流使用锁定版本支持的 API；确需原生 API 时放入 platform 文件。初始化三个目标的构建 CI，但不发布。
- [ ] **4. 验证。** `pnpm --filter @codex-pet/desktop test -- frame-meter`、`pnpm --filter @codex-pet/desktop tauri build --debug --no-bundle`；Windows 本机运行同样命令。分别实测前述输入/后台场景并记录数据，未测项标记未验证；阶段一资源预算在该报告中锁定。
- [x] **5. 提交。** `git commit -m "添加双平台桌宠窗口与性能诊断"`，只暂存本任务文件和生成的依赖锁文件。

实现与代码审阅已通过（4739933）；步骤4的本机自动检查及部分原生查询已执行，完整双平台/物理交互验收仍未通过，保留未勾选状态。按该步骤的未测项记录约定，继续阶段一来源探测，检查点A仍为未通过。

### Task 3:Codex 桌面事件探测、Hook 转换与安全安装

**文件：** 新建 `crates/pet-bridge/{Cargo.toml,src/main.rs,src/codex.rs,src/hook_install.rs,tests/codex.rs,tests/hook_install.rs}`、`integrations/codex/{README.md,hooks.template.json,compatibility.json,fixtures/*.json}`、`docs/validation/desktop-pet/codex.md`；修改根 Cargo workspace。

**接口：** `normalize_codex(input: &serde_json::Value, identity: &SourceIdentity, now_ms: u64) -> Result<Vec<EventEnvelope>, AdapterError>`；`plan_hook_install(existing: &[u8], bridge: &Path) -> Result<HookPatch, InstallError>`、`apply_hook_patch(path: &Path, patch: &HookPatch) -> Result<(), InstallError>`；`HookPatch { before_hash, after_bytes, owned_entry_ids }`，应用前检查并发修改。CLI `pet-bridge codex [--probe-dir PATH]` 从 stdin 接收原始 Hook；探测模式只输出脱敏后的统一事件文件。

- [x] **1. 写失败测试。** 版本化样例覆盖已支持生命周期、有限子代理身份与各不支持路径；PermissionRequest、提问、取消、继续不伪造等待/解除；`Stop → outcome Unknown`。`merge_preserves_foreign_hooks` 断言自有接入安装两次不重复，撤销不改第三方条目；中文/空格路径作为一个正确命令参数。原始 Hook 包含 100 KiB 提示词时仍产出小于 64 KiB 的状态事件，摘要不包含原始提示词和工具参数。
- [x] **2. 运行并确认失败。** `cargo test -p pet-bridge --test codex --test hook_install`。
- [x] **3. 实现。** Hook 输出 `{}`，诊断只写 stderr，绝不返回批准/拒绝决定。安装备份、清单和内容 hash；撤销只移除仍匹配的自有条目，不用旧备份覆盖用户新修改。默认不更改真实配置，安装是显式操作。按用户确认的分级范围声明能力，Codex精确待处理同步为不支持；可证明的ThreadSpawn身份仅做有限生命周期映射，未知父关系或终止路径不猜。先保留 probe-dir 路径，可靠投递由任务 6 接入。
- [ ] **4. 实机验证。** `cargo test -p pet-bridge --test codex --test hook_install`；在明确授权的配置中验证各版本已声明生命周期；不把0.145的CLI开始证据外推桌面、停止或中断。Codex精确待处理同步按已批准范围记为不支持，可靠终态/子代理范围逐项记录。真实两平台覆盖及至少3会话的 `codex://threads/<id>` 保留到检查点A与任务11/12，不以另起server或未验标签冒充通过；无环境可先完成实现和能力记录。
- [x] **5. 提交。** `git commit -m "添加 Codex 桌宠通知适配与接入验证"`。

按用户2026-11-16确认的分级目标，实现/能力记录已复审通过（2120d1b）；步骤4真实桌面、Windows与3会话导航仍未通过，保留未勾选，转A/任务11-12发行阻断。继续独立Harness实现不表示这些验收已完成。

### Task 4:Harness 通知插件与精确会话入口

**文件：** 新建 `integrations/deepseek-harness/{package.json,tsconfig.json,src/index.ts,src/events.ts,src/open-session.ts,tests/events.test.ts,tests/open-session.test.ts,README.md,compatibility.json,fixtures/*.json}`、`docs/validation/desktop-pet/harness.md`；若必须补宿主入口，增加 `patches/desktop-session-deep-link.patch`。

**接口：** `apply(ctx: Context, config: PluginConfig): void`；`toPetEvents(observation: HarnessObservation, identity: SourceIdentity): EventEnvelope[]`；`PluginConfig { bridgePath: string; installationId: string; probeDir?: string }`。HarnessObservation 为本任务定义的宿主事件联合类型，包含会话ID、父会话ID、原始请求turn、真实seq、请求生命周期及挑选后的状态字段。`openHarnessSession(sessionId: string): Promise<{ status: 'requested' | 'unavailable'; reason?: string }>` 不把仅唤起应用作为精确打开。

- [x] **1. 写失败测试。** `approval_decided_clears_only_matching_request`、`question_listener_always_delegates`、`cancelled_question_resolves_attention`、`parent_child_ids_remain_distinct`及timed超时后跨回合保留/late reply精确解除、无seq请求先于turn/start到达，对 `next` 调用一次、保留返回值与异常分别断言；转发器失败时原宿主流程仍继续。
- [x] **2. 运行并确认失败。** `pnpm --filter @codex-pet/deepseek-integration test -- events open-session`。
- [x] **3. 实现并锁定跳转路径。** 订阅公开事件，不修改agent-loop；提问观察不领取请求。安装身份显式持久化；日志流实例/真实seq仅声明已证明的正常持久前缀，崩溃回退交Task6源代次处理。允许本任务最小扩展pet-core请求生命周期/协议2与harness桥接probe，精确匹配跨回合持续请求，所有消费者/旧版默认兼容同步。先验证 Host/Client 公开扩展点能否定向调用 `uiWorkspace.openSession`，注意远程事件有显式白名单。若不足，维护最小 `dsh://session/<encoded-id>` 补丁：宿主新增 `apps/desktop/src/session-deep-link.ts` 和对应测试，修改 `main.ts`、`ipc.ts`、`preload-app.ts`，Client 插件接受目标会话；覆盖冷启动排队、热启动和 Windows second-instance。补丁保存在本仓库，在 Harness 现有分支验证；修改前读取其 AGENTS.md，使用受支持的 dsh profile 启动方式。
- [ ] **4. 验证。** 上述单测通过；两平台真实 Harness 至少 3 个会话，原应用回答、审批撤回后请求解除；点击目标链接在隐藏、已启动、冷启动状态准确打开会话。插件路径或所需补丁、兼容版本及安装方法必须写入报告，不能只留下候选路径。运行宿主补丁相关定向测试与其要求的检查，不默认跑全仓测试。
- [ ] **5. 提交并过检查点 A。** `git commit -m "添加 Harness 通知插件与会话跳转接入"`。将分级来源结论、已有实测和Windows缺项写入报告；未验项保持阻断最终发行，可继续任务5的独立实现。

代码和安装交付已通过独立审阅，主仓修复提交 `f9d9af3`，宿主 `2809f54899`。受支持临时 profile 已验证 Host 激活与 Client 工厂交付，真实导航及双平台验收见 [Harness 验证记录](../../validation/desktop-pet/harness.md)，检查点 A 仍待补。

### Task 5:日志、去重与有界待发送区

**文件：** 新建 `crates/pet-core/src/{storage,paths}.rs`、`crates/pet-core/tests/storage.rs`、`crates/pet-bridge/src/outbox.rs`、`crates/pet-bridge/tests/outbox.rs`；修改 pet-core 导出和测试 fixture。

**接口：** `JournalStore::open(paths: AppPaths) -> Result<Self, StorageError>`、`ingest(&mut self, event: EventEnvelope) -> Result<IngestAck, StorageError>`、`recover(&mut self, now_ms: u64) -> Result<PetSnapshot, StorageError>`、`compact(&mut self, now_ms: u64) -> Result<(), StorageError>`、`mark_read(&mut self, key: &SessionKey) -> Result<PetSnapshot, StorageError>`、`clear_history(&mut self) -> Result<PetSnapshot, StorageError>`；`IngestAck { event_id: String, duplicate: bool, revision: u64 }`。`Outbox::enqueue(&self, event: &EventEnvelope) -> Result<(), DeliveryError>`、`pending(&self, now_ms: u64) -> Result<Vec<QueuedEvent>, DeliveryError>`、`acknowledge(&self, event_id: &str) -> Result<(), DeliveryError>`；QueuedEvent 包含 envelope、queued_at_ms 与文件路径，保留期依据本地入队时间。

- [x] **1. 写失败测试。** `acknowledged_event_survives_restart`、`partial_tail_preserves_prior_records`、`compaction_keeps_dedup_for_24_hours`；写入 1,001 条历史后重试早期事件仍 `duplicate == true`。`read_and_clear_survive_restart` 断言已读/清空标记持久化，尚未解决的 pending 保留，revision 不倒退。outbox 用注入时钟测试 2,001 条、10 MiB、24 小时淘汰顺序、并发写入和溢出数量；损坏文件隔离并保留诊断。
- [x] **2. 运行并确认失败。** `cargo test -p pet-core --test storage`、`cargo test -p pet-bridge --test outbox`。
- [x] **3. 实现。** 单写入者管理 journal，持久化成功后返回 ACK；原子写临时文件并替换，跨进程锁保护 outbox 配额/清理，拒绝符号链接和目录逃逸。日志记录区分事件、已读和清空操作，快照保留完整状态、revision、活动请求与必要的因果标记；裁剪可见历史不能丢掉这些信息。清空只清可见历史，不移除活动请求、宠物或去重索引。压缩和恢复保留独立去重信息；明确磁盘写入失败时不确认收件，已提交记录与视图按 revision 一致恢复。
- [x] **4. 验证。** 重跑以上测试；用临时目录模拟截断、响应前崩溃与权限拒绝，重启后检查历史、去重和 pending，所有操作不得触碰真实用户数据目录。
- [x] **5. 提交。** `git commit -m "添加可靠事件存储与离线待发送区"`。

主仓本地实现与审阅修复至 `ea17b8d`。macOS 故障测试及 Windows x64 交叉检查通过，Windows 原生耐久和实际断电留待平台验收。

### Task 6:本地入口、可靠转发与恢复归并

**文件：** 新建 `apps/desktop/src-tauri/src/broker.rs`、`apps/desktop/src-tauri/tests/broker.rs`、`crates/pet-bridge/src/delivery.rs`、`crates/pet-bridge/tests/delivery.rs`；修改 bridge/main、桌面 lib、两个适配器配置与文档。

**接口：** `BrokerHandle::start(paths: AppPaths) -> Result<Self, BrokerError>`、`snapshot(&self) -> PetSnapshot`、`subscribe(&self) -> tokio::sync::broadcast::Receiver<PetSnapshot>`；POST `/v1/events` 输入 EventEnvelope、返回 IngestAck。发现文件 `Discovery { schema_version: u16, port: u16, token: String }`。`deliver(event: &EventEnvelope, paths: &AppPaths) -> Result<DeliveryOutcome, DeliveryError>`；DeliveryOutcome 为 Delivered 或 Queued。

- [x] **1. 写失败测试。** 真实 loopback 端口测试错令牌/带 Origin/超限 body 被拒绝、持久化失败不 ACK；ACK 丢失后重试只产生一次通知。`offline_requested_then_resolved_never_bubbles` 将同一批离线 requested/resolved/end 归并后断言只展示仍有效项目；旧发现文件和重启令牌轮换可恢复。
- [x] **2. 运行并确认失败。** `cargo test -p pet-desktop --test broker`、`cargo test -p pet-bridge --test delivery`。
- [x] **3. 实现。** 随机 loopback 端口和用户目录私有发现文件，验证 token 与来源；bridge 先 enqueue，再最多尝试 200 ms 网络投递，收到持久化 ACK 后删除。应用启动/休眠恢复扫描、notify 文件监测共同触发补发；启动先归并整批状态再通知，去重贯穿 HTTP 与文件扫描。probe-dir 仅作为显式诊断模式保留。
- [ ] **4. 验证。** 重跑测试；实际关闭桌宠、产生提醒、重启，验证历史和当前状态正确，原任务不中断。Windows 转发器无控制台闪烁；磁盘完全不可写时输出诊断并正常退出宿主 Hook，不声称已留存。
- [x] **5. 提交。** `git commit -m "接通桌宠本地通知入口与断线恢复"`。

先交付本机投递和离线恢复，再实现 Harness 来源自身的权威恢复。现有公开接口能观察连续会话，以及用原始调用 ID 恢复正常持久化的定时请求。普通直接提问的观察器重载和未同步会话尾部的崩溃回退，需要宿主补充请求身份、作用域、完整恢复边界及日志代次。恢复前须识别源代次与每个会话的水位，确切区分仍待处理和已解除的请求，避免旧代回合结束或请求解除记录影响新代。用只读旁观者接口接入，保持原任务与原应用提问流程。两部分分别审阅，真实来源恢复进入最终发行验收。

本机可靠投递与 Harness 来源恢复已经实现并通过独立复审。桌宠进度至 `f616d4e`，配套 Harness 补丁目标为 `c507c325`。隔离验证覆盖 macOS 离线补发、进程强杀、旧凭证兼容和原问题服务的准确解除。步骤 4 保留 Windows 原生无控制台、系统休眠及真实应用操作的验收待办。

### Task 7:旧宠物导入和独立动画包格式

**文件：** 新建 `crates/pet-core/src/pet_package.rs`、`crates/pet-core/tests/pet_package.rs`、`tests/fixtures/pet-packages/`、`apps/desktop/src/types.ts`、`docs/pet-format.md`。

**接口：** `import_pet(source: &Path, destination: &Path) -> Result<PetManifest, PetError>`；`PetManifest { version: u16, id: String, name: String, atlas: Atlas, animations: BTreeMap<String, Animation>, look_directions: Option<Vec<FrameRect>> }`；Atlas 包含图像相对路径、宽高、格子宽高；Animation 包含 `frames: Vec<AnimationFrame>` 和 `loop_start: Option<usize>`；AnimationFrame 包含 FrameRect 与 `duration_ms: u32`；FrameRect 为整数 x/y/width/height。

- [x] **1. 写失败测试。** v1 1536×1872、v2 1536×2288 导入后九状态帧数为 `[6,8,8,4,5,8,6,6,6]`，v2 的 16 个方向不混入普通动画；内部格式能接受超过 8 帧动作。拒绝无效解码、20 MiB 以上文件、超过 4096×4096 的图像、越界格子、零时长、路径逃逸；导入前后原包字节不变。
- [x] **2. 运行并确认失败。** `cargo test -p pet-core --test pet_package`。
- [x] **3. 实现。** JSON 元数据和 PNG/WebP 解码分别验证；导入复制到应用数据目录，以内部显式帧矩形和时间表示，文档固定时间单位。旧待机使用基础时长 `[280,110,110,140,140,320]`，不沿用 Codex 的额外 6 倍延时；其他动作沿用已核查基础时长。
- [x] **4. 验证。** 重跑测试并导入本仓库至少一个 v1、一个 v2 包；检查九状态与方向联系表，确认新包与原包独立且清空历史不会删除素材。
- [x] **5. 提交。** `git commit -m "添加旧宠物包导入与独立动画格式"`。

实现及独立复审通过，最终提交 `611415d`。月影 v1、大肥鱼 v2 导入后可重载，清空历史保留两套素材，原包 SHA-256 一致。Windows x64 交叉检查通过，Windows 原生导入留待平台验收。

### Task 8:Canvas 播放器与可见性生命周期

**文件：** 新建 `apps/desktop/src/animation/{clock,renderer}.ts`、`apps/desktop/src/animation/{clock,renderer}.test.ts`、`apps/desktop/src/components/pet.ts`；修改诊断和主入口。

**接口：** `AnimationClock.sample(animation: Animation, elapsedMs: number): { frameIndex: number; rect: FrameRect }`；`PetRenderer.mount(canvas: HTMLCanvasElement, manifest: PetManifest): Promise<void>`、`setState(state: string, nowMs: number): void`、`setVisible(visible: boolean, nowMs: number): void`、`setFps(fps: 30|60): void`、`dispose(): void`。浏览器边界类型在 types.ts 与 manifest JSON fixture 对齐。

- [ ] **1. 写失败测试。** 不同时长边界、loop_start、一次跳过 5 秒后直接采样当前帧；30 FPS 下 0/16/34 ms 的采样不在每次 rAF 都绘制。隐藏后已安排的回调被取消且不继续安排；恢复从当前状态重新计时，重复 mount/dispose 不泄漏循环。
- [ ] **2. 运行并确认失败。** `pnpm --filter @codex-pet/desktop test -- animation`。
- [ ] **3. 实现。** 图集加载一次并缓存；drawImage 裁切，不逐帧解码或改写完整 UI。rAF 用 elapsed 时间采样、按目标 FPS 节流；移动和整体轻变换独立于角色姿态，不把 60 FPS 宣传为 60 个独立姿态。
- [ ] **4. 验证。** 重跑测试；两平台九状态视觉检查、可见未聚焦 60 秒间隔统计、隐藏/休眠恢复；对照任务 2 的报告确认无退化。
- [ ] **5. 提交。** `git commit -m "添加流畅动画播放与后台生命周期管理"`。

### Task 9:可恢复的窗口位置、输入命中与托盘

**文件：** 修改 `windows.rs`、`platform/{mod,macos,windows}.rs`；新建 `src-tauri/src/placement.rs`、`src-tauri/tests/placement.rs`、`src-tauri/src/tray.rs`，更新 `platforms.md`。

**接口：** `clamp_placement(value: WindowPlacement, monitors: &[WorkArea]) -> WindowPlacement`；WindowPlacement 包含逻辑坐标 x/y、scale、可选 monitor_id，WorkArea 包含 id、x/y/width/height/scale_factor。`platform::update_hit_region(frame: &FrameRect, mask: &AlphaMask) -> Result<(), PlatformError>`；AlphaMask 在本任务定义为宽高和透明命中位图，从已导入图集按帧缓存，不逐帧重新读取像素。

- [ ] **1. 写失败测试。** 负坐标副屏、被移除的屏幕、100/150/200% 混合缩放、缩放后窗口大于工作区都夹回可见位置。原生实验用例明确“点击宠物能操作、点击透明边缘落到下面应用、通知不抢输入、托盘始终可恢复”。
- [ ] **2. 运行并确认失败。** `cargo test -p pet-desktop --test placement`；原生问题先在真机复现，不用测试 mock 的返回值替代命中行为。
- [ ] **3. 实现。** 保存逻辑坐标，显示器变化后重新夹取；输入、置顶与焦点策略沿用任务 2 通过的方案。托盘提供显示/隐藏、通知、设置、退出；用户主动打开通知列表才使用可聚焦窗口。普通模式不覆盖独占全屏。
- [ ] **4. 验证。** 单测通过；两平台执行混合缩放、屏幕移除、跨屏拖动、全屏、休眠矩阵，记录实际结果；隐藏不关闭 broker，退出才停止接收。
- [ ] **5. 提交。** `git commit -m "完善桌宠窗口交互与多显示器恢复"`。

### Task 10:通知列表、设置和来源接入状态

**文件：** 新建 `src-tauri/src/{commands,settings}.rs`、`src-tauri/tests/settings.rs`、`apps/desktop/src/{api,types}.ts`、`src/components/{notifications,settings}.ts`、对应 `.test.ts`、`src/locales/zh-CN.ts`、`src/styles.css`，修改桌面入口与 capabilities。

**接口：** Tauri commands `get_snapshot() -> PetSnapshot`、`mark_read(key: SessionKey) -> PetSnapshot`、`clear_history() -> PetSnapshot`、`get_settings() -> Settings`、`update_settings(patch: SettingsPatch) -> Settings`、`import_pet_package(path: String) -> PetManifest`、`integration_status() -> Vec<IntegrationStatus>`、`apply_integration(source: Source, action: Install|Remove) -> InstallReport`。Settings 包含 fps、sound、autostart、pet_id、muted_sources、placement；后端广播 `pet:snapshot`，revision 单调递增。

IntegrationStatus 在本任务定义为 source、installation_id、可选 version、installed、capabilities、可选 problem；每项能力包含名称和 Verified/Unsupported/Unverified 状态。InstallReport 包含 changed、可选 backup_path、owned_entry_ids、warnings。SettingsPatch 的字段均为 Settings 对应字段的可选更新；Tauri command 的存储/安装错误返回结构化错误，不返回虚假成功值。get_snapshot 通过 broker 获取状态，mark_read/clear_history 通过单写入者持久化后再广播。

- [ ] **1. 写失败测试。** DOM 用真实 fixture 断言来源/项目/摘要、未读标记和待处理自动解除；先订阅再取快照，较旧 revision 不覆盖较新状态。设置原子保存，非法 FPS 被拒绝；清空历史保留宠物和去重索引，mark_read 不解除仍未解决的审批。
- [ ] **2. 运行并确认失败。** `pnpm --filter @codex-pet/desktop test -- components api`、`cargo test -p pet-desktop --test settings`。
- [ ] **3. 实现。** 仅展示业务信息和可恢复错误；来源卡片展示版本、能力和安装/撤销状态。声音与开机启动默认关闭，安装来源和导入包由明确 UI 动作触发。前端只允许必要的本地资源和受限 Tauri 命令，后台事件不得调用 window focus。
- [ ] **4. 验证。** 重跑相关测试、`pnpm typecheck`；两平台实际安装/撤销来源、静音、切宠物、30/60 切换、重启恢复，文案不出现“任务成功”替代未知结束。
- [ ] **5. 提交。** `git commit -m "添加桌宠通知管理与设置界面"`。

### Task 11:来源导航与完整通知点击链路

**文件：** 新建 `src-tauri/src/navigation.rs`、`src-tauri/tests/navigation.rs`、`apps/desktop/src/components/notifications.test.ts` 的点击链路用例，修改 commands、两个来源兼容表与 README。

**接口：** `open_session(key: SessionKey) -> Result<OpenOutcome, NavigationError>`；OpenOutcome 为 Requested、ApplicationOnly 或 Unavailable。Rust 根据已登记的来源安装信息构造链接/调用经过验证的 Harness 入口；不执行事件中任意提供的命令或 URL。系统接收深链只返回 Requested，不能宣称已确认来源界面加载成功。

- [ ] **1. 写失败测试。** 不支持版本、未知安装 ID、恶意 scheme、带控制字符 ID、已卸载宿主、CLI 无桌面对应身份分别返回准确结果；命令形态不经过 shell 拼接。点击 Requested 展示“已发送打开请求”，ApplicationOnly 明示“打开应用”，失败保留通知及复制会话 ID 操作。
- [ ] **2. 运行并确认失败。** `cargo test -p pet-desktop --test navigation`、`pnpm --filter @codex-pet/desktop test -- notifications`。
- [ ] **3. 实现。** 复用任务 3/4 已验证的路径，URI 参数编码、来源安装信息验证和冷启动处理各自归适配器所有；不添加批准、回答或取消任务的接口。
- [ ] **4. 验证。** 两平台每来源至少 3 个会话，混合正在运行、等待审批、已结束状态；从气泡和历史点击，实际目标会话均正确。CLI 降级清晰，原应用处理请求后桌宠解除提示；验证接入真实链路而非仅调用 mocked opener。
- [ ] **5. 提交。** `git commit -m "接通通知点击与原会话精确定位"`。

### Task 12:双平台安装包、故障演练与发行验收

**文件：** 修改 `.github/workflows/desktop-pet.yml`、Tauri 平台配置、根 package 脚本、`README.md`、来源 README；新建 `scripts/desktop-pet/{validate-release,prepare-resources}.mjs`、`docs/validation/desktop-pet/release-checklist.md`、必要安装测试样例。

**接口：** `validate-release.mjs --artifacts PATH --reports PATH` 验证三种架构产物、独立转发器、资源清单和兼容报告完整性；缺失真实 GUI 报告时退出非零，不能把构建成功变为发行验收通过。`prepare-resources.mjs` 只复制发行所需素材到构建目录，原 pet-output 不变。

- [ ] **1. 定义失败验收。** 缺转发器、错架构、缺 macOS/Windows 运行报告、缺兼容版本、源码资源被覆盖均拒绝发布包；全新 Windows 用户目录、WebView2 缺失/下载失败、中文空格安装路径、升级后旧 Hook 路径和卸载保留第三方配置列为真实安装用例。
- [ ] **2. 运行并确认失败。** `node scripts/desktop-pet/validate-release.mjs --artifacts tests/fixtures/release/incomplete --reports tests/fixtures/release/incomplete-reports`，应退出非零并列出缺项；补充脚本行为测试，不测试静态配置文本。
- [ ] **3. 实现分发。** macOS arm64/x64 应用包和 DMG、Windows x64 每用户 NSIS 包，随包带目标架构 bridge。Windows 安装器处理 WebView2 并报告失败；测试产物清楚标为未签名。CI 只构建/上传产物，公开发布和证书接入另行安排。
- [ ] **4. 完成全链路验收。** `cargo fmt --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`、`pnpm test`、`pnpm typecheck`、`pnpm build`；各原生构建机运行 `pnpm --filter @codex-pet/desktop tauri build --target <本机验收目标>`。随后实测磁盘写入失败、响应丢失、重启、历史压缩、来源升级失配、休眠和安装卸载；测正常延迟 P95<500 ms、未聚焦动画 60 秒和三模式各 5 分钟资源。Windows 10/11 与 macOS 13+ 的实际覆盖逐项记录，未验证环境不能填通过。
- [ ] **5. 提交与交付。** `git commit -m "添加双平台打包与桌宠发行验收"`；交付安装包、准确的兼容矩阵、性能报告、接入/撤销说明和剩余限制；不擅自推送、发布或合并。

## 设计覆盖与计划自审

| 设计章节 | 对应任务 |
| --- | --- |
| 目的、方案、范围和默认环境 | 全局约束、检查点 A、任务 2/12 |
| 用户行为和通知 | 任务 1/9/10/11 |
| 进程、事件入口和持久化 | 任务 3/5/6 |
| 统一事件和多会话状态 | 任务 1/5/6 |
| Codex 适配 | 任务 3/6/11 |
| Harness 适配 | 任务 4/6/11 |
| 动画和包导入 | 任务 7/8 |
| 窗口、平台分发 | 任务 2/9/12 |
| 模块划分和验收矩阵 | 目录表、全部任务、任务 12 |

自审项目：设计需求均有任务归属；共享字段和函数名在任务间一致；步骤区分测试、红灯、实现、绿灯及提交；五项审阅重点都有所属行为测试；未把探测成功或构建成功提前写成真实接入通过。只有直接相互依赖的组件放在本计划，未扩大成聊天客户端或远程代理平台。

## 执行交接

计划完成后，由用户审阅并选择执行方式：当前会话逐任务实施，或子代理逐任务实施并逐任务独立审阅。建议当前会话执行，因为来源探测、协议、持久化和桌面状态共享接口较多，按顺序推进更容易保持一致；关键路径仍必须先经过真实双平台验证。

当前检查点 A 未运行，Windows 交互验收环境尚未确认。原型粗估 5～8 人日、日常可用版 15～25 人日，按检查点 A 结果修正；本计划不是完成时间承诺。

参考：[Tauri Vite](https://v2.tauri.app/start/frontend/vite/)、[Tauri 测试](https://v2.tauri.app/develop/tests/)、[Codex Hooks](https://learn.chatgpt.com/docs/hooks)、[Node 发布计划](https://github.com/nodejs/Release#release-schedule)；其余接口依据见设计文档及本机 Harness 源码。
