# Codex 桌宠 Hook 接入

首版按用户确认的分级范围提供生命周期提醒。**当前适配器不支持 Codex 精确审批、提问等待及其回答/拒绝/取消/撤回解除同步**，不会生成猜测的请求事件；Harness 的完整待处理目标与通用请求协议保留。正常投递由任务 6 接入，目前提供转换、显式安装/撤销及脱敏 probe-dir。能力证据见 [compatibility.json](compatibility.json) 和[验证记录](../../docs/validation/desktop-pet/codex.md)。

| 能力 | 当前实现和证据等级 |
| --- | --- |
| UserPromptSubmit 开始 | 已实现；仅 0.145.0 隔离 CLI 的一次真实 Hook 已验证 |
| Stop / SubagentStop | 已实现 Unknown 结束；不等于任务成功；真实停止未验证 |
| Interrupt | 已实现根回合中断；0.145.0 无此事件，alpha4/alpha11 有准确源码；未实测 |
| ThreadSpawn 子代理 | 有限身份与正常开始/结束映射；源码和合成测试，未实测 |
| 精确待处理同步 | 当前适配器不支持，默认不订阅 PermissionRequest |
| 精确会话跳转 | 任务 11 待实现/验收，真实验证 0/3 |
| Windows / 现有桌面覆盖 | 未验证，保留到检查点 A 与任务 11/12 的发行门 |

## 身份与生命周期边界

根事件使用公开 `session_id`；带 `agent_id` 的 UserPromptSubmit 与 SubagentStart/SubagentStop 使用实际子线程 ID 作为统一 `sessionId`，`turnId` 保留该子线程的 `turn_context.sub_id`。Stop、SubagentStop 均转为 Unknown 结束；不把普通 Stop 或主线程 Interrupt 扇出到全部子代理。

准确 0.169.0-alpha.11.1 源码说明原始 `session_id` 是根与全部后代共享的身份，不是直接父线程 ID。当前载荷没有直接父 ID，故 `parentSessionId` 留空；嵌套子代理不猜父，不采信未公开的 parent 字段，也不从 transcript 路径推导。通用核心与 Harness 的真实父关系/折叠能力不变。

ThreadSpawn 在 alpha11 的 Startup/Fork 可产生 SubagentStart，正常子回合结束产生 SubagentStop；内部/系统子代理及部分终止路径没有这套覆盖，子代理 Interrupt 被 producer 排除。独立核对的 0.145.0 源码只证明 Startup 开始与正常 SubagentStop，不能把 Fork 覆盖倒推旧版。子代理取消、异常退出、所有内部代理和直接父关系都不保证；源码存在不等于桌面已触发。

安装 ID 由计划生成 UUID，写入命令/清单，重复安装和路径升级保留。CLI 默认观察范围为 `hook-session:<统一 sessionId>`，因此根、多个子线程不会共享根作用域；同一个子线程多次 Hook 保持稳定。`--instance-id` 可显式提供已验证宿主的运行实例。这个兜底不表示宿主进程生命周期、存活或重启。

`sequence` 和 `upstreamEventId` 不造值；无序多轮当前投影可能 Unknown，但每个明确 turn 的结束事实保留独立历史/提醒，精确去重，不重开已知终态，不清其他回合 pending/未读。Task5 必须持久化私有 `confirmed_turn_ends` 等因果标记，不能只存公开快照。不生成快照、心跳或断线推断。

## 生成并显式应用安装计划

默认只订阅实际处理的五个生命周期：UserPromptSubmit、Stop、Interrupt、SubagentStart、SubagentStop。宿主版本可能忽略不支持的事件；安装成功不等于来源能力验证。

```sh
cargo build -p pet-bridge
PET_TEST_ROOT="$(mktemp -d)"
PET_BRIDGE="$PWD/target/debug/pet-bridge"
"$PET_BRIDGE" hooks plan-install --config "$PET_TEST_ROOT/hooks.json" --bridge "$PET_BRIDGE" --plan "$PET_TEST_ROOT/install-plan.json"
```

