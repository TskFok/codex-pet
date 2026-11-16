# Codex分级接入的依据

日期：2026-11-16。用户已明确选择分级支持：Codex提供可验证生命周期提醒和会话跳转，完整待处理提醒先由DeepSeek Harness提供。下述研究未改宿主配置、运行循环或安全设置，不代表真实桌面/Windows验收已经通过。

## 首版边界

当前适配器处理UserPromptSubmit、Stop、Interrupt及有限ThreadSpawn开始/正常停止。Stop结果保持Unknown；旧CLI0.145不发Interrupt。精确审批/提问等待与解除明确不支持，不以工具完成、模型继续或日志猜测状态。源码、合成fixture与真实执行的证据等级分别记录于[兼容表](../../../integrations/codex/compatibility.json)和[验证记录](codex.md)。

独立CLI0.145已在一次明确授权的正常信任隔离测试中观察到开始Hook→bridge→probe；没有观察到完整停止、审批、提问或解除。当前桌面内置CLI为0.169.0-alpha.11.1，0.167版本仅作历史证据。当前桌面、Windows包装、子代理实际覆盖与至少三个会话精确导航仍待运行验收。

## 准确版本的关键证据

| 问题 | 已核对的边界 | 官方源码/文档 |
| --- | --- | --- |
| 审批Hook | PermissionRequest输入没有请求ID或tool_use_id；它发生在审批处理前，不证明用户框已等待 | [alpha11输入schema](https://github.com/openai/codex/blob/rust-v0.169.0-alpha.11.1/codex-rs/hooks/src/schema.rs#L302)、[Hooks](https://learn.chatgpt.com/docs/hooks#permissionrequest) |
| 异步提问 | 问题消息生成后即返回accepted，工具完成与模型继续不能表示用户已回答/取消 | [alpha11异步处理器](https://github.com/openai/codex/blob/rust-v0.169.0-alpha.11.1/codex-rs/core/src/tools/handlers/request_user_input_async.rs#L131) |
| 历史日志 | 正式rollout过滤审批/原生提问等瞬态事件，SQLite投影无法补回它们 | [alpha11持久化策略](https://github.com/openai/codex/blob/rust-v0.169.0-alpha.11.1/codex-rs/rollout/src/policy.rs#L162) |
| 服务器请求 | app-server有真实JSON-RPC请求ID和精确resolved，但当前桌面的受支持观察入口尚未建立 | [请求ID分配](https://github.com/openai/codex/blob/rust-v0.169.0-alpha.11.1/codex-rs/app-server/src/outgoing_message.rs#L384)、[解除通知](https://github.com/openai/codex/blob/rust-v0.169.0-alpha.11.1/codex-rs/app-server/src/outgoing_message.rs#L232) |
| 重连 | resume响应先于pending重放，且resume可改客户端信息/生命周期，不是纯只读快照完成屏障 | [alpha11恢复生命周期](https://github.com/openai/codex/blob/rust-v0.169.0-alpha.11.1/codex-rs/app-server/src/request_processors/thread_lifecycle.rs#L1256) |
| 子代理 | agent_id是具体线程，session_id由根与所有后代共享；公开Hook没有直接父ID，正常结束也不覆盖全部取消路径 | [线程与根身份](https://github.com/openai/codex/blob/rust-v0.169.0-alpha.11.1/codex-rs/core/src/session/session.rs#L775)、[生命周期范围](https://github.com/openai/codex/blob/rust-v0.169.0-alpha.11.1/codex-rs/core/src/hook_runtime.rs#L137) |

子事件使用真实agent_id和子turn_id，parent_session_id保持空；不把根伪装直接父。通用核心与Harness保留真实父层级和请求关联能力。

研究只在当前版本对默认控制socket做过一次只读查询，返回不存在；没有证明所有入口绝对不存在。未找到桌面受支持的自定义CLI启动入口；IDE的cliExecutable及桌面文件处理器不能作为该入口。另起server或自己管理CLI会话会改变接入对象，不算观察既有桌面。

## 后续完整接入的前提

需要原宿主提供受支持的只读请求镜像、真实request/item/agent身份、回答/取消/撤回精确终态及重连完整性边界，异步问题也必须覆盖。取得前置条件后再单独设计和验证；当前分级适配器不产生回答、批准或任务控制。真实平台与导航缺项继续阻断对应运行能力和最终发行结论。
