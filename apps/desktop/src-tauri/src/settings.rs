use crate::placement::WindowPlacement;
use pet_core::{AppPaths, Source, StorageError, paths};
use serde::{Deserialize, Serialize, Serializer};
use std::{fmt, fs::OpenOptions, io::Read};

pub const DEFAULT_PET: &str = "yueying-cape-dancer";
pub const DEFAULT_PET_NAME: &str = "夏乌拉";
pub const DEEPSEEK_DEFAULT_MODEL: &str = "deepseek-chat";
pub const DEEPSEEK_MODELS: &[&str] = &["deepseek-chat", "deepseek-reasoner"];
pub const DEFAULT_ACTIVITY_SPEED: f64 = 1.0;
pub const MIN_ACTIVITY_SPEED: f64 = 0.5;
pub const MAX_ACTIVITY_SPEED: f64 = 2.0;
pub const MIN_ACTIVITY_RANGE_SPAN: f64 = 0.1;
pub const DEFAULT_BUBBLE_AUTO_CLOSE_SECONDS: u32 = 15;
pub const MAX_BUBBLE_AUTO_CLOSE_SECONDS: u32 = 600;
pub const DEFAULT_SOUND_VOLUME: f64 = 0.7;
pub const MIN_SOUND_VOLUME: f64 = 0.0;
pub const MAX_SOUND_VOLUME: f64 = 1.0;
const MAX_API_KEY_LEN: usize = 512;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SoundPack {
    #[default]
    Healing,
    Retro,
    Minimal,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityRangePreset {
    #[default]
    Full,
    Bottom,
    LowerHalf,
    LeftEdge,
    RightEdge,
    Custom,
}

impl ActivityRangePreset {
    pub fn bounds(self) -> Option<(f64, f64, f64, f64)> {
        match self {
            Self::Full => Some((0.0, 1.0, 0.0, 1.0)),
            Self::Bottom => Some((0.0, 1.0, 0.7, 1.0)),
            Self::LowerHalf => Some((0.0, 1.0, 0.5, 1.0)),
            Self::LeftEdge => Some((0.0, 0.25, 0.0, 1.0)),
            Self::RightEdge => Some((0.75, 1.0, 0.0, 1.0)),
            Self::Custom => None,
        }
    }

    pub fn label_zh(self) -> &'static str {
        match self {
            Self::Full => "全屏工作区",
            Self::Bottom => "屏幕底部",
            Self::LowerHalf => "屏幕下半区",
            Self::LeftEdge => "左侧边缘",
            Self::RightEdge => "右侧边缘",
            Self::Custom => "自定义区域",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivityRange {
    pub preset: ActivityRangePreset,
    pub min_x: f64,
    pub max_x: f64,
    pub min_y: f64,
    pub max_y: f64,
}

impl Default for ActivityRange {
    fn default() -> Self {
        Self::from_preset(ActivityRangePreset::Full)
    }
}

impl ActivityRange {
    pub fn from_preset(preset: ActivityRangePreset) -> Self {
        let (min_x, max_x, min_y, max_y) = preset.bounds().unwrap_or((0.0, 1.0, 0.0, 1.0));
        Self {
            preset,
            min_x,
            max_x,
            min_y,
            max_y,
        }
    }

    pub fn normalized(&self) -> Self {
        if let Some((min_x, max_x, min_y, max_y)) = self.preset.bounds() {
            Self {
                preset: self.preset,
                min_x,
                max_x,
                min_y,
                max_y,
            }
        } else {
            self.clone()
        }
    }

    pub fn validate(&self) -> Result<(), StorageError> {
        let values = [self.min_x, self.max_x, self.min_y, self.max_y];
        if !values
            .into_iter()
            .all(|v| v.is_finite() && (0.0..=1.0).contains(&v))
        {
            return Err(StorageError::InvalidData(
                "自由活动范围坐标必须在 0 到 1 之间",
            ));
        }
        const EPS: f64 = 1e-6;
        if self.max_x - self.min_x + EPS < MIN_ACTIVITY_RANGE_SPAN
            || self.max_y - self.min_y + EPS < MIN_ACTIVITY_RANGE_SPAN
        {
            return Err(StorageError::InvalidData(
                "自由活动范围宽高至少需占屏幕 10%",
            ));
        }
        if let Some((min_x, max_x, min_y, max_y)) = self.preset.bounds()
            && ((self.min_x - min_x).abs() > EPS
                || (self.max_x - max_x).abs() > EPS
                || (self.min_y - min_y).abs() > EPS
                || (self.max_y - max_y).abs() > EPS)
        {
            return Err(StorageError::InvalidData("自由活动范围预设与坐标不匹配"));
        }
        Ok(())
    }
}

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
    pub interaction_sound: bool,
    pub ambient_sound: bool,
    pub sound_pack: SoundPack,
    pub sound_volume: f64,
    pub autostart: bool,
    pub pet_id: String,
    pub muted_sources: Vec<Source>,
    pub notify_subagents: bool,
    pub deepseek_enabled: bool,
    pub deepseek_api_key: String,
    pub deepseek_model: String,
    pub activity_range: ActivityRange,
    pub activity_speed: f64,
    pub bubble_auto_close_seconds: u32,
}
impl fmt::Debug for Preferences {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Preferences")
            .field("revision", &self.revision)
            .field("fps", &self.fps)
            .field("sound", &self.sound)
            .field("interaction_sound", &self.interaction_sound)
            .field("ambient_sound", &self.ambient_sound)
            .field("sound_pack", &self.sound_pack)
            .field("sound_volume", &self.sound_volume)
            .field("autostart", &self.autostart)
            .field("pet_id", &self.pet_id)
            .field("muted_sources", &self.muted_sources)
            .field("notify_subagents", &self.notify_subagents)
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
            .field("activity_range", &self.activity_range)
            .field("activity_speed", &self.activity_speed)
            .field("bubble_auto_close_seconds", &self.bubble_auto_close_seconds)
            .finish()
    }
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            revision: 0,
            fps: 30,
            sound: false,
            interaction_sound: true,
            ambient_sound: true,
            sound_pack: SoundPack::default(),
            sound_volume: DEFAULT_SOUND_VOLUME,
            autostart: false,
            pet_id: DEFAULT_PET.into(),
            muted_sources: Vec::new(),
            notify_subagents: true,
            deepseek_enabled: true,
            deepseek_api_key: String::new(),
            deepseek_model: DEEPSEEK_DEFAULT_MODEL.into(),
            activity_range: ActivityRange::default(),
            activity_speed: DEFAULT_ACTIVITY_SPEED,
            bubble_auto_close_seconds: DEFAULT_BUBBLE_AUTO_CLOSE_SECONDS,
        }
    }
}
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsPatch {
    pub fps: Option<u8>,
    pub sound: Option<bool>,
    pub interaction_sound: Option<bool>,
    pub ambient_sound: Option<bool>,
    pub sound_pack: Option<SoundPack>,
    pub sound_volume: Option<f64>,
    pub autostart: Option<bool>,
    pub pet_id: Option<String>,
    pub muted_sources: Option<Vec<Source>>,
    pub notify_subagents: Option<bool>,
    pub placement: Option<PlacementPatch>,
    pub deepseek_enabled: Option<bool>,
    pub deepseek_api_key: Option<String>,
    pub deepseek_model: Option<String>,
    pub activity_range: Option<ActivityRange>,
    pub activity_speed: Option<f64>,
    pub bubble_auto_close_seconds: Option<u32>,
}
impl fmt::Debug for SettingsPatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SettingsPatch")
            .field("fps", &self.fps)
            .field("sound", &self.sound)
            .field("interaction_sound", &self.interaction_sound)
            .field("ambient_sound", &self.ambient_sound)
            .field("sound_pack", &self.sound_pack)
            .field("sound_volume", &self.sound_volume)
            .field("autostart", &self.autostart)
            .field("pet_id", &self.pet_id)
            .field("muted_sources", &self.muted_sources)
            .field("notify_subagents", &self.notify_subagents)
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
            .field("activity_range", &self.activity_range)
            .field("activity_speed", &self.activity_speed)
            .field("bubble_auto_close_seconds", &self.bubble_auto_close_seconds)
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
    interaction_sound: bool,
    ambient_sound: bool,
    sound_pack: SoundPack,
    sound_volume: f64,
    autostart: bool,
    pet_id: &'a str,
    muted_sources: &'a [Source],
    notify_subagents: bool,
    deepseek_enabled: bool,
    deepseek_model: &'a str,
    deepseek_api_key_configured: bool,
    deepseek_api_key_hint: Option<String>,
    activity_range: &'a ActivityRange,
    activity_speed: f64,
    bubble_auto_close_seconds: u32,
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
            interaction_sound: prefs.interaction_sound,
            ambient_sound: prefs.ambient_sound,
            sound_pack: prefs.sound_pack,
            sound_volume: prefs.sound_volume,
            autostart: prefs.autostart,
            pet_id: &prefs.pet_id,
            muted_sources: &prefs.muted_sources,
            notify_subagents: prefs.notify_subagents,
            deepseek_enabled: prefs.deepseek_enabled,
            deepseek_model: &prefs.deepseek_model,
            deepseek_api_key_configured: hint.is_some(),
            deepseek_api_key_hint: hint,
            activity_range: &prefs.activity_range,
            activity_speed: prefs.activity_speed,
            bubble_auto_close_seconds: prefs.bubble_auto_close_seconds,
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
        self.activity_range.validate()?;
        if !self.activity_speed.is_finite()
            || !(MIN_ACTIVITY_SPEED..=MAX_ACTIVITY_SPEED).contains(&self.activity_speed)
        {
            return Err(StorageError::InvalidData(
                "跑动速率必须在 0.5x 到 2.0x 之间",
            ));
        }
        if !self.sound_volume.is_finite()
            || !(MIN_SOUND_VOLUME..=MAX_SOUND_VOLUME).contains(&self.sound_volume)
        {
            return Err(StorageError::InvalidData("音效音量必须在 0% 到 100% 之间"));
        }
        if self.bubble_auto_close_seconds > MAX_BUBBLE_AUTO_CLOSE_SECONDS {
            return Err(StorageError::InvalidData(
                "任务气泡自动关闭时长必须在 0 到 600 秒之间",
            ));
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
        if let Some(interaction_sound) = patch.interaction_sound {
            next.interaction_sound = interaction_sound;
        }
        if let Some(ambient_sound) = patch.ambient_sound {
            next.ambient_sound = ambient_sound;
        }
        if let Some(sound_pack) = patch.sound_pack {
            next.sound_pack = sound_pack;
        }
        if let Some(sound_volume) = patch.sound_volume {
            next.sound_volume = sound_volume;
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
        if let Some(notify_subagents) = patch.notify_subagents {
            next.notify_subagents = notify_subagents;
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
        if let Some(range) = &patch.activity_range {
            next.activity_range = range.normalized();
        }
        if let Some(speed) = patch.activity_speed {
            next.activity_speed = speed;
        }
        if let Some(seconds) = patch.bubble_auto_close_seconds {
            next.bubble_auto_close_seconds = seconds;
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
