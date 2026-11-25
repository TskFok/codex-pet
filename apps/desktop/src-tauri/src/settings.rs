use crate::placement::WindowPlacement;
use pet_core::{AppPaths, Source, StorageError, paths};
use serde::{Deserialize, Serialize, Serializer};
use std::{fmt, fs::OpenOptions, io::Read};

pub const DEFAULT_PET: &str = "yueying-cape-dancer";
pub const DEEPSEEK_DEFAULT_MODEL: &str = "deepseek-chat";
pub const DEEPSEEK_MODELS: &[&str] = &["deepseek-chat", "deepseek-reasoner"];
const MAX_API_KEY_LEN: usize = 512;

pub fn valid_deepseek_model(model: &str) -> bool {
    DEEPSEEK_MODELS.contains(&model)
}

pub fn mask_api_key(key: &str) -> Option<String> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return None;
    }
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() >= 10 {
        let prefix: String = chars[..4].iter().collect();
        let suffix: String = chars[chars.len() - 4..].iter().collect();
        Some(format!("{prefix}•••{suffix}"))
    } else {
        Some("••••".into())
    }
}

#[derive(Clone, PartialEq, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Preferences {
    pub revision: u64,
    pub fps: u8,
    pub sound: bool,
    pub autostart: bool,
    pub pet_id: String,
    pub muted_sources: Vec<Source>,
    pub deepseek_enabled: bool,
    pub deepseek_api_key: String,
    pub deepseek_model: String,
}
impl fmt::Debug for Preferences {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Preferences")
            .field("revision", &self.revision)
            .field("fps", &self.fps)
            .field("sound", &self.sound)
            .field("autostart", &self.autostart)
            .field("pet_id", &self.pet_id)
            .field("muted_sources", &self.muted_sources)
            .field("deepseek_enabled", &self.deepseek_enabled)
            .field(
                "deepseek_api_key",
                &if self.deepseek_api_key.is_empty() {
                    ""
                } else {
                    "[redacted]"
                },
            )
            .field("deepseek_model", &self.deepseek_model)
            .finish()
    }
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
            deepseek_enabled: true,
            deepseek_api_key: String::new(),
            deepseek_model: DEEPSEEK_DEFAULT_MODEL.into(),
        }
    }
}
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsPatch {
    pub fps: Option<u8>,
    pub sound: Option<bool>,
    pub autostart: Option<bool>,
    pub pet_id: Option<String>,
    pub muted_sources: Option<Vec<Source>>,
    pub placement: Option<PlacementPatch>,
    pub deepseek_enabled: Option<bool>,
    pub deepseek_api_key: Option<String>,
    pub deepseek_model: Option<String>,
}
impl fmt::Debug for SettingsPatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SettingsPatch")
            .field("fps", &self.fps)
            .field("sound", &self.sound)
            .field("autostart", &self.autostart)
            .field("pet_id", &self.pet_id)
            .field("muted_sources", &self.muted_sources)
            .field("placement", &self.placement)
            .field("deepseek_enabled", &self.deepseek_enabled)
            .field(
                "deepseek_api_key",
                &self
                    .deepseek_api_key
                    .as_deref()
                    .map(|key| if key.is_empty() { "" } else { "[redacted]" }),
            )
            .field("deepseek_model", &self.deepseek_model)
            .finish()
    }
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
#[derive(Clone)]
pub struct Settings {
    pub preferences: Preferences,
    pub placement: WindowPlacement,
}

/// 面向前端 IPC 的设置视图，绝不向 WebView 广播或回传明文 DeepSeek API Key。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SettingsWire<'a> {
    revision: u64,
    fps: u8,
    sound: bool,
    autostart: bool,
    pet_id: &'a str,
    muted_sources: &'a [Source],
    deepseek_enabled: bool,
    deepseek_model: &'a str,
    deepseek_api_key_configured: bool,
    deepseek_api_key_hint: Option<String>,
    placement: &'a WindowPlacement,
}

impl Serialize for Settings {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let prefs = &self.preferences;
        let hint = mask_api_key(&prefs.deepseek_api_key);
        SettingsWire {
            revision: prefs.revision,
            fps: prefs.fps,
            sound: prefs.sound,
            autostart: prefs.autostart,
            pet_id: &prefs.pet_id,
            muted_sources: &prefs.muted_sources,
            deepseek_enabled: prefs.deepseek_enabled,
            deepseek_model: &prefs.deepseek_model,
            deepseek_api_key_configured: hint.is_some(),
            deepseek_api_key_hint: hint,
            placement: &self.placement,
        }
        .serialize(serializer)
    }
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
        if !valid_deepseek_model(&self.deepseek_model) {
            return Err(StorageError::InvalidData("不支持的 DeepSeek 模型"));
        }
        if self.deepseek_api_key.len() > MAX_API_KEY_LEN
            || self
                .deepseek_api_key
                .chars()
                .any(|c| c.is_control() || c.is_whitespace())
        {
            return Err(StorageError::InvalidData("DeepSeek API Key 格式无效"));
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
        if let Some(enabled) = patch.deepseek_enabled {
            next.deepseek_enabled = enabled;
        }
        if let Some(api_key) = &patch.deepseek_api_key {
            next.deepseek_api_key = api_key.trim().to_owned();
        }
        if let Some(model) = &patch.deepseek_model {
            next.deepseek_model = model.trim().to_owned();
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
