//! 原生转发器的有界、跨进程待发送区。
use pet_core::{
    AppPaths, EventEnvelope, MAX_EVENT_BYTES, StorageError, decode_event,
    paths::{
        atomic_write, durable_rename, is_link, open_regular, prepare_directory, regular_metadata,
        require_regular, sync_directory, wall_clock_ms,
    },
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{self, Read},
    path::{Component, Path, PathBuf},
};

const MAX_PENDING: usize = 2_000;
const MAX_BYTES: u64 = 10 * 1024 * 1024;
const MAX_AGE_MS: u64 = 24 * 60 * 60 * 1_000;
const QUARANTINE_SAMPLES: u64 = 32;
const STAGING_PREFIX: &str = ".outbox-stage-";
const LOCK: &str = ".outbox.lock";
const STATE: &str = ".outbox-state.json";
const QUARANTINE: &str = "quarantine";

pub type DeliveryError = StorageError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedEvent {
    pub envelope: EventEnvelope,
    pub queued_at_ms: u64,
    pub path: PathBuf,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutboxDiagnostics {
    pub expired_discarded: u64,
    pub overflow_discarded: u64,
    pub corrupt_isolated: u64,
}

#[derive(Default, Serialize, Deserialize)]
struct DiskState {
    diagnostics: OutboxDiagnostics,
    // 计数与清理清单一起发布。进程中途停止时，下次持锁继续清理。
    cleanup: Vec<Removal>,
}

#[derive(Serialize, Deserialize)]
struct Removal {
    name: String,
    quarantine_index: Option<u64>,
}

struct Slot {
    name: String,
    order: u64,
    queued_at_ms: u64,
    size: u64,
}

impl Slot {
    fn from_name(name: String, size: u64) -> Result<Self, String> {
        let Some(parts) = name
            .strip_prefix("q-")
            .and_then(|name| name.strip_suffix(".json"))
        else {
            return Err(name);
        };
        let mut parts = parts.split('-');
        let (Some(order), Some(queued_at_ms), Some(key), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(name);
        };
        if order.len() != 20
            || queued_at_ms.len() != 20
            || key.len() != 64
            || !key
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err(name);
        }
        let (Ok(order), Ok(queued_at_ms)) = (order.parse(), queued_at_ms.parse()) else {
            return Err(name);
        };
        Ok(Self {
            name,
            order,
            queued_at_ms,
            size,
        })
    }

    fn key(&self) -> &str {
        // from_name 和 enqueue 均保证这一固定格式，键从倒数 .json 前截取。
        let offset = self.name.len() - ".json".len();
        &self.name[offset - 64..offset]
    }
}

pub struct Outbox {
    paths: AppPaths,
    clock: Box<dyn Fn() -> u64 + Send + Sync>,
}

impl Outbox {
    pub fn new(paths: AppPaths) -> Result<Self, DeliveryError> {
        Self::with_clock(paths, wall_clock_ms)
    }

    pub fn with_clock(
        paths: AppPaths,
        clock: impl Fn() -> u64 + Send + Sync + 'static,
    ) -> Result<Self, DeliveryError> {
        paths.prepare_outbox()?;
        Ok(Self {
            paths,
            clock: Box::new(clock),
        })
    }

    pub fn enqueue(&self, event: &EventEnvelope) -> Result<(), DeliveryError> {
        let bytes = serde_json::to_vec(event)?;
        decode_event(&bytes)?;
        let key = event_key(&event.event_id);
        let _lock = self.lock()?;
        let now = (self.clock)();
        let mut state = self.load_state()?;
        let mut slots = self.scan(&mut state)?;
        self.expire(now, &mut state, &mut slots);
        // 恢复上一进程在发布后、修剪前退出时可能遗留的超额项。
        self.prune_unverified_overflow(&mut state, &mut slots)?;
        let existing = slots.iter().position(|slot| slot.key() == key);
        let already_queued = if let Some(index) = existing {
            let slot = &slots[index];
            if self.read_event(slot)?.is_some() {
                true
            } else {
                state.isolate(slots.remove(index).name, true);
                false
            }
        } else {
            false
        };
        self.commit_cleanup(&mut state)?;
        if !already_queued {
            let order = slots.last().map_or(Ok(0), |slot| {
                slot.order
                    .checked_add(1)
                    .ok_or(StorageError::InvalidData("待发送区序号已耗尽"))
            })?;
            let name = format!("q-{order:020}-{now:020}-{key}.json");
            atomic_write(&self.paths.outbox.join(&name), STAGING_PREFIX, &bytes)?;
            sync_directory(&self.paths.outbox)?;
            slots.push(Slot {
                name,
                order,
                queued_at_ms: now,
                size: bytes.len() as u64,
            });
        }
        self.prune_unverified_overflow(&mut state, &mut slots)?;
        self.commit_cleanup(&mut state)
    }

