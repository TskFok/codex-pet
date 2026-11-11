# 桌宠窗口阶段一验证

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

## 可复现实验入口

先使用 `.node-version` 的 Node 24.11.1、pnpm 10.28.2 和 `rust-toolchain.toml` 的 Rust 1.93.1：

```sh
pnpm install --frozen-lockfile
pnpm --filter @codex-pet/desktop test -- frame-meter
pnpm --filter @codex-pet/desktop tauri build --debug --no-bundle
# macOS：每阶段单独运行 300 秒，完成后终止该次启动的进程。
PET_VALIDATION_PHASE=animation PET_DIAGNOSTICS_DIR="$PWD/diagnostics/animation" target/debug/pet-desktop
# phase 可选 idle / animation / hidden。默认 animation；idle 只画首帧，hidden 在创建后立即隐藏。
```

Windows PowerShell 使用相同测试、构建命令，再运行：

```powershell
$env:PET_VALIDATION_PHASE = 'animation'
$env:PET_DIAGNOSTICS_DIR = "$PWD\diagnostics\animation"
.\target\debug\pet-desktop.exe
```

`runtime.jsonl` 每 60 秒记录一段，包含 OS 版本/架构、实际 WebView 版本、Tauri 和前端依赖版本、窗口可见/聚焦状态、DPI、位置/大小、全局光标位置、累计绘制数及实际绘制间隔。间隔不做去噪，P95 为 nearest-rank；恢复可见后开始新段，不把隐藏时间混入绘制间隔。只有显式提供 `PET_DIAGNOSTICS_DIR` 才落盘；默认仅控制台诊断。五分钟运行应包含至少四个完整 60 秒段（最后一段与终止时间可能竞争）。

macOS 的 CUA 观察使用临时 `.app` 包装同一调试二进制以便识别窗口；此包装不等同正式应用包签名或分发验证。

## 实现与平台限制

- 窗口在创建时设置透明、无边框、无阴影、置顶、`focused(false)` 和 `focusable(false)`，从不调用 focus。macOS 另外关闭启动时强制激活，使用 Accessory 激活策略。
- 绘制角色为椭圆几何测试图；图标逐字节复制自 `pet-output/yueying-cape-dancer/run/frames/idle/00.png`，ICO 仅封装同一 PNG，不修改原素材。
- 实体命中椭圆为中心 `(80,84)`、半轴 `(52,60)` 逻辑像素。16ms 轮询全局光标，换算当前窗口 DPI，按命中区切换整窗 `set_ignore_cursor_events`；透明轨迹也穿透。轮询在快速进入实体并立即点击时存在最多一个周期的竞争，不能当作逐像素原生命中测试。
- 实体按下后按初始锚点拖动，平台层分别读取 macOS `NSEvent.pressedMouseButtons` / Windows `GetAsyncKeyState(VK_LBUTTON)`，避免异步 WebView IPC 丢失当前拖动事件；在透明区按下再移入实体不会捕获拖动。混合 DPI 和跨屏拖动仍须真机验证。
- macOS 14+ 设置 `background_throttling(Disabled)`；Windows 和 macOS 13 没有对应支持保证。隐藏/文档不可见时取消 RAF，原生命中轮询降到 250ms；休眠由系统暂停，恢复后的实际系统行为仍需专项实验。
- macOS 2.12.1+ `macOSPrivateApi` 已无效，因此没有沿用旧配置。
- CI 只检查与上传调试二进制：macOS arm64 / x86_64、Windows x86_64；没有自动发布，也没有用 CI 构建代替 Windows 10/11 桌面交互验收。

