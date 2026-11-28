use crate::{
    commands::{self, PetChoice},
    notification_paths,
    placement::PlacementState,
    platform,
    settings::{PlacementPatch, Preferences, SettingsPatch},
};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{
    Manager, WebviewUrl, WebviewWindowBuilder,
    menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::TrayIconBuilder,
};

const SCALE_PRESETS: &[(u16, f64)] = &[(75, 0.75), (100, 1.0), (125, 1.25), (150, 1.5), (200, 2.0)];
const MENU_NOTIFICATIONS_ID: &str = "pet-menu:notifications";
const MENU_SETTINGS_ID: &str = "pet-menu:settings";
const MENU_HIDE_ID: &str = "pet-menu:hide";
const MENU_QUIT_ID: &str = "pet-menu:quit";
const MENU_SOUND_ON_ID: &str = "pet-menu:sound:on";
const MENU_SOUND_OFF_ID: &str = "pet-menu:sound:off";
const MENU_INTERACTION_SOUND_ON_ID: &str = "pet-menu:interaction-sound:on";
const MENU_INTERACTION_SOUND_OFF_ID: &str = "pet-menu:interaction-sound:off";
const MENU_NOTIFY_SUBAGENTS_ON_ID: &str = "pet-menu:notify-subagents:on";
const MENU_NOTIFY_SUBAGENTS_OFF_ID: &str = "pet-menu:notify-subagents:off";
const MENU_AUTONOMY_ON_ID: &str = "pet-menu:autonomy:on";
const MENU_AUTONOMY_OFF_ID: &str = "pet-menu:autonomy:off";
const MENU_SCALE_PREFIX: &str = "pet-menu:scale:";
const MENU_PET_PREFIX: &str = "pet-menu:pet:";

#[derive(Debug, PartialEq)]
enum MenuAction {
    TogglePet,
    HidePet,
    OpenNotifications,
    OpenSettings,
    SetSound(bool),
    SetInteractionSound(bool),
    SetNotifySubagents(bool),
    SetAutonomy(bool),
    SetScale(f64),
    SelectPet(String),
    Quit,
}

impl MenuAction {
    fn parse(id: &str) -> Option<Self> {
        match id {
            "toggle-pet" => Some(Self::TogglePet),
            MENU_HIDE_ID => Some(Self::HidePet),
            "notifications" | MENU_NOTIFICATIONS_ID => Some(Self::OpenNotifications),
            "settings" | MENU_SETTINGS_ID => Some(Self::OpenSettings),
            "quit" | MENU_QUIT_ID => Some(Self::Quit),
            MENU_SOUND_ON_ID => Some(Self::SetSound(true)),
            MENU_SOUND_OFF_ID => Some(Self::SetSound(false)),
            MENU_INTERACTION_SOUND_ON_ID => Some(Self::SetInteractionSound(true)),
            MENU_INTERACTION_SOUND_OFF_ID => Some(Self::SetInteractionSound(false)),
            MENU_NOTIFY_SUBAGENTS_ON_ID => Some(Self::SetNotifySubagents(true)),
            MENU_NOTIFY_SUBAGENTS_OFF_ID => Some(Self::SetNotifySubagents(false)),
            MENU_AUTONOMY_ON_ID => Some(Self::SetAutonomy(true)),
            MENU_AUTONOMY_OFF_ID => Some(Self::SetAutonomy(false)),
            _ => {
                if let Some(percent) = id.strip_prefix(MENU_SCALE_PREFIX) {
                    let percent: u16 = percent.parse().ok()?;
                    let (_, scale) = SCALE_PRESETS.iter().find(|&&(p, _)| p == percent)?;
                    return Some(Self::SetScale(*scale));
                }
                if let Some(pet_id) = id.strip_prefix(MENU_PET_PREFIX)
                    && pet_core::pet_package::valid_pet_id(pet_id)
                {
                    return Some(Self::SelectPet(pet_id.to_owned()));
                }
                None
            }
        }
    }
}

fn scale_matches(current: f64, preset: f64) -> bool {
    (current - preset).abs() < 1e-6
}

fn sound_menu_id(currently_enabled: bool) -> &'static str {
    if currently_enabled {
        MENU_SOUND_OFF_ID
    } else {
        MENU_SOUND_ON_ID
    }
}

fn interaction_sound_menu_id(currently_enabled: bool) -> &'static str {
    if currently_enabled {
        MENU_INTERACTION_SOUND_OFF_ID
    } else {
        MENU_INTERACTION_SOUND_ON_ID
    }
}

