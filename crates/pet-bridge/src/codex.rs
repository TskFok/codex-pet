use pet_core::{
    EventEnvelope, EventKind, SCHEMA_VERSION, Source, SourceIdentity, TurnOutcome, decode_event,
};
use serde_json::Value;

pub type AdapterError = String;
pub const MAX_HOOK_BYTES: u64 = 8 * 1024 * 1024;

/// 公开 Hook 只转换有可靠语义的状态；不从工具参数或消息猜测请求关联。
pub fn normalize_codex(
    input: &Value,
    identity: &SourceIdentity,
    now_ms: u64,
) -> Result<Vec<EventEnvelope>, AdapterError> {
    if identity.source != Source::Codex
        || identity.installation_id.trim().is_empty()
        || identity.instance_id.trim().is_empty()
    {
        return Err("Codex 来源身份无效".into());
    }
    let name = required(input, "hook_event_name")?;
    let session_id = required(input, "session_id")?;
    // 子代理 Hook 使用父 session_id；在证明独立身份之前不合并到父会话。
    if input.get("agent_id").is_some_and(|id| !id.is_null()) || name.starts_with("Subagent") {
        return Ok(Vec::new());
    }
    let kind = match name {
        "UserPromptSubmit" => EventKind::TurnStarted,
        "Stop" => EventKind::TurnEnded {
            outcome: TurnOutcome::Unknown,
        },
        "Interrupt" => EventKind::TurnEnded {
            outcome: TurnOutcome::Interrupted,
        },
        // PermissionRequest 没有公开请求 ID 与解除事件。专用提问路径也未证实。
        _ => return Ok(Vec::new()),
    };
    let event = EventEnvelope {
        schema_version: SCHEMA_VERSION,
        event_id: uuid::Uuid::new_v4().to_string(),
        identity: identity.clone(),
        session_id: session_id.into(),
        parent_session_id: None,
        turn_id: Some(required(input, "turn_id")?.into()),
        upstream_event_id: None,
        sequence: None,
        occurred_at_ms: now_ms,
        received_at_ms: None,
        summary: None,
        project: None,
        kind,
    };
    let bytes = serde_json::to_vec(&event).map_err(|_| "统一事件序列化失败")?;
    decode_event(&bytes).map_err(|_| "统一事件不符合协议限制")?;
    Ok(vec![event])
}

fn required<'a>(input: &'a Value, field: &str) -> Result<&'a str, AdapterError> {
    input
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("Hook 缺少有效字段：{field}"))
}
