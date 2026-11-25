//! 单写入者事件日志，按持久化顺序重放收件、已读、清空和重启边界。
use crate::{
    AppPaths, EventEnvelope, PetSnapshot, ProtocolError, SessionKey, SessionStore, decode_event,
    paths::{atomic_write, open_regular, regular_metadata, sync_directory, wall_clock_ms},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions, TryLockError},
    io::{self, BufRead, BufReader, Seek, SeekFrom, Write},
};

const RETAIN_DEDUP_MS: u64 = 24 * 60 * 60 * 1_000;
const CHECKPOINT_FORMAT: u8 = 1;
const STAGING_PREFIX: &str = ".journal-stage-";

#[derive(Debug)]
pub enum StorageError {
    Io(io::Error),
    Json(serde_json::Error),
    InvalidEvent(ProtocolError),
    InvalidData(&'static str),
    UnsafePath(std::path::PathBuf),
    WriterBusy,
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "文件操作失败：{error}"),
            Self::Json(error) => write!(f, "本地记录格式无效：{error}"),
            Self::InvalidEvent(error) => write!(f, "事件无效：{error}"),
            Self::InvalidData(message) => f.write_str(message),
            Self::UnsafePath(path) => write!(f, "不安全的应用数据路径：{}", path.display()),
            Self::WriterBusy => f.write_str("另一个进程正在写入事件日志"),
        }
    }
}

impl std::error::Error for StorageError {}

impl From<io::Error> for StorageError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for StorageError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<ProtocolError> for StorageError {
    fn from(error: ProtocolError) -> Self {
        Self::InvalidEvent(error)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IngestAck {
    pub event_id: String,
    pub duplicate: bool,
    pub revision: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Record {
    Event { envelope: Box<EventEnvelope> },
    MarkRead { key: SessionKey },
    ClearHistory,
    Advance { now_ms: u64, restarted: bool },
}

#[derive(Default, Deserialize, Serialize)]
struct DurableState {
    #[serde(with = "crate::state::SessionCheckpoint")]
    sessions: SessionStore,
    dedup: HashMap<String, u64>,
}

impl DurableState {
    fn prune_dedup(&mut self, now_ms: u64) {
        self.dedup
            .retain(|_, received| now_ms.saturating_sub(*received) <= RETAIN_DEDUP_MS);
    }

    fn apply(&mut self, record: Record) -> Result<(), StorageError> {
        match record {
            Record::Event { envelope } => {
                let received_at_ms = envelope
                    .received_at_ms
                    .ok_or(StorageError::InvalidData("日志事件缺少本地接收时间"))?;
                self.sessions.apply_event(&envelope);
                self.dedup.insert(envelope.event_id, received_at_ms);
            }
            Record::MarkRead { key } => {
                self.sessions.mark_read(&key);
            }
            Record::ClearHistory => {
                self.sessions.clear_history();
            }
            Record::Advance { now_ms, restarted } => {
                self.sessions.advance_time(now_ms, restarted);
                self.prune_dedup(now_ms);
            }
        }
        Ok(())
    }
}

#[derive(Deserialize, Serialize)]
struct Checkpoint<T> {
    format: u8,
    checkpoint: T,
}

pub struct JournalStore {
    paths: AppPaths,
    state: DurableState,
    clock: Box<dyn Fn() -> u64 + Send + Sync>,
    tail: u64,
    recovered: bool,
    directory_sync_pending: bool,
    _writer_lock: File,
}

impl JournalStore {
    pub fn open(paths: AppPaths) -> Result<Self, StorageError> {
        Self::open_with_clock(paths, wall_clock_ms)
    }

    pub fn open_with_clock(
        paths: AppPaths,
        clock: impl Fn() -> u64 + Send + Sync + 'static,
    ) -> Result<Self, StorageError> {
        paths.prepare_root()?;
        let writer_lock = open_regular(
            &paths.root.join("journal.lock"),
            OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false),
        )?;
        match writer_lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Err(StorageError::WriterBusy),
            Err(TryLockError::Error(error)) => return Err(error.into()),
        }
        for entry in fs::read_dir(&paths.root)? {
            let entry = entry?;
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with(STAGING_PREFIX)
            {
                regular_metadata(&entry.path())?;
                fs::remove_file(entry.path())?;
            }
        }
        let mut file = open_regular(
            &paths.journal,
            OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false),
        )?;
        let (state, tail) = read_journal(&mut file)?;
        if file.metadata()?.len() != tail {
            file.set_len(tail)?;
            file.sync_all()?;
        }
        sync_directory(&paths.root)?;
        Ok(Self {
            paths,
            state,
            clock: Box::new(clock),
            tail,
            recovered: false,
            directory_sync_pending: false,
            _writer_lock: writer_lock,
        })
    }

