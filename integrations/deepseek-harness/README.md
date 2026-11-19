# DeepSeek Harness 桌宠接入

本包实现 Harness 审批、持续提问、精确解除和真实子会话元数据观察；正式 Host profile 加载、Client 工厂交付及真实 SDK→原生 bridge→Rust SessionStore 已验证。原生桌面三会话、冷热/隐藏精确定位和 Windows 尚未验收；正常配置通过原生 bridge 先耐久入队，桌宠关闭或通信中断时保留待发送项，桌宠启动后补发。原生 bridge 的成功退出表示接受事件，包括离线入队。

兼容 Harness `0.2.2-rc.1`，源码基线为 `fcc0ad462172522aaf5c93a7b2e01f6b9f0b0159`，已验证补丁提交为 `2809f54899deb4477517be5437521cd51b7c4a5e`；完整版本记录见 [compatibility.json](compatibility.json)。SDK/Cordis 采用锁定注册表版本及官方 peerDependencies 共享实例规则，无本机源码依赖。有 JSONL 写代次证明时使用协议 v3。旧宿主和暂未取得证明的会话继续使用 v2。接收端读 v1、v2 和 v3，v1 采用普通 Turn 语义。

Host 观察公开 `session/event` 的真实 turn/审批 seq。旧宿主及工具定时请求继续观察 `user-questions/request`，恰好调用一次 next，返回原答案并原样传播异常。所有可路由提问用 UntilResolved，出生回合在捕获时固定，无活动回合则省略。timed 超时不解除，晚到回复通过正式 SessionProjection 还原原始 callId/birth turn。投影只保留未 settle 调用及最后解除，生产代码不扫描弃用日志 API。父关系只取 `origin=subagent` 的真实 header.parentSession，不把普通 fork 猜成子任务。问题内容、答案、工具参数、工具名和提示词均不进入通知。

安装 ID 显式持久在配置中。旧协议的实例范围是 `session-log:<sessionId>:<header.createdAt>`。恢复协议使用 JSONL 写所有者的持久代次，序号来自会话原生日志，插件重载沿用原写代次。

有 [直接提问观察补丁](patches/direct-question-observation.patch) 的宿主公开接口版本为 1。补丁基于已验证的桌面跳转补丁提交。应用前核对 compatibility.json 的准确基线和目标文件状态，先执行 `git apply --unidiff-zero --check 补丁路径`，通过后去掉 `--check` 应用并按宿主文档构建。桌宠重载后恢复同一服务内带 Agent 且无工具等待的活动提问，通过明确的结束事件精确移除已回答、取消或失败的请求。服务保留最近 1,024 个结束事实，超出覆盖范围时保留待核实项。恢复按桥接器的接纳速度逐条转发，积压超出内存批次时重新读取宿主审计。暂时无法接纳会重试同一事件。配套 `pet-bridge harness` 以退出码 2 报告单条事件违反协议，桌宠记录该条恢复缺口并继续处理其他请求。观察器卸载后，在途发送结束，余下事实由下一次订阅重新读取。原发布版提供连续观察；补丁能力由宿主公开方法协商。

[JSONL 代次恢复补丁](patches/session-observation-recovery.patch) 在直接提问补丁之后应用。核对兼容表中的准确基线和目标文件，先用 `git apply --unidiff-zero --check 补丁路径`，检查通过后再应用并重新构建宿主。恢复现有会话时，宿主在写锁下读取物理有效前缀，再持久发布观察凭证。桌宠凭写代次隔离相同序号、相同回合 ID 和请求终态。已经观察到的旧通知仍留在历史，定时请求可从保留的出生事实重新显示。

单会话恢复消息报告观测切点及当前活动回合。只有宿主投影和所有相关出生事实完整、消息适合 64 KiB 上限时，才包含定时请求覆盖范围。普通提问的恢复由原问题服务另行提供明确终态，审批继续按已观察的原生日志事件处理。凭证缺失、损坏、超出最近 128 个写所有者的连续性窗口，或宿主未提供当前请求域证明时，按可证事实更新并保留待核实项。旧协议的延迟事件不能改写已确认的新代会话。