参考：[Tauri 配置](https://v2.tauri.app/reference/config/#windowconfig)、[窗口输入 API](https://v2.tauri.app/reference/javascript/api/namespacewindow/#setignorecursorevents)、[Vite 集成](https://v2.tauri.app/start/frontend/vite/)、[实际锁定 Tauri 2.12.4 源码](https://github.com/tauri-apps/tauri/tree/tauri-v2.12.4)、[GitHub runner 标签](https://github.com/actions/runner-images#available-images)。

## 当前平台覆盖

| 环境 | 构建/单测 | 窗口/输入/60秒/资源 |
| --- | --- | --- |
| macOS 27.0.1，Apple Silicon | 本机已构建，最终命令结果见下文 | 60秒帧率与主进程预算通过；输入/合成等未验，见下文 |
| macOS 13，Apple Silicon | 未验证 | 未验证；不支持后台节流开关，必须单独测 |
| macOS Intel | CI 已配置，尚未运行 | 未验证，需 Intel 真机 |
| Windows 10 22H2 x64 | 未验证 | 未验证，需原生 Windows、WebView2、MSVC 构建工具与交互桌面 |
| Windows 11 x64 | 未验证 | 未验证，同上 |

## 待人工交互验收的精确步骤

必须使用真实系统指针和键盘；本次 CUA 定向事件不会同步系统全局鼠标位置与按键状态，不能替代以下步骤。

1. 启动一个专用点击计数底板/测试页面，将它放到桌宠初始位置 `(120,180)` 后面；角色客户区为 160×180 逻辑像素。
2. 聚焦底板或临时编辑器并输入文字，启动桌宠。观察当前应用和输入光标是否保留，继续打字 60 秒，核对 JSONL 中 `runtime.focused=false`。不要通过点击桌宠来改变初始焦点。
3. 观察完整桌面合成，确认窗口外缘能看到底板，切换底板活动状态后角色仍置顶。单独截取桌宠窗口的图像不能证明合成或置顶。
4. 将真实指针移到角色客户区 `(80,84)`，按住左键拖动 100 逻辑像素，再释放。核对原生日志「桌宠窗口位置」或下一条 JSONL 的 `runtime.position`；缩放为 2 时应约移动 200 物理像素。再次按住拖动，确认释放状态复位。
5. 点击客户区 `(5,5)`、`(155,5)` 等透明角，底板计数应递增；点击实体中心应不递增。另测「透明区按下后拖入实体」，不得捕获桌宠拖动。重复快速进入并点击以评估 16ms 轮询竞争。
6. 保持角色可见、焦点在另一应用，运行 60 秒。读取该完整段 `frames.intervalsMs`，独立计算 `≤50ms` 的比例须≥95%，不能仅看平均 FPS。若发生隐藏/跨屏/休眠，该次需重测。
7. 分别运行 idle / animation / hidden 各 300 秒，记录进程范围及 WebView 归属，隐藏段累计绘制数须为 0。混合 DPI、显示器移除与休眠恢复单独登记，不由本机默认显示器结果代替。

Windows 10 22H2 和 Windows 11 需分别执行；CI 的 Windows Server 2022 构建成功也不代表这两个系统交互验收成功。

Windows 原生按键读取采用 [GetAsyncKeyState 官方约定](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getasynckeystate) 的负值高位判断；它读取物理左键，并受当前桌面/UIPI 权限条件影响。切换物理左右键或前台提升权限的场景尚未验证。macOS 使用 [NSEvent.pressedMouseButtons](https://developer.apple.com/documentation/appkit/nsevent/pressedmousebuttons)。

## 2026-11-11 本机最终实测与阶段一预算

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

本机未聚焦连续 60 秒绘制指标通过，不能由此推广 Windows/macOS 13。静止段四次完整诊断的累计绘制数均为 1；隐藏段至 240.401 秒的四次完整诊断均为 0、visible=false、documentVisible=false。采样在 300 秒终止，与第五次诊断定时器竞争，因此没有把不存在的第五段当作数据。

阶段一数值预算现锁定如下（以后调整必须记录依据）：

| 范围 | 动画 CPU 均值上限 | 静止 CPU 均值上限 | 隐藏 CPU 均值上限 | RSS 峰值上限 |
| --- | --- | --- | --- | --- |
| 主进程 | 10% | 5% | 1% | 128 MiB |
| 全部专用 WebView 进程 | 10% | 2% | 1% | 128 MiB |
| 完整应用合计 | 20% | 7% | 2% | 256 MiB |

本机主进程实测满足预算。WebKit 候选数据落在 WebView 预算内，但因归属未严格证明，**完整应用预算仍未验收**。Windows 与其他 macOS 版本/架构使用相同目标，等待各自原生实测，不能以本机预算通过代替。

原始证据在仓库 git-ignored `diagnostics/resources/{animation,idle,hidden}/{samples,runtime}.jsonl`，汇总为 `diagnostics/resources/summary.json`，底板输入证据为 `diagnostics/resources/underlay-input.log`；采样脚本和底板源码保留在本地任务报告目录。实验结束后，三个主进程、底板和上述 WebKit 候选 PID 均已退出。

最终自动化结果：前端 6 个测试通过，Rust workspace 40 个测试通过，类型检查、前端构建、fmt、clippy、Tauri debug `--no-bundle` 构建通过。真实透明合成、置顶、穿透、拖动、完整焦点场景、macOS 13/Intel/混合 DPI/休眠和 Windows 10/11 仍未验证。
