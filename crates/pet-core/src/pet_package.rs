//! 导入旧版宠物素材，校验应用自己的显式动画包。
use crate::{
    StorageError,
    paths::{is_link, open_regular, prepare_directory, publish_directory, sync_directory},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Cursor, Read, Write},
    path::{Component, Path, PathBuf},
};

const MANIFEST_FILE: &str = "pet.json";
const FORMAT_VERSION: u16 = 1;
const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;
const MAX_ATLAS_SIDE: u32 = 4096;
const LOOK_DIRECTIONS: usize = 16;
const LEGACY_WIDTH: u32 = 1536;
const LEGACY_CELL_WIDTH: u32 = 192;
const LEGACY_CELL_HEIGHT: u32 = 208;
const LEGACY_COLUMNS: usize = 8;

const LEGACY_ANIMATIONS: [(&str, &[u32], Option<usize>); 9] = [
    ("idle", &[280, 110, 110, 140, 140, 320], Some(0)),
    (
        "running-right",
        &[120, 120, 120, 120, 120, 120, 120, 220],
        Some(0),
    ),
    (
        "running-left",
        &[120, 120, 120, 120, 120, 120, 120, 220],
        Some(0),
    ),
    ("waving", &[140, 140, 140, 280], None),
    ("jumping", &[140, 140, 140, 140, 280], None),
    ("failed", &[140, 140, 140, 140, 140, 140, 140, 240], None),
    ("waiting", &[150, 150, 150, 150, 150, 260], Some(0)),
    ("running", &[120, 120, 120, 120, 120, 220], Some(0)),
    ("review", &[150, 150, 150, 150, 150, 280], Some(0)),
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PetManifest {
    pub version: u16,
    pub id: String,
    pub name: String,
    pub atlas: Atlas,
    pub animations: BTreeMap<String, Animation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub look_directions: Option<Vec<FrameRect>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Atlas {
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub cell_width: u32,
    pub cell_height: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Animation {
    pub frames: Vec<AnimationFrame>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loop_start: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnimationFrame {
    pub rect: FrameRect,
    pub duration_ms: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug)]
pub enum PetError {
    DestinationExists(PathBuf),
    FileTooLarge(PathBuf),
    Image(String),
    ImageTooLarge { width: u32, height: u32 },
    InvalidManifest(&'static str),
    Io(io::Error),
    Json(serde_json::Error),
    PublishedWithoutSync { path: PathBuf, error: StorageError },
    Storage(StorageError),
    UnsafePath(PathBuf),
}

impl fmt::Display for PetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DestinationExists(path) => write!(f, "宠物包目标已存在：{}", path.display()),
            Self::FileTooLarge(path) => write!(f, "文件超过 20 MiB：{}", path.display()),
            Self::Image(message) => write!(f, "宠物图集无效：{message}"),
            Self::ImageTooLarge { width, height } => {
                write!(f, "宠物图集尺寸 {width}×{height} 超过 4096×4096")
            }
            Self::InvalidManifest(message) => write!(f, "宠物清单无效：{message}"),
            Self::Io(error) => write!(f, "宠物包文件操作失败：{error}"),
            Self::Json(error) => write!(f, "宠物清单 JSON 无效：{error}"),
            Self::PublishedWithoutSync { path, error } => {
                write!(
                    f,
                    "宠物包已发布但父目录同步失败（{}）：{error}",
                    path.display()
                )
            }
            Self::Storage(error) => error.fmt(f),
            Self::UnsafePath(path) => write!(f, "不安全的宠物包路径：{}", path.display()),
        }
    }
}

impl std::error::Error for PetError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::PublishedWithoutSync { error, .. } | Self::Storage(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for PetError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for PetError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<StorageError> for PetError {
    fn from(error: StorageError) -> Self {
        match error {
            StorageError::UnsafePath(path) => Self::UnsafePath(path),
            StorageError::Io(error) => Self::Io(error),
            other => Self::Storage(other),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyManifest {
    id: String,
    display_name: String,
    spritesheet_path: String,
    #[serde(default = "legacy_v1")]
    sprite_version_number: u16,
}

fn legacy_v1() -> u16 {
    1
}

/// 将旧格式转换并发布到确切的包目录，通常是 `AppPaths::pets` 下的新子目录。
pub fn import_pet(source: &Path, destination: &Path) -> Result<PetManifest, PetError> {
    let source = PackageDirectory::open(source)?;
    let destination = std::path::absolute(destination)?;
    let parent = destination
        .parent()
        .ok_or_else(|| PetError::UnsafePath(destination.clone()))?;
    let name = destination
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .filter(|name| portable_component(name))
        .ok_or_else(|| PetError::UnsafePath(destination.clone()))?;
    if destination
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err(PetError::UnsafePath(destination));
    }
    check_destination(&source.path, &destination)?;

    let legacy: LegacyManifest = serde_json::from_slice(&source.read(MANIFEST_FILE)?)?;
    let (height, has_look) = match legacy.sprite_version_number {
        1 => (1872, false),
        2 => (2288, true),
        _ => return Err(PetError::InvalidManifest("不支持的旧宠物布局版本")),
    };
    let image = source.read(&legacy.spritesheet_path)?;
    let format = AtlasFormat::from_path(&legacy.spritesheet_path)?;
    let manifest = PetManifest {
        version: FORMAT_VERSION,
        id: legacy.id,
        name: legacy.display_name,
        atlas: Atlas {
            path: format.atlas_name().into(),
            width: LEGACY_WIDTH,
            height,
            cell_width: LEGACY_CELL_WIDTH,
            cell_height: LEGACY_CELL_HEIGHT,
        },
        animations: legacy_animations(),
        look_directions: has_look.then(|| {
            (0..LOOK_DIRECTIONS)
                .map(|index| {
                    legacy_rect(
                        LEGACY_ANIMATIONS.len() + index / LEGACY_COLUMNS,
                        index % LEGACY_COLUMNS,
                    )
                })
                .collect()
        }),
    };
    manifest.validate()?;
    validate_image(&image, format, &manifest.atlas)?;
    let manifest_json = serde_json::to_vec_pretty(&manifest)?;
    if manifest_json.len() as u64 > MAX_FILE_BYTES {
        return Err(PetError::FileTooLarge(destination.join(MANIFEST_FILE)));
    }

    prepare_directory(parent)?;
    let parent = parent.canonicalize()?;
    let destination = parent.join(name);
    check_destination(&source.path, &destination)?;

    let mut staging = tempfile::Builder::new();
    staging.prefix(".pet-stage-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        staging.permissions(fs::Permissions::from_mode(0o700));
    }
    let staging = staging.tempdir_in(&parent)?;
    write_new_file(&staging.path().join(&manifest.atlas.path), &image)?;
    write_new_file(&staging.path().join(MANIFEST_FILE), &manifest_json)?;
    sync_directory(staging.path())?;
    if let Err(error) = publish_directory(staging.path(), &destination) {
        if error.kind() == io::ErrorKind::AlreadyExists
            || fs::symlink_metadata(&destination).is_ok()
        {
            return Err(PetError::DestinationExists(destination));
        }
        return Err(error.into());
    }
    let _ = staging.keep();
    sync_directory(&parent).map_err(|error| PetError::PublishedWithoutSync {
        path: destination,
        error,
    })?;
    Ok(manifest)
}

/// 读取并校验应用自己的动画包，含图集实际尺寸和像素数据。
pub fn load_pet(package: &Path) -> Result<PetManifest, PetError> {
    let package = PackageDirectory::open(package)?;
    let manifest: PetManifest = serde_json::from_slice(&package.read(MANIFEST_FILE)?)?;
    manifest.validate()?;
    let image = package.read(&manifest.atlas.path)?;
    let format = AtlasFormat::from_path(&manifest.atlas.path)?;
    validate_image(&image, format, &manifest.atlas)?;
    Ok(manifest)
}

impl PetManifest {
    fn validate(&self) -> Result<(), PetError> {
        if self.version != FORMAT_VERSION {
            return Err(PetError::InvalidManifest("不支持的动画包版本"));
        }
        if !portable_component(&self.id) || self.name.trim().is_empty() {
            return Err(PetError::InvalidManifest("宠物标识或名称无效"));
        }
        let atlas = &self.atlas;
        validate_dimensions(atlas.width, atlas.height)?;
        if atlas.cell_width == 0
            || atlas.cell_width > atlas.width
            || atlas.cell_height == 0
            || atlas.cell_height > atlas.height
        {
            return Err(PetError::InvalidManifest("图集格子尺寸无效"));
        }
        if self.animations.is_empty() {
            return Err(PetError::InvalidManifest("动画列表为空"));
        }
        for (name, animation) in &self.animations {
            if name.trim().is_empty() || animation.frames.is_empty() {
                return Err(PetError::InvalidManifest("动作名称或帧列表为空"));
            }
            if animation
                .loop_start
                .is_some_and(|start| start >= animation.frames.len())
            {
                return Err(PetError::InvalidManifest("动作循环起点越界"));
            }
            for frame in &animation.frames {
                if frame.duration_ms == 0 {
                    return Err(PetError::InvalidManifest("动作帧时长必须大于零"));
                }
                frame.rect.validate(atlas)?;
            }
        }
        if let Some(directions) = &self.look_directions {
            if directions.len() != LOOK_DIRECTIONS {
                return Err(PetError::InvalidManifest("视线姿态必须有 16 个方向"));
            }
            for rect in directions {
                rect.validate(atlas)?;
            }
        }
        Ok(())
    }
}

impl FrameRect {
    fn validate(self, atlas: &Atlas) -> Result<(), PetError> {
        if self.width == 0
            || self.height == 0
            || self
                .x
                .checked_add(self.width)
                .is_none_or(|right| right > atlas.width)
            || self
                .y
                .checked_add(self.height)
                .is_none_or(|bottom| bottom > atlas.height)
        {
            return Err(PetError::InvalidManifest("帧矩形越出图集边界或尺寸为空"));
        }
        Ok(())
    }
}

fn legacy_animations() -> BTreeMap<String, Animation> {
    LEGACY_ANIMATIONS
        .iter()
        .enumerate()
        .map(|(row, (name, durations, loop_start))| {
            let frames = durations
                .iter()
                .enumerate()
                .map(|(column, &duration_ms)| AnimationFrame {
                    rect: legacy_rect(row, column),
                    duration_ms,
                })
                .collect();
            (
                (*name).into(),
                Animation {
                    frames,
                    loop_start: *loop_start,
                },
            )
        })
        .collect()
}

fn legacy_rect(row: usize, column: usize) -> FrameRect {
    FrameRect {
        x: column as u32 * LEGACY_CELL_WIDTH,
        y: row as u32 * LEGACY_CELL_HEIGHT,
        width: LEGACY_CELL_WIDTH,
        height: LEGACY_CELL_HEIGHT,
    }
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), PetError> {
    let mut file = open_regular(path, OpenOptions::new().write(true).create_new(true))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn check_destination(source: &Path, destination: &Path) -> Result<(), PetError> {
    match fs::symlink_metadata(destination) {
        Ok(_) => return Err(PetError::DestinationExists(destination.into())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    // 用已有祖先的实际路径识别同一个目录的别名，例如大小写不敏感的卷。
    for ancestor in destination.ancestors().skip(1) {
        match fs::symlink_metadata(ancestor) {
            Ok(_) => {
                require_directory(ancestor)?;
                if ancestor.canonicalize()?.starts_with(source) {
                    return Err(PetError::UnsafePath(destination.into()));
                }
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn portable_component(name: &str) -> bool {
    if name.is_empty()
        || name.ends_with(['.', ' '])
        || name
            .chars()
            .any(|ch| ch.is_control() || "<>:\"/\\|?*".contains(ch))
    {
        return false;
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ');
    let upper = stem.to_ascii_uppercase();
    if matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return false;
    }
    !upper
        .strip_prefix("COM")
        .or_else(|| upper.strip_prefix("LPT"))
        .is_some_and(|number| {
            matches!(
                number,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
}

fn require_directory(path: &Path) -> Result<(), PetError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || is_link(&metadata) {
        return Err(PetError::UnsafePath(path.into()));
    }
    Ok(())
}

struct PackageDirectory {
    path: PathBuf,
    #[cfg(unix)]
    directory: File,
    // 读取期间保留 Windows 目录句柄，阻止被移走或替换成重解析点。
    #[cfg(windows)]
    _directory: File,
}

impl PackageDirectory {
    fn open(path: &Path) -> Result<Self, PetError> {
        require_directory(path)?;
        let path = path.canonicalize()?;
        #[cfg(unix)]
        {
            use rustix::fs::{CWD, openat};
            let directory = openat(CWD, &path, directory_flags(), rustix::fs::Mode::empty())
                .map_err(|error| package_open_error(&path, error.into()))?;
            Ok(Self {
                path,
                directory: directory.into(),
            })
        }
        #[cfg(windows)]
        {
            let directory = lock_directory(&path)?;
            Ok(Self {
                path,
                _directory: directory,
            })
        }
        #[cfg(not(any(unix, windows)))]
        Ok(Self { path })
    }

    fn read(&self, relative: &str) -> Result<Vec<u8>, PetError> {
        if !relative.split('/').all(portable_component) {
            return Err(PetError::UnsafePath(relative.into()));
        }
        let file = self.open_file(relative)?;
        let path = self.path.join(relative);
        if file.metadata()?.len() > MAX_FILE_BYTES {
            return Err(PetError::FileTooLarge(path));
        }
        let mut bytes = Vec::new();
        file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(PetError::FileTooLarge(path));
        }
        Ok(bytes)
    }

    #[cfg(unix)]
    fn open_file(&self, relative: &str) -> Result<File, PetError> {
        use crate::paths::require_regular;
        use rustix::fs::{AtFlags, FileType, Mode, OFlags, openat, statat};

        let mut directory = self.directory.try_clone()?;
        let mut path = self.path.clone();
        let mut parts = relative.split('/').peekable();
        while let Some(part) = parts.next() {
            path.push(part);
            if parts.peek().is_none() {
                let metadata =
                    statat(&directory, part, AtFlags::SYMLINK_NOFOLLOW).map_err(io::Error::from)?;
                if !FileType::from_raw_mode(metadata.st_mode).is_file() {
                    return Err(PetError::UnsafePath(path));
                }
                let file = openat(
                    &directory,
                    part,
                    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .map_err(|error| package_open_error(&path, error.into()))?;
                let file = File::from(file);
                require_regular(&path, &file.metadata()?)?;
                return Ok(file);
            }
            directory = openat(&directory, part, directory_flags(), Mode::empty())
                .map_err(|error| package_open_error(&path, error.into()))?
                .into();
        }
        Err(PetError::UnsafePath(path))
    }

    #[cfg(not(unix))]
    fn open_file(&self, relative: &str) -> Result<File, PetError> {
        let mut path = self.path.clone();
        #[cfg(windows)]
        let mut directories = Vec::new();
        let mut parts = relative.split('/').peekable();
        while let Some(part) = parts.next() {
            path.push(part);
            if parts.peek().is_some() {
                #[cfg(windows)]
                directories.push(lock_directory(&path)?);
                #[cfg(not(windows))]
                require_directory(&path)?;
            }
        }
        Ok(open_regular(&path, OpenOptions::new().read(true))?)
    }
}

#[cfg(unix)]
fn directory_flags() -> rustix::fs::OFlags {
    use rustix::fs::OFlags;
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
}

#[cfg(unix)]
fn package_open_error(path: &Path, error: io::Error) -> PetError {
    if error.kind() == io::ErrorKind::NotADirectory || error.raw_os_error() == Some(libc::ELOOP) {
        PetError::UnsafePath(path.into())
    } else {
        error.into()
    }
}

#[cfg(windows)]
fn lock_directory(path: &Path) -> Result<File, PetError> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
        FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    let directory = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let metadata = directory.metadata()?;
    if !metadata.is_dir() || is_link(&metadata) {
        return Err(PetError::UnsafePath(path.into()));
    }
    Ok(directory)
}

#[derive(Clone, Copy)]
enum AtlasFormat {
    Png,
    Webp,
}

impl AtlasFormat {
    fn from_path(path: &str) -> Result<Self, PetError> {
        let extension = Path::new(path)
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or_default();
        if extension.eq_ignore_ascii_case("png") {
            Ok(Self::Png)
        } else if extension.eq_ignore_ascii_case("webp") {
            Ok(Self::Webp)
        } else {
            Err(PetError::Image("只支持 PNG 与 WebP 图集".into()))
        }
    }

    fn atlas_name(self) -> &'static str {
        match self {
            Self::Png => "atlas.png",
            Self::Webp => "atlas.webp",
        }
    }
}

fn validate_dimensions(width: u32, height: u32) -> Result<(), PetError> {
    if width > MAX_ATLAS_SIDE || height > MAX_ATLAS_SIDE {
        return Err(PetError::ImageTooLarge { width, height });
    }
    if width == 0 || height == 0 {
        return Err(PetError::InvalidManifest("图集宽高必须大于零"));
    }
    Ok(())
}

fn validate_atlas_size(atlas: &Atlas, width: u32, height: u32) -> Result<(), PetError> {
    validate_dimensions(width, height)?;
    if (width, height) != (atlas.width, atlas.height) {
        return Err(PetError::InvalidManifest("图集尺寸与清单或旧布局不符"));
    }
    Ok(())
}

fn image_error(error: impl fmt::Display) -> PetError {
    PetError::Image(error.to_string())
}

fn validate_image(bytes: &[u8], format: AtlasFormat, atlas: &Atlas) -> Result<(), PetError> {
    match format {
        AtlasFormat::Png => {
            let mut decoder = png::Decoder::new_with_limits(
                Cursor::new(bytes),
                png::Limits {
                    bytes: 16 * 1024 * 1024,
                },
            );
            decoder.set_ignore_text_chunk(true);
            decoder.set_ignore_iccp_chunk(true);
            let header = decoder.read_header_info().map_err(image_error)?;
            validate_atlas_size(atlas, header.width, header.height)?;
            let mut reader = decoder.read_info().map_err(image_error)?;
            if reader.info().animation_control.is_some() {
                return Err(image_error("PNG 图集必须是静态图像"));
            }
            let size = reader
                .output_buffer_size()
                .ok_or_else(|| image_error("PNG 像素缓冲区过大"))?;
            reader.next_frame(&mut vec![0; size]).map_err(image_error)?;
            reader.finish().map_err(image_error)?;
        }
        AtlasFormat::Webp => {
            validate_webp_container(bytes)?;
            let mut decoder =
                image_webp::WebPDecoder::new(Cursor::new(bytes)).map_err(image_error)?;
            let (width, height) = decoder.dimensions();
            validate_atlas_size(atlas, width, height)?;
            let size = decoder
                .output_buffer_size()
                .ok_or_else(|| image_error("WebP 像素缓冲区过大"))?;
            decoder
                .read_image(&mut vec![0; size])
                .map_err(image_error)?;
        }
    }
    Ok(())
}

fn validate_webp_container(bytes: &[u8]) -> Result<(), PetError> {
    let invalid = || image_error("WebP 容器头或分块长度无效");
    let [
        b'R',
        b'I',
        b'F',
        b'F',
        a,
        b,
        c,
        d,
        b'W',
        b'E',
        b'B',
        b'P',
        ..,
    ] = bytes
    else {
        return Err(invalid());
    };
    if u32::from_le_bytes([*a, *b, *c, *d]) as usize != bytes.len() - 8 {
        return Err(invalid());
    }
    let mut chunks = &bytes[12..];
    while !chunks.is_empty() {
        let [a, b, c, d] = chunks.get(4..8).ok_or_else(invalid)? else {
            return Err(invalid());
        };
        let size = u32::from_le_bytes([*a, *b, *c, *d]) as usize;
        let tag = &chunks[..4];
        let data = chunks[8..].get(..size).ok_or_else(invalid)?;
        chunks = chunks[8..].get(size + size % 2..).ok_or_else(invalid)?;
        match tag {
            b"ANIM" | b"ANMF" => return Err(image_error("WebP 图集必须是静态图像")),
            b"VP8 " => {
                // 扩展容器的 VP8 载荷自己声明宽高，解码器按该宽高分配像素。
                let [a, b, c, d] = data.get(6..10).ok_or_else(invalid)? else {
                    return Err(invalid());
                };
                let width = u32::from(u16::from_le_bytes([*a, *b]) & 0x3fff);
                let height = u32::from(u16::from_le_bytes([*c, *d]) & 0x3fff);
                validate_dimensions(width, height)?;
            }
            _ => {}
        }
    }
    Ok(())
}
