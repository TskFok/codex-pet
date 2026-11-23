use crate::{
    NotificationConnection, NotificationRuntime,
    broker::{BrokerView, HistoryAction},
    navigation::{self, OpenIntent, OpenOutcome},
    notification_paths,
    placement::{PlacementState, WindowPlacement},
    platform,
    settings::{DEFAULT_PET, Preferences, Settings, SettingsPatch},
};
use base64::Engine;
use pet_core::{
    AppPaths, PetSnapshot, SessionKey, StorageError, paths,
    pet_package::{self, PetManifest},
};
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{Emitter, Manager, State};
use tauri_plugin_autostart::ManagerExt;

#[derive(Debug, Serialize)]
pub struct CommandError {
    pub code: &'static str,
    pub message: String,
}
impl CommandError {
    pub fn new(code: &'static str, message: impl std::fmt::Display) -> Self {
        Self {
            code,
            message: message.to_string(),
        }
    }
}
impl From<StorageError> for CommandError {
    fn from(e: StorageError) -> Self {
        Self::new("storage", e)
    }
}
impl From<pet_package::PetError> for CommandError {
    fn from(e: pet_package::PetError) -> Self {
        Self::new("pet_package", e)
    }
}
pub type Result<T> = std::result::Result<T, CommandError>;
#[derive(Default)]
pub struct SettingsLock(Mutex<()>);
fn broker(state: &NotificationRuntime) -> Result<BrokerView> {
    match &*state.connection.read().unwrap() {
        NotificationConnection::Ready(view) => Ok(view.clone()),
        _ => Err(CommandError::new(
            "broker_unavailable",
            "本机通知服务尚未就绪，请重试",
        )),
    }
}
#[tauri::command]
pub fn get_snapshot(state: State<'_, NotificationRuntime>) -> Result<PetSnapshot> {
    Ok(broker(&state)?.snapshot())
}
#[tauri::command]
pub async fn mark_read(
    state: State<'_, NotificationRuntime>,
    key: SessionKey,
) -> Result<PetSnapshot> {
    Ok(broker(&state)?
        .history(HistoryAction::MarkRead(key))
        .await?)
}
#[tauri::command]
pub async fn clear_history(state: State<'_, NotificationRuntime>) -> Result<PetSnapshot> {
    Ok(broker(&state)?.history(HistoryAction::Clear).await?)
}
fn known_session(state: &NotificationRuntime, key: &SessionKey) -> Result<()> {
    let snapshot = broker(state)?.snapshot();
    if snapshot.sessions.iter().any(|session| &session.key == key)
        || snapshot
            .notifications
            .iter()
            .any(|notification| &notification.key == key)
    {
        return Ok(());
    }
    Err(CommandError::new("navigation", "未找到该会话的本机通知"))
}

#[tauri::command]
pub async fn open_session(
    app: tauri::AppHandle,
    state: State<'_, NotificationRuntime>,
    key: SessionKey,
    intent: OpenIntent,
) -> Result<OpenOutcome> {
    known_session(&state, &key)?;
    tauri::async_runtime::spawn_blocking(move || {
        let registration =
            crate::integrations::navigation_registration(&app, key.source, &key.installation_id)?;
        navigation::open_session(&key, registration.as_ref(), intent, |path, url| {
            navigation::system::dispatch(key.source, path, url)
        })
        .map_err(|error| CommandError::new("navigation", error))
    })
    .await
    .map_err(|_| CommandError::new("navigation", "打开请求中断，请重试"))?
}

#[tauri::command]
pub fn copy_session_id(
    window: tauri::WebviewWindow,
    state: State<'_, NotificationRuntime>,
    key: SessionKey,
) -> Result<()> {
    known_session(&state, &key)?;
    if key.session_id.contains('\0') {
        return Err(CommandError::new("clipboard", "会话标识含无效字符"));
    }
    navigation::system::copy(&window, &key.session_id)
        .map_err(|_| CommandError::new("clipboard", "无法写入系统剪贴板，请重试"))
}

#[tauri::command]
pub fn get_playback(state: State<'_, platform::Playback>) -> platform::PlaybackSignal {
    state.snapshot()
}

trait SettingsHost {
    fn autostart(&self) -> Result<bool>;
    fn set_autostart(&self, value: bool) -> Result<()>;
    fn placement(&self) -> WindowPlacement;
    fn apply_placement(&self, value: WindowPlacement) -> Result<()>;
    fn publish(&self, settings: &Settings);
}
impl SettingsHost for tauri::AppHandle {
    fn autostart(&self) -> Result<bool> {
        self.autolaunch()
            .is_enabled()
            .map_err(|e| CommandError::new("autostart", e))
    }
    fn set_autostart(&self, value: bool) -> Result<()> {
        set_autostart(self, value)
    }
    fn placement(&self) -> WindowPlacement {
        self.state::<PlacementState>().current()
    }
    fn apply_placement(&self, value: WindowPlacement) -> Result<()> {
        platform::apply_placement(self, value).map_err(|e| CommandError::new("placement", e))
    }
    fn publish(&self, settings: &Settings) {
        let _ = self.emit("pet:settings", settings);
    }
}