    pub fn ingest(&mut self, mut event: EventEnvelope) -> Result<IngestAck, StorageError> {
        decode_event(&serde_json::to_vec(&event)?)?;
        // 不信任事件附带的接收时间，将本机时间绑定到落盘记录。
        let now = (self.clock)().max(self.state.sessions.time());
        event.received_at_ms = Some(now);
        self.ensure_recovered(now)?;
        let event_id = event.event_id.clone();
        let duplicate = self
            .state
            .dedup
            .get(&event_id)
            .is_some_and(|received| now.saturating_sub(*received) <= RETAIN_DEDUP_MS);
        if !duplicate {
            self.commit(Record::Event {
                envelope: Box::new(event),
            })?;
        }
        Ok(IngestAck {
            event_id,
            duplicate,
            revision: self.state.sessions.revision(),
        })
    }

    pub fn recover(&mut self, now_ms: u64) -> Result<PetSnapshot, StorageError> {
        let restarted = !self.recovered;
        let now_ms = now_ms.max(self.state.sessions.time());
        if restarted || now_ms > self.state.sessions.time() {
            self.commit(Record::Advance { now_ms, restarted })?;
            self.recovered = true;
        }
        Ok(self.snapshot())
    }

    pub fn snapshot(&self) -> PetSnapshot {
        self.state.sessions.snapshot()
    }

    pub fn compact(&mut self, now_ms: u64) -> Result<(), StorageError> {
        self.recover(now_ms)?;
        self.state.prune_dedup(self.state.sessions.time());
        let bytes = encode_line(&Checkpoint {
            format: CHECKPOINT_FORMAT,
            checkpoint: &self.state,
        })?;
        self.paths.prepare_root()?;
        atomic_write(&self.paths.journal, STAGING_PREFIX, &bytes)?;
        self.tail = bytes.len() as u64;
        self.directory_sync_pending = true;
        self.finish_publication()
    }

    pub fn mark_read(&mut self, key: &SessionKey) -> Result<PetSnapshot, StorageError> {
        self.ensure_recovered((self.clock)())?;
        self.commit(Record::MarkRead { key: key.clone() })?;
        Ok(self.snapshot())
    }

    pub fn mark_read_many(&mut self, keys: &[SessionKey]) -> Result<PetSnapshot, StorageError> {
        self.ensure_recovered((self.clock)())?;
        let mut seen = HashSet::new();
        let records: Vec<Record> = keys
            .iter()
            .filter(|key| seen.insert(*key))
            .map(|key| Record::MarkRead { key: key.clone() })
            .collect();
        if !records.is_empty() {
            self.commit_many(records)?;
        }
        Ok(self.snapshot())
    }

    pub fn clear_history(&mut self) -> Result<PetSnapshot, StorageError> {
        self.ensure_recovered((self.clock)())?;
        self.commit(Record::ClearHistory)?;
        Ok(self.snapshot())
    }

    fn ensure_recovered(&mut self, now_ms: u64) -> Result<(), StorageError> {
        if !self.recovered {
            self.recover(now_ms)?;
        }
        Ok(())
    }

    fn finish_publication(&mut self) -> Result<(), StorageError> {
        if self.directory_sync_pending {
            sync_directory(&self.paths.root)?;
            self.directory_sync_pending = false;
        }
        Ok(())
    }

    fn commit(&mut self, record: Record) -> Result<(), StorageError> {
        self.commit_many(vec![record])
    }

    fn commit_many(&mut self, records: Vec<Record>) -> Result<(), StorageError> {
        if records.is_empty() {
            return Ok(());
        }
        let mut bytes = Vec::new();
        for record in &records {
            bytes.extend_from_slice(&encode_line(record)?);
        }
        self.paths.prepare_root()?;
        self.finish_publication()?;
        let mut file = open_regular(&self.paths.journal, OpenOptions::new().write(true))?;
        match file.metadata()?.len().cmp(&self.tail) {
            std::cmp::Ordering::Less => {
                return Err(StorageError::InvalidData("日志在写入期间被截断"));
            }
            std::cmp::Ordering::Greater => file.set_len(self.tail)?,
            std::cmp::Ordering::Equal => {}
        }
        file.seek(SeekFrom::Start(self.tail))?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        self.tail += bytes.len() as u64;
        for record in records {
            self.state.apply(record)?;
        }
        Ok(())
    }
}

fn read_journal(file: &mut File) -> Result<(DurableState, u64), StorageError> {
    let mut reader = BufReader::new(file);
    let mut state = DurableState::default();
    let mut tail = 0;
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 || line.last() != Some(&b'\n') {
            break;
        }
        apply_line(&mut state, &line, tail == 0)?;
        tail += line.len() as u64;
    }
    Ok((state, tail))
}

fn apply_line(state: &mut DurableState, line: &[u8], first: bool) -> Result<(), StorageError> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum JournalLine {
        Operation(Record),
        Checkpoint(Box<Checkpoint<DurableState>>),
    }
    match serde_json::from_slice(line)? {
        JournalLine::Operation(record) => state.apply(record),
        JournalLine::Checkpoint(checkpoint) if first && checkpoint.format == CHECKPOINT_FORMAT => {
            *state = checkpoint.checkpoint;
            Ok(())
        }
        JournalLine::Checkpoint(_) => Err(StorageError::InvalidData("日志检查点位置或版本无效")),
    }
}

fn encode_line(value: &impl Serialize) -> Result<Vec<u8>, StorageError> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}