    pub fn pending(&self, now_ms: u64) -> Result<Vec<QueuedEvent>, DeliveryError> {
        let _lock = self.lock()?;
        let mut state = self.load_state()?;
        let mut slots = self.scan(&mut state)?;
        self.expire(now_ms, &mut state, &mut slots);
        let mut events = Vec::with_capacity(slots.len());
        let mut valid = Vec::with_capacity(slots.len());
        for slot in slots {
            if let Some(envelope) = self.read_event(&slot)? {
                events.push(QueuedEvent {
                    envelope,
                    queued_at_ms: slot.queued_at_ms,
                    path: self.paths.outbox.join(&slot.name),
                });
                valid.push(slot);
            } else {
                state.isolate(slot.name, true);
            }
        }
        let excess = self.enforce_limits(&mut state, &mut valid);
        self.commit_cleanup(&mut state)?;
        events.drain(..excess);
        Ok(events)
    }

    pub fn acknowledge(&self, event_id: &str) -> Result<(), DeliveryError> {
        self.acknowledge_many([event_id])
    }

    pub fn acknowledge_many<'a>(
        &self,
        event_ids: impl IntoIterator<Item = &'a str>,
    ) -> Result<(), DeliveryError> {
        let keys: HashSet<_> = event_ids.into_iter().map(event_key).collect();
        if keys.is_empty() {
            return Ok(());
        }
        let _lock = self.lock()?;
        let mut state = self.load_state()?;
        let mut slots = self.scan(&mut state)?;
        let now = (self.clock)();
        self.expire(now, &mut state, &mut slots);
        slots.retain(|slot| {
            if keys.contains(slot.key()) {
                state.discard(slot.name.clone());
                false
            } else {
                true
            }
        });
        self.prune_unverified_overflow(&mut state, &mut slots)?;
        self.commit_cleanup(&mut state)
    }

    /// 读取跨进程累计清理数量。调用方可保存上次展示的计数。
    pub fn diagnostics(&self) -> Result<OutboxDiagnostics, DeliveryError> {
        let _lock = self.lock()?;
        Ok(self.load_state()?.diagnostics)
    }

    fn lock(&self) -> Result<File, DeliveryError> {
        self.paths.prepare_outbox()?;
        let file = open_regular(
            &self.paths.outbox.join(LOCK),
            OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false),
        )?;
        file.lock()?;
        Ok(file)
    }

    fn load_state(&self) -> Result<DiskState, DeliveryError> {
        let path = self.paths.outbox.join(STATE);
        let mut state = match open_regular(&path, OpenOptions::new().read(true)) {
            Ok(file) => serde_json::from_reader(file)?,
            Err(StorageError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
                DiskState::default()
            }
            Err(error) => return Err(error),
        };
        self.complete_cleanup(&mut state)?;
        Ok(state)
    }

    fn scan(&self, state: &mut DiskState) -> Result<Vec<Slot>, DeliveryError> {
        let mut slots = Vec::new();
        for entry in fs::read_dir(&self.paths.outbox)? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name();
            match name.to_str() {
                Some(LOCK | STATE) => continue,
                Some(QUARANTINE) => {
                    let metadata = entry.metadata()?;
                    if !metadata.is_dir() || is_link(&metadata) {
                        return Err(StorageError::UnsafePath(path));
                    }
                    continue;
                }
                Some(name) if name.starts_with(STAGING_PREFIX) => {
                    regular_metadata(&path)?;
                    fs::remove_file(&path)?;
                    continue;
                }
                _ => {}
            }
            let metadata = entry.metadata()?;
            require_regular(&path, &metadata)?;
            // 无法序列化为清理计划的本地名称不属于待发送项。
            let Ok(name) = name.into_string() else {
                continue;
            };
            let size = metadata.len();
            let retain_sample = size <= MAX_EVENT_BYTES as u64;
            match Slot::from_name(name, size) {
                Ok(slot) if retain_sample => slots.push(slot),
                Ok(slot) => state.isolate(slot.name, false),
                Err(name) => {
                    if cleanup_target(&name)?.is_some() {
                        state.isolate(name, retain_sample);
                    }
                }
            }
        }
        slots.sort_unstable_by(|left, right| {
            left.order
                .cmp(&right.order)
                .then_with(|| left.name.cmp(&right.name))
        });
        Ok(slots)
    }

    fn read_event(&self, slot: &Slot) -> Result<Option<EventEnvelope>, DeliveryError> {
        let file = open_regular(
            &self.paths.outbox.join(&slot.name),
            OpenOptions::new().read(true),
        )?;
        let mut bytes = Vec::with_capacity(slot.size as usize);
        file.take(MAX_EVENT_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        Ok(decode_event(&bytes)
            .ok()
            .filter(|event| event_key(&event.event_id) == slot.key()))
    }

    fn expire(&self, now_ms: u64, state: &mut DiskState, slots: &mut Vec<Slot>) {
        slots.retain(|slot| {
            if now_ms.saturating_sub(slot.queued_at_ms) >= MAX_AGE_MS {
                state.diagnostics.expired_discarded =
                    state.diagnostics.expired_discarded.saturating_add(1);
                state.discard(slot.name.clone());
                false
            } else {
                true
            }
        });
    }

    fn prune_unverified_overflow(
        &self,
        state: &mut DiskState,
        slots: &mut Vec<Slot>,
    ) -> Result<(), DeliveryError> {
        if slots.len() <= MAX_PENDING
            && slots.iter().map(|slot| slot.size).sum::<u64>() <= MAX_BYTES
        {
            return Ok(());
        }
        let mut valid = Vec::with_capacity(slots.len());
        for slot in slots.drain(..) {
            if self.read_event(&slot)?.is_some() {
                valid.push(slot);
            } else {
                state.isolate(slot.name, true);
            }
        }
        *slots = valid;
        self.enforce_limits(state, slots);
        Ok(())
    }

    fn enforce_limits(&self, state: &mut DiskState, slots: &mut Vec<Slot>) -> usize {
        let mut size = slots.iter().map(|slot| slot.size).sum::<u64>();
        let mut excess = 0;
        while slots.len() - excess > MAX_PENDING || size > MAX_BYTES {
            size -= slots[excess].size;
            excess += 1;
        }
        for slot in slots.drain(..excess) {
            state.diagnostics.overflow_discarded =
                state.diagnostics.overflow_discarded.saturating_add(1);
            state.discard(slot.name);
        }
        excess
    }

    fn save_state(&self, state: &DiskState) -> Result<(), DeliveryError> {
        atomic_write(
            &self.paths.outbox.join(STATE),
            STAGING_PREFIX,
            &serde_json::to_vec(state)?,
        )?;
        sync_directory(&self.paths.outbox)
    }

    fn commit_cleanup(&self, state: &mut DiskState) -> Result<(), DeliveryError> {
        if !state.cleanup.is_empty() {
            self.save_state(state)?;
            self.complete_cleanup(state)?;
        }
        Ok(())
    }

    fn complete_cleanup(&self, state: &mut DiskState) -> Result<(), DeliveryError> {
        if state.cleanup.is_empty() {
            return Ok(());
        }
        let quarantine = self.paths.outbox.join(QUARANTINE);
        let mut sync_quarantine = false;
        for entry in &state.cleanup {
            let Some(name) = cleanup_target(&entry.name)? else {
                continue;
            };
            if entry.quarantine_index.is_some() && !sync_quarantine {
                prepare_directory(&quarantine)?;
                sync_quarantine = true;
            }
            let path = self.paths.outbox.join(name);
            match regular_metadata(&path) {
                Ok(_) => {}
                Err(StorageError::Io(error)) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            }
            if let Some(index) = entry.quarantine_index {
                let slot = index % QUARANTINE_SAMPLES;
                durable_rename(&path, &quarantine.join(format!("{slot:02}.bad")))?;
            } else {
                remove_slot(&path)?;
            }
        }
        if sync_quarantine {
            sync_directory(&quarantine)?;
        }
        sync_directory(&self.paths.outbox)?;
        state.cleanup.clear();
        self.save_state(state)
    }
}

