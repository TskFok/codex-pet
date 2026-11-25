use super::WebTarget;
#[cfg(any(windows, target_os = "macos"))]
use serde_json::Value;
#[cfg(any(windows, target_os = "macos"))]
use std::io::{self, Seek, SeekFrom};
use std::{
    fs::File,
    io::Read,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream},
    path::Path,
    time::Duration,
};

#[cfg(any(windows, target_os = "macos"))]
const PACKAGE: &str = "@deepseek-ai/dsh-desktop";
const DEFAULT_WEB_HOST: &str = "127.0.0.1";
const DEFAULT_WEB_PORT: u16 = 3080;
const WEB_SESSION_MARKER: &[u8] = b"codex-pet-session";

/// 校验与本包 `engines.dsh`（`>=0.2.2-rc.1 <0.3.0-0`）一致的 Harness 桌面版本。
pub(crate) fn compatible_desktop_version(version: &str) -> bool {
    let without_build = version.split('+').next().unwrap_or("");
    let (core, pre) = match without_build.split_once('-') {
        Some((core, pre)) if !pre.is_empty() => (core, Some(pre)),
        Some(_) => return false,
        None => (without_build, None),
    };
    let mut parts = core.split('.');
    let (Some(major), Some(minor), Some(patch), None) = (
        parts.next().and_then(|v| v.parse::<u32>().ok()),
        parts.next().and_then(|v| v.parse::<u32>().ok()),
        parts.next().and_then(|v| v.parse::<u32>().ok()),
        parts.next(),
    ) else {
        return false;
    };
    if major != 0 || minor != 2 {
        return false;
    }
    if patch > 2 {
        return true;
    }
    if patch < 2 {
        return false;
    }
    match pre {
        None => true,
        Some(tag) => tag
            .strip_prefix("rc.")
            .and_then(|rest| rest.split('.').next())
            .and_then(|n| n.parse::<u32>().ok())
            .is_some_and(|n| n >= 1),
    }
}

/// 检查系统注册目标自带的发布归档。仅查固定归档条目，不解包或执行宿主。
#[cfg(any(windows, target_os = "macos"))]
pub(super) fn archive_support(archive: &Path) -> Option<bool> {
    let mut file = File::open(archive).ok()?;
    let mut sizes = [0; 16];
    file.read_exact(&mut sizes).ok()?;
    let size = |offset| u32::from_le_bytes(sizes[offset..offset + 4].try_into().unwrap());
    let header = size(4) as usize;
    let json = size(12) as usize;
    if size(0) != 4
        || !(8..=16 * 1024 * 1024).contains(&header)
        || size(8) as usize != header - 4
        || json > header - 8
        || header - 8 - json > 3
    {
        return None;
    }
    let mut bytes = vec![0; json];
    file.read_exact(&mut bytes).ok()?;
    let entries: Value = serde_json::from_slice(&bytes).ok()?;
    let base = 8 + header as u64;
    let manifest = entry(
        &mut file,
        &entries["files"]["package.json"],
        base,
        128 * 1024,
    )
    .ok()?;
    let manifest: Value = serde_json::from_slice(&manifest).ok()?;
    if manifest["name"] != PACKAGE || manifest["main"] != "lib/main.js" {
        return None;
    }
    if !manifest["version"]
        .as_str()
        .is_some_and(compatible_desktop_version)
    {
        return Some(false);
    }
    let lib = &entries["files"]["lib"]["files"];
    let has =
        |content: &[u8], marker: &[u8]| content.windows(marker.len()).any(|part| part == marker);
    let supported = entry(&mut file, &lib["main.js"], base, 16 * 1024 * 1024)
        .ok()
        .is_some_and(|data| {
            has(&data, b"dsh-desktop:session-links-pending")
                && has(&data, b"dsh-desktop:session-links-acknowledge")
        })
        && entry(&mut file, &lib["preload-app.cjs"], base, 1024 * 1024)
            .ok()
            .is_some_and(|data| {
                has(&data, b"sessionLinks") && has(&data, b"dsh-desktop:session-links-changed")
            });
    Some(supported)
}

