//! 桌宠文件位置与跨平台文件操作。
use crate::StorageError;
use std::{
    env,
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Write},
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppPaths {
    pub root: PathBuf,
    pub outbox: PathBuf,
    pub pets: PathBuf,
    pub journal: PathBuf,
    /// 共享路径定义。当前去重索引随日志检查点一起写入。
    pub dedup: PathBuf,
    pub settings: PathBuf,
    pub discovery: PathBuf,
}

impl AppPaths {
    pub fn for_user() -> Result<Self, StorageError> {
        #[cfg(target_os = "macos")]
        let base =
            env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"));
        #[cfg(windows)]
        let base = env::var_os("LOCALAPPDATA").map(PathBuf::from);
        #[cfg(not(any(target_os = "macos", windows)))]
        let base = env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));

        let base = base
            .filter(|path| path.is_absolute())
            .ok_or(StorageError::InvalidData("无法确定当前用户的应用数据目录"))?;
        Ok(Self::under(&base.join("Codex Pet")))
    }

    pub fn under(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            outbox: root.join("outbox"),
            pets: root.join("pets"),
            journal: root.join("journal.jsonl"),
            dedup: root.join("dedup.json"),
            settings: root.join("settings.json"),
            discovery: root.join("discovery.json"),
        }
    }

    pub fn prepare_root(&self) -> Result<(), StorageError> {
        if *self != Self::under(&self.root) {
            return Err(StorageError::UnsafePath(self.root.clone()));
        }
        prepare_directory(&self.root)
    }

    pub fn prepare_outbox(&self) -> Result<(), StorageError> {
        self.prepare_root()?;
        prepare_directory(&self.outbox)
    }
}

/// 逐段拒绝链接与重解析点，创建仅当前用户可访问的目录。
pub fn prepare_directory(path: &Path) -> Result<(), StorageError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return Err(StorageError::UnsafePath(path.into()));
    }
    let mut prefix = PathBuf::new();
    for component in path.components() {
        prefix.push(component.as_os_str());
        match component {
            Component::Prefix(_) | Component::RootDir => continue,
            Component::Normal(_) => {}
            _ => return Err(StorageError::UnsafePath(path.into())),
        }
        match fs::symlink_metadata(&prefix) {
            Ok(metadata) if metadata.is_dir() && !is_link(&metadata) => {}
            Ok(_) => return Err(StorageError::UnsafePath(prefix)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                #[cfg(unix)]
                let created = {
                    use std::os::unix::fs::DirBuilderExt;
                    fs::DirBuilder::new().mode(0o700).create(&prefix)
                };
                #[cfg(not(unix))]
                let created = fs::create_dir(&prefix);
                match created {
                    Ok(()) => sync_directory(prefix.parent().expect("非根目录有父目录"))?,
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error.into()),
                }
                let metadata = fs::symlink_metadata(&prefix)?;
                if !metadata.is_dir() || is_link(&metadata) {
                    return Err(StorageError::UnsafePath(prefix));
                }
            }
            Err(error) => return Err(error.into()),
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if fs::symlink_metadata(path)?.mode() & 0o077 != 0 {
            return Err(StorageError::UnsafePath(path.into()));
        }
    }
    Ok(())
}

pub fn is_link(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

/// 检查磁盘对象类型，不跟随最后一段链接。
pub fn regular_metadata(path: &Path) -> Result<Metadata, StorageError> {
    let metadata = fs::symlink_metadata(path)?;
    require_regular(path, &metadata)?;
    Ok(metadata)
}

pub fn require_regular(path: &Path, metadata: &Metadata) -> Result<(), StorageError> {
    if !metadata.is_file() || is_link(metadata) {
        return Err(StorageError::UnsafePath(path.into()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(StorageError::UnsafePath(path.into()));
        }
    }
    Ok(())
}

pub fn open_regular(path: &Path, options: &mut OpenOptions) -> Result<File, StorageError> {
    match regular_metadata(path) {
        Ok(_) => {}
        Err(StorageError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    require_regular(path, &file.metadata()?)?;
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
        };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // 已打开的句柄在查询期间保持有效。
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err(io::Error::last_os_error().into());
        }
        if info.nNumberOfLinks != 1 {
            return Err(StorageError::UnsafePath(path.into()));
        }
    }
    Ok(file)
}

/// 同目录暂存并同步内容，然后发布文件。调用者在提交时同步父目录。
pub fn atomic_write(path: &Path, prefix: &str, bytes: &[u8]) -> Result<(), StorageError> {
    let parent = path
        .parent()
        .ok_or_else(|| StorageError::UnsafePath(path.into()))?;
    let mut staging = tempfile::Builder::new()
        .prefix(prefix)
        .make_in(parent, |temporary| {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            options.open(temporary)
        })?;
    staging.write_all(bytes)?;
    staging.as_file().sync_all()?;
    durable_rename(staging.path(), path)?;
    Ok(())
}

pub fn durable_rename(from: &Path, to: &Path) -> Result<(), StorageError> {
    match regular_metadata(to) {
        Ok(_) => {}
        Err(StorageError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    #[cfg(not(windows))]
    fs::rename(from, to)?;
    #[cfg(windows)]
    move_file_through(from, to, true)?;
    Ok(())
}

/// 在同卷原子发布完整目录，已有同名文件或目录时失败。
pub(crate) fn publish_directory(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        use rustix::fs::{CWD, RenameFlags, renameat_with};
        renameat_with(CWD, from, CWD, to, RenameFlags::NOREPLACE)?;
        Ok(())
    }
    #[cfg(windows)]
    return move_file_through(from, to, false);
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        let _ = (from, to);
        Err(io::ErrorKind::Unsupported.into())
    }
}

#[cfg(windows)]
fn move_file_through(from: &Path, to: &Path, replace: bool) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let from: Vec<u16> = from.as_os_str().encode_wide().chain([0]).collect();
    let to: Vec<u16> = to.as_os_str().encode_wide().chain([0]).collect();
    if from[..from.len() - 1].contains(&0) || to[..to.len() - 1].contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "文件路径包含空字符",
        ));
    }
    let mut flags = MOVEFILE_WRITE_THROUGH;
    if replace {
        flags |= MOVEFILE_REPLACE_EXISTING;
    }
    // 两个缓冲区在调用期间有效并以 NUL 结尾，移动限定在同一个卷内。
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), flags) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub fn sync_directory(path: &Path) -> Result<(), StorageError> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    // Windows 原子发布使用带写穿透的 MoveFileExW。
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

pub fn wall_clock_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