`--config` 和 `--bridge` 必须为绝对路径，父目录已存在。stdout 是合并后的配置供审阅；计划包含原内容 SHA-256、修改后字节和自有条目哈希，不写 hooks.json。模板仅供结构审阅，须用安装器生成实际路径与 UUID。

审阅后显式应用：

```sh
"$PET_BRIDGE" hooks apply --config "$PET_TEST_ROOT/hooks.json" --plan "$PET_TEST_ROOT/install-plan.json"
```

重复安装不重复；路径升级保留 UUID。升级会识别旧 v1 完整未改的四事件配置，移除自有 PermissionRequest、添加两个子代理事件，保留第三方审批 Hook；可能保留空的 PermissionRequest 数组，但没有本适配器命令。旧自有条目带保留标记但内容被修改时明确报冲突，避免覆盖；无法再识别为自有的条目按外来内容保留。迁移后用**最新已应用计划/清单**撤销全部当前条目，不拿旧清单代替升级清单。

应用前和提交前各核对内容哈希，同目录独占锁防止安装器交叠；Unix 以 0600 创建备份/清单/临时文件，原子替换并同步目录。生成 `.hooks.json.pet-bridge.<UUID>.backup.json` 与 `.manifest.json`。不遵守锁的外部编辑器仍有检查后竞态，不声称文件系统 CAS；崩溃遗留锁需确认无安装器运行后人工清理。

## 正常信任与脱敏探测

不会默认读取或更改真实用户 Hook/安全设置。非托管 Hook 必须由用户在来源正常界面审阅/信任（CLI `/hooks`，桌面 Hooks 设置或提示）；安装器不写信任状态，不返回批准/拒绝。Hook stdout 固定 `{}`，错误只 stderr，不含原始 JSON。正常投递未接入时明确诊断。

```sh
printf '%s' '{"hook_event_name":"UserPromptSubmit","session_id":"test-session","turn_id":"test-turn","prompt":"仅测试"}' |
  "$PET_BRIDGE" codex --probe-dir "$PET_TEST_ROOT/events"
```

真实 Hook 验证要在明确授权的隔离配置命令后添加 `--probe-dir '绝对测试目录'`，再正常信任修改后的具体内容。测试修改会使原安装清单不匹配，须保留具体配置/哈希并仅清理隔离目录；不能自动装到真实来源或绕过信任。

probe 目录仅当前用户访问，保存 `.installation-id` 与 UUID 事件文件，不保存原始 Hook、提示词、参数、消息正文、cwd 或 transcript。输入上限 8 MiB，先归一化再检查 64 KiB 事件上限；超限 JSON 不截断解析，无可靠摘要时省略。

2026-11-12 已授权的一次隔离 CLI 测试只观察到一条 378 字节开始事件。`fixtures/live-start-cli-0.145.json` 是身份/时间脱敏的统一事件，非原始 Hook。其余 fixture 均注明合成/负面证据；alpha11 子代理 fixture 未标实测。本轮没有重复或扩用该授权，旧临时四事件配置未迁移、未运行。

## 按当前清单撤销

```sh
"$PET_BRIDGE" hooks plan-remove --config "$PET_TEST_ROOT/hooks.json" --manifest "$PET_TEST_ROOT/install-plan.json" --plan "$PET_TEST_ROOT/remove-plan.json"
"$PET_BRIDGE" hooks apply --config "$PET_TEST_ROOT/hooks.json" --plan "$PET_TEST_ROOT/remove-plan.json"
```

先审阅再应用。只删除最新安装清单内容哈希仍匹配的自有条目；第三方更新、后来新增配置和改过的自有条目保留。不以旧备份覆盖当前文件。

## Windows

`commandWindows` 使用 `powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand`，UTF16LE 脚本以转义单引号路径和独立参数调用 bridge。没有修改 ExecutionPolicy、提权或禁用安全设置。锁定 0.145.0 源码证明外层为 COMSPEC/cmd /C；中文/空格/shell 元字符不作为外层结构解释。Windows 原生 stdin/stdout、编码、启动时间、1秒上限及当前桌面版本 shell 均未验，原生测试不能由 macOS 结果或交叉构建代替。
