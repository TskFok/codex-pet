# Codex 接入验证记录

记录日期：2026-11-12。环境 macOS 27.0.1（26A434）、arm64。独立 CLI 为 `0.145.0`，桌面内置二进制为 `0.167.0-alpha.4.1`（`/Applications/ChatGPT.app/Contents/Resources/codex-cli/bin/codex`）。旧 `alpha.3.2` schema 仅作历史，不能替代当前验证。

当前结果是**转换器和安全安装器可测试，完整桌面接入未通过检查点 A**。自动测试、生成协议、文档支持和真实宿主事件分别记录，不能互相替代。

| 能力 | 自动转换/安装 | 独立 CLI 实测 | 已有桌面实测 | Windows |
| --- | --- | --- | --- | --- |
| 提交开始 | 合成 fixture 通过 | 隔离 CLI 可启动，Hook 待正常信任 | 未验证 | 未验证 |
| Stop | 转换 Unknown，通过 | 未触发验证 | 未验证 | 未验证 |
| Interrupt | 转换 Interrupted，通过 | 未触发验证 | 未验证 | 未验证 |
| 审批出现与精确解除 | 未实现：缺公开关联链路 | 未验证 | 未验证 | 未验证 |
| 提问、回答/拒绝/取消/撤回 | 保守忽略，负面测试通过 | 未验证 | 未验证 | 未验证 |
| 继续执行 | 不从任意工具事件推断 | 未验证 | 未验证 | 未验证 |
| 安装、重复安装、升级路径、仅撤销自有条目 | 本机文件/进程测试通过 | 隔离配置安装通过 | 未改真实配置 | 原生测试待执行 |
| 正常可靠投递 | 留任务 6 | 未接入 | 未接入 | 未接入 |
| 现有宿主补充连接 | schema 有请求/解除结构 | daemon 控制 socket 缺失 | 未连到现有桌面 | 未验证 |
| `codex://threads/<id>` 精确定位 | 不在转换器实现 | 0/3 会话验证 | 0/3 会话验证 | 未验证 |

## 已取得的具体证据

独立 CLI 与桌面内置二进制现场执行 `--version` 得到上列版本。新版桌面二进制执行只读 `app-server daemon version` 返回 `failed to connect to /Users/ushopal/.codex/app-server-control/app-server-control.sock`，原因 `No such file or directory (os error 2)`。没有启动 daemon、远程控制或另起 app-server 来代替当前宿主观察。

使用独立临时 CODEX_HOME、独立测试 cwd 和明确测试提示词进行 CLI 探测，不复制真实 auth/config。Ollama 方案因无本地服务失败；之后隔离离线 provider `http://127.0.0.1:9/v1` 运行到 `thread.started`、`turn.started` 后因502失败。它们是 exec JSON 事件，**不是 Hook 已触发的证据**。`--ignore-user-config` 的试验也不能用作用户层 Hook 发现已成功的证明。

独立 CLI 的隔离终端最终显示 `Hooks need review`，界面提示3个新/修改 Hook 和继续但不信任则不会运行。JSON 配置是4个事件，二者差异未解释，不能宣称4个事件都已加载或运行。没有信任 Hook。测试目录的目录信任提示仅影响该隔离配置。可审阅测试包列出具体命令、文件、哈希、写入与网络边界；后续正常信任和一次测试需用户明确授权，未收到授权不得执行。

测试配置仅指向离线 provider，不使用真实模型；Codex 自身仍会尝试远程插件目录（未登录401），不能称完全无网络。没有记录真实会话提示词、工具参数或凭证。

## 版本化字段依据

[官方 Hooks 文档](https://learn.chatgpt.com/docs/hooks) 是发布行为参考。锁定 `rust-v0.145.0` 的 [PermissionRequest 源码](https://github.com/openai/codex/blob/rust-v0.145.0/codex-rs/hooks/src/events/permission_request.rs) 中，内部 `run_id_suffix` 没有进入 `build_command_input` 的 stdin 字段；[PostToolUse 源码](https://github.com/openai/codex/blob/rust-v0.145.0/codex-rs/hooks/src/events/post_tool_use.rs) 虽输出 `tool_use_id`，不能凭内部名称为前者造一个对应 ID。

[Windows command runner](https://github.com/openai/codex/blob/rust-v0.145.0/codex-rs/hooks/src/engine/command_runner.rs) 与[官方测试](https://github.com/openai/codex/blob/rust-v0.145.0/codex-rs/hooks/src/engine/command_runner_tests.rs) 证明该版本的默认 COMSPEC/cmd /C 与外层引用方式；不证明 alpha.4.1 Windows 已实测。公开匹配 Hook 存在并发，不能凭收到时间构造可信跨调用序号。

[App Server 文档](https://learn.chatgpt.com/docs/app-server) 和本机生成 schema 只证明协议存在请求、结束、请求解除结构。没有现有宿主连接时，不启用这类输入，不宣称已观察审批或提问。

`integrations/codex/fixtures/*.json` 明确标记为合成或负面案例，不是真实 Hook 采集。开始/审批/停止的形状以0.145.0源码核对；Interrupt仅来自当前文档，版本留空，不能说0.145.0支持。假设性的取消响应不是已经观察到的宿主行为。

## 身份、隐私与未决项

安装 UUID 持久保存在配置/清单，重复安装和路径升级保留。公开 Hook 的 `hook-session:<session_id>` 实例兜底仅代表会话观察范围，不代表宿主进程生命周期。无顺序、快照、心跳或断线推断；子代理暂时跳过，避免父子会话混淆。原始输入上限8 MiB；统一事件上限64 KiB；不转发提示词、工具参数、回答正文、cwd 或 transcript。

缺口需主控裁定：先由用户正常信任具体隔离 Hook，再验证真实 CLI 事件；若审批/提问/解除仍无法从受支持入口获得，必须修订接入设计或暂缓完整 Codex 支持。启用受支持的现有宿主控制入口属于另一项需明确授权的真实配置变化，不能擅自执行。Windows 原生验收和至少3个真实会话深链精确定位均未完成。

## 新文档事件与锁定版本的差异

锁定 [0.145.0 的事件表](https://github.com/openai/codex/blob/rust-v0.145.0/codex-rs/hooks/src/lib.rs) 仅列11项，缺少 `Interrupt`、`PreModelRequest`、`PostModelRequest`。隔离信任界面显示3项而配置含4个事件，可能与Interrupt不在旧事件表有关，尚不能断言实际加载机制。安装器模板保留当前文档候选Interrupt，不代表旧CLI支持它。

当前文档的模型尝试事件不能作为“同会话全部待处理已解除”的已知屏障：0.145.0没有这些事件；alpha.4.1未取得该事件真实覆盖及等待审批/提问期间不会并发进入模型阶段的契约。重试与子代理作用域也需区分。不实现清空pending，不凭本机接收顺序替代源端因果。

提问应独立判断：[PreToolUse](https://github.com/openai/codex/blob/rust-v0.145.0/codex-rs/hooks/src/events/pre_tool_use.rs) 与PostToolUse均序列化tool_use_id；[request_user_input](https://github.com/openai/codex/blob/rust-v0.145.0/codex-rs/core/src/tools/handlers/request_user_input.rs) 使用默认工具运行接口，[registry](https://github.com/openai/codex/blob/rust-v0.145.0/codex-rs/core/src/tools/registry.rs) 仅成功结果执行post。取消或不可用模式会返回错误，pre不证明已经呈现问题，单靠pre/post不保证取消解除。它是值得实测的独立候选，状态保持Unverified，而非因PermissionRequest缺ID断定不支持。