#[cfg(any(windows, target_os = "macos"))]
fn entry(file: &mut File, item: &Value, base: u64, limit: u64) -> io::Result<Vec<u8>> {
    let missing = || io::Error::other("缺少可核对的宿主发布条目");
    let size = item["size"]
        .as_u64()
        .filter(|n| *n <= limit)
        .ok_or_else(missing)?;
    let offset = item["offset"]
        .as_str()
        .and_then(|s| s.parse::<u64>().ok())
        .ok_or_else(missing)?;
    let start = base.checked_add(offset).ok_or_else(missing)?;
    if start
        .checked_add(size)
        .is_none_or(|end| end > file.metadata().map_or(0, |m| m.len()))
    {
        return Err(missing());
    }
    file.seek(SeekFrom::Start(start))?;
    let mut data = vec![0; size as usize];
    file.read_exact(&mut data)?;
    Ok(data)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct WebServerConfig {
    host: String,
    port: u16,
    tls: bool,
}

impl Default for WebServerConfig {
    fn default() -> Self {
        Self {
            host: DEFAULT_WEB_HOST.into(),
            port: DEFAULT_WEB_PORT,
            tls: false,
        }
    }
}

fn unquote_yaml(raw: &str) -> &str {
    let trimmed = raw.trim();
    if trimmed.len() >= 2
        && ((trimmed.starts_with('\'') && trimmed.ends_with('\''))
            || (trimmed.starts_with('"') && trimmed.ends_with('"')))
    {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    }
}

fn strip_yaml_comment(raw: &str) -> &str {
    let trimmed = raw.trim();
    if trimmed.starts_with('\'') || trimmed.starts_with('"') {
        let quote = trimmed.as_bytes()[0];
        if let Some(end) = trimmed[1..].find(char::from(quote)) {
            return &trimmed[..end + 2];
        }
        return trimmed;
    }
    trimmed
        .split_once(" #")
        .map_or(trimmed, |(before, _)| before.trim_end())
}

fn apply_webserver_patch(yaml: &str, cfg: &mut WebServerConfig) {
    let mut in_webserver = false;
    let mut config_indent: Option<usize> = None;
    for raw_line in yaml.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = raw_line.len() - raw_line.trim_start_matches(' ').len();
        if indent == 0 && trimmed.starts_with('-') {
            in_webserver = false;
            config_indent = None;
            let rest = trimmed.trim_start_matches('-').trim();
            if let Some((key, val)) = rest.split_once(':') {
                let val = unquote_yaml(strip_yaml_comment(val));
                if (key.trim() == "id" && val == "webserver")
                    || (key.trim() == "name" && val == "@deepseek-ai/dsh-host-webserver")
                {
                    in_webserver = true;
                }
            }
            continue;
        }
        if let Some(ci) = config_indent {
            if indent <= ci {
                config_indent = None;
            } else if in_webserver && let Some((key, val)) = trimmed.split_once(':') {
                let key = key.trim();
                let val = unquote_yaml(strip_yaml_comment(val));
                match key {
                    "host" if !val.is_empty() && !val.starts_with("!!") => {
                        cfg.host = val.to_owned();
                    }
                    "port" => {
                        if let Ok(port) = val.parse::<u16>()
                            && port > 0
                        {
                            cfg.port = port;
                        }
                    }
                    "tls" => {
                        cfg.tls = !matches!(val, "false" | "null" | "~");
                    }
                    _ => {}
                }
                continue;
            }
        }
        if indent > 0
            && let Some((key, val)) = trimmed.split_once(':')
        {
            let key = key.trim();
            let val = unquote_yaml(strip_yaml_comment(val));
            if (key == "id" && val == "webserver")
                || (key == "name" && val == "@deepseek-ai/dsh-host-webserver")
            {
                in_webserver = true;
            } else if in_webserver && key == "config" && val.is_empty() {
                config_indent = Some(indent);
            }
        }
    }
}

