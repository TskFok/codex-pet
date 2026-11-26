//! 持久位置为目标显示器工作区左上角的逻辑偏移。只在同一显示器内做 DPI 换算。
use pet_core::{AppPaths, StorageError, paths};
use serde::{Deserialize, Serialize};
use std::{fs::OpenOptions, io::Read};

pub const WIDTH: f64 = 240.;
pub const HEIGHT: f64 = 440.;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowPlacement {
    pub x: f64,
    pub y: f64,
    pub scale: f64,
    pub monitor_id: Option<String>,
}

impl Default for WindowPlacement {
    fn default() -> Self {
        Self {
            x: 120.,
            y: 180.,
            scale: 1.,
            monitor_id: None,
        }
    }
}

/// 平台提供的屏幕坐标和工作区尺寸，以目标屏幕像素计。不能跨屏比较不同坐标系的原点。
pub struct WorkArea {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale_factor: f64,
}
impl WorkArea {
    pub(crate) fn usable(&self) -> bool {
        [self.width, self.height, self.scale_factor]
            .into_iter()
            .all(|n| n.is_finite() && n > 0.)
    }
    pub fn capture(&self, x: f64, y: f64, scale: f64) -> WindowPlacement {
        WindowPlacement {
            x: (x - self.x) / self.scale_factor,
            y: (y - self.y) / self.scale_factor,
            scale,
            monitor_id: Some(self.id.clone()),
        }
    }
    pub fn position(&self, p: &WindowPlacement) -> (f64, f64) {
        (
            self.x + p.x * self.scale_factor,
            self.y + p.y * self.scale_factor,
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesktopContext {
    pub placement: WindowPlacement,
    pub work_area_width: f64,
    pub work_area_height: f64,
    pub max_x: f64,
    pub max_y: f64,
}

/// 列表首项为主显示器。短时没有工作区时沿用原位置，等待下次显示器事件。
pub fn clamp_placement(mut value: WindowPlacement, monitors: &[WorkArea]) -> WindowPlacement {
    let usable = monitors.iter().filter(|area| area.usable());
    let Some(area) = usable
        .clone()
        .find(|area| Some(&area.id) == value.monitor_id.as_ref())
        .or_else(|| usable.into_iter().next())
    else {
        return value;
    };
    let width = area.width / area.scale_factor;
    let height = area.height / area.scale_factor;
    if !value.scale.is_finite() || value.scale <= 0. {
        value.scale = 1.;
    }
    value.scale = value.scale.min(width / WIDTH).min(height / HEIGHT);
    value.x = value.x.max(0.).min((width - WIDTH * value.scale).max(0.));
    value.y = value.y.max(0.).min((height - HEIGHT * value.scale).max(0.));
    value.monitor_id = Some(area.id.clone());
    value
}

pub fn desktop_context_for(
    value: WindowPlacement,
    monitors: &[WorkArea],
) -> Option<DesktopContext> {
    let fit = clamp_placement(value, monitors);
    let area = monitors
        .iter()
        .find(|area| area.usable() && Some(&area.id) == fit.monitor_id.as_ref())?;
    let work_area_width = area.width / area.scale_factor;
    let work_area_height = area.height / area.scale_factor;
    let max_x = (work_area_width - WIDTH * fit.scale).max(0.);
    let max_y = (work_area_height - HEIGHT * fit.scale).max(0.);
    Some(DesktopContext {
        placement: fit,
        work_area_width,
        work_area_height,
        max_x,
        max_y,
    })
}

pub(crate) fn load(paths: &AppPaths) -> Result<WindowPlacement, StorageError> {
    paths.prepare_root()?;
    let path = paths.root.join("placement.json");
    let file = match paths::open_regular(&path, OpenOptions::new().read(true)) {
        Err(StorageError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(WindowPlacement::default());
        }
        result => result?,
    };
    let mut data = Vec::new();
    file.take(4097).read_to_end(&mut data)?;
    if data.len() > 4096 {
        return Err(StorageError::InvalidData("窗口位置太长"));
    }
    serde_json::from_slice(&data).map_err(|_| StorageError::InvalidData("窗口位置无效"))
}

pub(crate) fn save(paths: &AppPaths, value: &WindowPlacement) -> Result<(), StorageError> {
    paths.prepare_root()?;
    let data = serde_json::to_vec(value).map_err(|_| StorageError::InvalidData("窗口位置无效"))?;
    paths::atomic_write(&paths.root.join("placement.json"), ".placement-", &data)?;
    paths::sync_directory(&paths.root)
}

struct StoredPlacement {
    current: WindowPlacement,
    dirty: bool,
}
pub(crate) struct PlacementState(std::sync::Mutex<StoredPlacement>);

fn persist(value: &WindowPlacement) -> Result<(), StorageError> {
    crate::notification_paths().and_then(|paths| save(&paths, value))
}

impl PlacementState {
    pub fn new(value: WindowPlacement) -> Self {
        Self(std::sync::Mutex::new(StoredPlacement {
            current: value,
            dirty: false,
        }))
    }
    pub fn current(&self) -> WindowPlacement {
        self.0.lock().unwrap().current.clone()
    }
    pub fn remember(&self, value: WindowPlacement) {
        self.remember_with(value, persist);
    }
    fn remember_with(
        &self,
        value: WindowPlacement,
        write: impl FnOnce(&WindowPlacement) -> Result<(), StorageError>,
    ) {
        let mut placement = self.0.lock().unwrap();
        if placement.current != value {
            placement.current = value;
            placement.dirty = true;
        }
        placement.flush(write);
    }
    pub fn retry_write(&self) {
        self.0.lock().unwrap().flush(persist);
    }
}

impl StoredPlacement {
    fn flush(&mut self, write: impl FnOnce(&WindowPlacement) -> Result<(), StorageError>) {
        if self.dirty {
            match write(&self.current) {
                Ok(()) => self.dirty = false,
                Err(error) => eprintln!("保存桌宠位置失败：{error}"),
            }
        }
    }
}

#[cfg(test)]
mod persistence_tests {
    use super::*;

    #[test]
    fn same_position_is_retried_after_failed_atomic_publish() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::under(&temp.path().canonicalize().unwrap().join("app"));
        paths.prepare_root().unwrap();
        let state = PlacementState::new(WindowPlacement::default());
        let target = WindowPlacement {
            x: 85.,
            ..WindowPlacement::default()
        };
        let path = paths.root.join("placement.json");
        std::fs::create_dir(&path).unwrap();
        let write = |value: &WindowPlacement| save(&paths, value);
        state.remember_with(target.clone(), write);
        assert_eq!(state.current(), target);
        std::fs::remove_dir(&path).unwrap();
        state.remember_with(target.clone(), write);
        assert_eq!(load(&paths).unwrap(), target);
    }
}
