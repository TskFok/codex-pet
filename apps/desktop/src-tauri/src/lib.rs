pub mod broker;
pub mod platform;
mod windows;

use broker::{BrokerHandle, BrokerStartup, BrokerView};
use pet_core::{AppPaths, PetSnapshot};
use serde::Serialize;
use std::{
    fs,
    io::Write,
    sync::{Mutex, OnceLock, RwLock},
    time::Duration,
};
use tauri::{Emitter, Manager};
use tokio::sync::{Notify, broadcast};

const INITIAL_START_RETRY: Duration = Duration::from_secs(2);
const MAX_START_RETRY: Duration = Duration::from_secs(30);

#[derive(Default)]
struct DiagnosticState(Mutex<()>);

#[derive(Default)]
struct NotificationRuntime {
    connection: RwLock<NotificationConnection>,
    exit: Notify,
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
        let mut startup = BrokerStartup::new(paths);
        let mut retry_delay = INITIAL_START_RETRY;
        let mut last_failure = None;
        loop {
            let started = tokio::select! {
                biased;
                _ = self.exit.notified() => return None,
                result = async {
                    tokio::task::spawn_blocking(move || {
                        let result = startup.attempt();
                        (startup, result)
                    }).await
                } => result,
            };
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
                _ = self.exit.notified() => return None,
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

fn start_notifications(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let state = app.state::<NotificationRuntime>();
        let paths = match AppPaths::for_user() {
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
        *state.connection.write().unwrap() = NotificationConnection::Ready(broker.view());
        let initial = broker.snapshot();
        let mut last_revision = initial.revision;
        let _ = app.emit_to("pet", "pet:snapshot", initial);
        loop {
            let snapshot = tokio::select! {
                _ = state.exit.notified() => break,
                update = updates.recv() => match update {
                    Ok(snapshot) => snapshot,
                    Err(broadcast::error::RecvError::Lagged(_)) => broker.snapshot(),
                    Err(broadcast::error::RecvError::Closed) => break,
                },
            };
            if snapshot.revision > last_revision {
                last_revision = snapshot.revision;
                let _ = app.emit_to("pet", "pet:snapshot", snapshot);
            }
        }
        // 回收日志线程可能需要完成正在进行的落盘，交给后台线程。
        tauri::async_runtime::spawn_blocking(move || drop(broker))
            .await
            .ok();
    });
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
    notification_service: NotificationService,
    #[cfg(target_os = "macos")]
    input_route: Option<serde_json::Value>,
}

#[tauri::command]
fn runtime_info(
    window: tauri::WebviewWindow,
    notifications: tauri::State<'_, NotificationRuntime>,
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
fn pointer_input(
    window: tauri::WebviewWindow,
    input: windows::PointerInput,
) -> Result<bool, String> {
    windows::dispatch_pointer_input(input, || window.start_dragging()).map_err(|e| e.to_string())
}

pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(target_os = "macos")]
    let builder = builder.activate_ignoring_other_apps(false);
    #[cfg(target_os = "macos")]
    let builder = builder.on_window_event(|window, event| {
        if window.label() == "pet" && matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
            platform::shutdown_input();
        }
    });
    let app = builder
        .manage(DiagnosticState::default())
        .manage(NotificationRuntime::default())
        .invoke_handler(tauri::generate_handler![
            runtime_info,
            record_diagnostic,
            pointer_input,
            pet_snapshot
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            platform::create_pet_window(app.handle())?;
            if std::env::var("PET_VALIDATION_PHASE").as_deref() == Ok("hidden") {
                platform::set_pet_visible(app.handle(), false)?;
            }
            start_notifications(app.handle().clone());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("桌宠窗口启动失败");
    app.run(|app, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            app.state::<NotificationRuntime>().exit.notify_one();
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

        runtime.exit.notify_one();
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
