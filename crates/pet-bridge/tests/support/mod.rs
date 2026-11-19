use pet_core::AppPaths;
use std::{path::Path, process::Command};

#[allow(dead_code)] // Windows 安装器测试经 shell 使用同一隔离配置。
pub fn isolated_bridge(root: &Path) -> (Command, AppPaths) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pet-bridge"));
    let paths = isolate_user_paths(&mut command, root);
    (command, paths)
}

pub fn isolate_user_paths(command: &mut Command, root: &Path) -> AppPaths {
    let root = root.canonicalize().unwrap();
    #[cfg(target_os = "macos")]
    let base = {
        command.env("HOME", &root);
        root.join("Library/Application Support")
    };
    #[cfg(windows)]
    let base = {
        command.env("LOCALAPPDATA", &root);
        root
    };
    #[cfg(not(any(target_os = "macos", windows)))]
    let base = {
        command.env("XDG_DATA_HOME", &root);
        root
    };
    AppPaths::under(&base.join("Codex Pet"))
}
