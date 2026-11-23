pub mod broker;
mod commands;
mod integrations;
pub mod navigation;
pub mod placement;
pub mod platform;
pub mod settings;
mod tray;
mod windows;

use broker::{BrokerHandle, BrokerStartup, BrokerView};
use pet_core::{AppPaths, PetSnapshot, StorageError};
use platform::{Playback, PlaybackSignal};
use serde::Serialize;
use std::{
    fs,
    io::Write,
    sync::{Mutex, OnceLock, RwLock},
    time::Duration,
};
use tauri::{Emitter, Manager};
use tokio::sync::{broadcast, watch};

const INITIAL_START_RETRY: Duration = Duration::from_secs(2);
const MAX_START_RETRY: Duration = Duration::from_secs(30);

#[derive(Default)]
struct DiagnosticState(Mutex<()>);

#[derive(Default)]
struct NotificationRuntime {
    connection: RwLock<NotificationConnection>,
    exit: watch::Sender<bool>,
    worker: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

#[derive(Default)]
enum NotificationConnection {
    #[default]
    Starting,
    Ready(BrokerView),
    Unavailable,
}

#[derive(Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum NotificationService {
    Starting,
    Online,
    Unavailable { message: &'static str },
}

impl NotificationRuntime {
    fn status(&self) -> NotificationService {
        match &*self.connection.read().unwrap() {
            NotificationConnection::Starting => NotificationService::Starting,
            NotificationConnection::Ready(view) => match view.fault() {
                Some(message) => NotificationService::Unavailable { message },
                None => NotificationService::Online,
            },
            NotificationConnection::Unavailable => NotificationService::Unavailable {
                message: "无法启动本机通知服务",
            },
        }
    }

    async fn wait_for_broker(&self, paths: AppPaths) -> Option<BrokerHandle> {
        self.wait_for_broker_with(paths, BrokerStartup::attempt)
            .await
    }

