use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u16 = 2;
pub const RECOVERY_SCHEMA_VERSION: u16 = 3;
pub const MAX_EVENT_BYTES: usize = 64 * 1024;
pub const MAX_SUMMARY_CHARS: usize = 512;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Codex,
    Deepseek,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceIdentity {
    pub source: Source,
    pub installation_id: String,
    pub instance_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObservationGeneration {
    pub version: u16,
    pub stream_id: String,
    pub epoch_id: String,
    pub generation: u64,
    pub recovered_event_count: u64,
    pub ancestors: Vec<ObservationAncestor>,
}

impl ObservationGeneration {
    pub(crate) fn request_epoch(&self, birth: Option<u64>) -> Option<&str> {
        let Some(birth) = birth.filter(|birth| *birth < self.recovered_event_count) else {
            return Some(&self.epoch_id);
        };
        self.ancestors
            .iter()
            .rev()
            .find(|ancestor| birth >= ancestor.first_sequence)
            .map(|ancestor| ancestor.epoch_id.as_str())
    }

    fn covers_birth(&self, birth: u64) -> bool {
        self.ancestors
            .first()
            .is_none_or(|first| first.generation == 1 || birth >= first.first_sequence)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObservationAncestor {
    pub epoch_id: String,
    pub generation: u64,
    pub first_sequence: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecoveredQuestion {
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    pub birth_sequence: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionKey {
    pub source: Source,
    pub installation_id: String,
    pub session_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectContext {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AttentionReason {
    Answer,
    Approval,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RequestLifetime {
    #[default]
    Turn,
    UntilResolved,
}

impl RequestLifetime {
    fn is_turn(&self) -> bool {
        *self == Self::Turn
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnOutcome {
    Completed,
    Interrupted,
    Failed,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum EventKind {
    TurnStarted,
    AttentionRequested {
        request_id: String,
        reason: AttentionReason,
        #[serde(default, skip_serializing_if = "RequestLifetime::is_turn")]
        lifetime: RequestLifetime,
    },
    AttentionResolved {
        request_id: String,
    },
    TurnEnded {
        outcome: TurnOutcome,
    },
    SourceDisconnected,
    SourceSnapshot {
        sessions: Vec<SessionProjection>,
    },
    SourceRecovery {
        snapshot_event_count: u64,
        running: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        timed: Option<Vec<RecoveredQuestion>>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventEnvelope {
    pub schema_version: u16,
    pub event_id: String,
    pub identity: SourceIdentity,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream_event_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observation: Option<ObservationGeneration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question_birth: Option<u64>,
    pub occurred_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub received_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<ProjectContext>,
    pub kind: EventKind,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Running,
    NeedsAttention,
    Ended,
    Interrupted,
    Failed,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PendingRequest {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    pub reason: AttentionReason,
    #[serde(default, skip_serializing_if = "RequestLifetime::is_turn")]
    pub lifetime: RequestLifetime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionProjection {
    pub key: SessionKey,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    pub status: SessionStatus,
    pub pending: Vec<PendingRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<ProjectContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub unread: bool,
    pub last_event_at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum NotificationKind {
    Attention {
        request_id: String,
        reason: AttentionReason,
        #[serde(default, skip_serializing_if = "RequestLifetime::is_turn")]
        lifetime: RequestLifetime,
        resolved: bool,
    },
    TurnEnded {
        outcome: TurnOutcome,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationRecord {
    pub id: String,
    pub key: SessionKey,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    pub kind: NotificationKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<ProjectContext>,
    pub occurred_at_ms: u64,
    pub read: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PetSnapshot {
    pub revision: u64,
    pub sessions: Vec<SessionProjection>,
    pub notifications: Vec<NotificationRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<SessionKey>,
}

#[derive(Debug)]
pub struct ProtocolError(pub String);
impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ProtocolError {}

/// 跨进程入口：完整解析 JSON 后验证协议，不截断超限输入。
pub fn decode_event(bytes: &[u8]) -> Result<EventEnvelope, ProtocolError> {
    if bytes.len() > MAX_EVENT_BYTES {
        return invalid("统一事件超过 64 KiB");
    }
    let event: EventEnvelope =
        serde_json::from_slice(bytes).map_err(|error| ProtocolError(error.to_string()))?;
    match event.schema_version {
        1 | SCHEMA_VERSION => {
            if event.observation.is_some()
                || event.question_birth.is_some()
                || matches!(event.kind, EventKind::SourceRecovery { .. })
            {
                return invalid("旧版协议不支持来源代次");
            }
        }
        RECOVERY_SCHEMA_VERSION => validate_recovery(&event)?,
        _ => return invalid("不支持的事件协议版本"),
    }
    if event.schema_version == 1 {
        let has_until = match &event.kind {
            EventKind::AttentionRequested { lifetime, .. } => {
                *lifetime == RequestLifetime::UntilResolved
            }
            EventKind::SourceSnapshot { sessions } => sessions.iter().any(|session| {
                session
                    .pending
                    .iter()
                    .any(|pending| pending.lifetime == RequestLifetime::UntilResolved)
            }),
            _ => false,
        };
        if has_until {
            return invalid("协议 v1 不支持跨回合请求生命周期");
        }
    }
    nonempty(&event.event_id)?;
    nonempty(&event.identity.installation_id)?;
    nonempty(&event.identity.instance_id)?;
    nonempty(&event.session_id)?;
    optional_id(&event.parent_session_id)?;
    optional_id(&event.turn_id)?;
    optional_id(&event.upstream_event_id)?;
    if event.parent_session_id.as_deref() == Some(&event.session_id) {
        return invalid("会话不能是自己的父会话");
    }
    summary(&event.summary)?;
    project(&event.project)?;
    match &event.kind {
        EventKind::AttentionRequested { request_id, .. }
        | EventKind::AttentionResolved { request_id } => nonempty(request_id)?,
        EventKind::SourceSnapshot { sessions } => {
            let mut keys = std::collections::HashSet::new();
            for session in sessions {
                if session.key.source != event.identity.source
                    || session.key.installation_id != event.identity.installation_id
                {
                    return invalid("来源快照包含其他来源或安装的会话");
                }
                nonempty(&session.key.session_id)?;
                if !keys.insert(&session.key) {
                    return invalid("来源快照会话重复");
                }
                optional_id(&session.turn_id)?;
                optional_id(&session.parent_session_id)?;
                if session.parent_session_id.as_deref() == Some(&session.key.session_id) {
                    return invalid("会话不能是自己的父会话");
                }
                summary(&session.summary)?;
                project(&session.project)?;
                if (session.status == SessionStatus::NeedsAttention && session.pending.is_empty())
                    || (!session.pending.is_empty()
                        && !matches!(
                            session.status,
                            SessionStatus::NeedsAttention | SessionStatus::Unknown
                        ))
                {
                    return invalid("会话状态与待处理请求不一致");
                }
                let mut requests = std::collections::HashSet::new();
                for pending in &session.pending {
                    nonempty(&pending.id)?;
                    optional_id(&pending.turn_id)?;
                    summary(&pending.summary)?;
                    if (pending.lifetime == RequestLifetime::Turn
                        && pending.turn_id != session.turn_id)
                        || !requests.insert((&pending.turn_id, &pending.id))
                    {
                        return invalid("请求回合不匹配或请求重复");
                    }
                }
            }
        }
        _ => {}
    }
    Ok(event)
}

fn validate_recovery(event: &EventEnvelope) -> Result<(), ProtocolError> {
    let Some(proof) = &event.observation else {
        return invalid("恢复协议缺少来源证明");
    };
    let max_ancestors = proof.generation.saturating_sub(1).min(128);
    if event.identity.source != Source::Deepseek
        || proof.version != 1
        || proof.epoch_id != event.identity.instance_id
        || proof.generation == 0
        || proof.ancestors.len() as u64 != max_ancestors
    {
        return invalid("来源代次证明不完整");
    }
    nonempty(&proof.stream_id)?;
    let mut last_sequence = 0;
    let mut epochs = std::collections::HashSet::from([&proof.epoch_id]);
    for (index, ancestor) in proof.ancestors.iter().enumerate() {
        nonempty(&ancestor.epoch_id)?;
        if !epochs.insert(&ancestor.epoch_id)
            || ancestor.generation != proof.generation - max_ancestors + index as u64
            || ancestor.first_sequence < last_sequence
            || ancestor.first_sequence > proof.recovered_event_count
        {
            return invalid("来源代次的前缀位置无效");
        }
        last_sequence = ancestor.first_sequence;
    }
    match &event.kind {
        EventKind::SourceRecovery {
            snapshot_event_count,
            running,
            timed,
        } => {
            if *snapshot_event_count < proof.recovered_event_count
                || event.sequence != snapshot_event_count.checked_sub(1)
                || *running != event.turn_id.is_some()
                || event.question_birth.is_some()
            {
                return invalid("会话恢复快照的切点或回合无效");
            }
            if let Some(questions) = timed {
                let mut identities = std::collections::HashSet::new();
                for question in questions {
                    nonempty(&question.request_id)?;
                    optional_id(&question.turn_id)?;
                    if question.birth_sequence >= *snapshot_event_count
                        || !proof.covers_birth(question.birth_sequence)
                        || !identities.insert((&question.turn_id, &question.request_id))
                    {
                        return invalid("定时问题的出生位置或请求标识无效");
                    }
                }
            }
        }
        EventKind::SourceSnapshot { .. } | EventKind::SourceDisconnected => {
            return invalid("恢复协议需要明确的单会话状态");
        }
        EventKind::AttentionRequested { lifetime, .. } => {
            if event.question_birth.is_some() && *lifetime != RequestLifetime::UntilResolved {
                return invalid("定时问题必须等待精确解除");
            }
        }
        EventKind::AttentionResolved { .. } => {}
        _ if event.question_birth.is_some() => return invalid("只有提问才携带出生位置"),
        _ => {}
    }
    if event
        .question_birth
        .is_some_and(|birth| !proof.covers_birth(birth))
    {
        return invalid("来源证明未覆盖问题的出生位置");
    }
    if !matches!(event.kind, EventKind::SourceRecovery { .. })
        && event
            .sequence
            .is_some_and(|seq| seq < proof.recovered_event_count)
    {
        return invalid("新代事件位于已恢复的前缀内");
    }
    Ok(())
}

fn invalid<T>(message: &str) -> Result<T, ProtocolError> {
    Err(ProtocolError(message.into()))
}
fn nonempty(value: &str) -> Result<(), ProtocolError> {
    if value.trim().is_empty() {
        invalid("标识或名称不能为空")
    } else {
        Ok(())
    }
}
fn optional_id(value: &Option<String>) -> Result<(), ProtocolError> {
    if let Some(value) = value {
        nonempty(value)?;
    }
    Ok(())
}
fn summary(value: &Option<String>) -> Result<(), ProtocolError> {
    if value
        .as_ref()
        .is_some_and(|value| value.chars().count() > MAX_SUMMARY_CHARS)
    {
        invalid("摘要超过 512 个 Unicode 字符")
    } else {
        Ok(())
    }
}
fn project(value: &Option<ProjectContext>) -> Result<(), ProtocolError> {
    if let Some(value) = value {
        nonempty(&value.name)?;
        optional_id(&value.directory)?;
    }
    Ok(())
}

impl EventEnvelope {
    pub fn session_key(&self) -> SessionKey {
        SessionKey {
            source: self.identity.source,
            installation_id: self.identity.installation_id.clone(),
            session_id: self.session_id.clone(),
        }
    }
}
