# 桌宠窗口阶段一验证

## Task 9 月影窗口与托盘（2026-11-20 至 21）

本机为 macOS 27.0.1（26A434）、Apple Silicon、内置 Retina 2×，WebKit 22625.1.29.11.28，Tauri 2.12.4，应用 0.1.0。实测使用 debug 二进制，显式指定规范化的临时 `PET_DIAGNOSTICS_DIR` 和 `PET_VALIDATION_DATA_DIR`。最终 macOS 实验二进制 SHA-256 为 `0b30990226e9d8020dccc0abbfedee1455c57cf0b724f08c537c89691e4c03d7`。保留的月影 WebP SHA-256 为 `70a06d18162ad3b5344afdc7e28d4dbd70b70c0d32ab0bfcac52fe7315b63130`。

| 项目 | 已观察到的结果 |
| --- | --- |
| 月影点击与拖动 | 旧椭圆对 3,180 个首帧测试点有 649 个错配。真实桌宠改用 Canvas 姿态的非零 alpha 后，测试底板收到透明空隙点击。落在旧椭圆外的角色像素收到原生 mouseDown，系统拖动使保存位置从 (120,180) 变为 (174,158)。静止首帧的 28,124 个逻辑点均与 WindowServer 在该点选取的 Retina 子像素一致。 |
| 位置和缩放 | 以 1.5 倍尺寸和已消失屏幕的保存偏移 (90000,90000) 启动。应用在当前工作区保存了 (1272,679)，AppKit 父、子窗首份报告均为 (1272,0,240,270)，Canvas 物理尺寸为 480×540。 |
| 动画与原生命中 | 在 1.5 倍 running 动作上，从 ScreenCaptureKit 取出画面后查询 143 个 WindowServer 命中点。100 次采样有 87 次完全一致。两端各自与上次相同的 65 次全部一致，覆盖 5 个不同轮廓。其余 13 次处于切换附近，独立采样含时间差，双窗切换的逐帧原子性待验。 |
| 可见且未聚焦 | 同一 1.5 倍运行的首个完整 60.000 秒统计段记录 1,800 个绘制间隔，P95 35 ms，最大 45 ms，全部 ≤50 ms。 |
| 托盘与后台 | 系统已注册桌宠图标。本机折叠了新图标，实验副本调用该图标的原生 `show_menu()` 展开，再通过 AX 选择真实菜单项。隐藏时父、子窗均不可见，broker 仍确认事件，前台 PID 保持不变。显示时两窗恢复。主动选择通知与设置分别打开独立前台窗口。选择退出后进程及本次 discovery 均消失。图标的物理展开操作待验。 |
| Windows | 用 Rust MSVC x64 target、Apple Clang 及 Zig Resinator 完成 `cargo check --locked --target x86_64-pc-windows-msvc -p pet-desktop --all-targets`。仍需在 Windows 10 22H2 / 11 真机验收原生区域、WebView2 点击拖动、托盘、缩放、全屏和休眠。 |

物理屏幕热拔插、混合 DPI 实际跨屏拖动、锁屏、硬件休眠、独占全屏、macOS 13 和 Intel 仍待验证。常规 macOS/Windows 通知只走 broker 和事件订阅。任务 10 接通知、设置两个窗口的业务内容。

原始录制、输入底板、诊断和复现实验保存在本地 `.superpowers/sdd/2026-11-11-tauri-desktop-pet/task-9-evidence/`。位置测试包含负坐标副屏、显示器移除、100/150/200% DPI、工作区比桌宠小与临时空工作区。窗口位置单独存储在应用数据根的 `placement.json` 中，x/y 是指定显示器工作区左上角的逻辑偏移，scale 是角色倍率。

## 固定实验与实现前基线

2026-11-11：仓库没有桌面应用，macOS/Windows 五个用例均为「无可执行应用，未验证」。Windows 尚无已确认交互环境。不得以透明 CSS、单元测试或交叉编译替代系统行为验收。