    async fn wait_for_broker_with(
        &self,
        paths: AppPaths,
        attempt: impl Fn(&mut BrokerStartup) -> Result<BrokerHandle, StorageError>
        + Clone
        + Send
        + 'static,
    ) -> Option<BrokerHandle> {
        let mut startup = BrokerStartup::new(paths);
        let mut retry_delay = INITIAL_START_RETRY;
        let mut last_failure = None;
        let mut exit = self.exit.subscribe();
        loop {
            if *exit.borrow() {
                return None;
            }
            let attempt = attempt.clone();
            // 已派发的启动任务可能正持有日志锁或发布发现文件，退出必须等它返回。
            let started = tokio::task::spawn_blocking(move || {
                let result = attempt(&mut startup);
                (startup, result)
            })
            .await;
            if *exit.borrow() {
                // 析构 BrokerHandle 可能等待 HTTP 与日志线程。退出等待也覆盖此回收。
                let _ = tokio::task::spawn_blocking(move || drop(started)).await;
                return None;
            }
            let (pending, result) = match started {
                Ok(result) => result,
                Err(error) => {
                    *self.connection.write().unwrap() = NotificationConnection::Unavailable;
                    eprintln!("桌宠通知：通知启动任务中断，{error}");
                    return None;
                }
            };
            startup = pending;
            let failure = match result {
                Ok(broker) => return Some(broker),
                Err(error) => format!("无法启动本机通知服务，{error}"),
            };
            *self.connection.write().unwrap() = NotificationConnection::Unavailable;
            if last_failure.as_ref() != Some(&failure) {
                eprintln!("桌宠通知：{failure}，将在后台重试");
                last_failure = Some(failure);
            }
            tokio::select! {
                biased;
                _ = exit.wait_for(|stopped| *stopped) => return None,
                _ = tokio::time::sleep(retry_delay) => {}
            }
            retry_delay = (retry_delay * 2).min(MAX_START_RETRY);
        }
    }
}

#[tauri::command]
fn pet_snapshot(state: tauri::State<'_, NotificationRuntime>) -> Option<PetSnapshot> {
    match &*state.connection.read().unwrap() {
        NotificationConnection::Ready(view) => Some(view.snapshot()),
        _ => None,
    }
}

fn notification_paths() -> Result<AppPaths, StorageError> {
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("PET_VALIDATION_DATA_DIR")
        && std::env::var_os("PET_DIAGNOSTICS_DIR").is_some_and(|directory| !directory.is_empty())
    {
        let root = std::path::PathBuf::from(path);
        if !root.is_absolute() {
            return Err(StorageError::InvalidData("诊断应用数据目录必须是绝对路径"));
        }
        return Ok(AppPaths::under(&root));
    }
    AppPaths::for_user()
}

fn start_notifications(app: tauri::AppHandle) {
    let target = app.clone();
    let worker = tauri::async_runtime::spawn(async move {
        let state = app.state::<NotificationRuntime>();
        let paths = match notification_paths() {
            Ok(paths) => paths,
            Err(error) => {
                eprintln!("桌宠通知：无法定位本机应用数据目录，{error}");
                *state.connection.write().unwrap() = NotificationConnection::Unavailable;
                return;
            }
        };
        let Some(broker) = state.wait_for_broker(paths).await else {
            return;
        };
        let mut updates = broker.subscribe();
        let mut exit = state.exit.subscribe();
        *state.connection.write().unwrap() = NotificationConnection::Ready(broker.view());
        let initial = broker.snapshot();
        let mut last_revision = initial.revision;
        let _ = app.emit("pet:snapshot", initial);
        loop {
            let snapshot = tokio::select! {
                _ = exit.wait_for(|stopped| *stopped) => break,
                update = updates.recv() => match update {
                    Ok(snapshot) => snapshot,
                    Err(broadcast::error::RecvError::Lagged(_)) => broker.snapshot(),
                    Err(broadcast::error::RecvError::Closed) => break,
                },
            };
            if snapshot.revision > last_revision {
                last_revision = snapshot.revision;
                let _ = app.emit("pet:snapshot", snapshot);
            }
        }
        // 回收日志线程可能需要完成正在进行的落盘，交给后台线程。
        tauri::async_runtime::spawn_blocking(move || drop(broker))
            .await
            .ok();
    });
    *target.state::<NotificationRuntime>().worker.lock().unwrap() = Some(worker);
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeInfo {
    os: &'static str,
    arch: &'static str,
    os_version: String,
    webview: String,
    tauri: &'static str,
    app_version: &'static str,
    visible: bool,
    focused: bool,
    scale_factor: f64,
    position: tauri::PhysicalPosition<i32>,
    size: tauri::PhysicalSize<u32>,
    phase: String,
    playback: PlaybackSignal,
    #[cfg(debug_assertions)]
    #[serde(skip_serializing_if = "Option::is_none")]
    validation_state: Option<String>,
    #[cfg(debug_assertions)]
    #[serde(skip_serializing_if = "Option::is_none")]
    validation_fps: Option<u8>,
    notification_service: NotificationService,
    #[cfg(target_os = "macos")]
    input_route: Option<serde_json::Value>,
}

#[tauri::command]
fn runtime_info(
    window: tauri::WebviewWindow,
    notifications: tauri::State<'_, NotificationRuntime>,
    playback: tauri::State<'_, Playback>,
) -> Result<RuntimeInfo, String> {
    Ok(RuntimeInfo {
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        os_version: OS_VERSION.get_or_init(os_version).clone(),
        webview: tauri::webview_version().map_err(|e| e.to_string())?,
        tauri: tauri::VERSION,
        app_version: env!("CARGO_PKG_VERSION"),
        visible: window.is_visible().map_err(|e| e.to_string())?
            && !window.is_minimized().map_err(|e| e.to_string())?,
        focused: window.is_focused().map_err(|e| e.to_string())?,
        scale_factor: window.scale_factor().map_err(|e| e.to_string())?,
        position: window.inner_position().map_err(|e| e.to_string())?,
        size: window.inner_size().map_err(|e| e.to_string())?,
        #[cfg(target_os = "macos")]
        input_route: platform::route_report(&window),
        phase: std::env::var("PET_VALIDATION_PHASE").unwrap_or_else(|_| "animation".into()),
        playback: playback.snapshot(),
        #[cfg(debug_assertions)]
        validation_state: std::env::var("PET_VALIDATION_STATE").ok(),
        #[cfg(debug_assertions)]
        validation_fps: std::env::var("PET_VALIDATION_FPS")
            .ok()
            .and_then(|fps| fps.parse().ok()),
        notification_service: notifications.status(),
    })
}

static OS_VERSION: OnceLock<String> = OnceLock::new();

fn os_version() -> String {
    #[cfg(target_os = "macos")]
    let output = std::process::Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output();
    #[cfg(target_os = "windows")]
    let output = {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("cmd")
            .args(["/C", "ver"])
            .creation_flags(0x08000000) // CREATE_NO_WINDOW，诊断不能弹出控制台抢焦点。
            .output()
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let output = std::process::Command::new("uname").arg("-r").output();
    output
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_else(|| "unknown".into())
}

/// 仅显式设置环境变量时把诊断写入指定实验目录，不接触宿主配置。
#[tauri::command]
fn record_diagnostic(
    report: serde_json::Value,
    state: tauri::State<'_, DiagnosticState>,
) -> Result<(), String> {
    let line = serde_json::to_string(&report).map_err(|e| e.to_string())?;
    if line.len() > 1024 * 1024 {
        return Err("诊断报告超过 1 MiB".into());
    }
    if let Some(directory) = std::env::var_os("PET_DIAGNOSTICS_DIR") {
        let _guard = state.0.lock().map_err(|e| e.to_string())?;
        fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let path = std::path::PathBuf::from(directory).join("runtime.jsonl");
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| e.to_string())?;
        writeln!(file, "{line}").map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn pointer_input(window: tauri::WebviewWindow) -> Result<(), String> {
    if window.label() != "pet" {
        return Err("需要桌宠窗口".into());
    }
    window.start_dragging().map_err(|e| e.to_string())
}

#[tauri::command]
fn update_hit_region(
    window: tauri::WebviewWindow,
    frame: pet_core::pet_package::FrameRect,
    mask: windows::AlphaMask,
) -> Result<(), String> {
    if window.label() != "pet" || !mask.validate() {
        return Err("无效的桌宠命中区域".into());
    }
    platform::update_hit_region(&window, &frame, mask).map_err(|e| e.to_string())
}

pub fn run() {
    let builder = tauri::Builder::default().plugin(tauri_plugin_autostart::Builder::new().build());
    #[cfg(target_os = "macos")]
    let builder = builder.activate_ignoring_other_apps(false);
    let builder = builder.on_window_event(|window, event| {
        if window.label() == "pet"
            && let tauri::WindowEvent::CloseRequested { api, .. } = event
        {
            api.prevent_close();
            if let Err(error) = platform::set_pet_visible(window.app_handle(), false) {
                eprintln!("隐藏桌宠失败：{error}");
            }
        }
    });
    let app = builder
        .manage(DiagnosticState::default())
        .manage(commands::SettingsLock::default())
        .manage(integrations::IntegrationLock::default())
        .manage(Playback::default())
        .manage(NotificationRuntime::default())
        .invoke_handler(tauri::generate_handler![
            runtime_info,
            record_diagnostic,
            pointer_input,
            update_hit_region,
            pet_snapshot,
            commands::get_snapshot,
            commands::mark_read,
            commands::clear_history,
            commands::get_settings,
            commands::update_settings,
            commands::import_pet_package,
            commands::choose_pet_directory,
            commands::list_pets,
            commands::get_pet_asset,
            commands::get_playback,
            commands::play_alert,
            integrations::integration_status,
            integrations::apply_integration
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let saved = notification_paths()
                .and_then(|p| placement::load(&p))
                .unwrap_or_else(|e| {
                    eprintln!("桌宠使用默认位置：{e}");
                    placement::WindowPlacement::default()
                });
            app.manage(placement::PlacementState::new(saved));
            platform::create_pet_window(app.handle())?;
            tray::install(app.handle())?;
            start_notifications(app.handle().clone());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("桌宠窗口启动失败");
    app.run(|app, event| {
        if matches!(
            event,
            tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
        ) {
            platform::flush_placement(app);
        }
        if matches!(event, tauri::RunEvent::Exit) {
            let state = app.state::<NotificationRuntime>();
            state.exit.send_replace(true);
            let worker = state.worker.lock().unwrap().take();
            if let Some(worker) = worker {
                let _ = tauri::async_runtime::block_on(worker);
            }
            #[cfg(target_os = "macos")]
            platform::shutdown_input();
        }
    });
}

#[cfg(test)]
mod notification_runtime_tests {
    use super::*;
    use pet_core::{JournalStore, StorageError};

    #[tokio::test]
    async fn exit_joins_inflight_start_and_releases_published_broker_or_pending_writer() {
        use std::sync::{Arc, Barrier};
        for publish_fails in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let paths = AppPaths::under(&directory.path().canonicalize().unwrap().join("app"));
            if publish_fails {
                paths.prepare_root().unwrap();
                fs::create_dir(&paths.discovery).unwrap();
            }
            let state = Arc::new(NotificationRuntime::default());
            let runtime = state.clone();
            let root = paths.clone();
            let (started, mut entered) = tokio::sync::mpsc::unbounded_channel();
            let release = Arc::new(Barrier::new(2));
            let gate = release.clone();
            let mut worker = tokio::spawn(async move {
                runtime
                    .wait_for_broker_with(root, move |startup| {
                        let result = startup.attempt();
                        started.send(result.is_ok()).unwrap();
                        gate.wait();
                        result
                    })
                    .await
            });
            let success = entered.recv().await.unwrap();
            let published = paths.discovery.is_file();
            let locked = matches!(
                JournalStore::open(paths.clone()),
                Err(StorageError::WriterBusy)
            );
            state.exit.send_replace(true);
            let early = tokio::time::timeout(Duration::from_millis(100), &mut worker).await;
            release.wait();
            assert_eq!(success, !publish_fails);
            assert_eq!(published, !publish_fails);
            assert!(locked, "处于启动屏障的线程必须保有写锁");
            assert!(early.is_err(), "退出越过了持锁的启动线程");
            assert!(worker.await.unwrap().is_none());
            assert!(!paths.discovery.is_file());
            assert!(JournalStore::open(paths).is_ok());
        }
    }

    #[tokio::test]
    async fn preexisting_exit_does_not_dispatch_startup() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::under(&temp.path().canonicalize().unwrap().join("app"));
        let runtime = NotificationRuntime::default();
        runtime.exit.send_replace(true);
        assert!(
            runtime
                .wait_for_broker_with(paths.clone(), |_| panic!("不应派发启动操作"))
                .await
                .is_none()
        );
        assert!(!paths.root.exists());
    }

    #[tokio::test]
    async fn discovery_retry_keeps_the_buffered_running_state_and_one_restart() {
        use pet_bridge::outbox::Outbox;
        use pet_core::{SessionStatus, decode_event, paths::wall_clock_ms};

        let temporary = tempfile::tempdir().unwrap();
        let paths = AppPaths::under(&temporary.path().canonicalize().unwrap().join("app"));
        let outbox = Outbox::new(paths.clone()).unwrap();
        let mut event = decode_event(br#"{"schemaVersion":2,"eventId":"queued-start","identity":{"source":"deepseek","installationId":"fixture-install","instanceId":"fixture-instance"},"sessionId":"fixture-session","turnId":"fixture-turn","occurredAtMs":100,"kind":{"type":"turn_started"}}"#).unwrap();
        event.occurred_at_ms = wall_clock_ms();
        outbox.enqueue(&event).unwrap();
        fs::create_dir(&paths.discovery).unwrap();

        let runtime = NotificationRuntime::default();
        let starting = runtime.wait_for_broker(paths.clone());
        tokio::pin!(starting);
        tokio::time::timeout(Duration::from_secs(1), async {
            tokio::select! {
                _ = &mut starting => panic!("发现路径不可写时应保持等待"),
                _ = async {
                    while !matches!(runtime.status(), NotificationService::Unavailable { .. }) {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                } => {}
            }
        })
        .await
        .expect("未观察到发现文件发布失败");

        let journal = fs::read_to_string(&paths.journal).unwrap();
        assert!(journal.contains("queued-start"), "发现文件失败前已落盘");
        assert!(
            outbox.pending(wall_clock_ms()).unwrap().is_empty(),
            "已按原事务完成文件确认"
        );
        let kept_writer_lock = matches!(
            JournalStore::open(paths.clone()),
            Err(StorageError::WriterBusy)
        );
        fs::remove_dir(&paths.discovery).unwrap();

        let broker = tokio::time::timeout(Duration::from_secs(6), starting)
            .await
            .expect("同一次启动应在发现路径恢复后重试")
            .expect("启动重试应返回服务");
        let snapshot = broker.snapshot();
        assert_eq!(
            snapshot.sessions[0].status,
            SessionStatus::Running,
            "首份快照应保留已补发的开始事实"
        );
        assert!(kept_writer_lock, "启动后段失败期间仍由原日志写入器持锁");
        let journal = fs::read_to_string(&paths.journal).unwrap();
        assert_eq!(journal.matches(r#""restarted":true"#).count(), 1);
    }

    #[tokio::test]
    async fn exit_interrupts_startup_backoff_without_taking_the_writer_lock() {
        let temporary = tempfile::tempdir().unwrap();
        let paths = AppPaths::under(&temporary.path().canonicalize().unwrap().join("app"));
        let _writer = JournalStore::open(paths.clone()).unwrap();
        let runtime = NotificationRuntime::default();
        let starting = runtime.wait_for_broker(paths.clone());
        tokio::pin!(starting);

        tokio::time::timeout(Duration::from_secs(1), async {
            tokio::select! {
                _ = &mut starting => panic!("持锁期间服务启动流程应继续等待"),
                _ = async {
                    while !matches!(runtime.status(), NotificationService::Unavailable { .. }) {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                } => {}
            }
        })
        .await
        .expect("未观察到写锁故障");

        runtime.exit.send_replace(true);
        assert!(
            tokio::time::timeout(Duration::from_millis(300), starting)
                .await
                .expect("退出信号应打断启动退避")
                .is_none()
        );
        assert!(matches!(
            JournalStore::open(paths),
            Err(StorageError::WriterBusy)
        ));
    }
}
