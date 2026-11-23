use crate::{
    NotificationConnection, NotificationRuntime,
    broker::{BrokerView, HistoryAction},
    notification_paths,
    placement::PlacementState,
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
#[tauri::command]
pub fn get_playback(state: State<'_, platform::Playback>) -> platform::PlaybackSignal {
    state.snapshot()
}

fn settings(app: &tauri::AppHandle, mut prefs: Preferences) -> Result<Settings> {
    prefs.autostart = app
        .autolaunch()
        .is_enabled()
        .map_err(|e| CommandError::new("autostart", e))?;
    Ok(Settings {
        preferences: prefs,
        placement: app.state::<PlacementState>().current(),
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
    let paths = notification_paths()?;
    let current = Preferences::load(&paths)?;
    let next = current.patched(&patch)?;
    if patch.pet_id.is_some() && next.pet_id != DEFAULT_PET {
        verified_pet(&paths, &next.pet_id)?;
    }
    let previous_placement = app.state::<PlacementState>().current();
    let target_placement = patch
        .placement
        .as_ref()
        .map(|p| p.resolve(&previous_placement));
    let placement_changed = target_placement
        .as_ref()
        .is_some_and(|p| p != &previous_placement);
    let mut previous_autostart = None;
    if let Some(value) = patch.autostart {
        let old = app
            .autolaunch()
            .is_enabled()
            .map_err(|e| CommandError::new("autostart", e))?;
        if old != value {
            previous_autostart = Some(old);
        }
    }
    let result = (|| {
        if previous_autostart.is_some() {
            set_autostart(&app, next.autostart)?;
        }
        if let Some(target) = target_placement
            && placement_changed
        {
            platform::apply_placement(&app, target)
                .map_err(|e| CommandError::new("placement", e))?;
            if crate::placement::load(&paths)? != app.state::<PlacementState>().current() {
                return Err(CommandError::new("placement", "未能保存桌宠位置，请重试"));
            }
        }
        if next != current {
            next.save(&paths)?;
        }
        Ok(())
    })();
    if let Err(mut error) = result {
        let mut rollback = Vec::new();
        if placement_changed && let Err(e) = platform::apply_placement(&app, previous_placement) {
            rollback.push(e.to_string());
        }
        if let Some(old) = previous_autostart
            && let Err(e) = set_autostart(&app, old)
        {
            rollback.push(e.message);
        }
        if !rollback.is_empty() {
            error
                .message
                .push_str(&format!("；还原系统设置时出错：{}", rollback.join("，")));
        }
        return Err(error);
    }
    let result = settings(&app, next)?;
    let _ = app.emit("pet:settings", &result);
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