| 用例 | 两系统相同操作 | 通过条件 |
| --- | --- | --- |
| 透明窄窗口 | 160×180 逻辑像素无边框窗口置于另一测试窗口上 | 可见几何角色与轨迹，外缘能看到下层，保持置顶 |
| 拖动 | 按住角色实体拖动 100 像素后释放 | 窗口跟随，释放后可再次拖动 |
| 外缘穿透 | 点击角色椭圆外透明区，下层测试窗口记录点击 | 下层收到点击；角色实体可拖动；两个行为分别记录 |
| 后台不抢焦点 | 先聚焦下层窗口，启动桌宠并等待 60 秒 | 下层保持焦点，桌宠诊断 focused=false |
| 未聚焦播放 | 角色可见，持续操作另一测试窗口 60 秒 | 至少 95% 绘制间隔 ≤50ms，独立报告间隔与 P95 |

资源实验：分别以静止（idle）、动画（animation）、隐藏（hidden）运行 5 分钟；记录主进程与 WebView 子进程 CPU/RSS、隐藏绘制计数。实际测量结果与锁定预算在后续记录，不以猜测当实测。

## 修复 round1（4739933）：窗口输入结构与历史验证

2026-11-12：移除光标/按键轮询和从采样推断的拖动锚点。点击路由由操作系统在真实按下时处理，拖动只由本次实际 mouseDown / PointerDown 触发。

- macOS：Tauri + Canvas 显示窗固定整窗穿透，原生 CPU 椭圆输入代理位于其下方。代理是 parent，Canvas 是 above child；WindowServer 拖动代理时整组移动。两窗同为不可聚焦窗口，显隐使用 `platform::set_pet_visible` 操作整组，关闭/Exit 清理代理关联与强引用。
- 代理底色与 Canvas 不透明实体一致，细节仍由 Canvas 显示。此为几何原型；Task9 须使用缓存 AlphaMask 并复核半透明边界。轨迹移入实体内部，避免在输入遮罩外绘图。
- Windows：以 `SetWindowRgn` 设置系统椭圆区域，按真实客户区物理尺寸与窗口/客户区偏移换算。实际 PointerDown 坐标、按钮和阶段在 Rust 入口验证后调用原生拖动。初始区域失败会使初始化失败；后续失败不删除回调、不提交缓存，下次窗口生命周期事件重试。
- 原有 GetCursorPos/GetAsyncKeyState/NSEvent.pressedMouseButtons 路径已移除，不存在其一次失败后永久终止输入线程的路径。单次原生拖动失败不保留关闭状态，下一个真实 Down 仍可处理。
- macOS 14+ 禁用背景节流；Windows/macOS 13 对该 API 无支持保证。隐藏/文档不可见时停止 RAF。未使用猜测的 CGS 私有 ABI，也未增加输入监控权限。

### 可复现入口

使用 `.node-version` 的 Node 24.11.1、pnpm 10.28.2、Rust 1.93.1：

```sh
pnpm install --frozen-lockfile
pnpm --filter @codex-pet/desktop test -- frame-meter
cargo test -p pet-desktop --lib
pnpm --filter @codex-pet/desktop tauri build --debug --no-bundle
PET_VALIDATION_PHASE=animation PET_DIAGNOSTICS_DIR="$PWD/diagnostics/animation" target/debug/pet-desktop
```

Windows PowerShell 在相同测试/构建后设置 `$env:PET_VALIDATION_PHASE = 'animation'`、`$env:PET_DIAGNOSTICS_DIR = "$PWD\diagnostics\animation"`，再运行 `.\target\debug\pet-desktop.exe`。phase 可选 idle / animation / hidden；默认动画，idle 只画首帧，hidden 在初始化时隐藏整组。