fn notify_subagents_menu_id(currently_enabled: bool) -> &'static str {
    if currently_enabled {
        MENU_NOTIFY_SUBAGENTS_OFF_ID
    } else {
        MENU_NOTIFY_SUBAGENTS_ON_ID
    }
}

fn autonomy_menu_id(currently_enabled: bool) -> &'static str {
    if currently_enabled {
        MENU_AUTONOMY_OFF_ID
    } else {
        MENU_AUTONOMY_ON_ID
    }
}

fn open_panel(app: &tauri::AppHandle, label: &str, title: &str) -> tauri::Result<()> {
    let window = match app.get_webview_window(label) {
        Some(window) => window,
        None => {
            let (width, height, min_width, min_height) = if label == "settings" {
                (780., 600., 620., 460.)
            } else {
                (560., 560., 420., 360.)
            };
            WebviewWindowBuilder::new(
                app,
                label,
                WebviewUrl::App(format!("index.html#{label}").into()),
            )
            .title(title)
            .inner_size(width, height)
            .min_inner_size(min_width, min_height)
            .visible(false)
            .build()?
        }
    };
    window.unminimize()?;
    window.show()?;
    window.set_focus()
}

fn visibility_title(visible: bool) -> &'static str {
    if visible {
        "隐藏桌宠"
    } else {
        "显示桌宠"
    }
}

struct ToggleItem(MenuItem<tauri::Wry>);
pub(crate) fn sync_visibility(app: &tauri::AppHandle, visible: bool) -> tauri::Result<()> {
    if let Some(toggle) = app.try_state::<ToggleItem>() {
        toggle.0.set_text(visibility_title(visible))?;
    }
    Ok(())
}

fn request_panel(
    app: &tauri::AppHandle,
    label: &'static str,
    title: &'static str,
) -> tauri::Result<()> {
    #[cfg(target_os = "windows")]
    {
        // WebView2 不允许在同步窗口或托盘回调里建窗。同一标签的连续点击共享一个窗口。
        static OPENING: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let app = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let _guard = OPENING.lock().unwrap();
            if let Err(error) = open_panel(&app, label, title) {
                eprintln!("打开{title}失败：{error}");
            }
        });
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    open_panel(app, label, title)
}

pub(crate) fn open_notifications(app: &tauri::AppHandle) -> tauri::Result<()> {
    request_panel(app, "notifications", "通知")
}

fn load_context_menu_state(app: &tauri::AppHandle) -> (Preferences, Vec<PetChoice>, f64) {
    let (prefs, pets) = match notification_paths() {
        Ok(paths) => (
            Preferences::load(&paths).unwrap_or_default(),
            commands::list_pet_choices(&paths).unwrap_or_else(|_| commands::builtin_pet_choices()),
        ),
        Err(_) => (Preferences::default(), commands::builtin_pet_choices()),
    };
    let scale = app
        .try_state::<PlacementState>()
        .map(|state| state.current().scale)
        .unwrap_or(1.0);
    (prefs, pets, scale)
}