## 生成并安装配置包

先在本仓根执行 `pnpm install --frozen-lockfile`、`cargo build -p pet-bridge` 和 `pnpm --filter @codex-pet/deepseek-integration build`。代码包本身是库；下面命令向指定**不存在的新目录**复制已构建代码并生成真正的 `dsh.bundle` 安装包：

```sh
node integrations/deepseek-harness/create-install.mjs \
  --output '/绝对路径/桌宠安装包' \
  --bridge '/绝对路径/pet-bridge'
```

需要脱敏探测时，显式加 `--probe-dir "/绝对路径/私有探测目录"`。探测模式只写该目录，日常配置省略该选项。

首次生成一次 UUID 并写入安装包 `cordis.patch.json`；命令不安装插件、不写宿主 profile。新包保留固定注册表依赖，复制实际 dist，代码不指向本机源码/link。重复输出目录明确拒绝，避免覆盖已有 ID 或用户改动。升级在新目录生成，显式增加 `--installation-id '原配置UUID'`；保留原 ID，不重新生成一个安装身份。Windows bridge 指向原生 `pet-bridge.exe`。

在原应用的插件页选择安装第三方插件，输入生成包的绝对路径或预构建 tarball，审阅其中 bridgePath、probeDir、installationId 后安装/启用。配置包可在其目录执行 `pnpm pack --pack-destination /绝对路径/输出目录`；无 prepare/postinstall 脚本。桌面 profile 由应用独占，禁止用 CLI 管理 `profiles/desktop`。独立 CLI 测试 profile 可用官方 `pnpm dsh plugin --profile 测试名 add 安装包路径`，启动使用 `pnpm dsh --profile 测试名 --no-open`；不要再叠加 `web` shorthand 选择两次 profile。

从开发期 plain 库迁移为配置 bundle 时，先移除同名自有 plain 依赖再安装，或显式选择新 bundle 层；替换一个已存在的依赖会保留原 bundle 选择，不等于自动启用新层。正常已有 bundle 升级也应检查原启用状态。

撤销在原应用插件页停用并移除本 bundle，只移除该包/配置层，不删除用户会话、其它插件或用户数据。升级前保存 UUID。探测文件和离线安装包是显式输出，可由用户另行清理；生成器不清理宿主目录。

## 精确会话入口

Host 无法直接调用 Client uiWorkspace：Remote 能力和事件均由构建期白名单选择。仅在宿主HEAD等于上述准确BASE、补丁涉及的9个目标文件均无用户改动（新增文件路径尚不存在）时应用 [桌面补丁](patches/desktop-session-deep-link.patch)，补丁为零上下文diff，执行 `git apply --unidiff-zero --check 补丁路径` 后再以相同参数去掉 `--check` 应用，按官方文档构建。撤销补丁先禁用本包，并确认HEAD为上述已验证补丁提交且目标文件无用户改动，再执行 `git apply --unidiff-zero --reverse --check 补丁路径`；检查失败时保留用户改动进行比对，不强行覆盖。

补丁在 whenReady 前排队 `dsh://session/<编码ID>`，处理 Windows second-instance；工作区未就绪或渲染重载时目标不提前消费。Client 通过经主窗口/主 frame/原点验证的 IPC 读取目标，保留引用并等待 sessions.retain(...).ready 成功后调用 uiWorkspace.openSession；不存在目标确认 unavailable，保留原选择。Client 来自本包正式模块工厂，没有私有反射或自造跨 Remote 事件。

`openHarnessSession(sessionId)` 默认返回 unavailable；调用方显式提供已确认安装补丁的 launcher 后才能请求。requested 只表示系统接收深链请求，不能代替实际 UI 定位证据。补丁的实际系统协议注册、浏览器 apply、冷热隐藏与 Windows 定位仍在发行阻断清单。

开发检查：根 `pnpm test`、`pnpm typecheck`、`pnpm build` 实际执行本包；Vitest 空集失败，build 会加载 Host/Client 构建产物验证入口。详见[验证记录](../../docs/validation/desktop-pet/harness.md)。
