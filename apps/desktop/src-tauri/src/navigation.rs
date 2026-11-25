pub(crate) mod harness;
pub(crate) mod system;

use pet_core::{SessionKey, Source};
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenIntent {
    Session,
    Application,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum OpenOutcome {
    Requested,
    ApplicationOnly { opened: bool },
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NavigationError {
    UnknownInstallation,
    InvalidSessionId,
    InvalidHostPath,
    DispatchFailed,
}

impl std::fmt::Display for NavigationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::UnknownInstallation => "未知的来源安装，请检查接入设置",
            Self::InvalidSessionId => "会话标识不适用于桌面入口，可复制后在来源应用查找",
            Self::InvalidHostPath => "来源应用路径无效，请检查安装",
            Self::DispatchFailed => "系统未接受打开请求，请检查来源应用的协议注册",
        })
    }
}

/// 从本地接入登记和操作系统提取的只读信息。入口命令只接收会话键和操作。
pub struct Registration {
    pub source: Source,
    pub installation_id: String,
    pub desktop: Option<Desktop>,
}

pub struct Desktop {
    pub path: PathBuf,
    pub supports_session: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebTarget {
    pub origin: String,
    pub supports_session: bool,
}

pub(crate) enum Target {
    Desktop(Registration),
    Web {
        source: Source,
        installation_id: String,
        target: Option<WebTarget>,
    },
}

pub fn open_session(
    key: &SessionKey,
    registration: Option<&Registration>,
    intent: OpenIntent,
    dispatch: impl FnOnce(&Path, Option<&str>) -> io::Result<()>,
) -> Result<OpenOutcome, NavigationError> {
    let registration = registration
        .filter(|r| r.source == key.source && r.installation_id == key.installation_id)
        .ok_or(NavigationError::UnknownInstallation)?;
    let url = match intent {
        OpenIntent::Session => Some(session_url(key)?),
        OpenIntent::Application => match key.source {
            Source::Codex => None,
            Source::Deepseek => Some("dsh://open".into()),
        },
    };
    let Some(desktop) = &registration.desktop else {
        return Ok(OpenOutcome::Unavailable);
    };
    if !valid_path(&desktop.path) {
        return Err(NavigationError::InvalidHostPath);
    }
    if matches!(intent, OpenIntent::Session) && !desktop.supports_session {
        return Ok(OpenOutcome::ApplicationOnly { opened: false });
    }
    dispatch(&desktop.path, url.as_deref()).map_err(|_| NavigationError::DispatchFailed)?;
    Ok(match intent {
        OpenIntent::Session => OpenOutcome::Requested,
        OpenIntent::Application => OpenOutcome::ApplicationOnly { opened: true },
    })
}

pub fn open_web_session(
    key: &SessionKey,
    expected_source: Source,
    expected_installation_id: &str,
    web: Option<&WebTarget>,
    intent: OpenIntent,
    dispatch: impl FnOnce(&str) -> io::Result<()>,
) -> Result<OpenOutcome, NavigationError> {
    if expected_source != key.source
        || expected_installation_id != key.installation_id
        || key.source != Source::Deepseek
    {
        return Err(NavigationError::UnknownInstallation);
    }
    let encoded_session = match intent {
        OpenIntent::Session => Some(encode_harness_session_id(&key.session_id)?),
        OpenIntent::Application => None,
    };
    let Some(web) = web else {
        return Ok(OpenOutcome::Unavailable);
    };
    if !valid_loopback_origin(&web.origin) {
        return Err(NavigationError::InvalidHostPath);
    }
    if matches!(intent, OpenIntent::Session) && !web.supports_session {
        return Ok(OpenOutcome::ApplicationOnly { opened: false });
    }
    let origin = web.origin.trim_end_matches('/');
    let url = match encoded_session {
        Some(encoded) => format!("{origin}/#codex-pet-session={encoded}"),
        None => format!("{origin}/"),
    };
    dispatch(&url).map_err(|_| NavigationError::DispatchFailed)?;
    Ok(match intent {
        OpenIntent::Session => OpenOutcome::Requested,
        OpenIntent::Application => OpenOutcome::ApplicationOnly { opened: true },
    })
}

fn valid_path(path: &Path) -> bool {
    path.is_absolute()
        && path
            .to_str()
            .is_some_and(|value| !value.chars().any(char::is_control) && !value.starts_with(r"\\"))
        && !path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
}

pub(crate) fn valid_loopback_origin(origin: &str) -> bool {
    let origin = origin.trim_end_matches('/');
    let Some(rest) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    else {
        return false;
    };
    if rest.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return false;
    }
    let Some((host, port)) = rest.rsplit_once(':') else {
        return false;
    };
    matches!(host, "127.0.0.1" | "localhost" | "[::1]") && port.parse::<u16>().is_ok_and(|p| p > 0)
}

fn encode_harness_session_id(id: &str) -> Result<String, NavigationError> {
    if id.trim().is_empty()
        || id.encode_utf16().count() > 1024
        || matches!(id, "." | "..")
        || id.contains("://")
        || id.chars().any(char::is_control)
    {
        return Err(NavigationError::InvalidSessionId);
    }
    let mut encoded = String::with_capacity(id.len());
    for byte in id.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write;
            write!(encoded, "%{byte:02X}").expect("写入字符串");
        }
    }
    Ok(encoded)
}

fn session_url(key: &SessionKey) -> Result<String, NavigationError> {
    let id = &key.session_id;
    match key.source {
        Source::Codex => uuid::Uuid::parse_str(id)
            .ok()
            .filter(|uuid| uuid.hyphenated().to_string().eq_ignore_ascii_case(id))
            .map(|uuid| format!("codex://threads/{uuid}"))
            .ok_or(NavigationError::InvalidSessionId),
        Source::Deepseek => Ok(format!("dsh://session/{}", encode_harness_session_id(id)?)),
    }
}
