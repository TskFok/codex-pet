//! 收件、离线补发和日志维护共用一个后台写入线程。
use axum::{
    Json, Router,
    body::to_bytes,
    extract::{Request, State},
    http::{HeaderValue, StatusCode, header},
    routing::post,
};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use pet_bridge::{
    delivery::{Discovery, EVENTS_ROUTE},
    outbox::Outbox,
};
use pet_core::{
    AppPaths, EventEnvelope, IngestAck, JournalStore, MAX_EVENT_BYTES, PetSnapshot, StorageError,
    decode_event, paths::wall_clock_ms,
};
use std::{
    fs, io,
    net::{Ipv4Addr, TcpListener},
    sync::{
        Arc, RwLock,
        mpsc::{self, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use tokio::sync::{broadcast, oneshot};

const SCAN_INTERVAL: Duration = Duration::from_secs(2);
const COMPACT_INTERVAL: Duration = Duration::from_secs(5 * 60);
const BODY_TIMEOUT: Duration = Duration::from_secs(2);

pub type BrokerError = StorageError;

pub struct BrokerHandle {
    view: BrokerView,
    commands: SyncSender<Work>,
    worker: Option<JoinHandle<()>>,
    server: Option<JoinHandle<()>>,
    stop_server: Option<oneshot::Sender<()>>,
}

#[derive(Clone)]
pub(crate) struct BrokerView(Arc<Shared>);

struct Shared {
    latest: RwLock<PetSnapshot>,
    faults: RwLock<Vec<FaultStage>>,
    updates: broadcast::Sender<PetSnapshot>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FaultStage {
    Listener,
    OutboxScan,
    JournalAppend,
    OutboxConfirm,
    JournalCompact,
}

impl FaultStage {
    fn message(self) -> &'static str {
        match self {
            Self::Listener => "本机入口已停止",
            Self::OutboxScan => "无法扫描待发送区",
            Self::JournalAppend => "事件日志写入失败",
            Self::OutboxConfirm => "待发送区确认失败",
            Self::JournalCompact => "事件日志压缩失败",
        }
    }
}

impl BrokerView {
    fn new() -> Self {
        Self(Arc::new(Shared {
            latest: RwLock::new(PetSnapshot::default()),
            faults: RwLock::new(Vec::new()),
            updates: broadcast::channel(32).0,
        }))
    }

    pub(crate) fn snapshot(&self) -> PetSnapshot {
        self.0.latest.read().unwrap().clone()
    }

    pub(crate) fn fault(&self) -> Option<&'static str> {
        self.0
            .faults
            .read()
            .unwrap()
            .first()
            .copied()
            .map(FaultStage::message)
    }

    fn set_fault(&self, stage: FaultStage, error: &BrokerError) {
        let mut faults = self.0.faults.write().unwrap();
        if !faults.contains(&stage) {
            faults.push(stage);
            drop(faults);
            eprintln!("桌宠通知：{}，{error}", stage.message());
        }
    }

    fn track<T>(
        &self,
        stage: FaultStage,
        result: Result<T, BrokerError>,
    ) -> Result<T, BrokerError> {
        match &result {
            Ok(_) => self
                .0
                .faults
                .write()
                .unwrap()
                .retain(|fault| *fault != stage),
            Err(error) => self.set_fault(stage, error),
        }
        result
    }

    fn publish(&self, snapshot: PetSnapshot) {
        let mut latest = self.0.latest.write().unwrap();
        if latest.revision != snapshot.revision {
            *latest = snapshot;
            let _ = self.0.updates.send(latest.clone());
        }
    }
}

pub(crate) struct BrokerStartup {
    paths: AppPaths,
    writer: Option<Writer>,
    recovered: bool,
}

impl BrokerStartup {
    pub(crate) fn new(paths: AppPaths) -> Self {
        Self {
            paths,
            writer: None,
            recovered: false,
        }
    }

    pub(crate) fn attempt(&mut self) -> Result<BrokerHandle, BrokerError> {
        let paths = &self.paths;
        if self.writer.is_none() {
            let journal = JournalStore::open(paths.clone())?;
            let outbox = Outbox::new(paths.clone())?;
            self.writer = Some(Writer::new(journal, outbox));
        }
        let writer = self.writer.as_mut().expect("已打开日志写入器");
        if !self.recovered {
            writer.journal.recover(wall_clock_ms())?;
            self.recovered = true;
        }

        let (commands, jobs) = mpsc::sync_channel(64);
        // 先注册监测，再读初始队列。扫描期间发生的变更会留在频道内。
        let watcher = match watch_outbox(paths, commands.clone()) {
            Ok(watcher) => Some(watcher),
            Err(error) => {
                eprintln!("桌宠通知：无法监测待发送区，继续定时回扫，{error}");
                None
            }
        };
        let view = BrokerView::new();
        writer.drain(None, &view)?;
        *view.0.latest.write().unwrap() = writer.journal.snapshot();

        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let discovery = Discovery::new(listener.local_addr()?.port());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let listener = {
            let _runtime = runtime.enter();
            tokio::net::TcpListener::from_std(listener)?
        };

        let (stop_server, shutdown) = oneshot::channel();
        let mut broker = BrokerHandle {
            view: view.clone(),
            commands: commands.clone(),
            worker: None,
            server: None,
            stop_server: Some(stop_server),
        };
        // 发布完成才交接日志。失败时先断开交接，再回收候命线程。
        let (handoff, waiting) = mpsc::channel::<Writer>();
        let worker_view = view.clone();
        let worker_paths = paths.clone();
        let worker_discovery = discovery.clone();
        broker.worker = Some(
            thread::Builder::new()
                .name("pet-notification-writer".into())
                .spawn(move || {
                    let Ok(mut writer) = waiting.recv() else {
                        return;
                    };
                    let _watcher = watcher;
                    writer.run(jobs, worker_view);
                    // 在仍持有日志写锁时移除本次发现记录。
                    if Discovery::read(&worker_paths)
                        .is_ok_and(|current| current == worker_discovery)
                    {
                        let _ = fs::remove_file(&worker_paths.discovery);
                    }
                })?,
        );
        let app = Router::new()
            .route(EVENTS_ROUTE, post(ingest))
            .with_state(HttpState {
                commands,
                authorization: format!("Bearer {}", discovery.token),
            });
        broker.server = Some(
            thread::Builder::new()
                .name("pet-notification-loopback".into())
                .spawn(move || {
                    runtime.block_on(async {
                        tokio::select! {
                            biased;
                            _ = shutdown => {}
                            result = async { axum::serve(listener, app).await } => {
                                let error = result.err().unwrap_or_else(|| io::Error::other("监听任务意外结束"));
                                view.set_fault(FaultStage::Listener, &error.into());
                            }
                        }
                    });
                })?,
        );
        discovery.publish(paths)?;
        let writer = self.writer.take().expect("启动完成后交接日志写入器");
        if let Err(mpsc::SendError(writer)) = handoff.send(writer) {
            self.writer = Some(writer);
            return Err(io::Error::other("通知写入线程在交接前停止").into());
        }
        Ok(broker)
    }
}

impl BrokerHandle {
    /// 读取日志和补发会执行磁盘操作，应用须从后台启动。
    pub fn start(paths: AppPaths) -> Result<Self, BrokerError> {
        BrokerStartup::new(paths).attempt()
    }

    pub fn snapshot(&self) -> PetSnapshot {
        self.view.snapshot()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<PetSnapshot> {
        self.view.0.updates.subscribe()
    }

    pub fn rescan(&self) {
        let _ = self.commands.try_send(Work::Rescan);
    }

    pub(crate) fn view(&self) -> BrokerView {
        self.view.clone()
    }
}

impl Drop for BrokerHandle {
    fn drop(&mut self) {
        if let Some(stop) = self.stop_server.take() {
            let _ = stop.send(());
        }
        if let Some(server) = self.server.take() {
            let _ = server.join();
        }
        let _ = self.commands.send(Work::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

enum Work {
    Ingest(
        Box<EventEnvelope>,
        oneshot::Sender<Result<IngestAck, BrokerError>>,
    ),
    Rescan,
    Stop,
}

struct Writer {
    journal: JournalStore,
    outbox: Outbox,
    pending_confirmations: Vec<String>,
}

impl Writer {
    fn new(journal: JournalStore, outbox: Outbox) -> Self {
        Self {
            journal,
            outbox,
            pending_confirmations: Vec::new(),
        }
    }

    fn run(&mut self, jobs: mpsc::Receiver<Work>, view: BrokerView) {
        let mut next_scan = Instant::now() + SCAN_INTERVAL;
        let mut next_compact = Instant::now() + COMPACT_INTERVAL;
        loop {
            let now = Instant::now();
            // 回扫弥补系统休眠以及文件监测漏报，持续有请求时也不能饿死维护。
            if now >= next_scan {
                self.process(None, &view).ok();
                next_scan = Instant::now() + SCAN_INTERVAL;
            }
            if now >= next_compact {
                self.compact(&view);
                next_compact = Instant::now() + COMPACT_INTERVAL;
            }
            let deadline = next_scan.min(next_compact);
            match jobs.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(Work::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Ok(Work::Rescan) => {
                    self.process(None, &view).ok();
                }
                Ok(Work::Ingest(event, reply)) => {
                    let result = self
                        .process(Some(*event), &view)
                        .map(|ack| ack.expect("HTTP 收件应有确认"));
                    let _ = reply.send(result);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }

    fn compact(&mut self, view: &BrokerView) {
        let result = self.journal.compact(wall_clock_ms());
        if view.track(FaultStage::JournalCompact, result).is_ok() {
            view.publish(self.journal.snapshot());
        }
    }

    fn process(
        &mut self,
        incoming: Option<EventEnvelope>,
        view: &BrokerView,
    ) -> Result<Option<IngestAck>, StorageError> {
        let ack = self.drain(incoming, view)?;
        view.publish(self.journal.snapshot());
        Ok(ack)
    }

    fn append(
        &mut self,
        event: EventEnvelope,
        view: &BrokerView,
    ) -> Result<IngestAck, StorageError> {
        let result = self.journal.ingest(event);
        // 重复确认来自去重记录，不会重新执行失败的日志写入。
        if result.as_ref().is_ok_and(|ack| ack.duplicate) {
            return result;
        }
        view.track(FaultStage::JournalAppend, result)
    }

    fn confirm(&mut self, view: &BrokerView) -> Result<(), StorageError> {
        if self.pending_confirmations.is_empty() {
            return Ok(());
        }
        let result = self
            .outbox
            .acknowledge_many(self.pending_confirmations.iter().map(String::as_str));
        view.track(FaultStage::OutboxConfirm, result)?;
        self.pending_confirmations.clear();
        Ok(())
    }

    fn drain(
        &mut self,
        incoming: Option<EventEnvelope>,
        view: &BrokerView,
    ) -> Result<Option<IngestAck>, StorageError> {
        // 重试原有 ID，即使其他进程已经重放确认计划且队列已空。
        self.confirm(view)?;
        let pending = view.track(FaultStage::OutboxScan, self.outbox.pending(wall_clock_ms()))?;
        let mut confirmed = Vec::with_capacity(pending.len());
        let mut response = None;
        // HTTP 到达时同时接收先入队的事件，避免抢在来源前一条离线事件之前。
        for queued in pending {
            let ack = self.append(queued.envelope, view)?;
            if incoming
                .as_ref()
                .is_some_and(|event| event.event_id == ack.event_id)
            {
                // 该次请求由 bridge 收到完整 ACK 后清理，也可由后续回扫收尾。
                response = Some(ack);
            } else {
                confirmed.push(ack.event_id);
            }
        }
        self.pending_confirmations = confirmed;
        self.confirm(view)?;
        if let Some(event) = incoming
            && response.is_none()
        {
            response = Some(self.append(event, view)?);
        }
        Ok(response)
    }
}

fn watch_outbox(
    paths: &AppPaths,
    commands: SyncSender<Work>,
) -> notify::Result<RecommendedWatcher> {
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<Event>| {
        let relevant = result.map_or(true, |event| {
            event.need_rescan()
                || (!event.kind.is_access()
                    && !event.kind.is_remove()
                    && event.paths.iter().any(|path| {
                        path.file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| name.starts_with("q-") && name.ends_with(".json"))
                    }))
        });
        if relevant {
            let _ = commands.try_send(Work::Rescan);
        }
    })?;
    watcher.watch(&paths.outbox, RecursiveMode::NonRecursive)?;
    Ok(watcher)
}

#[derive(Clone)]
struct HttpState {
    commands: SyncSender<Work>,
    authorization: String,
}

async fn ingest(
    State(state): State<HttpState>,
    request: Request,
) -> Result<Json<IngestAck>, StatusCode> {
    let headers = request.headers();
    if headers.contains_key(header::ORIGIN) {
        return Err(StatusCode::FORBIDDEN);
    }
    if !headers
        .get(header::AUTHORIZATION)
        .map(HeaderValue::as_bytes)
        .is_some_and(|token| bool::from(token.ct_eq(state.authorization.as_bytes())))
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let bytes = tokio::time::timeout(BODY_TIMEOUT, to_bytes(request.into_body(), MAX_EVENT_BYTES))
        .await
        .map_err(|_| StatusCode::REQUEST_TIMEOUT)?
        .map_err(|_| StatusCode::PAYLOAD_TOO_LARGE)?;
    let event = decode_event(&bytes).map_err(|_| StatusCode::BAD_REQUEST)?;
    let (reply, response) = oneshot::channel();
    state
        .commands
        .try_send(Work::Ingest(Box::new(event), reply))
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let ack = response
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(ack))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn writer() -> (tempfile::TempDir, AppPaths, Writer, BrokerView) {
        let root = tempfile::tempdir().unwrap();
        let paths = AppPaths::under(&root.path().canonicalize().unwrap().join("app"));
        let mut journal = JournalStore::open(paths.clone()).unwrap();
        journal.recover(wall_clock_ms()).unwrap();
        let view = BrokerView::new();
        let writer = Writer::new(journal, Outbox::new(paths.clone()).unwrap());
        (root, paths, writer, view)
    }

    fn event(id: &str) -> EventEnvelope {
        let mut event = decode_event(br#"{"schemaVersion":2,"eventId":"fixture","identity":{"source":"deepseek","installationId":"fixture-install","instanceId":"fixture-instance"},"sessionId":"fixture-session","turnId":"fixture-turn","occurredAtMs":100,"kind":{"type":"turn_started"}}"#).unwrap();
        event.event_id = id.into();
        event.occurred_at_ms = wall_clock_ms();
        event
    }

    fn block_journal(paths: &AppPaths) {
        fs::rename(&paths.journal, paths.root.join("working-journal")).unwrap();
        fs::create_dir(&paths.journal).unwrap();
    }

    fn restore_journal(paths: &AppPaths) {
        fs::remove_dir(&paths.journal).unwrap();
        fs::rename(paths.root.join("working-journal"), &paths.journal).unwrap();
    }

    #[test]
    fn empty_scan_and_duplicate_ack_leave_append_fault_until_append_succeeds() {
        let (_root, paths, mut writer, view) = writer();
        writer.process(Some(event("known")), &view).unwrap();

        block_journal(&paths);
        assert!(writer.process(Some(event("new")), &view).is_err());
        let failed = view.fault().expect("追加失败应该报告故障");
        writer.process(None, &view).unwrap();
        assert_eq!(view.fault(), Some(failed), "空回扫未执行日志追加");

        assert!(
            writer
                .process(Some(event("known")), &view)
                .unwrap()
                .unwrap()
                .duplicate
        );
        assert_eq!(view.fault(), Some(failed), "重复确认未执行日志追加");

        restore_journal(&paths);
        assert!(
            !writer
                .process(Some(event("new")), &view)
                .unwrap()
                .unwrap()
                .duplicate
        );
        assert_eq!(view.fault(), None);
    }

    #[test]
    fn compaction_and_append_only_clear_their_own_faults() {
        let (_root, paths, mut writer, view) = writer();
        block_journal(&paths);
        writer.compact(&view);
        assert_eq!(view.fault(), Some("事件日志压缩失败"));
        writer.process(None, &view).unwrap();
        assert_eq!(
            view.fault(),
            Some("事件日志压缩失败"),
            "空回扫未执行日志压缩"
        );

        assert!(writer.process(Some(event("new")), &view).is_err());
        restore_journal(&paths);
        writer.compact(&view);
        assert!(view.fault().is_some(), "压缩成功后仍有追加故障");
        assert_ne!(view.fault(), Some("事件日志压缩失败"));

        // 记录第二次压缩失败，验证接下来的成功追加也不隐藏它。
        block_journal(&paths);
        writer.compact(&view);
        restore_journal(&paths);
        writer.process(Some(event("new")), &view).unwrap();
        assert_eq!(view.fault(), Some("事件日志压缩失败"), "写入成功未执行压缩");
        writer.compact(&view);
        assert_eq!(view.fault(), None);
    }

    #[test]
    fn interrupted_confirmation_retries_its_ids_even_after_another_scan_finished_cleanup() {
        use std::{path::PathBuf, sync::Mutex};

        let (_root, paths, mut writer, view) = writer();
        let hold = Arc::new(Mutex::new(None::<PathBuf>));
        let from_clock = hold.clone();
        let backup = paths.root.join("blocked-queue-item");
        let clock_backup = backup.clone();
        writer.outbox = Outbox::with_clock(paths.clone(), move || {
            if let Some(slot) = from_clock.lock().unwrap().take() {
                // acknowledge_many 已读入这批槽位。阻塞后一个删除，保留原计划。
                fs::rename(&slot, &clock_backup).unwrap();
                fs::create_dir(slot).unwrap();
            }
            wall_clock_ms()
        })
        .unwrap();
        writer.outbox.enqueue(&event("first-confirm")).unwrap();
        writer.outbox.enqueue(&event("second-confirm")).unwrap();
        let pending = writer.outbox.pending(wall_clock_ms()).unwrap();
        *hold.lock().unwrap() = Some(pending[1].path.clone());

        assert!(writer.process(None, &view).is_err());
        assert_eq!(view.fault(), Some("待发送区确认失败"));
        let journal = fs::read_to_string(&paths.journal).unwrap();
        assert!(journal.contains("first-confirm"));
        assert!(journal.contains("second-confirm"));
        assert!(!pending[0].path.exists(), "确认计划已执行第一项删除");
        let disk_plan = fs::read_to_string(paths.outbox.join(".outbox-state.json")).unwrap();
        assert!(disk_plan.contains(pending[1].path.file_name().unwrap().to_str().unwrap()));

        view.set_fault(
            FaultStage::Listener,
            &io::Error::other("独立监听故障").into(),
        );
        fs::remove_dir(&pending[1].path).unwrap();
        fs::rename(backup, &pending[1].path).unwrap();
        let other_reader = Outbox::new(paths).unwrap();
        assert!(other_reader.pending(wall_clock_ms()).unwrap().is_empty());
        writer.process(None, &view).unwrap();
        assert_eq!(
            view.fault(),
            Some("本机入口已停止"),
            "应重试原确认 ID 并只清该故障"
        );
    }

    #[test]
    fn empty_scan_without_a_transaction_cannot_clear_confirmation_fault() {
        let (_root, _paths, mut writer, view) = writer();
        view.set_fault(
            FaultStage::OutboxConfirm,
            &io::Error::other("无确认计划的故障状态").into(),
        );
        writer.process(None, &view).unwrap();
        assert_eq!(view.fault(), Some("待发送区确认失败"));
    }

    #[test]
    fn file_processing_keeps_listener_failure_visible() {
        let (_root, _paths, mut writer, view) = writer();
        let stopped = "本机入口已停止";
        view.set_fault(
            FaultStage::Listener,
            &io::Error::other("测试监听线程退出").into(),
        );
        writer.process(None, &view).unwrap();
        writer.outbox.enqueue(&event("queued")).unwrap();
        writer.process(None, &view).unwrap();
        writer.compact(&view);
        assert_eq!(view.snapshot().sessions.len(), 1);
        assert_eq!(view.fault(), Some(stopped));
    }
}