fn read_small_text(path: &Path, limit: u64) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > limit {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn resolve_webserver_config(profile_dir: &Path, dsh_home: Option<&Path>) -> WebServerConfig {
    let mut cfg = WebServerConfig::default();
    if let Some(text) = read_small_text(&profile_dir.join("cordis.patch.yml"), 4 * 1024 * 1024) {
        apply_webserver_patch(&text, &mut cfg);
    }
    if let Some(home) = dsh_home
        && let Some(text) = read_small_text(&home.join("cordis.patch.yml"), 4 * 1024 * 1024)
    {
        apply_webserver_patch(&text, &mut cfg);
    }
    cfg
}

fn loopback_addrs(host: &str, port: u16) -> Option<(&'static str, Vec<SocketAddr>)> {
    match host {
        "127.0.0.1" => Some((
            "127.0.0.1",
            vec![SocketAddr::from((Ipv4Addr::LOCALHOST, port))],
        )),
        "localhost" => Some((
            "localhost",
            vec![
                SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
                SocketAddr::from((Ipv6Addr::LOCALHOST, port)),
            ],
        )),
        "::1" | "[::1]" => Some(("[::1]", vec![SocketAddr::from((Ipv6Addr::LOCALHOST, port))])),
        _ => None,
    }
}

fn is_loopback_listening(addrs: &[SocketAddr]) -> bool {
    let timeout = Duration::from_millis(150);
    addrs
        .iter()
        .any(|addr| TcpStream::connect_timeout(addr, timeout).is_ok())
}

pub(crate) fn web_client_supports_session(client_js: &Path) -> bool {
    let Ok(file) = File::open(client_js) else {
        return false;
    };
    let mut bytes = Vec::new();
    if file.take(1024 * 1024 + 1).read_to_end(&mut bytes).is_err() || bytes.len() > 1024 * 1024 {
        return false;
    }
    bytes
        .windows(WEB_SESSION_MARKER.len())
        .any(|part| part == WEB_SESSION_MARKER)
}

/// 检查本地运行的 Harness Web 入口，仅允许回环地址并校验已安装客户端插件能力。
pub(crate) fn web_target(
    profile_dir: &Path,
    dsh_home: Option<&Path>,
    package_dir: &Path,
) -> Option<WebTarget> {
    let cfg = resolve_webserver_config(profile_dir, dsh_home);
    let (host, addrs) = loopback_addrs(&cfg.host, cfg.port)?;
    let scheme = if cfg.tls { "https" } else { "http" };
    let origin = format!("{scheme}://{host}:{}", cfg.port);
    if !super::valid_loopback_origin(&origin) || !is_loopback_listening(&addrs) {
        return None;
    }
    let supports_session = web_client_supports_session(&package_dir.join("dist/client.js"));
    Some(WebTarget {
        origin,
        supports_session,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn desktop_version_matches_plugin_engine_range() {
        for ok in [
            "0.2.2-rc.1",
            "0.2.2-rc.5",
            "0.2.2",
            "0.2.3-alpha.2",
            "0.2.9",
        ] {
            assert!(compatible_desktop_version(ok), "{ok}");
        }
        for bad in ["0.2.1", "0.2.2-alpha.1", "0.2.2-rc.0", "0.3.0", "1.0.0", ""] {
            assert!(!compatible_desktop_version(bad), "{bad}");
        }
    }

    #[test]
    fn webserver_patch_parser_reads_loopback_overrides_and_ignores_other_entries() {
        let yaml = r#"
- id: ui-settings-general
  config:
    port: 9999
- id: webserver
  name: "@deepseek-ai/dsh-host-webserver"
  config:
    host: "localhost"
    port: 3099 # custom port
    tls:
      cert: /tmp/cert.pem
"#;
        let mut cfg = WebServerConfig::default();
        apply_webserver_patch(yaml, &mut cfg);
        assert_eq!(
            cfg,
            WebServerConfig {
                host: "localhost".into(),
                port: 3099,
                tls: true,
            }
        );
    }

    #[test]
    fn web_target_detects_running_loopback_server_and_client_marker() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let temp = tempfile::tempdir().unwrap();
        let profile = temp.path().join("profiles/web");
        let pkg = profile.join("node_modules/@codex-pet/deepseek-integration");
        std::fs::create_dir_all(pkg.join("dist")).unwrap();
        std::fs::write(
            profile.join("cordis.patch.yml"),
            format!("- id: webserver\n  config:\n    port: {port}\n"),
        )
        .unwrap();
        std::fs::write(pkg.join("dist/client.js"), "console.log('old client');").unwrap();

        let target = web_target(&profile, Some(temp.path()), &pkg).unwrap();
        assert_eq!(target.origin, format!("http://127.0.0.1:{port}"));
        assert!(!target.supports_session);

        std::fs::write(
            pkg.join("dist/client.js"),
            "const prefix = '#codex-pet-session=';",
        )
        .unwrap();
        let upgraded = web_target(&profile, Some(temp.path()), &pkg).unwrap();
        assert!(upgraded.supports_session);

        drop(listener);
        assert!(web_target(&profile, Some(temp.path()), &pkg).is_none());
    }
}
