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
    fs,
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
    fault: RwLock<Option<&'static str>>,
    updates: broadcast::Sender<PetSnapshot>,
}

impl BrokerView {
    pub(crate) fn snapshot(&self) -> PetSnapshot {
        self.0.latest.read().unwrap().clone()
    }

    pub(crate) fn fault(&self) -> Option<&'static str> {
        *self.0.fault.read().unwrap()
    }

    fn set_fault(&self, message: &'static str, error: &BrokerError) {
        let mut fault = self.0.fault.write().unwrap();
        if *fault != Some(message) {
            eprintln!("桌宠通知：{message}，{error}");
        }
        *fault = Some(message);
    }

    fn publish(&self, snapshot: PetSnapshot) {
        let mut latest = self.0.latest.write().unwrap();
        if latest.revision != snapshot.revision {
            *latest = snapshot;
            let _ = self.0.updates.send(latest.clone());
        }
    }
}

impl BrokerHandle {
    /// 读取日志和补发会执行磁盘操作，应用须从后台启动。
    pub fn start(paths: AppPaths) -> Result<Self, BrokerError> {
        let mut journal = JournalStore::open(paths.clone())?;
        journal.recover(wall_clock_ms())?;
        let mut writer = Writer {
            journal,
            outbox: Outbox::new(paths.clone())?,
        };
        let (commands, jobs) = mpsc::sync_channel(64);
        // 先注册监测，再读初始队列。扫描期间发生的变更会留在频道内。
        let watcher = match watch_outbox(&paths, commands.clone()) {
            Ok(watcher) => Some(watcher),
            Err(error) => {
                eprintln!("桌宠通知：无法监测待发送区，继续定时回扫，{error}");
                None
            }
        };
        writer.drain(None)?;
        let view = BrokerView(Arc::new(Shared {
            latest: RwLock::new(writer.journal.snapshot()),
            fault: RwLock::new(None),
            updates: broadcast::channel(32).0,
        }));

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

        let worker_view = view.clone();
        let worker_paths = paths.clone();
        let worker_discovery = discovery.clone();
        let worker = thread::Builder::new()
            .name("pet-notification-writer".into())
            .spawn(move || {
                let _watcher = watcher;
                writer.run(jobs, worker_view);
                // 在仍持有日志写锁时移除本次发现记录。
                if Discovery::read(&worker_paths).is_ok_and(|current| current == worker_discovery) {
                    let _ = fs::remove_file(&worker_paths.discovery);
                }
            })?;
        let (stop_server, shutdown) = oneshot::channel();
        let mut broker = Self {
            view: view.clone(),
            commands: commands.clone(),
            worker: Some(worker),
            server: None,
            stop_server: Some(stop_server),
        };
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
                            result = async { axum::serve(listener, app).await } => {
                                if let Err(error) = result {
                                    view.set_fault("本机入口已停止", &error.into());
                                }
                            }
                            _ = shutdown => {}
                        }
                    });
                })?,
        );
        discovery.publish(&paths)?;
        Ok(broker)
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
}

impl Writer {
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
                match self.journal.compact(wall_clock_ms()) {
                    Ok(()) => view.publish(self.journal.snapshot()),
                    Err(error) => view.set_fault("事件日志压缩失败", &error),
                }
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

    fn process(
        &mut self,
        incoming: Option<EventEnvelope>,
        view: &BrokerView,
    ) -> Result<Option<IngestAck>, StorageError> {
        let result = self.drain(incoming);
        match &result {
            Ok(_) => {
                *view.0.fault.write().unwrap() = None;
                view.publish(self.journal.snapshot());
            }
            Err(error) => view.set_fault("收件尚未完成", error),
        }
        result
    }

    fn drain(
        &mut self,
        incoming: Option<EventEnvelope>,
    ) -> Result<Option<IngestAck>, StorageError> {
        let pending = self.outbox.pending(wall_clock_ms())?;
        let mut confirmed = Vec::with_capacity(pending.len());
        let mut response = None;
        // HTTP 到达时同时接收先入队的事件，避免抢在来源前一条离线事件之前。
        for queued in pending {
            let ack = self.journal.ingest(queued.envelope)?;
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
        self.outbox
            .acknowledge_many(confirmed.iter().map(String::as_str))?;
        if let Some(event) = incoming
            && response.is_none()
        {
            response = Some(self.journal.ingest(event)?);
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
