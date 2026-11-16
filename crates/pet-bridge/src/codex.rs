use pet_core::{
    EventEnvelope, EventKind, SCHEMA_VERSION, Source, SourceIdentity, TurnOutcome, decode_event,
};
use serde_json::Value;

pub type AdapterError = String;
pub const MAX_HOOK_BYTES: u64 = 8 * 1024 * 1024;

/// 原生 Hook 的 session_id 是根与所有后代共享的身份；agent_id 才是具体子线程。
/// 不将共享根 ID 伪装成直接父 ID，也不接受未公开的 parent 字段。
pub fn hook_session_id(input: &Value) -> Result<&str, AdapterError> {
    let root = required(input, "session_id")?;
    let agent = match input.get("agent_id") {
        None | Some(Value::Null) => None,
        Some(Value::String(agent)) if !agent.trim().is_empty() && agent != root => {
            Some(agent.as_str())
        }
        _ => return Err("Hook 子线程身份无效或与根身份冲突".into()),
    };
    if matches!(
        input["hook_event_name"].as_str(),
        Some("SubagentStart" | "SubagentStop")
    ) && agent.is_none()
    {
        return Err("子代理生命周期 Hook 缺少 agent_id".into());
    }
    Ok(agent.unwrap_or(root))
}

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
    let session_id = hook_session_id(input)?;
    // 锁定 producer 的子线程正常结束使用 SubagentStop；Interrupt 对子代理直接跳过。
    if matches!(name, "Stop" | "Interrupt") && input.get("agent_id").is_some_and(|id| !id.is_null())
    {
        return Ok(Vec::new());
    }
    let kind = match name {
        "UserPromptSubmit" | "SubagentStart" => EventKind::TurnStarted,
        "Stop" | "SubagentStop" => EventKind::TurnEnded {
            outcome: TurnOutcome::Unknown,
        },
        "Interrupt" => EventKind::TurnEnded {
            outcome: TurnOutcome::Interrupted,
        },
        // 当前适配器不支持 Codex 精确审批/提问等待及解除，不从工具事件猜测。
        _ => return Ok(Vec::new()),
    };
    let event = EventEnvelope {
        schema_version: SCHEMA_VERSION,
        event_id: uuid::Uuid::new_v4().to_string(),
        identity: identity.clone(),
        session_id: session_id.into(),
        parent_session_id: None,
        // 已核对生命周期 producer：这里是当前子 turn_context.sub_id，不是父回合。
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