fn build_pet_context_menu(app: &tauri::AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let (prefs, pets, current_scale) = load_context_menu_state(app);

    let notifications = MenuItem::with_id(app, MENU_NOTIFICATIONS_ID, "通知", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, MENU_SETTINGS_ID, "设置", true, None::<&str>)?;
    let separator_top = PredefinedMenuItem::separator(app)?;

    let pet_items: Vec<CheckMenuItem<tauri::Wry>> = pets
        .iter()
        .map(|pet| {
            CheckMenuItem::with_id(
                app,
                format!("{MENU_PET_PREFIX}{}", pet.id),
                &pet.name,
                true,
                pet.id == prefs.pet_id,
                None::<&str>,
            )
        })
        .collect::<tauri::Result<_>>()?;
    let pet_refs: Vec<&dyn IsMenuItem<tauri::Wry>> = pet_items
        .iter()
        .map(|item| item as &dyn IsMenuItem<tauri::Wry>)
        .collect();
    let pet_submenu = Submenu::with_items(app, "切换宠物", true, &pet_refs)?;

    let scale_items: Vec<CheckMenuItem<tauri::Wry>> = SCALE_PRESETS
        .iter()
        .map(|&(percent, scale)| {
            CheckMenuItem::with_id(
                app,
                format!("{MENU_SCALE_PREFIX}{percent}"),
                format!("{percent}%"),
                true,
                scale_matches(current_scale, scale),
                None::<&str>,
            )
        })
        .collect::<tauri::Result<_>>()?;
    let scale_refs: Vec<&dyn IsMenuItem<tauri::Wry>> = scale_items
        .iter()
        .map(|item| item as &dyn IsMenuItem<tauri::Wry>)
        .collect();
    let scale_submenu = Submenu::with_items(app, "显示大小", true, &scale_refs)?;

    let interaction_sound = CheckMenuItem::with_id(
        app,
        interaction_sound_menu_id(prefs.interaction_sound),
        "互动音效",
        true,
        prefs.interaction_sound,
        None::<&str>,
    )?;
    let sound = CheckMenuItem::with_id(
        app,
        sound_menu_id(prefs.sound),
        "提醒声音",
        true,
        prefs.sound,
        None::<&str>,
    )?;
    let notify_subagents = CheckMenuItem::with_id(
        app,
        notify_subagents_menu_id(prefs.notify_subagents),
        "提示 Subagent 信息",
        true,
        prefs.notify_subagents,
        None::<&str>,
    )?;
    let has_deepseek_key = !prefs.deepseek_api_key.trim().is_empty();
    let autonomy = CheckMenuItem::with_id(
        app,
        autonomy_menu_id(prefs.deepseek_enabled),
        if has_deepseek_key {
            "自由活动（DeepSeek）"
        } else {
            "自由活动（需配置 DeepSeek Key）"
        },
        has_deepseek_key,
        has_deepseek_key && prefs.deepseek_enabled,
        None::<&str>,
    )?;
    let separator_bottom = PredefinedMenuItem::separator(app)?;
    let hide = MenuItem::with_id(app, MENU_HIDE_ID, "隐藏桌宠", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, MENU_QUIT_ID, "退出", true, None::<&str>)?;

    Menu::with_items(
        app,
        &[
            &notifications,
            &settings,
            &separator_top,
            &pet_submenu,
            &scale_submenu,
            &interaction_sound,
            &sound,
            &notify_subagents,
            &autonomy,
            &separator_bottom,
            &hide,
            &quit,
        ],
    )
}

static CONTEXT_MENU_OPEN: AtomicBool = AtomicBool::new(false);

struct ContextMenuGuard;
impl Drop for ContextMenuGuard {
    fn drop(&mut self) {
        CONTEXT_MENU_OPEN.store(false, Ordering::SeqCst);
    }
}

pub(crate) fn show_pet_context_menu(app: &tauri::AppHandle) -> tauri::Result<()> {
    if CONTEXT_MENU_OPEN
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Ok(());
    }
    let _guard = ContextMenuGuard;
    let Some(window) = app.get_webview_window("pet") else {
        return Ok(());
    };
    let menu = build_pet_context_menu(app)?;
    window.popup_menu(&menu)
}

fn handle_menu_event(app: &tauri::AppHandle, id: &str) {
    let Some(action) = MenuAction::parse(id) else {
        return;
    };
    let result: Result<(), String> = match action {
        MenuAction::TogglePet => {
            let next = !app.state::<platform::Playback>().snapshot().visible;
            platform::set_pet_visible(app, next).map_err(|e| e.to_string())
        }
        MenuAction::HidePet => platform::set_pet_visible(app, false).map_err(|e| e.to_string()),
        MenuAction::OpenNotifications => {
            request_panel(app, "notifications", "通知").map_err(|e| e.to_string())
        }
        MenuAction::OpenSettings => {
            request_panel(app, "settings", "设置").map_err(|e| e.to_string())
        }
        MenuAction::SetSound(sound) => commands::apply_settings_patch(
            app,
            SettingsPatch {
                sound: Some(sound),
                ..Default::default()
            },
        )
        .map(|_| ())
        .map_err(|e| e.message),
        MenuAction::SetInteractionSound(interaction_sound) => commands::apply_settings_patch(
            app,
            SettingsPatch {
                interaction_sound: Some(interaction_sound),
                ..Default::default()
            },
        )
        .map(|_| ())
        .map_err(|e| e.message),
        MenuAction::SetNotifySubagents(notify_subagents) => commands::apply_settings_patch(
            app,
            SettingsPatch {
                notify_subagents: Some(notify_subagents),
                ..Default::default()
            },
        )
        .map(|_| ())
        .map_err(|e| e.message),
        MenuAction::SetAutonomy(enabled) => commands::apply_settings_patch(
            app,
            SettingsPatch {
                deepseek_enabled: Some(enabled),
                ..Default::default()
            },
        )
        .map(|_| ())
        .map_err(|e| e.message),
        MenuAction::SetScale(scale) => commands::apply_settings_patch(
            app,
            SettingsPatch {
                placement: Some(PlacementPatch::Scale { scale }),
                ..Default::default()
            },
        )
        .map(|_| ())
        .map_err(|e| e.message),
        MenuAction::SelectPet(pet_id) => commands::apply_settings_patch(
            app,
            SettingsPatch {
                pet_id: Some(pet_id),
                ..Default::default()
            },
        )
        .map(|_| ())
        .map_err(|e| e.message),
        MenuAction::Quit => {
            app.exit(0);
            Ok(())
        }
    };
    if let Err(error) = result {
        eprintln!("桌宠菜单操作失败：{error}");
    }
}

