# Antigravity 桌宠 Hook 接入

本适配器按分级范围支持 Antigravity（Antigravity 2.0 桌面端、Antigravity IDE 与 `agy` CLI）的生命周期提醒。当前订阅 `PreInvocation` 与 `Stop`，不挂载会干预权限决策的 `PreToolUse`；**不支持精确审批/提问待处理同步**，原会话导航采用「打开应用 + 复制会话 ID」降级策略。能力边界见 [compatibility.json](compatibility.json)。

| 能力 | 当前实现与状态 |
| --- | --- |
| 回合开始（`PreInvocation`） | 已实现；仅在首个调用（`invocationNum <= 1` 或省略）时上报 `TurnStarted`，跳过同回合中间重复调用；未在真机宿主实测 |
| 回合结束 / 中断 / 失败（`Stop`） | 已实现；按 `terminationReason` 精确区分 `NO_TOOL_CALL`（`completed`）、`USER_CANCELED`（`interrupted`）、`ERROR`（`failed`），其余映射为 `unknown`；未在真机宿主实测 |
| 项目工作区识别 | 已实现；仅提取 `workspacePaths[0]` 的末级目录名作为展示标题，不持久化完整本机路径 |
| 精确审批 / 提问等待与解除 | 当前适配器不支持，默认不订阅 `PreToolUse` / `PostToolUse` |
| 原会话跳转 | Antigravity 桌面端无公开会话级 URL Scheme；默认降级为「打开应用」+「复制会话 ID」（`conversationId`） |

## 身份、隐私与契约边界

- 使用 Antigravity 公共 Hook 字段 `conversationId` 作为统一 `sessionId`，`Stop` 的 `executionNum` 映射为 `execution-<N>` 作为 `turnId` 以实现多回合独立提醒与去重。
- 绝不保存 `lastUserInput`、`transcriptPath`、`artifactDirectoryPath`、错误原文或完整工作区路径。
- Antigravity Hooks 为同步执行命令，适配器保持 `timeout: 1`，在 200 ms 内尝试投递至本机桌宠服务并持久化待发送区；无论成功或失败均输出符合契约的非阻断 JSON（`PreInvocation` / `SessionStart` 输出 `{}`，`Stop` 输出 `{"decision":"allow"}`），绝不卡死或阻断宿主循环。

## 生成并显式应用安装计划

默认写入 Antigravity 全局配置 `~/.gemini/config/hooks.json`（可通过 `ANTIGRAVITY_CONFIG_HOME` 覆盖根目录）下的独立命名键 `"codex-pet"`，保留同一文件中的其他命名 Hook；也可将 `--config` 指向插件目录或工作区 `.agents/hooks.json`。

```sh
cargo build -p pet-bridge
PET_TEST_ROOT="$(mktemp -d)"
PET_BRIDGE="$PWD/target/debug/pet-bridge"
"$PET_BRIDGE" antigravity-hooks plan-install \
  --config "$PET_TEST_ROOT/hooks.json" \
  --bridge "$PET_BRIDGE" \
  --plan "$PET_TEST_ROOT/install-plan.json"
```

审阅输出后显式应用：

```sh
"$PET_BRIDGE" antigravity-hooks apply \
  --config "$PET_TEST_ROOT/hooks.json" \
  --plan "$PET_TEST_ROOT/install-plan.json"
```

## 脱敏探测

```sh
printf '%s' '{"conversationId":"ec33ebf9-0cba-4100-8142-c61503f6c587","executionNum":1,"terminationReason":"NO_TOOL_CALL","workspacePaths":["/workspace/codex-pet"]}' |
  "$PET_BRIDGE" antigravity --event stop --probe-dir "$PET_TEST_ROOT/events"
```

## 按清单撤销

```sh
"$PET_BRIDGE" antigravity-hooks plan-remove \
  --config "$PET_TEST_ROOT/hooks.json" \
  --manifest "$PET_TEST_ROOT/install-plan.json" \
  --plan "$PET_TEST_ROOT/remove-plan.json"
"$PET_BRIDGE" antigravity-hooks apply \
  --config "$PET_TEST_ROOT/hooks.json" \
  --plan "$PET_TEST_ROOT/remove-plan.json"
```

仅删除内容哈希仍与安装清单一致的 `"codex-pet"` 条目，完整保留第三方 Hook 及用户手动修改后的配置。