fn settings(host: &impl SettingsHost, mut prefs: Preferences) -> Result<Settings> {
    prefs.autostart = host.autostart()?;
    Ok(Settings {
        preferences: prefs,
        placement: host.placement(),
    })
}
#[tauri::command]
pub fn get_settings(app: tauri::AppHandle, gate: State<'_, SettingsLock>) -> Result<Settings> {
    let _guard = gate.0.lock().unwrap();
    let prefs = Preferences::load(&notification_paths()?)?;
    settings(&app, prefs)
}

fn set_autostart(app: &tauri::AppHandle, value: bool) -> Result<()> {
    let manager = app.autolaunch();
    if value {
        manager.enable()
    } else {
        manager.disable()
    }
    .map_err(|e| CommandError::new("autostart", e))?;
    if manager
        .is_enabled()
        .map_err(|e| CommandError::new("autostart", e))?
        != value
    {
        return Err(CommandError::new(
            "autostart",
            "系统未确认登录启动项，请检查系统设置",
        ));
    }
    Ok(())
}

#[tauri::command]
pub fn update_settings(
    app: tauri::AppHandle,
    gate: State<'_, SettingsLock>,
    patch: SettingsPatch,
) -> Result<Settings> {
    let _guard = gate.0.lock().unwrap();
    commit_settings(&app, &notification_paths()?, patch)
}

fn commit_settings(
    host: &impl SettingsHost,
    paths: &AppPaths,
    patch: SettingsPatch,
) -> Result<Settings> {
    let current = Preferences::load(paths)?;
    let next = current.patched(&patch)?;
    if patch.pet_id.is_some() && next.pet_id != DEFAULT_PET {
        verified_pet(paths, &next.pet_id)?;
    }
    let previous_placement = host.placement();
    let target_placement = patch
        .placement
        .as_ref()
        .map(|p| p.resolve(&previous_placement));
    let placement_changed = target_placement
        .as_ref()
        .is_some_and(|p| p != &previous_placement);
    // 在产生副作用之前获取本次响应需要的系统状态。显式设置时，平台实现负责验证写入结果。
    let original_autostart = host.autostart()?;
    let requested_autostart = patch.autostart.unwrap_or(original_autostart);
    let change_autostart = requested_autostart != original_autostart;
    let result = (|| {
        if change_autostart {
            host.set_autostart(requested_autostart)?;
        }
        if let Some(target) = target_placement
            && placement_changed
        {
            host.apply_placement(target)?;
            if crate::placement::load(paths)? != host.placement() {
                return Err(CommandError::new("placement", "未能保存桌宠位置，请重试"));
            }
        }
        if next != current {
            next.save(paths)?;
        }
        Ok(())
    })();
    if let Err(mut error) = result {
        let mut rollback = Vec::new();
        if placement_changed && let Err(e) = host.apply_placement(previous_placement) {
            rollback.push(e.message);
        }
        if change_autostart && let Err(e) = host.set_autostart(original_autostart) {
            rollback.push(e.message);
        }
        if !rollback.is_empty() {
            error
                .message
                .push_str(&format!("；还原系统设置时出错：{}", rollback.join("，")));
        }
        return Err(error);
    }
    let result = Settings {
        preferences: Preferences {
            autostart: requested_autostart,
            ..next
        },
        placement: host.placement(),
    };
    host.publish(&result);
    Ok(result)
}

