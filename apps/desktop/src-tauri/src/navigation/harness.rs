use serde_json::Value;
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
};

const PACKAGE: &str = "@deepseek-ai/dsh-desktop";

/// 检查系统注册目标自带的发布归档。仅查固定归档条目，不解包或执行宿主。
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
    if manifest["version"] != "0.2.2-rc.1" {
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
