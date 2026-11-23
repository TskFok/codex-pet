use super::{Desktop, valid_path};
use pet_core::Source;
use std::{io, path::Path};

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString, NSWorkspace};
    use objc2_foundation::{NSBundle, NSString, NSURL};
    use std::{
        path::PathBuf,
        process::{Command, Stdio},
    };

    pub(super) fn desktop(source: Source) -> Option<Desktop> {
        let (link, id) = match source {
            Source::Codex => ("codex://threads", "com.openai.codex"),
            Source::Deepseek => ("dsh://open", "com.deepseek.harness"),
        };
        let url = NSURL::URLWithString(&NSString::from_str(link))?;
        let workspace = NSWorkspace::sharedWorkspace();
        let matches_bundle = |url: &NSURL| {
            NSBundle::bundleWithURL(url).is_some_and(|bundle| {
                bundle
                    .bundleIdentifier()
                    .is_some_and(|value| value.to_string() == id)
                    && bundle
                        .executableURL()
                        .and_then(|url| url.path())
                        .is_some_and(|path| Path::new(&path.to_string()).is_file())
            })
        };
        let handler = workspace
            .URLForApplicationToOpenURL(&url)
            .filter(|url| matches_bundle(url));
        let registered = handler.is_some();
        let app = handler.or_else(|| {
            (source == Source::Codex)
                .then(|| workspace.URLForApplicationWithBundleIdentifier(&NSString::from_str(id)))
                .flatten()
                .filter(|url| matches_bundle(url))
        })?;
        let path = PathBuf::from(app.path()?.to_string()).canonicalize().ok()?;
        if !valid_path(&path) {
            return None;
        }
        let supports_session = match source {
            Source::Codex => {
                let signed = Command::new("/usr/bin/codesign")
                    .args(["--verify", "-R=identifier \"com.openai.codex\" and anchor apple generic and certificate leaf[subject.OU] = \"2DC432GLL2\""])
                    .arg(&path)
                    .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
                    .status().is_ok_and(|s| s.success());
                if !signed {
                    return None;
                }
                registered
            }
            Source::Deepseek => {
                super::super::harness::archive_support(&path.join("Contents/Resources/app.asar"))?
            }
        };
        Some(Desktop {
            path,
            supports_session,
        })
    }

    pub(super) fn dispatch(_source: Source, path: &Path, url: Option<&str>) -> io::Result<()> {
        let status = Command::new("/usr/bin/open")
            .arg("-a")
            .arg(path)
            .args(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        if !status.success() {
            return Err(io::Error::other("系统未接受打开请求"));
        }
        Ok(())
    }

    pub(super) fn copy(_window: &tauri::WebviewWindow, id: &str) -> io::Result<()> {
        let pasteboard = NSPasteboard::generalPasteboard();
        pasteboard.clearContents();
        // SAFETY: AppKit 导出的 NSString 类型常量。
        if !pasteboard.setString_forType(&NSString::from_str(id), unsafe { NSPasteboardTypeString })
        {
            return Err(io::Error::other("无法写入系统剪贴板"));
        }
        Ok(())
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::{
        ffi::c_void,
        os::windows::ffi::{OsStrExt, OsStringExt},
        path::PathBuf,
        process::Command,
    };

    const CODEX_PACKAGE: &str = "OpenAI.Codex_2p2nqsd0c76g0!";

    fn wide(value: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
        value.as_ref().encode_wide().chain([0]).collect()
    }

    #[link(name = "shlwapi")]
    unsafe extern "system" {
        fn AssocQueryStringW(
            flags: u32,
            field: u32,
            association: *const u16,
            extra: *const u16,
            out: *mut u16,
            len: *mut u32,
        ) -> i32;
    }

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteW(
            hwnd: *mut c_void,
            operation: *const u16,
            file: *const u16,
            parameters: *const u16,
            directory: *const u16,
            show: i32,
        ) -> isize;
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn OpenClipboard(owner: *mut c_void) -> i32;
        fn EmptyClipboard() -> i32;
        fn SetClipboardData(format: u32, handle: *mut c_void) -> *mut c_void;
        fn CloseClipboard() -> i32;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalAlloc(flags: u32, size: usize) -> *mut c_void;
        fn GlobalLock(handle: *mut c_void) -> *mut c_void;
        fn GlobalUnlock(handle: *mut c_void) -> i32;
        fn GlobalFree(handle: *mut c_void) -> *mut c_void;
    }

    fn association(scheme: &str, field: u32) -> Option<std::ffi::OsString> {
        const ASSOCF_IS_PROTOCOL: u32 = 0x1000;
        let scheme = wide(scheme);
        let mut output = vec![0u16; 32768];
        let mut count = output.len() as u32;
        // SAFETY: 两个有效 NUL 结尾入参，输出包含传入长度指定的可写空间。
        (unsafe {
            AssocQueryStringW(
                ASSOCF_IS_PROTOCOL,
                field,
                scheme.as_ptr(),
                std::ptr::null(),
                output.as_mut_ptr(),
                &mut count,
            )
        } == 0
            && count > 1
            && count as usize <= output.len())
        .then(|| std::ffi::OsString::from_wide(&output[..count as usize - 1]))
    }

    fn codex_appid() -> Option<String> {
        const ASSOCSTR_APPID: u32 = 21;
        let app = association("codex", ASSOCSTR_APPID)?.into_string().ok()?;
        let app_id = app.strip_prefix(CODEX_PACKAGE)?;
        (!app_id.is_empty()
            && app_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.'))
        .then_some(app)
    }

    pub(super) fn desktop(source: Source) -> Option<Desktop> {
        const ASSOCSTR_EXECUTABLE: u32 = 2;
        let scheme = match source {
            Source::Codex => "codex",
            Source::Deepseek => "dsh",
        };
        let path = PathBuf::from(association(scheme, ASSOCSTR_EXECUTABLE)?);
        if !valid_path(&path) || !path.is_file() {
            return None;
        }
        let supports_session = match source {
            Source::Codex => {
                codex_appid()?;
                true
            }
            Source::Deepseek => {
                // 校验 Electron 桌面协议登记和同目录打包资源；不执行登记字符串。
                const ASSOCSTR_COMMAND: u32 = 1;
                let command = association(scheme, ASSOCSTR_COMMAND)?.into_string().ok()?;
                let expected = format!("\"{}\" \"%1\"", path.to_str()?);
                if !command.eq_ignore_ascii_case(&expected) {
                    return None;
                }
                super::super::harness::archive_support(&path.parent()?.join("resources/app.asar"))?
            }
        };
        Some(Desktop {
            path,
            supports_session,
        })
    }

    pub(super) fn dispatch(source: Source, path: &Path, url: Option<&str>) -> io::Result<()> {
        if !path.is_file() {
            return Err(io::Error::other("来源应用已卸载"));
        }
        if source == Source::Deepseek {
            // Windows Electron cold/second-instance 接收单独的 URL 参数。
            Command::new(path).args(url).spawn()?;
            return Ok(());
        }
        let app_id = codex_appid().ok_or_else(|| io::Error::other("Codex 协议登记已变更"))?;
        let app = format!("shell:AppsFolder\\{app_id}");
        let target = wide(url.unwrap_or(&app));
        // SAFETY: 参数为固定协议的独立 URI 或官方包 AUMID，且为 NUL 结尾。
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                wide("open").as_ptr(),
                target.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
            )
        };
        if result <= 32 {
            return Err(io::Error::other("系统未接受打开请求"));
        }
        Ok(())
    }

    pub(super) fn copy(window: &tauri::WebviewWindow, id: &str) -> io::Result<()> {
        let owner = window.hwnd().map_err(io::Error::other)?;
        let data = wide(id);
        // SAFETY: HWND 在命令期间有效。可移动缓冲成功转移所有权后由系统释放。
        unsafe {
            let buffer = GlobalAlloc(0x0002, data.len() * std::mem::size_of::<u16>());
            if buffer.is_null() {
                return Err(io::Error::last_os_error());
            }
            let pointer = GlobalLock(buffer).cast::<u16>();
            if pointer.is_null() {
                GlobalFree(buffer);
                return Err(io::Error::last_os_error());
            }
            pointer.copy_from_nonoverlapping(data.as_ptr(), data.len());
            GlobalUnlock(buffer);
            if OpenClipboard(owner.0) == 0 {
                GlobalFree(buffer);
                return Err(io::Error::last_os_error());
            }
            let success = EmptyClipboard() != 0 && !SetClipboardData(13, buffer).is_null();
            CloseClipboard();
            if !success {
                GlobalFree(buffer);
                return Err(io::Error::other("无法写入系统剪贴板"));
            }
        }
        Ok(())
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod native {
    use super::*;
    pub(super) fn desktop(_: Source) -> Option<Desktop> {
        None
    }
    pub(super) fn dispatch(_: Source, _: &Path, _: Option<&str>) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    pub(super) fn copy(_: &tauri::WebviewWindow, _: &str) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

pub(crate) fn desktop(source: Source) -> Option<Desktop> {
    native::desktop(source)
}
pub(crate) fn dispatch(source: Source, path: &Path, url: Option<&str>) -> io::Result<()> {
    native::dispatch(source, path, url)
}
pub(crate) fn copy(window: &tauri::WebviewWindow, id: &str) -> io::Result<()> {
    native::copy(window, id)
}