#[derive(Serialize)]
pub struct PetChoice {
    id: String,
    name: String,
}
#[tauri::command]
pub async fn choose_pet_directory() -> Result<Option<PathBuf>> {
    Ok(rfd::AsyncFileDialog::new()
        .set_title("选择含 pet.json 和图集的宠物包目录")
        .pick_folder()
        .await
        .map(|f| f.path().into()))
}
#[tauri::command]
pub async fn import_pet_package(path: PathBuf) -> Result<PetManifest> {
    if !path.is_absolute() {
        return Err(CommandError::new(
            "pet_package",
            "请选择绝对路径的宠物包目录",
        ));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let paths = notification_paths()?;
        paths.prepare_root()?;
        Ok(pet_package::import_pet_to_library(
            &path,
            &paths.pets,
            DEFAULT_PET,
        )?)
    })
    .await
    .map_err(|e| CommandError::new("pet_package", e))?
}
fn library_path(paths: &AppPaths, id: &str) -> Result<PathBuf> {
    if !pet_package::valid_pet_id(id) {
        return Err(CommandError::new("pet_package", "无效的宠物标识"));
    }
    Ok(paths.pets.join(id))
}
fn verified_pet(paths: &AppPaths, id: &str) -> Result<(PetManifest, Vec<u8>)> {
    let (manifest, bytes) = pet_package::load_pet_with_atlas(&library_path(paths, id)?)?;
    if manifest.id != id {
        return Err(CommandError::new(
            "pet_package",
            "宠物包目录和注册标识不匹配",
        ));
    }
    Ok((manifest, bytes))
}
#[tauri::command]
pub async fn list_pets() -> Result<Vec<PetChoice>> {
    tauri::async_runtime::spawn_blocking(|| {
        let paths = notification_paths()?;
        let mut choices = vec![PetChoice {
            id: DEFAULT_PET.into(),
            name: "月影".into(),
        }];
        paths.prepare_root()?;
        paths::prepare_directory(&paths.pets)?;
        for dir in std::fs::read_dir(&paths.pets).map_err(StorageError::from)? {
            let dir = dir.map_err(StorageError::from)?;
            if !dir.file_type().map_err(StorageError::from)?.is_dir() {
                continue;
            }
            if let Ok(manifest) = pet_package::describe_pet(&dir.path())
                && manifest.id != DEFAULT_PET
                && dir.file_name() == manifest.id.as_str()
            {
                choices.push(PetChoice {
                    id: manifest.id,
                    name: manifest.name,
                });
            }
        }
        choices[1..].sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        Ok(choices)
    })
    .await
    .map_err(|e| CommandError::new("pet_package", e))?
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PetAsset {
    manifest: PetManifest,
    atlas_url: String,
}
#[tauri::command]
pub async fn get_pet_asset(id: String) -> Result<PetAsset> {
    tauri::async_runtime::spawn_blocking(move || {
        let (manifest, bytes) = verified_pet(&notification_paths()?, &id)?;
        let mime = if Path::new(&manifest.atlas.path)
            .extension()
            .is_some_and(|s| s == "png")
        {
            "image/png"
        } else {
            "image/webp"
        };
        let atlas_url = format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        );
        Ok(PetAsset {
            manifest,
            atlas_url,
        })
    })
    .await
    .map_err(|e| CommandError::new("pet_package", e))?
}

#[tauri::command]
pub fn play_alert(source: pet_core::Source) -> Result<()> {
    let prefs = Preferences::load(&notification_paths()?)?;
    if !prefs.sound || prefs.muted_sources.contains(&source) {
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        #[link(name = "AppKit", kind = "framework")]
        unsafe extern "C" {
            fn NSBeep();
        }
        // 系统提醒音不创建窗口，也无需 WebView 的媒体播放授权。
        unsafe {
            NSBeep();
        }
    }
    #[cfg(windows)]
    {
        #[link(name = "user32")]
        unsafe extern "system" {
            fn MessageBeep(kind: u32) -> i32;
        }
        if unsafe { MessageBeep(0x40) } == 0 {
            return Err(CommandError::new("sound", std::io::Error::last_os_error()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod settings_commit_tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    #[derive(Default)]
    struct Host {
        reads: Cell<u32>,
        fail_at: Cell<u32>,
        enabled: Cell<bool>,
        writes: Cell<u32>,
        events: RefCell<Vec<Settings>>,
    }
    impl SettingsHost for Host {
        fn autostart(&self) -> Result<bool> {
            self.reads.set(self.reads.get() + 1);
            if self.reads.get() == self.fail_at.get() {
                Err(CommandError::new("autostart", "读取失败"))
            } else {
                Ok(self.enabled.get())
            }
        }
        fn set_autostart(&self, value: bool) -> Result<()> {
            self.enabled.set(value);
            self.writes.set(self.writes.get() + 1);
            Ok(())
        }
        fn placement(&self) -> WindowPlacement {
            WindowPlacement::default()
        }
        fn apply_placement(&self, _: WindowPlacement) -> Result<()> {
            panic!("未请求位置修改")
        }
        fn publish(&self, value: &Settings) {
            self.events.borrow_mut().push(value.clone());
        }
    }
    #[test]
    fn preference_read_failure_precedes_all_side_effects() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::under(&dir.path().canonicalize().unwrap().join("app"));
        let host = Host::default();
        host.fail_at.set(1);
        let result = commit_settings(
            &host,
            &paths,
            SettingsPatch {
                fps: Some(60),
                ..Default::default()
            },
        );
        assert!(result.is_err());
        assert!(!paths.settings.exists());
        assert_eq!(host.writes.get(), 0);
        assert!(host.events.borrow().is_empty());
    }
    #[test]
    fn confirmed_login_change_publishes_and_does_not_requery_after_save() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::under(&dir.path().canonicalize().unwrap().join("app"));
        let host = Host::default();
        host.fail_at.set(2);
        let result = commit_settings(
            &host,
            &paths,
            SettingsPatch {
                fps: Some(60),
                autostart: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(host.reads.get(), 1);
        assert_eq!(host.writes.get(), 1);
        assert!(host.enabled.get());
        assert_eq!(host.events.borrow().len(), 1);
        assert_eq!(result.preferences, Preferences::load(&paths).unwrap());
        assert_eq!(host.events.borrow()[0].preferences, result.preferences);
        assert_eq!(result.preferences.fps, 60);
        assert!(result.preferences.autostart);
    }
}
