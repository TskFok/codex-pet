//! 先耐久入队，再尝试向本机桌宠确认投递。
use crate::outbox::Outbox;
use pet_core::{
    AppPaths, EventEnvelope, IngestAck, StorageError,
    paths::{atomic_write, open_regular, sync_directory},
};
use serde::{Deserialize, Serialize};
use std::{fs::OpenOptions, io::Read, time::Duration};

pub const EVENTS_ROUTE: &str = "/v1/events";
const DISCOVERY_VERSION: u16 = 1;
const MAX_DISCOVERY_BYTES: usize = 4096;
const MAX_ACK_BYTES: usize = 4096;
const NETWORK_BUDGET: Duration = Duration::from_millis(200);

pub type DeliveryError = StorageError;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Discovery {
    pub schema_version: u16,
    pub port: u16,
    pub token: String,
}

impl Discovery {
    pub fn new(port: u16) -> Self {
        Self {
            schema_version: DISCOVERY_VERSION,
            port,
            token: uuid::Uuid::new_v4().simple().to_string(),
        }
    }

    pub fn publish(&self, paths: &AppPaths) -> Result<(), StorageError> {
        paths.prepare_root()?;
        atomic_write(
            &paths.discovery,
            ".discovery-stage-",
            &serde_json::to_vec(self)?,
        )?;
        sync_directory(&paths.root)
    }

    pub fn read(paths: &AppPaths) -> Result<Self, StorageError> {
        let file = open_regular(&paths.discovery, OpenOptions::new().read(true))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if file.metadata()?.permissions().mode() & 0o077 != 0 {
                return Err(StorageError::UnsafePath(paths.discovery.clone()));
            }
        }
        let mut bytes = Vec::new();
        file.take((MAX_DISCOVERY_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_DISCOVERY_BYTES {
            return Err(StorageError::InvalidData("桌宠发现文件超限"));
        }
        let discovery: Self = serde_json::from_slice(&bytes)?;
        if discovery.schema_version != DISCOVERY_VERSION
            || discovery.port == 0
            || discovery.token.len() != 32
            || !discovery.token.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(StorageError::InvalidData("桌宠发现记录无效"));
        }
        Ok(discovery)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeliveryOutcome {
    Delivered,
    Queued,
}

pub fn deliver(event: &EventEnvelope, paths: &AppPaths) -> Result<DeliveryOutcome, DeliveryError> {
    let outbox = Outbox::new(paths.clone())?;
    outbox.enqueue(event)?;
    let Ok(discovery) = Discovery::read(paths) else {
        return Ok(DeliveryOutcome::Queued);
    };
    let bytes = serde_json::to_vec(event)?;
    if confirms(discovery, bytes, &event.event_id) {
        outbox.acknowledge(&event.event_id)?;
        Ok(DeliveryOutcome::Delivered)
    } else {
        Ok(DeliveryOutcome::Queued)
    }
}

fn confirms(discovery: Discovery, bytes: Vec<u8>, event_id: &str) -> bool {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return false;
    };
    runtime.block_on(async {
        tokio::time::timeout(NETWORK_BUDGET, receive_ack(discovery, bytes))
            .await
            .is_ok_and(|ack| ack.is_some_and(|ack| ack.event_id == event_id))
    })
}

async fn receive_ack(discovery: Discovery, bytes: Vec<u8>) -> Option<IngestAck> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .ok()?;
    let mut reply = client
        .post(format!("http://127.0.0.1:{}{EVENTS_ROUTE}", discovery.port))
        .bearer_auth(discovery.token)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(bytes)
        .send()
        .await
        .ok()?;
    if reply.status() != reqwest::StatusCode::OK
        || reply
            .content_length()
            .is_some_and(|length| length > MAX_ACK_BYTES as u64)
    {
        return None;
    }
    let mut body = Vec::new();
    while let Some(chunk) = reply.chunk().await.ok()? {
        if chunk.len() > MAX_ACK_BYTES - body.len() {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).ok()
}
