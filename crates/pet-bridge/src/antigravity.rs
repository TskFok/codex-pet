use pet_core::{
    EventEnvelope, EventKind, ProjectContext, SCHEMA_VERSION, Source, SourceIdentity, TurnOutcome,
    decode_event,
};
use serde_json::Value;

pub type AdapterError = String;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookEvent {
    SessionStart,
    PreInvocation,
    Stop,
}

impl HookEvent {
    pub fn parse(value: &str) -> Result<Self, AdapterError> {
        match value {
            "session-start" | "SessionStart" => Ok(Self::SessionStart),
            "pre-invocation" | "PreInvocation" => Ok(Self::PreInvocation),
            "stop" | "Stop" => Ok(Self::Stop),
            _ => Err(format!("不支持的 Antigravity Hook 事件：{value}")),
        }
    }

    pub fn cli_name(self) -> &'static str {
        match self {
            Self::SessionStart => "session-start",
            Self::PreInvocation => "pre-invocation",
            Self::Stop => "stop",
        }
    }

    /// 遵循 Antigravity Hook 输出契约：绝不阻断或强制继续宿主循环。
    pub fn stdout_response(self) -> &'static str {
        match self {
            Self::SessionStart | Self::PreInvocation => "{}",
            Self::Stop => "{\"decision\":\"allow\"}",
        }
    }
}

/// Antigravity 公共 Hook 输入以 camelCase `conversationId` 标识会话。
pub fn hook_session_id(input: &Value) -> Result<&str, AdapterError> {
    required(input, "conversationId")
}

/// 仅提取有明确语义的生命周期状态，绝不保存提示词、错误原文、transcript 或完整本机路径。
pub fn normalize_antigravity(
    event: HookEvent,
    input: &Value,
    identity: &SourceIdentity,
    now_ms: u64,
) -> Result<Vec<EventEnvelope>, AdapterError> {
    if identity.source != Source::Antigravity
        || identity.installation_id.trim().is_empty()
        || identity.instance_id.trim().is_empty()
    {
        return Err("Antigravity 来源身份无效".into());
    }
    let session_id = hook_session_id(input)?;
    let (turn_id, kind) = match event {
        HookEvent::SessionStart => (None, EventKind::TurnStarted),
        HookEvent::PreInvocation => {
            if let Some(invocation) = optional_u64(input, "invocationNum")?
                && invocation > 1
            {
                return Ok(Vec::new());
            }
            (None, EventKind::TurnStarted)
        }
        HookEvent::Stop => {
            let turn_id =
                optional_u64(input, "executionNum")?.map(|value| format!("execution-{value}"));
            let outcome = match input.get("terminationReason").and_then(Value::as_str) {
                Some("NO_TOOL_CALL") => TurnOutcome::Completed,
                Some("USER_CANCELED") => TurnOutcome::Interrupted,
                Some("ERROR") => TurnOutcome::Failed,
                _ => TurnOutcome::Unknown,
            };
            (turn_id, EventKind::TurnEnded { outcome })
        }
    };
    let is_subagent = input.get("isSubagent").and_then(Value::as_bool) == Some(true)
        || input
            .get("parentConversationId")
            .and_then(Value::as_str)
            .map(str::trim)
            .is_some_and(|parent| !parent.is_empty() && parent != session_id);
    let envelope = EventEnvelope {
        schema_version: SCHEMA_VERSION,
        event_id: uuid::Uuid::new_v4().to_string(),
        identity: identity.clone(),
        session_id: session_id.into(),
        parent_session_id: None,
        is_subagent,
        turn_id,
        upstream_event_id: None,
        sequence: None,
        observation: None,
        question_birth: None,
        direct_question_owner: None,
        occurred_at_ms: now_ms,
        received_at_ms: None,
        summary: None,
        project: project_from_workspaces(input),
        kind,
    };
    let bytes = serde_json::to_vec(&envelope).map_err(|_| "统一事件序列化失败")?;
    decode_event(&bytes).map_err(|_| "统一事件不符合协议限制")?;
    Ok(vec![envelope])
}

fn required<'a>(input: &'a Value, field: &str) -> Result<&'a str, AdapterError> {
    input
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("Hook 缺少有效字段：{field}"))
}

fn optional_u64(input: &Value, field: &str) -> Result<Option<u64>, AdapterError> {
    match input.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(value)) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("Hook 字段无效：{field}")),
        _ => Err(format!("Hook 字段无效：{field}")),
    }
}

fn project_from_workspaces(input: &Value) -> Option<ProjectContext> {
    let first = input
        .get("workspacePaths")
        .and_then(Value::as_array)?
        .iter()
        .find_map(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())?;
    let trimmed = first.trim_end_matches(['/', '\\']);
    let name = trimmed
        .rsplit(['/', '\\'])
        .next()
        .map(str::trim)
        .filter(|segment| {
            !segment.is_empty()
                && !segment.chars().any(char::is_control)
                && segment.chars().count() <= 128
        })?;
    Some(ProjectContext {
        name: name.into(),
        directory: None,
    })
}
