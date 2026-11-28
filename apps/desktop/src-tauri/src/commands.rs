use crate::{
    NotificationConnection, NotificationRuntime,
    autonomy::{self, AutonomyRuntime, DeepSeekActivityStatus, DeepSeekClient, DeepSeekTestResult},
    broker::{BrokerView, HistoryAction},
    navigation::{self, OpenIntent, OpenOutcome},
    notification_paths,
    placement::{PlacementState, WindowPlacement},
    platform,
    settings::{
        BUILTIN_PET_IDS, BUILTIN_PETS, Preferences, Settings, SettingsPatch, is_builtin_pet,
    },
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
pub async fn mark_read_many(
    state: State<'_, NotificationRuntime>,
    keys: Vec<SessionKey>,
) -> Result<PetSnapshot> {
    Ok(broker(&state)?
        .history(HistoryAction::MarkReadMany(keys))
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
        let target =
            crate::integrations::navigation_registration(&app, key.source, &key.installation_id)?;
        match target {
            Some(navigation::Target::Desktop(registration)) => {
                navigation::open_session(&key, Some(&registration), intent, |path, url| {
                    navigation::system::dispatch(key.source, path, url)
                })
            }
            Some(navigation::Target::Web {
                source,
                installation_id,
                target,
            }) => navigation::open_web_session(
                &key,
                source,
                &installation_id,
                target.as_ref(),
                intent,
                navigation::system::dispatch_url,
            ),
            None => navigation::open_session(&key, None, intent, |path, url| {
                navigation::system::dispatch(key.source, path, url)
            }),
        }
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
        platform::flush_placement(self);
        self.state::<PlacementState>().current()
    }
    fn apply_placement(&self, value: WindowPlacement) -> Result<()> {
        platform::apply_placement(self, value).map_err(|e| CommandError::new("placement", e))
    }
    fn publish(&self, settings: &Settings) {
        self.state::<NotificationRuntime>()
            .set_notify_subagents(settings.preferences.notify_subagents);
        let _ = self.emit("pet:settings", settings);
        autonomy::on_settings_updated(self);
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

#[tauri::command]
pub fn get_deepseek_activity_status(
    gate: State<'_, SettingsLock>,
    runtime: State<'_, AutonomyRuntime>,
) -> Result<DeepSeekActivityStatus> {
    let _guard = gate.0.lock().unwrap();
    let prefs = Preferences::load(&notification_paths()?)?;
    let mut status = runtime.status();
    status.configured = !prefs.deepseek_api_key.trim().is_empty();
    status.enabled = prefs.deepseek_enabled;
    status.model = prefs.deepseek_model;
    if !status.configured || !status.enabled {
        status.running = false;
    }
    Ok(status)
}

#[tauri::command]
pub async fn test_deepseek_connection(
    gate: State<'_, SettingsLock>,
    api_key: Option<String>,
    model: Option<String>,
) -> Result<DeepSeekTestResult> {
    let (resolved_key, resolved_model) = {
        let _guard = gate.0.lock().unwrap();
        let prefs = Preferences::load(&notification_paths()?)?;
        let key = api_key
            .map(|k| k.trim().to_owned())
            .filter(|k| !k.is_empty())
            .unwrap_or(prefs.deepseek_api_key);
        let model = model
            .map(|m| m.trim().to_owned())
            .filter(|m| !m.is_empty())
            .unwrap_or(prefs.deepseek_model);
        (key, model)
    };
    let client = DeepSeekClient::new().map_err(|e| CommandError::new("deepseek", e.message))?;
    client
        .test_connection(&resolved_key, &resolved_model)
        .await
        .map_err(|e| CommandError::new("deepseek", e.message))
}

#[tauri::command]
pub fn trigger_pet_activity(
    gate: State<'_, SettingsLock>,
    runtime: State<'_, AutonomyRuntime>,
) -> Result<()> {
    let _guard = gate.0.lock().unwrap();
    let prefs = Preferences::load(&notification_paths()?)?;
    if prefs.deepseek_api_key.trim().is_empty() {
        return Err(CommandError::new(
            "deepseek",
            "请先配置 DeepSeek API Key 后再触发自由活动",
        ));
    }
    if !prefs.deepseek_enabled {
        return Err(CommandError::new("deepseek", "请先启用桌宠自由活动开关"));
    }
    runtime.request_manual_trigger();
    Ok(())
}

#[tauri::command]
pub fn interrupt_pet_activity(runtime: State<'_, AutonomyRuntime>) {
    runtime.notify_user_interaction();
}

pub(crate) fn apply_settings_patch(
    app: &tauri::AppHandle,
    patch: SettingsPatch,
) -> Result<Settings> {
    let gate = app.state::<SettingsLock>();
    let _guard = gate.0.lock().unwrap();
    commit_settings(app, &notification_paths()?, patch)
}

fn commit_settings(
    host: &impl SettingsHost,
    paths: &AppPaths,
    patch: SettingsPatch,
) -> Result<Settings> {
    let current = Preferences::load(paths)?;
    let next = current.patched(&patch)?;
    if patch.pet_id.is_some() && !is_builtin_pet(&next.pet_id) {
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
    pub(crate) id: String,
    pub(crate) name: String,
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
        Ok(pet_package::import_pet_to_library_with_reserved(
            &path,
            &paths.pets,
            BUILTIN_PET_IDS,
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
pub(crate) fn builtin_pet_choices() -> Vec<PetChoice> {
    BUILTIN_PETS
        .iter()
        .map(|&(id, name)| PetChoice {
            id: id.into(),
            name: name.into(),
        })
        .collect()
}

pub(crate) fn list_pet_choices(paths: &AppPaths) -> Result<Vec<PetChoice>> {
    let mut choices = builtin_pet_choices();
    paths.prepare_root()?;
    paths::prepare_directory(&paths.pets)?;
    for dir in std::fs::read_dir(&paths.pets).map_err(StorageError::from)? {
        let dir = dir.map_err(StorageError::from)?;
        if !dir.file_type().map_err(StorageError::from)?.is_dir() {
            continue;
        }
        if let Ok(manifest) = pet_package::describe_pet(&dir.path())
            && !is_builtin_pet(&manifest.id)
            && dir.file_name() == manifest.id.as_str()
        {
            choices.push(PetChoice {
                id: manifest.id,
                name: manifest.name,
            });
        }
    }
    choices[BUILTIN_PETS.len()..].sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    Ok(choices)
}

#[tauri::command]
pub async fn list_pets() -> Result<Vec<PetChoice>> {
    tauri::async_runtime::spawn_blocking(|| list_pet_choices(&notification_paths()?))
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
pub fn play_alert(
    app: tauri::AppHandle,
    audio: State<'_, crate::audio::AudioRuntime>,
    playback: State<'_, platform::Playback>,
    source: pet_core::Source,
    cue: Option<crate::audio::PetSoundCue>,
) -> Result<()> {
    if playback.snapshot().suspended {
        return Ok(());
    }
    let prefs = Preferences::load(&notification_paths()?)?;
    if !prefs.sound || prefs.muted_sources.contains(&source) {
        return Ok(());
    }
    let resolved_cue = cue
        .filter(|c| c.category() == crate::audio::SoundCategory::Alert)
        .unwrap_or(crate::audio::PetSoundCue::AlertApproval);
    audio
        .play(
            &app,
            resolved_cue,
            prefs.sound_pack,
            prefs.sound_volume,
            &prefs.pet_id,
        )
        .map_err(|e| CommandError::new("sound", e))
}

#[tauri::command]
pub fn play_pet_sound(
    app: tauri::AppHandle,
    audio: State<'_, crate::audio::AudioRuntime>,
    playback: State<'_, platform::Playback>,
    cue: crate::audio::PetSoundCue,
) -> Result<()> {
    let snap = playback.snapshot();
    if !snap.visible || snap.suspended {
        return Ok(());
    }
    let prefs = Preferences::load(&notification_paths()?)?;
    let enabled = match cue.category() {
        crate::audio::SoundCategory::Interaction => prefs.interaction_sound,
        crate::audio::SoundCategory::Ambient => prefs.ambient_sound,
        crate::audio::SoundCategory::Alert => prefs.sound,
    };
    if !enabled {
        return Ok(());
    }
    audio
        .play(
            &app,
            cue,
            prefs.sound_pack,
            prefs.sound_volume,
            &prefs.pet_id,
        )
        .map_err(|e| CommandError::new("sound", e))
}

#[tauri::command]
pub fn preview_pet_sound(
    app: tauri::AppHandle,
    audio: State<'_, crate::audio::AudioRuntime>,
    playback: State<'_, platform::Playback>,
    cue: crate::audio::PetSoundCue,
    sound_pack: Option<crate::settings::SoundPack>,
    sound_volume: Option<f64>,
    pet_id: Option<String>,
) -> Result<()> {
    if playback.snapshot().suspended {
        return Ok(());
    }
    let prefs = Preferences::load(&notification_paths()?)?;
    let pack = sound_pack.unwrap_or(prefs.sound_pack);
    let requested_volume = sound_volume
        .filter(|v| v.is_finite())
        .unwrap_or(prefs.sound_volume)
        .clamp(0.0, 1.0);
    let effective_volume = if requested_volume <= 0.005 {
        0.30
    } else {
        requested_volume
    };
    let target_pet = pet_id
        .filter(|id| pet_core::pet_package::valid_pet_id(id))
        .unwrap_or(prefs.pet_id);
    audio
        .play_preview(&app, cue, pack, effective_volume, &target_pet)
        .map_err(|e| CommandError::new("sound", e))
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
    fn empty_library_lists_both_builtin_pets_in_default_order() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::under(&dir.path().canonicalize().unwrap().join("app"));
        let choices = list_pet_choices(&paths).unwrap();
        assert_eq!(
            choices
                .iter()
                .map(|pet| (pet.id.as_str(), pet.name.as_str()))
                .collect::<Vec<_>>(),
            [
                ("yueying-cape-dancer", "夏乌拉"),
                ("deepseek-whale-maid", "DeepSeek娘"),
            ]
        );
    }

    #[test]
    fn commits_builtin_pet_without_a_library_and_rejects_unknown_pet() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::under(&dir.path().canonicalize().unwrap().join("app"));
        let host = Host::default();
        let select = |id: &str| {
            commit_settings(
                &host,
                &paths,
                SettingsPatch {
                    pet_id: Some(id.into()),
                    ..Default::default()
                },
            )
        };
        assert_eq!(
            Preferences::load(&paths).unwrap().pet_id,
            "yueying-cape-dancer"
        );
        let selected = select("deepseek-whale-maid").unwrap();
        assert_eq!(selected.preferences.pet_id, "deepseek-whale-maid");
        assert!(!paths.pets.exists());
        assert!(matches!(
            select("unknown-pet"),
            Err(CommandError {
                code: "pet_package",
                ..
            })
        ));
        assert_eq!(Preferences::load(&paths).unwrap(), selected.preferences);
        assert_eq!(host.events.borrow().len(), 1);
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
