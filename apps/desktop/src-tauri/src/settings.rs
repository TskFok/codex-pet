use crate::placement::WindowPlacement;
use pet_core::{AppPaths, Source, StorageError, paths};
use serde::{Deserialize, Serialize};
use std::{fs::OpenOptions, io::Read};

pub const DEFAULT_PET: &str = "yueying-cape-dancer";

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Preferences {
    pub revision: u64,
    pub fps: u8,
    pub sound: bool,
    pub autostart: bool,
    pub pet_id: String,
    pub muted_sources: Vec<Source>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            revision: 0,
            fps: 30,
            sound: false,
            autostart: false,
            pet_id: DEFAULT_PET.into(),
            muted_sources: Vec::new(),
        }
    }
}
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsPatch {
    pub fps: Option<u8>,
    pub sound: Option<bool>,
    pub autostart: Option<bool>,
    pub pet_id: Option<String>,
    pub muted_sources: Option<Vec<Source>>,
    pub placement: Option<PlacementPatch>,
}
#[derive(Debug, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum PlacementPatch {
    Position(WindowPlacement),
    Scale { scale: f64 },
}
impl PlacementPatch {
    pub fn resolve(&self, current: &WindowPlacement) -> WindowPlacement {
        match self {
            Self::Position(position) => position.clone(),
            Self::Scale { scale } => WindowPlacement {
                scale: *scale,
                ..current.clone()
            },
        }
    }
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(flatten)]
    pub preferences: Preferences,
    pub placement: WindowPlacement,
}

impl Preferences {
    pub fn load(paths: &AppPaths) -> Result<Self, StorageError> {
        paths.prepare_root()?;
        let file = match paths::open_regular(&paths.settings, OpenOptions::new().read(true)) {
            Err(StorageError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            file => file?,
        };
        let mut data = Vec::new();
        file.take(16385).read_to_end(&mut data)?;
        if data.len() > 16384 {
            return Err(StorageError::InvalidData("设置文件过大"));
        }
        let prefs: Self = serde_json::from_slice(&data)
            .map_err(|_| StorageError::InvalidData("设置文件格式无效"))?;
        prefs.validate()?;
        Ok(prefs)
    }
    fn validate(&self) -> Result<(), StorageError> {
        if !matches!(self.fps, 30 | 60) {
            return Err(StorageError::InvalidData("帧率必须是 30 或 60"));
        }
        if !pet_core::pet_package::valid_pet_id(&self.pet_id) {
            return Err(StorageError::InvalidData("宠物标识无效"));
        }
        Ok(())
    }
    pub fn patched(&self, patch: &SettingsPatch) -> Result<Self, StorageError> {
        let mut next = self.clone();
        if let Some(fps) = patch.fps {
            next.fps = fps;
        }
        if let Some(sound) = patch.sound {
            next.sound = sound;
        }
        if let Some(autostart) = patch.autostart {
            next.autostart = autostart;
        }
        if let Some(id) = &patch.pet_id {
            next.pet_id = id.clone();
        }
        if let Some(sources) = &patch.muted_sources {
            next.muted_sources = sources.clone();
            next.muted_sources.sort();
            next.muted_sources.dedup();
        }
        if let Some(p) = patch
            .placement
            .as_ref()
            .map(|p| p.resolve(&WindowPlacement::default()))
            && (!p.x.is_finite() || !p.y.is_finite() || !p.scale.is_finite() || p.scale <= 0.0)
        {
            return Err(StorageError::InvalidData("窗口位置或缩放无效"));
        }
        next.validate()?;
        if next != *self || patch.placement.is_some() {
            next.revision = self
                .revision
                .checked_add(1)
                .ok_or(StorageError::InvalidData("设置修订号溢出"))?;
        }
        Ok(next)
    }
    pub fn save(&self, paths: &AppPaths) -> Result<(), StorageError> {
        self.validate()?;
        paths.prepare_root()?;
        let data = serde_json::to_vec_pretty(self)
            .map_err(|_| StorageError::InvalidData("无法编码设置"))?;
        paths::atomic_write(&paths.settings, ".settings-", &data)?;
        paths::sync_directory(&paths.root)
    }
}