impl DiskState {
    fn discard(&mut self, name: String) {
        self.cleanup.push(Removal {
            name,
            quarantine_index: None,
        });
    }

    fn isolate(&mut self, name: String, retain_sample: bool) {
        let index = self.diagnostics.corrupt_isolated;
        self.diagnostics.corrupt_isolated = index.saturating_add(1);
        self.cleanup.push(Removal {
            name,
            quarantine_index: retain_sample.then_some(index),
        });
    }
}

fn cleanup_target(name: &str) -> Result<Option<&Path>, DeliveryError> {
    let path = Path::new(name);
    let mut parts = path.components();
    if !matches!(parts.next(), Some(Component::Normal(part)) if part == path.as_os_str())
        || parts.next().is_some()
        || (cfg!(windows) && name.contains(['\\', ':']))
    {
        return Err(StorageError::UnsafePath(path.into()));
    }
    // 管理文件及 Windows 会重解释的名称不加入清理计划。旧计划也不操作它们。
    let normalized = name.to_ascii_lowercase();
    if (cfg!(windows) && name.ends_with(['.', ' ']))
        || matches!(normalized.as_str(), LOCK | STATE | QUARANTINE)
        || normalized.starts_with(STAGING_PREFIX)
    {
        return Ok(None);
    }
    Ok(Some(path))
}

fn remove_slot(path: &Path) -> Result<(), DeliveryError> {
    #[cfg(not(windows))]
    fs::remove_file(path)?;
    #[cfg(windows)]
    {
        // 写穿透移出队列，后续清理即使被意外中断也不会重新投递。
        let staging = path.with_file_name(format!("{STAGING_PREFIX}discard"));
        durable_rename(path, &staging)?;
        fs::remove_file(staging)?;
    }
    Ok(())
}

fn event_key(id: &str) -> String {
    format!("{:x}", Sha256::digest(id.as_bytes()))
}