pub(crate) fn install(app: &tauri::AppHandle) -> tauri::Result<()> {
    let item = |id: &str, title: &str| MenuItem::with_id(app, id, title, true, None::<&str>);
    let toggle = item(
        "toggle-pet",
        visibility_title(app.state::<platform::Playback>().snapshot().visible),
    )?;
    let menu = Menu::with_items(
        app,
        &[
            &toggle,
            &item("notifications", "通知")?,
            &item("settings", "设置")?,
            &item("quit", "退出")?,
        ],
    )?;
    app.manage(ToggleItem(toggle));
    let icon = app
        .default_window_icon()
        .ok_or_else(|| std::io::Error::other("缺少桌宠托盘图标"))?;
    TrayIconBuilder::with_id("pet-tray")
        .tooltip("Codex 桌宠")
        .icon(icon.clone())
        .menu(&menu)
        .on_menu_event(move |app, event| {
            handle_menu_event(app, event.id.as_ref());
        })
        .build(app)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_action_parses_tray_and_context_menu_ids() {
        assert_eq!(MenuAction::parse("toggle-pet"), Some(MenuAction::TogglePet));
        assert_eq!(MenuAction::parse(MENU_HIDE_ID), Some(MenuAction::HidePet));
        assert_eq!(
            MenuAction::parse("notifications"),
            Some(MenuAction::OpenNotifications)
        );
        assert_eq!(
            MenuAction::parse(MENU_NOTIFICATIONS_ID),
            Some(MenuAction::OpenNotifications)
        );
        assert_eq!(
            MenuAction::parse("settings"),
            Some(MenuAction::OpenSettings)
        );
        assert_eq!(
            MenuAction::parse(MENU_SETTINGS_ID),
            Some(MenuAction::OpenSettings)
        );
        assert_eq!(MenuAction::parse("quit"), Some(MenuAction::Quit));
        assert_eq!(MenuAction::parse(MENU_QUIT_ID), Some(MenuAction::Quit));
        assert_eq!(
            MenuAction::parse(sound_menu_id(false)),
            Some(MenuAction::SetSound(true))
        );
        assert_eq!(
            MenuAction::parse(sound_menu_id(true)),
            Some(MenuAction::SetSound(false))
        );
        assert_eq!(
            MenuAction::parse(interaction_sound_menu_id(false)),
            Some(MenuAction::SetInteractionSound(true))
        );
        assert_eq!(
            MenuAction::parse(interaction_sound_menu_id(true)),
            Some(MenuAction::SetInteractionSound(false))
        );
        assert_eq!(
            MenuAction::parse(notify_subagents_menu_id(false)),
            Some(MenuAction::SetNotifySubagents(true))
        );
        assert_eq!(
            MenuAction::parse(notify_subagents_menu_id(true)),
            Some(MenuAction::SetNotifySubagents(false))
        );
        assert_eq!(
            MenuAction::parse(autonomy_menu_id(false)),
            Some(MenuAction::SetAutonomy(true))
        );
        assert_eq!(
            MenuAction::parse(autonomy_menu_id(true)),
            Some(MenuAction::SetAutonomy(false))
        );
    }

    #[test]
    fn menu_action_validates_scale_presets_and_pet_ids() {
        for &(percent, scale) in SCALE_PRESETS {
            assert_eq!(
                MenuAction::parse(&format!("{MENU_SCALE_PREFIX}{percent}")),
                Some(MenuAction::SetScale(scale))
            );
            assert!(scale_matches(scale, scale));
        }
        assert_eq!(MenuAction::parse("pet-menu:scale:999"), None);
        assert_eq!(MenuAction::parse("pet-menu:scale:abc"), None);
        assert_eq!(
            MenuAction::parse("pet-menu:pet:yueying-cape-dancer"),
            Some(MenuAction::SelectPet("yueying-cape-dancer".into()))
        );
        assert_eq!(
            MenuAction::parse("pet-menu:pet:deepseek-whale-maid"),
            Some(MenuAction::SelectPet("deepseek-whale-maid".into()))
        );
        assert_eq!(MenuAction::parse("pet-menu:pet:../escape"), None);
        assert_eq!(MenuAction::parse("pet-menu:pet:"), None);
        assert_eq!(MenuAction::parse("unknown-action"), None);
    }
}
