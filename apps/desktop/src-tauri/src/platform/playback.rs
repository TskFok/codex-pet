use serde::Serialize;
use std::sync::Mutex;
use tauri::{Emitter, Manager};

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PlaybackSignal {
    pub(crate) visible: bool,
    pub(super) suspended: bool,
    pub(super) revision: u64,
}

impl Default for PlaybackSignal {
    fn default() -> Self {
        Self {
            visible: true,
            suspended: false,
            revision: 0,
        }
    }
}

#[derive(Default)]
pub(crate) struct Playback(Mutex<PlaybackSignal>);

impl Playback {
    pub(crate) fn snapshot(&self) -> PlaybackSignal {
        *self.0.lock().unwrap()
    }

    fn update(&self, visible: Option<bool>, suspended: Option<bool>) -> Option<PlaybackSignal> {
        let mut state = self.0.lock().unwrap();
        let visible = visible.unwrap_or(state.visible);
        let suspended = suspended.unwrap_or(state.suspended);
        if (visible, suspended) == (state.visible, state.suspended) {
            return None;
        }
        *state = PlaybackSignal {
            visible,
            suspended,
            revision: state.revision + 1,
        };
        Some(*state)
    }
}

pub(super) fn set_visible(app: &tauri::AppHandle, visible: bool) {
    emit(app, Some(visible), None);
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(super) fn set_suspended(app: &tauri::AppHandle, suspended: bool) {
    emit(app, None, Some(suspended));
}

fn emit(app: &tauri::AppHandle, visible: Option<bool>, suspended: Option<bool>) {
    if let Some(signal) = app.state::<Playback>().update(visible, suspended) {
        let _ = app.emit_to("pet", "pet:playback", signal);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wake_does_not_show_hidden_pet_and_repeated_power_notices_do_not_advance_revision() {
        let playback = Playback::default();
        assert!(playback.update(None, Some(true)).unwrap().suspended);
        assert!(playback.update(Some(false), None).is_some());
        let wake = playback.update(None, Some(false)).unwrap();
        assert!(!wake.visible);
        assert!(!wake.suspended);
        assert_eq!(wake.revision, 3);
        assert!(playback.update(None, Some(false)).is_none());
    }
}
