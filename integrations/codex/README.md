# Codex 桌宠 Hook 探测接入

当前交付转换器、显式安装/撤销工具和脱敏探测模式。正常通知投递留待任务 6；当前现有桌面会话的完整审批、提问、解除和精确跳转尚未验证，不能据此宣称完整 Codex 支持。版本与逐项证据见 [compatibility.json](compatibility.json) 和[验证记录](../../docs/validation/desktop-pet/codex.md)。

## 先生成可审阅计划

不会默认读取或修改真实用户配置。`--config` 与 `--bridge` 使用绝对路径；目标父目录须已存在。推荐先在临时测试目录执行：

```sh
cargo build -p pet-bridge
PET_TEST_ROOT="$(mktemp -d)"
PET_BRIDGE="$PWD/target/debug/pet-bridge"
"$PET_BRIDGE" hooks plan-install --config "$PET_TEST_ROOT/hooks.json" --bridge "$PET_BRIDGE" --plan "$PET_TEST_ROOT/install-plan.json"
```

标准输出是合并后的 Hook JSON；计划文件包含原始内容 SHA-256、修改后字节和自有条目哈希。生成计划不会写 `hooks.json`。`hooks.template.json` 仅作结构示例，路径与安装 ID 必须由安装器生成。

审阅后显式应用：

```sh
"$PET_BRIDGE" hooks apply --config "$PET_TEST_ROOT/hooks.json" --plan "$PET_TEST_ROOT/install-plan.json"
```

安装器保留第三方字段和条目，应用前、提交前各核对配置哈希。同目录独占锁防止多个安装器交叠；以 `0600` 创建备份、清单、临时文件（Unix），原子替换后同步目录。崩溃遗留的锁必须确认没有安装操作正在运行后人工清理。用户编辑器不遵守此锁，最后一次检查与文件替换间仍有极短竞态窗口；这不是跨进程文件系统 CAS。

应用生成 `.hooks.json.pet-bridge.<事务 UUID>.backup.json` 与 `.manifest.json`。备份只供审查和人工恢复，撤销不会直接恢复旧备份。重复安装不增加条目；升级二进制路径时保留安装 UUID，仅替换完整匹配生成格式的旧条目。修改过的自有条目会阻止自动安装升级，避免覆盖用户改动。Codex 通常需要重新审阅变更后的 Hook。

## 正常信任与探测

非托管 Hook 必须由用户在原应用的信任界面审阅；CLI 使用 `/hooks`，桌面在 Hooks 设置或提示处审阅。安装器不写宿主信任状态，不改安全设置，不返回批准或拒绝。所有 Hook 调用（包括无效 JSON、超限输入、文件写入错误和参数错误）在 stdout 只输出 `{}`，诊断只在 stderr。

当前安装命令没有正常投递目的地，执行会诊断“通知投递尚未接入”。要独立检查归一化行为，可明确指定探测目录：

```sh
printf '%s' '{"hook_event_name":"UserPromptSubmit","session_id":"test-session","turn_id":"test-turn","prompt":"仅测试"}' |
  "$PET_BRIDGE" codex --probe-dir "$PET_TEST_ROOT/events"
```

要验证宿主真实 Hook，需在隔离配置的命令后明确添加 `--probe-dir '绝对测试目录'`，再由用户正常信任**修改后的具体命令**。这属于测试配置，修改后的条目不会匹配原安装清单；保留原配置与测试配置的哈希，测试结束清理隔离目录。不要自动安装到真实 `$CODEX_HOME`、绕过信任或为制造审批修改真实安全设置。

探测目录仅当前用户可访问；不使用已开放组/其他用户权限的现有目录。保存 `.installation-id` 和 UUID 命名的统一事件，不保存原始 Hook、提示词、工具参数、消息正文、cwd 或 transcript。输入完整读取至 8 MiB 上限，先归一化再应用 64 KiB 统一事件限制，超限不截断 JSON 后继续解析。没有可靠事件摘要时省略摘要。

## 事件与身份边界

- `UserPromptSubmit` → `turn_started`；`Stop` → `turn_ended/unknown`；候选 `Interrupt` → `turn_ended/interrupted`。锁定0.145.0源码事件表没有Interrupt，当前文档与实际版本须分别核对，不能声称旧CLI支持。`Stop` 不证明任务成功，也不能排除其他 Hook 要求继续执行。
- `PermissionRequest` 没有公开可与解除事件配对的请求 ID。本实现不会造出待处理请求，也不会用工具完成、用户新消息、超时或静默伪造解除。提问、回答、拒绝、取消、撤回和继续执行仍有能力缺口。
- 安装 ID 是安装计划生成并写入命令的 UUID，重复安装、二进制路径升级不变。`--instance-id` 可由以后已验证的宿主连接显式提供。
- 公开 Hook 无宿主进程生命周期 ID，默认 `instance_id = hook-session:<session_id>` 只表示“会话观察范围”，不代表宿主进程实例或重启检测。未给安装 ID 的手工探测将 UUID 持久化到探测目录。
- `sequence` 与 `upstreamEventId` 保持缺省；不从本机时间戳或 Hook 到达顺序捏造上游顺序。每次 Hook 调用有独立事件 UUID；后续可靠投递必须留存并复用该统一事件。
- 当前跳过带 `agent_id` 的 Hook 和子代理事件，避免将父 `session_id` 误归成子代理会话。不产生快照、断线、心跳或存活推断。

## 只撤销仍匹配的自有条目

使用**最近一次已应用安装/升级**的计划或同目录生成的安装清单：

```sh
"$PET_BRIDGE" hooks plan-remove --config "$PET_TEST_ROOT/hooks.json" --manifest "$PET_TEST_ROOT/install-plan.json" --plan "$PET_TEST_ROOT/remove-plan.json"
"$PET_BRIDGE" hooks apply --config "$PET_TEST_ROOT/hooks.json" --plan "$PET_TEST_ROOT/remove-plan.json"
```

先审阅计划再应用。用户后来新增或修改的第三方条目保留；用户修改过的自有条目也保留，需人工检查。不得用旧备份覆盖当前配置。

## Windows 命令

生成的 `commandWindows` 调用 `powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand`。UTF-16LE 编码脚本使用调用运算符 `&` 和转义后的单引号路径，把 `codex`、`--installation-id`、UUID 作为独立参数，继承标准输入/输出。它不关闭 ExecutionPolicy、不提权、不禁用安全设置。锁定 CLI 0.145.0 的官方命令运行器使用 `COMSPEC` / `cmd.exe /C`；额外编码避免中文、空格、单引号和 shell 元字符成为外层命令结构。

Windows 原生执行测试已写入 `tests/hook_install.rs`，本机 macOS 未执行；桌面 alpha.4.1 的 Windows shell 行为与真实 Windows 接入仍待验。PowerShell 启动开销与 Interrupt 的 1 秒默认上限也须实测。
