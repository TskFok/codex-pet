# Codex 分级接入验证

更新：2026-11-16。用户已确认按来源分级支持：Codex 提供有证据的生命周期提醒和会话跳转，**当前适配器不支持精确审批/提问等待及解除同步**。Harness 完整待处理目标保留。代码审阅门与真实运行验收分开，未验项继续进入检查点 A 与任务 11/12 发行阻断清单。

## 当前能力与证据

| 能力 | 实现/源码 | 真实运行证据与限制 |
| --- | --- | --- |
| 根开始 UserPromptSubmit | 已实现 | 仅0.145.0隔离CLI一次开始Hook已验证；不外推其他来源/版本 |
| Stop | Unknown结束，非业务成功 | 未触发验证；其他Hook可能要求继续执行 |
| Interrupt | 根回合Interrupted | 0.145.0不支持；历史alpha4、当前alpha11源码存在；未实测 |
| ThreadSpawn子代理 | 实际agent_id隔离，子自身turn，有限开始/正常结束 | 仅精确源码和合成测试；内部/系统代理、部分终止、子代理取消不保证 |
| 直接父关系 | 当前原生Hook不给直接父ID，parent留空 | 不把共享根ID冒充直接父，不猜嵌套关系；核心/Harness保留真实父关系能力 |
| 审批/提问/回答/拒绝/取消/撤回精确同步 | 当前适配器不支持，不默认安装PermissionRequest | 不以负面测试或其他工具事件伪造待处理/解除 |
| 安装/重复/路径升级/旧配置迁移/撤销 | 本机文件和进程行为测试 | 未改真实来源；新五事件配置未真实安装或信任 |
| 正常可靠投递 | 留任务6 | 当前只有probe-dir |
| 精确会话跳转 | 留任务11 | 真实0/3会话，两平台未验 |
| Windows/现有桌面/Intel覆盖 | 实现/源码不等于运行验收 | 未完成，继续A/11-12，不标通过 |

## 准确版本身份依据

当前桌面内置CLI为0.169.0-alpha.11.1，独立CLI为0.145.0（沿用主控2026-11-16研究的版本依据，本轮无新宿主探测）。0.167.0-alpha.4.1是2026-11-12历史基线，不能当当前进程事实。

alpha11 [`core/src/session/session.rs:775-782`](https://github.com/openai/codex/blob/rust-v0.169.0-alpha.11.1/codex-rs/core/src/session/session.rs#L775) 区分具体thread_id与根/后代共享session_id，1090附近说明session_id为根线程ID。Hook载荷没有直接parent_thread_id，内部ThreadSpawn的父字段没有被序列化。[schema](https://github.com/openai/codex/blob/rust-v0.169.0-alpha.11.1/codex-rs/hooks/src/schema.rs#L550) 中SubagentStart/Stop只有session_id、agent_id和自身turn_id等字段。

[`hook_runtime.rs:137-160`](https://github.com/openai/codex/blob/rust-v0.169.0-alpha.11.1/codex-rs/core/src/hook_runtime.rs#L137) 证明ThreadSpawn Startup/Fork派发SubagentStart，turn_id来自当前子turn_context.sub_id；[正常停止:383](https://github.com/openai/codex/blob/rust-v0.169.0-alpha.11.1/codex-rs/core/src/hook_runtime.rs#L383) 派发SubagentStop；[agent身份:1100](https://github.com/openai/codex/blob/rust-v0.169.0-alpha.11.1/codex-rs/core/src/hook_runtime.rs#L1100) 为sess.thread_id；[UserPromptSubmit:708](https://github.com/openai/codex/blob/rust-v0.169.0-alpha.11.1/codex-rs/core/src/hook_runtime.rs#L708) 同样使用该子turn上下文与ThreadSpawn身份。故统一sessionId取agent_id、turnId保留自身turn；parentSessionId留空，不读取伪造parent字段或转录路径。

内部/系统子代理跳过这套开始/停止路径，Interrupt对任何SubAgent来源直接返回，不能扇出根中断。独立核对的[0.145.0 producer](https://github.com/openai/codex/blob/rust-v0.145.0/codex-rs/core/src/hook_runtime.rs#L103)只证明Startup开始与正常SubagentStop；该版本[session getter](https://github.com/openai/codex/blob/rust-v0.145.0/codex-rs/core/src/session/session.rs#L460)同样区分共享根与具体线程，不把alpha11 Fork语义倒推旧版。

历史alpha4准确tag事件表含14项（Interrupt/模型尝试事件）；0.145事件表11项、不含Interrupt/模型事件。当前alpha11也有14项，但“进入下一模型阶段等于全部pending解除”的契约没有建立，未采用清空屏障。[准确版本PermissionRequest](https://github.com/openai/codex/blob/rust-v0.169.0-alpha.11.1/codex-rs/hooks/src/events/permission_request.rs#L175)仍没有可公开配对的请求ID；Hook出现也不保证审批UI已成为待处理。App-server协议有真实请求关联，但没有已证明的当前桌面受支持观察连接，不另启server代替。

## 已有真实试验（2026-11-12，不在本轮重复）

macOS 27.0.1 / 26A434 / arm64。用户授权原临时四事件包后，0.145.0正常TUI逐项信任PermissionRequest/UserPromptSubmit/Stop，各Installed1/Active1；未加载Interrupt。固定提示词ephemeral CLI只运行一次，provider为127.0.0.1:9，不调用真实模型。退出1，exec类型thread.started/item.completed/turn.started/error/turn.failed。真实probe新增1条378字节turn_started事件，不含提示词、参数或回答；`fixtures/live-start-cli-0.145.json`仅保留脱敏统一事件，rawHookCaptured=false。

当时hooks.json SHA-256为`22533cc31c285cdfacf32ba2a9b3b98ecd4a3939f3b02e44eaeba0e974d58dec`，测试bridge SHA-256为`511a431f083fc1980c7297276b6872bc65a35fe35d93990afe75e6927d9e3a2b`。这些是旧测试包/二进制历史，不能声称本轮重编译仍相同；本轮未执行或更新旧临时Hook。

此前无Ollama服务、离线provider失败、exec输出及生成schema都不算真实Hook证据；TUI与测试进程已退出。Codex曾尝试官方插件目录，未登录401，不能称完全无网络。2026-11-12与主控11-16只读daemon探测均无控制socket，不是已接入现有桌面的证据；本轮不重复该探测、不启daemon/远控、不改真实安全设置。

## 本轮代码门与后续验收

默认只装实际处理的UserPromptSubmit/Stop/Interrupt/SubagentStart/SubagentStop。旧v1未改PermissionRequest自有项在计划中迁移移除，保留UUID、第三方与用户修改内容；新增配置重复安装、路径升级和按最新清单撤销有行为测试。不将宿主忽略旧版未知事件当运行支持。

父子/兄弟/嵌套ID独立、子自身turn与父请求保留、无直接父关系不猜、重复结束/迟到开始、空/非法agent、CLI子会话观察范围、默认订阅与旧迁移由合成数据和真实本机进程/文件测试覆盖，不标实测子代理Verified。无序投影可Unknown，明确终态历史仍保留，`confirmed_turn_ends`仍需Task5持久化。

Windows原生包装/编码/启动时间、macOS Intel、真实桌面停止/中断/子代理覆盖，以及至少3会话精确导航仍未验证，留检查点A/任务11-12发行门。代码可按用户批准范围继续独立后续任务；不能填成两个平台均可用。