仅指定 `PET_DIAGNOSTICS_DIR` 才写入 `runtime.jsonl`，每 60 秒报告 OS/版本/架构、WebView 与依赖版本、可见/聚焦、DPI、位置/大小、累计绘制数及真实绘制间隔。macOS 另有 `runtime.inputRoute`，记录两窗编号/父子关系/显隐/位置和公开原生命中查询。原型的全局光标字段已移除。

### 本轮原生证据

测试环境仍为 macOS 27.0.1 / aarch64 / 缩放 2 / WebKit 22625.1.29.11.28。公开 [NSWindow 命中查询](https://developer.apple.com/documentation/appkit/nswindow/windownumber(at:belowwindowwithwindownumber:)) 使用真实 mouseDown 的路由规则，避免 CUA 定向事件不改变全局指针的问题。

| 实验 | 实体 / 透明角结果 | 结论 |
| --- | --- | --- |
| WK + 输入覆盖视图，ignore=false | 两点都命中 renderer 29409 | 自动 alpha 路由未成立 |
| 移除覆盖视图，opaque=false、背景 alpha=0 | 两点都命中 renderer 29471 | 覆盖视图不是该问题来源 |
| 显式透明 under-page 背景 | 两点都命中 renderer 29481 | 不能依赖本机 WK 合成的自动透明命中 |
| 独立 CPU 椭圆，窗口 29502 | 实体命中 29502；角命中 108/2606 | 原生 CPU alpha 命中成立 |
| 独立 proxy 29522 / render 29523 child | 实体命中 29522；角命中 2606 | 固定穿透显示窗与代理分工成立 |
| 实际 Tauri renderer 29549 / proxy 29550 | 实体命中 29550；角命中 108，等于 belowProxy 108 | 当前组合的系统路由通过 |

所有组的相对测试位置是客户区 `(80,84)` 与 `(5,5)`；前面三组未打印绝对坐标，保留相对点定义及原始诊断位置，不补造绝对值。CPU/实际代理组打印的屏幕坐标为实体 `(200,718)`、角 `(125,797)`，原生 frame 为 `(120,622,160,180)`。父窗移动到 `(220,722)` 后，子窗同幅移动，两个测试点成为 `(300,818)`、`(225,897)`；orderOut 两窗都隐藏，orderFront 两窗都恢复；解除父子关联并关闭后两窗都不可见、parent=0。

实际 Tauri 组合在 60.008 秒报告 1798 个绘制间隔，P95=50ms，≤50ms 为 99.555%，focused=false；120.008 秒段 P95=50ms、≤50ms 为 99.610%。隐藏组 renderer 29561 / proxy 29562 均不可见，60.103 / 120.204 秒累计绘制数均为 0，两测试点均命中下层。执行中断期间隐藏进程继续产生日志，共保留 23 条、均为 0 次绘制/两窗隐藏；这不包含 CPU/RSS 采样，不充当完整资源验证。

原始输出：`diagnostics/fix-round1-proxy/runtime.jsonl`、`diagnostics/fix-round1-hidden/runtime.jsonl`；独立原生对照源码/输出与 RED/GREEN/完整检查日志保存在 `.superpowers/sdd/2026-11-11-tauri-desktop-pet/`。恢复后进程检查确认四类实验进程均不存在。CUA 访问隐藏应用超时，未据此宣称实际 Tauri 自然退出已验证；独立双窗关闭对照已通过。

### 当前覆盖与后续接口约束

- 前端 6 项、Rust workspace 43 项测试、类型检查、fmt/clippy、macOS debug 构建通过。Windows 10/11 原生构建/交互、macOS 13/Intel、混合 DPI、休眠、真实指针完整拖动、另一应用持续保持键盘焦点和完整视觉合成仍待验证；三目标 CI 已配置，未实际执行。
- 真实指针验收仍按开头五个固定用例：透明角点击应到专用底板；实体按下拖动、释放、再次拖动；快速跨边界点击；透明区按下再移入实体不能触发拖动。当前原生路由查询不代替完整物理交互流程。
- Task8 的位置、显隐和关闭必须操作整个窗口组，不能只移动/隐藏 Canvas child；诊断快照边界问题（审阅 Minor1）按主控安排留 Task8 整合。本轮未扩大为诊断重构。
- Task9 将原型椭圆替换为缓存 AlphaMask；Task12 重做最终资源和平台验收。既定预算保留在以下历史表中，当前修复版本尚未完成完整资源预算验收。

参考：[Tauri 配置限制](https://v2.tauri.app/reference/config/#windowconfig)、[Windows 系统区域及所有权](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowrgn)。

## Task 8：月影 Canvas 播放与后台帧率（2026-11-20）

本机环境为 macOS 27.0.1（26A434）、Apple Silicon、显示缩放 2、WebKit 22625.1.29.11.28、Tauri 2.12.4。计时取 Canvas 绘制完成后的 `performance.now()`。每段同步冻结计数和间隔，原生状态另记返回时刻。P95 使用 nearest-rank。

| 模式 | 观测时长 | 实际绘制间隔数 | P95 | ≤ 50 ms | 原生可见 / 聚焦 |
| --- | ---: | ---: | ---: | ---: | --- |
| 默认 30 FPS | 60.003 s | 1799 | 35 ms | 100% | 是 / 否 |
| 60 FPS | 60.003 s | 3594 | 18 ms | 99.972% | 是 / 否 |
| 隐藏 | 60.104 s | 0（累计绘制 0） | — | — | 否 / 否 |

默认档的 128 次前台进程采样覆盖约 64 秒，桌宠从未是采样到的前台进程。60 FPS 档按间隔折算为 59.905 次绘制/秒，动作姿态仍使用图集清单中的时长。4739933 在相同系统/WebKit 的原型报告中为 1798 个间隔、P95 50 ms、99.555% ≤ 50 ms，使用当次 rAF 时间戳。两版均达到默认档目标。

macOS 检查了九个动作的原生窗口截图。输入代理仍用原生椭圆，底色改为 1% 黑色，实测实体测试点仍命中代理、透明角仍落到下层。隔离测试副本通过原有窗口组入口完成隐藏和再显示，并向 AppKit 通知中心注入睡眠/唤醒通知，分别检查了暂停、从当前动作开头恢复，以及隐藏期间收到唤醒仍保持隐藏。真机系统休眠、实际物理拖动、Windows 原生表现及最终三模式资源占用待验。Windows 交叉类型检查覆盖电源事件与共享状态模块，完整桌宠交叉检查停在 Tauri 资源工具 `llvm-rc` 缺失。

隔离复测使用 debug 构建。先创建并解析临时目录的绝对路径，再指定诊断与 broker 数据目录：

```sh
validation_root=$(mktemp -d "${TMPDIR:-/tmp}/codex-pet-validation.XXXXXX")
validation_root=$(cd "$validation_root" && pwd -P)
PET_VALIDATION_PHASE=animation \
  PET_DIAGNOSTICS_DIR="$validation_root/diagnostics" \
  PET_VALIDATION_DATA_DIR="$validation_root/app" \
  target/debug/pet-desktop
```

同一运行入口支持用 `PET_VALIDATION_FPS=60` 切换帧率，或用 `PET_VALIDATION_STATE` 指定图集清单中的动作。原始间隔、焦点采样、窗口截图和测试说明保存在本地 `.superpowers/sdd/2026-11-11-tauri-desktop-pet/task-8-evidence/`。

## 69fbf87 原型历史资源基线（2026-11-11）

以下十五分钟数据全部来自 `69fbf8734fe2f11a5d6501823bc09dae32a5484a` 的轮询原型。修复后输入架构已改变；这些数值不作为当前版本的新测量，完整资源验收留给 Task12。

环境：macOS 27.0.1 / aarch64，缩放 2，WebKit 22625.1.29.11.28；Node 24.11.1 / Rust 1.93.1 / Tauri 2.12.4，debug 二进制（最低部署目标 13.0，SDK 27.0）。三个阶段各 300 秒、61 次资源采样，排除前 10 秒预热后统计。

| 状态 | 主进程 CPU 均值/采样峰值 | 主进程 RSS 均值/峰值 MiB | WebKit 候选 CPU 均值/采样峰值 | WebKit 候选 RSS 均值/峰值 MiB |
| --- | --- | --- | --- | --- |
| 动画 | 6.38% / 11.40% | 53.20 / 89.38 | 5.81% / 7.90% | 33.51 / 49.09 |
| 静止 | 2.99% / 3.60% | 70.29 / 88.22 | 0.00% / 0.00% | 28.75 / 46.00 |
| 隐藏 | 0.11% / 0.60% | 51.33 / 87.20 | 0.05% / 0.60% | 20.03 / 43.22 |

统计范围：主进程分别 PID 1511 / 6213 / 8511，无可追踪普通子进程；WebKit 候选分别 1514/1515/1516、6216/6219/6249、8514/8515/8516，均由启动前后新增 PID 得到，PPID=1，**未严格证明归属，不能当作完整应用总占用**。CPU 使用 `ps %cpu` 系统近期估计的采样均值（单核 100% 口径），RSS 以 KiB/1024 转 MiB；其他用户应用同时运行，不是隔离基准。

| 动画完整段结束时间 | 绘制间隔数 | P95 | ≤50ms 比例 | visible / focused |
| --- | --- | --- | --- | --- |
| 60.006s | 1796 | 34ms | 99.833% | true / false |
| 120.002s | 1796 | 34ms | 99.443% | true / false |
| 180.002s | 1799 | 34ms | 100.000% | true / false |
| 240.002s | 1799 | 34ms | 100.000% | true / false |

69fbf87 原型的本机未聚焦 60 秒绘制指标通过；此结论不推广到修复后的版本或 Windows/macOS 13。静止段四次完整诊断的累计绘制数均为 1；隐藏段至 240.401 秒的四次完整诊断均为 0、visible=false、documentVisible=false。采样在 300 秒终止，与第五次诊断定时器竞争，因此没有把不存在的第五段当作数据。

阶段一数值预算现锁定如下（以后调整必须记录依据）：

| 范围 | 动画 CPU 均值上限 | 静止 CPU 均值上限 | 隐藏 CPU 均值上限 | RSS 峰值上限 |
| --- | --- | --- | --- | --- |
| 主进程 | 10% | 5% | 1% | 128 MiB |
| 全部专用 WebView 进程 | 10% | 2% | 1% | 128 MiB |
| 完整应用合计 | 20% | 7% | 2% | 256 MiB |

69fbf87 原型主进程实测满足预算。WebKit 候选数据落在 WebView 预算内，但因归属未严格证明，**完整应用预算仍未验收**。Windows 与其他 macOS 版本/架构使用相同目标，等待各自原生实测，不能以本机预算通过代替。

原始证据在仓库 git-ignored `diagnostics/resources/{animation,idle,hidden}/{samples,runtime}.jsonl`，汇总为 `diagnostics/resources/summary.json`，底板输入证据为 `diagnostics/resources/underlay-input.log`；采样脚本和底板源码保留在本地任务报告目录。实验结束后，三个主进程、底板和上述 WebKit 候选 PID 均已退出。

69fbf87 当时的自动化结果：前端 6 个测试通过，Rust workspace 40 个测试通过，类型检查、前端构建、fmt、clippy、Tauri debug `--no-bundle` 构建通过。真实透明合成、置顶、穿透、拖动、完整焦点场景、macOS 13/Intel/混合 DPI/休眠和 Windows 10/11 仍未验证。
